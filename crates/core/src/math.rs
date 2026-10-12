//! Deterministic math: the only place the engine calls transcendental functions.
//!
//! Output must be bit-identical on native targets and WebAssembly, and the platform's own `sin`,
//! `exp` and friends may differ between them in the last bit. Every such call goes through the
//! `libm` crate, which computes the same bits everywhere, and the rest of the crate calls these
//! wrappers; a test scans the sources to keep it that way. The image helpers below are the tone
//! stage's resampling and blur.
//!
//! Gaussian smoothing and bilinear interpolation follow R. C. Gonzalez and R. E. Woods, *Digital
//! Image Processing*, 3rd ed., Pearson Prentice Hall, 2008, chapter 2 (image interpolation) and
//! chapter 3 (smoothing spatial filters). Pairwise summation is analysed in N. J. Higham, "The
//! accuracy of floating point summation", *SIAM J. Sci. Comput.* 14(4), 1993, pp. 783-799.

/// `e` raised to `x`.
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

/// `x` raised to `y`, in f32.
pub fn powf32(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

/// The sine of `x` radians.
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

/// The cosine of `x` radians.
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// The angle of the point `(x, y)` from the positive x axis, in radians in `[-pi, pi]`.
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// The natural logarithm of `x`.
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

/// The length of the vector `(x, y)`, without overflow or underflow in between.
pub fn hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}

/// A normalised 1-D Gaussian with standard deviation `sigma` samples and radius `ceil(3 sigma)`,
/// so it has `2 ceil(3 sigma) + 1` taps summing to 1. A `sigma` that is not positive gives `[1]`.
pub fn gaussian_kernel(sigma: f64) -> Vec<f32> {
    if sigma.is_nan() || sigma <= 0.0 {
        return vec![1.0];
    }
    let r = (3.0 * sigma).ceil() as i64;
    let w: Vec<f64> = (-r..=r)
        .map(|x| exp(-((x * x) as f64) / (2.0 * sigma * sigma)))
        .collect();
    let total: f64 = w.iter().sum();
    w.iter().map(|v| (v / total) as f32).collect()
}

/// Blur a row-major `width x height` grid with a separable Gaussian of `sigma` pixels, rows
/// first, then columns. Samples past an edge take the edge value.
pub fn blur(values: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    let kernel = gaussian_kernel(sigma);
    let r = kernel.len() / 2;
    let pass = |src: &[f32], along_x: bool| -> Vec<f32> {
        let mut out = vec![0.0f32; src.len()];
        for y in 0..height {
            for x in 0..width {
                let mut acc = 0.0f32;
                for (i, &k) in kernel.iter().enumerate() {
                    // Tap i reads offset i - r, clamped into the grid.
                    let (sx, sy) = if along_x {
                        ((x + i).saturating_sub(r).min(width - 1), y)
                    } else {
                        (x, (y + i).saturating_sub(r).min(height - 1))
                    };
                    acc += k * src[sy * width + sx];
                }
                out[y * width + x] = acc;
            }
        }
        out
    };
    pass(&pass(values, true), false)
}

/// Resample a non-empty row-major `sw x sh` grid to `tw x th` bilinearly. Target pixel `j` takes
/// its value at source index `(j + 0.5) * s / t - 0.5` on each axis, so pixel centres line up;
/// that index is computed in f64 and clamped into the source. The interpolation is in f64, as the
/// sum over the four neighbours of value times row weight times column weight, where the lower
/// weight is `1 - frac` and the upper is `1 - lower`; the result is cast to f32.
pub fn resample(src: &[f32], sw: usize, sh: usize, tw: usize, th: usize) -> Vec<f32> {
    // Per target index: the source index below, the one above, and their weights.
    let taps = |s: usize, t: usize| -> Vec<(usize, usize, f64, f64)> {
        (0..t)
            .map(|j| {
                let c = ((j as f64 + 0.5) * s as f64 / t as f64 - 0.5).clamp(0.0, (s - 1) as f64);
                let i0 = c.floor() as usize;
                let w0 = 1.0 - (c - i0 as f64);
                (i0, (i0 + 1).min(s - 1), w0, 1.0 - w0)
            })
            .collect()
    };
    let (cols, rows) = (taps(sw, tw), taps(sh, th));
    let at = |x: usize, y: usize| f64::from(src[y * sw + x]);
    let mut out = Vec::with_capacity(tw * th);
    for &(y0, y1, a0, a1) in &rows {
        for &(x0, x1, b0, b1) in &cols {
            let v = at(x0, y0) * a0 * b0
                + at(x1, y0) * a0 * b1
                + at(x0, y1) * a1 * b0
                + at(x1, y1) * a1 * b1;
            out.push(v as f32);
        }
    }
    out
}

/// Average `k x k` blocks of a row-major `w x h` grid, for `k` at least 1. A side shorter than `k`
/// is one block, and pixels past the last whole block are dropped. Returns the averages and their
/// width and height.
///
/// Everything is f32, summed in the order the Python engine's float32 block mean uses, so the two
/// engines agree bit for bit: each row of a block is summed with [`pairwise_sum`], the row sums are
/// added in order starting from 0, and the total is divided by the block's pixel count.
pub fn block_average(src: &[f32], w: usize, h: usize, k: usize) -> (Vec<f32>, usize, usize) {
    let (by, bx) = (k.min(h), k.min(w));
    let (nh, nw) = (h / by, w / bx);
    let count = (by * bx) as f32;
    let mut out = Vec::with_capacity(nh * nw);
    for row in 0..nh {
        for col in 0..nw {
            let mut sum = 0.0f32;
            for y in row * by..(row + 1) * by {
                sum += pairwise_sum(&src[y * w + col * bx..y * w + (col + 1) * bx]);
            }
            out.push(sum / count);
        }
    }
    (out, nw, nh)
}

/// Sum `a` in f32 in numpy's pairwise order (Higham 1993 describes pairwise summation). Fewer than
/// 8 values add in sequence from 0. Up to 128 values use 8 accumulators: accumulator `j` starts at
/// `a[j]` and adds every eighth value after it over the largest multiple of 8, the accumulators
/// combine as `((r0 + r1) + (r2 + r3)) + ((r4 + r5) + (r6 + r7))`, then the leftover values add in
/// sequence. More than 128 values split at half the length rounded down to a multiple of 8, and
/// the two halves' sums add.
fn pairwise_sum(a: &[f32]) -> f32 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0f32;
        for v in a {
            res += v;
        }
        res
    } else if n <= 128 {
        let mut r = [0.0f32; 8];
        r.copy_from_slice(&a[..8]);
        let (chunks, rest) = a[8..].as_chunks::<8>();
        for chunk in chunks {
            for (acc, v) in r.iter_mut().zip(chunk) {
                *acc += v;
            }
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        for v in rest {
            res += v;
        }
        res
    } else {
        let half = n / 2 - (n / 2) % 8;
        pairwise_sum(&a[..half]) + pairwise_sum(&a[half..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `.rs` file under `dir` except `src/math.rs`, with the platform float methods it calls.
    fn scan(dir: &std::path::Path) -> Vec<String> {
        let skip = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/math.rs");
        // Built at runtime so the patterns never appear as literals in any scanned file.
        let methods = [
            "sin", "cos", "tan", "atan2", "exp", "ln", "log", "powf", "mul_add", "sin_cos", "atan",
            "asin", "acos", "tanh", "exp2", "exp_m1", "ln_1p", "log2", "log10", "hypot", "cbrt",
            "powi",
        ];
        let qualified = [
            "sin", "exp", "ln", "log", "powf", "hypot", "atan2", "cos", "tan",
        ];
        let calls: Vec<String> = methods
            .iter()
            .map(|name| format!(".{name}("))
            .chain(
                qualified
                    .iter()
                    .flat_map(|name| [format!("f64::{name}("), format!("f32::{name}(")]),
            )
            .collect();
        let mut bad = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                bad.extend(scan(&path));
            } else if path.extension().is_some_and(|e| e == "rs") && path != skip {
                let text = std::fs::read_to_string(&path).unwrap();
                for call in &calls {
                    if text.contains(call.as_str()) {
                        bad.push(format!("{} calls {call}", path.display()));
                    }
                }
            }
        }
        bad
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn resample_keeps_pixel_centres_aligned() {
        // 96 -> 64: target j samples source 1.5 j + 0.25; source is a ramp s / 95.
        let src: Vec<f32> = (0..96).map(|s| s as f32 / 95.0).collect();
        let out = resample(&src, 96, 1, 64, 1);
        for j in 0..64 {
            assert!((out[j] - (1.5 * j as f32 + 0.25) / 95.0).abs() < 1e-5);
        }
    }
    #[test]
    fn block_average_drops_ragged_edge() {
        let (out, w, h) = block_average(&[1.0, 3.0, 9.0, 5.0, 7.0, 9.0], 3, 2, 2);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![4.0]);
    }
    #[test]
    fn resample_matches_the_python_engine_bits() {
        // 5 x 3 -> 7 x 4; bits from the Python engine's resize (float32 in, float32 out).
        let src: Vec<f32> = (0..15).map(|i| ((i * 7919) % 1000) as f32 / 7.0).collect();
        let want = [
            0x0_u32, 0x42960a73, 0x42fff58e, 0x42ef6db7, 0x42dee5e1, 0x42ce5e0b, 0x42c12492,
            0x42548000, 0x429a4000, 0x42b7a344, 0x42a71b6e, 0x42969398, 0x42860bc1, 0x4271a492,
            0x427d36db, 0x4262c3eb, 0x4241b43f, 0x4220a492, 0x428c5249, 0x429a672f, 0x428d2db7,
            0x41d92492, 0x41a43eb1, 0x41443eb1, 0x40800000, 0x42c38d10, 0x43025398, 0x42f76db7,
        ];
        let got: Vec<u32> = resample(&src, 5, 3, 7, 4)
            .iter()
            .map(|v| v.to_bits())
            .collect();
        assert_eq!(got, want);
    }
    #[test]
    fn block_average_covers_several_blocks() {
        #[rustfmt::skip]
        let src = [
            1.0, 2.0, 3.0, 4.0, 100.0,
            5.0, 6.0, 7.0, 8.0, 100.0,
            9.0, 10.0, 11.0, 12.0, 100.0,
            13.0, 14.0, 15.0, 16.0, 100.0,
        ];
        assert_eq!(
            block_average(&src, 5, 4, 2),
            (vec![3.5, 5.5, 11.5, 13.5], 2, 2)
        );
    }
    #[test]
    fn block_average_makes_a_short_side_one_block() {
        let src = [
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
        ];
        assert_eq!(block_average(&src, 6, 2, 3), (vec![5.0, 8.0], 2, 1));
    }
    #[test]
    fn block_average_matches_the_python_engine_bits() {
        // 130-wide blocks exercise every pairwise branch; a running sum gives 0x4291d20c, 0x428b7268.
        let src: Vec<f32> = (0..780).map(|i| ((i * 7919) % 1000) as f32 / 7.0).collect();
        let (out, w, h) = block_average(&src, 260, 3, 130);
        assert_eq!((w, h), (2, 1));
        assert_eq!(
            out.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            [0x4291d20d, 0x428b7266]
        );
    }
    #[test]
    fn blur_lowers_variation_and_keeps_mean() {
        let v: Vec<f32> = (0..64).map(|i| ((i / 8 + i % 8) % 2) as f32).collect();
        let b = blur(&v, 8, 8, 1.0);
        let (m0, m1) = (v.iter().sum::<f32>() / 64.0, b.iter().sum::<f32>() / 64.0);
        assert!((m0 - m1).abs() < 0.05);
        let var = |x: &[f32], m: f32| x.iter().map(|a| (a - m).powi(2)).sum::<f32>();
        assert!(var(&b, m1) < var(&v, m0));
    }
    #[test]
    fn gaussian_kernel_has_radius_three_sigma_and_sums_to_one() {
        let k = gaussian_kernel(1.2);
        assert_eq!(k.len(), 2 * 4 + 1);
        assert!((k.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(k[4] > k[3] && k[3] == k[5]);
        assert_eq!(gaussian_kernel(0.0), vec![1.0]);
    }
    #[test]
    fn source_scan_finds_no_platform_float_methods_outside_math() {
        // Walks crates/core/src, skips math.rs, fails on ".sin(", ".cos(", ".tan(", ".atan2(", ".exp(", ".ln(", ".log(", ".powf(", ".mul_add(".
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let bad = scan(std::path::Path::new(root)); // helper in the test module
        assert!(bad.is_empty(), "{bad:?}");
    }
}
