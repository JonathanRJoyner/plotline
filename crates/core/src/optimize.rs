//! Optimize: the value of a style's density setting whose drawing follows its tone map most
//! closely for its plot time, within its limits, offered as a suggestion and never applied.
//!
//! The density setting is the one the budget solver fits ([`Density`]). Its tone error
//! ([`crate::tone_match`]) has a low inside its range: lines too far apart cannot show the tones,
//! and lines too close run together until the midtones turn black. Across the range the error has
//! more than one dip, so the search is a scan rather than a bracketing search: a first trial at
//! the current value, then a grid on `ln(value)` in steps of half an octave from a quarter to four
//! times it, within the schema's range, for at most [`TRIALS`] runs. A value the grid gives twice
//! after rounding to the setting's grain runs once.
//!
//! A trial is left out, scoring worse than any other, when its run fails (a denser value can pass
//! a style's vertex budget), when its drawing leaves nothing to compare, goes over the budget, plots for longer than [`SLOWER`] times the current value's time or
//! [`LONGEST`] minutes, whichever is longer, or has a plot lint warning or error the current
//! value's drawing does not. The time limit and the lint rule are the looks' limits (see
//! `docs/decisions/2026-10-16-looks.md`), measured against the current value instead of the
//! defaults; a budget in minutes replaces the time limit. The current value is never too slow and
//! never newly warned, so only a budget can leave it out. The stop rule counts trials, never
//! time, so every target takes the same steps.
//!
//! The best trial is the quickest to plot of those whose tone error is within [`CLOSE`] of the
//! lowest. Near its low the error is flat, and neighbouring values differ by up to a fifth as the
//! marks fall differently on the image's detail, so the lowest error alone buys accidents with
//! plot time (see `docs/decisions/2026-10-16-optimize-limits.md`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};

use crate::cache::StageCache;
use crate::fit::{Density, Metric};
use crate::lint::{self, Severity};
use crate::math::{exp, ln};
use crate::runner::{Run, RunState, Shared};
use crate::stage::{Artifact, Frame};
use crate::stats::{self, Speeds};
use crate::style::Style;
use crate::types::Drawing;

/// The most runs one search makes: the current value, then [`STEPS`] on each side of it.
pub const TRIALS: usize = 9;
/// The grid's steps on each side of the current value, each half an octave, so the grid reaches
/// a quarter and four times the current value.
const STEPS: i32 = 4;
/// A trial may plot for this many times the current value's time, or [`LONGEST`] minutes if that
/// is more.
pub const SLOWER: f64 = 2.0;
pub const LONGEST: f64 = 60.0;
/// A trial whose tone error is within this share of the lowest follows the tones as closely: a
/// tenth, the margin by which a look must beat Balanced (`docs/decisions/2026-10-16-looks.md`).
pub const CLOSE: f64 = 0.1;

/// A budget the drawing must stay within: `metric` at most `target`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Budget {
    pub metric: Metric,
    pub target: f64,
}

/// One trial: the value tried, its drawing's tone error if there was anything to compare, its
/// plot time, the budget's metric when there is a budget, and what left it out, if anything.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Trial {
    pub value: f64,
    pub error: Option<f64>,
    /// The estimated plot time in minutes, `None` when the run failed.
    pub minutes: Option<f64>,
    pub metric: Option<f64>,
    /// Whether the drawing went over the budget.
    pub over: bool,
    /// Whether it plots for longer than the time limit allows.
    pub slow: bool,
    /// The codes of the plot lint warnings and errors it has that the current value's drawing
    /// does not.
    pub warnings: Vec<&'static str>,
    /// Why the run made no drawing, such as a dot budget it went past.
    pub failed: Option<String>,
}

impl Trial {
    /// The score the search compares: the tone error, or infinity for a trial left out.
    fn score(&self) -> f64 {
        match self.error {
            Some(e) if !self.over && !self.slow && self.warnings.is_empty() => e,
            _ => f64::INFINITY,
        }
    }
}

/// What a finished search found.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Optimum {
    /// The stage and key of the setting searched.
    pub stage: String,
    pub key: String,
    /// The best value, as the JSON the setting takes, its tone error and its plot time in minutes.
    pub value: Value,
    pub error: f64,
    pub minutes: f64,
    /// The current value, its tone error if it had one within the budget, and its plot time.
    pub current: Value,
    pub current_error: Option<f64>,
    pub current_minutes: f64,
    pub budget: Option<Budget>,
    /// The time limit in minutes, `None` when a budget in minutes replaced it.
    pub limit_minutes: Option<f64>,
    /// The trial left out only for its plot time with the lowest tone error, when that error is
    /// more than [`CLOSE`] under every trial's within the limits.
    pub beyond: Option<Trial>,
    /// Every trial, in order.
    pub trials: Vec<Trial>,
    /// A sentence for people, and the patch that applies the best value, `{"settings": {stage:
    /// {key: value}}}` as an analysis suggestion's, or `None` when the current value is best.
    pub reason: String,
    pub patch: Option<Value>,
}

/// The search apart from any run: [`Search::next`] gives the value to try and [`Search::record`]
/// takes its trial.
#[derive(Clone, Debug)]
pub struct Search {
    density: Density,
    /// The values to try, the current one first.
    values: Vec<f64>,
    /// Whether the time limit applies, which a budget in minutes replaces.
    timed: bool,
    /// The plot lint warnings and errors of the current value's drawing.
    known: Vec<&'static str>,
    trials: Vec<Trial>,
}

impl Search {
    /// A search over `density` around `current`, with the time limit when `timed`.
    pub fn new(density: Density, current: f64, timed: bool) -> Search {
        // The current value as set, unrounded, so the first trial's drawing is the settings'.
        let mut values = vec![current];
        let base = ln(current.max(f64::MIN_POSITIVE));
        for k in (-STEPS..=STEPS).filter(|&k| k != 0) {
            let v = density.grain(exp(base + f64::from(k) * 0.5 * ln(2.0)));
            if !values.contains(&v) {
                values.push(v);
            }
        }
        Search {
            density,
            values,
            timed,
            known: Vec::new(),
            trials: Vec::new(),
        }
    }

    /// The value to try next, or `None` once every value has run.
    pub fn next(&self) -> Option<f64> {
        self.values.get(self.trials.len()).copied()
    }

    /// Record the trial of the value [`Search::next`] gave, with every lint warning and error its
    /// drawing has: the first trial's become the known ones, and a later trial keeps only those
    /// it adds and is marked slow when it breaks the time limit.
    pub fn record(&mut self, mut trial: Trial) {
        match self.trials.first() {
            None => self.known = std::mem::take(&mut trial.warnings),
            Some(_) => {
                let limit = self.limit();
                trial.slow = (trial.minutes.zip(limit)).is_some_and(|(m, l)| m > l);
                trial.warnings.retain(|w| !self.known.contains(w));
            }
        }
        self.trials.push(trial);
    }

    /// The trials so far, in order.
    pub fn trials(&self) -> &[Trial] {
        &self.trials
    }

    /// The time limit in minutes once the first trial has run, `None` without one.
    pub fn limit(&self) -> Option<f64> {
        let first = self.trials.first().filter(|_| self.timed)?;
        Some((SLOWER * first.minutes.unwrap_or(0.0)).max(LONGEST))
    }

    /// The lowest score, infinite when no trial has one.
    fn lowest(&self) -> f64 {
        self.trials
            .iter()
            .map(Trial::score)
            .fold(f64::INFINITY, f64::min)
    }

    /// The quickest trial whose score is within [`CLOSE`] of the lowest, the earliest of equals;
    /// `None` when none has a score.
    pub fn best(&self) -> Option<&Trial> {
        let near = (1.0 + CLOSE) * self.lowest();
        self.trials
            .iter()
            .filter(|t| t.score().is_finite() && t.score() <= near)
            .fold(None, |best: Option<&Trial>, t| match best {
                Some(b) if b.minutes <= t.minutes => Some(b),
                _ => Some(t),
            })
    }

    /// The trial left out only for its plot time with the lowest tone error, when that error is
    /// more than [`CLOSE`] under the lowest score, so that it would have been best without the
    /// limit; the earliest of equals.
    pub fn beyond(&self) -> Option<&Trial> {
        let slow = |t: &&Trial| t.slow && !t.over && t.warnings.is_empty() && t.error.is_some();
        (self.trials.iter().filter(slow))
            .fold(None, |best: Option<&Trial>, t| match best {
                Some(b) if b.error <= t.error => Some(b),
                _ => Some(t),
            })
            .filter(|t| t.error.is_some_and(|e| (1.0 + CLOSE) * e < self.lowest()))
    }
}

/// A search over a snapshot of a session, driven like a [`Run`] one
/// [`OptimizeRun::next_frame`] at a time.
///
/// Each trial is a run of the style with the density setting changed, sharing the session's
/// cache and cancel flag and writing a slot of its own. A trial's frames pass through with their
/// label prefixed by the trial, and after each trial comes a frame of stage `optimize` with its
/// tone error, its plot time and anything that left it out. A later trial whose run fails is left
/// out with its message; the first is the current value, so its failure ends the search, as a run
/// of the current settings would. At the end the first trial's drawing goes to the session's slot,
/// so the drawing still matches the settings, which the search leaves alone.
pub struct OptimizeRun {
    style: Arc<Style>,
    settings: Value,
    roots: [Artifact; 3],
    cache: Arc<Mutex<StageCache>>,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    generation: u64,
    speeds: Speeds,
    budget: Option<Budget>,
    search: Search,
    /// The trial under way, its own slot and its settings.
    trial: Option<(Run, Arc<Mutex<Shared>>, Value)>,
    /// The first trial's drawing, made at the current settings.
    first: Option<Arc<Drawing>>,
    state: RunState,
    error: Option<String>,
}

impl OptimizeRun {
    /// A search that has not started: `settings` and `roots` as [`Run::new`] takes them, over
    /// `density` from its value in `settings`, within `budget` if there is one.
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
        budget: Option<Budget>,
    ) -> OptimizeRun {
        let current = settings[&density.stage][&density.key]
            .as_f64()
            .unwrap_or(density.min);
        OptimizeRun {
            style,
            settings,
            roots,
            cache,
            shared,
            cancel,
            generation,
            speeds,
            budget,
            search: Search::new(
                density,
                current,
                !matches!(budget, Some(b) if b.metric == Metric::Minutes),
            ),
            trial: None,
            first: None,
            state: RunState::Idle,
            error: None,
        }
    }

    /// Do work until there is a frame to show, and return it; `None` once the search has ended,
    /// after writing the end state, and on success the first trial's drawing, to the session.
    pub fn next_frame(&mut self) -> Option<Frame> {
        match self.state {
            RunState::Idle => self.state = RunState::Running,
            RunState::Running => {}
            RunState::Done | RunState::Cancelled | RunState::Error => return None,
        }
        let (n, total) = (self.search.trials().len() + 1, self.search.values.len());
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                self.end(RunState::Cancelled, None);
                return None;
            }
            if let Some((run, slot, settings)) = &mut self.trial {
                if let Some(mut frame) = run.next_frame() {
                    frame.label = format!("try {n} of {total}: {}", frame.label);
                    return Some(frame);
                }
                let value = self.search.next().expect("a trial was asked for");
                let trial = match run.state() {
                    RunState::Done => {
                        let drawing = slot.lock().unwrap().drawing.clone();
                        let drawing = drawing.expect("a finished run leaves its drawing");
                        let measure = lint::measure(&drawing);
                        // Only warnings and errors count, and the pens and machine decide only
                        // notes and the off-machine error, which no density setting moves.
                        let found = lint::check(
                            &drawing,
                            &measure,
                            &self.style,
                            settings,
                            &[],
                            self.speeds,
                            None,
                        );
                        let warnings = (found.into_iter())
                            .filter(|f| f.severity != Severity::Info)
                            .map(|f| f.code)
                            .collect();
                        let seconds = stats::compute(&drawing, self.speeds).estimated_seconds;
                        let metric = self.budget.map(|b| b.metric.of(&drawing, self.speeds, run));
                        let over =
                            matches!((self.budget, metric), (Some(b), Some(m)) if m > b.target);
                        if self.first.is_none() {
                            self.first = Some(drawing);
                        }
                        Trial {
                            value,
                            error: measure.tone_match.map(|t| t.error),
                            minutes: Some(seconds / 60.0),
                            metric,
                            over,
                            slow: false,
                            warnings,
                            failed: None,
                        }
                    }
                    RunState::Cancelled => {
                        self.end(RunState::Cancelled, None);
                        return None;
                    }
                    _ if self.first.is_none() => {
                        let error = run.error();
                        self.end(RunState::Error, error);
                        return None;
                    }
                    _ => Trial {
                        value,
                        error: None,
                        minutes: None,
                        metric: None,
                        over: false,
                        slow: false,
                        warnings: vec![],
                        failed: Some(run.error().unwrap_or_else(|| "the run failed".to_string())),
                    },
                };
                self.trial = None;
                self.search.record(trial);
                let t = self
                    .search
                    .trials()
                    .last()
                    .expect("a trial was just recorded");
                let score = match (&t.failed, t.error) {
                    (Some(why), _) => format!("failed, {why}"),
                    (None, Some(e)) => format!("tone error {e:.3}"),
                    (None, None) => "no tone to compare".to_string(),
                };
                let mut notes: Vec<String> = t
                    .minutes
                    .map(|m| Metric::Minutes.show(m))
                    .into_iter()
                    .collect();
                if let (Some(b), Some(m)) = (self.budget, t.metric) {
                    if b.metric != Metric::Minutes {
                        notes.push(b.metric.show(m));
                    }
                }
                if t.over {
                    notes.push("over the target".to_string());
                }
                if t.slow {
                    notes.push("too slow".to_string());
                }
                if !t.warnings.is_empty() {
                    notes.push(format!("lint: {}", t.warnings.join(", ")));
                }
                let key = &self.search.density.key;
                let notes: String = notes.iter().map(|x| format!(", {x}")).collect();
                return Some(Frame {
                    stage: "optimize".to_string(),
                    progress: 1.0,
                    label: format!("try {n} of {total}: {key} {}: {score}{notes}", t.value),
                    points: None,
                    paths: None,
                    pen: 0,
                });
            }
            let Some(value) = self.search.next() else {
                self.end(RunState::Done, None);
                return None;
            };
            let d = &self.search.density;
            let mut settings = self.settings.clone();
            settings[&d.stage][&d.key] = d.to_json(value);
            let slot = Arc::<Mutex<Shared>>::default();
            let run = Run::new(
                self.style.clone(),
                settings.clone(),
                self.roots.clone(),
                self.cache.clone(),
                slot.clone(),
                self.cancel.clone(),
                0,
            );
            self.trial = Some((run, slot, settings));
        }
    }

    /// Ask the search to stop before its next step.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The flag [`OptimizeRun::cancel`] sets.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancel.clone()
    }

    /// Where the search is.
    pub fn state(&self) -> RunState {
        self.state
    }

    /// The failing trial's message, once the search has ended in [`RunState::Error`].
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// What the search found, once it has ended in [`RunState::Done`]; `None` when no trial had a
    /// tone error within its limits.
    pub fn result(&self) -> Option<Optimum> {
        if self.state != RunState::Done {
            return None;
        }
        let best = self.search.best()?;
        let d = &self.search.density;
        let first = &self.search.trials()[0];
        let current_error = (first.score().is_finite()).then_some(first.score());
        let value = d.to_json(best.value);
        let error = best.error.expect("the best trial has a score");
        let minutes = best.minutes.expect("the best trial has a drawing");
        // A search that ends done made the first trial's drawing.
        let current_minutes = first.minutes.expect("the first trial has a drawing");
        let unit = if d.integer { "" } else { " mm" };
        let within = match self.budget {
            Some(b) => format!(" within {}", b.metric.show(b.target)),
            None => String::new(),
        };
        let time = |m: f64| Metric::Minutes.show(m);
        let (mut reason, patch) = if best.value == first.value {
            let reason = format!(
                "{} {}{unit} already follows the image's tones best for its plot time{within}, \
                 with a tone error of {error:.3} and {} to plot.",
                d.key,
                first.value,
                time(current_minutes)
            );
            (reason, None)
        } else {
            let against = match current_error {
                Some(e) => format!(
                    "against {e:.3} and {} at {}{unit}",
                    time(current_minutes),
                    first.value
                ),
                None => format!(
                    "where {}{unit} has none{within} and takes {}",
                    first.value,
                    time(current_minutes)
                ),
            };
            let reason = format!(
                "{} {}{unit} follows the image's tones best for its plot time{within}: a tone \
                 error of {error:.3} and {} to plot, {against}.",
                d.key,
                best.value,
                time(minutes)
            );
            let patch = json!({"settings": {d.stage.clone(): {d.key.clone(): value.clone()}}});
            (reason, Some(patch))
        };
        let limit_minutes = self.search.limit();
        let beyond = self.search.beyond().cloned();
        if let (Some(b), Some(limit)) = (&beyond, limit_minutes) {
            reason += &format!(
                " {} {}{unit} follows them more closely, with a tone error of {:.3}, but takes {} \
                 to plot, over the limit of {}; a budget in minutes replaces the limit.",
                d.key,
                b.value,
                b.error.expect("a trial beyond the limit has a tone error"),
                time(b.minutes.expect("a slow trial has a plot time")),
                time(limit)
            );
        }
        Some(Optimum {
            stage: d.stage.clone(),
            key: d.key.clone(),
            value,
            error,
            minutes,
            current: d.to_json(first.value),
            current_error,
            current_minutes,
            budget: self.budget,
            limit_minutes,
            beyond,
            trials: self.search.trials().to_vec(),
            reason,
            patch,
        })
    }

    /// Set the search's end state and, while the session's slot is still its generation, copy
    /// it there with the first trial's drawing on success.
    fn end(&mut self, state: RunState, error: Option<String>) {
        let mut shared = self.shared.lock().unwrap();
        if shared.generation == self.generation {
            if state == RunState::Done {
                shared.drawing = self.first.clone();
            }
            shared.state = state;
            shared.error = error.clone();
        }
        drop(shared);
        self.state = state;
        self.error = error;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spacing() -> Density {
        Density {
            stage: "s".into(),
            key: "spacing_mm".into(),
            direct: false,
            min: 0.1,
            max: 20.0,
            integer: false,
        }
    }

    fn trial(value: f64, error: Option<f64>) -> Trial {
        Trial {
            value,
            error,
            minutes: Some(10.0),
            metric: None,
            over: false,
            slow: false,
            warnings: vec![],
            failed: None,
        }
    }

    /// Run `s` to the end with `f(value)` as each value's trial.
    fn scan(mut s: Search, f: impl Fn(f64) -> Trial) -> Search {
        while let Some(v) = s.next() {
            s.record(f(v));
        }
        s
    }

    fn values(s: &Search) -> Vec<f64> {
        s.trials().iter().map(|t| t.value).collect()
    }

    #[test]
    fn it_scans_half_octaves_from_a_quarter_to_four_times_the_current_value() {
        let s = scan(Search::new(spacing(), 2.0, true), |v| {
            trial(v, Some((ln(v) - ln(3.0)).abs()))
        });
        let grid = [2.0, 0.5, 0.707, 1.0, 1.414, 2.828, 4.0, 5.657, 8.0];
        assert_eq!(values(&s), grid);
        assert_eq!(s.trials().len(), TRIALS);
        assert_eq!(s.best().unwrap().value, 2.828);
    }

    #[test]
    fn it_finds_the_lower_of_two_dips() {
        // A dip at 0.707 and a lower one at 5.657, with a ridge between.
        let f = |v: f64| (0.1 + (ln(v) - ln(0.707)).abs()).min((ln(v) - ln(5.657)).abs());
        let s = scan(Search::new(spacing(), 2.0, true), |v| trial(v, Some(f(v))));
        assert_eq!(s.best().unwrap().value, 5.657);
    }

    #[test]
    fn it_runs_each_value_once_at_the_ends_of_the_range() {
        let s = scan(Search::new(spacing(), 0.15, true), |v| trial(v, Some(v)));
        assert_eq!(values(&s), [0.15, 0.1, 0.106, 0.212, 0.3, 0.424, 0.6]);
        let count = Density {
            min: 1.0,
            integer: true,
            ..spacing()
        };
        let s = scan(Search::new(count, 4.0, true), |v| trial(v, Some(v)));
        assert_eq!(values(&s), [4.0, 1.0, 2.0, 3.0, 6.0, 8.0, 11.0, 16.0]);
    }

    #[test]
    fn the_budget_leaves_out_values_over_it() {
        // Error falls as the spacing narrows, but spacings under 2.5 break the budget.
        let s = scan(Search::new(spacing(), 2.0, true), |v| Trial {
            over: v < 2.5,
            ..trial(v, Some(v / 10.0))
        });
        assert_eq!(s.best().unwrap().value, 2.828);
        let s = scan(Search::new(spacing(), 2.0, true), |v| Trial {
            over: true,
            ..trial(v, Some(v))
        });
        assert_eq!(s.best(), None);
    }

    #[test]
    fn slow_and_newly_warned_values_are_left_out() {
        // Error falls as the spacing narrows. The current value's drawing takes 40 minutes and
        // already warns of the margin; under 1 mm a drawing takes over 80, and at 1 mm it overdraws.
        let at = |v: f64| Trial {
            minutes: Some(80.0 / v),
            warnings: match v {
                1.0 => vec!["off_area", "overdraw"],
                _ => vec!["off_area"],
            },
            ..trial(v, Some(v))
        };
        let s = scan(Search::new(spacing(), 2.0, true), at);
        let t = s.trials();
        assert!(t.iter().all(|t| t.slow == (t.value < 1.0)), "{t:?}");
        assert!(t.iter().all(|t| t.warnings
            == (if t.value == 1.0 {
                vec!["overdraw"]
            } else {
                vec![]
            })));
        assert_eq!(s.best().unwrap().value, 1.414);
        // A budget in minutes replaces the time limit.
        let s = scan(Search::new(spacing(), 2.0, false), at);
        assert!(s.trials().iter().all(|t| !t.slow));
        assert_eq!(s.best().unwrap().value, 0.5);
    }

    /// A search over the hidden `_slow` style's frame count from `frames`, whose runs fail from 3
    /// frames on.
    fn slow_search(frames: u64) -> OptimizeRun {
        let style = Arc::new(crate::registry::slow_style());
        let mut settings = style.defaults();
        settings["b"] = json!({"frames": frames, "delay_ms": 0, "fail_after": 4});
        let density = Density {
            stage: "b".into(),
            key: "frames".into(),
            direct: true,
            min: 0.0,
            max: 100000.0,
            integer: true,
        };
        OptimizeRun::new(
            style,
            settings,
            crate::test_util::roots(),
            Arc::new(Mutex::new(StageCache::new(1 << 20))),
            Arc::default(),
            Arc::default(),
            0,
            Speeds::default(),
            density,
            None,
        )
    }

    #[test]
    fn a_trial_that_fails_is_left_out_with_its_message() {
        let mut job = slow_search(2);
        let labels: Vec<String> = std::iter::from_fn(|| job.next_frame())
            .filter(|f| f.stage == "optimize")
            .map(|f| f.label)
            .collect();
        assert_eq!(job.state(), RunState::Done);
        let t = job.search.trials();
        let failed: Vec<f64> = (t.iter().filter(|t| t.failed.is_some()))
            .map(|t| t.value)
            .collect();
        assert_eq!(failed, [3.0, 4.0, 6.0, 8.0], "{t:?}");
        assert!(t.iter().all(|t| t.failed.is_some() == t.minutes.is_none()));
        assert!(
            labels[2].starts_with("try 3 of 6: frames 3: failed, "),
            "{labels:?}"
        );
        // The current value failing fails the search, as a run of it would.
        let mut job = slow_search(4);
        while job.next_frame().is_some() {}
        assert_eq!(job.state(), RunState::Error);
        assert!(job.error().is_some());
    }

    #[test]
    fn the_quickest_value_within_a_tenth_of_the_lowest_error_is_best() {
        // Error falls as the spacing narrows, by under a tenth from 1 mm down, and time rises.
        let at = |v: f64| Trial {
            minutes: Some(40.0 / v),
            ..trial(v, Some(if v < 1.5 { 0.08 + v / 100.0 } else { v / 10.0 }))
        };
        let s = scan(Search::new(spacing(), 2.0, false), at);
        assert_eq!(s.best().unwrap().value, 1.0);
        assert_eq!(s.beyond(), None);
        // Equal times keep the earliest within the tenth.
        let s = scan(Search::new(spacing(), 2.0, true), |v| {
            trial(v, Some(if v <= 2.0 { 1.0 } else { 1.05 }))
        });
        assert_eq!(s.best().unwrap().value, 2.0);
    }

    #[test]
    fn a_much_closer_value_left_out_for_time_is_beyond_the_limit() {
        // The current value takes 20 minutes, so the limit is an hour: under 0.7 mm is too slow,
        // and 0.5 mm follows the tones more than a tenth more closely than anything quicker.
        let s = scan(Search::new(spacing(), 2.0, true), |v| Trial {
            minutes: Some(40.0 / v),
            ..trial(v, Some(v))
        });
        assert_eq!(s.limit(), Some(LONGEST));
        assert_eq!(s.best().unwrap().value, 0.707);
        assert_eq!(s.beyond().unwrap().value, 0.5);
        // Within a tenth it is not.
        let s = scan(Search::new(spacing(), 2.0, true), |v| Trial {
            minutes: Some(40.0 / v),
            ..trial(v, Some(1.0 + v / 100.0))
        });
        assert_eq!(s.beyond(), None);
        // A budget in minutes replaces the limit, so nothing is beyond it.
        let s = scan(Search::new(spacing(), 2.0, false), |v| Trial {
            minutes: Some(40.0 / v),
            ..trial(v, Some(v))
        });
        assert_eq!((s.limit(), s.beyond()), (None, None));
    }

    #[test]
    fn the_time_limit_is_never_under_an_hour() {
        // The current value takes 20 minutes, so the limit is an hour, not 40 minutes.
        let s = scan(Search::new(spacing(), 2.0, true), |v| Trial {
            minutes: Some(40.0 / v),
            ..trial(v, Some(v))
        });
        assert!(s.trials().iter().all(|t| t.slow == (t.value < 0.7)));
    }
}
