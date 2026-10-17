//! The stage contract: what a stage needs and provides, and the run it hands back.
//!
//! A stage runs as a step function rather than one call because the engine also runs in a
//! WebAssembly worker, which can only honour a cancel message between calls into the engine.
//! [`Stage::start`] validates the settings and returns a [`StageRun`]; each [`StageRun::step`]
//! does one chunk of work and returns a progress [`Frame`] or the finished [`Artifact`].

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rand::SeedableRng;
use rand_pcg::Pcg64;
use serde_json::Value;
use thiserror::Error;

use crate::models::ModelStore;
use crate::settings::SettingsError;
use crate::types::{Canvas, Image, Layer, Paths, Pen, Points, ToneMap, Tones};

/// The kind of artifact a stage requires or provides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The source image.
    Image,
    /// The page.
    Canvas,
    /// The pens.
    Pens,
    /// The tone map.
    Tone,
    /// One tone map per pen, from the stage that splits the drawing between the pens.
    Tones,
    /// Points on the page.
    Points,
    /// Paths on the page.
    Paths,
    /// Paths grouped by pen.
    Layers,
}

impl Kind {
    /// The kinds a run is given rather than computes.
    pub const ROOTS: [Kind; 3] = [Kind::Image, Kind::Canvas, Kind::Pens];

    /// The lowercase name clients see.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Canvas => "canvas",
            Kind::Pens => "pens",
            Kind::Tone => "tone",
            Kind::Tones => "tones",
            Kind::Points => "points",
            Kind::Paths => "paths",
            Kind::Layers => "layers",
        }
    }
}

/// A stage's output or a run's root input, shared by reference count rather than copied.
#[derive(Clone, Debug)]
pub enum Artifact {
    /// The source image.
    Image(Arc<Image>),
    /// The page.
    Canvas(Arc<Canvas>),
    /// The pens.
    Pens(Arc<Vec<Pen>>),
    /// The tone map.
    Tone(Arc<ToneMap>),
    /// One tone map per pen, from the stage that splits the drawing between the pens.
    Tones(Arc<Tones>),
    /// Points on the page.
    Points(Arc<Points>),
    /// Paths on the page.
    Paths(Arc<Paths>),
    /// Paths grouped by pen.
    Layers(Arc<Vec<Layer>>),
}

impl Artifact {
    /// The artifact's kind.
    pub fn kind(&self) -> Kind {
        match self {
            Artifact::Image(_) => Kind::Image,
            Artifact::Canvas(_) => Kind::Canvas,
            Artifact::Pens(_) => Kind::Pens,
            Artifact::Tone(_) => Kind::Tone,
            Artifact::Tones(_) => Kind::Tones,
            Artifact::Points(_) => Kind::Points,
            Artifact::Paths(_) => Kind::Paths,
            Artifact::Layers(_) => Kind::Layers,
        }
    }

    /// The size of the artifact's buffers in bytes; the canvas and the pens count as 64.
    pub fn bytes(&self) -> usize {
        let paths = |p: &Paths| p.coords().len() * 16 + p.offsets().len() * 4;
        match self {
            Artifact::Image(i) => i.rgb.len() * 4,
            Artifact::Canvas(_) | Artifact::Pens(_) => 64,
            Artifact::Tone(t) => t.ink.len() * 4,
            Artifact::Tones(t) => t.maps.iter().map(|m| m.ink.len() * 4).sum(),
            Artifact::Points(p) => p.0.len() * 16,
            Artifact::Paths(p) => paths(p),
            Artifact::Layers(l) => l.iter().map(|layer| paths(&layer.paths)).sum(),
        }
    }
}

/// Progress from a running stage, with optional partial geometry to preview.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The stage's name.
    pub stage: String,
    /// How far the stage has got, from 0 to 1.
    pub progress: f32,
    /// What the stage is doing, for users.
    pub label: String,
    /// Points placed so far, if the stage has any to show.
    pub points: Option<Points>,
    /// Paths made so far, if the stage has any to show.
    pub paths: Option<Paths>,
    /// The pen whose lane gave the frame; 0 outside lanes. The runner sets it.
    pub pen: usize,
}

/// What one step of a run produced.
#[derive(Debug)]
pub enum Step {
    /// The run is not finished; here is its progress.
    Frame(Frame),
    /// The run is finished; here is its output.
    Done(Artifact),
}

/// The artifacts a stage declared it requires, by kind.
pub struct Inputs(HashMap<Kind, Artifact>);

/// A getter that returns the input of one kind and panics if the stage did not declare it.
macro_rules! input {
    ($name:ident, $kind:ident, $ty:ty) => {
        #[doc = concat!("The ", stringify!($name), " input. Panics if the stage did not declare it.")]
        pub fn $name(&self) -> &Arc<$ty> {
            match self.0.get(&Kind::$kind) {
                Some(Artifact::$kind(a)) => a,
                _ => panic!("stage did not declare {}", Kind::$kind.name()),
            }
        }
    };
}

impl Inputs {
    /// Wrap the inputs; the runner supplies every kind the stage requires. Panics if an artifact
    /// is filed under another kind.
    pub fn new(map: HashMap<Kind, Artifact>) -> Inputs {
        assert!(
            map.iter().all(|(k, a)| a.kind() == *k),
            "input filed under the wrong kind"
        );
        Inputs(map)
    }

    input!(image, Image, Image);
    input!(canvas, Canvas, Canvas);
    input!(pens, Pens, Vec<Pen>);
    input!(tone, Tone, ToneMap);
    input!(tones, Tones, Tones);
    input!(points, Points, Points);
    input!(paths, Paths, Paths);
    input!(layers, Layers, Vec<Layer>);
}

/// Which of a run's pens a stage after the split is drawing for: `index` of `count`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lane {
    /// The pen's position in the run's pens.
    pub index: usize,
    /// How many pens the run splits between.
    pub count: usize,
}

impl Lane {
    /// The lane of a stage outside any split: the only one.
    pub const SINGLE: Lane = Lane { index: 0, count: 1 };
}

/// What a running stage may use besides its inputs: the cancel flag, seeded randomness, the lane
/// it runs in, the lane's part of a per-pen budget and the session's learned line models.
pub struct RunContext {
    cancel: Arc<AtomicBool>,
    lane: Lane,
    budget: [f64; 2],
    models: Arc<ModelStore>,
}

impl RunContext {
    /// A context in [`Lane::SINGLE`], with a whole budget and no models, that reports cancelled
    /// once `cancel` is set.
    pub fn new(cancel: Arc<AtomicBool>) -> RunContext {
        RunContext {
            cancel,
            lane: Lane::SINGLE,
            budget: [0.0, 1.0],
            models: Arc::default(),
        }
    }

    /// The context for the same run in `lane`.
    pub fn in_lane(self, lane: Lane) -> RunContext {
        RunContext { lane, ..self }
    }

    /// The context for the same run with `models` for the stages that run them.
    pub fn with_models(self, models: Arc<ModelStore>) -> RunContext {
        RunContext { models, ..self }
    }

    /// The learned line models the run may use.
    pub fn models(&self) -> &ModelStore {
        &self.models
    }

    /// The context for the same run whose lane spends `budget` of a per-pen budget; see
    /// [`budget`].
    ///
    /// [`budget`]: RunContext::budget
    pub fn with_budget(self, budget: [f64; 2]) -> RunContext {
        RunContext { budget, ..self }
    }

    /// The lane the stage runs in.
    pub fn lane(&self) -> Lane {
        self.lane
    }

    /// The lane's part of a budget a stage spends per pen, such as the Voronoi placer's points,
    /// as `[start, end]` counted in whole budgets: of a budget `b` per pen, the lane spends
    /// `round(b * end) - round(b * start)`, so lanes whose parts meet spend their total exactly.
    /// The split stage sets the parts ([`Tones`]); outside a split it is `[0, 1]`.
    pub fn budget(&self) -> [f64; 2] {
        self.budget
    }

    /// Whether the run has been cancelled.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// A generator seeded from `seed` and the lane, so the same seed gives the same output
    /// everywhere. Stages draw randomness only from here.
    ///
    /// The first lane, and so a run with one pen, uses `seed` alone; lane `i` adds `i * 2^32`, so
    /// with the `u32` seed settings no lane repeats any first lane's stream. Each pen then draws
    /// an independent pattern, which is what the Demichel equations of the colour model assume
    /// (I. Amidror and R. D. Hersch, "Neugebauer and Demichel: dependence and independence in
    /// n-screen superpositions for colour printing", Color Research and Application 25(4), 2000).
    pub fn rng(&self, seed: u64) -> Pcg64 {
        Pcg64::seed_from_u64(seed.wrapping_add((self.lane.index as u64) << 32))
    }
}

/// Why a stage stopped without an artifact.
#[derive(Debug, Error)]
pub enum StageError {
    /// The run was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The settings did not validate.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// The stage failed; the message is shown to users.
    #[error("{0}")]
    Failed(String),
}

/// One step of a drawing pipeline: it requires some kinds, provides one, and has settings.
pub trait Stage: Send + Sync {
    /// The stable identifier, used in settings errors and cache keys.
    fn name(&self) -> &'static str;
    /// The name shown to users.
    fn label(&self) -> &'static str;
    /// Bumped whenever the output for the same inputs and settings changes.
    fn version(&self) -> u32;
    /// The kinds the stage reads.
    fn requires(&self) -> &'static [Kind];
    /// The kind the stage produces.
    fn provides(&self) -> Kind;
    /// The JSON Schema of the stage's settings.
    fn schema(&self) -> Value;
    /// The default settings.
    fn defaults(&self) -> Value;
    /// Validate `settings` and return the run that will produce the artifact.
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError>;
}

/// A started stage. `Send` because a host may drive the steps on a worker thread.
pub trait StageRun: Send {
    /// Do one chunk of work and return its progress, or the artifact once finished.
    fn step(&mut self) -> Result<Step, StageError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    #[test]
    fn rng_is_seeded_and_cancel_is_shared() {
        let flag = Arc::new(AtomicBool::new(false));
        let ctx = RunContext::new(flag.clone());
        let (mut a, mut b) = (ctx.rng(7), ctx.rng(7));
        assert_eq!(a.next_u64(), b.next_u64());
        assert!(!ctx.cancelled());
        flag.store(true, Ordering::Relaxed);
        assert!(ctx.cancelled());
    }
    #[test]
    fn each_lane_after_the_first_draws_its_own_stream() {
        let first = |lane| {
            RunContext::new(Arc::default())
                .in_lane(lane)
                .rng(7)
                .next_u64()
        };
        let single = first(Lane::SINGLE);
        assert_eq!(first(Lane { index: 0, count: 2 }), single);
        assert_ne!(first(Lane { index: 1, count: 2 }), single);
    }
    #[test]
    fn artifact_bytes_count_buffers() {
        let mut p = Paths::new();
        p.push(&[[0.0, 0.0], [1.0, 1.0]]);
        assert_eq!(Artifact::Paths(Arc::new(p)).bytes(), 2 * 16 + 2 * 4);
        assert_eq!(
            Artifact::Points(Arc::new(Points(vec![[0.0, 0.0]; 3]))).bytes(),
            48
        );
        let two = crate::test_util::tones(&[0.25, 0.5]);
        assert_eq!((two.kind(), two.bytes()), (Kind::Tones, 8));
    }
    #[test]
    fn a_context_is_single_unless_put_in_a_lane() {
        let ctx = RunContext::new(Arc::default());
        assert_eq!(ctx.lane(), Lane::SINGLE);
        assert_eq!(ctx.budget(), [0.0, 1.0]);
        let lane = Lane { index: 1, count: 3 };
        let ctx = ctx.in_lane(lane).with_budget([0.25, 0.5]);
        assert_eq!((ctx.lane(), ctx.budget()), (lane, [0.25, 0.5]));
    }
    #[test]
    fn tones_input_is_declared_by_kind() {
        let inputs = Inputs::new(HashMap::from([(
            Kind::Tones,
            crate::test_util::tones(&[0.5]),
        )]));
        assert_eq!(inputs.tones().maps.len(), 1);
        assert_eq!(Kind::Tones.name(), "tones");
    }
}
