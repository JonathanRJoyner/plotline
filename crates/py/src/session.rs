//! The `Session` and `Run` classes, thin wrappers over the engine's session layer.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

use numpy::{PyArray3, PyReadonlyArray3};
use plotline_core::{image, Canvas, Pen, RunState, Speeds, DEFAULT_CACHE_BYTES};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde::de::DeserializeOwned;

use crate::convert::{
    drawing_to_py, frame_to_py, image_array, preview_error, py_to_value, serde_to_py,
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

    fn speeds<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        serde_to_py(py, &self.inner.speeds())
    }

    /// Use `{"draw_mm_s", "travel_mm_s", "pen_lift_s"}` as the plot speeds.
    fn set_speeds(&mut self, speeds: &Bound<'_, PyAny>) -> PyResult<()> {
        let speeds: Speeds = from_py(speeds)?;
        self.inner.set_speeds(speeds);
        Ok(())
    }

    /// Cancel the latest run and start a new one over a snapshot of the session. Raises
    /// `RuntimeError` when no image is set.
    fn start(&mut self) -> PyResult<Run> {
        let run = self.inner.start().map_err(session_error)?;
        Ok(Run {
            cancel: run.cancel_flag(),
            run: Mutex::new(run),
        })
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

/// One run, driven a frame at a time. The engine run is not `Sync`, so it sits behind a lock
/// that `next_frame` holds, with the interpreter lock released, for the whole engine call. The
/// other methods never wait for that lock: `cancel` sets the run's cancel flag directly, and
/// `state` and `error` report a run that is busy in `next_frame` as running with no error.
#[pyclass]
pub struct Run {
    run: Mutex<plotline_core::Run>,
    cancel: Arc<AtomicBool>,
}

impl Run {
    /// `f` of the run, or `busy` while another thread is inside `next_frame`.
    fn peek<T>(&self, busy: T, f: impl FnOnce(&plotline_core::Run) -> T) -> T {
        match self.run.try_lock() {
            Ok(run) => f(&run),
            Err(TryLockError::Poisoned(e)) => f(&e.into_inner()),
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
}
