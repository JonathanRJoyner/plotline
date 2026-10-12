//! Synthetic test images, generated in code so the repo holds no third-party pictures.
//!
//! Each function returns an RGB [`Image`] with three equal channels and values in 0..1. The
//! values follow the Python engine's test images, except [`noise`], which is only deterministic.

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
