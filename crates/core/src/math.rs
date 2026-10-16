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
//! accuracy of floating point summation", *SIAM J. Sci. Comput.* 14(4), 1993, pp. 783-799. A wide
//! blur cascades box filters as in W. M. Wells, "Efficient synthesis of Gaussian filters by
//! cascaded uniform filters", *IEEE Trans. PAMI* 8(2), 1986, pp. 234-239, with the box widths of
//! P. Kovesi, "Fast almost-Gaussian filtering", *Proc. DICTA*, 2010, pp. 121-125.

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

/// From this sigma in pixels up, [`blur`] runs as three box filters, whose cost does not grow
/// with sigma; below it the true Gaussian is cheap and the boxes would be too coarse.
const BOX_SIGMA: f64 = 4.0;

/// Blur a row-major `width x height` grid with a Gaussian of `sigma` pixels, rows first, then
/// columns. Samples past an edge take the edge value. A wide blur is three box filters in each
/// direction, within a few percent of the Gaussian.
pub fn blur(values: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    if sigma >= BOX_SIGMA {
        return box_blur(values, width, height, sigma);
    }
    gaussian_blur(values, width, height, sigma)
}

/// The odd widths of three box filters whose cascade has a variance close to `sigma`^2: the
/// two odd widths around the ideal one, mixed so the variance comes closest, after Kovesi (2010).
fn box_widths(sigma: f64) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut lo = ideal.floor() as usize;
    if lo.is_multiple_of(2) {
        lo -= 1;
    }
    let l = lo as f64;
    // How many of the three take the lower width.
    let m = ((12.0 * sigma * sigma - 3.0 * l * l - 12.0 * l - 9.0) / (-4.0 * l - 4.0))
        .round()
        .clamp(0.0, 3.0) as usize;
    [0, 1, 2].map(|i| if i < m { lo } else { lo + 2 })
}

/// Three box filters along the rows, then three along the columns, each a running sum, so every
/// pixel costs the same whatever the width (Wells, 1986).
fn box_blur(values: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
    let mut v = values.to_vec();
    for w in box_widths(sigma) {
        v = box_lines(&v, height, width, |y, x| y * width + x, w);
    }
    for w in box_widths(sigma) {
        v = box_lines(&v, width, height, |x, y| y * width + x, w);
    }
    v
}

/// The mean of a row-major `width x height` grid over the square of `2r + 1` samples around each
/// sample, along the rows and then the columns, each a running sum (Wells, 1986), so every pixel
/// costs the same whatever `r`. Samples past an edge take the edge value.
pub fn box_mean(values: &[f32], width: usize, height: usize, r: usize) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }
    let w = 2 * r + 1;
    let v = box_lines(values, height, width, |y, x| y * width + x, w);
    box_lines(&v, width, height, |x, y| y * width + x, w)
}

/// The mean over a window of odd width `w` along each of `count` lines of `len` samples, where
/// `at(line, j)` indexes sample `j` of a line. Samples past an edge take the edge value.
fn box_lines(
    src: &[f32],
    count: usize,
    len: usize,
    at: impl Fn(usize, usize) -> usize,
    w: usize,
) -> Vec<f32> {
    let r = (w / 2) as isize;
    let last = len as isize - 1;
    let mut out = vec![0.0f32; src.len()];
    for line in 0..count {
        let get = |j: isize| f64::from(src[at(line, j.clamp(0, last) as usize)]);
        let mut sum: f64 = (-r..=r).map(get).sum();
        for j in 0..len {
            out[at(line, j)] = (sum / w as f64) as f32;
            sum += get(j as isize + r + 1) - get(j as isize - r);
        }
    }
    out
}

/// The separable Gaussian of [`gaussian_kernel`], applied tap by tap.
fn gaussian_blur(values: &[f32], width: usize, height: usize, sigma: f64) -> Vec<f32> {
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
/// its value at source index `c = (j + 0.5) * s / t - 0.5` on each axis, so pixel centres line
/// up. Everything is f64 until the final cast to f32: the lower weight is `1 - (c - floor(c))` and
/// the upper is `1 - lower`, both from the unclamped `c`; the two taps `floor(c)` and
/// `floor(c) + 1` are clamped into the source, so past an edge both read the edge sample; the value
/// is the sum over the four taps of sample times row weight times column weight. This matches
/// the Python engine's resize bit for bit.
pub fn resample(src: &[f32], sw: usize, sh: usize, tw: usize, th: usize) -> Vec<f32> {
    // Per target index: the two taps and their weights.
    let taps = |s: usize, t: usize| -> Vec<(usize, usize, f64, f64)> {
        let last = (s - 1) as f64;
        (0..t)
            .map(|j| {
                let c = (j as f64 + 0.5) * s as f64 / t as f64 - 0.5;
                let f = c.floor();
                let w0 = 1.0 - (c - f);
                let (i0, i1) = (f.clamp(0.0, last), (f + 1.0).clamp(0.0, last));
                (i0 as usize, i1 as usize, w0, 1.0 - w0)
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
/// Everything is f32, summed in the order numpy's float32 mean over the block axes uses, so the
/// result matches the Python engine bit for bit when it passes a contiguous array. A block is
/// split into groups of whole rows; each group's values, in row-major order, are summed with
/// [`pairwise_sum`], the group sums are added in order from 0, and the total is divided by the
/// block's pixel count. The group is:
/// - one row when there is more than one block across;
/// - the whole block when there is one block across and it spans the source rows, since numpy
///   then sums the block as one run;
/// - `8192 / bx` rows when there is one block across but the source is wider (the ragged column
///   is dropped), since numpy then reads the block through its 8192-value buffer in whole rows.
pub fn block_average(src: &[f32], w: usize, h: usize, k: usize) -> (Vec<f32>, usize, usize) {
    let (by, bx) = (k.min(h), k.min(w));
    let (nh, nw) = (h / by, w / bx);
    let group = if nw > 1 {
        1
    } else if bx == w {
        by
    } else {
        (8192 / bx).max(1)
    };
    let count = (by * bx) as f32;
    let mut out = Vec::with_capacity(nh * nw);
    let mut run = Vec::with_capacity(group * bx);
    for row in 0..nh {
        for col in 0..nw {
            let rows = row * by..(row + 1) * by;
            let mut sum = 0.0f32;
            for start in rows.clone().step_by(group) {
                run.clear();
                for y in start..(start + group).min(rows.end) {
                    run.extend_from_slice(&src[y * w + col * bx..y * w + (col + 1) * bx]);
                }
                sum += pairwise_sum(&run);
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
        let calls: Vec<String> = methods
            .iter()
            .flat_map(|name| [".", "f64::", "f32::"].map(|prefix| format!("{prefix}{name}(")))
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
    fn resample_takes_weights_from_the_unclamped_coordinate() {
        // 2 x 1 -> 4 x 10, upsampling; bits from the Python engine's resize. Every target row reads
        // the one source row, but the row weights come from the unclamped row coordinate, so rows 3
        // and 6 land 1 ulp lower in column 2; clamping the coordinate first gives 0x42c4edb8 there.
        let src = [0.0, 919.0 / 7.0];
        let mut want = [0x0_u32, 0x42034925, 0x42c4edb8, 0x43034925].repeat(10);
        want[3 * 4 + 2] = 0x42c4edb7;
        want[6 * 4 + 2] = 0x42c4edb7;
        let got: Vec<u32> = resample(&src, 2, 1, 4, 10)
            .iter()
            .map(|v| v.to_bits())
            .collect();
        assert_eq!(got, want);
    }
    #[test]
    fn block_average_sums_a_3x40_strip_with_k_20_as_one_run_per_block() {
        // One block across spanning the rows: numpy sums each 20 x 3 block in one pairwise pass.
        // Bits from the Python engine; a row-by-row sum gives 0x428d1861 for the first block.
        let src: Vec<f32> = (0..120).map(|i| ((i * 7919) % 1000) as f32 / 7.0).collect();
        let (out, w, h) = block_average(&src, 3, 40, 20);
        assert_eq!((w, h), (1, 2));
        assert_eq!(
            out.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            [0x428d1862, 0x428f0000]
        );
    }
    #[test]
    fn block_average_reads_a_150x100_ragged_column_with_k_100_in_buffered_rows() {
        // One 100 x 100 block across a 150-wide source: numpy reads it in runs of 81 whole rows.
        // Bits from the Python engine; one pass over the whole block gives 0x428eb6dc.
        let src: Vec<f32> = (0..15000)
            .map(|i| ((i * 7919) % 1000) as f32 / 7.0)
            .collect();
        let (out, w, h) = block_average(&src, 150, 100, 100);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out[0].to_bits(), 0x428eb6db);
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
    fn box_widths_are_odd_and_reach_the_asked_sigma() {
        for sigma in [BOX_SIGMA, 7.3, 25.0, 144.0] {
            let w = box_widths(sigma);
            assert!(w.iter().all(|w| w % 2 == 1), "{sigma}: {w:?}");
            // A box of width w has variance (w^2 - 1) / 12, and variances add under convolution.
            let var: f64 = w.iter().map(|&w| ((w * w - 1) as f64) / 12.0).sum();
            assert!((var.sqrt() / sigma - 1.0).abs() < 0.05, "{sigma}: {w:?}");
        }
    }
    #[test]
    fn a_wide_blur_runs_as_boxes_and_stays_close_to_the_gaussian() {
        // A step edge across a 400 x 3 grid.
        let (w, h) = (400, 3);
        let v: Vec<f32> = (0..w * h)
            .map(|i| if i % w < 200 { 0.0 } else { 1.0 })
            .collect();
        for sigma in [BOX_SIGMA, 20.0] {
            let fast = blur(&v, w, h, sigma);
            let exact = gaussian_blur(&v, w, h, sigma);
            let worst = fast
                .iter()
                .zip(&exact)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            assert!(worst < 0.02, "{sigma}: {worst}");
            assert!(fast[..w].iter().all(|x| (0.0..=1.0).contains(x)));
        }
    }
    #[test]
    fn box_mean_matches_a_direct_mean_with_edge_clamping() {
        let (w, h) = (7usize, 5usize);
        let v: Vec<f32> = (0..w * h)
            .map(|i| ((i * 7919) % 1000) as f32 / 1000.0)
            .collect();
        for r in [1usize, 2, 9] {
            let got = box_mean(&v, w, h, r);
            for y in 0..h {
                for x in 0..w {
                    let mut sum = 0.0f64;
                    for dy in -(r as isize)..=r as isize {
                        for dx in -(r as isize)..=r as isize {
                            let sy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
                            let sx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
                            sum += f64::from(v[sy * w + sx]);
                        }
                    }
                    let want = (sum / ((2 * r + 1) * (2 * r + 1)) as f64) as f32;
                    assert!((got[y * w + x] - want).abs() < 1e-6, "r {r} at ({x}, {y})");
                }
            }
        }
        assert_eq!(box_mean(&vec![0.37; w * h], w, h, 3), vec![0.37; w * h]);
        assert!(box_mean(&[], 0, 0, 2).is_empty());
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
