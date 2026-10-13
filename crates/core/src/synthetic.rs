//! Synthetic test images, generated in code so the repo holds no third-party pictures.
//!
//! Each function returns an RGB [`Image`] with values in 0..1. The grey images have three equal
//! channels and follow the Python engine's test images, except [`noise`], which is only
//! deterministic. [`color_bars`] and [`scene`] are in colour, for the colour-to-tone path.

use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64;

use crate::math;
use crate::types::Image;

/// Repeat a row-major luminance into three equal RGB channels.
fn grey(width: u32, height: u32, luminance: impl Iterator<Item = f32>) -> Image {
    let rgb = luminance.flat_map(|v| [v; 3]).collect();
    Image { width, height, rgb }
}

/// Every pixel's `(x, y)`, in row-major order.
fn pixels(w: u32, h: u32) -> impl Iterator<Item = (u32, u32)> {
    (0..h).flat_map(move |y| (0..w).map(move |x| (x, y)))
}

/// Luminance `x / (w - 1)`: black at the left edge, white at the right. Computed as numpy's
/// `linspace(0, 1, w)` does, `x * (1 / (w - 1))` in f64 with the last column exactly 1; an image
/// one pixel wide is black.
pub fn gradient(w: u32, h: u32) -> Image {
    let step = 1.0 / f64::from(w.saturating_sub(1));
    let value = |x: u32| match x {
        0 => 0.0,
        x if x == w - 1 => 1.0,
        x => (f64::from(x) * step) as f32,
    };
    grey(w, h, pixels(w, h).map(|(x, _)| value(x)))
}

/// Square cells of side `w / cells` (rounded down) alternating 0 and 1, with 0 at the top left.
/// Panics if `cells` is greater than `w`.
pub fn checkerboard(w: u32, h: u32, cells: u32) -> Image {
    let side = w / cells;
    grey(
        w,
        h,
        pixels(w, h).map(|(x, y)| ((x / side + y / side) % 2) as f32),
    )
}

/// The distance from the centre over half the diagonal, clamped to 1: 0 in the middle, near 1 at
/// the corners.
pub fn radial(w: u32, h: u32) -> Image {
    let (wf, hf) = (f64::from(w), f64::from(h));
    let (cx, cy, half) = (
        (wf - 1.0) / 2.0,
        (hf - 1.0) / 2.0,
        math::hypot(wf, hf) / 2.0,
    );
    let value = |(x, y): (u32, u32)| {
        let d = math::hypot(f64::from(x) - cx, f64::from(y) - cy) / half;
        d.clamp(0.0, 1.0) as f32
    };
    grey(w, h, pixels(w, h).map(value))
}

/// Uniform random luminance in 0..1 from a [`Pcg64`] seeded with `seed`, equal in all channels.
pub fn noise(w: u32, h: u32, seed: u64) -> Image {
    let mut rng = Pcg64::seed_from_u64(seed);
    grey(w, h, pixels(w, h).map(|_| rng.random::<f64>() as f32))
}

/// The grey level `value` everywhere.
pub fn flat(w: u32, h: u32, value: f32) -> Image {
    let rgb = vec![value; w as usize * h as usize * 3];
    Image {
        width: w,
        height: h,
        rgb,
    }
}

/// Eight vertical bars in the corners of the RGB cube, light to dark by luminance: white, yellow,
/// cyan, green, magenta, red, blue and black. Column `x` is in bar `x * 8 / w`.
pub fn color_bars(w: u32, h: u32) -> Image {
    const BARS: [[f32; 3]; 8] = [
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
    ];
    let rgb = pixels(w, h)
        .flat_map(|(x, _)| BARS[(x * 8 / w) as usize])
        .collect();
    Image {
        width: w,
        height: h,
        rgb,
    }
}

/// A made-up landscape that stands in for a colour photo: a sky that pales towards the horizon,
/// a yellow sun, a light green hill with a red barn on it and a shaded dark green hill in front.
/// It mixes smooth gradients with hard colour edges. Only sums and products are used, so it is
/// the same on every platform.
pub fn scene(w: u32, h: u32) -> Image {
    let (wf, hf) = (f64::from(w), f64::from(h));
    let color = |(x, y): (u32, u32)| {
        // The pixel centre as a fraction of the width and height.
        let (fx, fy) = ((f64::from(x) + 0.5) / wf, (f64::from(y) + 0.5) / hf);
        let sq = |v: f64| v * v;
        let near_hill = 0.65 + 0.9 * sq(fx - 0.8);
        let far_hill = 0.5 + 0.8 * sq(fx - 0.3);
        let barn = (0.2..0.36).contains(&fx) && (0.4..0.6).contains(&fy);
        let door = (0.26..0.3).contains(&fx) && fy >= 0.5;
        // The sun is round on the page, so its distance is measured in heights.
        let sun = sq((fx - 0.75) * wf / hf) + sq(fy - 0.25) < sq(0.12);
        let rgb = if fy > near_hill {
            let shade = 1.2 - 0.5 * fy;
            [0.1 * shade, 0.45 * shade, 0.15 * shade]
        } else if barn && door {
            [0.3, 0.05, 0.05]
        } else if barn {
            [0.7, 0.12, 0.1]
        } else if fy > far_hill {
            [0.35, 0.6, 0.25]
        } else if sun {
            [1.0, 0.85, 0.2]
        } else {
            [0.15 + 0.6 * fy, 0.35 + 0.5 * fy, 0.75 + 0.2 * fy]
        };
        rgb.map(|c| c as f32)
    };
    let rgb = pixels(w, h).flat_map(color).collect();
    Image {
        width: w,
        height: h,
        rgb,
    }
}
