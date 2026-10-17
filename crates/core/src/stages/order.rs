//! The ordering stage: join the paths that meet, simplify them, drop short ones, then reorder and
//! reverse the rest to cut pen-up travel.
//!
//! Joining is [`join::join`], which cites Edmonds and Johnson's 1973 postman paper for the fewest
//! trails a graph needs and Hierholzer's 1873 paper for the trails.
//!
//! Simplification is the Ramer-Douglas-Peucker algorithm in [`raster::simplify`], which cites
//! Douglas and Peucker's 1973 paper.
//!
//! The order comes from the nearest-neighbour heuristic for the travelling salesman problem, which
//! Rosenkrantz, Stearns and Lewis analyse in "An analysis of several heuristics for the traveling
//! salesman problem" (SIAM Journal on Computing 6(3), 1977). Here it runs on paths, which have two
//! endpoints: the pen starts at the top-left corner of the drawing area and repeatedly moves to the
//! closest endpoint of a path not yet drawn, then draws that path to its other end. A grid index
//! over the endpoints finds the closest one without comparing against every path.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::join;
use crate::raster;
use crate::settings::{defaults_for, schema_for, validate};
use crate::spatial::{bounds, GridIndex};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::Paths;

/// Whether and how the ordering stage joins the paths that meet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Join {
    /// Each path stays as the style drew it.
    Off,
    /// The paths that meet are drawn as the fewest trails that draw each edge once.
    On,
    /// As on, and short edges between two trail ends are drawn twice, which joins their trails.
    Retrace,
}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the ordering stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OrderSettings {
    /// Join paths that meet into the fewest pen-down trails, drawing an edge two paths share once; retrace also draws some short edges twice, so the pen lifts less.
    pub join: Join,
    /// Reorder paths to cut pen-up travel.
    pub optimize: bool,
    /// Drop paths shorter than this. Dots have zero length, so any positive value removes them.
    #[schemars(range(min = 0.0, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub min_length_mm: f64,
    /// Remove vertices that lie within this distance of the simplified line; 0 keeps every vertex. An eighth of the pen width cannot be seen at the pen's own width.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01, "x-unit" = "mm"))]
    pub simplify_mm: f64,
}

impl Default for OrderSettings {
    fn default() -> Self {
        OrderSettings {
            join: Join::Off,
            optimize: true,
            min_length_mm: 0.0,
            simplify_mm: 0.0,
        }
    }
}

/// With `join` on, rebuild the paths as the fewest trails over the graph of their vertices; with
/// `retrace`, fewer still by drawing short edges twice (see [`join::join`]). Then simplify each
/// path of three or more vertices to within `simplify_mm`, keeping its end points;
/// at 0 the paths are left as they are. Drop the paths shorter than `min_length_mm`, then, with
/// `optimize` on, put the rest in nearest-neighbour order from the top-left corner of the drawing
/// area.
///
/// Each path is entered at its nearer endpoint, so one whose last vertex is the nearer is
/// reversed. Exact ties go to the endpoint listed first: every path's first vertex in input order,
/// then every path's last vertex. Without `optimize` the paths that remain keep their order and
/// direction.
///
/// About every twentieth of the paths a frame holds the paths ordered so far, and the last frame,
/// at progress 1, holds all of them.
pub struct OrderStage;

impl Stage for OrderStage {
    fn name(&self) -> &'static str {
        "order"
    }
    fn label(&self) -> &'static str {
        "Path order"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Paths, Kind::Canvas]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<OrderSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<OrderSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<OrderSettings>(self.name(), &self.schema(), settings)?;
        let mut paths = inputs.paths().clone();
        if s.join != Join::Off {
            paths = Arc::new(join::join(&paths, s.join == Join::Retrace));
        }
        if s.simplify_mm > 0.0 {
            let mut simplified = Paths::new();
            // Deserialized paths may be empty, and an empty path draws nothing.
            for p in paths.iter().filter(|p| !p.is_empty()) {
                simplified.push(&raster::simplify(p, s.simplify_mm));
            }
            paths = Arc::new(simplified);
        }
        let kept: Vec<usize> = (0..paths.len())
            .filter(|&i| {
                let p = paths.get(i);
                // Deserialized paths may be empty, and an empty path draws nothing.
                !p.is_empty() && Paths::path_length(p) >= s.min_length_mm
            })
            .collect();
        let n = kept.len();
        let mut ordered = Paths::new();
        let mut index = None;
        if n == 0 || !s.optimize {
            for &i in &kept {
                ordered.push(paths.get(i));
            }
        } else {
            // All starts, then all ends: endpoint j belongs to kept path j % n, its last when j >= n.
            let first = kept.iter().map(|&i| paths.get(i)[0]);
            let last = kept.iter().map(|&i| paths.get(i)[paths.get(i).len() - 1]);
            let ends: Vec<[f64; 2]> = first.chain(last).collect();
            let (lo, hi) = bounds(&ends);
            let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
            // About two endpoints per cell, and never more than 257 cells along an axis.
            let cell = (w * h / n as f64).sqrt().max(w.max(h) / 256.0).max(1e-9);
            index = Some(GridIndex::new(&ends, cell));
        }
        let [x, y, _, _] = inputs.canvas().drawing_rect();
        Ok(Box::new(OrderRun {
            paths,
            kept,
            index,
            alive: vec![true; 2 * n],
            pen: [x, y],
            every: (n / 20).max(1),
            ordered: Some(ordered),
            last_frame: false,
        }))
    }
}

/// Each step orders the next twentieth of the paths and reports them; the last step gives them.
struct OrderRun {
    paths: Arc<Paths>,
    /// The indices of the paths long enough to keep, in input order.
    kept: Vec<usize>,
    /// The endpoints of the kept paths, or `None` when the kept paths keep their order.
    index: Option<GridIndex>,
    /// Per endpoint, whether its path is still to be drawn.
    alive: Vec<bool>,
    /// Where the pen is.
    pen: [f64; 2],
    /// Paths ordered per step.
    every: usize,
    /// The paths in drawing order so far; `None` once the run has given them.
    ordered: Option<Paths>,
    /// Whether the frame at progress 1 has been given.
    last_frame: bool,
}

impl StageRun for OrderRun {
    fn step(&mut self) -> Result<Step, StageError> {
        let Some(ordered) = self.ordered.as_mut() else {
            return Err(StageError::Failed(
                "the order run has already finished".into(),
            ));
        };
        if self.last_frame {
            let ordered = self.ordered.take().expect("checked above");
            return Ok(Step::Done(Artifact::Paths(Arc::new(ordered))));
        }
        let n = self.kept.len();
        if let Some(index) = &self.index {
            for _ in 0..self.every.min(n - ordered.len()) {
                // With no endpoint at a finite distance, as with NaN ones, take the first left.
                let j = index
                    .nearest(self.pen, &self.alive)
                    .or_else(|| self.alive.iter().position(|&a| a))
                    .expect("a path is still to be drawn");
                let (i, reversed) = (j % n, j >= n);
                self.alive[i] = false;
                self.alive[i + n] = false;
                let path = self.paths.get(self.kept[i]);
                if reversed {
                    ordered.push_reversed(path);
                    self.pen = path[0];
                } else {
                    ordered.push(path);
                    self.pen = path[path.len() - 1];
                }
            }
        }
        let count = ordered.len();
        self.last_frame = count == n;
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress: if count == n {
                1.0
            } else {
                count as f32 / n as f32
            },
            label: format!("{count}/{n} paths"),
            points: None,
            paths: Some(ordered.clone()),
            pen: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::merge;
    use crate::types::{Canvas, Fit};
    use rand::{Rng, SeedableRng};
    use rand_pcg::Pcg64;
    use serde_json::json;
    use std::collections::HashMap;

    /// Run the order stage over `paths` on a 100 mm square page with a 10 mm margin, with `patch`
    /// over the defaults; returns the frames and the ordered paths.
    fn order(paths: Vec<Vec<[f64; 2]>>, patch: Value) -> (Vec<Frame>, Arc<Paths>) {
        let mut input = Paths::new();
        for p in &paths {
            input.push(p);
        }
        let canvas = Canvas::new(100.0, 100.0, 10.0, Fit::Fit).unwrap();
        let inputs = Inputs::new(HashMap::from([
            (Kind::Paths, Artifact::Paths(Arc::new(input))),
            (Kind::Canvas, Artifact::Canvas(Arc::new(canvas))),
        ]));
        let settings = merge(&OrderStage.defaults(), &patch);
        let ctx = RunContext::new(Arc::default());
        let mut run = OrderStage.start(inputs, &settings, ctx).unwrap();
        let mut frames = Vec::new();
        loop {
            match run.step().unwrap() {
                Step::Frame(f) => frames.push(f),
                Step::Done(Artifact::Paths(p)) => return (frames, p),
                Step::Done(a) => panic!("expected paths, got {}", a.kind().name()),
            }
        }
    }

    #[test]
    fn greedy_order_and_reversal() {
        let (frames, out) = order(
            vec![
                vec![[50.0, 50.0], [60.0, 50.0]],
                vec![[12.0, 10.0], [20.0, 10.0]],
                vec![[40.0, 10.0], [30.0, 10.0]],
            ],
            json!({}),
        );
        assert_eq!(out.get(0), &[[12.0, 10.0], [20.0, 10.0]]);
        assert_eq!(out.get(1), &[[30.0, 10.0], [40.0, 10.0]]);
        assert_eq!(out.get(2), &[[50.0, 50.0], [60.0, 50.0]]);
        assert_eq!(frames.last().unwrap().progress, 1.0);
        assert_eq!(frames.last().unwrap().paths.as_ref().unwrap().len(), 3);
    }
    #[test]
    fn min_length_drops_short_paths_but_keeps_dots_at_zero() {
        let paths = || vec![vec![[5.0, 5.0]], vec![[0.0, 0.0], [0.0, 0.5]]];
        assert_eq!(order(paths(), json!({"min_length_mm": 1.0})).1.len(), 0);
        assert_eq!(order(paths(), json!({})).1.len(), 2);
    }
    #[test]
    fn simplify_drops_vertices_within_the_tolerance_and_keeps_the_ends() {
        let line = vec![[10.0, 10.0], [15.0, 10.02], [20.0, 10.0], [25.0, 15.0]];
        let kept = [[10.0, 10.0], [20.0, 10.0], [25.0, 15.0]];
        assert_eq!(order(vec![line.clone()], json!({})).1.get(0), &line);
        let (_, out) = order(vec![line.clone()], json!({"simplify_mm": 0.05}));
        assert_eq!(out.get(0), &kept);
        // Two vertices and one pass through unchanged.
        let short = vec![vec![[12.0, 12.0], [13.0, 12.0]], vec![[30.0, 30.0]]];
        let (_, out) = order(short.clone(), json!({"simplify_mm": 1.0}));
        assert_eq!((out.get(0), out.get(1)), (&short[0][..], &short[1][..]));
    }
    #[test]
    fn simplify_runs_before_the_length_filter() {
        // A closed jitter 0.16 mm long that simplifies to a point.
        let jitter = || {
            vec![vec![
                [10.0; 2],
                [10.0, 10.04],
                [10.0; 2],
                [10.0, 10.04],
                [10.0; 2],
            ]]
        };
        let patch = |simplify: f64| json!({"min_length_mm": 0.1, "simplify_mm": simplify});
        assert_eq!(order(jitter(), patch(0.0)).1.len(), 1);
        assert_eq!(order(jitter(), patch(0.05)).1.len(), 0);
    }
    #[test]
    fn optimize_off_keeps_order() {
        let (_, out) = order(
            vec![
                vec![[90.0, 90.0], [91.0, 90.0]],
                vec![[10.0, 10.0], [11.0, 10.0]],
            ],
            json!({"optimize": false}),
        );
        assert_eq!(out.get(0), &[[90.0, 90.0], [91.0, 90.0]]);
    }
    #[test]
    fn ordering_never_increases_travel() {
        let mut rng = Pcg64::seed_from_u64(1);
        let paths: Vec<Vec<[f64; 2]>> = (0..50)
            .map(|_| {
                let p = [rng.random_range(10.0..90.0), rng.random_range(10.0..90.0)];
                vec![p, [p[0] + 1.0, p[1]]]
            })
            .collect();
        let before = {
            let mut p = Paths::new();
            for q in &paths {
                p.push(q);
            }
            p.travel_length([10.0, 10.0])
        };
        assert!(order(paths, json!({})).1.travel_length([10.0, 10.0]) <= before);
    }
    #[test]
    fn starts_at_the_drawing_rect_corner() {
        let (_, out) = order(vec![vec![[2.0, 2.0]], vec![[12.0, 12.0]]], json!({}));
        assert_eq!(out.get(0), &[[12.0, 12.0]]);
    }
    #[test]
    fn paths_with_non_finite_endpoints_are_kept() {
        let (_, out) = order(vec![vec![[f64::NAN, 0.0]], vec![[20.0, 20.0]]], json!({}));
        assert_eq!(out.len(), 2);
        assert_eq!(out.get(0), &[[20.0, 20.0]]);
        assert!(out.get(1)[0][0].is_nan());
    }
    #[test]
    fn empty_input() {
        assert_eq!(order(vec![], json!({})).1.len(), 0);
    }
    #[test]
    fn frames_every_twentieth() {
        let paths: Vec<_> = (0..100).map(|i| vec![[i as f64, 50.0]]).collect();
        let (frames, _) = order(paths, json!({}));
        assert_eq!(frames.len(), 20);
        assert_eq!(frames[0].paths.as_ref().unwrap().len(), 5);
    }
}
