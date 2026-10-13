//! The spiral stage: one Archimedean spiral from the centre out, rippling where the image is dark.
//!
//! The spiral `r = spacing * theta / (2 pi)` (Archimedes, *On Spirals*, definition 1) is centred
//! on the tone map and sampled every twelfth of a wavelength of arc. Each sample moves along the
//! radius by a sine of the arc length, scaled by the ink there, as the waves stage does across its
//! lines. The spiral runs until it has passed the map's corners; where it is off the map the pen
//! is up, so it breaks into as few paths as the map's shape allows.
//!
//! When the drawing is split between pens, each pen's spiral starts at its share of a full turn, so
//! that the pens' turns do not lie on each other, and the pen is up where its lane has almost no
//! ink, as it is off the map.

use std::f64::consts::PI;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, inside, ChunkRun, LANE_MIN_INK};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the spiral stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SpiralSettings {
    /// Distance between neighbouring turns.
    #[schemars(range(min = 0.5, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// Length of one ripple along the spiral; shorter ripples look darker but take longer to draw.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub wavelength_mm: f64,
    /// How far a ripple swings over full ink, as a share of half the turn spacing.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub amplitude: f64,
}

impl Default for SpiralSettings {
    fn default() -> Self {
        SpiralSettings {
            spacing_mm: 2.0,
            wavelength_mm: 1.0,
            amplitude: 0.9,
        }
    }
}

/// One rippling spiral over the tone map.
pub struct SpiralStage;

impl Stage for SpiralStage {
    fn name(&self) -> &'static str {
        "spiral"
    }
    fn label(&self) -> &'static str {
        "Spiral"
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
        schema_for::<SpiralSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<SpiralSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<SpiralSettings>(self.name(), &self.schema(), settings)?;
        let tone = inputs.tone().clone();
        let rect = tone.rect_mm();
        let lane = ctx.lane();
        // Each pen's spiral starts at its share of a full turn.
        let phi = 2.0 * PI * lane.index as f64 / lane.count as f64;
        let [x, y, w, h] = rect;
        let centre = [x + w / 2.0, y + h / 2.0];
        let turns = ((math::hypot(w, h) / 2.0 / s.spacing_mm).ceil() as usize).max(1);
        let swing = s.amplitude * s.spacing_mm / 2.0;
        let step = s.wavelength_mm / 12.0;
        // Split between pens, a lane lifts the pen over no ink as it does off the map.
        let lift = lane.count > 1;
        // Where the spiral has got to, carried from one turn to the next: angle and arc length.
        let (mut theta, mut arc) = (0.0_f64, 0.0_f64);
        Ok(Box::new(ChunkRun::new(
            turns,
            "turns",
            Box::new(move |k, paths, open| {
                let end = 2.0 * PI * (k + 1) as f64;
                while theta < end {
                    let r = s.spacing_mm * theta / (2.0 * PI);
                    let (sin, cos) = (math::sin(theta + phi), math::cos(theta + phi));
                    let base = [centre[0] + r * cos, centre[1] + r * sin];
                    let ink = inside(rect, base).then(|| tone.sample(base));
                    if let Some(ink) = ink.filter(|&ink| !lift || ink >= LANE_MIN_INK) {
                        let off =
                            swing * f64::from(ink) * math::sin(2.0 * PI * arc / s.wavelength_mm);
                        let rr = (r + off).max(0.0);
                        open.push(clamp_to(rect, [centre[0] + rr * cos, centre[1] + rr * sin]));
                    } else if !open.is_empty() {
                        if open.len() >= 2 {
                            paths.push(open);
                        }
                        open.clear();
                    }
                    arc += step;
                    theta += step / r.max(step);
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
        let s = SpiralStage.schema();
        assert_eq!(bounds(&s, "spacing_mm"), (0.5, 10.0, 2.0));
        assert_eq!(bounds(&s, "wavelength_mm"), (0.2, 10.0, 1.0));
        assert_eq!(bounds(&s, "amplitude"), (0.0, 1.0, 0.9));
        assert_eq!(
            (SpiralStage.name(), SpiralStage.label()),
            ("spiral", "Spiral")
        );
    }

    #[test]
    fn starts_at_the_centre_and_winds_outwards() {
        let (frames, paths) = run_tone_stage(&SpiralStage, tone_map(10, 5, |_, _| 0.0), json!({}));
        let first = paths.get(0);
        assert_eq!(first[0], [20.0, 15.0]);
        let r = |v: &[f64; 2]| math::hypot(v[0] - 20.0, v[1] - 15.0);
        assert!(first.windows(2).all(|w| r(&w[1]) >= r(&w[0]) - 1e-9));
        // The 10 mm tall rect holds the spiral out to a radius of 5 mm, two and a half turns.
        assert!(r(&first[first.len() - 1]) > 4.5);
        assert!(frames.last().unwrap().label.ends_with("turns"));
    }

    #[test]
    fn a_second_pen_turns_the_spiral_half_round() {
        let run = |index| {
            let tone = tone_map(10, 5, |_, _| 1.0);
            let lane = Lane { index, count: 2 };
            run_tone_stage_in_lane(&SpiralStage, tone, json!({}), lane).1
        };
        let (first, second) = (run(0), run(1));
        assert!(!first.is_empty());
        assert_eq!(first.offsets(), second.offsets());
        assert_inside(&second, RECT);
        // Half a turn about the rect's centre (20, 15) sends each vertex v to 2 c - v.
        for (v0, v1) in first.coords().iter().zip(second.coords()) {
            assert!((v1[0] - (40.0 - v0[0])).abs() < 1e-9, "{v0:?} {v1:?}");
            assert!((v1[1] - (30.0 - v0[1])).abs() < 1e-9, "{v0:?} {v1:?}");
        }
    }

    #[test]
    fn a_shared_lane_lifts_the_pen_over_no_ink() {
        // Ink only in the left half: in a lane the spiral draws there and nowhere else.
        let half = |x: u32, _| if x < 5 { 1.0 } else { 0.0 };
        let lane = Lane { index: 0, count: 2 };
        let (_, paths) =
            run_tone_stage_in_lane(&SpiralStage, tone_map(10, 5, half), json!({}), lane);
        assert!(paths.len() > 1);
        assert!(paths.coords().iter().all(|v| v[0] < 21.0));
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        let (_, paths) = run_tone_stage(
            &SpiralStage,
            tone_map(10, 5, |_, _| 1.0),
            json!({"amplitude": 1.0, "spacing_mm": 3.0}),
        );
        assert!(paths.len() > 1);
        assert!(paths.iter().all(|p| p.len() >= 2));
        assert_inside(&paths, RECT);
    }
}
