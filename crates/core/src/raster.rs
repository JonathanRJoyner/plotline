//! Binary-raster geometry for styles that draw an image's own shapes: distance to the nearest
//! background cell, iso-lines of a field, thinning to a skeleton and walking it into polylines,
//! plus smoothing and simplification of the polylines that come out.
//!
//! Grids are row-major, `w * h` cells, and a cell's centre sits at its integer coordinates.
//! Callers pad their grids with a border of background cells, so contours close and thinning
//! never reads past an edge.
//!
//! - Distance transform: P. F. Felzenszwalb and D. P. Huttenlocher, "Distance Transforms of
//!   Sampled Functions", *Theory of Computing* 8, 2012, pp. 415-428 (the lower envelope of
//!   parabolas, one dimension at a time).
//! - Iso-lines: the two-dimensional case of W. E. Lorensen and H. E. Cline, "Marching Cubes: A
//!   High Resolution 3D Surface Construction Algorithm", *SIGGRAPH* 1987, with saddles split by
//!   the cell's mean.
//! - Thinning: T. Y. Zhang and C. Y. Suen, "A Fast Parallel Algorithm for Thinning Digital
//!   Patterns", *Communications of the ACM* 27(3), 1984, pp. 236-239.
//! - Simplification: D. H. Douglas and T. K. Peucker, "Algorithms for the reduction of the number
//!   of points required to represent a digitized line or its caricature", *The Canadian
//!   Cartographer* 10(2), 1973, pp. 112-122.

use std::collections::HashMap;

use crate::math;

/// Stands in for an infinite squared distance; finite so the envelope arithmetic stays finite.
const FAR: f64 = 1e20;

/// The squared distance transform of `f` along one line, written to `out`: `out[q]` is the least
/// `(q - p)^2 + f[p]` over every `p`. `v` and `z` are scratch of at least `f.len()` and
/// `f.len() + 1` entries.
fn envelope(f: &[f64], out: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let cross = |q: usize, p: usize| {
        let (qf, pf) = (q as f64, p as f64);
        ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * qf - 2.0 * pf)
    };
    let mut k = 0;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        let mut s = cross(q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = cross(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, o) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let d = q as f64 - v[k] as f64;
        *o = d * d + f[v[k]];
    }
}

/// For every cell of `mask`, the Euclidean distance from its centre to the centre of the nearest
/// cell that is not set; 0 for cells that are not set. A grid with no unset cell gives distances
/// past any useful length.
pub fn distance(mask: &[bool], w: usize, h: usize) -> Vec<f32> {
    let mut d: Vec<f64> = mask.iter().map(|&m| if m { FAR } else { 0.0 }).collect();
    let n = w.max(h);
    let (mut f, mut out, mut v, mut z) = (
        vec![0.0; n],
        vec![0.0; n],
        vec![0usize; n],
        vec![0.0; n + 1],
    );
    for x in 0..w {
        for y in 0..h {
            f[y] = d[y * w + x];
        }
        envelope(&f[..h], &mut out[..h], &mut v, &mut z);
        for y in 0..h {
            d[y * w + x] = out[y];
        }
    }
    for y in 0..h {
        let row = &mut d[y * w..(y + 1) * w];
        f[..w].copy_from_slice(row);
        envelope(&f[..w], &mut out[..w], &mut v, &mut z);
        row.copy_from_slice(&out[..w]);
    }
    d.into_iter().map(|v| v.sqrt() as f32).collect()
}

/// The closed iso-lines of `field` at every level `first + j * step` (`j` from 0) that is at most
/// `last`, as polylines in cell coordinates whose last vertex repeats the first. A cell counts as
/// inside a level when its value reaches it. `field` must be below `first` along the grid's
/// border, so every line closes. Lines come in the order their first cell is met, row by row.
pub fn iso_lines(
    field: &[f32],
    w: usize,
    h: usize,
    first: f64,
    step: f64,
    last: f64,
) -> Vec<Vec<[f64; 2]>> {
    let level = |j: usize| first + j as f64 * step;
    let at = |x: usize, y: usize| f64::from(field[y * w + x]);
    // Edge ids: 2 * cell for the edge from a cell to its right neighbour, plus 1 for the edge to
    // the one below. A segment joins two edges of one cell at one level. Keys are `u64`: on wasm32
    // a level index times the edge count passes `usize::MAX` on a large map with many levels.
    let edges = 2 * w as u64 * h as u64;
    let key = |j: usize, e: usize| j as u64 * edges + e as u64;
    let mut segments: Vec<(usize, usize, usize)> = Vec::new();
    for y in 0..h.saturating_sub(1) {
        for x in 0..w.saturating_sub(1) {
            let c = [at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)];
            let lo = c.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if hi < first || lo > last {
                continue;
            }
            // One level either side of the estimate, so rounding never skips a crossing.
            let j0 = (((lo - first) / step).floor().max(0.0) as usize).saturating_sub(1);
            let j1 = ((hi.min(last) - first) / step).floor().max(0.0) as usize + 1;
            let (top, right) = (2 * (y * w + x), 2 * (y * w + x + 1) + 1);
            let (bottom, left) = (2 * ((y + 1) * w + x), 2 * (y * w + x) + 1);
            for j in j0..=j1 {
                let l = level(j);
                if l > last {
                    break;
                }
                let case = c
                    .iter()
                    .enumerate()
                    .fold(0, |k, (i, &v)| k | (usize::from(v >= l) << i));
                let centre_in = c.iter().sum::<f64>() / 4.0 >= l;
                let pairs: &[(usize, usize)] = match case {
                    1 | 14 => &[(left, top)],
                    2 | 13 => &[(top, right)],
                    3 | 12 => &[(left, right)],
                    4 | 11 => &[(right, bottom)],
                    6 | 9 => &[(top, bottom)],
                    7 | 8 => &[(left, bottom)],
                    5 if centre_in => &[(top, right), (bottom, left)],
                    5 => &[(left, top), (right, bottom)],
                    10 if centre_in => &[(left, top), (right, bottom)],
                    10 => &[(top, right), (bottom, left)],
                    _ => &[],
                };
                segments.extend(pairs.iter().map(|&(a, b)| (j, a, b)));
            }
        }
    }
    // The point where level j crosses an edge, by linear interpolation along it.
    let point = |j: usize, e: usize| -> [f64; 2] {
        let (cell, down) = (e / 2, e % 2 == 1);
        let (x, y) = (cell % w, cell / w);
        let (x2, y2) = if down { (x, y + 1) } else { (x + 1, y) };
        let (a, b) = (at(x, y), at(x2, y2));
        let t = if (b - a).abs() > 1e-12 {
            (level(j) - a) / (b - a)
        } else {
            0.5
        };
        [
            x as f64 + t * (x2 as f64 - x as f64),
            y as f64 + t * (y2 as f64 - y as f64),
        ]
    };
    // Every crossed edge belongs to exactly two segments of its level, one from each cell.
    let mut by_edge: HashMap<u64, [usize; 2]> = HashMap::with_capacity(segments.len());
    for (i, &(j, a, b)) in segments.iter().enumerate() {
        for e in [a, b] {
            by_edge
                .entry(key(j, e))
                .and_modify(|s| s[1] = i)
                .or_insert([i, usize::MAX]);
        }
    }
    let mut used = vec![false; segments.len()];
    let mut lines = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        let (j, a, b) = segments[start];
        used[start] = true;
        let mut line = vec![point(j, a), point(j, b)];
        let (mut seg, mut edge) = (start, b);
        while edge != a {
            let [s0, s1] = by_edge[&key(j, edge)];
            let next = if s0 == seg { s1 } else { s0 };
            if next == usize::MAX || used[next] {
                break; // only reachable if the border rule above was broken
            }
            used[next] = true;
            let (_, na, nb) = segments[next];
            edge = if na == edge { nb } else { na };
            seg = next;
            line.push(point(j, edge));
        }
        lines.push(line);
    }
    lines
}

/// The 8 neighbours of a cell as (dx, dy), clockwise from north, in the order Zhang and Suen
/// number them P2 to P9.
const RING: [(isize, isize); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

/// Thin `mask` in place to a skeleton one cell wide that keeps its connectivity. Cells on the
/// grid's border must be unset.
pub fn thin(mask: &mut [bool], w: usize) {
    let at = |m: &[bool], i: usize, (dx, dy): (isize, isize)| {
        m[(i as isize + dy * w as isize + dx) as usize]
    };
    let removable = |m: &[bool], i: usize, second: bool| {
        let p: Vec<bool> = RING.iter().map(|&o| at(m, i, o)).collect();
        let b = p.iter().filter(|&&v| v).count();
        let a = (0..8).filter(|&k| !p[k] && p[(k + 1) % 8]).count();
        // P2, P4, P6, P8 are p[0], p[2], p[4], p[6].
        let (n, e, s, west) = (p[0], p[2], p[4], p[6]);
        let side = if second {
            !(n && e && west) && !(n && s && west)
        } else {
            !(n && e && s) && !(e && s && west)
        };
        (2..=6).contains(&b) && a == 1 && side
    };
    // Only a cell next to a removed one can become removable, so later passes visit just those
    // and the cells that survived the last look.
    let mut seen = vec![false; mask.len()];
    let mut todo: Vec<usize> = (0..mask.len())
        .filter(|&i| mask[i] && RING.iter().any(|&o| !at(mask, i, o)))
        .collect();
    let (mut pass, mut idle) = (0, 0);
    while idle < 2 && !todo.is_empty() {
        let gone: Vec<usize> = todo
            .iter()
            .copied()
            .filter(|&i| mask[i] && removable(mask, i, pass % 2 == 1))
            .collect();
        idle = if gone.is_empty() { idle + 1 } else { 0 };
        for &i in &gone {
            mask[i] = false;
        }
        let mut next = Vec::with_capacity(todo.len());
        let around = gone.iter().flat_map(|&i| {
            RING.iter()
                .map(move |&(dx, dy)| (i as isize + dy * w as isize + dx) as usize)
        });
        for i in todo.iter().copied().chain(around) {
            if mask[i] && !seen[i] {
                seen[i] = true;
                next.push(i);
            }
        }
        for &i in &next {
            seen[i] = false;
        }
        todo = next;
        pass += 1;
    }
}

/// The polylines of a skeleton `mask` one cell wide, in cell coordinates, each with whether it is
/// closed. Lines run between end and junction cells; loops with neither come out closed; a cell
/// with no neighbour is a one-vertex line. A line from a free end to a junction shorter than
/// `min_spur` cells is dropped. Cells on the grid's border must be unset.
pub fn skeleton_lines(mask: &[bool], w: usize, min_spur: f64) -> Vec<(Vec<[f64; 2]>, bool)> {
    let step = |i: usize, (dx, dy): (isize, isize)| (i as isize + dy * w as isize + dx) as usize;
    // A diagonal step is left out where a four-way neighbour already joins the two cells, so a
    // corner does not become a three-way junction.
    let linked = |i: usize, k: usize| {
        let (dx, dy) = RING[k];
        mask[step(i, (dx, dy))]
            && (dx == 0 || dy == 0 || !(mask[step(i, (dx, 0))] || mask[step(i, (0, dy))]))
    };
    let degree = |i: usize| (0..8).filter(|&k| linked(i, k)).count();
    let mut walked = vec![0u8; mask.len()];
    let xy = |i: usize| [(i % w) as f64, (i / w) as f64];
    let mut out = Vec::new();
    // Walks from `i` out along direction `k` until a cell whose degree is not 2, or back to `i`.
    let walk = |i: usize, k: usize, walked: &mut [u8]| -> (Vec<usize>, bool) {
        let mut cells = vec![i];
        let (mut cur, mut dir) = (i, k);
        loop {
            walked[cur] |= 1 << dir;
            let next = step(cur, RING[dir]);
            walked[next] |= 1 << ((dir + 4) % 8);
            cells.push(next);
            if next == i {
                return (cells, true);
            }
            if degree(next) != 2 {
                return (cells, false);
            }
            match (0..8).find(|&d| linked(next, d) && walked[next] & (1 << d) == 0) {
                Some(d) => (cur, dir) = (next, d),
                None => return (cells, false),
            }
        }
    };
    let cells: Vec<usize> = (0..mask.len()).filter(|&i| mask[i]).collect();
    for &i in &cells {
        let deg = degree(i);
        if deg == 0 {
            out.push((vec![xy(i)], false));
        }
        if deg == 2 {
            continue;
        }
        for k in 0..8 {
            if linked(i, k) && walked[i] & (1 << k) == 0 {
                let (line, _) = walk(i, k, &mut walked);
                let ends = [degree(line[0]), degree(*line.last().unwrap())];
                let spur = ends.contains(&1) && ends.iter().any(|&d| d >= 3);
                if spur && ((line.len() - 1) as f64) < min_spur {
                    continue;
                }
                out.push((line.into_iter().map(xy).collect(), false));
            }
        }
    }
    for &i in &cells {
        if degree(i) == 2 {
            if let Some(k) = (0..8).find(|&k| linked(i, k) && walked[i] & (1 << k) == 0) {
                let (line, closed) = walk(i, k, &mut walked);
                out.push((line.into_iter().map(xy).collect(), closed));
            }
        }
    }
    out
}

/// The 8-connected groups of set cells in `mask`, each as its cell indices, in the order their
/// first cell is met row by row. Cells on the grid's border must be unset.
pub fn groups(mask: &[bool], w: usize) -> Vec<Vec<usize>> {
    let mut seen = vec![false; mask.len()];
    let (mut out, mut stack) = (Vec::new(), Vec::new());
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut group = Vec::new();
        stack.push(start);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            group.push(i);
            for &(dx, dy) in &RING {
                let n = (i as isize + dy * w as isize + dx) as usize;
                if mask[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        out.push(group);
    }
    out
}

/// Clear every 8-connected group of set cells in `mask` smaller than `min_cells`. Cells on the
/// grid's border must be unset.
pub fn drop_small(mask: &mut [bool], w: usize, min_cells: usize) {
    if min_cells <= 1 {
        return;
    }
    for group in groups(mask, w) {
        if group.len() < min_cells {
            for i in group {
                mask[i] = false;
            }
        }
    }
}

/// Which cells have their centre within `radius` of a polyline of `lines` (cell coordinates).
/// Each segment is checked exactly against the cells of its bounding box grown by `radius`.
pub fn cover(lines: &[Vec<[f64; 2]>], w: usize, h: usize, radius: f64) -> Vec<bool> {
    let mut out = vec![false; w * h];
    let span = |lo: f64, hi: f64, n: usize| {
        let a = (lo - radius).ceil().max(0.0) as usize;
        let b = ((hi + radius).floor().max(-1.0) + 1.0).min(n as f64) as usize;
        a..b.max(a)
    };
    for line in lines {
        let pairs = line.windows(2).map(|s| (s[0], s[1]));
        let single = (line.len() == 1).then(|| (line[0], line[0]));
        for (a, b) in pairs.chain(single) {
            for y in span(a[1].min(b[1]), a[1].max(b[1]), h) {
                for x in span(a[0].min(b[0]), a[0].max(b[0]), w) {
                    if !out[y * w + x] && to_segment([x as f64, y as f64], a, b) <= radius {
                        out[y * w + x] = true;
                    }
                }
            }
        }
    }
    out
}

/// `line` smoothed by a Gaussian of `sigma` vertices along it. A closed line (last vertex equal
/// to the first) wraps around and stays closed; an open one keeps its end points.
pub fn smooth(line: &[[f64; 2]], sigma: f64, closed: bool) -> Vec<[f64; 2]> {
    let kernel = math::gaussian_kernel(sigma);
    let r = (kernel.len() / 2) as isize;
    let body = if closed {
        &line[..line.len() - 1]
    } else {
        line
    };
    let n = body.len() as isize;
    if n < 3 {
        return line.to_vec();
    }
    let mut out: Vec<[f64; 2]> = (0..n)
        .map(|i| {
            if !closed && (i == 0 || i == n - 1) {
                return body[i as usize];
            }
            let mut p = [0.0, 0.0];
            for (t, &k) in kernel.iter().enumerate() {
                let j = i + t as isize - r;
                let j = if closed {
                    j.rem_euclid(n)
                } else {
                    j.clamp(0, n - 1)
                };
                p[0] += f64::from(k) * body[j as usize][0];
                p[1] += f64::from(k) * body[j as usize][1];
            }
            p
        })
        .collect();
    if closed {
        out.push(out[0]);
    }
    out
}

/// The distance from `p` to the segment `a`-`b`.
fn to_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    math::hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy)
}

/// `line` with the vertices that lie within `tol` of the simplified line removed, by
/// Ramer-Douglas-Peucker. The end points are kept, so a closed line stays closed.
pub fn simplify(line: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    if line.len() < 3 {
        return line.to_vec();
    }
    let mut keep = vec![false; line.len()];
    keep[0] = true;
    keep[line.len() - 1] = true;
    let mut stack = vec![(0, line.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let far = (a + 1..b)
            .map(|i| (i, to_segment(line[i], line[a], line[b])))
            .fold((a, -1.0), |m, c| if c.1 > m.1 { c } else { m });
        if far.1 > tol {
            keep[far.0] = true;
            stack.push((a, far.0));
            stack.push((far.0, b));
        }
    }
    line.iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| *p)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` by `h` grid set where `f(x, y)` holds.
    fn grid(w: usize, h: usize, f: impl Fn(usize, usize) -> bool) -> Vec<bool> {
        (0..w * h).map(|i| f(i % w, i / w)).collect()
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn distance_matches_brute_force() {
        let (w, h) = (23, 17);
        // A scatter of holes in an inset block, with no structure the transform could lean on.
        let mask = grid(w, h, |x, y| {
            x > 0 && y > 0 && x < w - 1 && y < h - 1 && (x * 7 + y * 13 + x * y) % 11 != 0
        });
        let d = distance(&mask, w, h);
        for i in 0..w * h {
            let mut best = f64::INFINITY;
            for j in 0..w * h {
                if !mask[j] {
                    let (dx, dy) = (
                        (i % w) as f64 - (j % w) as f64,
                        (i / w) as f64 - (j / w) as f64,
                    );
                    best = best.min((dx * dx + dy * dy).sqrt());
                }
            }
            assert!(
                (f64::from(d[i]) - best).abs() < 1e-4,
                "cell {i}: {} vs {best}",
                d[i]
            );
        }
    }

    #[test]
    fn iso_lines_close_around_a_square() {
        let (w, h) = (12, 12);
        let mask = grid(w, h, |x, y| (3..9).contains(&x) && (3..9).contains(&y));
        let d = distance(&mask, w, h);
        let lines = iso_lines(&d, w, h, 0.5, 1.0, 10.0);
        // Depths 1, 2 and 3 hold levels 0.5, 1.5 and 2.5: three nested rings.
        assert_eq!(lines.len(), 3);
        for l in &lines {
            assert_eq!(l.first(), l.last());
            assert!(l.len() > 4);
        }
        let outer = &lines[0];
        let xs: Vec<f64> = outer.iter().map(|p| p[0]).collect();
        let (lo, hi) = (
            xs.iter().copied().fold(99.0, f64::min),
            xs.iter().copied().fold(-1.0, f64::max),
        );
        assert!(
            (lo - 2.5).abs() < 1e-9 && (hi - 8.5).abs() < 1e-9,
            "{lo} {hi}"
        );
    }

    #[test]
    fn thinning_a_bar_leaves_a_line() {
        let (w, h) = (20, 9);
        let mut mask = grid(w, h, |x, y| (2..18).contains(&x) && (2..7).contains(&y));
        thin(&mut mask, w);
        let rows: Vec<usize> = (0..h)
            .filter(|&y| (0..w).any(|x| mask[y * w + x]))
            .collect();
        assert_eq!(rows, vec![4]);
        let lines = skeleton_lines(&mask, w, 0.0);
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].1);
        assert!(lines[0].0.len() >= 8);
    }

    #[test]
    fn skeleton_of_a_ring_is_one_closed_loop() {
        let (w, h) = (16, 16);
        let mask = grid(w, h, |x, y| {
            let (dx, dy) = (x as f64 - 7.5, y as f64 - 7.5);
            (16.0..30.25).contains(&(dx * dx + dy * dy))
        });
        let mut skel = mask.clone();
        thin(&mut skel, w);
        let lines = skeleton_lines(&skel, w, 3.0);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].1);
    }

    #[test]
    fn short_spurs_go_and_long_branches_stay() {
        let (w, h) = (24, 20);
        // A T: a long bar with a stem of 8 cells and a stub of 2.
        let mask = grid(w, h, |x, y| {
            (y == 5 && (2..22).contains(&x))
                || (x == 12 && (6..14).contains(&y))
                || (x == 5 && (6..8).contains(&y))
        });
        let lines = skeleton_lines(&mask, w, 4.0);
        let longest = lines.iter().map(|l| l.0.len()).max().unwrap();
        assert!(longest >= 8);
        assert!(
            lines
                .iter()
                .all(|l| !l.0.iter().any(|p| p[0] == 5.0 && p[1] == 7.0)),
            "{lines:?}"
        );
    }

    #[test]
    fn small_groups_are_dropped() {
        let (w, h) = (10, 6);
        let mut mask = grid(w, h, |x, y| {
            (y == 2 && (1..4).contains(&x)) || (x == 7 && y == 3)
        });
        drop_small(&mut mask, w, 2);
        assert_eq!(mask.iter().filter(|&&m| m).count(), 3);
    }

    #[test]
    fn cover_marks_cells_near_a_line() {
        let c = cover(&[vec![[2.0, 2.0], [7.0, 2.0]]], 10, 5, 1.0);
        let row = |y: usize| (0..10).filter(|&x| c[y * 10 + x]).collect::<Vec<_>>();
        assert_eq!(row(2), vec![1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(row(1), vec![2, 3, 4, 5, 6, 7]);
        assert!(row(4).is_empty());
    }

    #[test]
    fn simplify_keeps_corners() {
        let l: Vec<[f64; 2]> = (0..=10)
            .map(|i| [f64::from(i), 0.0])
            .chain((1..=10).map(|i| [10.0, f64::from(i)]))
            .collect();
        assert_eq!(
            simplify(&l, 0.1),
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]
        );
    }

    #[test]
    fn smooth_keeps_ends_and_closure() {
        let open = vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0], [3.0, 1.0]];
        let s = smooth(&open, 1.0, false);
        assert_eq!((s[0], s[3]), (open[0], open[3]));
        let closed = vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]];
        let s = smooth(&closed, 1.0, true);
        assert_eq!(s.first(), s.last());
        assert_eq!(s.len(), 5);
    }
}
