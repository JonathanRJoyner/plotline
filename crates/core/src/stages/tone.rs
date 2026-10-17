//! The tone stage: turn the RGB image into a tone map on the canvas.
//!
//! Luminance uses the ITU-R BT.601 weights, Y = 0.299 R + 0.587 G + 0.114 B (Recommendation
//! ITU-R BT.601, "Studio encoding parameters of digital television for standard 4:3 and
//! wide-screen 16:9 aspect ratios"). A source much larger than the tone map is averaged in blocks
//! before it is sampled bilinearly, so fine detail is averaged rather than skipped.
//!
//! With `flatten_mm` set, the paper's own tone is divided out of the resized map before the
//! levels; see [`flatten`].
//!
//! With `smooth_mm` set, the map is then smoothed with the guided filter of K. He, J. Sun and
//! X. Tang, which averages away noise and paper grain but keeps edges; see [`guided`].
//!
//! With `levels` auto, the black and white points come from the resized map: the single-channel
//! case of the "Simplest Color Balance" (N. Limare, J.-L. Lisani, J.-M. Morel, A. B. Petro and
//! C. Sbert, *Image Processing On Line* 1, 2011, doi:10.5201/ipol.2011.llmps-scb), written from
//! the paper's description. See [`crate::analysis::levels`].
//!
//! With `response` auto, the run measures the style's own tone response on a step wedge and gives
//! the later stages the tone map through the inverse lookup; see [`crate::response`]. The stage
//! itself makes the same map either way, which is the one the drawing keeps.
//!
//! With `focus` points, the map is blurred more the further a pixel lies from the nearest point,
//! so the detail the drawing keeps sits near the points. D. DeCarlo and A. Santella, "Stylization
//! and abstraction of photographs", *ACM Transactions on Graphics* 21(3), 2002, pp. 769-776,
//! doi:10.1145/566654.566650, keep detail near where a viewer looked and remove more of it with
//! distance; here the abstraction is a blur whose strength grows with that distance, written from
//! the paper's description, and the points are chosen by a person or an assistant.
//!
//! Every step runs in f32 in the Python engine's order, with each setting rounded to f32 where
//! numpy rounds it, so both engines give the same values.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::analysis;
use crate::math;
use crate::settings::{defaults_for, schema_for, validate, SettingsError};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Canvas, Fit, Image, ToneMap};

/// Where the tone stage takes its black and white points from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Levels {
    /// The Black and White settings.
    Manual,
    /// The tones of the image itself.
    Auto,
}

/// Whether the tone map is corrected for the style's own tone response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Response {
    /// The tone map as the other settings make it.
    Off,
    /// Corrected so the drawing's tones follow the tone map.
    Auto,
}

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
    /// Manual uses Black and White. Auto sets them from the image so that Clip percent of its pixels turn full black and as many full white.
    pub levels: Levels,
    /// Share of pixels, in percent, that auto levels turn full black and, again, full white; used only when Levels is Auto.
    #[schemars(range(min = 0.0, max = 10.0), extend("x-step" = 0.1))]
    pub clip_percent: f64,
    /// Midtone curve applied to the luminance: above 1 darkens the midtones, below 1 lightens them.
    #[schemars(range(min = 0.1, max = 5.0), extend("x-step" = 0.05))]
    pub gamma: f64,
    /// Contrast around mid-grey: positive values push tones apart, negative values squeeze them towards mid-grey. Mid-grey itself stays put, so a style that thresholds at 0.5, such as grid at its default, shows no change.
    #[schemars(range(min = -1.0, max = 1.0), extend("x-step" = 0.05))]
    pub contrast: f64,
    /// Evens out grey, yellowed or unevenly lit paper: the paper's tone is estimated over this width and divided out, so it draws as white; set it wider than the widest stroke, for drawings and text rather than photos; 0 turns it off.
    #[schemars(
        range(min = 0.0, max = 50.0),
        extend("x-step" = 0.5, "x-unit" = "mm", "x-label" = "Flatten paper")
    )]
    pub flatten_mm: f64,
    /// Smooths noise and paper grain over this radius but keeps edges: tone steps of about a tenth of the range or more stay sharp, unlike Blur; 0 turns it off.
    #[schemars(
        range(min = 0.0, max = 10.0),
        extend(
            "x-step" = 0.1,
            "x-unit" = "mm",
            "x-label" = "Smooth, keep edges",
            "x-advanced" = true
        )
    )]
    pub smooth_mm: f64,
    /// Gaussian blur, given as sigma in mm, that smooths the tone map; 0 turns it off.
    #[schemars(
        range(min = 0.0, max = 10.0),
        extend("x-step" = 0.1, "x-unit" = "mm", "x-advanced" = true)
    )]
    pub blur_mm: f64,
    /// Auto draws a small test wedge with the style's settings and the first pen, measures how much of the paper each tone covers, and corrects the tone map the style draws from so the drawing's tones follow the image; off leaves it as it is.
    pub response: Response,
    /// Points to keep detail around, as `[x, y]` shares of the image's width and height from its top-left corner; set by clicking the preview.
    #[schemars(extend(
        "x-hidden" = true,
        "maxItems" = 16,
        "items" = {
            "type": "array",
            "minItems": 2,
            "maxItems": 2,
            "items": {"type": "number", "minimum": 0.0, "maximum": 1.0},
        },
    ))]
    pub focus: Vec<Vec<f64>>,
    /// Radius around each focus point that keeps all its detail; used only with focus points.
    #[schemars(
        range(min = 1.0, max = 300.0),
        extend("x-step" = 1.0, "x-unit" = "mm", "x-label" = "Focus radius")
    )]
    pub focus_mm: f64,
    /// Blur where the image is furthest from every focus point; detail fades into it beyond the focus radius. Used only with focus points.
    #[schemars(range(min = 0.5, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub focus_blur_mm: f64,
}

/// The most focus points a run takes.
const MAX_FOCUS: usize = 16;

/// The order the settings form shows the settings in (`x-order` in the schema, which lists its
/// properties alphabetically): Levels first, as it decides whether Black and White apply, then the
/// tone settings, then the advanced and hidden ones in the struct's order.
const ORDER: [&str; 14] = [
    "levels",
    "black",
    "white",
    "clip_percent",
    "gamma",
    "contrast",
    "invert",
    "flatten_mm",
    "response",
    "resolution",
    "blur_mm",
    "focus",
    "focus_mm",
    "focus_blur_mm",
];

impl Default for ToneSettings {
    fn default() -> Self {
        ToneSettings {
            resolution: 1000,
            invert: false,
            black: 0.0,
            white: 1.0,
            levels: Levels::Manual,
            clip_percent: 0.5,
            gamma: 1.0,
            contrast: 0.0,
            flatten_mm: 0.0,
            smooth_mm: 0.0,
            blur_mm: 0.0,
            response: Response::Off,
            focus: Vec::new(),
            focus_mm: 40.0,
            focus_blur_mm: 3.0,
        }
    }
}

/// Turn the image into a tone map. In order: luminance, a centre crop for `fill`, resize, flatten
/// (when `flatten_mm` is set), smooth (when `smooth_mm` is set), levels (found from the filtered
/// map when `levels` is auto), gamma, contrast, invert, blur and the blur that grows away from the
/// focus points.
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
        let mut schema = schema_for::<ToneSettings>();
        schema["x-order"] = Value::from(ORDER.as_slice());
        schema
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
        check_focus(&settings.focus)?;
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
            pending: None,
            map: None,
        }))
    }
}

/// The luminance between the first step and the last, with the filters still to run.
struct Pending {
    y: Vec<f32>,
    w: usize,
    h: usize,
    canvas: Arc<Canvas>,
    settings: ToneSettings,
    /// The source image's width and height, which the focus points are shares of.
    size: (u32, u32),
    /// The guided filter, from its first step to its last.
    guided: Option<Guided>,
    /// Steps done, the first one included, and in all.
    done: usize,
    total: usize,
}

/// The first step resizes. With `flatten_mm` and `smooth_mm` 0 it finishes the map and reports one
/// frame. Otherwise one step flattens when `flatten_mm` is set and four smooth when `smooth_mm` is,
/// and the last of them finishes the map. One more step gives the map.
struct ToneRun {
    /// What the first step reads; `None` once it has run.
    input: Option<(Arc<Image>, Arc<Canvas>, ToneSettings)>,
    /// The map between the first step and the last filter step.
    pending: Option<Pending>,
    /// The finished map, before the last step.
    map: Option<ToneMap>,
}

fn frame(progress: f32, label: &str) -> Step {
    Step::Frame(Frame {
        stage: String::new(),
        progress,
        label: label.into(),
        points: None,
        paths: None,
        pen: 0,
    })
}

impl StageRun for ToneRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some((image, canvas, settings)) = self.input.take() {
            let (y, w, h) = resized(&image, &canvas, &settings);
            let size = (image.width, image.height);
            let filters =
                (settings.flatten_mm > 0.0) as usize + 4 * (settings.smooth_mm > 0.0) as usize;
            if filters == 0 {
                self.map = Some(finish(y, (w, h), &canvas, &settings, size));
                return Ok(frame(1.0, "tone map"));
            }
            let total = 1 + filters;
            self.pending = Some(Pending {
                y,
                w,
                h,
                canvas,
                settings,
                size,
                guided: None,
                done: 1,
                total,
            });
            return Ok(frame(1.0 / total as f32, "tone map"));
        }
        if let Some(mut p) = self.pending.take() {
            let (mm_per_px, _) = p.canvas.image_transform(p.w as u32, p.h as u32);
            let label = if p.settings.flatten_mm > 0.0 && p.done == 1 {
                p.y = flatten(&p.y, p.w, p.h, p.settings.flatten_mm / mm_per_px);
                "flatten paper"
            } else {
                let g = p.guided.get_or_insert_with(|| {
                    let r = ((p.settings.smooth_mm / mm_per_px).round() as usize).max(1);
                    Guided::new(std::mem::take(&mut p.y), p.w, p.h, r)
                });
                if g.step() {
                    p.y = p
                        .guided
                        .take()
                        .expect("the filter is running")
                        .into_output();
                }
                "smooth"
            };
            p.done += 1;
            let progress = p.done as f32 / p.total as f32;
            if p.done == p.total {
                self.map = Some(finish(p.y, (p.w, p.h), &p.canvas, &p.settings, p.size));
            } else {
                self.pending = Some(p);
            }
            return Ok(frame(progress, label));
        }
        let map = self.map.take().expect("the tone run has already finished");
        Ok(Step::Done(Artifact::Tone(Arc::new(map))))
    }
}

/// The resized luminance of `image` on `canvas` and its width and height: luminance, a centre
/// crop for `fill` and the resize.
fn resized(image: &Image, canvas: &Canvas, s: &ToneSettings) -> (Vec<f32>, usize, usize) {
    let (mut lum, mut w, mut h) = (
        image.luminance(),
        image.width as usize,
        image.height as usize,
    );
    if canvas.fit == Fit::Fill {
        let [_, _, dw, dh] = canvas.drawing_rect();
        (lum, w, h) = crop_to_aspect(&lum, w, h, dw / dh);
    }
    resize(&lum, w, h, s.resolution as usize)
}

/// The tone map from the luminance `y`, `tw x th` on `canvas`, made from an image of `size` pixels,
/// once it is resized, flattened and smoothed: levels to the focus falloff, in the order the module
/// documents.
fn finish(
    y: Vec<f32>,
    (tw, th): (usize, usize),
    canvas: &Canvas,
    s: &ToneSettings,
    (iw, ih): (u32, u32),
) -> ToneMap {
    let (tw32, th32) = (tw as u32, th as u32);
    let (mm_per_px, origin_mm) = canvas.image_transform(tw32, th32);
    let (black, white) = match s.levels {
        Levels::Manual => (s.black, s.white),
        Levels::Auto => {
            let (b, w) = analysis::levels(&y, s.clip_percent);
            // A near-flat image keeps full levels, so its noise is not stretched into ink.
            if w - b < 1.0 / 64.0 {
                (0.0, 1.0)
            } else {
                (b, w)
            }
        }
    };
    // numpy takes `max(white - black, 1e-6)` and `1 + contrast` in f64 and rounds the result to
    // f32; white <= black makes the levels a hard threshold at black instead of dividing by zero.
    let (black, span) = (black as f32, (white - black).max(1e-6) as f32);
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
    let ink = if s.blur_mm > 0.0 {
        math::blur(&ink, tw, th, s.blur_mm / mm_per_px)
    } else {
        ink
    };
    let ink = if s.focus.is_empty() {
        ink
    } else {
        let points = s
            .focus
            .iter()
            .map(|p| canvas.page_point([p[0], p[1]], iw, ih));
        let points: Vec<[f64; 2]> = points.collect();
        focus_falloff(&ink, (tw, th), (mm_per_px, origin_mm), &points, s)
    };
    ToneMap {
        width: tw32,
        height: th32,
        ink: ink.into_iter().map(|v| v.clamp(0.0, 1.0)).collect(),
        mm_per_px,
        origin_mm,
    }
}

/// Refuse more than [`MAX_FOCUS`] points, and a point that is not two numbers from 0 to 1.
fn check_focus(points: &[Vec<f64>]) -> Result<(), SettingsError> {
    let invalid = |message: String| SettingsError::Invalid {
        stage: "tone".into(),
        key: Some("focus".into()),
        message,
    };
    if points.len() > MAX_FOCUS {
        let n = points.len();
        return Err(invalid(format!(
            "{n} points given; at most {MAX_FOCUS} are allowed"
        )));
    }
    for (i, p) in points.iter().enumerate() {
        // A range check, so a NaN fails it too.
        if p.len() != 2 || !p.iter().all(|c| (0.0..=1.0).contains(c)) {
            let n = i + 1;
            return Err(invalid(format!(
                "point {n} is {p:?}; each point must be two numbers from 0 to 1"
            )));
        }
    }
    Ok(())
}

/// `ink`, a `tw x th` map of `mm_per_px` mm pixels with its top-left corner at `origin_mm`, blurred
/// more the further a pixel centre lies from the nearest of `focus`, which are page points in mm.
/// With `d` that distance, the weight `w = clamp((d - focus_mm) / (2 focus_mm), 0, 1)` picks a
/// blur between none (within `focus_mm`) and `focus_blur_mm` (from three radii out). The map is
/// blurred at a quarter, a half, three quarters and all of `focus_blur_mm`, and a pixel takes its
/// ink from the two levels either side of `4 w`, level 0 being `ink` itself.
fn focus_falloff(
    ink: &[f32],
    (tw, th): (usize, usize),
    (mm_per_px, origin_mm): (f64, [f64; 2]),
    focus: &[[f64; 2]],
    s: &ToneSettings,
) -> Vec<f32> {
    let blurred: Vec<Vec<f32>> = (1..=4)
        .map(|k| {
            math::blur(
                ink,
                tw,
                th,
                s.focus_blur_mm * f64::from(k) / 4.0 / mm_per_px,
            )
        })
        .collect();
    let level = |i: usize, p: usize| if i == 0 { ink[p] } else { blurred[i - 1][p] };
    let mut out = Vec::with_capacity(ink.len());
    for y in 0..th {
        let py = (y as f64 + 0.5) * mm_per_px + origin_mm[1];
        for x in 0..tw {
            let px = (x as f64 + 0.5) * mm_per_px + origin_mm[0];
            let near = focus
                .iter()
                .map(|f| (f[0] - px) * (f[0] - px) + (f[1] - py) * (f[1] - py))
                .fold(f64::INFINITY, f64::min);
            let w = ((near.sqrt() - s.focus_mm) / (2.0 * s.focus_mm)).clamp(0.0, 1.0);
            let i = ((w * 4.0) as usize).min(3);
            let t = (w * 4.0 - i as f64) as f32;
            let p = y * tw + x;
            // Exact at both ends of the weight: t is 0 within the radius and 1 past three.
            out.push(level(i, p) * (1.0 - t) + level(i + 1, p) * t);
        }
    }
    out
}

/// Divide the paper's own tone out of a row-major `w x h` luminance grid `y`, for a window of
/// `window_px` pixels. The paper is estimated as a grey-level closing of `y`, a maximum filter then
/// a minimum filter wider than the widest stroke, and `y` is divided by it. This is background
/// subtraction by a morphological closing, after S. R. Sternberg, "Biomedical image processing",
/// *IEEE Computer* 16(1), 1983, pp. 22-34, doi:10.1109/MC.1983.1654163, whose rolling ball the flat
/// square window stands in for. Document binarisation uses the same estimate-and-divide step: S. Lu,
/// B. Su and C. L. Tan, "Document image binarization using background estimation and stroke
/// edges", *IJDAR* 13(4), 2010, pp. 303-314, doi:10.1007/s10032-010-0130-8.
///
/// The paper is smooth, so it is estimated on a coarse grid of `k x k` blocks, `k = max(1,
/// floor(n / 8), ceil(max(w, h) / 1024))` for `n = window_px`, each holding the largest value in it
/// (blocks at the right and bottom edges are partial). On the grid the closing runs over a square
/// of `2r + 1` blocks, `r = max(1, ceil(n / (2k)))`, rows then columns, the window cut off at the
/// edges. The grid is blurred with a sigma of `r` blocks, which rounds the closing's plateaus off,
/// and resampled back so that block centres line up with pixel centres. The result is
/// `min(1, y / paper)`, and 0 where the paper is 0.
///
/// A dark area wider than the window reads as paper and turns white, so the window must be wider
/// than the widest stroke, and flattening suits drawings and text rather than photographs. It
/// assumes light paper. Only comparisons, [`math::blur`], [`math::resample`] and one f32 division
/// are used, so native and WebAssembly builds give the same bits.
pub fn flatten(y: &[f32], w: usize, h: usize, window_px: f64) -> Vec<f32> {
    if y.is_empty() {
        return Vec::new();
    }
    let k = ((window_px / 8.0).floor() as usize)
        .max(w.max(h).div_ceil(1024))
        .max(1);
    let (cw, ch) = (w.div_ceil(k), h.div_ceil(k));
    // Luminance is not negative, so a block starts from 0.
    let mut grid = vec![0.0f32; cw * ch];
    for (i, row) in y.chunks_exact(w).enumerate() {
        for (j, &v) in row.iter().enumerate() {
            let g = &mut grid[(i / k) * cw + j / k];
            *g = g.max(v);
        }
    }
    let r = ((window_px / (2.0 * k as f64)).ceil() as usize).max(1);
    for keep in [f32::max as fn(f32, f32) -> f32, f32::min] {
        grid = window(&grid, (cw, ch), r, keep, true);
        grid = window(&grid, (cw, ch), r, keep, false);
    }
    let grid = math::blur(&grid, cw, ch, r as f64);
    let paper = math::resample(&grid, cw, ch, cw * k, ch * k);
    let mut out = Vec::with_capacity(y.len());
    for (i, row) in y.chunks_exact(w).enumerate() {
        for (j, &v) in row.iter().enumerate() {
            let b = paper[i * cw * k + j];
            out.push(if b > 0.0 { (v / b).min(1.0) } else { 0.0 });
        }
    }
    out
}

/// `keep`, the larger or the smaller of two values, over the `2r + 1` samples around each sample
/// of a row-major grid of `(w, h)`, along its rows or along its columns, with the window cut off at
/// the edges.
fn window(
    g: &[f32],
    (w, h): (usize, usize),
    r: usize,
    keep: fn(f32, f32) -> f32,
    rows: bool,
) -> Vec<f32> {
    let (len, lines) = if rows { (w, h) } else { (h, w) };
    let at = |line: usize, j: usize| if rows { line * w + j } else { j * w + line };
    let mut out = vec![0.0f32; g.len()];
    for line in 0..lines {
        for j in 0..len {
            let (lo, hi) = (j.saturating_sub(r), (j + r).min(len - 1));
            out[at(line, j)] = (lo + 1..=hi).fold(g[at(line, lo)], |m, i| keep(m, g[at(line, i)]));
        }
    }
    out
}

/// The penalty on the slope of the guided filter's fit: a tone step of about a tenth of the range,
/// `0.1^2` in variance, counts as an edge.
const EPS: f32 = 0.01;

/// The four steps of [`guided`], run one at a time so that the tone run can report each.
struct Guided {
    y: Vec<f32>,
    w: usize,
    h: usize,
    r: usize,
    /// Steps done, 0 to 4.
    done: usize,
    mean_i: Vec<f32>,
    mean_ii: Vec<f32>,
    b: Vec<f32>,
    mean_a: Vec<f32>,
    q: Vec<f32>,
}

impl Guided {
    /// A filter for the row-major `w x h` grid `y` with a window of `2r + 1` pixels.
    fn new(y: Vec<f32>, w: usize, h: usize, r: usize) -> Guided {
        Guided {
            y,
            w,
            h,
            r,
            done: 0,
            mean_i: Vec::new(),
            mean_ii: Vec::new(),
            b: Vec::new(),
            mean_a: Vec::new(),
            q: Vec::new(),
        }
    }

    /// Run the next of the four steps; true once the fourth has run.
    fn step(&mut self) -> bool {
        let (w, h, r) = (self.w, self.h, self.r);
        match self.done {
            0 => self.mean_i = math::box_mean(&self.y, w, h, r),
            1 => {
                let squares: Vec<f32> = self.y.iter().map(|&v| v * v).collect();
                self.mean_ii = math::box_mean(&squares, w, h, r);
            }
            2 => {
                let (mean_i, mean_ii) = (
                    std::mem::take(&mut self.mean_i),
                    std::mem::take(&mut self.mean_ii),
                );
                let (mut a, mut b) = (
                    Vec::with_capacity(mean_i.len()),
                    Vec::with_capacity(mean_i.len()),
                );
                for (&m, &mm) in mean_i.iter().zip(&mean_ii) {
                    let var = (mm - m * m).max(0.0);
                    let slope = var / (var + EPS);
                    a.push(slope);
                    b.push((1.0 - slope) * m);
                }
                self.mean_a = math::box_mean(&a, w, h, r);
                self.b = b;
            }
            3 => {
                let mean_b = math::box_mean(&self.b, w, h, r);
                let rows = self.y.iter().zip(&self.mean_a).zip(&mean_b);
                self.q = rows
                    .map(|((&v, &a), &b)| (a * v + b).clamp(0.0, 1.0))
                    .collect();
            }
            _ => panic!("the guided filter has already run its four steps"),
        }
        self.done += 1;
        self.done == 4
    }

    /// The smoothed grid, once [`Guided::step`] has returned true.
    fn into_output(self) -> Vec<f32> {
        assert_eq!(self.done, 4, "the guided filter has not run its four steps");
        self.q
    }
}

/// The row-major `w x h` grid `y` smoothed by the guided filter of K. He, J. Sun and X. Tang,
/// "Guided image filtering", *ECCV 2010*, LNCS 6311, pp. 1-14, doi:10.1007/978-3-642-15549-9_1
/// (journal version *IEEE TPAMI* 35(6), 2013, pp. 1397-1409, doi:10.1109/TPAMI.2012.213), with the
/// image as its own guide.
///
/// In each window of `2r + 1` pixels the output is a linear function of the input, `q = a I + b`,
/// fitted by least squares with a penalty [`EPS`] on `a`: where the window's variance is well above
/// it (an edge) `a` is near 1 and the edge stays, and where it is well below (noise on a flat tone)
/// `a` is near 0 and the window's mean takes over. With `box` the mean over a window,
/// [`math::box_mean`]:
///
/// 1. `mean_I = box(I)`.
/// 2. `mean_II = box(I * I)`.
/// 3. `var = max(0, mean_II - mean_I^2)`, `a = var / (var + EPS)`, `b = (1 - a) mean_I` and
///    `mean_a = box(a)`.
/// 4. `mean_b = box(b)`, `q = clamp(mean_a I + mean_b, 0, 1)`.
///
/// The output averages convex combinations of a pixel and window means, so it stays within the
/// input's own range. Only f32 arithmetic and [`math::box_mean`] are used, so native and WebAssembly
/// builds give the same bits.
///
/// The tone run steps through the same code in four steps.
pub fn guided(y: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut filter = Guided::new(y.to_vec(), w, h, r);
    while !filter.step() {}
    filter.into_output()
}

/// The width and height of the tone map the stage makes from a `w x h` image on `canvas` at
/// `resolution`, without making it.
pub(crate) fn map_size(w: usize, h: usize, canvas: &Canvas, resolution: u32) -> (usize, usize) {
    let (w, h) = if canvas.fit == Fit::Fill {
        let [_, _, dw, dh] = canvas.drawing_rect();
        crop_size(w, h, dw / dh)
    } else {
        (w, h)
    };
    resize_size(w, h, resolution as usize)
}

/// The width and height of a centre crop of a `w x h` grid to a width over height of `aspect`, to
/// the nearest pixel with halves to even as Python's `round` does.
fn crop_size(w: usize, h: usize, aspect: f64) -> (usize, usize) {
    let ch = h.min(((w as f64 / aspect).round_ties_even() as usize).max(1));
    let cw = w.min(((h as f64 * aspect).round_ties_even() as usize).max(1));
    (cw, ch)
}

/// The centre crop of a `w x h` grid to a width over height of `aspect`, as `[x, y, w, h]`; see
/// [`crop_size`].
pub(crate) fn crop_rect(w: usize, h: usize, aspect: f64) -> [usize; 4] {
    let (cw, ch) = crop_size(w, h, aspect);
    [(w - cw) / 2, (h - ch) / 2, cw, ch]
}

/// Centre-crop a row-major `w x h` grid to a width over height of `aspect`, see [`crop_rect`].
/// Returns the crop and its width and height.
pub(crate) fn crop_to_aspect(
    lum: &[f32],
    w: usize,
    h: usize,
    aspect: f64,
) -> (Vec<f32>, usize, usize) {
    let [x0, y0, cw, ch] = crop_rect(w, h, aspect);
    let rows = lum[y0 * w..(y0 + ch) * w].chunks_exact(w);
    let crop = rows.flat_map(|row| &row[x0..x0 + cw]).copied().collect();
    (crop, cw, ch)
}

/// Resize a row-major `w x h` grid to `long_edge` pixels on the longer side, keeping the aspect
/// ratio (halves to even) and at least one pixel on the other side. A source at least twice the
/// target is first averaged in `k x k` blocks, `k = max(w, h) / long_edge`, then resampled.
/// Returns the grid and its width and height.
pub(crate) fn resize(
    lum: &[f32],
    w: usize,
    h: usize,
    long_edge: usize,
) -> (Vec<f32>, usize, usize) {
    let (tw, th) = resize_size(w, h, long_edge);
    let k = w.max(h) / long_edge;
    if k >= 2 {
        let (blocks, bw, bh) = math::block_average(lum, w, h, k);
        return (math::resample(&blocks, bw, bh, tw, th), tw, th);
    }
    (math::resample(lum, w, h, tw, th), tw, th)
}

/// The width and height [`resize`] gives a `w x h` grid.
fn resize_size(w: usize, h: usize, long_edge: usize) -> (usize, usize) {
    let short = |other: usize, long: usize| {
        let exact = (other * long_edge) as f64 / long as f64;
        (exact.round_ties_even() as usize).max(1)
    };
    if h >= w {
        (short(w, h), long_edge)
    } else {
        (long_edge, short(h, w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::merge;
    use crate::synthetic::{
        checkerboard, color_bars, flat, flat_noise, gradient, lit_strokes, noise, ramp, still_life,
    };
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
    fn colour_bars_take_ink_by_luminance() {
        // White, yellow, cyan, green, magenta, red, blue and black: each bar's ink is 1 - Y.
        let (_, t) = run_tone(color_bars(64, 48), SQUARE, json!({}));
        let want = [0.0, 0.114, 0.299, 0.413, 0.587, 0.701, 0.886, 1.0];
        let row = &t.ink[24 * 64..25 * 64];
        for (i, w) in want.iter().enumerate() {
            let v = row[8 * i + 4];
            assert!((v - w).abs() < 1e-5, "bar {i}: ink {v}, want {w}");
        }
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
    fn auto_levels_stretch_a_narrow_ramp() {
        // A bin edge is off by up to 1/4096, which the span of about 0.4 turns into 1/1024.
        let patch = json!({"levels": "auto", "clip_percent": 0});
        let (_, t) = run_tone(ramp(256, 64, 0.3, 0.7), SQUARE, patch);
        let lo = t.ink.iter().copied().fold(1.0, f32::min);
        let hi = t.ink.iter().copied().fold(0.0, f32::max);
        assert!(lo <= 1.0 / 1024.0, "least ink {lo}");
        assert!(hi >= 1.0 - 1.0 / 1024.0, "most ink {hi}");
    }
    #[test]
    fn auto_levels_leave_a_flat_image_flat() {
        let auto = run_tone(flat(64, 64, 0.5), SQUARE, json!({"levels": "auto"})).1;
        let manual = run_tone(flat(64, 64, 0.5), SQUARE, json!({"levels": "manual"})).1;
        assert_eq!(auto.ink, manual.ink);
    }
    #[test]
    fn auto_levels_ignore_black_and_white() {
        let image = || ramp(256, 64, 0.3, 0.7);
        let auto = json!({"levels": "auto", "clip_percent": 0});
        let moved = json!({"levels": "auto", "clip_percent": 0, "black": 0.4, "white": 0.6});
        assert_eq!(
            run_tone(image(), SQUARE, auto).1.ink,
            run_tone(image(), SQUARE, moved).1.ink
        );
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
    fn map_size_is_the_size_the_stage_makes() {
        let fill = Canvas::new(100.0, 50.0, 0.0, Fit::Fill).unwrap();
        for (canvas, (w, h), resolution) in [
            (SQUARE, (300, 200), 100),
            (SQUARE, (200, 300), 64),
            (fill.clone(), (64, 64), 64),
            (fill, (1000, 333), 128),
        ] {
            let patch = json!({"resolution": resolution});
            let (_, t) = run_tone(flat(w, h, 0.5), canvas.clone(), patch);
            let size = map_size(w as usize, h as usize, &canvas, resolution);
            assert_eq!(size, (t.width as usize, t.height as usize));
        }
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
    /// The distance in mm from the centre of map pixel `(x, y)` to the page point `at`.
    fn distance(t: &ToneMap, (x, y): (usize, usize), at: [f64; 2]) -> f64 {
        let [mx, my] = t.to_mm([x as f64 + 0.5, y as f64 + 0.5]);
        ((mx - at[0]) * (mx - at[0]) + (my - at[1]) * (my - at[1])).sqrt()
    }
    #[test]
    fn no_focus_points_leave_the_map_as_it_was() {
        let plain = run_tone(still_life(60, 88), SQUARE, json!({})).1;
        let patch = json!({"focus": [], "focus_mm": 5, "focus_blur_mm": 9});
        assert_eq!(run_tone(still_life(60, 88), SQUARE, patch).1, plain);
    }
    #[test]
    fn detail_stays_within_the_radius_and_fades_to_the_blur() {
        let image = || still_life(120, 120);
        let patch = json!({"focus": [[0.5, 0.5]], "focus_mm": 15, "focus_blur_mm": 4});
        let plain = run_tone(image(), SQUARE, json!({})).1;
        let soft = run_tone(image(), SQUARE, json!({"blur_mm": 4})).1;
        let t = run_tone(image(), SQUARE, patch.clone()).1;
        assert_eq!(*t, *run_tone(image(), SQUARE, patch).1, "determinism");
        let (mut between, mut changed) = (0, 0);
        for y in 0..64 {
            for x in 0..64 {
                let (i, d) = (y * 64 + x, distance(&t, (x, y), [50.0, 50.0]));
                if d <= 14.9 {
                    assert_eq!(t.ink[i], plain.ink[i], "({x}, {y}) is {d} mm out");
                } else if d >= 45.1 {
                    assert!(
                        (t.ink[i] - soft.ink[i]).abs() < 1e-6,
                        "({x}, {y}) is {d} mm out"
                    );
                    changed += usize::from(soft.ink[i] != plain.ink[i]);
                } else {
                    between += usize::from(t.ink[i] != plain.ink[i] && t.ink[i] != soft.ink[i]);
                }
            }
        }
        assert!(
            changed > 100 && between > 100,
            "{changed} blurred, {between} between"
        );
    }
    #[test]
    fn the_mean_ink_barely_moves() {
        let image = || still_life(60, 88);
        let mean = |t: &ToneMap| t.ink.iter().sum::<f32>() / t.ink.len() as f32;
        let plain = mean(&run_tone(image(), SQUARE, json!({})).1);
        let patch = json!({"focus": [[0.3, 0.4], [0.7, 0.8]], "focus_mm": 10, "focus_blur_mm": 6});
        let focused = mean(&run_tone(image(), SQUARE, patch).1);
        assert!(
            (focused - plain).abs() < 0.02 * plain,
            "{plain} became {focused}"
        );
    }
    #[test]
    fn the_nearest_of_several_points_counts() {
        let patch = json!({"focus": [[0.1, 0.1], [0.9, 0.9]], "focus_mm": 12});
        let plain = run_tone(still_life(120, 120), SQUARE, json!({})).1;
        let t = run_tone(still_life(120, 120), SQUARE, patch).1;
        for at in [[10.0, 10.0], [90.0, 90.0]] {
            let near = (0..64).flat_map(|y| (0..64).map(move |x| (x, y)));
            for (x, y) in near.filter(|&p| distance(&t, p, at) <= 11.9) {
                assert_eq!(t.ink[y * 64 + x], plain.ink[y * 64 + x]);
            }
        }
        // The corner far from both is blurred.
        assert_ne!(t.ink[63], plain.ink[63]);
    }
    #[test]
    fn focus_points_follow_the_fill_crop_and_may_lie_outside_it() {
        // A 100 x 50 image on a 50 x 100 page keeps columns 37 to 61 of its 100, so the share
        // 0.4325 is a quarter of the way across the page. The map is 32 x 64 with 1.5625 mm pixels.
        let page = || Canvas::new(50.0, 100.0, 0.0, Fit::Fill).unwrap();
        let image = || checkerboard(100, 50, 50);
        let at = |x: usize, y: usize| y * 32 + x;
        let plain = run_tone(image(), page(), json!({})).1;
        let soft = run_tone(image(), page(), json!({"blur_mm": 3})).1;
        let patch = json!({"focus": [[0.4325, 0.5]], "focus_mm": 8});
        let t = run_tone(image(), page(), patch).1;
        for (x, y) in [(8, 32), (6, 30)] {
            assert_ne!(plain.ink[at(x, y)], soft.ink[at(x, y)]);
            assert_eq!(t.ink[at(x, y)], plain.ink[at(x, y)], "({x}, {y})");
        }
        for (x, y) in [(24, 32), (23, 20)] {
            assert_ne!(plain.ink[at(x, y)], soft.ink[at(x, y)]);
            assert!(
                (t.ink[at(x, y)] - soft.ink[at(x, y)]).abs() < 1e-6,
                "({x}, {y})"
            );
        }
        // A point left of the crop, 54 mm off the page, still keeps the pixels within 60 mm of it.
        let patch = json!({"focus": [[0.1, 0.5]], "focus_mm": 60});
        let t = run_tone(image(), page(), patch).1;
        assert_eq!(t.ink[at(0, 32)], plain.ink[at(0, 32)]);
        assert_ne!(t.ink[at(31, 32)], plain.ink[at(31, 32)]);
    }
    #[test]
    fn bad_focus_points_are_refused_naming_the_point() {
        let start = |focus: Value| {
            let settings = merge(&ToneStage.defaults(), &json!({"focus": focus}));
            let inputs = Inputs::new(HashMap::from([
                (Kind::Image, Artifact::Image(Arc::new(flat(8, 8, 0.5)))),
                (Kind::Canvas, Artifact::Canvas(Arc::new(SQUARE))),
            ]));
            let ctx = RunContext::new(Arc::default());
            ToneStage
                .start(inputs, &settings, ctx)
                .err()
                .map(|e| e.to_string())
        };
        assert_eq!(
            start(json!([[0.5, 0.5], [1.5, 0.2]])).unwrap(),
            "stage 'tone', setting 'focus': point 2 is [1.5, 0.2]; each point must be two numbers from 0 to 1"
        );
        assert!(start(json!([[0.1, -0.1]]))
            .unwrap()
            .contains("point 1 is [0.1, -0.1]"));
        assert!(start(json!([[0.5], []]))
            .unwrap()
            .contains("point 1 is [0.5]; each point"));
        let sixteen = vec![json!([0.5, 0.5]); 16];
        assert!(start(json!(sixteen)).is_none());
        let seventeen = vec![json!([0.5, 0.5]); 17];
        assert_eq!(
            start(json!(seventeen)).unwrap(),
            "stage 'tone', setting 'focus': 17 points given; at most 16 are allowed"
        );
        let nan = check_focus(&[vec![0.5, 0.5], vec![0.5, f64::NAN]])
            .unwrap_err()
            .to_string();
        assert!(nan.contains("point 2 is [0.5, NaN]"), "{nan}");
        assert!(check_focus(&[vec![f64::INFINITY, 0.5]]).is_err());
        assert!(check_focus(&[vec![0.0, 1.0]]).is_ok());
    }
    #[test]
    fn x_order_lists_every_setting_once() {
        let schema = ToneStage.schema();
        let mut order: Vec<&str> = schema["x-order"]
            .as_array()
            .expect("the schema declares x-order")
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        let mut keys: Vec<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(order[0], "levels");
        order.sort_unstable();
        keys.sort_unstable();
        assert_eq!(order, keys);
    }
    #[test]
    fn the_focus_settings_are_declared() {
        let schema = ToneStage.schema();
        let p = &schema["properties"];
        assert_eq!(p["focus"]["x-hidden"], true);
        assert_eq!(p["focus"]["default"], json!([]));
        assert_eq!(p["focus"]["maxItems"], MAX_FOCUS);
        assert_eq!(p["focus"]["items"]["items"]["maximum"], 1.0);
        assert_eq!(
            (
                p["focus_mm"]["minimum"].as_f64(),
                p["focus_mm"]["maximum"].as_f64()
            ),
            (Some(1.0), Some(300.0))
        );
        assert_eq!(p["focus_mm"]["default"], 40.0);
        assert_eq!(
            (
                p["focus_blur_mm"]["minimum"].as_f64(),
                p["focus_blur_mm"]["maximum"].as_f64()
            ),
            (Some(0.5), Some(10.0))
        );
        assert_eq!(p["focus_blur_mm"]["default"], 3.0);
    }
    /// The pixels of `lit_strokes` that are paper, and the ones inside a vertical stroke, a few
    /// pixels from where it meets another.
    fn is_paper(x: usize, y: usize) -> bool {
        x % 16 >= 2 && y % 16 >= 2
    }
    fn is_stroke(x: usize, y: usize) -> bool {
        x % 16 < 2 && (4..14).contains(&(y % 16))
    }

    #[test]
    fn flatten_turns_lit_paper_white_and_keeps_the_strokes() {
        let image = lit_strokes(160, 160, 0.4);
        let y = flatten(&image.luminance(), 160, 160, 8.0);
        for (i, &v) in y.iter().enumerate() {
            let (x, row) = (i % 160, i / 160);
            if is_paper(x, row) {
                assert!(v >= 0.97, "paper at ({x}, {row}) is {v}");
            } else if is_stroke(x, row) {
                assert!(v <= 0.3, "stroke at ({x}, {row}) is {v}");
            }
        }
    }
    #[test]
    fn flatten_leaves_a_flat_image_white_and_a_black_one_black() {
        // Windows with one block, with several, and a grid whose last blocks are partial.
        for (w, h, window) in [(40, 30, 3.0), (40, 30, 20.0), (37, 19, 40.0), (1, 1, 5.0)] {
            let grey = flatten(&vec![0.6; w * h], w, h, window);
            assert!(grey.iter().all(|&v| (1.0 - 1e-6..=1.0).contains(&v)));
            assert_eq!(flatten(&vec![0.0; w * h], w, h, window), vec![0.0; w * h]);
        }
    }
    #[test]
    fn flatten_blocks_follow_a_wide_map() {
        // 1300 pixels wide forces blocks of 2 whatever the window.
        let y: Vec<f32> = (0..1300 * 3)
            .map(|i| 0.5 + 0.5 * (i % 1300) as f32 / 1300.0)
            .collect();
        let flat = flatten(&y, 1300, 3, 6.0);
        assert_eq!(flat.len(), y.len());
        assert!(flat.iter().all(|&v| v > 0.95 && v <= 1.0));
    }
    #[test]
    fn flatten_off_makes_the_map_it_always_did() {
        let image = || lit_strokes(160, 160, 0.4);
        let (frames, off) = run_tone(image(), SQUARE, json!({"flatten_mm": 0.0}));
        let (_, default) = run_tone(image(), SQUARE, json!({}));
        assert_eq!(off.ink, default.ink);
        let labels: Vec<_> = frames
            .iter()
            .map(|f| (f.label.as_str(), f.progress))
            .collect();
        assert_eq!(labels, [("tone map", 1.0)]);
    }
    #[test]
    fn flatten_on_reports_two_frames_and_whitens_the_paper() {
        let image = || lit_strokes(160, 160, 0.4);
        let (frames, on) = run_tone(
            image(),
            SQUARE,
            json!({"resolution": 160, "flatten_mm": 5.0}),
        );
        let labels: Vec<_> = frames
            .iter()
            .map(|f| (f.label.as_str(), f.progress))
            .collect();
        assert_eq!(labels, [("tone map", 0.5), ("flatten paper", 1.0)]);
        let (_, off) = run_tone(image(), SQUARE, json!({"resolution": 160}));
        let paper = |t: &ToneMap| t.ink[88 * 160 + 8];
        assert!(paper(&off) > 0.5 && paper(&on) < 0.05);
    }
    /// A two-tone step, 0.2 on the left half and 0.8 on the right, `w x h`, with the noise on it
    /// (uniform, standard deviation 0.03) and without.
    fn noisy_step(w: usize, h: usize) -> (Vec<f32>, Vec<f32>) {
        let noise = flat_noise(w as u32, h as u32, 0.5, 0.03, 7).luminance();
        let clean: Vec<f32> = (0..w * h)
            .map(|i| if i % w < w / 2 { 0.2 } else { 0.8 })
            .collect();
        let noisy = clean.iter().zip(&noise).map(|(c, n)| c + n - 0.5).collect();
        (noisy, clean)
    }
    /// The root mean square of `got - clean` over the columns `xs` of a `w`-wide grid.
    fn rms(got: &[f32], clean: &[f32], w: usize, xs: std::ops::Range<usize>) -> f32 {
        let d: Vec<f32> = (got.iter().zip(clean).enumerate())
            .filter(|(i, _)| xs.contains(&(i % w)))
            .map(|(_, (g, c))| g - c)
            .collect();
        (d.iter().map(|v| v * v).sum::<f32>() / d.len() as f32).sqrt()
    }
    #[test]
    fn guided_keeps_a_step_that_a_blur_softens_and_still_lowers_the_noise() {
        let (w, h, r) = (64, 32, 4);
        let (noisy, clean) = noisy_step(w, h);
        let smooth = guided(&noisy, w, h, r);
        let blurred = math::blur(&noisy, w, h, r as f64);
        // The step's size at the edge, from the middle row of the profile averaged down the rows.
        let jump = |g: &[f32]| {
            let at = |x: usize| (0..h).map(|y| g[y * w + x]).sum::<f32>() / h as f32;
            at(w / 2) - at(w / 2 - 1)
        };
        // Away from the step by more than the window, where only noise is left.
        let flat = |g: &[f32]| rms(g, &clean, w, 8..24).max(rms(g, &clean, w, 40..56));
        let (jg, jb) = (jump(&smooth), jump(&blurred));
        let (ng, nb, n0) = (flat(&smooth), flat(&blurred), flat(&noisy));
        // Measured: the step is 0.53 after guided and 0.06 after the blur; the noise falls from
        // 0.030 to 0.0039 and to 0.0029.
        assert!(
            jg > 0.5 && jb < 0.3,
            "the step is 0.6: guided {jg}, blur {jb}"
        );
        assert!(
            ng < n0 / 5.0 && ng < 2.0 * nb,
            "noise: guided {ng}, blur {nb}, input {n0}"
        );
    }
    #[test]
    fn guided_leaves_a_flat_image_flat() {
        for (w, h, r) in [(16, 12, 1), (5, 40, 3), (1, 1, 2), (30, 30, 40)] {
            let y = vec![0.6f32; w * h];
            let out = guided(&y, w, h, r);
            assert!(
                out.iter().all(|v| (v - 0.6).abs() < 1e-6),
                "{w} x {h}, r {r}"
            );
        }
        assert!(guided(&[], 0, 0, 1).is_empty());
    }
    #[test]
    fn the_steps_of_the_guided_filter_give_the_function_bits() {
        let (w, h) = (40, 24);
        let (noisy, _) = noisy_step(w, h);
        let mut filter = Guided::new(noisy.clone(), w, h, 3);
        let finished: Vec<bool> = (0..4).map(|_| filter.step()).collect();
        assert_eq!(finished, [false, false, false, true]);
        assert_eq!(filter.into_output(), guided(&noisy, w, h, 3));
    }
    #[test]
    fn smooth_off_makes_the_map_it_always_did() {
        let image = || lit_strokes(160, 160, 0.4);
        let (frames, off) = run_tone(image(), SQUARE, json!({"smooth_mm": 0.0}));
        let (_, default) = run_tone(image(), SQUARE, json!({}));
        assert_eq!(off.ink, default.ink);
        let labels: Vec<_> = frames
            .iter()
            .map(|f| (f.label.as_str(), f.progress))
            .collect();
        assert_eq!(labels, [("tone map", 1.0)]);
    }
    #[test]
    fn smooth_on_reports_four_frames_after_the_resize() {
        let image = || noise(64, 64, 3);
        let (frames, on) = run_tone(image(), SQUARE, json!({"smooth_mm": 2.0}));
        let labels: Vec<_> = frames
            .iter()
            .map(|f| (f.label.as_str(), f.progress))
            .collect();
        let want = [
            ("tone map", 0.2),
            ("smooth", 0.4),
            ("smooth", 0.6),
            ("smooth", 0.8),
            ("smooth", 1.0),
        ];
        assert_eq!(labels, want.map(|(l, p)| (l, p as f32)));
        // The map is the smoothed luminance through the rest of the stage: ink is one minus it.
        let (w, h) = (64, 64);
        let (mm_per_px, _) = SQUARE.image_transform(w, h);
        let r = ((2.0 / mm_per_px).round() as usize).max(1);
        let lum = noise(64, 64, 3).luminance();
        let want = guided(&lum, w as usize, h as usize, r);
        assert!(on
            .ink
            .iter()
            .zip(&want)
            .all(|(i, q)| (i + q - 1.0).abs() < 1e-6));
        let (_, off) = run_tone(image(), SQUARE, json!({}));
        assert_ne!(on.ink, off.ink);
    }
    #[test]
    fn flatten_and_smooth_run_in_that_order_and_make_the_documented_map() {
        let image = || lit_strokes(160, 160, 0.4);
        let patch = json!({"resolution": 160, "flatten_mm": 5.0, "smooth_mm": 0.6});
        let (frames, on) = run_tone(image(), SQUARE, patch);
        let labels: Vec<_> = frames.iter().map(|f| f.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "tone map",
                "flatten paper",
                "smooth",
                "smooth",
                "smooth",
                "smooth"
            ]
        );
        let progress: Vec<_> = frames.iter().map(|f| f.progress).collect();
        assert!(progress.windows(2).all(|p| p[0] < p[1]) && progress[5] == 1.0);
        let (mm_per_px, _) = SQUARE.image_transform(160, 160);
        let r = ((0.6 / mm_per_px).round() as usize).max(1);
        let flat = flatten(&image().luminance(), 160, 160, 5.0 / mm_per_px);
        let want = guided(&flat, 160, 160, r);
        assert!(on
            .ink
            .iter()
            .zip(&want)
            .all(|(i, q)| (i + q - 1.0).abs() < 1e-6));
    }
    #[test]
    #[ignore = "timing: cargo test --release flatten_timing -- --ignored --nocapture"]
    fn flatten_timing() {
        let (w, h) = (4000, 2800);
        let y: Vec<f32> = (0..w * h)
            .map(|i| 0.5 + 0.5 * ((i % w) as f32 / w as f32))
            .collect();
        let start = std::time::Instant::now();
        let out = flatten(&y, w, h, 250.0);
        eprintln!(
            "flatten of {w} x {h} with a 250-pixel window: {:?}",
            start.elapsed()
        );
        assert_eq!(out.len(), y.len());
    }
    #[test]
    #[ignore = "timing: cargo test --release guided_timing -- --ignored --nocapture"]
    fn guided_timing() {
        let (w, h) = (4000, 2800);
        let y: Vec<f32> = (0..w * h)
            .map(|i| 0.5 + 0.5 * ((i % w) as f32 / w as f32))
            .collect();
        let start = std::time::Instant::now();
        let out = guided(&y, w, h, 15);
        eprintln!("guided of {w} x {h} with r = 15: {:?}", start.elapsed());
        assert_eq!(out.len(), y.len());
        let mut filter = Guided::new(y, w, h, 15);
        for n in 1..=4 {
            let start = std::time::Instant::now();
            filter.step();
            eprintln!("step {n}: {:?}", start.elapsed());
        }
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
        let s = still_life(60, 88);
        let lo = s.rgb.iter().copied().fold(1.0, f32::min);
        let hi = s.rgb.iter().copied().fold(0.0, f32::max);
        assert!(lo < 0.2 && hi > 0.95, "the still life spans {lo} to {hi}");
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
