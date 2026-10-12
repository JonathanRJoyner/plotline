//! Test doubles shared by the engine's unit tests: a stage that does nothing but report, and
//! small artifacts to give it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache::StageCache;
use crate::runner::{Run, Shared};
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
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

/// Run roots: a 4 by 4 flat grey image, a 100 mm square page with no margin, and one black pen.
pub(crate) fn roots() -> [Artifact; 3] {
    let image = Image {
        width: 4,
        height: 4,
        rgb: vec![0.5; 4 * 4 * 3],
    };
    let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
    let pen = Pen::new("Black 0.3", 0.3, "#000000").unwrap();
    [
        Artifact::Image(Arc::new(image)),
        Artifact::Canvas(Arc::new(canvas)),
        Artifact::Pens(Arc::new(vec![pen])),
    ]
}

/// A drawing on an A4 page with a 10 mm margin and one layer of the pen "Black 0.3" holding
/// `paths`, in order, and no tone map.
pub(crate) fn drawing_with(paths: Vec<Vec<[f64; 2]>>) -> Drawing {
    let mut layer_paths = Paths::new();
    for path in &paths {
        layer_paths.push(path);
    }
    Drawing {
        canvas: Canvas::new(210.0, 297.0, 10.0, Fit::Fit).unwrap(),
        layers: vec![Layer {
            pen: Pen::new("Black 0.3", 0.3, "#000000").unwrap(),
            paths: layer_paths,
        }],
        tone: None,
    }
}

/// Every frame `run` gives until it stops.
pub(crate) fn drain(run: &mut Run) -> Vec<Frame> {
    std::iter::from_fn(|| run.next_frame()).collect()
}

/// Run `style` with its default settings over `image` and the other [`roots`], with a fresh cache,
/// and return the drawing. Panics if the run ends without one.
pub(crate) fn run_to_drawing(style: &Arc<Style>, image: Image) -> Arc<Drawing> {
    let [_, canvas, pens] = roots();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let mut run = Run::new(
        style.clone(),
        style.defaults(),
        [Artifact::Image(Arc::new(image)), canvas, pens],
        Arc::new(Mutex::new(StageCache::new(1 << 20))),
        shared.clone(),
        Arc::default(),
    );
    drain(&mut run);
    let drawing = shared.lock().unwrap().drawing.clone();
    drawing.unwrap_or_else(|| panic!("style {:?} gave no drawing", style.name))
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
