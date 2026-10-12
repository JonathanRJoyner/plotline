//! A uniform grid bucket index over points, for nearest-point queries that skip dead points.
//!
//! The cell method follows Bentley, Weide and Yao, "Optimal Expected-Time Algorithms for Closest
//! Point Problems", ACM Transactions on Mathematical Software 6(4), 1980.
//!
//! Squared distances are plain `dx * dx + dy * dy`, so every target computes the same bits.

/// Points bucketed into square cells over their bounding box.
pub struct GridIndex {
    points: Vec<[f64; 2]>,
    /// The bounding box's top-left corner in mm.
    origin: [f64; 2],
    cell: f64,
    cols: usize,
    rows: usize,
    /// The point indices in each cell, row-major, each list ascending.
    cells: Vec<Vec<usize>>,
}

/// The bounding box of `points` as `([min x, min y], [max x, max y])`; NaN coordinates are
/// skipped, and no points give infinities.
pub(crate) fn bounds(points: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    points.iter().fold(
        ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
        |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1])],
                [hi[0].max(p[0]), hi[1].max(p[1])],
            )
        },
    )
}

impl GridIndex {
    /// Bucket `points` into cells of side `cell_mm` over their bounding box. The grid has
    /// `floor(w / cell_mm) + 1` columns for a box `w` wide, and as many rows likewise, so the
    /// caller picks `cell_mm` to keep that small.
    pub fn new(points: &[[f64; 2]], cell_mm: f64) -> GridIndex {
        let (lo, hi) = bounds(points);
        let origin = if points.is_empty() { [0.0; 2] } else { lo };
        // A float to usize cast saturates and sends NaN to 0, so odd sizes give one cell.
        let span = |a: usize| ((hi[a] - origin[a]) / cell_mm).floor() as usize;
        let (cols, rows) = (span(0).saturating_add(1), span(1).saturating_add(1));
        let mut index = GridIndex {
            points: points.to_vec(),
            origin,
            cell: cell_mm,
            cols,
            rows,
            cells: vec![Vec::new(); cols * rows],
        };
        for (i, &p) in points.iter().enumerate() {
            let (c, r) = index.cell_of(p);
            index.cells[r * cols + c].push(i);
        }
        index
    }

    /// The column and row of the cell holding `p`, clamped into the grid.
    fn cell_of(&self, p: [f64; 2]) -> (usize, usize) {
        let at = |a: usize, n: usize| {
            (((p[a] - self.origin[a]) / self.cell).floor() as usize).min(n - 1)
        };
        (at(0, self.cols), at(1, self.rows))
    }

    /// The index of the alive point nearest `from`, comparing `(squared distance, index)` so that
    /// exact ties go to the lowest index; `None` when no alive point lies at a finite distance.
    /// `alive` has one entry per point.
    ///
    /// The search visits rings of cells around `from`'s cell, ring `r` being the cells `r` steps
    /// away in the larger of the two directions, and stops once a ring cannot hold anything as
    /// close as the best point found.
    pub fn nearest(&self, from: [f64; 2], alive: &[bool]) -> Option<usize> {
        let (cx, cy) = self.cell_of(from);
        let (cx, cy, cols, rows) = (
            cx as isize,
            cy as isize,
            self.cols as isize,
            self.rows as isize,
        );
        let last_ring = cx.max(cols - 1 - cx).max(cy).max(rows - 1 - cy);
        let mut best: Option<(f64, usize)> = None;
        for r in 0..=last_ring {
            // Every point in ring r lies more than r - 1 cells away; the extra half cell covers
            // rounding in the cell assignment.
            let gap = (r as f64 - 1.5) * self.cell;
            if let Some((d, _)) = best {
                if gap > 0.0 && gap * gap > d {
                    break;
                }
            }
            for y in (cy - r).max(0)..=(cy + r).min(rows - 1) {
                let edge_row = y == cy - r || y == cy + r;
                let step = if edge_row { 1 } else { (2 * r) as usize };
                for x in (cx - r..=cx + r).step_by(step) {
                    if x < 0 || x >= cols {
                        continue;
                    }
                    for &i in &self.cells[y as usize * self.cols + x as usize] {
                        if !alive[i] {
                            continue;
                        }
                        let (dx, dy) = (self.points[i][0] - from[0], self.points[i][1] - from[1]);
                        let d = dx * dx + dy * dy;
                        // A NaN distance would stick as the best and keep the search going.
                        if !d.is_finite() {
                            continue;
                        }
                        if best.is_none_or(|(bd, bi)| d < bd || (d == bd && i < bi)) {
                            best = Some((d, i));
                        }
                    }
                }
            }
        }
        best.map(|(_, i)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_at_no_finite_distance_are_never_returned() {
        // The NaN point sits in the query's own cell, so it is the first candidate seen.
        let pts = [[f64::NAN, 0.0], [3.0, 4.0]];
        let idx = GridIndex::new(&pts, 1.0);
        assert_eq!(idx.nearest([0.0, 0.0], &[true; 2]), Some(1));
        assert_eq!(idx.nearest([0.0, 0.0], &[true, false]), None);
    }
    #[test]
    fn nearest_alive_with_lowest_index_on_ties() {
        let pts = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0], [5.0, 5.0]];
        let idx = GridIndex::new(&pts, 5.0);
        assert_eq!(idx.nearest([9.0, 1.0], &[true; 4]), Some(1));
        assert_eq!(idx.nearest([9.0, 1.0], &[true, false, true, true]), Some(3));
        assert_eq!(idx.nearest([5.0, 0.0], &[true; 4]), Some(0)); // 0 and 1 tie at 5 mm
        assert_eq!(idx.nearest([0.0, 0.0], &[false; 4]), None);
    }
    #[test]
    fn nearest_matches_a_scan_of_every_point() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand_pcg::Pcg64::seed_from_u64(3);
        // Whole and half millimetres make exact ties common; queries also fall outside the grid.
        let pts: Vec<[f64; 2]> = (0..300)
            .map(|_| {
                [
                    rng.random_range(0..40) as f64,
                    rng.random_range(0..25) as f64,
                ]
            })
            .collect();
        let idx = GridIndex::new(&pts, 2.5);
        for _ in 0..500 {
            let alive: Vec<bool> = (0..pts.len()).map(|_| rng.random_bool(0.2)).collect();
            let mut half = |lo: i32, hi: i32| f64::from(rng.random_range(2 * lo..2 * hi)) / 2.0;
            let from = [half(-20, 60), half(-20, 45)];
            let d = |i: usize| {
                let (dx, dy) = (pts[i][0] - from[0], pts[i][1] - from[1]);
                dx * dx + dy * dy
            };
            let want = (0..pts.len())
                .filter(|&i| alive[i])
                .min_by(|&a, &b| d(a).total_cmp(&d(b)).then(a.cmp(&b)));
            assert_eq!(idx.nearest(from, &alive), want);
        }
    }
}
