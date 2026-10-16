//! Optimize: the value of a style's density setting whose drawing follows its tone map most
//! closely, within an optional budget, offered as a suggestion and never applied.
//!
//! The density setting is the one the budget solver fits ([`Density`]). Its tone error
//! ([`crate::tone_match`]) has a minimum inside its range: lines too far apart cannot show the
//! tones, and lines too close run together until the midtones turn black. The search is golden-
//! section on `ln(value)` (J. Kiefer, "Sequential minimax search for a maximum", *Proc. AMS* 4(3),
//! 1953, pp. 502-506) over a quarter to four times the current value, after a first trial at the
//! current value itself, for [`TRIALS`] runs in all. A trial whose drawing goes over the budget,
//! or that leaves nothing to compare, scores worse than any other; when both of the bracket's
//! inner trials do, the search moves towards sparser values if either went over the budget and
//! towards denser ones otherwise. The stop rule counts trials, never time, so every target takes
//! the same steps on every target.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{json, Value};

use crate::cache::StageCache;
use crate::fit::{Density, Metric};
use crate::math::{exp, ln};
use crate::runner::{Run, RunState, Shared};
use crate::stage::{Artifact, Frame};
use crate::stats::Speeds;
use crate::style::Style;
use crate::tone_match;
use crate::types::Drawing;

/// The runs one search makes, the first at the current value.
pub const TRIALS: usize = 8;
/// The search looks from the current value divided by this to the current value times it.
const REACH: f64 = 4.0;
/// `1 / phi`, the share of the bracket golden-section keeps each step.
const INV_PHI: f64 = 0.618_033_988_749_894_9;

/// A budget the drawing must stay within: `metric` at most `target`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Budget {
    pub metric: Metric,
    pub target: f64,
}

/// One trial: the value tried, its drawing's tone error if there was anything to compare, and the
/// budget's metric when there is a budget.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Trial {
    pub value: f64,
    pub error: Option<f64>,
    pub metric: Option<f64>,
    /// Whether the drawing went over the budget.
    pub over: bool,
}

impl Trial {
    /// The score golden-section compares: the tone error, or infinity for a trial over the
    /// budget or with nothing to compare.
    fn score(&self) -> f64 {
        match self.error {
            Some(e) if !self.over => e,
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
    /// The best value, as the JSON the setting takes, and its tone error.
    pub value: Value,
    pub error: f64,
    /// The current value and its tone error, if it had one within the budget.
    pub current: Value,
    pub current_error: Option<f64>,
    pub budget: Option<Budget>,
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
    current: f64,
    trials: Vec<Trial>,
    /// The bracket in `ln(value)`, and its two inner points with their trials once run.
    a: f64,
    b: f64,
    c: (f64, Option<Trial>),
    d: (f64, Option<Trial>),
}

impl Search {
    /// A search over `density` around `current`.
    pub fn new(density: Density, current: f64) -> Search {
        let u = ln(current.clamp(density.min.max(f64::MIN_POSITIVE), density.max));
        let (a, b) = (
            (u - ln(REACH)).max(ln(density.min.max(f64::MIN_POSITIVE))),
            (u + ln(REACH)).min(ln(density.max)),
        );
        Search {
            current: density.grain(current),
            density,
            trials: Vec::new(),
            a,
            b,
            c: (b - INV_PHI * (b - a), None),
            d: (a + INV_PHI * (b - a), None),
        }
    }

    /// The value to try next, or `None` once [`TRIALS`] have run.
    pub fn next(&self) -> Option<f64> {
        if self.trials.len() >= TRIALS {
            return None;
        }
        if self.trials.is_empty() {
            return Some(self.current);
        }
        let u = if self.c.1.is_none() {
            self.c.0
        } else {
            self.d.0
        };
        Some(self.density.grain(exp(u)))
    }

    /// Record the trial of the value [`Search::next`] gave, and narrow the bracket once both of
    /// its inner points have run.
    pub fn record(&mut self, trial: Trial) {
        let first = self.trials.is_empty();
        self.trials.push(trial);
        if first {
            return;
        }
        if self.c.1.is_none() {
            self.c.1 = Some(trial);
        } else {
            self.d.1 = Some(trial);
        }
        let (Some(tc), Some(td)) = (self.c.1, self.d.1) else {
            return;
        };
        let (fc, fd) = (tc.score(), td.score());
        // With no score on either side, a budget broken means too dense, and nothing to compare
        // means too sparse; values rise towards sparser for a spacing and denser for a count.
        let lower = if fc.is_infinite() && fd.is_infinite() {
            let sparser = tc.over || td.over;
            sparser == self.density.direct
        } else {
            fc <= fd
        };
        if lower {
            self.b = self.d.0;
            self.d = self.c;
            self.c = (self.b - INV_PHI * (self.b - self.a), None);
        } else {
            self.a = self.c.0;
            self.c = self.d;
            self.d = (self.a + INV_PHI * (self.b - self.a), None);
        }
    }

    /// The trials so far, in order.
    pub fn trials(&self) -> &[Trial] {
        &self.trials
    }

    /// The trial with the lowest score, the earliest of equals; `None` when none has a score.
    pub fn best(&self) -> Option<Trial> {
        self.trials.iter().filter(|t| t.score().is_finite()).fold(
            None,
            |best: Option<Trial>, &t| match best {
                Some(b) if b.score() <= t.score() => Some(b),
                _ => Some(t),
            },
        )
    }
}

/// A search over a snapshot of a session, driven like a [`Run`] one
/// [`OptimizeRun::next_frame`] at a time.
///
/// Each trial is a run of the style with the density setting changed, sharing the session's
/// cache and cancel flag and writing a slot of its own. A trial's frames pass through with their
/// label prefixed by the trial, and after each trial comes a frame of stage `optimize` with its
/// tone error. The first trial is the current value, and at the end its drawing goes to the
/// session's slot, so the drawing still matches the settings, which the search leaves alone.
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
    /// The trial under way and its own slot.
    trial: Option<(Run, Arc<Mutex<Shared>>)>,
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
            search: Search::new(density, current),
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
        let n = self.search.trials().len() + 1;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                self.end(RunState::Cancelled, None);
                return None;
            }
            if let Some((run, slot)) = &mut self.trial {
                if let Some(mut frame) = run.next_frame() {
                    frame.label = format!("try {n} of {TRIALS}: {}", frame.label);
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
                let value = self.search.next().expect("a trial was asked for");
                let error = tone_match::error(&drawing);
                let metric = self.budget.map(|b| b.metric.of(&drawing, self.speeds, run));
                let over = matches!((self.budget, metric), (Some(b), Some(m)) if m > b.target);
                let trial = Trial {
                    value,
                    error,
                    metric,
                    over,
                };
                if self.first.is_none() {
                    self.first = Some(drawing);
                }
                self.trial = None;
                self.search.record(trial);
                let d = &self.search.density;
                let score = match trial.error {
                    Some(e) => format!("tone error {e:.3}"),
                    None => "no tone to compare".to_string(),
                };
                let budget = match (self.budget, metric) {
                    (Some(b), Some(m)) => format!(", {}", b.metric.show(m)),
                    _ => String::new(),
                };
                return Some(Frame {
                    stage: "optimize".to_string(),
                    progress: 1.0,
                    label: format!("try {n} of {TRIALS}: {} {value}: {score}{budget}", d.key),
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
    /// tone error within the budget.
    pub fn result(&self) -> Option<Optimum> {
        if self.state != RunState::Done {
            return None;
        }
        let best = self.search.best()?;
        let d = &self.search.density;
        let first = self.search.trials()[0];
        let current_error = (first.score().is_finite()).then_some(first.score());
        let value = d.to_json(best.value);
        let error = best.error.expect("the best trial has a score");
        let unit = if d.integer { "" } else { " mm" };
        let within = match self.budget {
            Some(b) => format!(" within {}", b.metric.show(b.target)),
            None => String::new(),
        };
        let (reason, patch) = if best.value == first.value {
            let reason = format!(
                "{} {}{unit} already follows the image's tones best{within}, with a tone error of \
                 {error:.3}.",
                d.key, first.value
            );
            (reason, None)
        } else {
            let against = match current_error {
                Some(e) => format!("against {e:.3} at {}{unit}", first.value),
                None => format!("where {}{unit} has none{within}", first.value),
            };
            let reason = format!(
                "{} {}{unit} follows the image's tones best{within}: a tone error of {error:.3}, \
                 {against}.",
                d.key, best.value
            );
            let patch = json!({"settings": {d.stage.clone(): {d.key.clone(): value.clone()}}});
            (reason, Some(patch))
        };
        Some(Optimum {
            stage: d.stage.clone(),
            key: d.key.clone(),
            value,
            error,
            current: d.to_json(first.value),
            current_error,
            budget: self.budget,
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

    /// Run a search with `f(value)` as the tone error and `over(value)` for the budget.
    fn search(f: impl Fn(f64) -> Option<f64>, over: impl Fn(f64) -> bool) -> Search {
        let mut s = Search::new(spacing(), 2.0);
        while let Some(v) = s.next() {
            let error = f(v);
            s.record(Trial {
                value: v,
                error,
                metric: None,
                over: over(v),
            });
        }
        s
    }

    #[test]
    fn it_finds_a_minimum_inside_the_bracket() {
        let s = search(|v| Some((ln(v) - ln(3.0)).abs()), |_| false);
        assert_eq!(s.trials().len(), TRIALS);
        assert_eq!(s.trials()[0].value, 2.0);
        let best = s.best().unwrap().value;
        assert!((best - 3.0).abs() < 0.3, "{best}");
    }

    #[test]
    fn it_stays_inside_a_quarter_to_four_times_the_current_value() {
        let s = search(|v| Some(1.0 / v), |_| false);
        assert!(s.trials().iter().all(|t| (0.5..=8.0).contains(&t.value)));
        assert!(s.best().unwrap().value > 7.0);
    }

    #[test]
    fn the_budget_pushes_it_towards_sparser_values() {
        // Error falls as the spacing narrows, but spacings under 2.5 break the budget.
        let s = search(|v| Some(v / 10.0), |v| v < 2.5);
        let best = s.best().unwrap();
        assert!(!best.over && (2.5..3.2).contains(&best.value), "{best:?}");
    }

    #[test]
    fn no_trial_within_the_budget_finds_nothing() {
        let s = search(Some, |_| true);
        assert_eq!(s.best(), None);
    }
}
