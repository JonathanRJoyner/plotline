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

use serde::Serialize;

use crate::math;
use crate::types::ToneMap;

/// The eye filter's sigma in mm: line texture at the default spacings of 1 to 2 mm fades under
/// it, as when a plot is seen from a metre or two. A taste default, recorded in the brief.
pub const EYE_SIGMA_MM: f64 = 1.5;
/// Below this gain the drawing's ink does not follow its tone, and there is nothing to compare.
const MIN_GAIN: f64 = 0.01;
/// SSIM's constants for values in 0..1: `(0.01)^2` and `(0.03)^2` (Wang et al., 2004).
const C1: f64 = 1e-4;
const C2: f64 = 9e-4;

/// How a drawing's tones follow its tone map.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ToneMatch {
    /// The RMS difference, in tone-map ink, between the tone map and the drawing's coverage put on
    /// its scale, both seen through the eye filter; 0 is a perfect match.
    pub error: f64,
    /// The mean structural similarity of the same two maps; 1 is identical.
    pub ssim: f64,
    /// The coverage the fitted line puts at white, and at full ink.
    pub floor: f64,
    pub reach: f64,
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
/// see [`ToneMatch`]. `None` when the tone map is flat or the drawing's ink does not rise with it.
pub fn compare(coverage: &[f32], t: &ToneMap) -> Option<ToneMatch> {
    let f = Fitted::new(coverage, t)?;
    let (w, h) = (t.width as usize, t.height as usize);
    Some(ToneMatch {
        error: f.error,
        ssim: ssim(&f.x, &f.y, w, h, f.sigma),
        floor: f.floor,
        reach: f.floor + f.gain,
    })
}

/// Both maps through the eye filter, the coverage put on the tone map's scale by the fitted line,
/// and the RMS difference.
struct Fitted {
    x: Vec<f32>,
    y: Vec<f32>,
    sigma: f64,
    floor: f64,
    gain: f64,
    error: f64,
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
        if var <= 0.0 || cov / var < MIN_GAIN {
            return None;
        }
        let gain = cov / var;
        let floor = mc - gain * my;
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
            error: (sq / n).sqrt(),
        })
    }
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
    fn a_bent_response_scores_worse_than_a_straight_one() {
        let t = scene();
        let bent: Vec<f32> = t.ink.iter().map(|v| v * v * v).collect();
        let m = compare(&bent, &t).unwrap();
        assert!(m.error > 0.03 && m.ssim < 0.999, "{m:?}");
    }

    #[test]
    fn a_blank_or_flat_drawing_has_no_match() {
        let t = scene();
        assert_eq!(compare(&vec![0.0; t.ink.len()], &t), None);
        assert_eq!(compare(&vec![0.4; t.ink.len()], &t), None);
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
