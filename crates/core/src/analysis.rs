//! Image statistics: what an image looks like in numbers, for the suggestions built on them.
//!
//! [`measure`] works on the luminance and colour of the image after a block average to at most
//! 1024 pixels on the longer edge. It reports the range the tones use, Otsu's two-level
//! separability (N. Otsu, "A threshold selection method from gray-level histograms", *IEEE
//! Transactions on Systems, Man, and Cybernetics* 9(1), 1979, pp. 62-66,
//! doi:10.1109/TSMC.1979.4310076), colourfulness (D. Hasler and S. Süsstrunk, "Measuring
//! colorfulness in natural images", *Human Vision and Electronic Imaging VIII*, SPIE 5007, 2003,
//! doi:10.1117/12.477378), the noise level (J. Immerkær, "Fast noise variance estimation",
//! *Computer Vision and Image Understanding* 64(2), 1996, pp. 300-302,
//! doi:10.1006/cviu.1996.0060) and the tone at the border.
//!
//! Every value is a deterministic function of the pixels: integer histograms with fixed bin
//! edges, sums in row-major order, and `sqrt` (exact in IEEE 754) as the only float library call,
//! so native and WebAssembly builds agree bit for bit.

use serde::Serialize;
use serde_json::{json, Value};

use crate::math;
use crate::types::{Canvas, Fit, Image};

/// Bins for the black and white points.
const LEVEL_BINS: usize = 4096;
/// Bins for Otsu's threshold.
const OTSU_BINS: usize = 256;
/// The longest side of the grid the statistics are measured on.
const ANALYSIS_PX: usize = 1024;

/// What [`measure`] reports about an image. Luminance is 0..1; the shares are 0..1.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ImageStats {
    /// The image's own size, not the analysis grid's.
    pub width_px: u32,
    pub height_px: u32,
    /// The 0.5th and 99.5th luminance percentiles.
    pub p_low: f64,
    pub p_high: f64,
    pub mean: f64,
    /// Otsu's threshold: the upper edge of the last bin counted as dark.
    pub otsu: f64,
    /// Otsu's separability, 0 for a flat image to 1 for a pure two-tone one.
    pub two_level: f64,
    /// Shares of pixels below 0.25, from 0.25 to 0.75, and above 0.75.
    pub dark: f64,
    pub mid: f64,
    pub light: f64,
    /// Hasler and Süsstrunk's measure on 0..255 channel values: 0 for grey, past 40 for vivid.
    pub colourfulness: f64,
    /// The noise standard deviation on 0..1 luminance, at the analysis scale.
    pub noise: f64,
    /// Mean luminance of the outer band, 5 % of the shorter edge wide.
    pub border: f64,
}

/// Counts of `lum` in `bins` equal bins on 0..1; bin `min(floor(v * bins), bins - 1)`.
fn histogram(lum: &[f32], bins: usize) -> Vec<u64> {
    let mut hist = vec![0; bins];
    for &v in lum {
        // The cast floors a positive value and sends a negative or NaN one to bin 0.
        hist[((v * bins as f32) as usize).min(bins - 1)] += 1;
    }
    hist
}

/// The first bin, in the order given, where the running count passes `limit`.
fn first_past(hist: &[u64], limit: f64, mut order: impl Iterator<Item = usize>) -> Option<usize> {
    let mut seen = 0;
    order.find(|&i| {
        seen += hist[i];
        seen as f64 > limit
    })
}

/// The black and white points that clip `clip_percent` of the pixels at each end, on a 4096-bin
/// histogram: the lower edge of the first bin where the count from the dark end exceeds
/// `clip_percent / 100 * n`, and the upper edge of the first bin where the count from the light
/// end does. With nothing to clip past (a clip of 100 or more) they are 0 and 1.
pub fn levels(lum: &[f32], clip_percent: f64) -> (f64, f64) {
    let hist = histogram(lum, LEVEL_BINS);
    let limit = clip_percent / 100.0 * lum.len() as f64;
    let bins = LEVEL_BINS as f64;
    let black = first_past(&hist, limit, 0..LEVEL_BINS).map_or(0.0, |i| i as f64 / bins);
    let white =
        first_past(&hist, limit, (0..LEVEL_BINS).rev()).map_or(1.0, |i| (i + 1) as f64 / bins);
    (black, white)
}

/// Otsu's threshold and separability `max(between-class variance) / total variance` on a 256-bin
/// histogram. A flat image has no split: its threshold stays at the first bin's edge and its
/// separability is 0.
fn otsu(lum: &[f32]) -> (f64, f64) {
    let hist = histogram(lum, OTSU_BINS);
    let n: u64 = hist.iter().sum();
    let total: u64 = hist.iter().enumerate().map(|(i, &c)| i as u64 * c).sum();
    let nf = n as f64;
    let mean = total as f64 / nf;
    let variance = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| (i as f64 - mean) * (i as f64 - mean) * c as f64)
        .sum::<f64>()
        / nf;
    // Best between-class variance so far and the bin that ends the dark class.
    let (mut best, mut best_bin) = (0.0, 0);
    let (mut n0, mut s0) = (0, 0);
    for (bin, &c) in hist.iter().enumerate() {
        n0 += c;
        s0 += bin as u64 * c;
        let n1 = n - n0;
        if n0 == 0 || n1 == 0 {
            continue;
        }
        let gap = s0 as f64 / n0 as f64 - (total - s0) as f64 / n1 as f64;
        let between = (n0 as f64 / nf) * (n1 as f64 / nf) * gap * gap;
        if between > best {
            (best, best_bin) = (between, bin);
        }
    }
    let eta = if variance == 0.0 {
        0.0
    } else {
        (best / variance).min(1.0)
    };
    ((best_bin + 1) as f64 / OTSU_BINS as f64, eta)
}

/// Hasler and Süsstrunk's colourfulness of channels `r`, `g` and `b` (0..1 planes):
/// `sqrt(var(rg) + var(yb)) + 0.3 * sqrt(mean(rg)^2 + mean(yb)^2)` with rg = R - G and
/// yb = (R + G) / 2 - B on 0..255 values.
fn colourfulness(r: &[f32], g: &[f32], b: &[f32]) -> f64 {
    let opponents = || {
        r.iter().zip(g).zip(b).map(|((&r, &g), &b)| {
            let [r, g, b] = [r, g, b].map(|c| f64::from(c) * 255.0);
            (r - g, 0.5 * (r + g) - b)
        })
    };
    let n = r.len() as f64;
    let (sum_rg, sum_yb) = opponents().fold((0.0, 0.0), |(a, c), (rg, yb)| (a + rg, c + yb));
    let (mean_rg, mean_yb) = (sum_rg / n, sum_yb / n);
    let (ss_rg, ss_yb) = opponents().fold((0.0, 0.0), |(a, c), (rg, yb)| {
        (
            a + (rg - mean_rg) * (rg - mean_rg),
            c + (yb - mean_yb) * (yb - mean_yb),
        )
    });
    (ss_rg / n + ss_yb / n).sqrt() + 0.3 * (mean_rg * mean_rg + mean_yb * mean_yb).sqrt()
}

/// The noise standard deviation of a `w x h` luminance grid: the mean absolute response of the
/// mask `[[1, -2, 1], [-2, 4, -2], [1, -2, 1]]` over the interior, times `sqrt(pi / 2)` and over
/// 6, the root of the mask's squared weights (Immerkær). 0 when a side is under 3.
fn noise(lum: &[f32], w: usize, h: usize) -> f64 {
    if w < 3 || h < 3 {
        return 0.0;
    }
    let at = |x: usize, y: usize| f64::from(lum[y * w + x]);
    let mut sum = 0.0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let corners = at(x - 1, y - 1) + at(x + 1, y - 1) + at(x - 1, y + 1) + at(x + 1, y + 1);
            let edges = at(x, y - 1) + at(x - 1, y) + at(x + 1, y) + at(x, y + 1);
            sum += (corners - 2.0 * edges + 4.0 * at(x, y)).abs();
        }
    }
    1.2533141373155003 * sum / (6 * (w - 2) * (h - 2)) as f64
}

/// Mean of the pixels in the outer band of a `w x h` grid, 5 % of the shorter edge wide and at
/// least one pixel.
fn border(lum: &[f32], w: usize, h: usize) -> f64 {
    let band = ((0.05 * w.min(h) as f64).round() as usize).max(1);
    let (mut sum, mut count) = (0.0, 0u64);
    for (i, &v) in lum.iter().enumerate() {
        let (x, y) = (i % w, i / w);
        if x < band || y < band || x + band >= w || y + band >= h {
            sum += f64::from(v);
            count += 1;
        }
    }
    sum / count as f64
}

/// Measure `image`; see [`ImageStats`]. An image with no pixels reports its size and zero for
/// everything else.
pub fn measure(image: &Image) -> ImageStats {
    let (w, h) = (image.width as usize, image.height as usize);
    if w == 0 || h == 0 {
        return ImageStats {
            width_px: image.width,
            height_px: image.height,
            ..ImageStats::default()
        };
    }
    let k = w.max(h).div_ceil(ANALYSIS_PX).max(1);
    let (lum, gw, gh) = math::block_average(&image.luminance(), w, h, k);
    let [r, g, b] = [0, 1, 2].map(|c| {
        let plane: Vec<f32> = image.rgb.iter().skip(c).step_by(3).copied().collect();
        math::block_average(&plane, w, h, k).0
    });
    let n = lum.len() as f64;
    let (p_low, p_high) = levels(&lum, 0.5);
    let (otsu, two_level) = otsu(&lum);
    let dark = lum.iter().filter(|&&v| v < 0.25).count();
    let light = lum.iter().filter(|&&v| v > 0.75).count();
    ImageStats {
        width_px: image.width,
        height_px: image.height,
        p_low,
        p_high,
        mean: lum.iter().map(|&v| f64::from(v)).sum::<f64>() / n,
        otsu,
        two_level,
        dark: dark as f64 / n,
        mid: (lum.len() - dark - light) as f64 / n,
        light: light as f64 / n,
        colourfulness: colourfulness(&r, &g, &b),
        noise: noise(&lum, gw, gh),
        border: border(&lum, gw, gh),
    }
}

/// A change that would suit the image, and why. The reasons are fixed sentences, written for a
/// person and read by the assistant as well.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Suggestion {
    pub kind: &'static str,
    pub reason: &'static str,
    /// Any of `style` (a style name), `settings` (`{stage: {key: value}}`, applied after the style)
    /// and `page` (`{width_mm, height_mm}`); `None` for a note with nothing to apply.
    pub patch: Option<Value>,
}

/// What [`measure`] found and what [`suggest`] makes of it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Analysis {
    pub stats: ImageStats,
    pub suggestions: Vec<Suggestion>,
}

const LINE_ART: &str = "This looks like line art. The Ink style keeps its shapes and line weights.";
const NARROW: &str =
    "The image uses a narrow range of tones. Auto levels stretch it over the full range.";
const LIGHT_MARKS: &str =
    "This looks like light marks on a dark ground. Inverting it gives the marks the ink.";
const NOISY: &str =
    "The image is noisy. A blur of one tone-map pixel keeps the noise from turning into specks.";
const TURN: &str = "The image would draw larger with the page turned.";
const COLOUR: &str = "This is a colour image. Each pen draws one colour, and splitting colours across pens is not built yet, so it is drawn in the first pen's tones.";

/// How much larger the turned page must draw the image to be worth suggesting.
const TURN_GAIN: f64 = 1.3;

/// Suggestions for the session as it stands, in a fixed order, each left out when already in
/// effect. `settings` is `{stage: {key: value}}`; a missing `tone` key counts as its default
/// (`levels` manual, `invert` off, `blur_mm` 0, `resolution` 1000). The cut-offs are hand-set
/// starting values, not a fitted model.
pub fn suggest(
    stats: &ImageStats,
    canvas: &Canvas,
    style: &str,
    settings: &Value,
) -> Vec<Suggestion> {
    let mut out = Vec::new();
    // Nothing to say about no pixels; the zeroed statistics would trip the rules below.
    if stats.width_px == 0 || stats.height_px == 0 {
        return out;
    }
    let tone = |key: &str| settings.get("tone").and_then(|t| t.get(key));
    let mut add = |kind, reason, patch| {
        out.push(Suggestion {
            kind,
            reason,
            patch,
        })
    };
    if stats.two_level >= 0.85 && stats.mid <= 0.15 && style != "ink" {
        add("line_art", LINE_ART, Some(json!({"style": "ink"})));
    }
    if stats.p_high - stats.p_low < 0.6
        && tone("levels").and_then(Value::as_str).unwrap_or("manual") == "manual"
    {
        let patch = json!({"settings": {"tone": {"levels": "auto"}}});
        add("levels", NARROW, Some(patch));
    }
    if stats.border < 0.3
        && stats.mean < 0.5
        && stats.two_level >= 0.85
        && !tone("invert").and_then(Value::as_bool).unwrap_or(false)
    {
        let patch = json!({"settings": {"tone": {"invert": true}}});
        add("invert", LIGHT_MARKS, Some(patch));
    }
    let [_, _, dw, dh] = canvas.drawing_rect();
    if stats.noise > 0.03 && tone("blur_mm").and_then(Value::as_f64).unwrap_or(0.0) <= 0.0 {
        let resolution = tone("resolution").and_then(Value::as_f64).unwrap_or(1000.0);
        let blur = (dw.max(dh) / resolution.max(1.0) * 10.0).ceil() / 10.0;
        let patch = json!({"settings": {"tone": {"blur_mm": blur}}});
        add("noise", NOISY, Some(patch));
    }
    let [w, h] = [stats.width_px, stats.height_px].map(f64::from);
    // The turned page's drawing area is (H - 2m) x (W - 2m).
    let fit_scale = |aw: f64, ah: f64| (aw / w).min(ah / h);
    if canvas.fit == Fit::Fit
        && (w > h) != (dw > dh)
        && fit_scale(dh, dw) >= TURN_GAIN * fit_scale(dw, dh)
    {
        let patch = json!({"page": {"width_mm": canvas.height_mm, "height_mm": canvas.width_mm}});
        add("orientation", TURN, Some(patch));
    }
    if stats.colourfulness > 40.0 {
        add("colour", COLOUR, None);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::get_style;
    use crate::synthetic::{color_bars, flat, flat_noise, ramp, strokes};

    fn page() -> Canvas {
        Canvas::new(210.0, 297.0, 10.0, Fit::Fit).unwrap()
    }

    fn defaults(style: &str) -> Value {
        get_style(style).unwrap().defaults()
    }

    fn run(image: &Image, style: &str, settings: &Value) -> Vec<Suggestion> {
        suggest(&measure(image), &page(), style, settings)
    }

    fn find<'a>(all: &'a [Suggestion], kind: &str) -> Option<&'a Suggestion> {
        all.iter().find(|s| s.kind == kind)
    }

    fn patch(all: &[Suggestion], kind: &str) -> Option<Value> {
        find(all, kind).and_then(|s| s.patch.clone())
    }

    #[test]
    fn line_art_is_two_level() {
        let s = measure(&strokes(256, 256, 0.0, 1.0));
        assert!(s.two_level >= 0.85, "{}", s.two_level);
        assert!(s.mid <= 0.15, "{}", s.mid);
    }

    #[test]
    fn a_gradient_is_not() {
        let s = measure(&ramp(256, 64, 0.0, 1.0));
        assert!(s.two_level < 0.85, "{}", s.two_level);
        assert!((s.mid - 0.5).abs() < 0.05, "{}", s.mid);
        assert!(s.p_high - s.p_low > 0.9, "{} {}", s.p_low, s.p_high);
    }

    #[test]
    fn a_narrow_ramp_has_a_narrow_range() {
        let s = measure(&ramp(256, 64, 0.4, 0.6));
        assert!(s.p_high - s.p_low < 0.25, "{} {}", s.p_low, s.p_high);
    }

    #[test]
    fn noise_is_estimated() {
        let s = measure(&flat_noise(256, 256, 0.5, 0.1, 7));
        assert!((s.noise - 0.1).abs() < 0.02, "{}", s.noise);
        let s = measure(&flat(256, 256, 0.5));
        assert_eq!(s.noise, 0.0);
        assert_eq!(s.two_level, 0.0);
    }

    #[test]
    fn light_marks_on_dark() {
        let s = measure(&strokes(256, 256, 1.0, 0.0));
        assert!(s.border < 0.3, "{}", s.border);
        assert!(s.mean < 0.5, "{}", s.mean);
    }

    #[test]
    fn colour_bars_are_colourful() {
        assert!(measure(&color_bars(256, 64)).colourfulness > 40.0);
        assert_eq!(measure(&ramp(256, 64, 0.0, 1.0)).colourfulness, 0.0);
    }

    #[test]
    fn tiny_images_measure() {
        assert_eq!(measure(&flat(1, 1, 0.5)).noise, 0.0);
        assert_eq!(measure(&flat(2, 2, 0.5)).noise, 0.0);
    }

    #[test]
    fn empty_images_measure() {
        for (w, h) in [(0, 0), (0, 5), (5, 0)] {
            let s = measure(&flat(w, h, 0.5));
            assert_eq!((s.width_px, s.height_px), (w, h));
            assert_eq!(s.two_level, 0.0);
        }
    }

    #[test]
    fn big_images_are_downsampled() {
        let big = measure(&ramp(3000, 10, 0.0, 1.0));
        assert_eq!(big.width_px, 3000);
        assert_eq!(big.height_px, 10);
        let small = measure(&ramp(1000, 10, 0.0, 1.0));
        assert!((big.mean - small.mean).abs() < 0.01);
    }

    #[test]
    fn levels_finds_the_clip_points() {
        let lum: Vec<f32> = (0..1000).map(|i| i as f32 / 999.0).collect();
        let (black, white) = levels(&lum, 0.5);
        assert!((black - 0.005).abs() <= 1.0 / 4096.0, "{black}");
        assert!((white - 0.995).abs() <= 1.0 / 4096.0, "{white}");
    }

    #[test]
    fn line_art_suggests_ink() {
        let image = strokes(256, 256, 0.0, 1.0);
        let all = run(&image, "grid", &defaults("grid"));
        assert_eq!(patch(&all, "line_art"), Some(json!({"style": "ink"})));
        assert!(find(&run(&image, "ink", &defaults("ink")), "line_art").is_none());
    }

    #[test]
    fn a_narrow_range_suggests_auto_levels() {
        let image = ramp(256, 64, 0.4, 0.6);
        let mut settings = defaults("grid");
        assert_eq!(
            patch(&run(&image, "grid", &settings), "levels"),
            Some(json!({"settings": {"tone": {"levels": "auto"}}}))
        );
        settings["tone"]["levels"] = json!("auto");
        assert!(find(&run(&image, "grid", &settings), "levels").is_none());
    }

    #[test]
    fn light_marks_suggest_invert() {
        let image = strokes(256, 256, 1.0, 0.0);
        let mut settings = defaults("grid");
        assert_eq!(
            patch(&run(&image, "grid", &settings), "invert"),
            Some(json!({"settings": {"tone": {"invert": true}}}))
        );
        settings["tone"]["invert"] = json!(true);
        assert!(find(&run(&image, "grid", &settings), "invert").is_none());
        // A dark photograph is not two-level.
        let dark = run(&ramp(256, 64, 0.0, 0.4), "grid", &defaults("grid"));
        assert!(find(&dark, "invert").is_none());
    }

    #[test]
    fn noise_suggests_one_tone_pixel_of_blur() {
        let image = flat_noise(256, 256, 0.5, 0.1, 7);
        let mut settings = defaults("grid");
        assert_eq!(settings["tone"]["resolution"], json!(1000));
        assert_eq!(
            patch(&run(&image, "grid", &settings), "noise"),
            Some(json!({"settings": {"tone": {"blur_mm": 0.3}}}))
        );
        settings["tone"]["blur_mm"] = json!(0.5);
        assert!(find(&run(&image, "grid", &settings), "noise").is_none());
    }

    #[test]
    fn a_landscape_image_suggests_turning_the_page() {
        let settings = defaults("grid");
        let wide = run(&ramp(400, 200, 0.0, 1.0), "grid", &settings);
        assert_eq!(
            patch(&wide, "orientation"),
            Some(json!({"page": {"width_mm": 297.0, "height_mm": 210.0}}))
        );
        let tall = run(&ramp(200, 400, 0.0, 1.0), "grid", &settings);
        assert!(find(&tall, "orientation").is_none());
        let mut fill = page();
        fill.fit = Fit::Fill;
        let stats = measure(&ramp(400, 200, 0.0, 1.0));
        assert!(find(&suggest(&stats, &fill, "grid", &settings), "orientation").is_none());
    }

    #[test]
    fn colour_is_a_note() {
        let all = run(&color_bars(256, 64), "grid", &defaults("grid"));
        let colour = find(&all, "colour").expect("colour bars are colourful");
        assert_eq!(colour.patch, None);
    }

    #[test]
    fn rules_run_in_order() {
        let all = run(
            &flat_noise(256, 256, 0.5, 0.1, 7),
            "grid",
            &defaults("grid"),
        );
        let kinds: Vec<_> = all.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, ["levels", "noise"]);
    }

    #[test]
    fn empty_images_suggest_nothing() {
        for (w, h) in [(0, 0), (0, 5), (5, 0)] {
            assert!(run(&flat(w, h, 0.5), "grid", &defaults("grid")).is_empty());
        }
    }

    #[test]
    fn reasons_are_the_fixed_sentences() {
        let settings = defaults("grid");
        let cases = [
            (
                strokes(256, 256, 0.0, 1.0),
                "line_art",
                "This looks like line art. The Ink style keeps its shapes and line weights.",
            ),
            (
                ramp(256, 64, 0.4, 0.6),
                "levels",
                "The image uses a narrow range of tones. Auto levels stretch it over the full range.",
            ),
            (
                strokes(256, 256, 1.0, 0.0),
                "invert",
                "This looks like light marks on a dark ground. Inverting it gives the marks the ink.",
            ),
            (
                flat_noise(256, 256, 0.5, 0.1, 7),
                "noise",
                "The image is noisy. A blur of one tone-map pixel keeps the noise from turning into specks.",
            ),
            (
                ramp(400, 200, 0.0, 1.0),
                "orientation",
                "The image would draw larger with the page turned.",
            ),
            (
                color_bars(256, 64),
                "colour",
                "This is a colour image. Each pen draws one colour, and splitting colours across pens is not built yet, so it is drawn in the first pen's tones.",
            ),
        ];
        for (image, kind, reason) in cases {
            let all = run(&image, "grid", &settings);
            let found = find(&all, kind).unwrap_or_else(|| panic!("no {kind} suggestion"));
            assert_eq!(found.reason, reason);
        }
    }

    #[test]
    fn serialises_as_the_spec_shows() {
        let all = run(&strokes(256, 256, 0.0, 1.0), "grid", &defaults("grid"));
        assert_eq!(
            serde_json::to_value(find(&all, "line_art").unwrap()).unwrap(),
            json!({
                "kind": "line_art",
                "reason": "This looks like line art. The Ink style keeps its shapes and line weights.",
                "patch": {"style": "ink"}
            })
        );
        let note = run(&color_bars(256, 64), "grid", &defaults("grid"));
        let note = serde_json::to_value(find(&note, "colour").unwrap()).unwrap();
        assert_eq!(note["patch"], Value::Null);
    }
}
