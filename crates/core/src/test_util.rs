//! Test doubles shared by the engine's unit tests: a stage that does nothing but report, and
//! small artifacts to give it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache::StageCache;
use crate::machine::{Area, Machine, PenLift};
use crate::models::{file_id, ModelStore, StandIn, STANDIN_BYTES, STANDIN_HALF_BYTES};
use crate::runner::{Run, Shared};
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{
    Artifact, Frame, Inputs, Kind, Lane, RunContext, Stage, StageError, StageRun, Step,
};
use crate::style::Style;
use crate::types::{Canvas, Drawing, Fit, Image, Layer, Paths, Pen, Points, ToneMap};

/// `n` points at the origin.
pub(crate) fn points(n: usize) -> Artifact {
    Artifact::Points(Arc::new(Points(vec![[0.0, 0.0]; n])))
}

/// `n` paths of one vertex each, at the origin.
pub(crate) fn paths(n: usize) -> Artifact {
    let mut p = Paths::new();
    for _ in 0..n {
        p.push(&[[0.0, 0.0]]);
    }
    Artifact::Paths(Arc::new(p))
}

/// A tone map of one pixel with ink 0.5.
pub(crate) fn tone() -> Artifact {
    Artifact::Tone(Arc::new(ToneMap {
        width: 1,
        height: 1,
        ink: vec![0.5],
        mm_per_px: 1.0,
        origin_mm: [0.0, 0.0],
    }))
}

/// One tone map per value of `inks`, each of one pixel with that ink, 1 mm per pixel, its corner
/// at the origin.
pub(crate) fn tones(inks: &[f32]) -> Artifact {
    let map = |&ink| {
        Arc::new(ToneMap {
            width: 1,
            height: 1,
            ink: vec![ink],
            mm_per_px: 1.0,
            origin_mm: [0.0, 0.0],
        })
    };
    Artifact::Tones(Arc::new(inks.iter().map(map).collect()))
}

/// Run roots: a 4 by 4 flat grey image, a 100 mm square page with no margin, and one black pen.
pub(crate) fn roots() -> [Artifact; 3] {
    roots_with(vec![black()])
}

/// [`roots`] with `pens` in place of the one black pen.
pub(crate) fn roots_with(pens: Vec<Pen>) -> [Artifact; 3] {
    let image = Image {
        width: 4,
        height: 4,
        rgb: vec![0.5; 4 * 4 * 3],
    };
    let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
    [
        Artifact::Image(Arc::new(image)),
        Artifact::Canvas(Arc::new(canvas)),
        Artifact::Pens(Arc::new(pens)),
    ]
}

/// A drawing on `canvas` with one layer of `pen` holding `paths`, in order, and no tone map.
fn drawing_on(canvas: Canvas, pen: Pen, paths: Vec<Vec<[f64; 2]>>) -> Drawing {
    let mut layer_paths = Paths::new();
    for path in &paths {
        layer_paths.push(path);
    }
    Drawing {
        canvas,
        layers: vec![Layer {
            pen,
            paths: layer_paths,
        }],
        tone: None,
        preset: None,
    }
}

/// An A4 page with a 10 mm margin.
pub(crate) fn a4() -> Canvas {
    Canvas::new(210.0, 297.0, 10.0, Fit::Fit).unwrap()
}

/// The pen "Black 0.3": 0.3 mm wide, black.
pub(crate) fn black() -> Pen {
    Pen::new("Black 0.3", 0.3, "#000000").unwrap()
}

/// A drawing on [`a4`] with one layer of [`black`] holding `paths`, in order, and no tone map.
pub(crate) fn drawing_with(paths: Vec<Vec<[f64; 2]>>) -> Drawing {
    drawing_on(a4(), black(), paths)
}

/// A drawing on a 100 by 100 mm page with no margin and one layer of a pen `pen_width_mm` wide in
/// `color` holding `paths`, in order, and no tone map.
pub(crate) fn drawing_100(paths: Vec<Vec<[f64; 2]>>, pen_width_mm: f64, color: &str) -> Drawing {
    drawing_on(
        Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap(),
        Pen::new("Pen", pen_width_mm, color).unwrap(),
        paths,
    )
}

/// [`drawing_100`] with the pen "Black 0.3" and a tone map of full ink over the whole page.
pub(crate) fn drawing_100_with_tone(paths: Vec<Vec<[f64; 2]>>) -> Drawing {
    let mut d = drawing_100(paths, 0.3, "#000000");
    d.tone = Some(Arc::new(ToneMap {
        width: 1,
        height: 1,
        ink: vec![1.0],
        mm_per_px: 100.0,
        origin_mm: [0.0, 0.0],
    }));
    d
}

/// Every frame `run` gives until it stops.
pub(crate) fn drain(run: &mut Run) -> Vec<Frame> {
    std::iter::from_fn(|| run.next_frame()).collect()
}

/// The id a session gives the stand-in model that answers with the Sobel magnitude.
pub(crate) fn standin_id() -> String {
    file_id(STANDIN_BYTES)
}

/// The id a session gives the stand-in model that answers with half the Sobel magnitude.
pub(crate) fn standin_half_id() -> String {
    file_id(STANDIN_HALF_BYTES)
}

/// A model store with the stand-in runtime and both stand-in models loaded.
pub(crate) fn standin_store() -> Arc<ModelStore> {
    let store = ModelStore::new(Some(Arc::new(StandIn::default())));
    store.add(None, STANDIN_BYTES).unwrap();
    store.add(None, STANDIN_HALF_BYTES).unwrap();
    Arc::new(store)
}

/// Run `style` with its default settings over `image` and the other [`roots`], with a fresh cache,
/// and return the drawing. A style that runs a model gets [`standin_store`], the model
/// [`standin_id`] and the least resolution. Panics if the run ends without a drawing.
pub(crate) fn run_to_drawing(style: &Arc<Style>, image: Image) -> Arc<Drawing> {
    let [_, canvas, pens] = roots();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut settings = style.defaults();
    if style.stage_index("model").is_some() {
        // The stand-in finds lines where the picture changes by a sixteenth of its range in a
        // pixel, so a small test image is not stretched far.
        settings["model"] = serde_json::json!({"model": standin_id(), "resolution": 256});
    }
    let mut run = Run::new(
        style.clone(),
        settings,
        [Artifact::Image(Arc::new(image)), canvas, pens],
        Arc::new(Mutex::new(StageCache::new(1 << 20))),
        shared.clone(),
        Arc::default(),
        0,
    )
    .with_models(standin_store());
    drain(&mut run);
    let drawing = shared.lock().unwrap().drawing.clone();
    drawing.unwrap_or_else(|| panic!("style {:?} gave no drawing", style.name))
}

/// A `w` by `h` tone map with `ink(x, y)` per pixel, 2 mm per pixel, its corner at (10, 10) mm.
pub(crate) fn tone_map(w: u32, h: u32, ink: impl Fn(u32, u32) -> f32) -> ToneMap {
    let ink = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| ink(x, y));
    ToneMap {
        width: w,
        height: h,
        ink: ink.collect(),
        mm_per_px: 2.0,
        origin_mm: [10.0, 10.0],
    }
}

/// Run `stage` with `patch` over its defaults, giving it each of `inputs` under its own kind;
/// returns the frames and the artifact. Panics on an error.
pub(crate) fn run_stage(
    stage: &dyn Stage,
    inputs: Vec<Artifact>,
    patch: Value,
) -> (Vec<Frame>, Artifact) {
    run_stage_in_lane(stage, inputs, patch, Lane::SINGLE)
}

/// [`run_stage`] for the stage's run in `lane` of a split.
pub(crate) fn run_stage_in_lane(
    stage: &dyn Stage,
    inputs: Vec<Artifact>,
    patch: Value,
    lane: Lane,
) -> (Vec<Frame>, Artifact) {
    let settings = crate::settings::merge(&stage.defaults(), &patch);
    let inputs = Inputs::new(inputs.into_iter().map(|a| (a.kind(), a)).collect());
    let ctx = RunContext::new(Arc::default()).in_lane(lane);
    let mut run = stage.start(inputs, &settings, ctx).unwrap();
    let mut frames = Vec::new();
    loop {
        match run.step().unwrap() {
            Step::Frame(f) => frames.push(f),
            Step::Done(a) => return (frames, a),
        }
    }
}

/// Run a stage that turns a tone map into paths, with `patch` over its defaults; returns the
/// frames and the paths.
pub(crate) fn run_tone_stage(
    stage: &dyn Stage,
    tone: ToneMap,
    patch: Value,
) -> (Vec<Frame>, Arc<Paths>) {
    run_tone_stage_in_lane(stage, tone, patch, Lane::SINGLE)
}

/// [`run_tone_stage`] for the stage's run in `lane` of a split.
pub(crate) fn run_tone_stage_in_lane(
    stage: &dyn Stage,
    tone: ToneMap,
    patch: Value,
    lane: Lane,
) -> (Vec<Frame>, Arc<Paths>) {
    match run_stage_in_lane(stage, vec![Artifact::Tone(Arc::new(tone))], patch, lane) {
        (frames, Artifact::Paths(p)) => (frames, p),
        (_, a) => panic!("expected paths, got {}", a.kind().name()),
    }
}

/// Panic unless every vertex of `paths` lies in `rect` (`[x, y, w, h]`).
pub(crate) fn assert_inside(paths: &Paths, rect: [f64; 4]) {
    let [x, y, w, h] = rect;
    for p in paths.coords() {
        let ok =
            x - 1e-9 <= p[0] && p[0] <= x + w + 1e-9 && y - 1e-9 <= p[1] && p[1] <= y + h + 1e-9;
        assert!(ok, "{p:?} is outside {rect:?}");
    }
}

/// The minimum, maximum and default of setting `key` in `schema`.
pub(crate) fn bounds(schema: &Value, key: &str) -> (f64, f64, f64) {
    let p = &schema["properties"][key];
    (
        p["minimum"].as_f64().unwrap(),
        p["maximum"].as_f64().unwrap(),
        p["default"].as_f64().unwrap(),
    )
}

/// A pen-fitted printer: a 200 by 174 mm area with its corner at (28, 42), the pen down at Z 4,
/// up at 9 and safe at 20, and feeds of 3600, 6000 and 1200 mm/min.
pub(crate) fn machine() -> Machine {
    Machine {
        name: "Test printer".to_string(),
        area: Area {
            x: 28.0,
            y: 42.0,
            width: 200.0,
            height: 174.0,
        },
        z_down_mm: 4.0,
        z_up_mm: 9.0,
        z_safe_mm: 20.0,
        draw_feed: 3600.0,
        travel_feed: 6000.0,
        z_feed: 1200.0,
        settle_ms: 50,
        start_gcode: "M104 S0\nM140 S0".to_string(),
        end_gcode: "G1 Z20 F600\nG1 Y250 F3600".to_string(),
        pen_lift: PenLift::Z,
        pen_down_gcode: String::new(),
        pen_up_gcode: String::new(),
        max_segment_mm: None,
    }
}

/// [`machine`] with its pen lifted by a servo command and a 100 ms dwell, and end lines that stay
/// in the area, since there is no safe height to leave it at.
pub(crate) fn servo_machine() -> Machine {
    Machine {
        pen_lift: PenLift::Gcode,
        pen_down_gcode: "M280 P0 S30".to_string(),
        pen_up_gcode: "M280 P0 S90".to_string(),
        settle_ms: 100,
        end_gcode: "M400".to_string(),
        ..machine()
    }
}

/// The [`Spy`]'s settings; the one knob does nothing but take part in validation and cache keys.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SpySettings {
    /// Test knob.
    pub knob: i64,
}

/// A stage with a chosen name, kinds and output. It reports `frames` frames, then gives `value`,
/// and counts its starts, so runner tests can tell what ran. Its frames leave `stage` empty for
/// the runner to fill in.
pub(crate) struct Spy {
    name: &'static str,
    label: &'static str,
    requires: &'static [Kind],
    provides: Kind,
    value: Artifact,
    frames: usize,
    fail_after: Option<usize>,
    cooperative_cancel: bool,
    runs: Arc<AtomicUsize>,
}

impl Spy {
    /// A spy that reports one frame and then gives `value`; its label is `name` capitalised.
    pub(crate) fn new(
        name: &'static str,
        requires: &'static [Kind],
        provides: Kind,
        value: Artifact,
    ) -> Spy {
        let mut label = name.to_string();
        label[..1].make_ascii_uppercase();
        Spy {
            name,
            // A leak per test double is cheaper than making `label` an owned string.
            label: Box::leak(label.into_boxed_str()),
            requires,
            provides,
            value,
            frames: 1,
            fail_after: None,
            cooperative_cancel: false,
            runs: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Report `n` frames before the artifact.
    pub(crate) fn with_frames(mut self, n: usize) -> Spy {
        self.frames = n;
        self
    }

    /// Fail with [`StageError::Failed`] at step `k`, counting steps from 1.
    pub(crate) fn with_fail_after(mut self, k: usize) -> Spy {
        self.fail_after = Some(k);
        self
    }

    /// Make every step a long chunk that ends only once `ctx.cancelled()` is set, then returns
    /// [`StageError::Cancelled`], as a stage that checks the flag mid-chunk does. A test sets the
    /// flag from another thread after the spy has started. Panics after 10 s without a cancel.
    pub(crate) fn with_cooperative_cancel(mut self) -> Spy {
        self.cooperative_cancel = true;
        self
    }

    /// How many runs have started.
    pub(crate) fn runs(&self) -> usize {
        self.runs.load(Ordering::SeqCst)
    }
}

impl Stage for Spy {
    fn name(&self) -> &'static str {
        self.name
    }
    fn label(&self) -> &'static str {
        self.label
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        self.requires
    }
    fn provides(&self) -> Kind {
        self.provides
    }
    fn schema(&self) -> Value {
        schema_for::<SpySettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<SpySettings>()
    }
    fn start(
        &self,
        _inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        validate::<SpySettings>(self.name, &self.schema(), settings)?;
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(SpyRun {
            value: self.value.clone(),
            frames: self.frames,
            fail_after: self.fail_after,
            cancel: self.cooperative_cancel.then_some(ctx),
            step: 0,
        }))
    }
}

struct SpyRun {
    value: Artifact,
    frames: usize,
    fail_after: Option<usize>,
    /// The context to wait on, when the spy cancels cooperatively.
    cancel: Option<RunContext>,
    /// Steps taken so far.
    step: usize,
}

impl StageRun for SpyRun {
    fn step(&mut self) -> Result<Step, StageError> {
        self.step += 1;
        if let Some(ctx) = &self.cancel {
            let start = std::time::Instant::now();
            while !ctx.cancelled() {
                let waited = start.elapsed();
                assert!(waited.as_secs() < 10, "spy waited 10 s for a cancel");
                std::thread::yield_now();
            }
            return Err(StageError::Cancelled);
        }
        if self.fail_after == Some(self.step) {
            return Err(StageError::Failed("spy failed".into()));
        }
        if self.step > self.frames {
            return Ok(Step::Done(self.value.clone()));
        }
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress: self.step as f32 / self.frames as f32,
            label: format!("{}/{}", self.step, self.frames),
            points: None,
            paths: None,
            pen: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn start(spy: &Spy, settings: &Value) -> Result<Box<dyn StageRun>, StageError> {
        let ctx = RunContext::new(Arc::default());
        spy.start(Inputs::new(HashMap::new()), settings, ctx)
    }

    #[test]
    fn spy_reports_frames_then_the_value() {
        let spy = Spy::new("spy", &[], Kind::Points, points(2)).with_frames(2);
        assert_eq!(spy.label(), "Spy");
        let mut run = start(&spy, &json!({})).unwrap();
        for (i, label) in ["1/2", "2/2"].into_iter().enumerate() {
            let Step::Frame(f) = run.step().unwrap() else {
                panic!("expected a frame");
            };
            assert_eq!((f.stage.as_str(), f.label.as_str()), ("", label));
            assert_eq!(f.progress, (i + 1) as f32 / 2.0);
        }
        let Step::Done(a) = run.step().unwrap() else {
            panic!("expected the artifact");
        };
        assert_eq!(a.kind(), Kind::Points);
    }

    #[test]
    fn spy_fails_at_the_chosen_step_and_counts_starts() {
        let spy = Spy::new("spy", &[], Kind::Points, points(1))
            .with_frames(3)
            .with_fail_after(2);
        let mut run = start(&spy, &json!({"knob": 4})).unwrap();
        assert!(matches!(run.step(), Ok(Step::Frame(_))));
        assert!(matches!(run.step(), Err(StageError::Failed(m)) if m == "spy failed"));
        assert_eq!(spy.runs(), 1);
        assert!(start(&spy, &json!({"nob": 4})).is_err());
        assert_eq!(spy.runs(), 1);
    }
}
