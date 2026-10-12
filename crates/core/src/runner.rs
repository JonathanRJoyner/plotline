//! The runner: runs a style's stages in order, one frame per call, reusing cached outputs.
//!
//! A stage's output is cached under a key hashed from the stage's name, version and settings and
//! the keys of its inputs, so a change to one stage's settings reruns that stage and the stages
//! after it, and nothing before it. The roots are keyed by their content. Hashes are BLAKE3
//! (O'Connor, Aumasson, Neves and Wilcox-O'Hearn, "BLAKE3: one function, fast everywhere", 2020).
//!
//! [`Run`] is driven by its host one [`Run::next_frame`] call at a time, so a host can cancel or
//! drop it between frames. The cache lock is held only while reading or storing one artifact.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::cache::StageCache;
use crate::settings::merge;
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageRun, Step};
use crate::style::{Source, Style};
use crate::types::{Canvas, Drawing, Layer, Paths};

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
    /// The output and key of each finished stage, in style order.
    done: Vec<(Artifact, String)>,
    /// The stage in progress and its key.
    active: Option<(Box<dyn StageRun>, String)>,
    state: RunState,
    error: Option<String>,
}

impl Run {
    /// A run that has not started. `roots` holds one image, one canvas and one pens artifact in
    /// any order; `settings` maps stage names to settings merged over each stage's defaults, and a
    /// missing stage means its defaults. Hashes the roots. Panics if a root kind is missing.
    pub fn new(
        style: Arc<Style>,
        settings: Value,
        roots: [Artifact; 3],
        cache: Arc<Mutex<StageCache>>,
        shared: Arc<Mutex<Shared>>,
        cancel: Arc<AtomicBool>,
    ) -> Run {
        let roots: HashMap<Kind, (Artifact, String)> = roots
            .into_iter()
            .map(|a| (a.kind(), (a.clone(), root_key(a.kind(), &a))))
            .collect();
        assert!(
            Kind::ROOTS.iter().all(|k| roots.contains_key(k)),
            "a run needs one image, one canvas and one pens root"
        );
        Run {
            style,
            settings,
            roots,
            cache,
            shared,
            cancel,
            done: Vec::new(),
            active: None,
            state: RunState::Idle,
            error: None,
        }
    }

    /// Do work until there is a frame to show, and return it. Returns `None` once the run has
    /// ended, after writing the end state, the drawing on success, and the message on failure to
    /// the shared slot. The cancel flag is checked before every stage step.
    pub fn next_frame(&mut self) -> Option<Frame> {
        match self.state {
            RunState::Idle => self.write(RunState::Running, None, None),
            RunState::Running => {}
            RunState::Done | RunState::Cancelled | RunState::Error => return None,
        }
        loop {
            let index = self.done.len();
            let Some(stage) = self.style.stages.get(index).cloned() else {
                match self.drawing() {
                    Ok(drawing) => self.write(RunState::Done, None, Some(drawing)),
                    Err(e) => self.write(RunState::Error, Some(e), None),
                }
                return None;
            };
            if self.cancel.load(Ordering::Relaxed) {
                self.write(RunState::Cancelled, None, None);
                return None;
            }
            if self.active.is_none() {
                let defaults = stage.defaults();
                let settings = match self.settings.get(stage.name()) {
                    Some(s) => merge(&defaults, s),
                    None => defaults,
                };
                let mut inputs = HashMap::new();
                let mut keys = BTreeMap::new();
                for &(kind, source) in self.style.sources(index) {
                    let (artifact, key) = match source {
                        Source::Root(k) => &self.roots[&k],
                        Source::Stage(i) => &self.done[i],
                    };
                    inputs.insert(kind, artifact.clone());
                    keys.insert(kind.name(), key.as_str());
                }
                let key = stage_key(&*stage, &settings, &keys);
                let cached = self.cache.lock().unwrap().get(&key);
                if let Some(artifact) = cached {
                    self.done.push((artifact, key));
                    return Some(Frame {
                        stage: stage.name().to_string(),
                        progress: 1.0,
                        label: "cached".to_string(),
                        points: None,
                        paths: None,
                    });
                }
                let ctx = RunContext::new(self.cancel.clone());
                match stage.start(Inputs::new(inputs), &settings, ctx) {
                    Ok(run) => self.active = Some((run, key)),
                    Err(e) => {
                        self.write(RunState::Error, Some(e.to_string()), None);
                        return None;
                    }
                }
            }
            let (run, _) = self.active.as_mut().expect("a stage is active");
            match run.step() {
                Ok(Step::Frame(mut frame)) => {
                    frame.stage = stage.name().to_string();
                    return Some(frame);
                }
                Ok(Step::Done(artifact)) => {
                    let (_, key) = self.active.take().expect("a stage is active");
                    self.cache.lock().unwrap().put(&key, artifact.clone());
                    self.done.push((artifact, key));
                }
                Err(e) => {
                    self.write(RunState::Error, Some(e.to_string()), None);
                    return None;
                }
            }
        }
    }

    /// Ask the run to stop before its next stage step.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Where the run is.
    pub fn state(&self) -> RunState {
        self.state
    }

    /// The failing stage's message, once the run has ended in [`RunState::Error`].
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// The drawing the finished stages make: the last stage's layers, or its paths on one layer
    /// of the first pen, with the tone map of the last stage that provides one.
    fn drawing(&self) -> Result<Drawing, String> {
        let (Artifact::Canvas(canvas), _) = &self.roots[&Kind::Canvas] else {
            unreachable!("roots are filed by kind");
        };
        let (Artifact::Pens(pens), _) = &self.roots[&Kind::Pens] else {
            unreachable!("roots are filed by kind");
        };
        let layers = match &self.done.last().expect("a style has stages").0 {
            Artifact::Layers(layers) => layers.to_vec(),
            Artifact::Paths(paths) => {
                let pen = pens.first().ok_or("a run needs at least one pen")?;
                vec![Layer {
                    pen: pen.clone(),
                    paths: Paths::clone(paths),
                }]
            }
            other => panic!(
                "the last stage gave {}, not paths or layers",
                other.kind().name()
            ),
        };
        let tone = (self.style.stages.iter().zip(&self.done))
            .rev()
            .find(|(s, _)| s.provides() == Kind::Tone)
            .and_then(|(_, (artifact, _))| match artifact {
                Artifact::Tone(t) => Some(t.clone()),
                _ => None,
            });
        Ok(Drawing {
            canvas: Canvas::clone(canvas),
            layers,
            tone,
        })
    }

    /// Set the run's state and error and copy them to the shared slot, with the drawing if there
    /// is one, under one lock so readers never see a half-written slot.
    fn write(&mut self, state: RunState, error: Option<String>, drawing: Option<Drawing>) {
        let mut shared = self.shared.lock().unwrap();
        if let Some(drawing) = drawing {
            shared.drawing = Some(Arc::new(drawing));
        }
        shared.state = state;
        shared.error = error.clone();
        self.state = state;
        self.error = error;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::StageCache;
    use crate::stage::{Frame, Kind};
    use crate::style::Style;
    use crate::test_util::{drain, paths, points, roots, tone, Spy};
    use crate::types::Pen;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

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
        );
        let frames = drain(&mut again);
        assert_eq!(
            (frames[0].stage.as_str(), frames[0].label.as_str()),
            ("a", "cached")
        );
        assert_eq!((a.runs(), b.runs()), (1, 2));
        assert!(shared.lock().unwrap().drawing.is_some());
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
        );
        assert!(run.next_frame().is_some());
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
