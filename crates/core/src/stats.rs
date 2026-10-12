//! Drawing statistics and a plot time estimate.
//!
//! [`compute`] counts the paths of a finished drawing, measures the distance the pen draws and the
//! distance it travels lifted, and turns them into seconds at the speeds of a [`Speeds`].

use serde::{Deserialize, Serialize};

use crate::types::Drawing;

/// How fast a plotter moves, as a front end lets the user set it.
///
/// `draw_mm_s` is the speed with the pen down and `travel_mm_s` the speed with it up, both in mm
/// per second and above zero. `pen_lift_s` is the time to lift the pen and set it down again,
/// which every path costs once.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Speeds {
    /// The speed with the pen down, in mm per second.
    pub draw_mm_s: f64,
    /// The speed with the pen up, in mm per second.
    pub travel_mm_s: f64,
    /// The time to lift the pen and set it down again, in seconds.
    pub pen_lift_s: f64,
}

impl Default for Speeds {
    fn default() -> Speeds {
        Speeds {
            draw_mm_s: 50.0,
            travel_mm_s: 100.0,
            pen_lift_s: 0.2,
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
/// moves between layers count as travel. The time in seconds is the pen-down distance over
/// `draw_mm_s`, plus the pen-up distance over `travel_mm_s`, plus `pen_lift_s` for each path.
/// Swapping pens between layers is not timed.
pub fn compute(d: &Drawing, speeds: Speeds) -> Stats {
    let [x, y, _, _] = d.canvas.drawing_rect();
    let mut pen = [x, y];
    let (mut path_count, mut down, mut up) = (0, 0.0, 0.0);
    for layer in &d.layers {
        path_count += layer.paths.len();
        down += layer.paths.total_length();
        up += layer.paths.travel_length(pen);
        // The next layer's first move starts where this layer's last drawn vertex is.
        if let Some(end) = layer.paths.iter().filter_map(|p| p.last()).last() {
            pen = *end;
        }
    }
    Stats {
        path_count,
        pen_down_mm: down,
        pen_up_mm: up,
        estimated_seconds: down / speeds.draw_mm_s
            + up / speeds.travel_mm_s
            + path_count as f64 * speeds.pen_lift_s,
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
        assert!((s.estimated_seconds - (5.0 / 50.0 + 10.0 / 100.0 + 2.0 * 0.2)).abs() < 1e-12);
    }
}
