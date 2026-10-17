//! Measures the looks (see `docs/decisions/2026-10-16-looks.md`): for every listed style with a
//! density setting, the values of its Faithful, Quick and Fine looks, from drawings of the
//! regression gallery's three test images and seven more on A4 with the default 10 mm margin, the default
//! 0.3 mm pen and the default speeds.
//!
//! Run it with `cargo run --release -p plotline-core --example looks [-- style ...]`. It prints a
//! table per style, then the looks it chose as JSON, on stdout, and progress on stderr. Every
//! style is drawn about a hundred times, so it takes several minutes.
//!
//! The density is scanned on a grid from a quarter to four times the default, with the tone
//! stage's `response` off and `auto`. No look may warn (any plot lint finding of severity warning
//! or error) on any image, or plot for longer than twice Balanced's time on the same image or an
//! hour, whichever is longer.
//!
//! Each drawing is scored by `ToneMatch::score`, the tone error over blank paper's divided by a
//! power of the contrast, and a trial by its median score over the images.
//!
//! - **Faithful:** the lowest median score, `auto` only where it beats `off` by 5 %, offered
//!   when it beats Balanced by 10 %.
//! - **Quick:** where Balanced's median plot time is over 40 minutes, the median of what the
//!   budget fit finds for 30 minutes, offered when its median score is at most twice
//!   Balanced's.
//! - **Fine:** the densest value whose median score stays within a quarter of Balanced's, with
//!   every value between it and the default passing too, unless it lies within a step of
//!   Faithful's.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::thread;

use plotline_core::fit::Density;
use plotline_core::lint::Severity;
use plotline_core::registry::list_styles;
use plotline_core::settings::merge;
use plotline_core::synthetic::{
    checkerboard, color_bars, gradient, noise, scene, still_life, strokes,
};
use plotline_core::tone_match::ToneMatch;
use plotline_core::{Image, Job, RunState, Session};
use serde_json::{json, Map, Value};

/// The plot time Quick aims at, and the Balanced time above which a style gets one, in minutes.
const QUICK_MINUTES: f64 = 30.0;
const QUICK_FROM: f64 = 40.0;
/// No look plots longer than this many times Balanced, or [`LONGEST`] minutes if that is more.
const SLOWER: f64 = 2.0;
const LONGEST: f64 = 60.0;
/// The scan reaches this many times sparser and denser than the default.
const REACH: f64 = 4.0;
/// The ratio between neighbouring values on the scan's grid.
const STEP: f64 = 1.19;
/// How much lower a median score must be to count as better, for a look over Balanced and for
/// `auto` over `off`; and how much higher Fine's may be than Balanced's.
const BETTER: f64 = 0.9;
const AUTO_BETTER: f64 = 0.95;
const FINE_WORSE: f64 = 1.25;
/// How much higher Quick's mean tone error may be than Balanced's: a style whose pen-down length
/// alone takes most of half an hour reaches it only by a spacing that loses the picture.
const QUICK_WORSE: f64 = 2.0;

/// `img` with every channel passed through `f`.
fn shade(mut img: Image, f: impl Fn(f32) -> f32) -> Image {
    img.rgb.iter_mut().for_each(|v| *v = f(*v));
    img
}

/// Three dark discs on white paper, like a poster of flat shapes.
fn poster(w: u32, h: u32) -> Image {
    let discs = [(0.3, 0.36, 0.12), (0.7, 0.73, 0.08), (0.4, 1.09, 0.15)];
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let (u, v) = (f64::from(x) / f64::from(w), f64::from(y) / f64::from(w));
            let ink = (discs.iter())
                .any(|&(cx, cy, r)| (u - cx) * (u - cx) + (v - cy) * (v - cy) < r * r);
            let g = if ink { 0.1 } else { 1.0 };
            rgb.extend([g, g, g]);
        }
    }
    Image {
        width: w,
        height: h,
        rgb,
    }
}

/// The images, the shape of the default A4 drawing area: the gallery's three, then flat shapes
/// on white, a checkerboard, colour bars, line art, a busy texture, and the scene made dark and
/// made pale (the colour and styles audit's F13).
fn images() -> Vec<Image> {
    vec![
        still_life(600, 875),
        scene(600, 875),
        gradient(600, 875),
        poster(600, 875),
        checkerboard(600, 875, 6),
        color_bars(600, 875),
        strokes(600, 875, 0.0, 1.0),
        noise(600, 875, 7),
        shade(scene(600, 875), |v| v * 0.35),
        shade(scene(600, 875), |v| 1.0 - (1.0 - v) * 0.35),
    ]
}

/// The images' names, in the order of [`images`].
const NAMES: [&str; 10] = [
    "still life",
    "scene",
    "gradient",
    "poster",
    "checker",
    "bars",
    "lines",
    "noise",
    "dark",
    "pale",
];

/// One drawing's measurements.
#[derive(Clone)]
struct Eval {
    minutes: f64,
    score: Option<f64>,
    warnings: Vec<&'static str>,
}

/// A patch and its drawings on every image.
#[derive(Clone)]
struct Trial {
    patch: Value,
    evals: Vec<Eval>,
}

impl Trial {
    /// The median score over the images; a drawing without one counts as infinite.
    fn score(&self) -> f64 {
        median(
            self.evals
                .iter()
                .map(|e| e.score.unwrap_or(f64::INFINITY))
                .collect(),
        )
    }

    fn median_minutes(&self) -> f64 {
        median(self.evals.iter().map(|e| e.minutes).collect())
    }

    fn clean(&self) -> bool {
        self.evals.iter().all(|e| e.warnings.is_empty())
    }

    /// Clean, and within the time `balanced` allows on every image.
    fn allowed(&self, balanced: &Trial) -> bool {
        let quick = (self.evals.iter().zip(&balanced.evals))
            .all(|(e, b)| e.minutes <= (SLOWER * b.minutes).max(LONGEST));
        self.clean() && quick
    }

    fn row(&self, name: &str) -> String {
        let cells: Vec<String> = (self.evals.iter())
            .map(|e| {
                let warns = if e.warnings.is_empty() {
                    "-".to_string()
                } else {
                    e.warnings.join(",")
                };
                format!(
                    "{:.4} {:.1} min {warns}",
                    e.score.unwrap_or(f64::NAN),
                    e.minutes
                )
            })
            .collect();
        format!(
            "| {name} | `{}` | {} | {:.4} |\n",
            self.patch,
            cells.join(" | "),
            self.score()
        )
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// Take every frame of `job` and say how it ended.
fn drive(job: &mut dyn Job) -> Result<(), String> {
    while job.next_frame().is_some() {}
    match job.state() {
        RunState::Done => Ok(()),
        _ => Err(job.error().unwrap_or_else(|| "cancelled".into())),
    }
}

/// One session per image, so each keeps its own tone stages cached.
struct Bench {
    style: String,
    sessions: Vec<Session>,
}

impl Bench {
    fn new(style: &str) -> Bench {
        let sessions = (images().into_iter())
            .map(|image| {
                let mut s = Session::new(128 << 20);
                s.set_image(image);
                s.set_style(style).unwrap();
                s
            })
            .collect();
        Bench {
            style: style.to_string(),
            sessions,
        }
    }

    /// Put session `i` at the style's defaults with `patch` (`{stage: {key: value}}`) over them.
    fn setup(&mut self, i: usize, patch: &Value) -> &mut Session {
        let s = &mut self.sessions[i];
        s.reset_settings();
        for (stage, values) in patch.as_object().unwrap() {
            s.update(stage, values).unwrap();
        }
        s
    }

    /// The defaults with `patch` drawn on every image.
    fn trial(&mut self, patch: &Value) -> Trial {
        let style = self.style.clone();
        let evals = (0..self.sessions.len())
            .map(|i| {
                let s = self.setup(i, patch);
                let mut run = s.start().unwrap();
                if let Err(e) = drive(&mut run) {
                    // A refused drawing, such as one past the dot budget, is no look.
                    eprintln!("{style} {patch}: {e}");
                    return Eval {
                        minutes: f64::INFINITY,
                        score: None,
                        warnings: vec!["failed"],
                    };
                }
                let mut warnings: Vec<&'static str> = (s.lint().unwrap().iter())
                    .filter(|f| matches!(f.severity, Severity::Warning | Severity::Error))
                    .map(|f| f.code)
                    .collect();
                warnings.dedup();
                Eval {
                    minutes: s.stats().unwrap().estimated_seconds / 60.0,
                    score: s.tone_match().map(|t| ToneMatch::score(&t)),
                    warnings,
                }
            })
            .collect();
        Trial {
            patch: patch.clone(),
            evals,
        }
    }

    /// The value the budget fit finds for [`QUICK_MINUTES`] on each image, from the defaults.
    fn quick(&mut self) -> Vec<f64> {
        (0..self.sessions.len())
            .map(|i| {
                let s = self.setup(i, &json!({}));
                let mut job = s.fit("minutes", QUICK_MINUTES).unwrap();
                match drive(&mut job) {
                    Ok(()) => Job::result(&job).unwrap()["value"].as_f64().unwrap(),
                    Err(_) => f64::NAN,
                }
            })
            .collect()
    }
}

/// `v` on the setting's grid: two significant figures for a count, the 0.1 mm step for a length.
fn tidy(d: &Density, v: f64) -> f64 {
    let v = if d.integer {
        let p = 10f64.powi(v.log10().floor() as i32 - 1).max(1.0);
        (v / p).round() * p
    } else {
        (v * 10.0).round() / 10.0
    };
    v.clamp(d.min, d.max)
}

/// `{stage: {key: v}}` merged over `patch`.
fn with(patch: &Value, d: &Density, v: f64) -> Value {
    let value = if d.integer { json!(v as u64) } else { json!(v) };
    merge(patch, &json!({ d.stage.clone(): { d.key.clone(): value } }))
}

/// The scan's values from [`REACH`] times sparser than `base` to as many times denser, sparsest
/// first, with `base` among them.
fn grid(d: &Density, base: f64) -> Vec<f64> {
    let n = (REACH.ln() / STEP.ln()).round() as i32;
    let mut values: Vec<f64> = (-n..=n).map(|k| tidy(d, base * STEP.powi(k))).collect();
    values.push(base);
    values.sort_by(|a, b| a.total_cmp(b));
    values.dedup();
    if !d.direct {
        values.reverse();
    }
    values
}

/// What one style's measurement found: its tables, and its looks as `{name: settings}`.
struct Found {
    table: String,
    looks: Map<String, Value>,
}

fn measure(style: &str) -> Found {
    let mut table = format!(
        "\n### {style}\n\n| look | settings | {} | median score |\n|---|---|{}---|\n",
        NAMES.join(" | "),
        "---|".repeat(NAMES.len())
    );
    let mut looks = Map::new();
    let mut bench = Bench::new(style);
    let balanced = bench.trial(&json!({}));
    table += &balanced.row("balanced");
    let st = bench.sessions[0].style().clone();
    let Some(d) = Density::of(&st) else {
        table += "\nNo density setting.\n";
        return Found { table, looks };
    };
    let base = st.defaults()[&d.stage][&d.key].as_f64().unwrap();
    let values = grid(&d, base);
    let variants = [json!({}), json!({"tone": {"response": "auto"}})];
    let scans: Vec<Vec<Trial>> = (variants.iter())
        .map(|variant| {
            (values.iter())
                .map(|&v| {
                    let trial = bench.trial(&with(variant, &d, v));
                    table += &trial.row("scan");
                    trial
                })
                .collect()
        })
        .collect();

    // Faithful: the lowest median score of each variant, `auto` only when it is clearly lower.
    let best = |scan: &Vec<Trial>| {
        (scan.iter())
            .filter(|t| t.allowed(&balanced))
            .min_by(|a, b| a.score().total_cmp(&b.score()))
            .cloned()
    };
    let faithful = match (best(&scans[0]), best(&scans[1])) {
        (Some(off), Some(auto)) if auto.score() < AUTO_BETTER * off.score() => Some(auto),
        (Some(off), _) => Some(off),
        (None, auto) => auto,
    };
    let faithful = faithful.filter(|t| t.score() < BETTER * balanced.score());

    // Quick.
    let mut quick = None;
    if balanced.median_minutes() > QUICK_FROM {
        let found = bench.quick();
        let v = tidy(&d, median(found.clone()));
        let v = if v.is_nan() { base } else { v };
        let sparser = if d.direct { v < base } else { v > base };
        let trial = bench.trial(&with(&json!({}), &d, v));
        table += &trial.row(&format!("quick, the fit found {found:?}"));
        if sparser && trial.clean() && trial.score() <= QUICK_WORSE * balanced.score() {
            quick = Some(trial);
        }
    }

    // Fine: the plain scan, denser than the default, while every value passes.
    let at = values.iter().position(|&v| v == base).unwrap();
    let fine = (scans[0][at + 1..].iter())
        .take_while(|t| t.allowed(&balanced) && t.score() <= FINE_WORSE * balanced.score())
        .last()
        .cloned();
    // Fine within a step of Faithful draws as densely, with tones no closer: Faithful stays.
    let place = |t: &Trial| {
        let v = t.patch[&d.stage][&d.key].as_f64().unwrap();
        values.iter().position(|&w| w == v).unwrap()
    };
    let fine = fine.filter(|n| {
        faithful
            .as_ref()
            .is_none_or(|f| place(f).abs_diff(place(n)) > 1)
    });

    for (name, trial) in [("faithful", faithful), ("quick", quick), ("fine", fine)] {
        if let Some(t) = trial {
            table += &t.row(&format!("**{name}**"));
            looks.insert(name.to_string(), t.patch);
        }
    }
    Found { table, looks }
}

fn main() {
    let wanted: Vec<String> = std::env::args().skip(1).collect();
    let styles: Vec<String> = (list_styles().iter())
        .map(|s| s.name.clone())
        .filter(|n| wanted.is_empty() || wanted.contains(n))
        .collect();
    let queue = Arc::new(Mutex::new(styles.clone()));
    let found = Arc::new(Mutex::new(BTreeMap::new()));
    let workers = thread::available_parallelism().map_or(4, |n| n.get());
    let handles: Vec<_> = (0..workers.min(styles.len()))
        .map(|_| {
            let (queue, found) = (queue.clone(), found.clone());
            thread::spawn(move || loop {
                let Some(style) = queue.lock().unwrap().pop() else {
                    return;
                };
                eprintln!("measuring {style}");
                let result = measure(&style);
                eprintln!("measured {style}");
                found.lock().unwrap().insert(style, result);
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let found = found.lock().unwrap();
    let mut chosen = Map::new();
    for style in &styles {
        print!("{}", found[style].table);
        chosen.insert(style.clone(), Value::Object(found[style].looks.clone()));
    }
    println!("\n{}", serde_json::to_string_pretty(&chosen).unwrap());
}
