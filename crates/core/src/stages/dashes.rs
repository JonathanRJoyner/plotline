//! The dash renderer: turns each point into one short straight stroke along the image's edges.
//!
//! A stroke laid perpendicular to the tone gradient runs along the edge under it. Orienting
//! strokes by the image gradient follows A. Hertzmann, "Painterly rendering with curved brush
//! strokes of multiple sizes", *Proc. SIGGRAPH*, 1998, pp. 453-460, which draws long curved
//! strokes where this draws one short straight dash per point.

use std::f64::consts::PI;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{self, clamp_to};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::{Paths, Points, ToneMap};

/// Below this gradient, in ink per mm, a point counts as flat and its dash lies at the set angle.
const FLAT: f64 = 1e-3;

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the dash renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DashSettings {
    /// Dash length.
    #[schemars(range(min = 0.2, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub length_mm: f64,
    /// Turn from the edge direction, or the fixed direction when edges are not followed.
    #[schemars(range(min = 0.0, max = 180.0), extend("x-step" = 1.0, "x-unit" = "deg"))]
    pub angle_deg: f64,
    /// Lay dashes along the image's edges; off lays every dash at the fixed angle.
    pub follow_edges: bool,
}

impl Default for DashSettings {
    fn default() -> Self {
        DashSettings {
            length_mm: 1.5,
            angle_deg: 0.0,
            follow_edges: true,
        }
    }
}

/// The unit direction of the dash at `p`. Angles turn counter-clockwise as seen on the page, like
/// every other angle in the project (see [`lines::direction`]). The dash lies along the edge, which
/// is perpendicular to the gradient found by central differences half a dash length either side,
/// turned by `angle_deg`; or at `angle_deg` from horizontal where the gradient is flat or edges are
/// ignored.
fn direction(tone: &ToneMap, s: &DashSettings, p: [f64; 2]) -> [f64; 2] {
    let mut deg = s.angle_deg;
    if s.follow_edges {
        let h = s.length_mm / 2.0;
        let ink = |dx: f64, dy: f64| f64::from(tone.sample([p[0] + dx, p[1] + dy]));
        let gx = (ink(h, 0.0) - ink(-h, 0.0)) / s.length_mm;
        // The page's y axis points down, so the gradient's angle counter-clockwise is of (gx, -gy).
        let gy = (ink(0.0, -h) - ink(0.0, h)) / s.length_mm;
        if math::hypot(gx, gy) >= FLAT {
            deg += math::atan2(gy, gx) * 180.0 / PI + 90.0;
        }
    }
    lines::direction(deg).0
}

/// Turn each point into one dash of `length_mm` centred on it, both ends clamped to the tone map.
///
/// The run reports one frame, labelled with the dash count, then gives the paths.
pub struct DashRenderer;

impl Stage for DashRenderer {
    fn name(&self) -> &'static str {
        "dashes"
    }
    fn label(&self) -> &'static str {
        "Dashes"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points, Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<DashSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<DashSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<DashSettings>(self.name(), &self.schema(), settings)?;
        Ok(Box::new(DashRun {
            job: Some(Job {
                points: inputs.points().clone(),
                tone: inputs.tone().clone(),
                s,
            }),
            paths: None,
        }))
    }
}

/// What the first step reads.
struct Job {
    points: Arc<Points>,
    tone: Arc<ToneMap>,
    s: DashSettings,
}

/// The work happens in the first step, which reports one frame; the second gives the paths.
struct DashRun {
    /// `None` once the first step has run.
    job: Option<Job>,
    /// The finished paths, between the first step and the second.
    paths: Option<Paths>,
}

impl StageRun for DashRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some(Job { points, tone, s }) = self.job.take() {
            let rect = tone.rect_mm();
            let reach = s.length_mm / 2.0;
            let mut paths = Paths::new();
            for &p in &points.0 {
                let [ux, uy] = direction(&tone, &s, p);
                let (dx, dy) = (reach * ux, reach * uy);
                paths.push(&[
                    clamp_to(rect, [p[0] - dx, p[1] - dy]),
                    clamp_to(rect, [p[0] + dx, p[1] + dy]),
                ]);
            }
            let frame = Frame {
                stage: String::new(),
                progress: 1.0,
                label: format!("{} dashes", paths.len()),
                points: None,
                paths: Some(paths.clone()),
            };
            self.paths = Some(paths);
            return Ok(Step::Frame(frame));
        }
        let Some(paths) = self.paths.take() else {
            return Err(StageError::Failed(
                "the dashes run has already finished".into(),
            ));
        };
        Ok(Step::Done(Artifact::Paths(Arc::new(paths))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math;
    use crate::test_util::{assert_inside, bounds, run_stage, tone_map};
    use crate::types::Points;
    use serde_json::{json, Value};
    use std::sync::Arc;

    /// Run the renderer over `points` on a 20 x 20 px map (40 x 40 mm from (10, 10)) of `ink`.
    fn run(points: Vec<[f64; 2]>, ink: impl Fn(u32, u32) -> f32, patch: Value) -> Arc<Paths> {
        let tone = Artifact::Tone(Arc::new(tone_map(20, 20, ink)));
        let points = Artifact::Points(Arc::new(Points(points)));
        match run_stage(&DashRenderer, vec![points, tone], patch).1 {
            Artifact::Paths(p) => p,
            a => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    /// Ink 1 left of the vertical edge at x = 30 mm, 0 right of it.
    fn edge(x: u32, _y: u32) -> f32 {
        if x < 10 {
            1.0
        } else {
            0.0
        }
    }

    /// The vector from the first to the last vertex of the one dash `paths` holds.
    fn span(paths: &Paths) -> [f64; 2] {
        let d = paths.get(0);
        assert_eq!(d.len(), 2);
        [d[1][0] - d[0][0], d[1][1] - d[0][1]]
    }

    #[test]
    fn settings_schema() {
        let s = DashRenderer.schema();
        assert_eq!(bounds(&s, "length_mm"), (0.2, 10.0, 1.5));
        assert_eq!(bounds(&s, "angle_deg"), (0.0, 180.0, 0.0));
        assert_eq!(DashRenderer.defaults()["follow_edges"], true);
        assert_eq!(
            (DashRenderer.name(), DashRenderer.label()),
            ("dashes", "Dashes")
        );
        assert_eq!(DashRenderer.version(), 1);
        assert_eq!(
            (DashRenderer.requires(), DashRenderer.provides()),
            (&[Kind::Points, Kind::Tone][..], Kind::Paths)
        );
    }

    #[test]
    fn dashes_lie_along_a_vertical_edge() {
        let paths = run(vec![[30.0, 30.0]], edge, json!({}));
        let [dx, dy] = span(&paths);
        assert!(dx.abs() < 1e-6, "{dx}");
        assert!((math::hypot(dx, dy) - 1.5).abs() < 1e-9);
        // Centred on the point.
        let d = paths.get(0);
        assert!(((d[0][1] + d[1][1]) / 2.0 - 30.0).abs() < 1e-9);
    }

    #[test]
    fn the_angle_turns_the_dash_from_the_edge() {
        let paths = run(vec![[30.0, 30.0]], edge, json!({"angle_deg": 90}));
        let [dx, dy] = span(&paths);
        assert!(dy.abs() < 1e-6, "{dy}");
        assert!((dx.abs() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn flat_ink_uses_the_angle() {
        let paths = run(vec![[30.0, 30.0]], |_, _| 0.5, json!({"angle_deg": 90}));
        let [dx, dy] = span(&paths);
        assert!(dx.abs() < 1e-6, "{dx}");
        assert!((dy.abs() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn fixed_angle_ignores_edges() {
        let patch = json!({"follow_edges": false, "angle_deg": 0});
        let paths = run(vec![[30.0, 30.0]], edge, patch);
        let [dx, dy] = span(&paths);
        assert!(dy.abs() < 1e-6, "{dy}");
        assert!((dx.abs() - 1.5).abs() < 1e-9);
    }

    #[test]
    fn a_fixed_angle_turns_counter_clockwise_on_the_page() {
        // Like hatch: 45 degrees rises to the right, and the page's y axis points down.
        let patch = json!({"follow_edges": false, "angle_deg": 45});
        let [dx, dy] = span(&run(vec![[30.0, 30.0]], edge, patch));
        let want = crate::stages::lines::direction(45.0).0;
        assert!(dx > 0.0 && dy < 0.0, "{dx} {dy}");
        assert!((dx - 1.5 * want[0]).abs() < 1e-9 && (dy - 1.5 * want[1]).abs() < 1e-9);
    }

    #[test]
    fn the_turn_from_an_edge_is_counter_clockwise_on_the_page() {
        // The vertical edge turned 45 degrees counter-clockwise leans like a backslash: its top
        // end moves left, so dx and dy (y down) share a sign. A clockwise turn would give a slash.
        let [dx, dy] = span(&run(vec![[30.0, 30.0]], edge, json!({"angle_deg": 45})));
        assert!(dx * dy > 0.0, "{dx} {dy}");
        assert!(
            (dx.abs() - 1.5 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9,
            "{dx}"
        );
        assert!((dx.abs() - dy.abs()).abs() < 1e-6, "{dx} {dy}");
    }

    #[test]
    fn a_dash_on_a_diagonal_ramp_runs_across_the_gradient() {
        // Ink grows to the right and down the page, so the edge runs from lower left to upper
        // right: one of dx and dy (y down) is positive and the other negative. Reading the
        // gradient's y with the page's sign instead would lay the dash along the gradient.
        let ramp = |x: u32, y: u32| (x + y) as f32 / 38.0;
        let [dx, dy] = span(&run(vec![[30.0, 30.0]], ramp, json!({"angle_deg": 0})));
        assert!(dx * dy < 0.0, "{dx} {dy}");
        assert!((dx.abs() - dy.abs()).abs() < 1e-6, "{dx} {dy}");
    }

    #[test]
    fn dashes_stay_on_the_map() {
        let rect = [10.0, 10.0, 40.0, 40.0];
        let corners = vec![[10.0, 10.0], [50.0, 10.0], [10.0, 50.0], [50.0, 50.0]];
        let paths = run(
            corners,
            |_, _| 0.5,
            json!({"angle_deg": 45, "length_mm": 10}),
        );
        assert_eq!(paths.len(), 4);
        assert_inside(&paths, rect);
    }

    #[test]
    fn one_frame_of_dashes_then_the_paths() {
        let tone = Artifact::Tone(Arc::new(tone_map(20, 20, edge)));
        let points = Artifact::Points(Arc::new(Points(vec![[30.0, 30.0], [20.0, 20.0]])));
        let (frames, paths) = run_stage(&DashRenderer, vec![points, tone], json!({}));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].label, "2 dashes");
        assert_eq!(frames[0].progress, 1.0);
        assert_eq!(frames[0].paths.as_ref().unwrap().len(), 2);
        let Artifact::Paths(p) = paths else {
            panic!("expected paths")
        };
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"length_mm": 0.1}),
            json!({"angle_deg": 181}),
            json!({"follow_edges": "yes"}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&DashRenderer.defaults(), &patch);
            let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let points = Artifact::Points(Arc::new(Points(vec![])));
            let inputs = Inputs::new([points, tone].into_iter().map(|a| (a.kind(), a)).collect());
            let ctx = RunContext::new(Arc::default());
            let err = DashRenderer.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }

    #[test]
    fn a_finished_run_fails_instead_of_panicking() {
        let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
        let points = Artifact::Points(Arc::new(Points(vec![])));
        let inputs = Inputs::new([points, tone].into_iter().map(|a| (a.kind(), a)).collect());
        let ctx = RunContext::new(Arc::default());
        let mut run = DashRenderer
            .start(inputs, &DashRenderer.defaults(), ctx)
            .unwrap();
        assert!(matches!(run.step(), Ok(Step::Frame(_))));
        assert!(matches!(run.step(), Ok(Step::Done(_))));
        assert!(matches!(run.step(), Err(StageError::Failed(_))));
    }
}
