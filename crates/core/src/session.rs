//! The session: one working context that holds the state a front end edits and the latest result.
//!
//! A [`Session`] holds the source image, the canvas, the pens, the chosen style with the settings
//! of each of its stages, the plot speeds, the stage cache its runs share, and the result slot
//! its runs write: the latest drawing, the run state and the error. [`Session::start`] hands back
//! a [`Run`] for the host to drive a frame at a time, and the outputs are read from the slot.
//!
//! The session holds no threads, files or network code, so it runs the same natively and in
//! WebAssembly. Hosts add those: a host drives the [`Run`] on a worker thread or in a loop of its
//! own, and reads and writes files itself, passing the session bytes and taking text and bytes
//! back. The CLI, the GUI and the MCP server call this layer and nothing below it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use thiserror::Error;

use crate::cache::{CacheStats, StageCache};
use crate::export::gcode::{self, GcodeError};
use crate::export::{json, svg};
use crate::image::{self, ImageError};
use crate::machine::{Machine, MachineError};
use crate::presets::{self, PresetError};
use crate::preview::{self, PreviewError};
use crate::registry::get_style;
use crate::runner::{Run, RunState, Shared};
use crate::settings::{merge, validate, SettingsError};
use crate::stage::Artifact;
use crate::stats::{self, Speeds, Stats};
use crate::style::{Style, StyleError};
use crate::types::{Canvas, Drawing, Fit, Image, Pen, TypeError};

/// Why a session refused a change or could not start a run; the message is shown to users.
#[derive(Debug, Error)]
pub enum SessionError {
    /// A style was rejected.
    #[error(transparent)]
    Style(#[from] StyleError),
    /// A stage refused its settings.
    #[error(transparent)]
    Settings(#[from] SettingsError),
    /// A preset could not be read.
    #[error(transparent)]
    Preset(#[from] PresetError),
    /// A canvas or pen was not well formed.
    #[error(transparent)]
    Type(#[from] TypeError),
    /// An image could not be decoded.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// A machine was not well formed.
    #[error("machine: {0}")]
    Machine(#[from] MachineError),
    /// A layer could not be written as G-code.
    #[error(transparent)]
    Gcode(#[from] GcodeError),
    /// G-code was asked for with no machine set.
    #[error("no machine is set")]
    NoMachine,
    /// An export was asked for before any run finished.
    #[error("no drawing has been made yet")]
    NoDrawing,
    /// No style has this name.
    #[error("unknown style {0:?}")]
    UnknownStyle(String),
    /// The style has no stage of this name; `stages` lists the ones it has.
    #[error("style {style:?} has no stage {stage:?}; its stages are {stages}")]
    UnknownStage {
        style: String,
        stage: String,
        stages: String,
    },
    /// The pens were set to an empty list.
    #[error("a session needs at least one pen")]
    NoPens,
    /// A run was started before an image was set.
    #[error("no image is loaded")]
    NoImage,
}

/// One working context: an image, a canvas, pens, a style with its settings, the plot speeds, the
/// stage cache and the latest result.
///
/// The setters check their input and change nothing when they fail. At most one run writes the
/// result slot: [`Session::start`] cancels the run before it, and a run that ends after a newer
/// one started leaves the slot alone.
pub struct Session {
    image: Option<Arc<Image>>,
    canvas: Canvas,
    pens: Vec<Pen>,
    /// The machine the G-code export writes for, if one is set.
    machine: Option<Machine>,
    style: Arc<Style>,
    /// Every stage's full settings object, keyed by stage name.
    settings: Value,
    speeds: Speeds,
    cache: Arc<Mutex<StageCache>>,
    shared: Arc<Mutex<Shared>>,
    /// The cancel flag of the latest run.
    cancel: Arc<AtomicBool>,
}

impl Session {
    /// A session with a stage cache of `cache_bytes`, no image, an A4 page with a 10 mm margin
    /// that fits the image, the pen "Black 0.3", the `grid` style at its defaults, the default
    /// speeds and no run.
    pub fn new(cache_bytes: usize) -> Session {
        let style = get_style("grid").expect("the grid style is built in");
        Session {
            image: None,
            canvas: Canvas::new(210.0, 297.0, 10.0, Fit::Fit).expect("A4 is a valid canvas"),
            pens: vec![Pen::new("Black 0.3", 0.3, "#000000").expect("the default pen is valid")],
            machine: None,
            settings: style.defaults(),
            style,
            speeds: Speeds::default(),
            cache: Arc::new(Mutex::new(StageCache::new(cache_bytes))),
            shared: Arc::default(),
            cancel: Arc::default(),
        }
    }

    /// Decode PNG, JPEG or WebP `bytes` with [`image::decode`] and use the picture as the source image.
    /// On error the image is unchanged.
    pub fn set_image_bytes(&mut self, bytes: &[u8]) -> Result<(), ImageError> {
        self.image = Some(Arc::new(image::decode(bytes)?));
        Ok(())
    }

    /// Use `image` as the source image.
    pub fn set_image(&mut self, image: Image) {
        self.image = Some(Arc::new(image));
    }

    /// The source image, if one is set.
    pub fn image(&self) -> Option<&Arc<Image>> {
        self.image.as_ref()
    }

    /// The page.
    pub fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    /// Use `c` as the page.
    pub fn set_canvas(&mut self, c: Canvas) {
        self.canvas = c;
    }

    /// The pens, in drawing order.
    pub fn pens(&self) -> &[Pen] {
        &self.pens
    }

    /// Use `pens`, in drawing order. Fails when there are none, because every drawing needs a pen.
    pub fn set_pens(&mut self, pens: Vec<Pen>) -> Result<(), SessionError> {
        if pens.is_empty() {
            return Err(SessionError::NoPens);
        }
        self.pens = pens;
        Ok(())
    }

    /// The machine the G-code export writes for, if one is set.
    pub fn machine(&self) -> Option<&Machine> {
        self.machine.as_ref()
    }

    /// Use `m` as the machine, or none. A machine also sets the speeds the stats time plots at
    /// to the ones it implies. A machine that fails [`Machine::validate`] changes nothing.
    pub fn set_machine(&mut self, m: Option<Machine>) -> Result<(), SessionError> {
        if let Some(m) = &m {
            m.validate()?;
            self.speeds = m.speeds();
        }
        self.machine = m;
        Ok(())
    }

    /// The chosen style.
    pub fn style(&self) -> &Arc<Style> {
        &self.style
    }

    /// Switch to the built-in style called `name` and reset every stage to its defaults. The cache
    /// is kept, so stages the styles share are not run again.
    pub fn set_style(&mut self, name: &str) -> Result<(), SessionError> {
        let style = get_style(name).ok_or_else(|| SessionError::UnknownStyle(name.to_string()))?;
        self.settings = style.defaults();
        self.style = style;
        Ok(())
    }

    /// Every stage's full settings object, keyed by stage name.
    pub fn settings(&self) -> &Value {
        &self.settings
    }

    /// Merge `patch` over the settings of `stage` and keep the result once the stage's schema
    /// accepts the whole object. Fails, changing nothing, for an unknown stage or a refused value.
    pub fn update(&mut self, stage: &str, patch: &Value) -> Result<(), SessionError> {
        let Some(index) = self.style.stage_index(stage) else {
            let names: Vec<&str> = self.style.stages.iter().map(|s| s.name()).collect();
            return Err(SessionError::UnknownStage {
                style: self.style.name.clone(),
                stage: stage.to_string(),
                stages: names.join(", "),
            });
        };
        let merged = merge(&self.settings[stage], patch);
        validate::<Value>(stage, &self.style.stages[index].schema(), &merged)?;
        self.settings[stage] = merged;
        Ok(())
    }

    /// The speeds [`Session::stats`] times a plot at.
    pub fn speeds(&self) -> Speeds {
        self.speeds
    }

    /// Time plots at `s`.
    pub fn set_speeds(&mut self, s: Speeds) {
        self.speeds = s;
    }

    /// Cancel the latest run and return a new one over a snapshot of the style, the settings, the
    /// image, the canvas and the pens, sharing the cache and the result slot. The slot shows
    /// [`RunState::Running`] with no error until the new run ends, and keeps the last drawing.
    /// Later changes to the session do not reach the run. Fails when no image is set.
    pub fn start(&mut self) -> Result<Run, SessionError> {
        let image = self.image.clone().ok_or(SessionError::NoImage)?;
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::default();
        let generation = {
            let mut shared = self.shared.lock().unwrap();
            shared.generation += 1;
            shared.state = RunState::Running;
            shared.error = None;
            shared.generation
        };
        Ok(Run::new(
            self.style.clone(),
            self.settings.clone(),
            [
                Artifact::Image(image),
                Artifact::Canvas(Arc::new(self.canvas.clone())),
                Artifact::Pens(Arc::new(self.pens.clone())),
            ],
            self.cache.clone(),
            self.shared.clone(),
            self.cancel.clone(),
            generation,
        ))
    }

    /// The state the latest run to write the slot left there, or [`RunState::Idle`] before any.
    pub fn state(&self) -> RunState {
        self.shared.lock().unwrap().state
    }

    /// The last finished drawing.
    pub fn drawing(&self) -> Option<Arc<Drawing>> {
        self.shared.lock().unwrap().drawing.clone()
    }

    /// The error message when the latest run ended in [`RunState::Error`].
    pub fn error(&self) -> Option<String> {
        self.shared.lock().unwrap().error.clone()
    }

    /// The last finished drawing as SVG text.
    pub fn svg(&self) -> Option<String> {
        self.drawing().map(|d| svg::to_svg(&d))
    }

    /// The last finished drawing in the `plotline-paths` JSON format.
    pub fn json(&self) -> Option<Value> {
        self.drawing().map(|d| json::to_value(&d))
    }

    /// Layer `layer` of the last finished drawing as G-code for the machine. Fails without a
    /// machine, without a drawing, for a layer the drawing lacks, and when the page does not fit
    /// the machine's area.
    pub fn gcode(&self, layer: usize) -> Result<String, SessionError> {
        let m = self.machine.as_ref().ok_or(SessionError::NoMachine)?;
        let d = self.drawing().ok_or(SessionError::NoDrawing)?;
        Ok(gcode::to_gcode(&d, layer, m)?)
    }

    /// The stats of the last finished drawing at the session's speeds.
    pub fn stats(&self) -> Option<Stats> {
        self.drawing().map(|d| stats::compute(&d, self.speeds))
    }

    /// The last finished drawing as PNG bytes `width_px` wide, of the page or of `region`, in page
    /// mm as `[x, y, width, height]`; see [`preview::render_png`].
    pub fn preview_png(
        &self,
        width_px: u32,
        travel: bool,
        source: bool,
        region: Option<[f64; 4]>,
    ) -> Option<Result<Vec<u8>, PreviewError>> {
        self.drawing()
            .map(|d| preview::render_png(&d, width_px, travel, source, region))
    }

    /// The style, canvas, pens and settings as a preset.
    pub fn preset(&self) -> Value {
        presets::to_value(
            &self.style.name,
            &self.canvas,
            &self.pens,
            self.machine.as_ref(),
            &self.settings,
        )
    }

    /// Apply the preset `v`: its style, which resets every stage to its defaults, its canvas, its
    /// pens, then its settings stage by stage in the style's order, so a key the preset leaves out
    /// keeps its default. Returns one warning for each setting a stage does not have, then one for
    /// each stage the style does not have, sorted by name; all of them are dropped.
    ///
    /// Fails before anything changes when [`presets::parse`] rejects `v`. An unknown style or a
    /// refused setting fails where it is found, after the earlier steps have been applied.
    /// Warnings gathered before an error are dropped with the `Err`.
    pub fn load_preset(&mut self, v: &Value) -> Result<Vec<String>, SessionError> {
        let data = presets::parse(v)?;
        self.set_style(&data.style)?;
        self.set_canvas(data.canvas);
        self.set_pens(data.pens)?;
        self.set_machine(data.machine)?;
        let mut warnings = Vec::new();
        for stage in self.style.stages.clone() {
            let Some(Value::Object(values)) = data.settings.get(stage.name()) else {
                continue;
            };
            let schema = stage.schema();
            let mut kept = values.clone();
            kept.retain(|key, _| {
                let known = schema["properties"].get(key).is_some();
                if !known {
                    let stage = stage.name();
                    warnings.push(format!(
                        "ignoring unknown setting {key:?} of stage {stage:?}"
                    ));
                }
                known
            });
            self.update(stage.name(), &Value::Object(kept))?;
        }
        let stages = data
            .settings
            .as_object()
            .expect("parse checks the settings");
        for stage in stages.keys() {
            if self.style.stage_index(stage).is_none() {
                warnings.push(format!("ignoring unknown stage {stage:?}"));
            }
        }
        Ok(warnings)
    }

    /// What the stage cache holds and how often it was asked.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.lock().unwrap().stats()
    }

    /// Empty the stage cache.
    pub fn clear_cache(&mut self) {
        self.cache.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::gradient;
    use serde_json::json;

    #[test]
    fn defaults_and_setters() {
        let mut s = Session::new(1 << 20);
        assert_eq!(s.canvas().width_mm, 210.0);
        assert_eq!(s.pens()[0].name, "Black 0.3");
        assert_eq!(s.style().name, "grid");
        assert!(s.set_pens(vec![]).is_err());
        assert!(s.set_style("nope").is_err());
        s.update("tone", &json!({"resolution": 128})).unwrap();
        assert_eq!(s.settings()["tone"]["resolution"], 128);
        assert!(s.update("tone", &json!({"resolution": 1})).is_err());
        assert_eq!(s.settings()["tone"]["resolution"], 128);
        assert!(s
            .update("nope", &json!({}))
            .unwrap_err()
            .to_string()
            .contains("its stages are tone, points, render, order"));
        s.set_style("grid").unwrap();
        assert_eq!(s.settings()["tone"]["resolution"], 1000);
    }
    #[test]
    #[allow(clippy::len_zero)] // the brief's assertion, kept as written
    fn start_needs_an_image_and_runs_to_a_drawing() {
        let mut s = Session::new(1 << 20);
        assert!(s.start().is_err());
        s.set_image(gradient(64, 48));
        s.update("tone", &json!({"resolution": 64})).unwrap();
        let mut run = s.start().unwrap();
        while run.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Done);
        assert!(s.drawing().unwrap().layers[0].paths.len() > 0);
        assert!(s.svg().unwrap().starts_with("<svg"));
        assert!(s.stats().unwrap().path_count > 0);
        assert!(s.preview_png(100, false, false, None).unwrap().is_ok());
    }
    #[test]
    fn a_new_run_cancels_the_previous_one() {
        let mut s = Session::new(1 << 20);
        s.set_image(gradient(8, 8));
        s.set_style("_slow").unwrap();
        s.update("b", &json!({"frames": 1000, "delay_ms": 0}))
            .unwrap();
        let mut first = s.start().unwrap();
        first.next_frame();
        first.next_frame();
        let mut second = s.start().unwrap();
        assert!(first.next_frame().is_none());
        assert_eq!(first.state(), RunState::Cancelled);
        assert_eq!(s.state(), RunState::Running); // the cancelled run left the slot alone
        while second.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Done);
    }
    #[test]
    fn start_shows_running_with_no_error_before_the_first_frame() {
        let mut s = Session::new(1 << 20);
        s.set_image(gradient(8, 8));
        s.set_style("_slow").unwrap();
        s.update("b", &json!({"frames": 3, "delay_ms": 0, "fail_after": 1}))
            .unwrap();
        let mut run = s.start().unwrap();
        while run.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Error);
        assert!(s.error().is_some());
        let _run = s.start().unwrap();
        assert_eq!(s.state(), RunState::Running);
        assert!(s.error().is_none());
    }
    #[test]
    fn second_run_with_same_settings_is_all_cached() {
        let mut s = Session::new(1 << 20);
        s.set_image(gradient(64, 48));
        s.update("tone", &json!({"resolution": 64})).unwrap();
        let mut r = s.start().unwrap();
        while r.next_frame().is_some() {}
        let mut r = s.start().unwrap();
        let mut labels = vec![];
        while let Some(f) = r.next_frame() {
            labels.push(f.label);
        }
        assert_eq!(labels, vec!["cached"; 4]);
        assert_eq!(s.cache_stats().hits, 4);
    }
    #[test]
    fn preset_round_trip_and_warnings() {
        let mut s = Session::new(1 << 20);
        s.update("points", &json!({"spacing_mm": 3.0})).unwrap();
        let mut v = s.preset();
        v["settings"]["points"]["bogus"] = json!(1);
        v["settings"]["nostage"] = json!({});
        let mut t = Session::new(1 << 20);
        let warnings = t.load_preset(&v).unwrap();
        assert_eq!(t.settings()["points"]["spacing_mm"], 3.0);
        assert_eq!(
            warnings,
            vec![
                "ignoring unknown setting \"bogus\" of stage \"points\"",
                "ignoring unknown stage \"nostage\""
            ]
        );
    }

    #[test]
    fn a_machine_sets_the_speeds_and_rides_in_the_preset() {
        let mut s = Session::new(1 << 20);
        assert!(s.machine().is_none());
        let m = crate::test_util::machine();
        s.set_machine(Some(m.clone())).unwrap();
        assert_eq!(s.speeds().draw_mm_s, 60.0);
        assert_eq!(s.preset()["machine"]["name"], m.name);
        let mut t = Session::new(1 << 20);
        t.load_preset(&s.preset()).unwrap();
        assert_eq!(t.machine(), Some(&m));
        let mut bad = m.clone();
        bad.z_up_mm = 1.0;
        assert!(s.set_machine(Some(bad)).is_err());
        assert_eq!(s.machine(), Some(&m));
        s.set_machine(None).unwrap();
        assert!(s.machine().is_none());
    }
    #[test]
    fn gcode_needs_a_machine_and_a_drawing() {
        let mut s = Session::new(1 << 20);
        assert!(s.gcode(0).unwrap_err().to_string().contains("machine"));
        s.set_machine(Some(crate::test_util::machine())).unwrap();
        assert!(s.gcode(0).unwrap_err().to_string().contains("drawing"));
        s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap());
        s.set_image(gradient(64, 48));
        s.update("tone", &json!({"resolution": 64})).unwrap();
        let mut run = s.start().unwrap();
        while run.next_frame().is_some() {}
        let g = s.gcode(0).unwrap();
        assert!(
            g.contains("G1 Z4 F1200")
                && g.ends_with(
                    "G1 Y250 F3600
"
                ),
            "{g}"
        );
        assert!(s.gcode(1).unwrap_err().to_string().contains("no layer 1"));
    }
}
