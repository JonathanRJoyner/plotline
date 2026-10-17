//! The Pen use stage: share the tone map between the pens.
//!
//! In `one_pen` mode the first pen draws everything, so the one map is the tone map itself. In
//! `light_to_dark` mode the ink range is cut into one band per pen, in the pens' list order: with
//! `n` pens the first pen's ink rises over the lightest `1/n` of the range and then stays full, and
//! each later pen starts where the one before it is full, so it adds ink only to darker tones.
//! Adding strokes tone by tone, so that a darker tone keeps the marks of the lighter ones, is how
//! tonal art maps build hatching (Praun, Hoppe, Webb and Finkelstein, "Real-time hatching",
//! SIGGRAPH 2001).
//!
//! In `by_colour` mode each pen's map is its coverage of each pixel's colour. The image's red,
//! green and blue are sampled on the tone map's grid. Each colour's channel inks are moved so their
//! luma becomes the tone map's ink, which carries the Image settings over: they are shifted to the
//! tone's ink and their spread about their luma is scaled down only as far as needed to stay in
//! [0, 1], so a grey or white pixel becomes grey and the rule has no jump near white. The pens are
//! transparent inks laid at independent positions, which holds for marks turned against each
//! other as for rotated print screens (Amidror and Hersch, "Neugebauer and Demichel: dependence
//! and independence in n-screen superpositions for colour printing", Color Research and
//! Application 25(4), 2000). Under independence the Demichel equations (Demichel, Procédé, 1924)
//! give the share of paper under each overlap of inks, and the Neugebauer model (Neugebauer, "Die
//! theoretischen Grundlagen des Mehrfarbenbuchdrucks", Zeitschrift für wissenschaftliche
//! Photographie 36, 1937) sums their colours; per channel the sum is
//! `R_c(a) = prod_i (1 - a_i k_ic)`, with `k_ic` one minus pen `i`'s channel `c`.
//!
//! Colours are used as stored, sRGB-encoded, not linearised. The tone stage takes ink as one minus
//! the encoded luminance, so one black pen on a grey image covers what `one_pen` gives it, and a
//! product of colours is nearly the same in encoded and linear values since the encoding is close
//! to a power law.
//!
//! The coverages minimise `E(a) = sum_c (R_c(a) - t_c)^2 + LAMBDA sum_i a_i` over [0, 1]. The
//! small ink penalty picks one answer when several match, and gives the greys to a black pen,
//! which reaches them with less ink than colours stacked. `E` is quadratic in each coverage alone,
//! so exact coordinate descent applies (Wright, "Coordinate descent algorithms", Mathematical
//! Programming 151, 2015), and no update raises `E`. Each sweep visits the darkest pen first:
//! from zero, a sweep in list order lets cyan, magenta and yellow claim a black before the black
//! pen is visited, a coordinate-wise minimum the descent cannot leave, while the darkest pen first
//! takes the shared darkness and the penalty keeps the colour pens out of the greys. The solve
//! runs in f64 with plain arithmetic, so it gives the same bits on every target. Solving every
//! pixel would be slow, so it runs once per run on a `LEVELS`³ grid of colours, and each pixel
//! interpolates the grid trilinearly.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::preview::rgb;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::stages::tone::{crop_to_source, resize};
use crate::types::{Canvas, Image, Pen, ToneMap};

/// How the pens share the drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SplitMode {
    /// The first pen draws everything.
    OnePen,
    /// Pens share the darkness in list order: the first draws from the lightest tones, each later pen adds only to darker ones.
    LightToDark,
    /// Each pen draws the part of the image its colour makes up.
    ByColour,
}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the pen use stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SplitSettings {
    /// How the pens share the drawing: the first pen alone, light to dark in list order, or each pen by its colour; list light inks first, since pens plot in list order.
    #[schemars(extend("x-label" = "Pen use"))]
    pub mode: SplitMode,
}

impl Default for SplitSettings {
    fn default() -> Self {
        SplitSettings {
            mode: SplitMode::OnePen,
        }
    }
}

/// Share the tone map between the pens: one map per pen that draws, the lane's own tone for every
/// stage after this one.
pub struct SplitStage;

impl Stage for SplitStage {
    fn name(&self) -> &'static str {
        "split"
    }
    fn label(&self) -> &'static str {
        "Pen use"
    }
    fn version(&self) -> u32 {
        1
    }
    // The image and the canvas are for the modes that read colour; the others leave them alone.
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Image, Kind::Canvas, Kind::Pens, Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Tones
    }
    fn schema(&self) -> Value {
        schema_for::<SplitSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<SplitSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let settings = validate::<SplitSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(SplitRun {
            input: Some((inputs, settings.mode)),
            colour: None,
            maps: None,
        }))
    }
}

/// The most pixels one band of the colour lookup covers, so that Stop stays responsive on a large
/// map. A step ends at a row boundary, so it holds whole rows.
const PIXELS_PER_STEP: usize = 200_000;

/// `one_pen` and `light_to_dark` do their work in the first step, which reports one frame; the
/// second gives the maps. `by_colour` spends its first steps on preparation, one piece each: a
/// colour plane, then the next, then the third, then the table. Then each step looks up a band of
/// rows and reports how many are done, and the step after the last band gives the maps.
struct SplitRun {
    /// What the first step reads: the inputs and the mode; `None` once it has run.
    input: Option<(Inputs, SplitMode)>,
    /// The colour lookup in progress, from the first step to the last band.
    colour: Option<ColourSplit>,
    /// The finished maps, before the step that gives them.
    maps: Option<Vec<Arc<ToneMap>>>,
}

/// A frame with no points or paths to show.
fn frame(progress: f32, label: impl Into<String>) -> Frame {
    Frame {
        stage: String::new(),
        progress,
        label: label.into(),
        points: None,
        paths: None,
        pen: 0,
    }
}

impl StageRun for SplitRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some((inputs, mode)) = self.input.take() {
            let (tone, pens) = (inputs.tone(), inputs.pens());
            match mode {
                SplitMode::OnePen => self.maps = Some(vec![tone.clone()]),
                SplitMode::LightToDark => {
                    let maps = bands(tone, pens.len());
                    self.maps = Some(maps.into_iter().map(Arc::new).collect());
                }
                SplitMode::ByColour => {
                    let (image, canvas) = (inputs.image(), inputs.canvas());
                    self.colour = Some(ColourSplit::new(image, canvas, pens, tone));
                }
            }
            if self.maps.is_some() {
                return Ok(Step::Frame(frame(1.0, "pen use")));
            }
        }
        if let Some(split) = &mut self.colour {
            if !split.ready() {
                split.prepare()?;
                return Ok(Step::Frame(frame(0.0, "pen use: colour")));
            }
            let h = split.tone.height as usize;
            split.look_up((split.next_row + split.rows_per_step).min(h));
            let done = split.next_row;
            let progress = if done == h {
                1.0
            } else {
                done as f32 / h as f32
            };
            let label = format!("pen use: {done}/{h} rows");
            if done == h {
                let split = self.colour.take().expect("the colour lookup is running");
                self.maps = Some(split.into_maps().into_iter().map(Arc::new).collect());
            }
            return Ok(Step::Frame(frame(progress, label)));
        }
        let maps = self
            .maps
            .take()
            .expect("the split run has already finished");
        Ok(Step::Done(Artifact::Tones(Arc::new(maps))))
    }
}

/// `n` maps that cut `tone`'s ink range into equal bands: map `k` has ink
/// `clamp(n * t - k, 0, 1)` for the tone's ink `t`, and the size and transform of `tone`. Together
/// they hold `n * t` of ink, and map `k + 1` has ink only where map `k` is full.
pub(crate) fn bands(tone: &ToneMap, n: usize) -> Vec<ToneMap> {
    (0..n)
        .map(|k| ToneMap {
            width: tone.width,
            height: tone.height,
            ink: tone
                .ink
                .iter()
                .map(|&t| (n as f32 * t - k as f32).clamp(0.0, 1.0))
                .collect(),
            mm_per_px: tone.mm_per_px,
            origin_mm: tone.origin_mm,
        })
        .collect()
}

/// The colour lookup for the by-colour mode: for each pixel of the tone map, every pen's coverage
/// of the pixel's colour. The colour is sampled on the tone map's grid with the tone stage's own
/// crop and resize, and rescaled to the tone map's ink before the table lookup. [`prepare`]
/// samples the planes and solves the table one piece at a time, and [`look_up`] then covers a band
/// of rows at a time.
///
/// [`prepare`]: ColourSplit::prepare
/// [`look_up`]: ColourSplit::look_up
struct ColourSplit {
    image: Arc<Image>,
    canvas: Arc<Canvas>,
    /// How much of each channel each pen absorbs.
    absorb: Vec<[f64; 3]>,
    tone: Arc<ToneMap>,
    /// The image's red, green and blue on the tone map's grid, row-major, as far as sampled.
    planes: Vec<Vec<f32>>,
    /// The table, once solved, after the planes.
    table: Option<Table>,
    /// Per pen, the coverage of the pixels looked up so far, row-major.
    inks: Vec<Vec<f32>>,
    /// The coverages of the pixel in hand, one per pen.
    a: Vec<f64>,
    /// The rows a band holds at most.
    rows_per_step: usize,
    /// The first row not looked up yet.
    next_row: usize,
}

impl ColourSplit {
    fn new(
        image: &Arc<Image>,
        canvas: &Arc<Canvas>,
        pens: &[Pen],
        tone: &Arc<ToneMap>,
    ) -> ColourSplit {
        ColourSplit {
            image: image.clone(),
            canvas: canvas.clone(),
            absorb: pens.iter().map(absorption).collect(),
            planes: Vec::new(),
            table: None,
            inks: vec![Vec::with_capacity(tone.ink.len()); pens.len()],
            a: vec![0.0; pens.len()],
            rows_per_step: (PIXELS_PER_STEP / tone.width as usize).max(1),
            next_row: 0,
            tone: tone.clone(),
        }
    }

    /// Whether the planes are sampled and the table is solved.
    fn ready(&self) -> bool {
        self.table.is_some()
    }

    /// The next piece of preparation: the next of the red, green and blue planes, sampled on the
    /// tone map's grid, and after them the table. Fails if the colour grid and the tone map differ
    /// in size.
    fn prepare(&mut self) -> Result<(), StageError> {
        let k = self.planes.len();
        if k == 3 {
            self.table = Some(Table::new(&self.absorb));
            return Ok(());
        }
        let (image, canvas) = (&self.image, &self.canvas);
        let (w, h) = (image.width as usize, image.height as usize);
        let long_edge = self.tone.width.max(self.tone.height) as usize;
        let plane: Vec<f32> = image.rgb.iter().skip(k).step_by(3).copied().collect();
        let (plane, cw, ch) = crop_to_source(&plane, w, h, canvas);
        let (plane, cw, ch) = resize(&plane, cw, ch, long_edge);
        if (cw, ch) != (self.tone.width as usize, self.tone.height as usize) {
            return Err(StageError::Failed(format!(
                "the colour grid is {cw} x {ch} pixels but the tone map is {} x {}",
                self.tone.width, self.tone.height
            )));
        }
        self.planes.push(plane);
        Ok(())
    }

    /// Look up the rows from `next_row` up to row `end`, not including it. Needs [`ready`].
    ///
    /// [`ready`]: ColourSplit::ready
    fn look_up(&mut self, end: usize) {
        let w = self.tone.width as usize;
        let [r, g, b] = &self.planes[..] else {
            unreachable!("the planes are sampled");
        };
        let table = self.table.as_ref().expect("the table is solved");
        for p in self.next_row * w..end * w {
            table.lookup(rescale([r[p], g[p], b[p]], self.tone.ink[p]), &mut self.a);
            for (ink, &a) in self.inks.iter_mut().zip(&self.a) {
                ink.push((a as f32).clamp(0.0, 1.0));
            }
        }
        self.next_row = end;
    }

    /// One map per pen, with the size and transform of the tone map.
    fn into_maps(self) -> Vec<ToneMap> {
        let tone = &self.tone;
        self.inks
            .into_iter()
            .map(|ink| ToneMap {
                width: tone.width,
                height: tone.height,
                ink,
                mm_per_px: tone.mm_per_px,
                origin_mm: tone.origin_mm,
            })
            .collect()
    }
}

/// The ink penalty: small, so it only picks one of several coverages that match the colour.
pub(crate) const LAMBDA: f64 = 1e-3;
/// Coordinate-descent sweeps per colour.
pub(crate) const SWEEPS: usize = 64;
/// Colours per axis of the lookup table, at `j / (LEVELS - 1)`.
pub(crate) const LEVELS: usize = 17;

/// The colour `c` moved to the tone map's ink `t`. With channel inks `ink_k = 1 - c_k` and their
/// BT.601 luma `u`, each ink becomes `t + beta (ink_k - u)`, where `beta` is the largest value up
/// to 1 that keeps every channel in [0, 1]. The luma of the result is `t`, the colour is unchanged
/// when `t = u`, a grey or white pixel becomes the grey `1 - t`, and the rule is continuous.
pub(crate) fn rescale(c: [f32; 3], t: f32) -> [f64; 3] {
    let ink = c.map(|k| 1.0 - f64::from(k));
    let u = 0.299 * ink[0] + 0.587 * ink[1] + 0.114 * ink[2];
    let (hi, lo) = (
        ink[0].max(ink[1]).max(ink[2]),
        ink[0].min(ink[1]).min(ink[2]),
    );
    let t = f64::from(t);
    let mut beta: f64 = 1.0;
    // A bound whose denominator is not positive cannot be reached, so it is skipped.
    if hi - u > 0.0 {
        beta = beta.min((1.0 - t) / (hi - u));
    }
    if u - lo > 0.0 {
        beta = beta.min(t / (u - lo));
    }
    // The clamp only guards against rounding.
    ink.map(|k| 1.0 - (t + beta * (k - u)).clamp(0.0, 1.0))
}

/// How much of each channel a pen's colour absorbs: `1 - byte / 255`.
pub(crate) fn absorption(pen: &Pen) -> [f64; 3] {
    rgb(&pen.color).map(|byte| 1.0 - f64::from(byte) / 255.0)
}

/// The pens by total absorption, darkest first; pens that tie keep their list order.
pub(crate) fn sweep_order(absorb: &[[f64; 3]]) -> Vec<usize> {
    let total = |i: usize| absorb[i].iter().sum::<f64>();
    let mut order: Vec<usize> = (0..absorb.len()).collect();
    order.sort_by(|&i, &j| total(j).total_cmp(&total(i)));
    order
}

/// One pass of exact coordinate descent on [`energy`]: each pen in `order` in turn takes the
/// coverage that minimises `E` with the others held, clamped to [0, 1], or 0 for a pen that
/// cannot change the colour.
pub(crate) fn sweep(a: &mut [f64], target: [f64; 3], absorb: &[[f64; 3]], order: &[usize]) {
    for &i in order {
        let (mut num, mut den) = (0.0, 0.0);
        for c in 0..3 {
            let q: f64 = (0..a.len())
                .filter(|&j| j != i)
                .map(|j| 1.0 - a[j] * absorb[j][c])
                .product();
            let g = absorb[i][c] * q;
            num += g * (q - target[c]);
            den += g * g;
        }
        a[i] = if den > 0.0 {
            ((num - LAMBDA / 2.0) / den).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
}

/// `E(a) = sum_c (R_c(a) - t_c)^2 + LAMBDA sum_i a_i`, with `R_c(a) = prod_i (1 - a_i k_ic)`.
#[cfg(test)]
pub(crate) fn energy(a: &[f64], target: [f64; 3], absorb: &[[f64; 3]]) -> f64 {
    let fit: f64 = (0..3)
        .map(|c| {
            let r: f64 = a.iter().zip(absorb).map(|(&a, k)| 1.0 - a * k[c]).product();
            (r - target[c]) * (r - target[c])
        })
        .sum();
    fit + LAMBDA * a.iter().sum::<f64>()
}

/// The pens' coverages for `target`: from zero, [`SWEEPS`] sweeps in [`sweep_order`].
pub(crate) fn coverages(target: [f64; 3], absorb: &[[f64; 3]]) -> Vec<f64> {
    // Darkest first: from zero, a sweep in list order lets cyan, magenta and yellow claim a black
    // before the black pen is visited, a coordinate-wise minimum the descent cannot leave. The
    // darkest pen first takes the shared darkness, and the penalty keeps the others out of greys.
    let order = sweep_order(absorb);
    let mut a = vec![0.0; absorb.len()];
    for _ in 0..SWEEPS {
        sweep(&mut a, target, absorb, &order);
    }
    a
}

/// The pens' coverages solved on a `LEVELS`³ grid of colours, read back by trilinear
/// interpolation.
pub(crate) struct Table {
    /// The number of pens.
    pens: usize,
    /// The coverages at colour `(r, g, b) / (LEVELS - 1)`, at `((r * LEVELS + g) * LEVELS + b) *
    /// pens`, pen by pen.
    a: Vec<f64>,
}

impl Table {
    /// Solve [`coverages`] at every grid colour for pens of absorption `absorb`.
    pub(crate) fn new(absorb: &[[f64; 3]]) -> Table {
        let level = |j: usize| j as f64 / (LEVELS - 1) as f64;
        let mut a = Vec::with_capacity(LEVELS * LEVELS * LEVELS * absorb.len());
        for r in 0..LEVELS {
            for g in 0..LEVELS {
                for b in 0..LEVELS {
                    a.extend(coverages([level(r), level(g), level(b)], absorb));
                }
            }
        }
        Table {
            pens: absorb.len(),
            a,
        }
    }

    /// Write the coverages at colour `c` into `out`, one per pen, interpolated trilinearly with
    /// each channel clamped to [0, 1].
    pub(crate) fn lookup(&self, c: [f64; 3], out: &mut [f64]) {
        let axis = |v: f64| {
            let x = v.clamp(0.0, 1.0) * (LEVELS - 1) as f64;
            let i = (x as usize).min(LEVELS - 2);
            (i, x - i as f64)
        };
        let [(r, fr), (g, fg), (b, fb)] = c.map(axis);
        let weight = |side: usize, f: f64| if side == 1 { f } else { 1.0 - f };
        out.fill(0.0);
        for corner in 0..8 {
            let (dr, dg, db) = (corner >> 2, corner >> 1 & 1, corner & 1);
            let w = weight(dr, fr) * weight(dg, fg) * weight(db, fb);
            let at = (((r + dr) * LEVELS + g + dg) * LEVELS + b + db) * self.pens;
            for (o, &a) in out.iter_mut().zip(&self.a[at..at + self.pens]) {
                *o += w * a;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::merge;
    use crate::stages::tone::ToneStage;
    use crate::synthetic::{color_bars, flat, gradient, scene};
    use crate::test_util::{a4, black, tone_map};
    use crate::types::Fit;
    use rand::{Rng, SeedableRng};
    use rand_pcg::Pcg64;
    use serde_json::json;
    use std::collections::HashMap;

    /// Run the split stage to the end with `patch` over the defaults.
    fn run_split(
        image: Image,
        canvas: Canvas,
        pens: Vec<Pen>,
        tone: ToneMap,
        patch: Value,
    ) -> Result<(Vec<Frame>, Vec<Arc<ToneMap>>), StageError> {
        let settings = merge(&SplitStage.defaults(), &patch);
        let inputs = Inputs::new(HashMap::from([
            (Kind::Image, Artifact::Image(Arc::new(image))),
            (Kind::Canvas, Artifact::Canvas(Arc::new(canvas))),
            (Kind::Pens, Artifact::Pens(Arc::new(pens))),
            (Kind::Tone, Artifact::Tone(Arc::new(tone))),
        ]));
        let mut run = SplitStage.start(inputs, &settings, RunContext::new(Arc::default()))?;
        let mut frames = Vec::new();
        loop {
            match run.step()? {
                Step::Frame(f) => frames.push(f),
                Step::Done(Artifact::Tones(maps)) => return Ok((frames, maps.to_vec())),
                Step::Done(a) => panic!("expected tone maps, got {}", a.kind().name()),
            }
        }
    }

    /// [`run_split`] on a flat image and a 100 mm square page, which these modes do not read.
    fn split(pens: usize, tone: ToneMap, patch: Value) -> Vec<Arc<ToneMap>> {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let pens = vec![black(); pens];
        run_split(flat(4, 4, 0.5), canvas, pens, tone, patch)
            .unwrap()
            .1
    }

    #[test]
    fn settings_schema() {
        assert_eq!(
            (SplitStage.name(), SplitStage.label()),
            ("split", "Pen use")
        );
        assert_eq!(SplitStage.defaults(), json!({"mode": "one_pen"}));
        let schema = SplitStage.schema();
        let variants = schema["$defs"]["SplitMode"]["oneOf"].as_array().unwrap();
        let names: Vec<_> = variants
            .iter()
            .map(|v| v["const"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["one_pen", "light_to_dark", "by_colour"]);
        assert!(variants.iter().all(|v| v["description"].is_string()));
        assert!(schema["properties"]["mode"]["description"].is_string());
        // The form shows the setting under the stage's own name.
        assert_eq!(schema["properties"]["mode"]["x-label"], "Pen use");
    }

    #[test]
    fn one_pen_passes_the_map_through() {
        let tone = tone_map(16, 4, |x, y| (x + y) as f32 / 19.0);
        let maps = split(2, tone.clone(), json!({}));
        assert_eq!(maps.len(), 1);
        assert_eq!(*maps[0], tone);
    }

    #[test]
    fn light_to_dark_splits_the_tone_in_list_order() {
        let tone = tone_map(16, 4, |x, _| x as f32 / 15.0);
        let maps = split(2, tone.clone(), json!({"mode": "light_to_dark"}));
        assert_eq!(maps.len(), 2);
        let want = |k: f32| -> Vec<f32> {
            tone.ink
                .iter()
                .map(|&t| (2.0 * t - k).clamp(0.0, 1.0))
                .collect()
        };
        assert_eq!(maps[0].ink, want(0.0));
        assert_eq!(maps[1].ink, want(1.0));
        for m in &maps {
            assert_eq!((m.width, m.height), (tone.width, tone.height));
            assert_eq!((m.mm_per_px, m.origin_mm), (tone.mm_per_px, tone.origin_mm));
        }
        let one = split(1, tone.clone(), json!({"mode": "light_to_dark"}));
        assert_eq!(one.len(), 1);
        assert_eq!(*one[0], tone);
    }

    #[test]
    fn bands_add_up_and_nest() {
        let tone = tone_map(11, 1, |x, _| x as f32 / 10.0);
        let maps = bands(&tone, 3);
        assert_eq!(maps.len(), 3);
        for i in 0..11 {
            let t = tone.ink[i];
            // Each later pen starts only once the one before is full, and together they give 3t.
            assert!(maps[1].ink[i] == 0.0 || maps[0].ink[i] == 1.0);
            assert!(maps[2].ink[i] == 0.0 || maps[1].ink[i] == 1.0);
            let total: f32 = maps.iter().map(|m| m.ink[i]).sum();
            assert!((total - 3.0 * t).abs() < 1e-5, "pixel {i}: {total}");
        }
    }

    const CMYK: [&str; 4] = ["#00ffff", "#ff00ff", "#ffff00", "#000000"];

    /// Pens 0.3 mm wide in `colors`, in order.
    fn pens(colors: &[&str]) -> Vec<Pen> {
        colors
            .iter()
            .map(|c| Pen::new("Pen", 0.3, c).unwrap())
            .collect()
    }

    /// The tone stage at `resolution` on `image`.
    fn tone_of(image: &Image, canvas: &Canvas, resolution: u32) -> Arc<ToneMap> {
        let settings = merge(&ToneStage.defaults(), &json!({"resolution": resolution}));
        let inputs = Inputs::new(HashMap::from([
            (Kind::Image, Artifact::Image(Arc::new(image.clone()))),
            (Kind::Canvas, Artifact::Canvas(Arc::new(canvas.clone()))),
        ]));
        let mut run = ToneStage
            .start(inputs, &settings, RunContext::new(Arc::default()))
            .unwrap();
        loop {
            if let Step::Done(Artifact::Tone(t)) = run.step().unwrap() {
                return t;
            }
        }
    }

    /// The tone stage at resolution 64 on `image`, then the split by colour. Returns the tone map
    /// and the lanes.
    fn split_image(
        image: Image,
        canvas: Canvas,
        pens: Vec<Pen>,
    ) -> (Arc<ToneMap>, Vec<Arc<ToneMap>>) {
        let tone = tone_of(&image, &canvas, 64);
        let patch = json!({"mode": "by_colour"});
        let maps = run_split(image, canvas, pens, (*tone).clone(), patch)
            .unwrap()
            .1;
        (tone, maps)
    }

    #[test]
    fn one_black_pen_matches_the_tone() {
        let (tone, maps) = split_image(gradient(64, 48), a4(), vec![black()]);
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].ink.len(), tone.ink.len());
        for (p, (&a, &t)) in maps[0].ink.iter().zip(&tone.ink).enumerate() {
            assert!((a - t).abs() < 1e-3, "pixel {p}: {a} for tone {t}");
        }
    }

    #[test]
    fn cmyk_pens_take_the_colour_bars() {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let (tone, maps) = split_image(color_bars(64, 48), canvas, pens(&CMYK));
        // Bars left to right: white, yellow, cyan, green, magenta, red, blue, black.
        let bar = |i: usize| -> [f32; 4] {
            let p = 24 * tone.width as usize + 8 * i + 4;
            [0, 1, 2, 3].map(|pen| maps[pen].ink[p])
        };
        let white = bar(0);
        assert!(white.iter().all(|&v| v <= 1e-3), "white: {white:?}");
        let [c, m, y, k] = bar(7);
        assert!(
            k > 0.9 && c < 0.2 && m < 0.2 && y < 0.2,
            "black: {:?}",
            bar(7)
        );
        let [c, m, y, k] = bar(5);
        assert!(
            m > 0.8 && y > 0.8 && c < 0.2 && k < 0.2,
            "red: {:?}",
            bar(5)
        );
        let [c, m, y, k] = bar(2);
        assert!(
            c > 0.8 && m < 0.2 && y < 0.2 && k < 0.2,
            "cyan: {:?}",
            bar(2)
        );
    }

    #[test]
    fn a_white_pen_takes_no_ink() {
        let absorb: Vec<_> = pens(&["#ffffff", "#000000"])
            .iter()
            .map(absorption)
            .collect();
        let a = coverages([0.5; 3], &absorb);
        assert_eq!(a[0], 0.0);
        assert!((a[1] - 0.5).abs() < 1e-3, "{a:?}");
    }

    #[test]
    fn two_pens_of_one_colour_share_without_nan() {
        let absorb: Vec<_> = pens(&["#000000", "#000000"])
            .iter()
            .map(absorption)
            .collect();
        let a = coverages([0.5; 3], &absorb);
        assert!(a.iter().all(|v| v.is_finite()), "{a:?}");
        assert!((a[0] - 0.5).abs() < 1e-3 && a[1] < 1e-3, "{a:?}");
    }

    #[test]
    fn a_sweep_never_raises_the_energy() {
        let mut rng = Pcg64::seed_from_u64(7);
        for _ in 0..200 {
            let target: [f64; 3] = std::array::from_fn(|_| rng.random());
            let n = rng.random_range(1..=5);
            let absorb: Vec<[f64; 3]> = (0..n)
                .map(|_| std::array::from_fn(|_| rng.random()))
                .collect();
            let order = sweep_order(&absorb);
            let mut a = vec![0.0; n];
            for _ in 0..10 {
                let before = energy(&a, target, &absorb);
                sweep(&mut a, target, &absorb, &order);
                let after = energy(&a, target, &absorb);
                assert!(
                    after <= before + 1e-12,
                    "{before} to {after} for {target:?} and {absorb:?}"
                );
            }
        }
    }

    #[test]
    fn white_takes_grey_under_inverted_tone() {
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6);
        assert!(close(rescale([1.0; 3], 0.6), [0.4; 3]));
        for c in [[0.2, 0.5, 0.9], [1.0, 0.0, 0.0], [0.0; 3], [0.7, 0.7, 0.1]] {
            let [r, g, b] = c;
            let y: f32 = 0.299 * r + 0.587 * g + 0.114 * b;
            assert!(close(rescale(c, 1.0 - y), c.map(f64::from)), "{c:?}");
        }
    }

    #[test]
    fn rescale_does_not_jump_near_white() {
        let c = rescale([1.0, 1.0, 0.996], 1.0);
        assert!(c.iter().all(|&v| v.abs() < 0.01), "{c:?}");
    }

    #[test]
    fn rescale_gives_the_tone_s_luma_in_range() {
        for c in [
            [0.2, 0.5, 0.9],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.996],
            [0.0, 0.3, 1.0],
        ] {
            for t in [0.0, 0.1, 0.35, 0.5, 0.8, 0.99, 1.0] {
                let out = rescale(c, t);
                let [r, g, b] = out.map(|v| 1.0 - v);
                let luma = 0.299 * r + 0.587 * g + 0.114 * b;
                assert!((luma - f64::from(t)).abs() < 1e-6, "{c:?} at {t}: {out:?}");
                assert!(
                    out.iter().all(|v| (0.0..=1.0).contains(v)),
                    "{c:?} at {t}: {out:?}"
                );
            }
        }
    }

    #[test]
    fn lightening_keeps_the_hue() {
        let [r, g, b] = rescale([1.0, 0.0, 0.0], 0.5);
        assert!(
            (r - 1.0).abs() < 1e-6 && (g - b).abs() < 1e-6,
            "{:?}",
            [r, g, b]
        );
    }

    #[test]
    fn a_fill_page_and_a_small_image_give_maps_of_the_tone_size() {
        let canvas = Canvas::new(100.0, 150.0, 0.0, Fit::Fill).unwrap();
        let (tone, maps) = split_image(scene(30, 20), canvas, pens(&["#000000", "#ff0000"]));
        assert_eq!(maps.len(), 2);
        for m in &maps {
            assert_eq!(
                (m.width, m.height, m.ink.len()),
                (tone.width, tone.height, tone.ink.len())
            );
            assert_eq!((m.mm_per_px, m.origin_mm), (tone.mm_per_px, tone.origin_mm));
        }
    }

    #[test]
    fn by_colour_reads_the_colour_planes_through_the_canvas_crop() {
        // Red at both ends and blue from column 10 to 64. The crop keeps exactly the blue, so
        // planes cut anywhere else would put red into the red pen's map.
        let rgb = (0..40)
            .flat_map(|_| {
                (0..100).flat_map(|x| {
                    if (10..65).contains(&x) {
                        [0.0, 0.0, 1.0]
                    } else {
                        [1.0, 0.0, 0.0]
                    }
                })
            })
            .collect();
        let image = Image {
            width: 100,
            height: 40,
            rgb,
        };
        let canvas = Canvas::new(100.0, 40.0, 0.0, Fit::Fit)
            .unwrap()
            .with_crop([0.1, 0.0, 0.35, 0.0])
            .unwrap();
        let (tone, maps) = split_image(image, canvas, pens(&["#ff0000", "#0000ff"]));
        assert_eq!(maps[0].ink.len(), tone.ink.len());
        assert!(maps[0].ink.iter().all(|&v| v < 0.01));
        assert!(maps[1].ink.iter().all(|&v| v > 0.5));
    }

    #[test]
    fn by_colour_is_deterministic() {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let bits = || -> Vec<u32> {
            let maps = split_image(scene(64, 48), canvas.clone(), pens(&CMYK)).1;
            maps.iter()
                .flat_map(|m| m.ink.iter().map(|v| v.to_bits()))
                .collect()
        };
        assert_eq!(bits(), bits());
    }

    #[test]
    fn by_colour_refuses_a_tone_map_of_another_size() {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let tone = tone_map(16, 4, |_, _| 0.5);
        let patch = json!({"mode": "by_colour"});
        let err = run_split(flat(4, 4, 0.5), canvas, vec![black()], tone, patch).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the colour grid is 16 x 16 pixels but the tone map is 16 x 4"
        );
    }

    #[test]
    fn the_stage_reports_one_frame_then_the_maps() {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let tone = tone_map(4, 4, |_, _| 0.5);
        let (frames, maps) =
            run_split(flat(4, 4, 0.5), canvas, vec![black()], tone, json!({})).unwrap();
        assert_eq!(maps.len(), 1);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            (frames[0].progress, frames[0].label.as_str()),
            (1.0, "pen use")
        );
    }

    #[test]
    fn by_colour_reports_the_colour_then_each_band_of_rows() {
        // 40 000 pixels wide, so a band holds 5 rows and the 12 rows take 3 bands.
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let tone = tone_map(40_000, 12, |x, _| (x % 7) as f32 / 7.0);
        let patch = json!({"mode": "by_colour"});
        let pens = pens(&["#000000", "#ff0000"]);
        let (frames, maps) = run_split(scene(10_000, 3), canvas, pens, tone, patch).unwrap();
        let steps: Vec<_> = frames
            .iter()
            .map(|f| (f.label.as_str(), f.progress))
            .collect();
        assert_eq!(
            steps,
            [
                // Three colour planes, then the table.
                ("pen use: colour", 0.0),
                ("pen use: colour", 0.0),
                ("pen use: colour", 0.0),
                ("pen use: colour", 0.0),
                ("pen use: 5/12 rows", 5.0 / 12.0),
                ("pen use: 10/12 rows", 10.0 / 12.0),
                ("pen use: 12/12 rows", 1.0),
            ]
        );
        assert_eq!(maps.len(), 2);
        assert!(maps.iter().all(|m| m.ink.len() == 40_000 * 12));
    }

    /// A colour split of `image` that has done its preparation.
    fn prepared(image: &Image, canvas: &Canvas, pens: &[Pen], tone: &Arc<ToneMap>) -> ColourSplit {
        let (image, canvas) = (Arc::new(image.clone()), Arc::new(canvas.clone()));
        let mut split = ColourSplit::new(&image, &canvas, pens, tone);
        while !split.ready() {
            split.prepare().unwrap();
        }
        split
    }

    #[test]
    fn bands_of_one_row_give_the_maps_of_one_band() {
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let (image, pens) = (scene(64, 48), pens(&CMYK));
        let tone = tone_of(&image, &canvas, 64);
        let mut one_band = prepared(&image, &canvas, &pens, &tone);
        one_band.look_up(tone.height as usize);
        let whole = one_band.into_maps();
        let mut split = prepared(&image, &canvas, &pens, &tone);
        for end in 1..=tone.height as usize {
            split.look_up(end);
        }
        let by_row = split.into_maps();
        assert_eq!(whole.len(), 4);
        for (a, b) in whole.iter().zip(&by_row) {
            let bits = |m: &ToneMap| m.ink.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(a), bits(b));
        }
    }

    /// Time each step of a by-colour run at a 4000-pixel map with eight pens, for the brief. Run
    /// it in a release build: `cargo test --release -p plotline-core --lib by_colour_step_timings
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing, for release builds"]
    fn by_colour_step_timings() {
        use std::time::Instant;
        let canvas = a4();
        let image = scene(4000, 2800);
        let tone = tone_of(&image, &canvas, 4000);
        let colors = [
            "#000000", "#00ffff", "#ff00ff", "#ffff00", "#ff0000", "#00ff00", "#0000ff", "#8b4513",
        ];
        let settings = merge(&SplitStage.defaults(), &json!({"mode": "by_colour"}));
        let inputs = Inputs::new(HashMap::from([
            (Kind::Image, Artifact::Image(Arc::new(image))),
            (Kind::Canvas, Artifact::Canvas(Arc::new(canvas))),
            (Kind::Pens, Artifact::Pens(Arc::new(pens(&colors)))),
            (Kind::Tone, Artifact::Tone(tone.clone())),
        ]));
        let mut run = SplitStage
            .start(inputs, &settings, RunContext::new(Arc::default()))
            .unwrap();
        let mut times = Vec::new();
        loop {
            let started = Instant::now();
            let step = run.step().unwrap();
            let took = started.elapsed();
            match step {
                Step::Frame(f) => times.push((took, f.label)),
                Step::Done(_) => break,
            }
        }
        let total: std::time::Duration = times.iter().map(|(t, _)| *t).sum();
        let (longest, label) = times.iter().max_by_key(|(t, _)| *t).unwrap();
        let prepare: Vec<_> = times
            .iter()
            .filter(|(_, label)| label == "pen use: colour")
            .map(|(t, _)| *t)
            .collect();
        println!(
            "tone map {} x {}: {} steps, total {total:?}, preparation {prepare:?}, longest {longest:?} ({label})",
            tone.width,
            tone.height,
            times.len(),
        );
    }

    #[test]
    fn an_unknown_mode_is_refused() {
        let tone = tone_map(4, 4, |_, _| 0.5);
        let canvas = Canvas::new(100.0, 100.0, 0.0, Fit::Fit).unwrap();
        let err = run_split(
            flat(4, 4, 0.5),
            canvas,
            vec![black()],
            tone,
            json!({"mode": "darkest_first"}),
        )
        .unwrap_err();
        assert!(matches!(err, StageError::Settings(_)), "{err:?}");
    }
}
