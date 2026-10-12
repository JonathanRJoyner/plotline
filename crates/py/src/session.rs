//! The `Session` and `Run` classes, thin wrappers over the engine's session layer.

use std::sync::Mutex;

use numpy::{PyArray3, PyReadonlyArray3};
use plotline_core::{image, Canvas, Pen, RunState, Speeds, DEFAULT_CACHE_BYTES};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde::de::DeserializeOwned;

use crate::convert::{
    drawing_to_py, frame_to_py, image_array, py_to_value, runtime_error, serde_to_py,
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
        Session {
            inner: plotline_core::Session::new(cache_bytes),
        }
    }

    /// Decode PNG or JPEG bytes and use the picture as the source image.
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

    /// Use `{"width_mm", "height_mm", "margin_mm", "fit"}` as the page.
    fn set_canvas(&mut self, canvas: &Bound<'_, PyAny>) -> PyResult<()> {
        let c: Canvas = from_py(canvas)?;
        let c = Canvas::new(c.width_mm, c.height_mm, c.margin_mm, c.fit).map_err(value_error)?;
        self.inner.set_canvas(c);
        Ok(())
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

    fn style_name(&self) -> String {
        self.inner.style().name.clone()
    }

    fn set_style(&mut self, name: &str) -> PyResult<()> {
        self.inner.set_style(name).map_err(session_error)
    }

    fn settings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        value_to_py(py, self.inner.settings())
    }

    /// Merge `patch` over the settings of `stage`; a refused value changes nothing.
    fn update(&mut self, stage: &str, patch: &Bound<'_, PyAny>) -> PyResult<()> {
        let patch = py_to_value(patch)?;
        self.inner.update(stage, &patch).map_err(session_error)
    }

    fn speeds<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, &self.inner.speeds())
    }

    /// Use `{"draw_mm_s", "travel_mm_s", "pen_lift_s"}` as the plot speeds.
    fn set_speeds(&mut self, speeds: &Bound<'_, PyAny>) -> PyResult<()> {
        let speeds: Speeds = from_py(speeds)?;
        self.inner.set_speeds(speeds);
        Ok(())
    }

    /// Cancel the latest run and start a new one over a snapshot of the session.
    fn start(&mut self) -> PyResult<Run> {
        let run = self.inner.start().map_err(session_error)?;
        Ok(Run(Mutex::new(run)))
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

    fn preview_png<'py>(
        &self,
        py: Python<'py>,
        width_px: u32,
        travel: bool,
        source: bool,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let png = self.inner.preview_png(width_px, travel, source);
        let png = png.transpose().map_err(runtime_error)?;
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

/// One run, driven a frame at a time. The engine run is not `Sync`, so it sits behind a lock,
/// and every method takes that lock with the interpreter lock released, so that no thread holds
/// one lock while it waits for the other.
#[pyclass]
pub struct Run(Mutex<plotline_core::Run>);

impl Run {
    fn with<T: Send>(
        &self,
        py: Python<'_>,
        f: impl FnOnce(&mut plotline_core::Run) -> T + Send,
    ) -> T {
        py.detach(|| f(&mut self.0.lock().unwrap()))
    }
}

#[pymethods]
impl Run {
    /// The next frame, or `None` once the run has ended.
    fn next_frame<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let frame = self.with(py, |run| run.next_frame());
        frame.map(|f| frame_to_py(py, f)).transpose()
    }

    /// Ask the run to stop; the next frame is `None`.
    fn cancel(&self, py: Python<'_>) {
        self.with(py, |run| run.cancel());
    }

    fn state(&self, py: Python<'_>) -> &'static str {
        state_name(self.with(py, |run| run.state()))
    }

    fn error(&self, py: Python<'_>) -> Option<String> {
        self.with(py, |run| run.error())
    }
}
