//! The single-line renderer: one unbroken line through every point.
//!
//! The line is a travelling-salesman tour, built and improved as D. S. Johnson and L. A. McGeoch
//! describe in "The traveling salesman problem: a case study in local optimization", in *Local
//! Search in Combinatorial Optimization*, Wiley, 1997, pp. 215-310. Nearest-neighbour construction
//! starts from the point nearest the top-left corner of the points' bounding box. 2-opt and Or-opt
//! (moving a segment of 1 to 3 points) then shorten the tour, trying for each point only its
//! Delaunay neighbours, nearest first, and a queue of don't-look bits decides which points to try.
//! The tour is an array with each point's position in it, and a move reverses the shorter of the
//! two paths it could reverse.
//!
//! Uncrossing two edges always shortens a tour, so a 2-opt local optimum in the plane has no
//! crossings. The candidate lists can miss such a move, as when a long edge crosses short ones, so
//! once the queue is empty a scan over a grid of cells uncrosses whatever crossings are left and
//! the queue runs again, until a scan finds none.

use std::collections::VecDeque;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::graph::delaunay_edges;
use crate::settings::{defaults_for, schema_for, validate};
use crate::spatial::{bounds, GridIndex};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, Points};

/// Points taken from the queue per step, so that Stop stays responsive on a large tour.
const CHECKS_PER_STEP: usize = 2000;

/// The least gain, in mm, that counts as an improvement; rounding cannot make a move this good.
const MIN_GAIN: f64 = 1e-12;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the single-line renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TspSettings {
    /// Shorten the line and remove its crossings; off leaves the nearest-neighbour line.
    pub optimize: bool,
    /// End the line where it started.
    pub closed: bool,
}

impl Default for TspSettings {
    fn default() -> Self {
        TspSettings {
            optimize: true,
            closed: false,
        }
    }
}

/// The side of a grid cell over `n` points within the bounds `lo` to `hi`: about two points per
/// cell, and never more than 257 cells along an axis.
fn cell_size(lo: [f64; 2], hi: [f64; 2], n: usize) -> f64 {
    let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
    (2.0 * w * h / n as f64)
        .sqrt()
        .max(w.max(h) / 256.0)
        .max(1e-9)
}

/// Whether the segments `a`-`b` and `c`-`d` cross at a point inside both, by the signs of their
/// orientations.
fn crosses(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let turn = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    // Not `signum`, which gives 1 for 0, so that touching segments do not count.
    let apart = |x: f64, y: f64| (x > 0.0 && y < 0.0) || (x < 0.0 && y > 0.0);
    apart(turn(a, b, c), turn(a, b, d)) && apart(turn(c, d, a), turn(c, d, b))
}

/// The points in nearest-neighbour order, from the point nearest the top-left corner of their
/// bounding box. Ties go to the lowest index.
fn nearest_neighbour(pts: &[[f64; 2]]) -> Vec<usize> {
    let n = pts.len();
    let (lo, hi) = bounds(pts);
    let index = GridIndex::new(pts, cell_size(lo, hi, n));
    let mut alive = vec![true; n];
    let mut order = Vec::with_capacity(n);
    let mut at = lo;
    while order.len() < n {
        // With no point at a finite distance, as with NaN ones, take the first left.
        let i = index
            .nearest(at, &alive)
            .or_else(|| alive.iter().position(|&a| a))
            .expect("a point is still to be visited");
        alive[i] = false;
        order.push(i);
        at = pts[i];
    }
    order
}

/// A cyclic tour as an array of point indices, with each point's position in it.
struct Tour {
    pts: Arc<Points>,
    order: Vec<usize>,
    pos: Vec<usize>,
}

impl Tour {
    fn new(pts: Arc<Points>, order: Vec<usize>) -> Tour {
        let mut pos = vec![0; order.len()];
        for (i, &p) in order.iter().enumerate() {
            pos[p] = i;
        }
        Tour { pts, order, pos }
    }

    fn dist(&self, a: usize, b: usize) -> f64 {
        let (p, q) = (self.pts.0[a], self.pts.0[b]);
        let (dx, dy) = (p[0] - q[0], p[1] - q[1]);
        (dx * dx + dy * dy).sqrt()
    }

    /// The point after `a`, going forward or backward.
    fn step(&self, a: usize, forward: bool) -> usize {
        let n = self.order.len();
        let i = if forward { 1 } else { n - 1 };
        self.order[(self.pos[a] + i) % n]
    }

    /// How many steps from `a` to `x`, going forward or backward.
    fn steps(&self, a: usize, x: usize, forward: bool) -> usize {
        let n = self.order.len();
        let (from, to) = if forward { (a, x) } else { (x, a) };
        (self.pos[to] + n - self.pos[from]) % n
    }

    /// Replace the edges `a`-`b` and `c`-`d` by `a`-`c` and `b`-`d`, where `b` follows `a` and `d`
    /// follows `c` in the same direction. Either the path from `b` to `c` or the one from `d` to
    /// `a` is reversed, whichever is shorter; both give the same cycle.
    fn flip(&mut self, a: usize, b: usize, c: usize, d: usize) {
        let n = self.order.len();
        // Going forward, the path to reverse runs from `from` to `to`.
        let (from, to) = if self.step(a, true) == b {
            (b, c)
        } else {
            (a, d)
        };
        let (mut i, mut j) = (self.pos[from], self.pos[to]);
        let len = (j + n - i) % n + 1;
        if 2 * len > n {
            (i, j) = ((j + 1) % n, (i + n - 1) % n);
        }
        for _ in 0..len.min(n - len) / 2 {
            self.order.swap(i, j);
            self.pos[self.order[i]] = i;
            self.pos[self.order[j]] = j;
            i = (i + 1) % n;
            j = (j + n - 1) % n;
        }
    }

    /// The first improving 2-opt move from `a`, applied: for each tour neighbour `b` of `a`, and
    /// each candidate `c` closer to `a` than `b` is, `c`'s neighbour `e` on the same side. Gives
    /// the points whose edges changed.
    fn two_opt(&mut self, a: usize, cand: &[usize]) -> Option<Vec<usize>> {
        for forward in [true, false] {
            let b = self.step(a, forward);
            let ab = self.dist(a, b);
            for &c in cand {
                let ac = self.dist(a, c);
                if ac >= ab {
                    break;
                }
                let e = self.step(c, forward);
                if c == b || e == a {
                    continue;
                }
                if ab + self.dist(c, e) - ac - self.dist(b, e) > MIN_GAIN {
                    self.flip(a, b, c, e);
                    return Some(vec![a, b, c, e]);
                }
            }
        }
        None
    }

    /// The first improving Or-opt move from `a`, applied: the segment of 1 to 3 points from `a`,
    /// going either way, moves between a candidate `c` and the point `d` after it in that
    /// direction, either way round. Gives the points whose edges changed.
    fn or_opt(&mut self, a: usize, cand: &[usize]) -> Option<Vec<usize>> {
        for forward in [true, false] {
            let p = self.step(a, !forward);
            let mut s = a;
            for k in 1..=3 {
                if k > 1 {
                    s = self.step(s, forward);
                }
                let q = self.step(s, forward);
                if q == p {
                    break;
                }
                // The segment runs from `a` to `s`, between `p` and `q`.
                let cut = self.dist(p, a) + self.dist(s, q) - self.dist(p, q);
                for &c in cand {
                    let d = self.step(c, forward);
                    if self.steps(a, c, forward) < k || self.steps(a, d, forward) < k {
                        continue;
                    }
                    let base = cut + self.dist(c, d);
                    let same = base - self.dist(c, a) - self.dist(s, d);
                    let turned = base - self.dist(c, s) - self.dist(a, d);
                    if same.max(turned) > MIN_GAIN {
                        // p a..s q, c d  ->  p c..q s..a d  ->  p q..c s..a d  [->  p q..c a..s d]
                        self.flip(p, a, c, d);
                        self.flip(p, c, q, s);
                        if same >= turned {
                            self.flip(c, s, a, d);
                        }
                        return Some(vec![p, q, a, s, c, d]);
                    }
                }
            }
        }
        None
    }

    /// Whether `a`-`b` is an edge of the tour.
    fn has_edge(&self, a: usize, b: usize) -> bool {
        self.step(a, true) == b || self.step(a, false) == b
    }

    /// Uncross the pairs of crossing edges that one scan finds, each by the 2-opt move that
    /// uncrosses it, while both edges are still in the tour. The candidate lists miss such a move
    /// when neither new edge is a Delaunay edge, as when a long edge crosses short ones. The scan
    /// buckets each edge into the grid cells its bounding box covers and tests the pairs sharing
    /// a cell. Gives the points whose edges changed.
    fn uncross(&mut self) -> Vec<usize> {
        let n = self.order.len();
        let pts = &self.pts.0;
        let at = |i: usize| pts[self.order[i % n]];
        let (lo, hi) = bounds(pts);
        let cell = cell_size(lo, hi, n);
        let span = |a: usize| ((hi[a] - lo[a]) / cell).floor() as usize + 1;
        let (cols, rows) = (span(0), span(1));
        // A float to usize cast saturates and sends NaN to 0; `min` keeps it on the grid.
        let col = |v: f64, a: usize, n: usize| (((v - lo[a]) / cell).floor() as usize).min(n - 1);
        let mut cells = vec![Vec::new(); cols * rows];
        for i in 0..n {
            let (p, q) = (at(i), at(i + 1));
            for r in col(p[1].min(q[1]), 1, rows)..=col(p[1].max(q[1]), 1, rows) {
                for c in col(p[0].min(q[0]), 0, cols)..=col(p[0].max(q[0]), 0, cols) {
                    cells[r * cols + c].push(i);
                }
            }
        }
        let mut pairs = Vec::new();
        for list in &cells {
            for (k, &i) in list.iter().enumerate() {
                for &j in &list[k + 1..] {
                    if crosses(at(i), at(i + 1), at(j), at(j + 1)) {
                        pairs.push([
                            self.order[i],
                            self.order[(i + 1) % n],
                            self.order[j],
                            self.order[(j + 1) % n],
                        ]);
                    }
                }
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        let mut touched = Vec::new();
        for [a, b, c, e] in pairs {
            if !self.has_edge(a, b) || !self.has_edge(c, e) {
                continue;
            }
            // `flip` needs `e` after `c` in the direction that `b` comes after `a`.
            let (c, e) = if (self.step(a, true) == b) == (self.step(c, true) == e) {
                (c, e)
            } else {
                (e, c)
            };
            if self.dist(a, b) + self.dist(c, e) - self.dist(a, c) - self.dist(b, e) > MIN_GAIN {
                self.flip(a, b, c, e);
                touched.extend([a, b, c, e]);
            }
        }
        touched
    }

    /// The tour's points in order from position `start`, with the first again at the end when
    /// `closed`.
    fn path(&self, start: usize, closed: bool) -> Vec<[f64; 2]> {
        let n = self.order.len();
        let extra = usize::from(closed && n > 0);
        (0..n + extra)
            .map(|i| self.pts.0[self.order[(start + i) % n]])
            .collect()
    }
}

/// Draw one line through every point: a nearest-neighbour tour, shortened by 2-opt and Or-opt
/// and uncrossed when `optimize` is on, then opened at its longest edge unless `closed` is on.
/// Without `optimize` the nearest-neighbour order is the line. The line's vertices are the points
/// themselves, so it stays wherever the placer kept them.
///
/// The run reports a frame after construction, labelled `construction`, then one per
/// [`CHECKS_PER_STEP`] points taken from the queue, labelled `improving: k moves`, each holding
/// the tour so far.
pub struct TspRenderer;

impl Stage for TspRenderer {
    fn name(&self) -> &'static str {
        "tsp"
    }
    fn label(&self) -> &'static str {
        "Single line"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<TspSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<TspSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<TspSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(TspRun {
            tour: Tour::new(inputs.points().clone(), Vec::new()),
            s,
            cand: Vec::new(),
            queue: VecDeque::new(),
            queued: Vec::new(),
            moves: 0,
            checked: 0,
            phase: Phase::Construct,
        }))
    }
}

/// Where a run is.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// The next step builds the nearest-neighbour tour.
    Construct,
    /// The next step takes points from the queue.
    Improve,
    /// The next step gives the line.
    Finish,
    /// The line has been given.
    Done,
}

struct TspRun {
    /// Empty until construction.
    tour: Tour,
    s: TspSettings,
    /// Each point's Delaunay neighbours, nearest first, ties to the lowest index.
    cand: Vec<Vec<usize>>,
    /// The points to try, each at most once.
    queue: VecDeque<usize>,
    /// Per point, whether it is in the queue.
    queued: Vec<bool>,
    /// Improving moves made so far.
    moves: usize,
    /// The most points ever off the queue, for progress that never goes back.
    checked: usize,
    phase: Phase,
}

impl TspRun {
    fn frame(&self, progress: f32, label: String, path: Vec<[f64; 2]>) -> Frame {
        let mut paths = Paths::new();
        if !path.is_empty() {
            paths.push(&path);
        }
        Frame {
            stage: String::new(),
            progress,
            label,
            points: None,
            paths: Some(paths),
        }
    }

    /// Build the nearest-neighbour tour, and the candidate lists if it is to be improved.
    fn construct(&mut self) -> Frame {
        let pts = self.tour.pts.clone();
        let n = pts.0.len();
        self.tour = Tour::new(pts.clone(), nearest_neighbour(&pts.0));
        // Under four points every tour is as short as any other.
        if self.s.optimize && n >= 4 {
            let mut cand = vec![Vec::new(); n];
            for (a, b) in delaunay_edges(&pts.0) {
                cand[a].push(b);
                cand[b].push(a);
            }
            for (a, list) in cand.iter_mut().enumerate() {
                let d = |x: &usize| self.tour.dist(a, *x);
                list.sort_by(|x, y| d(x).total_cmp(&d(y)).then(x.cmp(y)));
            }
            self.cand = cand;
            self.queue = (0..n).collect();
            self.queued = vec![true; n];
            self.phase = Phase::Improve;
        } else {
            self.phase = Phase::Finish;
        }
        let progress = if self.phase == Phase::Finish {
            1.0
        } else {
            0.0
        };
        self.frame(progress, "construction".into(), self.tour.path(0, false))
    }

    /// Take the next points from the queue and apply the first improving move from each.
    fn improve(&mut self) -> Frame {
        for _ in 0..CHECKS_PER_STEP {
            let Some(a) = self.queue.pop_front() else {
                break;
            };
            self.queued[a] = false;
            let moved = self
                .tour
                .two_opt(a, &self.cand[a])
                .or_else(|| self.tour.or_opt(a, &self.cand[a]));
            if let Some(touched) = moved {
                self.moves += 1;
                self.activate(touched);
            }
        }
        let n = self.queued.len();
        self.checked = self.checked.max(n - self.queue.len());
        if self.queue.is_empty() {
            let touched = self.tour.uncross();
            if touched.is_empty() {
                self.phase = Phase::Finish;
            }
            self.moves += touched.len() / 4;
            self.activate(touched);
        }
        let progress = self.checked as f32 / n as f32;
        let label = format!("improving: {} moves", self.moves);
        self.frame(progress, label, self.tour.path(0, true))
    }

    /// Queue each of `points` that is not queued already, in order.
    fn activate(&mut self, points: Vec<usize>) {
        for x in points {
            if !self.queued[x] {
                self.queued[x] = true;
                self.queue.push_back(x);
            }
        }
    }

    /// The finished line: the tour opened at its longest edge (ties to the lowest position) when
    /// it was improved as a cycle and is not to be closed.
    fn line(&self) -> Vec<[f64; 2]> {
        let t = &self.tour;
        let n = t.order.len();
        if self.s.closed || !self.s.optimize || n == 0 {
            return t.path(0, self.s.closed);
        }
        let edge = |i: usize| t.dist(t.order[i], t.order[(i + 1) % n]);
        let longest = (1..n).fold(0, |best, i| if edge(i) > edge(best) { i } else { best });
        t.path(longest + 1, false)
    }
}

impl StageRun for TspRun {
    fn step(&mut self) -> Result<Step, StageError> {
        Ok(match self.phase {
            Phase::Construct => Step::Frame(self.construct()),
            Phase::Improve => Step::Frame(self.improve()),
            Phase::Finish => {
                self.phase = Phase::Done;
                let mut paths = Paths::new();
                let line = self.line();
                if !line.is_empty() {
                    paths.push(&line);
                }
                Step::Done(Artifact::Paths(Arc::new(paths)))
            }
            Phase::Done => {
                return Err(StageError::Failed(
                    "the tsp run has already finished".into(),
                ))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::run_stage;
    use rand::{Rng, SeedableRng};
    use rand_pcg::Pcg64;
    use serde_json::json;

    /// `n` seeded random points in a 100 mm square.
    fn random_points(n: usize, seed: u64) -> Vec<[f64; 2]> {
        let mut rng = Pcg64::seed_from_u64(seed);
        (0..n)
            .map(|_| [rng.random_range(0.0..100.0), rng.random_range(0.0..100.0)])
            .collect()
    }

    fn run(points: Vec<[f64; 2]>, patch: Value) -> (Vec<Frame>, Arc<Paths>) {
        match run_stage(
            &TspRenderer,
            vec![Artifact::Points(Arc::new(Points(points)))],
            patch,
        ) {
            (frames, Artifact::Paths(p)) => (frames, p),
            (_, a) => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    fn sorted(mut v: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
        v.sort_by(|p, q| p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1])));
        v
    }

    /// The pairs of segments of `line` that share no vertex and cross.
    fn crossings(line: &[[f64; 2]]) -> usize {
        let segs: Vec<_> = line.windows(2).map(|w| (w[0], w[1])).collect();
        let mut n = 0;
        for i in 0..segs.len() {
            for j in i + 2..segs.len() {
                if crosses(segs[i].0, segs[i].1, segs[j].0, segs[j].1) {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn settings_schema() {
        let t = TspRenderer;
        assert_eq!(
            (t.name(), t.label(), t.version()),
            ("tsp", "Single line", 1)
        );
        assert_eq!(
            (t.requires(), t.provides()),
            (&[Kind::Points][..], Kind::Paths)
        );
        assert_eq!(t.defaults(), json!({"optimize": true, "closed": false}));
        let s = t.schema();
        for (key, help) in [
            (
                "optimize",
                "Shorten the line and remove its crossings; off leaves the nearest-neighbour line.",
            ),
            ("closed", "End the line where it started."),
        ] {
            assert_eq!(s["properties"][key]["description"], help, "{key}");
        }
    }

    #[test]
    fn one_path_through_every_point() {
        let pts = random_points(200, 7);
        let (frames, paths) = run(pts.clone(), json!({}));
        assert_eq!(paths.len(), 1);
        assert_eq!(paths.get(0).len(), 200);
        assert_eq!(sorted(paths.get(0).to_vec()), sorted(pts));
        assert_eq!(frames[0].label, "construction");
        assert!(frames[1..]
            .iter()
            .all(|f| f.label.starts_with("improving: ")));
    }

    #[test]
    fn no_two_segments_cross() {
        let (_, paths) = run(random_points(200, 7), json!({}));
        assert_eq!(crossings(paths.get(0)), 0);
        // The nearest-neighbour line this starts from does cross itself.
        let (_, nn) = run(random_points(200, 7), json!({"optimize": false}));
        assert!(crossings(nn.get(0)) > 0);
    }

    #[test]
    fn closed_ends_where_it_starts() {
        let (_, paths) = run(random_points(200, 7), json!({"closed": true}));
        let line = paths.get(0);
        assert_eq!(line.len(), 201);
        assert_eq!(line[0], line[200]);
        assert_eq!(crossings(line), 0);
    }

    #[test]
    fn optimize_shortens_the_line() {
        let (_, nn) = run(random_points(200, 7), json!({"optimize": false}));
        let (_, opt) = run(random_points(200, 7), json!({}));
        assert!(opt.total_length() < nn.total_length());
    }

    #[test]
    fn tsp_visits_duplicate_points() {
        let pts = vec![[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [2.0, 0.0]];
        for optimize in [true, false] {
            let (_, paths) = run(pts.clone(), json!({ "optimize": optimize }));
            assert_eq!(sorted(paths.get(0).to_vec()), sorted(pts.clone()));
        }
    }

    /// The tour's edges, each as `(low, high)`, sorted.
    fn edges(t: &Tour) -> Vec<(usize, usize)> {
        let n = t.order.len();
        let mut e: Vec<_> = (0..n)
            .map(|i| (t.order[i], t.order[(i + 1) % n]))
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        e.sort_unstable();
        e
    }

    fn length(t: &Tour) -> f64 {
        edges(t).iter().map(|&(a, b)| t.dist(a, b)).sum()
    }

    /// `old` without `cut` and with `add`, sorted.
    fn swap_edges(
        old: &[(usize, usize)],
        cut: &[(usize, usize)],
        add: &[(usize, usize)],
    ) -> Vec<(usize, usize)> {
        let key = |&(a, b): &(usize, usize)| (a.min(b), a.max(b));
        let mut e = old.to_vec();
        for c in cut {
            let i = e
                .iter()
                .position(|x| *x == key(c))
                .expect("a cut edge is in the tour");
            e.remove(i);
        }
        e.extend(add.iter().map(key));
        e.sort_unstable();
        e
    }

    #[test]
    fn moves_change_exactly_the_edges_they_claim() {
        // Every other point is a candidate, so small tours reach every kind of move, wrapping
        // around the array and reversing either side.
        let (mut two, mut or) = (0, 0);
        for seed in 0..40 {
            let n = 5 + seed as usize % 8;
            let pts = Arc::new(Points(random_points(n, seed)));
            let mut t = Tour::new(pts, (0..n).collect());
            let cand: Vec<Vec<usize>> = (0..n)
                .map(|a| {
                    let mut c: Vec<usize> = (0..n).filter(|&x| x != a).collect();
                    c.sort_by(|&x, &y| t.dist(a, x).total_cmp(&t.dist(a, y)));
                    c
                })
                .collect();
            loop {
                let mut moved = false;
                for (a, list) in cand.iter().enumerate() {
                    let (old, before) = (edges(&t), length(&t));
                    if let Some(m) = t.two_opt(a, list) {
                        let [a, b, c, e] = m[..] else { unreachable!() };
                        assert_eq!(
                            edges(&t),
                            swap_edges(&old, &[(a, b), (c, e)], &[(a, c), (b, e)])
                        );
                        two += 1;
                    } else if let Some(m) = t.or_opt(a, list) {
                        let [p, q, a, s, c, d] = m[..] else {
                            unreachable!()
                        };
                        let cut = [(p, a), (s, q), (c, d)];
                        let same = swap_edges(&old, &cut, &[(p, q), (c, a), (s, d)]);
                        let turned = swap_edges(&old, &cut, &[(p, q), (c, s), (a, d)]);
                        assert!(edges(&t) == same || edges(&t) == turned, "seed {seed}");
                        or += 1;
                    } else {
                        continue;
                    }
                    moved = true;
                    assert!(length(&t) < before, "seed {seed}");
                    for (i, &x) in t.order.iter().enumerate() {
                        assert_eq!(t.pos[x], i);
                    }
                }
                if !moved {
                    break;
                }
            }
        }
        assert!(two > 0 && or > 0, "{two} 2-opt moves, {or} Or-opt moves");
    }

    #[test]
    fn crossings_the_candidate_lists_miss_are_uncrossed() {
        // Without the uncrossing scan these keep a long edge across short ones, which neither
        // 2-opt nor Or-opt can reach through Delaunay neighbours: one in the closed line from the
        // 200 points, one even in the open line from these 300.
        let (_, closed) = run(random_points(200, 2), json!({"closed": true}));
        assert_eq!(crossings(closed.get(0)), 0);
        let (_, open) = run(random_points(300, 185), json!({}));
        assert_eq!(crossings(open.get(0)), 0);
    }

    #[test]
    fn few_points() {
        assert!(run(vec![], json!({})).1.is_empty());
        let (_, one) = run(vec![[3.0, 4.0]], json!({}));
        assert_eq!((one.len(), one.get(0)), (1, &[[3.0, 4.0]][..]));
        let (_, two) = run(vec![[3.0, 4.0], [5.0, 6.0]], json!({}));
        assert_eq!((two.len(), two.get(0).len()), (1, 2));
    }
}
