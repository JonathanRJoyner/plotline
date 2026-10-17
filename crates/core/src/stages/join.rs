//! Joining paths that meet into the fewest pen-down trails.
//!
//! The vertices of the paths become the vertices of a graph, with the vertices closer than
//! [`SNAP_MM`] taken as one, and each segment between two of them an edge, drawn once however
//! often the paths repeat it. A connected graph with `k > 0` vertices of odd degree cannot be drawn
//! in fewer than `k / 2` trails that use each edge once, since every trail has two ends and an odd
//! vertex must be the end of one; this is the bound J. Edmonds and E. L. Johnson start the
//! postman problem from ("Matching, Euler tours and the Chinese postman", *Mathematical
//! Programming* 5, 1973, pp. 88-124). The join meets it: it pairs the odd vertices of each
//! component with virtual edges, which makes every degree even, finds an Euler circuit of each
//! component as C. Hierholzer describes ("Ueber die Möglichkeit, einen Linienzug ohne Wiederholung
//! und ohne Unterbrechung zu umfahren", *Mathematische Annalen* 6, 1873, pp. 30-32), and cuts the
//! circuit at the virtual edges.
//!
//! Edmonds and Johnson go below the bound by drawing some edges twice: a set of paths that pairs
//! the odd vertices at the least total length, found by weighted matching. With `retrace` the join
//! takes a greedy share of that: the edges whose two ends are both still odd, shortest first, are
//! drawn a second time while their component has at least four odd vertices, and each edge so
//! doubled makes two odd vertices even, which saves a trail and so a pen lift.

use crate::types::Paths;

/// Vertices closer than this, in mm, are one vertex of the graph: far under any pen's width, and
/// far over the rounding between two computations of one point.
pub const SNAP_MM: f64 = 1e-3;

/// The root of `i`'s set, the lowest index in it, halving the path to it on the way.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Put `a` and `b` in one set, rooted at the lower of their roots.
fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (root(parent, a), root(parent, b));
    parent[ra.max(rb)] = ra.min(rb);
}

/// For each of `points`, the index of the lowest point it is joined to by a chain of points closer
/// than [`SNAP_MM`]. The points are sorted into cells [`SNAP_MM`] wide, so only a cell and its
/// eight neighbours are compared; a non-finite point is joined to none.
fn snap(points: &[[f64; 2]]) -> Vec<usize> {
    let cell = |p: [f64; 2]| {
        (
            (p[0] / SNAP_MM).floor() as i64,
            (p[1] / SNAP_MM).floor() as i64,
        )
    };
    let mut cells: Vec<((i64, i64), usize)> =
        (0..points.len()).map(|i| (cell(points[i]), i)).collect();
    cells.sort_unstable();
    let mut parent: Vec<usize> = (0..points.len()).collect();
    for &((cx, cy), i) in &cells {
        for (dx, dy) in [
            (-1, -1),
            (-1, 0),
            (-1, 1),
            (0, -1),
            (0, 0),
            (0, 1),
            (1, -1),
            (1, 0),
            (1, 1),
        ] {
            let key = (cx.saturating_add(dx), cy.saturating_add(dy));
            let from = cells.partition_point(|c| c.0 < key);
            for &(k, j) in &cells[from..] {
                if k != key {
                    break;
                }
                let (ex, ey) = (points[i][0] - points[j][0], points[i][1] - points[j][1]);
                if j > i && ex * ex + ey * ey < SNAP_MM * SNAP_MM {
                    union(&mut parent, i, j);
                }
            }
        }
    }
    (0..points.len()).map(|i| root(&mut parent, i)).collect()
}

/// `paths` as the fewest trails that draw each of their edges once, or, with `retrace`, fewer
/// still by drawing some short edges twice. See the module documentation.
///
/// A path of one vertex, a dot, or of no edge longer than [`SNAP_MM`] stays as it is, and comes
/// first. The trails follow, by component in the order of each component's first vertex in the
/// input; a component with odd vertices gives open trails between them, and one without gives a
/// closed trail from its first vertex. If the trails would be more paths
/// than the input, which can happen only when the input draws an edge twice, the input is
/// returned unchanged.
pub fn join(paths: &Paths, retrace: bool) -> Paths {
    let coords = paths.coords();
    let rep = snap(coords);
    // The graph's vertices: one per snapped set, numbered in the order the input first has them.
    let mut id = vec![usize::MAX; coords.len()];
    let mut at: Vec<[f64; 2]> = Vec::new();
    for i in 0..coords.len() {
        if id[rep[i]] == usize::MAX {
            id[rep[i]] = at.len();
            at.push(coords[rep[i]]);
        }
        id[i] = id[rep[i]];
    }
    let mut out = Paths::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for p in paths.iter() {
        let before = edges.len();
        for k in start + 1..start + p.len() {
            let (a, b) = (id[k - 1], id[k]);
            if a != b {
                edges.push((a.min(b), a.max(b)));
            }
        }
        if edges.len() == before {
            out.push(p);
        }
        start += p.len();
    }
    edges.sort_unstable();
    edges.dedup();
    let n = at.len();
    let mut degree = vec![0usize; n];
    for &(a, b) in &edges {
        degree[a] += 1;
        degree[b] += 1;
    }
    let mut parent: Vec<usize> = (0..n).collect();
    for &(a, b) in &edges {
        union(&mut parent, a, b);
    }
    if retrace {
        let length = |(a, b): (usize, usize)| {
            let (dx, dy) = (at[a][0] - at[b][0], at[a][1] - at[b][1]);
            dx * dx + dy * dy
        };
        // A component with `k` odd vertices gives `k / 2` trails, so a doubled edge saves one only
        // while its component has at least four.
        let mut odd_left = vec![0usize; n];
        for v in (0..n).filter(|&v| degree[v] % 2 == 1) {
            odd_left[root(&mut parent, v)] += 1;
        }
        let mut odd: Vec<(usize, usize)> = (edges.iter().copied())
            .filter(|&(a, b)| degree[a] % 2 == 1 && degree[b] % 2 == 1)
            .collect();
        odd.sort_by(|&e, &f| length(e).total_cmp(&length(f)).then(e.cmp(&f)));
        for (a, b) in odd {
            let r = root(&mut parent, a);
            if degree[a] % 2 == 1 && degree[b] % 2 == 1 && odd_left[r] >= 4 {
                edges.push((a, b));
                degree[a] += 1;
                degree[b] += 1;
                odd_left[r] -= 2;
            }
        }
    }
    // Pair the odd vertices of each component, in vertex order, with virtual edges.
    let real = edges.len();
    let mut waiting = vec![usize::MAX; n];
    for v in (0..n).filter(|&v| degree[v] % 2 == 1) {
        let r = root(&mut parent, v);
        if waiting[r] == usize::MAX {
            waiting[r] = v;
        } else {
            edges.push((waiting[r], v));
            waiting[r] = usize::MAX;
        }
    }
    let mut adj: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (e, &(a, b)) in edges.iter().enumerate() {
        adj[a].push((b, e));
        adj[b].push((a, e));
    }
    let mut used = vec![false; edges.len()];
    let mut next = vec![0usize; n];
    // Every degree is even now, so one walk from a component's first vertex uses all its edges.
    for v in 0..n {
        if adj[v].iter().all(|&(_, e)| used[e]) {
            continue;
        }
        // Hierholzer's walk: each vertex, with the edge it was reached by, leaves the stack once
        // its edges are used, so `circuit` holds the circuit backwards.
        let mut stack: Vec<(usize, usize)> = vec![(v, usize::MAX)];
        let mut circuit: Vec<(usize, usize)> = Vec::new();
        while let Some(&(u, _)) = stack.last() {
            while next[u] < adj[u].len() && used[adj[u][next[u]].1] {
                next[u] += 1;
            }
            if let Some(&(w, e)) = adj[u].get(next[u]) {
                used[e] = true;
                stack.push((w, e));
            } else {
                circuit.push(stack.pop().expect("the stack is not empty"));
            }
        }
        circuit.reverse();
        // Now circuit[q + 1].1 is the edge from circuit[q].0 on, and the last vertex is the first
        // again. Cut the circuit at each virtual edge, starting after one, so no trail wraps round.
        let m = circuit.len() - 1;
        let virtual_after = |q: usize| circuit[q + 1].1 >= real;
        let Some(p) = (0..m).position(virtual_after) else {
            out.push(&circuit.iter().map(|&(w, _)| at[w]).collect::<Vec<_>>());
            continue;
        };
        let mut trail = Vec::new();
        for k in 1..=m {
            let q = (p + k) % m;
            trail.push(at[circuit[q].0]);
            if virtual_after(q) {
                if trail.len() > 1 {
                    out.push(&trail);
                }
                trail.clear();
            }
        }
    }
    if out.len() > paths.len() {
        return paths.clone();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[&[[f64; 2]]]) -> Paths {
        let mut p = Paths::new();
        for q in list {
            p.push(q);
        }
        p
    }

    /// The edges of `p` as sorted pairs of points, each once.
    fn edges(p: &Paths) -> Vec<[[i64; 2]; 2]> {
        let key = |v: [f64; 2]| [(v[0] * 1e6).round() as i64, (v[1] * 1e6).round() as i64];
        let mut e: Vec<[[i64; 2]; 2]> = (p.iter())
            .flat_map(|q| q.windows(2).map(|s| [key(s[0]), key(s[1])]))
            .filter(|s| s[0] != s[1])
            .map(|mut s| {
                s.sort();
                s
            })
            .collect();
        e.sort();
        e.dedup();
        e
    }

    #[test]
    fn pieces_of_one_line_join_into_one() {
        let input = paths(&[
            &[[0.0, 0.0], [1.0, 0.0]],
            &[[2.0, 0.0], [1.0, 0.0]],
            &[[2.0, 0.0], [3.0, 0.0]],
        ]);
        let out = join(&input, false);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.get(0),
            &[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]]
        );
    }
    #[test]
    fn two_squares_sharing_a_side_draw_it_once_in_one_trail() {
        // The shared side's ends are the only odd vertices, so one trail draws all seven sides.
        let left: &[[f64; 2]] = &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let right: &[[f64; 2]] = &[[1.0, 0.0], [2.0, 0.0], [2.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let input = paths(&[left, right]);
        let out = join(&input, false);
        assert_eq!(out.len(), 1);
        assert_eq!(out.get(0).len(), 8);
        assert_eq!(edges(&out), edges(&input));
        assert!((out.total_length() - 7.0).abs() < 1e-12);
    }
    #[test]
    fn a_three_armed_star_needs_two_trails_and_retrace_makes_it_one() {
        let c = [5.0, 5.0];
        let arms = [[6.0, 5.0], [5.0, 7.0], [3.0, 5.0]];
        let input = paths(
            &arms
                .map(|a| [c, a])
                .iter()
                .map(|p| &p[..])
                .collect::<Vec<_>>(),
        );
        let out = join(&input, false);
        assert_eq!(out.len(), 2);
        assert_eq!(edges(&out), edges(&input));
        // The centre and every arm's end are odd. The shortest arm is drawn twice, which leaves the
        // two longer arms' ends as the only odd vertices.
        let out = join(&input, true);
        assert_eq!(out.len(), 1);
        assert_eq!(edges(&out), edges(&input));
        assert!((out.total_length() - (1.0 + 2.0 + 2.0 + 1.0)).abs() < 1e-12);
    }
    #[test]
    fn retrace_never_doubles_a_line_that_is_one_trail_already() {
        let input = paths(&[
            &[[0.0, 0.0], [1.0, 0.0]],
            &[[5.0, 5.0], [6.0, 5.0], [6.0, 7.0]],
        ]);
        let out = join(&input, true);
        assert_eq!(out.len(), 2);
        assert_eq!(out.total_length(), input.total_length());
    }
    #[test]
    fn near_ends_snap_and_far_ones_do_not() {
        let near = paths(&[
            &[[0.0, 0.0], [1.0, 0.0]],
            &[[1.0 + SNAP_MM / 2.0, 0.0], [2.0, 0.0]],
        ]);
        assert_eq!(join(&near, false).len(), 1);
        let far = paths(&[
            &[[0.0, 0.0], [1.0, 0.0]],
            &[[1.0 + 2.0 * SNAP_MM, 0.0], [2.0, 0.0]],
        ]);
        assert_eq!(join(&far, false).len(), 2);
    }
    #[test]
    fn a_repeated_path_is_drawn_once() {
        let line: &[[f64; 2]] = &[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]];
        let out = join(
            &paths(&[line, line, &[[2.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]),
            false,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out.get(0), line);
    }
    #[test]
    fn dots_and_points_stay_and_come_first() {
        let input = paths(&[
            &[[0.0, 0.0], [1.0, 0.0]],
            &[[4.0, 4.0]],
            &[[7.0, 7.0], [7.0, 7.0]],
        ]);
        let out = join(&input, false);
        assert_eq!(out.len(), 3);
        assert_eq!(out.get(0), &[[4.0, 4.0]]);
        assert_eq!(out.get(1), &[[7.0, 7.0], [7.0, 7.0]]);
        assert_eq!(out.get(2), &[[0.0, 0.0], [1.0, 0.0]]);
    }
    #[test]
    fn a_path_that_retraces_itself_is_kept_when_joining_would_split_it() {
        // One path draws the edge to (2, 1) twice; drawing it once leaves a star of three arms.
        let input = paths(&[&[[0.0, 0.0], [1.0, 0.0], [2.0, 1.0], [1.0, 0.0], [2.0, -1.0]]]);
        assert_eq!(join(&input, false).get(0), input.get(0));
    }
    #[test]
    fn empty_and_non_finite_input() {
        assert_eq!(join(&Paths::new(), true).len(), 0);
        let input = paths(&[&[[f64::NAN, 0.0], [1.0, 0.0]], &[[1.0, 0.0], [2.0, 0.0]]]);
        let out = join(&input, false);
        assert_eq!(out.len(), 1);
        assert_eq!(out.get(0).len(), 3);
    }
}
