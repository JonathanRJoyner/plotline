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
//! doi:10.1006/cviu.1996.0060) and the tone at the border. It also measures Otsu's separability
//! once the paper's tone is flattened out (see [`crate::stages::tone::flatten`]), which tells a
//! drawing on uneven paper from a photograph. For a colour image it also picks the pens that draw
//! it best; see [`ImageStats::palette`].
//!
//! Every value is a deterministic function of the pixels: integer histograms with fixed bin
//! edges, sums in row-major order, and `sqrt` (exact in IEEE 754) as the only float library call
//! besides `exp` and `ln` for the palette's powers, which go through [`crate::math`] (`libm`), so
//! native and WebAssembly builds agree bit for bit.

use serde::Serialize;
use serde_json::{json, Value};

use crate::math;
use crate::pens::INKS;
use crate::preview::rgb;
use crate::stages::split;
use crate::stages::tone::flatten;
use crate::types::{Canvas, Fit, Image, Pen};

/// Bins for the black and white points.
const LEVEL_BINS: usize = 4096;
/// Bins for Otsu's threshold.
const OTSU_BINS: usize = 256;
/// The longest side of the grid the statistics are measured on.
const ANALYSIS_PX: usize = 1024;
/// Colourfulness past which an image counts as a colour image, and gets a palette.
const COLOUR_CUT: f64 = 40.0;
/// Bins per channel for the palette fit's samples.
const PALETTE_BINS: usize = 8;
/// The most inks a palette holds.
const PALETTE_MAX: usize = 4;
/// The least an added ink must lower the mean colour difference to be kept: about one just
/// noticeable difference in CIELAB.
const PALETTE_GAIN: f64 = 2.3;

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
    /// Otsu's separability after the paper's tone is flattened out with a window of a sixteenth of
    /// the grid's longer side: well above `two_level` for a drawing on grey or unevenly lit paper.
    pub flat_two_level: f64,
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
    /// The inks from [`INKS`] that draw a colour image best under the Pen use stage's colour model,
    /// lightest first by luma; empty for an image that is not colourful enough (colourfulness
    /// past 40) or that no ink improves on white paper.
    pub palette: Vec<&'static str>,
    /// The palette's mean CIE 1976 colour difference ΔE*ab from the image (white paper's, if the
    /// palette is empty); 0 when the image is not colourful enough for the fit to run.
    pub palette_error: f64,
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

/// A colour in CIELAB (CIE 1976 L*a*b*) from encoded sRGB `[r, g, b]` on 0..1. Each channel is
/// decoded with the IEC 61966-2-1 curve, `c / 12.92` up to 0.04045 and `((c + 0.055) / 1.055)^2.4`
/// above it; the sRGB primaries under D65 take it to CIE XYZ, which is divided by the white point
/// (the matrix's row sums, so white is exactly L* 100, a* = b* = 0); then
/// `L = 116 f(Y) - 16`, `a = 500 (f(X) - f(Y))`, `b = 200 (f(Y) - f(Z))`, with `f(t)` the cube
/// root above `(6/29)^3` and `t / (3 (6/29)^2) + 4/29` below (CIE 15:2004, *Colorimetry*, 3rd ed.;
/// IEC 61966-2-1:1999, *Colour management in multimedia systems: default RGB colour space,
/// sRGB*). Channels outside 0..1 are clamped to it.
fn lab(srgb: [f64; 3]) -> [f64; 3] {
    const M: [[f64; 3]; 3] = [
        [0.4124, 0.3576, 0.1805],
        [0.2126, 0.7152, 0.0722],
        [0.0193, 0.1192, 0.9505],
    ];
    let linear = srgb.map(|c| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.04045 {
            c / 12.92
        } else {
            math::exp(2.4 * math::ln((c + 0.055) / 1.055))
        }
    });
    let f = |row: [f64; 3]| {
        let t = (row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2])
            / (row[0] + row[1] + row[2]);
        let edge: f64 = 6.0 / 29.0;
        if t > edge * edge * edge {
            math::exp(math::ln(t) / 3.0)
        } else {
            t / (3.0 * edge * edge) + 4.0 / 29.0
        }
    };
    let [x, y, z] = M.map(f);
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// The CIE 1976 colour difference ΔE*ab: the distance between two CIELAB colours.
fn delta_e(p: [f64; 3], q: [f64; 3]) -> f64 {
    let d = [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// One bin of the palette fit: its mean colour as encoded sRGB, that colour in CIELAB, and the
/// number of pixels.
struct Sample {
    rgb: [f64; 3],
    lab: [f64; 3],
    weight: f64,
}

/// The samples of the planes `r`, `g` and `b`: the pixels binned `PALETTE_BINS`³ on their encoded
/// values, bin `min(floor(v * 8), 7)` per channel, one sample for each non-empty bin in bin-index
/// order.
fn samples(r: &[f32], g: &[f32], b: &[f32]) -> Vec<Sample> {
    let bin = |v: f32| ((v * PALETTE_BINS as f32) as usize).min(PALETTE_BINS - 1);
    let mut bins = vec![([0.0f64; 3], 0u64); PALETTE_BINS * PALETTE_BINS * PALETTE_BINS];
    for ((&r, &g), &b) in r.iter().zip(g).zip(b) {
        let (sum, count) = &mut bins[(bin(r) * PALETTE_BINS + bin(g)) * PALETTE_BINS + bin(b)];
        for (s, v) in sum.iter_mut().zip([r, g, b]) {
            *s += f64::from(v);
        }
        *count += 1;
    }
    bins.into_iter()
        .filter(|&(_, count)| count > 0)
        .map(|(sum, count)| {
            let rgb = sum.map(|s| s / count as f64);
            Sample {
                rgb,
                lab: lab(rgb),
                weight: count as f64,
            }
        })
        .collect()
}

/// The mean ΔE*ab, weighted by pixel count, between each sample and its reproduction by inks of
/// absorption `absorb`: the colour model of the Pen use stage (see `stages/split.rs`), where
/// [`split::coverages`] gives the coverages that match the sample best and the reproduction is
/// `R_c = prod_i (1 - a_i k_ic)`. No inks reproduce the white paper.
fn mean_error(samples: &[Sample], absorb: &[[f64; 3]]) -> f64 {
    let (mut sum, mut weight) = (0.0, 0.0);
    for s in samples {
        let a = split::coverages(s.rgb, absorb);
        let shown = [0, 1, 2].map(|c| {
            a.iter()
                .zip(absorb)
                .map(|(&a, k)| 1.0 - a * k[c])
                .product::<f64>()
        });
        sum += s.weight * delta_e(s.lab, lab(shown));
        weight += s.weight;
    }
    sum / weight
}

/// The inks of [`INKS`] that draw an image best, by forward stepwise selection (T. Hastie,
/// R. Tibshirani and J. Friedman, *The Elements of Statistical Learning*, 2nd ed., Springer,
/// 2009, section 3.3.2): from no ink, each step adds the ink that lowers [`mean_error`] most, the
/// earlier ink in [`INKS`] on a tie, and the fit stops at `PALETTE_MAX` inks or when the best
/// addition lowers the error by under `PALETTE_GAIN`, about one just noticeable difference (M.
/// Mahy, L. Van Eycken and A. Oosterlinck, "Evaluation of uniform color spaces developed after
/// the adoption of CIELAB and CIELUV", *Color Research and Application* 19(2), 1994). Returns
/// the ink names, lightest first (BT.601 luma, ties in [`INKS`] order), and the chosen set's mean
/// error.
///
/// The commonest colours of the image are not the answer: the inks are transparent and mix, so a
/// green is best drawn by yellow over blue, and the inks that reproduce an image best are not its
/// most common colours.
fn fit_palette(samples: &[Sample]) -> (Vec<&'static str>, f64) {
    let absorb: Vec<[f64; 3]> = INKS
        .iter()
        .map(|&(name, hex)| {
            split::absorption(&Pen::new(name, 1.0, hex).expect("the inks are valid colours"))
        })
        .collect();
    // The set is always in `INKS` order, so the result does not depend on the order of adding.
    let error_of = |set: &[usize]| {
        let absorb: Vec<_> = set.iter().map(|&i| absorb[i]).collect();
        mean_error(samples, &absorb)
    };
    let mut chosen: Vec<usize> = Vec::new();
    let mut error = error_of(&chosen);
    while chosen.len() < PALETTE_MAX {
        let mut best: Option<(usize, f64)> = None;
        for i in (0..INKS.len()).filter(|i| !chosen.contains(i)) {
            let mut set = chosen.clone();
            set.push(i);
            set.sort_unstable();
            let e = error_of(&set);
            if best.is_none_or(|(_, b)| e < b) {
                best = Some((i, e));
            }
        }
        match best {
            Some((i, e)) if error - e >= PALETTE_GAIN => {
                chosen.push(i);
                chosen.sort_unstable();
                error = e;
            }
            _ => break,
        }
    }
    let luma = |i: usize| {
        let [r, g, b] = rgb(INKS[i].1).map(|byte| f64::from(byte) / 255.0);
        0.299 * r + 0.587 * g + 0.114 * b
    };
    chosen.sort_by(|&i, &j| luma(j).total_cmp(&luma(i)));
    (chosen.iter().map(|&i| INKS[i].0).collect(), error)
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
    let flat_two_level = otsu(&flatten(&lum, gw, gh, gw.max(gh) as f64 / 16.0)).1;
    let (otsu, two_level) = otsu(&lum);
    let colourfulness = colourfulness(&r, &g, &b);
    let (palette, palette_error) = if colourfulness > COLOUR_CUT {
        fit_palette(&samples(&r, &g, &b))
    } else {
        (Vec::new(), 0.0)
    };
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
        flat_two_level,
        dark: dark as f64 / n,
        mid: (lum.len() - dark - light) as f64 / n,
        light: light as f64 / n,
        colourfulness,
        noise: noise(&lum, gw, gh),
        border: border(&lum, gw, gh),
        palette,
        palette_error,
    }
}

/// A change that would suit the image, and why. The reasons are sentences written for a person
/// and read by the assistant as well; all but the colour one are fixed.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Suggestion {
    pub kind: &'static str,
    pub reason: String,
    /// Any of `style` (a style name), `pens` (a list of `{name, width_mm, color}`, applied after
    /// the style), `settings` (`{stage: {key: value}}`, applied after the pens) and `page`
    /// (`{width_mm, height_mm}`); `None` for a note with nothing to apply.
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
const PAPER: &str =
    "The paper's tone is uneven. Flatten paper evens it out, so the paper draws as white.";
const NOISY: &str =
    "The image is noisy. A blur of one tone-map pixel keeps the noise from turning into specks.";
const TURN: &str = "The image would draw larger with the page turned.";

/// How much larger the turned page must draw the image to be worth suggesting.
const TURN_GAIN: f64 = 1.3;

/// Suggestions for the session as it stands, in a fixed order, each left out when already in
/// effect. `settings` is `{stage: {key: value}}`; a missing `tone` or `split` key counts as its
/// default (`levels` manual, `flatten_mm` 0, `invert` off, `blur_mm` 0, `resolution` 1000, `mode`
/// one pen). `pens` are the session's: the colour suggestion gives the palette's pens the first
/// one's width, and is left out when Pen use is by colour and the pens already have the palette's
/// colours. The cut-offs are hand-set starting values, not a fitted model.
pub fn suggest(
    stats: &ImageStats,
    canvas: &Canvas,
    style: &str,
    settings: &Value,
    pens: &[Pen],
) -> Vec<Suggestion> {
    let mut out = Vec::new();
    // Nothing to say about no pixels; the zeroed statistics would trip the rules below.
    if stats.width_px == 0 || stats.height_px == 0 {
        return out;
    }
    let tone = |key: &str| settings.get("tone").and_then(|t| t.get(key));
    let mut add = |kind, reason: &str, patch| {
        out.push(Suggestion {
            kind,
            reason: reason.to_string(),
            patch,
        })
    };
    if stats.two_level >= 0.85 && stats.mid <= 0.15 && style != "ink" {
        add("line_art", LINE_ART, Some(json!({"style": "ink"})));
    }
    // Under 1/64 the tone stage keeps black 0 and white 1, so there is nothing to stretch.
    let range = stats.p_high - stats.p_low;
    if (1.0 / 64.0..0.6).contains(&range)
        && tone("levels").and_then(Value::as_str).unwrap_or("manual") == "manual"
    {
        let patch = json!({"settings": {"tone": {"levels": "auto"}}});
        add("levels", NARROW, Some(patch));
    }
    let [_, _, dw, dh] = canvas.drawing_rect();
    // Flattening that turns an image two-level is what a drawing on uneven paper shows.
    // A colour image's flat patches also flatten to two levels, and colour has its own suggestion.
    if stats.flat_two_level >= 0.85
        && stats.flat_two_level >= stats.two_level + 0.05
        && stats.colourfulness <= COLOUR_CUT
        && tone("flatten_mm").and_then(Value::as_f64).unwrap_or(0.0) <= 0.0
    {
        // A sixteenth of the longer side, to the nearest half mm and within the schema's range.
        let window = ((dw.max(dh) / 16.0 * 2.0).round() / 2.0).clamp(0.5, 50.0);
        let patch = json!({"settings": {"tone": {"flatten_mm": window}}});
        add("paper", PAPER, Some(patch));
    }
    if stats.border < 0.3
        && stats.mean < 0.5
        && stats.two_level >= 0.85
        && !tone("invert").and_then(Value::as_bool).unwrap_or(false)
    {
        let patch = json!({"settings": {"tone": {"invert": true}}});
        add("invert", LIGHT_MARKS, Some(patch));
    }
    if stats.noise > 0.03 && tone("blur_mm").and_then(Value::as_f64).unwrap_or(0.0) <= 0.0 {
        let resolution = tone("resolution").and_then(Value::as_f64).unwrap_or(1000.0);
        // The schema's largest blur is 10 mm.
        let blur = ((dw.max(dh) / resolution.max(1.0) * 10.0).ceil() / 10.0).min(10.0);
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
    let by_colour = settings
        .get("split")
        .and_then(|s| s.get("mode"))
        .and_then(Value::as_str)
        == Some("by_colour");
    if stats.colourfulness > COLOUR_CUT && stats.palette.len() >= 2 {
        let hex = |name: &str| {
            INKS.iter()
                .find(|ink| ink.0 == name)
                .map_or("", |ink| ink.1)
        };
        // In effect when the pens' colours and the palette's are the same set, ignoring case.
        let inked = |pen: &Pen| {
            stats
                .palette
                .iter()
                .any(|&n| pen.color.eq_ignore_ascii_case(hex(n)))
        };
        let held = |n: &str| pens.iter().any(|p| p.color.eq_ignore_ascii_case(hex(n)));
        let in_effect =
            by_colour && pens.iter().all(inked) && stats.palette.iter().all(|&n| held(n));
        if !in_effect {
            let width = pens.first().map_or(0.3, |p| p.width_mm);
            let list: Vec<Value> = stats
                .palette
                .iter()
                .map(|&name| {
                    let label = format!("{} {width}", capitalise(name));
                    json!({"name": label, "width_mm": width, "color": hex(name)})
                })
                .collect();
            let (last, rest) = stats.palette.split_last().expect("two or more inks");
            let named = capitalise(&format!("{} and {last}", rest.join(", ")));
            let reason = format!(
                "This is a colour image. {named} pens draw its colours best, with Pen use set to by colour."
            );
            let patch = json!({"pens": list, "settings": {"split": {"mode": "by_colour"}}});
            add("colour", &reason, Some(patch));
        }
    }
    out
}

/// `word` with its first letter in capitals.
fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_ascii_uppercase().to_string() + chars.as_str()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::get_style;
    use crate::synthetic::{
        color_bars, flat, flat_noise, lit_strokes, ramp, scene, still_life, strokes,
    };

    fn page() -> Canvas {
        Canvas::new(210.0, 297.0, 10.0, Fit::Fit).unwrap()
    }

    fn defaults(style: &str) -> Value {
        get_style(style).unwrap().defaults()
    }

    fn black(width: f64) -> Vec<Pen> {
        vec![Pen::new("Black", width, "#000000").unwrap()]
    }

    fn run(image: &Image, style: &str, settings: &Value) -> Vec<Suggestion> {
        run_with(image, style, settings, &black(0.3))
    }

    fn run_with(image: &Image, style: &str, settings: &Value, pens: &[Pen]) -> Vec<Suggestion> {
        suggest(&measure(image), &page(), style, settings, pens)
    }

    /// Pens in the given inks, in order, at 0.3 mm.
    fn inks(names: &[&str]) -> Vec<Pen> {
        let hex = |name: &str| INKS.iter().find(|ink| ink.0 == name).unwrap().1;
        names
            .iter()
            .map(|&name| Pen::new(name, 0.3, hex(name)).unwrap())
            .collect()
    }

    /// The mean ΔE*ab of drawing `image` (analysis scale 1) with only a black pen.
    fn black_alone(image: &Image) -> f64 {
        let [r, g, b] = [0, 1, 2]
            .map(|c| -> Vec<f32> { image.rgb.iter().skip(c).step_by(3).copied().collect() });
        let absorb = split::absorption(&black(1.0)[0]);
        mean_error(&samples(&r, &g, &b), &[absorb])
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
    fn flattening_makes_a_lit_drawing_two_level() {
        let lit = measure(&lit_strokes(256, 256, 0.4));
        assert!(lit.two_level < 0.8, "{}", lit.two_level);
        assert!(lit.flat_two_level >= 0.99, "{}", lit.flat_two_level);
        // Already even paper has nothing to flatten out, and a flat image no split.
        let even = measure(&strokes(256, 256, 0.0, 1.0));
        assert!((even.flat_two_level - even.two_level).abs() < 0.01);
        assert_eq!(measure(&flat(256, 256, 0.5)).flat_two_level, 0.0);
    }

    #[test]
    fn uneven_paper_suggests_flatten_paper() {
        let image = lit_strokes(256, 256, 0.4);
        let mut settings = defaults("grid");
        assert_eq!(
            patch(&run(&image, "grid", &settings), "paper"),
            Some(json!({"settings": {"tone": {"flatten_mm": 17.5}}}))
        );
        // A missing tone key counts as off, and any window turns the suggestion off.
        assert!(find(&run(&image, "grid", &json!({})), "paper").is_some());
        settings["tone"]["flatten_mm"] = json!(10.0);
        assert!(find(&run(&image, "grid", &settings), "paper").is_none());
    }

    #[test]
    fn photographs_and_even_line_art_get_no_flatten_paper() {
        let settings = defaults("grid");
        for image in [
            still_life(600, 875),
            scene(400, 300),
            ramp(256, 64, 0.0, 1.0),
            strokes(256, 256, 0.0, 1.0),
            flat(256, 256, 0.5),
            color_bars(256, 64),
        ] {
            assert!(find(&run(&image, "grid", &settings), "paper").is_none());
        }
    }

    #[test]
    fn the_flatten_window_is_a_sixteenth_of_the_longer_side() {
        let stats = measure(&lit_strokes(256, 256, 0.4));
        let window = |canvas: Canvas| {
            let all = suggest(&stats, &canvas, "grid", &json!({}), &black(0.3));
            patch(&all, "paper").unwrap()["settings"]["tone"]["flatten_mm"].clone()
        };
        // 277 / 16 = 17.3125 on A4 and 100 / 16 = 6.25, to the nearest half mm; 1000 / 16 is past the
        // schema's 50 and 5 / 16 under its half mm.
        assert_eq!(window(page()), json!(17.5));
        assert_eq!(
            window(Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap()),
            json!(6.5)
        );
        assert_eq!(
            window(Canvas::new(1000.0, 800.0, 0.0, Fit::Fit).unwrap()),
            json!(50.0)
        );
        assert_eq!(
            window(Canvas::new(5.0, 5.0, 0.0, Fit::Fit).unwrap()),
            json!(0.5)
        );
    }

    #[test]
    fn a_flat_image_gets_no_levels() {
        // The tone stage ignores a range under 1/64, so the hint would change nothing.
        let all = run(&flat(256, 256, 0.5), "grid", &defaults("grid"));
        assert!(find(&all, "levels").is_none());
    }

    #[test]
    fn missing_tone_settings_count_as_defaults() {
        let none = json!({});
        let narrow = run(&ramp(256, 64, 0.4, 0.6), "grid", &none);
        assert!(find(&narrow, "levels").is_some());
        let noisy = run(&flat_noise(256, 256, 0.5, 0.1, 7), "grid", &none);
        assert_eq!(
            patch(&noisy, "noise"),
            Some(json!({"settings": {"tone": {"blur_mm": 0.3}}}))
        );
        let marks = run(&strokes(256, 256, 1.0, 0.0), "grid", &none);
        assert!(find(&marks, "invert").is_some());
    }

    #[test]
    fn the_blur_stays_within_the_schema() {
        let mut settings = defaults("grid");
        settings["tone"]["resolution"] = json!(1);
        let all = run(&flat_noise(256, 256, 0.5, 0.1, 7), "grid", &settings);
        assert_eq!(
            patch(&all, "noise"),
            Some(json!({"settings": {"tone": {"blur_mm": 10.0}}}))
        );
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
        let pens = black(0.3);
        assert!(find(
            &suggest(&stats, &fill, "grid", &settings, &pens),
            "orientation"
        )
        .is_none());
    }

    #[test]
    fn cielab_matches_the_cie_values() {
        let near = |got: [f64; 3], want: [f64; 3], tolerance: f64| {
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() <= tolerance, "{got:?} against {want:?}");
            }
        };
        near(lab([1.0, 1.0, 1.0]), [100.0, 0.0, 0.0], 1e-9);
        near(lab([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0], 1e-12);
        // sRGB red under D65, as tabulated for the sRGB primaries.
        near(lab([1.0, 0.0, 0.0]), [53.24, 80.09, 67.20], 0.2);
        assert_eq!(delta_e([0.0, 0.0, 0.0], [3.0, 4.0, 0.0]), 5.0);
    }

    #[test]
    fn colour_bars_get_a_palette() {
        let image = color_bars(256, 64);
        let stats = measure(&image);
        assert!(
            (2..=4).contains(&stats.palette.len()),
            "{:?}",
            stats.palette
        );
        let black = black_alone(&image);
        assert!(
            stats.palette_error < black,
            "{} {black}",
            stats.palette_error
        );
        // Lightest first by BT.601 luma.
        let luma = |name: &str| {
            let hex = INKS.iter().find(|ink| ink.0 == name).unwrap().1;
            let [r, g, b] = rgb(hex).map(f64::from);
            0.299 * r + 0.587 * g + 0.114 * b
        };
        assert!(stats.palette.windows(2).all(|w| luma(w[0]) >= luma(w[1])));
    }

    #[test]
    fn a_grey_image_gets_no_palette() {
        let image = ramp(256, 64, 0.0, 1.0);
        let stats = measure(&image);
        assert!(stats.palette.is_empty());
        assert_eq!(stats.palette_error, 0.0);
        assert!(find(&run(&image, "grid", &defaults("grid")), "colour").is_none());
    }

    #[test]
    fn colour_suggests_the_palette_pens() {
        let image = color_bars(256, 64);
        let palette = measure(&image).palette;
        let all = run_with(&image, "grid", &defaults("grid"), &black(0.5));
        let colour = patch(&all, "colour").expect("colour bars are colourful");
        let pens = colour["pens"].as_array().unwrap();
        assert_eq!(pens.len(), palette.len());
        for (pen, name) in pens.iter().zip(&palette) {
            let hex = INKS.iter().find(|ink| ink.0 == *name).unwrap().1;
            let label = format!("{}{} 0.5", name[..1].to_uppercase(), &name[1..]);
            assert_eq!(pen, &json!({"name": label, "width_mm": 0.5, "color": hex}));
        }
        assert_eq!(colour["settings"], json!({"split": {"mode": "by_colour"}}));
        // With no pens at all the width is the default one.
        let none = run_with(&image, "grid", &defaults("grid"), &[]);
        assert_eq!(patch(&none, "colour").unwrap()["pens"][0]["width_mm"], 0.3);
    }

    #[test]
    fn colour_is_left_out_once_in_effect() {
        let image = color_bars(256, 64);
        let palette = measure(&image).palette;
        let mut settings = defaults("grid");
        let mut pens = inks(&palette);
        // The palette's pens alone do not set Pen use.
        assert!(find(&run_with(&image, "grid", &settings, &pens), "colour").is_some());
        settings["split"]["mode"] = json!("by_colour");
        assert!(find(&run_with(&image, "grid", &settings, &pens), "colour").is_none());
        // The set counts, not the order, the case of the hex digits or the widths.
        pens.reverse();
        for pen in &mut pens {
            pen.color = pen.color.to_uppercase();
            pen.width_mm = 0.7;
        }
        assert!(find(&run_with(&image, "grid", &settings, &pens), "colour").is_none());
        // By colour with other pens, or with a pen of the set missing, still gets the suggestion.
        let other = run_with(&image, "grid", &settings, &black(0.3));
        assert!(find(&other, "colour").is_some());
        pens.pop();
        assert!(find(&run_with(&image, "grid", &settings, &pens), "colour").is_some());
        // Splitting by tone still draws the colours in grey, and a missing stage is not by colour.
        let pens = inks(&palette);
        settings["split"]["mode"] = json!("light_to_dark");
        assert!(find(&run_with(&image, "grid", &settings, &pens), "colour").is_some());
        assert!(find(&run_with(&image, "grid", &json!({}), &pens), "colour").is_some());
    }

    #[test]
    fn the_reason_names_the_inks() {
        let stats = |palette: &[&'static str]| ImageStats {
            width_px: 10,
            height_px: 10,
            colourfulness: 50.0,
            palette: palette.to_vec(),
            ..ImageStats::default()
        };
        let reason = |palette: &[&'static str]| {
            let all = suggest(&stats(palette), &page(), "grid", &json!({}), &black(0.3));
            find(&all, "colour").map(|s| s.reason.clone())
        };
        let draws = "pens draw its colours best, with Pen use set to by colour.";
        assert_eq!(
            reason(&["yellow", "blue"]),
            Some(format!("This is a colour image. Yellow and blue {draws}"))
        );
        assert_eq!(
            reason(&["yellow", "red", "black"]),
            Some(format!(
                "This is a colour image. Yellow, red and black {draws}"
            ))
        );
        assert_eq!(
            reason(&["yellow", "red", "blue", "black"]),
            Some(format!(
                "This is a colour image. Yellow, red, blue and black {draws}"
            ))
        );
        // One ink is not worth a suggestion, and a grey image has no palette.
        assert_eq!(reason(&["black"]), None);
        assert_eq!(reason(&[]), None);
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
                lit_strokes(256, 256, 0.4),
                "paper",
                "The paper's tone is uneven. Flatten paper evens it out, so the paper draws as white.",
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
        let stats = ImageStats {
            width_px: 10,
            height_px: 10,
            colourfulness: 50.0,
            palette: vec!["yellow", "blue"],
            ..ImageStats::default()
        };
        let all = suggest(&stats, &page(), "grid", &json!({}), &black(0.3));
        assert_eq!(
            serde_json::to_value(find(&all, "colour").unwrap()).unwrap(),
            json!({
                "kind": "colour",
                "reason": "This is a colour image. Yellow and blue pens draw its colours best, with Pen use set to by colour.",
                "patch": {
                    "pens": [
                        {"name": "Yellow 0.3", "width_mm": 0.3, "color": "#f5c400"},
                        {"name": "Blue 0.3", "width_mm": 0.3, "color": "#1d4fb8"}
                    ],
                    "settings": {"split": {"mode": "by_colour"}}
                }
            })
        );
    }
}
