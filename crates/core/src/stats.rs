//! Drawing statistics and a plot time estimate.
//!
//! [`compute`] counts the paths of a finished drawing, measures the distance the pen draws and the
//! distance it travels lifted, and times the moves at the speeds and acceleration of a [`Speeds`].

use serde::{Deserialize, Serialize};

use crate::math;
use crate::types::Drawing;

/// The acceleration a [`Speeds`] and a machine take when none is given, in mm/s². At 1000 mm/s²
/// the pen reaches the default 50 mm/s drawing speed in 1.25 mm and 100 mm/s travel in 5 mm.
pub const DEFAULT_ACCEL_MM_S2: f64 = 1000.0;

/// The default of every `accel_mm_s2` field, for serde.
pub(crate) fn default_accel() -> f64 {
    DEFAULT_ACCEL_MM_S2
}

/// How far the path through a corner may pass from the vertex, in mm, which sets how fast the pen
/// may take a corner. 0.01 mm is the default that S. Jeon gives for the cornering rule in
/// [`junction_speed`].
const JUNCTION_DEVIATION_MM: f64 = 0.01;

/// How fast a plotter moves, as a front end lets the user set it.
///
/// `draw_mm_s` is the speed with the pen down and `travel_mm_s` the speed with it up, both in mm
/// per second and above zero. `accel_mm_s2` is how fast the pen speeds up and slows down, in mm
/// per second squared and above zero; JSON without it takes [`DEFAULT_ACCEL_MM_S2`]. `pen_lift_s`
/// is the time to lift the pen and set it down again, which every path costs once.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Speeds {
    /// The speed with the pen down, in mm per second.
    pub draw_mm_s: f64,
    /// The speed with the pen up, in mm per second.
    pub travel_mm_s: f64,
    /// The time to lift the pen and set it down again, in seconds.
    pub pen_lift_s: f64,
    /// How fast the pen speeds up and slows down, in mm per second squared.
    #[serde(default = "default_accel")]
    pub accel_mm_s2: f64,
}

impl Default for Speeds {
    fn default() -> Speeds {
        Speeds {
            draw_mm_s: 50.0,
            travel_mm_s: 100.0,
            pen_lift_s: 0.2,
            accel_mm_s2: DEFAULT_ACCEL_MM_S2,
        }
    }
}

/// What a drawing costs to plot: the number of paths, the pen-down and pen-up distances in mm and
/// the estimated plot time in seconds.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Stats {
    /// The number of paths over all layers.
    pub path_count: usize,
    /// The distance the pen draws, in mm.
    pub pen_down_mm: f64,
    /// The distance the pen travels lifted, in mm.
    pub pen_up_mm: f64,
    /// The estimated plot time, in seconds.
    pub estimated_seconds: f64,
}

/// The stats of `d` when plotted at `speeds`.
///
/// The paths of all layers are taken in order as one run, which the pen starts at the top-left
/// corner of the drawing rect, where the ordering stage starts. The move to the first path and the
/// moves between layers count as travel. Each travel move is one straight move from rest to rest;
/// each path is drawn from rest to rest, slowing at its vertices as [`junction_speed`] allows.
/// Every straight move follows a trapezoidal velocity profile at `accel_mm_s2` (see
/// [`move_seconds`]), so the time is never below the distances over the speeds. `pen_lift_s` is
/// added for each path. Swapping pens between layers is not timed.
pub fn compute(d: &Drawing, speeds: Speeds) -> Stats {
    timed(d, speeds).0
}

/// [`compute`], and the seconds of its pen-up travel alone.
pub(crate) fn timed(d: &Drawing, speeds: Speeds) -> (Stats, f64) {
    let [x, y, _, _] = d.canvas.drawing_rect();
    let mut pen = [x, y];
    let (mut path_count, mut down, mut up) = (0, 0.0, 0.0);
    let (mut draw_s, mut travel_s) = (0.0, 0.0);
    let mut plan = Plan::default();
    let a = speeds.accel_mm_s2;
    for layer in &d.layers {
        path_count += layer.paths.len();
        down += layer.paths.total_length();
        up += layer.paths.travel_length(pen);
        for path in layer.paths.iter() {
            // An empty path moves nothing, and the next layer's first move starts where this
            // layer's last drawn vertex is.
            if let (Some(first), Some(last)) = (path.first(), path.last()) {
                travel_s += move_seconds(distance(pen, *first), 0.0, 0.0, speeds.travel_mm_s, a);
                draw_s += plan.seconds(path, speeds.draw_mm_s, a);
                pen = *last;
            }
        }
    }
    let stats = Stats {
        path_count,
        pen_down_mm: down,
        pen_up_mm: up,
        estimated_seconds: draw_s + travel_s + path_count as f64 * speeds.pen_lift_s,
    };
    (stats, travel_s)
}

fn distance(p: [f64; 2], q: [f64; 2]) -> f64 {
    math::hypot(q[0] - p[0], q[1] - p[1])
}

/// The seconds of a straight move of `l` mm entered at `u` and left at `w` mm/s, at cruise speed
/// `v` and acceleration `a`, on a trapezoidal velocity profile: speed up at `a`, cruise at `v`,
/// slow down at `a` (L. Biagiotti and C. Melchiorri, "Trajectory Planning for Automatic Machines
/// and Robots", Springer, 2008, section 3.2). A move too short to reach `v` peaks below it, so one
/// from rest to rest then takes 2·sqrt(l / a). `u` and `w` must be reachable from each other over
/// `l`, as [`Plan::seconds`] makes them.
fn move_seconds(l: f64, u: f64, w: f64, v: f64, a: f64) -> f64 {
    let peak2 = a * l + (u * u + w * w) / 2.0;
    if peak2 < v * v {
        (2.0 * peak2.sqrt() - u - w) / a
    } else {
        // The cruise time l / v plus what the two ramps lose against it, never below l / v.
        l / v + ((v - u) * (v - u) + (v - w) * (v - w)) / (2.0 * a * v)
    }
}

/// The fastest the pen may pass the vertex between straight moves of unit directions `d0` and
/// `d1`, at most `v`. The pen is taken to round the corner on the circle that passes
/// [`JUNCTION_DEVIATION_MM`] from the vertex, at the speed whose centripetal acceleration is `a`;
/// that circle's radius is the deviation times sin(θ/2) / (1 − sin(θ/2)), where θ is the angle
/// between the two moves, 180° for no turn (S. Jeon, "Improving Grbl: Cornering Algorithm", 2011).
fn junction_speed(d0: [f64; 2], d1: [f64; 2], v: f64, a: f64) -> f64 {
    let cos = -(d0[0] * d1[0] + d0[1] * d1[1]);
    let sin_half = ((1.0 - cos) / 2.0).max(0.0).sqrt();
    if sin_half >= 1.0 {
        return v;
    }
    (a * JUNCTION_DEVIATION_MM * sin_half / (1.0 - sin_half))
        .sqrt()
        .min(v)
}

/// The scratch space for timing a path, kept between paths so they share one allocation.
#[derive(Default)]
struct Plan {
    /// The length and unit direction of each move of the path with a length.
    moves: Vec<(f64, [f64; 2])>,
    /// The speed at each end of each move: `ends[k]` enters move `k`, `ends[k + 1]` leaves it.
    ends: Vec<f64>,
}

impl Plan {
    /// The seconds to draw `path` at cruise speed `v` and acceleration `a`, starting and ending at
    /// rest. Each vertex is held to its [`junction_speed`], then a backward pass lowers each
    /// vertex's speed to one the pen can still slow down from before the end, and a forward pass
    /// to one it can reach from the start, as a look-ahead motion planner does (Jeon, 2011).
    fn seconds(&mut self, path: &[[f64; 2]], v: f64, a: f64) -> f64 {
        self.moves.clear();
        for w in path.windows(2) {
            let l = distance(w[0], w[1]);
            // A repeated vertex is no move and has no direction.
            if l > 0.0 {
                let dir = [(w[1][0] - w[0][0]) / l, (w[1][1] - w[0][1]) / l];
                self.moves.push((l, dir));
            }
        }
        let m = self.moves.len();
        if m == 0 {
            return 0.0;
        }
        self.ends.clear();
        self.ends.push(0.0);
        let mv = &self.moves;
        self.ends
            .extend((1..m).map(|k| junction_speed(mv[k - 1].1, mv[k].1, v, a)));
        self.ends.push(0.0);
        let e = &mut self.ends;
        for k in (1..m).rev() {
            e[k] = e[k].min((e[k + 1] * e[k + 1] + 2.0 * a * mv[k].0).sqrt());
        }
        for k in 1..m {
            e[k] = e[k].min((e[k - 1] * e[k - 1] + 2.0 * a * mv[k - 1].0).sqrt());
        }
        (0..m)
            .map(|k| move_seconds(mv[k].0, e[k], e[k + 1], v, a))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::drawing_with;

    #[test]
    fn stats_count_and_time() {
        // Down 5, up 0 to the first vertex and 10 between the paths.
        let d = drawing_with(vec![vec![[10.0, 10.0], [13.0, 14.0]], vec![[23.0, 14.0]]]);
        let s = compute(&d, Speeds::default());
        assert_eq!(s.path_count, 2);
        assert_eq!(s.pen_down_mm, 5.0);
        assert_eq!(s.pen_up_mm, 10.0);
        // Both moves reach their speed: each adds v / a to the time at that speed.
        let draw = 5.0 / 50.0 + 50.0 / 1000.0;
        let travel = 10.0 / 100.0 + 100.0 / 1000.0;
        assert!((s.estimated_seconds - (draw + travel + 2.0 * 0.2)).abs() < 1e-12);
    }

    #[test]
    fn a_long_move_cruises_and_a_short_one_never_does() {
        // 100 mm at 50 mm/s and 1000 mm/s²: 1.25 mm up to speed, 97.5 mm at it, 1.25 mm down.
        let long = move_seconds(100.0, 0.0, 0.0, 50.0, 1000.0);
        assert!((long - (0.05 + 97.5 / 50.0 + 0.05)).abs() < 1e-12, "{long}");
        // 1 mm peaks at sqrt(1000) mm/s, below 50, and takes 2·sqrt(d/a).
        let short = move_seconds(1.0, 0.0, 0.0, 50.0, 1000.0);
        assert!(
            (short - 2.0 * (1.0f64 / 1000.0).sqrt()).abs() < 1e-12,
            "{short}"
        );
        assert!(short > 1.0 / 50.0);
        // Without a limit on acceleration the time is the distance over the speed.
        assert_eq!(move_seconds(100.0, 0.0, 0.0, 50.0, f64::INFINITY), 2.0);
    }

    #[test]
    fn corners_slow_the_pen_and_straight_vertices_do_not() {
        let mut plan = Plan::default();
        let one = plan.seconds(&[[0.0, 0.0], [100.0, 0.0]], 50.0, 1000.0);
        let through = plan.seconds(
            &[[0.0, 0.0], [40.0, 0.0], [40.0, 0.0], [100.0, 0.0]],
            50.0,
            1000.0,
        );
        assert!((through - one).abs() < 1e-12, "{through} {one}");
        let corner = plan.seconds(&[[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]], 50.0, 1000.0);
        let stop = 2.0 * move_seconds(50.0, 0.0, 0.0, 50.0, 1000.0);
        assert!(one < corner && corner < stop, "{one} {corner} {stop}");
        // Turning back on itself stops the pen.
        let back = plan.seconds(&[[0.0, 0.0], [50.0, 0.0], [0.0, 0.0]], 50.0, 1000.0);
        assert!((back - stop).abs() < 1e-12, "{back} {stop}");
    }
}
