//! The built-in styles. Those whose names start with `_` exist for tests: [`get_style`] finds
//! them and [`list_styles`] leaves them out.

use std::sync::Arc;

use crate::stage::Stage;
use crate::stages::dashes::DashRenderer;
use crate::stages::grid::{DotRenderer, GridPlacer};
use crate::stages::hatch::HatchStage;
use crate::stages::ink::InkStage;
use crate::stages::order::OrderStage;
use crate::stages::sketch::SketchStage;
use crate::stages::slow::{SlowPaths, SlowPoint};
use crate::stages::spiral::SpiralStage;
use crate::stages::stipple::StippleRenderer;
use crate::stages::tone::ToneStage;
use crate::stages::tsp::TspRenderer;
use crate::stages::voronoi::VoronoiPlacer;
use crate::stages::waves::WaveStage;
use crate::style::Style;
use serde_json::json;

/// A style of tone, a stage that places points, a stage that draws them, and path order.
fn point_style(
    name: &str,
    label: &str,
    description: &str,
    placer: Arc<dyn Stage>,
    renderer: Arc<dyn Stage>,
) -> Style {
    Style::new(
        name,
        label,
        description,
        vec![Arc::new(ToneStage), placer, renderer, Arc::new(OrderStage)],
    )
    .expect("a point style is valid")
}

/// Dots on a regular grid: tone, points, render, order.
pub fn grid_style() -> Style {
    point_style(
        "grid",
        "Grid dots",
        "Dots on a regular grid, one wherever the image is dark enough, drawn in an order that \
         keeps the pen's travel short.",
        Arc::new(GridPlacer),
        Arc::new(DotRenderer),
    )
}

/// A style of tone, one stage that turns the tone map into paths, and path order.
fn tone_style(name: &str, label: &str, description: &str, stage: Arc<dyn Stage>) -> Style {
    Style::new(
        name,
        label,
        description,
        vec![Arc::new(ToneStage), stage, Arc::new(OrderStage)],
    )
    .expect("a tone style is valid")
}

/// Straight lines over the dark areas: tone, hatch, order.
pub fn hatch_style() -> Style {
    tone_style(
        "hatch",
        "Hatching",
        "Straight parallel lines wherever the image is dark enough; more levels cross them over \
         darker areas.",
        Arc::new(HatchStage),
    )
}

/// Rippling lines: tone, waves, order.
pub fn waves_style() -> Style {
    tone_style(
        "waves",
        "Waves",
        "Parallel lines that ripple more where the image is darker, one pen stroke per line.",
        Arc::new(WaveStage),
    )
}

/// One rippling spiral: tone, spiral, order.
pub fn spiral_style() -> Style {
    tone_style(
        "spiral",
        "Spiral",
        "One spiral from the centre out that ripples more where the image is darker.",
        Arc::new(SpiralStage),
    )
}

/// An image's own marks, outlined and filled: tone, ink, order. A finer tone map than the other
/// styles use keeps the hairlines of ink drawings, which averaging into fewer pixels greys out.
pub fn ink_style() -> Style {
    tone_style(
        "ink",
        "Ink",
        "Draws the marks of line art, lettering and inked drawings as they are: every ink shape          is outlined and filled with rings, and strokes thinner than the pen get one line down          the middle.",
        Arc::new(InkStage),
    )
    .with_defaults("tone", json!({"resolution": 2500}))
}

/// Evenly spread dots, each sized by the ink: tone, voronoi, stipple, order.
pub fn stipple_style() -> Style {
    point_style(
        "stipple",
        "Stipple",
        "Evenly spread dots, dense in dark areas, each dot sized by the ink under it.",
        Arc::new(VoronoiPlacer),
        Arc::new(StippleRenderer),
    )
}

/// One unbroken, non-crossing line through evenly spread points: tone, voronoi, tsp, order.
pub fn line_style() -> Style {
    point_style(
        "line",
        "Single line",
        "One unbroken, non-crossing line through every point.",
        Arc::new(VoronoiPlacer),
        Arc::new(TspRenderer),
    )
}

/// Short strokes that follow the image's edges: tone, voronoi, dashes, order.
pub fn dashes_style() -> Style {
    point_style(
        "dashes",
        "Dashes",
        "Short strokes that follow the image's edges.",
        Arc::new(VoronoiPlacer),
        Arc::new(DashRenderer),
    )
}

/// Straight strokes chained from dark to dark: tone, sketch, order.
pub fn sketch_style() -> Style {
    tone_style(
        "sketch",
        "Sketch lines",
        "Straight strokes chained from dark to dark, like a quick pencil sketch.",
        Arc::new(SketchStage),
    )
}

/// Dots on a regular grid, each sized by the ink: tone, points, stipple, order.
pub fn halftone_style() -> Style {
    point_style(
        "halftone",
        "Halftone",
        "Dots on a regular grid, each sized by the ink, like a printed halftone.",
        Arc::new(GridPlacer),
        Arc::new(StippleRenderer),
    )
    .with_defaults("points", json!({"spacing_mm": 2.0, "threshold": 0.02}))
    .with_defaults("stipple", json!({"size_mm": 2.0, "vary": 1.0}))
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
        Arc::new(stipple_style()),
        Arc::new(line_style()),
        Arc::new(dashes_style()),
        Arc::new(sketch_style()),
        Arc::new(halftone_style()),
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
    use crate::synthetic::{flat, gradient};
    use crate::test_util::{assert_inside, run_to_drawing};
    use std::collections::BTreeMap;

    #[test]
    fn lists_builtins_and_hides_test_styles() {
        assert_eq!(
            list_styles()
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            [
                "grid", "hatch", "waves", "spiral", "ink", "stipple", "line", "dashes", "sketch",
                "halftone"
            ]
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
    #[test]
    fn stages_sharing_a_name_share_label_and_schema() {
        // The cache key holds a stage's name, version and settings, so two different stages under
        // one name would hand each other their outputs.
        let mut seen = BTreeMap::new();
        for style in builtins() {
            for stage in &style.stages {
                let (label, schema) = seen
                    .entry(stage.name())
                    .or_insert_with(|| (stage.label(), stage.schema()));
                assert_eq!(*label, stage.label(), "stage {:?}", stage.name());
                assert_eq!(*schema, stage.schema(), "stage {:?}", stage.name());
            }
        }
    }
    #[test]
    fn every_style_survives_tiny_white_and_black_images() {
        for style in list_styles() {
            for image in [flat(1, 1, 0.0), flat(64, 48, 1.0), flat(64, 48, 0.0)] {
                let drawing = run_to_drawing(&style, image);
                let rect = drawing.canvas.drawing_rect();
                for layer in &drawing.layers {
                    assert_inside(&layer.paths, rect);
                }
            }
        }
    }
}
