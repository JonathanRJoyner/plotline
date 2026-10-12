//! The hatch stage: straight parallel lines drawn only where the image is dark enough.
//!
//! Each line of a family [`scanlines`] lays over the tone map is sampled every half pixel, and each
//! run of samples whose ink reaches the threshold becomes one straight path. Further levels add
//! families at other angles over darker tones, which is cross-hatching. This is the engraver's
//! technique rather than a published algorithm, so there is nothing to cite.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{sample_line, scanlines, ChunkRun};
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the hatch stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct HatchSettings {
    /// Distance between neighbouring lines.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
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
            threshold: 0.5,
            levels: 1,
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
        1
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
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<HatchSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let rect = tone.rect_mm();
        let step = tone.mm_per_px / 2.0;
        let n = f64::from(s.levels);
        let mut lines = Vec::new();
        for k in 0..s.levels {
            let k = f64::from(k);
            let angle = s.angle_deg + 180.0 * k / n;
            // The tone map holds f32, so the level is compared in f32 like grid's threshold.
            let level = (s.threshold + (1.0 - s.threshold) * k / n) as f32;
            let family = scanlines(rect, angle, s.spacing_mm);
            lines.extend(family.into_iter().map(|(a, b)| (a, b, level)));
        }
        let total = lines.len();
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
                            Some((first, _, n)) => (first, p, n + 1),
                            None => (p, p, 1),
                        });
                    } else {
                        end(run.take());
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
    use crate::test_util::{assert_inside, bounds, run_tone_stage, tone_map};
    use serde_json::json;

    const RECT: [f64; 4] = [10.0, 10.0, 20.0, 10.0];

    #[test]
    fn settings_schema() {
        let s = HatchStage.schema();
        assert_eq!(bounds(&s, "spacing_mm"), (0.2, 10.0, 1.0));
        assert_eq!(bounds(&s, "angle_deg"), (0.0, 180.0, 45.0));
        assert_eq!(bounds(&s, "threshold"), (0.0, 1.0, 0.5));
        assert_eq!(bounds(&s, "levels"), (1.0, 4.0, 1.0));
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
            json!({"angle_deg": 0.0}),
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
            json!({"angle_deg": 0.0}),
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
            json!({"levels": 2}),
        );
        let rising = paths.iter().filter(|p| slope(p) < 0.0).count();
        let falling = paths.iter().filter(|p| slope(p) > 0.0).count();
        assert!(rising > 0 && falling > 0, "{rising} {falling}");
        // Ink 0.6 reaches the first level (0.5) but not the second (0.75).
        let (_, light) = run_tone_stage(
            &HatchStage,
            tone_map(10, 5, |_, _| 0.6),
            json!({"levels": 2}),
        );
        assert_eq!(light.len(), rising);
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
