//! The contours stage: level lines of the smoothed tone map, like a topographic map.
//!
//! The tone map is blurred, padded by one empty pixel on every side so every level closes, and
//! traced at `levels` evenly spaced values between white and full ink, one level at a time, which
//! crowds the lines where the tone changes fast. Each line is simplified to within half a pixel,
//! mapped to page mm and clamped to the map. Equally spaced levels of a smoothed field follow no published plotting
//! algorithm, so there is nothing to cite for them; the iso-line tracer, [`raster::iso_lines`],
//! cites its source in `raster.rs`, and so does the simplification.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, ChunkRun};
use crate::math;
use crate::raster;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the contours stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ContourSettings {
    /// Contour levels between white and full ink.
    #[schemars(range(min = 1, max = 64))]
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
/// Level `j` of `L` lies at ink `j / (L + 1)` for `j` from 1 to `L`. Each level is traced in its
/// own step of the run, lowest first, and its lines come in the order the tracer finds them. Each
/// line is one path, closed: its last vertex repeats the first. A line that simplifies to a single
/// point is dropped.
pub struct ContourStage;

impl Stage for ContourStage {
    fn name(&self) -> &'static str {
        "contours"
    }
    fn label(&self) -> &'static str {
        "Contours"
    }
    fn version(&self) -> u32 {
        1
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
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<ContourSettings>(self.name(), &self.schema(), settings)?;
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
        Ok(Box::new(ChunkRun::new(
            s.levels as usize,
            "levels",
            Box::new(move |i, paths, _| {
                // Level i + 1, summed as the tracer sums `first + j * step`.
                let level = step + i as f64 * step;
                for line in raster::iso_lines(&padded, pw, ph, level, step, level) {
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
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{assert_inside, bounds, run_tone_stage, tone_map};
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn settings_schema() {
        let s = ContourStage.schema();
        assert_eq!(bounds(&s, "levels"), (1.0, 64.0, 12.0));
        assert_eq!(bounds(&s, "smooth_mm"), (0.0, 10.0, 0.5));
        assert_eq!(
            (ContourStage.name(), ContourStage.label()),
            ("contours", "Contours")
        );
        assert_eq!(ContourStage.version(), 1);
        assert_eq!(
            (ContourStage.requires(), ContourStage.provides()),
            (&[Kind::Tone][..], Kind::Paths)
        );
    }

    #[test]
    fn a_radial_gradient_gives_nested_closed_loops() {
        // The centre is the corner shared by pixels 19 and 20, so level l lies at 20 (1 - l)
        // pixels: 16, 12, 8 and 4, or 32, 24, 16 and 8 mm from (50, 50).
        let ink = |x: u32, y: u32| {
            let r = math::hypot(f64::from(x) + 0.5 - 20.0, f64::from(y) + 0.5 - 20.0);
            (1.0 - r / 20.0).max(0.0) as f32
        };
        let (_, paths) = run_tone_stage(
            &ContourStage,
            tone_map(40, 40, ink),
            json!({"levels": 4, "smooth_mm": 0.0}),
        );
        assert_eq!(paths.len(), 4);
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
        for (r, want) in radii.iter().zip([32.0, 24.0, 16.0, 8.0]) {
            assert!((r - want).abs() < 1.0, "{radii:?}");
        }
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
    fn every_vertex_stays_on_the_map() {
        // Flat black rises from the padding at the border, so every line runs along it. Only
        // levels below 1/2 lie outside the map, and are clamped onto its edge; the others lie
        // inside, within half a pixel of it.
        let tone = tone_map(10, 10, |_, _| 1.0);
        let rect = tone.rect_mm();
        let (_, paths) = run_tone_stage(&ContourStage, tone, json!({}));
        assert_eq!(paths.len(), 12);
        assert_inside(&paths, rect);
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
