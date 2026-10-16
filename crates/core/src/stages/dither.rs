//! The error-diffusion placer: a point placer that decides where the dots go by Floyd-Steinberg
//! error diffusion on a grid.
//!
//! The grid is the grid placer's: cells of `spacing_mm` over the tone map's rectangle, sampled at
//! their centres. A cell whose ink plus the error carried into it reaches 0.5 gets a point at its
//! centre, and the difference between that value and the 0 or 1 it was rounded to is shared with
//! the neighbours not scanned yet, so the dots follow the image's tone on average. The weights are
//! 7/16 to the next cell in the row and 3/16, 5/16 and 1/16 to the row below.
//!
//! Sources: R. W. Floyd and L. Steinberg, "An adaptive algorithm for spatial greyscale", *Proc.
//! SID* 17(2), 1976, pp. 75-77. Serpentine scanning follows R. Ulichney, *Digital Halftoning*, MIT
//! Press, 1987.
//!
//! When the drawing is split between pens, each pen's grid moves along the diagonal by its share of
//! a cell, as the grid placer's does, so that the pens' dots do not lie on each other. Two lattices
//! of the same period and angle superpose with no moiré whatever their offset (I. Amidror, *The
//! Theory of the Moiré Phenomenon*, 2nd ed., Springer, 2009).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::grid::lane_grid;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Points, ToneMap};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the error-diffusion placer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DitherSettings {
    /// Grid cell size; one dot at most per cell.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// Scan alternate rows in opposite directions, which breaks up diagonal patterns.
    pub serpentine: bool,
}

impl Default for DitherSettings {
    fn default() -> Self {
        DitherSettings {
            spacing_mm: 1.0,
            serpentine: true,
        }
    }
}

/// Place points on a grid by Floyd-Steinberg error diffusion.
///
/// The grid has the grid placer's cells, centred at `x0 + spacing * (i + 0.5)` across and likewise
/// down, and moves with the lane as the grid placer's does. Points come out row by row, left to right, whichever way a row was scanned. The value
/// tested against 0.5 is the tone sample plus the carried error, in `f64`; error that would land
/// outside the grid is dropped.
pub struct DitherPlacer;

impl Stage for DitherPlacer {
    fn name(&self) -> &'static str {
        "dither"
    }
    fn label(&self) -> &'static str {
        "Error diffusion"
    }
    fn version(&self) -> u32 {
        2
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Points
    }
    fn schema(&self) -> Value {
        schema_for::<DitherSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<DitherSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<DitherSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let (origin, cols, rows) = lane_grid(tone.rect_mm(), s.spacing_mm, ctx.lane());
        Ok(Box::new(DitherRun {
            tone,
            origin,
            spacing: s.spacing_mm,
            serpentine: s.serpentine,
            cols,
            rows,
            rows_per_frame: (rows / 50).max(1),
            next_row: 0,
            no_cells: rows == 0 || cols == 0,
            // One slot per column: error is added to a cell's slot until the row is scanned.
            here: vec![0.0; cols],
            below: vec![0.0; cols],
            points: Vec::new(),
        }))
    }
}

/// Each step scans the next few rows, so a run reports about 50 frames however many rows it has.
struct DitherRun {
    tone: Arc<ToneMap>,
    /// The top-left corner of the grid in mm.
    origin: [f64; 2],
    spacing: f64,
    serpentine: bool,
    cols: usize,
    rows: usize,
    rows_per_frame: usize,
    /// The first row not scanned yet.
    next_row: usize,
    /// Whether the one "no grid cells" frame is still to come.
    no_cells: bool,
    /// The error carried into each cell of the row being scanned.
    here: Vec<f64>,
    /// The error carried into each cell of the row after it.
    below: Vec<f64>,
    /// The points placed so far, row by row.
    points: Vec<[f64; 2]>,
}

impl DitherRun {
    /// Scan row `j`: place its points and share each cell's error with the cells yet to come.
    fn scan(&mut self, j: usize) {
        let y = self.origin[1] + self.spacing * (j as f64 + 0.5);
        let reversed = self.serpentine && j % 2 == 1;
        let first = self.points.len();
        for k in 0..self.cols {
            let i = if reversed { self.cols - 1 - k } else { k };
            let centre = [self.origin[0] + self.spacing * (i as f64 + 0.5), y];
            let value = f64::from(self.tone.sample(centre)) + self.here[i];
            let dot = value >= 0.5;
            if dot {
                self.points.push(centre);
            }
            let err = value - if dot { 1.0 } else { 0.0 };
            // The cell on the scan's far side in this row, then the three below.
            let (ahead, behind) = if reversed {
                (i.checked_sub(1), Some(i + 1))
            } else {
                (Some(i + 1), i.checked_sub(1))
            };
            let give = |cells: &mut Vec<f64>, cell: Option<usize>, weight: f64| {
                if let Some(c) = cell.filter(|&c| c < self.cols) {
                    cells[c] += err * weight / 16.0;
                }
            };
            give(&mut self.here, ahead, 7.0);
            give(&mut self.below, behind, 3.0);
            give(&mut self.below, Some(i), 5.0);
            give(&mut self.below, ahead, 1.0);
        }
        if reversed {
            self.points[first..].reverse();
        }
        std::mem::swap(&mut self.here, &mut self.below);
        self.below.fill(0.0);
    }
}

impl StageRun for DitherRun {
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
            self.scan(j);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{Frame, Lane};
    use crate::test_util::{bounds, run_stage_in_lane, tone_map};
    use serde_json::json;
    use std::collections::HashMap;

    fn run_dither(tone: ToneMap, patch: Value) -> (Vec<Frame>, Arc<Points>) {
        run_dither_in_lane(tone, patch, Lane::SINGLE)
    }

    fn run_dither_in_lane(tone: ToneMap, patch: Value, lane: Lane) -> (Vec<Frame>, Arc<Points>) {
        match run_stage_in_lane(
            &DitherPlacer,
            vec![Artifact::Tone(Arc::new(tone))],
            patch,
            lane,
        ) {
            (frames, Artifact::Points(p)) => (frames, p),
            (_, a) => panic!("expected points, got {}", a.kind().name()),
        }
    }

    /// The points of a run, one text row per grid row: `#` for a point, `.` for none. The map is
    /// `cols` by `rows` cells of 2 mm at spacing 2.
    fn picture(pts: &Points, cols: usize, rows: usize) -> Vec<String> {
        let mut grid = vec![vec!['.'; cols]; rows];
        for p in &pts.0 {
            let (i, j) = (
                ((p[0] - 11.0) / 2.0) as usize,
                ((p[1] - 11.0) / 2.0) as usize,
            );
            assert_eq!([p[0], p[1]], [11.0 + 2.0 * i as f64, 11.0 + 2.0 * j as f64]);
            grid[j][i] = '#';
        }
        grid.into_iter().map(|r| r.into_iter().collect()).collect()
    }

    #[test]
    fn settings_schema() {
        let s = DitherPlacer.schema();
        assert_eq!(bounds(&s, "spacing_mm"), (0.2, 10.0, 1.0));
        assert_eq!(s["properties"]["spacing_mm"]["x-step"], 0.1);
        assert_eq!(s["properties"]["spacing_mm"]["x-unit"], "mm");
        assert_eq!(
            s["properties"]["spacing_mm"]["description"],
            "Grid cell size; one dot at most per cell."
        );
        assert_eq!(s["properties"]["serpentine"]["type"], "boolean");
        assert_eq!(s["properties"]["serpentine"]["default"], true);
        assert_eq!(
            s["properties"]["serpentine"]["description"],
            "Scan alternate rows in opposite directions, which breaks up diagonal patterns."
        );
        assert_eq!(
            (
                DitherPlacer.name(),
                DitherPlacer.label(),
                DitherPlacer.version()
            ),
            ("dither", "Error diffusion", 2)
        );
        assert_eq!(
            (DitherPlacer.requires(), DitherPlacer.provides()),
            (&[Kind::Tone][..], Kind::Points)
        );
    }

    #[test]
    fn half_ink_dots_about_half_the_cells() {
        // 20 x 20 px at 2 mm per px is 40 x 40 mm: 1600 cells at spacing 1.
        let (_, pts) = run_dither(tone_map(20, 20, |_, _| 0.5), json!({}));
        assert!(pts.0.len().abs_diff(800) <= 16, "{} points", pts.0.len());
    }

    #[test]
    fn white_and_black() {
        let (_, none) = run_dither(tone_map(20, 20, |_, _| 0.0), json!({}));
        assert!(none.0.is_empty());
        let (_, all) = run_dither(tone_map(20, 20, |_, _| 1.0), json!({}));
        assert_eq!(all.0.len(), 1600);
        assert_eq!(all.0[0], [10.5, 10.5]);
        assert_eq!(all.0[1599], [49.5, 49.5]);
    }

    #[test]
    fn serpentine_changes_the_pattern() {
        let flat = || tone_map(20, 20, |_, _| 0.3);
        let (_, on) = run_dither(flat(), json!({"serpentine": true}));
        let (_, off) = run_dither(flat(), json!({"serpentine": false}));
        assert_ne!(on.0, off.0);
    }

    #[test]
    fn matches_a_reference_run_by_hand() {
        // Worked out apart from the engine, in f64 with the f32 value of 0.3, on 8 by 4 cells.
        let flat = || tone_map(8, 4, |_, _| 0.3);
        let (_, off) = run_dither(flat(), json!({"spacing_mm": 2.0, "serpentine": false}));
        assert_eq!(
            picture(&off, 8, 4),
            ["...#....", ".#...#.#", "..#.#...", "#....#.#"]
        );
        let (_, on) = run_dither(flat(), json!({"spacing_mm": 2.0, "serpentine": true}));
        assert_eq!(
            picture(&on, 8, 4),
            ["...#....", ".#..#.#.", "..#...#.", ".#...#.."]
        );
    }

    #[test]
    fn a_second_pen_moves_the_grid() {
        let run = |lane| {
            let patch = json!({"spacing_mm": 2.0});
            run_dither_in_lane(tone_map(20, 20, |_, _| 0.5), patch, lane).1
        };
        let (single, first) = (run(Lane::SINGLE), run(Lane { index: 0, count: 2 }));
        assert_eq!(first.0, single.0);
        // Pen 0's centres lie on odd mm, 11 to 49. Pen 1's grid sits half a cell in on both axes,
        // so its centres lie on even mm, 12 to 48, and no dot lands on one of pen 0's.
        let second = run(Lane { index: 1, count: 2 });
        assert!(
            second.0.len().abs_diff(19 * 19 / 2) <= 8,
            "{}",
            second.0.len()
        );
        assert!(single
            .0
            .iter()
            .all(|p| p[0] % 2.0 == 1.0 && p[1] % 2.0 == 1.0));
        assert!(second
            .0
            .iter()
            .all(|p| p[0] % 2.0 == 0.0 && p[1] % 2.0 == 0.0));
        assert!(second
            .0
            .iter()
            .flatten()
            .all(|&v| (12.0..=48.0).contains(&v)));
    }

    #[test]
    fn points_come_row_by_row_left_to_right_even_on_reversed_rows() {
        let (_, pts) = run_dither(tone_map(20, 20, |_, _| 0.5), json!({"spacing_mm": 2.0}));
        let key = |p: &[f64; 2]| (p[1], p[0]);
        assert!(pts.0.windows(2).all(|w| key(&w[0]) < key(&w[1])));
    }

    #[test]
    fn frames_come_by_rows() {
        // 200 rows at one row per cell: 4 rows to a frame, 50 frames.
        let (frames, pts) = run_dither(tone_map(1, 200, |_, _| 1.0), json!({"spacing_mm": 2.0}));
        assert_eq!(frames.len(), 50);
        assert_eq!(frames[49].progress, 1.0);
        assert_eq!(frames[49].label, "row 200/200");
        assert_eq!(frames[49].points.as_ref().unwrap().0, pts.0);
        assert_eq!(frames[0].points.as_ref().unwrap().0.len(), 4);
    }

    #[test]
    fn a_map_smaller_than_a_cell_gives_one_empty_frame() {
        let (frames, pts) = run_dither(tone_map(1, 1, |_, _| 1.0), json!({"spacing_mm": 10.0}));
        assert!(pts.0.is_empty());
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].label, "no grid cells");
        // columns without rows
        let (frames, _) = run_dither(tone_map(100, 1, |_, _| 1.0), json!({"spacing_mm": 4.0}));
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"spacing_mm": 0.1}),
            json!({"serpentine": 1}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&DitherPlacer.defaults(), &patch);
            let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let inputs = Inputs::new(HashMap::from([(Kind::Tone, tone)]));
            let ctx = RunContext::new(Arc::default());
            let err = DitherPlacer.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
}
