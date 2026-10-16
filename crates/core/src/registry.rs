//! The built-in styles. Those whose names start with `_` exist for tests: [`get_style`] finds
//! them and [`list_styles`] leaves them out.

use std::sync::Arc;

use crate::stage::Stage;
use crate::stages::contours::ContourStage;
use crate::stages::dashes::DashRenderer;
use crate::stages::dither::DitherPlacer;
use crate::stages::edges::{CellsRenderer, DelaunayRenderer, MstRenderer};
use crate::stages::flow::FlowStage;
use crate::stages::grid::{DotRenderer, GridPlacer};
use crate::stages::hatch::HatchStage;
use crate::stages::hilbert::HilbertStage;
use crate::stages::ink::InkStage;
use crate::stages::order::OrderStage;
use crate::stages::poisson::PoissonPlacer;
use crate::stages::sketch::SketchStage;
use crate::stages::slow::{SlowPaths, SlowPoint};
use crate::stages::spiral::SpiralStage;
use crate::stages::split::SplitStage;
use crate::stages::stipple::StippleRenderer;
use crate::stages::tone::ToneStage;
use crate::stages::truchet::TruchetStage;
use crate::stages::tsp::TspRenderer;
use crate::stages::voronoi::VoronoiPlacer;
use crate::stages::waves::WaveStage;
use crate::style::Style;
use serde_json::json;

/// A style of tone, the split between pens, a stage that places points, a stage that draws them,
/// and path order.
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
        vec![
            Arc::new(ToneStage),
            Arc::new(SplitStage),
            placer,
            renderer,
            Arc::new(OrderStage),
        ],
    )
    .expect("a point style is valid")
}

/// Dots on a regular grid: tone, split, points, render, order.
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

/// A style of tone, the split between pens, one stage that turns the tone map into paths, and
/// path order.
fn tone_style(name: &str, label: &str, description: &str, stage: Arc<dyn Stage>) -> Style {
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
    .expect("a tone style is valid")
}

/// Straight lines over the dark areas: tone, split, hatch, order.
pub fn hatch_style() -> Style {
    tone_style(
        "hatch",
        "Hatching",
        "Straight parallel lines wherever the image is dark enough; more levels cross them over \
         darker areas.",
        Arc::new(HatchStage),
    )
}

/// The tone gamma the rippling styles start from. Images store grey gamma-encoded while ink and
/// paper mix linearly, so at a gamma of 1 their midtones drew too light; see "Rippling gamma" in
/// the brief.
const RIPPLE_GAMMA: f64 = 1.9;

/// Rippling lines: tone, split, waves, order.
pub fn waves_style() -> Style {
    tone_style(
        "waves",
        "Waves",
        "Parallel lines that ripple more where the image is darker, one pen stroke per line.",
        Arc::new(WaveStage),
    )
    .with_defaults("tone", json!({"gamma": RIPPLE_GAMMA}))
}

/// One rippling spiral: tone, split, spiral, order.
pub fn spiral_style() -> Style {
    tone_style(
        "spiral",
        "Spiral",
        "One spiral from the centre out that ripples more where the image is darker.",
        Arc::new(SpiralStage),
    )
    .with_defaults("tone", json!({"gamma": RIPPLE_GAMMA}))
}

/// An image's own marks, outlined and filled: tone, split, ink, order. A finer tone map than the
/// other styles use keeps the hairlines of ink drawings, which averaging into fewer pixels greys
/// out.
pub fn ink_style() -> Style {
    tone_style(
        "ink",
        "Ink",
        "Draws the marks of line art, lettering and inked drawings as they are: every ink shape \
         is outlined and filled with rings, and strokes thinner than the pen get one line down \
         the middle.",
        Arc::new(InkStage),
    )
    .with_defaults("tone", json!({"resolution": 2500}))
}

/// Evenly spread dots, each sized by the ink: tone, split, voronoi, stipple, order.
pub fn stipple_style() -> Style {
    point_style(
        "stipple",
        "Stipple",
        "Evenly spread dots, dense in dark areas, each dot sized by the ink under it.",
        Arc::new(VoronoiPlacer),
        Arc::new(StippleRenderer),
    )
}

/// One unbroken, non-crossing line through evenly spread points: tone, split, voronoi, tsp, order.
pub fn line_style() -> Style {
    point_style(
        "line",
        "Single line",
        "One unbroken, non-crossing line through every point.",
        Arc::new(VoronoiPlacer),
        Arc::new(TspRenderer),
    )
}

/// Short strokes that follow the image's edges: tone, split, voronoi, dashes, order.
pub fn dashes_style() -> Style {
    point_style(
        "dashes",
        "Dashes",
        "Short strokes that follow the image's edges.",
        Arc::new(VoronoiPlacer),
        Arc::new(DashRenderer),
    )
}

/// Straight strokes chained from dark to dark: tone, split, sketch, order.
pub fn sketch_style() -> Style {
    tone_style(
        "sketch",
        "Sketch lines",
        "Straight strokes chained from dark to dark, like a quick pencil sketch.",
        Arc::new(SketchStage),
    )
}

/// The shortest tree joining evenly spread points: tone, split, voronoi, mst, order.
pub fn tree_style() -> Style {
    point_style(
        "tree",
        "Tree",
        "The shortest tree joining every point.",
        Arc::new(VoronoiPlacer),
        Arc::new(MstRenderer),
    )
}

/// A triangle mesh over evenly spread points: tone, split, voronoi, delaunay, order.
pub fn mesh_style() -> Style {
    point_style(
        "mesh",
        "Mesh",
        "A triangle mesh over the points.",
        Arc::new(VoronoiPlacer),
        Arc::new(DelaunayRenderer),
    )
}

/// The outlines of the points' Voronoi cells: tone, split, voronoi, cells, order.
pub fn cells_style() -> Style {
    point_style(
        "cells",
        "Cells",
        "The outlines of the points' Voronoi cells.",
        Arc::new(VoronoiPlacer),
        Arc::new(CellsRenderer),
    )
}

/// Dots on a regular grid, each sized by the ink: tone, split, points, stipple, order.
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

/// Dots where error diffusion puts them: tone, split, dither, render, order.
pub fn dither_style() -> Style {
    point_style(
        "dither",
        "Dither",
        "Dots where error diffusion puts them.",
        Arc::new(DitherPlacer),
        Arc::new(DotRenderer),
    )
}

/// Circles packed without overlapping: tone, split, poisson, stipple, order.
pub fn circles_style() -> Style {
    point_style(
        "circles",
        "Circles",
        "Circles packed without overlapping, larger and closer where it is darker.",
        Arc::new(PoissonPlacer),
        Arc::new(StippleRenderer),
    )
    .with_defaults("poisson", json!({"spacing_mm": 3.0}))
    .with_defaults(
        "stipple",
        json!({"size_mm": 3.0, "vary": 1.0, "shape": "circle"}),
    )
}

/// Level lines of the image, like a topographic map: tone, split, contours, order.
pub fn contours_style() -> Style {
    tone_style(
        "contours",
        "Contours",
        "Level lines of the image, like a topographic map.",
        Arc::new(ContourStage),
    )
}

/// Evenly spaced lines along the image's edges, closer where it is darker: tone, split, flow, order.
pub fn flow_style() -> Style {
    tone_style(
        "flow",
        "Flow lines",
        "Evenly spaced lines that follow the image's edges, closer together where it is darker.",
        Arc::new(FlowStage),
    )
}

/// One space-filling curve over the whole page that folds tighter where it is darker: tone, split,
/// hilbert, order.
pub fn hilbert_style() -> Style {
    tone_style(
        "hilbert",
        "Hilbert curve",
        "A space-filling curve that folds tighter where it is darker, broken where it leaves the \
         image.",
        Arc::new(HilbertStage),
    )
}

/// Quarter-circle arcs on tiles in random turns, nested more deeply where it is darker, joined into
/// curves: tone, split, truchet, order.
pub fn truchet_style() -> Style {
    tone_style(
        "truchet",
        "Truchet tiles",
        "Quarter-circle arcs on square tiles in random turns, joining into winding curves, with \
         more nested arcs where the image is darker.",
        Arc::new(TruchetStage),
    )
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
        Arc::new(tree_style()),
        Arc::new(mesh_style()),
        Arc::new(cells_style()),
        Arc::new(halftone_style()),
        Arc::new(dither_style()),
        Arc::new(circles_style()),
        Arc::new(contours_style()),
        Arc::new(flow_style()),
        Arc::new(hilbert_style()),
        Arc::new(truchet_style()),
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
    use crate::stage::{Artifact, Kind, Lane};
    use crate::synthetic::{flat, gradient, radial, scene};
    use crate::test_util::{assert_inside, black, run_stage_in_lane, run_to_drawing, tone_map};
    use crate::types::{Paths, ToneMap};
    use std::collections::{BTreeMap, HashMap, HashSet};

    /// The paths that the stages of `style` between the split and path order draw over `tone` in
    /// `lane`, at the style's defaults, with a black pen.
    fn lane_paths(style: &Style, tone: &Arc<ToneMap>, lane: Lane) -> Arc<Paths> {
        let defaults = style.defaults();
        let first = style.stage_index("split").unwrap() + 1;
        let last = style.stage_index("order").unwrap();
        let mut made = HashMap::from([
            (Kind::Tone, Artifact::Tone(tone.clone())),
            (Kind::Pens, Artifact::Pens(Arc::new(vec![black()]))),
        ]);
        for stage in &style.stages[first..last] {
            let inputs = stage.requires().iter().map(|k| made[k].clone()).collect();
            let settings = defaults[stage.name()].clone();
            let (_, out) = run_stage_in_lane(stage.as_ref(), inputs, settings, lane);
            made.insert(out.kind(), out);
        }
        match &made[&Kind::Paths] {
            Artifact::Paths(p) => p.clone(),
            a => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    #[test]
    fn lists_builtins_and_hides_test_styles() {
        assert_eq!(
            list_styles()
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            [
                "grid", "hatch", "waves", "spiral", "ink", "stipple", "line", "dashes", "sketch",
                "tree", "mesh", "cells", "halftone", "dither", "circles", "contours", "flow",
                "hilbert", "truchet"
            ]
        );
        assert!(get_style("_slow").is_some());
        assert!(get_style("nope").is_none());
    }
    #[test]
    fn every_style_but_ink_has_one_density_setting() {
        // The budget solver fits the one setting marked x-density; see crate::fit.
        for style in list_styles() {
            let marks: Vec<String> = (style.stages.iter())
                .flat_map(|stage| {
                    let schema = stage.schema();
                    let props = schema["properties"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                    props.into_iter().filter_map(move |(key, p)| {
                        p.get("x-density")
                            .map(|_| format!("{}.{key}", stage.name()))
                    })
                })
                .collect();
            let density = crate::fit::Density::of(&style);
            if style.name == "ink" {
                assert!(marks.is_empty() && density.is_none());
                continue;
            }
            let d = density.unwrap_or_else(|| panic!("{} has no density setting", style.name));
            assert_eq!(marks, [format!("{}.{}", d.stage, d.key)], "{}", style.name);
            assert!(d.min > 0.0 && d.min < d.max, "{}", style.name);
        }
    }
    #[test]
    fn descriptions_have_no_runs_of_spaces() {
        // A string continued onto the next line without a backslash keeps the indent.
        for style in list_styles() {
            assert!(!style.description.contains("  "), "{}", style.name);
        }
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
    fn rippling_styles_darken_their_midtones() {
        let gamma = |name: &str| get_style(name).unwrap().defaults()["tone"]["gamma"].clone();
        assert_eq!(gamma("waves"), json!(1.9));
        assert_eq!(gamma("spiral"), json!(1.9));
        assert_eq!(gamma("hatch"), json!(1.0));
        assert_eq!(gamma("grid"), json!(1.0));
    }
    #[test]
    fn every_style_draws_the_colour_scene() {
        for style in list_styles() {
            let d = run_to_drawing(&style, scene(64, 48));
            assert!(!d.layers[0].paths.is_empty(), "{} drew nothing", style.name);
        }
    }
    #[test]
    fn every_style_but_ink_keeps_its_pens_apart() {
        // The same tone in two lanes, as where two pens both hold ink: a cone of ink over a floor
        // of 0.15, under which a shared lane never lifts the pen, so only the spreading rules tell
        // the lanes apart. The map is 80 by 60 mm at 0.2 mm a pixel, about 24 pixels to each
        // Voronoi point as on real maps; on a map coarser than the points, relaxation puts many of
        // them on the same pixel centres in every lane.
        let cone = tone_map(400, 300, |x, y| {
            let r = crate::math::hypot(f64::from(x) - 200.0, f64::from(y) - 150.0);
            (0.15 + 0.7 * (1.0 - r / 150.0).max(0.0)) as f32
        });
        let tone = Arc::new(ToneMap {
            mm_per_px: 0.2,
            ..cone
        });
        let [x, y, w, h] = tone.rect_mm();
        // Lines clamped onto the map's edge share it whatever the lane, so only vertices off the
        // edge count.
        let off_edge = |p: &&[f64; 2]| x < p[0] && p[0] < x + w && y < p[1] && p[1] < y + h;
        let bits = |p: &[f64; 2]| p.map(f64::to_bits);
        for style in list_styles().into_iter().filter(|s| s.name != "ink") {
            let name = &style.name;
            let single = lane_paths(&style, &tone, Lane::SINGLE);
            let first = lane_paths(&style, &tone, Lane { index: 0, count: 2 });
            let second = lane_paths(&style, &tone, Lane { index: 1, count: 2 });
            // The first pen draws as it would alone, so one-pen drawings do not change.
            assert_eq!(first.coords(), single.coords(), "{name}");
            // At most one in fifty of the second pen's vertices lands on one of the first's. Lines
            // turned against each other cross, and a crossing can be a vertex of both: about one
            // in a hundred of the waves'.
            let ones: HashSet<[u64; 2]> = first.coords().iter().map(bits).collect();
            let inner: Vec<&[f64; 2]> = second.coords().iter().filter(off_edge).collect();
            let shared = inner.iter().filter(|p| ones.contains(&bits(p))).count();
            assert!(inner.len() >= 50, "{name}: {} vertices", inner.len());
            assert!(
                shared * 50 <= inner.len(),
                "{name}: {shared} of {} vertices shared",
                inner.len()
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
    fn the_voronoi_styles_share_tone_and_point_defaults() {
        // Equal settings give equal cache keys, so switching between these styles at their
        // defaults reuses the points.
        let first = get_style("stipple").unwrap().defaults();
        for name in ["line", "dashes", "tree", "mesh", "cells"] {
            let d = get_style(name).unwrap().defaults();
            assert_eq!(d["tone"], first["tone"], "{name}");
            assert_eq!(d["voronoi"], first["voronoi"], "{name}");
        }
    }
    /// Prints the time of `stipple` and `line` at their defaults on a 1000 by 750 radial image.
    /// Run with `cargo test --release -p plotline-core --lib default_timings -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn default_timings() {
        for name in ["stipple", "line"] {
            let style = get_style(name).unwrap();
            let start = std::time::Instant::now();
            run_to_drawing(&style, radial(1000, 750));
            println!("{name}: {:.2} s", start.elapsed().as_secs_f64());
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
