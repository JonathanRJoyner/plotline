//! The coverage raster: how many separate passes of the pen cover each spot of the page.
//!
//! Every path is laid at its true pen width into a grid of pass counts. A cell counts a pass when
//! its centre lies within half the pen width of the path, which is point sampling of the stroke's
//! outline, a capsule around each segment and a disc around a lone vertex. A per-cell index of the
//! last path that counted it makes a path that crosses itself count once per cell. That index is
//! an implementation choice, not a published method.
//!
//! Each row of a capsule is one interval, because a capsule is convex, so a segment costs one
//! interval per row it spans and the cells inside it. All arithmetic is plain `f64` with `sqrt`,
//! `floor` and `ceil` (exact in IEEE 754) and paths are stamped in order, so the counts are the
//! same on every target.

use crate::types::Paths;

/// The grid stays under this many cells, the ink stage's bound for a browser tab.
pub const MAX_CELLS: f64 = 12e6;
/// The smallest cell, in mm.
const MIN_CELL_MM: f64 = 0.1;

/// Pass counts over a page rectangle, row-major, in square cells.
pub struct Coverage {
    /// The page position of the grid's top-left corner in mm.
    pub origin_mm: [f64; 2],
    /// The side of a cell in mm.
    pub cell_mm: f64,
    /// Columns and rows.
    pub width: usize,
    pub height: usize,
    /// The passes over each cell, saturating at 255.
    pub count: Vec<u8>,
    /// The id of the last path that counted each cell; 0 for none.
    last: Vec<u32>,
    /// The id the next stamped path gets. Ids only grow, so clearing `count` between stamps keeps
    /// every path counted once.
    next: u32,
}

/// The cell size for a pen `pen_mm` wide over `rect_mm` (`[x, y, w, h]`): a third of the pen, at
/// least 0.1 mm, and grown until the grid holds at most [`MAX_CELLS`] cells.
pub fn cell_size(pen_mm: f64, rect_mm: [f64; 4]) -> f64 {
    let [_, _, w, h] = rect_mm;
    let cells = |c: f64| (w / c).ceil() * (h / c).ceil();
    let mut c = (pen_mm / 3.0)
        .max(MIN_CELL_MM)
        .max((w * h / MAX_CELLS).sqrt());
    while cells(c) > MAX_CELLS {
        c *= 1.01;
    }
    c
}

/// The `x` interval where `lo <= a x + b <= hi`: everything when `a` is 0 and `b` is inside,
/// nothing when it is outside.
fn solve(a: f64, b: f64, lo: f64, hi: f64) -> Option<(f64, f64)> {
    if a == 0.0 {
        return (lo <= b && b <= hi).then_some((f64::NEG_INFINITY, f64::INFINITY));
    }
    let (x0, x1) = ((lo - b) / a, (hi - b) / a);
    Some((x0.min(x1), x0.max(x1)))
}

/// The set of points within `r` of segment `a`-`b`, with the segment's length and direction.
struct Capsule {
    a: [f64; 2],
    b: [f64; 2],
    r: f64,
    len: f64,
    u: [f64; 2],
}

impl Capsule {
    fn new(a: [f64; 2], b: [f64; 2], r: f64) -> Capsule {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let u = if len > 0.0 {
            [dx / len, dy / len]
        } else {
            [0.0; 2]
        };
        Capsule { a, b, r, len, u }
    }

    /// The `x` interval of the capsule on the line at `y`.
    fn span(&self, y: f64) -> Option<(f64, f64)> {
        let Capsule { a, b, r, len, u } = *self;
        let disc = |p: [f64; 2]| {
            let dy = y - p[1];
            let k = r * r - dy * dy;
            (k >= 0.0).then(|| (p[0] - k.sqrt(), p[0] + k.sqrt()))
        };
        // The swept rectangle: along the segment within [0, len], across it within [-r, r].
        let band = (len > 0.0)
            .then(|| {
                let along = solve(u[0], (y - a[1]) * u[1] - a[0] * u[0], 0.0, len)?;
                let across = solve(-u[1], (y - a[1]) * u[0] + a[0] * u[1], -r, r)?;
                let (l, h) = (along.0.max(across.0), along.1.min(across.1));
                (l <= h).then_some((l, h))
            })
            .flatten();
        // The pieces make up a convex capsule, so their union on one line is one interval.
        [disc(a), disc(b), band]
            .into_iter()
            .flatten()
            .reduce(|(l0, h0), (l1, h1)| (l0.min(l1), h0.max(h1)))
    }
}

impl Coverage {
    /// An empty grid over `rect_mm` (`[x, y, w, h]`) of `ceil(w / cell_mm)` by `ceil(h /
    /// cell_mm)` cells.
    pub fn new(rect_mm: [f64; 4], cell_mm: f64) -> Coverage {
        let [x, y, w, h] = rect_mm;
        let (width, height) = ((w / cell_mm).ceil() as usize, (h / cell_mm).ceil() as usize);
        Coverage {
            origin_mm: [x, y],
            cell_mm,
            width,
            height,
            count: vec![0; width * height],
            last: vec![0; width * height],
            next: 1,
        }
    }

    /// The first and last cell index along one axis whose centre lies in `[lo, hi]` mm, clipped to
    /// the grid; `None` when there is none.
    fn cells(&self, axis: usize, lo: f64, hi: f64) -> Option<(usize, usize)> {
        let n = [self.width, self.height][axis] as f64;
        let at = |v: f64| (v - self.origin_mm[axis]) / self.cell_mm - 0.5;
        let (first, last) = (at(lo).ceil().max(0.0), at(hi).floor().min(n - 1.0));
        (first <= last).then_some((first as usize, last as usize))
    }

    /// Lay every path of `paths` at a pen `pen_mm` wide, adding one pass to each cell a path
    /// covers, however often the path covers it.
    pub fn stamp(&mut self, paths: &Paths, pen_mm: f64) {
        let r = pen_mm / 2.0;
        for path in paths.iter() {
            let id = self.next;
            self.next += 1;
            // A lone vertex is a segment of no length, which leaves its disc.
            let segments = path.windows(2).map(|s| (s[0], s[1]));
            let lone = (path.len() == 1).then(|| (path[0], path[0]));
            for (a, b) in segments.chain(lone) {
                let Some((top, bottom)) = self.cells(1, a[1].min(b[1]) - r, a[1].max(b[1]) + r)
                else {
                    continue;
                };
                let capsule = Capsule::new(a, b, r);
                for row in top..=bottom {
                    let y = self.origin_mm[1] + (row as f64 + 0.5) * self.cell_mm;
                    let Some((l, h)) = capsule.span(y) else {
                        continue;
                    };
                    let Some((left, right)) = self.cells(0, l, h) else {
                        continue;
                    };
                    for i in row * self.width + left..=row * self.width + right {
                        if self.last[i] != id {
                            self.last[i] = id;
                            self.count[i] = self.count[i].saturating_add(1);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&[[f64; 2]]]) -> Paths {
        let mut p = Paths::new();
        for path in list {
            p.push(path);
        }
        p
    }

    fn inked(c: &Coverage) -> usize {
        c.count.iter().filter(|&&n| n > 0).count()
    }

    #[test]
    fn a_line_covers_a_band_one_pen_wide() {
        // Cells 0.1 mm; a 0.36 mm pen along y = 5 covers the rows centred at 4.85, 4.95, 5.05
        // and 5.15, and on the row at 4.95 the columns centred from 1.85 to 9.15, the round caps'
        // ends included.
        let mut c = Coverage::new([0.0, 0.0, 10.0, 10.0], 0.1);
        c.stamp(&paths(&[&[[2.0, 5.0], [9.0, 5.0]]]), 0.36);
        let row = |y: usize| (0..100).filter(|&x| c.count[y * 100 + x] > 0).count();
        assert_eq!((row(47), row(48), row(51), row(52)), (0, 72, 72, 0));
        assert_eq!(row(49), 74);
        assert!(c.count.iter().all(|&n| n <= 1));
    }

    #[test]
    fn crossing_paths_count_twice_and_a_self_crossing_once() {
        let mut c = Coverage::new([0.0, 0.0, 10.0, 10.0], 0.1);
        c.stamp(
            &paths(&[&[[1.0, 5.0], [9.0, 5.0]], &[[5.0, 1.0], [5.0, 9.0]]]),
            0.3,
        );
        assert_eq!(c.count[50 * 100 + 50], 2);
        let mut d = Coverage::new([0.0, 0.0, 10.0, 10.0], 0.1);
        let loop_back = [[1.0, 5.0], [9.0, 5.0], [9.0, 6.0], [5.0, 6.0], [5.0, 1.0]];
        d.stamp(&paths(&[&loop_back]), 0.3);
        assert_eq!(*d.count.iter().max().unwrap(), 1);
    }

    #[test]
    fn a_dot_stamps_a_disc() {
        let mut c = Coverage::new([0.0, 0.0, 2.0, 2.0], 0.1);
        c.stamp(&paths(&[&[[1.0, 1.0]]]), 0.6);
        // Centres within 0.3 mm of (1, 1): the area is about 0.28 mm2, 28 cells of 0.01 mm2.
        let n = inked(&c);
        assert!((24..=32).contains(&n), "{n}");
    }

    #[test]
    fn marks_off_the_grid_are_clipped() {
        let mut c = Coverage::new([0.0, 0.0, 1.0, 1.0], 0.1);
        // Rows centred at 0.45, 0.55 and 0.65 lie within 0.15 mm of y = 0.52.
        c.stamp(&paths(&[&[[-5.0, 0.52], [5.0, 0.52]], &[[3.0, 3.0]]]), 0.3);
        assert_eq!(inked(&c), 30);
    }

    #[test]
    fn cell_size_keeps_the_bound() {
        assert_eq!(cell_size(0.3, [0.0, 0.0, 210.0, 297.0]), 0.1);
        assert!((cell_size(0.6, [0.0, 0.0, 210.0, 297.0]) - 0.2).abs() < 1e-12);
        let c = cell_size(0.3, [0.0, 0.0, 1000.0, 1000.0]);
        assert!(
            (1000.0 / c).ceil() * (1000.0 / c).ceil() <= MAX_CELLS,
            "{c}"
        );
        assert!(c < 0.3, "{c}");
    }

    #[test]
    fn the_same_paths_give_the_same_counts() {
        let p = paths(&[&[[1.0, 1.0], [8.3, 7.1], [2.2, 9.0]], &[[5.0, 5.0]]]);
        let run = || {
            let mut c = Coverage::new([0.0, 0.0, 10.0, 10.0], 0.1);
            c.stamp(&p, 0.45);
            c.count
        };
        assert_eq!(run(), run());
    }
}
