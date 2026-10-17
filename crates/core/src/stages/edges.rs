//! The graph renderers: lines drawn between the points a placer made. [`MstRenderer`] draws the
//! Euclidean minimum spanning tree, [`DelaunayRenderer`] the edges of the Delaunay triangulation,
//! and [`CellsRenderer`] the outlines of the Voronoi cells.
//!
//! The tree is found with J. B. Kruskal's algorithm ("On the shortest spanning subtree of a graph
//! and the traveling salesman problem", *Proc. AMS* 7(1), 1956, pp. 48-50), keeping the trees of
//! the forest in a union-find structure (R. E. Tarjan, "Efficiency of a good but not linear set
//! union algorithm", *J. ACM* 22(2), 1975, pp. 215-225). It looks at the Delaunay edges alone,
//! because the Euclidean minimum spanning tree is a subgraph of the Delaunay triangulation (M. I.
//! Shamos and D. Hoey, "Closest-point problems", *Proc. FOCS*, 1975, pp. 151-162). The
//! triangulation is B. Delaunay's, "Sur la sphère vide", 1934, computed as in [`super::graph`].
//! The Voronoi diagram is its dual: a Voronoi vertex is the circumcentre of a Delaunay triangle,
//! and the Voronoi edge between two vertices crosses the Delaunay edge their triangles share
//! (F. Aurenhammer, "Voronoi diagrams: a survey of a fundamental geometric data structure", *ACM
//! Computing Surveys* 23(3), 1991, pp. 345-405).

use std::sync::Arc;

use delaunator::EMPTY;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::graph::{clip_polyline, delaunay_edges, trails, triangulate};
use crate::math;
use crate::settings::{defaults_for, schema_for, validate};
use crate::stage::{Artifact, Frame, Inputs, Kind, RunContext, Stage, StageError, StageRun, Step};
use crate::types::Paths;

/// Settings for a renderer that takes none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct NoSettings {}

// Each help text stays on one line: schemars keeps the line breaks of a doc comment.
/// Settings for the triangle mesh renderer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct DelaunaySettings {
    /// Leave out edges longer than this, such as those across light areas.
    #[schemars(range(min = 0.5, max = 50.0), extend("x-step" = 0.5, "x-unit" = "mm"))]
    pub max_edge_mm: f64,
}

impl Default for DelaunaySettings {
    fn default() -> Self {
        DelaunaySettings { max_edge_mm: 8.0 }
    }
}

/// The length of the segment from point `a` to point `b`.
fn length(points: &[[f64; 2]], (a, b): (usize, usize)) -> f64 {
    math::hypot(points[b][0] - points[a][0], points[b][1] - points[a][1])
}

/// The root of `i`'s set, halving the path to it on the way.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// The edges of the Euclidean minimum spanning tree of `points`: Kruskal's algorithm over the
/// Delaunay edges, shortest first, ties to the lower `(a, b)`. A duplicate of a point has no edge.
fn spanning_tree(points: &[[f64; 2]]) -> Vec<(usize, usize)> {
    let mut edges: Vec<(f64, usize, usize)> = delaunay_edges(points)
        .into_iter()
        .map(|e| (length(points, e), e.0, e.1))
        .collect();
    edges.sort_by(|p, q| p.0.total_cmp(&q.0).then(p.1.cmp(&q.1)).then(p.2.cmp(&q.2)));
    let mut parent: Vec<usize> = (0..points.len()).collect();
    let mut tree = Vec::new();
    for (_, a, b) in edges {
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
            tree.push((a, b));
        }
    }
    tree
}

/// The centre of the circle through `a`, `b` and `c`; not finite if they are collinear.
fn circumcentre(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> [f64; 2] {
    let (bx, by) = (b[0] - a[0], b[1] - a[1]);
    let (cx, cy) = (c[0] - a[0], c[1] - a[1]);
    let d = 2.0 * (bx * cy - by * cx);
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    [
        a[0] + (cy * b2 - by * c2) / d,
        a[1] + (bx * c2 - cx * b2) / d,
    ]
}

/// The Voronoi edges of `points` that have two ends, as paths clipped to `rect`: one per Delaunay
/// edge shared by two triangles, joining their circumcentres. Edges on the hull are left out,
/// since their Voronoi edges run off to infinity, and so is any edge with an end that is not finite
/// (a sliver of collinear points).
fn voronoi_paths(points: &[[f64; 2]], rect: [f64; 4]) -> Paths {
    let tri = triangulate(points);
    let (corners, _) = tri.triangles.as_chunks::<3>();
    let centres: Vec<[f64; 2]> = corners
        .iter()
        .map(|t| circumcentre(points[t[0]], points[t[1]], points[t[2]]))
        .collect();
    let edges: Vec<(usize, usize)> = tri
        .halfedges
        .iter()
        .enumerate()
        .filter(|&(e, &h)| h != EMPTY && e < h)
        .map(|(e, &h)| (e / 3, h / 3))
        .filter(|&(s, t)| centres[s].iter().chain(&centres[t]).all(|v| v.is_finite()))
        .collect();
    let mut paths = Paths::new();
    for trail in trails(centres.len(), &edges) {
        let line: Vec<[f64; 2]> = trail.iter().map(|&t| centres[t]).collect();
        for piece in clip_polyline(rect, &line) {
            paths.push(&piece);
        }
    }
    paths
}

/// The trails over `edges` between `points`, as paths.
fn trail_paths(points: &[[f64; 2]], edges: &[(usize, usize)]) -> Paths {
    let mut paths = Paths::new();
    for trail in trails(points.len(), edges) {
        let line: Vec<[f64; 2]> = trail.iter().map(|&i| points[i]).collect();
        paths.push(&line);
    }
    paths
}

/// A run that does all its work in the first step, which reports one frame; the second step gives
/// the paths.
fn start_run(
    name: &'static str,
    job: impl FnOnce() -> Paths + Send + 'static,
) -> Box<dyn StageRun> {
    Box::new(EdgeRun {
        name,
        job: Some(Box::new(job)),
        paths: None,
    })
}

struct EdgeRun {
    /// The stage's name, for the error of a step after the last.
    name: &'static str,
    /// The work; `None` once the first step has run.
    job: Option<Box<dyn FnOnce() -> Paths + Send>>,
    /// The finished paths, between the first step and the second.
    paths: Option<Paths>,
}

impl StageRun for EdgeRun {
    fn step(&mut self) -> Result<Step, StageError> {
        if let Some(job) = self.job.take() {
            let paths = job();
            let frame = Frame {
                stage: String::new(),
                progress: 1.0,
                label: format!("{} segments", paths.coords().len() - paths.len()),
                points: None,
                paths: Some(paths.clone()),
                pen: 0,
            };
            self.paths = Some(paths);
            return Ok(Step::Frame(frame));
        }
        let Some(paths) = self.paths.take() else {
            return Err(StageError::Failed(format!(
                "the {} run has already finished",
                self.name
            )));
        };
        Ok(Step::Done(Artifact::Paths(Arc::new(paths))))
    }
}

/// Draw the Euclidean minimum spanning tree of the points: the shortest set of segments that joins
/// them all.
///
/// The run reports one frame, labelled with the segment count, then gives the paths.
pub struct MstRenderer;

impl Stage for MstRenderer {
    fn name(&self) -> &'static str {
        "mst"
    }
    fn label(&self) -> &'static str {
        "Spanning tree"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<NoSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<NoSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        validate::<NoSettings>(self.name(), &self.schema(), settings)?;
        let points = inputs.points().clone();
        Ok(start_run(self.name(), move || {
            trail_paths(&points.0, &spanning_tree(&points.0))
        }))
    }
}

/// Draw every Delaunay edge between the points that is no longer than `max_edge_mm`.
///
/// The run reports one frame, labelled with the segment count, then gives the paths.
pub struct DelaunayRenderer;

impl Stage for DelaunayRenderer {
    fn name(&self) -> &'static str {
        "delaunay"
    }
    fn label(&self) -> &'static str {
        "Triangle mesh"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<DelaunaySettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<DelaunaySettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        let s = validate::<DelaunaySettings>(self.name(), &self.schema(), settings)?;
        let points = inputs.points().clone();
        Ok(start_run(self.name(), move || {
            let mut edges = delaunay_edges(&points.0);
            edges.retain(|&e| length(&points.0, e) <= s.max_edge_mm);
            trail_paths(&points.0, &edges)
        }))
    }
}

/// Draw the outlines of the Voronoi cells of the points, clipped to the tone map's rect. The cells
/// on the hull are open, so their outer edges are left out.
///
/// The run reports one frame, labelled with the segment count, then gives the paths.
pub struct CellsRenderer;

impl Stage for CellsRenderer {
    fn name(&self) -> &'static str {
        "cells"
    }
    fn label(&self) -> &'static str {
        "Voronoi cells"
    }
    fn version(&self) -> u32 {
        1
    }
    fn requires(&self) -> &'static [Kind] {
        &[Kind::Points, Kind::Tone]
    }
    fn provides(&self) -> Kind {
        Kind::Paths
    }
    fn schema(&self) -> Value {
        schema_for::<NoSettings>()
    }
    fn defaults(&self) -> Value {
        defaults_for::<NoSettings>()
    }
    fn start(
        &self,
        inputs: Inputs,
        settings: &Value,
        _ctx: RunContext,
    ) -> Result<Box<dyn StageRun>, StageError> {
        validate::<NoSettings>(self.name(), &self.schema(), settings)?;
        let points = inputs.points().clone();
        let rect = inputs.tone().rect_mm();
        Ok(start_run(self.name(), move || {
            voronoi_paths(&points.0, rect)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{assert_inside, bounds, run_stage, tone_map};
    use crate::types::Points;
    use serde_json::json;

    /// The 40 mm square the 20 x 20 px tone map covers, from (10, 10) mm.
    const RECT: [f64; 4] = [10.0, 10.0, 40.0, 40.0];

    /// `n` points spread over [`RECT`] by a fixed linear congruential generator.
    fn scatter(n: usize) -> Vec<[f64; 2]> {
        let mut seed = 2024_u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1_u64 << 53) as f64
        };
        (0..n)
            .map(|_| [10.0 + next() * 40.0, 10.0 + next() * 40.0])
            .collect()
    }

    /// Run `stage` over `points` and a flat tone map covering [`RECT`].
    fn run(stage: &dyn Stage, points: Vec<[f64; 2]>, patch: Value) -> Arc<Paths> {
        let tone = Artifact::Tone(Arc::new(tone_map(20, 20, |_, _| 0.5)));
        let points = Artifact::Points(Arc::new(Points(points, None)));
        match run_stage(stage, vec![points, tone], patch).1 {
            Artifact::Paths(p) => p,
            a => panic!("expected paths, got {}", a.kind().name()),
        }
    }

    /// The number of segments in `paths`.
    fn segments(paths: &Paths) -> usize {
        paths.iter().map(|p| p.len() - 1).sum()
    }

    /// The length of the longest segment in `paths`.
    fn longest(paths: &Paths) -> f64 {
        paths
            .iter()
            .flat_map(|p| p.windows(2))
            .map(|w| math::hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]))
            .fold(0.0, f64::max)
    }

    #[test]
    fn settings_schema() {
        assert_eq!(
            bounds(&DelaunayRenderer.schema(), "max_edge_mm"),
            (0.5, 50.0, 8.0)
        );
        let stages: [&dyn Stage; 3] = [&MstRenderer, &DelaunayRenderer, &CellsRenderer];
        let names = [
            ("mst", "Spanning tree"),
            ("delaunay", "Triangle mesh"),
            ("cells", "Voronoi cells"),
        ];
        for (stage, (name, label)) in stages.into_iter().zip(names) {
            assert_eq!((stage.name(), stage.label()), (name, label));
            assert_eq!((stage.version(), stage.provides()), (1, Kind::Paths));
        }
        assert_eq!(MstRenderer.requires(), &[Kind::Points]);
        assert_eq!(DelaunayRenderer.requires(), &[Kind::Points]);
        assert_eq!(CellsRenderer.requires(), &[Kind::Points, Kind::Tone]);
        assert!(MstRenderer.schema()["properties"].is_null());
        assert!(CellsRenderer.schema()["properties"].is_null());
    }

    #[test]
    fn the_tree_has_n_minus_one_edges() {
        let paths = run(&MstRenderer, scatter(100), json!({}));
        assert_eq!(segments(&paths), 99);
    }

    #[test]
    fn the_tree_is_no_longer_than_the_mesh() {
        let points = scatter(100);
        let tree = run(&MstRenderer, points.clone(), json!({}));
        let mesh = run(&DelaunayRenderer, points, json!({"max_edge_mm": 50}));
        assert!(segments(&mesh) > segments(&tree));
        assert!(tree.total_length() <= mesh.total_length());
    }

    #[test]
    fn the_tree_drops_the_longest_side_of_a_triangle() {
        // A 3-4-5 right triangle: the tree keeps the sides 3 and 4 and drops the hypotenuse. A
        // maximum spanning tree would keep 4 and 5 and measure 9.
        let points = vec![[10.0, 10.0], [13.0, 10.0], [10.0, 14.0]];
        let paths = run(&MstRenderer, points, json!({}));
        assert_eq!(segments(&paths), 2);
        assert!((paths.total_length() - 7.0).abs() < 1e-9);
    }

    #[test]
    fn the_tree_is_as_short_as_prims_over_every_pair() {
        // Prim's algorithm on the complete graph, O(n^2): the Delaunay edges must hold a tree of
        // the same length.
        let points = scatter(100);
        let n = points.len();
        let dist = |i: usize, j: usize| {
            math::hypot(points[i][0] - points[j][0], points[i][1] - points[j][1])
        };
        let mut best: Vec<f64> = (0..n).map(|j| dist(0, j)).collect();
        let mut done = vec![false; n];
        done[0] = true;
        let mut total = 0.0;
        for _ in 1..n {
            let next = (0..n)
                .filter(|&j| !done[j])
                .min_by(|&a, &b| best[a].total_cmp(&best[b]))
                .unwrap();
            total += best[next];
            done[next] = true;
            for (j, b) in best.iter_mut().enumerate() {
                *b = b.min(dist(next, j));
            }
        }
        let tree = run(&MstRenderer, points, json!({}));
        assert!(
            (tree.total_length() - total).abs() < 1e-9,
            "{} {total}",
            tree.total_length()
        );
    }

    #[test]
    fn long_mesh_edges_are_left_out() {
        // Two clusters of five points 30 mm apart.
        let cluster = |x: f64| (0..5).map(move |i| [x + f64::from(i % 3), 20.0 + f64::from(i / 3)]);
        let points: Vec<[f64; 2]> = cluster(12.0).chain(cluster(42.0)).collect();
        let all = run(
            &DelaunayRenderer,
            points.clone(),
            json!({"max_edge_mm": 50}),
        );
        assert!(longest(&all) > 25.0);
        let paths = run(&DelaunayRenderer, points, json!({"max_edge_mm": 8}));
        assert!(segments(&paths) > 0);
        assert!(longest(&paths) <= 8.0 + 1e-9, "{}", longest(&paths));
    }

    #[test]
    fn cells_stay_inside_the_rect() {
        let paths = run(&CellsRenderer, scatter(100), json!({}));
        assert!(!paths.is_empty());
        assert_inside(&paths, RECT);
    }

    #[test]
    fn cells_of_a_square_lattice() {
        // Points at 20, 30 and 40 mm, so the four squares have centres at 25 and 35 mm. The
        // Voronoi edges between neighbouring centres form a ring of four 10 mm sides. The hull's
        // edges have unbounded cells and are left out, and the two triangles of a square share a
        // centre, so their diagonal adds a zero-length edge.
        let lattice: Vec<[f64; 2]> = (0..9)
            .map(|i| {
                [
                    20.0 + 10.0 * f64::from(i % 3),
                    20.0 + 10.0 * f64::from(i / 3),
                ]
            })
            .collect();
        let paths = run(&CellsRenderer, lattice, json!({}));
        assert!((paths.total_length() - 40.0).abs() < 1e-6, "{paths:?}");
        assert_inside(&paths, RECT);
    }

    #[test]
    fn cells_are_cut_where_they_leave_the_rect() {
        // The flat triangle (10, 10), (50, 10), (30, 11) has its circumcentre at y = -189.5, far
        // above the rect on the page, so the two Voronoi edges that reach it are cut at y = 10.
        let points = vec![[10.0, 10.0], [50.0, 10.0], [30.0, 11.0], [30.0, 49.0]];
        let paths = run(&CellsRenderer, points, json!({}));
        assert_inside(&paths, RECT);
        let cut = paths.coords().iter().filter(|p| (p[1] - 10.0).abs() < 1e-9);
        assert_eq!(cut.count(), 2);
    }

    #[test]
    fn too_few_or_collinear_points_give_no_cells_and_no_panic() {
        assert!(run(&CellsRenderer, vec![], json!({})).is_empty());
        assert!(run(&CellsRenderer, vec![[20.0, 20.0]], json!({})).is_empty());
        let line = vec![[20.0, 20.0], [25.0, 20.0], [30.0, 20.0], [35.0, 20.0]];
        assert!(run(&CellsRenderer, line.clone(), json!({})).is_empty());
        assert!(run(&MstRenderer, vec![], json!({})).is_empty());
        assert_eq!(
            run(&MstRenderer, line.clone(), json!({})).total_length(),
            15.0
        );
        assert_eq!(segments(&run(&DelaunayRenderer, line, json!({}))), 3);
    }

    #[test]
    fn one_frame_then_the_paths() {
        let stages: [&dyn Stage; 3] = [&MstRenderer, &DelaunayRenderer, &CellsRenderer];
        for stage in stages {
            let tone = Artifact::Tone(Arc::new(tone_map(20, 20, |_, _| 0.5)));
            let points = Artifact::Points(Arc::new(Points(scatter(30), None)));
            let (frames, paths) = run_stage(stage, vec![points, tone], json!({}));
            assert_eq!(frames.len(), 1, "{}", stage.name());
            let Artifact::Paths(p) = paths else {
                panic!("expected paths")
            };
            assert_eq!(frames[0].progress, 1.0);
            assert_eq!(frames[0].label, format!("{} segments", segments(&p)));
            assert_eq!(frames[0].paths.as_ref().unwrap().coords(), p.coords());
        }
    }

    #[test]
    fn bad_settings_are_rejected_at_start() {
        let cases: [(&dyn Stage, Value); 4] = [
            (&DelaunayRenderer, json!({"max_edge_mm": 0.4})),
            (&DelaunayRenderer, json!({"max_edge_mm": 51})),
            (&DelaunayRenderer, json!({"nope": 1})),
            (&MstRenderer, json!({"nope": 1})),
        ];
        for (stage, patch) in cases {
            let settings = crate::settings::merge(&stage.defaults(), &patch);
            let points = Artifact::Points(Arc::new(Points(vec![], None)));
            let inputs = Inputs::new([points].into_iter().map(|a| (a.kind(), a)).collect());
            let ctx = RunContext::new(Arc::default());
            let err = stage.start(inputs, &settings, ctx).err().unwrap();
            assert!(matches!(err, StageError::Settings(_)), "{err:?}");
        }
    }

    #[test]
    fn a_finished_run_fails_instead_of_panicking() {
        let stages: [&dyn Stage; 3] = [&MstRenderer, &DelaunayRenderer, &CellsRenderer];
        for stage in stages {
            let tone = Artifact::Tone(Arc::new(tone_map(1, 1, |_, _| 1.0)));
            let points = Artifact::Points(Arc::new(Points(vec![], None)));
            let inputs = Inputs::new([points, tone].into_iter().map(|a| (a.kind(), a)).collect());
            let ctx = RunContext::new(Arc::default());
            let mut run = stage.start(inputs, &stage.defaults(), ctx).unwrap();
            assert!(matches!(run.step(), Ok(Step::Frame(_))));
            assert!(matches!(run.step(), Ok(Step::Done(_))));
            assert!(matches!(run.step(), Err(StageError::Failed(_))));
        }
    }
}
