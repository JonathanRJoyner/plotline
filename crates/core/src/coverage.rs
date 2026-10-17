//! The coverage raster: how many separate passes of the pen cover each spot of the page.
//!
//! Every path is laid at its true pen width into a grid of pass counts. A cell counts a pass when
//! its centre lies within half the pen width of the path, which is point sampling of the stroke's
//! outline, a capsule around each segment and a disc around a lone vertex. A per-cell index of the
//! last path that counted it makes a path that crosses itself count once per cell. That index is
//! an implementation choice, not a published method. A [jittered](Coverage::jittered) grid samples
//! each cell at a point of its own instead of its centre.
//!
//! Each row of a capsule is one interval, because a capsule is convex, so a segment costs one
//! interval per row it spans and the cells inside it. A jittered row is a band one cell high: the
//! cells wholly inside the capsule across the band count without a test, and only those its edge
//! crosses test their sample. All arithmetic is plain `f64` with `sqrt`, `floor` and `ceil`
//! (exact in IEEE 754) or wrapping `u64`, and paths are stamped in order, so the counts are the
//! same on every target.

use crate::types::Paths;

/// The grid stays under this many cells, the ink stage's bound for a browser tab.
pub const MAX_CELLS: f64 = 12e6;
/// The smallest cell, in mm.
const MIN_CELL_MM: f64 = 0.1;

/// The SplitMix64 output function of G. L. Steele Jr., D. Lea and C. H. Flood, "Fast splittable
/// pseudorandom number generators", *OOPSLA* 2014, pp. 453-472: 64 well-mixed bits from `x`.
fn mix(x: u64) -> u64 {
    let z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

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
    /// Whether each cell is sampled at a point of its own rather than its centre.
    jittered: bool,
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

    /// Whether `p` lies in the capsule.
    fn holds(&self, p: [f64; 2]) -> bool {
        let Capsule { a, r, len, u, .. } = *self;
        let (dx, dy) = (p[0] - a[0], p[1] - a[1]);
        let t = (dx * u[0] + dy * u[1]).clamp(0.0, len);
        let (ex, ey) = (dx - t * u[0], dy - t * u[1]);
        ex * ex + ey * ey <= r * r
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
            jittered: false,
        }
    }

    /// [`Coverage::new`] sampling each cell at a random point within it, which is jittered
    /// sampling (R. L. Cook, "Stochastic sampling in computer graphics", *ACM Trans. Graphics*
    /// 5(1), 1986, pp. 51-72). A line at 0, 45 or 90 degrees no longer keeps one phase against the
    /// samples along its length, so its width stops snapping to whole cells, and the error that
    /// was a bias shared by every line of a hatch becomes noise that the eye filter averages out.
    /// Each cell's point is a hash of its column and row ([`mix`]) rather than a draw from a
    /// generator, so it needs no other cell's and is the same on every target. Unlike offsets that
    /// step by fixed irrationals, a hash has no direction of its own for a line to lock to.
    pub fn jittered(rect_mm: [f64; 4], cell_mm: f64) -> Coverage {
        Coverage {
            jittered: true,
            ..Coverage::new(rect_mm, cell_mm)
        }
    }

    /// The jittered sample of the cell in column `i` and row `j`, in mm: the high and low 32 bits
    /// of its hash place it across and down the cell.
    fn sample(&self, i: usize, j: usize) -> [f64; 2] {
        let bits = mix((j as u64) << 32 | i as u64);
        let at = |cell: usize, share: u64, axis: usize| {
            let share = share as f64 / 4_294_967_296.0;
            self.origin_mm[axis] + (cell as f64 + share) * self.cell_mm
        };
        [at(i, bits >> 32, 0), at(j, bits & 0xffff_ffff, 1)]
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
        // How far a sample may lie from its cell's centre along each axis.
        let h = if self.jittered {
            self.cell_mm / 2.0
        } else {
            0.0
        };
        for path in paths.iter() {
            let id = self.next;
            self.next += 1;
            // A lone vertex is a segment of no length, which leaves its disc.
            let segments = path.windows(2).map(|s| (s[0], s[1]));
            let lone = (path.len() == 1).then(|| (path[0], path[0]));
            for (a, b) in segments.chain(lone) {
                let (lo, hi) = (a[1].min(b[1]) - r - h, a[1].max(b[1]) + r + h);
                let Some((top, bottom)) = self.cells(1, lo, hi) else {
                    continue;
                };
                let capsule = Capsule::new(a, b, r);
                let union = |(l0, h0): (f64, f64), (l1, h1): (f64, f64)| (l0.min(l1), h0.max(h1));
                for row in top..=bottom {
                    let y = self.origin_mm[1] + (row as f64 + 0.5) * self.cell_mm;
                    let (y0, y1) = (y - h, y + h);
                    // Across the band from y0 to y1 the capsule's left edge is convex and its
                    // right edge concave, so it reaches furthest at the band's top or bottom or
                    // at a disc's centre inside the band, and it holds the band all the way
                    // across where the intervals at the top and bottom meet.
                    let ends = [y0, y1].map(|v| capsule.span(v));
                    let tips = [a, b].into_iter().filter(|p| y0 < p[1] && p[1] < y1);
                    let far = ends.into_iter().flatten();
                    let Some((far_l, far_r)) =
                        far.chain(tips.map(|p| (p[0] - r, p[0] + r))).reduce(union)
                    else {
                        continue;
                    };
                    let inner = match ends {
                        [Some((l0, r0)), Some((l1, r1))] => (l0.max(l1), r0.min(r1)),
                        _ => (f64::INFINITY, f64::NEG_INFINITY),
                    };
                    let Some((left, right)) = self.cells(0, far_l - h, far_r + h) else {
                        continue;
                    };
                    for col in left..=right {
                        let i = row * self.width + col;
                        if self.last[i] == id {
                            continue;
                        }
                        // A centred row's cells are those whose centres lie in its interval.
                        let x = self.origin_mm[0] + (col as f64 + 0.5) * self.cell_mm;
                        if h == 0.0
                            || (inner.0 <= x - h && x + h <= inner.1)
                            || capsule.holds(self.sample(col, row))
                        {
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
        for grid in [Coverage::new, Coverage::jittered] {
            let mut c = grid([0.0, 0.0, 10.0, 10.0], 0.1);
            c.stamp(
                &paths(&[&[[1.0, 5.0], [9.0, 5.0]], &[[5.0, 1.0], [5.0, 9.0]]]),
                0.3,
            );
            assert_eq!(c.count[50 * 100 + 50], 2);
            let mut d = grid([0.0, 0.0, 10.0, 10.0], 0.1);
            let loop_back = [[1.0, 5.0], [9.0, 5.0], [9.0, 6.0], [5.0, 6.0], [5.0, 1.0]];
            d.stamp(&paths(&[&loop_back]), 0.3);
            assert_eq!(*d.count.iter().max().unwrap(), 1);
        }
    }

    #[test]
    fn a_jittered_line_covers_its_width_wherever_it_falls() {
        // A 0.25 mm pen on cells of 0.1 mm, along a 16 mm line at 0, 45 and 90 degrees shifted
        // across a cell in twentieths. Centre sampling snaps the band to whole rows of cells and
        // covers 0.85 to 1.13 of the area at 45 degrees; jittered, it is 0.98 to 1.04.
        let (pen, length) = (0.25, 16.0);
        let area = length * pen + std::f64::consts::PI * pen * pen / 4.0;
        let spread = |grid: fn([f64; 4], f64) -> Coverage, angle: f64| {
            let (dx, dy) = (
                crate::math::cos(angle.to_radians()),
                crate::math::sin(angle.to_radians()),
            );
            let shares: Vec<f64> = (0..20)
                .map(|k| {
                    let o = 0.1 * k as f64 / 20.0;
                    let a = [2.0 + o, 2.0 + 0.7 * o];
                    let mut c = grid([0.0, 0.0, 20.0, 20.0], 0.1);
                    c.stamp(
                        &paths(&[&[a, [a[0] + length * dx, a[1] + length * dy]]]),
                        pen,
                    );
                    inked(&c) as f64 * 0.01 / area
                })
                .collect();
            let lo = shares.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = shares.iter().copied().fold(0.0, f64::max);
            (lo, hi)
        };
        for angle in [0.0, 45.0, 90.0] {
            let (lo, hi) = spread(Coverage::jittered, angle);
            assert!(0.95 < lo && hi < 1.05, "{angle}: {lo} to {hi}");
        }
        let (lo, hi) = spread(Coverage::new, 45.0);
        assert!(hi - lo > 0.2, "{lo} to {hi}");
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
        for grid in [Coverage::new, Coverage::jittered] {
            let run = || {
                let mut c = grid([0.0, 0.0, 10.0, 10.0], 0.1);
                c.stamp(&p, 0.45);
                c.count
            };
            assert_eq!(run(), run());
        }
    }
}
