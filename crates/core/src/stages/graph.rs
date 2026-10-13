//! Graph helpers for the renderers that draw edges between points: a Delaunay triangulation, a
//! greedy split of a graph into pen-down trails, and clipping of a polyline to a rectangle.
//!
//! The triangulation is B. Delaunay, "Sur la sphère vide", *Bulletin de l'Académie des Sciences
//! de l'URSS*, 1934, computed by the `delaunator` crate. Polylines are clipped segment by segment
//! with the slab method: Y.-D. Liang and B. A. Barsky, "A new concept and method for line
//! clipping", *ACM Transactions on Graphics* 3(1), 1984, pp. 1-22.

use delaunator::{Point, Triangulation};

use super::lines::clamp_to;

/// The Delaunay triangulation of `points`. Of points within rounding of each other only one is
/// triangulated, and which copy survives is unspecified; all-collinear input gives no triangles.
pub(crate) fn triangulate(points: &[[f64; 2]]) -> Triangulation {
    let pts: Vec<Point> = points.iter().map(|p| Point { x: p[0], y: p[1] }).collect();
    delaunator::triangulate(&pts)
}

/// Every Delaunay edge once as `(a, b)` with `a < b`, sorted. Without triangles (under three
/// distinct points, or all collinear) the distinct points sorted by `(x, y, index)` are chained
/// instead. A duplicate of a point is in no edge.
pub(crate) fn delaunay_edges(points: &[[f64; 2]]) -> Vec<(usize, usize)> {
    let tri = triangulate(points).triangles;
    let mut edges: Vec<(usize, usize)> = if tri.is_empty() {
        let mut order: Vec<usize> = (0..points.len()).collect();
        order.sort_by(|&i, &j| {
            let (p, q) = (points[i], points[j]);
            p[0].total_cmp(&q[0])
                .then(p[1].total_cmp(&q[1]))
                .then(i.cmp(&j))
        });
        order.dedup_by(|j, i| points[*i] == points[*j]);
        order.windows(2).map(|w| (w[0], w[1])).collect()
    } else {
        let (triangles, _) = tri.as_chunks::<3>();
        triangles
            .iter()
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .collect()
    };
    for e in &mut edges {
        *e = (e.0.min(e.1), e.0.max(e.1));
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

/// Splits the edges of a graph on `n` vertices into trails, each a list of vertices with an edge
/// between neighbours, using every edge once. Each trail starts at the lowest-index vertex of odd
/// remaining degree, or else the lowest with any edge left, and follows the unused edge to the
/// lowest-numbered neighbour until it is stuck. A trail from an odd vertex ends at another odd
/// vertex, so a connected graph with `k` odd vertices gives `k / 2` open trails, plus closed ones.
pub(crate) fn trails(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    // Neighbours with the edge's index, sorted so that the first unused one is the lowest.
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (i, &(a, b)) in edges.iter().enumerate() {
        adj[a].push((b, i));
        adj[b].push((a, i));
    }
    adj.iter_mut().for_each(|a| a.sort_unstable());
    let mut degree: Vec<usize> = adj.iter().map(Vec::len).collect();
    let mut used = vec![false; edges.len()];
    let mut next = vec![0; n];
    // While odd vertices remain, walks start and end at odd ones, so a vertex skipped as even never
    // turns odd, and a vertex with no edges left never regains one: both cursors only advance.
    let (mut odd, mut any) = (0, 0);
    let mut out = Vec::new();
    loop {
        while odd < n && degree[odd].is_multiple_of(2) {
            odd += 1;
        }
        while any < n && degree[any] == 0 {
            any += 1;
        }
        let start = if odd < n { odd } else { any };
        if start == n {
            return out;
        }
        let mut trail = vec![start];
        let mut at = start;
        loop {
            while next[at] < adj[at].len() && used[adj[at][next[at]].1] {
                next[at] += 1;
            }
            let Some(&(to, edge)) = adj[at].get(next[at]) else {
                break;
            };
            used[edge] = true;
            degree[at] -= 1;
            degree[to] -= 1;
            trail.push(to);
            at = to;
        }
        out.push(trail);
    }
}

/// The part of the segment `a` to `b` inside `rect` (`[x, y, w, h]`), as the `t` interval of
/// `a + t * (b - a)` it covers, or `None`. Liang-Barsky.
fn clip_segment(rect: [f64; 4], a: [f64; 2], b: [f64; 2]) -> Option<(f64, f64)> {
    let [x, y, w, h] = rect;
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-dx, a[0] - x),
        (dx, x + w - a[0]),
        (-dy, a[1] - y),
        (dy, y + h - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    (t0 <= t1).then_some((t0, t1))
}

/// The parts of the polyline `line` inside `rect` (`[x, y, w, h]`): each segment is clipped,
/// consecutive pieces are joined, and the line is split where it leaves the rect. Repeated
/// vertices are merged, and pieces of one vertex are dropped. Every vertex lies in `rect`.
pub(crate) fn clip_polyline(rect: [f64; 4], line: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    let mut piece: Vec<[f64; 2]> = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let Some((t0, t1)) = clip_segment(rect, a, b) else {
            finish(&mut out, &mut piece);
            continue;
        };
        // Clamped throughout: a `t` that rounds to 0 or 1 can stand for an end a few ulp outside.
        let at = |t: f64| match t {
            t if t <= 0.0 => clamp_to(rect, a),
            t if t >= 1.0 => clamp_to(rect, b),
            t => clamp_to(rect, [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]),
        };
        let (p0, p1) = (at(t0), at(t1));
        if piece.last() != Some(&p0) {
            finish(&mut out, &mut piece);
            piece.push(p0);
        }
        if piece.last() != Some(&p1) {
            piece.push(p1);
        }
        if t1 < 1.0 {
            finish(&mut out, &mut piece);
        }
    }
    finish(&mut out, &mut piece);
    out
}

/// Moves `piece` to `out` if it has two vertices or more, and empties it.
fn finish(out: &mut Vec<Vec<[f64; 2]>>, piece: &mut Vec<[f64; 2]>) {
    if piece.len() >= 2 {
        out.push(std::mem::take(piece));
    }
    piece.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[Vec<[f64; 2]>], b: &[Vec<[f64; 2]>]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|(p, q)| {
                p.len() == q.len()
                    && p.iter()
                        .zip(q)
                        .all(|(u, v)| (u[0] - v[0]).abs() < 1e-9 && (u[1] - v[1]).abs() < 1e-9)
            })
    }

    #[test]
    fn square_gives_five_edges() {
        let edges = delaunay_edges(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]]);
        assert_eq!(edges.len(), 5);
        assert!(edges.iter().all(|&(a, b)| a < b));
        assert!(edges.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn delaunay_edges_handles_degenerate_input() {
        assert_eq!(delaunay_edges(&[]), []);
        assert_eq!(delaunay_edges(&[[1.0, 2.0]]), []);
        assert_eq!(delaunay_edges(&[[0.0, 0.0], [1.0, 1.0]]), [(0, 1)]);
        // Collinear, out of order: chained by x.
        let line = [[3.0, 0.0], [1.0, 0.0], [0.0, 0.0], [2.0, 0.0]];
        assert_eq!(delaunay_edges(&line), [(0, 3), (1, 2), (1, 3)]);
        // Collinear and vertical: chained by y.
        let upright = [[0.0, 2.0], [0.0, 0.0], [0.0, 1.0]];
        assert_eq!(delaunay_edges(&upright), [(0, 2), (1, 2)]);
        // A duplicate point is in no edge.
        assert_eq!(
            delaunay_edges(&[[0.0, 0.0], [0.0, 0.0], [1.0, 1.0]]),
            [(0, 2)]
        );
        let square = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 1.0]];
        let edges = delaunay_edges(&square);
        assert_eq!(edges.len(), 5);
        let uses = |v: usize| edges.iter().filter(|e| e.0 == v || e.1 == v).count();
        assert!(uses(3) == 0 || uses(4) == 0);
    }

    #[test]
    fn a_path_graph_is_one_trail() {
        assert_eq!(trails(4, &[(0, 1), (1, 2), (2, 3)]), [[0, 1, 2, 3]]);
    }

    #[test]
    fn a_four_armed_star_is_two_trails() {
        let t = trails(5, &[(0, 1), (0, 2), (0, 3), (0, 4)]);
        assert_eq!(t, [vec![1, 0, 2], vec![3, 0, 4]]);
    }

    #[test]
    fn a_cycle_is_one_closed_trail() {
        assert_eq!(trails(3, &[(0, 1), (1, 2), (0, 2)]), [[0, 1, 2, 0]]);
    }

    #[test]
    fn trails_cover_a_delaunay_graph_with_the_fewest_open_trails() {
        let mut seed = 12345_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1_u64 << 53) as f64
        };
        let points: Vec<[f64; 2]> = (0..300).map(|_| [next() * 100.0, next() * 100.0]).collect();
        let edges = delaunay_edges(&points);
        let t = trails(points.len(), &edges);
        let mut used: Vec<(usize, usize)> = t
            .iter()
            .flat_map(|trail| trail.windows(2).map(|w| (w[0].min(w[1]), w[0].max(w[1]))))
            .collect();
        used.sort();
        assert_eq!(used, edges);
        let mut degree = vec![0_usize; points.len()];
        for &(a, b) in &edges {
            degree[a] += 1;
            degree[b] += 1;
        }
        let odd = degree.iter().filter(|d| *d % 2 == 1).count();
        let open = t
            .iter()
            .filter(|trail| trail[0] != trail[trail.len() - 1])
            .count();
        assert_eq!(open, odd / 2);
    }

    #[test]
    fn clip_polyline_splits_where_the_line_leaves() {
        let line = [
            [-5.0, 5.0],
            [5.0, 5.0],
            [5.0, 15.0],
            [5.0, 20.0],
            [8.0, 5.0],
        ];
        let pieces = clip_polyline([0.0, 0.0, 10.0, 10.0], &line);
        let want = vec![
            vec![[0.0, 5.0], [5.0, 5.0], [5.0, 10.0]],
            vec![[7.0, 10.0], [8.0, 5.0]],
        ];
        assert!(close(&pieces, &want), "{pieces:?}");
    }

    #[test]
    fn clip_polyline_keeps_an_end_just_outside_inside() {
        // The end lies one ulp (about 5.7e-14) beyond the right edge. Over a span this long the
        // clip's `t` rounds to exactly 1, so the end is taken as given unless it is clamped.
        let edge = 300.0_f64;
        let beyond = f64::from_bits(edge.to_bits() + 1);
        assert!(beyond - edge > 5e-14);
        let rect = [0.0, 0.0, edge, edge];
        let pieces = clip_polyline(rect, &[[-1000.0, 150.0], [beyond, 150.0]]);
        let last = pieces[0][pieces[0].len() - 1];
        assert!(last[0] <= edge, "{last:?}");
    }

    #[test]
    fn clip_polyline_drops_single_points_and_repeats() {
        let rect = [0.0, 0.0, 10.0, 10.0];
        // Touches the corner only, then runs outside.
        assert!(clip_polyline(rect, &[[-5.0, 5.0], [5.0, -5.0]]).is_empty());
        assert!(clip_polyline(rect, &[[4.0, 4.0]]).is_empty());
        assert!(clip_polyline(rect, &[[4.0, 4.0], [4.0, 4.0]]).is_empty());
        let kept = clip_polyline(rect, &[[1.0, 1.0], [1.0, 1.0], [2.0, 2.0]]);
        assert_eq!(kept, [[[1.0, 1.0], [2.0, 2.0]]]);
    }
}
