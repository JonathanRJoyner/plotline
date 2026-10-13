//! The flow lines stage: evenly spaced lines that follow the image's edges, closer together where
//! it is darker.
//!
//! These are evenly spaced streamlines after B. Jobard and W. Lefer, "Creating evenly-spaced
//! streamlines of arbitrary density", *Proc. Visualization in Scientific Computing*, 1997,
//! pp. 43-55. The field is the gradient of the blurred tone map turned a quarter turn, so it runs
//! along the edges, plus a small drift at `angle_deg` that carries the lines where the image is
//! flat. The separation at `p` is `d(p) = max_spacing - (max_spacing - min_spacing) * ink(p)`.
//! Each line is traced both ways from its seed with second-order Runge-Kutta (midpoint) steps of
//! `min_spacing / 2`. It stops at the rect's edge, after 20000 steps each way, where the field
//! vanishes, or within `d / 2` of a sample of another line, or of its own line more than `d`
//! back along it, or within half a step of its own sample 3 or more back, so that an orbit shorter
//! than `d` is not drawn round and round. Seeds lie `d(p)` to either side of each sample `p` of the
//! lines kept, taken first in first out, and such a seed `q` needs every sample at least
//! `min(d(q), d(p))` away, so seeds spread towards lighter tone as well as darker. The first seed
//! is the rect's centre; when the seeds run out, a coarse grid at `max_spacing` is scanned for
//! points with every sample at least `d(q)` away. A line shorter than `2 * min_spacing` is
//! dropped, and its samples are taken back out of the bucket grid, so it blocks nothing.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{self, inside};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, ToneMap};

/// The weight of the drift at `angle_deg` against the gradient, in ink per mm.
const DRIFT: f64 = 0.05;
/// The most steps a line takes each way from its seed.
const MAX_STEPS: i64 = 20000;
/// The most lines traced per step.
const LINES_PER_FRAME: usize = 50;
/// A line stops within half a step of its own sample this many samples back, or more.
const RETRACE: i64 = 3;
/// How far inside a seed's separation, in mm, a sample may lie without blocking it.
const SLACK: f64 = 1e-9;
/// The end of a bucket's list of samples, and the line of a seed being checked.
const NONE: usize = usize::MAX;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the flow lines stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct FlowSettings {
    /// Line spacing over full ink.
    #[schemars(range(min = 0.3, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub min_spacing_mm: f64,
    /// Line spacing over white.
    #[schemars(range(min = 1.0, max = 30.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub max_spacing_mm: f64,
    /// Blur before the edge directions are measured; more gives longer, smoother lines.
    #[schemars(range(min = 0.0, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub smooth_mm: f64,
    /// Direction the lines take where the image is flat.
    #[schemars(range(min = 0.0, max = 180.0), extend("x-step" = 1.0, "x-unit" = "deg"))]
    pub angle_deg: f64,
}

impl Default for FlowSettings {
    fn default() -> Self {
        FlowSettings {
            min_spacing_mm: 0.8,
            max_spacing_mm: 6.0,
            smooth_mm: 2.0,
            angle_deg: 0.0,
        }
    }
}

/// Draw evenly spaced streamlines along the image's edges.
///
/// A maximum spacing below the minimum is raised to it, so the lines are evenly spaced at the
/// minimum. Each step traces up to 50 lines and reports a frame holding the lines kept so far.
pub struct FlowStage;

impl Stage for FlowStage {
    fn name(&self) -> &'static str {
        "flow"
    }
    fn label(&self) -> &'static str {
        "Flow lines"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<FlowSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<FlowSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<FlowSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(FlowRun::new(inputs.tone().clone(), &s)))
    }
}

/// A traced point: where it is, its line, and its place along the line, negative behind the seed.
struct Sample {
    p: [f64; 2],
    line: usize,
    seq: i64,
}

struct FlowRun {
    tone: Arc<ToneMap>,
    /// The blurred map's gradient along x and along y (down the page), in ink per mm, held as maps
    /// so they sample bilinearly.
    gx: ToneMap,
    gy: ToneMap,
    /// The unit drift at `angle_deg`.
    drift: [f64; 2],
    min: f64,
    max: f64,
    /// The tracing step, `min / 2`.
    step: f64,
    rect: [f64; 4],
    /// The bucket grid, with cells of `min`: per bucket the newest sample in it, per sample the
    /// next older one in its bucket.
    cols: usize,
    rows: usize,
    head: Vec<usize>,
    next: Vec<usize>,
    /// The samples of the lines kept, and of the line being traced.
    samples: Vec<Sample>,
    /// Lines traced so far, kept or not, which numbers the next.
    traced: usize,
    /// Whether the centre has been seeded.
    centred: bool,
    /// The next seed candidate: sample `cursor / 2`, on side `cursor % 2`.
    cursor: usize,
    /// The coarse grid's columns and rows, and its next point in row-major order.
    coarse: [usize; 2],
    scan: usize,
    /// The lines kept.
    paths: Paths,
    /// About how long the lines will be in all, for progress.
    expected: f64,
}

impl FlowRun {
    fn new(tone: Arc<ToneMap>, s: &FlowSettings) -> FlowRun {
        let (w, h) = (tone.width as usize, tone.height as usize);
        let b = math::blur(&tone.ink, w, h, s.smooth_mm / tone.mm_per_px);
        let at = |x: usize, y: usize| f64::from(b[y * w + x]);
        // Central differences; past an edge the edge value repeats, as in the blur.
        let span = 2.0 * tone.mm_per_px;
        let (mut gx, mut gy) = (Vec::with_capacity(w * h), Vec::with_capacity(w * h));
        for y in 0..h {
            for x in 0..w {
                gx.push(((at((x + 1).min(w - 1), y) - at(x.saturating_sub(1), y)) / span) as f32);
                gy.push(((at(x, (y + 1).min(h - 1)) - at(x, y.saturating_sub(1))) / span) as f32);
            }
        }
        let map = |ink| ToneMap {
            width: tone.width,
            height: tone.height,
            ink,
            mm_per_px: tone.mm_per_px,
            origin_mm: tone.origin_mm,
        };
        let rect = tone.rect_mm();
        let min = s.min_spacing_mm;
        let max = s.max_spacing_mm.max(min);
        let step = min / 2.0;
        let cols = (rect[2] / min) as usize + 1;
        let rows = (rect[3] / min) as usize + 1;
        let coarse = [2, 3].map(|i| ((rect[i] / max).ceil() as usize).max(1));
        let mut run = FlowRun {
            gx: map(gx),
            gy: map(gy),
            drift: lines::direction(s.angle_deg).0,
            min,
            max,
            step,
            rect,
            cols,
            rows,
            head: vec![NONE; cols * rows],
            next: Vec::new(),
            samples: Vec::new(),
            traced: 0,
            centred: false,
            cursor: 0,
            coarse,
            scan: 0,
            paths: Paths::new(),
            expected: 0.0,
            tone,
        };
        let px_area = run.tone.mm_per_px * run.tone.mm_per_px;
        run.expected = run
            .tone
            .ink
            .iter()
            .map(|&ink| px_area / run.separation(f64::from(ink)))
            .sum();
        run
    }

    /// The line separation over `ink`.
    fn separation(&self, ink: f64) -> f64 {
        self.max - (self.max - self.min) * ink.clamp(0.0, 1.0)
    }

    /// The line separation at `p`.
    fn spacing(&self, p: [f64; 2]) -> f64 {
        self.separation(f64::from(self.tone.sample(p)))
    }

    /// The unit field direction at `p`, or `None` where the field vanishes.
    fn direction(&self, p: [f64; 2]) -> Option<[f64; 2]> {
        let (gx, gy) = (f64::from(self.gx.sample(p)), f64::from(self.gy.sample(p)));
        let v = [-gy + DRIFT * self.drift[0], gx + DRIFT * self.drift[1]];
        let n = math::hypot(v[0], v[1]);
        (n > 0.0).then(|| [v[0] / n, v[1] / n])
    }

    /// One midpoint step from `p`, along the field when `sign` is 1 and against it when -1.
    fn advance(&self, p: [f64; 2], sign: i64) -> Option<[f64; 2]> {
        let h = sign as f64 * self.step;
        let a = self.direction(p)?;
        let b = self.direction([p[0] + 0.5 * h * a[0], p[1] + 0.5 * h * a[1]])?;
        Some([p[0] + h * b[0], p[1] + h * b[1]])
    }

    /// The bucket column and row of `p`, clamped into the grid.
    fn cell(&self, p: [f64; 2]) -> (usize, usize) {
        let at = |v: f64, lo: f64, n: usize| (((v - lo) / self.min) as usize).min(n - 1);
        (
            at(p[0], self.rect[0], self.cols),
            at(p[1], self.rect[1], self.rows),
        )
    }

    /// Whether a stored sample lies within `r` of `q`, other than one of `line` within `skip` of
    /// `seq` in sequence.
    fn crowded(&self, q: [f64; 2], r: f64, line: usize, seq: i64, skip: i64) -> bool {
        let (col, row) = self.cell(q);
        let reach = (r / self.min).ceil() as usize;
        for j in row.saturating_sub(reach)..=(row + reach).min(self.rows - 1) {
            for i in col.saturating_sub(reach)..=(col + reach).min(self.cols - 1) {
                let mut k = self.head[j * self.cols + i];
                while k != NONE {
                    let s = &self.samples[k];
                    let (dx, dy) = (s.p[0] - q[0], s.p[1] - q[1]);
                    if dx * dx + dy * dy < r * r && !(s.line == line && (s.seq - seq).abs() <= skip)
                    {
                        return true;
                    }
                    k = self.next[k];
                }
            }
        }
        false
    }

    /// Store `p` as sample `seq` of `line`.
    fn store(&mut self, p: [f64; 2], line: usize, seq: i64) {
        let (col, row) = self.cell(p);
        let slot = &mut self.head[row * self.cols + col];
        self.next.push(*slot);
        *slot = self.samples.len();
        self.samples.push(Sample { p, line, seq });
    }

    /// Whether `q` may seed a line: inside the rect, with no sample within its separation or
    /// within `r`, the separation at the sample it came from.
    fn free(&self, q: [f64; 2], r: f64) -> bool {
        // A candidate lies `r` from the sample it came from; rounding must not put that sample
        // within it.
        let reach = self.spacing(q).min(r) - SLACK;
        inside(self.rect, q) && !self.crowded(q, reach, NONE, 0, 0)
    }

    /// The next free seed: the centre first, then the candidates beside the samples in order,
    /// then the coarse grid, resuming where it left off.
    fn next_seed(&mut self) -> Option<[f64; 2]> {
        let [x, y, w, h] = self.rect;
        if !self.centred {
            self.centred = true;
            return Some([x + w / 2.0, y + h / 2.0]);
        }
        while self.cursor < 2 * self.samples.len() {
            let c = self.cursor;
            self.cursor += 1;
            let p = self.samples[c / 2].p;
            let Some(v) = self.direction(p) else {
                continue;
            };
            let r = self.spacing(p);
            let d = if c.is_multiple_of(2) { r } else { -r };
            let q = [p[0] - d * v[1], p[1] + d * v[0]];
            if self.free(q, r) {
                return Some(q);
            }
        }
        let [cols, rows] = self.coarse;
        while self.scan < cols * rows {
            let (i, j) = (self.scan % cols, self.scan / cols);
            self.scan += 1;
            let q = [
                x + (i as f64 + 0.5) * w / cols as f64,
                y + (j as f64 + 0.5) * h / rows as f64,
            ];
            if self.free(q, f64::INFINITY) {
                return Some(q);
            }
        }
        None
    }

    /// Trace one way from `p`, storing each sample and adding it to `out`.
    fn follow(&mut self, mut p: [f64; 2], sign: i64, line: usize, out: &mut Vec<[f64; 2]>) {
        for n in 1..=MAX_STEPS {
            let Some(q) = self.advance(p, sign) else {
                return;
            };
            if !inside(self.rect, q) {
                return;
            }
            // Samples of this line within an arc of `d` behind `q` never block it at `d / 2`, but
            // any 3 or more back block it at half a step, so a small orbit is not retraced.
            let d = self.spacing(q);
            let (seq, skip) = (sign * n, (d / self.step).ceil() as i64);
            if self.crowded(q, d / 2.0, line, seq, skip)
                || self.crowded(q, self.step / 2.0, line, seq, RETRACE - 1)
            {
                return;
            }
            self.store(q, line, seq);
            out.push(q);
            p = q;
        }
    }

    /// Trace a line both ways from `seed` and keep it if it is long enough; otherwise take its
    /// samples back out of the bucket grid.
    fn trace(&mut self, seed: [f64; 2]) {
        let line = self.traced;
        self.traced += 1;
        let first = self.samples.len();
        self.store(seed, line, 0);
        let mut ahead = vec![seed];
        self.follow(seed, 1, line, &mut ahead);
        let mut path = Vec::new();
        self.follow(seed, -1, line, &mut path);
        path.reverse();
        path.extend(ahead);
        if Paths::path_length(&path) >= 2.0 * self.min {
            self.paths.push(&path);
            return;
        }
        // Newest first, so each sample is the head of its bucket when it goes.
        while self.samples.len() > first {
            let k = self.samples.len() - 1;
            let (col, row) = self.cell(self.samples[k].p);
            self.head[row * self.cols + col] = self.next[k];
            self.samples.pop();
            self.next.pop();
        }
    }
}

impl StageRun for FlowRun {
    fn step(&mut self) -> Result<Step, StageError> {
        for _ in 0..LINES_PER_FRAME {
            let Some(seed) = self.next_seed() else {
                let paths = std::mem::take(&mut self.paths);
                return Ok(Step::Done(Artifact::Paths(Arc::new(paths))));
            };
            self.trace(seed);
        }
        let n = self.paths.len();
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress: (self.paths.total_length() / self.expected).min(1.0) as f32,
            label: format!("{n} lines"),
            points: None,
            paths: Some(self.paths.clone()),
            pen: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{assert_inside, bounds, run_tone_stage, tone_map};
    use serde_json::json;

    /// A noise-like ink: a fixed hash of the pixel's position.
    fn noise(x: u32, y: u32) -> f32 {
        ((x * 7919 + y * 104_729) % 101) as f32 / 100.0
    }

    #[test]
    fn settings_schema() {
        let s = FlowStage.schema();
        assert_eq!(bounds(&s, "min_spacing_mm"), (0.3, 10.0, 0.8));
        assert_eq!(bounds(&s, "max_spacing_mm"), (1.0, 30.0, 6.0));
        assert_eq!(bounds(&s, "smooth_mm"), (0.0, 10.0, 2.0));
        assert_eq!(bounds(&s, "angle_deg"), (0.0, 180.0, 0.0));
        assert_eq!(
            (FlowStage.name(), FlowStage.label()),
            ("flow", "Flow lines")
        );
        assert_eq!(FlowStage.version(), 1);
        assert_eq!(
            (FlowStage.requires(), FlowStage.provides()),
            (&[Kind::Tone][..], Kind::Paths)
        );
        let _: FlowSettings = serde_json::from_value(FlowStage.defaults()).unwrap();
    }

    #[test]
    fn flat_image_gives_parallel_lines_at_the_angle() {
        let flat = || tone_map(30, 20, |_, _| 0.5);
        let (_, paths) = run_tone_stage(&FlowStage, flat(), json!({}));
        assert!(paths.len() > 5, "{} lines", paths.len());
        for path in paths.iter() {
            assert!(path.len() >= 2);
            assert!(
                path.iter().all(|p| (p[1] - path[0][1]).abs() < 1e-6),
                "{path:?}"
            );
        }
        // Counter-clockwise on the page, whose y axis points down: x grows as y shrinks.
        let (_, paths) = run_tone_stage(&FlowStage, flat(), json!({"angle_deg": 45.0}));
        for s in paths.iter().flat_map(|p| p.windows(2)) {
            let (dx, dy) = (s[1][0] - s[0][0], s[1][1] - s[0][1]);
            assert!(dx * dy < 0.0 && (dx + dy).abs() < 1e-9, "{s:?}");
        }
    }

    #[test]
    fn spacing_follows_the_ink() {
        // Flat maps give straight lines exactly one spacing apart; a maximum below the minimum
        // spaces them at the minimum.
        for (ink, patch, gap) in [
            (0.0, json!({}), 6.0),
            (1.0, json!({}), 0.8),
            (
                0.0,
                json!({"min_spacing_mm": 3.0, "max_spacing_mm": 1.0}),
                3.0,
            ),
        ] {
            let (_, paths) = run_tone_stage(&FlowStage, tone_map(30, 20, |_, _| ink), patch);
            let mut ys: Vec<f64> = paths.iter().map(|p| p[0][1]).collect();
            ys.sort_by(f64::total_cmp);
            assert!(ys.len() > 2, "{ys:?}");
            for w in ys.windows(2) {
                assert!((w[1] - w[0] - gap).abs() < 1e-9, "ink {ink}: {ys:?}");
            }
        }
    }

    #[test]
    fn lines_keep_their_distance() {
        let min = 0.8;
        for smooth in [0.0, 2.0] {
            let (_, paths) = run_tone_stage(
                &FlowStage,
                tone_map(40, 30, noise),
                json!({ "smooth_mm": smooth }),
            );
            assert!(paths.len() > 10, "{} lines", paths.len());
            let tagged: Vec<(usize, [f64; 2])> = paths
                .iter()
                .enumerate()
                .flat_map(|(i, p)| p.iter().map(move |&v| (i, v)))
                .collect();
            for (a, &(i, p)) in tagged.iter().enumerate() {
                for &(j, q) in &tagged[a + 1..] {
                    let gap = math::hypot(p[0] - q[0], p[1] - q[1]);
                    assert!(
                        i == j || gap >= min / 2.0 - 1e-9,
                        "lines {i} and {j}: {p:?} {q:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn darker_areas_get_closer_lines() {
        // Ink on the left half, which ends at x = 10 + 20 * 2 mm.
        let tone = tone_map(40, 30, |x, _| if x < 20 { 1.0 } else { 0.0 });
        let (_, paths) = run_tone_stage(&FlowStage, tone, json!({}));
        let (mut left, mut right) = (0.0, 0.0);
        for s in paths.iter().flat_map(|p| p.windows(2)) {
            let len = math::hypot(s[1][0] - s[0][0], s[1][1] - s[0][1]);
            if s[0][0] + s[1][0] < 2.0 * 50.0 {
                left += len;
            } else {
                right += len;
            }
        }
        assert!(left > 2.0 * right, "left {left} right {right}");
    }

    #[test]
    fn light_ward_spacing_follows_d() {
        // Full ink at the top fading to white at the bottom, 200 mm tall, centred at y = 110: the
        // lines run straight across, and each gap is the separation where it lies, on the dark side
        // of the centre and on the light side.
        let tone = tone_map(20, 100, |_, y| 1.0 - (y as f32 + 0.5) / 100.0);
        let d = |y: f64| 6.0 - 5.2 * f64::from(tone.sample([30.0, y]));
        let (_, paths) = run_tone_stage(&FlowStage, tone.clone(), json!({}));
        let mut ys: Vec<f64> = paths.iter().map(|p| p[0][1]).collect();
        ys.sort_by(f64::total_cmp);
        for side in [|y: f64| y < 110.0, |y: f64| y > 110.0] {
            let n = ys.iter().filter(|&&y| side(y)).count();
            assert!(n > 5, "{ys:?}");
        }
        for w in ys.windows(2) {
            let (gap, want) = (w[1] - w[0], d((w[0] + w[1]) / 2.0));
            assert!(
                (gap / want - 1.0).abs() < 0.05,
                "gap {gap} below {} wants {want}: {ys:?}",
                w[0]
            );
        }
    }

    #[test]
    fn no_line_retraces_itself() {
        // Half a step at the default spacing. The lattice noise, on a 4 mm lattice, makes orbits
        // of a few mm; the per-pixel noise without smoothing makes orbits a few steps long.
        let near = 0.8 / 4.0;
        let lattice = |x: u32, y: u32| {
            let (fx, fy) = ((x as f32 + 0.5) / 2.0, (y as f32 + 0.5) / 2.0);
            let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
            let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
            let top = noise(x0, y0) * (1.0 - tx) + noise(x0 + 1, y0) * tx;
            let bottom = noise(x0, y0 + 1) * (1.0 - tx) + noise(x0 + 1, y0 + 1) * tx;
            top * (1.0 - ty) + bottom * ty
        };
        for (tone, patch) in [
            (tone_map(40, 30, lattice), json!({})),
            (tone_map(40, 30, noise), json!({"smooth_mm": 0.0})),
        ] {
            let (_, paths) = run_tone_stage(&FlowStage, tone, patch.clone());
            assert!(paths.len() > 10, "{} lines", paths.len());
            for (k, path) in paths.iter().enumerate() {
                for i in 3..path.len() {
                    for j in 0..i - 2 {
                        let (p, q) = (path[i], path[j]);
                        assert!(
                            math::hypot(p[0] - q[0], p[1] - q[1]) >= near - 1e-9,
                            "{patch}, line {k}: vertex {i} meets vertex {j}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        let tone = tone_map(30, 20, noise);
        let rect = tone.rect_mm();
        let (_, paths) = run_tone_stage(&FlowStage, tone, json!({"min_spacing_mm": 0.3}));
        assert!(!paths.is_empty());
        assert_inside(&paths, rect);
    }

    #[test]
    fn frames_show_the_lines_appearing() {
        // Flat black, 80 mm tall at 0.8 mm: about 100 lines, so at least two frames.
        let (frames, paths) = run_tone_stage(&FlowStage, tone_map(40, 40, |_, _| 1.0), json!({}));
        let counts: Vec<usize> = frames
            .iter()
            .map(|f| f.paths.as_ref().unwrap().len())
            .collect();
        assert!(frames.len() >= 2, "{counts:?}");
        let mut last = 0;
        for n in counts.iter().copied().chain([paths.len()]) {
            assert!(
                last <= n && n <= last + 50,
                "{counts:?} then {}",
                paths.len()
            );
            last = n;
        }
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"min_spacing_mm": 0.2}),
            json!({"max_spacing_mm": 31}),
            json!({"smooth_mm": -1}),
            json!({"angle_deg": 181}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&FlowStage.defaults(), &patch);
            let tone = crate::stage::Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let inputs = Inputs::new([(Kind::Tone, tone)].into_iter().collect());
            let ctx = RunContext::new(Arc::default());
            let err = FlowStage.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
}
