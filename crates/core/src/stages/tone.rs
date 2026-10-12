//! The tone stage: turn the RGB image into a tone map on the canvas.
//!
//! Luminance uses the ITU-R BT.601 weights, Y = 0.299 R + 0.587 G + 0.114 B (Recommendation
//! ITU-R BT.601, "Studio encoding parameters of digital television for standard 4:3 and
//! wide-screen 16:9 aspect ratios"). A source much larger than the tone map is averaged in blocks
//! before it is sampled bilinearly, so fine detail is averaged rather than skipped.
//!
//! Every step runs in f32 in the Python engine's order, with each setting rounded to f32 where
//! numpy rounds it, so both engines give the same values.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Canvas, Fit, Image, ToneMap};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the tone stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ToneSettings {
    /// Length of the tone map's longer edge in pixels; more pixels follow the image more closely but run slower.
    #[schemars(range(min = 64, max = 4000), extend("x-unit" = "px"))]
    pub resolution: u32,
    /// Swap light and dark, so that light areas get the ink.
    pub invert: bool,
    /// Luminance at or below this level becomes black, which takes full ink.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub black: f64,
    /// Luminance at or above this level becomes white, which takes no ink.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub white: f64,
    /// Midtone curve applied to the luminance: above 1 darkens the midtones, below 1 lightens them.
    #[schemars(range(min = 0.1, max = 5.0), extend("x-step" = 0.05))]
    pub gamma: f64,
    /// Contrast around mid-grey: positive values push tones apart, negative values squeeze them towards mid-grey.
    #[schemars(range(min = -1.0, max = 1.0), extend("x-step" = 0.05))]
    pub contrast: f64,
    /// Gaussian blur, given as sigma in mm, that smooths the tone map; 0 turns it off.
    #[schemars(
        range(min = 0.0, max = 10.0),
        extend("x-step" = 0.1, "x-unit" = "mm", "x-advanced" = true)
    )]
    pub blur_mm: f64,
}

impl Default for ToneSettings {
    fn default() -> Self {
        ToneSettings {
            resolution: 1000,
            invert: false,
            black: 0.0,
            white: 1.0,
            gamma: 1.0,
            contrast: 0.0,
            blur_mm: 0.0,
        }
    }
}

/// Turn the image into a tone map. In order: luminance, a centre crop for `fill`, resize, levels,
/// gamma, contrast, invert and blur.
pub struct ToneStage;

impl Stage for ToneStage {
    fn name(&self) -> &'static str {
        "tone"
    }
    fn label(&self) -> &'static str {
        "Tone"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Image, Kind::Canvas]
    }
    fn provides(&self) -> Kind {
        Kind::Tone
    }
    fn schema(&self) -> Value {
        schema_for::<ToneSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<ToneSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let settings = validate::<ToneSettings>(self.name(), &self.schema(), settings)?;
        let (image, canvas) = (inputs.image().clone(), inputs.canvas().clone());
        // Image's fields are public, so check here what the resize would otherwise panic on.
        let (w, h, n) = (image.width as usize, image.height as usize, image.rgb.len());
        if w == 0 || h == 0 || n != w * h * 3 {
            return Err(StageError::Failed(format!(
                "the image is {w} x {h} pixels with {n} values; it needs at least one pixel \
                 and 3 values per pixel"
            )));
        }
        Ok(Box::new(ToneRun {
            input: Some((image, canvas, settings)),
            map: None,
        }))
    }
}

/// The work happens in the first step, which reports one frame; the second gives the map.
struct ToneRun {
    /// What the first step reads; `None` once it has run.
    input: Option<(Arc<Image>, Arc<Canvas>, ToneSettings)>,
    /// The finished map, between the first step and the second.
    map: Option<ToneMap>,
}

impl StageRun for ToneRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some((image, canvas, settings)) = self.input.take() {
            self.map = Some(tone_map(&image, &canvas, &settings));
            return Ok(Step::Frame(Frame {
                stage: String::new(),
                progress: 1.0,
                label: "tone map".into(),
                points: None,
                paths: None,
            }));
        }
        let map = self.map.take().expect("the tone run has already finished");
        Ok(Step::Done(Artifact::Tone(Arc::new(map))))
    }
}

/// The tone map for `image` on `canvas`, in the order the module documents.
fn tone_map(image: &Image, canvas: &Canvas, s: &ToneSettings) -> ToneMap {
    let (mut lum, mut w, mut h) = (
        image.luminance(),
        image.width as usize,
        image.height as usize,
    );
    if canvas.fit == Fit::Fill {
        let [_, _, dw, dh] = canvas.drawing_rect();
        (lum, w, h) = crop_to_aspect(&lum, w, h, dw / dh);
    }
    let (y, tw, th) = resize(&lum, w, h, s.resolution as usize);
    // numpy takes `max(white - black, 1e-6)` and `1 + contrast` in f64 and rounds the result to
    // f32; white <= black makes the levels a hard threshold at black instead of dividing by zero.
    let (black, span) = (s.black as f32, (s.white - s.black).max(1e-6) as f32);
    let (gamma, gain) = (s.gamma as f32, (1.0 + s.contrast) as f32);
    let ink: Vec<f32> = y
        .iter()
        .map(|&v| {
            let v = ((v - black) / span).clamp(0.0, 1.0);
            let v = math::powf32(v, gamma);
            let v = (0.5 + (v - 0.5) * gain).clamp(0.0, 1.0);
            let ink = 1.0 - v;
            if s.invert {
                1.0 - ink
            } else {
                ink
            }
        })
        .collect();
    let (tw32, th32) = (tw as u32, th as u32);
    let (mm_per_px, origin_mm) = canvas.image_transform(tw32, th32);
    let ink = if s.blur_mm > 0.0 {
        math::blur(&ink, tw, th, s.blur_mm / mm_per_px)
    } else {
        ink
    };
    ToneMap {
        width: tw32,
        height: th32,
        ink: ink.into_iter().map(|v| v.clamp(0.0, 1.0)).collect(),
        mm_per_px,
        origin_mm,
    }
}

/// Centre-crop a row-major `w x h` grid to a width over height of `aspect`, to the nearest pixel
/// with halves to even as Python's `round` does. Returns the crop and its width and height.
fn crop_to_aspect(lum: &[f32], w: usize, h: usize, aspect: f64) -> (Vec<f32>, usize, usize) {
    let ch = h.min(((w as f64 / aspect).round_ties_even() as usize).max(1));
    let cw = w.min(((h as f64 * aspect).round_ties_even() as usize).max(1));
    let (y0, x0) = ((h - ch) / 2, (w - cw) / 2);
    let rows = lum[y0 * w..(y0 + ch) * w].chunks_exact(w);
    let crop = rows.flat_map(|row| &row[x0..x0 + cw]).copied().collect();
    (crop, cw, ch)
}

/// Resize a row-major `w x h` grid to `long_edge` pixels on the longer side, keeping the aspect
/// ratio (halves to even) and at least one pixel on the other side. A source at least twice the
/// target is first averaged in `k x k` blocks, `k = max(w, h) / long_edge`, then resampled.
/// Returns the grid and its width and height.
fn resize(lum: &[f32], w: usize, h: usize, long_edge: usize) -> (Vec<f32>, usize, usize) {
    let short = |other: usize, long: usize| {
        let exact = (other * long_edge) as f64 / long as f64;
        (exact.round_ties_even() as usize).max(1)
    };
    let (tw, th) = if h >= w {
        (short(w, h), long_edge)
    } else {
        (long_edge, short(h, w))
    };
    let k = w.max(h) / long_edge;
    if k >= 2 {
        let (blocks, bw, bh) = math::block_average(lum, w, h, k);
        return (math::resample(&blocks, bw, bh, tw, th), tw, th);
    }
    (math::resample(lum, w, h, tw, th), tw, th)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::merge;
    use crate::synthetic::{checkerboard, flat, gradient, noise};
    use serde_json::json;
    use std::collections::HashMap;

    const SQUARE: Canvas = Canvas {
        width_mm: 100.0,
        height_mm: 100.0,
        margin_mm: 0.0,
        fit: Fit::Fit,
    };

    /// Run the tone stage to the end with `patch` over the defaults; the resolution is 64 unless
    /// the patch gives one.
    fn run_tone(image: Image, canvas: Canvas, mut patch: Value) -> (Vec<Frame>, Arc<ToneMap>) {
        if patch.get("resolution").is_none() {
            patch["resolution"] = json!(64);
        }
        let settings = merge(&ToneStage.defaults(), &patch);
        let inputs = Inputs::new(HashMap::from([
            (Kind::Image, Artifact::Image(Arc::new(image))),
            (Kind::Canvas, Artifact::Canvas(Arc::new(canvas))),
        ]));
        let ctx = RunContext::new(Arc::default());
        let mut run = ToneStage.start(inputs, &settings, ctx).unwrap();
        let mut frames = Vec::new();
        loop {
            match run.step().unwrap() {
                Step::Frame(f) => frames.push(f),
                Step::Done(Artifact::Tone(t)) => return (frames, t),
                Step::Done(a) => panic!("expected a tone map, got {}", a.kind().name()),
            }
        }
    }

    #[test]
    fn shape_and_transform() {
        let (frames, t) = run_tone(gradient(64, 48), SQUARE, json!({}));
        assert_eq!((t.width, t.height), (64, 48));
        assert!((t.mm_per_px - 1.5625).abs() < 1e-12 && t.origin_mm == [0.0, 12.5]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].progress, 1.0);
    }
    #[test]
    fn gradient_ink_falls_left_to_right() {
        let (_, t) = run_tone(gradient(64, 48), SQUARE, json!({}));
        let row = &t.ink[12 * 64..13 * 64];
        assert!(row[0] > 0.9 && row[63] < 0.1 && row.windows(2).all(|w| w[1] <= w[0] + 1e-6));
    }
    #[test]
    fn invert_levels_gamma_contrast() {
        let (_, a) = run_tone(gradient(64, 48), SQUARE, json!({}));
        let (_, b) = run_tone(gradient(64, 48), SQUARE, json!({"invert": true}));
        assert!(a
            .ink
            .iter()
            .zip(&b.ink)
            .all(|(x, y)| (x + y - 1.0).abs() < 1e-6));
        assert!(run_tone(gradient(64, 48), SQUARE, json!({"black": 0.5}))
            .1
            .ink[12 * 64..12 * 64 + 14]
            .iter()
            .all(|v| *v > 0.99));
        assert!(
            (run_tone(flat(16, 16, 0.5), SQUARE, json!({"gamma": 2.0}))
                .1
                .ink[0]
                - 0.75)
                .abs()
                < 1e-6
        );
        assert!(
            run_tone(flat(16, 16, 0.75), SQUARE, json!({"contrast": 1.0}))
                .1
                .ink[0]
                .abs()
                < 1e-6
        );
        assert!(
            (run_tone(flat(16, 16, 0.75), SQUARE, json!({"contrast": -1.0}))
                .1
                .ink[0]
                - 0.5)
                .abs()
                < 1e-6
        );
    }
    #[test]
    fn adjustments_apply_in_the_documented_order() {
        let kw = json!({"black": 0.2, "white": 0.8, "gamma": 2.0, "contrast": 0.5});
        assert!(
            (run_tone(flat(16, 16, 0.6), SQUARE, kw.clone()).1.ink[0] - 7.0 / 12.0).abs() < 1e-5
        );
        let mut inv = kw.clone();
        inv["invert"] = json!(true);
        assert!((run_tone(flat(16, 16, 0.6), SQUARE, inv).1.ink[0] - 5.0 / 12.0).abs() < 1e-5);
        assert!(
            (run_tone(
                flat(16, 16, 0.25),
                SQUARE,
                json!({"black": 0.5, "gamma": 2.0})
            )
            .1
            .ink[0]
                - 1.0)
                .abs()
                < 1e-6
        );
    }
    #[test]
    fn white_not_above_black_is_a_hard_threshold() {
        for white in [0.5, 0.25] {
            let ink = run_tone(
                gradient(64, 48),
                SQUARE,
                json!({"black": 0.5, "white": white}),
            )
            .1
            .ink
            .clone();
            assert!(
                ink[12 * 64..12 * 64 + 32].iter().all(|v| *v == 1.0)
                    && ink[12 * 64 + 32..13 * 64].iter().all(|v| *v == 0.0)
            );
        }
    }
    #[test]
    fn fill_crops_to_the_drawing_aspect() {
        let (_, t) = run_tone(
            flat(64, 64, 0.5),
            Canvas::new(100.0, 50.0, 0.0, Fit::Fill).unwrap(),
            json!({}),
        );
        assert_eq!((t.width, t.height), (64, 32));
        assert_eq!(t.rect_mm(), [0.0, 0.0, 100.0, 50.0]);
    }
    #[test]
    fn fill_keeps_the_centre_of_a_wide_image_for_a_tall_page() {
        let (_, t) = run_tone(
            gradient(100, 50),
            Canvas::new(50.0, 100.0, 0.0, Fit::Fill).unwrap(),
            json!({}),
        );
        assert_eq!((t.width, t.height), (32, 64));
        assert!(
            (t.ink[0] - (1.0 - 37.0 / 99.0)).abs() < 1e-3
                && (t.ink[31] - (1.0 - 61.0 / 99.0)).abs() < 1e-3
        );
    }
    #[test]
    fn extreme_aspect_ratios_keep_their_values() {
        let (_, t) = run_tone(gradient(4000, 1), SQUARE, json!({"resolution": 100}));
        assert_eq!((t.width, t.height), (100, 1));
        for j in 0..100 {
            assert!((t.ink[j] - (1.0 - (40.0 * j as f32 + 19.5) / 3999.0)).abs() < 1e-5);
        }
        assert_eq!(
            {
                let t = run_tone(flat(300, 200, 0.5), SQUARE, json!({"resolution": 100})).1;
                (t.width, t.height)
            },
            (100, 67)
        );
        assert_eq!(
            {
                let t = run_tone(flat(200, 300, 0.5), SQUARE, json!({"resolution": 100})).1;
                (t.width, t.height)
            },
            (67, 100)
        );
        assert_eq!(
            {
                let t = run_tone(flat(4000, 1, 0.5), SQUARE, json!({"resolution": 100})).1;
                (t.width, t.height)
            },
            (100, 1)
        );
    }
    #[test]
    fn downscaling_averages_rather_than_skips() {
        let (_, t) = run_tone(checkerboard(256, 256, 128), SQUARE, json!({}));
        let mean = t.ink.iter().sum::<f32>() / t.ink.len() as f32;
        assert!((mean - 0.5).abs() < 0.05);
    }
    #[test]
    fn blur_is_measured_in_mm_and_lowers_variation() {
        // Squares by multiplying, since the source scan forbids the float power methods here.
        let std = |t: &ToneMap| {
            let m = t.ink.iter().sum::<f32>() / t.ink.len() as f32;
            (t.ink.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / t.ink.len() as f32).sqrt()
        };
        let cb = checkerboard(64, 64, 8);
        let plain = std(&run_tone(cb.clone(), SQUARE, json!({})).1);
        let small = std(&run_tone(cb.clone(), SQUARE, json!({"blur_mm": 3.0})).1);
        let big = std(&run_tone(
            cb,
            Canvas::new(200.0, 200.0, 0.0, Fit::Fit).unwrap(),
            json!({"blur_mm": 3.0}),
        )
        .1);
        assert!(small < plain && big > small);
    }
    #[test]
    fn synthetic_images_have_the_documented_values() {
        let g = gradient(64, 48);
        assert_eq!(g.rgb[0], 0.0);
        assert_eq!(g.rgb[63 * 3], 1.0);
        let c = checkerboard(64, 48, 8);
        assert_eq!(c.rgb[0], 0.0);
        assert_eq!(c.rgb[8 * 3], 1.0);
        assert_eq!(noise(4, 4, 1).rgb, noise(4, 4, 1).rgb);
        assert_ne!(noise(4, 4, 1).rgb, noise(4, 4, 2).rgb);
    }
    #[test]
    fn an_image_without_pixels_or_with_the_wrong_buffer_is_an_error() {
        let wrong_buffer = Image {
            width: 2,
            height: 2,
            rgb: vec![0.5; 3],
        };
        for image in [flat(0, 4, 0.5), flat(4, 0, 0.5), wrong_buffer] {
            let inputs = Inputs::new(HashMap::from([
                (Kind::Image, Artifact::Image(Arc::new(image))),
                (Kind::Canvas, Artifact::Canvas(Arc::new(SQUARE))),
            ]));
            let ctx = RunContext::new(Arc::default());
            let err = ToneStage
                .start(inputs, &ToneStage.defaults(), ctx)
                .err()
                .unwrap();
            assert!(matches!(err, StageError::Failed(_)), "{err:?}");
        }
    }
}
