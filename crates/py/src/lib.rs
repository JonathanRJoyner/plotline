//! `plotline._core`: the Rust engine as a Python extension. Every function here calls the
//! engine's session layer, registry or synthetic images; no algorithm lives in this crate.

mod convert;
mod session;

use numpy::PyArray3;
use plotline_core::{registry, synthetic, SessionError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use convert::{image_array, session_error, value_error, value_to_py};

/// The built-in styles as `{"name", "label", "description"}`.
#[pyfunction]
fn list_styles(py: Python<'_>) -> PyResult<Vec<Bound<'_, PyDict>>> {
    registry::list_styles()
        .iter()
        .map(|s| {
            let d = PyDict::new(py);
            d.set_item("name", &s.name)?;
            d.set_item("label", &s.label)?;
            d.set_item("description", &s.description)?;
            Ok(d)
        })
        .collect()
}

/// The style called `name` with each stage's settings as JSON Schema.
#[pyfunction]
fn describe<'py>(py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
    let style = registry::get_style(name)
        .ok_or_else(|| session_error(SessionError::UnknownStyle(name.to_string())))?;
    value_to_py(py, &style.schema())
}

#[pyfunction]
fn synthetic_gradient(py: Python<'_>, w: u32, h: u32) -> Bound<'_, PyArray3<f32>> {
    image_array(py, &synthetic::gradient(w, h))
}

#[pyfunction]
fn synthetic_checkerboard(
    py: Python<'_>,
    w: u32,
    h: u32,
    cells: u32,
) -> PyResult<Bound<'_, PyArray3<f32>>> {
    // The engine panics on these, and a panic reaches Python as a BaseException.
    if cells == 0 || cells > w {
        return Err(value_error(format!(
            "cells must be between 1 and the width {w}, got {cells}"
        )));
    }
    Ok(image_array(py, &synthetic::checkerboard(w, h, cells)))
}

#[pyfunction]
fn synthetic_radial(py: Python<'_>, w: u32, h: u32) -> Bound<'_, PyArray3<f32>> {
    image_array(py, &synthetic::radial(w, h))
}

#[pyfunction]
fn synthetic_noise(py: Python<'_>, w: u32, h: u32, seed: u64) -> Bound<'_, PyArray3<f32>> {
    image_array(py, &synthetic::noise(w, h, seed))
}

#[pyfunction]
fn synthetic_flat(py: Python<'_>, w: u32, h: u32, value: f32) -> Bound<'_, PyArray3<f32>> {
    image_array(py, &synthetic::flat(w, h, value))
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<session::Session>()?;
    m.add_class::<session::Run>()?;
    m.add_function(wrap_pyfunction!(list_styles, m)?)?;
    m.add_function(wrap_pyfunction!(describe, m)?)?;
    m.add_function(wrap_pyfunction!(synthetic_gradient, m)?)?;
    m.add_function(wrap_pyfunction!(synthetic_checkerboard, m)?)?;
    m.add_function(wrap_pyfunction!(synthetic_radial, m)?)?;
    m.add_function(wrap_pyfunction!(synthetic_noise, m)?)?;
    m.add_function(wrap_pyfunction!(synthetic_flat, m)?)?;
    Ok(())
}
