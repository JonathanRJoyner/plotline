//! The built-in styles. Those whose names start with `_` exist for tests: [`get_style`] finds
//! them and [`list_styles`] leaves them out.

use std::sync::Arc;

use crate::stages::grid::{DotRenderer, GridPlacer};
use crate::stages::order::OrderStage;
use crate::stages::slow::{SlowPaths, SlowPoint};
use crate::stages::tone::ToneStage;
use crate::style::Style;

/// Dots on a regular grid: tone, points, render, order.
pub fn grid_style() -> Style {
    Style::new(
        "grid",
        "Grid dots",
        "Dots on a regular grid, one wherever the image is dark enough, drawn in an order that \
         keeps the pen's travel short.",
        vec![
            Arc::new(ToneStage),
            Arc::new(GridPlacer),
            Arc::new(DotRenderer),
            Arc::new(OrderStage),
        ],
    )
    .expect("the grid style is valid")
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
    vec![Arc::new(grid_style()), Arc::new(slow_style())]
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
    use crate::synthetic::gradient;
    use crate::test_util::run_to_drawing;

    #[test]
    fn lists_builtins_and_hides_test_styles() {
        assert_eq!(
            list_styles()
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["grid"]
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
            let d1 = run_to_drawing(&style, gradient(64, 48));
            let d2 = run_to_drawing(&style, gradient(64, 48));
            assert_eq!(d1.layers[0].paths.coords(), d2.layers[0].paths.coords());
            assert_eq!(
                d1.tone.as_ref().map(|t| t.ink.clone()),
                d2.tone.as_ref().map(|t| t.ink.clone())
            );
        }
    }
}
