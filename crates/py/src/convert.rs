//! Conversions between engine values and Python objects, and engine errors as Python exceptions.

use numpy::ndarray::{ArrayView2, ArrayView3};
use numpy::{PyArray1, PyArray2, PyArray3, ToPyArray};
use plotline_core::{Frame, Image, Paths, PreviewError, SessionError};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyString, PyTuple};
use pyo3::IntoPyObjectExt;
use serde::Serialize;
use serde_json::{Map, Number, Value};

/// A `ValueError` carrying `e`'s message.
pub fn value_error(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// A `RuntimeError` carrying `e`'s message.
pub fn runtime_error(e: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// The Python exception for a session error: `ValueError` for a refused input, `RuntimeError` for
/// a session that is not ready, such as a run started before an image was set. The match is
/// exhaustive so that a new variant has to be placed here.
pub fn session_error(e: SessionError) -> PyErr {
    match e {
        SessionError::Style(_)
        | SessionError::Settings(_)
        | SessionError::Preset(_)
        | SessionError::Type(_)
        | SessionError::Image(_)
        | SessionError::UnknownStyle(_)
        | SessionError::UnknownStage { .. }
        | SessionError::Machine(_)
        | SessionError::Gcode(_)
        | SessionError::NoPens
        | SessionError::Fit(_) => value_error(e),
        SessionError::NoImage | SessionError::NoMachine | SessionError::NoDrawing => {
            runtime_error(e)
        }
    }
}

/// The Python exception for a preview error: `ValueError` for a refused region, `RuntimeError` for
/// a picture that could not be made. The match is exhaustive so that a new variant has to be
/// placed here.
pub fn preview_error(e: PreviewError) -> PyErr {
    match e {
        PreviewError::Region(_) => value_error(e),
        PreviewError::Encode(_) | PreviewError::Empty => runtime_error(e),
    }
}

/// `v` as plain Python objects: `None`, `bool`, `int`, `float`, `str`, `list` and `dict`.
pub fn value_to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    match v {
        Value::Null => Ok(py.None().into_bound(py)),
        Value::Bool(b) => b.into_bound_py_any(py),
        Value::Number(n) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.into_bound_py_any(py),
            (None, Some(u)) => u.into_bound_py_any(py),
            _ => n.as_f64().into_bound_py_any(py),
        },
        Value::String(s) => s.into_bound_py_any(py),
        Value::Array(items) => {
            let items: Vec<_> = items
                .iter()
                .map(|x| value_to_py(py, x))
                .collect::<PyResult<_>>()?;
            PyList::new(py, items)?.into_bound_py_any(py)
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (k, x) in map {
                dict.set_item(k, value_to_py(py, x)?)?;
            }
            Ok(dict.into_any())
        }
    }
}

/// `x` serialized to JSON and then converted with [`value_to_py`].
pub fn serde_to_py<'py>(
    py: Python<'py>,
    x: &(impl Serialize + ?Sized),
) -> PyResult<Bound<'py, PyAny>> {
    value_to_py(py, &serde_json::to_value(x).map_err(runtime_error)?)
}

/// A Python object as JSON: `None`, `bool`, `str`, `dict` with `str` keys, `list`, `tuple`, and
/// anything that converts to an integer or a finite float. Anything else is a `ValueError`, and so
/// is nesting deeper than [`MAX_DEPTH`].
pub fn py_to_value(obj: &Bound<'_, PyAny>) -> PyResult<Value> {
    nested_value(obj, 0)
}

/// How deep [`py_to_value`] follows dicts and lists. Settings nest a few levels; anything deeper
/// is most likely a dict or list that contains itself, which would otherwise overflow the stack.
const MAX_DEPTH: usize = 64;

fn nested_value(obj: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
    if depth > MAX_DEPTH {
        return Err(value_error("settings nest too deeply"));
    }
    if obj.is_none() {
        return Ok(Value::Null);
    }
    if let Ok(b) = obj.cast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(Value::String(s.to_str()?.to_string()));
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        let mut map = Map::new();
        for (k, x) in dict.iter() {
            let key: String = k
                .extract()
                .map_err(|_| value_error(format!("dict keys must be strings, got {k:?}")))?;
            map.insert(key, nested_value(&x, depth + 1)?);
        }
        return Ok(Value::Object(map));
    }
    if obj.is_instance_of::<PyList>() || obj.is_instance_of::<PyTuple>() {
        return obj
            .try_iter()?
            .map(|x| nested_value(&x?, depth + 1))
            .collect();
    }
    // Integer extraction goes through `__index__`, which floats lack, so 2.0 stays a float.
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(i.into());
    }
    if let Ok(u) = obj.extract::<u64>() {
        return Ok(u.into());
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Number::from_f64(f)
            .map(Value::Number)
            .ok_or_else(|| value_error(format!("{f} is not a finite number")));
    }
    Err(value_error(format!(
        "cannot use {obj:?} as a setting value"
    )))
}

/// Points as an `(N, 2)` float64 array.
pub fn xy_array<'py>(py: Python<'py>, xy: &[[f64; 2]]) -> Bound<'py, PyArray2<f64>> {
    ArrayView2::from_shape((xy.len(), 2), xy.as_flattened())
        .expect("two values per point")
        .to_pyarray(py)
}

/// An image as an `(H, W, 3)` float32 array.
pub fn image_array<'py>(py: Python<'py>, image: &Image) -> Bound<'py, PyArray3<f32>> {
    let shape = (image.height as usize, image.width as usize, 3);
    ArrayView3::from_shape(shape, &image.rgb)
        .expect("three values per pixel")
        .to_pyarray(py)
}

/// Paths as their `(M, 2)` coordinates and their `uint32` offsets.
fn paths_arrays<'py>(
    py: Python<'py>,
    paths: &Paths,
) -> (Bound<'py, PyArray2<f64>>, Bound<'py, PyArray1<u32>>) {
    (
        xy_array(py, paths.coords()),
        PyArray1::from_slice(py, paths.offsets()),
    )
}

/// A frame as `{"stage", "progress", "label", "pen", "points", "coords", "offsets"}`.
pub fn frame_to_py<'py>(py: Python<'py>, f: Frame) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("stage", f.stage)?;
    d.set_item("progress", f.progress)?;
    d.set_item("label", f.label)?;
    d.set_item("pen", f.pen)?;
    d.set_item("points", f.points.map(|p| xy_array(py, &p.0)))?;
    let (coords, offsets) = f.paths.map(|p| paths_arrays(py, &p)).unzip();
    d.set_item("coords", coords)?;
    d.set_item("offsets", offsets)?;
    Ok(d)
}

/// A drawing as `{"canvas", "layers": [{"pen", "coords", "offsets"}], "tone"}`.
pub fn drawing_to_py<'py>(
    py: Python<'py>,
    drawing: &plotline_core::Drawing,
) -> PyResult<Bound<'py, PyDict>> {
    let layers = PyList::empty(py);
    for layer in &drawing.layers {
        let (coords, offsets) = paths_arrays(py, &layer.paths);
        let l = PyDict::new(py);
        l.set_item("pen", serde_to_py(py, &layer.pen)?)?;
        l.set_item("coords", coords)?;
        l.set_item("offsets", offsets)?;
        layers.append(l)?;
    }
    let tone = match &drawing.tone {
        None => None,
        Some(t) => {
            let ink = ArrayView2::from_shape((t.height as usize, t.width as usize), &t.ink)
                .expect("one value per tone map pixel")
                .to_pyarray(py);
            let tone = PyDict::new(py);
            tone.set_item("ink", ink)?;
            tone.set_item("mm_per_px", t.mm_per_px)?;
            tone.set_item("origin_mm", (t.origin_mm[0], t.origin_mm[1]))?;
            Some(tone)
        }
    };
    let d = PyDict::new(py);
    d.set_item("canvas", serde_to_py(py, &drawing.canvas)?)?;
    d.set_item("layers", layers)?;
    d.set_item("tone", tone)?;
    Ok(d)
}
