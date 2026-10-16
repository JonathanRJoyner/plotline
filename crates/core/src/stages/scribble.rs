//! The scribble stage: circular loops traced along a path, small and close where the image is
//! dark, wide and loose where it is light.
//!
//! After C.-C. Chiu, Y.-H. Lo, R.-R. Lee and H.-K. Chu, "Tone- and Feature-Aware Circular Scribble
//! Art", *Computer Graphics Forum* 34(7), 2015 (Proc. Pacific Graphics), pp. 225-234,
//! doi:10.1111/cgf.12761. A point turns on a disk whose centre moves along the path (the paper's
//! rolling disk, a trochoid). With luminance `L` under the centre, the disk's radius is
//! `Rmin + L (Rmax - Rmin)` and the centre moves `Vmin + L (Vmax - Vmin)` in one turn, the turning
//! speed being constant; while the move per turn is under the loop's circumference the point
//! loops. The disk's normal tilts the loops into ellipses; here the tilt is one fixed ellipse for
//! the whole drawing.
//!
//! plotline takes the path from the `tsp` stage in place of the paper's tour of superpixels, and
//! leaves out the paper's cut of the radius near edges, its blurred tone curve and the jitter of
//! its normal.

use std::f64::consts::PI;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contours::Budgeted;
use super::lines::{clamp_to, direction, ChunkRun};
use super::stipple::arc_step;
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Inputs, Kind, RunContext, Stage, StageError, StageRun};
use crate::types::Paths;

/// The most vertices one run may make, as for the stipple dots: about 320 MB of coordinates
/// before the frames copy them.
const VERTEX_BUDGET: usize = 20_000_000;

/// What the run fails with once it passes [`VERTEX_BUDGET`].
const TOO_BIG: &str =
    "the scribble needs over 20 million vertices; use wider spacing, fewer points or a wider pen";

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the scribble stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ScribbleSettings {
    /// Radius of a loop over full ink.
    #[schemars(range(min = 0.1, max = 10.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub radius_mm: f64,
    /// Radius of a loop over white.
    #[schemars(range(min = 0.1, max = 20.0), extend("x-step" = 0.1, "x-unit" = "mm"))]
    pub white_radius_mm: f64,
    /// How far the loops move along the line in one turn over full ink; closer loops draw darker.
    #[schemars(range(min = 0.05, max = 10.0), extend("x-step" = 0.05, "x-unit" = "mm"))]
    pub spacing_mm: f64,
    /// How far the loops move along the line in one turn over white.
    #[schemars(range(min = 0.05, max = 20.0), extend("x-step" = 0.05, "x-unit" = "mm"))]
    pub white_spacing_mm: f64,
    /// How far the loops flatten into ellipses; 0 draws circles.
    #[schemars(range(min = 0.0, max = 0.9), extend("x-step" = 0.05))]
    pub flatten: f64,
    /// Direction of the flattened loops' long axis, counter-clockwise from horizontal.
    #[schemars(range(min = 0.0, max = 180.0), extend("x-step" = 1.0, "x-unit" = "deg"))]
    pub tilt_deg: f64,
}

impl Default for ScribbleSettings {
    fn default() -> Self {
        ScribbleSettings {
            radius_mm: 0.8,
            white_radius_mm: 2.5,
            spacing_mm: 0.5,
            white_spacing_mm: 8.0,
            flatten: 0.0,
            tilt_deg: 0.0,
        }
    }
}

/// How a loop is drawn: the settings, the first pen's width and the map the loops stay on.
struct Loops {
    s: ScribbleSettings,
    pen_w: f64,
    rect: [f64; 4],
    /// The long axis of a flattened loop, and the unit normal to it.
    axes: ([f64; 2], [f64; 2]),
}

impl Loops {
    /// For a centre with `ink` under it: the loop's radius, how far the centre moves in one turn,
    /// and the vertices in a turn, which is [`arc_step`] apart on the loop and between 8 and 64.
    fn at(&self, ink: f32) -> (f64, f64, f64) {
        let s = &self.s;
        let light = f64::from(1.0 - ink).clamp(0.0, 1.0);
        let r = s.radius_mm + light * (s.white_radius_mm - s.radius_mm);
        let v = s.spacing_mm + light * (s.white_spacing_mm - s.spacing_mm);
        let k = (2.0 * PI * r / arc_step(r, self.pen_w))
            .ceil()
            .clamp(8.0, 64.0);
        (r, v, k)
    }

    /// The point at turn angle `phi` on the loop of radius `r` around `c`, inside the map. The
    /// loop turns counter-clockwise as seen on the page, whose y axis points down.
    fn vertex(&self, c: [f64; 2], r: f64, phi: f64) -> [f64; 2] {
        let (long, normal) = self.axes;
        let (along, across) = (
            r * math::cos(phi),
            (1.0 - self.s.flatten) * r * math::sin(phi),
        );
        clamp_to(
            self.rect,
            [
                c[0] + along * long[0] - across * normal[0],
                c[1] + along * long[1] - across * normal[1],
            ],
        )
    }
}

/// The part of an input path from `a` to `b`, lying from `s0` to `s1` along the path.
struct Segment {
    a: [f64; 2],
    b: [f64; 2],
    s0: f64,
    s1: f64,
    first: bool,
    last: bool,
}

/// The segments of every path of `paths`, in order. A path of one vertex, or of no length, is one
/// segment from its first vertex to itself, which ends at arc length 0.
fn segments(paths: &Paths) -> Vec<Segment> {
    let mut out = Vec::new();
    for path in paths.iter() {
        let length = Paths::path_length(path);
        if path.len() < 2 || length <= 0.0 {
            let a = path[0];
            out.push(Segment {
                a,
                b: a,
                s0: 0.0,
                s1: 0.0,
                first: true,
                last: true,
            });
            continue;
        }
        let mut s = 0.0;
        for (i, w) in path.windows(2).enumerate() {
            let s1 = s + math::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]);
            out.push(Segment {
                a: w[0],
                b: w[1],
                s0: s,
                s1,
                first: i == 0,
                last: i + 2 == path.len(),
            });
            s = s1;
        }
    }
    out
}

/// Trace loops along each input path: one output path per input path. A centre moves along the
/// path while a point turns around it, and each vertex is that point after a step of `2 PI / k`
/// of a turn, where the loop has `k` vertices (see [`Loops::at`]). The radius and the move per
/// turn come from the ink under the centre. The first vertex lies a radius from the path's start
/// along the loops' long axis, and the last is a point of a loop around its end. A path of one
/// vertex, or of no length, becomes one closed loop around its vertex.
///
/// The run is a [`ChunkRun`] over the input segments, with the turn angle and the arc length
/// carried from one segment to the next, reporting about 30 frames labelled with the segments
/// done. It fails once the paths pass 20 million vertices.
pub struct ScribbleStage;

/// The run of [`ScribbleStage`], failing once its paths pass `limit` vertices.
fn run_with_limit(
    inputs: Inputs,
    settings: &Value,
    limit: usize,
) -> Result<Box<dyn StageRun>, StageError> {
    let s = validate::<ScribbleSettings>("scribble", &ScribbleStage.schema(), settings)?;
    let pen_w = inputs
        .pens()
        .first()
        .ok_or_else(|| StageError::Failed("scribble needs a pen".into()))?
        .width_mm;
    let tone = inputs.tone().clone();
    let loops = Loops {
        axes: direction(s.tilt_deg),
        s,
        pen_w,
        rect: tone.rect_mm(),
    };
    let segments = segments(inputs.paths());
    let (mut phi, mut arc) = (0.0, 0.0);
    let run = ChunkRun::new(
        segments.len(),
        "segments",
        Box::new(move |i, paths, open| {
            let seg = &segments[i];
            if seg.first {
                (phi, arc) = (0.0, 0.0);
            }
            if seg.last && seg.s1 == 0.0 {
                let (r, _, k) = loops.at(tone.sample(seg.a));
                let mut ring: Vec<[f64; 2]> = (0..k as usize)
                    .map(|j| loops.vertex(seg.a, r, 2.0 * PI * j as f64 / k))
                    .collect();
                // Closed exactly, not to within the rounding of a full turn.
                ring.push(ring[0]);
                paths.push(&ring);
                return;
            }
            while arc < seg.s1 {
                let f = (arc - seg.s0) / (seg.s1 - seg.s0);
                let c = [
                    seg.a[0] + (seg.b[0] - seg.a[0]) * f,
                    seg.a[1] + (seg.b[1] - seg.a[1]) * f,
                ];
                let (r, v, k) = loops.at(tone.sample(c));
                open.push(loops.vertex(c, r, phi));
                phi += 2.0 * PI / k;
                arc += v / k;
            }
            if seg.last {
                let (r, _, _) = loops.at(tone.sample(seg.b));
                open.push(loops.vertex(seg.b, r, phi));
                paths.push(open);
                open.clear();
            }
        }),
    );
    Ok(Box::new(Budgeted::new(run, limit, TOO_BIG)))
}

impl Stage for ScribbleStage {
    fn name(&self) -> &'static str {
        "scribble"
    }
    fn label(&self) -> &'static str {
        "Scribble"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Paths, Kind::Tone, Kind::Pens]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<ScribbleSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<ScribbleSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        run_with_limit(inputs, settings, VERTEX_BUDGET)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{Artifact, Frame, Step};
    use crate::test_util::{assert_inside, bounds, run_stage, tone_map};
    use crate::types::{Pen, ToneMap};
    use serde_json::json;
    use std::sync::Arc;

    /// The map is 20 by 10 mm from (10, 10), and the line runs through its middle.
    const RECT: [f64; 4] = [10.0, 10.0, 20.0, 10.0];
    const LINE: [[f64; 2]; 2] = [[14.0, 15.0], [26.0, 15.0]];

    fn white() -> ToneMap {
        tone_map(10, 5, |_, _| 0.0)
    }

    fn black() -> ToneMap {
        tone_map(10, 5, |_, _| 1.0)
    }

    fn inputs(lines: &[&[[f64; 2]]], tone: ToneMap) -> Vec<Artifact> {
        let mut paths = Paths::new();
        for line in lines {
            paths.push(line);
        }
        let pen = Pen::new("p", 0.3, "#000000").unwrap();
        vec![
            Artifact::Paths(Arc::new(paths)),
            Artifact::Tone(Arc::new(tone)),
            Artifact::Pens(Arc::new(vec![pen])),
        ]
    }

    /// Run the stage over `lines` and the tone map, with a 0.3 mm pen and `patch` over loops of
    /// 1 mm over ink and 3 mm over white, 1 and 4 mm a turn, so that the tests do not move with
    /// the defaults.
    fn run(lines: &[&[[f64; 2]]], tone: ToneMap, patch: Value) -> (Vec<Frame>, Arc<Paths>) {
        let base = json!({
            "radius_mm": 1.0, "white_radius_mm": 3.0, "spacing_mm": 1.0, "white_spacing_mm": 4.0
        });
        let patch = crate::settings::merge(&base, &patch);
        match run_stage(&ScribbleStage, inputs(lines, tone), patch) {
            (frames, Artifact::Paths(p)) => (frames, p),
            (_, a) => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    /// The most a vertex of `path` lies from the line `y = 15`.
    fn reach(path: &[[f64; 2]]) -> f64 {
        path.iter()
            .fold(0.0, |m, v| f64::max(m, (v[1] - 15.0).abs()))
    }

    #[test]
    fn settings_schema() {
        let s = ScribbleStage.schema();
        assert_eq!(bounds(&s, "radius_mm"), (0.1, 10.0, 0.8));
        assert_eq!(bounds(&s, "white_radius_mm"), (0.1, 20.0, 2.5));
        assert_eq!(bounds(&s, "spacing_mm"), (0.05, 10.0, 0.5));
        assert_eq!(bounds(&s, "white_spacing_mm"), (0.05, 20.0, 8.0));
        assert_eq!(bounds(&s, "flatten"), (0.0, 0.9, 0.0));
        assert_eq!(bounds(&s, "tilt_deg"), (0.0, 180.0, 0.0));
        assert_eq!(
            (ScribbleStage.name(), ScribbleStage.label()),
            ("scribble", "Scribble")
        );
        assert_eq!(
            (ScribbleStage.requires(), ScribbleStage.provides()),
            (&[Kind::Paths, Kind::Tone, Kind::Pens][..], Kind::Paths)
        );
    }

    #[test]
    fn over_white_the_loops_are_wide_and_far_apart() {
        let (_, paths) = run(&[&LINE], white(), json!({}));
        assert_eq!(paths.len(), 1);
        let path = paths.get(0);
        assert!((reach(path) - 3.0).abs() < 1e-6, "{}", reach(path));
        // Each turn crosses the line twice; the 12 mm line takes 3 turns of 4 mm.
        let above: Vec<bool> = path.iter().map(|v| v[1] > 15.0).collect();
        let crossings = above.windows(2).filter(|w| w[0] != w[1]).count();
        assert!((crossings as f64 / 2.0 - 3.0).abs() <= 1.0, "{crossings}");
        // The loops start a radius to the right of the line's start and end around its end.
        assert!((path[0][0] - 17.0).abs() < 1e-9 && (path[0][1] - 15.0).abs() < 1e-9);
        let last = path[path.len() - 1];
        let gap = math::hypot(last[0] - 26.0, last[1] - 15.0);
        assert!((gap - 3.0).abs() < 1e-9, "{gap}");
    }

    #[test]
    fn over_full_ink_the_loops_are_small_and_close() {
        let (_, paths) = run(&[&LINE], black(), json!({}));
        let path = paths.get(0);
        assert!((reach(path) - 1.0).abs() < 1e-6, "{}", reach(path));
        let (_, white_paths) = run(&[&LINE], white(), json!({}));
        assert!(path.len() > 3 * white_paths.get(0).len());
    }

    #[test]
    fn loops_turn_counter_clockwise_on_the_page() {
        let (_, paths) = run(&[&LINE], white(), json!({}));
        // From the rightmost point the loop goes up the page, towards smaller y.
        assert!(paths.get(0)[1][1] < 15.0);
    }

    #[test]
    fn the_ink_under_the_centre_sets_the_loop() {
        // Ink to the left of x = 20 mm, white to the right.
        let half = |x: u32, _| if x < 5 { 1.0 } else { 0.0 };
        let (_, paths) = run(
            &[&[[12.0, 15.0], [28.0, 15.0]]],
            tone_map(10, 5, half),
            json!({}),
        );
        let path = paths.get(0);
        let dark: Vec<_> = path.iter().filter(|v| v[0] < 16.0).copied().collect();
        let light: Vec<_> = path.iter().filter(|v| v[0] > 24.0).copied().collect();
        assert!((reach(&dark) - 1.0).abs() < 1e-6, "{}", reach(&dark));
        assert!((reach(&light) - 3.0).abs() < 1e-6, "{}", reach(&light));
    }

    #[test]
    fn flatten_narrows_the_loops_across_the_line() {
        let (_, round) = run(&[&LINE], white(), json!({}));
        let (_, flat) = run(&[&LINE], white(), json!({"flatten": 0.5}));
        assert!((reach(flat.get(0)) / reach(round.get(0)) - 0.5).abs() < 1e-6);
        // Turned a quarter turn, the long axis stands across the line instead.
        let (_, turned) = run(&[&LINE], white(), json!({"flatten": 0.5, "tilt_deg": 90.0}));
        assert!((reach(turned.get(0)) - reach(round.get(0))).abs() < 1e-6);
    }

    #[test]
    fn a_bent_path_scribbles_as_one_path() {
        // The same line with a vertex in the middle gives the same loops, in one path.
        let (_, straight) = run(&[&LINE], white(), json!({}));
        let (_, bent) = run(
            &[&[[14.0, 15.0], [20.0, 15.0], [26.0, 15.0]]],
            white(),
            json!({}),
        );
        assert_eq!(bent.len(), 1);
        let (a, b) = (straight.get(0), bent.get(0));
        assert_eq!(a.len(), b.len());
        for (u, v) in a.iter().zip(b) {
            assert!((u[0] - v[0]).abs() < 1e-9 && (u[1] - v[1]).abs() < 1e-9);
        }
    }

    #[test]
    fn each_input_path_gives_one_scribble() {
        let (_, paths) = run(&[&LINE, &[[14.0, 12.0], [26.0, 12.0]]], white(), json!({}));
        assert_eq!(paths.len(), 2);
        // The second starts afresh at angle 0 around its own start.
        assert!((paths.get(1)[0][0] - 17.0).abs() < 1e-9);
    }

    #[test]
    fn a_point_or_a_path_of_no_length_is_one_closed_loop() {
        let point = [20.0, 15.0];
        for line in [&[point][..], &[point, point][..]] {
            let (_, paths) = run(&[line], white(), json!({}));
            assert_eq!(paths.len(), 1);
            let path = paths.get(0);
            assert_eq!(path.len(), 65);
            assert_eq!(path[0], path[64]);
            for v in path {
                let r = math::hypot(v[0] - 20.0, v[1] - 15.0);
                assert!((r - 3.0).abs() < 1e-9, "{r}");
            }
        }
    }

    #[test]
    fn every_vertex_stays_on_the_map() {
        // Along the top edge and round the corner, with loops wider than the distance to the edge.
        let edge: &[[f64; 2]] = &[[10.0, 10.0], [30.0, 10.0], [30.0, 20.0]];
        let (_, paths) = run(&[edge, &[[10.0, 20.0]]], white(), json!({"flatten": 0.3}));
        assert_inside(&paths, RECT);
    }

    #[test]
    fn the_same_inputs_give_the_same_coordinates() {
        let (_, a) = run(&[&LINE], white(), json!({"flatten": 0.4, "tilt_deg": 30.0}));
        let (_, b) = run(&[&LINE], white(), json!({"flatten": 0.4, "tilt_deg": 30.0}));
        assert_eq!(a.coords(), b.coords());
    }

    #[test]
    fn about_thirty_frames_of_a_growing_scribble() {
        let line: Vec<[f64; 2]> = (0..61).map(|i| [12.0 + f64::from(i) * 0.2, 15.0]).collect();
        let (frames, paths) = run(&[&line], white(), json!({}));
        assert_eq!(paths.len(), 1);
        assert_eq!(frames.len(), 30);
        assert_eq!(frames[0].label, "2/60 segments");
        let sizes: Vec<usize> = frames
            .iter()
            .map(|f| f.paths.as_ref().unwrap().coords().len())
            .collect();
        assert!(
            sizes[0] > 0 && sizes.windows(2).all(|w| w[0] <= w[1]),
            "{sizes:?}"
        );
        let last = frames.last().unwrap();
        assert_eq!(
            (last.label.as_str(), last.progress),
            ("60/60 segments", 1.0)
        );
        assert_eq!(last.paths.as_ref().unwrap().coords(), paths.coords());
    }

    #[test]
    fn a_run_past_its_vertex_budget_fails() {
        // The loops over 12 mm at 0.05 mm a turn take about 15,000 vertices (240 turns of 64).
        let patch = json!({"spacing_mm": 0.05, "white_spacing_mm": 0.05});
        let settings = crate::settings::merge(&ScribbleStage.defaults(), &patch);
        let start = |limit| {
            let given = inputs(&[&LINE], white());
            let given = Inputs::new(given.into_iter().map(|a| (a.kind(), a)).collect());
            run_with_limit(given, &settings, limit).unwrap()
        };
        match start(1000).step() {
            Err(StageError::Failed(m)) => assert_eq!(m, TOO_BIG),
            other => panic!("{other:?}"),
        }
        assert!(matches!(start(20_000).step(), Ok(Step::Frame(_))));
        assert!(TOO_BIG.contains(&format!("{} million", VERTEX_BUDGET / 1_000_000)));
    }
}
