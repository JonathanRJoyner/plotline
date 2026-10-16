//! Synthetic test images, generated in code so the repo holds no third-party pictures.
//!
//! Each function returns an RGB [`Image`] with values in 0..1. The grey images have three equal
//! channels and follow the Python engine's test images, except [`noise`], which is only
//! deterministic, [`still_life`], the GUI's sample, and [`strokes`], [`lit_strokes`], [`ramp`] and
//! [`flat_noise`], made for the image analysis. [`color_bars`] and [`scene`] are in colour, for the
//! colour-to-tone path.

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

/// A 3-vector: a point or a direction in the still life's world, y up.
type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `o + d * t`: the point `t` along the ray from `o` in direction `d`.
fn at(o: V3, d: V3, t: f64) -> V3 {
    [o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t]
}

fn unit(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// A solid standing on the still life's floor.
enum Solid {
    /// Centre, radius and albedo.
    Sphere(V3, f64, f64),
    /// Centre, half the side, the cosine and sine of its turn about the vertical, and albedo.
    Cube(V3, f64, [f64; 2], f64),
}

impl Solid {
    /// The nearest hit in front of `o` along the unit direction `d`, as distance, unit normal and
    /// albedo. A ray is assumed to start outside the solid.
    fn hit(&self, o: V3, d: V3) -> Option<(f64, V3, f64)> {
        match *self {
            Solid::Sphere(c, r, albedo) => {
                let oc = sub(o, c);
                let b = dot(oc, d);
                let disc = b * b - (dot(oc, oc) - r * r);
                let t = -b - disc.sqrt();
                (disc >= 0.0 && t > 1e-9).then(|| (t, unit(sub(at(o, d, t), c)), albedo))
            }
            Solid::Cube(c, half, [cos, sin], albedo) => {
                // Into the cube's frame by turning back about the vertical, then the slabs.
                let local = |v: V3| [cos * v[0] + sin * v[2], v[1], cos * v[2] - sin * v[0]];
                let (lo, ld) = (local(sub(o, c)), local(d));
                let (mut near, mut far, mut axis) = (f64::NEG_INFINITY, f64::INFINITY, 0);
                for i in 0..3 {
                    let (t1, t2) = ((-half - lo[i]) / ld[i], (half - lo[i]) / ld[i]);
                    if t1.min(t2) > near {
                        (near, axis) = (t1.min(t2), i);
                    }
                    far = far.min(t1.max(t2));
                }
                let mut n = [0.0; 3];
                n[axis] = -ld[axis].signum();
                let world = [cos * n[0] - sin * n[2], n[1], sin * n[0] + cos * n[2]];
                (near <= far && near > 1e-9).then_some((near, world, albedo))
            }
        }
    }
}

/// The still life's solids: a dark ball in front, a white ball, and a grey cube with a small
/// ball on top at the back.
const SOLIDS: [Solid; 4] = [
    Solid::Sphere([0.5, 0.5, -1.3], 0.5, 0.1),
    Solid::Sphere([-0.6, 1.0, 0.5], 1.0, 0.85),
    Solid::Cube([1.0, 0.75, 2.2], 0.75, [0.8, 0.6], 0.45),
    Solid::Sphere([1.0, 2.05, 2.2], 0.55, 0.85),
];

/// The nearest hit of the solids and the floor (the plane y = 0, albedo 0.9) in front of `o`.
fn scene_hit(o: V3, d: V3) -> Option<(f64, V3, f64)> {
    let floor = (d[1] < 0.0).then(|| (-o[1] / d[1], [0.0, 1.0, 0.0], 0.9));
    SOLIDS
        .iter()
        .filter_map(|s| s.hit(o, d))
        .chain(floor)
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// A still life, the GUI's sample image: three balls and a cube on a floor before a studio
/// backdrop, lit from the upper left. It has the range a style must show, from a white
/// highlight to a near-black ball, with smooth shading, soft and hard edges, and a horizon.
///
/// Each pixel averages four rays from a pinhole camera on a 2 x 2 grid. A ray takes the nearest
/// of the ray-sphere, ray-cube and ray-floor hits; the cube is intersected with the slab method
/// of T. L. Kay and J. T. Kajiya, "Ray tracing complex scenes", *Computer Graphics* 20(4)
/// (SIGGRAPH '86), 1986, pp. 269-278. Surfaces are Lambertian under a key light and a
/// hemisphere fill, with highlights from J. F. Blinn, "Models of light reflection for computer
/// synthesized pictures", *Computer Graphics* 11(2) (SIGGRAPH '77), 1977, pp. 192-198. The key
/// light is a disc, sampled as in R. L. Cook, T. Porter and L. Carpenter, "Distributed ray
/// tracing", *Computer Graphics* 18(3) (SIGGRAPH '84), 1984, pp. 137-145, at 32 fixed points
/// laid out on the spiral of H. Vogel, "A better way to construct the sunflower head",
/// *Mathematical Biosciences* 44, 1979, pp. 179-189, turned a little for each of a pixel's
/// rays, so the shadows are soft and the image has no noise. Linear light is shown with a
/// gamma of 1 / 2.2.
pub fn still_life(w: u32, h: u32) -> Image {
    const KEY: f64 = 0.9;
    const FILL: f64 = 0.18;
    const LIGHTS: usize = 32;
    let eye = [0.3, 3.4, -6.4];
    let forward = unit(sub([0.2, 0.95, 0.6], eye));
    let right = unit(cross([0.0, 1.0, 0.0], forward));
    let up = cross(forward, right);
    let key = unit([-0.6, 0.75, -0.45]);
    let ku = unit(cross(key, [0.0, 1.0, 0.0]));
    let kv = cross(key, ku);
    // For each of a pixel's four rays, the key light's disc as 32 directions, outermost last.
    let lights: Vec<Vec<V3>> = (0..4)
        .map(|ray| {
            (0..LIGHTS)
                .map(|i| {
                    let i = i as f64;
                    let r = 0.12 * ((i + 0.5) / LIGHTS as f64).sqrt();
                    // The golden angle, pi (3 - sqrt 5), between neighbouring points.
                    let a = 2.399_963 * i + 0.5 * f64::from(ray);
                    let (cu, cv) = (r * math::cos(a), r * math::sin(a));
                    unit([0, 1, 2].map(|k| key[k] + ku[k] * cu + kv[k] * cv))
                })
                .collect()
        })
        .collect();
    let backdrop = |d: V3| 0.85 - 0.3 * d[1].max(0.0);
    let half = 0.4;
    let (wf, hf) = (f64::from(w), f64::from(h));
    let shade = |px: f64, py: f64, ray: usize| {
        let (u, v) = (
            (2.0 * px / wf - 1.0) * half * wf / hf,
            (1.0 - 2.0 * py / hf) * half,
        );
        let d = unit([0, 1, 2].map(|k| forward[k] + right[k] * u + up[k] * v));
        let Some((t, n, albedo)) = scene_hit(eye, d) else {
            return backdrop(d);
        };
        let p = at(at(eye, d, t), n, 1e-6);
        let lit = |l: &&V3| SOLIDS.iter().all(|s| s.hit(p, **l).is_none());
        // A face turned from the key gets no direct light, so it needs no shadow rays.
        // shortcut: when the four outermost directions agree the rest are skipped, which could
        // miss an occluder smaller than the light's disc; none here is.
        let visible = if dot(n, key) <= 0.0 {
            0.0
        } else {
            match lights[ray][LIGHTS - 4..].iter().filter(lit).count() {
                0 => 0.0,
                4 => 1.0,
                _ => lights[ray].iter().filter(lit).count() as f64 / LIGHTS as f64,
            }
        };
        let diffuse = dot(n, key).max(0.0) * visible;
        // Blinn's exponent 32 by squaring five times; the source scan forbids the power methods.
        let mut spec = dot(n, unit(sub(key, d))).max(0.0);
        for _ in 0..5 {
            spec *= spec;
        }
        let surface = albedo * (KEY * diffuse + FILL * (0.5 + 0.5 * n[1])) + 0.4 * spec * visible;
        // The floor fades into the backdrop with distance, so the horizon is soft.
        let fade = math::exp(-(t - 7.0).max(0.0) / 5.0);
        surface * fade + backdrop(d) * (1.0 - fade)
    };
    let value = |(x, y): (u32, u32)| {
        let (x, y) = (f64::from(x), f64::from(y));
        let sum: f64 = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
            .iter()
            .enumerate()
            .map(|(i, (sx, sy))| shade(x + sx, y + sy, i))
            .sum();
        math::powf32((sum / 4.0).clamp(0.0, 1.0) as f32, 1.0 / 2.2)
    };
    grey(w, h, pixels(w, h).map(value))
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

/// Line art: `ink` in 2-pixel lines every 16 pixels in both directions, `paper` between them.
pub fn strokes(w: u32, h: u32, ink: f32, paper: f32) -> Image {
    let value = |(x, y): (u32, u32)| if x % 16 < 2 || y % 16 < 2 { ink } else { paper };
    grey(w, h, pixels(w, h).map(value))
}

/// Line art as a scan lit from one side: [`strokes`] with ink 0.2 on paper 1, times a light that
/// rises in equal steps from `dim` at the left edge to 1 at the right.
pub fn lit_strokes(w: u32, h: u32, dim: f32) -> Image {
    let span = f64::from(w.saturating_sub(1).max(1));
    let value = |(x, y): (u32, u32)| {
        let light = f64::from(dim) + (1.0 - f64::from(dim)) * f64::from(x) / span;
        let tone = if x % 16 < 2 || y % 16 < 2 { 0.2 } else { 1.0 };
        (tone * light) as f32
    };
    grey(w, h, pixels(w, h).map(value))
}

/// Luminance from `from` at the left edge to `to` at the right, in equal steps; an image one
/// pixel wide is `from`.
pub fn ramp(w: u32, h: u32, from: f32, to: f32) -> Image {
    let span = f64::from(w.saturating_sub(1).max(1));
    let (from, to) = (f64::from(from), f64::from(to));
    let value = |(x, _): (u32, u32)| (from + (to - from) * f64::from(x) / span) as f32;
    grey(w, h, pixels(w, h).map(value))
}

/// `value` plus uniform noise of standard deviation `sigma` (half-width `sigma * sqrt(3)`), equal
/// in all channels, clamped to 0..1. Deterministic from a [`Pcg64`] seeded with `seed`.
pub fn flat_noise(w: u32, h: u32, value: f32, sigma: f32, seed: u64) -> Image {
    let mut rng = Pcg64::seed_from_u64(seed);
    let half = f64::from(sigma) * 3f64.sqrt();
    let value = f64::from(value);
    let v = |_| (value + half * (2.0 * rng.random::<f64>() - 1.0)).clamp(0.0, 1.0) as f32;
    grey(w, h, pixels(w, h).map(v))
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
