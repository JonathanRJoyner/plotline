//! Families of parallel lines over a rectangle, shared by the line styles.
//!
//! Angles are in degrees, counter-clockwise from horizontal as seen on the page. The page's y axis
//! points down, so a line at angle `a` runs along `(cos a, -sin a)`. Lines are clipped to the
//! rectangle with the slab method: Y.-D. Liang and B. A. Barsky, "A new concept and method for
//! line clipping", *ACM Transactions on Graphics* 3(1), 1984, pp. 1-22.

use std::f64::consts::PI;
use std::sync::Arc;

use crate::math;
use crate::stage::{Artifact, Frame, StageError, StageRun, Step};
use crate::types::Paths;

/// The least ink below which the line styles lift the pen when the drawing is split between pens.
/// They draw a straight line over no ink, so without it a pen whose colour the image lacks would
/// cover the page; the margin also hides the small residue the colour table's interpolation leaves
/// where a pen has no share. See [`lane_floor`] for the rule above it.
pub const LANE_MIN_INK: f32 = 0.02;

/// The ink below which a lane lifts its pen when the style's lines over white, `pen_mm` wide, lie
/// `spacing_mm` apart. Such a bare line covers `pen_mm / spacing_mm` of the paper however little
/// ink is below it, so where the ink asks for less than half of that, bare paper is nearer to it
/// than the line: lifting picks the nearer of the two levels the pen can give there, as a
/// two-level quantiser picks the level nearest its input. It is never below [`LANE_MIN_INK`].
pub fn lane_floor(pen_mm: f64, spacing_mm: f64) -> f32 {
    let bare = (pen_mm / spacing_mm).min(1.0);
    ((bare / 2.0) as f32).max(LANE_MIN_INK)
}

/// Points from `a` to `b` no more than `step` apart, both ends included.
pub fn sample_line(a: [f64; 2], b: [f64; 2], step: f64) -> impl Iterator<Item = [f64; 2]> {
    let len = math::hypot(b[0] - a[0], b[1] - a[1]);
    let n = ((len / step).ceil() as usize).max(1);
    (0..=n).map(move |j| {
        let f = j as f64 / n as f64;
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f]
    })
}

/// Whether `p` lies in `rect` (`[x, y, w, h]` in mm).
pub fn inside(rect: [f64; 4], p: [f64; 2]) -> bool {
    let [x, y, w, h] = rect;
    x <= p[0] && p[0] <= x + w && y <= p[1] && p[1] <= y + h
}

/// Adds the paths of work item `i`: finished paths to the first list, and the vertices of a path
/// that carries on into the next item to the second, which the run ends when nothing follows.
pub type MakePaths = Box<dyn FnMut(usize, &mut Paths, &mut Vec<[f64; 2]>) + Send>;

/// A run that makes paths a chunk of work items at a time, reporting about 30 frames that hold
/// the paths so far, labelled with the items done and `unit`. An open path of one vertex is
/// dropped.
pub struct ChunkRun {
    make: MakePaths,
    total: usize,
    per_frame: usize,
    next: usize,
    unit: &'static str,
    framed: bool,
    paths: Paths,
    open: Vec<[f64; 2]>,
}

impl ChunkRun {
    /// A run over `total` items of `unit`, each made by `make`.
    pub fn new(total: usize, unit: &'static str, make: MakePaths) -> ChunkRun {
        ChunkRun {
            make,
            total,
            per_frame: total.div_ceil(30).max(1),
            next: 0,
            unit,
            framed: false,
            paths: Paths::new(),
            open: Vec::new(),
        }
    }
}

impl StageRun for ChunkRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if self.framed && self.next >= self.total {
            if self.open.len() >= 2 {
                self.paths.push(&self.open);
            }
            self.open.clear();
            let paths = std::mem::take(&mut self.paths);
            return Ok(Step::Done(Artifact::Paths(Arc::new(paths))));
        }
        let end = (self.next + self.per_frame).min(self.total);
        for i in self.next..end {
            (self.make)(i, &mut self.paths, &mut self.open);
        }
        self.next = end;
        self.framed = true;
        let mut shown = self.paths.clone();
        if self.open.len() >= 2 {
            shown.push(&self.open);
        }
        let progress = if self.total == 0 {
            1.0
        } else {
            end as f32 / self.total as f32
        };
        Ok(Step::Frame(Frame {
            stage: String::new(),
            progress,
            label: format!("{end}/{} {}", self.total, self.unit),
            points: None,
            paths: Some(shown),
            pen: 0,
        }))
    }
}

/// The unit direction of lines at `angle_deg`, and the unit normal the family is spaced along.
pub fn direction(angle_deg: f64) -> ([f64; 2], [f64; 2]) {
    let a = angle_deg * PI / 180.0;
    let (s, c) = (math::sin(a), math::cos(a));
    ([c, -s], [s, c])
}

/// `p` moved into `rect` (`[x, y, w, h]` in mm) along each axis.
pub fn clamp_to(rect: [f64; 4], p: [f64; 2]) -> [f64; 2] {
    let [x, y, w, h] = rect;
    [p[0].clamp(x, x + w), p[1].clamp(y, y + h)]
}

/// The part of the line `o * normal + t * dir` inside `rect`, as a `t` interval, or `None`.
pub(super) fn clip(rect: [f64; 4], o: f64, dir: [f64; 2], normal: [f64; 2]) -> Option<(f64, f64)> {
    let [x, y, w, h] = rect;
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (lo, hi, base, d) in [
        (x, x + w, o * normal[0], dir[0]),
        (y, y + h, o * normal[1], dir[1]),
    ] {
        if d.abs() < 1e-12 {
            if base < lo || base > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - base) / d, (hi - base) / d);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    (t1 - t0 > 1e-9).then_some((t0, t1))
}

/// Lines `spacing` mm apart at `angle_deg` across `rect` (`[x, y, w, h]` in mm), each clipped to
/// it and given as its two end points, in order across the rect. The first line sits half a
/// spacing in from the rect's edge, so a rect `n` spacings across holds `n` lines.
pub fn scanlines(rect: [f64; 4], angle_deg: f64, spacing: f64) -> Vec<([f64; 2], [f64; 2])> {
    let (dir, normal) = direction(angle_deg);
    let [x, y, w, h] = rect;
    let offsets = [[x, y], [x + w, y], [x, y + h], [x + w, y + h]]
        .map(|c| c[0] * normal[0] + c[1] * normal[1]);
    let lo = offsets.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = offsets.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut lines = Vec::new();
    let mut k = 0usize;
    loop {
        let o = lo + spacing * (k as f64 + 0.5);
        if o >= hi {
            return lines;
        }
        k += 1;
        if let Some((t0, t1)) = clip(rect, o, dir, normal) {
            let at = |t: f64| {
                clamp_to(
                    rect,
                    [o * normal[0] + t * dir[0], o * normal[1] + t * dir[1]],
                )
            };
            lines.push((at(t0), at(t1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    #[test]
    fn a_lane_lifts_below_half_a_bare_line() {
        // 0.3 mm lines 2 mm apart cover 15 %, so the pen lifts below 7.5 %.
        assert_eq!(lane_floor(0.3, 2.0), 0.075);
        // Never below the least ink that lifts, and lines wider than their spacing cover all.
        assert_eq!(lane_floor(0.03, 10.0), LANE_MIN_INK);
        assert_eq!(lane_floor(3.0, 2.0), 0.5);
    }

    #[test]
    fn horizontal_lines_start_half_a_spacing_in() {
        let lines = scanlines([0.0, 0.0, 10.0, 4.0], 0.0, 1.0);
        assert_eq!(lines.len(), 4);
        for (i, (a, b)) in lines.iter().enumerate() {
            let y = 0.5 + i as f64;
            assert!(close(*a, [0.0, y]) && close(*b, [10.0, y]), "{a:?} {b:?}");
        }
    }

    #[test]
    fn ninety_degrees_is_vertical() {
        let lines = scanlines([2.0, 1.0, 4.0, 10.0], 90.0, 1.0);
        assert_eq!(lines.len(), 4);
        for (a, b) in &lines {
            assert!((a[0] - b[0]).abs() < 1e-9);
            assert!((b[1] - a[1]).abs() > 10.0 - 1e-9);
        }
    }

    #[test]
    fn angled_lines_are_clipped_to_the_rect() {
        let rect = [1.0, 2.0, 30.0, 20.0];
        let lines = scanlines(rect, 45.0, 0.7);
        assert!(lines.len() > 20);
        for (a, b) in &lines {
            for p in [a, b] {
                assert!(
                    rect[0] - 1e-9 <= p[0] && p[0] <= rect[0] + rect[2] + 1e-9,
                    "{p:?}"
                );
                assert!(
                    rect[1] - 1e-9 <= p[1] && p[1] <= rect[1] + rect[3] + 1e-9,
                    "{p:?}"
                );
            }
            // 45 degrees counter-clockwise on the page: x grows as y shrinks.
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            assert!(dx > 0.0 && (dx + dy).abs() < 1e-9, "{a:?} {b:?}");
        }
    }

    #[test]
    fn clamp_keeps_a_point_inside() {
        assert_eq!(clamp_to([0.0, 0.0, 10.0, 4.0], [-1.0, 5.0]), [0.0, 4.0]);
        assert_eq!(clamp_to([0.0, 0.0, 10.0, 4.0], [3.0, 2.0]), [3.0, 2.0]);
    }
}
