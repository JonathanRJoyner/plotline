//! Tone match: how closely a drawing's tones follow the tone map it was drawn from, at viewing
//! distance.
//!
//! The coverage raster's inked cells are averaged into the tone map's pixels, and both maps are
//! blurred by the same Gaussian eye filter, the blur-and-compare of model-based halftoning (T. N.
//! Pappas and D. L. Neuhoff, "Least-squares model-based halftoning", *IEEE Trans. Image
//! Processing* 8(8), 1999, pp. 1102-1116), with a Gaussian standing in for the eye's contrast
//! sensitivity (R. Näsänen, "Visibility of halftone dot textures", *IEEE Trans. SMC* 14(6), 1984,
//! pp. 920-924). No style reaches full black, and some ink their white, so the blurred coverage is
//! fitted to the blurred tone by a least-squares line and put on the tone map's scale before the
//! two are compared: a drawing is judged on how its tones follow the image, not on how dark it
//! gets. The RMS difference is the tone error; the structure is scored separately with SSIM (Z.
//! Wang, A. C. Bovik, H. R. Sheikh and E. P. Simoncelli, "Image quality assessment: from error
//! visibility to structural similarity", *IEEE Trans. Image Processing* 13(4), 2004, pp.
//! 600-612). All sums run in pixel order in f64, so the numbers are the same on every target.
//!
//! How dark the drawing gets is reported beside the error as the fitted line's rise from white to
//! full ink, its contrast. The error never exceeds blank paper's, the spread of the tone map about
//! its mean. [`ToneMatch::score`], which Optimize ranks, divides the error by a power of the
//! contrast, so of two drawings that follow the tones alike the darker one scores better (see
//! `docs/decisions/2026-10-17-contrast-and-colour-match.md`). A drawing in more than one colour is
//! also compared with the source image in CIELAB by [`colour`].

use serde::Serialize;

use crate::analysis::{delta_e, lab};
use crate::math;
use crate::preview::{render_ink, rgb};
use crate::types::{Drawing, Image, ToneMap};

/// The eye filter's sigma in mm: line texture at the default spacings of 1 to 2 mm fades under
/// it, as when a plot is seen from a metre or two. A taste default, recorded in the brief.
pub const EYE_SIGMA_MM: f64 = 1.5;
/// Below this gain the drawing's ink does not follow its tone, and there is nothing to compare.
const MIN_GAIN: f64 = 0.01;
/// The power of the contrast that [`ToneMatch::score`] divides the error by; measured on the
/// gallery's images, see `docs/decisions/2026-10-17-contrast-and-colour-match.md`.
pub const CONTRAST_WEIGHT: f64 = 0.25;
/// SSIM's constants for values in 0..1: `(0.01)^2` and `(0.03)^2` (Wang et al., 2004).
const C1: f64 = 1e-4;
const C2: f64 = 9e-4;

/// How a drawing's tones follow its tone map.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ToneMatch {
    /// The RMS difference, in tone-map ink, between the tone map and the drawing's coverage put on
    /// its scale, both seen through the eye filter; 0 is a perfect match, and never above
    /// `blank`.
    pub error: f64,
    /// The mean structural similarity of the same two maps; 1 is identical.
    pub ssim: f64,
    /// The coverage the fitted line puts at white, and at full ink.
    pub floor: f64,
    pub reach: f64,
    /// How much of the tone map's range the drawing reaches, unscaled: the fitted line's rise
    /// from white to full ink, `reach - floor`, and 0 when it does not rise. At 1 the drawing inks
    /// as much more where the image is darkest than where it is lightest as the tone map does.
    pub contrast: f64,
    /// Blank paper's tone error: the RMS of the blurred tone map about its mean, which a drawing
    /// whose ink does not follow the tones scores.
    pub blank: f64,
    /// How the drawing's colours follow the source image, for a drawing in more than one colour;
    /// see [`colour`].
    pub colour: Option<ColourMatch>,
}

impl ToneMatch {
    /// What Optimize ranks, lower is better: the error as a share of blank paper's, divided by the
    /// contrast, at most 1, to the power [`CONTRAST_WEIGHT`]. Infinite for a drawing with no
    /// contrast or no better than blank paper, so blank paper never beats a drawing.
    pub fn score(&self) -> f64 {
        if self.contrast <= 0.0 || self.error >= self.blank {
            return f64::INFINITY;
        }
        let shape = self.error / self.blank;
        shape / math::exp(CONTRAST_WEIGHT * math::ln(self.contrast.min(1.0)))
    }
}

/// How a drawing's colours follow its source image, at viewing distance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ColourMatch {
    /// The mean CIE 1976 colour difference ΔE*ab between the drawing and the source image, both
    /// through the eye filter, over the image's rectangle.
    pub error: f64,
    /// The same for blank paper.
    pub blank: f64,
    /// The share of blank paper's difference that the drawing takes away, `1 - error / blank`: 0
    /// for blank paper, 1 for a perfect match, below 0 for a drawing further from the image.
    pub gain: f64,
}

/// The share of inked cells over each pixel of `t`: the box average of a raster of `width` by
/// `height` square cells of `cell_mm` from `origin_mm`, row-major, at the tone map's scale. A
/// pixel no cell centre falls in takes the cell under its own centre, or nothing off the raster.
pub fn pixel_coverage(
    inked: &[bool],
    width: usize,
    height: usize,
    origin_mm: [f64; 2],
    cell_mm: f64,
    t: &ToneMap,
) -> Vec<f32> {
    let (tw, th) = (t.width as usize, t.height as usize);
    // The pixel along one axis that a cell centre falls in, if any.
    let pixel = |i: usize, axis: usize, pixels: usize| {
        let mm = origin_mm[axis] + (i as f64 + 0.5) * cell_mm;
        let p = ((mm - t.origin_mm[axis]) / t.mm_per_px).floor();
        (0.0..pixels as f64).contains(&p).then_some(p as usize)
    };
    let xs: Vec<Option<usize>> = (0..width).map(|i| pixel(i, 0, tw)).collect();
    let (mut hits, mut cells) = (vec![0u32; tw * th], vec![0u32; tw * th]);
    for row in 0..height {
        let Some(y) = pixel(row, 1, th) else { continue };
        for (x, &on) in xs.iter().zip(&inked[row * width..(row + 1) * width]) {
            if let Some(x) = x {
                hits[y * tw + x] += u32::from(on);
                cells[y * tw + x] += 1;
            }
        }
    }
    // The cell along one axis under a pixel's centre, if any.
    let cell = |p: usize, axis: usize, n: usize| {
        let mm = t.origin_mm[axis] + (p as f64 + 0.5) * t.mm_per_px;
        let c = ((mm - origin_mm[axis]) / cell_mm).floor();
        (0.0..n as f64).contains(&c).then_some(c as usize)
    };
    (0..tw * th)
        .map(|i| match cells[i] {
            0 => match (cell(i % tw, 0, width), cell(i / tw, 1, height)) {
                (Some(x), Some(y)) => f32::from(u8::from(inked[y * width + x])),
                _ => 0.0,
            },
            n => (f64::from(hits[i]) / f64::from(n)) as f32,
        })
        .collect()
}

/// Compare `coverage`, one value per pixel of `t` as [`pixel_coverage`] gives it, with `t`'s ink;
/// see [`ToneMatch`], whose `colour` this leaves out. `None` when the tone map is flat.
pub fn compare(coverage: &[f32], t: &ToneMap) -> Option<ToneMatch> {
    let f = Fitted::new(coverage, t)?;
    let (w, h) = (t.width as usize, t.height as usize);
    Some(ToneMatch {
        error: f.error,
        ssim: ssim(&f.x, &f.y, w, h, f.sigma),
        floor: f.floor,
        reach: f.floor + f.gain,
        contrast: f.gain.max(0.0),
        blank: f.blank,
        colour: None,
    })
}

/// Both maps through the eye filter, the coverage put on the tone map's scale by the fitted line,
/// and the RMS difference, at most blank paper's. Below [`MIN_GAIN`] the coverage cannot be put on
/// the scale and is taken as the tone map's mean, blank paper's best guess.
struct Fitted {
    x: Vec<f32>,
    y: Vec<f32>,
    sigma: f64,
    floor: f64,
    gain: f64,
    error: f64,
    blank: f64,
}

impl Fitted {
    fn new(coverage: &[f32], t: &ToneMap) -> Option<Fitted> {
        let (w, h) = (t.width as usize, t.height as usize);
        let sigma = EYE_SIGMA_MM / t.mm_per_px;
        let c = math::blur(coverage, w, h, sigma);
        let y = math::blur(&t.ink, w, h, sigma);
        let n = (w * h) as f64;
        let mean = |v: &[f32]| v.iter().map(|&a| f64::from(a)).sum::<f64>() / n;
        let (mc, my) = (mean(&c), mean(&y));
        let (mut var, mut cov) = (0.0, 0.0);
        for (&a, &b) in c.iter().zip(&y) {
            let (dc, dy) = (f64::from(a) - mc, f64::from(b) - my);
            var += dy * dy;
            cov += dy * dc;
        }
        if var <= 0.0 {
            return None;
        }
        let gain = cov / var;
        let floor = mc - gain * my;
        let blank = (var / n).sqrt();
        if gain < MIN_GAIN {
            return Some(Fitted {
                x: vec![my as f32; w * h],
                y,
                sigma,
                floor,
                gain,
                error: blank,
                blank,
            });
        }
        // The coverage on the tone map's scale.
        let x: Vec<f32> = c
            .iter()
            .map(|&a| ((f64::from(a) - floor) / gain) as f32)
            .collect();
        let sq = x
            .iter()
            .zip(&y)
            .map(|(&a, &b)| (f64::from(a) - f64::from(b)) * (f64::from(a) - f64::from(b)))
            .sum::<f64>();
        Some(Fitted {
            x,
            y,
            sigma,
            floor,
            gain,
            error: (sq / n).sqrt().min(blank),
            blank,
        })
    }
}

/// The page is rendered at this many pixels per mm and averaged down to [`COLOUR_PX_MM`], so a
/// line a few tenths of a millimetre wide keeps its share of each compared pixel.
const RENDER_PX_MM: f64 = 4.0;
const COLOUR_PX_MM: f64 = 2.0;

/// How the colours of `d`, drawn on `paper`, follow `image` at viewing distance; see
/// [`ColourMatch`]. The drawing is rendered as the preview draws its ink, both pictures are put on a
/// grid of [`COLOUR_PX_MM`] over the tone map's rectangle, the part of the image the page shows,
/// and blurred by the eye filter, and each pixel's CIE 1976 ΔE*ab is averaged (CIE 15:2004,
/// *Colorimetry*, 3rd ed.), and blank paper's the same way. `None` without a tone map, when its
/// rectangle has no pixel, or when blank paper matches the image.
pub fn colour(d: &Drawing, image: &Image, paper: &str) -> Option<ColourMatch> {
    let page_px = (d.canvas.width_mm * RENDER_PX_MM).round() as u32;
    let (rw, rh, rgba) = render_ink(d, page_px, paper);
    let [ix, iy, iw, ih] = d.tone.as_ref()?.rect_mm();
    let k = (RENDER_PX_MM / COLOUR_PX_MM) as usize;
    let (w, h) = (
        (iw * COLOUR_PX_MM).floor() as usize,
        (ih * COLOUR_PX_MM).floor() as usize,
    );
    if rgba.is_empty() || w == 0 || h == 0 || image.width == 0 || image.height == 0 {
        return None;
    }
    let (ox, oy) = (
        (ix * RENDER_PX_MM).round() as usize,
        (iy * RENDER_PX_MM).round() as usize,
    );
    let (rw, rh) = (rw as usize, rh as usize);
    let sw = image.width as usize;
    let [x0, y0, cw, ch] = d.canvas.shown(image.width, image.height);
    let mut drawn = [vec![0f32; w * h], vec![0f32; w * h], vec![0f32; w * h]];
    let mut source = [vec![0f32; w * h], vec![0f32; w * h], vec![0f32; w * h]];
    for y in 0..h {
        for x in 0..w {
            // The box average of the rendered pixels, in encoded values: close enough to compare.
            let mut sum = [0u32; 3];
            for dy in 0..k {
                for dx in 0..k {
                    let px = (ox + x * k + dx).min(rw - 1);
                    let py = (oy + y * k + dy).min(rh - 1);
                    let i = 4 * (py * rw + px);
                    for (s, &v) in sum.iter_mut().zip(&rgba[i..i + 3]) {
                        *s += u32::from(v);
                    }
                }
            }
            // The source pixel under the grid pixel's centre, in the part the page shows.
            let sx = x0 + ((2 * x + 1) * cw / (2 * w)).min(cw - 1);
            let sy = y0 + ((2 * y + 1) * ch / (2 * h)).min(ch - 1);
            let si = 3 * (sy * sw + sx);
            for c in 0..3 {
                drawn[c][y * w + x] = sum[c] as f32 / (255 * k * k) as f32;
                source[c][y * w + x] = image.rgb[si + c];
            }
        }
    }
    let sigma = EYE_SIGMA_MM * COLOUR_PX_MM;
    let [dr, dg, db] = drawn.map(|p| math::blur(&p, w, h, sigma));
    let [sr, sg, sb] = source.map(|p| math::blur(&p, w, h, sigma));
    let paper_lab = lab(rgb(paper).map(|v| f64::from(v) / 255.0));
    let (mut error, mut blank) = (0.0, 0.0);
    for i in 0..w * h {
        let s = lab([sr[i], sg[i], sb[i]].map(f64::from));
        error += delta_e(lab([dr[i], dg[i], db[i]].map(f64::from)), s);
        blank += delta_e(paper_lab, s);
    }
    let n = (w * h) as f64;
    let (error, blank) = (error / n, blank / n);
    (blank > 0.0).then(|| ColourMatch {
        error,
        blank,
        gain: 1.0 - error / blank,
    })
}

/// Whether `d` draws in more than one colour, which the luminance tone map cannot judge.
pub fn coloured(d: &Drawing) -> bool {
    let first = d.layers.first().map(|l| l.pen.color.to_ascii_lowercase());
    (d.layers.iter()).any(|l| Some(l.pen.color.to_ascii_lowercase()) != first)
}

/// The mean SSIM of `x` and `y`, with local statistics over Gaussian windows of `sigma` pixels.
fn ssim(x: &[f32], y: &[f32], w: usize, h: usize, sigma: f64) -> f64 {
    let product = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(p, q)| p * q).collect::<Vec<_>>();
    let local = |v: &[f32]| math::blur(v, w, h, sigma);
    let (mx, my) = (local(x), local(y));
    let (xx, yy, xy) = (
        local(&product(x, x)),
        local(&product(y, y)),
        local(&product(x, y)),
    );
    let mut total = 0.0;
    for i in 0..w * h {
        let (ax, ay) = (f64::from(mx[i]), f64::from(my[i]));
        let (vx, vy) = (f64::from(xx[i]) - ax * ax, f64::from(yy[i]) - ay * ay);
        let cxy = f64::from(xy[i]) - ax * ay;
        total +=
            ((2.0 * ax * ay + C1) * (2.0 * cxy + C2)) / ((ax * ax + ay * ay + C1) * (vx + vy + C2));
    }
    total / (w * h) as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone map of `w` by `h` pixels of 0.5 mm from the page's corner.
    fn map(w: u32, h: u32, ink: impl Fn(usize, usize) -> f32) -> ToneMap {
        let ink = (0..(w * h) as usize)
            .map(|i| ink(i % w as usize, i / w as usize))
            .collect();
        ToneMap {
            width: w,
            height: h,
            ink,
            mm_per_px: 0.5,
            origin_mm: [0.0, 0.0],
            gamma: 1.0,
        }
    }

    /// A smooth ramp with a dark disc, so the map has both tone and structure.
    fn scene() -> ToneMap {
        map(80, 60, |x, y| {
            let (dx, dy) = (x as f32 - 50.0, y as f32 - 30.0);
            let disc = if dx * dx + dy * dy < 150.0 { 0.4 } else { 0.0 };
            (x as f32 / 79.0 * 0.6 + disc).min(1.0)
        })
    }

    #[test]
    fn coverage_equal_to_the_tone_matches_exactly() {
        let t = scene();
        let m = compare(&t.ink, &t).unwrap();
        assert!(m.error < 1e-5 && (m.ssim - 1.0).abs() < 1e-5, "{m:?}");
        assert!(
            m.floor.abs() < 1e-5 && (m.reach - 1.0).abs() < 1e-5,
            "{m:?}"
        );
        assert!((m.contrast - 1.0).abs() < 1e-5, "{m:?}");
    }

    #[test]
    fn a_scale_and_a_floor_are_not_errors() {
        let t = scene();
        let c: Vec<f32> = t.ink.iter().map(|v| 0.15 + 0.5 * v).collect();
        let m = compare(&c, &t).unwrap();
        assert!(m.error < 1e-4 && m.ssim > 0.9999, "{m:?}");
        assert!(
            (m.floor - 0.15).abs() < 1e-4 && (m.reach - 0.65).abs() < 1e-4,
            "{m:?}"
        );
    }

    #[test]
    fn a_fainter_drawing_of_the_same_shape_scores_worse() {
        let t = scene();
        // Every drawing has some error, so the two shapes are the same, slightly bent.
        let dark: Vec<f32> = t.ink.iter().map(|v| 0.6 * v * v).collect();
        let faint: Vec<f32> = t.ink.iter().map(|v| 0.06 * v * v).collect();
        let (dark, faint) = (compare(&dark, &t).unwrap(), compare(&faint, &t).unwrap());
        assert!(
            (dark.error - faint.error).abs() < 1e-4,
            "{dark:?} {faint:?}"
        );
        assert!(dark.contrast > 5.0 * faint.contrast, "{dark:?} {faint:?}");
        assert!(dark.score() < faint.score(), "{dark:?} {faint:?}");
    }

    #[test]
    fn the_error_never_exceeds_blank_papers() {
        let t = scene();
        // A gain just over the floor, whose fitted error would be divided by almost nothing.
        let noisy: Vec<f32> = (t.ink.iter().enumerate())
            .map(|(i, v)| 0.02 * v + if i % 2 == 0 { 0.3 } else { 0.0 })
            .collect();
        let m = compare(&noisy, &t).unwrap();
        assert!(m.error <= m.blank && m.blank > 0.0, "{m:?}");
    }

    #[test]
    fn a_bent_response_scores_worse_than_a_straight_one() {
        let t = scene();
        let bent: Vec<f32> = t.ink.iter().map(|v| v * v * v).collect();
        let m = compare(&bent, &t).unwrap();
        assert!(m.error > 0.03 && m.ssim < 0.999, "{m:?}");
    }

    #[test]
    fn a_blank_drawing_scores_blank_paper_and_a_flat_map_has_no_match() {
        let t = scene();
        for flat in [0.0, 0.4] {
            let m = compare(&vec![flat; t.ink.len()], &t).unwrap();
            assert_eq!((m.error, m.contrast), (m.blank, 0.0), "{m:?}");
            assert_eq!(m.score(), f64::INFINITY);
        }
        let flat = map(20, 20, |_, _| 0.5);
        assert_eq!(compare(&flat.ink, &flat), None);
    }

    #[test]
    fn cells_are_averaged_into_pixels() {
        // Pixels of 0.5 mm over cells of 0.25 mm: each pixel holds four cells. The left column of
        // cells is inked, so the left half of every pixel in the first column is.
        let t = map(4, 2, |_, _| 0.0);
        let mut inked = vec![false; 8 * 4];
        for row in 0..4 {
            inked[row * 8] = true;
        }
        let c = pixel_coverage(&inked, 8, 4, [0.0, 0.0], 0.25, &t);
        assert_eq!(c, [0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn a_pixel_smaller_than_a_cell_takes_the_cell_under_it() {
        // Cells of 1 mm over pixels of 0.5 mm: the cell centres at 0.5 and 1.5 mm fall in pixels
        // 1 and 3, and pixels 0 and 2 take the cells under their centres, 0.25 and 1.25 mm.
        let t = map(4, 1, |_, _| 0.0);
        let c = pixel_coverage(&[true, false], 2, 1, [0.0, 0.0], 1.0, &t);
        assert_eq!(c, [1.0, 1.0, 0.0, 0.0]);
    }
}
