//! The hatch stage: straight parallel lines drawn only where the image is dark enough.
//!
//! Each line of a family [`scanlines`] lays over the tone map is sampled every half pixel, and each
//! run of samples whose ink reaches the threshold becomes one straight path. A light gap narrower
//! than the line spacing does not end a run: it could not be seen between neighbouring lines, and
//! bridging it spares the pen a lift. Further levels add
//! families at other angles over darker tones, which is cross-hatching. This is the engraver's
//! technique, and there is no published algorithm for the technique itself.
//!
//! When the drawing is split between pens, the levels of all the pens together are spread evenly
//! over the half turn, so that no two pens draw lines at the same angle; lines at close angles
//! make moiré patterns where they overlap (I. Amidror, *The Theory of the Moiré Phenomenon*, 2nd
//! ed., Springer, 2009).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{sample_line, scanlines, ChunkRun};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the hatch stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HatchSettings {
    /// Distance between neighbouring lines.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-density" = "inverse", "x-step" = 0.1, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// Direction of the lines, counter-clockwise from horizontal.
    #[schemars(range(min = 0.0, max = 180.0), extend("x-step" = 1.0, "x-unit" = "deg"))]
    pub angle_deg: f64,
    /// Least ink density a line is drawn over: 0 draws every line in full and 1 only over full ink.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub threshold: f64,
    /// Layers of lines. Each further layer turns by 180 / levels degrees and covers only darker ink, spread evenly between the threshold and full ink.
    #[schemars(range(min = 1, max = 4))]
    pub levels: u32,
}

impl Default for HatchSettings {
    fn default() -> Self {
        HatchSettings {
            spacing_mm: 1.0,
            angle_deg: 45.0,
            threshold: 0.25,
            levels: 3,
        }
    }
}

/// Lines over the dark parts of the tone map, in one or more directions.
pub struct HatchStage;

impl Stage for HatchStage {
    fn name(&self) -> &'static str {
        "hatch"
    }
    fn label(&self) -> &'static str {
        "Hatching"
    }
    fn version(&self) -> u32 {
        2
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<HatchSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<HatchSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<HatchSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let rect = tone.rect_mm();
        let step = tone.mm_per_px / 2.0;
        let n = f64::from(s.levels);
        let lane = ctx.lane();
        let (pen, pens) = (lane.index as f64, lane.count as f64);
        let mut lines = Vec::new();
        for k in 0..s.levels {
            let k = f64::from(k);
            // The levels of all the pens together are spread evenly over the half turn.
            let angle = s.angle_deg + 180.0 * (k * pens + pen) / (n * pens);
            // The tone map holds f32, so the level is compared in f32 like grid's threshold.
            let level = (s.threshold + (1.0 - s.threshold) * k / n) as f32;
            let family = scanlines(rect, angle, s.spacing_mm);
            lines.extend(family.into_iter().map(|(a, b)| (a, b, level)));
        }
        let total = lines.len();
        let bridge = s.spacing_mm;
        Ok(Box::new(ChunkRun::new(
            total,
            "lines",
            Box::new(move |i, paths, _| {
                let (a, b, level) = lines[i];
                // The first and last dark sample of the current run, and how many it holds.
                let mut run: Option<([f64; 2], [f64; 2], usize)> = None;
                let mut end = |run: Option<([f64; 2], [f64; 2], usize)>| {
                    if let Some((first, last, n)) = run {
                        if n >= 2 {
                            paths.push(&[first, last]);
                        }
                    }
                };
                for p in sample_line(a, b, step) {
                    if tone.sample(p) >= level {
                        run = Some(match run {
                            Some((first, last, n))
                                if math::hypot(p[0] - last[0], p[1] - last[1]) <= bridge =>
                            {
                                (first, p, n + 1)
                            }
                            other => {
                                end(other);
                                (p, p, 1)
                            }
                        });
                    }
                }
                end(run);
            }),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::Lane;
    use crate::test_util::{
        assert_inside, bounds, run_tone_stage, run_tone_stage_in_lane, tone_map,
    };
    use serde_json::json;

    const RECT: [f64; 4] = [10.0, 10.0, 20.0, 10.0];

    #[test]
    fn settings_schema() {
        let s = HatchStage.schema();
        assert_eq!(bounds(&s, "spacing_mm"), (0.2, 10.0, 1.0));
        assert_eq!(bounds(&s, "angle_deg"), (0.0, 180.0, 45.0));
        assert_eq!(bounds(&s, "threshold"), (0.0, 1.0, 0.25));
        assert_eq!(bounds(&s, "levels"), (1.0, 4.0, 3.0));
        assert_eq!(
            (HatchStage.name(), HatchStage.label()),
            ("hatch", "Hatching")
        );
    }

    #[test]
    fn white_gives_no_paths() {
        let (frames, paths) = run_tone_stage(&HatchStage, tone_map(10, 5, |_, _| 0.0), json!({}));
        assert!(paths.is_empty());
        assert_eq!(frames.last().unwrap().progress, 1.0);
    }

    #[test]
    fn full_ink_gives_one_line_per_spacing() {
        let (_, paths) = run_tone_stage(
            &HatchStage,
            tone_map(10, 5, |_, _| 1.0),
            json!({"angle_deg": 0.0, "levels": 1}),
        );
        assert_eq!(paths.len(), 10);
        for (i, p) in paths.iter().enumerate() {
            let y = 10.5 + i as f64;
            assert_eq!(p.len(), 2);
            assert!((p[0][1] - y).abs() < 1e-9 && (p[1][1] - y).abs() < 1e-9);
            assert!(
                (p[0][0] - 10.0).abs() < 1e-9 && (p[1][0] - 30.0).abs() < 1e-9,
                "{p:?}"
            );
        }
    }

    #[test]
    fn lines_stop_where_the_ink_does() {
        let half = |x: u32, _| if x < 5 { 1.0 } else { 0.0 };
        let (_, paths) = run_tone_stage(
            &HatchStage,
            tone_map(10, 5, half),
            json!({"angle_deg": 0.0, "levels": 1}),
        );
        assert_eq!(paths.len(), 10);
        for p in paths.iter() {
            assert!(
                (p[0][0] - 10.0).abs() < 1e-9 && (19.0..=21.0).contains(&p[1][0]),
                "{p:?}"
            );
        }
    }

    #[test]
    fn more_levels_cross_the_dark_areas() {
        let slope = |p: &[[f64; 2]]| (p[1][0] - p[0][0]) * (p[1][1] - p[0][1]);
        let (_, paths) = run_tone_stage(
            &HatchStage,
            tone_map(10, 5, |_, _| 1.0),
            json!({"levels": 2, "threshold": 0.5}),
        );
        let rising = paths.iter().filter(|p| slope(p) < 0.0).count();
        let falling = paths.iter().filter(|p| slope(p) > 0.0).count();
        assert!(rising > 0 && falling > 0, "{rising} {falling}");
        // Ink 0.6 reaches the first level (0.5) but not the second (0.75).
        let (_, light) = run_tone_stage(
            &HatchStage,
            tone_map(10, 5, |_, _| 0.6),
            json!({"levels": 2, "threshold": 0.5}),
        );
        assert_eq!(light.len(), rising);
    }

    #[test]
    fn gaps_narrower_than_the_spacing_are_bridged() {
        // Ink everywhere but column 4, a gap 2 mm wide.
        let gap = |x: u32, _| if x == 4 { 0.0 } else { 1.0 };
        let lines = |spacing: f64| {
            let patch = json!({"angle_deg": 0.0, "levels": 1, "spacing_mm": spacing});
            run_tone_stage(&HatchStage, tone_map(10, 5, gap), patch)
                .1
                .len()
        };
        // At 5 mm spacing two lines cross the map, each bridging the gap; at 1 mm, ten lines break.
        assert_eq!(lines(5.0), 2);
        assert_eq!(lines(1.0), 20);
    }

    #[test]
    fn a_second_pen_turns_the_hatch() {
        let patch = || json!({"angle_deg": 0.0, "levels": 1, "spacing_mm": 2.0});
        let run = |index| {
            let tone = tone_map(20, 20, |_, _| 1.0);
            run_tone_stage_in_lane(&HatchStage, tone, patch(), Lane { index, count: 2 }).1
        };
        let (first, second) = (run(0), run(1));
        assert!(!first.is_empty() && !second.is_empty());
        // Pen 0 keeps the angle, so both ends of a path share a y; pen 1 turns it 90 degrees.
        assert!(first.iter().all(|p| (p[0][1] - p[1][1]).abs() < 1e-9));
        assert!(second.iter().all(|p| (p[0][0] - p[1][0]).abs() < 1e-9));
        assert_inside(&second, [10.0, 10.0, 40.0, 40.0]);
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        let noise = |x: u32, y: u32| ((x * 7 + y * 13) % 10) as f32 / 9.0;
        let (_, paths) = run_tone_stage(
            &HatchStage,
            tone_map(10, 5, noise),
            json!({"angle_deg": 30.0, "spacing_mm": 0.4}),
        );
        assert!(!paths.is_empty());
        assert_inside(&paths, RECT);
    }
}
