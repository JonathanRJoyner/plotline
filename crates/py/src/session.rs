//! The `Session` and `Run` classes, thin wrappers over the engine's session layer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

use numpy::{PyArray3, PyReadonlyArray3};
use plotline_core::machine::Machine;
use plotline_core::{image, Canvas, Job, Pen, RunState, Speeds, DEFAULT_CACHE_BYTES};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde::de::DeserializeOwned;

use crate::convert::{
    drawing_to_py, frame_to_py, image_array, model_error, preview_error, py_to_value, serde_to_py,
    session_error, value_error, value_to_py,
};

/// `obj` converted to JSON and read as a `T`; a mismatch is a `ValueError`.
fn from_py<T: DeserializeOwned>(obj: &Bound<'_, PyAny>) -> PyResult<T> {
    serde_json::from_value(py_to_value(obj)?).map_err(value_error)
}

fn state_name(s: RunState) -> &'static str {
    match s {
        RunState::Idle => "idle",
        RunState::Running => "running",
        RunState::Done => "done",
        RunState::Cancelled => "cancelled",
        RunState::Error => "error",
    }
}

/// One working context: image, canvas, pens, style, settings, speeds, cache and latest result.
#[pyclass]
pub struct Session {
    inner: plotline_core::Session,
}

#[pymethods]
impl Session {
    #[new]
    #[pyo3(signature = (cache_bytes = DEFAULT_CACHE_BYTES))]
    fn new(cache_bytes: usize) -> Session {
        let mut inner = plotline_core::Session::new(cache_bytes);
        inner.set_model_runtime(Arc::new(plotline_models::TractRuntime::default()));
        Session { inner }
    }

    /// Load the ONNX file `data` as a model and return its entry, `{id, label, overlap, entry}`.
    /// With an `id` the bytes must be that catalogue model, with the size and SHA-256 it lists;
    /// with `None` they are a file of your own, named `file-` and 12 hex digits of its hash.
    /// Raises `ValueError` for a file or id the engine refuses and changes nothing.
    #[pyo3(signature = (id, data))]
    fn add_model<'py>(
        &mut self,
        py: Python<'py>,
        id: Option<&str>,
        data: &[u8],
    ) -> PyResult<Bound<'py, PyAny>> {
        // Parsing and optimising a large graph takes seconds, so other threads keep running.
        let info = py.detach(|| self.inner.add_model(id, data));
        serde_to_py(py, &info.map_err(model_error)?)
    }

    /// Unload the model `id`; an id the session does not hold changes nothing.
    fn remove_model(&mut self, id: &str) {
        self.inner.remove_model(id);
    }

    /// The models the session holds, in id order, as `{id, label, overlap, entry}`.
    fn models<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, &self.inner.models())
    }

    /// Decode PNG, JPEG or WebP bytes and use the picture as the source image.
    fn set_image_bytes(&mut self, data: &[u8]) -> PyResult<()> {
        self.inner.set_image_bytes(data).map_err(value_error)
    }

    /// Use an `(H, W, 3)` float32 array with values in 0..1 as the source image.
    fn set_image(&mut self, array: &Bound<'_, PyAny>) -> PyResult<()> {
        let array: PyReadonlyArray3<f32> = array
            .extract()
            .map_err(|_| value_error("expected an (H, W, 3) float32 array"))?;
        let view = array.as_array();
        let &[h, w, c] = view.shape() else {
            unreachable!("a three-dimensional array has three sides")
        };
        if c != 3 {
            return Err(value_error(format!("expected 3 channels, got {c}")));
        }
        let side = |n: usize| u32::try_from(n).map_err(value_error);
        let image = image::from_rgb_f32(side(w)?, side(h)?, view.iter().copied().collect())
            .map_err(value_error)?;
        self.inner.set_image(image);
        Ok(())
    }

    /// The source image as an `(H, W, 3)` float32 array, or `None`.
    fn image<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyArray3<f32>>> {
        self.inner.image().map(|i| image_array(py, i))
    }

    fn canvas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, self.inner.canvas())
    }

    /// Use `{"width_mm", "height_mm", "margin_mm", "fit"}` as the page, and `"crop"` if there is one.
    fn set_canvas(&mut self, canvas: &Bound<'_, PyAny>) -> PyResult<()> {
        let c: Canvas = from_py(canvas)?;
        let c = Canvas::new(c.width_mm, c.height_mm, c.margin_mm, c.fit)
            .and_then(|new| new.with_crop(c.crop))
            .map_err(value_error)?;
        self.inner.set_canvas(c);
        Ok(())
    }

    /// The colour of the paper the preview shows, written as `#rrggbb`; white until it is set.
    fn paper(&self) -> String {
        self.inner.paper().to_string()
    }

    /// Use `color`, written as `#rrggbb`, as the paper the preview shows. Anything else raises
    /// `ValueError` and changes nothing.
    fn set_paper(&mut self, color: &str) -> PyResult<()> {
        self.inner.set_paper(color).map_err(session_error)
    }

    fn pens<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, self.inner.pens())
    }

    /// Use a list of `{"name", "width_mm", "color"}` as the pens, in drawing order.
    fn set_pens(&mut self, pens: &Bound<'_, PyAny>) -> PyResult<()> {
        let pens: Vec<Pen> = from_py(pens)?;
        let pens = pens
            .iter()
            .map(|p| Pen::new(&p.name, p.width_mm, &p.color))
            .collect::<Result<_, _>>()
            .map_err(value_error)?;
        self.inner.set_pens(pens).map_err(session_error)
    }

    /// Use a list of `{"name", "width_mm", "color"}` as the user's own pens, which the image
    /// analysis picks its palette from; an empty list clears it.
    fn set_pen_library(&mut self, pens: &Bound<'_, PyAny>) -> PyResult<()> {
        let pens: Vec<Pen> = from_py(pens)?;
        let pens = pens
            .iter()
            .map(|p| Pen::new(&p.name, p.width_mm, &p.color))
            .collect::<Result<_, _>>()
            .map_err(value_error)?;
        self.inner.set_pen_library(pens);
        Ok(())
    }

    fn style_name(&self) -> String {
        self.inner.style().name.clone()
    }

    fn set_style(&mut self, name: &str) -> PyResult<()> {
        self.inner.set_style(name).map_err(session_error)
    }

    fn reset_settings(&mut self) {
        self.inner.reset_settings();
    }

    fn settings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        value_to_py(py, self.inner.settings())
    }

    /// Merge `patch` over the settings of `stage`; a refused value changes nothing.
    fn update(&mut self, stage: &str, patch: &Bound<'_, PyAny>) -> PyResult<()> {
        let patch = py_to_value(patch)?;
        self.inner.update(stage, &patch).map_err(session_error)
    }

    /// Set the settings the style's look `name` sets, as `update` does for each of its stages. An
    /// unknown name raises `ValueError`, naming the style's looks, and changes nothing.
    fn apply_look(&mut self, name: &str) -> PyResult<()> {
        self.inner.apply_look(name).map_err(session_error)
    }

    /// The name of the style's look the settings are at, or `None` when they are at none.
    fn look(&self) -> Option<String> {
        self.inner.look()
    }

    fn speeds<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, &self.inner.speeds())
    }

    /// Use `{"draw_mm_s", "travel_mm_s", "pen_lift_s", "accel_mm_s2"}` as the plot speeds.
    fn set_speeds(&mut self, speeds: &Bound<'_, PyAny>) -> PyResult<()> {
        let speeds: Speeds = from_py(speeds)?;
        self.inner.set_speeds(speeds);
        Ok(())
    }

    /// The machine as a dict, or `None`.
    fn machine<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, &self.inner.machine())
    }

    /// Use a machine dict, or `None` for none. A machine also sets the speeds to the ones it
    /// implies. A machine the engine refuses raises `ValueError` and changes nothing.
    fn set_machine(&mut self, machine: &Bound<'_, PyAny>) -> PyResult<()> {
        let machine: Option<Machine> = from_py(machine)?;
        self.inner.set_machine(machine).map_err(session_error)
    }

    /// Layer `layer` of the latest result as G-code for the machine. Raises `RuntimeError` without
    /// a machine or a result and `ValueError` for a layer the result lacks or a page that does not
    /// fit the machine's area.
    fn gcode(&self, layer: usize) -> PyResult<String> {
        self.inner.gcode(layer).map_err(session_error)
    }

    /// The latest result as one HPGL program with `units_per_mm` plotter units to the millimetre.
    /// Raises `RuntimeError` without a result and `ValueError` for a resolution that is not above
    /// 0 and at most 1000.
    fn hpgl(&self, units_per_mm: f64) -> PyResult<String> {
        self.inner.hpgl(units_per_mm).map_err(session_error)
    }

    /// Cancel the latest run and start a new one over a snapshot of the session. Raises
    /// `RuntimeError` when no image is set.
    fn start(&mut self) -> PyResult<Run> {
        let run = self.inner.start().map_err(session_error)?;
        Ok(Run::new(Box::new(run)))
    }

    /// Cancel the latest run and start a fit of the style's density setting to `target` of
    /// `metric`: `minutes`, `paths`, `metres` or `dots`. The settings are left as they are; once
    /// the run is done, `result()` holds the value to apply. Raises `RuntimeError` when no image
    /// is set and `ValueError` for a fit the engine refuses.
    fn fit(&mut self, metric: &str, target: f64) -> PyResult<Run> {
        let fit = self.inner.fit(metric, target).map_err(session_error)?;
        Ok(Run::new(Box::new(fit)))
    }

    /// Cancel the latest run and make the test chart for the page and the first pen the latest
    /// drawing. Needs no image. Raises `ValueError` when the chart does not fit the page.
    fn testchart(&mut self) -> PyResult<()> {
        self.inner.testchart().map_err(session_error)
    }

    /// Cancel the latest run and start a search of the style's density setting for the value
    /// whose drawing follows the tone map most closely for its plot time, every trial within
    /// `target` of `metric` when both are given. The settings are left as they are; once the run
    /// is done, `result()` holds the suggestion. Raises `RuntimeError` when no image is set and
    /// `ValueError` for a search the engine refuses or only one of `metric` and `target`.
    #[pyo3(signature = (metric = None, target = None))]
    fn optimize(&mut self, metric: Option<&str>, target: Option<f64>) -> PyResult<Run> {
        let budget = match (metric, target) {
            (Some(m), Some(t)) => Some((m, t)),
            (None, None) => None,
            _ => return Err(value_error("give both a metric and a target, or neither")),
        };
        let search = self.inner.optimize(budget).map_err(session_error)?;
        Ok(Run::new(Box::new(search)))
    }

    fn state(&self) -> &'static str {
        state_name(self.inner.state())
    }

    fn error(&self) -> Option<String> {
        self.inner.error()
    }

    fn drawing<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        self.inner
            .drawing()
            .map(|d| drawing_to_py(py, &d))
            .transpose()
    }

    fn svg(&self) -> Option<String> {
        self.inner.svg()
    }

    fn json<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.inner.json().map(|v| value_to_py(py, &v)).transpose()
    }

    fn stats<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.inner.stats().map(|s| serde_to_py(py, &s)).transpose()
    }

    fn lint<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.inner.lint().map(|f| serde_to_py(py, &f)).transpose()
    }

    fn tone_match<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.inner
            .tone_match()
            .map(|m| serde_to_py(py, &m))
            .transpose()
    }

    fn analysis<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        self.inner
            .analysis()
            .map(|a| serde_to_py(py, &a))
            .transpose()
    }

    #[pyo3(signature = (width_px, travel, source, region = None))]
    fn preview_png<'py>(
        &self,
        py: Python<'py>,
        width_px: u32,
        travel: bool,
        source: bool,
        region: Option<[f64; 4]>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let png = self.inner.preview_png(width_px, travel, source, region);
        let png = png.transpose().map_err(preview_error)?;
        Ok(png.map(|bytes| PyBytes::new(py, &bytes)))
    }

    fn preset<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        value_to_py(py, &self.inner.preset())
    }

    /// Apply a preset and return a warning for each setting or stage it ignored.
    fn load_preset(&mut self, preset: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
        let preset = py_to_value(preset)?;
        self.inner.load_preset(&preset).map_err(session_error)
    }

    fn cache_stats<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, &self.inner.cache_stats())
    }

    fn clear_cache(&mut self) {
        self.inner.clear_cache();
    }
}

/// One run, fit or optimize search, driven a frame at a time. The engine job is not `Sync`, so it sits behind a
/// lock that `next_frame` holds, with the interpreter lock released, for the whole engine call.
/// The other methods never wait for that lock: `cancel` sets the job's cancel flag directly, and
/// `state`, `error` and `result` report a job that is busy in `next_frame` as running with no
/// error or result.
#[pyclass]
pub struct Run {
    run: Mutex<Box<dyn Job>>,
    cancel: Arc<AtomicBool>,
}

impl Run {
    fn new(job: Box<dyn Job>) -> Run {
        Run {
            cancel: job.cancel_flag(),
            run: Mutex::new(job),
        }
    }

    /// `f` of the job, or `busy` while another thread is inside `next_frame`.
    fn peek<T>(&self, busy: T, f: impl FnOnce(&dyn Job) -> T) -> T {
        match self.run.try_lock() {
            Ok(run) => f(&**run),
            Err(TryLockError::Poisoned(e)) => f(&**e.into_inner()),
            Err(TryLockError::WouldBlock) => busy,
        }
    }
}

#[pymethods]
impl Run {
    /// The next frame, or `None` once the run has ended.
    fn next_frame<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        // A panic in an earlier call poisons the lock; the run is still safe to read and drive.
        let frame = py.detach(|| {
            let mut run = self.run.lock().unwrap_or_else(PoisonError::into_inner);
            run.next_frame()
        });
        frame.map(|f| frame_to_py(py, f)).transpose()
    }

    /// Ask the run to stop; the stage in progress sees it at its next check, and the next frame
    /// is `None`. Returns at once, also while another thread is inside `next_frame`.
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn state(&self) -> &'static str {
        state_name(self.peek(RunState::Running, |run| run.state()))
    }

    fn error(&self) -> Option<String> {
        self.peek(None, |run| run.error())
    }

    /// What a fit chose, as `{stage, key, metric, target, value, achieved, samples}`, or what a
    /// search found, as `{stage, key, value, error, minutes, current, current_error,
    /// current_minutes, budget, trials, reason, patch}`, once it is done; `None` for a plain run.
    fn result<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        let found = self.peek(None, |run| run.result());
        found.map(|r| value_to_py(py, &r)).transpose()
    }
}
