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
//! It keeps a saliency map, the spectral residual of X. Hou and L. Zhang, "Saliency detection: a
//! spectral residual approach", *Proc. CVPR*, 2007, pp. 1-8, doi:10.1109/CVPR.2007.383267,
//! written from the paper's description. [`suggest`] searches it for a crop to the subject after
//! R. Suh, H. Ling, B. B. Bederson and D. W. Jacobs, "Automatic thumbnail cropping and its
//! effectiveness", *Proc. UIST*, 2003, pp. 95-104, doi:10.1145/964696.964707; see [`framing`].
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
use crate::stages::tone::{self, flatten};
use crate::types::{Canvas, Fit, Image, Pen, MAX_CROP};

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
/// The longer side of the grid the saliency map is made on: the scale of Hou and Zhang.
const SALIENCY_PX: usize = 64;
/// The standard deviation, in pixels of that grid, of the Gaussian that smooths the saliency
/// map: Hou and Zhang's.
const SALIENCY_SIGMA: f64 = 8.0;
/// A Fourier amplitude below this counts as this, so that its logarithm is finite.
const AMPLITUDE_FLOOR: f64 = 1e-12;
/// The sizes the crop search tries, in twentieths of the largest rectangle of the page's aspect
/// that fits the image: 1 down to 0.3.
const SIZES: std::ops::RangeInclusive<usize> = 6..=20;
/// The saliency shares the crop search tries, in twentieths: 0.5 to 0.95.
const SHARES: std::ops::RangeInclusive<usize> = 10..=19;
/// The most area, as a share of the largest rectangle's, that a suggested crop may keep. Hand-set;
/// see `docs/decisions/2026-10-16-auto-framing.md`.
const FRAMING_AREA: f64 = 0.7;
/// The least a suggested crop must raise the saliency per unit of area over the whole image's.
/// Hand-set, like [`FRAMING_AREA`].
const FRAMING_DENSITY: f64 = 1.3;
/// How near, per side, a crop must be to the suggested one to count as already in effect.
const CROP_TOLERANCE: f64 = 0.01 + 1e-9;

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
    /// The saliency map for the framing suggestion, kept out of the statistics' JSON.
    #[serde(skip)]
    saliency: Option<Saliency>,
}

/// A saliency map: `values` is `width x height`, row-major, larger where the eye is drawn.
#[derive(Clone, Debug, Default, PartialEq)]
struct Saliency {
    width: usize,
    height: usize,
    values: Vec<f64>,
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

/// One pass of a discrete Fourier transform along each of `lines` lines of `len` samples of the
/// complex grid `(re, im)`, where `at(line, j)` indexes sample `j` of a line: `X_k = sum_j x_j
/// e^{-2 pi i j k / len}`, or with `inverse` the sum with `e^{+2 pi i j k / len}`. The twiddle
/// factors are computed once per pass from [`math::cos`] and [`math::sin`], and `j k` is reduced
/// mod `len` first, so each is the correctly rounded value of a small angle.
fn dft_pass(
    (re, im): (&[f64], &[f64]),
    lines: usize,
    len: usize,
    at: impl Fn(usize, usize) -> usize,
    inverse: bool,
) -> (Vec<f64>, Vec<f64>) {
    let sign = if inverse { 1.0 } else { -1.0 };
    let angle = |m: usize| 2.0 * std::f64::consts::PI * m as f64 / len as f64;
    let cos: Vec<f64> = (0..len).map(|m| math::cos(angle(m))).collect();
    let sin: Vec<f64> = (0..len).map(|m| sign * math::sin(angle(m))).collect();
    let (mut out_re, mut out_im) = (vec![0.0; re.len()], vec![0.0; re.len()]);
    for line in 0..lines {
        for k in 0..len {
            let (mut sum_re, mut sum_im) = (0.0, 0.0);
            for j in 0..len {
                let (i, m) = (at(line, j), j * k % len);
                sum_re += re[i] * cos[m] - im[i] * sin[m];
                sum_im += re[i] * sin[m] + im[i] * cos[m];
            }
            out_re[at(line, k)] = sum_re;
            out_im[at(line, k)] = sum_im;
        }
    }
    (out_re, out_im)
}

/// The 2D discrete Fourier transform of the row-major `w x h` complex grid `(re, im)`, or with
/// `inverse` its inverse without the `1 / (w h)` factor: every row, then every column, by the
/// sums of [`dft_pass`]. There is no FFT library on purpose: the common one picks its kernels at
/// run time, which could make the native and browser builds differ in the last bit.
fn dft2(grid: (&[f64], &[f64]), w: usize, h: usize, inverse: bool) -> (Vec<f64>, Vec<f64>) {
    let rows = dft_pass(grid, h, w, |y, x| y * w + x, inverse);
    dft_pass((&rows.0, &rows.1), w, h, |x, y| y * w + x, inverse)
}

/// The spectral residual saliency map of a row-major `w x h` luminance grid, after Hou and Zhang
/// (2007). The grid is first resized to `SALIENCY_PX` pixels on its longer side, as
/// [`tone::resize`] does. With `F` that grid's Fourier transform, `L = ln |F|` (an amplitude under
/// `AMPLITUDE_FLOOR` counts as it) and `P` its phase, the spectral residual is `R = L -
/// mean3x3(L)` with the spectrum's edges wrapped, and the map is `|F^-1[exp(R + iP)]|^2` smoothed
/// by a Gaussian of `SALIENCY_SIGMA` pixels through [`math::blur`]. `None` for a grid under 8
/// pixels on a side, before or after the resize, or one whose tones differ by under 1/64: they
/// have no subject worth finding, and a flat grid's transform is only rounding noise, which the
/// residual would show.
fn saliency(grid: &[f32], gw: usize, gh: usize) -> Option<Saliency> {
    if gw.min(gh) < 8 {
        return None;
    }
    let (lum, w, h) = tone::resize(grid, gw, gh, SALIENCY_PX);
    let (lo, hi) = (lum.iter()).fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    if w.min(h) < 8 || hi - lo < 1.0 / 64.0 {
        return None;
    }
    let re: Vec<f64> = lum.iter().map(|&v| f64::from(v)).collect();
    let (fre, fim) = dft2((&re, &vec![0.0; re.len()]), w, h, false);
    let log: Vec<f64> = (fre.iter().zip(&fim))
        .map(|(&a, &b)| math::ln(math::hypot(a, b).max(AMPLITUDE_FLOOR)))
        .collect();
    let wrapped = |x: usize, dx: usize, n: usize| (x + n + dx - 1) % n;
    let (mut ure, mut uim) = (vec![0.0; re.len()], vec![0.0; re.len()]);
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for dy in 0..3 {
                for dx in 0..3 {
                    sum += log[wrapped(y, dy, h) * w + wrapped(x, dx, w)];
                }
            }
            let i = y * w + x;
            let amplitude = math::exp(log[i] - sum / 9.0);
            let phase = math::atan2(fim[i], fre[i]);
            ure[i] = amplitude * math::cos(phase);
            uim[i] = amplitude * math::sin(phase);
        }
    }
    let (sre, sim) = dft2((&ure, &uim), w, h, true);
    let n = (w * h) as f64;
    let power: Vec<f32> = (sre.iter().zip(&sim))
        .map(|(&a, &b)| ((a * a + b * b) / (n * n)) as f32)
        .collect();
    let values = math::blur(&power, w, h, SALIENCY_SIGMA);
    Some(Saliency {
        width: w,
        height: h,
        values: values.into_iter().map(f64::from).collect(),
    })
}

/// The crop that frames the subject of `map` in a rectangle of width over height `aspect`, as
/// shares cut from the left, top, right and bottom; `None` when no crop concentrates the subject.
///
/// After Suh, Ling, Bederson and Jacobs (2003): the smallest rectangle that holds a share `lambda`
/// of the saliency, with `lambda` taken where the area needed starts to climb fastest. Two
/// changes: the rectangle has the page's aspect, and on a 64-pixel map the search is exhaustive,
/// over a summed-area table, rather than greedy.
///
/// The largest rectangle of the aspect that fits the map is size 1. For each size `s` in `SIZES`
/// (1 down to 0.3 by 0.05), `f(s)` is the most saliency a rectangle of that size holds over every
/// position on the grid. For each `lambda` in `SHARES` (0.50 to 0.95 by 0.05), `A(lambda)` is the
/// area `s^2` of the smallest size with `f(s) >= lambda`, or 1 where none reaches it. The
/// `lambda` before the largest step in `A` (the smaller one on a tie) picks the size, and the
/// crop is the best rectangle of that size, the one nearest the middle on a tie. It is kept only
/// if its area is at most `FRAMING_AREA` and its saliency per unit area at least `FRAMING_DENSITY`
/// times the map's. The shares are rounded down to 0.01 so the subject is not cut, and kept to
/// `MAX_CROP`.
fn framing(map: &Saliency, aspect: f64) -> Option<[f64; 4]> {
    let (mw, mh) = (map.width, map.height);
    let stride = mw + 1;
    let mut table = vec![0.0f64; stride * (mh + 1)];
    for y in 0..mh {
        let mut row = 0.0;
        for x in 0..mw {
            row += map.values[y * mw + x];
            table[(y + 1) * stride + x + 1] = table[y * stride + x + 1] + row;
        }
    }
    let total = table[stride * (mh + 1) - 1];
    // Written as a bool so that NaN fails the check.
    let usable = total > 0.0;
    if !usable {
        return None;
    }
    let sum = |x: usize, y: usize, w: usize, h: usize| {
        table[(y + h) * stride + x + w] - table[y * stride + x + w] - table[(y + h) * stride + x]
            + table[y * stride + x]
    };
    // Size 1, in pixels of the map.
    let (w1, h1) = if aspect >= mw as f64 / mh as f64 {
        (mw as f64, mw as f64 / aspect)
    } else {
        (mh as f64 * aspect, mh as f64)
    };
    // For each size, from 1 down: its rectangle and where it holds the most.
    let best: Vec<(usize, usize, usize, usize, f64)> = SIZES
        .rev()
        .map(|twentieths| {
            let s = twentieths as f64 / 20.0;
            let rw = ((s * w1).round() as usize).clamp(1, mw);
            let rh = ((s * h1).round() as usize).clamp(1, mh);
            let off_centre = |x: usize, y: usize| {
                let (dx, dy) = (x as f64 + rw as f64 / 2.0, y as f64 + rh as f64 / 2.0);
                (dx - mw as f64 / 2.0) * (dx - mw as f64 / 2.0)
                    + (dy - mh as f64 / 2.0) * (dy - mh as f64 / 2.0)
            };
            let mut top = (0, 0, f64::MIN);
            for y in 0..=mh - rh {
                for x in 0..=mw - rw {
                    let held = sum(x, y, rw, rh);
                    let better = held > top.2
                        || (held == top.2 && off_centre(x, y) < off_centre(top.0, top.1));
                    if better {
                        top = (x, y, held);
                    }
                }
            }
            (top.0, top.1, rw, rh, top.2 / total)
        })
        .collect();
    // The index into `best` of the smallest size that holds `share`; 0, size 1, if none does.
    let smallest = |share: f64| best.iter().rposition(|b| b.4 >= share).unwrap_or(0);
    let area = |i: usize| {
        let s = (SIZES.end() - i) as f64 / 20.0;
        s * s
    };
    let areas: Vec<f64> = SHARES.map(|t| area(smallest(t as f64 / 20.0))).collect();
    let mut step = (0, f64::MIN);
    for (j, pair) in areas.windows(2).enumerate() {
        if pair[1] - pair[0] > step.1 {
            step = (j, pair[1] - pair[0]);
        }
    }
    let size = smallest((SHARES.start() + step.0) as f64 / 20.0);
    let (x, y, rw, rh, share) = best[size];
    let density = share / ((rw * rh) as f64 / (mw * mh) as f64);
    if area(size) > FRAMING_AREA || density < FRAMING_DENSITY {
        return None;
    }
    // Rounded down, so the crop keeps a little more than the rectangle; the nudge keeps a share
    // that is a whole number of hundredths from falling to the one below.
    let cut = |pixels: usize, of: usize| {
        ((pixels as f64 / of as f64 * 100.0 + 1e-9).floor() / 100.0).min(MAX_CROP)
    };
    Some([
        cut(x, mw),
        cut(y, mh),
        cut(mw - x - rw, mw),
        cut(mh - y - rh, mh),
    ])
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
        saliency: saliency(&lum, gw, gh),
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
    /// (`{width_mm, height_mm}` or `{crop}`); `None` for a note with nothing to apply.
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
const FRAMING: &str = "The subject fills only part of the image. Cropping to it draws it larger.";

/// How much larger the turned page must draw the image to be worth suggesting.
const TURN_GAIN: f64 = 1.3;

/// Suggestions for the session as it stands, in a fixed order, each left out when already in
/// effect. `settings` is `{stage: {key: value}}`; a missing `tone` or `split` key counts as its
/// default (`levels` manual, `flatten_mm` 0, `invert` off, `blur_mm` 0, `resolution` 1000, `mode`
/// one pen). `pens` are the session's: the colour suggestion gives the palette's pens the first
/// one's width, and is left out when Pen use is by colour and the pens already have the palette's
/// colours. The framing suggestion crops to the subject in the shape of the canvas's drawing area,
/// and is left out when the canvas's crop is within 0.01 of it on every side. The cut-offs are
/// hand-set starting values, not a fitted model.
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
    // The part of the image that is drawn, so a crop to the page's shape does not ask for a turn.
    let [left, top, right, bottom] = canvas.crop;
    let [w, h] = [stats.width_px, stats.height_px].map(f64::from);
    let [w, h] = [w * (1.0 - left - right), h * (1.0 - top - bottom)];
    // The turned page's drawing area is (H - 2m) x (W - 2m).
    let fit_scale = |aw: f64, ah: f64| (aw / w).min(ah / h);
    if canvas.fit == Fit::Fit
        && (w > h) != (dw > dh)
        && fit_scale(dh, dw) >= TURN_GAIN * fit_scale(dw, dh)
    {
        let patch = json!({"page": {"width_mm": canvas.height_mm, "height_mm": canvas.width_mm}});
        add("orientation", TURN, Some(patch));
    }
    if let Some(crop) = stats.saliency.as_ref().and_then(|s| framing(s, dw / dh)) {
        let held = canvas
            .crop
            .iter()
            .zip(crop)
            .all(|(a, b)| (a - b).abs() <= CROP_TOLERANCE);
        if !held {
            add("framing", FRAMING, Some(json!({"page": {"crop": crop}})));
        }
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
        color_bars, flat, flat_noise, lit_strokes, noise, ramp, scene, still_life, strokes,
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
            (
                disc(256, 256, 0.7, 0.3, 0.05),
                "framing",
                "The subject fills only part of the image. Cropping to it draws it larger.",
            ),
        ];
        for (image, kind, reason) in cases {
            let all = run(&image, "grid", &settings);
            let found = find(&all, kind).unwrap_or_else(|| panic!("no {kind} suggestion"));
            assert_eq!(found.reason, reason);
        }
    }

    /// A dark disc on a light field; its centre and radius are shares of the width and height.
    fn disc(w: u32, h: u32, cx: f32, cy: f32, r: f32) -> Image {
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / w as f32 - cx, y as f32 / h as f32 - cy);
                rgb.extend([if u * u + v * v <= r * r { 0.1 } else { 0.95 }; 3]);
            }
        }
        Image {
            width: w,
            height: h,
            rgb,
        }
    }

    /// A page whose drawing area is as wide as it is tall, the shape of a square image.
    fn square() -> Canvas {
        Canvas::new(200.0, 200.0, 0.0, Fit::Fit).unwrap()
    }

    fn landscape() -> Canvas {
        Canvas::new(297.0, 210.0, 10.0, Fit::Fit).unwrap()
    }

    /// The crop that `framing` suggests for `image` on `canvas`, if it does.
    fn crop_of(image: &Image, canvas: &Canvas) -> Option<[f64; 4]> {
        let all = suggest(
            &measure(image),
            canvas,
            "grid",
            &defaults("grid"),
            &black(0.3),
        );
        patch(&all, "framing").map(|p| serde_json::from_value(p["page"]["crop"].clone()).unwrap())
    }

    #[test]
    fn the_transform_matches_a_slow_reference() {
        let (w, h) = (8, 6);
        let re: Vec<f64> = (0..w * h)
            .map(|i| ((i * 37 + 11) % 23) as f64 / 7.0 - 1.0)
            .collect();
        let im: Vec<f64> = (0..w * h)
            .map(|i| ((i * 13 + 5) % 17) as f64 / 5.0 - 1.5)
            .collect();
        for inverse in [false, true] {
            let sign = if inverse { 1.0 } else { -1.0 };
            let (fre, fim) = dft2((&re, &im), w, h, inverse);
            for (v, u) in (0..h).flat_map(|v| (0..w).map(move |u| (v, u))) {
                let (mut sum_re, mut sum_im) = (0.0, 0.0);
                for (y, x) in (0..h).flat_map(|y| (0..w).map(move |x| (y, x))) {
                    let turns = (u * x) as f64 / w as f64 + (v * y) as f64 / h as f64;
                    let angle = sign * 2.0 * std::f64::consts::PI * turns;
                    let (c, s) = (math::cos(angle), math::sin(angle));
                    let i = y * w + x;
                    sum_re += re[i] * c - im[i] * s;
                    sum_im += re[i] * s + im[i] * c;
                }
                let i = v * w + u;
                assert!(
                    (fre[i] - sum_re).abs() < 1e-9,
                    "re {u},{v}: {} {sum_re}",
                    fre[i]
                );
                assert!(
                    (fim[i] - sum_im).abs() < 1e-9,
                    "im {u},{v}: {} {sum_im}",
                    fim[i]
                );
            }
        }
        // The inverse gives the grid back, times its pixel count.
        let (fre, fim) = dft2((&re, &im), w, h, false);
        let (bre, bim) = dft2((&fre, &fim), w, h, true);
        let n = (w * h) as f64;
        assert!(re.iter().zip(&bre).all(|(a, b)| (a - b / n).abs() < 1e-12));
        assert!(im.iter().zip(&bim).all(|(a, b)| (a - b / n).abs() < 1e-12));
    }

    #[test]
    fn a_disc_off_the_middle_gets_a_crop_that_holds_all_of_it() {
        for (image, canvas, [cx, cy, r]) in [
            (disc(256, 256, 0.3, 0.4, 0.1), square(), [0.3, 0.4, 0.1]),
            (disc(256, 256, 0.7, 0.3, 0.05), page(), [0.7, 0.3, 0.05]),
            (
                disc(256, 256, 0.25, 0.75, 0.08),
                landscape(),
                [0.25, 0.75, 0.08],
            ),
        ] {
            let crop = crop_of(&image, &canvas).expect("a crop");
            let [left, top, right, bottom] = crop;
            let held =
                left <= cx - r && top <= cy - r && 1.0 - right >= cx + r && 1.0 - bottom >= cy + r;
            assert!(held, "{crop:?} does not hold the disc at {cx}, {cy}");
            assert!(crop.iter().any(|&s| s > 0.0) && crop.iter().all(|&s| s <= MAX_CROP));
            // Whole hundredths, so the patch reads well.
            assert!(crop
                .iter()
                .all(|&s| (s * 100.0 - (s * 100.0).round()).abs() < 1e-9));
        }
    }

    #[test]
    fn the_framing_crop_follows_the_page() {
        let image = disc(256, 256, 0.7, 0.3, 0.05);
        let [tall, wide] = [page(), landscape()].map(|c| {
            let [_, _, dw, dh] = c.drawing_rect();
            let crop = crop_of(&image, &c).expect("a crop");
            // The cut image has the drawing area's shape, to the rounding of the shares and the map.
            let shape = (1.0 - crop[0] - crop[2]) / (1.0 - crop[1] - crop[3]);
            assert!(
                (shape / (dw / dh) - 1.0).abs() < 0.1,
                "{crop:?} on {dw} x {dh}"
            );
            crop
        });
        assert_ne!(tall, wide);
    }

    #[test]
    fn the_framing_crop_is_pinned() {
        // The same literals are in crates/wasm/tests/smoke.rs, so CI proves both targets agree.
        let small = disc(256, 256, 0.7, 0.3, 0.05);
        assert_eq!(crop_of(&small, &page()), Some([0.45, 0.0, 0.04, 0.29]));
        let big = disc(256, 256, 0.3, 0.4, 0.1);
        assert_eq!(crop_of(&big, &square()), Some([0.0, 0.06, 0.29, 0.23]));
    }

    #[test]
    fn images_with_no_subject_get_no_framing() {
        let images = [
            flat(256, 256, 0.5),
            ramp(256, 256, 0.0, 1.0),
            noise(256, 256, 1),
            noise(256, 256, 2),
            noise(300, 200, 3),
            flat_noise(256, 256, 0.5, 0.1, 7),
        ];
        for (i, image) in images.iter().enumerate() {
            for canvas in [page(), landscape(), square()] {
                assert_eq!(crop_of(image, &canvas), None, "image {i}");
            }
        }
    }

    #[test]
    fn the_still_life_and_the_scene_fill_their_frames() {
        // Their subjects are spread over the whole picture, so no crop concentrates them.
        for image in [still_life(600, 875), scene(800, 600), scene(600, 800)] {
            for canvas in [page(), landscape(), square()] {
                assert_eq!(crop_of(&image, &canvas), None);
            }
        }
    }

    #[test]
    fn a_thin_or_tiny_image_has_no_saliency() {
        for image in [
            ramp(200, 1, 0.0, 1.0),
            ramp(7, 40, 0.0, 1.0),
            ramp(4, 4, 0.0, 1.0),
        ] {
            assert!(measure(&image).saliency.is_none());
        }
        assert!(measure(&ramp(8, 8, 0.0, 1.0)).saliency.is_some());
    }

    #[test]
    fn applying_the_crop_removes_the_suggestion() {
        let stats = measure(&disc(256, 256, 0.3, 0.4, 0.1));
        let suggested = |canvas: &Canvas| {
            let all = suggest(&stats, canvas, "grid", &defaults("grid"), &black(0.3));
            find(&all, "framing").is_some()
        };
        let crop = [0.0, 0.06, 0.29, 0.23];
        assert!(suggested(&square()));
        assert!(!suggested(&square().with_crop(crop).unwrap()));
        // Within 0.01 on every side counts as in effect; 0.02 off does not.
        let near = crop.map(|s| s + 0.005);
        assert!(!suggested(&square().with_crop(near).unwrap()));
        let off = [0.02, 0.06, 0.29, 0.23];
        assert!(suggested(&square().with_crop(off).unwrap()));
    }

    #[test]
    fn orientation_counts_only_the_part_that_is_drawn() {
        let stats = measure(&ramp(400, 200, 0.0, 1.0));
        let turned = |canvas: &Canvas| {
            let all = suggest(&stats, canvas, "grid", &defaults("grid"), &black(0.3));
            find(&all, "orientation").is_some()
        };
        assert!(turned(&page()));
        // The middle 40 percent of the width is 160 x 200 pixels, which suits a portrait page.
        assert!(!turned(&page().with_crop([0.3, 0.0, 0.3, 0.0]).unwrap()));
    }

    #[test]
    fn framing_follows_orientation() {
        let image = disc(400, 200, 0.7, 0.3, 0.1);
        let all = run(&image, "grid", &defaults("grid"));
        let kinds: Vec<_> = all.iter().map(|s| s.kind).collect();
        assert_eq!(kinds, ["line_art", "orientation", "framing"]);
    }

    #[test]
    fn the_saliency_map_is_deterministic_and_stays_out_of_the_json() {
        let image = still_life(300, 440);
        let (a, b) = (measure(&image), measure(&image));
        assert_eq!(a, b);
        let map = a.saliency.as_ref().expect("a saliency map");
        let bits: Vec<u64> = map.values.iter().map(|v| v.to_bits()).collect();
        assert_eq!((map.width, map.height, bits.len()), (44, 64, 44 * 64));
        assert!(map.values.iter().all(|v| v.is_finite() && *v > 0.0));
        assert!(serde_json::to_value(&a).unwrap().get("saliency").is_none());
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
