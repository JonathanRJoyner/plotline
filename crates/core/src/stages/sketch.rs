//! The sketch stage: straight strokes chained from dark to dark, like a quick pencil sketch.
//!
//! The stage keeps a residual copy of the tone map and draws greedily. From the pen it tries a
//! number of straight segments of one length at random angles, scores each by the mean residual
//! ink along it, draws the best and lowers the residual under it, so the next segment seeks ink
//! that is still there. A stroke ends when no segment finds enough ink, and the pen jumps to the
//! darkest residual pixel to start the next. The greedy residual-subtraction loop also drives
//! string-art methods: M. Birsak, F. Rist, P. Wonka and P. Musialski, "String art: towards
//! computational fabrication of string images", *Computer Graphics Forum* 37(2), 2018,
//! pp. 263-274, doi:10.1111/cgf.13359. That paper picks nails and threads for a physical string
//! image; this stage borrows only the loop, and the segments, scores and restarts are its own.

use std::f64::consts::TAU;
use std::sync::Arc;

use rand::Rng;
use rand_pcg::Pcg64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, sample_line};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, ToneMap};

/// The side of a block of pixels whose darkest ink is kept, in pixels.
const BLOCK: usize = 16;

/// The most ink samples one step of the run reads, so a step stays short at the widest settings.
const SAMPLE_BUDGET: usize = 2_000_000;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the sketch stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SketchSettings {
    /// Length of each straight segment.
    #[schemars(range(min = 1.0, max = 50.0), extend("x-step" = 0.5, "x-unit" = "mm"))]
    pub length_mm: f64,
    /// Directions tried per segment; more find darker paths but run slower.
    #[schemars(range(min = 4, max = 256))]
    pub candidates: u32,
    /// Ink removed under each drawn segment; less gives more, denser strokes.
    #[schemars(range(min = 0.05, max = 1.0), extend("x-step" = 0.05))]
    pub lighten: f64,
    /// Ink left under which no stroke is drawn.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub stop: f64,
    /// Most segments to draw.
    #[schemars(range(min = 100, max = 50000))]
    pub max_lines: u32,
    /// Directions tried.
    #[schemars(range(min = 0, max = 9999))]
    pub seed: u32,
}

impl Default for SketchSettings {
    fn default() -> Self {
        SketchSettings {
            length_mm: 6.0,
            candidates: 48,
            lighten: 0.3,
            stop: 0.2,
            max_lines: 5000,
            seed: 1,
        }
    }
}

/// The ink still left to draw over: a copy of the tone map that segments lower, and the darkest
/// ink of each block of [`BLOCK`] by [`BLOCK`] pixels, so that the darkest pixel is found without
/// scanning the whole map.
struct Residual {
    tone: Arc<ToneMap>,
    /// The ink left, row-major.
    ink: Vec<f32>,
    /// Per pixel, the number of the segment that last lowered it.
    stamp: Vec<u32>,
    /// Blocks across the map.
    across: usize,
    /// The darkest ink of each block, row-major.
    peak: Vec<f32>,
}

impl Residual {
    fn new(tone: Arc<ToneMap>) -> Residual {
        let (w, h) = (tone.width as usize, tone.height as usize);
        let (across, down) = (w.div_ceil(BLOCK), h.div_ceil(BLOCK));
        let mut r = Residual {
            ink: tone.ink.clone(),
            stamp: vec![0; w * h],
            across,
            peak: vec![0.0; across * down],
            tone,
        };
        for b in 0..across * down {
            r.refresh(b);
        }
        r
    }

    /// The index of the pixel `p` (page mm) falls in; points off the map count as the edge pixel.
    fn pixel(&self, p: [f64; 2]) -> usize {
        let (w, h) = (self.tone.width as usize, self.tone.height as usize);
        let [x, y] = self.tone.to_px(p);
        let col = x.floor().clamp(0.0, (w - 1) as f64) as usize;
        let row = y.floor().clamp(0.0, (h - 1) as f64) as usize;
        row * w + col
    }

    /// The pixel `i` as the centre of its square, in page mm.
    fn centre(&self, i: usize) -> [f64; 2] {
        let w = self.tone.width as usize;
        self.tone
            .to_mm([(i % w) as f64 + 0.5, (i / w) as f64 + 0.5])
    }

    /// The index of the block pixel `i` lies in.
    fn block_of(&self, i: usize) -> usize {
        let w = self.tone.width as usize;
        (i / w / BLOCK) * self.across + (i % w) / BLOCK
    }

    /// The pixel columns and rows of block `b`, as `(first column, one past the last, first row,
    /// one past the last)`.
    fn block_span(&self, b: usize) -> (usize, usize, usize, usize) {
        let (w, h) = (self.tone.width as usize, self.tone.height as usize);
        let (c0, r0) = ((b % self.across) * BLOCK, (b / self.across) * BLOCK);
        (c0, (c0 + BLOCK).min(w), r0, (r0 + BLOCK).min(h))
    }

    /// Recompute the darkest ink of block `b`.
    fn refresh(&mut self, b: usize) {
        let w = self.tone.width as usize;
        let (c0, c1, r0, r1) = self.block_span(b);
        let mut peak = f32::NEG_INFINITY;
        for row in r0..r1 {
            for &v in &self.ink[row * w + c0..row * w + c1] {
                peak = peak.max(v);
            }
        }
        self.peak[b] = peak;
    }

    /// The darkest pixel and its ink. Ties go to the first block in row-major order, then to the
    /// first pixel in it.
    fn darkest(&self) -> (usize, f32) {
        let mut block = 0;
        for (b, &p) in self.peak.iter().enumerate() {
            if p > self.peak[block] {
                block = b;
            }
        }
        let w = self.tone.width as usize;
        let (c0, c1, r0, r1) = self.block_span(block);
        let mut best = (r0 * w + c0, f32::NEG_INFINITY);
        for row in r0..r1 {
            for col in c0..c1 {
                let i = row * w + col;
                if self.ink[i] > best.1 {
                    best = (i, self.ink[i]);
                }
            }
        }
        best
    }

    /// The mean ink at the pixels nearest to points half a pixel apart along `a` to `b`.
    fn mean_along(&self, a: [f64; 2], b: [f64; 2]) -> f64 {
        let (mut sum, mut n) = (0.0, 0u32);
        for p in sample_line(a, b, self.tone.mm_per_px / 2.0) {
            sum += f64::from(self.ink[self.pixel(p)]);
            n += 1;
        }
        sum / f64::from(n)
    }

    /// Lower the ink by `by`, to 0 at the least, at every pixel that a sample half a pixel apart
    /// along `a` to `b` falls in. `serial` numbers the segment, from 1, and each pixel is lowered
    /// once per segment however many samples fall in it.
    fn lighten(&mut self, a: [f64; 2], b: [f64; 2], by: f32, serial: u32) {
        let mut touched = Vec::new();
        for p in sample_line(a, b, self.tone.mm_per_px / 2.0) {
            let i = self.pixel(p);
            if self.stamp[i] != serial {
                self.stamp[i] = serial;
                self.ink[i] = (self.ink[i] - by).max(0.0);
                touched.push(self.block_of(i));
            }
        }
        touched.sort_unstable();
        touched.dedup();
        for b in touched {
            self.refresh(b);
        }
    }

    /// Leave no ink at pixel `i`.
    fn zero(&mut self, i: usize) {
        self.ink[i] = 0.0;
        let b = self.block_of(i);
        self.refresh(b);
    }
}

/// Chain straight strokes over the dark parts of the tone map, one segment at a time.
///
/// A stroke starts at the centre of the darkest pixel of the residual. At each step of it the stage
/// tries `candidates` segments of `length_mm` at angles drawn uniformly from the seeded generator,
/// each end clamped to the tone map, and skips those that end less than a pixel from the pen. The
/// best is the one with the highest mean residual ink at the pixels nearest to points half a pixel
/// apart along it (the first of equals wins). If that mean reaches `stop`, the segment becomes the
/// next vertex of the stroke and every pixel under it loses `lighten` ink. Otherwise the stroke
/// ends and the pen moves to the darkest pixel, unless it holds under `stop` ink or none at all, in
/// which case the run is over. The run is also over after `max_lines` segments.
///
/// A stroke that ends with one vertex is dropped, and the ink at its pixel is cleared so that the
/// pixel is not the darkest again. The run reports a frame of the strokes so far after about a
/// thirtieth of `max_lines` passes of the loop, or fewer when those passes would read more than
/// [`SAMPLE_BUDGET`] ink samples, then gives the paths.
pub struct SketchStage;

impl Stage for SketchStage {
    fn name(&self) -> &'static str {
        "sketch"
    }
    fn label(&self) -> &'static str {
        "Sketch lines"
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
        schema_for::<SketchSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<SketchSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<SketchSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        // A candidate reads a sample every half pixel along its length.
        let per_pass = s.candidates as usize * (2.0 * s.length_mm / tone.mm_per_px).ceil() as usize;
        let budgeted = u32::try_from(SAMPLE_BUDGET / per_pass).unwrap_or(u32::MAX);
        Ok(Box::new(SketchRun {
            per_step: (s.max_lines / 30).min(budgeted).max(1),
            residual: Residual::new(tone),
            rng: ctx.rng(u64::from(s.seed)),
            s,
            paths: Paths::new(),
            stroke: Vec::new(),
            drawn: 0,
            phase: Phase::Drawing,
        }))
    }
}

/// Where a run is.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// Segments are still to be drawn.
    Drawing,
    /// Nothing is left to draw; the next step gives the paths.
    Finished,
    /// The paths have been given.
    Delivered,
}

struct SketchRun {
    s: SketchSettings,
    /// Passes of the loop in one step.
    per_step: u32,
    residual: Residual,
    rng: Pcg64,
    /// The strokes that have ended.
    paths: Paths,
    /// The stroke in progress; empty when the pen needs a new place to start.
    stroke: Vec<[f64; 2]>,
    /// Segments drawn so far.
    drawn: u32,
    phase: Phase,
}

impl SketchRun {
    /// The best of the candidate segments from `pen`: its mean ink and its far end, or `None` when
    /// no candidate reaches a pixel away.
    fn best_segment(&mut self, pen: [f64; 2]) -> Option<(f64, [f64; 2])> {
        let rect = self.residual.tone.rect_mm();
        let pixel = self.residual.tone.mm_per_px;
        let mut best: Option<(f64, [f64; 2])> = None;
        for _ in 0..self.s.candidates {
            let angle = self.rng.random::<f64>() * TAU;
            let end = clamp_to(
                rect,
                [
                    pen[0] + self.s.length_mm * math::cos(angle),
                    pen[1] + self.s.length_mm * math::sin(angle),
                ],
            );
            if math::hypot(end[0] - pen[0], end[1] - pen[1]) < pixel {
                continue;
            }
            let score = self.residual.mean_along(pen, end);
            if best.is_none_or(|(top, _)| score > top) {
                best = Some((score, end));
            }
        }
        best
    }

    /// Give the stroke in progress to the paths, or drop it if it has one vertex.
    fn end_stroke(&mut self) {
        if self.stroke.len() >= 2 {
            self.paths.push(&self.stroke);
        } else if let Some(&start) = self.stroke.first() {
            let i = self.residual.pixel(start);
            self.residual.zero(i);
        }
        self.stroke.clear();
    }

    /// One pass of the loop: draw a segment, end the stroke, or end the run.
    fn advance(&mut self) {
        if self.drawn >= self.s.max_lines {
            return self.finish();
        }
        let pen = match self.stroke.last() {
            Some(&pen) => pen,
            None => {
                let (i, ink) = self.residual.darkest();
                if ink <= 0.0 || f64::from(ink) < self.s.stop {
                    return self.finish();
                }
                let start = self.residual.centre(i);
                self.stroke.push(start);
                start
            }
        };
        match self.best_segment(pen) {
            Some((score, end)) if score >= self.s.stop => {
                self.stroke.push(end);
                self.drawn += 1;
                let by = self.s.lighten as f32;
                self.residual.lighten(pen, end, by, self.drawn);
            }
            _ => self.end_stroke(),
        }
    }

    /// End the run, keeping the stroke in progress.
    fn finish(&mut self) {
        self.end_stroke();
        self.phase = Phase::Finished;
    }
}

impl StageRun for SketchRun {
    fn step(&mut self) -> Result<Step, StageError> {
        match self.phase {
            Phase::Delivered => {
                return Err(StageError::Failed(
                    "the sketch run has already finished".into(),
                ))
            }
            Phase::Finished => {
                self.phase = Phase::Delivered;
                let paths = std::mem::take(&mut self.paths);
                return Ok(Step::Done(Artifact::Paths(Arc::new(paths))));
            }
            Phase::Drawing => {}
        }
        for _ in 0..self.per_step {
            if self.phase != Phase::Drawing {
                break;
            }
            self.advance();
        }
        let mut shown = self.paths.clone();
        if self.stroke.len() >= 2 {
            shown.push(&self.stroke);
        }
        let progress = if self.phase == Phase::Drawing {
            self.drawn as f32 / self.s.max_lines as f32
        } else {
            1.0
        };
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress,
            label: format!("{}/{} segments", self.drawn, self.s.max_lines),
            points: None,
            paths: Some(shown),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{assert_inside, bounds, run_tone_stage, tone_map};
    use serde_json::json;
    use std::sync::Arc;

    /// The segments `paths` holds: one fewer than the vertices of each path.
    fn segments(paths: &Paths) -> usize {
        paths.iter().map(|p| p.len() - 1).sum()
    }

    fn residual(w: u32, h: u32, ink: impl Fn(u32, u32) -> f32) -> Residual {
        Residual::new(Arc::new(tone_map(w, h, ink)))
    }

    /// Panic unless every block's kept maximum is the maximum of its pixels, found afresh.
    fn assert_peaks_current(r: &Residual) {
        let (w, h) = (r.tone.width as usize, r.tone.height as usize);
        let mut want = vec![f32::NEG_INFINITY; r.peak.len()];
        for row in 0..h {
            for col in 0..w {
                let b = (row / BLOCK) * w.div_ceil(BLOCK) + col / BLOCK;
                want[b] = want[b].max(r.ink[row * w + col]);
            }
        }
        assert_eq!(r.peak, want);
    }

    #[test]
    fn settings_schema() {
        let s = SketchStage.schema();
        assert_eq!(bounds(&s, "length_mm"), (1.0, 50.0, 6.0));
        assert_eq!(bounds(&s, "candidates"), (4.0, 256.0, 48.0));
        assert_eq!(bounds(&s, "lighten"), (0.05, 1.0, 0.3));
        assert_eq!(bounds(&s, "stop"), (0.0, 1.0, 0.2));
        assert_eq!(bounds(&s, "max_lines"), (100.0, 50000.0, 5000.0));
        assert_eq!(bounds(&s, "seed"), (0.0, 9999.0, 1.0));
        assert_eq!(
            (SketchStage.name(), SketchStage.label()),
            ("sketch", "Sketch lines")
        );
        assert_eq!(SketchStage.version(), 1);
        assert_eq!(
            (SketchStage.requires(), SketchStage.provides()),
            (&[Kind::Tone][..], Kind::Paths)
        );
    }

    #[test]
    fn white_gives_no_strokes() {
        let (frames, paths) = run_tone_stage(&SketchStage, tone_map(20, 20, |_, _| 0.0), json!({}));
        assert!(paths.is_empty());
        assert_eq!(frames.last().unwrap().progress, 1.0);
    }

    #[test]
    fn strokes_cover_a_dark_bar() {
        // The bar spans rows 18 to 21 of 2 mm each from y = 10 mm: 46 to 54 mm.
        let bar = |_, y| if (18..22).contains(&y) { 1.0 } else { 0.0 };
        let (_, paths) = run_tone_stage(&SketchStage, tone_map(40, 40, bar), json!({}));
        assert!(!paths.is_empty());
        for p in paths.coords() {
            assert!((46.0 - 6.0..=54.0 + 6.0).contains(&p[1]), "{p:?}");
        }
    }

    #[test]
    fn max_lines_caps_the_segments() {
        let (_, paths) = run_tone_stage(
            &SketchStage,
            tone_map(40, 40, |_, _| 1.0),
            json!({"max_lines": 100}),
        );
        assert_eq!(segments(&paths), 100);
    }

    #[test]
    fn strokes_stay_on_the_map() {
        // A segment of 50 mm from anywhere on a 40 mm map is clamped nearly every time.
        let (_, paths) = run_tone_stage(
            &SketchStage,
            tone_map(20, 20, |_, _| 1.0),
            json!({"length_mm": 50, "max_lines": 300}),
        );
        assert!(segments(&paths) > 20);
        assert_inside(&paths, [10.0, 10.0, 40.0, 40.0]);
    }

    #[test]
    fn a_seed_gives_the_same_strokes_and_another_gives_others() {
        let ink = |x: u32, y: u32| ((x * 7 + y * 3) % 10) as f32 / 10.0;
        let run = |patch| run_tone_stage(&SketchStage, tone_map(20, 20, ink), patch).1;
        let (a, b) = (run(json!({})), run(json!({})));
        assert!(!a.is_empty());
        assert_eq!(*a, *b);
        assert_ne!(*a, *run(json!({"seed": 2})));
    }

    #[test]
    fn the_first_stroke_starts_at_the_darkest_pixel() {
        let map = tone_map(20, 20, |x, y| if (x, y) == (7, 9) { 1.0 } else { 0.5 });
        let (_, paths) = run_tone_stage(&SketchStage, map, json!({}));
        // The centre of pixel (7, 9): 2 mm per pixel from (10, 10) mm.
        assert_eq!(paths.get(0)[0], [25.0, 29.0]);
    }

    #[test]
    fn a_lone_pixel_too_dark_to_chain_is_dropped_and_the_run_ends() {
        // No segment from it averages 0.9, so its stroke keeps one vertex and is dropped; the
        // pixel must then stop being the darkest, or the run would try it for ever.
        let map = tone_map(20, 20, |x, y| if (x, y) == (7, 9) { 1.0 } else { 0.0 });
        let (frames, paths) = run_tone_stage(&SketchStage, map, json!({"stop": 0.9}));
        assert!(paths.is_empty());
        assert_eq!(frames.last().unwrap().progress, 1.0);
    }

    #[test]
    fn frames_hold_the_strokes_so_far_and_grow_by_a_thirtieth_at_most() {
        let (frames, paths) = run_tone_stage(
            &SketchStage,
            tone_map(40, 40, |_, _| 1.0),
            json!({"max_lines": 1000}),
        );
        assert!(frames.len() >= 30, "{}", frames.len());
        let mut before = 0;
        for f in &frames {
            let now = segments(f.paths.as_ref().unwrap());
            assert!(before <= now && now - before <= 33, "{before} {now}");
            before = now;
        }
        assert_eq!(before, segments(&paths));
        let last = frames.last().unwrap();
        assert_eq!(last.progress, 1.0);
        assert_eq!(last.label, format!("{before}/1000 segments"));
    }

    #[test]
    fn a_step_at_the_widest_settings_stays_within_the_sample_budget() {
        // A segment of 50 mm samples 50 pixels of 2 mm, so 256 candidates take 12,800 samples a
        // pass and the budget allows 156 passes a step, far fewer than a thirtieth of 50,000. On
        // full ink with a light touch every pass draws a segment, so the label counts the passes.
        let tone = Artifact::Tone(Arc::new(tone_map(100, 100, |_, _| 1.0)));
        let inputs = Inputs::new([(Kind::Tone, tone)].into_iter().collect());
        let patch =
            json!({"candidates": 256, "length_mm": 50.0, "max_lines": 50000, "lighten": 0.05});
        let settings = crate::settings::merge(&SketchStage.defaults(), &patch);
        let mut run = SketchStage
            .start(inputs, &settings, RunContext::new(Arc::default()))
            .unwrap();
        let Ok(Step::Frame(frame)) = run.step() else {
            panic!("expected a frame");
        };
        assert_eq!(SAMPLE_BUDGET / (256 * 50), 156);
        assert_eq!(frame.label, "156/50000 segments");
    }

    #[test]
    fn the_darkest_pixel_is_the_first_of_the_first_block_that_holds_it() {
        // 40 by 40 pixels make 3 by 3 blocks. (20, 17) comes first in row-major pixel order, but
        // its block (1, 1) comes after block (0, 1), which holds (2, 20) and (1, 21).
        let tie = |x: u32, y: u32| matches!((x, y), (2, 20) | (1, 21) | (20, 17));
        let mut r = residual(40, 40, |x, y| if tie(x, y) { 0.9 } else { 0.1 });
        assert_eq!(r.darkest(), (20 * 40 + 2, 0.9));
        r.zero(20 * 40 + 2);
        assert_peaks_current(&r);
        assert_eq!(r.darkest(), (21 * 40 + 1, 0.9));
        r.zero(21 * 40 + 1);
        assert_peaks_current(&r);
        assert_eq!(r.darkest(), (17 * 40 + 20, 0.9));
        r.zero(17 * 40 + 20);
        assert_peaks_current(&r);
        assert_eq!(r.darkest(), (0, 0.1));
    }

    #[test]
    fn lightening_the_only_maximum_moves_the_darkest_pixel_to_another_block() {
        // 40 by 20 pixels make 3 by 2 blocks. (3, 3), in block (0, 0), is the darkest pixel and
        // (20, 4), in block (1, 0), the next.
        let ink = |x: u32, y: u32| match (x, y) {
            (3, 3) => 0.9,
            (20, 4) => 0.6,
            _ => 0.1,
        };
        let mut r = residual(40, 20, ink);
        assert_eq!(r.darkest(), (3 * 40 + 3, 0.9));
        // Both ends fall in pixel (3, 3), whose centre is at (17, 17) mm.
        r.lighten([16.5, 17.0], [17.5, 17.0], 0.5, 1);
        assert_peaks_current(&r);
        assert_eq!(r.darkest(), (4 * 40 + 20, 0.6));
    }

    #[test]
    fn a_segment_lowers_each_pixel_once_however_often_it_samples_it() {
        // Full ink on row 2 from pixel 0 to pixel 4 only, so lowering it lowers the block maximum.
        let mut r = residual(10, 10, |x, y| if y == 2 && x < 5 { 1.0 } else { 0.5 });
        // Along that row, sampled every half pixel: twice per pixel.
        let (a, b) = ([10.5, 15.0], [19.5, 15.0]);
        r.lighten(a, b, 0.3, 1);
        assert_peaks_current(&r);
        for (i, &v) in r.ink.iter().enumerate() {
            let want = if (20..25).contains(&i) { 0.7 } else { 0.5 };
            assert!((v - want).abs() < 1e-6, "pixel {i}: {v}");
        }
        r.lighten(a, b, 0.3, 2);
        assert_peaks_current(&r);
        assert!((r.ink[22] - 0.4).abs() < 1e-6);
        r.lighten(a, b, 0.3, 3);
        r.lighten(a, b, 0.3, 4);
        assert_peaks_current(&r);
        assert_eq!(r.ink[22], 0.0);
        assert_eq!(r.darkest(), (0, 0.5));
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"length_mm": 0.5}),
            json!({"candidates": 3}),
            json!({"lighten": 0.0}),
            json!({"stop": 1.5}),
            json!({"max_lines": 99}),
            json!({"seed": 10000}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&SketchStage.defaults(), &patch);
            let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let inputs = Inputs::new([(Kind::Tone, tone)].into_iter().collect());
            let ctx = RunContext::new(Arc::default());
            let err = SketchStage.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }

    #[test]
    fn a_finished_run_fails_instead_of_panicking() {
        let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 0.0)));
        let inputs = Inputs::new([(Kind::Tone, tone)].into_iter().collect());
        let ctx = RunContext::new(Arc::default());
        let mut run = SketchStage
            .start(inputs, &SketchStage.defaults(), ctx)
            .unwrap();
        assert!(matches!(run.step(), Ok(Step::Frame(_))));
        assert!(matches!(run.step(), Ok(Step::Done(_))));
        assert!(matches!(run.step(), Err(StageError::Failed(_))));
    }
}
