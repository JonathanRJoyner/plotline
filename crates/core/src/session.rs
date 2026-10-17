//! The session: one working context that holds the state a front end edits and the latest result.
//!
//! A [`Session`] holds the source image, the canvas, the pens, the paper colour, the chosen style
//! with the settings of each of its stages, the plot speeds, the stage cache its runs share, and
//! the result slot its runs write: the latest drawing, the run state and the error.
//! [`Session::start`] hands back a [`Run`] for the host to drive a frame at a time, and the
//! outputs are read from the slot.
//!
//! The session holds no threads, files or network code, so it runs the same natively and in
//! WebAssembly. Hosts add those: a host drives the [`Run`] on a worker thread or in a loop of its
//! own, and reads and writes files itself, passing the session bytes and taking text and bytes
//! back. The CLI, the GUI and the MCP server call this layer and nothing below it.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use serde_json::{json, Value};
use thiserror::Error;

use crate::analysis::{self, Analysis, ImageStats};
use crate::cache::{CacheStats, StageCache};
use crate::export::gcode::{self, GcodeError};
use crate::export::hpgl::{self, HpglError};
use crate::export::{json, svg};
use crate::fit::{places_points, Density, FitRun, Metric};
use crate::image::{self, ImageError};
use crate::lint::{self, Finding, Measure};
use crate::machine::{Machine, MachineError};
use crate::optimize::{Budget, OptimizeRun};
use crate::presets::{self, PresetError};
use crate::preview::{self, PreviewError};
use crate::registry::get_style;
use crate::runner::{Run, RunState, Shared};
use crate::settings::{merge, validate, SettingsError};
use crate::stage::Artifact;
use crate::stats::{self, Speeds, Stats};
use crate::style::{same, Look, Style, StyleError};
use crate::testchart;
use crate::tone_match::{self, ColourMatch, ToneMatch};
use crate::types::{check_color, Canvas, Drawing, Fit, Image, Pen, TypeError, DEFAULT_PAPER};

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
    /// The drawing could not be written as HPGL.
    #[error(transparent)]
    Hpgl(#[from] HpglError),
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
    /// The style has no look of this name; `looks` lists the ones it has.
    #[error("style {style:?} has no look {look:?}; its looks are {looks}")]
    UnknownLook {
        style: String,
        look: String,
        looks: String,
    },
    /// The pens were set to an empty list.
    #[error("a session needs at least one pen")]
    NoPens,
    /// A run was started before an image was set.
    #[error("no image is loaded")]
    NoImage,
    /// A fit was refused; the message says why.
    #[error("{0}")]
    Fit(String),
    /// The test chart could not be made; the message says why.
    #[error("{0}")]
    Testchart(String),
}

/// One working context: an image, a canvas, pens, a style with its settings, the plot speeds, the
/// stage cache and the latest result.
///
/// The setters check their input and change nothing when they fail. At most one run writes the
/// result slot: [`Session::start`] cancels the run before it, and a run that ends after a newer
/// one started leaves the slot alone.
pub struct Session {
    image: Option<Arc<Image>>,
    /// The statistics of `image`, measured when it is set.
    image_stats: Option<ImageStats>,
    canvas: Canvas,
    pens: Vec<Pen>,
    /// The user's own pens, which the image analysis picks a palette from. No stage, preset or
    /// undo history reads it, so it never changes a drawing.
    pen_library: Vec<Pen>,
    /// The colour of the paper, written as `#rrggbb`. Only the preview and the preset read it,
    /// so no stage's cache key holds it and changing it reruns nothing.
    paper: String,
    /// The machine the G-code export writes for, if one is set.
    machine: Option<Machine>,
    style: Arc<Style>,
    /// Every stage's full settings object, keyed by stage name.
    settings: Value,
    /// The settings, as `(stage, key)`, that the user set to something other than the style's
    /// default; a style switch keeps these.
    changed: BTreeSet<(String, String)>,
    speeds: Speeds,
    cache: Arc<Mutex<StageCache>>,
    shared: Arc<Mutex<Shared>>,
    /// The cancel flag of the latest run.
    cancel: Arc<AtomicBool>,
    /// The plot lint measurement of the drawing it was taken for, kept until the drawing changes.
    measured: Mutex<Option<(Weak<Drawing>, Arc<Measure>)>>,
    /// The colour match of the drawing and paper it was taken for.
    coloured: Mutex<Option<Coloured>>,
}

/// A colour match, and the drawing and paper it was taken for.
type Coloured = (Weak<Drawing>, String, Option<ColourMatch>);

impl Session {
    /// A session with a stage cache of `cache_bytes`, no image, an A4 page with a 10 mm margin
    /// that fits the image, white paper, the pen "Black 0.3", the `grid` style at its defaults,
    /// the default speeds and no run.
    pub fn new(cache_bytes: usize) -> Session {
        let style = get_style("grid").expect("the grid style is built in");
        Session {
            image: None,
            image_stats: None,
            canvas: Canvas::new(210.0, 297.0, 10.0, Fit::Fit).expect("A4 is a valid canvas"),
            pens: vec![Pen::new("Black 0.3", 0.3, "#000000").expect("the default pen is valid")],
            pen_library: Vec::new(),
            paper: DEFAULT_PAPER.to_string(),
            machine: None,
            settings: style.defaults(),
            changed: BTreeSet::new(),
            style,
            speeds: Speeds::default(),
            cache: Arc::new(Mutex::new(StageCache::new(cache_bytes))),
            shared: Arc::default(),
            cancel: Arc::default(),
            measured: Mutex::default(),
            coloured: Mutex::default(),
        }
    }

    /// Decode PNG, JPEG or WebP `bytes` with [`image::decode`] and use the picture as the source image.
    /// On error the image is unchanged.
    pub fn set_image_bytes(&mut self, bytes: &[u8]) -> Result<(), ImageError> {
        self.set_image(image::decode(bytes)?);
        Ok(())
    }

    /// Use `image` as the source image and measure it.
    pub fn set_image(&mut self, image: Image) {
        self.image_stats = Some(analysis::measure_with(&image, &self.pen_library));
        self.image = Some(Arc::new(image));
    }

    /// What the source image looks like and what would suit it; see [`analysis::suggest`]. The
    /// statistics were measured when the image was set; the suggestions are worked out now from
    /// the current page, style and settings. `None` without an image.
    pub fn analysis(&self) -> Option<Analysis> {
        let stats = self.image_stats.clone()?;
        let suggestions = analysis::suggest(
            &stats,
            &self.canvas,
            &self.style.name,
            &self.settings,
            &self.pens,
            &self.pen_library,
        );
        Some(Analysis { stats, suggestions })
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

    /// The share `[x, y]` of the source image, from its top-left corner, under the page point
    /// `mm`; see [`Canvas::image_point`]. `None` without an image or where the page shows none.
    pub fn image_point(&self, mm: [f64; 2]) -> Option<[f64; 2]> {
        let image = self.image.as_ref()?;
        self.canvas.image_point(mm, image.width, image.height)
    }

    /// The page point in mm where the share `[x, y]` of the source image is drawn; see
    /// [`Canvas::page_point`]. `None` without an image.
    pub fn page_point(&self, share: [f64; 2]) -> Option<[f64; 2]> {
        let image = self.image.as_ref()?;
        Some(self.canvas.page_point(share, image.width, image.height))
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

    /// The user's own pens; see [`Session::set_pen_library`].
    pub fn pen_library(&self) -> &[Pen] {
        &self.pen_library
    }

    /// Use `pens` as the user's own pens, which the image analysis fits its palette to and finds
    /// the inks worth adding to; an empty list clears it. The image, if any, is measured again.
    /// Nothing else reads the library, so a run and the cache are not touched.
    pub fn set_pen_library(&mut self, pens: Vec<Pen>) {
        self.pen_library = pens;
        if let Some(image) = &self.image {
            self.image_stats = Some(analysis::measure_with(image, &self.pen_library));
        }
    }

    /// The colour of the paper, written as `#rrggbb`: [`DEFAULT_PAPER`] until it is set.
    pub fn paper(&self) -> &str {
        &self.paper
    }

    /// Use `color`, written as `#rrggbb`, as the paper the preview shows. Fails, changing
    /// nothing, for anything else. The paper is no input of any stage, so a run in progress and
    /// the cache are not touched.
    pub fn set_paper(&mut self, color: &str) -> Result<(), SessionError> {
        check_color("paper", color)?;
        self.paper = color.to_string();
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

    /// Switch to the built-in style called `name`. Every stage starts from the new style's
    /// defaults, and a stage the old style also has keeps each value the user set to something
    /// other than a default, so a style's own defaults still apply where the user changed nothing.
    /// A value stays set until the user sets it back to the style's default, even when it equals
    /// the default of a style switched through, so switching away and back returns it. A stage
    /// name means one schema, so the kept values stay valid. The cache is kept, so a shared stage
    /// with the same settings and inputs, such as the Voronoi points, is not run again.
    pub fn set_style(&mut self, name: &str) -> Result<(), SessionError> {
        let style = get_style(name).ok_or_else(|| SessionError::UnknownStyle(name.to_string()))?;
        let mut settings = style.defaults();
        self.changed
            .retain(|(stage, _)| style.stage_index(stage).is_some());
        for (stage, key) in &self.changed {
            settings[stage][key] = self.settings[stage][key].clone();
        }
        self.settings = settings;
        self.style = style;
        Ok(())
    }

    /// Put every stage of the style back to the style's defaults, except Pen use while there is
    /// more than one pen: it goes with the pens, which a reset leaves alone, and resetting it
    /// would leave the first pen, often a light ink, to draw everything
    /// (docs/decisions/2026-10-17-reset-keeps-pen-use.md).
    pub fn reset_settings(&mut self) {
        let keep = self.pens.len() > 1 && self.style.stage_index("split").is_some();
        let split = self.settings["split"].clone();
        self.settings = self.style.defaults();
        self.changed.retain(|(stage, _)| keep && stage == "split");
        if keep {
            self.settings["split"] = split;
        }
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
        let defaults = self.style.defaults();
        for (key, value) in patch.as_object().into_iter().flatten() {
            let mark = (stage.to_string(), key.clone());
            if same(value, &defaults[stage][key]) {
                self.changed.remove(&mark);
            } else {
                self.changed.insert(mark);
            }
        }
        self.settings[stage] = merged;
        Ok(())
    }

    /// Set every key the look `name` of the style sets to the look's value, through
    /// [`Session::update`]: a key set to its default stops counting as the user's change, and any
    /// other counts as one. Nothing else changes, neither the image settings and Pen use nor a
    /// setting no look of the style sets. Fails, changing nothing, for a name the style has no
    /// look of; see [`Style::looks`].
    pub fn apply_look(&mut self, name: &str) -> Result<(), SessionError> {
        let looks = self.style.looks();
        let Some(look) = looks.iter().find(|l| l.name == name) else {
            let names: Vec<&str> = looks.iter().map(|l| l.name.as_str()).collect();
            return Err(SessionError::UnknownLook {
                style: self.style.name.clone(),
                look: name.to_string(),
                looks: names.join(", "),
            });
        };
        for (stage, values) in look.settings.as_object().into_iter().flatten() {
            self.update(stage, values)?;
        }
        Ok(())
    }

    /// The name of the first look of the style whose settings all equal the current ones, numbers
    /// by value, or `None` when none does. It is worked out from the settings on each call, so it
    /// follows an edit, an undo, a preset, a reset and a style switch with nothing stored.
    pub fn look(&self) -> Option<String> {
        let holds = |look: &Look| {
            (look.settings.as_object().into_iter().flatten()).all(|(stage, values)| {
                (values.as_object().into_iter().flatten())
                    .all(|(key, v)| same(v, &self.settings[stage][key]))
            })
        };
        self.style.looks().into_iter().find(holds).map(|l| l.name)
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
        let generation = self.claim();
        Ok(Run::new(
            self.style.clone(),
            self.settings.clone(),
            self.roots(image),
            self.cache.clone(),
            self.shared.clone(),
            self.cancel.clone(),
            generation,
        ))
    }

    /// Cancel the latest run and return a fit, which reruns the style with its density setting
    /// changed until `metric` of the drawing (`minutes`, `paths`, `metres` or `dots`) is near
    /// `target`; see [`fit`](crate::fit). It works over a snapshot of the session, the speeds
    /// included, and claims the result slot as [`Session::start`] does. The settings stay as they
    /// are: once the fit is done, apply [`FitRun::result`]'s value with [`Session::update`].
    ///
    /// Fails, changing nothing, when no image is set, for an unknown metric, a target that is not
    /// a number above 0, a style with no density setting, and `dots` for a style that places no
    /// points.
    pub fn fit(&mut self, metric: &str, target: f64) -> Result<FitRun, SessionError> {
        let image = self.image.clone().ok_or(SessionError::NoImage)?;
        let density = self.density()?;
        let metric = self.metric(metric, target)?;
        let generation = self.claim();
        Ok(FitRun::new(
            self.style.clone(),
            self.settings.clone(),
            self.roots(image),
            self.cache.clone(),
            self.shared.clone(),
            self.cancel.clone(),
            generation,
            self.speeds,
            density,
            metric,
            target,
        ))
    }

    /// Cancel the latest run and make the [test chart](testchart) for the canvas and the first pen
    /// the latest drawing, in state [`RunState::Done`]. It needs no image. Fails, changing nothing,
    /// when the chart does not fit the page or the pen is too thin.
    pub fn testchart(&mut self) -> Result<(), SessionError> {
        let drawing =
            testchart::chart(&self.canvas, &self.pens[0]).map_err(SessionError::Testchart)?;
        self.claim();
        let mut shared = self.shared.lock().unwrap();
        shared.drawing = Some(Arc::new(drawing));
        shared.state = RunState::Done;
        Ok(())
    }

    /// Cancel the latest run and return a search of the style's density setting for the value
    /// whose drawing follows the tone map most closely, within the time and lint limits and with
    /// every trial's `metric` (as for [`Session::fit`]) at most `target` when a budget is given;
    /// see [`optimize`](crate::optimize). It works over a snapshot of the session and claims the result
    /// slot as [`Session::start`] does, and ends with the drawing of the current settings. The
    /// settings stay as they are: [`OptimizeRun::result`] suggests a patch, applied only with
    /// [`Session::update`].
    ///
    /// Fails, changing nothing, as [`Session::fit`] does.
    pub fn optimize(&mut self, budget: Option<(&str, f64)>) -> Result<OptimizeRun, SessionError> {
        let image = self.image.clone().ok_or(SessionError::NoImage)?;
        let density = self.density()?;
        let budget = match budget {
            Some((metric, target)) => Some(Budget {
                metric: self.metric(metric, target)?,
                target,
            }),
            None => None,
        };
        let generation = self.claim();
        Ok(OptimizeRun::new(
            self.style.clone(),
            self.settings.clone(),
            self.roots(image),
            self.cache.clone(),
            self.shared.clone(),
            self.cancel.clone(),
            generation,
            self.speeds,
            density,
            budget,
        ))
    }

    /// The style's density setting, or why it has none.
    fn density(&self) -> Result<Density, SessionError> {
        Density::of(&self.style).ok_or_else(|| {
            let name = &self.style.name;
            SessionError::Fit(format!("style {name:?} has no density setting to fit"))
        })
    }

    /// The metric called `name` for a `target`, or why the pair is refused.
    fn metric(&self, name: &str, target: f64) -> Result<Metric, SessionError> {
        let refuse = |message: String| Err(SessionError::Fit(message));
        let Some(metric) = Metric::parse(name) else {
            let names: Vec<&str> = Metric::ALL.iter().map(|m| m.name()).collect();
            return refuse(format!("unknown metric {name:?}; use {}", names.join(", ")));
        };
        if !(target.is_finite() && target > 0.0) {
            return refuse(format!("the target must be a number above 0, got {target}"));
        }
        if metric == Metric::Dots && !places_points(&self.style) {
            let style = &self.style.name;
            return refuse(format!(
                "style {style:?} places no dots; fit minutes, paths or metres"
            ));
        }
        Ok(metric)
    }

    /// Cancel the latest run, give the next one a fresh cancel flag, and show the slot as running
    /// with no error under a new generation, which this returns.
    fn claim(&mut self) -> u64 {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::default();
        let mut shared = self.shared.lock().unwrap();
        shared.generation += 1;
        shared.state = RunState::Running;
        shared.error = None;
        shared.generation
    }

    /// A run of the style called `name` at the settings [`Session::set_style`] would give it, over
    /// the image, the canvas and the pens, and the slot that run writes. The run shares the stage
    /// cache, so it reuses what earlier runs left there and leaves its own stages for later ones,
    /// but it has its own slot and cancel flag: it does not cancel the latest run, and the
    /// session's style, settings, state and drawing stay as they were. The GUI draws every style
    /// this way. Fails for an unknown style or when no image is set.
    pub fn trial(&mut self, name: &str) -> Result<(Run, Arc<Mutex<Shared>>), SessionError> {
        let image = self.image.clone().ok_or(SessionError::NoImage)?;
        // Switch and switch back, so a trial gets whatever settings a real switch would.
        let (style, settings) = (self.style.clone(), self.settings.clone());
        let changed = self.changed.clone();
        self.set_style(name)?;
        self.changed = changed;
        let style = std::mem::replace(&mut self.style, style);
        let settings = std::mem::replace(&mut self.settings, settings);
        let slot = Arc::<Mutex<Shared>>::default();
        let run = Run::new(
            style,
            settings,
            self.roots(image),
            self.cache.clone(),
            slot.clone(),
            Arc::default(),
            0, // a fresh slot's generation
        );
        Ok((run, slot))
    }

    /// The roots a run starts from: `image`, the canvas and the pens.
    fn roots(&self, image: Arc<Image>) -> [Artifact; 3] {
        [
            Artifact::Image(image),
            Artifact::Canvas(Arc::new(self.canvas.clone())),
            Artifact::Pens(Arc::new(self.pens.clone())),
        ]
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

    /// The last finished drawing as SVG text, with its preset embedded; see
    /// [`Session::embedded_preset`].
    pub fn svg(&self) -> Option<String> {
        self.drawing()
            .map(|d| svg::to_svg(&d, self.embedded_preset(&d).as_ref()))
    }

    /// The last finished drawing in the `plotline-paths` JSON format, with its preset embedded.
    pub fn json(&self) -> Option<Value> {
        self.drawing()
            .map(|d| json::to_value(&d, self.embedded_preset(&d).as_ref()))
    }

    /// The preset of the run that made `d` with the session's machine and paper in place of the
    /// run's null one and white, so loading an export later keeps the user's machine and paper.
    /// `None` for a drawing no run made.
    fn embedded_preset(&self, d: &Drawing) -> Option<Value> {
        let mut preset = d.preset.clone()?;
        preset["machine"] = json!(self.machine);
        preset["paper"] = json!(self.paper);
        Some(preset)
    }

    /// Layer `layer` of the last finished drawing as G-code for the machine. Fails without a
    /// machine, without a drawing, for a layer the drawing lacks, and when the page does not fit
    /// the machine's area.
    pub fn gcode(&self, layer: usize) -> Result<String, SessionError> {
        let m = self.machine.as_ref().ok_or(SessionError::NoMachine)?;
        let d = self.drawing().ok_or(SessionError::NoDrawing)?;
        Ok(gcode::to_gcode(&d, layer, m)?)
    }

    /// The last finished drawing as one HPGL program with `units_per_mm` plotter units to the
    /// millimetre; see [`hpgl::to_hpgl`]. Fails without a drawing and for a resolution that is not
    /// above 0 and at most [`hpgl::MAX_UNITS_PER_MM`].
    pub fn hpgl(&self, units_per_mm: f64) -> Result<String, SessionError> {
        let d = self.drawing().ok_or(SessionError::NoDrawing)?;
        Ok(hpgl::to_hpgl(&d, units_per_mm)?)
    }

    /// The stats of the last finished drawing at the session's speeds.
    pub fn stats(&self) -> Option<Stats> {
        self.drawing().map(|d| stats::compute(&d, self.speeds))
    }

    /// The plot lint findings for the last finished drawing; see [`lint::check`]. The drawing is
    /// measured on the first call after it is made, so runs nobody lints, such as trials, cost
    /// nothing; the findings are worked out on each call from the current style, settings, pens,
    /// speeds and machine, so their patches start from the current settings.
    pub fn lint(&self) -> Option<Vec<Finding>> {
        let (drawing, measure) = self.measure()?;
        Some(lint::check(
            &drawing,
            &measure,
            &self.style,
            &self.settings,
            &self.pens,
            self.speeds,
            self.machine.as_ref(),
        ))
    }

    /// How the last finished drawing's tones follow its tone map; see [`crate::tone_match`]. It
    /// comes from the same measurement as [`Session::lint`], with the colour match of a drawing
    /// in more than one colour against the image on the session's paper. `None` without a
    /// drawing, and when there is nothing to compare.
    pub fn tone_match(&self) -> Option<ToneMatch> {
        let (d, m) = self.measure()?;
        let mut t = m.tone_match?;
        if let Some(image) = self.image.as_ref().filter(|_| tone_match::coloured(&d)) {
            let mut coloured = self.coloured.lock().unwrap();
            t.colour = match &*coloured {
                Some((w, paper, c))
                    if Weak::ptr_eq(w, &Arc::downgrade(&d)) && *paper == self.paper =>
                {
                    *c
                }
                _ => {
                    let c = tone_match::colour(&d, image, &self.paper);
                    *coloured = Some((Arc::downgrade(&d), self.paper.clone(), c));
                    c
                }
            };
        }
        Some(t)
    }

    /// The last finished drawing and its plot lint measurement, taken on the first call after
    /// the drawing is made and kept until it changes.
    fn measure(&self) -> Option<(Arc<Drawing>, Arc<Measure>)> {
        let drawing = self.drawing()?;
        let mut measured = self.measured.lock().unwrap();
        let measure = match &*measured {
            Some((d, m)) if Weak::ptr_eq(d, &Arc::downgrade(&drawing)) => m.clone(),
            _ => {
                let m = Arc::new(lint::measure(&drawing));
                *measured = Some((Arc::downgrade(&drawing), m.clone()));
                m
            }
        };
        Some((drawing, measure))
    }

    /// The last finished drawing as PNG bytes `width_px` wide on the session's paper, of the page
    /// or of `region`, in page mm as `[x, y, width, height]`; see [`preview::render_png`].
    pub fn preview_png(
        &self,
        width_px: u32,
        travel: bool,
        source: bool,
        region: Option<[f64; 4]>,
    ) -> Option<Result<Vec<u8>, PreviewError>> {
        self.drawing()
            .map(|d| preview::render_png(&d, width_px, &self.paper, travel, source, region))
    }

    /// The style, canvas, pens, machine, paper and settings as a preset.
    pub fn preset(&self) -> Value {
        presets::to_value(
            &self.style.name,
            &self.canvas,
            &self.pens,
            self.machine.as_ref(),
            &self.paper,
            &self.settings,
        )
    }

    /// Apply the preset `v`: its style with every stage at its defaults, its canvas, its paper
    /// (white when it has none), its pens, then its settings stage by stage in the style's order,
    /// so a key the preset leaves out takes its default. Returns one warning for each setting a stage does not have, then one for
    /// each stage the style does not have, sorted by name; all of them are dropped.
    ///
    /// Fails before anything changes when [`presets::parse`] rejects `v`. An unknown style or a
    /// refused setting fails where it is found, after the earlier steps have been applied.
    /// Warnings gathered before an error are dropped with the `Err`.
    pub fn load_preset(&mut self, v: &Value) -> Result<Vec<String>, SessionError> {
        let data = presets::parse(v)?;
        self.set_style(&data.style)?;
        // Pen use included, which a reset would keep for the session's pens.
        self.settings = self.style.defaults();
        self.changed.clear();
        self.set_canvas(data.canvas);
        self.set_paper(&data.paper)?;
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
    use crate::analysis::measure;
    use crate::synthetic::{color_bars, flat, gradient, ramp, still_life, strokes};
    use serde_json::json;

    #[test]
    fn analysis_follows_the_image() {
        let mut s = Session::new(1 << 20);
        assert!(s.analysis().is_none());
        s.set_image(strokes(256, 256, 0.0, 1.0));
        let line_art = s.analysis().unwrap();
        assert!(line_art.suggestions.iter().any(|g| g.kind == "line_art"));
        // The suggestions are worked out on each call, so they follow the style.
        s.set_style("ink").unwrap();
        let ink = s.analysis().unwrap();
        assert!(ink.suggestions.iter().all(|g| g.kind != "line_art"));
        assert_eq!(ink.stats, line_art.stats);
        s.set_image(ramp(256, 64, 0.0, 1.0));
        assert_ne!(s.analysis().unwrap().stats, line_art.stats);
    }
    #[test]
    fn a_colour_image_suggests_pens_at_the_sessions_width() {
        let mut s = Session::new(1 << 20);
        s.set_pens(vec![Pen::new("Fine", 0.5, "#000000").unwrap()])
            .unwrap();
        s.set_image(color_bars(256, 64));
        let analysis = s.analysis().unwrap();
        let colour = analysis.suggestions.iter().find(|g| g.kind == "colour");
        let patch = colour.and_then(|g| g.patch.as_ref()).unwrap();
        let pens = patch["pens"].as_array().unwrap();
        assert_eq!(pens.len(), analysis.stats.palette.len());
        assert!(pens.iter().all(|p| p["width_mm"] == 0.5));
        assert_eq!(patch["settings"]["split"]["mode"], "by_colour");
    }
    #[test]
    fn a_new_library_measures_the_image_again() {
        let mut s = Session::new(1 << 20);
        s.set_image(color_bars(256, 64));
        let kinds = |s: &Session| -> Vec<&str> {
            s.analysis()
                .unwrap()
                .suggestions
                .iter()
                .map(|g| g.kind)
                .collect()
        };
        assert!(!kinds(&s).contains(&"pens_to_add"));
        let library = vec![
            Pen::new("Lemon", 0.5, "#f5c400").unwrap(),
            Pen::new("Ink", 0.3, "#000000").unwrap(),
        ];
        s.set_pen_library(library.clone());
        assert_eq!(s.pen_library(), library.as_slice());
        let stats = s.analysis().unwrap().stats;
        assert!(stats.palette.iter().all(|ink| library
            .iter()
            .any(|p| p.color.eq_ignore_ascii_case(&ink.color))));
        assert!(kinds(&s).contains(&"pens_to_add"));
        // An empty library goes back to the inks.
        s.set_pen_library(Vec::new());
        assert_eq!(s.analysis().unwrap().stats, measure(&color_bars(256, 64)));
    }
    #[test]
    fn the_library_is_not_in_the_preset() {
        let mut s = Session::new(1 << 20);
        let before = s.preset();
        s.set_pen_library(vec![Pen::new("Lemon", 0.5, "#f5c400").unwrap()]);
        assert_eq!(s.preset(), before);
    }
    #[test]
    fn a_refused_image_keeps_the_analysis() {
        let mut s = Session::new(1 << 20);
        s.set_image(gradient(64, 48));
        let before = s.analysis();
        assert!(s.set_image_bytes(b"not an image").is_err());
        assert_eq!(s.analysis(), before);
    }
    #[test]
    fn still_life_analysis_is_pinned() {
        // The same literals are in crates/wasm/tests/smoke.rs, so CI proves both targets agree.
        let stats = measure(&still_life(600, 875));
        assert_eq!(stats.two_level, 0.8165753611527483);
        assert_eq!(stats.p_low, 0.1220703125);
    }

    #[test]
    fn points_go_between_the_page_and_the_image() {
        let mut s = Session::new(1 << 20);
        assert!(s.image_point([105.0, 148.5]).is_none() && s.page_point([0.5, 0.5]).is_none());
        s.set_image(gradient(64, 48));
        // The A4 page's centre is the centre of the fitted image.
        assert_eq!(s.image_point([105.0, 148.5]), Some([0.5, 0.5]));
        assert_eq!(s.page_point([0.5, 0.5]), Some([105.0, 148.5]));
        assert_eq!(s.image_point([105.0, 5.0]), None);
    }
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
            .contains("its stages are tone, split, points, render, order"));
        s.set_style("grid").unwrap(); // the same style: nothing changes
        assert_eq!(s.settings()["tone"]["resolution"], 128);
        s.set_style("waves").unwrap();
        s.update("split", &json!({"mode": "light_to_dark"}))
            .unwrap();
        s.reset_settings();
        assert_eq!(s.settings(), &s.style().defaults());
        assert_eq!(s.settings()["tone"]["gamma"], 1.9);
    }
    #[test]
    fn a_switch_keeps_changed_values_and_takes_the_new_style_defaults() {
        let mut s = Session::new(1 << 20);
        s.set_style("hatch").unwrap();
        s.set_style("waves").unwrap();
        assert_eq!(s.settings()["tone"]["gamma"], 1.9);
        s.set_style("hatch").unwrap();
        assert_eq!(s.settings()["tone"]["gamma"], 1.0);
        // An integer equal to the default is no change; JavaScript sends 1.0 as 1.
        s.update("tone", &json!({"gamma": 1})).unwrap();
        s.set_style("waves").unwrap();
        assert_eq!(s.settings()["tone"]["gamma"], 1.9);
        s.update("tone", &json!({"gamma": 1.4})).unwrap();
        s.update("split", &json!({"mode": "light_to_dark"}))
            .unwrap();
        s.set_style("hatch").unwrap();
        assert_eq!(s.settings()["tone"]["gamma"], 1.4);
        assert_eq!(s.settings()["split"]["mode"], "light_to_dark");
        // A value the user set comes back after a switch away and back, even when it is the
        // default of the style in between: 1 is hatch's gamma.
        s.set_style("waves").unwrap();
        s.update("tone", &json!({"gamma": 1.0})).unwrap();
        s.set_style("hatch").unwrap();
        s.set_style("waves").unwrap();
        assert_eq!(s.settings()["tone"]["gamma"], 1.0);
        // Set back to the style's default, it is no longer the user's.
        s.update("tone", &json!({"gamma": 1.9})).unwrap();
        s.set_style("hatch").unwrap();
        assert_eq!(s.settings()["tone"]["gamma"], 1.0);
        // A preset starts from its style's defaults, whatever the session held.
        let mut v = Session::new(1 << 20).preset();
        let stages = v["settings"].as_object_mut().unwrap();
        stages.remove("tone");
        stages.remove("split");
        assert!(s.load_preset(&v).unwrap().is_empty());
        assert_eq!(s.settings()["tone"]["gamma"], 1.0);
        assert_eq!(s.settings()["split"]["mode"], "one_pen");
    }
    fn mark(stage: &str, key: &str) -> (String, String) {
        (stage.to_string(), key.to_string())
    }

    #[test]
    fn a_look_sets_its_keys_and_is_then_the_look() {
        let mut s = Session::new(1 << 20);
        s.set_style("waves").unwrap();
        assert_eq!(s.look().as_deref(), Some("balanced"));
        s.apply_look("fine").unwrap();
        assert_eq!(s.look().as_deref(), Some("fine"));
        assert_eq!(s.settings()["waves"]["spacing_mm"], 1.0);
        assert!(s.changed.contains(&mark("waves", "spacing_mm")));
        s.apply_look("faithful").unwrap();
        assert_eq!(s.look().as_deref(), Some("faithful"));
        assert_eq!(s.settings()["waves"]["spacing_mm"], 2.8);
        // Balanced is the defaults, so its keys stop counting as the user's changes.
        s.apply_look("balanced").unwrap();
        assert_eq!(s.look().as_deref(), Some("balanced"));
        assert_eq!(s.settings(), &s.style().defaults());
        assert!(s.changed.is_empty());
    }
    #[test]
    fn a_reset_keeps_pen_use_while_there_are_several_pens() {
        let mut s = Session::new(1 << 20);
        s.set_style("waves").unwrap();
        let yellow = Pen::new("Yellow", 0.3, "#f5c400").unwrap();
        let black = Pen::new("Black", 0.3, "#000000").unwrap();
        s.set_pens(vec![yellow.clone(), black]).unwrap();
        s.update("split", &json!({"mode": "by_colour"})).unwrap();
        s.update("tone", &json!({"gamma": 1.0})).unwrap();
        s.reset_settings();
        assert_eq!(s.settings()["split"]["mode"], "by_colour");
        assert_eq!(s.settings()["tone"], s.style().defaults()["tone"]);
        // Still the user's change, so a style switch carries it.
        s.set_style("hatch").unwrap();
        assert_eq!(s.settings()["split"]["mode"], "by_colour");
        // With one pen Pen use has nothing to share, so it resets with the rest.
        s.set_pens(vec![yellow]).unwrap();
        s.reset_settings();
        assert_eq!(s.settings(), &s.style().defaults());
        assert!(s.changed.is_empty());
    }

    #[test]
    fn a_reset_and_an_edit_move_the_look() {
        let mut s = Session::new(1 << 20);
        s.set_style("waves").unwrap();
        s.apply_look("fine").unwrap();
        s.reset_settings();
        assert_eq!(s.look().as_deref(), Some("balanced"));
        s.apply_look("fine").unwrap();
        // A key of the look set to another value is no look; set back, in either JSON number
        // form, it is the look again.
        s.update("waves", &json!({"spacing_mm": 1.5})).unwrap();
        assert_eq!(s.look(), None);
        s.update("waves", &json!({"spacing_mm": 1})).unwrap();
        assert_eq!(s.look().as_deref(), Some("fine"));
        // A setting no look of the style sets leaves the look alone.
        s.update("tone", &json!({"gamma": 1.4})).unwrap();
        s.update("waves", &json!({"amplitude": 0.5})).unwrap();
        assert_eq!(s.look().as_deref(), Some("fine"));
    }
    #[test]
    fn an_unknown_look_fails_and_changes_nothing() {
        let mut s = Session::new(1 << 20);
        s.set_style("waves").unwrap();
        s.apply_look("fine").unwrap();
        let (settings, changed) = (s.settings().clone(), s.changed.clone());
        let message = |e: SessionError| e.to_string();
        assert_eq!(
            message(s.apply_look("nope").unwrap_err()),
            r#"style "waves" has no look "nope"; its looks are balanced, faithful, fine"#
        );
        // A look in the list that this style does not declare is unknown to it too.
        assert!(matches!(
            s.apply_look("quick"),
            Err(SessionError::UnknownLook { .. })
        ));
        assert_eq!(s.settings(), &settings);
        assert_eq!(s.changed, changed);
        // A style with no looks but Balanced has that one.
        s.set_style("ink").unwrap();
        assert_eq!(
            message(s.apply_look("fine").unwrap_err()),
            r#"style "ink" has no look "fine"; its looks are balanced"#
        );
        assert_eq!(s.look().as_deref(), Some("balanced"));
        s.apply_look("balanced").unwrap();
    }
    #[test]
    fn a_look_leaves_pen_use_and_the_image_settings_alone() {
        let mut s = Session::new(1 << 20);
        s.set_style("line").unwrap();
        s.update("split", &json!({"mode": "light_to_dark"}))
            .unwrap();
        s.update("tone", &json!({"invert": true})).unwrap();
        let before = s.settings().clone();
        s.apply_look("faithful").unwrap();
        assert_eq!(s.look().as_deref(), Some("faithful"));
        assert_eq!(s.settings()["split"], before["split"]);
        assert_eq!(s.settings()["tone"]["invert"], true);
        assert_eq!(s.settings()["tone"]["response"], "auto");
        assert_eq!(s.settings()["voronoi"]["count"], 20000);
        assert_eq!(
            s.settings()["voronoi"]["iterations"],
            before["voronoi"]["iterations"]
        );
        s.apply_look("balanced").unwrap();
        assert_eq!(s.settings()["split"], before["split"]);
        assert_eq!(s.settings()["tone"]["invert"], true);
        assert_eq!(s.settings()["voronoi"], before["voronoi"]);
    }
    #[test]
    fn a_look_survives_a_switch_to_a_style_sharing_its_stages_and_back() {
        let mut s = Session::new(1 << 20);
        s.set_style("stipple").unwrap();
        s.apply_look("fine").unwrap();
        s.set_style("line").unwrap();
        // The point count is kept, so the points are reused; line's looks are not this one.
        assert_eq!(s.settings()["voronoi"]["count"], 12000);
        assert_eq!(s.look(), None);
        s.set_style("stipple").unwrap();
        assert_eq!(s.look().as_deref(), Some("fine"));
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
        assert_eq!(labels, vec!["cached"; 5]);
        assert_eq!(s.cache_stats().hits, 5);
    }
    #[test]
    fn the_paper_is_white_until_set_and_takes_only_rrggbb() {
        let mut s = Session::new(1 << 20);
        assert_eq!(s.paper(), "#ffffff");
        assert_eq!(s.paper(), DEFAULT_PAPER);
        s.set_paper("#f5f0e1").unwrap();
        assert_eq!(s.paper(), "#f5f0e1");
        for bad in ["cream", "#fff"] {
            let e = s.set_paper(bad).unwrap_err().to_string();
            assert_eq!(e, format!("paper must be written as #rrggbb, got {bad:?}"));
            assert_eq!(s.paper(), "#f5f0e1");
        }
    }
    #[test]
    fn a_new_paper_reruns_nothing_and_repaints() {
        let mut s = Session::new(1 << 20);
        s.set_image(gradient(64, 48));
        s.update("tone", &json!({"resolution": 64})).unwrap();
        let mut r = s.start().unwrap();
        while r.next_frame().is_some() {}
        let white = s.preview_png(100, false, false, None).unwrap().unwrap();
        s.set_paper("#000000").unwrap();
        let black = s.preview_png(100, false, false, None).unwrap().unwrap();
        assert_ne!(white, black);
        let mut r = s.start().unwrap();
        let mut labels = vec![];
        while let Some(f) = r.next_frame() {
            labels.push(f.label);
        }
        assert_eq!(labels, vec!["cached"; 5]);
    }
    #[test]
    fn the_paper_rides_in_the_preset() {
        let mut s = Session::new(1 << 20);
        assert_eq!(s.preset()["paper"], "#ffffff");
        s.set_paper("#f5f0e1").unwrap();
        assert_eq!(s.preset()["paper"], "#f5f0e1");
        let mut t = Session::new(1 << 20);
        assert!(t.load_preset(&s.preset()).unwrap().is_empty());
        assert_eq!(t.paper(), "#f5f0e1");
        // A preset without the key is white paper.
        let mut v = s.preset();
        v.as_object_mut().unwrap().remove("paper");
        assert!(t.load_preset(&v).unwrap().is_empty());
        assert_eq!(t.paper(), DEFAULT_PAPER);
        // A bad paper is refused before anything changes.
        t.set_paper("#112233").unwrap();
        v["paper"] = json!("cream");
        assert!(t.load_preset(&v).is_err());
        assert_eq!(t.paper(), "#112233");
    }
    #[test]
    fn switching_voronoi_style_at_the_defaults_reuses_the_points() {
        let mut s = Session::new(256 << 20);
        s.set_image(gradient(64, 48));
        s.set_style("stipple").unwrap();
        let mut r = s.start().unwrap();
        while r.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Done);
        s.set_style("line").unwrap();
        let mut r = s.start().unwrap();
        let mut voronoi = vec![];
        while let Some(f) = r.next_frame() {
            if f.stage == "voronoi" {
                voronoi.push(f.label);
            }
        }
        assert_eq!(s.state(), RunState::Done);
        assert_eq!(voronoi, ["cached"]);
    }
    #[test]
    fn a_changed_relaxation_survives_a_switch_to_line() {
        let mut s = Session::new(256 << 20);
        s.set_image(gradient(64, 48));
        s.set_style("stipple").unwrap();
        s.update("voronoi", &json!({"iterations": 5})).unwrap();
        let mut r = s.start().unwrap();
        while r.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Done);
        s.set_style("line").unwrap();
        assert_eq!(s.settings()["voronoi"]["iterations"], 5);
        let mut r = s.start().unwrap();
        let mut voronoi = vec![];
        while let Some(f) = r.next_frame() {
            if f.stage == "voronoi" {
                voronoi.push(f.label);
            }
        }
        assert_eq!(s.state(), RunState::Done);
        assert_eq!(voronoi, ["cached"]);
    }
    #[test]
    fn a_trial_shares_the_cache_and_leaves_the_session_alone() {
        let mut s = Session::new(256 << 20);
        assert!(matches!(s.trial("line"), Err(SessionError::NoImage)));
        s.set_image(gradient(64, 48));
        s.set_style("stipple").unwrap();
        let mut r = s.start().unwrap();
        while r.next_frame().is_some() {}
        let (drawing, settings) = (s.drawing().unwrap(), s.settings().clone());
        assert!(s.trial("nope").is_err());
        let mut latest = s.start().unwrap();
        let (mut trial, slot) = s.trial("line").unwrap();
        let mut voronoi = vec![];
        while let Some(f) = trial.next_frame() {
            if f.stage == "voronoi" {
                voronoi.push(f.label);
            }
        }
        assert_eq!(voronoi, ["cached"]);
        assert_eq!(slot.lock().unwrap().state, RunState::Done);
        assert!(slot.lock().unwrap().drawing.is_some());
        assert_eq!(s.style().name, "stipple");
        assert_eq!(s.settings(), &settings);
        assert_eq!(s.state(), RunState::Running);
        assert!(Arc::ptr_eq(&s.drawing().unwrap(), &drawing));
        while latest.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Done); // the trial did not cancel it
    }
    /// Run `s` to the end and return its frames' stages and labels.
    fn drain(s: &mut Session) -> Vec<(String, String)> {
        let mut run = s.start().unwrap();
        let mut seen = vec![];
        while let Some(f) = run.next_frame() {
            seen.push((f.stage, f.label));
        }
        assert_eq!(s.state(), RunState::Done, "{:?}", s.error());
        seen
    }

    #[test]
    fn auto_response_runs_the_wedge_first_and_keeps_the_tone_map() {
        let mut s = Session::new(256 << 20);
        s.set_image(gradient(96, 64));
        s.set_canvas(Canvas::new(120.0, 80.0, 0.0, Fit::Fit).unwrap());
        s.set_style("circles").unwrap();
        s.update("tone", &json!({"resolution": 96})).unwrap();
        let off = drain(&mut s);
        assert!(off.iter().all(|(_, label)| !label.starts_with("response")));
        let plain = s.drawing().unwrap();
        s.update("tone", &json!({"response": "auto"})).unwrap();
        let auto = drain(&mut s);
        let wedge: Vec<_> = auto
            .iter()
            .filter(|(_, l)| l.starts_with("response: "))
            .collect();
        assert!(!wedge.is_empty() && wedge.iter().all(|(stage, _)| stage == "tone"));
        let drawn = s.drawing().unwrap();
        // The drawing keeps the tone map as the tone stage made it, and its marks change.
        assert_eq!(
            drawn.tone.as_ref().unwrap().ink,
            plain.tone.as_ref().unwrap().ink
        );
        assert_ne!(drawn.layers[0].paths, plain.layers[0].paths);
        // A second run finds the wedge and every stage cached.
        let again = drain(&mut s);
        assert!(
            again.iter().all(|(_, l)| l.ends_with("cached")),
            "{again:?}"
        );
        assert_eq!(s.drawing().unwrap().layers[0].paths, drawn.layers[0].paths);
        // Larger circles change the response, so the placer before them runs again, which with
        // the response off it would not.
        s.update("stipple", &json!({"size_mm": 2.0})).unwrap();
        let larger = drain(&mut s);
        assert!(larger
            .iter()
            .any(|(stage, l)| stage == "poisson" && l != "cached"));
        s.update("tone", &json!({"response": "off"})).unwrap();
        drain(&mut s);
        s.update("stipple", &json!({"size_mm": 2.5})).unwrap();
        let plain = drain(&mut s);
        assert!(plain
            .iter()
            .any(|(stage, l)| stage == "poisson" && l == "cached"));
    }

    #[test]
    fn a_cancel_during_the_wedge_cancels_the_run() {
        let mut s = Session::new(256 << 20);
        s.set_image(gradient(64, 48));
        s.set_style("circles").unwrap();
        s.update("tone", &json!({"resolution": 64, "response": "auto"}))
            .unwrap();
        let mut run = s.start().unwrap();
        let first = run.next_frame().unwrap();
        assert!(first.label.starts_with("response: "), "{}", first.label);
        run.cancel();
        assert!(run.next_frame().is_none());
        assert_eq!(run.state(), RunState::Cancelled);
    }

    #[test]
    fn auto_response_lowers_the_tone_error_of_the_joined_point_styles() {
        // Joined points cover the paper as the square root of their density, the clearest bend
        // a style has, so on a gradient the correction must pay off.
        for style in ["line", "tree", "mesh"] {
            let mut s = Session::new(256 << 20);
            s.set_image(gradient(300, 200));
            s.set_canvas(Canvas::new(150.0, 100.0, 0.0, Fit::Fit).unwrap());
            s.set_style(style).unwrap();
            s.update("voronoi", &json!({"count": 1500, "iterations": 10}))
                .unwrap();
            drain(&mut s);
            let off = s.tone_match().unwrap();
            s.update("tone", &json!({"response": "auto"})).unwrap();
            drain(&mut s);
            let auto = s.tone_match().unwrap();
            assert!(auto.error < off.error, "{style}: {auto:?} against {off:?}");
        }
    }

    #[test]
    fn optimize_suggests_a_spacing_and_leaves_the_settings_alone() {
        let mut s = Session::new(256 << 20);
        s.set_image(still_life(120, 175));
        s.set_canvas(Canvas::new(60.0, 87.5, 0.0, Fit::Fit).unwrap());
        s.set_style("hatch").unwrap();
        s.update("tone", &json!({"resolution": 175})).unwrap();
        assert!(s.optimize(Some(("nope", 1.0))).is_err());
        let settings = s.settings().clone();
        let mut job = s.optimize(None).unwrap();
        let mut labels = vec![];
        while let Some(f) = job.next_frame() {
            if f.stage == "optimize" {
                labels.push(f.label);
            }
        }
        assert_eq!(labels.len(), crate::optimize::TRIALS);
        assert!(
            labels[0].starts_with("try 1 of 9: spacing_mm 1: tone error")
                && labels[0].ends_with(" min"),
            "{labels:?}"
        );
        let found = job.result().unwrap();
        assert_eq!(s.settings(), &settings);
        assert_eq!(s.state(), RunState::Done);
        // The session shows the drawing of the current settings, which the first trial made.
        drain(&mut s);
        let drawn = s.drawing().unwrap();
        assert_eq!(found.current, json!(1.0));
        let close = (1.0 + crate::optimize::CLOSE) * found.current_error.unwrap();
        assert!(found.error.unwrap() <= close);
        assert_eq!(
            found.current_minutes,
            s.stats().unwrap().estimated_seconds / 60.0
        );
        assert_eq!(found.patch.is_some(), found.value != found.current);
        assert!(!drawn.layers[0].paths.is_empty());
        // A budget the first trial breaks keeps every chosen trial within it.
        let minutes = s.stats().unwrap().estimated_seconds / 60.0;
        let mut job = s.optimize(Some(("minutes", minutes * 0.7))).unwrap();
        while job.next_frame().is_some() {}
        let budgeted = job.result().unwrap();
        assert_eq!(budgeted.current_error, None);
        let best = budgeted
            .trials
            .iter()
            .find(|t| json!(t.value) == budgeted.value)
            .unwrap();
        assert!(
            !best.over && best.metric.unwrap() <= minutes * 0.7,
            "{budgeted:?}"
        );
    }

    #[test]
    fn optimize_on_a_blank_drawing_says_why_and_suggests_nothing() {
        let mut s = Session::new(64 << 20);
        s.set_image(flat(40, 30, 1.0));
        s.set_canvas(Canvas::new(40.0, 30.0, 0.0, Fit::Fit).unwrap());
        s.set_style("hatch").unwrap();
        let mut job = s.optimize(None).unwrap();
        while job.next_frame().is_some() {}
        assert_eq!(job.state(), RunState::Done);
        let found = job.result().unwrap();
        assert_eq!((found.error, &found.patch), (None, &None));
        assert_eq!((&found.value, found.current_error), (&found.current, None));
        let why = "No value of spacing_mm from 0.25 to 4 mm draws anything here";
        assert!(found.reason.starts_with(why), "{}", found.reason);
    }

    #[test]
    fn the_tone_match_and_the_response_are_pinned() {
        // The same setup and literals are in crates/wasm/tests/smoke.rs, so CI proves both
        // targets agree on the eye filter, the wedge and the lookup.
        let mut s = Session::new(1 << 24);
        s.set_image(gradient(64, 48));
        s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap());
        s.set_style("waves").unwrap();
        s.update("tone", &json!({"resolution": 64})).unwrap();
        drain(&mut s);
        let off = s.tone_match().unwrap();
        s.update("tone", &json!({"response": "auto"})).unwrap();
        drain(&mut s);
        let auto = s.tone_match().unwrap();
        assert_eq!(
            (off.error, off.ssim),
            (0.08197368769144689, 0.7488977156057808)
        );
        assert_eq!(auto.error, 0.08880655753709105);
    }

    #[test]
    fn the_focus_map_is_pinned() {
        // The same setup and literal are in crates/wasm/tests/smoke.rs, so CI proves both
        // targets blur the tone map away from the focus points alike.
        let mut s = Session::new(1 << 24);
        s.set_image(still_life(120, 120));
        s.set_canvas(Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap());
        let focus = json!([[0.3, 0.4], [0.8, 0.7]]);
        s.update("tone", &json!({"resolution": 64, "focus": focus}))
            .unwrap();
        drain(&mut s);
        let drawing = s.drawing().unwrap();
        let ink = &drawing.tone.as_ref().unwrap().ink;
        let sum = (ink.iter()).fold(0u32, |h, v| h.wrapping_mul(31).wrapping_add(v.to_bits()));
        assert_eq!(sum, 686_476_666);
    }

    #[test]
    fn the_test_chart_becomes_the_drawing() {
        let mut s = Session::new(1 << 20);
        assert!(s.drawing().is_none());
        s.testchart().unwrap(); // no image is needed
        assert_eq!(s.state(), RunState::Done);
        assert!(s.error().is_none());
        assert!(s.svg().unwrap().starts_with("<svg"));
        assert!(s.stats().unwrap().path_count > 0);
        // The PASSES patches are drawn over by design.
        let codes: Vec<&str> = s.lint().unwrap().iter().map(|f| f.code).collect();
        assert!(codes.contains(&"overdraw"), "{codes:?}");
    }
    #[test]
    fn a_refused_test_chart_keeps_the_old_drawing() {
        let mut s = Session::new(1 << 20);
        s.testchart().unwrap();
        let before = s.drawing().unwrap();
        s.set_image(gradient(8, 8));
        s.set_style("_slow").unwrap();
        s.update("b", &json!({"frames": 3, "delay_ms": 0})).unwrap();
        let mut run = s.start().unwrap();
        s.set_canvas(Canvas::new(74.0, 105.0, 10.0, Fit::Fit).unwrap());
        s.set_pens(vec![Pen::new("fat", 2.0, "#000000").unwrap()])
            .unwrap();
        let e = s.testchart().unwrap_err();
        assert!(matches!(e, SessionError::Testchart(_)));
        assert!(e.to_string().contains("too small"), "{e}");
        assert!(Arc::ptr_eq(&s.drawing().unwrap(), &before));
        // The refusal did not claim the slot, so the run it would have cancelled still runs.
        assert_eq!(s.state(), RunState::Running);
        while run.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Done);
    }
    #[test]
    fn a_test_chart_cancels_the_latest_run() {
        let mut s = Session::new(1 << 20);
        s.set_image(gradient(8, 8));
        s.set_style("_slow").unwrap();
        s.update("b", &json!({"frames": 1000, "delay_ms": 0}))
            .unwrap();
        let mut run = s.start().unwrap();
        run.next_frame();
        s.testchart().unwrap();
        assert!(run.next_frame().is_none());
        assert_eq!(run.state(), RunState::Cancelled);
        assert_eq!(s.state(), RunState::Done);
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
    fn exports_load_back_as_the_runs_own_preset_with_the_sessions_machine_and_paper() {
        let mut s = Session::new(1 << 20);
        s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap());
        s.set_paper("#f5f0e1").unwrap();
        s.set_pens(vec![Pen::new("A & <B>", 0.4, "#112233").unwrap()])
            .unwrap();
        s.update("tone", &json!({"resolution": 64})).unwrap();
        s.update("points", &json!({"spacing_mm": 3.0})).unwrap();
        s.set_image(gradient(64, 48));
        let mut run = s.start().unwrap();
        while run.next_frame().is_some() {}
        let sent = s.preset();
        for text in [s.svg().unwrap(), s.json().unwrap().to_string()] {
            let mut t = Session::new(1 << 20);
            let v = presets::from_text(&text).unwrap();
            assert!(t.load_preset(&v).unwrap().is_empty());
            assert_eq!(t.preset(), sent);
        }
        // Later changes do not reach the drawing's own preset, but the machine and the paper are
        // the session's.
        s.update("points", &json!({"spacing_mm": 4.0})).unwrap();
        s.set_machine(Some(crate::test_util::machine())).unwrap();
        s.set_paper("#202020").unwrap();
        let v = presets::from_text(&s.svg().unwrap()).unwrap();
        assert_eq!(v["settings"]["points"]["spacing_mm"], 3.0);
        assert_eq!(v["machine"], s.preset()["machine"]);
        assert_eq!(v["paper"], "#202020");
        assert_eq!(s.json().unwrap()["preset"], v);
    }

    #[test]
    fn an_old_preset_loads_in_one_pen_mode_and_a_mode_round_trips() {
        let mut v = Session::new(1 << 20).preset();
        v["settings"].as_object_mut().unwrap().remove("split");
        let mut s = Session::new(1 << 20);
        assert!(s.load_preset(&v).unwrap().is_empty());
        assert_eq!(s.settings()["split"]["mode"], "one_pen");
        s.update("split", &json!({"mode": "light_to_dark"}))
            .unwrap();
        let mut t = Session::new(1 << 20);
        assert!(t.load_preset(&s.preset()).unwrap().is_empty());
        assert_eq!(t.settings()["split"]["mode"], "light_to_dark");
        // A preset starts from its style's defaults, even where a reset would keep Pen use.
        let black = Pen::new("Black", 0.3, "#000000").unwrap();
        t.set_pens(vec![black.clone(), black]).unwrap();
        assert!(t.load_preset(&v).unwrap().is_empty());
        assert_eq!(t.settings()["split"]["mode"], "one_pen");
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

    #[test]
    fn hpgl_needs_a_drawing_but_no_machine() {
        let mut s = Session::new(1 << 20);
        assert!(s.hpgl(40.0).unwrap_err().to_string().contains("drawing"));
        s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap());
        s.set_image(gradient(64, 48));
        s.update("tone", &json!({"resolution": 64})).unwrap();
        let mut run = s.start().unwrap();
        while run.next_frame().is_some() {}
        let h = s.hpgl(40.0).unwrap();
        assert!(h.starts_with("IN;\nSP1;\nPU"), "{h}");
        assert!(h.ends_with("PU;\nSP0;\n"), "{h}");
        let err = s.hpgl(0.0).unwrap_err();
        assert!(matches!(err, SessionError::Hpgl(_)), "{err}");
    }
}
