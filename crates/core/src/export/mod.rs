//! Exporters: a finished drawing as text that other programs read. [`svg`] writes plain SVG,
//! [`json`] writes the `plotline-paths` format as a JSON value, [`gcode`] writes one pen layer for
//! a machine and [`hpgl`] writes the whole drawing for a plotter.

pub mod gcode;
pub mod hpgl;
pub mod json;
pub mod svg;
