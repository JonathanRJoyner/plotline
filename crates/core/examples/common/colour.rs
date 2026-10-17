//! The colour measure the audit and the colour-lanes examples share: the drawing rendered at
//! 8 px per mm, averaged to 2 px per mm, blurred with the tone match's 1.5 mm eye filter and
//! compared with the source image in CIE Lab (CIE 15:2004, *Colorimetry*, 3rd ed.), ΔE as CIE76.

use plotline_core::preview::render_rgba;
use plotline_core::types::Drawing;
use plotline_core::Image;
use serde_json::{json, Value};

/// The colour comparison renders at this many px per mm, then averages down to `CMP_PX_MM`.
const RENDER_PX_MM: f64 = 8.0;
const CMP_PX_MM: f64 = 2.0;
/// The eye filter of the tone match, 1.5 mm.
const EYE_MM: f64 = 1.5;

/// sRGB 0..1 to CIE Lab (D65).
fn lab(rgb: [f64; 3]) -> [f64; 3] {
    let lin = |c: f64| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let [r, g, b] = rgb.map(lin);
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f64| {
        if t > 0.008856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// Separable Gaussian blur of a `w x h` plane, edges clamped.
fn blur(p: &[f64], w: usize, h: usize, sigma: f64) -> Vec<f64> {
    let r = (3.0 * sigma).ceil() as isize;
    let k: Vec<f64> = (-r..=r)
        .map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f64 = k.iter().sum();
    let k: Vec<f64> = k.iter().map(|v| v / sum).collect();
    let mut tmp = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, kv) in k.iter().enumerate() {
                let xx = (x as isize + j as isize - r).clamp(0, w as isize - 1) as usize;
                acc += kv * p[y * w + xx];
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut outp = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (j, kv) in k.iter().enumerate() {
                let yy = (y as isize + j as isize - r).clamp(0, h as isize - 1) as usize;
                acc += kv * tmp[yy * w + x];
            }
            outp[y * w + x] = acc;
        }
    }
    outp
}

/// How the drawing, seen through the eye filter, compares with the source image over the image's
/// rectangle: mean CIE76 ΔE, the same for blank paper, and the mean darkness of each.
pub fn colour_match(d: &Drawing, paper: &str, image: &Image) -> Value {
    let page_w = d.canvas.width_mm;
    let (w_px, h_px, rgba) = render_rgba(
        d,
        (page_w * RENDER_PX_MM).round() as u32,
        paper,
        false,
        false,
        None,
    );
    if rgba.is_empty() {
        return Value::Null;
    }
    let [ix, iy, iw, ih] = d.canvas.image_rect(image.width, image.height);
    let f = (RENDER_PX_MM / CMP_PX_MM) as usize;
    let (cw, ch) = (
        (iw * CMP_PX_MM).floor() as usize,
        (ih * CMP_PX_MM).floor() as usize,
    );
    let (ox, oy) = (
        (ix * RENDER_PX_MM).round() as usize,
        (iy * RENDER_PX_MM).round() as usize,
    );
    let mut draw = [vec![0.0; cw * ch], vec![0.0; cw * ch], vec![0.0; cw * ch]];
    let mut src = [vec![0.0; cw * ch], vec![0.0; cw * ch], vec![0.0; cw * ch]];
    for y in 0..ch {
        for x in 0..cw {
            let mut acc = [0.0; 3];
            for dy in 0..f {
                for dx in 0..f {
                    let (px, py) = (ox + x * f + dx, oy + y * f + dy);
                    if px < w_px as usize && py < h_px as usize {
                        let i = 4 * (py * w_px as usize + px);
                        for (a, &v) in acc.iter_mut().zip(&rgba[i..i + 3]) {
                            *a += f64::from(v) / 255.0;
                        }
                    }
                }
            }
            // Box average in encoded values; close enough for a relative comparison.
            for (plane, a) in draw.iter_mut().zip(acc) {
                plane[y * cw + x] = a / (f * f) as f64;
            }
            let sx = (((x as f64 + 0.5) / cw as f64) * f64::from(image.width)) as usize;
            let sy = (((y as f64 + 0.5) / ch as f64) * f64::from(image.height)) as usize;
            let si = 3
                * (sy.min(image.height as usize - 1) * image.width as usize
                    + sx.min(image.width as usize - 1));
            for (plane, &v) in src.iter_mut().zip(&image.rgb[si..si + 3]) {
                plane[y * cw + x] = f64::from(v);
            }
        }
    }
    let sigma = EYE_MM * CMP_PX_MM;
    let draw: Vec<Vec<f64>> = draw.iter().map(|p| blur(p, cw, ch, sigma)).collect();
    let src: Vec<Vec<f64>> = src.iter().map(|p| blur(p, cw, ch, sigma)).collect();
    let paper_rgb = {
        let h = paper.trim_start_matches('#');
        let v = |i: usize| f64::from(u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(255)) / 255.0;
        [v(0), v(2), v(4)]
    };
    let paper_lab = lab(paper_rgb);
    let n = (cw * ch) as f64;
    let (mut de, mut de_paper, mut dark_d, mut dark_s, mut chroma_d, mut chroma_s) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for i in 0..cw * ch {
        let a = lab([draw[0][i], draw[1][i], draw[2][i]]);
        let b = lab([src[0][i], src[1][i], src[2][i]]);
        let dist = |p: [f64; 3], q: [f64; 3]| {
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
        };
        de += dist(a, b);
        de_paper += dist(paper_lab, b);
        dark_d += 1.0 - a[0] / 100.0;
        dark_s += 1.0 - b[0] / 100.0;
        chroma_d += (a[1] * a[1] + a[2] * a[2]).sqrt();
        chroma_s += (b[1] * b[1] + b[2] * b[2]).sqrt();
    }
    json!({
        "de": de / n,
        "de_paper": de_paper / n,
        "dark_drawing": dark_d / n,
        "dark_source": dark_s / n,
        "chroma_drawing": chroma_d / n,
        "chroma_source": chroma_s / n,
    })
}
