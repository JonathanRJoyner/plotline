//! Measures how the weight of contrast in Optimize's score moves its picks, on the regression
//! gallery's three pictures at A4 with the 0.3 mm pen, for every style with a density setting.
//!
//! Run it with `cargo run --release -p plotline-core --example contrast_weight`. Each style's
//! search runs once; its trials are then ranked again offline for each weight in [`WEIGHTS`], as
//! [`plotline_core::optimize::Search::best`] ranks them: the quickest of the trials within
//! [`CLOSE`] of the lowest `error / contrast^w`, of those not left out. Blank paper's error is the
//! same for every trial of one picture, so it does not change the ranking. Weight 0 is the tone
//! error alone, the score before contrast counted. It prints one line per style and picture and
//! the median contrast and plot time change from the current value for each weight; see
//! `docs/decisions/2026-10-17-contrast-and-colour-match.md`.

use plotline_core::optimize::{Trial, CLOSE};
use plotline_core::registry::list_styles;
use plotline_core::synthetic::{gradient, scene, still_life};
use plotline_core::{RunState, Session};

const WEIGHTS: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// The trial a search ranking by `error / contrast^w` picks, as `Search::best` does.
fn pick(trials: &[Trial], w: f64) -> Option<&Trial> {
    let rank = |t: &Trial| match (t.error, t.contrast) {
        (Some(e), Some(c)) if t.score.is_some() && !t.over && !t.slow && t.warnings.is_empty() => {
            e / c.min(1.0).powf(w)
        }
        _ => f64::INFINITY,
    };
    let lowest = trials.iter().map(rank).fold(f64::INFINITY, f64::min);
    (trials.iter())
        .filter(|t| rank(t).is_finite() && rank(t) <= (1.0 + CLOSE) * lowest)
        .fold(None, |best: Option<&Trial>, t| match best {
            Some(b) if b.minutes <= t.minutes => Some(b),
            _ => Some(t),
        })
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    if v.is_empty() {
        f64::NAN
    } else {
        v[v.len() / 2]
    }
}

fn main() {
    let images = [
        ("still life", still_life(600, 875)),
        ("scene", scene(600, 875)),
        ("gradient", gradient(600, 875)),
    ];
    // For each weight, the contrast and plot time of its pick over the current value's.
    let mut ratios = vec![(Vec::new(), Vec::new()); WEIGHTS.len()];
    for style in list_styles() {
        for (name, image) in &images {
            let mut s = Session::new(1 << 30);
            s.set_image(image.clone());
            s.set_style(&style.name).unwrap();
            let Ok(mut job) = s.optimize(None) else {
                break; // no density setting
            };
            while job.next_frame().is_some() {}
            if job.state() != RunState::Done {
                println!("{} on the {name}: {:?}", style.name, job.error());
                continue;
            }
            let o = job.result().unwrap();
            let first = &o.trials[0];
            let mut line = format!("{:9} {:10} now {:>6}", style.name, name, o.current);
            for (i, &w) in WEIGHTS.iter().enumerate() {
                match pick(&o.trials, w) {
                    Some(t) => {
                        let c = t.contrast.unwrap_or(0.0);
                        line += &format!(
                            " | w{w}: {:>6} c{:.2} e{:.3} {:.0}m",
                            t.value,
                            c,
                            t.error.unwrap_or(f64::NAN),
                            t.minutes.unwrap_or(f64::NAN)
                        );
                        if let (Some(c0), Some(m0), Some(m)) =
                            (first.contrast, first.minutes, t.minutes)
                        {
                            if c0 > 0.0 && m0 > 0.0 {
                                ratios[i].0.push(c / c0);
                                ratios[i].1.push(m / m0);
                            }
                        }
                    }
                    None => line += &format!(" | w{w}: none"),
                }
            }
            println!("{line}");
        }
    }
    for (w, (c, m)) in WEIGHTS.iter().zip(ratios) {
        println!(
            "weight {w}: median contrast x{:.2}, median plot time x{:.2}, over {} searches",
            median(c.clone()),
            median(m),
            c.len()
        );
    }
}
