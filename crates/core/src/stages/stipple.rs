//! The stipple renderer: turns each point into a dot whose area follows the ink under it.
//!
//! A dot of diameter `d` is drawn as an Archimedean spiral from its centre, one pen width between
//! turns, out to the radius where the pen's outer edge meets the dot's rim, and closed by one full
//! circle there; a dot no wider than the pen is a single vertex. The spiral fill is what a pen
//! plotter does to colour in a disc; neither it nor the area-follows-ink sizing is a published
//! algorithm, so there is nothing to cite.

use std::f64::consts::PI;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::lines::{clamp_to, ChunkRun};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};

/// How the stipple stage draws a dot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DotShape {
    /// A single point when the dot is no wider than the pen, otherwise a spiral fill.
    Auto,
    /// Always a single point, whatever the dot's size.
    Point,
    /// The dot's outer circle alone, left hollow.
    Circle,
    /// A spiral from the centre out to the rim, closed by a circle, so the dot plots solid.
    Spiral,
}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the stipple renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct StippleSettings {
    /// Dot diameter over full ink.
    #[schemars(range(min = 0.1, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub size_mm: f64,
    /// How much lighter ink shrinks a dot: 0 keeps every dot the same size.
    #[schemars(range(min = 0.0, max = 1.0), extend("x-step" = 0.01))]
    pub vary: f64,
    /// How a dot is drawn.
    pub shape: DotShape,
}

impl Default for StippleSettings {
    fn default() -> Self {
        StippleSettings {
            size_mm: 0.8,
            vary: 0.5,
            shape: DotShape::Auto,
        }
    }
}

/// The most vertices one run may make, about 320 MB of coordinates before the frames copy them.
const VERTEX_BUDGET: f64 = 20_000_000.0;

/// How a dot of diameter `d` is drawn with a pen `pen_w` wide: `auto` resolved, and `point` when
/// the dot has no room for a ring.
fn resolve(d: f64, pen_w: f64, shape: DotShape) -> DotShape {
    match shape {
        _ if d <= pen_w => DotShape::Point,
        DotShape::Auto => DotShape::Spiral,
        other => other,
    }
}

/// The spacing of arc samples at radius `r`: a quarter pen width, or a 64th of the turn on a wide
/// arc, so a dot's vertex count grows with its turns rather than its area.
fn arc_step(r: f64, pen_w: f64) -> f64 {
    (pen_w / 4.0).max(2.0 * PI * r / 64.0)
}

/// An upper bound on the length of [`dot_path`] for the same dot. A spiral step turns the least
/// at radius `8 pen_w / PI`, where both terms of [`arc_step`] agree, by about 0.0893 rad, so a turn
/// takes at most 71 steps. The circle takes at most 64 steps after its first vertex, which is the
/// spiral's last; the spiral adds its centre.
fn dot_vertices(d: f64, pen_w: f64, shape: DotShape) -> f64 {
    let rim = (d - pen_w) / 2.0;
    let circle = (2.0 * PI * rim / arc_step(rim, pen_w)).ceil() + 1.0;
    match resolve(d, pen_w, shape) {
        DotShape::Spiral => (71.0 * rim / pen_w).ceil() + 1.0 + circle,
        DotShape::Circle => circle,
        _ => 1.0,
    }
}

/// The vertices of a dot of diameter `d` centred on `centre`, drawn with a pen `pen_w` wide. The
/// path reaches `(d - pen_w) / 2` from the centre, so the pen's own width brings the ink out to
/// `d`. A dot with no room for that is one vertex. Arcs at radius `r` are sampled no more than
/// `max(pen_w / 4, 2 PI r / 64)` apart (see [`arc_step`]). The path may leave the page; the
/// caller clamps it.
pub(crate) fn dot_path(centre: [f64; 2], d: f64, pen_w: f64, shape: DotShape) -> Vec<[f64; 2]> {
    let shape = resolve(d, pen_w, shape);
    let rim = (d - pen_w) / 2.0;
    if shape == DotShape::Point {
        return vec![centre];
    }
    let at = |r: f64, theta: f64| {
        [
            centre[0] + r * math::cos(theta),
            centre[1] + r * math::sin(theta),
        ]
    };
    let mut path = Vec::new();
    // The spiral is r = pitch * theta, one pen width further out per turn.
    let pitch = pen_w / (2.0 * PI);
    let end = rim / pitch;
    if shape == DotShape::Spiral {
        path.push(centre);
        let mut theta = 0.0;
        while theta < end {
            // The arc over a step is at most dtheta times the speed at its far end, and r grows by
            // at most `step` over the step, so the chord is no longer than `step`.
            let step = arc_step(pitch * theta, pen_w);
            let speed = math::hypot(pitch * theta + step, pitch);
            theta = (theta + step / speed).min(end);
            path.push(at(pitch * theta, theta));
        }
    }
    // The circle starts where the spiral ended, or at angle 0 when it stands alone.
    let start = if shape == DotShape::Spiral { end } else { 0.0 };
    let n = ((2.0 * PI * rim / arc_step(rim, pen_w)).ceil() as usize).max(1);
    if shape == DotShape::Circle {
        path.push(at(rim, start));
    }
    let first = path[path.len() - 1];
    path.extend((1..n).map(|j| at(rim, start + 2.0 * PI * j as f64 / n as f64)));
    // Closed exactly, not to within the rounding of a full turn.
    path.push(first);
    path
}

/// Turn each point into a dot of diameter `size_mm * sqrt(1 - vary + vary * ink)`, where `ink` is
/// the tone map's ink under the point, so a dot's area follows the ink. Dots are drawn for the
/// first pen.
///
/// The run makes the dots a chunk at a time, reporting about 30 frames labelled with the dots
/// done, then gives the paths. It fails at the start if the dots would need more than 20 million
/// vertices, which a fine pen under large dots can reach.
pub struct StippleRenderer;

impl Stage for StippleRenderer {
    fn name(&self) -> &'static str {
        "stipple"
    }
    fn label(&self) -> &'static str {
        "Stipple dots"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points, Kind::Tone, Kind::Pens]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<StippleSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<StippleSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<StippleSettings>(self.name(), &self.schema(), settings)?;
        let pen_w = inputs
            .pens()
            .first()
            .ok_or_else(|| StageError::Failed("stipple needs a pen".into()))?
            .width_mm;
        let (points, tone) = (inputs.points().clone(), inputs.tone().clone());
        // `max` guards the root against ink a hair outside 0..1.
        let diameter =
            move |ink: f32| s.size_mm * (1.0 - s.vary + s.vary * f64::from(ink)).max(0.0).sqrt();
        let need: f64 = points
            .0
            .iter()
            .map(|&p| dot_vertices(diameter(tone.sample(p)), pen_w, s.shape))
            .sum();
        if need > VERTEX_BUDGET {
            return Err(StageError::Failed(format!(
                "these dots need about {:.0} million vertices; use a wider pen, smaller or fewer dots",
                need / 1e6
            )));
        }
        let rect = tone.rect_mm();
        Ok(Box::new(ChunkRun::new(
            points.0.len(),
            "dots",
            Box::new(move |i, paths, _| {
                let p = points.0[i];
                let dot: Vec<[f64; 2]> = dot_path(p, diameter(tone.sample(p)), pen_w, s.shape)
                    .into_iter()
                    .map(|v| clamp_to(rect, v))
                    .collect();
                paths.push(&dot);
            }),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math;
    use crate::stage::{Artifact, Frame};
    use crate::test_util::{bounds, run_stage, tone_map};
    use crate::types::{Paths, Pen, Points};
    use serde_json::{json, Value};
    use std::sync::Arc;

    const C: [f64; 2] = [20.0, 20.0];

    fn pens() -> Artifact {
        pen(0.3)
    }

    fn pen(width_mm: f64) -> Artifact {
        Artifact::Pens(Arc::new(vec![Pen::new("p", width_mm, "#000000").unwrap()]))
    }

    /// Panic unless every step of `path` is no longer than the arc step at the distance of its
    /// first end from `c`.
    fn assert_spacing(path: &[[f64; 2]], c: [f64; 2], pen_w: f64) {
        for w in path.windows(2) {
            let gap = math::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let r = math::hypot(w[0][0] - c[0], w[0][1] - c[1]);
            let most = (pen_w / 4.0).max(2.0 * PI * r / 64.0);
            assert!(gap <= most + 1e-9, "{gap} > {most} at r {r}");
        }
    }

    /// Run the renderer over `points` on a flat 10 x 10 px map (20 x 20 mm from (10, 10)) of `ink`.
    fn run(points: Vec<[f64; 2]>, ink: f32, patch: Value) -> (Vec<Frame>, Arc<Paths>) {
        let tone = Artifact::Tone(Arc::new(tone_map(10, 10, |_, _| ink)));
        let points = Artifact::Points(Arc::new(Points(points)));
        match run_stage(&StippleRenderer, vec![points, tone, pens()], patch) {
            (frames, Artifact::Paths(p)) => (frames, p),
            (_, a) => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    fn reach(path: &[[f64; 2]], c: [f64; 2]) -> f64 {
        let far = |m: f64, p: &[f64; 2]| m.max(math::hypot(p[0] - c[0], p[1] - c[1]));
        path.iter().fold(0.0, far)
    }

    #[test]
    fn settings_schema() {
        let s = StippleRenderer.schema();
        assert_eq!(bounds(&s, "size_mm"), (0.1, 10.0, 0.8));
        assert_eq!(bounds(&s, "vary"), (0.0, 1.0, 0.5));
        assert_eq!(StippleRenderer.defaults()["shape"], "auto");
        assert_eq!(
            (StippleRenderer.name(), StippleRenderer.label()),
            ("stipple", "Stipple dots")
        );
        assert_eq!(
            (StippleRenderer.requires(), StippleRenderer.provides()),
            (&[Kind::Points, Kind::Tone, Kind::Pens][..], Kind::Paths)
        );
    }

    #[test]
    fn a_dot_no_wider_than_the_pen_is_one_vertex() {
        assert_eq!(dot_path(C, 0.3, 0.3, DotShape::Auto).len(), 1);
        assert_eq!(dot_path(C, 0.1, 0.3, DotShape::Spiral), vec![C]);
        assert_eq!(dot_path(C, 2.0, 0.3, DotShape::Point), vec![C]);
    }

    #[test]
    fn a_wide_dot_spirals_out_to_its_rim() {
        let path = dot_path([0.0, 0.0], 2.0, 0.3, DotShape::Auto);
        assert_eq!(path[0], [0.0, 0.0]);
        assert!((reach(&path, [0.0, 0.0]) - 0.85).abs() < 1e-9);
        assert_spacing(&path, [0.0, 0.0], 0.3);
        // The spiral turns about 0.85 / 0.3 times, so it has more vertices than its rim alone.
        assert!(path.len() > dot_path([0.0, 0.0], 2.0, 0.3, DotShape::Circle).len());
    }

    #[test]
    fn circle_is_the_rim_alone() {
        let path = dot_path([0.0, 0.0], 2.0, 0.3, DotShape::Circle);
        for p in &path {
            assert!((math::hypot(p[0], p[1]) - 0.85).abs() < 1e-9, "{p:?}");
        }
        assert_eq!(path.first(), path.last());
        assert_spacing(&path, [0.0, 0.0], 0.3);
        // A quarter pen apart the rim would need 72 vertices; a 64th of the turn caps it at 64.
        assert_eq!(path.len(), 65);
    }

    #[test]
    fn a_small_arc_keeps_the_quarter_pen_spacing() {
        // A rim of 0.1 mm is 0.63 mm round, under 64 quarter pens, so the quarter pen sets it.
        let path = dot_path([0.0, 0.0], 0.5, 0.3, DotShape::Circle);
        assert_eq!(path.len(), 10);
        assert_spacing(&path, [0.0, 0.0], 0.3);
    }

    #[test]
    fn a_wide_dot_with_a_fine_pen_takes_at_most_71_vertices_a_turn() {
        // 99.5 turns: a quarter pen apart, the spiral alone would need about 125,000 vertices.
        let path = dot_path([0.0, 0.0], 10.0, 0.05, DotShape::Spiral);
        assert!((reach(&path, [0.0, 0.0]) - 4.975).abs() < 1e-9);
        assert_spacing(&path, [0.0, 0.0], 0.05);
        assert!(path.len() <= 71 * 100 + 66, "{}", path.len());
        assert!(path.len() > 64 * 99, "{}", path.len());
    }

    #[test]
    fn the_vertex_estimate_bounds_every_dot() {
        for shape in [
            DotShape::Auto,
            DotShape::Point,
            DotShape::Circle,
            DotShape::Spiral,
        ] {
            for pen_w in [0.03, 0.05, 0.3, 1.0] {
                for d in (1..=100).map(|i| f64::from(i) * 0.1) {
                    let len = dot_path([0.0, 0.0], d, pen_w, shape).len() as f64;
                    let most = dot_vertices(d, pen_w, shape);
                    assert!(len <= most, "{shape:?} {pen_w} {d}: {len} > {most}");
                    // Close enough that the budget refuses only what is near it.
                    assert!(
                        most <= 1.25 * len + 70.0,
                        "{shape:?} {pen_w} {d}: {most} vs {len}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_circle_no_wider_than_the_pen_is_one_vertex() {
        assert_eq!(dot_path(C, 0.2, 0.3, DotShape::Circle), vec![C]);
    }

    #[test]
    fn diameter_follows_the_square_root_of_ink() {
        let patch = json!({"size_mm": 2.0, "vary": 1.0});
        let (_, full) = run(vec![C], 1.0, patch.clone());
        assert!((reach(full.get(0), C) - 0.85).abs() < 1e-9);
        let (_, quarter) = run(vec![C], 0.25, patch);
        assert!((reach(quarter.get(0), C) - 0.35).abs() < 1e-9);
        // With no variation every dot keeps the full size.
        let (_, same) = run(vec![C], 0.25, json!({"size_mm": 2.0, "vary": 0.0}));
        assert!((reach(same.get(0), C) - 0.85).abs() < 1e-9);
    }

    #[test]
    fn about_thirty_frames_of_dots_then_the_paths() {
        let points = (0..300)
            .map(|i| [10.0 + f64::from(i) / 15.0, 20.0])
            .collect();
        let (frames, paths) = run(points, 1.0, json!({}));
        assert_eq!(paths.len(), 300);
        assert_eq!(frames.len(), 30);
        assert_eq!(frames[0].label, "10/300 dots");
        assert_eq!(frames[0].paths.as_ref().unwrap().len(), 10);
        let last = frames.last().unwrap();
        assert_eq!(last.label, "300/300 dots");
        assert_eq!(last.progress, 1.0);
        assert_eq!(last.paths.as_ref().unwrap().len(), 300);
    }

    #[test]
    fn dots_beyond_the_vertex_budget_fail_at_start() {
        // 3000 dots of 10 mm with a 0.05 mm pen need about 21 million vertices.
        let start = |n: usize| {
            let tone = Artifact::Tone(Arc::new(tone_map(10, 10, |_, _| 1.0)));
            let points = Artifact::Points(Arc::new(Points(vec![C; n])));
            let inputs = Inputs::new(
                [points, tone, pen(0.05)]
                    .into_iter()
                    .map(|a| (a.kind(), a))
                    .collect(),
            );
            let patch = json!({"size_mm": 10.0, "shape": "spiral"});
            let settings = crate::settings::merge(&StippleRenderer.defaults(), &patch);
            StippleRenderer.start(inputs, &settings, RunContext::new(Arc::default()))
        };
        match start(3000).err().unwrap() {
            StageError::Failed(m) => {
                assert!(
                    m.starts_with("these dots need about 21 million vertices"),
                    "{m}"
                );
                assert!(m.contains("wider pen"), "{m}");
            }
            e => panic!("{e:?}"),
        }
        // A tenth as many dots fit.
        assert!(start(300).is_ok());
    }

    #[test]
    fn a_dot_at_the_edge_stays_on_the_map() {
        let rect = [10.0, 10.0, 20.0, 20.0];
        let (_, paths) = run(
            vec![[10.0, 10.0], [30.0, 30.0]],
            1.0,
            json!({"size_mm": 4.0}),
        );
        crate::test_util::assert_inside(&paths, rect);
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        for patch in [
            json!({"size_mm": 0.05}),
            json!({"vary": 1.5}),
            json!({"shape": "square"}),
            json!({"nope": 1}),
        ] {
            let settings = crate::settings::merge(&StippleRenderer.defaults(), &patch);
            let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let points = Artifact::Points(Arc::new(Points(vec![])));
            let inputs = Inputs::new(
                [points, tone, pens()]
                    .into_iter()
                    .map(|a| (a.kind(), a))
                    .collect(),
            );
            let ctx = RunContext::new(Arc::default());
            let err = StippleRenderer.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }
}
