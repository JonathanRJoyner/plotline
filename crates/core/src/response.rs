//! Auto tone response: how much of the paper a style covers at each tone, measured on a step
//! wedge, and the lookup that corrects the tone map for it.
//!
//! A run whose tone stage has `response` auto first runs its own style over a [`Wedge`], 4 by 4
//! grey patches at the real tone map's pixel size, with the tone stage at its plain defaults and
//! the first pen alone. The share of the coverage raster's cells inked in each patch is the
//! style's response to that patch's tone. Pool-adjacent-violators makes the response
//! non-decreasing (M. Ayer, H. D. Brunk, G. M. Ewing, W. T. Reid and E. Silverman, "An empirical
//! distribution function for sampling with incomplete information", *Ann. Math. Statist.* 26(4),
//! 1955, pp. 641-647), and its inverse becomes a lookup the tone stage applies last, so that the
//! drawing's coverage rises in a straight line from white to the darkest tone the style reaches.
//! Linearising a device by inverting its measured tone curve is standard print calibration (R.
//! Bala, "Device characterization", in G. Sharma (ed.), *Digital Color Imaging Handbook*, CRC
//! Press, 2003). Aiming at the style's own darkest tone rather than full black keeps the shapes,
//! which the 2026-10-12 spike recorded in the brief ("Tone matching").
//!
//! The wedge run shares the stage cache, and its stages' keys hold the wedge, the pen and every
//! stage's settings, so a run whose style settings and pen have not changed finds it cached.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::cache::StageCache;
use crate::coverage::{self, Coverage};
use crate::fit::Density;
use crate::runner::{Run, Shared};
use crate::settings::{merge, validate};
use crate::stage::{Artifact, Kind};
use crate::stages::tone::{self, Response, ToneSettings};
use crate::style::Style;
use crate::types::{Canvas, Drawing, Fit, Image, Pen};

/// Patches along each side of the wedge.
const SIDE: usize = 4;
/// The wedge's steps, at inks `i / (STEPS - 1)`.
pub const STEPS: usize = SIDE * SIDE;
/// A patch is at least this wide in mm, at least this many of the style's spacings, and at least
/// [`PATCH_PX`] pixels, which keeps the wedge's tone map within the tone stage's least resolution.
const PATCH_MM: f64 = 12.0;
const PATCH_SPACINGS: f64 = 8.0;
const PATCH_PX: usize = 16;
/// A patch is at most this wide in mm and in pixels, so a wide spacing or a fine tone map keeps
/// the wedge's run short; such a patch holds fewer spacings and measures more roughly.
const MAX_PATCH_MM: f64 = 40.0;
const MAX_PATCH_PX: usize = 256;
/// The share of a patch's side left out on each side when it is measured, away from its edges.
const INSET: f64 = 0.1;
/// How much wider than the coverage raster's usual cells the wedge's are: `1 + (sqrt(5) - 1) / 20`.
const CELL_STRETCH: f64 = 1.061_803_398_874_989_5;
/// The lookup's samples over the tones from 0 to 1.
pub const LOOKUP: usize = 257;
/// The degree of the polynomial fitted to the wedge's response. The patches' coverages are off by
/// a few thousandths, which through straight lines between them puts bands in a smooth gradient;
/// a fifth degree smooths that and still follows the steep start of the connected renderers'.
const DEGREE: usize = 5;
/// A response that covers less than this much more paper at its darkest than at white is left
/// alone: there is nothing to invert.
const MIN_RANGE: f64 = 0.01;

/// The step in the patch at `row` and `col`: rows run left to right and right to left in turn, so
/// neighbours along a row differ by one step.
fn step_at(row: usize, col: usize) -> usize {
    row * SIDE
        + if row.is_multiple_of(2) {
            col
        } else {
            SIDE - 1 - col
        }
}

/// A step wedge: its image, a page it fills at a given pixel size, and the patches' size.
pub struct Wedge {
    /// The grey patches, white to black in [`step_at`] order.
    pub image: Arc<Image>,
    /// A square page with no margin that the image fills at `mm_per_px`.
    pub canvas: Canvas,
    patch_px: usize,
    mm_per_px: f64,
}

impl Wedge {
    /// A wedge of pixels `mm_per_px` wide whose patches are `patch_mm` wide, within the bounds
    /// above.
    pub fn new(mm_per_px: f64, patch_mm: f64) -> Wedge {
        let patch_mm = patch_mm.min(MAX_PATCH_MM);
        let patch_px = ((patch_mm / mm_per_px).ceil() as usize).clamp(PATCH_PX, MAX_PATCH_PX);
        let side = SIDE * patch_px;
        let mut rgb = Vec::with_capacity(side * side * 3);
        for y in 0..side {
            for x in 0..side {
                let step = step_at(y / patch_px, x / patch_px);
                rgb.extend([1.0 - step as f32 / (STEPS - 1) as f32; 3]);
            }
        }
        let mm = side as f64 * mm_per_px;
        Wedge {
            image: Arc::new(Image {
                width: side as u32,
                height: side as u32,
                rgb,
            }),
            canvas: Canvas::new(mm, mm, 0.0, Fit::Fit).expect("a wedge is a valid canvas"),
            patch_px,
            mm_per_px,
        }
    }

    /// `settings` for a run of `style` over the wedge: the tone stage at its plain defaults with a
    /// resolution of the wedge's width, so the wedge's tone map is the wedge itself, and the stage
    /// that splits between the pens at its defaults, one pen.
    fn settings(&self, style: &Style, settings: &Value) -> Value {
        let mut out = settings.clone();
        for stage in &style.stages {
            match stage.provides() {
                Kind::Tone => {
                    let plain = ToneSettings {
                        resolution: self.image.width,
                        ..ToneSettings::default()
                    };
                    out[stage.name()] = serde_json::to_value(plain).expect("settings serialize");
                }
                Kind::Tones => out[stage.name()] = stage.defaults(),
                _ => {}
            }
        }
        out
    }

    /// For each step, the mean ink of the wedge's tone map and the share of the coverage raster's
    /// cells the first layer inks, inside the patch inset by [`INSET`] on each side.
    pub fn measure(&self, d: &Drawing) -> Vec<(f64, f64)> {
        let tone = d.tone.as_deref().expect("the wedge run keeps its tone map");
        let mm = self.canvas.width_mm;
        let rect = [0.0, 0.0, mm, mm];
        let pen = d.layers.first().map_or(0.3, |l| l.pen.width_mm);
        // The wedge's marks are as regular as the style makes them, and a spacing in whole tenths
        // of a mm laid on cells of a tenth would cover the same cells in every patch, so the
        // cells are widened by a factor no short ratio of whole numbers comes near.
        let mut grid = Coverage::new(rect, coverage::cell_size(pen, rect) * CELL_STRETCH);
        if let Some(layer) = d.layers.first() {
            grid.stamp(&layer.paths, pen);
        }
        let patch = self.patch_px as f64 * self.mm_per_px;
        // The cells or pixels of `size` mm from `origin` whose centres lie in [a, b), of `n`.
        let span = |a: f64, b: f64, origin: f64, size: f64, n: usize| {
            let at = |v: f64| (((v - origin) / size - 0.5).ceil().max(0.0) as usize).min(n);
            at(a)..at(b)
        };
        let mean = |values: &[f64]| values.iter().sum::<f64>() / values.len().max(1) as f64;
        // A weight that falls smoothly to 0 at both ends of a window, `(4 u (1 - u))^2` at the
        // share `u` across it, so a partial line at the window's edge barely counts.
        let weights = |r: std::ops::Range<usize>, a: f64, b: f64, size: f64| -> Vec<f64> {
            r.map(|i| {
                let u = (((i as f64 + 0.5) * size - a) / (b - a)).clamp(0.0, 1.0);
                (4.0 * u * (1.0 - u)) * (4.0 * u * (1.0 - u))
            })
            .collect()
        };
        let mut out = vec![(0.0, 0.0); STEPS];
        for row in 0..SIDE {
            for col in 0..SIDE {
                let (x0, y0) = (col as f64 * patch, row as f64 * patch);
                let (lo, hi) = (INSET * patch, (1.0 - INSET) * patch);
                let (tw, th) = (tone.width as usize, tone.height as usize);
                let px = span(x0 + lo, x0 + hi, tone.origin_mm[0], tone.mm_per_px, tw);
                let py = span(y0 + lo, y0 + hi, tone.origin_mm[1], tone.mm_per_px, th);
                let inks: Vec<f64> = py
                    .flat_map(|y| px.clone().map(move |x| (x, y)))
                    .map(|(x, y)| f64::from(tone.ink[y * tw + x]))
                    .collect();
                let cx = span(x0 + lo, x0 + hi, 0.0, grid.cell_mm, grid.width);
                let cy = span(y0 + lo, y0 + hi, 0.0, grid.cell_mm, grid.height);
                let (wx, wy) = (
                    weights(cx.clone(), x0 + lo, x0 + hi, grid.cell_mm),
                    weights(cy.clone(), y0 + lo, y0 + hi, grid.cell_mm),
                );
                let (mut sum, mut total) = (0.0, 0.0);
                for (y, b) in cy.clone().zip(&wy) {
                    for (x, a) in cx.clone().zip(&wx) {
                        let on = f64::from(u8::from(grid.count[y * grid.width + x] > 0));
                        sum += a * b * on;
                        total += a * b;
                    }
                }
                let covered = if total > 0.0 { sum / total } else { 0.0 };
                out[step_at(row, col)] = (mean(&inks), covered);
            }
        }
        out
    }
}

/// `y` made non-decreasing by pool-adjacent-violators with equal weights: each run of values that
/// falls is replaced by its mean, merging backwards until the means rise.
pub fn pava(y: &[f64]) -> Vec<f64> {
    // Each block is a sum and a count.
    let mut blocks: Vec<(f64, usize)> = Vec::with_capacity(y.len());
    for &v in y {
        blocks.push((v, 1));
        while let [.., (s0, n0), (s1, n1)] = blocks[..] {
            if s0 / n0 as f64 <= s1 / n1 as f64 {
                break;
            }
            blocks.pop();
            *blocks.last_mut().expect("two blocks were there") = (s0 + s1, n0 + n1);
        }
    }
    blocks
        .iter()
        .flat_map(|&(s, n)| std::iter::repeat_n(s / n as f64, n))
        .collect()
}

/// The lookup for `samples` of tone and coverage: the response is fitted by a least-squares
/// polynomial of degree [`DEGREE`], sampled at [`LOOKUP`] tones from 0 to 1 and made
/// non-decreasing with [`pava`], and tone `t` goes to the least tone whose coverage reaches
/// `c_min + t (c_max - c_min)`, between the samples on straight lines. `None` when the response
/// rises less than [`MIN_RANGE`].
pub fn lookup(samples: &[(f64, f64)]) -> Option<Vec<f32>> {
    let c = polyfit(samples, DEGREE)?;
    let xs: Vec<f64> = (0..LOOKUP)
        .map(|j| j as f64 / (LOOKUP - 1) as f64)
        .collect();
    let fitted: Vec<f64> = xs
        .iter()
        .map(|&x| c.iter().rev().fold(0.0, |acc, &k| acc * x + k))
        .collect();
    if !fitted.iter().all(|v| v.is_finite()) {
        return None;
    }
    let ys = pava(&fitted);
    let (lo, hi) = (*ys.first()?, *ys.last()?);
    // Written so that a fit gone to NaN counts as no response too.
    if !(hi - lo >= MIN_RANGE) {
        return None;
    }
    let inverse = |target: f64| {
        if target <= lo {
            return 0.0;
        }
        // The first sample at or above the target, which the last always is; the one before it
        // is below.
        let i = ys.iter().position(|&y| y >= target).unwrap_or(LOOKUP - 1);
        let (x0, x1, y0, y1) = (xs[i - 1], xs[i], ys[i - 1], ys[i]);
        (x0 + (target - y0) / (y1 - y0) * (x1 - x0)).clamp(0.0, 1.0)
    };
    Some(
        (0..LOOKUP)
            .map(|j| inverse((lo + j as f64 / (LOOKUP - 1) as f64 * (hi - lo)).min(hi)) as f32)
            .collect(),
    )
}

/// The least-squares polynomial of degree `deg` through `samples`, lowest power first, by the
/// normal equations and Gaussian elimination with partial pivoting; `None` when they are singular,
/// as with fewer distinct samples than coefficients.
fn polyfit(samples: &[(f64, f64)], deg: usize) -> Option<Vec<f64>> {
    let n = deg + 1;
    let mut a = vec![vec![0.0; n + 1]; n];
    for &(x, y) in samples {
        let powers: Vec<f64> = (0..2 * n)
            .scan(1.0, |p, _| {
                let v = *p;
                *p *= x;
                Some(v)
            })
            .collect();
        for i in 0..n {
            for j in 0..n {
                a[i][j] += powers[i + j];
            }
            a[i][n] += powers[i] * y;
        }
    }
    for col in 0..n {
        let pivot = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        a.swap(col, pivot);
        if a[col][col] == 0.0 {
            return None;
        }
        let (top, rest) = a.split_at_mut(col + 1);
        let pivot = &top[col];
        for row in rest {
            let f = row[col] / pivot[col];
            for (v, p) in row[col..].iter_mut().zip(&pivot[col..]) {
                *v -= f * p;
            }
        }
    }
    let mut c = vec![0.0; n];
    for i in (0..n).rev() {
        let s: f64 = (i + 1..n).map(|k| a[i][k] * c[k]).sum();
        c[i] = (a[i][n] - s) / a[i][i];
    }
    Some(c)
}

/// `v` through `lookup`, interpolated linearly between its samples.
pub fn apply(lookup: &[f32], v: f32) -> f32 {
    let p = v.clamp(0.0, 1.0) * (lookup.len() - 1) as f32;
    let i = (p as usize).min(lookup.len() - 2);
    lookup[i] + (lookup[i + 1] - lookup[i]) * (p - i as f32)
}

/// The BLAKE3 hex digest of `lookup`'s values as little-endian `f32`.
pub fn key(lookup: &[f32]) -> String {
    let bytes: Vec<u8> = lookup.iter().flat_map(|v| v.to_le_bytes()).collect();
    blake3::hash(&bytes).to_hex().to_string()
}

/// The key of a tone map whose key is `tone_key` read through a lookup whose key is
/// `lookup_key`: the BLAKE3 hex digest of the canonical JSON `{"response": lookup_key, "tone":
/// tone_key}`.
pub fn corrected_key(tone_key: &str, lookup_key: &str) -> String {
    let key = serde_json::json!({"response": lookup_key, "tone": tone_key});
    blake3::hash(crate::runner::canonical_json(&key).as_bytes())
        .to_hex()
        .to_string()
}

/// The wedge run that a run with `response` auto drives before its first stage, and its slot.
pub(crate) struct Prelude {
    pub run: Box<Run>,
    slot: Arc<Mutex<Shared>>,
    wedge: Wedge,
}

impl Prelude {
    /// The wedge run for a run of `style` at `settings` over `image`, `canvas` and `pens`,
    /// sharing `cache` and `cancel`; `None` unless the style's tone stage takes valid settings
    /// with `response` auto and there is a pen.
    pub fn new(
        style: &Arc<Style>,
        settings: &Value,
        [image, canvas, pens]: [&Artifact; 3],
        cache: &Arc<Mutex<StageCache>>,
        cancel: &Arc<AtomicBool>,
    ) -> Option<Prelude> {
        let (Artifact::Image(image), Artifact::Canvas(canvas), Artifact::Pens(pens)) =
            (image, canvas, pens)
        else {
            return None;
        };
        let stage = style.stages.iter().find(|s| s.provides() == Kind::Tone)?;
        // As the runner merges them: a stage the settings leave out takes its own defaults.
        let own = match settings.get(stage.name()) {
            Some(s) => merge(&stage.defaults(), s),
            None => stage.defaults(),
        };
        let tone = validate::<ToneSettings>(stage.name(), &stage.schema(), &own).ok()?;
        if tone.response != Response::Auto {
            return None;
        }
        let pen: &Pen = pens.first()?;
        let (w, h) = (image.width as usize, image.height as usize);
        let (tw, th) = tone::map_size(w, h, canvas, tone.resolution);
        let mm_per_px = canvas.image_transform(tw as u32, th as u32).0;
        let spacing = Density::of(style)
            .filter(|d| !d.direct)
            .and_then(|d| settings[&d.stage][&d.key].as_f64())
            .unwrap_or(0.0);
        let wedge = Wedge::new(mm_per_px, PATCH_MM.max(PATCH_SPACINGS * spacing));
        let slot = Arc::<Mutex<Shared>>::default();
        let run = Box::new(Run::new(
            style.clone(),
            wedge.settings(style, settings),
            [
                Artifact::Image(wedge.image.clone()),
                Artifact::Canvas(Arc::new(wedge.canvas.clone())),
                Artifact::Pens(Arc::new(vec![pen.clone()])),
            ],
            cache.clone(),
            slot.clone(),
            cancel.clone(),
            0, // a fresh slot's generation
        ));
        Some(Prelude { run, slot, wedge })
    }

    /// The lookup the finished wedge run gives; `None` when its response cannot be inverted.
    pub fn lookup(&self) -> Option<Vec<f32>> {
        let drawing = self.slot.lock().unwrap().drawing.clone()?;
        lookup(&self.wedge.measure(&drawing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pava_pools_falling_runs() {
        assert_eq!(pava(&[0.0, 1.0, 2.0]), [0.0, 1.0, 2.0]);
        assert_eq!(pava(&[1.0, 3.0, 2.0, 4.0]), [1.0, 2.5, 2.5, 4.0]);
        // A late drop pools back across earlier blocks until the means rise.
        let third = 5.0 / 3.0;
        assert_eq!(pava(&[1.0, 2.0, 3.0, 0.0]), [1.0, third, third, third]);
        assert_eq!(pava(&[]), Vec::<f64>::new());
    }

    #[test]
    fn the_lookup_inverts_a_known_curve() {
        // Coverage s^2 from 0 to 0.5: tone t should go to sqrt(t), whose coverage 0.5 t is on
        // the straight line to the darkest the style reaches.
        let samples: Vec<(f64, f64)> = (0..=64)
            .map(|i| {
                let s = i as f64 / 64.0;
                (s, 0.5 * s * s)
            })
            .collect();
        let l = lookup(&samples).unwrap();
        assert_eq!(l.len(), LOOKUP);
        assert_eq!((l[0], l[LOOKUP - 1]), (0.0, 1.0));
        for (j, &v) in l.iter().enumerate() {
            let t = j as f64 / 256.0;
            assert!((f64::from(v) - t.sqrt()).abs() < 0.02, "tone {t}: {v}");
        }
    }

    #[test]
    fn a_flat_response_gives_no_lookup() {
        let flat: Vec<(f64, f64)> = (0..16).map(|i| (i as f64 / 15.0, 0.2)).collect();
        assert_eq!(lookup(&flat), None);
        let falling: Vec<(f64, f64)> = (0..16)
            .map(|i| (i as f64 / 15.0, 0.5 - 0.01 * i as f64))
            .collect();
        assert_eq!(lookup(&falling), None);
    }

    #[test]
    fn an_s_shaped_response_is_straightened() {
        // Coverage 0.4 (3 s^2 - 2 s^3): its own midpoint stays put and its flat ends steepen.
        let samples: Vec<(f64, f64)> = (0..16)
            .map(|i| {
                let s = i as f64 / 15.0;
                (s, 0.4 * (3.0 * s * s - 2.0 * s * s * s))
            })
            .collect();
        let l = lookup(&samples).unwrap();
        assert!((l[128] - 0.5).abs() < 1e-3, "{}", l[128]);
        assert!(
            l[32] > 0.125 + 0.05 && l[224] < 0.875 - 0.05,
            "{} {}",
            l[32],
            l[224]
        );
    }

    #[test]
    fn noise_on_a_straight_response_leaves_a_smooth_rising_lookup() {
        // A straight response with patches off by up to 0.005, as the wedge's are.
        let samples: Vec<(f64, f64)> = (0..16)
            .map(|i| {
                let s = i as f64 / 15.0;
                let noise = [0.005, -0.004, 0.002, -0.005][i % 4];
                (s, 0.15 + 0.4 * s + noise)
            })
            .collect();
        let l = lookup(&samples).unwrap();
        assert!(l.windows(2).all(|w| w[1] >= w[0]));
        for (j, &v) in l.iter().enumerate() {
            assert!(
                (f64::from(v) - j as f64 / 256.0).abs() < 0.04,
                "tone {j}/256: {v}"
            );
        }
    }

    #[test]
    fn the_top_of_the_range_is_found_whatever_the_rounding() {
        // `lo + 1 * (hi - lo)` can round above `hi`; the plot laws found ranges where it did.
        for a in 0..40 {
            for b in 1..40 {
                let (lo, span) = (a as f64 * 0.0137, b as f64 * 0.0213);
                let samples: Vec<(f64, f64)> = (0..16)
                    .map(|i| {
                        let s = i as f64 / 15.0;
                        (s, lo + span * s)
                    })
                    .collect();
                let l = lookup(&samples).unwrap();
                assert!(
                    (l[LOOKUP - 1] - 1.0).abs() < 1e-3,
                    "{lo} {span}: {}",
                    l[LOOKUP - 1]
                );
            }
        }
    }

    #[test]
    fn apply_interpolates_and_clamps() {
        let l = [0.0, 0.5, 1.0];
        assert_eq!(apply(&l, 0.25), 0.25);
        assert_eq!(apply(&l, 1.0), 1.0);
        assert_eq!(apply(&l, 2.0), 1.0);
        assert_eq!(apply(&l, -1.0), 0.0);
    }

    #[test]
    fn the_wedge_steps_from_white_to_black_in_snake_order() {
        let w = Wedge::new(0.25, 20.0);
        assert_eq!(w.patch_px, 80);
        assert_eq!((w.image.width, w.canvas.width_mm), (320, 80.0));
        let grey = |x: usize, y: usize| w.image.rgb[(y * 320 + x) * 3];
        assert_eq!(grey(0, 0), 1.0);
        assert_eq!(grey(319, 0), 1.0 - 3.0 / 15.0);
        assert_eq!(grey(319, 80), 1.0 - 4.0 / 15.0);
        assert_eq!(grey(0, 319), 0.0);
        // A coarse map still gets patches of 16 pixels.
        assert_eq!(Wedge::new(4.0, 20.0).patch_px, PATCH_PX);
    }
}
