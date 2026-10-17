//! The model stage: run a learned line model over the image and give its lines as a tone map.
//!
//! The image is placed on the page as the tone stage places it, with the same centre crop for
//! `fill` and the same resize to `resolution`, so a learned drawing lines up with every other
//! style on the same image. The model then sees it in tiles of [`TILE`] pixels. A tile keeps its
//! centre, `TILE - 2 * overlap` pixels on a side, and the margin of `overlap` pixels around it
//! only gives that centre context; pixels past the image's edge are filled by mirroring the image
//! about its edge pixel. This is the overlap-tile strategy with mirrored borders of O.
//! Ronneberger, P. Fischer and T. Brox, "U-Net: convolutional networks for biomedical image
//! segmentation", MICCAI 2015, pp. 234-241.
//!
//! One step runs one tile, so progress moves and Stop takes effect between tiles. The map's ink is
//! the model's answer: 0 for paper and 1 for a full line.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{ModelError, ModelRuntime, TILE};
use crate::settings::{defaults_for, schema_for, validate, SettingsError};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::stages::tone::{check_image, crop_to_source, resize};
use crate::types::{Canvas, Image, ToneMap};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the model stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ModelSettings {
    /// The id of a loaded model; the style picker sets it, and it is empty until one is chosen.
    pub model: String,
    /// Length of the image's longer edge in pixels as the model sees it; more pixels find finer lines but run slower.
    #[schemars(range(min = 256, max = 2048), extend("x-unit" = "px"))]
    pub resolution: u32,
    /// Swap line and paper in the model's answer, for a model file of your own that draws white lines on black.
    pub invert: bool,
}

impl Default for ModelSettings {
    fn default() -> Self {
        ModelSettings {
            model: String::new(),
            resolution: 1024,
            invert: false,
        }
    }
}

/// Run the chosen model over the image in tiles and give the lines it finds as a tone map on the
/// canvas.
pub struct ModelStage;

impl Stage for ModelStage {
    fn name(&self) -> &'static str {
        "model"
    }
    fn label(&self) -> &'static str {
        "Model"
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
        schema_for::<ModelSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<ModelSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<ModelSettings>(self.name(), &self.schema(), settings)?;
        let wrong = |message: String| {
            StageError::from(SettingsError::Invalid {
                stage: self.name().to_string(),
                key: Some("model".to_string()),
                message,
            })
        };
        if s.model.is_empty() {
            return Err(wrong(
                "choose a model; get one from Styles, then Get models, or run \
                 `plotline models list`"
                    .to_string(),
            ));
        }
        let Some(info) = ctx.models().get(&s.model) else {
            let id = &s.model;
            // A file of your own is not in the catalogue, so `get` cannot fetch it again.
            return Err(wrong(if id.starts_with("file-") {
                format!(
                    "model {id} is not loaded; it is a file of your own, so load it again with \
                     Load your own, or run `plotline models add <file>`"
                )
            } else {
                format!(
                    "model {id} is not loaded; get it from Styles, then Get models, or run \
                     `plotline models get {id}`"
                )
            }));
        };
        let runtime = ctx
            .models()
            .runtime()
            .ok_or_else(|| StageError::Failed(ModelError::NoRuntime.to_string()))?;
        let (image, canvas) = (inputs.image(), inputs.canvas());
        check_image(image)?;
        let res = s.resolution as usize;
        let [(r, w, h), (g, _, _), (b, _, _)] = [0, 1, 2].map(|c| plane(image, canvas, c, res));
        let (mm_per_px, origin_mm) = canvas.image_transform(w as u32, h as u32);
        Ok(Box::new(ModelRun {
            ctx,
            runtime,
            id: s.model,
            planes: [r, g, b],
            size: (w, h),
            overlap: info.overlap,
            invert: s.invert,
            tiles: tiles(w, h, info.overlap),
            next: 0,
            ink: vec![0.0; w * h],
            place: (mm_per_px, origin_mm),
        }))
    }
}

/// Colour channel `channel` of `image`, cut to the canvas's crop and for `fill` to the drawing's
/// aspect, and resized to `resolution` on the long side, as the tone stage does for luminance.
/// Returns the plane and its width and height.
fn plane(
    image: &Image,
    canvas: &Canvas,
    channel: usize,
    resolution: usize,
) -> (Vec<f32>, usize, usize) {
    let (w, h) = (image.width as usize, image.height as usize);
    let plane: Vec<f32> = image.rgb.iter().skip(channel).step_by(3).copied().collect();
    let (plane, w, h) = crop_to_source(&plane, w, h, canvas);
    resize(&plane, w, h, resolution)
}

/// The side, in pixels, of the part of a tile that is kept.
fn centre(overlap: usize) -> usize {
    TILE.saturating_sub(2 * overlap).max(1)
}

/// The top-left corners, in row-major order, of the kept centres that cover a `w x h` map: each
/// pixel lies in exactly one centre, and a centre on the right or bottom edge may run past it.
pub(crate) fn tiles(w: usize, h: usize, overlap: usize) -> Vec<[usize; 2]> {
    let step = centre(overlap);
    (0..h)
        .step_by(step)
        .flat_map(|y| (0..w).step_by(step).map(move |x| [x, y]))
        .collect()
}

/// The index in `0..n` that `i` falls on when `0..n` is reflected about its end pixels, as in
/// `2 1 0 1 2 .. n-1 n-2 ..` and repeating while still out of range.
fn mirror(i: isize, n: usize) -> usize {
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let i = i.rem_euclid(period as isize) as usize;
    i.min(period - i)
}

/// The run: it holds the resized planes and the map under construction, and does a tile a step.
struct ModelRun {
    ctx: RunContext,
    runtime: Arc<dyn ModelRuntime>,
    id: String,
    /// The red, green and blue planes, each `size.0 x size.1`.
    planes: [Vec<f32>; 3],
    size: (usize, usize),
    overlap: usize,
    invert: bool,
    tiles: Vec<[usize; 2]>,
    /// The tiles done.
    next: usize,
    /// The map so far, row-major.
    ink: Vec<f32>,
    /// `mm_per_px` and `origin_mm` of the finished map.
    place: (f64, [f64; 2]),
}

impl StageRun for ModelRun {
    fn step(&mut self) -> Result<Step, StageError> {
        let (w, h) = self.size;
        let Some(&[cx, cy]) = self.tiles.get(self.next) else {
            let (mm_per_px, origin_mm) = self.place;
            return Ok(Step::Done(Artifact::Tone(Arc::new(ToneMap {
                width: w as u32,
                height: h as u32,
                ink: std::mem::take(&mut self.ink),
                mm_per_px,
                origin_mm,
                // A model's line map has no midtone curve.
                gamma: 1.0,
            }))));
        };
        // The window's pixel `t` is the image's `c - overlap + t`, mirrored where it falls outside.
        let at = |c: usize, t: usize, n: usize| mirror((c + t) as isize - self.overlap as isize, n);
        let cols: Vec<usize> = (0..TILE).map(|t| at(cx, t, w)).collect();
        let rows: Vec<usize> = (0..TILE).map(|t| at(cy, t, h) * w).collect();
        let mut rgb = Vec::with_capacity(3 * TILE * TILE);
        for plane in &self.planes {
            for &row in &rows {
                rgb.extend(cols.iter().map(|&col| plane[row + col]));
            }
        }
        let out = self
            .runtime
            .infer(&self.id, &rgb)
            .map_err(|e| StageError::Failed(e.to_string()))?;
        if out.len() != TILE * TILE {
            return Err(StageError::Failed(format!(
                "the model gave {} values for a {TILE} x {TILE} tile",
                out.len()
            )));
        }
        let o = self.overlap;
        for dy in 0..centre(o).min(h - cy) {
            for dx in 0..centre(o).min(w - cx) {
                let v = out[(o + dy) * TILE + o + dx];
                let v = if self.invert { 1.0 - v } else { v };
                // A NaN from a broken model draws as paper.
                self.ink[(cy + dy) * w + cx + dx] =
                    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
            }
        }
        self.next += 1;
        if self.ctx.cancelled() {
            return Err(StageError::Cancelled);
        }
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress: self.next as f32 / self.tiles.len() as f32,
            // The stage's name goes in front wherever the label is shown.
            label: format!("tile {} of {}", self.next, self.tiles.len()),
            points: None,
            paths: None,
            pen: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::StageCache;
    use crate::models::{ModelStore, StandIn, STANDIN_BYTES, STANDIN_FLOOR_BYTES};
    use crate::runner::{Run, RunState, Shared};
    use crate::session::Session;
    use crate::stages::ink::InkStage;
    use crate::stages::tone::ToneStage;
    use crate::style::Style;
    use crate::synthetic::{flat, gradient, still_life};
    use crate::test_util::{black, drain};
    use crate::types::Fit;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::Mutex;

    /// A model store holding the stand-in Sobel model, and that model's id.
    fn standin() -> (Arc<ModelStore>, String) {
        let mut session = Session::new(1 << 20);
        session.set_model_runtime(Arc::new(StandIn::default()));
        let id = session.add_model(None, STANDIN_BYTES).unwrap().id;
        (session.model_store(), id)
    }

    fn page(fit: Fit) -> Canvas {
        Canvas::new(210.0, 297.0, 10.0, fit).unwrap()
    }

    /// Run `stage` over `image` on `canvas` to the end, with `models` in its context.
    fn run_stage(
        stage: &dyn Stage,
        image: &Image,
        canvas: &Canvas,
        settings: Value,
        models: Arc<ModelStore>,
    ) -> Result<(Vec<Frame>, Arc<ToneMap>), StageError> {
        let inputs = Inputs::new(HashMap::from([
            (Kind::Image, Artifact::Image(Arc::new(image.clone()))),
            (Kind::Canvas, Artifact::Canvas(Arc::new(canvas.clone()))),
        ]));
        let ctx = RunContext::new(Arc::default()).with_models(models);
        let mut run = stage.start(inputs, &settings, ctx)?;
        let mut frames = Vec::new();
        loop {
            match run.step()? {
                Step::Frame(f) => frames.push(f),
                Step::Done(Artifact::Tone(t)) => return Ok((frames, t)),
                Step::Done(a) => panic!("expected a tone map, got {}", a.kind().name()),
            }
        }
    }

    /// The map the model stage makes with the stand-in model at `resolution`.
    fn learned(image: &Image, resolution: u32, patch: Value) -> Arc<ToneMap> {
        let (models, id) = standin();
        let mut settings = json!({"model": id, "resolution": resolution});
        settings
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        run_stage(&ModelStage, image, &page(Fit::Fit), settings, models)
            .unwrap()
            .1
    }

    #[test]
    fn tiles_cover_the_map_once() {
        for (w, h, overlap) in [
            (1, 1, 64),
            (384, 384, 64),
            (385, 384, 64),
            (1024, 768, 32),
            (4000, 1, 64),
        ] {
            let centre = TILE - 2 * overlap;
            let mut hits = vec![0u8; w * h];
            for [x, y] in tiles(w, h, overlap) {
                for yy in y..(y + centre).min(h) {
                    for xx in x..(x + centre).min(w) {
                        hits[yy * w + xx] += 1;
                    }
                }
            }
            assert!(hits.iter().all(|&n| n == 1), "{w} x {h}, overlap {overlap}");
        }
        // Row-major: along a row, then down.
        assert_eq!(tiles(384, 384, 64), [[0, 0]]);
        assert_eq!(
            tiles(385, 385, 64),
            [[0, 0], [384, 0], [0, 384], [384, 384]]
        );
    }

    #[test]
    fn mirroring_reflects_about_the_edge_pixel() {
        let row = |n| (-3..8).map(|i| mirror(i, n)).collect::<Vec<_>>();
        assert_eq!(row(5), [3, 2, 1, 0, 1, 2, 3, 4, 3, 2, 1]);
        assert_eq!(row(2), [1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1]);
        assert_eq!(row(1), [0; 11]);
    }

    #[test]
    fn the_map_lines_up_with_the_tone_map() {
        let (models, id) = standin();
        let image = gradient(400, 300);
        for fit in [Fit::Fit, Fit::Fill] {
            let canvas = page(fit);
            let settings = json!({"model": id, "resolution": 300});
            let (_, line) =
                run_stage(&ModelStage, &image, &canvas, settings, models.clone()).unwrap();
            let settings = json!({"resolution": 300});
            let (_, tone) =
                run_stage(&ToneStage, &image, &canvas, settings, Arc::default()).unwrap();
            assert_eq!((line.width, line.height), (tone.width, tone.height));
            assert_eq!(line.mm_per_px, tone.mm_per_px);
            assert_eq!(line.origin_mm, tone.origin_mm);
        }
    }

    #[test]
    fn an_empty_model_setting_is_a_settings_error() {
        let (models, _) = standin();
        let err = run_stage(
            &ModelStage,
            &gradient(8, 8),
            &page(Fit::Fit),
            json!({}),
            models,
        )
        .unwrap_err();
        let StageError::Settings(SettingsError::Invalid { message, .. }) = err else {
            panic!("{err:?}");
        };
        assert_eq!(
            message,
            "choose a model; get one from Styles, then Get models, or run `plotline models list`"
        );
    }

    #[test]
    fn a_missing_model_is_a_settings_error() {
        let models = Arc::new(ModelStore::default());
        let settings = json!({"model": "contour-1"});
        let err = run_stage(
            &ModelStage,
            &gradient(8, 8),
            &page(Fit::Fit),
            settings,
            models,
        )
        .unwrap_err();
        let StageError::Settings(SettingsError::Invalid { message, .. }) = err else {
            panic!("{err:?}");
        };
        assert_eq!(
            message,
            "model contour-1 is not loaded; get it from Styles, then Get models, or run \
             `plotline models get contour-1`"
        );
    }

    #[test]
    fn a_missing_file_of_your_own_says_how_to_load_it() {
        // `plotline models get` knows only the catalogue's models, so it is no advice here.
        let models = Arc::new(ModelStore::default());
        let settings = json!({"model": "file-0123456789ab"});
        let err = run_stage(
            &ModelStage,
            &gradient(8, 8),
            &page(Fit::Fit),
            settings,
            models,
        )
        .unwrap_err();
        let StageError::Settings(SettingsError::Invalid { message, .. }) = err else {
            panic!("{err:?}");
        };
        assert_eq!(
            message,
            "model file-0123456789ab is not loaded; it is a file of your own, so load it again \
             with Load your own, or run `plotline models add <file>`"
        );
    }

    #[test]
    fn one_step_runs_one_tile() {
        let (models, id) = standin();
        let image = Image {
            width: 1024,
            height: 768,
            rgb: vec![0.5; 1024 * 768 * 3],
        };
        let settings = json!({"model": id, "resolution": 1024});
        let (frames, _) =
            run_stage(&ModelStage, &image, &page(Fit::Fit), settings, models).unwrap();
        assert_eq!(frames.len(), tiles(1024, 768, 64).len());
        assert!(frames.windows(2).all(|w| w[0].progress < w[1].progress));
        assert_eq!(frames.last().unwrap().progress, 1.0);
        // The label follows the stage name wherever it is shown, so it holds no `model` of its own.
        let total = frames.len();
        for (i, f) in frames.iter().enumerate() {
            assert_eq!(f.label, format!("tile {} of {total}", i + 1));
        }
    }

    #[test]
    fn invert_flips_the_map() {
        let image = still_life(200, 150);
        let plain = learned(&image, 256, json!({}));
        let flipped = learned(&image, 256, json!({"invert": true}));
        assert!(
            plain.ink.iter().any(|&v| v > 0.2),
            "the model found no lines"
        );
        assert!(plain
            .ink
            .iter()
            .zip(&flipped.ink)
            .all(|(a, b)| (a + b - 1.0).abs() < 1e-6));
    }

    #[test]
    fn a_blank_answer_passes_through_unchanged() {
        // The stage adds no floor and removes none: a flat picture maps to what the model says
        // for blank paper, 0 for the plain stand-in and 0.5 for the one with a floor.
        for (bytes, want) in [(STANDIN_BYTES, 0.0), (STANDIN_FLOOR_BYTES, 0.5)] {
            let mut session = Session::new(1 << 20);
            session.set_model_runtime(Arc::new(StandIn::default()));
            let id = session.add_model(None, bytes).unwrap().id;
            let settings = json!({"model": id, "resolution": 300});
            let (_, map) = run_stage(
                &ModelStage,
                &flat(300, 200, 1.0),
                &page(Fit::Fit),
                settings,
                session.model_store(),
            )
            .unwrap();
            assert!(map.ink.iter().all(|&v| v == want), "floor {want}");
        }
    }

    #[test]
    fn seams_match_a_whole_run() {
        let image = still_life(600, 400);
        let (w, h) = (600, 400);
        let map = learned(&image, 600, json!({}));
        assert_eq!((map.width as usize, map.height as usize), (w, h));
        // The stand-in's Sobel over the whole resized image, borders mirrored as the tiler does.
        let plane = |c: usize| {
            let plane: Vec<f32> = image.rgb.iter().skip(c).step_by(3).copied().collect();
            resize(&plane, w, h, 600).0
        };
        let (r, g, b) = (plane(0), plane(1), plane(2));
        let luma: Vec<f32> = (0..w * h)
            .map(|i| 0.299 * r[i] + 0.587 * g[i] + 0.114 * b[i])
            .collect();
        let at = |x: isize, y: isize| luma[mirror(y, h) * w + mirror(x, w)];
        let mut worst = 0.0f32;
        for y in 0..h as isize {
            for x in 0..w as isize {
                let gx = (at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1))
                    - (at(x - 1, y - 1) + 2.0 * at(x - 1, y) + at(x - 1, y + 1));
                let gy = (at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1))
                    - (at(x - 1, y - 1) + 2.0 * at(x, y - 1) + at(x + 1, y - 1));
                let want = (gx * gx + gy * gy).sqrt().clamp(0.0, 1.0);
                let got = map.ink[y as usize * w + x as usize];
                worst = worst.max((got - want).abs());
            }
        }
        assert!(map.ink.iter().any(|&v| v > 0.2), "the model found no lines");
        assert!(worst < 1e-6, "worst seam error {worst}");
    }

    #[test]
    fn tiny_and_extreme_images_draw() {
        for (w, h) in [(1, 1), (4000, 1)] {
            let image = Image {
                width: w,
                height: h,
                rgb: (0..w * h * 3).map(|i| (i % 7) as f32 / 6.0).collect(),
            };
            let map = learned(&image, 256, json!({}));
            assert_eq!(map.ink.len(), (map.width * map.height) as usize);
            assert!(map
                .ink
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn a_cancelled_run_caches_nothing() {
        let (models, id) = standin();
        let style = Arc::new(
            Style::new(
                "t",
                "T",
                "d",
                vec![Arc::new(ModelStage), Arc::new(InkStage)],
            )
            .unwrap(),
        );
        let mut settings = style.defaults();
        settings["model"] = json!({"model": id, "resolution": 800});
        let roots = || {
            [
                Artifact::Image(Arc::new(still_life(800, 600))),
                Artifact::Canvas(Arc::new(page(Fit::Fit))),
                Artifact::Pens(Arc::new(vec![black()])),
            ]
        };
        let make = |cache: &Arc<Mutex<StageCache>>, cancel: Arc<AtomicBool>| {
            let shared = Arc::new(Mutex::new(Shared::default()));
            let run = Run::new(
                style.clone(),
                settings.clone(),
                roots(),
                cache.clone(),
                shared.clone(),
                cancel,
                0,
            )
            .with_models(models.clone());
            (run, shared)
        };
        let fresh = || Arc::new(Mutex::new(StageCache::new(1 << 26)));
        let tone_of = |shared: &Arc<Mutex<Shared>>| {
            shared
                .lock()
                .unwrap()
                .drawing
                .clone()
                .unwrap()
                .tone
                .clone()
                .unwrap()
        };

        let cache = fresh();
        let (mut run, _) = make(&cache, Arc::default());
        let frame = run.next_frame().unwrap();
        assert_eq!(
            (frame.stage.as_str(), frame.label.as_str()),
            ("model", "tile 1 of 6")
        );
        run.cancel();
        assert!(run.next_frame().is_none());
        assert_eq!(run.state(), RunState::Cancelled);
        assert!(cache.lock().unwrap().is_empty());

        // A flag set while a tile runs ends the stage at that tile, whatever drives it.
        let [image, canvas, _] = roots();
        let inputs = Inputs::new(HashMap::from([
            (Kind::Image, image),
            (Kind::Canvas, canvas),
        ]));
        let ctx = RunContext::new(Arc::new(AtomicBool::new(true))).with_models(models.clone());
        let mut stage = ModelStage.start(inputs, &settings["model"], ctx).unwrap();
        assert!(matches!(stage.step(), Err(StageError::Cancelled)));

        let (mut again, shared) = make(&cache, Arc::default());
        drain(&mut again);
        assert_eq!(again.state(), RunState::Done);
        let (mut whole, whole_shared) = make(&fresh(), Arc::default());
        drain(&mut whole);
        assert_eq!(tone_of(&shared).ink, tone_of(&whole_shared).ink);
    }
}
