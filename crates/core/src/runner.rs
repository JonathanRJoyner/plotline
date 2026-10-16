//! The runner: runs a style's stages in order, one frame per call, reusing cached outputs.
//!
//! A stage's output is cached under a key hashed from the stage's name, version and settings and
//! the keys of its inputs, so a change to one stage's settings reruns that stage and the stages
//! after it, and nothing before it. The roots are keyed by their content. Hashes are BLAKE3
//! (O'Connor, Aumasson, Neves and Wilcox-O'Hearn, "BLAKE3: one function, fast everywhere", 2020).
//!
//! A stage that gives tone maps (`Tones`) splits the run into lanes: the stages after it run once
//! per map, as lane `index` of `count`, with that map and the pen of the same index. A lane's tone
//! map is keyed by its content and its pen by its own, and the keys of its stages carry
//! `"{index}/{count}"`, since a stage may read its lane. Each lane gives one layer.
//!
//! A run whose tone stage has `response` auto first drives a run of its style over a step wedge,
//! whose frames pass through as the tone stage's. The later stages then read the tone map through
//! the lookup the wedge gave, keyed by the tone stage's key and the lookup's content, and the
//! drawing keeps the map as the tone stage made it; see [`crate::response`].
//!
//! [`Run`] is driven by its host one [`Run::next_frame`] call at a time, so a host can cancel or
//! drop it between frames. The cache lock is held only while reading or storing one artifact.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::cache::StageCache;
use crate::presets;
use crate::response::{self, Prelude};
use crate::settings::merge;
use crate::stage::{
    Artifact, Frame, Inputs, Kind, Lane, RunContext, Stage, StageError, StageRun, Step,
};
use crate::style::{Source, Style};
use crate::types::{Canvas, Drawing, Layer, Paths, Pen, ToneMap};

/// `v` as JSON with object keys sorted and no whitespace, so equal values give equal text.
pub fn canonical_json(v: &Value) -> String {
    // serde_json's map is a BTreeMap unless its `preserve_order` feature is on; the test pins this.
    serde_json::to_string(v).expect("a JSON value always serializes")
}

/// The BLAKE3 hex digest of a root artifact's content. For the image: the width and height as
/// little-endian `u32`, then each float of `rgb` as little-endian `f32`. For the canvas and the
/// pens: their canonical JSON. Panics for any other kind, or if `artifact` is not of `kind`.
pub fn root_key(kind: Kind, artifact: &Artifact) -> String {
    let json = |v: serde_json::Result<Value>| canonical_json(&v.expect("roots serialize"));
    let bytes = match (kind, artifact) {
        (Kind::Image, Artifact::Image(i)) => {
            let mut b = Vec::with_capacity(8 + i.rgb.len() * 4);
            b.extend(i.width.to_le_bytes());
            b.extend(i.height.to_le_bytes());
            b.extend(i.rgb.iter().flat_map(|x| x.to_le_bytes()));
            b
        }
        (Kind::Canvas, Artifact::Canvas(c)) => json(serde_json::to_value(&**c)).into_bytes(),
        (Kind::Pens, Artifact::Pens(p)) => json(serde_json::to_value(&**p)).into_bytes(),
        _ => panic!(
            "root_key: {} with a {} artifact is not a root",
            kind.name(),
            artifact.kind().name()
        ),
    };
    blake3::hash(&bytes).to_hex().to_string()
}

/// The BLAKE3 hex digest of a tone map's content, laid out as [`root_key`] lays out the image: the
/// width and height as little-endian `u32`, the pixel size and the origin's x and y as
/// little-endian `f64`, then each ink value as little-endian `f32`.
pub fn tone_key(t: &ToneMap) -> String {
    let mut b = Vec::with_capacity(32 + t.ink.len() * 4);
    b.extend(t.width.to_le_bytes());
    b.extend(t.height.to_le_bytes());
    b.extend(
        [t.mm_per_px, t.origin_mm[0], t.origin_mm[1]]
            .iter()
            .flat_map(|x| x.to_le_bytes()),
    );
    b.extend(t.ink.iter().flat_map(|x| x.to_le_bytes()));
    blake3::hash(&b).to_hex().to_string()
}

/// The BLAKE3 hex digest of the canonical JSON
/// `{"stage": name, "version": version, "settings": settings, "inputs": {kind name: key}}`.
pub fn stage_key(stage: &dyn Stage, settings: &Value, inputs: &BTreeMap<&str, &str>) -> String {
    let key = json!({
        "stage": stage.name(),
        "version": stage.version(),
        "settings": settings,
        "inputs": inputs,
    });
    blake3::hash(canonical_json(&key).as_bytes())
        .to_hex()
        .to_string()
}

/// Where a run is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    /// Not started.
    #[default]
    Idle,
    /// Started and not finished.
    Running,
    /// Finished with a drawing.
    Done,
    /// Stopped by the cancel flag.
    Cancelled,
    /// Stopped by a stage error.
    Error,
}

/// The session's result slot, which a run writes when it starts and when it ends.
#[derive(Default)]
pub struct Shared {
    /// The last finished drawing; only a run that ends in [`RunState::Done`] replaces it.
    pub drawing: Option<Arc<Drawing>>,
    /// The state of the run that last wrote the slot.
    pub state: RunState,
    /// The error message when `state` is [`RunState::Error`].
    pub error: Option<String>,
    /// The generation of the newest run. Only a run of this generation writes the slot, so an
    /// older run that ends late cannot overwrite a newer run's state or drawing.
    pub generation: u64,
}

/// One run of a style over three roots, driven a frame at a time by [`Run::next_frame`].
pub struct Run {
    style: Arc<Style>,
    settings: Value,
    /// Each root artifact and its key.
    roots: HashMap<Kind, (Artifact, String)>,
    cache: Arc<Mutex<StageCache>>,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    /// The slot's generation this run was started for.
    generation: u64,
    /// The output and key of each finished stage, in style order, up to and including the split.
    done: Vec<(Artifact, String)>,
    /// One lane per pen once the split has run; empty before that and in a style with no split.
    lanes: Vec<LaneState>,
    /// The stage in progress and its key.
    active: Option<(Box<dyn StageRun>, String)>,
    /// The wedge run to drive before the first stage, while it has not ended.
    prelude: Option<Prelude>,
    /// The tone response lookup the wedge gave, and its key.
    response: Option<(Arc<Vec<f32>>, String)>,
    /// The tone map as the tone stage made it, when the later stages read it through the lookup.
    source: Option<Arc<ToneMap>>,
    state: RunState,
    error: Option<String>,
}

/// One pen's lane once the split has run: its map and pen list with their keys, and the output and
/// key of each finished stage after the split.
struct LaneState {
    tone: (Artifact, String),
    pens: (Artifact, String),
    done: Vec<(Artifact, String)>,
}

impl Run {
    /// A run that has not started. `roots` holds one image, one canvas and one pens artifact in
    /// any order; `settings` maps stage names to settings merged over each stage's defaults, and a
    /// missing stage means its defaults. The run writes `shared` only while its `generation`
    /// equals `generation`. Hashes the roots. Panics if a root kind is missing.
    pub fn new(
        style: Arc<Style>,
        settings: Value,
        roots: [Artifact; 3],
        cache: Arc<Mutex<StageCache>>,
        shared: Arc<Mutex<Shared>>,
        cancel: Arc<AtomicBool>,
        generation: u64,
    ) -> Run {
        let roots: HashMap<Kind, (Artifact, String)> = roots
            .into_iter()
            .map(|a| (a.kind(), (a.clone(), root_key(a.kind(), &a))))
            .collect();
        assert!(
            Kind::ROOTS.iter().all(|k| roots.contains_key(k)),
            "a run needs one image, one canvas and one pens root"
        );
        let prelude = Prelude::new(
            &style,
            &settings,
            Kind::ROOTS.map(|k| &roots[&k].0),
            &cache,
            &cancel,
        );
        Run {
            style,
            settings,
            roots,
            cache,
            shared,
            cancel,
            generation,
            done: Vec::new(),
            lanes: Vec::new(),
            active: None,
            prelude,
            response: None,
            source: None,
            state: RunState::Idle,
            error: None,
        }
    }

    /// Do work until there is a frame to show, and return it. Returns `None` once the run has
    /// ended, after writing the end state, the drawing on success, and the message on failure to
    /// the shared slot. The cancel flag is checked before every stage step and before the run
    /// ends, and a stage that returns [`StageError::Cancelled`] also ends the run as cancelled.
    pub fn next_frame(&mut self) -> Option<Frame> {
        match self.state {
            RunState::Idle => self.write(RunState::Running, None, None),
            RunState::Running => {}
            RunState::Done | RunState::Cancelled | RunState::Error => return None,
        }
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                self.write(RunState::Cancelled, None, None);
                return None;
            }
            if let Some(prelude) = &mut self.prelude {
                if let Some(frame) = prelude.run.next_frame() {
                    // The wedge's geometry is in its own coordinates, so only the progress shows.
                    let tone = self
                        .style
                        .stages
                        .iter()
                        .find(|s| s.provides() == Kind::Tone);
                    return Some(Frame {
                        stage: tone.map_or("", |s| s.name()).to_string(),
                        progress: frame.progress,
                        label: format!("response: {} {}", frame.stage, frame.label),
                        points: None,
                        paths: None,
                        pen: 0,
                    });
                }
                let prelude = self.prelude.take().expect("the prelude is there");
                match prelude.run.state() {
                    RunState::Done => {}
                    RunState::Cancelled => {
                        self.write(RunState::Cancelled, None, None);
                        return None;
                    }
                    _ => {
                        let error = prelude.run.error().unwrap_or_default();
                        self.write(
                            RunState::Error,
                            Some(format!("tone response: {error}")),
                            None,
                        );
                        return None;
                    }
                }
                self.response = prelude.lookup().map(|l| {
                    let key = response::key(&l);
                    (Arc::new(l), key)
                });
                continue;
            }
            let Some((index, lane)) = self.next_stage() else {
                match self.drawing() {
                    Ok(drawing) => self.write(RunState::Done, None, Some(drawing)),
                    Err(e) => self.write(RunState::Error, Some(e), None),
                }
                return None;
            };
            let stage = self.style.stages[index].clone();
            let pen = lane.map_or(0, |l| l.index);
            if self.active.is_none() {
                let defaults = stage.defaults();
                let settings = match self.settings.get(stage.name()) {
                    Some(s) => merge(&defaults, s),
                    None => defaults,
                };
                // The first stage after the split, whose output in a lane is the lane's own.
                let first = self.style.split().map_or(usize::MAX, |s| s + 1);
                let mut inputs = HashMap::new();
                let mut keys = BTreeMap::new();
                for &(kind, source) in self.style.sources(index) {
                    let (artifact, key) = match (source, lane) {
                        (Source::Root(k), _) => &self.roots[&k],
                        (Source::Stage(j), Some(l)) if j >= first => {
                            &self.lanes[l.index].done[j - first]
                        }
                        (Source::Stage(j), _) => &self.done[j],
                        (Source::Lane(Kind::Tone), Some(l)) => &self.lanes[l.index].tone,
                        (Source::Lane(Kind::Pens), Some(l)) => &self.lanes[l.index].pens,
                        (Source::Lane(k), _) => unreachable!("no lane source for {}", k.name()),
                    };
                    inputs.insert(kind, artifact.clone());
                    keys.insert(kind.name(), key.as_str());
                }
                // A stage may read its lane, so the lane is part of its key.
                let tag = lane.map(|l| format!("{}/{}", l.index, l.count));
                if let Some(tag) = &tag {
                    keys.insert("lane", tag);
                }
                let key = stage_key(&*stage, &settings, &keys);
                let cached = self.cache.lock().unwrap().get(&key);
                if let Some(artifact) = cached {
                    // The last stage's frame carries its paths, so a pen that is wholly cached
                    // still draws in a preview while the next pen runs. Earlier stages' geometry
                    // is not what the drawing becomes, and a last stage gives paths or layers.
                    let paths = match &artifact {
                        Artifact::Paths(p) if index + 1 == self.style.stages.len() => {
                            Some(Paths::clone(p))
                        }
                        _ => None,
                    };
                    if let Err(e) = self.land(index, lane, artifact, key) {
                        self.fail(e);
                        return None;
                    }
                    return Some(Frame {
                        stage: stage.name().to_string(),
                        progress: 1.0,
                        label: "cached".to_string(),
                        points: None,
                        paths,
                        pen,
                    });
                }
                let ctx =
                    RunContext::new(self.cancel.clone()).in_lane(lane.unwrap_or(Lane::SINGLE));
                match stage.start(Inputs::new(inputs), &settings, ctx) {
                    Ok(run) => self.active = Some((run, key)),
                    Err(e) => {
                        self.fail(e);
                        return None;
                    }
                }
            }
            let (run, _) = self.active.as_mut().expect("a stage is active");
            match run.step() {
                Ok(Step::Frame(mut frame)) => {
                    frame.stage = stage.name().to_string();
                    frame.pen = pen;
                    return Some(frame);
                }
                Ok(Step::Done(artifact)) => {
                    let (_, key) = self.active.take().expect("a stage is active");
                    self.cache.lock().unwrap().put(&key, artifact.clone());
                    if let Err(e) = self.land(index, lane, artifact, key) {
                        self.fail(e);
                        return None;
                    }
                }
                Err(e) => {
                    self.fail(e);
                    return None;
                }
            }
        }
    }

    /// Ask the run to stop before its next stage step.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The flag [`Run::cancel`] sets, for a host that cancels from another thread while this run
    /// is busy in [`Run::next_frame`].
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Where the run is.
    pub fn state(&self) -> RunState {
        self.state
    }

    /// The failing stage's message, once the run has ended in [`RunState::Error`].
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// The points the run has placed: the last points of each lane summed, or the last points
    /// before any split; `None` when a lane, or a run without lanes, has none.
    pub fn point_count(&self) -> Option<usize> {
        let last = |done: &[(Artifact, String)]| {
            done.iter().rev().find_map(|(artifact, _)| match artifact {
                Artifact::Points(p) => Some(p.0.len()),
                _ => None,
            })
        };
        if self.lanes.is_empty() {
            last(&self.done)
        } else {
            self.lanes.iter().map(|lane| last(&lane.done)).sum()
        }
    }

    /// The next stage to run and its lane, or `None` once every stage has run. Up to and
    /// including the split, stages run once with no lane; after it, every remaining stage of lane
    /// 0, then of lane 1, and so on.
    fn next_stage(&self) -> Option<(usize, Option<Lane>)> {
        let count = self.style.stages.len();
        match self.style.split() {
            Some(split) if self.done.len() > split => {
                let per_lane = count - split - 1;
                let index = self.lanes.iter().position(|l| l.done.len() < per_lane)?;
                let lane = Lane {
                    index,
                    count: self.lanes.len(),
                };
                Some((split + 1 + self.lanes[index].done.len(), Some(lane)))
            }
            _ => (self.done.len() < count).then_some((self.done.len(), None)),
        }
    }

    /// File stage `index`'s output and key, in `lane` if it has one. The split's output opens one
    /// lane per tone map, each with the pen of the same index; it fails with no maps or more maps
    /// than pens.
    fn land(
        &mut self,
        index: usize,
        lane: Option<Lane>,
        artifact: Artifact,
        key: String,
    ) -> Result<(), StageError> {
        if let Some(l) = lane {
            self.lanes[l.index].done.push((artifact, key));
            return Ok(());
        }
        // A tone map with a response lookup goes on through it, under a key of its own.
        let (artifact, key) = match (&artifact, &self.response) {
            (Artifact::Tone(map), Some((lookup, lookup_key))) => {
                self.source = Some(map.clone());
                let ink = map
                    .ink
                    .iter()
                    .map(|&v| response::apply(lookup, v))
                    .collect();
                let corrected = ToneMap {
                    ink,
                    ..ToneMap::clone(map)
                };
                (
                    Artifact::Tone(Arc::new(corrected)),
                    response::corrected_key(&key, lookup_key),
                )
            }
            _ => (artifact, key),
        };
        if self.style.split() == Some(index) {
            let Artifact::Tones(maps) = &artifact else {
                panic!("the split stage gave {}, not tones", artifact.kind().name());
            };
            let (Artifact::Pens(pens), _) = &self.roots[&Kind::Pens] else {
                unreachable!("roots are filed by kind");
            };
            let (n, m) = (maps.len(), pens.len());
            if n == 0 || n > m {
                let name = self.style.stages[index].name();
                let s = if m == 1 { "" } else { "s" };
                let message = format!("stage {name:?} gave {n} tone maps for {m} pen{s}");
                return Err(StageError::Failed(message));
            }
            let open = |(map, pen): (&Arc<ToneMap>, &Pen)| {
                let pens = Artifact::Pens(Arc::new(vec![pen.clone()]));
                let pens_key = root_key(Kind::Pens, &pens);
                LaneState {
                    tone: (Artifact::Tone(map.clone()), tone_key(map)),
                    pens: (pens, pens_key),
                    done: Vec::new(),
                }
            };
            self.lanes = maps.iter().zip(pens.iter()).map(open).collect();
        }
        self.done.push((artifact, key));
        Ok(())
    }

    /// The drawing the finished stages make, with the tone map of the last stage before any split
    /// that provides one. With lanes, layer `i` is pen `i` with lane `i`'s last paths, empty or
    /// not. Without, it is the last stage's layers, or its paths on one layer of the first pen.
    fn drawing(&self) -> Result<Drawing, String> {
        let (Artifact::Canvas(canvas), _) = &self.roots[&Kind::Canvas] else {
            unreachable!("roots are filed by kind");
        };
        let (Artifact::Pens(pens), _) = &self.roots[&Kind::Pens] else {
            unreachable!("roots are filed by kind");
        };
        let layer = |artifact: &Artifact, pen: &Pen| match artifact {
            Artifact::Paths(paths) => Layer {
                pen: pen.clone(),
                paths: Paths::clone(paths),
            },
            other => panic!(
                "the last stage gave {}, not paths or layers",
                other.kind().name()
            ),
        };
        let layers = if self.lanes.is_empty() {
            match &self.done.last().expect("a style has stages").0 {
                Artifact::Layers(layers) => layers.to_vec(),
                last => vec![layer(
                    last,
                    pens.first().ok_or("a run needs at least one pen")?,
                )],
            }
        } else {
            (self.lanes.iter().zip(pens.iter()))
                .map(|(lane, pen)| layer(&lane.done.last().expect("a lane runs a stage").0, pen))
                .collect()
        };
        let tone = (self.style.stages.iter().zip(&self.done))
            .rev()
            .find(|(s, _)| s.provides() == Kind::Tone)
            .and_then(|(_, (artifact, _))| match artifact {
                Artifact::Tone(t) => Some(self.source.clone().unwrap_or_else(|| t.clone())),
                _ => None,
            });
        // The run's own snapshot, not the session's state, which may have moved on; it holds no
        // machine, so the session fills that in when it exports.
        let preset = presets::to_value(&self.style.name, canvas, pens, None, &self.settings);
        Ok(Drawing {
            canvas: Canvas::clone(canvas),
            layers,
            tone,
            preset: Some(preset),
        })
    }

    /// End the run on a stage error. A stage that saw the cancel flag ends it as cancelled.
    fn fail(&mut self, e: StageError) {
        match e {
            StageError::Cancelled => self.write(RunState::Cancelled, None, None),
            e => self.write(RunState::Error, Some(e.to_string()), None),
        }
    }

    /// Set the run's state and error and, while the slot is still this run's generation, copy
    /// them to it with the drawing if there is one, under one lock so readers never see a
    /// half-written slot.
    fn write(&mut self, state: RunState, error: Option<String>, drawing: Option<Drawing>) {
        let mut shared = self.shared.lock().unwrap();
        if shared.generation == self.generation {
            if let Some(drawing) = drawing {
                shared.drawing = Some(Arc::new(drawing));
            }
            shared.state = state;
            shared.error = error.clone();
        }
        self.state = state;
        self.error = error;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::StageCache;
    use crate::settings::{defaults_for, schema_for};
    use crate::stage::{Frame, Kind};
    use crate::style::Style;
    use crate::test_util::{
        black, drain, paths, points, roots, roots_with, tone, tone_map, tones, Spy, SpySettings,
    };
    use crate::types::Pen;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// A lane stage named "e" that reports one frame labelled "echo", then gives one path: its
    /// map's first ink and its pen's width, then its lane's index and count.
    struct Echo;

    impl Stage for Echo {
        fn name(&self) -> &'static str {
            "e"
        }
        fn label(&self) -> &'static str {
            "Echo"
        }
        fn version(&self) -> u32 {
            1
        }
        fn requires(&self) -> &'static [Kind] {
            &[Kind::Tone, Kind::Pens]
        }
        fn provides(&self) -> Kind {
            Kind::Paths
        }
        fn schema(&self) -> Value {
            schema_for::<SpySettings>()
        }
        fn defaults(&self) -> Value {
            defaults_for::<SpySettings>()
        }
        fn start(
            &self,
            inputs: Inputs,
            _settings: &Value,
            ctx: RunContext,
        ) -> Result<Box<dyn StageRun>, StageError> {
            let lane = ctx.lane();
            let mut paths = Paths::new();
            paths.push(&[
                [inputs.tone().ink[0] as f64, inputs.pens()[0].width_mm],
                [lane.index as f64, lane.count as f64],
            ]);
            Ok(Box::new(EchoRun {
                paths,
                framed: false,
            }))
        }
    }

    struct EchoRun {
        paths: Paths,
        /// Whether the frame has been reported.
        framed: bool,
    }

    impl StageRun for EchoRun {
        fn step(&mut self) -> Result<Step, StageError> {
            if std::mem::replace(&mut self.framed, true) {
                return Ok(Step::Done(Artifact::Paths(Arc::new(self.paths.clone()))));
            }
            Ok(Step::Frame(Frame {
                stage: String::new(),
                progress: 0.5,
                label: "echo".to_string(),
                points: None,
                paths: None,
                pen: 0,
            }))
        }
    }

    fn two_pens() -> Vec<Pen> {
        vec![
            Pen::new("A", 0.3, "#000000").unwrap(),
            Pen::new("B", 0.5, "#ff0000").unwrap(),
        ]
    }

    /// Drain a run of `style` with `settings` over [`roots_with`] `pens` on `cache`; return the
    /// run, its frames and the drawing it left in a fresh slot, if any.
    fn run_pens(
        style: &Arc<Style>,
        settings: &Value,
        pens: Vec<Pen>,
        cache: &Arc<Mutex<StageCache>>,
    ) -> (Run, Vec<Frame>, Option<Arc<Drawing>>) {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut run = Run::new(
            style.clone(),
            settings.clone(),
            roots_with(pens),
            cache.clone(),
            shared.clone(),
            Arc::default(),
            0,
        );
        let frames = drain(&mut run);
        let drawing = shared.lock().unwrap().drawing.clone();
        (run, frames, drawing)
    }

    /// Each frame's stage, pen and label.
    fn seen(frames: &[Frame]) -> Vec<(&str, usize, &str)> {
        (frames.iter())
            .map(|f| (f.stage.as_str(), f.pen, f.label.as_str()))
            .collect()
    }

    #[test]
    fn a_split_runs_the_rest_once_per_pen_and_makes_a_layer_each() {
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let s = Arc::new(Spy::new(
            "s",
            &[Kind::Tone, Kind::Pens],
            Kind::Tones,
            tones(&[0.25, 0.75]),
        ));
        let style =
            Arc::new(Style::new("x", "X", "d", vec![t.clone(), s, Arc::new(Echo)]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        let (run, frames, d) = run_pens(&style, &style.defaults(), two_pens(), &cache);
        assert_eq!(run.state(), RunState::Done);
        assert_eq!(
            seen(&frames),
            [
                ("t", 0, "1/1"),
                ("s", 0, "1/1"),
                ("e", 0, "echo"),
                ("e", 1, "echo")
            ]
        );
        let d = d.unwrap();
        let pens: Vec<_> = d.layers.iter().map(|l| l.pen.name.as_str()).collect();
        assert_eq!(pens, ["A", "B"]);
        assert_eq!(d.layers[0].paths.coords(), [[0.25, 0.3], [0.0, 2.0]]);
        assert_eq!(d.layers[1].paths.coords(), [[0.75, 0.5], [1.0, 2.0]]);
        // The tone map is t's, from before the split.
        assert_eq!(d.tone.as_ref().unwrap().ink, [0.5]);
        // Lanes with equal maps and equal pens are still keyed apart by their lane.
        let same = Arc::new(Spy::new(
            "s",
            &[Kind::Tone],
            Kind::Tones,
            tones(&[0.5, 0.5]),
        ));
        let style = Arc::new(Style::new("x", "X", "d", vec![t, same, Arc::new(Echo)]).unwrap());
        let a = two_pens()[0].clone();
        let (_, frames, d) = run_pens(&style, &style.defaults(), vec![a.clone(), a], &cache);
        assert_eq!(seen(&frames)[3], ("e", 1, "echo"));
        assert_eq!(
            d.unwrap().layers[1].paths.coords(),
            [[0.5, 0.3], [1.0, 2.0]]
        );
    }

    #[test]
    fn lanes_are_cached_by_their_map_and_their_pen() {
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let s = Arc::new(Spy::new(
            "s",
            &[Kind::Tone, Kind::Pens],
            Kind::Tones,
            tones(&[0.25, 0.75]),
        ));
        let x = Arc::new(Spy::new("x", &[Kind::Tone], Kind::Paths, paths(0)));
        let style =
            Arc::new(Style::new("st", "S", "d", vec![t.clone(), s.clone(), x.clone()]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        let mut settings = style.defaults();
        let (_, _, d) = run_pens(&style, &settings, two_pens(), &cache);
        assert_eq!(x.runs(), 2);
        let sizes: Vec<_> = d.unwrap().layers.iter().map(|l| l.paths.len()).collect();
        assert_eq!(sizes, [0, 0]);
        // The same roots and settings again are all cached.
        let (run, frames, _) = run_pens(&style, &settings, two_pens(), &cache);
        assert_eq!(run.state(), RunState::Done);
        assert_eq!(x.runs(), 2);
        assert_eq!(
            seen(&frames),
            [
                ("t", 0, "cached"),
                ("s", 0, "cached"),
                ("x", 0, "cached"),
                ("x", 1, "cached")
            ]
        );
        // The split reruns on new settings but gives the same maps, so no lane reruns.
        settings["s"]["knob"] = json!(1);
        run_pens(&style, &settings, two_pens(), &cache);
        assert_eq!((s.runs(), x.runs()), (2, 2));
        // A new colour for pen B reruns the split, which reads the pens, but x reads only its map.
        let blue = vec![
            two_pens()[0].clone(),
            Pen::new("B", 0.5, "#0000ff").unwrap(),
        ];
        let (_, _, d) = run_pens(&style, &settings, blue.clone(), &cache);
        assert_eq!((s.runs(), x.runs()), (3, 2));
        assert_eq!(d.unwrap().layers[1].pen.color, "#0000ff");
        // A lane stage that reads its pen reruns only in the lane whose pen changed.
        let y = Arc::new(Spy::new(
            "y",
            &[Kind::Tone, Kind::Pens],
            Kind::Paths,
            paths(1),
        ));
        let style = Arc::new(Style::new("st", "S", "d", vec![t, s, y.clone()]).unwrap());
        run_pens(&style, &settings, two_pens(), &cache);
        assert_eq!(y.runs(), 2);
        let (_, frames, d) = run_pens(&style, &settings, blue, &cache);
        assert_eq!(y.runs(), 3);
        assert_eq!(seen(&frames)[2..], [("y", 0, "cached"), ("y", 1, "1/1")]);
        assert_eq!(d.unwrap().layers[1].paths.len(), 1);
    }

    #[test]
    fn the_last_stage_of_a_cached_lane_still_shows_its_paths() {
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let s = Arc::new(Spy::new(
            "s",
            &[Kind::Tone, Kind::Pens],
            Kind::Tones,
            tones(&[0.25, 0.75]),
        ));
        let m = Arc::new(Spy::new("m", &[Kind::Tone], Kind::Points, points(2)));
        let style = Arc::new(Style::new("x", "X", "d", vec![t, s, m, Arc::new(Echo)]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        run_pens(&style, &style.defaults(), two_pens(), &cache);
        // Everything is cached on the second run; only the last stage's frames carry geometry.
        let (_, frames, _) = run_pens(&style, &style.defaults(), two_pens(), &cache);
        assert_eq!(
            seen(&frames),
            [
                ("t", 0, "cached"),
                ("s", 0, "cached"),
                ("m", 0, "cached"),
                ("e", 0, "cached"),
                ("m", 1, "cached"),
                ("e", 1, "cached")
            ]
        );
        for f in frames.iter().filter(|f| f.stage != "e") {
            assert!(f.points.is_none() && f.paths.is_none(), "{}", f.stage);
        }
        assert_eq!(
            frames[3].paths.as_ref().unwrap().coords(),
            [[0.25, 0.3], [0.0, 2.0]]
        );
        assert_eq!(
            frames[5].paths.as_ref().unwrap().coords(),
            [[0.75, 0.5], [1.0, 2.0]]
        );
    }

    #[test]
    fn more_maps_than_pens_ends_the_run_in_error() {
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let x = Arc::new(Spy::new("x", &[Kind::Tone], Kind::Paths, paths(0)));
        let cases: [(&[f32], &str); 2] = [
            (&[0.1, 0.2], r#"stage "s" gave 2 tone maps for 1 pen"#),
            (&[], r#"stage "s" gave 0 tone maps for 1 pen"#),
        ];
        for (inks, error) in cases {
            let s = Arc::new(Spy::new("s", &[Kind::Tone], Kind::Tones, tones(inks)));
            let style =
                Arc::new(Style::new("st", "S", "d", vec![t.clone(), s, x.clone()]).unwrap());
            let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
            // The second run takes the split from the cache and fails the same way.
            for _ in 0..2 {
                let (run, _, d) = run_pens(&style, &style.defaults(), vec![black()], &cache);
                assert_eq!(run.state(), RunState::Error);
                assert_eq!(run.error().as_deref(), Some(error));
                assert!(d.is_none());
            }
        }
        assert_eq!(x.runs(), 0);
    }

    #[test]
    fn cancel_between_lanes_ends_cancelled_and_keeps_the_split() {
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let s = Arc::new(Spy::new(
            "s",
            &[Kind::Tone],
            Kind::Tones,
            tones(&[0.25, 0.75]),
        ));
        let x = Arc::new(Spy::new("x", &[Kind::Tone], Kind::Paths, paths(1)).with_frames(3));
        let style = Arc::new(Style::new("st", "S", "d", vec![t, s, x.clone()]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        // Run until `stop` holds for a frame, cancel there, and return the frames.
        let cancel_at = |stop: &dyn Fn(&Frame) -> bool| {
            let shared = Arc::new(Mutex::new(Shared::default()));
            let mut run = Run::new(
                style.clone(),
                style.defaults(),
                roots_with(two_pens()),
                cache.clone(),
                shared.clone(),
                Arc::default(),
                0,
            );
            let mut frames = Vec::new();
            while let Some(f) = run.next_frame() {
                if stop(&f) {
                    run.cancel();
                }
                frames.push(f);
            }
            assert_eq!(run.state(), RunState::Cancelled);
            let slot = shared.lock().unwrap();
            assert_eq!(slot.state, RunState::Cancelled);
            assert!(slot.drawing.is_none());
            frames
        };
        let frames = cancel_at(&|f| f.label == "1/3");
        assert_eq!(
            seen(&frames),
            [("t", 0, "1/1"), ("s", 0, "1/1"), ("x", 0, "1/3")]
        );
        assert_eq!(x.runs(), 1);
        assert_eq!(cache.lock().unwrap().len(), 2); // t and s

        // A cancel in lane 1 keeps the split and lane 0.
        let frames = cancel_at(&|f| f.pen == 1);
        assert_eq!(
            seen(&frames),
            [
                ("t", 0, "cached"),
                ("s", 0, "cached"),
                ("x", 0, "1/3"),
                ("x", 0, "2/3"),
                ("x", 0, "3/3"),
                ("x", 1, "1/3")
            ]
        );
        assert_eq!(x.runs(), 3);
        assert_eq!(cache.lock().unwrap().len(), 3); // t, s and lane 0's x

        // A fresh run reruns only lane 1.
        let (run, frames, d) = run_pens(&style, &style.defaults(), two_pens(), &cache);
        assert_eq!(run.state(), RunState::Done);
        assert_eq!(
            seen(&frames),
            [
                ("t", 0, "cached"),
                ("s", 0, "cached"),
                ("x", 0, "cached"),
                ("x", 1, "1/3"),
                ("x", 1, "2/3"),
                ("x", 1, "3/3")
            ]
        );
        assert_eq!(x.runs(), 4);
        assert_eq!(d.unwrap().layers.len(), 2);
    }

    #[test]
    fn tone_key_follows_the_content() {
        let ink = |x: u32, y: u32| (x + 2 * y) as f32 / 4.0;
        let map = tone_map(2, 2, ink);
        let key = tone_key(&map);
        assert_eq!(key.len(), 64);
        assert_eq!(key, tone_key(&tone_map(2, 2, ink)));
        let changed = |change: &dyn Fn(&mut ToneMap)| {
            let mut m = map.clone();
            change(&mut m);
            tone_key(&m)
        };
        assert_ne!(key, changed(&|m| m.ink[3] = 0.5));
        assert_ne!(key, changed(&|m| m.mm_per_px = 3.0));
        assert_ne!(key, changed(&|m| m.origin_mm[0] = 11.0));
        assert_ne!(key, changed(&|m| m.origin_mm[1] = 11.0));
    }

    #[test]
    fn runs_in_order_fills_stage_names_and_builds_the_drawing() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Points, points(2)).with_frames(3));
        let b = Arc::new(Spy::new("b", &[Kind::Points], Kind::Paths, paths(2)));
        let style = Arc::new(Style::new("s", "S", "d", vec![a.clone(), b.clone()]).unwrap());
        let (cache, shared) = (
            Arc::new(Mutex::new(StageCache::new(1 << 20))),
            Arc::new(Mutex::new(Shared::default())),
        );
        let mut run = Run::new(
            style.clone(),
            style.defaults(),
            roots(),
            cache.clone(),
            shared.clone(),
            Arc::default(),
            0,
        );
        assert_eq!(run.state(), RunState::Idle);
        let frames = drain(&mut run);
        assert_eq!(
            frames.iter().map(|f| f.stage.as_str()).collect::<Vec<_>>(),
            ["a", "a", "a", "b"]
        );
        assert_eq!(run.state(), RunState::Done);
        assert!(run.next_frame().is_none());
        let slot = shared.lock().unwrap();
        assert_eq!(slot.state, RunState::Done);
        let d = slot.drawing.clone().unwrap();
        assert_eq!(d.layers.len(), 1);
        assert_eq!(d.layers[0].paths.len(), 2);
        assert_eq!(d.layers[0].pen.name, "Black 0.3");
        assert!(d.tone.is_none());
    }

    #[test]
    fn cached_stages_are_skipped_and_only_changed_stages_rerun() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Points, points(2)).with_frames(3));
        let b = Arc::new(Spy::new("b", &[Kind::Points], Kind::Paths, paths(2)));
        let c = Arc::new(Spy::new("c", &[Kind::Paths], Kind::Paths, paths(1)));
        let style =
            Arc::new(Style::new("s", "S", "d", vec![a.clone(), b.clone(), c.clone()]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let run_all = |style: &Arc<Style>, settings: &Value| -> Vec<Frame> {
            let mut run = Run::new(
                style.clone(),
                settings.clone(),
                roots(),
                cache.clone(),
                shared.clone(),
                Arc::default(),
                0,
            );
            let frames = drain(&mut run);
            assert_eq!(run.state(), RunState::Done);
            frames
        };
        let mut settings = style.defaults();
        run_all(&style, &settings);
        assert_eq!((a.runs(), b.runs(), c.runs()), (1, 1, 1));
        settings["c"]["knob"] = json!(1);
        run_all(&style, &settings);
        assert_eq!((a.runs(), b.runs(), c.runs()), (1, 1, 2));
        settings["a"]["knob"] = json!(1);
        run_all(&style, &settings);
        assert_eq!((a.runs(), b.runs(), c.runs()), (2, 2, 3));
        let frames = run_all(&style, &settings);
        assert_eq!((a.runs(), b.runs(), c.runs()), (2, 2, 3));
        assert_eq!(frames.len(), 3);
        assert!(frames
            .iter()
            .all(|f| f.label == "cached" && f.progress == 1.0));
        assert_eq!(
            frames.iter().map(|f| f.stage.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        // A missing stage entry means its defaults, which the first run already cached.
        let frames = run_all(&style, &json!({}));
        assert_eq!((a.runs(), b.runs(), c.runs()), (2, 2, 3));
        assert!(frames.iter().all(|f| f.label == "cached"));
    }

    #[test]
    fn cancel_stops_at_the_next_step_and_keeps_earlier_stages() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Points, points(1)));
        let b = Arc::new(Spy::new("b", &[Kind::Points], Kind::Paths, paths(1)).with_frames(100));
        let style = Arc::new(Style::new("s", "S", "d", vec![a.clone(), b.clone()]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut run = Run::new(
            style.clone(),
            style.defaults(),
            roots(),
            cache.clone(),
            shared.clone(),
            cancel.clone(),
            0,
        );
        let mut n = 0;
        while run.next_frame().is_some() {
            n += 1;
            if n == 3 {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        assert_eq!(n, 3);
        assert_eq!(run.state(), RunState::Cancelled);
        assert!(run.next_frame().is_none());
        assert_eq!(cache.lock().unwrap().len(), 1); // only a
        assert!(shared.lock().unwrap().drawing.is_none());
        assert_eq!(shared.lock().unwrap().state, RunState::Cancelled);
        // A fresh run takes a from the cache and runs b again.
        let mut again = Run::new(
            style.clone(),
            style.defaults(),
            roots(),
            cache.clone(),
            shared.clone(),
            Arc::default(),
            0,
        );
        let frames = drain(&mut again);
        assert_eq!(
            (frames[0].stage.as_str(), frames[0].label.as_str()),
            ("a", "cached")
        );
        assert_eq!((a.runs(), b.runs()), (1, 2));
        let before = shared.lock().unwrap().drawing.clone().unwrap();
        // A cancel after the last stage's cached frame still ends the run cancelled.
        let mut third = Run::new(
            style.clone(),
            style.defaults(),
            roots(),
            cache.clone(),
            shared.clone(),
            Arc::default(),
            0,
        );
        assert_eq!(third.next_frame().unwrap().label, "cached");
        let last = third.next_frame().unwrap();
        assert_eq!((last.stage.as_str(), last.label.as_str()), ("b", "cached"));
        third.cancel();
        assert!(third.next_frame().is_none());
        assert_eq!(third.state(), RunState::Cancelled);
        let after = shared.lock().unwrap().drawing.clone().unwrap();
        assert!(Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn a_stage_that_sees_the_cancel_flag_ends_the_run_cancelled() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Points, points(1)));
        let b = Arc::new(
            Spy::new("b", &[Kind::Points], Kind::Paths, paths(1)).with_cooperative_cancel(),
        );
        let style = Arc::new(Style::new("s", "S", "d", vec![a, b.clone()]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut run = Run::new(
            style.clone(),
            style.defaults(),
            roots(),
            cache.clone(),
            shared.clone(),
            cancel.clone(),
            0,
        );
        let worker = std::thread::spawn(move || {
            let frames = drain(&mut run);
            (run, frames)
        });
        // b starts after the runner's own flag check, so only b's step can see this cancel.
        while b.runs() == 0 && !worker.is_finished() {
            std::thread::yield_now();
        }
        cancel.store(true, Ordering::Relaxed);
        let (run, frames) = worker.join().unwrap();
        assert_eq!(frames.len(), 1); // a's frame
        assert_eq!(b.runs(), 1);
        assert_eq!(run.state(), RunState::Cancelled);
        assert_eq!(run.error(), None);
        let slot = shared.lock().unwrap();
        assert_eq!(
            (slot.state, slot.error.clone()),
            (RunState::Cancelled, None)
        );
        assert!(slot.drawing.is_none());
        assert_eq!(cache.lock().unwrap().len(), 1); // only a
    }

    #[test]
    fn a_run_of_an_older_generation_never_writes_the_slot() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Paths, paths(1)).with_frames(3));
        let style = Arc::new(Style::new("s", "S", "d", vec![a.clone()]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        let shared = Arc::new(Mutex::new(Shared {
            generation: 1,
            ..Shared::default()
        }));
        let run = |generation| {
            Run::new(
                style.clone(),
                style.defaults(),
                roots(),
                cache.clone(),
                shared.clone(),
                Arc::default(),
                generation,
            )
        };
        let mut old = run(1);
        assert!(old.next_frame().is_some());
        shared.lock().unwrap().generation = 2;
        let mut new = run(2);
        assert!(new.next_frame().is_some());
        // The older run is cancelled and ends while the newer one is running.
        old.cancel();
        assert!(old.next_frame().is_none());
        assert_eq!(old.state(), RunState::Cancelled);
        assert_eq!(shared.lock().unwrap().state, RunState::Running);
        drain(&mut new);
        let drawing = shared.lock().unwrap().drawing.clone().unwrap();
        // An older run that finishes after the newer one, with a drawing or an error, changes
        // nothing either.
        let mut late = run(1);
        drain(&mut late);
        assert_eq!(late.state(), RunState::Done);
        let failing =
            Arc::new(Spy::new("f", &[Kind::Image], Kind::Paths, paths(1)).with_fail_after(1));
        let failing = Arc::new(Style::new("s", "S", "d", vec![failing]).unwrap());
        let mut bad = Run::new(
            failing.clone(),
            failing.defaults(),
            roots(),
            cache.clone(),
            shared.clone(),
            Arc::default(),
            1,
        );
        drain(&mut bad);
        assert_eq!(bad.state(), RunState::Error);
        let slot = shared.lock().unwrap();
        assert_eq!((slot.state, slot.error.clone()), (RunState::Done, None));
        assert!(Arc::ptr_eq(slot.drawing.as_ref().unwrap(), &drawing));
    }

    #[test]
    fn cancel_method_sets_the_shared_flag() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Paths, paths(1)).with_frames(5));
        let style = Arc::new(Style::new("s", "S", "d", vec![a.clone()]).unwrap());
        let cancel = Arc::new(AtomicBool::new(false));
        let mut run = Run::new(
            style.clone(),
            style.defaults(),
            roots(),
            Arc::new(Mutex::new(StageCache::new(1 << 20))),
            Arc::new(Mutex::new(Shared::default())),
            cancel.clone(),
            0,
        );
        assert!(run.next_frame().is_some());
        assert!(Arc::ptr_eq(&run.cancel_flag(), &cancel));
        run.cancel();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(run.next_frame().is_none());
        assert_eq!(run.state(), RunState::Cancelled);
    }

    #[test]
    fn a_failing_stage_ends_in_error_with_its_message() {
        let a = Arc::new(Spy::new("a", &[Kind::Image], Kind::Paths, paths(1)).with_fail_after(1));
        let style = Arc::new(Style::new("s", "S", "d", vec![a.clone()]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut run = Run::new(
            style.clone(),
            style.defaults(),
            roots(),
            cache.clone(),
            shared.clone(),
            Arc::default(),
            0,
        );
        assert!(drain(&mut run).is_empty());
        assert_eq!(run.state(), RunState::Error);
        assert_eq!(run.error().unwrap(), "spy failed");
        {
            let slot = shared.lock().unwrap();
            assert_eq!(slot.state, RunState::Error);
            assert_eq!(slot.error.as_deref(), Some("spy failed"));
            assert!(slot.drawing.is_none());
        }
        assert!(cache.lock().unwrap().is_empty());
        // Settings the stage rejects end the run the same way.
        let mut run = Run::new(
            style.clone(),
            json!({"a": {"nob": 1}}),
            roots(),
            cache,
            shared,
            Arc::default(),
            0,
        );
        assert!(drain(&mut run).is_empty());
        assert_eq!(run.state(), RunState::Error);
        assert!(run.error().unwrap().contains("nob"));
    }

    #[test]
    fn layers_pass_through_and_tone_comes_from_the_tone_stage_even_when_cached() {
        let pen = Pen::new("Red 0.5", 0.5, "#ff0000").unwrap();
        let layer = Layer {
            pen,
            paths: Paths::new(),
        };
        let layers = Artifact::Layers(Arc::new(vec![layer.clone(), layer]));
        let t = Arc::new(Spy::new("t", &[Kind::Image], Kind::Tone, tone()));
        let l = Arc::new(Spy::new("l", &[Kind::Tone], Kind::Layers, layers));
        let style = Arc::new(Style::new("s", "S", "d", vec![t, l]).unwrap());
        let cache = Arc::new(Mutex::new(StageCache::new(1 << 20)));
        for _ in 0..2 {
            let shared = Arc::new(Mutex::new(Shared::default()));
            let mut run = Run::new(
                style.clone(),
                style.defaults(),
                roots(),
                cache.clone(),
                shared.clone(),
                Arc::default(),
                0,
            );
            drain(&mut run);
            let d = shared.lock().unwrap().drawing.clone().unwrap();
            assert_eq!(d.layers.len(), 2);
            assert_eq!(d.layers[0].pen.name, "Red 0.5");
            assert_eq!(d.tone.as_ref().unwrap().ink, [0.5]);
            assert_eq!(d.canvas.width_mm, 100.0);
        }
        assert_eq!(cache.lock().unwrap().stats().hits, 2);
    }

    #[test]
    fn keys_change_with_settings_version_and_inputs_but_not_whitespace() {
        assert_eq!(
            canonical_json(&json!({"b": 1, "a": [1.5, true]})),
            r#"{"a":[1.5,true],"b":1}"#
        );
        let s = Spy::new("a", &[Kind::Image], Kind::Points, points(1));
        let k1 = stage_key(&s, &json!({"knob": 1}), &BTreeMap::from([("image", "x")]));
        assert_ne!(
            k1,
            stage_key(&s, &json!({"knob": 2}), &BTreeMap::from([("image", "x")]))
        );
        assert_ne!(
            k1,
            stage_key(&s, &json!({"knob": 1}), &BTreeMap::from([("image", "y")]))
        );
        assert_eq!(k1.len(), 64);
        let [image, canvas, pens] = roots();
        let key = root_key(Kind::Image, &image);
        assert_eq!(key.len(), 64);
        let Artifact::Image(i) = &image else {
            panic!("roots start with the image");
        };
        let mut darker = crate::types::Image::clone(i);
        darker.rgb[0] = 0.25;
        assert_ne!(
            key,
            root_key(Kind::Image, &Artifact::Image(Arc::new(darker)))
        );
        assert_ne!(root_key(Kind::Canvas, &canvas), root_key(Kind::Pens, &pens));
    }
}
