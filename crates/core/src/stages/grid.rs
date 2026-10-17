//! The grid placer and the dot renderer: the simplest point stage and the simplest renderer.
//!
//! The placer lays a regular grid over the tone map and keeps the cell centres where the ink
//! density reaches a threshold, so dark areas get points and light areas stay empty. The renderer
//! turns each point into a filled dot whose width is a share of the grid's spacing, drawn as the
//! stipple renderer draws one (a spiral, or a single vertex when the dot is no wider than the pen),
//! so a full grid inks about the same share of the paper at any spacing. Neither is a published
//! algorithm, so there is nothing to cite.
//!
//! When the drawing is split between pens, each pen's grid is shifted along the diagonal by its
//! share of a cell, so that the pens' dots do not lie on each other.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::stipple::{dot_run, DotShape};
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{
    Artifact, Frame, Inputs, Kind, Lane, RunContext, Stage, StageError, StageRun, Step,
};
use crate::types::{Points, ToneMap};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the grid placer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GridSettings {
    /// Distance between neighbouring grid points, along the rows and along the columns.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-density" = "inverse", "x-step" = 0.1, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// Least ink density that gets a point: 0 keeps every grid position and 1 keeps only full ink.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub threshold: f64,
}

impl Default for GridSettings {
    fn default() -> Self {
        GridSettings {
            spacing_mm: 1.0,
            threshold: 0.5,
        }
    }
}

/// Place a point at the centre of every grid cell whose ink density is at least the threshold.
///
/// The grid covers the tone map's rectangle. Cell centres sit at `x0 + spacing * (i + 0.5)` across
/// and likewise down, for as many whole cells as fit. Points come out row by row, left to right.
pub struct GridPlacer;

impl Stage for GridPlacer {
    fn name(&self) -> &'static str {
        "points"
    }
    fn label(&self) -> &'static str {
        "Grid"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Points
    }
    fn schema(&self) -> Value {
        schema_for::<GridSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<GridSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<GridSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let (origin, cols, rows) = lane_grid(tone.rect_mm(), s.spacing_mm, ctx.lane());
        Ok(Box::new(GridRun {
            tone,
            origin,
            spacing: s.spacing_mm,
            // numpy compares the f32 samples with the threshold rounded to f32.
            threshold: s.threshold as f32,
            cols,
            rows,
            rows_per_frame: (rows / 50).max(1),
            next_row: 0,
            no_cells: rows == 0 || cols == 0,
            points: Vec::new(),
        }))
    }
}

/// The grid of `spacing` cells over `rect` that `lane` uses: its top-left corner in mm, its columns
/// and its rows. Each pen's grid is shifted along the diagonal by its share of a cell, and loses
/// the cells that no longer fit; the first lane's grid starts at the rect's corner.
pub(super) fn lane_grid(rect: [f64; 4], spacing: f64, lane: Lane) -> ([f64; 2], usize, usize) {
    let [x0, y0, w, h] = rect;
    let d = spacing * lane.index as f64 / lane.count as f64;
    // The tolerance keeps the last cell when w / spacing is a whole number that rounds just
    // below; a float to usize cast saturates, so a NaN size gives no cells.
    let cols = ((w - d) / spacing + 1e-9).floor() as usize;
    let rows = ((h - d) / spacing + 1e-9).floor() as usize;
    ([x0 + d, y0 + d], cols, rows)
}

/// Each step places the next few rows, so a run reports about 50 frames however many rows it has.
struct GridRun {
    tone: Arc<ToneMap>,
    /// The top-left corner of the grid in mm.
    origin: [f64; 2],
    spacing: f64,
    threshold: f32,
    cols: usize,
    rows: usize,
    rows_per_frame: usize,
    /// The first row not placed yet.
    next_row: usize,
    /// Whether the one "no grid cells" frame is still to come.
    no_cells: bool,
    /// The points placed so far, row by row.
    points: Vec<[f64; 2]>,
}

impl StageRun for GridRun {
    fn step(&mut self) -> Result<Step, StageError> {
        let frame = |progress, label, points| {
            Step::Frame(Frame {
                stage: String::new(),
                progress,
                label,
                points: Some(Points(points, None)),
                paths: None,
                pen: 0,
            })
        };
        if self.no_cells {
            self.no_cells = false;
            // With columns but no rows or the reverse, the rows that remain must not be run.
            self.next_row = self.rows;
            return Ok(frame(1.0, "no grid cells".into(), Vec::new()));
        }
        if self.next_row >= self.rows {
            let points = std::mem::take(&mut self.points);
            return Ok(Step::Done(Artifact::Points(Arc::new(Points(
                points,
                Some(self.spacing),
            )))));
        }
        let end = (self.next_row + self.rows_per_frame).min(self.rows);
        for j in self.next_row..end {
            let y = self.origin[1] + self.spacing * (j as f64 + 0.5);
            for i in 0..self.cols {
                let centre = [self.origin[0] + self.spacing * (i as f64 + 0.5), y];
                if self.tone.sample(centre) >= self.threshold {
                    self.points.push(centre);
                }
            }
        }
        self.next_row = end;
        let label = format!("row {end}/{}", self.rows);
        Ok(frame(
            end as f32 / self.rows as f32,
            label,
            self.points.clone(),
        ))
    }
}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the dot renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DotSettings {
    /// Dot width as a share of the grid spacing: 1 makes neighbouring dots touch.
    #[schemars(range(min = 0.1, max = 1.5), extend("x-step" = 0.05, "x-share-of" = "spacing"))]
    pub size: f64,
}

impl Default for DotSettings {
    fn default() -> Self {
        DotSettings { size: 1.0 }
    }
}

/// Turn each point of a grid into a filled dot `size` times the grid's spacing wide, for the
/// first pen. The points must carry their grid's spacing, as the grid and error-diffusion placers'
/// do. The dots are made as [`dot_run`] makes them.
pub struct DotRenderer;

impl Stage for DotRenderer {
    fn name(&self) -> &'static str {
        "render"
    }
    fn label(&self) -> &'static str {
        "Dots"
    }
    fn version(&self) -> u32 {
        2
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points, Kind::Tone, Kind::Pens]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<DotSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<DotSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<DotSettings>(self.name(), &self.schema(), settings)?;
        let pen_w = inputs
            .pens()
            .first()
            .ok_or_else(|| StageError::Failed("dots need a pen".into()))?
            .width_mm;
        let Some(spacing) = inputs.points().1 else {
            return Err(StageError::Failed(
                "dots are sized by a grid, and these points lie on none".into(),
            ));
        };
        let d = s.size * spacing;
        dot_run(&inputs, pen_w, DotShape::Auto, move |_| d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math;
    use crate::settings::merge;
    use crate::stage::Lane;
    use crate::test_util::{self, tone_map};
    use crate::types::{Paths, Pen};
    use serde_json::json;
    use std::collections::HashMap;

    /// Run `stage` to the end with `patch` over its defaults; returns the frames and the artifact.
    fn run_stage(
        stage: &dyn Stage,
        input: Artifact,
        patch: Value,
        lane: Lane,
    ) -> (Vec<Frame>, Artifact) {
        let settings = merge(&stage.defaults(), &patch);
        let inputs = Inputs::new(HashMap::from([(input.kind(), input)]));
        let ctx = RunContext::new(Arc::default()).in_lane(lane);
        let mut run = stage.start(inputs, &settings, ctx).unwrap();
        let mut frames = Vec::new();
        loop {
            match run.step().unwrap() {
                Step::Frame(f) => frames.push(f),
                Step::Done(a) => return (frames, a),
            }
        }
    }

    fn run_grid(tone: Artifact, patch: Value) -> (Vec<Frame>, Arc<Points>) {
        run_grid_in_lane(tone, patch, Lane::SINGLE)
    }

    fn run_grid_in_lane(tone: Artifact, patch: Value, lane: Lane) -> (Vec<Frame>, Arc<Points>) {
        match run_stage(&GridPlacer, tone, patch, lane) {
            (frames, Artifact::Points(p)) => (frames, p),
            (_, a) => panic!("expected points, got {}", a.kind().name()),
        }
    }

    /// Draw `points` on a grid of `spacing` with a 0.3 mm pen over a flat 10 x 10 px map (20 x 20
    /// mm from (10, 10)), with `patch` over the dot renderer's defaults.
    fn run_dots(points: Vec<[f64; 2]>, spacing: f64, patch: Value) -> (Vec<Frame>, Arc<Paths>) {
        let points = Artifact::Points(Arc::new(Points(points, Some(spacing))));
        let tone = Artifact::Tone(Arc::new(tone_map(10, 10, |_, _| 1.0)));
        let pens = Artifact::Pens(Arc::new(vec![Pen::new("p", 0.3, "#000000").unwrap()]));
        match test_util::run_stage(&DotRenderer, vec![points, tone, pens], patch) {
            (frames, Artifact::Paths(p)) => (frames, p),
            (_, a) => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    /// The farthest a path's vertices lie from `c`.
    fn reach(path: &[[f64; 2]], c: [f64; 2]) -> f64 {
        let far = |m: f64, p: &[f64; 2]| m.max(math::hypot(p[0] - c[0], p[1] - c[1]));
        path.iter().fold(0.0, far)
    }

    fn tone(ink: Vec<f32>, w: u32, h: u32) -> Artifact {
        Artifact::Tone(Arc::new(ToneMap {
            width: w,
            height: h,
            ink,
            mm_per_px: 2.0,
            origin_mm: [10.0, 10.0],
            gamma: 1.0,
        }))
    }
    #[test]
    fn places_points_at_cell_centres_above_threshold() {
        // 10 x 5 px map at 2 mm per px covers 20 x 10 mm; spacing 2: 10 cols, 5 rows, 50 cells
        let (frames, pts) = run_grid(tone(vec![1.0; 50], 10, 5), json!({"spacing_mm": 2.0}));
        assert_eq!(pts.0.len(), 50);
        assert_eq!(pts.0[0], [11.0, 11.0]);
        assert_eq!(pts.0[49], [29.0, 19.0]);
        assert_eq!(frames.len(), 5);
        assert_eq!(frames[4].progress, 1.0);
        assert_eq!(
            frames
                .iter()
                .map(|f| f.points.as_ref().unwrap().0.len())
                .collect::<Vec<_>>(),
            [10, 20, 30, 40, 50]
        );
    }
    #[test]
    fn a_second_pen_staggers_the_grid() {
        let run = |index| {
            let lane = Lane { index, count: 2 };
            run_grid_in_lane(tone(vec![1.0; 50], 10, 5), json!({"spacing_mm": 2.0}), lane).1
        };
        // Pen 0 keeps the 50 cells. Pen 1's grid sits half a spacing in on both axes: 9 columns,
        // 4 rows, from (12, 12) to (28, 18).
        let (first, second) = (run(0), run(1));
        assert_eq!((first.0.len(), first.0[0]), (50, [11.0, 11.0]));
        assert_eq!(second.0.len(), 36);
        assert_eq!((second.0[0], second.0[35]), ([12.0, 12.0], [28.0, 18.0]));
    }
    #[test]
    fn threshold_keeps_only_dark_cells() {
        let mut ink = vec![0.0; 50];
        ink[0] = 1.0; // only pixel (0,0) is dark
        let (_, pts) = run_grid(
            tone(ink, 10, 5),
            json!({"spacing_mm": 2.0, "threshold": 0.6}),
        );
        assert_eq!(pts.0, vec![[11.0, 11.0]]);
    }
    #[test]
    fn frames_are_throttled_for_many_rows() {
        // 1 col, 200 rows
        let (frames, _) = run_grid(tone(vec![1.0; 200], 1, 200), json!({"spacing_mm": 2.0}));
        assert_eq!(frames.len(), 50);
    }
    #[test]
    fn one_pixel_high_map_still_places_points() {
        let (_, pts) = run_grid(tone(vec![1.0; 100], 100, 1), json!({"spacing_mm": 2.0}));
        assert_eq!(pts.0.len(), 100);
    }
    #[test]
    fn too_small_map_gives_no_cells() {
        let (frames, pts) = run_grid(tone(vec![1.0; 1], 1, 1), json!({"spacing_mm": 10.0}));
        assert!(pts.0.is_empty());
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].label, "no grid cells");
    }
    #[test]
    fn dots_are_filled_to_a_share_of_the_spacing() {
        let (frames, paths) = run_dots(vec![[15.0, 15.0], [17.0, 15.0]], 2.0, json!({}));
        assert_eq!(paths.len(), 2);
        // A 2 mm dot fills a 2 mm cell to its sides, so neighbours touch: the 0.3 mm pen's centre
        // spirals out to 0.85 mm, and its ink to 1 mm.
        let second = paths.get(1);
        assert_eq!(second[0], [17.0, 15.0]);
        assert!((reach(second, [17.0, 15.0]) - 0.85).abs() < 1e-9);
        assert_eq!(frames.last().unwrap().label, "2/2 dots");
        // Half the share is a 1 mm dot.
        let (_, half) = run_dots(vec![[15.0, 15.0]], 2.0, json!({"size": 0.5}));
        assert!((reach(half.get(0), [15.0, 15.0]) - 0.35).abs() < 1e-9);
    }
    #[test]
    fn a_dot_no_wider_than_the_pen_is_one_vertex() {
        // 0.9 of 0.3 mm is 0.27 mm, under the 0.3 mm pen.
        let (_, paths) = run_dots(vec![[15.0, 15.0]], 0.3, json!({"size": 0.9}));
        assert_eq!(paths.get(0), &[[15.0, 15.0]]);
    }
    #[test]
    fn points_off_a_grid_are_refused() {
        let points = Artifact::Points(Arc::new(Points(vec![[15.0, 15.0]], None)));
        let tone = Artifact::Tone(Arc::new(tone_map(10, 10, |_, _| 1.0)));
        let pens = Artifact::Pens(Arc::new(vec![Pen::new("p", 0.3, "#000000").unwrap()]));
        let inputs = Inputs::new(
            [points, tone, pens]
                .into_iter()
                .map(|a| (a.kind(), a))
                .collect(),
        );
        let ctx = RunContext::new(Arc::default());
        let err = DotRenderer
            .start(inputs, &DotRenderer.defaults(), ctx)
            .err()
            .unwrap();
        assert!(matches!(err, StageError::Failed(_)), "{err:?}");
    }
    #[test]
    fn the_placer_gives_its_spacing_with_its_points() {
        let (_, pts) = run_grid(tone(vec![1.0; 50], 10, 5), json!({"spacing_mm": 2.0}));
        assert_eq!(pts.1, Some(2.0));
    }
    #[test]
    fn no_cells_is_one_empty_frame_then_done() {
        let (frames, _) = run_grid(tone(vec![1.0; 100], 100, 1), json!({"spacing_mm": 4.0}));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].progress, 1.0);
        assert_eq!(frames[0].points.as_ref().unwrap().0.len(), 0);
    }
    #[test]
    fn the_threshold_is_compared_in_f32_like_numpy() {
        // f32(0.7) is below 0.7, so a flat 0.7 map passes at threshold 0.7 only in f32.
        let (_, pts) = run_grid(
            tone(vec![0.7; 4], 2, 2),
            json!({"spacing_mm": 2.0, "threshold": 0.7}),
        );
        assert_eq!(pts.0.len(), 4);
    }
    #[test]
    fn names_kinds_and_settings_schema() {
        let (g, d) = (GridPlacer, DotRenderer);
        assert_eq!(
            (g.name(), g.label(), d.name(), d.label()),
            ("points", "Grid", "render", "Dots")
        );
        assert_eq!(
            (g.requires(), g.provides()),
            (&[Kind::Tone][..], Kind::Points)
        );
        assert_eq!(
            (d.requires(), d.provides()),
            (&[Kind::Points, Kind::Tone, Kind::Pens][..], Kind::Paths)
        );
        let s = g.schema();
        let spacing = &s["properties"]["spacing_mm"];
        assert_eq!(spacing["default"], 1.0);
        assert_eq!(
            (spacing["minimum"].as_f64(), spacing["maximum"].as_f64()),
            (Some(0.2), Some(10.0))
        );
        assert_eq!(
            (spacing["x-step"].as_f64(), spacing["x-unit"].as_str()),
            (Some(0.1), Some("mm"))
        );
        assert_eq!(
            spacing["description"],
            "Distance between neighbouring grid points, along the rows and along the columns."
        );
        let threshold = &s["properties"]["threshold"];
        assert_eq!(threshold["default"], 0.5);
        assert_eq!(
            threshold["description"],
            "Least ink density that gets a point: 0 keeps every grid position and 1 keeps only full ink."
        );
        assert_eq!(test_util::bounds(&d.schema(), "size"), (0.1, 1.5, 1.0));
        assert_eq!(d.schema()["properties"]["size"]["x-share-of"], "spacing");
        assert_eq!(d.defaults(), json!({"size": 1.0}));
    }
    #[test]
    fn bad_settings_are_rejected_at_start() {
        let inputs = || Inputs::new(HashMap::from([(Kind::Tone, tone(vec![1.0; 1], 1, 1))]));
        for patch in [
            json!({"spacing_mm": 0.1}),
            json!({"threshold": 1.5}),
            json!({"nope": 1}),
        ] {
            let settings = merge(&GridPlacer.defaults(), &patch);
            let ctx = RunContext::new(Arc::default());
            let err = GridPlacer.start(inputs(), &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
    #[test]
    fn columns_without_rows_and_rows_without_columns_give_one_frame() {
        // 1 x 100 px at 2 mm per px is 2 x 200 mm: no columns, 50 rows at spacing 4.
        let (frames, pts) = run_grid(tone(vec![1.0; 100], 1, 100), json!({"spacing_mm": 4.0}));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].label, "no grid cells");
        assert!(pts.0.is_empty());
    }
}
