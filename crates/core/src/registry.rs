//! The built-in styles. Those whose names start with `_` exist for tests: [`get_style`] finds
//! them and [`list_styles`] leaves them out.

use std::sync::Arc;

use crate::stage::Stage;
use crate::stages::grid::{DotRenderer, GridPlacer};
use crate::stages::hatch::HatchStage;
use crate::stages::ink::InkStage;
use crate::stages::order::OrderStage;
use crate::stages::slow::{SlowPaths, SlowPoint};
use crate::stages::spiral::SpiralStage;
use crate::stages::split::SplitStage;
use crate::stages::tone::ToneStage;
use crate::stages::waves::WaveStage;
use crate::style::Style;
use serde_json::json;

/// Dots on a regular grid: tone, split, points, render, order.
pub fn grid_style() -> Style {
    Style::new(
        "grid",
        "Grid dots",
        "Dots on a regular grid, one wherever the image is dark enough, drawn in an order that \
         keeps the pen's travel short.",
        vec![
            Arc::new(ToneStage),
            Arc::new(SplitStage),
            Arc::new(GridPlacer),
            Arc::new(DotRenderer),
            Arc::new(OrderStage),
        ],
    )
    .expect("the grid style is valid")
}

/// A style of tone, the split between pens, one stage that turns the tone map into paths, and
/// path order.
fn line_style(name: &str, label: &str, description: &str, stage: Arc<dyn Stage>) -> Style {
    Style::new(
        name,
        label,
        description,
        vec![
            Arc::new(ToneStage),
            Arc::new(SplitStage),
            stage,
            Arc::new(OrderStage),
        ],
    )
    .expect("a line style is valid")
}

/// Straight lines over the dark areas: tone, split, hatch, order.
pub fn hatch_style() -> Style {
    line_style(
        "hatch",
        "Hatching",
        "Straight parallel lines wherever the image is dark enough; more levels cross them over \
         darker areas.",
        Arc::new(HatchStage),
    )
}

/// Rippling lines: tone, split, waves, order.
pub fn waves_style() -> Style {
    line_style(
        "waves",
        "Waves",
        "Parallel lines that ripple more where the image is darker, one pen stroke per line.",
        Arc::new(WaveStage),
    )
}

/// One rippling spiral: tone, split, spiral, order.
pub fn spiral_style() -> Style {
    line_style(
        "spiral",
        "Spiral",
        "One spiral from the centre out that ripples more where the image is darker.",
        Arc::new(SpiralStage),
    )
}

/// An image's own marks, outlined and filled: tone, split, ink, order. A finer tone map than the
/// other styles use keeps the hairlines of ink drawings, which averaging into fewer pixels greys
/// out.
pub fn ink_style() -> Style {
    line_style(
        "ink",
        "Ink",
        "Draws the marks of line art, lettering and inked drawings as they are: every ink shape          is outlined and filled with rings, and strokes thinner than the pen get one line down          the middle.",
        Arc::new(InkStage),
    )
    .with_defaults("tone", json!({"resolution": 2500}))
}

/// The hidden `_slow` style for host tests: stage `a` gives one point, and stage `b` takes a set
/// time over a set number of frames and can fail at a chosen step.
pub fn slow_style() -> Style {
    Style::new(
        "_slow",
        "Slow",
        "A run of known length for testing hosts.",
        vec![Arc::new(SlowPoint), Arc::new(SlowPaths)],
    )
    .expect("the slow style is valid")
}

/// Every built-in style, hidden ones included.
fn builtins() -> Vec<Arc<Style>> {
    vec![
        Arc::new(grid_style()),
        Arc::new(hatch_style()),
        Arc::new(waves_style()),
        Arc::new(spiral_style()),
        Arc::new(ink_style()),
        Arc::new(slow_style()),
    ]
}

/// The built-in styles whose names do not start with `_`, in a new list on every call.
pub fn list_styles() -> Vec<Arc<Style>> {
    builtins()
        .into_iter()
        .filter(|s| !s.name.starts_with('_'))
        .collect()
}

/// The built-in style called `name`, hidden ones included.
pub fn get_style(name: &str) -> Option<Arc<Style>> {
    builtins().into_iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::{gradient, scene};
    use crate::test_util::run_to_drawing;

    #[test]
    fn lists_builtins_and_hides_test_styles() {
        assert_eq!(
            list_styles()
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["grid", "hatch", "waves", "spiral", "ink"]
        );
        assert!(get_style("_slow").is_some());
        assert!(get_style("nope").is_none());
    }
    #[test]
    fn every_style_is_deterministic() {
        for style in list_styles()
            .into_iter()
            .chain([get_style("_slow").unwrap()])
        {
            for image in [gradient(64, 48), scene(64, 48)] {
                let d1 = run_to_drawing(&style, image.clone());
                let d2 = run_to_drawing(&style, image);
                assert_eq!(d1.layers[0].paths.coords(), d2.layers[0].paths.coords());
                assert_eq!(
                    d1.tone.as_ref().map(|t| t.ink.clone()),
                    d2.tone.as_ref().map(|t| t.ink.clone())
                );
            }
        }
    }
    #[test]
    fn every_listed_style_splits_right_after_tone() {
        for style in list_styles() {
            let tone = style.stage_index("tone");
            assert!(tone.is_some(), "{} has no tone stage", style.name);
            assert_eq!(
                style.stage_index("split"),
                tone.map(|i| i + 1),
                "{}",
                style.name
            );
        }
    }
    #[test]
    fn every_style_draws_the_colour_scene() {
        for style in list_styles() {
            let d = run_to_drawing(&style, scene(64, 48));
            assert!(!d.layers[0].paths.is_empty(), "{} drew nothing", style.name);
        }
    }
}
