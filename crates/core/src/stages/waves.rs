//! The waves stage: parallel lines that ripple more where the image is darker.
//!
//! Each line of a family [`scanlines`] lays over the tone map is sampled every twelfth of a
//! wavelength, and each sample moves along the line's normal by a sine of the distance along the
//! line's direction, scaled by the ink there. The distance is measured from one origin for every
//! line, so neighbouring lines ripple in phase. Each line is one path, so the pen lifts once per
//! line. Amplitude modulation of a carrier is textbook signal processing, not a published plotting
//! algorithm, so there is nothing to cite.
//!
//! When the drawing is split between pens, each pen's lines turn by its share of the half turn, so
//! that the pens do not draw over each other, and the pen lifts where its lane has almost no ink,
//! breaking a line into runs.

use std::f64::consts::PI;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, direction, sample_line, scanlines, ChunkRun, LANE_MIN_INK};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the waves stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct WaveSettings {
    /// Distance between neighbouring lines.
    #[schemars(range(min = 0.5, max = 10.0), extend("x-density" = "inverse", "x-step" = 0.1, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// Direction of the lines, counter-clockwise from horizontal.
    #[schemars(range(min = 0.0, max = 180.0), extend("x-step" = 1.0, "x-unit" = "deg"))]
    pub angle_deg: f64,
    /// Length of one ripple along the line; shorter ripples look darker but take longer to draw.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub wavelength_mm: f64,
    /// How far a ripple swings over full ink, as a share of half the line spacing.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub amplitude: f64,
}

impl Default for WaveSettings {
    fn default() -> Self {
        WaveSettings {
            spacing_mm: 2.0,
            angle_deg: 0.0,
            wavelength_mm: 1.0,
            amplitude: 0.9,
        }
    }
}

/// Rippling parallel lines, one path per line.
pub struct WaveStage;

impl Stage for WaveStage {
    fn name(&self) -> &'static str {
        "waves"
    }
    fn label(&self) -> &'static str {
        "Waves"
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
        schema_for::<WaveSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<WaveSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<WaveSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let rect = tone.rect_mm();
        let lane = ctx.lane();
        // Each pen's lines turn by its share of the half turn.
        let angle = s.angle_deg + 180.0 * lane.index as f64 / lane.count as f64;
        let lines = scanlines(rect, angle, s.spacing_mm);
        let (dir, normal) = direction(angle);
        let swing = s.amplitude * s.spacing_mm / 2.0;
        let step = s.wavelength_mm / 12.0;
        // Split between pens, a lane lifts the pen over no ink; alone, each line is drawn whole.
        let lift = lane.count > 1;
        Ok(Box::new(ChunkRun::new(
            lines.len(),
            "lines",
            Box::new(move |i, paths, _| {
                let (a, b) = lines[i];
                let mut wave: Vec<[f64; 2]> = Vec::new();
                for p in sample_line(a, b, step) {
                    let ink = tone.sample(p);
                    if lift && ink < LANE_MIN_INK {
                        if wave.len() >= 2 {
                            paths.push(&wave);
                        }
                        wave.clear();
                        continue;
                    }
                    let t = p[0] * dir[0] + p[1] * dir[1];
                    let off = swing * f64::from(ink) * math::sin(2.0 * PI * t / s.wavelength_mm);
                    // A ripple near the map's edge would leave it, and the page with it.
                    wave.push(clamp_to(
                        rect,
                        [p[0] + off * normal[0], p[1] + off * normal[1]],
                    ));
                }
                if wave.len() >= 2 {
                    paths.push(&wave);
                }
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
        let s = WaveStage.schema();
        assert_eq!(bounds(&s, "spacing_mm"), (0.5, 10.0, 2.0));
        assert_eq!(bounds(&s, "angle_deg"), (0.0, 180.0, 0.0));
        assert_eq!(bounds(&s, "wavelength_mm"), (0.2, 10.0, 1.0));
        assert_eq!(bounds(&s, "amplitude"), (0.0, 1.0, 0.9));
        assert_eq!((WaveStage.name(), WaveStage.label()), ("waves", "Waves"));
    }

    #[test]
    fn white_gives_straight_lines_across() {
        let (frames, paths) = run_tone_stage(&WaveStage, tone_map(10, 5, |_, _| 0.0), json!({}));
        assert_eq!(paths.len(), 5);
        for p in paths.iter() {
            assert!(p.len() > 2 && p.iter().all(|v| (v[1] - p[0][1]).abs() < 1e-9));
            let last = p[p.len() - 1];
            assert!((p[0][0] - 10.0).abs() < 1e-9 && (last[0] - 30.0).abs() < 1e-9);
        }
        assert!(frames.last().unwrap().label.ends_with("lines"));
    }

    #[test]
    fn full_ink_swings_by_the_amplitude() {
        let (_, paths) = run_tone_stage(&WaveStage, tone_map(10, 5, |_, _| 1.0), json!({}));
        let swing = paths
            .get(1)
            .iter()
            .map(|v| (v[1] - 13.0).abs())
            .fold(0.0, f64::max);
        assert!((swing - 0.9).abs() < 0.02, "{swing}");
    }

    #[test]
    fn a_second_pen_turns_the_waves() {
        // With no swing the lines are straight. Pen 0 would draw them level; pen 1 turns them to
        // 90 degrees, so each keeps one x.
        let tone = tone_map(10, 5, |_, _| 1.0);
        let lane = Lane { index: 1, count: 2 };
        let (_, paths) = run_tone_stage_in_lane(&WaveStage, tone, json!({"amplitude": 0.0}), lane);
        assert!(!paths.is_empty());
        for p in paths.iter() {
            assert!(p.len() > 2 && p.iter().all(|v| (v[0] - p[0][0]).abs() < 1e-9));
        }
        assert_inside(&paths, RECT);
    }

    #[test]
    fn a_shared_lane_lifts_the_pen_over_no_ink() {
        // Ink only in the left half: alone, each line crosses the map; in a lane it stops there.
        let half = |x: u32, _| if x < 5 { 1.0 } else { 0.0 };
        let (_, alone) = run_tone_stage(&WaveStage, tone_map(10, 5, half), json!({}));
        assert!(alone.iter().all(|p| p[p.len() - 1][0] > 29.0));
        let lane = Lane { index: 0, count: 2 };
        let (_, shared) =
            run_tone_stage_in_lane(&WaveStage, tone_map(10, 5, half), json!({}), lane);
        assert_eq!(shared.len(), alone.len());
        assert!(
            shared.iter().all(|p| p[p.len() - 1][0] < 21.0),
            "{shared:?}"
        );
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        let (_, paths) = run_tone_stage(
            &WaveStage,
            tone_map(10, 5, |_, _| 1.0),
            json!({"angle_deg": 30.0, "amplitude": 1.0, "spacing_mm": 3.0}),
        );
        assert_inside(&paths, RECT);
    }
}
