//! Every setting of every listed style changes the drawing. A control that does nothing is a bug
//! the GUI cannot show, so each setting is moved from its default to another legal value and the
//! style is run on a noise image; the paths must differ. The exceptions are listed with a reason,
//! and a listed exception that starts to matter fails too, so the list stays honest.

use plotline_core::registry::list_styles;
use plotline_core::synthetic::noise;
use plotline_core::{Canvas, Fit, Pen, Session};
use serde_json::{json, Value};

/// Settings that leave the drawing alone on purpose: style, stage, key and why.
const NO_OPS: &[(&str, &str, &str, &str)] = &[
    (
        "contours",
        "tone",
        "response",
        "level lines cover no more paper where the image is darker, so there is no response to invert",
    ),
    (
        "grid",
        "tone",
        "contrast",
        "contrast pivots on mid-grey, which is where grid's default threshold sits",
    ),
    (
        "ink",
        "tone",
        "contrast",
        "contrast pivots on mid-grey, which is where ink's default threshold sits",
    ),
    (
        "waves",
        "order",
        "min_length_mm",
        "every wave line crosses the whole drawing, far longer than the 10 mm limit",
    ),
    (
        "grid",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "hatch",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "waves",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "spiral",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "ink",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "stipple",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "line",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "dashes",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "sketch",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "tree",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "mesh",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "cells",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "halftone",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "dither",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "circles",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "contours",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "flow",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "hilbert",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "scribble",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "mosaic",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "truchet",
        "tone",
        "clip_percent",
        "used only when levels is auto, which is off by default",
    ),
    (
        "truchet",
        "tone",
        "resolution",
        "a tile's arc count is rounded from the mean of 16 samples of its ink, and resampling the 64 by 48 noise image to 2500 pixels instead of 1000 moves none of those means across a rounding point",
    ),
    (
        "line",
        "order",
        "min_length_mm",
        "the one line runs through every point, far longer than the 5 mm it is tested at",
    ),
    (
        "scribble",
        "order",
        "min_length_mm",
        "the one scribble runs along the line through every point, far longer than the 5 mm it is tested at",
    ),
    (
        "sketch",
        "order",
        "min_length_mm",
        "at its defaults the style draws one stroke on the noise image, far longer than 5 mm; a higher stop ends strokes sooner and then it drops the short ones",
    ),
    (
        "sketch",
        "order",
        "optimize",
        "at its defaults the style draws one stroke on the noise image, so there is nothing to reorder; a higher stop gives many strokes and then it reorders them",
    ),
    (
        "hilbert",
        "hilbert",
        "max_cell_mm",
        "on the noise image every large block averages mid-grey, so the ink rule folds to about the smallest cell before the largest fold is reached; over white it acts, as a white A4 page shows",
    ),
    (
        "hilbert",
        "order",
        "min_length_mm",
        "at its defaults the curve is one path, far longer than the 5 mm it is tested at; split between pens, a lane lifts over empty areas and leaves fragments, and then it drops the short ones",
    ),
    (
        "hilbert",
        "order",
        "optimize",
        "at its defaults the curve is one path that starts at the corner the pen starts from, so there is nothing to reorder; split between pens, a lane lifts over empty areas and gives many paths, and then it reorders them",
    ),
];

/// A legal value of `prop` other than `default`: the other boolean, the number halfway to the
/// farther bound, or the next enum variant.
fn another(prop: &Value, default: &Value, defs: &Value) -> Value {
    if let Some(variants) = enum_values(prop, defs) {
        return variants
            .into_iter()
            .find(|v| v != default)
            .expect("an enum with a second variant");
    }
    match prop["type"].as_str() {
        Some("boolean") => json!(!default.as_bool().expect("a boolean default")),
        Some(t @ ("integer" | "number")) => {
            let d = default.as_f64().expect("a numeric default");
            let far = match (prop["minimum"].as_f64(), prop["maximum"].as_f64()) {
                (Some(lo), Some(hi)) if d - lo > hi - d => lo,
                (_, Some(hi)) => hi,
                (Some(lo), None) => lo,
                (None, None) => 2.0 * d + 1.0,
            };
            let v = (d + far) / 2.0;
            if t == "integer" {
                json!(v.round() as i64)
            } else {
                json!(v)
            }
        }
        other => panic!("no way to vary a setting of type {other:?}; teach this test"),
    }
}

/// The variants of an enum setting, written directly or through a `$ref` into `$defs`.
fn enum_values(prop: &Value, defs: &Value) -> Option<Vec<Value>> {
    let target = match prop["$ref"].as_str().or(prop["allOf"][0]["$ref"].as_str()) {
        Some(r) => &defs[r.rsplit('/').next().unwrap()],
        None => prop,
    };
    if let Some(e) = target["enum"].as_array() {
        return Some(e.clone());
    }
    target["oneOf"]
        .as_array()
        .map(|vs| vs.iter().map(|v| v["const"].clone()).collect())
}

/// The drawing of `style` on the session's noise image with `patch` over `stage`, as paths JSON.
/// The session is shared between draws, so its cache gives stages upstream of the change back
/// instead of running them again, which matters for the relaxation of every Voronoi style.
fn draw(s: &mut Session, style: &str, stage: &str, patch: &Value) -> Value {
    s.set_style(style).unwrap();
    // A switch keeps changed values, so put every stage back to its defaults first.
    let defaults = s.style().defaults();
    for (name, values) in defaults.as_object().unwrap() {
        s.update(name, values).unwrap();
    }
    s.update(stage, patch).unwrap();
    let mut run = s.start().unwrap();
    while run.next_frame().is_some() {}
    let json = s.json().expect("the run leaves a drawing");
    assert!(run.error().is_none(), "{style}: {:?}", run.error());
    json["layers"].clone()
}

#[test]
fn every_setting_changes_the_drawing() {
    let mut s = Session::new(512 << 20);
    s.set_image(noise(64, 48, 1));
    s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Fit::Fit).unwrap());
    // Two pens, so that the split's light to dark mode has a second layer to add.
    s.set_pens(vec![
        Pen::new("Black", 0.3, "#000000").unwrap(),
        Pen::new("Red", 0.3, "#ff0000").unwrap(),
    ])
    .unwrap();
    let mut failures = Vec::new();
    for style in list_styles() {
        let schema = style.schema();
        let defaults = style.defaults();
        for stage in schema["stages"].as_array().unwrap() {
            let name = stage["name"].as_str().unwrap();
            let base = draw(&mut s, &style.name, name, &json!({}));
            let props = stage["schema"]["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            for (key, prop) in &props {
                let default = &defaults[name][key];
                let value = another(prop, default, &stage["schema"]["$defs"]);
                let changed = draw(&mut s, &style.name, name, &json!({ key: value })) != base;
                let listed = NO_OPS
                    .iter()
                    .any(|(s, g, k, _)| (*s, *g, *k) == (style.name.as_str(), name, key.as_str()));
                if changed == listed {
                    let what = if changed {
                        "is listed as a no-op but changes the drawing"
                    } else {
                        "changes nothing"
                    };
                    failures.push(format!("{}.{name}.{key} = {value} {what}", style.name));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
