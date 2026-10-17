//! The Poisson-disk placer: points packed so that no two are closer than a radius that grows
//! where the image is lighter.
//!
//! This is variable-radius Poisson-disk sampling after R. Bridson, "Fast Poisson disk sampling in
//! arbitrary dimensions", *SIGGRAPH Sketches*, 2007. A point at `p` needs every point stored
//! before it at least `r(p) = spacing / sqrt(ink(p))` away, capped at 20 spacings, and ink under
//! 1/400 takes no point. Growth runs from a list of active points: a random one is tried with 30
//! candidates, each at a random distance of one to two of its radii and a random angle, and is
//! dropped once all 30 fail. When no point is active, the next seed is a dart drawn by ink with
//! the weighted Voronoi placer's sampler, so a small dark area on a white page still gets a point.
//! Neighbours are found through a bucket grid with cells of `spacing`.
//!
//! When the drawing is split between pens, each lane draws its packing from its own random stream
//! ([`RunContext::rng`] cites why), so the pens' circles fall independently of each other.

use std::f64::consts::TAU;
use std::sync::Arc;

use rand::Rng;
use rand_pcg::Pcg64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::voronoi::sample_by_density;
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Points, ToneMap};

/// Ink under this takes no point.
const MIN_INK: f64 = 1.0 / 400.0;
/// The largest radius, in spacings.
const MAX_RADIUS: f64 = 20.0;
/// Candidates tried round an active point before it is dropped, and darts that fail in a row
/// before seeding stops.
const TRIES: usize = 30;
/// The end of a bucket's list of points.
const NONE: usize = usize::MAX;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the Poisson-disk placer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PoissonSettings {
    /// Distance between points over full ink; lighter areas space them further apart.
    #[schemars(range(min = 0.2, max = 20.0), extend("x-density" = "inverse", "x-step" = 0.1, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// Where the packing starts.
    #[schemars(range(min = 0, max = 9999))]
    pub seed: u32,
}

impl Default for PoissonSettings {
    fn default() -> Self {
        PoissonSettings {
            spacing_mm: 1.0,
            seed: 1,
        }
    }
}

/// Place points by variable-radius Poisson-disk sampling.
///
/// The darts are one batch of `max(expected, 30)` drawn at the start, where `expected` is the
/// rectangle's area over `spacing²`; they are used in order, and seeding also stops when they run
/// out. Points come out in the order they were accepted. A frame comes after every
/// `max(expected / 30, 1)` of them.
pub struct PoissonPlacer;

impl Stage for PoissonPlacer {
    fn name(&self) -> &'static str {
        "poisson"
    }
    fn label(&self) -> &'static str {
        "Poisson disk"
    }
    fn version(&self) -> u32 {
        2
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Points
    }
    fn schema(&self) -> Value {
        schema_for::<PoissonSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<PoissonSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<PoissonSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let [x0, y0, w, h] = tone.rect_mm();
        let expected = w * h / (s.spacing_mm * s.spacing_mm);
        let mut rng = ctx.rng(u64::from(s.seed));
        // One call: each rebuilds the density table.
        let darts = sample_by_density(&tone, 1.0, expected.max(30.0) as usize, &mut rng);
        let cols = (w / s.spacing_mm) as usize + 1;
        let rows = (h / s.spacing_mm) as usize + 1;
        Ok(Box::new(PoissonRun {
            tone,
            rng,
            origin: [x0, y0],
            far: [x0 + w, y0 + h],
            spacing: s.spacing_mm,
            darts: darts.into_iter(),
            dart_fails: 0,
            cols,
            rows,
            head: vec![NONE; cols * rows],
            next: Vec::new(),
            points: Vec::new(),
            radii: Vec::new(),
            active: Vec::new(),
            expected,
            frame_every: (expected / 30.0).max(1.0) as usize,
        }))
    }
}

struct PoissonRun {
    tone: Arc<ToneMap>,
    rng: Pcg64,
    /// The top-left and bottom-right corners of the tone rect in mm.
    origin: [f64; 2],
    far: [f64; 2],
    spacing: f64,
    /// The darts not used yet.
    darts: std::vec::IntoIter<[f64; 2]>,
    /// The darts that have failed since the last one that placed a point.
    dart_fails: usize,
    cols: usize,
    rows: usize,
    /// Per bucket, the newest point in it, or [`NONE`].
    head: Vec<usize>,
    /// Per point, the next older point in its bucket, or [`NONE`].
    next: Vec<usize>,
    /// The accepted points, in the order of acceptance.
    points: Vec<[f64; 2]>,
    /// The radius of each point.
    radii: Vec<f64>,
    /// Indices of the points that may still have room round them.
    active: Vec<usize>,
    /// The rectangle's area in squares of the spacing; a packing holds at most 1.155 times as many points.
    expected: f64,
    frame_every: usize,
}

impl PoissonRun {
    /// The radius at `p`, or `None` where there is too little ink for a point.
    fn radius(&self, p: [f64; 2]) -> Option<f64> {
        let ink = f64::from(self.tone.sample(p));
        // Written as `>=` so that NaN fails.
        (ink >= MIN_INK).then(|| (self.spacing / ink.sqrt()).min(MAX_RADIUS * self.spacing))
    }

    /// The bucket column and row of `p`.
    fn bucket(&self, p: [f64; 2]) -> (usize, usize) {
        let at = |v: f64, lo: f64, n: usize| (((v - lo) / self.spacing) as usize).min(n - 1);
        (
            at(p[0], self.origin[0], self.cols),
            at(p[1], self.origin[1], self.rows),
        )
    }

    /// Whether a stored point lies within `r` of `q`.
    fn crowded(&self, q: [f64; 2], r: f64) -> bool {
        let (col, row) = self.bucket(q);
        let reach = (r / self.spacing).ceil() as usize;
        for j in row.saturating_sub(reach)..=(row + reach).min(self.rows - 1) {
            for i in col.saturating_sub(reach)..=(col + reach).min(self.cols - 1) {
                let mut k = self.head[j * self.cols + i];
                while k != NONE {
                    let (dx, dy) = (self.points[k][0] - q[0], self.points[k][1] - q[1]);
                    if dx * dx + dy * dy < r * r {
                        return true;
                    }
                    k = self.next[k];
                }
            }
        }
        false
    }

    /// Store `q` if it lies in the rect, has ink, and has no stored point within its radius, and
    /// make it active. Returns whether it did.
    fn place(&mut self, q: [f64; 2]) -> bool {
        let inside = self.origin[0] <= q[0]
            && q[0] <= self.far[0]
            && self.origin[1] <= q[1]
            && q[1] <= self.far[1];
        if !inside {
            return false;
        }
        let Some(r) = self.radius(q) else {
            return false;
        };
        if self.crowded(q, r) {
            return false;
        }
        let (col, row) = self.bucket(q);
        let slot = &mut self.head[row * self.cols + col];
        self.next.push(*slot);
        *slot = self.points.len();
        self.active.push(self.points.len());
        self.points.push(q);
        self.radii.push(r);
        true
    }

    /// Try candidates round a random active point until one is placed; drop the point after 30
    /// failures.
    fn grow(&mut self) {
        let a = self.rng.random_range(0..self.active.len());
        let (p, r) = (self.points[self.active[a]], self.radii[self.active[a]]);
        for _ in 0..TRIES {
            let u = self.rng.random::<f64>();
            let v = self.rng.random::<f64>();
            let (dist, angle) = (r * (1.0 + u), TAU * v);
            let q = [
                p[0] + dist * math::cos(angle),
                p[1] + dist * math::sin(angle),
            ];
            if self.place(q) {
                return;
            }
        }
        self.active.swap_remove(a);
    }

    /// Place the next dart that fits, if 30 in a row have not failed. Returns whether one was
    /// placed.
    fn seed(&mut self) -> bool {
        while self.dart_fails < TRIES {
            let Some(dart) = self.darts.next() else {
                return false;
            };
            if self.place(dart) {
                self.dart_fails = 0;
                return true;
            }
            self.dart_fails += 1;
        }
        false
    }
}

impl StageRun for PoissonRun {
    fn step(&mut self) -> Result<Step, StageError> {
        let start = self.points.len();
        while self.points.len() - start < self.frame_every {
            if !self.active.is_empty() {
                self.grow();
            } else if !self.seed() {
                let points = std::mem::take(&mut self.points);
                return Ok(Step::Done(Artifact::Points(Arc::new(Points(points, None)))));
            }
        }
        let n = self.points.len();
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress: (n as f64 / self.expected).min(1.0) as f32,
            label: format!("{n} points"),
            points: Some(Points(self.points.clone(), None)),
            paths: None,
            pen: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Frame;
    use crate::test_util::{assert_inside, bounds, run_stage, tone_map};
    use serde_json::json;
    use std::collections::HashMap;

    fn run_poisson(tone: ToneMap, patch: Value) -> (Vec<Frame>, Arc<Points>) {
        match run_stage(&PoissonPlacer, vec![Artifact::Tone(Arc::new(tone))], patch) {
            (frames, Artifact::Points(p)) => (frames, p),
            (_, a) => panic!("expected points, got {}", a.kind().name()),
        }
    }

    /// Panic unless every point lies in the tone map's rect.
    fn assert_in_rect(pts: &Points, tone: &ToneMap) {
        let mut paths = crate::types::Paths::new();
        for p in &pts.0 {
            paths.push(&[*p]);
        }
        assert_inside(&paths, tone.rect_mm());
    }

    #[test]
    fn settings_schema() {
        let s = PoissonPlacer.schema();
        assert_eq!(bounds(&s, "spacing_mm"), (0.2, 20.0, 1.0));
        assert_eq!(s["properties"]["spacing_mm"]["x-step"], 0.1);
        assert_eq!(s["properties"]["spacing_mm"]["x-unit"], "mm");
        assert_eq!(
            s["properties"]["spacing_mm"]["description"],
            "Distance between points over full ink; lighter areas space them further apart."
        );
        assert_eq!(bounds(&s, "seed"), (0.0, 9999.0, 1.0));
        assert_eq!(
            s["properties"]["seed"]["description"],
            "Where the packing starts."
        );
        assert_eq!(
            (
                PoissonPlacer.name(),
                PoissonPlacer.label(),
                PoissonPlacer.version()
            ),
            ("poisson", "Poisson disk", 2)
        );
        assert_eq!(
            (PoissonPlacer.requires(), PoissonPlacer.provides()),
            (&[Kind::Tone][..], Kind::Points)
        );
    }

    #[test]
    fn no_two_points_closer_than_their_radii() {
        let spacing = 2.0;
        let tone = tone_map(40, 40, |x, y| ((x * 7 + y * 13) % 10) as f32 / 9.0);
        let (_, pts) = run_poisson(tone.clone(), json!({ "spacing_mm": spacing }));
        assert!(pts.0.len() > 100, "{} points", pts.0.len());
        let r = |p: [f64; 2]| (spacing / f64::from(tone.sample(p)).sqrt()).min(20.0 * spacing);
        for (i, p) in pts.0.iter().enumerate() {
            for q in &pts.0[i + 1..] {
                let d = ((p[0] - q[0]) * (p[0] - q[0]) + (p[1] - q[1]) * (p[1] - q[1])).sqrt();
                assert!(
                    d >= r(*p).min(r(*q)) - 1e-9,
                    "{p:?} and {q:?} are {d} apart"
                );
            }
        }
        assert_in_rect(&pts, &tone);
    }

    #[test]
    fn black_packs_at_the_spacing() {
        // 40 mm square at spacing 2: hexagonal packing holds 2 * 1600 / (sqrt(3) * 4) = 462.
        let tone = tone_map(20, 20, |_, _| 1.0);
        let (_, pts) = run_poisson(tone.clone(), json!({"spacing_mm": 2.0}));
        assert!((200..=462).contains(&pts.0.len()), "{} points", pts.0.len());
        assert_in_rect(&pts, &tone);
    }

    #[test]
    fn a_flat_black_page_at_the_style_spacing_fills_without_much_waste() {
        // An A4 page less margins at spacing 3: 5869 cells' worth, hexagonal bound 1.155 times that.
        let tone = tone_map(95, 139, |_, _| 1.0);
        let (_, pts) = run_poisson(tone.clone(), json!({"spacing_mm": 3.0}));
        let expected = 190.0 * 278.0 / 9.0;
        let n = pts.0.len() as f64;
        assert!(n > 0.5 * expected && n < 1.155 * expected, "{n} points");
        assert_in_rect(&pts, &tone);
    }

    #[test]
    fn lighter_areas_get_fewer_points() {
        // Ink 0.25 doubles the radius, so a point there takes four times the area.
        let tone = tone_map(40, 20, |x, _| if x < 20 { 1.0 } else { 0.25 });
        let (_, pts) = run_poisson(tone, json!({"spacing_mm": 2.0}));
        let dark = pts.0.iter().filter(|p| p[0] < 50.0).count();
        let light = pts.0.len() - dark;
        assert!(dark > 2 * light, "{dark} dark, {light} light");
        assert!(light > 0);
    }

    #[test]
    fn a_small_dark_spot_on_white_gets_points() {
        let tone = tone_map(50, 50, |x, y| {
            if (24..26).contains(&x) && (24..26).contains(&y) {
                1.0
            } else {
                0.0
            }
        });
        let (_, pts) = run_poisson(tone, json!({"spacing_mm": 1.0}));
        // The spot spans 58..62 mm; bilinear sampling spreads its ink up to a pixel past it.
        assert!(!pts.0.is_empty());
        for p in &pts.0 {
            assert!(
                (56.5..=63.5).contains(&p[0]) && (56.5..=63.5).contains(&p[1]),
                "{p:?}"
            );
        }
    }

    #[test]
    fn white_gives_no_points() {
        let (_, pts) = run_poisson(tone_map(20, 20, |_, _| 0.0), json!({}));
        assert!(pts.0.is_empty());
        // Ink under 1/400 takes no point either.
        let (_, pts) = run_poisson(tone_map(20, 20, |_, _| 0.002), json!({}));
        assert!(pts.0.is_empty());
    }

    #[test]
    fn frames_come_every_thirtieth_of_the_points_expected() {
        // 400 cells expected at spacing 2, so a frame every 13 points.
        let tone = tone_map(20, 20, |_, _| 1.0);
        let (frames, pts) = run_poisson(tone, json!({"spacing_mm": 2.0}));
        assert_eq!(frames.len(), pts.0.len() / 13);
        for (k, f) in frames.iter().enumerate() {
            let held = &f.points.as_ref().unwrap().0;
            assert_eq!(held[..], pts.0[..13 * (k + 1)], "frame {k}");
            assert_eq!(f.label, format!("{} points", 13 * (k + 1)));
            assert!(f.progress > 0.0 && f.progress <= 1.0);
        }
        // Fewer than 30 expected: a frame for every point.
        let (frames, pts) = run_poisson(tone_map(2, 2, |_, _| 1.0), json!({"spacing_mm": 2.0}));
        assert_eq!(frames.len(), pts.0.len());
    }

    #[test]
    fn the_seed_decides_the_packing() {
        let flat = || tone_map(20, 20, |_, _| 1.0);
        let (_, a) = run_poisson(flat(), json!({"seed": 7}));
        let (_, b) = run_poisson(flat(), json!({"seed": 7}));
        let (_, c) = run_poisson(flat(), json!({"seed": 8}));
        assert_eq!(a.0, b.0);
        assert_ne!(a.0, c.0);
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"spacing_mm": 0.1}),
            json!({"spacing_mm": 20.5}),
            json!({"seed": 10000}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&PoissonPlacer.defaults(), &patch);
            let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let inputs = Inputs::new(HashMap::from([(Kind::Tone, tone)]));
            let ctx = RunContext::new(Arc::default());
            let err = PoissonPlacer.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
}
