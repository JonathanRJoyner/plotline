//! The grid placer and the dot renderer: the simplest point stage and the simplest renderer.
//!
//! The placer lays a regular grid over the tone map and keeps the cell centres where the ink
//! density reaches a threshold, so dark areas get points and light areas stay empty. The renderer
//! turns each point into a path of one vertex, which is how a dot is stored. Neither is a published
//! algorithm, so there is nothing to cite.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, Points, ToneMap};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the grid placer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GridSettings {
    /// Distance between neighbouring grid points, along the rows and along the columns.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
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
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<GridSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let [x0, y0, w, h] = tone.rect_mm();
        // The tolerance keeps the last cell when w / spacing is a whole number that rounds just
        // below; a float to usize cast saturates, so a NaN size gives no cells.
        let cols = (w / s.spacing_mm + 1e-9).floor() as usize;
        let rows = (h / s.spacing_mm + 1e-9).floor() as usize;
        Ok(Box::new(GridRun {
            tone,
            origin: [x0, y0],
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
                points: Some(Points(points)),
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
            return Ok(Step::Done(Artifact::Points(Arc::new(Points(points)))));
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

/// Settings for the dot renderer, which takes none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DotSettings {}

/// Turn each point into a path of one vertex, which is a dot.
pub struct DotRenderer;

impl Stage for DotRenderer {
    fn name(&self) -> &'static str {
        "render"
    }
    fn label(&self) -> &'static str {
        "Dots"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points]
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
        validate::<DotSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(DotRun {
            points: Some(inputs.points().clone()),
            paths: None,
        }))
    }
}

/// The work happens in the first step, which reports one frame; the second gives the paths.
struct DotRun {
    /// What the first step reads; `None` once it has run.
    points: Option<Arc<Points>>,
    /// The finished paths, between the first step and the second.
    paths: Option<Paths>,
}

impl StageRun for DotRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some(points) = self.points.take() {
            let mut paths = Paths::new();
            for p in &points.0 {
                paths.push(std::slice::from_ref(p));
            }
            let frame = Frame {
                stage: String::new(),
                progress: 1.0,
                label: format!("{} dots", paths.len()),
                points: None,
                paths: Some(paths.clone()),
                pen: 0,
            };
            self.paths = Some(paths);
            return Ok(Step::Frame(frame));
        }
        let Some(paths) = self.paths.take() else {
            return Err(StageError::Failed(
                "the dot run has already finished".into(),
            ));
        };
        Ok(Step::Done(Artifact::Paths(Arc::new(paths))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::merge;
    use serde_json::json;
    use std::collections::HashMap;

    /// Run `stage` to the end with `patch` over its defaults; returns the frames and the artifact.
    fn run_stage(stage: &dyn Stage, input: Artifact, patch: Value) -> (Vec<Frame>, Artifact) {
        let settings = merge(&stage.defaults(), &patch);
        let inputs = Inputs::new(HashMap::from([(input.kind(), input)]));
        let mut run = stage
            .start(inputs, &settings, RunContext::new(Arc::default()))
            .unwrap();
        let mut frames = Vec::new();
        loop {
            match run.step().unwrap() {
                Step::Frame(f) => frames.push(f),
                Step::Done(a) => return (frames, a),
            }
        }
    }

    fn run_grid(tone: Artifact, patch: Value) -> (Vec<Frame>, Arc<Points>) {
        match run_stage(&GridPlacer, tone, patch) {
            (frames, Artifact::Points(p)) => (frames, p),
            (_, a) => panic!("expected points, got {}", a.kind().name()),
        }
    }

    fn run_dots(points: Artifact) -> (Vec<Frame>, Arc<Paths>) {
        match run_stage(&DotRenderer, points, json!({})) {
            (frames, Artifact::Paths(p)) => (frames, p),
            (_, a) => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    fn tone(ink: Vec<f32>, w: u32, h: u32) -> Artifact {
        Artifact::Tone(Arc::new(ToneMap {
            width: w,
            height: h,
            ink,
            mm_per_px: 2.0,
            origin_mm: [10.0, 10.0],
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
    fn dots_are_one_vertex_paths() {
        let (frames, paths) = run_dots(Artifact::Points(Arc::new(Points(vec![
            [1.0, 2.0],
            [3.0, 4.0],
        ]))));
        assert_eq!(paths.len(), 2);
        assert_eq!(paths.get(1), &[[3.0, 4.0]]);
        assert_eq!(frames[0].label, "2 dots");
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
            (&[Kind::Points][..], Kind::Paths)
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
        assert!(d.schema()["properties"].is_null());
        assert_eq!(d.defaults(), json!({}));
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
    #[test]
    fn a_finished_dot_run_fails_instead_of_panicking() {
        let points = Artifact::Points(Arc::new(Points(vec![[1.0, 2.0]])));
        let inputs = Inputs::new(HashMap::from([(Kind::Points, points)]));
        let ctx = RunContext::new(Arc::default());
        let mut run = DotRenderer.start(inputs, &json!({}), ctx).unwrap();
        assert!(matches!(run.step(), Ok(Step::Frame(_))));
        assert!(matches!(run.step(), Ok(Step::Done(_))));
        assert!(matches!(run.step(), Err(StageError::Failed(_))));
    }
}
