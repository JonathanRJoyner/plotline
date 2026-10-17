//! The contours stage: level lines of the smoothed tone map, like a topographic map.
//!
//! The tone map is blurred, padded by one empty pixel on every side so every level closes, and
//! traced at `levels` evenly spaced values between white and full ink, one level at a time, which
//! crowds the lines where the tone changes fast. A line that meets the map's edge is then cut open
//! there: its runs through the padding are left out, since every level inked at the edge would
//! otherwise run along it within half a pixel of the others, one pen width stacked many times
//! over. Each line is simplified to within half a pixel,
//! mapped to page mm and clamped to the map. Equally spaced levels of a smoothed field follow no
//! published plotting algorithm, so there is nothing to cite for them; the iso-line tracer,
//! [`raster::iso_lines`], cites its source in `raster.rs`, and so does the simplification.
//!
//! When the drawing is split between pens, each pen's levels move up by its share of a level step.
//! Over a smooth change of tone the levels draw near-parallel lines, a line grating, and two
//! gratings of one period superpose with no moiré whatever their offset (I. Amidror, *The Theory of
//! the Moiré Phenomenon*, 2nd ed., Springer, 2009), so the shift puts one pen's lines between
//! another's instead of on them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, ChunkRun};
use crate::math;
use crate::raster;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};

/// The most vertices one run may make, as for the stipple dots: about 320 MB of coordinates
/// before the frames copy them.
const VERTEX_BUDGET: usize = 20_000_000;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the contours stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ContourSettings {
    /// Contour levels between white and full ink.
    #[schemars(range(min = 1, max = 64), extend("x-density" = "direct"))]
    pub levels: u32,
    /// Blur before tracing; more gives rounder, calmer lines.
    #[schemars(range(min = 0.0, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub smooth_mm: f64,
}

impl Default for ContourSettings {
    fn default() -> Self {
        ContourSettings {
            levels: 12,
            smooth_mm: 0.5,
        }
    }
}

/// Draw the level lines of the tone map.
///
/// Level `j` of `L` lies at ink `j / (L + 1)` for `j` from 1 to `L`, and in lane `i` of `n` at
/// `(j + i / n) / (L + 1)`. Each level is traced in its
/// own step of the run, lowest first, and its lines come in the order the tracer finds them. A line
/// inside the map is one path, closed: its last vertex repeats the first. A line that meets the
/// map's edge gives one open path per stretch inside the map, ending on the edge. A line that
/// simplifies to a single point is dropped.
pub struct ContourStage;

impl Stage for ContourStage {
    fn name(&self) -> &'static str {
        "contours"
    }
    fn label(&self) -> &'static str {
        "Contours"
    }
    fn version(&self) -> u32 {
        3
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<ContourSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<ContourSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<ContourSettings>(self.name(), &self.schema(), settings)?;
        let lane = ctx.lane();
        let tone = inputs.tone().clone();
        let rect = tone.rect_mm();
        let (w, h) = (tone.width as usize, tone.height as usize);
        let blurred = math::blur(&tone.ink, w, h, s.smooth_mm / tone.mm_per_px);
        let (pw, ph) = (w + 2, h + 2);
        let mut padded = vec![0.0f32; pw * ph];
        for (y, row) in blurred.chunks(w).enumerate() {
            padded[(y + 1) * pw + 1..(y + 1) * pw + 1 + w].copy_from_slice(row);
        }
        let step = 1.0 / (f64::from(s.levels) + 1.0);
        // Each pen's levels move up by its share of a step; the first pen's stay where they were.
        let shift = step * lane.index as f64 / lane.count as f64;
        let run = ChunkRun::new(
            s.levels as usize,
            "levels",
            Box::new(move |i, paths, _| {
                // Level i + 1, summed as the tracer sums `first + j * step`, then the lane's shift.
                let level = step + i as f64 * step + shift;
                let lines = raster::iso_lines(&padded, pw, ph, level, step, level);
                for line in lines.iter().flat_map(|l| open_at_edge(l, w, h)) {
                    // Padded cell x is tone pixel x - 1, whose centre lies at x - 0.5.
                    let mm: Vec<[f64; 2]> = raster::simplify(&line, 0.5)
                        .into_iter()
                        .map(|[x, y]| clamp_to(rect, tone.to_mm([x - 0.5, y - 0.5])))
                        .collect();
                    if mm.iter().any(|v| *v != mm[0]) {
                        paths.push(&mm);
                    }
                }
            }),
        );
        Ok(Box::new(Budgeted::new(run, VERTEX_BUDGET, TOO_BIG)))
    }
}

/// Closed `line`, in the cells of a `w` by `h` map padded by one cell, as the stretches that lie
/// inside the map. A vertex lies in the padding when it is on an edge to a padding cell, that is
/// outside `[1, w]` by `[1, h]`. Each stretch keeps the padding vertex at either end, so it
/// reaches the map's edge, and a line with no vertex in the padding comes back whole.
fn open_at_edge(line: &[[f64; 2]], w: usize, h: usize) -> Vec<Vec<[f64; 2]>> {
    let (w, h) = (w as f64, h as f64);
    let padding = |[x, y]: [f64; 2]| x < 1.0 || y < 1.0 || x > w || y > h;
    // The last vertex repeats the first.
    let n = line.len() - 1;
    let Some(start) = (0..n).find(|&i| padding(line[i])) else {
        return vec![line.to_vec()];
    };
    let (mut pieces, mut piece) = (Vec::new(), Vec::new());
    for k in 1..=n {
        let (prev, v) = (line[(start + k - 1) % n], line[(start + k) % n]);
        if !padding(v) {
            if piece.is_empty() {
                piece.push(prev);
            }
            piece.push(v);
        } else if !piece.is_empty() {
            piece.push(v);
            pieces.push(std::mem::take(&mut piece));
        }
    }
    pieces
}

/// What the run fails with once it passes [`VERTEX_BUDGET`].
const TOO_BIG: &str =
    "these contours need over 20 million vertices; raise smooth mm or use fewer levels";

/// A run that fails with `message` once its paths pass `limit` vertices. Contour lines and
/// scribbles cannot be counted before they are made, so each frame's paths, which are all the
/// paths so far, are counted instead.
pub(crate) struct Budgeted {
    run: ChunkRun,
    limit: usize,
    message: &'static str,
}

impl Budgeted {
    pub(crate) fn new(run: ChunkRun, limit: usize, message: &'static str) -> Budgeted {
        Budgeted {
            run,
            limit,
            message,
        }
    }
}

impl StageRun for Budgeted {
    fn step(&mut self) -> Result<Step, StageError> {
        let step = self.run.step()?;
        let made = match &step {
            Step::Frame(f) => f.paths.as_ref().map_or(0, |p| p.coords().len()),
            Step::Done(Artifact::Paths(p)) => p.coords().len(),
            Step::Done(_) => 0,
        };
        if made > self.limit {
            return Err(StageError::Failed(self.message.to_string()));
        }
        Ok(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Lane;
    use crate::test_util::{
        assert_inside, bounds, run_tone_stage, run_tone_stage_in_lane, tone_map,
    };
    use crate::types::{Paths, ToneMap};
    use serde_json::json;
    use std::sync::Arc;

    /// A 40 by 40 px cone of ink whose tip is the corner shared by pixels 19 and 20, (50, 50) mm,
    /// falling to white 20 pixels out.
    fn cone() -> ToneMap {
        tone_map(40, 40, |x, y| {
            let r = math::hypot(f64::from(x) + 0.5 - 20.0, f64::from(y) + 0.5 - 20.0);
            (1.0 - r / 20.0).max(0.0) as f32
        })
    }

    /// The mean radius about (50, 50) of each of `paths`, which must be closed rings about it.
    fn ring_radii(paths: &Paths) -> Vec<f64> {
        let mut radii = Vec::new();
        for path in paths.iter() {
            assert_eq!(path.first(), path.last(), "a contour closes");
            let n = path.len() - 1;
            let vertices = &path[..n];
            let mean =
                |f: &dyn Fn(&[f64; 2]) -> f64| vertices.iter().map(f).sum::<f64>() / n as f64;
            // The mean radius hardly moves when a ring shifts, so the centre pins the mapping.
            let (cx, cy) = (mean(&|p| p[0]), mean(&|p| p[1]));
            assert!(
                (cx - 50.0).abs() < 0.25 && (cy - 50.0).abs() < 0.25,
                "{cx} {cy}"
            );
            radii.push(mean(&|p| math::hypot(p[0] - 50.0, p[1] - 50.0)));
        }
        radii
    }

    #[test]
    fn settings_schema() {
        let s = ContourStage.schema();
        assert_eq!(bounds(&s, "levels"), (1.0, 64.0, 12.0));
        assert_eq!(bounds(&s, "smooth_mm"), (0.0, 10.0, 0.5));
        assert_eq!(
            (ContourStage.name(), ContourStage.label()),
            ("contours", "Contours")
        );
        assert_eq!(ContourStage.version(), 3);
        assert_eq!(
            (ContourStage.requires(), ContourStage.provides()),
            (&[Kind::Tone][..], Kind::Paths)
        );
    }

    #[test]
    fn a_radial_gradient_gives_nested_closed_loops() {
        // Level l lies at 20 (1 - l) pixels from the tip: 16, 12, 8 and 4, or 32, 24, 16 and 8 mm.
        let (_, paths) = run_tone_stage(
            &ContourStage,
            cone(),
            json!({"levels": 4, "smooth_mm": 0.0}),
        );
        assert_eq!(paths.len(), 4);
        let radii = ring_radii(&paths);
        for (r, want) in radii.iter().zip([32.0, 24.0, 16.0, 8.0]) {
            assert!((r - want).abs() < 1.0, "{radii:?}");
        }
    }

    #[test]
    fn a_second_pen_draws_between_the_first_pens_levels() {
        let run = |lane| {
            let patch = json!({"levels": 4, "smooth_mm": 0.0});
            run_tone_stage_in_lane(&ContourStage, cone(), patch, lane).1
        };
        let single = run(Lane::SINGLE);
        assert_eq!(run(Lane { index: 0, count: 2 }).coords(), single.coords());
        // Pen 1's levels move up half a step, to 0.3, 0.5, 0.7 and 0.9: rings of 28, 20, 12 and
        // 4 mm, between pen 0's.
        let second = run(Lane { index: 1, count: 2 });
        assert_eq!(second.len(), 4);
        let radii = ring_radii(&second);
        for (r, want) in radii.iter().zip([28.0, 20.0, 12.0, 4.0]) {
            assert!((r - want).abs() < 1.0, "{radii:?}");
        }
        assert_inside(&second, cone().rect_mm());
    }

    #[test]
    fn every_level_gives_a_line() {
        // A black block inside white: each level traces its rim, so the count is the level count
        // unless rounding in the level spacing drops the last.
        for levels in 1..=64 {
            let block = |x, y| {
                if (2..8).contains(&x) && (2..8).contains(&y) {
                    1.0
                } else {
                    0.0
                }
            };
            let (_, paths) = run_tone_stage(
                &ContourStage,
                tone_map(10, 10, block),
                json!({"levels": levels, "smooth_mm": 0.0}),
            );
            assert_eq!(paths.len(), levels as usize, "levels {levels}");
        }
    }

    #[test]
    fn a_line_that_simplifies_to_a_point_is_dropped() {
        // A lone black pixel: level l rings it with a diamond 1 - l pixels from its centre. From
        // level 49 of 64 up, the diamond is under half a pixel across and simplifies to its first
        // vertex alone, so it is dropped rather than kept as a speck.
        let (_, paths) = run_tone_stage(
            &ContourStage,
            tone_map(5, 5, |x, y| if (x, y) == (2, 2) { 1.0 } else { 0.0 }),
            json!({"levels": 64, "smooth_mm": 0.0}),
        );
        assert_eq!(paths.len(), 48);
        for path in paths.iter() {
            assert!(path.iter().any(|v| *v != path[0]), "{path:?}");
        }
    }

    #[test]
    fn white_gives_nothing() {
        let (frames, paths) =
            run_tone_stage(&ContourStage, tone_map(20, 20, |_, _| 0.0), json!({}));
        assert!(paths.is_empty());
        assert_eq!(frames.last().unwrap().progress, 1.0);
    }

    #[test]
    fn a_line_ends_at_the_edge_instead_of_running_along_it() {
        // Flat black rises from the padding at the border, so every level would run along it, all
        // twelve within half a pixel of the edge: nothing is left once those runs are cut.
        let (_, paths) = run_tone_stage(&ContourStage, tone_map(10, 10, |_, _| 1.0), json!({}));
        assert!(paths.is_empty(), "{:?}", paths.coords());
        // Ink falling from the left edge: each level is one open line from the top edge to the
        // bottom one, and only its two ends lie within half a pixel (1 mm) of the edge.
        let tone = tone_map(10, 10, |x, _| 1.0 - x as f32 / 10.0);
        let [x, y, w, h] = tone.rect_mm();
        let (_, paths) = run_tone_stage(&ContourStage, tone, json!({"smooth_mm": 0.0}));
        assert!(!paths.is_empty());
        assert_inside(&paths, [x, y, w, h]);
        for path in paths.iter() {
            assert_ne!(path.first(), path.last(), "{path:?}");
            let near = |&[px, py]: &[f64; 2]| {
                (px - x).min(x + w - px).min(py - y).min(y + h - py) < 1.0 - 1e-9
            };
            assert!(path[1..path.len() - 1].iter().all(|v| !near(v)), "{path:?}");
        }
    }

    #[test]
    fn open_at_edge_keeps_a_line_inside_whole_and_cuts_one_along_the_padding() {
        let inside = [[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 1.0]];
        assert_eq!(open_at_edge(&inside, 3, 3), vec![inside.to_vec()]);
        // A loop through the padding on the left: the stretch inside keeps one padding vertex at
        // each end.
        let line = [
            [0.5, 1.0],
            [1.5, 1.0],
            [2.0, 2.0],
            [1.5, 3.0],
            [0.5, 3.0],
            [0.5, 1.0],
        ];
        assert_eq!(
            open_at_edge(&line, 3, 3),
            vec![vec![
                [0.5, 1.0],
                [1.5, 1.0],
                [2.0, 2.0],
                [1.5, 3.0],
                [0.5, 3.0]
            ]]
        );
    }

    #[test]
    fn frames_show_the_lines_appearing() {
        let (frames, paths) = run_tone_stage(
            &ContourStage,
            tone_map(10, 10, |x, y| if x < 8 && y < 8 { 1.0 } else { 0.0 }),
            json!({"levels": 64}),
        );
        assert_eq!(paths.len(), 64);
        assert!(frames.len() > 2);
        let counts: Vec<usize> = frames
            .iter()
            .map(|f| f.paths.as_ref().unwrap().len())
            .collect();
        assert!(
            counts.windows(2).all(|w| w[0] <= w[1]) && counts[0] < 64,
            "{counts:?}"
        );
        // One level is one item of work.
        assert_eq!(frames[0].label, "3/64 levels");
        assert_eq!(frames.last().unwrap().label, "64/64 levels");
    }

    #[test]
    fn a_run_past_its_vertex_budget_fails() {
        // Ten vertices an item against a limit of 25: the third item's step passes the limit.
        let run = ChunkRun::new(
            5,
            "levels",
            Box::new(|_, paths, _| paths.push(&[[0.0, 0.0]; 10])),
        );
        let mut run = Budgeted::new(run, 25, TOO_BIG);
        let results: Vec<_> = std::iter::from_fn(|| match run.step() {
            Ok(Step::Done(_)) => None,
            other => Some(other),
        })
        .take(10)
        .collect();
        assert!(
            matches!(results.last(), Some(Err(StageError::Failed(m))) if m.contains("million")),
            "{results:?}"
        );
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"levels": 0}),
            json!({"smooth_mm": 10.5}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&ContourStage.defaults(), &patch);
            let tone = crate::stage::Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let inputs = Inputs::new([(Kind::Tone, tone)].into_iter().collect());
            let ctx = RunContext::new(Arc::default());
            let err = ContourStage.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
}
