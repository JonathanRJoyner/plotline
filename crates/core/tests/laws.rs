//! Laws that every style and every setting keeps, checked on random inputs.
//!
//! Nothing here names a style or a setting. The inputs come from the registry and from each
//! stage's settings schema, so a new style or setting is covered with no edit to this file. A case
//! is a small image built in code (a two-colour gradient under discs, boxes and noise, or one of
//! the crate's synthetic pictures), a page and margin, one to three pens, a style, and a value for
//! each of its settings, Pen use included, from the schema's range: the default, either end, or
//! anything between. proptest shrinks a failing case and prints it. The look law is the one
//! exception: its cases are a style, one of its looks and one of three test images at the size of
//! the regression gallery's.
//!
//! Each law runs a fixed number of cases from a fixed seed, so a pull request's run is the same
//! every time. `PLOTLINE_LAWS_SCALE` multiplies the case counts and raises the speed caps, and
//! `PLOTLINE_LAWS_SEED` picks another seed; the weekly workflow sets both.
//!
//! A feature with a contract adds its law here (see CONTRIBUTING.md).

use std::fmt;
use std::sync::{Arc, Mutex};

use plotline_core::cache::StageCache;
use plotline_core::coverage::{cell_size, Coverage};
use plotline_core::export::hpgl::MAX_UNITS_PER_MM;
use plotline_core::fit::{places_points, Density, MAX_TRIALS, TOLERANCE};
use plotline_core::lint::Severity;
use plotline_core::machine::{Area, Machine, PenLift};
use plotline_core::math::{exp, ln};
use plotline_core::optimize::{CLOSE, LONGEST, SLOWER, TRIALS};
use plotline_core::pens::INKS;
use plotline_core::preview::{render_png_shrunk, render_rgba};
use plotline_core::raster::simplify;
use plotline_core::registry::{get_style, list_styles};
use plotline_core::runner::Shared;
use plotline_core::settings::merge;
use plotline_core::stage::{Artifact, Inputs, Kind, RunContext, Stage, Step};
use plotline_core::stages::join::join;
use plotline_core::stages::split::SplitStage;
use plotline_core::stages::tone::{flatten, guided, ToneStage};
use plotline_core::stats::{self, DEFAULT_ACCEL_MM_S2};
use plotline_core::style::Style;
use plotline_core::synthetic::{flat_noise, gradient, ramp, scene, still_life, strokes};
use plotline_core::tone_match;
use plotline_core::{
    Canvas, Drawing, Fit, Image, Paths, Pen, Run, RunState, Session, Speeds, ToneMap, Tones,
};
use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestRunner};
use serde_json::{json, Value};

/// Upper bounds on the settings whose cost grows fastest, so that a case runs in well under a
/// second. They narrow a range but never leave a setting out; a scaled-up run multiplies them by
/// four. A new setting needs a cap only if it makes the cases slow.
const SPEED_CAPS: &[(&str, &str, f64)] = &[
    ("tone", "resolution", 160.0),
    ("voronoi", "count", 2000.0),
    ("voronoi", "iterations", 20.0),
    ("sketch", "max_lines", 2000.0),
    ("sketch", "candidates", 64.0),
];

/// The seed of the pull request run. Every law must hold for any seed, since a new setting shifts
/// every case a seed draws; see docs/decisions/2026-10-16-property-law-fixes.md.
const SEED: u64 = 0x706c_6f74_6c69_6e65;

/// How many times more cases to run than a pull request does, from `PLOTLINE_LAWS_SCALE`.
fn scale() -> u32 {
    let v = std::env::var("PLOTLINE_LAWS_SCALE").ok();
    v.and_then(|v| v.parse().ok()).unwrap_or(1).max(1)
}

/// The cap on `stage.key`, if it has one.
fn cap(stage: &str, key: &str) -> Option<f64> {
    let raise = if scale() > 1 { 4.0 } else { 1.0 };
    (SPEED_CAPS.iter())
        .find(|(s, k, _)| (*s, *k) == (stage, key))
        .map(|(_, _, cap)| cap * raise)
}

/// Run `law` over `cases` cases of `strategy`, times the scale, and fail with the shrunk case.
fn check<S: Strategy>(
    name: &str,
    cases: u32,
    strategy: S,
    law: impl Fn(S::Value) -> Result<(), TestCaseError>,
) {
    let seed = std::env::var("PLOTLINE_LAWS_SEED").ok();
    let seed = seed.and_then(|s| s.parse().ok()).unwrap_or(SEED);
    let config = Config {
        cases: cases * scale(),
        rng_seed: RngSeed::Fixed(seed),
        failure_persistence: None,
        max_shrink_iters: 400,
        ..Config::default()
    };
    if let Err(e) = TestRunner::new(config).run(&strategy, law) {
        panic!("law {name:?} broken (PLOTLINE_LAWS_SEED={seed}): {e}");
    }
}

/// A JSON value that prints as JSON, so a shrunk case reads well.
#[derive(Clone)]
struct Json(Value);

impl fmt::Debug for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An image built in code, described by its parameters so that a shrunk case stays short.
#[derive(Clone, Debug)]
enum Picture {
    /// A gradient from `from` to `to` along `along`, discs and boxes over it, then noise of
    /// half-width `noise` in every channel.
    Drawn {
        w: u32,
        h: u32,
        from: [f32; 3],
        to: [f32; 3],
        along: [f32; 2],
        shapes: Vec<Shape>,
        noise: f32,
        seed: u64,
    },
    /// Line art.
    Strokes {
        w: u32,
        h: u32,
        ink: f32,
        paper: f32,
    },
    /// A ramp, often a narrow one.
    Ramp { w: u32, h: u32, from: f32, to: f32 },
    /// A flat grey with noise.
    Noisy {
        w: u32,
        h: u32,
        value: f32,
        sigma: f32,
        seed: u64,
    },
    /// The colour landscape.
    Scene { w: u32, h: u32 },
}

/// A disc or a box of one colour; the centre and radius are shares of the image's size.
#[derive(Clone, Debug)]
struct Shape {
    disc: bool,
    centre: [f32; 2],
    radius: f32,
    colour: [f32; 3],
}

/// A number in -1..1 from `seed` and `i` (splitmix64; S. Vigna, "Further scramblings of
/// Marsaglia's xorshift generators", 2017).
fn hash(seed: u64, i: u64) -> f32 {
    let mut z = seed ^ i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

impl Picture {
    fn image(&self) -> Image {
        match self {
            Picture::Drawn {
                w,
                h,
                from,
                to,
                along,
                shapes,
                noise,
                seed,
            } => {
                let mut rgb = Vec::with_capacity((w * h * 3) as usize);
                for y in 0..*h {
                    for x in 0..*w {
                        let u = (x as f32 + 0.5) / *w as f32;
                        let v = (y as f32 + 0.5) / *h as f32;
                        let t = (along[0] * (u - 0.5) + along[1] * (v - 0.5) + 0.5).clamp(0.0, 1.0);
                        let mut c = [0, 1, 2].map(|k| from[k] + (to[k] - from[k]) * t);
                        for s in shapes {
                            let (dx, dy) = (u - s.centre[0], v - s.centre[1]);
                            let inside = if s.disc {
                                dx * dx + dy * dy <= s.radius * s.radius
                            } else {
                                dx.abs() <= s.radius && dy.abs() <= s.radius
                            };
                            if inside {
                                c = s.colour;
                            }
                        }
                        let i = u64::from(y * w + x) * 3;
                        for (k, c) in c.iter().enumerate() {
                            rgb.push((c + noise * hash(*seed, i + k as u64)).clamp(0.0, 1.0));
                        }
                    }
                }
                Image {
                    width: *w,
                    height: *h,
                    rgb,
                }
            }
            Picture::Strokes { w, h, ink, paper } => strokes(*w, *h, *ink, *paper),
            Picture::Ramp { w, h, from, to } => ramp(*w, *h, *from, *to),
            Picture::Noisy {
                w,
                h,
                value,
                sigma,
                seed,
            } => flat_noise(*w, *h, *value, *sigma, *seed),
            Picture::Scene { w, h } => scene(*w, *h),
        }
    }
}

fn picture() -> impl Strategy<Value = Picture> {
    let side = || 1u32..=48;
    let unit = || 0.0f32..=1.0;
    let colour = || [unit(), unit(), unit()];
    let shape = (any::<bool>(), [unit(), unit()], 0.0f32..0.5, colour()).prop_map(
        |(disc, centre, radius, colour)| Shape {
            disc,
            centre,
            radius,
            colour,
        },
    );
    let drawn = (
        (side(), side()),
        (colour(), colour()),
        [-1.0f32..=1.0, -1.0f32..=1.0],
        prop::collection::vec(shape, 0..4),
        prop_oneof![Just(0.0f32), 0.0f32..0.3],
        any::<u64>(),
    )
        .prop_map(
            |((w, h), (from, to), along, shapes, noise, seed)| Picture::Drawn {
                w,
                h,
                from,
                to,
                along,
                shapes,
                noise,
                seed,
            },
        );
    prop_oneof![
        6 => drawn,
        1 => (side(), side(), unit(), unit())
            .prop_map(|(w, h, ink, paper)| Picture::Strokes { w, h, ink, paper }),
        1 => (side(), side(), unit(), unit())
            .prop_map(|(w, h, from, to)| Picture::Ramp { w, h, from, to }),
        1 => (side(), side(), unit(), 0.0f32..0.2, any::<u64>())
            .prop_map(|(w, h, value, sigma, seed)| Picture::Noisy { w, h, value, sigma, seed }),
        1 => (side(), side()).prop_map(|(w, h)| Picture::Scene { w, h }),
    ]
}

/// A page in mm, its margin as a share of the most it could be, whether the image fills it, and
/// the shares of the image cut from its left, top, right and bottom.
#[derive(Clone, Debug)]
struct Page {
    width: f64,
    height: f64,
    margin: f64,
    fill: bool,
    crop: [f64; 4],
}

impl Page {
    fn canvas(&self) -> Canvas {
        let margin = self.margin * self.width.min(self.height) / 2.0;
        let fit = if self.fill { Fit::Fill } else { Fit::Fit };
        let canvas = Canvas::new(self.width, self.height, margin, fit);
        canvas
            .and_then(|c| c.with_crop(self.crop))
            .expect("the margin leaves an area")
    }
}

fn page() -> impl Strategy<Value = Page> {
    (20.0..160.0f64, 20.0..160.0f64, 0.0..0.9f64, any::<bool>()).prop_map(
        |(width, height, margin, fill)| Page {
            width,
            height,
            margin,
            fill,
            crop: [0.0; 4],
        },
    )
}

/// The crop of a case, drawn from its `cancel_at` instead of from a strategy of its own, so that
/// every case the other laws drew before crops existed is still drawn: one case in four is
/// cropped, each side by up to the most it may be, and a shrunk case has none.
fn crop_of(cancel_at: usize) -> [f64; 4] {
    if cancel_at % 4 != 3 {
        return [0.0; 4];
    }
    [0, 1, 2, 3].map(|i| f64::from((hash(cancel_at as u64, i) + 1.0) / 2.0) * 0.45)
}

/// One to three pens as width in mm and colour, thin ones included.
fn pens() -> impl Strategy<Value = Vec<(f64, [u8; 3])>> {
    let width = prop_oneof![0.03..0.1f64, 0.1..2.0f64];
    prop::collection::vec((width, any::<[u8; 3]>()), 1..=3)
}

/// The choices of an enum setting, written in place or through a `$ref` into `$defs`.
fn choices(prop: &Value, defs: &Value) -> Option<Vec<Value>> {
    let target = match prop["$ref"].as_str().or(prop["allOf"][0]["$ref"].as_str()) {
        Some(r) => &defs[r.rsplit('/').next().unwrap()],
        None => prop,
    };
    if let Some(e) = target["enum"].as_array() {
        return Some(e.clone());
    }
    let variants = target["oneOf"].as_array()?;
    Some(variants.iter().map(|v| v["const"].clone()).collect())
}

/// Values of setting `stage.key` from its schema `prop`: any choice of an enum, either boolean,
/// and for a number its default, either end of its range or anything between, under any cap.
fn value(stage: &str, key: &str, prop: &Value, defs: &Value) -> BoxedStrategy<Value> {
    if let Some(choices) = choices(prop, defs) {
        return select(choices).boxed();
    }
    let range = || {
        let lo = prop["minimum"].as_f64();
        let hi = prop["maximum"].as_f64();
        let (lo, hi) = lo
            .zip(hi)
            .unwrap_or_else(|| panic!("{stage}.{key} has no range"));
        let hi = cap(stage, key).map_or(hi, |c| hi.min(c));
        let d = prop["default"].as_f64().expect("a numeric default");
        (lo, hi, d.clamp(lo, hi))
    };
    match prop["type"].as_str() {
        Some("boolean") => any::<bool>().prop_map(Value::from).boxed(),
        Some("integer") => {
            let (lo, hi, d) = range();
            let (lo, hi, d) = (lo as i64, hi as i64, d as i64);
            prop_oneof![1 => Just(d), 1 => Just(lo), 1 => Just(hi), 3 => lo..=hi]
                .prop_map(Value::from)
                .boxed()
        }
        Some("number") => {
            let (lo, hi, d) = range();
            prop_oneof![1 => Just(d), 1 => Just(lo), 1 => Just(hi), 3 => lo..=hi]
                .prop_map(Value::from)
                .boxed()
        }
        // A list of number lists, such as the focus points: none to three of them, each number
        // in the range the schema gives the items' items.
        Some("array") => {
            let item = &prop["items"];
            let n = item["minItems"].as_u64().expect("fixed-length items") as usize;
            let lo = item["items"]["minimum"].as_f64().expect("a range");
            let hi = item["items"]["maximum"].as_f64().expect("a range");
            let list = prop::collection::vec(lo..=hi, n);
            prop::collection::vec(list, 0..=3)
                .prop_map(Value::from)
                .boxed()
        }
        other => panic!("{stage}.{key} has type {other:?}; teach laws.rs to draw it"),
    }
}

/// Every setting of `style` as `(stage, key, values)`, from the style's schema.
fn settings_of(style: &Style) -> Vec<(String, String, BoxedStrategy<Value>)> {
    let schema = style.schema();
    let mut out = Vec::new();
    for stage in schema["stages"].as_array().unwrap() {
        let (name, s) = (stage["name"].as_str().unwrap(), &stage["schema"]);
        for (key, prop) in s["properties"].as_object().into_iter().flatten() {
            let values = value(name, key, prop, &s["$defs"]);
            out.push((name.to_string(), key.clone(), values));
        }
    }
    out
}

/// One value per setting, gathered as `{stage: {key: value}}`.
fn gather(parts: Vec<BoxedStrategy<(String, String, Value)>>) -> BoxedStrategy<Json> {
    parts
        .prop_map(|parts| {
            let mut out = json!({});
            for (stage, key, v) in parts {
                out[&stage][&key] = v;
            }
            Json(out)
        })
        .boxed()
}

/// A listed style's name and a value for each of its settings.
fn styled(names: Vec<String>) -> impl Strategy<Value = (String, Json)> {
    select(names).prop_flat_map(|name| {
        let style = get_style(&name).unwrap();
        let parts = (settings_of(&style).into_iter())
            .map(|(stage, key, values)| {
                values
                    .prop_map(move |v| (stage.clone(), key.clone(), v))
                    .boxed()
            })
            .collect();
        (Just(name), gather(parts))
    })
}

fn all_styles() -> Vec<String> {
    list_styles().iter().map(|s| s.name.clone()).collect()
}

/// The styles with a setting marked `x-density`, which the budget solver fits.
fn dense_styles() -> Vec<String> {
    (list_styles().iter())
        .filter(|s| Density::of(s).is_some())
        .map(|s| s.name.clone())
        .collect()
}

#[derive(Clone, Debug)]
struct Case {
    style: String,
    settings: Json,
    picture: Picture,
    page: Page,
    pens: Vec<(f64, [u8; 3])>,
    /// Where to cancel a run, as a count of frames taken modulo the run's frames plus one.
    cancel_at: usize,
}

fn case(names: Vec<String>) -> impl Strategy<Value = Case> {
    (styled(names), picture(), page(), pens(), any::<usize>()).prop_map(
        |((style, settings), picture, page, pens, cancel_at)| Case {
            style,
            settings,
            picture,
            page: Page {
                crop: crop_of(cancel_at),
                ..page
            },
            pens,
            cancel_at,
        },
    )
}

/// The pens of widths and colours `pens`.
fn pens_of(pens: &[(f64, [u8; 3])]) -> Vec<Pen> {
    (pens.iter().enumerate())
        .map(|(i, (width, [r, g, b]))| {
            let colour = format!("#{r:02x}{g:02x}{b:02x}");
            Pen::new(&format!("Pen {i}"), *width, &colour).unwrap()
        })
        .collect()
}

impl Case {
    fn pens(&self) -> Vec<Pen> {
        pens_of(&self.pens)
    }

    /// A fresh session, with its own cache, set up for this case.
    fn session(&self) -> Session {
        let mut s = Session::new(256 << 20);
        s.set_image(self.picture.image());
        s.set_canvas(self.page.canvas());
        s.set_pens(self.pens()).unwrap();
        s.set_style(&self.style).unwrap();
        for (stage, values) in self.settings.0.as_object().unwrap() {
            s.update(stage, values).unwrap();
        }
        s
    }
}

/// Take every frame of `run`, checking that each coordinate a frame shows is finite, and return
/// how many there were.
fn drain(run: &mut Run) -> Result<usize, TestCaseError> {
    let mut n = 0;
    while let Some(f) = run.next_frame() {
        n += 1;
        let points = f.points.iter().flat_map(|p| p.0.iter());
        let paths = f.paths.iter().flat_map(|p| p.coords().iter());
        for p in points.chain(paths) {
            let stage = &f.stage;
            prop_assert!(p.iter().all(|v| v.is_finite()), "{stage} frame {n}: {p:?}");
        }
    }
    Ok(n)
}

/// Run `s` to the end and return the frame count.
fn run(s: &mut Session) -> Result<usize, TestCaseError> {
    drain(&mut s.start().unwrap())
}

/// A hash of every byte of `d`: the page, each layer's pen and paths, and the tone map.
fn digest(d: &Drawing) -> String {
    let mut h = blake3::Hasher::new();
    h.update(serde_json::to_string(&d.canvas).unwrap().as_bytes());
    for layer in &d.layers {
        h.update(serde_json::to_string(&layer.pen).unwrap().as_bytes());
        for o in layer.paths.offsets() {
            h.update(&o.to_le_bytes());
        }
        for v in layer.paths.coords().iter().flatten() {
            h.update(&v.to_bits().to_le_bytes());
        }
    }
    if let Some(t) = &d.tone {
        for v in [t.width, t.height]
            .into_iter()
            .chain(t.ink.iter().map(|v| v.to_bits()))
        {
            h.update(&v.to_le_bytes());
        }
        for v in [t.mm_per_px, t.origin_mm[0], t.origin_mm[1]] {
            h.update(&v.to_bits().to_le_bytes());
        }
    }
    h.finalize().to_hex().to_string()
}

/// Everything a run leaves in its slot: the state, the error and a hash of the drawing.
type Outcome = (RunState, Option<String>, Option<String>);

fn outcome(state: RunState, error: Option<String>, d: Option<&Drawing>) -> Outcome {
    (state, error, d.map(digest))
}

/// What a run left in the session.
fn left(s: &Session) -> Outcome {
    outcome(s.state(), s.error(), s.drawing().as_deref())
}

/// A finished run ends done or with a message, never in a panic, which proptest catches; its
/// coordinates are finite and on the drawing area, and no path is shorter than the order stage's
/// minimum length.
fn finished(s: &Session) -> Result<(), TestCaseError> {
    match s.state() {
        RunState::Done => {}
        RunState::Error => {
            let e = s.error().unwrap_or_default();
            prop_assert!(!e.trim().is_empty(), "a failed run has no message");
        }
        other => prop_assert!(false, "the run ended {other:?}"),
    }
    let Some(d) = s.drawing() else { return Ok(()) };
    let [x, y, w, h] = d.canvas.drawing_rect();
    let min = s.settings()["order"]["min_length_mm"].as_f64();
    for (i, layer) in d.layers.iter().enumerate() {
        for path in layer.paths.iter() {
            prop_assert!(!path.is_empty(), "layer {i} has an empty path");
            for p in path {
                let inside = x - 1e-9 <= p[0]
                    && p[0] <= x + w + 1e-9
                    && y - 1e-9 <= p[1]
                    && p[1] <= y + h + 1e-9;
                prop_assert!(inside, "layer {i}: {p:?} is off {:?}", [x, y, w, h]);
            }
            if let Some(min) = min {
                let length = Paths::path_length(path);
                prop_assert!(length >= min, "layer {i}: a path of {length} mm < {min}");
            }
        }
    }
    Ok(())
}

#[test]
fn every_run_keeps_the_run_laws() {
    check("run", 24, case(all_styles()), |c| {
        let mut s = c.session();
        let frames = run(&mut s)?;
        finished(&s)?;

        // The same inputs give the same bytes, from a cold cache.
        let mut again = c.session();
        run(&mut again)?;
        prop_assert!(left(&s) == left(&again), "a second run differs");

        // A cancel after any frame ends the run, and the slot shows it with no drawing.
        let k = c.cancel_at % (frames + 1);
        let mut cancelled = c.session();
        let mut r = cancelled.start().unwrap();
        for i in 0..k {
            prop_assert!(
                r.next_frame().is_some(),
                "the run ended early, at frame {i}"
            );
        }
        r.cancel();
        prop_assert!(r.next_frame().is_none(), "a frame after the cancel at {k}");
        prop_assert_eq!(r.state(), RunState::Cancelled);
        prop_assert_eq!(cancelled.state(), RunState::Cancelled);
        prop_assert!(cancelled.drawing().is_none());

        // In one pen mode the run matches the style without its split, which has no lanes.
        if s.settings()["split"]["mode"] == "one_pen" {
            let style = get_style(&c.style).unwrap();
            let stages = (style.stages.iter())
                .filter(|st| st.name() != "split")
                .cloned()
                .collect();
            let plain = Style::new(&style.name, &style.label, &style.description, stages).unwrap();
            let slot = Arc::new(Mutex::new(Shared::default()));
            let roots = [
                Artifact::Image(Arc::new(c.picture.image())),
                Artifact::Canvas(Arc::new(c.page.canvas())),
                Artifact::Pens(Arc::new(c.pens())),
            ];
            let cache = Arc::new(Mutex::new(StageCache::new(256 << 20)));
            let (settings, cancel) = (s.settings().clone(), Arc::default());
            let mut r = Run::new(
                Arc::new(plain),
                settings,
                roots,
                cache,
                slot.clone(),
                cancel,
                0,
            );
            drain(&mut r)?;
            let slot = slot.lock().unwrap();
            let plain = outcome(slot.state, slot.error.clone(), slot.drawing.as_deref());
            prop_assert!(plain == left(&s), "the drawing differs without the split");
        }
        Ok(())
    });
}

#[test]
fn a_crop_of_zeros_leaves_every_drawing_unchanged() {
    check("zero crop", 24, case(all_styles()), |c| {
        let mut plain = c.clone();
        plain.page.crop = [0.0; 4];
        let canvas = plain.page.canvas();
        let mut json = serde_json::to_value(&canvas).unwrap();
        prop_assert!(
            json.get("crop").is_none(),
            "a crop of zeros is written: {json}"
        );
        // The same page as a file that spells the zeros out would give it.
        json["crop"] = json!([0.0, 0.0, 0.0, 0.0]);
        let spelled = serde_json::from_value::<Canvas>(json).unwrap();
        let mut a = plain.session();
        run(&mut a)?;
        let mut b = plain.session();
        b.set_canvas(spelled.with_crop([0.0; 4]).unwrap());
        run(&mut b)?;
        prop_assert!(left(&a) == left(&b), "a crop of zeros changes the drawing");
        Ok(())
    });
}

/// Values at the ends of a setting's range: either end or the default for a number, any value of
/// a boolean or an enum.
fn corner(stage: &str, key: &str, prop: &Value, defs: &Value) -> BoxedStrategy<Value> {
    let (Some(lo), Some(hi)) = (prop["minimum"].as_f64(), prop["maximum"].as_f64()) else {
        return value(stage, key, prop, defs);
    };
    let hi = cap(stage, key).map_or(hi, |c| hi.min(c));
    let ends = [lo, hi, prop["default"].as_f64().unwrap().clamp(lo, hi)];
    let ends = ends.map(|v| {
        if prop["type"] == "integer" {
            json!(v as i64)
        } else {
            json!(v)
        }
    });
    select(ends.to_vec()).boxed()
}

#[test]
fn a_run_at_the_ends_of_every_range_ends_with_a_drawing_or_a_message() {
    // Every number at an end of its range, the thinnest pens and the largest page: where size
    // budgets are passed, which must fail the run with a message.
    let strategy = (select(all_styles()), picture(), pens(), any::<bool>());
    let strategy = strategy.prop_flat_map(|(name, picture, pens, fill)| {
        let schema = get_style(&name).unwrap().schema();
        let mut parts = Vec::new();
        for stage in schema["stages"].as_array().unwrap() {
            let (st, sc) = (
                stage["name"].as_str().unwrap().to_string(),
                &stage["schema"],
            );
            for (key, prop) in sc["properties"].as_object().into_iter().flatten() {
                let (st, key2) = (st.clone(), key.clone());
                let values = corner(&st, key, prop, &sc["$defs"]);
                parts.push(
                    values
                        .prop_map(move |v| (st.clone(), key2.clone(), v))
                        .boxed(),
                );
            }
        }
        let pens: Vec<_> = pens.into_iter().map(|(_, colour)| (0.03, colour)).collect();
        let page = Page {
            width: 160.0,
            height: 160.0,
            margin: 0.0,
            fill,
            crop: [0.0; 4],
        };
        gather(parts).prop_map(move |settings| Case {
            style: name.clone(),
            settings,
            picture: picture.clone(),
            page: page.clone(),
            pens: pens.clone(),
            cancel_at: 0,
        })
    });
    check("ends", 48, strategy, |c| {
        let mut s = c.session();
        run(&mut s)?;
        finished(&s)
    });
}

/// Apply a suggestion's or a finding's patch as the front ends do: the style, then the settings,
/// then the page size or the crop with the rest of the page kept.
fn apply(s: &mut Session, patch: &Value) -> Result<(), TestCaseError> {
    if let Some(style) = patch["style"].as_str() {
        s.set_style(style)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
    }
    if let Some(pens) = patch["pens"].as_array() {
        let pen = |p: &Value| {
            let text = |k: &str| p[k].as_str().unwrap_or_default();
            Pen::new(
                text("name"),
                p["width_mm"].as_f64().unwrap_or(0.0),
                text("color"),
            )
        };
        let pens: Result<Vec<Pen>, _> = pens.iter().map(pen).collect();
        let pens = pens.map_err(|e| TestCaseError::fail(e.to_string()))?;
        s.set_pens(pens)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
    }
    for (stage, values) in patch["settings"].as_object().into_iter().flatten() {
        s.update(stage, values)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
    }
    if let Some(page) = patch["page"].as_object() {
        let c = s.canvas().clone();
        let size = |k: &str| page.get(k).and_then(Value::as_f64);
        let (w, h) = (size("width_mm"), size("height_mm"));
        let crop = match page.get("crop") {
            Some(v) => {
                serde_json::from_value(v.clone()).map_err(|e| TestCaseError::fail(e.to_string()))?
            }
            None => c.crop,
        };
        let canvas = Canvas::new(
            w.unwrap_or(c.width_mm),
            h.unwrap_or(c.height_mm),
            c.margin_mm,
            c.fit,
        );
        let canvas = canvas.and_then(|canvas| canvas.with_crop(crop));
        s.set_canvas(canvas.map_err(|e| TestCaseError::fail(e.to_string()))?);
    }
    Ok(())
}

#[test]
fn an_applied_suggestion_goes_away() {
    check("analysis", 256, case(all_styles()), |c| {
        let s = c.session();
        for g in s.analysis().unwrap().suggestions {
            let Some(patch) = &g.patch else { continue };
            let mut t = c.session();
            apply(&mut t, patch)?;
            let after = t.analysis().unwrap().suggestions;
            prop_assert!(
                after.iter().all(|h| h.kind != g.kind),
                "{} stays after its patch {patch}",
                g.kind
            );
        }
        Ok(())
    });
}

/// One to six library pens, each in a listed ink's colour or in any colour.
fn library() -> impl Strategy<Value = Vec<Pen>> {
    let colour = prop_oneof![
        select(INKS.to_vec()).prop_map(|(_, hex)| hex.to_string()),
        any::<[u8; 3]>().prop_map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}")),
    ];
    prop::collection::vec((0.1..2.0f64, colour), 1..=6).prop_map(|pens| {
        pens.into_iter()
            .enumerate()
            .map(|(i, (width, colour))| Pen::new(&format!("P{i}"), width, &colour).unwrap())
            .collect()
    })
}

#[test]
fn a_library_palette_uses_owned_pens() {
    // With a pen library the colour suggestion offers only library pens, and every ink it says is
    // worth adding is one the library lacks that gains a just noticeable difference (2.3 ΔE*ab).
    check(
        "palette library",
        64,
        (picture(), library()),
        |(picture, library)| {
            let mut s = Session::new(64 << 20);
            s.set_pen_library(library.clone());
            s.set_image(picture.image());
            let analysis = s.analysis().unwrap();
            let stats = &analysis.stats;
            let owns = |hex: &str| library.iter().any(|p| p.color.eq_ignore_ascii_case(hex));
            prop_assert!(stats.palette_additions.len() <= 3);
            for ink in &stats.palette_additions {
                prop_assert!(!owns(&ink.color), "{ink:?} is in the library");
                prop_assert!(
                    stats.palette_error - ink.error >= 2.3,
                    "{ink:?} gains too little on {}",
                    stats.palette_error
                );
            }
            if let Some(colour) = analysis.suggestions.iter().find(|g| g.kind == "colour") {
                let patch = colour.patch.as_ref().unwrap();
                for pen in patch["pens"].as_array().unwrap() {
                    prop_assert!(
                        library.iter().any(|p| json!(p) == *pen),
                        "{pen} is not a library pen"
                    );
                }
                apply(&mut s, patch)?;
                let after = s.analysis().unwrap().suggestions;
                prop_assert!(after.iter().all(|g| g.kind != "colour"));
            }
            Ok(())
        },
    );
}

#[test]
fn flattening_ignores_uniform_lighting() {
    // Dimming a picture evenly dims its paper just as much, so the flattened picture is the same.
    check(
        "flatten",
        256,
        (picture(), 1.0..=50.0f64, 0.25f32..=1.0),
        |(picture, window, gain)| {
            let image = picture.image();
            let (w, h) = (image.width as usize, image.height as usize);
            let lum = image.luminance();
            let dim: Vec<f32> = lum.iter().map(|v| v * gain).collect();
            let (plain, dimmed) = (flatten(&lum, w, h, window), flatten(&dim, w, h, window));
            for (i, (a, b)) in plain.iter().zip(&dimmed).enumerate() {
                prop_assert!(
                    (a - b).abs() <= 1e-4,
                    "pixel {i} of {w} x {h} is {a}, and {b} at gain {gain}"
                );
            }
            Ok(())
        },
    );
}

#[test]
fn smoothing_stays_within_the_tone_range() {
    // The output averages mixes of a pixel and window means, so it never leaves the picture's range.
    check("smooth", 256, (picture(), 1usize..=40), |(picture, r)| {
        let image = picture.image();
        let (w, h) = (image.width as usize, image.height as usize);
        let lum = image.luminance();
        let lo = lum.iter().copied().fold(f32::INFINITY, f32::min) - 1e-5;
        let hi = lum.iter().copied().fold(f32::NEG_INFINITY, f32::max) + 1e-5;
        for (i, q) in guided(&lum, w, h, r).iter().enumerate() {
            prop_assert!(
                (lo..=hi).contains(q),
                "pixel {i} of {w} x {h} is {q}, outside {lo} to {hi} at r {r}"
            );
        }
        Ok(())
    });
}

/// The lint findings whose number is a share inked, which rises as the drawing improves; every
/// other finding's number falls.
const RISING: &[&str] = &["empty", "dark"];

#[test]
fn an_applied_lint_fix_removes_or_shrinks_its_finding() {
    check("lint", 24, case(all_styles()), |c| {
        let mut s = c.session();
        run(&mut s)?;
        let Some(findings) = s.lint() else {
            return Ok(());
        };
        for f in findings {
            let Some(patch) = &f.patch else { continue };
            let mut t = c.session();
            apply(&mut t, patch)?;
            run(&mut t)?;
            let after = t.lint().unwrap_or_default();
            let Some(g) = after.iter().find(|g| g.code == f.code) else {
                continue;
            };
            let better = if RISING.contains(&f.code) {
                g.value > f.value
            } else {
                g.value < f.value
            };
            prop_assert!(
                better,
                "{} stays at {} after its patch {patch}, from {}",
                f.code,
                g.value,
                f.value
            );
        }
        Ok(())
    });
}

/// Whether two setting values are equal, numbers by value, as a style switch compares them.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

#[test]
fn a_switch_there_and_back_keeps_changed_values() {
    // Changes to style `a`, each sometimes set to style `b`'s default for the same setting.
    let strategy = (select(all_styles()), select(all_styles())).prop_flat_map(|(a, b)| {
        let theirs = get_style(&b).unwrap().defaults();
        let parts = (settings_of(&get_style(&a).unwrap()).into_iter())
            .map(|(stage, key, values)| {
                let values = match theirs.get(&stage).and_then(|d| d.get(&key)) {
                    Some(d) => prop_oneof![values, Just(d.clone())].boxed(),
                    None => values,
                };
                values
                    .prop_map(move |v| (stage.clone(), key.clone(), v))
                    .boxed()
            })
            .collect();
        (Just(a), Just(b), gather(parts))
    });
    check("switch", 512, strategy, |(a, b, changes)| {
        let mut s = Session::new(1 << 20);
        s.set_style(&a).unwrap();
        for (stage, values) in changes.0.as_object().unwrap() {
            s.update(stage, values).unwrap();
        }
        let before = s.settings().clone();
        let (ours, other) = (get_style(&a).unwrap(), get_style(&b).unwrap());
        s.set_style(&b).unwrap();
        s.set_style(&a).unwrap();
        let defaults = ours.defaults();
        for stage in ours
            .stages
            .iter()
            .filter(|st| other.stage_index(st.name()).is_some())
        {
            let name = stage.name();
            for (key, v) in before[name].as_object().unwrap() {
                if same(v, &defaults[name][key]) {
                    continue;
                }
                let now = &s.settings()[name][key];
                prop_assert!(same(now, v), "{name}.{key}: set to {v}, back as {now}");
            }
        }
        Ok(())
    });
}

/// `v` rounded as the budget solver rounds a trial: whole for an integer, else to 0.001.
fn grain(d: &Density, v: f64) -> f64 {
    let v = if d.integer {
        v.round()
    } else {
        (v * 1000.0).round() / 1000.0
    };
    v.clamp(d.min, d.max)
}

/// The metrics the budget solver can fit, from a finished run: minutes, paths, metres and the
/// points placed.
fn metrics(s: &Session, run: &Run) -> [f64; 4] {
    let stats = s.stats().unwrap();
    [
        stats.estimated_seconds / 60.0,
        stats.path_count as f64,
        stats.pen_down_mm / 1000.0,
        run.point_count().unwrap_or(0) as f64,
    ]
}

/// What the density setting `d` of `style` drives, from the `x-density-of` mark on its schema
/// property: `points`, `lines` or `length`. Unmarked, a style that places points is judged on its
/// points and any other on its length.
fn drives(style: &Style, d: &Density) -> String {
    let stage = style.stages.iter().find(|s| s.name() == d.stage).unwrap();
    match stage.schema()["properties"][&d.key]["x-density-of"].as_str() {
        Some(mark) => mark.to_string(),
        None if places_points(style) => "points".to_string(),
        None => "length".to_string(),
    }
}

#[test]
fn a_density_setting_moves_its_metrics_one_way() {
    let strategy = (case(dense_styles()), 0.0..1.0f64, 0.0..1.0f64);
    check("density", 24, strategy, |(c, p, q)| {
        let style = get_style(&c.style).unwrap();
        let d = Density::of(&style).unwrap();
        let hi = cap(&d.stage, &d.key).map_or(d.max, |cap| d.max.min(cap));
        let at = |share: f64| grain(&d, exp(ln(d.min) + share * (ln(hi) - ln(d.min))));
        let (lo, hi) = (at(p.min(q)), at(p.max(q)));
        // Values closer than a tenth differ by less than the drawings wobble: 1938 and 1961
        // Voronoi points drew a nearest-neighbour `line` 1.7 percent shorter.
        if hi < 1.1 * lo {
            return Ok(());
        }
        let mut s = c.session();
        // A minimum length drops more paths as marks get shorter, so with it a denser setting can
        // draw less; the solver bisects across such a drop.
        s.update("order", &json!({"min_length_mm": 0.0})).unwrap();
        // A tone response held to the image lightens the map as the style draws darker, so with
        // it a denser setting can draw less: with two pens light to dark, 3346 Voronoi points
        // emptied the second pen's band that 516 points drew in.
        s.update("tone", &json!({"response": "off"})).unwrap();
        let mut measure = |v: f64| -> Result<Option<[f64; 4]>, TestCaseError> {
            let v = if d.integer { json!(v as u64) } else { json!(v) };
            s.update(&d.stage, &json!({ &d.key: v })).unwrap();
            let mut r = s.start().unwrap();
            drain(&mut r)?;
            Ok((r.state() == RunState::Done).then(|| metrics(&s, &r)))
        };
        let (Some(a), Some(b)) = (measure(lo)?, measure(hi)?) else {
            return Ok(());
        };
        // The denser drawing has at least as much of one measure as the sparser one, give or
        // take 1 percent. The setting's `x-density-of` mark in its schema says which measure it
        // drives: `points` (the points placed), `lines` (the path count) or `length` (the
        // pen-down length). With no mark, a style that places points is judged on its points
        // and any other on its length. A style that places points is not judged on length by
        // default: its length follows how the renderer turns points into marks, since a dot no
        // wider than the pen is one vertex with no length, and points crowding a small dark
        // area sit on the pixel lattice, where a mesh with holes in it has longer edges than a
        // fuller one. The length may also fall short by one of the sparser drawing's average
        // paths: lines land on whole steps, so where they fall moves the length by about a line
        // (waves 3.8 percent shorter at a 6.3 mm spacing than at 7.7 mm, with six lines), and a
        // line ends within a tracing step of the edge (flow's step is half its minimum
        // spacing). A style whose spacing sets how many lines there are, but not how long they
        // are, marks `lines`, because its length does not follow the spacing: in waves and
        // spiral the ripple swing is a share of the spacing, so closer lines ripple less and
        // the total falls (waves with a 0.2 mm wavelength drew 0.99 m at one spacing and 0.53 m
        // at a closer one, with 4 and 18 paths), and in mosaic lines join into one path across
        // a gap of twice the spacing, so a sparser spacing joins with longer connectors. The
        // path count is otherwise left out, because it follows where lines break, such as where
        // a pen's share of the image runs out, and so is the time, because its pen-up travel
        // follows the path order, as much as the density.
        let (sparse, dense) = if d.direct { (a, b) } else { (b, a) };
        let (i, name, floor) = match drives(&style, &d).as_str() {
            "points" => (3, "dots", 0.99 * sparse[3]),
            "lines" => (1, "paths", 0.99 * sparse[1]),
            "length" => {
                let slack = sparse[2] / sparse[1].max(1.0);
                (2, "metres", (0.99 * sparse[2]).min(sparse[2] - slack))
            }
            other => panic!("{}.{} has x-density-of {other:?}", d.stage, d.key),
        };
        prop_assert!(
            dense[i] >= floor,
            "{}.{} {lo} gives {} {name}, {hi} gives {}",
            d.stage,
            d.key,
            a[i],
            b[i]
        );
        Ok(())
    });
}

#[test]
fn a_fit_lands_near_its_target_or_says_how_near_it_can() {
    let strategy = (case(dense_styles()), 0usize..4, -2.0..2.0f64);
    check("fit", 8, strategy, |(c, metric, octaves)| {
        let style = get_style(&c.style).unwrap();
        let d = Density::of(&style).unwrap();
        let names = ["minutes", "paths", "metres", "dots"];
        if names[metric] == "dots" && !places_points(&style) {
            return Ok(());
        }
        let mut s = c.session();
        let mut r = s.start().unwrap();
        drain(&mut r)?;
        if r.state() != RunState::Done {
            return Ok(());
        }
        let start = metrics(&s, &r)[metric];
        if start <= 0.0 {
            return Ok(());
        }
        let target = start * exp(octaves * ln(2.0));
        let mut f = s.fit(names[metric], target).unwrap();
        while f.next_frame().is_some() {}
        if f.state() == RunState::Error {
            prop_assert!(f.error().is_some_and(|e| !e.trim().is_empty()));
            return Ok(());
        }
        prop_assert_eq!(f.state(), RunState::Done);
        let fit = f.result().unwrap();
        prop_assert!(fit.samples.len() <= MAX_TRIALS);
        // A count cannot land between two whole numbers.
        let count =
            matches!(names[metric], "paths" | "dots") && (fit.achieved - target).abs() < 1.0;
        let within = count || (fit.achieved / target - 1.0).abs() <= TOLERANCE;
        // Trials on both sides of the target mean it falls in a jump no value reaches.
        let below = fit.samples.iter().any(|x| x.metric < target);
        let above = fit.samples.iter().any(|x| x.metric > target);
        if !within && !(below && above) {
            // Out of reach: then the fit is no farther from the target than the end of the
            // range that density points towards, give or take the fit's tolerance or one of the
            // end drawing's average paths. As in the density law, lines land on whole steps, so
            // a closer spacing can draw a line less: flow's minutes fell from 0.0642 to 0.0639
            // as its minimum spacing closed from 0.8 to 0.7 mm, and the search took that for a
            // cliff and stayed near 0.75 mm while 0.3 mm gave 0.0700.
            let end = if (target > start) == d.direct {
                d.max
            } else {
                d.min
            };
            let end = if d.integer {
                json!(end as u64)
            } else {
                json!(end)
            };
            s.update(&d.stage, &json!({ &d.key: end })).unwrap();
            let mut r = s.start().unwrap();
            drain(&mut r)?;
            let end_metrics = metrics(&s, &r);
            let at_end = end_metrics[metric];
            let off = |m: f64| (ln(m) - ln(target)).abs();
            let slack = ln(1.0 + TOLERANCE.max(1.0 / end_metrics[1].max(1.0)));
            prop_assert!(
                off(fit.achieved) <= off(at_end) + slack,
                "{} {target}: {} is farther than {at_end} at {end}: {:?}",
                names[metric],
                fit.achieved,
                fit.samples
            );
        }
        // The value the fit reports, applied, gives the metric it reports.
        s.update(&fit.stage, &json!({ &fit.key: fit.value.clone() }))
            .unwrap();
        let mut r = s.start().unwrap();
        drain(&mut r)?;
        prop_assert_eq!(metrics(&s, &r)[metric], fit.achieved);
        Ok(())
    });
}

/// The plot lint warnings and errors of the session's drawing, by code.
fn warned(s: &Session) -> Vec<&'static str> {
    (s.lint().unwrap_or_default().into_iter())
        .filter(|f| matches!(f.severity, Severity::Warning | Severity::Error))
        .map(|f| f.code)
        .collect()
}

#[test]
fn an_optimize_suggestion_keeps_the_limits() {
    check("optimize", 8, case(dense_styles()), |c| {
        let mut s = c.session();
        run(&mut s)?;
        if s.state() != RunState::Done {
            return Ok(());
        }
        let minutes = s.stats().unwrap().estimated_seconds / 60.0;
        let known = warned(&s);
        let mut job = s.optimize(None).unwrap();
        while job.next_frame().is_some() {}
        if job.state() == RunState::Error {
            prop_assert!(job.error().is_some_and(|e| !e.trim().is_empty()));
            return Ok(());
        }
        prop_assert_eq!(job.state(), RunState::Done);
        // A search that ends done always answers, if only to say why it has no suggestion.
        let found = job.result();
        prop_assert!(found.is_some());
        let found = found.unwrap();
        prop_assert!(found.trials.len() <= TRIALS);
        prop_assert_eq!(found.current_minutes, minutes);
        prop_assert!(!found.reason.is_empty());
        let Some(error) = found.error else {
            // Nothing scored: no trial within the limits had a score, and nothing is offered.
            prop_assert!(found.patch.is_none() && found.beyond.is_none());
            prop_assert!(found
                .trials
                .iter()
                .all(|t| t.score.is_none() || t.over || t.slow || !t.warnings.is_empty()));
            return Ok(());
        };
        // It is the quickest of the trials within the limits whose score is within a tenth of the
        // lowest, and only a trial left out for its time beats it by more.
        let kept: Vec<_> = (found.trials.iter())
            .filter(|t| t.score.is_some() && !t.over && !t.slow && t.warnings.is_empty())
            .collect();
        let lowest = kept
            .iter()
            .filter_map(|t| t.score)
            .fold(f64::INFINITY, f64::min);
        let best =
            (kept.iter()).find(|t| t.minutes == Some(found.minutes) && t.error == Some(error));
        prop_assert!(best.is_some_and(|t| t.score.unwrap() <= (1.0 + CLOSE) * lowest));
        prop_assert!(
            kept.iter()
                .all(|t| t.score.unwrap() > (1.0 + CLOSE) * lowest
                    || t.minutes >= Some(found.minutes))
        );
        if let Some(b) = &found.beyond {
            prop_assert!(b.slow && (1.0 + CLOSE) * b.score.unwrap() < lowest);
        }
        // The suggestion, applied, plots in the time it reports, within the limit, and with no
        // warning the current settings' drawing does not have.
        s.update(&found.stage, &json!({ &found.key: found.value.clone() }))
            .unwrap();
        run(&mut s)?;
        let after = s.stats().unwrap().estimated_seconds / 60.0;
        prop_assert_eq!(after, found.minutes);
        prop_assert!(
            after <= (SLOWER * minutes).max(LONGEST),
            "{after} min against {minutes}"
        );
        let new: Vec<_> = (warned(&s).into_iter())
            .filter(|w| !known.contains(w))
            .collect();
        prop_assert!(new.is_empty(), "{} = {}: {new:?}", found.key, found.value);
        Ok(())
    });
}

#[test]
fn a_test_chart_is_made_on_the_page_or_refused_with_a_message() {
    check("testchart", 48, (page(), pens()), |(pg, pens)| {
        let pens: Vec<Pen> = (pens.iter().enumerate())
            .map(|(i, (width, [r, g, b]))| {
                Pen::new(
                    &format!("Pen {i}"),
                    *width,
                    &format!("#{r:02x}{g:02x}{b:02x}"),
                )
                .unwrap()
            })
            .collect();
        let new = || {
            let mut s = Session::new(1 << 20);
            s.set_canvas(pg.canvas());
            s.set_pens(pens.clone()).unwrap();
            s
        };
        let mut s = new();
        match s.testchart() {
            Err(e) => {
                let message = e.to_string();
                prop_assert!(
                    message.contains("too small") || message.contains("too thin"),
                    "{message}"
                );
                prop_assert!(s.drawing().is_none() && s.state() == RunState::Idle);
            }
            Ok(()) => {
                prop_assert_eq!(s.state(), RunState::Done);
                let d = s.drawing().unwrap();
                prop_assert_eq!(d.layers.len(), 1);
                prop_assert_eq!(&d.layers[0].pen, &pens[0]);
                let [x, y, w, h] = d.canvas.drawing_rect();
                for p in d.layers[0].paths.coords() {
                    let inside = x - 1e-9 <= p[0]
                        && p[0] <= x + w + 1e-9
                        && y - 1e-9 <= p[1]
                        && p[1] <= y + h + 1e-9;
                    prop_assert!(inside, "{p:?} is off {:?}", [x, y, w, h]);
                }
                let mut t = new();
                t.testchart().unwrap();
                prop_assert_eq!(digest(&d), digest(&t.drawing().unwrap()));
            }
        }
        Ok(())
    });
}

#[test]
fn focus_points_keep_the_ink_within_their_radius() {
    // Every tone-map pixel within `focus_mm` of a focus point has the ink it has without focus.
    let points = prop::collection::vec(prop::array::uniform2(0.0..=1.0f64), 1..=3);
    check("focus", 12, (case(all_styles()), points), |(c, points)| {
        let drawn = |focus: Value| -> Result<Option<Arc<Drawing>>, TestCaseError> {
            let mut s = c.session();
            s.update("tone", &json!({ "focus": focus })).unwrap();
            run(&mut s)?;
            Ok(s.drawing())
        };
        // A run that ends in a message has no tone map to compare.
        let (Some(with), Some(without)) = (drawn(json!(points))?, drawn(json!([]))?) else {
            return Ok(());
        };
        let (Some(a), Some(b)) = (&with.tone, &without.tone) else {
            return Ok(());
        };
        prop_assert_eq!((a.width, a.height), (b.width, b.height));
        let radius = c.settings.0["tone"]["focus_mm"].as_f64().unwrap();
        let image = c.picture.image();
        let at: Vec<[f64; 2]> = (points.iter())
            .map(|&p| with.canvas.page_point(p, image.width, image.height))
            .collect();
        for y in 0..a.height as usize {
            for x in 0..a.width as usize {
                let [mx, my] = a.to_mm([x as f64 + 0.5, y as f64 + 0.5]);
                let near = at.iter().any(|p| {
                    let (dx, dy) = (mx - p[0], my - p[1]);
                    (dx * dx + dy * dy).sqrt() <= radius - 1e-6
                });
                let i = y * a.width as usize + x;
                prop_assert!(
                    !near || a.ink[i] == b.ink[i],
                    "({x}, {y}) changed within {radius} mm"
                );
            }
        }
        Ok(())
    });
}

/// The test images of the regression gallery, by name, which the look law draws.
const LOOK_IMAGES: [&str; 3] = ["still life", "scene", "gradient"];

#[test]
fn every_look_plots_clean() {
    // Balanced is the defaults, which some styles' own findings warn about, so it is left out.
    let looks: Vec<(String, String)> = (list_styles().iter())
        .flat_map(|s| {
            let others = s.looks().into_iter().filter(|l| l.name != "balanced");
            others.map(|l| (s.name.clone(), l.name)).collect::<Vec<_>>()
        })
        .collect();
    let strategy = (select(looks), 0..LOOK_IMAGES.len());
    check("looks", 12, strategy, |((style, look), image)| {
        let mut s = Session::new(256 << 20);
        s.set_image(match image {
            0 => still_life(600, 875),
            1 => scene(600, 875),
            _ => gradient(600, 875),
        });
        s.set_style(&style).unwrap();
        s.apply_look(&look).unwrap();
        run(&mut s)?;
        let case = format!("{style} at {look} on the {}", LOOK_IMAGES[image]);
        prop_assert_eq!(s.state(), RunState::Done, "{}: {:?}", case, s.error());
        let found: Vec<String> = (s.lint().unwrap_or_default().iter())
            .filter(|f| matches!(f.severity, Severity::Warning | Severity::Error))
            .map(|f| format!("{}: {}", f.code, f.message))
            .collect();
        prop_assert!(found.is_empty(), "{case} plots with {}", found.join("; "));
        Ok(())
    });
}

/// The X/Y points of a G-code file's moves, in order, and every other line.
fn moves(gcode: &str) -> (Vec<[f64; 2]>, Vec<&str>) {
    let (mut points, mut rest) = (vec![], vec![]);
    for line in gcode.lines() {
        let word = |c: char| {
            (line.split_whitespace())
                .find(|w| w.starts_with(c))
                .and_then(|w| w[1..].parse().ok())
        };
        let is_move = line.starts_with("G0 ") || line.starts_with("G1 ");
        match (is_move, word('X'), word('Y')) {
            (true, Some(x), Some(y)) => points.push([x, y]),
            _ => rest.push(line),
        }
    }
    (points, rest)
}

/// How far `p` is from the segment from `a` to `b`.
fn off_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

#[test]
fn split_moves_stay_on_the_move_they_cut_and_in_the_area() {
    let corner = (-100.0..100.0f64, -100.0..100.0f64);
    let strategy = (case(all_styles()), any::<bool>(), 0.5..20.0f64, corner);
    check("split", 12, strategy, |(c, servo, max, (x, y))| {
        let mut s = c.session();
        run(&mut s)?;
        let Some(d) = s.drawing() else {
            return Ok(());
        };
        let whole = Machine {
            name: "Law".to_string(),
            area: Area {
                x,
                y,
                width: c.page.width,
                height: c.page.height,
            },
            z_down_mm: 0.0,
            z_up_mm: 2.0,
            z_safe_mm: 5.0,
            draw_feed: 1000.0,
            travel_feed: 3000.0,
            z_feed: 500.0,
            settle_ms: 0,
            start_gcode: String::new(),
            end_gcode: String::new(),
            pen_lift: if servo { PenLift::Gcode } else { PenLift::Z },
            pen_down_gcode: "M280 P0 S30".to_string(),
            pen_up_gcode: "M280 P0 S90".to_string(),
            max_segment_mm: None,
            accel_mm_s2: DEFAULT_ACCEL_MM_S2,
        };
        let split = Machine {
            max_segment_mm: Some(max),
            ..whole.clone()
        };
        // Numbers are written to 0.001 mm, so a point may move by up to half of that each way.
        let eps = 2e-3;
        for layer in 0..d.layers.len() {
            s.set_machine(Some(whole.clone())).unwrap();
            let a = s.gcode(layer).unwrap();
            s.set_machine(Some(split.clone())).unwrap();
            let b = s.gcode(layer).unwrap();
            let ((w, rest_a), (p, rest_b)) = (moves(&a), moves(&b));
            prop_assert!(
                rest_a == rest_b,
                "layer {layer}: lines besides moves differ"
            );
            // The first travel, whose start the file does not know, is the same in both.
            prop_assert_eq!(w.first(), p.first());
            let mut j = 1;
            for i in 1..w.len() {
                loop {
                    prop_assert!(j < p.len(), "layer {layer}: {:?} is missing", w[i]);
                    let q = p[j];
                    let off = off_segment(q, w[i - 1], w[i]);
                    prop_assert!(off <= eps, "layer {layer}: {q:?} is {off} mm off its move");
                    let step = (q[0] - p[j - 1][0]).hypot(q[1] - p[j - 1][1]);
                    prop_assert!(
                        step <= max + eps,
                        "layer {layer}: a {step} mm piece > {max}"
                    );
                    prop_assert!(
                        whole.area.contains(q, eps),
                        "layer {layer}: {q:?} off the area"
                    );
                    j += 1;
                    if q == w[i] {
                        break;
                    }
                }
            }
            prop_assert_eq!(j, p.len().max(1));
        }
        Ok(())
    });
}

/// A polyline of up to 40 vertices, a tolerance, and whether no three of its vertices are
/// collinear: points on a coarse grid, which makes collinear and repeated vertices likely; free
/// points; or an arc, which has none collinear, with a tolerance of 0.
fn polyline() -> impl Strategy<Value = (Vec<[f64; 2]>, f64, bool)> {
    let point = (-50.0..50.0f64, -50.0..50.0f64).prop_map(|(x, y)| [x, y]);
    let grid = (-5..=5i32, -5..=5i32).prop_map(|(x, y)| [f64::from(x), f64::from(y)]);
    let tolerance = prop_oneof![Just(0.0), 0.0..0.1f64, 0.0..5.0f64];
    let arc = (1.0..100.0f64, 0.3..3.0f64, 3..40usize).prop_map(|(r, sweep, n)| {
        let angle = |i: usize| sweep * i as f64 / (n - 1) as f64;
        let line = (0..n).map(|i| [r * angle(i).cos(), r * angle(i).sin()]);
        (line.collect(), 0.0, true)
    });
    prop_oneof![
        (prop::collection::vec(grid, 0..40), tolerance.clone()).prop_map(|(l, t)| (l, t, false)),
        (prop::collection::vec(point, 0..40), tolerance).prop_map(|(l, t)| (l, t, false)),
        arc,
    ]
}

#[test]
fn simplify_keeps_the_ends_drops_no_more_than_the_tolerance_and_nothing_at_zero() {
    check("simplify", 512, polyline(), |(line, tol, no_collinear)| {
        let out = simplify(&line, tol);
        prop_assert_eq!(out.first(), line.first());
        prop_assert_eq!(out.last(), line.last());
        // The output is the input with some vertices removed.
        let mut rest = line.iter();
        prop_assert!(
            out.iter().all(|p| rest.any(|q| q == p)),
            "{out:?} is not a subsequence"
        );
        prop_assert!(out.len() <= line.len());
        // Every input vertex is within the tolerance of the output line.
        for p in &line {
            let off = (out.iter().zip(out.iter().skip(1)))
                .map(|(a, b)| off_segment(*p, *a, *b))
                .fold(f64::INFINITY, f64::min);
            let off = if out.len() == 1 {
                off_segment(*p, out[0], out[0])
            } else {
                off
            };
            prop_assert!(off <= tol + 1e-9, "{p:?} is {off} from {out:?}, over {tol}");
        }
        // Nothing is collinear, so a tolerance of 0 drops nothing.
        prop_assert!(!no_collinear || out == line, "{out:?} lost vertices");
        Ok(())
    });
}

/// Each segment of `paths` between two different points, as its two ends sorted, once.
fn segments_of(paths: &Paths) -> Vec<[[i64; 2]; 2]> {
    let point = |p: [f64; 2]| [p[0] as i64, p[1] as i64];
    let mut out: Vec<[[i64; 2]; 2]> = (paths.iter())
        .flat_map(|p| p.windows(2).map(|s| [point(s[0]), point(s[1])]))
        .filter(|s| s[0] != s[1])
        .map(|mut s| {
            s.sort();
            s
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

#[test]
fn joining_never_adds_a_path_and_draws_every_segment_and_dot() {
    // Paths on a coarse grid, so that paths meet, cross at vertices, repeat a segment, repeat a
    // vertex in place and come as dots.
    let vertex = (0..6i32, 0..6i32).prop_map(|(x, y)| [f64::from(x), f64::from(y)]);
    let paths = prop::collection::vec(prop::collection::vec(vertex, 1..7), 0..16);
    check("join", 512, paths, |list| {
        let mut input = Paths::new();
        for p in &list {
            input.push(p);
        }
        let dots = |p: &Paths| {
            let mut d: Vec<Vec<[f64; 2]>> = p
                .iter()
                .filter(|q| q.len() == 1)
                .map(<[_]>::to_vec)
                .collect();
            d.sort_by(|a, b| {
                a[0][0]
                    .total_cmp(&b[0][0])
                    .then(a[0][1].total_cmp(&b[0][1]))
            });
            d
        };
        let joined = join(&input, false);
        let retraced = join(&input, true);
        for out in [&joined, &retraced] {
            prop_assert!(
                out.len() <= input.len(),
                "{} paths from {}",
                out.len(),
                input.len()
            );
            prop_assert_eq!(segments_of(out), segments_of(&input));
            prop_assert_eq!(dots(out), dots(&input));
        }
        prop_assert!(retraced.len() <= joined.len());
        Ok(())
    });
}

#[test]
fn a_jittered_raster_covers_lines_at_their_width_at_any_angle_and_place() {
    // Eight parallel lines 20 mm long and 2 mm apart, at an angle on the raster's lattice half the
    // time. Centre sampling snaps every such line's width to whole cells the same way, by up to
    // 84 % of the area for the thinnest pen; jittered sampling's noise stayed within 4 % over
    // 10,000 cases.
    let angle = prop_oneof![select(vec![0.0, 45.0, 90.0, 135.0]), 0.0..180.0f64];
    let case = (angle, 0.1..1.0f64, 0.0..1.0f64, 0.0..1.0f64);
    check("jittered raster", 256, case, |(angle, pen, x, y)| {
        let rect = [0.0, 0.0, 40.0, 40.0];
        let cell = cell_size(pen, rect);
        let (ux, uy) = (angle.to_radians().cos(), angle.to_radians().sin());
        let mut lines = Paths::new();
        for k in 0..8 {
            let o = 2.0 * (f64::from(k) - 3.5);
            let (cx, cy) = (20.0 + x - o * uy, 20.0 + y + o * ux);
            lines.push(&[
                [cx - 10.0 * ux, cy - 10.0 * uy],
                [cx + 10.0 * ux, cy + 10.0 * uy],
            ]);
        }
        let mut c = Coverage::jittered(rect, cell);
        c.stamp(&lines, pen);
        let inked = c.count.iter().filter(|&&n| n > 0).count() as f64 * cell * cell;
        let area = 8.0 * (20.0 * pen + std::f64::consts::PI * pen * pen / 4.0);
        prop_assert!((inked / area - 1.0).abs() < 0.06, "{inked} mm2 of {area}");
        Ok(())
    });
}

/// The plot time with acceleration is never below the time at full speed all the way, the
/// distances over the speeds plus the lifts, which is what it was before acceleration counted; and
/// it comes to that time as the acceleration grows without limit.
#[test]
fn acceleration_never_shortens_the_plot_time() {
    let speed = || 1.0..500.0f64;
    let strategy = (
        case(all_styles()),
        speed(),
        speed(),
        0.0..2.0f64,
        1.0..1e5f64,
    );
    check("accel", 12, strategy, |(c, draw, travel, lift, accel)| {
        let mut s = c.session();
        run(&mut s)?;
        let Some(d) = s.drawing() else {
            return Ok(());
        };
        let speeds = Speeds {
            draw_mm_s: draw,
            travel_mm_s: travel,
            pen_lift_s: lift,
            accel_mm_s2: accel,
        };
        let st = stats::compute(&d, speeds);
        let full = st.pen_down_mm / draw + st.pen_up_mm / travel + st.path_count as f64 * lift;
        // The two sums add the same moves in another order, so they may differ by rounding.
        let eps = 1e-9 * full.max(1.0);
        prop_assert!(
            st.estimated_seconds >= full - eps,
            "{} s with acceleration, {full} s without",
            st.estimated_seconds
        );
        let unbounded = Speeds {
            accel_mm_s2: f64::INFINITY,
            ..speeds
        };
        let limit = stats::compute(&d, unbounded).estimated_seconds;
        prop_assert!((limit - full).abs() <= eps, "{limit} s against {full} s");
        Ok(())
    });
}

/// No drawing's tone error is above blank paper's, and blank paper never beats a drawing: a
/// drawing with no ink scores blank paper's error with no contrast, which Optimize never picks.
/// In colour, blank paper takes away none of its own difference from the image, and no drawing
/// takes away more than all of it.
#[test]
fn blank_paper_bounds_the_tone_match_and_never_wins() {
    check("blank", 12, case(all_styles()), |c| {
        let mut s = c.session();
        run(&mut s)?;
        let (Some(d), Some(m)) = (s.drawing(), s.tone_match()) else {
            return Ok(());
        };
        let tone = d.tone.as_deref().unwrap();
        prop_assert!(m.error <= m.blank, "{m:?}");
        let blank = tone_match::compare(&vec![0.0; tone.ink.len()], tone).unwrap();
        prop_assert_eq!((blank.error, blank.contrast), (m.blank, 0.0));
        prop_assert!(blank.score() >= m.score() && blank.score() == f64::INFINITY);
        if let Some(colour) = m.colour {
            prop_assert!(colour.gain <= 1.0, "{colour:?}");
            let empty = Drawing {
                layers: vec![],
                ..(*d).clone()
            };
            let paper = tone_match::colour(&empty, &c.picture.image(), s.paper()).unwrap();
            // The eye filter rounds a flat page in f32, so blank paper comes back to within a hair.
            prop_assert!(paper.gain.abs() < 1e-3, "{paper:?}");
        }
        Ok(())
    });
}

/// The plot playback's times, every pen-up move's start and end and every vertex's arrival, layer
/// by layer, never go back, and the last one is the estimated plot time; each layer has one move
/// per path and one time per vertex.
#[test]
fn the_timeline_rises_and_ends_at_the_plot_time() {
    let strategy = (case(all_styles()), 0.0..2.0f64, 1.0..1e5f64);
    check("timeline", 12, strategy, |(c, lift, accel)| {
        let mut s = c.session();
        s.set_speeds(Speeds {
            pen_lift_s: lift,
            accel_mm_s2: accel,
            ..Speeds::default()
        });
        run(&mut s)?;
        let (Some(d), Some(timeline)) = (s.drawing(), s.timeline()) else {
            return Ok(());
        };
        let times = timeline.layers;
        prop_assert_eq!(times.len(), d.layers.len());
        let mut last = 0.0;
        for (layer, t) in d.layers.iter().zip(&times) {
            prop_assert_eq!(t.travel.len(), layer.paths.len());
            prop_assert_eq!(t.vertices.len(), layer.paths.coords().len());
            for (i, [up, down]) in t.travel.iter().copied().enumerate() {
                let path = layer.paths.offsets()[i] as usize..layer.paths.offsets()[i + 1] as usize;
                for at in [up, down]
                    .into_iter()
                    .chain(t.vertices[path].iter().copied())
                {
                    prop_assert!(at >= last, "{at} s comes after {last} s");
                    last = at;
                }
            }
        }
        let total = s.stats().unwrap().estimated_seconds;
        // The two add the same moves in another order, so they may differ by rounding.
        prop_assert!(
            (last - total).abs() <= 1e-9 * total.max(1.0),
            "{last} s against {total} s"
        );
        Ok(())
    });
}

/// The `x,y` pairs of the argument list of an HPGL `PU` or `PD`; a lone `PD;` has none.
fn hpgl_pairs(args: &str) -> Vec<[i64; 2]> {
    let n: Vec<i64> = (args.split(',').filter(|v| !v.is_empty()))
        .map(|v| v.parse().unwrap())
        .collect();
    n.chunks(2).map(|p| [p[0], p[1]]).collect()
}

#[test]
fn hpgl_stays_on_the_page_and_keeps_every_vertex() {
    let units = prop_oneof![Just(40.0), 0.01..=MAX_UNITS_PER_MM];
    check("hpgl", 12, (case(all_styles()), units), |(c, units)| {
        let mut s = c.session();
        run(&mut s)?;
        let Some(d) = s.drawing() else {
            return Ok(());
        };
        let h = s.hpgl(units).unwrap();
        prop_assert!(h.is_ascii() && !h.contains('\r'), "not ASCII with LF ends");
        prop_assert!(h.starts_with("IN;\n") && h.ends_with("PU;\nSP0;\n"));
        // The page scaled by the resolution, and the vertices of PU and PD against the drawing's.
        let (w, hh) = (
            (c.page.width * units).round(),
            (c.page.height * units).round(),
        );
        let (mut pens, mut vertices) = (0, 0);
        for line in h.lines() {
            prop_assert!(line.ends_with(';'), "{line:?} does not end with ';'");
            let line = line.trim_end_matches(';');
            if line.starts_with("SP") {
                pens += (line != "SP0") as usize;
            } else if let Some(args) = line.strip_prefix("PU").or(line.strip_prefix("PD")) {
                for [x, y] in hpgl_pairs(args) {
                    vertices += 1;
                    prop_assert!(
                        (0.0..=w).contains(&(x as f64)) && (0.0..=hh).contains(&(y as f64)),
                        "({x}, {y}) is off the {w} by {hh} page"
                    );
                }
            }
        }
        let layers = d.layers.iter().filter(|l| !l.paths.is_empty()).count();
        prop_assert_eq!(pens, layers);
        let expected: usize = d.layers.iter().map(|l| l.paths.coords().len()).sum();
        prop_assert_eq!(vertices, expected);
        Ok(())
    });
}

/// The paper shows wherever the white-paper picture is pure white, and ink and outline only
/// darken it: no channel of the picture on coloured paper is lighter than the paper or than the
/// white-paper picture, give or take one level.
///
/// The picture on coloured paper is not the white-paper one times the paper: tiny-skia blends in
/// 8-bit fixed point and rounds at every blend, and a pixel crossed by several passes of the pen
/// rounds once per pass. A change of a few levels that is lost on a dark paper, because it rounds
/// to nothing there, adds up to tens of levels (35 over 240 cases) in a dense drawing. So the law
/// keeps what rounding cannot move.
#[test]
fn the_paper_shows_where_nothing_is_drawn_and_ink_only_darkens_it() {
    let strategy = (case(all_styles()), any::<[u8; 3]>());
    check("paper", 12, strategy, |(c, rgb)| {
        let mut s = c.session();
        run(&mut s)?;
        let Some(d) = s.drawing() else {
            return Ok(());
        };
        let paper = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        let (w, h, white) = render_rgba(&d, 200, "#ffffff", false, false, None);
        let (w2, h2, tinted) = render_rgba(&d, 200, &paper, false, false, None);
        prop_assert_eq!((w, h), (w2, h2));
        prop_assert_eq!(white.len(), 4 * (w * h) as usize);
        prop_assert_eq!(tinted.len(), white.len());
        for (i, (a, b)) in white.chunks(4).zip(tinted.chunks(4)).enumerate() {
            prop_assert!(a[3] == 255 && b[3] == 255, "pixel {i} is not opaque");
            if a[..3] == [255, 255, 255] {
                prop_assert!(b[..3] == rgb, "pixel {i} is {b:?}, not the paper {paper}");
            }
            for k in 0..3 {
                let most = u32::from(a[k].min(rgb[k])) + 1;
                prop_assert!(
                    u32::from(b[k]) <= most,
                    "pixel {i} channel {k}: {} on {paper} is lighter than the white-paper {}",
                    b[k],
                    a[k]
                );
            }
        }
        Ok(())
    });
}

/// How the GUI's style grid draws its pictures (`THUMB_PX` and `THUMB_SHRINK` in
/// web/src/app.tsx): 600 px wide with every mark at least 2 px, for a cell about 150 px wide.
const GRID_PX: u32 = 600;
const GRID_SHRINK: f64 = 2.0;
const CELL_PX: u32 = 150;

#[test]
fn every_style_draws_a_colour_image_with_several_pens_and_shows_in_the_style_grid() {
    // A colour scene on A4 with three coloured pens, in every Pen use mode. A dot style once drew
    // there but showed a blank cell: 0.3 mm dots 1 mm apart, a pixel wide in a 600 px picture,
    // average to almost white at the cell's size. A visible mark is a cell pixel at least 10 %
    // darker than the paper, and every style's cell has them on at least 3 % of its pixels.
    let first = &list_styles()[0];
    let split = first.stages[first.stage_index("split").unwrap()].schema();
    let modes = choices(&split["properties"]["mode"], &split["$defs"]).unwrap();
    let pens = [
        ("Red", "#d0202e"),
        ("Black", "#000000"),
        ("Yellow", "#f5d000"),
    ]
    .map(|(name, color)| Pen::new(name, 0.3, color).unwrap())
    .to_vec();
    for mode in &modes {
        let mut s = Session::new(1 << 30);
        s.set_image(scene(300, 225));
        s.set_pens(pens.clone()).unwrap();
        for style in all_styles() {
            s.set_style(&style).unwrap();
            s.update("split", &json!({ "mode": mode })).unwrap();
            for &(stage, key, cap) in SPEED_CAPS {
                let now = &s.settings()[stage][key];
                if now.as_f64().is_some_and(|v| v > cap) {
                    let cap = if now.is_u64() {
                        json!(cap as u64)
                    } else {
                        json!(cap)
                    };
                    s.update(stage, &json!({ key: cap })).unwrap();
                }
            }
            let mut r = s.start().unwrap();
            while r.next_frame().is_some() {}
            let d = s.drawing().unwrap();
            let paths: usize = d.layers.iter().map(|l| l.paths.len()).sum();
            assert!(paths > 0, "{style} in {mode} draws nothing");
            let png = render_png_shrunk(&d, GRID_PX, s.paper(), GRID_SHRINK).unwrap();
            let picture = image::load_from_memory(&png).unwrap().to_luma8();
            let height = CELL_PX * picture.height() / picture.width();
            let cell = image::imageops::resize(
                &picture,
                CELL_PX,
                height,
                image::imageops::FilterType::Triangle,
            );
            let marked = cell.pixels().filter(|p| p.0[0] <= 230).count() as f64
                / f64::from(CELL_PX * height);
            assert!(
                marked >= 0.03,
                "{style} in {mode} marks {:.1} % of its cell",
                100.0 * marked
            );
        }
    }
}

/// Run `stage` on `inputs` with `patch` over its defaults to its output.
fn run_stage(stage: &dyn Stage, inputs: Vec<Artifact>, patch: &Value) -> Artifact {
    let settings = merge(&stage.defaults(), patch);
    let inputs = Inputs::new(inputs.into_iter().map(|a| (a.kind(), a)).collect());
    let mut run = (stage.start(inputs, &settings, RunContext::new(Arc::default()))).unwrap();
    loop {
        if let Step::Done(a) = run.step().unwrap() {
            return a;
        }
    }
}

/// The Pen use stage's maps and budgets in `mode`, after the tone stage with `tone` over its
/// defaults.
fn split_in(image: &Image, canvas: &Canvas, pens: &[Pen], tone: &Value, mode: &Value) -> Tones {
    let (image, canvas) = (
        Artifact::Image(Arc::new(image.clone())),
        Artifact::Canvas(Arc::new(canvas.clone())),
    );
    let tone = run_stage(&ToneStage, vec![image.clone(), canvas.clone()], tone);
    let pens = Artifact::Pens(Arc::new(pens.to_vec()));
    match run_stage(
        &SplitStage,
        vec![image, canvas, pens, tone],
        &json!({ "mode": mode }),
    ) {
        Artifact::Tones(t) => Tones::clone(&t),
        a => panic!("expected tone maps, got {}", a.kind().name()),
    }
}

/// Each pen's map from the Pen use stage by colour, after the tone stage with `tone` over its
/// defaults.
fn by_colour(image: &Image, canvas: &Canvas, pens: &[Pen], tone: &Value) -> Vec<Arc<ToneMap>> {
    split_in(image, canvas, pens, tone, &json!("by_colour")).maps
}

#[test]
fn by_colour_a_gamma_bends_each_pens_share_and_adds_no_grey() {
    // With nothing after it in the tone stage, a gamma gives each pen `1 - (1 - a)^gamma` of the
    // share `a` of a colour it has at a gamma of 1, so a pen draws only where it draws at 1: a
    // darker gamma no longer turns into grey that the black pen draws over every colour.
    let strategy = (picture(), page(), pens(), 0.1..=5.0f64, any::<bool>());
    check(
        "gamma by colour",
        24,
        strategy,
        |(picture, page, pens, gamma, auto)| {
            let (image, canvas, pens) = (picture.image(), page.canvas(), pens_of(&pens));
            let levels = if auto { "auto" } else { "manual" };
            let at = |g: f64| {
                by_colour(
                    &image,
                    &canvas,
                    &pens,
                    &json!({"resolution": 64, "gamma": g, "levels": levels}),
                )
            };
            let (flat, bent) = (at(1.0), at(gamma));
            for (k, (a, b)) in flat.iter().zip(&bent).enumerate() {
                for (p, (&a, &b)) in a.ink.iter().zip(&b.ink).enumerate() {
                    let want = 1.0 - (1.0 - f64::from(a)).powf(gamma);
                    prop_assert!(
                        (f64::from(b) - want).abs() < 1e-3,
                        "pen {k}, pixel {p}: {b} at gamma {gamma}, want {want} from {a}"
                    );
                }
            }
            Ok(())
        },
    );
}

/// The styles whose density setting is the number of points their placer places.
fn counted_styles() -> Vec<String> {
    let counted = |style: &Style| {
        Density::of(style).is_some_and(|d| {
            let stage = style.stages.iter().find(|s| s.name() == d.stage).unwrap();
            d.direct && d.integer && stage.provides() == Kind::Points
        })
    };
    (list_styles().iter())
        .filter(|s| counted(s))
        .map(|s| s.name.clone())
        .collect()
}

#[test]
fn pens_share_a_point_count_between_them() {
    // A style whose density setting is a number of points places at most that many per pen. By
    // colour the pens share those budgets by ink: their parts run from 0 to the number of pens,
    // each as wide as its share of the ink times the pens; in the other modes each pen has a
    // whole budget of its own.
    let first = &list_styles()[0];
    let split = first.stages[first.stage_index("split").unwrap()].schema();
    let modes = choices(&split["properties"]["mode"], &split["$defs"]).unwrap();
    check(
        "shared count",
        16,
        (case(counted_styles()), select(modes)),
        |(c, mode)| {
            let style = get_style(&c.style).unwrap();
            let d = Density::of(&style).unwrap();
            let count = c.settings.0[&d.stage][&d.key].as_f64().unwrap();
            let mut s = c.session();
            s.update("split", &json!({ "mode": mode })).unwrap();
            let mut r = s.start().unwrap();
            drain(&mut r)?;
            if r.state() == RunState::Done {
                let pens = s.drawing().unwrap().layers.len() as f64;
                let placed = r.point_count().unwrap_or(0);
                prop_assert!(
                    placed as f64 <= count * pens,
                    "{placed} points for a count of {count} and {pens} pens"
                );
            }
            let (image, canvas) = (c.picture.image(), c.page.canvas());
            let tones = split_in(
                &image,
                &canvas,
                &c.pens(),
                &json!({"resolution": 64}),
                &mode,
            );
            let ink: Vec<f64> = (tones.maps.iter())
                .map(|m| m.ink.iter().map(|&v| f64::from(v)).sum())
                .collect();
            let (n, all) = (ink.len() as f64, ink.iter().sum::<f64>());
            if mode == "by_colour" && all > 0.0 {
                let [first, last] = [tones.budgets[0][0], tones.budgets.last().unwrap()[1]];
                prop_assert_eq!((first, last), (0.0, n));
                for (k, (b, i)) in tones.budgets.iter().zip(&ink).enumerate() {
                    let next = tones.budgets.get(k + 1).map_or(n, |b| b[0]);
                    prop_assert_eq!(b[1], next, "pen {} does not meet the next", k);
                    let want = n * i / all;
                    prop_assert!(
                        (b[1] - b[0] - want).abs() < 1e-9,
                        "pen {k}: {b:?}, want {want}"
                    );
                }
            } else {
                prop_assert!(
                    tones.budgets.iter().all(|b| *b == [0.0, 1.0]),
                    "{:?}",
                    tones.budgets
                );
            }
            Ok(())
        },
    );
}
