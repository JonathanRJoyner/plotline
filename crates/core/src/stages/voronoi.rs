//! The weighted Voronoi placer: points spread so that their density follows the ink.
//!
//! This is weighted Voronoi stippling, A. Secord, "Weighted Voronoi stippling", *Proc. NPAR*, 2002,
//! pp. 37-43. It starts from a random scatter drawn from the density and relaxes it with Lloyd's
//! method, S. P. Lloyd, "Least squares quantization in PCM", *IEEE Trans. Information Theory*
//! 28(2), 1982: each pass moves every point to the density-weighted centroid of the pixels that lie
//! nearer to it than to any other point. The cells are found per pixel, by nearest-point queries
//! through [`GridIndex`], rather than as exact polygons.
//!
//! When the drawing is split between pens, each lane draws its scatter from its own random stream
//! ([`RunContext::rng`] cites why), so the pens' points fall independently of each other. `count`
//! is a budget per pen: a lane places `round(count * end) - round(count * start)` points for its
//! part `[start, end]` of the budgets ([`RunContext::budget`]). Each pen places `count` of its
//! own, except by colour, where the pens share `count` per pen by their ink, so a pen whose colour
//! makes up a tenth of the ink places about a tenth of the points.

use std::sync::Arc;

use rand::Rng;
use rand_pcg::Pcg64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::spatial::GridIndex;
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Points, ToneMap};

/// The most pixels one relaxation step assigns, so that Stop stays responsive on a large map. A
/// step ends at a row boundary, so it holds whole rows.
const PIXELS_PER_STEP: usize = 200_000;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the weighted Voronoi placer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct VoronoiSettings {
    /// Points to place per pen; by colour the pens share them by their ink; more points give finer detail and a longer plot.
    #[schemars(range(min = 100, max = 50000), extend("x-density" = "direct"))]
    pub count: u32,
    /// Relaxation passes; more passes spread the points more evenly.
    #[schemars(range(min = 0, max = 200))]
    pub iterations: u32,
    /// Density follows ink to this power; the styles that join their points default to 2, since a line darkens with the square root of the points' density, and dots to 1.
    #[schemars(range(min = 0.5, max = 3.0), extend("x-step" = 0.1))]
    pub exponent: f64,
    /// Starting scatter.
    #[schemars(range(min = 0, max = 9999))]
    pub seed: u32,
}

impl Default for VoronoiSettings {
    fn default() -> Self {
        VoronoiSettings {
            count: 5000,
            iterations: 30,
            exponent: 1.0,
            seed: 1,
        }
    }
}

/// The density of each tone pixel, `ink ^ exponent`, row-major. A pixel without a positive density
/// gets 0.
fn densities(tone: &ToneMap, exponent: f64) -> Vec<f32> {
    let exponent = exponent as f32;
    // `max` turns the NaN of a negative ink into 0.
    let density = |&ink: &f32| math::powf32(ink, exponent).max(0.0);
    tone.ink.iter().map(density).collect()
}

/// Draw `n` points from the density `ink ^ exponent` by inverse-CDF sampling over the pixels in
/// row-major order, each jittered uniformly inside its pixel, in mm. A map with no density gives
/// no points.
pub(crate) fn sample_by_density(
    tone: &ToneMap,
    exponent: f64,
    n: usize,
    rng: &mut Pcg64,
) -> Vec<[f64; 2]> {
    let mut total = 0.0;
    let cdf: Vec<f64> = densities(tone, exponent)
        .into_iter()
        .map(|d| {
            total += f64::from(d);
            total
        })
        .collect();
    if total <= 0.0 {
        return Vec::new();
    }
    // The last pixel with density, which a draw that rounds up to `total` must not pass.
    let last = cdf.partition_point(|&c| c < total);
    let width = tone.width as usize;
    (0..n)
        .map(|_| {
            let u = rng.random::<f64>() * total;
            let i = cdf.partition_point(|&c| c <= u).min(last);
            let (col, row) = ((i % width) as f64, (i / width) as f64);
            tone.to_mm([col + rng.random::<f64>(), row + rng.random::<f64>()])
        })
        .collect()
}

/// Place points over the tone map so that their density follows `ink ^ exponent`, then spread them
/// evenly with Lloyd relaxation.
///
/// The run first reports the scatter, then one frame per step of each pass, a step covering up to
/// `PIXELS_PER_STEP` pixels. After the last pass, points that won no inked pixel in it are
/// dropped, so no stray dots sit in white areas. With no passes the scatter is the answer, and
/// nothing is dropped.
pub struct VoronoiPlacer;

impl Stage for VoronoiPlacer {
    fn name(&self) -> &'static str {
        "voronoi"
    }
    fn label(&self) -> &'static str {
        "Weighted Voronoi"
    }
    fn version(&self) -> u32 {
        3
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Points
    }
    fn schema(&self) -> Value {
        schema_for::<VoronoiSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<VoronoiSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<VoronoiSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let mut rng = ctx.rng(u64::from(s.seed));
        // Each lane places its part of the budgets; alone, or with one budget per pen, `count`.
        let [start, end] = ctx.budget();
        let share = |at: f64| (f64::from(s.count) * at).round();
        let count = (share(end) - share(start)) as usize;
        let points = sample_by_density(&tone, s.exponent, count, &mut rng);
        let [_, _, w_mm, h_mm] = tone.rect_mm();
        // About one point per cell, and never finer than a pixel.
        let cell = (w_mm * h_mm / count.max(1) as f64)
            .sqrt()
            .max(tone.mm_per_px);
        let n = points.len();
        Ok(Box::new(VoronoiRun {
            density: densities(&tone, s.exponent),
            rows_per_step: (PIXELS_PER_STEP / tone.width as usize).max(1),
            index: GridIndex::new(&points, cell),
            acc: vec![[0.0; 2]; n],
            mass: vec![0.0; n],
            alive: vec![true; n],
            tone,
            points,
            cell,
            iterations: s.iterations as usize,
            iteration: 0,
            next_row: 0,
            phase: Phase::Scatter,
        }))
    }
}

/// Where a run is.
#[derive(Clone, Copy)]
enum Phase {
    /// The first step reports the scatter.
    Scatter,
    /// Relaxing: the next step assigns the rows from `next_row` of pass `iteration`.
    Relax,
    /// The next step gives the finished points.
    Finish,
}

struct VoronoiRun {
    tone: Arc<ToneMap>,
    /// The density of each pixel, row-major.
    density: Vec<f32>,
    /// The current points.
    points: Vec<[f64; 2]>,
    /// The side of an index cell in mm.
    cell: f64,
    /// The points as they were when the current pass began.
    index: GridIndex,
    /// Per point, the density-weighted sum of the pixel centres it has won in the current pass.
    acc: Vec<[f64; 2]>,
    /// Per point, the density of the pixels it has won in the current pass; after the last pass,
    /// what is left of it says which points won nothing.
    mass: Vec<f64>,
    /// One `true` per point, because every point takes part.
    alive: Vec<bool>,
    rows_per_step: usize,
    iterations: usize,
    /// The pass in progress, counting from 0.
    iteration: usize,
    /// The first row of the pass not assigned yet.
    next_row: usize,
    phase: Phase,
}

impl VoronoiRun {
    /// A frame holding the current points.
    fn frame(&self, progress: f32, label: String) -> Frame {
        Frame {
            stage: String::new(),
            progress,
            label,
            points: Some(Points(self.points.clone(), None)),
            paths: None,
            pen: 0,
        }
    }

    /// Assign the next band of rows to their nearest points; once a pass has seen every row, move
    /// each point that won some density to the centroid of what it won.
    fn relax(&mut self) -> Frame {
        let (w, h) = (self.tone.width as usize, self.tone.height as usize);
        let end = (self.next_row + self.rows_per_step).min(h);
        for row in self.next_row..end {
            for col in 0..w {
                let d = f64::from(self.density[row * w + col]);
                if d > 0.0 {
                    let centre = self.tone.to_mm([col as f64 + 0.5, row as f64 + 0.5]);
                    if let Some(i) = self.index.nearest(centre, &self.alive) {
                        self.acc[i][0] += d * centre[0];
                        self.acc[i][1] += d * centre[1];
                        self.mass[i] += d;
                    }
                }
            }
        }
        let label = format!("relaxation {}/{}", self.iteration + 1, self.iterations);
        let done = self.iteration * h + end;
        let progress = (done as f64 / (self.iterations * h) as f64) as f32;
        self.next_row = end;
        if end == h {
            for ((p, a), &m) in self.points.iter_mut().zip(&self.acc).zip(&self.mass) {
                if m > 0.0 {
                    *p = [a[0] / m, a[1] / m];
                }
            }
            self.iteration += 1;
            self.next_row = 0;
            if self.iteration == self.iterations {
                self.phase = Phase::Finish;
            } else {
                self.index = GridIndex::new(&self.points, self.cell);
                self.acc.fill([0.0; 2]);
                self.mass.fill(0.0);
            }
        }
        self.frame(progress, label)
    }

    /// The points, without those that won no density in the last pass.
    fn finish(&mut self) -> Points {
        let points = std::mem::take(&mut self.points);
        if self.iterations == 0 {
            return Points(points, None);
        }
        let kept = points.into_iter().zip(&self.mass);
        Points(
            kept.filter(|&(_, &m)| m > 0.0).map(|(p, _)| p).collect(),
            None,
        )
    }
}

impl StageRun for VoronoiRun {
    fn step(&mut self) -> Result<Step, StageError> {
        Ok(match self.phase {
            Phase::Scatter => {
                // Only a map without ink draws no points, because `count` is at least 100, or a
                // lane by colour whose share of the points, which follows its ink, rounds to none.
                let no_ink = self.points.is_empty();
                let finished = no_ink || self.iterations == 0;
                self.phase = if finished {
                    Phase::Finish
                } else {
                    Phase::Relax
                };
                let label = if no_ink { "no ink" } else { "scatter" };
                Step::Frame(self.frame(if finished { 1.0 } else { 0.0 }, label.into()))
            }
            Phase::Relax => Step::Frame(self.relax()),
            Phase::Finish => Step::Done(Artifact::Points(Arc::new(self.finish()))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::merge;
    use crate::stage::Lane;
    use crate::test_util::{bounds, run_stage, tone_map};
    use serde_json::json;
    use std::collections::HashMap;

    fn run_voronoi(tone: ToneMap, patch: Value) -> (Vec<Frame>, Arc<Points>) {
        match run_stage(&VoronoiPlacer, vec![Artifact::Tone(Arc::new(tone))], patch) {
            (frames, Artifact::Points(p)) => (frames, p),
            (_, a) => panic!("expected points, got {}", a.kind().name()),
        }
    }

    #[test]
    fn settings_schema() {
        let v = VoronoiPlacer;
        assert_eq!((v.name(), v.label()), ("voronoi", "Weighted Voronoi"));
        assert_eq!(
            (v.requires(), v.provides()),
            (&[Kind::Tone][..], Kind::Points)
        );
        let s = v.schema();
        assert_eq!(bounds(&s, "count"), (100.0, 50000.0, 5000.0));
        assert_eq!(bounds(&s, "iterations"), (0.0, 200.0, 30.0));
        assert_eq!(bounds(&s, "exponent"), (0.5, 3.0, 1.0));
        assert_eq!(bounds(&s, "seed"), (0.0, 9999.0, 1.0));
        for (key, help) in [
            ("count", "Points to place per pen; by colour the pens share them by their ink; more points give finer detail and a longer plot."),
            ("iterations", "Relaxation passes; more passes spread the points more evenly."),
            ("exponent", "Density follows ink to this power; the styles that join their points default to 2, since a line darkens with the square root of the points' density, and dots to 1."),
            ("seed", "Starting scatter."),
        ] {
            assert_eq!(s["properties"][key]["description"], help, "{key}");
        }
    }

    #[test]
    fn points_are_denser_where_the_ink_is() {
        // 40 x 20 px at 2 mm per px, corner at (10, 10): the dark half ends at x = 50 mm.
        let map = tone_map(40, 20, |x, _| if x < 20 { 1.0 } else { 0.1 });
        let (_, pts) = run_voronoi(map, json!({"count": 400, "exponent": 1.0}));
        let dark = pts.0.iter().filter(|p| p[0] < 50.0).count();
        let light = pts.0.len() - dark;
        assert!(dark > 3 * light, "{dark} dark points, {light} light");
    }

    #[test]
    fn no_point_sits_on_white() {
        let map = tone_map(20, 20, |x, _| if x < 10 { 1.0 } else { 0.0 });
        let (_, pts) = run_voronoi(map, json!({"count": 300}));
        assert!(!pts.0.is_empty());
        for p in &pts.0 {
            assert!(p[0] <= 10.0 * 2.0 + 10.0 + 1e-9, "{p:?} is over white");
        }
    }

    #[test]
    fn white_gives_no_points() {
        let (frames, pts) = run_voronoi(tone_map(10, 10, |_, _| 0.0), json!({}));
        assert!(pts.0.is_empty());
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].label, "no ink");
        assert_eq!(frames.last().unwrap().progress, 1.0);
    }

    #[test]
    fn same_seed_same_points_other_seed_other_points() {
        let map = || tone_map(16, 16, |x, y| (x + y) as f32 / 30.0);
        let run = |seed: u32, iterations: u32| {
            let patch = json!({"count": 100, "iterations": iterations, "seed": seed});
            run_voronoi(map(), patch).1
        };
        assert_eq!(run(1, 3), run(1, 3));
        assert_ne!(run(1, 3), run(2, 3));
        // With no relaxation the scatter is the answer: every point stays, in one frame.
        let (frames, pts) = run_voronoi(map(), json!({"count": 100, "iterations": 0}));
        assert_eq!(pts.0.len(), 100);
        assert_eq!((frames.len(), frames[0].label.as_str()), (1, "scatter"));
        assert_eq!(frames[0].progress, 1.0);
    }

    #[test]
    fn relaxation_collapses_onto_a_lone_inked_pixel() {
        // Every scattered point starts inside pixel (3, 3). One pass gives the whole pixel to one
        // point, which moves to its centre, (10 + 3.5 * 2, 10 + 3.5 * 2) mm, and the rest are dropped.
        let map = tone_map(8, 8, |x, y| if (x, y) == (3, 3) { 1.0 } else { 0.0 });
        let (_, pts) = run_voronoi(map, json!({"count": 100, "iterations": 1}));
        assert_eq!(pts.0.len(), 1, "{:?}", pts.0);
        assert!((pts.0[0][0] - 17.0).abs() < 1e-9 && (pts.0[0][1] - 17.0).abs() < 1e-9);
    }

    #[test]
    fn relaxation_spreads_the_points() {
        // The distance between the two closest points.
        let tightest = |pts: &Points| {
            let mut best = f64::INFINITY;
            for (i, a) in pts.0.iter().enumerate() {
                for b in &pts.0[i + 1..] {
                    best = best.min(math::hypot(a[0] - b[0], a[1] - b[1]));
                }
            }
            best
        };
        let run = |iterations: u32| {
            let map = tone_map(40, 40, |_, _| 1.0);
            let patch = json!({"count": 200, "iterations": iterations, "seed": 1});
            run_voronoi(map, patch).1
        };
        let (scattered, relaxed) = (tightest(&run(0)), tightest(&run(30)));
        assert!(
            relaxed > scattered,
            "{relaxed} mm after 30 passes, {scattered} mm before"
        );
    }

    #[test]
    fn an_iteration_on_a_large_map_spans_several_steps() {
        let map = tone_map(1000, 1000, |_, _| 1.0);
        let (frames, _) = run_voronoi(map, json!({"count": 100, "iterations": 2}));
        assert!(frames.len() >= 10, "{} frames", frames.len());
        assert_eq!(frames.last().unwrap().progress, 1.0);
    }

    /// The points the placer gives lane 0 of 2 spending `budget` of the budgets, on a flat map.
    fn points_with_budget(budget: [f64; 2], patch: Value) -> usize {
        let settings = merge(&VoronoiPlacer.defaults(), &patch);
        let map = Artifact::Tone(Arc::new(tone_map(20, 20, |_, _| 0.5)));
        let inputs = Inputs::new(HashMap::from([(Kind::Tone, map)]));
        let lane = Lane { index: 0, count: 2 };
        let ctx = RunContext::new(Arc::default())
            .in_lane(lane)
            .with_budget(budget);
        let mut run = VoronoiPlacer.start(inputs, &settings, ctx).unwrap();
        loop {
            if let Step::Done(Artifact::Points(p)) = run.step().unwrap() {
                return p.0.len();
            }
        }
    }

    #[test]
    fn a_lane_places_its_part_of_the_budgets() {
        // Two pens by colour spend two budgets: one with a quarter of the ink spends half a
        // budget, the other one and a half.
        let patch = json!({"count": 400, "iterations": 0});
        assert_eq!(points_with_budget([0.0, 0.5], patch.clone()), 200);
        assert_eq!(points_with_budget([0.5, 2.0], patch.clone()), 600);
        // Parts that meet place their budgets together, however their ends round.
        let thirds = json!({"count": 100, "iterations": 0});
        let cuts = [0.0, 1.0 / 3.0, 7.0 / 3.0, 3.0];
        let parts: usize = (cuts.windows(2))
            .map(|w| points_with_budget([w[0], w[1]], thirds.clone()))
            .sum();
        assert_eq!(parts, 300);
        // A lane with no part places none.
        assert_eq!(points_with_budget([0.25, 0.25], patch), 0);
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        let inputs = || {
            let map = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            Inputs::new(HashMap::from([(Kind::Tone, map)]))
        };
        for patch in [
            json!({"count": 50}),
            json!({"exponent": 0.1}),
            json!({"nope": 1}),
        ] {
            let settings = merge(&VoronoiPlacer.defaults(), &patch);
            let ctx = RunContext::new(Arc::default());
            let err = VoronoiPlacer.start(inputs(), &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
}
