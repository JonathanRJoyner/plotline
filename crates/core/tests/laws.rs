//! Laws that every style and every setting keeps, checked on random inputs.
//!
//! Nothing here names a style or a setting. The inputs come from the registry and from each
//! stage's settings schema, so a new style or setting is covered with no edit to this file. A case
//! is a small image built in code (a two-colour gradient under discs, boxes and noise, or one of
//! the crate's synthetic pictures), a page and margin, one to three pens, a style, and a value for
//! each of its settings, Pen use included, from the schema's range: the default, either end, or
//! anything between. proptest shrinks a failing case and prints it.
//!
//! Each law runs a fixed number of cases from a fixed seed, so a pull request's run is the same
//! every time. `PLOTLINE_LAWS_SCALE` multiplies the case counts and raises the speed caps, and
//! `PLOTLINE_LAWS_SEED` picks another seed; the weekly workflow sets both.
//!
//! A feature with a contract adds its law here (see CONTRIBUTING.md).

use std::fmt;
use std::sync::{Arc, Mutex};

use plotline_core::cache::StageCache;
use plotline_core::fit::{places_points, Density, MAX_TRIALS, TOLERANCE};
use plotline_core::math::{exp, ln};
use plotline_core::registry::{get_style, list_styles};
use plotline_core::runner::Shared;
use plotline_core::stage::Artifact;
use plotline_core::style::Style;
use plotline_core::synthetic::{flat_noise, ramp, scene, strokes};
use plotline_core::{Canvas, Drawing, Fit, Image, Paths, Pen, Run, RunState, Session};
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

/// The seed of the pull request run.
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

/// A page in mm, its margin as a share of the most it could be, and whether the image fills it.
#[derive(Clone, Debug)]
struct Page {
    width: f64,
    height: f64,
    margin: f64,
    fill: bool,
}

impl Page {
    fn canvas(&self) -> Canvas {
        let margin = self.margin * self.width.min(self.height) / 2.0;
        let fit = if self.fill { Fit::Fill } else { Fit::Fit };
        Canvas::new(self.width, self.height, margin, fit).expect("the margin leaves an area")
    }
}

fn page() -> impl Strategy<Value = Page> {
    (20.0..160.0f64, 20.0..160.0f64, 0.0..0.9f64, any::<bool>()).prop_map(
        |(width, height, margin, fill)| Page {
            width,
            height,
            margin,
            fill,
        },
    )
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
            page,
            pens,
            cancel_at,
        },
    )
}

impl Case {
    fn pens(&self) -> Vec<Pen> {
        (self.pens.iter().enumerate())
            .map(|(i, (width, [r, g, b]))| {
                let colour = format!("#{r:02x}{g:02x}{b:02x}");
                Pen::new(&format!("Pen {i}"), *width, &colour).unwrap()
            })
            .collect()
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
/// then the page size with the margin and fit kept.
fn apply(s: &mut Session, patch: &Value) -> Result<(), TestCaseError> {
    if let Some(style) = patch["style"].as_str() {
        s.set_style(style)
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
        let canvas = Canvas::new(
            w.unwrap_or(c.width_mm),
            h.unwrap_or(c.height_mm),
            c.margin_mm,
            c.fit,
        );
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

#[test]
fn a_density_setting_moves_its_metrics_one_way() {
    let strategy = (case(dense_styles()), 0.0..1.0f64, 0.0..1.0f64);
    check("density", 24, strategy, |(c, p, q)| {
        let style = get_style(&c.style).unwrap();
        let d = Density::of(&style).unwrap();
        let hi = cap(&d.stage, &d.key).map_or(d.max, |cap| d.max.min(cap));
        let at = |share: f64| grain(&d, exp(ln(d.min) + share * (ln(hi) - ln(d.min))));
        let (lo, hi) = (at(p.min(q)), at(p.max(q)));
        if lo == hi {
            return Ok(());
        }
        let mut s = c.session();
        // A minimum length drops more paths as marks get shorter, so with it a denser setting can
        // draw less; the solver bisects across such a drop.
        s.update("order", &json!({"min_length_mm": 0.0})).unwrap();
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
        // The time, the pen-down length and the points of the denser drawing are at least those
        // of the sparser one, give or take 1 percent: lines and grids land on whole steps, which
        // wobble a little. The path count is left out, because it follows where lines break,
        // such as where the Hilbert curve leaves the image, as much as the density.
        let (sparse, dense) = if d.direct { (a, b) } else { (b, a) };
        for (i, name) in [(0, "minutes"), (2, "metres"), (3, "dots")] {
            prop_assert!(
                dense[i] >= 0.99 * sparse[i],
                "{}.{} {lo} gives {} {name}, {hi} gives {}",
                d.stage,
                d.key,
                a[i],
                b[i]
            );
        }
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
        let value = fit.value.as_f64().unwrap();
        // A count cannot land between two whole numbers.
        let count =
            matches!(names[metric], "paths" | "dots") && (fit.achieved - target).abs() < 1.0;
        let within = count || (fit.achieved / target - 1.0).abs() <= TOLERANCE;
        let at_end = value == d.min || value == d.max;
        let below = fit.samples.iter().any(|x| x.metric < target);
        let above = fit.samples.iter().any(|x| x.metric > target);
        prop_assert!(
            within || at_end || (below && above),
            "{} {target}: neither near, at an end of {}..{}, nor between two trials: {:?}",
            names[metric],
            d.min,
            d.max,
            fit.samples
        );
        // The value the fit reports, applied, gives the metric it reports.
        s.update(&fit.stage, &json!({ &fit.key: fit.value.clone() }))
            .unwrap();
        let mut r = s.start().unwrap();
        drain(&mut r)?;
        prop_assert_eq!(metrics(&s, &r)[metric], fit.achieved);
        Ok(())
    });
}
