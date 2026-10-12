//! Exporters: a finished drawing as text that other programs read. [`svg`] writes plain SVG and
//! [`json`] writes the `plotline-paths` format as a JSON value.

pub mod json;
pub mod svg;
