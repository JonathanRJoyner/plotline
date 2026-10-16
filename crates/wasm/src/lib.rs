//! The browser binding: the engine's session layer compiled to WebAssembly with wasm-bindgen.
//!
//! The GUI runs a [`Session`] inside a Web Worker and drives each [`Run`] a frame at a time,
//! posting frames to the page between steps. The page and the worker protocol belong to the GUI
//! spec; this crate only wraps the session. JSON crosses as text, geometry as fresh typed arrays
//! the worker can transfer, and every error as a `JsError` carrying the engine's message. No
//! algorithm lives here.

use std::sync::Arc;

use js_sys::{Array, Float64Array, Uint32Array, Uint8Array};
use plotline_core::machine::Machine;
use plotline_core::{
    image, pens, registry, synthetic, Canvas, Drawing, Frame, Layer, Pen, RunState, SessionError,
    DEFAULT_CACHE_BYTES,
};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

fn state_name(s: RunState) -> String {
    match s {
        RunState::Idle => "idle",
        RunState::Running => "running",
        RunState::Done => "done",
        RunState::Cancelled => "cancelled",
        RunState::Error => "error",
    }
    .to_string()
}

/// One working context: image, canvas, pens, style, settings, cache and latest result.
#[wasm_bindgen]
pub struct Session {
    inner: plotline_core::Session,
}

#[wasm_bindgen]
impl Session {
    /// A session with the default cache size, an A4 page, one black pen and the `grid` style.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)] // JavaScript builds it with `new Session()`
    pub fn new() -> Session {
        Session {
            inner: plotline_core::Session::new(DEFAULT_CACHE_BYTES),
        }
    }

    /// Decode PNG, JPEG or WebP bytes and use the picture as the source image.
    #[wasm_bindgen(js_name = setImageBytes)]
    pub fn set_image_bytes(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        Ok(self.inner.set_image_bytes(bytes)?)
    }

    /// Use row-major RGB floats in 0..1, three per pixel, as the source image.
    #[wasm_bindgen(js_name = setImage)]
    pub fn set_image(&mut self, width: u32, height: u32, rgb: &[f32]) -> Result<(), JsError> {
        let image = image::from_rgb_f32(width, height, rgb.to_vec())?;
        self.inner.set_image(image);
        Ok(())
    }

    /// Use `{"width_mm", "height_mm", "margin_mm", "fit"}` as the page.
    #[wasm_bindgen(js_name = setCanvas)]
    pub fn set_canvas(&mut self, json: &str) -> Result<(), JsError> {
        let c: Canvas = serde_json::from_str(json)?;
        self.inner
            .set_canvas(Canvas::new(c.width_mm, c.height_mm, c.margin_mm, c.fit)?);
        Ok(())
    }

    /// Use a list of `{"name", "width_mm", "color"}` as the pens, in drawing order.
    #[wasm_bindgen(js_name = setPens)]
    pub fn set_pens(&mut self, json: &str) -> Result<(), JsError> {
        let pens: Vec<Pen> = serde_json::from_str(json)?;
        let pens = pens
            .iter()
            .map(|p| Pen::new(&p.name, p.width_mm, &p.color))
            .collect::<Result<_, _>>()?;
        Ok(self.inner.set_pens(pens)?)
    }

    /// Use the machine in `json` for G-code export, or `null` for none. A machine also sets the
    /// speeds the stats time plots at.
    #[wasm_bindgen(js_name = setMachine)]
    pub fn set_machine(&mut self, json: &str) -> Result<(), JsError> {
        let m: Option<Machine> = serde_json::from_str(json)?;
        Ok(self.inner.set_machine(m)?)
    }

    /// The machine as JSON, or `undefined` when none is set.
    pub fn machine(&self) -> Option<String> {
        self.inner.machine().map(|m| json!(m).to_string())
    }

    /// Layer `layer` of the last finished drawing as G-code for the machine.
    pub fn gcode(&self, layer: usize) -> Result<String, JsError> {
        Ok(self.inner.gcode(layer)?)
    }

    /// Switch to the built-in style `name`; a stage the old style also has keeps the values the
    /// user changed, and everything else takes the new style's defaults.
    #[wasm_bindgen(js_name = setStyle)]
    pub fn set_style(&mut self, name: &str) -> Result<(), JsError> {
        Ok(self.inner.set_style(name)?)
    }

    /// Put every stage of the style, Pen use included, back to the style's defaults.
    #[wasm_bindgen(js_name = resetSettings)]
    pub fn reset_settings(&mut self) {
        self.inner.reset_settings();
    }

    /// Every stage's settings as JSON, keyed by stage name.
    pub fn settings(&self) -> String {
        self.inner.settings().to_string()
    }

    /// Merge the JSON object `json` over the settings of `stage`; a refused value changes nothing.
    pub fn update(&mut self, stage: &str, json: &str) -> Result<(), JsError> {
        let patch: Value = serde_json::from_str(json)?;
        Ok(self.inner.update(stage, &patch)?)
    }

    /// Time plots at the `{"draw_mm_s", "travel_mm_s", "pen_lift_s"}` in `json`.
    #[wasm_bindgen(js_name = setSpeeds)]
    pub fn set_speeds(&mut self, json: &str) -> Result<(), JsError> {
        self.inner.set_speeds(serde_json::from_str(json)?);
        Ok(())
    }

    /// The speeds the stats time a plot at, as `{"draw_mm_s", "travel_mm_s", "pen_lift_s"}` JSON.
    pub fn speeds(&self) -> String {
        json!(self.inner.speeds()).to_string()
    }

    /// Cancel the latest run and start a new one over a snapshot of the session. Fails when no
    /// image is set.
    pub fn start(&mut self) -> Result<Run, JsError> {
        Ok(Run {
            inner: self.inner.start()?,
        })
    }

    /// `idle`, `running`, `done`, `cancelled` or `error`, for the latest run.
    pub fn state(&self) -> String {
        state_name(self.inner.state())
    }

    /// The error message when the latest run ended in error.
    pub fn error(&self) -> Option<String> {
        self.inner.error()
    }

    /// The last finished drawing as SVG text.
    pub fn svg(&self) -> Option<String> {
        self.inner.svg()
    }

    /// The last finished drawing in the `plotline-paths` JSON format.
    pub fn json(&self) -> Option<String> {
        self.inner.json().map(|v| v.to_string())
    }

    /// The stats of the last finished drawing as JSON.
    pub fn stats(&self) -> Option<String> {
        self.inner.stats().map(|s| json!(s).to_string())
    }

    /// The image's statistics and the suggested changes as `{"stats", "suggestions"}` JSON, or
    /// `undefined` without an image.
    pub fn analysis(&self) -> Option<String> {
        self.inner.analysis().map(|a| json!(a).to_string())
    }

    /// The last finished drawing as PNG bytes `width` pixels wide, with the travel moves and the
    /// source image when asked.
    #[wasm_bindgen(js_name = previewPng)]
    pub fn preview_png(
        &self,
        width: u32,
        travel: bool,
        source: bool,
    ) -> Result<Option<Vec<u8>>, JsError> {
        Ok(self
            .inner
            .preview_png(width, travel, source, None)
            .transpose()?)
    }

    /// The style, canvas, pens and settings as a JSON preset.
    pub fn preset(&self) -> String {
        self.inner.preset().to_string()
    }

    /// Apply a JSON preset and return a warning for each setting or stage it ignored.
    #[wasm_bindgen(js_name = loadPreset)]
    pub fn load_preset(&mut self, json: &str) -> Result<Array, JsError> {
        let preset: Value = serde_json::from_str(json)?;
        let warnings = self.inner.load_preset(&preset)?;
        Ok(warnings.iter().map(|w| JsValue::from_str(w)).collect())
    }

    /// The last finished drawing.
    pub fn drawing(&self) -> Option<DrawingHandle> {
        self.inner
            .drawing()
            .map(|drawing| DrawingHandle { drawing })
    }
}

/// One run, driven a frame at a time by [`Run::next_frame`].
#[wasm_bindgen]
pub struct Run {
    inner: plotline_core::Run,
}

#[wasm_bindgen]
impl Run {
    /// The next frame, or `undefined` once the run has ended.
    #[wasm_bindgen(js_name = nextFrame)]
    pub fn next_frame(&mut self) -> Option<FrameHandle> {
        self.inner.next_frame().map(|frame| FrameHandle { frame })
    }

    /// Ask the run to stop; the next frame is `undefined`.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// `idle`, `running`, `done`, `cancelled` or `error`.
    pub fn state(&self) -> String {
        state_name(self.inner.state())
    }

    /// The failing stage's message, once the run has ended in error.
    pub fn error(&self) -> Option<String> {
        self.inner.error()
    }
}

/// One frame of a run. The geometry getters copy into a fresh typed array on each call.
#[wasm_bindgen]
pub struct FrameHandle {
    frame: Frame,
}

#[wasm_bindgen]
impl FrameHandle {
    /// The stage's name.
    pub fn stage(&self) -> String {
        self.frame.stage.clone()
    }

    /// How far the stage has got, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.frame.progress
    }

    /// What the stage is doing, for users.
    pub fn label(&self) -> String {
        self.frame.label.clone()
    }

    /// The index of the pen whose lane gave the frame; 0 outside lanes.
    pub fn pen(&self) -> u32 {
        self.frame.pen as u32
    }

    /// Points placed so far as `x0, y0, x1, y1, ...` in mm.
    pub fn points(&self) -> Option<Float64Array> {
        let points = self.frame.points.as_ref()?;
        Some(Float64Array::from(points.0.as_flattened()))
    }

    /// The coordinates of the paths made so far, as `x0, y0, x1, y1, ...` in mm.
    pub fn coords(&self) -> Option<Float64Array> {
        let paths = self.frame.paths.as_ref()?;
        Some(Float64Array::from(paths.coords().as_flattened()))
    }

    /// Where each path made so far starts in the points of `coords`, then the point count.
    pub fn offsets(&self) -> Option<Uint32Array> {
        let paths = self.frame.paths.as_ref()?;
        Some(Uint32Array::from(paths.offsets()))
    }
}

/// A finished drawing, read a layer at a time.
#[wasm_bindgen]
pub struct DrawingHandle {
    drawing: Arc<Drawing>,
}

impl DrawingHandle {
    fn layer(&self, i: usize) -> Result<&Layer, JsError> {
        let n = self.drawing.layers.len();
        self.drawing
            .layers
            .get(i)
            .ok_or_else(|| JsError::new(&format!("no layer {i}; the drawing has {n}")))
    }
}

#[wasm_bindgen]
impl DrawingHandle {
    /// The number of layers.
    #[wasm_bindgen(js_name = layerCount)]
    pub fn layer_count(&self) -> usize {
        self.drawing.layers.len()
    }

    /// The coordinates of layer `i`'s paths, as `x0, y0, x1, y1, ...` in mm.
    #[wasm_bindgen(js_name = layerCoords)]
    pub fn layer_coords(&self, i: usize) -> Result<Float64Array, JsError> {
        let coords = self.layer(i)?.paths.coords();
        Ok(Float64Array::from(coords.as_flattened()))
    }

    /// Where each of layer `i`'s paths starts in the points of its coordinates, then the count.
    #[wasm_bindgen(js_name = layerOffsets)]
    pub fn layer_offsets(&self, i: usize) -> Result<Uint32Array, JsError> {
        Ok(Uint32Array::from(self.layer(i)?.paths.offsets()))
    }

    /// Layer `i`'s pen as `{"name", "width_mm", "color"}` JSON.
    #[wasm_bindgen(js_name = layerPen)]
    pub fn layer_pen(&self, i: usize) -> Result<String, JsError> {
        Ok(json!(self.layer(i)?.pen).to_string())
    }

    /// The page the drawing was made for, as `{"width_mm", "height_mm", "margin_mm", "fit"}` JSON.
    pub fn canvas(&self) -> String {
        json!(self.drawing.canvas).to_string()
    }

    /// The page area the tone map covers, as `{"x", "y", "w", "h", "width_px", "height_px"}` JSON
    /// with the rect in mm, or `undefined` when the drawing has no tone map.
    #[wasm_bindgen(js_name = toneRect)]
    pub fn tone_rect(&self) -> Option<String> {
        let tone = self.drawing.tone.as_ref()?;
        let [x, y, w, h] = tone.rect_mm();
        Some(
            json!({"x": x, "y": y, "w": w, "h": h, "width_px": tone.width, "height_px": tone.height})
                .to_string(),
        )
    }

    /// The tone map as `width * height` grey bytes, row by row: 255 is no ink and 0 is full ink.
    #[wasm_bindgen(js_name = toneInk)]
    pub fn tone_ink(&self) -> Option<Uint8Array> {
        let tone = self.drawing.tone.as_ref()?;
        let bytes: Vec<u8> = tone
            .ink
            .iter()
            .map(|ink| (255.0 * (1.0 - ink)).round().clamp(0.0, 255.0) as u8)
            .collect();
        Some(Uint8Array::from(&bytes[..]))
    }
}

/// The built-in styles as a JSON list of `{"name", "label", "description"}`.
#[wasm_bindgen(js_name = listStyles)]
pub fn list_styles() -> String {
    let styles: Vec<Value> = registry::list_styles()
        .iter()
        .map(|s| json!({"name": s.name, "label": s.label, "description": s.description}))
        .collect();
    Value::from(styles).to_string()
}

/// The style called `name` with each stage's settings as JSON Schema, as JSON.
#[wasm_bindgen]
pub fn describe(name: &str) -> Result<String, JsError> {
    let style =
        registry::get_style(name).ok_or_else(|| SessionError::UnknownStyle(name.to_string()))?;
    Ok(style.schema().to_string())
}

/// A `w` by `h` left-to-right gradient as RGB floats; see [`synthetic::gradient`].
#[wasm_bindgen(js_name = syntheticGradient)]
pub fn synthetic_gradient(w: u32, h: u32) -> Vec<f32> {
    synthetic::gradient(w, h).rgb
}

/// The `w` by `h` still life the GUI offers as its sample, as RGB floats; see
/// [`synthetic::still_life`].
#[wasm_bindgen(js_name = syntheticStillLife)]
pub fn synthetic_still_life(w: u32, h: u32) -> Vec<f32> {
    synthetic::still_life(w, h).rgb
}

/// A `w` by `h` checkerboard `cells` wide as RGB floats; see [`synthetic::checkerboard`].
#[wasm_bindgen(js_name = syntheticCheckerboard)]
pub fn synthetic_checkerboard(w: u32, h: u32, cells: u32) -> Result<Vec<f32>, JsError> {
    // The engine panics on these, and a panic aborts the module.
    if cells == 0 || cells > w {
        return Err(JsError::new(&format!(
            "cells must be between 1 and the width {w}, got {cells}"
        )));
    }
    Ok(synthetic::checkerboard(w, h, cells).rgb)
}

/// The pen bank as a JSON list of `{"name", "width_mm", "color", "kind", "model"}`, in catalogue
/// order, grouped by kind.
#[wasm_bindgen(js_name = listPens)]
pub fn list_pens() -> String {
    json!(pens::bank()).to_string()
}

/// The ink colours as a JSON list of `{"name", "color"}`, in display order; see [`pens::INKS`].
#[wasm_bindgen(js_name = listInks)]
pub fn list_inks() -> String {
    let inks = pens::INKS.iter();
    Value::from_iter(inks.map(|(name, color)| json!({"name": name, "color": color}))).to_string()
}
