//! The budget solver: find the value of a style's density setting that meets a target.
//!
//! A stage marks its density setting in its schema with `"x-density"`: `"inverse"` when the
//! drawing gets sparser as the value rises, as with a spacing, and `"direct"` when it gets denser,
//! as with a count. A [`FitRun`] reruns the style with that setting changed, up to [`MAX_TRIALS`]
//! times, until a [`Metric`] of the drawing is within [`TOLERANCE`] of a target, and keeps the
//! trial nearest the target.
//!
//! The search works on `u = ln(value)` and `f(u) = ln(metric / target)`, which is close to a
//! straight line because the metrics follow powers of the density settings. It starts at the
//! current value, steps along the secant until two samples bracket the target, then narrows the
//! bracket with the Illinois variant of regula falsi (M. Dowell and P. Jarratt, "A modified
//! regula falsi method for computing the root of an equation", *BIT* 11(2), 1971, pp. 168-174),
//! bisecting instead where false position is undefined or a sample broke monotonicity. The stop
//! rules count trials, never time, so every target takes the same steps.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};

use crate::cache::StageCache;
use crate::math::{exp, ln};
use crate::runner::{Run, RunState, Shared};
use crate::stage::{Artifact, Frame, Kind};
use crate::stats::{self, Speeds};
use crate::style::Style;
use crate::types::Drawing;

/// The most trial runs one fit makes.
pub const MAX_TRIALS: usize = 8;
/// How close to the target, as a share of it, ends the search early.
pub const TOLERANCE: f64 = 0.02;

/// What a fit measures in a drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Metric {
    /// The estimated plot time in minutes.
    Minutes,
    /// The number of paths.
    Paths,
    /// The pen-down length in metres.
    Metres,
    /// The number of points the style placed.
    Dots,
}

impl Metric {
    /// Every metric, in the order front ends list them.
    pub const ALL: [Metric; 4] = [Metric::Minutes, Metric::Paths, Metric::Metres, Metric::Dots];

    /// The metric called `name`, as [`Metric::name`] gives it.
    pub fn parse(name: &str) -> Option<Metric> {
        Metric::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The lowercase name front ends use.
    pub fn name(self) -> &'static str {
        match self {
            Metric::Minutes => "minutes",
            Metric::Paths => "paths",
            Metric::Metres => "metres",
            Metric::Dots => "dots",
        }
    }

    /// `x` with its unit, for a progress label.
    fn show(self, x: f64) -> String {
        match self {
            Metric::Minutes => format!("{x:.1} min"),
            Metric::Paths => format!("{x} paths"),
            Metric::Metres => format!("{x:.2} m"),
            Metric::Dots => format!("{x} dots"),
        }
    }
}

/// A style's density setting: the stage and key marked `x-density`, which way the drawing's
/// density follows it, and its range from the schema.
#[derive(Clone, Debug, PartialEq)]
pub struct Density {
    /// The stage's name.
    pub stage: String,
    /// The setting's key.
    pub key: String,
    /// True when the drawing gets denser as the value rises, false when it gets sparser.
    pub direct: bool,
    /// The least value the schema allows.
    pub min: f64,
    /// The greatest value the schema allows.
    pub max: f64,
    /// True for a whole-number setting.
    pub integer: bool,
}

impl Density {
    /// The first setting of `style`'s stages marked `x-density` with `"inverse"` or `"direct"`
    /// and a range, or `None` when it has none.
    pub fn of(style: &Style) -> Option<Density> {
        style.stages.iter().find_map(|stage| {
            let schema = stage.schema();
            schema["properties"]
                .as_object()?
                .iter()
                .find_map(|(key, p)| {
                    let direct = match p["x-density"].as_str()? {
                        "direct" => true,
                        "inverse" => false,
                        _ => return None,
                    };
                    Some(Density {
                        stage: stage.name().to_string(),
                        key: key.clone(),
                        direct,
                        min: p["minimum"].as_f64()?,
                        max: p["maximum"].as_f64()?,
                        integer: p["type"] == "integer",
                    })
                })
        })
    }

    /// `v` clamped to the range and rounded to the setting's grain: a whole number for an
    /// integer, 0.001 otherwise.
    fn grain(&self, v: f64) -> f64 {
        let v = if self.integer {
            v.round()
        } else {
            (v * 1000.0).round() / 1000.0
        };
        v.clamp(self.min, self.max)
    }

    /// `v` as the JSON the setting takes.
    fn to_json(&self, v: f64) -> Value {
        if self.integer {
            json!(v as u64)
        } else {
            json!(v)
        }
    }
}

/// One trial: the value tried and the metric its drawing gave.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Sample {
    /// The setting's value.
    pub value: f64,
    /// The metric of the drawing at that value.
    pub metric: f64,
}

/// The search over one setting, apart from any run: [`Solver::next`] proposes a value and
/// [`Solver::record`] takes the metric it gave. See the module docs for the rules.
#[derive(Clone, Debug)]
pub struct Solver {
    density: Density,
    target: f64,
    samples: Vec<Sample>,
    /// The value to try next, or `None` once the search is over.
    next: Option<f64>,
    /// The bracket's ends as `(u, f)` once two samples straddle the target; an end's `f` may have
    /// been halved by the Illinois rule.
    bracket: Option<[(f64, f64); 2]>,
    /// The end the last false-position step replaced, which the Illinois rule watches.
    replaced: Option<usize>,
}

impl Solver {
    /// A search for `target` that first tries `current`, clamped and rounded to the setting.
    pub fn new(density: Density, current: f64, target: f64) -> Solver {
        let first = density.grain(current);
        Solver {
            density,
            target,
            samples: Vec::new(),
            next: Some(first),
            bracket: None,
            replaced: None,
        }
    }

    /// The value to try next, or `None` once the search is over.
    pub fn next(&self) -> Option<f64> {
        self.next
    }

    /// The trials so far, in order.
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// The trial nearest the target in ratio; ties go to the earlier one.
    pub fn best(&self) -> Option<Sample> {
        let mut best: Option<Sample> = None;
        for &s in &self.samples {
            if best.is_none_or(|b| self.f(s.metric).abs() < self.f(b.metric).abs()) {
                best = Some(s);
            }
        }
        best
    }

    /// Record that the value [`Solver::next`] proposed gave `metric`, and choose the next value.
    /// Panics when the search is over.
    pub fn record(&mut self, metric: f64) {
        let value = self.next.take().expect("record follows a proposed value");
        self.samples.push(Sample { value, metric });
        let f = self.f(metric);
        if f.abs() <= ln(1.0 + TOLERANCE) || self.samples.len() >= MAX_TRIALS {
            return;
        }
        let u = ln(value);
        let bisect = self.broke_monotonicity();
        let next = match self.bracket {
            Some(mut ends) => {
                // The new sample lies inside the bracket and replaces the end on its side.
                let side = usize::from((f < 0.0) != (ends[0].1 < 0.0));
                ends[side] = (u, f);
                if !bisect && self.replaced == Some(side) {
                    ends[1 - side].1 /= 2.0;
                }
                self.replaced = (!bisect).then_some(side);
                self.bracket = Some(ends);
                self.inside(ends, bisect)
            }
            None => match self.straddle(u, f) {
                Some(other) => {
                    let ends = [other, (u, f)];
                    self.bracket = Some(ends);
                    self.inside(ends, bisect)
                }
                None => self.outward(),
            },
        };
        let value = self.density.grain(exp(next));
        if self.samples.iter().all(|s| s.value != value) {
            self.next = Some(value);
        }
    }

    /// `ln(metric / target)`: 0 on target, below 0 under it, `-inf` for an empty drawing.
    fn f(&self, metric: f64) -> f64 {
        ln(metric) - ln(self.target)
    }

    /// The sign the metric's slope against `u` has: +1 for a direct setting, -1 for an inverse.
    fn sign(&self) -> f64 {
        if self.density.direct {
            1.0
        } else {
            -1.0
        }
    }

    /// Whether the newest sample's metric is out of order with its neighbours by value.
    fn broke_monotonicity(&self) -> bool {
        let newest = *self.samples.last().expect("a sample was just recorded");
        let below = (self.samples.iter())
            .filter(|s| s.value < newest.value)
            .max_by(|a, b| a.value.total_cmp(&b.value));
        let above = (self.samples.iter())
            .filter(|s| s.value > newest.value)
            .min_by(|a, b| a.value.total_cmp(&b.value));
        // Rising values must give rising metrics for a direct setting, falling for an inverse.
        let rises = |lo: &Sample, hi: &Sample| (hi.metric - lo.metric) * self.sign() >= 0.0;
        below.is_some_and(|b| !rises(b, &newest)) || above.is_some_and(|a| !rises(&newest, a))
    }

    /// The sample on the other side of the target from `(u, f)` nearest to it in `u`, as `(u, f)`.
    fn straddle(&self, u: f64, f: f64) -> Option<(f64, f64)> {
        (self.samples.iter())
            .map(|s| (ln(s.value), self.f(s.metric)))
            .filter(|&(_, g)| (g < 0.0) != (f < 0.0))
            .min_by(|a, b| (a.0 - u).abs().total_cmp(&(b.0 - u).abs()))
    }

    /// The next `u` inside the bracket: false position, or the midpoint when `bisect` is set or
    /// an end is infinite.
    fn inside(&self, [(ua, fa), (ub, fb)]: [(f64, f64); 2], bisect: bool) -> f64 {
        if bisect || !fa.is_finite() || !fb.is_finite() {
            (ua + ub) / 2.0
        } else {
            ub - fb * (ub - ua) / (fb - fa)
        }
    }

    /// The next `u` while every sample lies on one side of the target: a secant step from the
    /// sample nearest the target, clamped to the range. From an empty drawing, halfway to the
    /// range end that adds marks. When a sample already lies past the nearest one in the
    /// direction that should bring the metric closer, the metric fell off a cliff between them,
    /// as it cannot in a monotone curve, so the step goes halfway to it instead.
    fn outward(&self) -> f64 {
        let (lo, hi) = (ln(self.density.min), ln(self.density.max));
        let mut near: Vec<(f64, f64)> = (self.samples.iter())
            .map(|s| (ln(s.value), self.f(s.metric)))
            .collect();
        near.sort_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
        let (u0, f0) = near[0];
        if !f0.is_finite() {
            let end = if self.density.direct { hi } else { lo };
            return (u0 + end) / 2.0;
        }
        // Raising the metric means raising a direct setting and lowering an inverse one.
        let toward = if f0 < 0.0 { self.sign() } else { -self.sign() };
        let cliff = (near.iter())
            .filter(|&&(u, _)| (u - u0) * toward > 0.0)
            .min_by(|a, b| (a.0 - u0).abs().total_cmp(&(b.0 - u0).abs()));
        if let Some(&(u1, _)) = cliff {
            return (u0 + u1) / 2.0;
        }
        let measured = near.get(1).map(|&(u1, f1)| (f0 - f1) / (u0 - u1));
        let slope = match measured {
            Some(s) if s * self.sign() > 0.0 && (0.25..=4.0).contains(&s.abs()) => s,
            _ => self.sign(),
        };
        (u0 - f0 / slope).clamp(lo, hi)
    }
}

/// What a finished fit chose.
#[derive(Clone, Debug, Serialize)]
pub struct FitResult {
    /// The stage of the setting that was fitted.
    pub stage: String,
    /// The setting's key.
    pub key: String,
    /// What was measured.
    pub metric: Metric,
    /// The target.
    pub target: f64,
    /// The chosen value, as the JSON the setting takes.
    pub value: Value,
    /// The metric at the chosen value.
    pub achieved: f64,
    /// Every trial, in order.
    pub samples: Vec<Sample>,
}

/// A fit over a snapshot of a session, driven like a [`Run`] one [`FitRun::next_frame`] at a time.
///
/// Each trial is a run of the style with the density setting changed, sharing the session's
/// cache and cancel flag and writing a slot of its own, so the session's slot shows the fit as
/// running until it ends. A trial's frames pass through with their label prefixed by the trial,
/// and after each trial comes a frame of stage `fit` with the metric it gave. At the end the
/// nearest trial's drawing goes to the session's slot.
pub struct FitRun {
    style: Arc<Style>,
    settings: Value,
    roots: [Artifact; 3],
    cache: Arc<Mutex<StageCache>>,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    generation: u64,
    speeds: Speeds,
    metric: Metric,
    solver: Solver,
    /// The trial under way and its own slot.
    trial: Option<(Run, Arc<Mutex<Shared>>)>,
    /// The drawing of the solver's best trial.
    best: Option<Arc<Drawing>>,
    state: RunState,
    error: Option<String>,
}

impl FitRun {
    /// A fit that has not started: `settings` and `roots` as [`Run::new`] takes them, and the
    /// search for `target` of `metric` over `density`, starting from its value in `settings`.
    #[allow(clippy::too_many_arguments)] // the run's arguments plus the search's
    pub fn new(
        style: Arc<Style>,
        settings: Value,
        roots: [Artifact; 3],
        cache: Arc<Mutex<StageCache>>,
        shared: Arc<Mutex<Shared>>,
        cancel: Arc<AtomicBool>,
        generation: u64,
        speeds: Speeds,
        density: Density,
        metric: Metric,
        target: f64,
    ) -> FitRun {
        let current = settings[&density.stage][&density.key]
            .as_f64()
            .unwrap_or(density.min);
        FitRun {
            style,
            settings,
            roots,
            cache,
            shared,
            cancel,
            generation,
            speeds,
            metric,
            solver: Solver::new(density, current, target),
            trial: None,
            best: None,
            state: RunState::Idle,
            error: None,
        }
    }

    /// Do work until there is a frame to show, and return it; `None` once the fit has ended,
    /// after writing the end state, and on success the nearest trial's drawing, to the session.
    pub fn next_frame(&mut self) -> Option<Frame> {
        match self.state {
            RunState::Idle => self.state = RunState::Running,
            RunState::Running => {}
            RunState::Done | RunState::Cancelled | RunState::Error => return None,
        }
        let n = self.solver.samples().len() + 1;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                self.end(RunState::Cancelled, None);
                return None;
            }
            if let Some((run, slot)) = &mut self.trial {
                if let Some(mut frame) = run.next_frame() {
                    frame.label = format!("trial {n} of {MAX_TRIALS}: {}", frame.label);
                    return Some(frame);
                }
                match run.state() {
                    RunState::Done => {}
                    RunState::Cancelled => {
                        self.end(RunState::Cancelled, None);
                        return None;
                    }
                    _ => {
                        let error = run.error();
                        self.end(RunState::Error, error);
                        return None;
                    }
                }
                let drawing = slot.lock().unwrap().drawing.clone();
                let drawing = drawing.expect("a finished run leaves its drawing");
                let stats = stats::compute(&drawing, self.speeds);
                let metric = match self.metric {
                    Metric::Minutes => stats.estimated_seconds / 60.0,
                    Metric::Paths => stats.path_count as f64,
                    Metric::Metres => stats.pen_down_mm / 1000.0,
                    Metric::Dots => run.point_count().unwrap_or(0) as f64,
                };
                self.trial = None;
                self.solver.record(metric);
                if self.solver.best().map(|b| b.value) == Some(self.solver.samples()[n - 1].value) {
                    self.best = Some(drawing);
                }
                return Some(Frame {
                    stage: "fit".to_string(),
                    progress: n as f32 / MAX_TRIALS as f32,
                    label: format!("trial {n} of {MAX_TRIALS}: {}", self.metric.show(metric)),
                    points: None,
                    paths: None,
                    pen: 0,
                });
            }
            let Some(value) = self.solver.next() else {
                self.end(RunState::Done, None);
                return None;
            };
            let d = &self.solver.density;
            let mut settings = self.settings.clone();
            settings[&d.stage][&d.key] = d.to_json(value);
            let slot = Arc::<Mutex<Shared>>::default();
            let run = Run::new(
                self.style.clone(),
                settings,
                self.roots.clone(),
                self.cache.clone(),
                slot.clone(),
                self.cancel.clone(),
                0,
            );
            self.trial = Some((run, slot));
        }
    }

    /// Ask the fit to stop before its next step.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The flag [`FitRun::cancel`] sets.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Where the fit is.
    pub fn state(&self) -> RunState {
        self.state
    }

    /// The failing trial's message, once the fit has ended in [`RunState::Error`].
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// What the fit chose, once it has ended in [`RunState::Done`].
    pub fn result(&self) -> Option<FitResult> {
        if self.state != RunState::Done {
            return None;
        }
        let best = self.solver.best()?;
        let d = &self.solver.density;
        Some(FitResult {
            stage: d.stage.clone(),
            key: d.key.clone(),
            metric: self.metric,
            target: self.solver.target,
            value: d.to_json(best.value),
            achieved: best.metric,
            samples: self.solver.samples().to_vec(),
        })
    }

    /// Set the fit's end state and, while the session's slot is still this fit's generation, copy
    /// it there with the best drawing on success.
    fn end(&mut self, state: RunState, error: Option<String>) {
        let mut shared = self.shared.lock().unwrap();
        if shared.generation == self.generation {
            if state == RunState::Done {
                shared.drawing = self.best.clone();
            }
            shared.state = state;
            shared.error = error.clone();
        }
        drop(shared);
        self.state = state;
        self.error = error;
    }
}

/// What a host drives a frame at a time, a [`Run`] or a [`FitRun`], so one driver serves both.
pub trait Job: Send {
    /// The next frame, or `None` once the job has ended.
    fn next_frame(&mut self) -> Option<Frame>;
    /// The flag that cancels the job, which a host may set from another thread.
    fn cancel_flag(&self) -> Arc<AtomicBool>;
    /// Where the job is.
    fn state(&self) -> RunState;
    /// The message of a job that ended in [`RunState::Error`].
    fn error(&self) -> Option<String>;
    /// What a fit chose once it is done; always `None` for a run.
    fn fit(&self) -> Option<FitResult> {
        None
    }
}

impl Job for Run {
    fn next_frame(&mut self) -> Option<Frame> {
        Run::next_frame(self)
    }
    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Run::cancel_flag(self)
    }
    fn state(&self) -> RunState {
        Run::state(self)
    }
    fn error(&self) -> Option<String> {
        Run::error(self)
    }
}

impl Job for FitRun {
    fn next_frame(&mut self) -> Option<Frame> {
        FitRun::next_frame(self)
    }
    fn cancel_flag(&self) -> Arc<AtomicBool> {
        FitRun::cancel_flag(self)
    }
    fn state(&self) -> RunState {
        FitRun::state(self)
    }
    fn error(&self) -> Option<String> {
        FitRun::error(self)
    }
    fn fit(&self) -> Option<FitResult> {
        self.result()
    }
}

/// Whether `style` has a stage that places points, which the [`Metric::Dots`] metric counts.
pub fn places_points(style: &Style) -> bool {
    style.stages.iter().any(|s| s.provides() == Kind::Points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Session, SessionError};
    use crate::synthetic::gradient;
    use crate::types::{Canvas, Fit as Mode};

    fn density(direct: bool, min: f64, max: f64, integer: bool) -> Density {
        Density {
            stage: "s".into(),
            key: "k".into(),
            direct,
            min,
            max,
            integer,
        }
    }

    /// Run the solver on `metric` and return its samples.
    fn solve(d: Density, start: f64, target: f64, metric: impl Fn(f64) -> f64) -> Solver {
        let mut s = Solver::new(d, start, target);
        while let Some(v) = s.next() {
            s.record(metric(v));
        }
        s
    }

    fn within(s: &Solver, target: f64) -> bool {
        (s.best().unwrap().metric / target - 1.0).abs() <= TOLERANCE
    }

    #[test]
    fn power_laws_land_within_tolerance() {
        let spacing = || density(false, 0.2, 10.0, false);
        let count = density(true, 100.0, 50000.0, true);
        // Pen-down length against a spacing, dots against a spacing, points against a count.
        for s in [
            solve(spacing(), 1.0, 370.0, |v| 600.0 / v),
            solve(spacing(), 1.0, 12345.0, |v| 2.0e5 / (v * v)),
            solve(count, 5000.0, 20000.0, |v| 0.93 * v),
        ] {
            let target = s.target;
            assert!(within(&s, target), "{:?}", s.samples());
            assert!(s.samples().len() <= 4, "{:?}", s.samples());
        }
    }

    #[test]
    fn stairs_and_a_sawtooth_still_converge() {
        // A threshold style's staircase.
        let stairs = |v: f64| 1000.0 / ((v * 4.0).floor() / 4.0 + 0.25);
        let s = solve(density(false, 0.2, 10.0, false), 1.0, 700.0, stairs);
        assert!(s.samples().len() <= MAX_TRIALS);
        assert!(
            (s.best().unwrap().metric / 700.0 - 1.0).abs() < 0.2,
            "{:?}",
            s.samples()
        );
        // A style that draws almost nothing below 0.6 mm, as hatch on a coarse tone map does.
        let cliff = |v: f64| if v < 0.6 { 0.04 } else { 3.6 / v };
        let s = solve(density(false, 0.2, 10.0, false), 1.0, 6.5, cliff);
        assert!(s.best().unwrap().value < 0.62, "{:?}", s.samples());
        // A sawtooth that breaks monotonicity every 0.2 mm.
        let saw = |v: f64| 100.0 / (v * v) * (1.0 + 0.3 * (5.0 * v).fract());
        let s = solve(density(false, 0.2, 10.0, false), 3.0, 100.0, saw);
        assert!(s.samples().len() <= MAX_TRIALS);
        assert!(within(&s, 100.0), "{:?}", s.samples());
        // 80 falls in the jump at 1.2 mm, which no value reaches, so the search closes on the jump.
        let s = solve(density(false, 0.2, 10.0, false), 3.0, 80.0, saw);
        assert_eq!(s.samples().len(), MAX_TRIALS);
        let near = |v: f64| (s.samples().iter()).any(|x| (x.value - v).abs() < 0.01);
        assert!(near(1.195) && near(1.205), "{:?}", s.samples());
    }

    #[test]
    fn an_unreachable_target_stops_at_the_range_end() {
        let s = solve(density(false, 0.2, 10.0, false), 1.0, 1e9, |v| 100.0 / v);
        assert_eq!(s.best().unwrap().value, 0.2);
        assert!(s.samples().len() <= 4, "{:?}", s.samples());
        let s = solve(density(false, 0.2, 10.0, false), 1.0, 1e-9, |v| 100.0 / v);
        assert_eq!(s.best().unwrap().value, 10.0);
    }

    #[test]
    fn an_empty_drawing_steps_towards_marks() {
        // Nothing is drawn above a spacing of 4.
        let s = solve(density(false, 0.2, 10.0, false), 6.0, 50.0, |v| {
            if v > 4.0 {
                0.0
            } else {
                100.0 / v
            }
        });
        assert!(within(&s, 50.0), "{:?}", s.samples());
    }

    #[test]
    fn integers_round_and_stop_when_a_value_repeats() {
        // Levels give paths in steps of 37.
        let s = solve(density(true, 1.0, 64.0, true), 8.0, 500.0, |v| 37.0 * v);
        assert!(s.samples().iter().all(|x| x.value.fract() == 0.0));
        let best = s.best().unwrap();
        assert!((best.metric - 500.0).abs() <= 37.0, "{:?}", s.samples());
        assert!(s.samples().len() <= MAX_TRIALS);
    }

    #[test]
    fn the_first_trial_is_the_current_value_and_can_be_enough() {
        let s = solve(density(false, 0.2, 10.0, false), 1.2345, 81.0, |v| {
            100.0 / v
        });
        assert_eq!(s.samples()[0].value, 1.235);
        assert_eq!(s.samples().len(), 1);
    }

    /// A session over a 64 by 48 gradient on a 100 by 75 mm page, with `style` and `patches`.
    fn session(style: &str, patches: &[(&str, Value)]) -> Session {
        let mut s = Session::new(256 << 20);
        s.set_image(gradient(64, 48));
        s.set_canvas(Canvas::new(100.0, 75.0, 5.0, Mode::Fit).unwrap());
        s.set_style(style).unwrap();
        s.update("tone", &json!({"resolution": 64})).unwrap();
        for (stage, patch) in patches {
            s.update(stage, patch).unwrap();
        }
        s
    }

    /// Fit `s` to `target` of `metric` and return the result and the frames.
    fn fit(s: &mut Session, metric: &str, target: f64) -> (FitResult, Vec<Frame>) {
        let mut f = s.fit(metric, target).unwrap();
        let mut frames = Vec::new();
        while let Some(frame) = f.next_frame() {
            frames.push(frame);
        }
        assert_eq!(f.state(), RunState::Done, "{:?}", f.error());
        assert_eq!(s.state(), RunState::Done);
        (f.result().unwrap(), frames)
    }

    #[test]
    fn every_metric_lands_near_its_target() {
        let cases = [
            ("hatch", vec![], "minutes", 5.0),
            ("waves", vec![], "metres", 10.0),
            ("contours", vec![], "paths", 30.0),
            (
                "stipple",
                vec![("voronoi", json!({"count": 600, "iterations": 5}))],
                "dots",
                1500.0,
            ),
        ];
        for (style, patches, metric, target) in cases {
            let mut s = session(style, &patches);
            let (r, frames) = fit(&mut s, metric, target);
            let ratio = r.achieved / target;
            println!("{style} {metric}: {:?}", r.samples);
            assert!(r.samples.len() <= MAX_TRIALS, "{style}: {:?}", r.samples);
            if style == "contours" {
                // Levels are whole, so the paths move in steps.
                assert!((ratio - 1.0).abs() < 0.2, "{style}: {:?}", r.samples);
            } else {
                assert!((ratio - 1.0).abs() <= TOLERANCE, "{style}: {:?}", r.samples);
            }
            // The session's drawing is the chosen trial's.
            let stats = s.stats().unwrap();
            let shown = match metric {
                "minutes" => stats.estimated_seconds / 60.0,
                "metres" => stats.pen_down_mm / 1000.0,
                _ => r.achieved,
            };
            assert_eq!(shown, r.achieved);
            // One summary frame per trial, and the tone map is made once.
            let fits = frames.iter().filter(|f| f.stage == "fit").count();
            assert_eq!(fits, r.samples.len());
            let tone = frames
                .iter()
                .filter(|f| f.stage == "tone" && !f.label.ends_with("cached"));
            assert!(tone.count() <= 1);
            assert!(frames.iter().all(|f| f.label.starts_with("trial ")));
        }
    }

    #[test]
    fn a_fit_applies_with_update_and_reruns_from_the_cache() {
        let mut s = session("hatch", &[]);
        let (r, _) = fit(&mut s, "minutes", 5.0);
        s.update(&r.stage, &json!({ &r.key: r.value })).unwrap();
        let mut run = s.start().unwrap();
        let mut labels = Vec::new();
        while let Some(f) = run.next_frame() {
            labels.push(f.label);
        }
        assert!(labels.iter().all(|l| l == "cached"), "{labels:?}");
        assert_eq!(s.stats().unwrap().estimated_seconds / 60.0, r.achieved);
    }

    #[test]
    fn the_samples_are_pinned() {
        // The same literals are in crates/wasm/tests/smoke.rs, so CI proves both targets agree.
        let mut s = session("waves", &[]);
        let (r, _) = fit(&mut s, "metres", 10.0);
        let values: Vec<f64> = r.samples.iter().map(|x| x.value).collect();
        assert_eq!(values, [2.0, 1.472, 0.655, 0.883, 0.843]);
        assert_eq!(r.achieved, 9.934812216679127);
    }

    #[test]
    fn a_cancel_or_a_new_run_ends_the_fit() {
        let mut s = session("hatch", &[]);
        let mut f = s.fit("minutes", 6.0).unwrap();
        f.next_frame();
        f.cancel();
        assert!(f.next_frame().is_none());
        assert_eq!(f.state(), RunState::Cancelled);
        assert_eq!(s.state(), RunState::Cancelled);
        assert!(f.result().is_none());
        let mut f = s.fit("minutes", 6.0).unwrap();
        f.next_frame();
        let mut run = s.start().unwrap();
        assert!(f.next_frame().is_none());
        assert_eq!(f.state(), RunState::Cancelled);
        assert_eq!(s.state(), RunState::Running); // the cancelled fit left the slot alone
        while run.next_frame().is_some() {}
        assert_eq!(s.state(), RunState::Done);
    }

    #[test]
    fn bad_fits_are_refused_before_anything_changes() {
        let mut s = Session::new(1 << 20);
        assert!(matches!(s.fit("minutes", 1.0), Err(SessionError::NoImage)));
        let mut s = session("hatch", &[]);
        let before = s.settings().clone();
        let message = |s: &mut Session, m: &str, t: f64| s.fit(m, t).err().unwrap().to_string();
        assert!(message(&mut s, "hours", 1.0).contains("minutes, paths, metres, dots"));
        assert!(message(&mut s, "paths", 0.0).contains("above 0"));
        assert!(message(&mut s, "paths", f64::NAN).contains("above 0"));
        assert!(message(&mut s, "dots", 10.0).contains("places no dots"));
        s.set_style("ink").unwrap();
        assert!(message(&mut s, "paths", 10.0).contains("no density setting"));
        s.set_style("hatch").unwrap();
        s.update("tone", &json!({"resolution": 64})).unwrap();
        assert_eq!(s.settings(), &before);
        assert_eq!(s.state(), RunState::Idle);
    }

    #[test]
    fn metrics_parse_by_name() {
        for m in Metric::ALL {
            assert_eq!(Metric::parse(m.name()), Some(m));
        }
        assert_eq!(Metric::parse("hours"), None);
    }
}
