# Point family and more styles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 13 stages and 13 styles to the engine: the Voronoi family and Sketch Lines first, then more point placers and graph renderers, then styles that need only the tone map.

**Architecture:** Every new piece is a `Stage` in `crates/core/src/stages/`, written like `grid.rs`, `hatch.rs` and `waves.rs`. Each is registered as a style in `crates/core/src/registry.rs`. The GUI, CLI, MCP, presets and exports pick the styles up from the registry and the stage schemas, with no front-end change. The four Delaunay-based renderers share `stages/graph.rs`, which wraps the `delaunator` crate.

**Tech Stack:** Rust (stable, pinned), serde and schemars settings, `rand` 0.9 with `rand_pcg`, `libm` through `crate::math`, `delaunator` 1.1 (new), wasm-pack, Python tests through `uv run pytest`.

**Spec:** `docs/superpowers/specs/2026-10-12-point-family-and-more-styles-design.md`. Read it with this plan: the setting tables (ranges, defaults and help text) are there and are not repeated here.

## Global Constraints

- Settings are a struct deriving `Serialize, Deserialize, JsonSchema` with `#[serde(default, deny_unknown_fields)]`. The help text is a one-line doc comment, and ranges and steps go in `#[schemars(range(...), extend("x-step" = ..., "x-unit" = ...))]`, exactly as `HatchSettings` does it.
- Every value in a setting's range, default and help text is copied verbatim from the spec's table for that stage.
- Seeds: `seed: u32`, range 0 to 9999, default 1. The help text is in the spec. Randomness comes only from `ctx.rng(u64::from(s.seed))`.
- No platform transcendental methods outside `math.rs`: use `math::sin`, `math::cos`, `math::atan2`, `math::hypot`, `math::exp` and `math::ln`. `f64::sqrt` and `abs` are allowed. The source-scan test in `math.rs` enforces this.
- Ties go to the lowest index. Sums run in a fixed order. There is no `HashMap` iteration in any output path.
- Every vertex lies inside `tone.rect_mm()`: clamp with `lines::clamp_to` or clip with `graph::clip_polyline`.
- Each module docstring cites the sources the spec names for that stage. The three citations the spec marks "to be checked" are checked against the paper or the patent record before they are written. If one cannot be checked, the docstring says so.
- Stage names are unique per stage type (spec §2).
- No new dependency except `delaunator = "1.1"`. Its only dependency, `robust`, is MIT or Apache-2.0.
- Commit messages are plain English sentences like the existing history and end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Verification set before every commit: `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked -p plotline-core`. When the style list changes, also run `uv run pytest -q`.

## Review Focus

1. **A white image, or a style over a white area.** `voronoi` and `poisson` must not divide by a zero total density, and `sketch` and `dither` must end at once. Pinned by the registry test `every_style_survives_tiny_white_and_black_images` (Task 2), which covers each style as it is registered.
2. **A 1 by 1 pixel image, or a tone rect smaller than one spacing.** `flow`, `hilbert`, `poisson` and `contours` must not loop forever or panic. Pinned by the same registry test.
3. **Fewer than three points, collinear points or duplicate points fed to the Delaunay renderers.** They must still draw what they can. Pinned by `delaunay_edges_handles_degenerate_input` (Task 4) and `tsp_visits_duplicate_points` (Task 5).
4. **A large map at a high count.** Stop must stay responsive, so one relaxation iteration spans several steps. Pinned by `an_iteration_on_a_large_map_spans_several_steps` (Task 1).
5. **A flat black image.** It gives the most marks: the most Hilbert leaves, the densest Poisson packing and every flow line at its minimum spacing. It must finish, and stay on the page. Pinned by the registry test in Task 2 (black 64 by 48).

---

### Task 1: Weighted Voronoi placer

**Files:**
- Create: `crates/core/src/stages/voronoi.rs`
- Modify: `crates/core/src/stages/mod.rs` (add `pub mod voronoi;`)
- Modify: `crates/core/src/test_util.rs` (add `run_stage`)

**Interfaces:**
- Produces:
  - `pub struct VoronoiPlacer;`: name `"voronoi"`, label `"Weighted Voronoi"`, version 1, requires `[Kind::Tone]`, provides `Kind::Points`.
  - `pub struct VoronoiSettings { count: u32, iterations: u32, exponent: f64, seed: u32 }`.
  - `pub(crate) fn sample_by_density(tone: &ToneMap, exponent: f64, n: usize, rng: &mut Pcg64) -> Vec<[f64; 2]>`: `n` points drawn by inverse CDF over the pixels' `ink ^ exponent` in row-major order, each jittered uniformly inside its pixel, in mm. It returns no points when the total density is 0.
  - In `test_util.rs`: `pub(crate) fn run_stage(stage: &dyn Stage, inputs: Vec<Artifact>, patch: Value) -> (Vec<Frame>, Artifact)`. It merges `patch` over the stage's defaults, files each input under its own kind, and steps the run to the end. It panics on an error.

- [ ] **Step 1: Write the failing tests** in `voronoi.rs`'s test module, using `tone_map` and `run_stage`:

```rust
#[test] fn settings_schema() // bounds(&s,"count")==(100.0,50000.0,5000.0); iterations (0,200,30); exponent (0.5,3.0,1.0); seed (0,9999,1); name/label
#[test] fn points_are_denser_where_the_ink_is() // 40x20 map, ink 1.0 for x<20 else 0.1, exponent 1, count 400: dark half holds > 3x the points of the light half
#[test] fn no_point_sits_on_white() // 20x20 map, ink 1 inside x<10 else 0: every point's x <= 10*2+10 + 1e-9 (in mm)
#[test] fn white_gives_no_points() // all-zero map: points empty, last frame progress 1.0
#[test] fn same_seed_same_points_other_seed_other_points()
#[test] fn an_iteration_on_a_large_map_spans_several_steps() // 1000x1000 map of 1.0, count 100, iterations 2: frames.len() >= 10
#[test] fn bad_settings_are_rejected_at_start() // count 50, exponent 0.1, unknown key → StageError::Settings
```

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::voronoi`
Expected: compile error, `VoronoiPlacer` not found.

- [ ] **Step 3: Implement `VoronoiPlacer`**
  - `start` validates the settings, then draws `count` points with `sample_by_density`.
  - The run is a state machine. The first step returns a frame labelled `scatter` holding the initial points. Then come relaxation steps, each covering up to `PIXELS_PER_STEP = 200_000` pixels, row-aligned.
  - At an iteration's first row, build `GridIndex::new(&points, cell)` with `cell = sqrt(w_mm * h_mm / count).max(mm_per_px)`.
  - For each pixel centre with density `d > 0`, find `i = index.nearest(centre, &alive)` and add `d * centre` and `d` to accumulators `acc[i]` and `mass[i]`, both `f64`.
  - After the last row, move each point with `mass > 0` to `acc / mass`.
  - Each frame's label is `relaxation k/N` and its progress is the share of total pixel work done.
  - After the final iteration, drop points whose `mass == 0` in the last pass. With `iterations == 0` nothing is dropped.
  - Then return `Step::Done(Artifact::Points(..))`.
  - A zero total density gives one frame, `no ink`, and empty points.

- [ ] **Step 4: Run the tests and check they pass**

Run: `cargo test -p plotline-core --lib stages::voronoi`
Expected: 7 passed.

- [ ] **Step 5: Run the verification set from Global Constraints, then commit**

```bash
git add crates/core/src/stages/voronoi.rs crates/core/src/stages/mod.rs crates/core/src/test_util.rs
git commit -m "Add the weighted Voronoi placer"
```

---

### Task 2: Stipple renderer, the stipple and halftone styles, and the registry guards

**Files:**
- Create: `crates/core/src/stages/stipple.rs`
- Modify: `crates/core/src/stages/mod.rs`, `crates/core/src/registry.rs`
- Modify: `crates/core/tests/settings_matter.rs`
- Modify: `tests/test_core_binding.py:34`, `tests/test_session.py:39`

**Interfaces:**
- Consumes: `VoronoiPlacer` (Task 1), `GridPlacer` (`stages::grid`, stage name `points`), `ToneStage`, `OrderStage`.
- Produces:
  - `pub struct StippleRenderer;`: name `"stipple"`, label `"Stipple dots"`, version 1, requires `[Kind::Points, Kind::Tone, Kind::Pens]`, provides `Kind::Paths`.
  - `pub enum DotShape { Auto, Point, Circle, Spiral }`, serialized lowercase like `InkMode`.
  - `pub struct StippleSettings { size_mm: f64, vary: f64, shape: DotShape }`.
  - `pub(crate) fn dot_path(centre: [f64; 2], d: f64, pen_w: f64, shape: DotShape) -> Vec<[f64; 2]>`.
  - In `registry.rs`: `pub fn stipple_style()` and `pub fn halftone_style()`, and a private helper `fn point_style(name, label, description, placer: Arc<dyn Stage>, renderer: Arc<dyn Stage>) -> Style` that builds tone, placer, renderer and order. Later tasks reuse it.

- [ ] **Step 1: Write the failing tests**

In `stipple.rs`:

```rust
#[test] fn settings_schema() // size_mm (0.1,10,0.8); vary (0,1,0.5); shape default "auto"
#[test] fn a_dot_no_wider_than_the_pen_is_one_vertex() // dot_path(c, 0.3, 0.3, Auto).len()==1
#[test] fn a_wide_dot_spirals_out_to_its_rim() // dot_path([0,0], 2.0, 0.3, Auto): first vertex == centre; max distance from centre == (2.0-0.3)/2 within 1e-9; consecutive vertices <= 0.3/4 + 1e-9 apart
#[test] fn circle_is_the_rim_alone() // dot_path([0,0],2.0,0.3,Circle): every vertex at radius 0.85 ± 1e-9, last == first
#[test] fn diameter_follows_the_square_root_of_ink() // full-ink point, vary 1, size 2, pen 0.3 → spiral rim radius 0.85; ink 0.25 → d = 1.0, rim radius 0.35
```

In `registry.rs`'s test module:

```rust
#[test] fn stages_sharing_a_name_share_label_and_schema() // over every built-in style (hidden included): group stages by name; within a group, label() and schema() are equal
#[test] fn every_style_survives_tiny_white_and_black_images() // for each listed style: run_to_drawing on flat(1,1,0.0), flat(64,48,1.0), flat(64,48,0.0); every drawing's paths lie inside its canvas drawing_rect (assert_inside)
```

Update `lists_builtins_and_hides_test_styles` to expect `["grid", "hatch", "waves", "spiral", "ink", "stipple", "halftone"]`. Update both Python tests to the same list.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stipple registry`
Expected: compile error, `StippleRenderer` not found.

- [ ] **Step 3: Implement `StippleRenderer` and `dot_path`**
  - The diameter is `d = size_mm * sqrt(1 - vary + vary * ink)` at each point, with `ink = tone.sample(p)`. The pen width is the first pen's `width_mm`.
  - **Auto** is `Point` when `d <= pen_w`, else `Spiral`.
  - **Spiral:** `r = pen_w * theta / (2π)` from the centre to `R = (d - pen_w) / 2`, then one full circle at `R`, sampled no more than `pen_w / 4` apart along the arc. `Spiral` with `R <= 0` is one vertex.
  - **Circle:** the circle at `R`, closed.
  - Every vertex is clamped to the tone rect.
  - The run gives one frame, labelled `N dots`, then the paths, like `DotRun`.

- [ ] **Step 4: Register the styles** with `point_style`, using the spec's §1 labels and looks:
  - `stipple`: tone, `VoronoiPlacer`, `StippleRenderer`, order.
  - `halftone`: tone, `GridPlacer`, `StippleRenderer`, order, then `.with_defaults("points", json!({"spacing_mm": 2.0, "threshold": 0.02})).with_defaults("stipple", json!({"size_mm": 2.0, "vary": 1.0}))`.
  - Add both to `builtins()` in the list order above.

- [ ] **Step 5: Make `settings_matter.rs` share one session across draws**
  - Its run count grows with every Voronoi style, so `draw` takes `&mut Session`.
  - The test builds one `Session::new(512 << 20)` with the noise image and canvas set once.
  - `draw` calls `set_style`, which resets the settings, then `update`, then runs.
  - The cache then reuses unchanged upstream stages. What each draw compares is unchanged.

- [ ] **Step 6: Run the tests and check they pass**

Run: `cargo test -p plotline-core`
Expected: all pass. If `settings_matter` names a stipple or halftone setting as changing nothing, add it to `NO_OPS` with the reason, after confirming the setting does act on a larger image.

Run: `uv run pytest -q`
Expected: all pass.

- [ ] **Step 7: Run the verification set, then commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the stipple renderer and the stipple and halftone styles"
```

---

### Task 3: Stipple golden file, native and in WebAssembly

**Files:**
- Modify: `crates/core/tests/golden.rs`
- Create: `crates/core/golden/stipple_gradient.json` (generated)
- Modify: `crates/wasm/tests/smoke.rs`

**Interfaces:**
- Consumes: the `stipple` style (Task 2).
- Produces: `produce(style: &str, image: Image, patch: Value) -> Value` in `golden.rs`. `patch` is `{stage: {key: value}}` merged over `style.defaults()`. The two existing grid cases pass their current settings through it.

- [ ] **Step 1: Generalise `produce`, and add the case** `("stipple_gradient", "stipple", gradient(64, 48), json!({"tone": {"resolution": 64}, "voronoi": {"count": 300, "iterations": 10}}))`. Make the module docstring say "the grid and stipple styles".

- [ ] **Step 2: Generate the file and review it**

Run: `PLOTLINE_UPDATE_GOLDEN=1 cargo test -p plotline-core --test golden`, then `git diff --stat crates/core/golden`.
Expected: only `stipple_gradient.json` is new. The grid files are unchanged. The new file holds one layer of between 250 and 300 one-vertex or spiral paths.

- [ ] **Step 3: Add `stipple_matches_the_golden_file` to `smoke.rs`**
  - It uses the same session calls as the grid test: canvas `{"width_mm":100,"height_mm":75,"margin_mm":5,"fit":"fit"}`, `update("tone", r#"{"resolution":64}"#)` and `update("voronoi", r#"{"count":300,"iterations":10}"#)`, after `set_style("stipple")`.
  - It asserts an equal path count, and every coordinate within 1e-3 of `include_str!("../../core/golden/stipple_gradient.json")`.

- [ ] **Step 4: Run both**

Run: `cargo test -p plotline-core --test golden` and `cd crates/wasm && wasm-pack test --node`
Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core/tests/golden.rs crates/core/golden/stipple_gradient.json crates/wasm/tests/smoke.rs
git commit -m "Pin the stipple style in a golden file checked natively and in WebAssembly"
```

---

### Task 4: Graph helpers on delaunator

**Files:**
- Modify: `crates/core/Cargo.toml` (add `delaunator = "1.1"`), `Cargo.lock`
- Create: `crates/core/src/stages/graph.rs`
- Modify: `crates/core/src/stages/mod.rs`

**Interfaces:**
- Produces:
  - `pub(crate) fn triangulate(points: &[[f64; 2]]) -> delaunator::Triangulation`.
  - `pub(crate) fn delaunay_edges(points: &[[f64; 2]]) -> Vec<(usize, usize)>`: each edge once as `(a, b)` with `a < b`, sorted. When the triangulation has no triangles (fewer than three distinct points, or all collinear), it falls back to chaining the distinct points sorted by `(x, y, index)`. Duplicates of a point appear in no edge.
  - `pub(crate) fn trails(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>>`: the greedy walk of spec §4.3.
  - `pub(crate) fn clip_polyline(rect: [f64; 4], line: &[[f64; 2]]) -> Vec<Vec<[f64; 2]>>`: each segment is clipped by Liang–Barsky, consecutive in-rect pieces are joined, and the line is split where it leaves the rect. Pieces of one vertex are dropped.

- [ ] **Step 1: Add the dependency and check the licences**

Run: `cargo add -p plotline-core delaunator@1.1` then `cargo deny check licenses`. If cargo-deny is not installed, read the licence fields with `cargo metadata --format-version 1`.
Expected: delaunator is ISC and robust is MIT OR Apache-2.0. Both pass.

- [ ] **Step 2: Write the failing tests** in `graph.rs`

```rust
#[test] fn square_gives_five_edges() // 4 corners of a unit square → 5 edges (4 sides + 1 diagonal)
#[test] fn delaunay_edges_handles_degenerate_input() // [] → []; 1 point → []; 2 points → [(0,1)]; 4 collinear points given out of order → 3 edges chaining them by x; [p, p, q] (duplicate) → one edge, from an index of p to q
#[test] fn a_path_graph_is_one_trail() // n=4, edges (0,1),(1,2),(2,3) → [[0,1,2,3]]
#[test] fn a_four_armed_star_is_two_trails() // centre 0, arms 1..=4 → 2 trails, every edge used exactly once
#[test] fn a_cycle_is_one_closed_trail() // triangle → one trail of 4 vertices, first == last
#[test] fn clip_polyline_splits_where_the_line_leaves() // rect [0,0,10,10], line [-5,5]→[5,5]→[5,15]→[5,20]→[8,5]: pieces [[0,5],[5,5],[5,10]] and [[7,10],[8,5]] within 1e-9
```

- [ ] **Step 3: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::graph`
Expected: compile error.

- [ ] **Step 4: Implement the four functions**

`trails` builds sorted adjacency lists with a used flag per edge.
- **Start vertex:** the lowest-index vertex of odd remaining degree, or else the lowest with any remaining edge.
- **Walk:** follow the lowest-numbered unused neighbour until stuck.
- **Repeat** until every edge is used.

The Liang–Barsky citation is the one in `lines.rs`'s docstring.

- [ ] **Step 5: Run the tests and check they pass, then run the verification set and the wasm size**

Run: `cargo test -p plotline-core --lib stages::graph`, then the verification set.
Expected: pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/core/Cargo.toml crates/core/src/stages/graph.rs crates/core/src/stages/mod.rs
git commit -m "Add Delaunay edges, trail splitting and polyline clipping for the graph renderers"
```

---

### Task 5: Single-line (TSP) renderer and the `line` style

**Files:**
- Create: `crates/core/src/stages/tsp.rs`
- Modify: `crates/core/src/stages/mod.rs`, `crates/core/src/registry.rs`, `tests/test_core_binding.py`, `tests/test_session.py`

**Interfaces:**
- Consumes: `delaunay_edges` (Task 4), `GridIndex::new` and `GridIndex::nearest`, and `point_style` (Task 2).
- Produces:
  - `pub struct TspRenderer;`: name `"tsp"`, label `"Single line"`, version 1, requires `[Kind::Points]`, provides `Kind::Paths`.
  - `pub struct TspSettings { optimize: bool, closed: bool }`.
  - `pub fn line_style()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema() // optimize default true, closed default false
#[test] fn one_path_through_every_point() // 200 seeded random points in 100x100: 1 path of 200 vertices; the set of vertices equals the input set
#[test] fn no_two_segments_cross() // same 200 points: for every pair of non-adjacent segments, a proper-intersection test (orientation signs) finds none
#[test] fn closed_ends_where_it_starts() // closed true: 201 vertices, last == first
#[test] fn optimize_shortens_the_line() // optimize false vs true on the 200 points: optimised total_length < nearest-neighbour total_length
#[test] fn tsp_visits_duplicate_points() // [[0,0],[0,0],[1,0],[2,0]] → 4 vertices
#[test] fn few_points() // 0 points → no paths; 1 point → one 1-vertex path; 2 points → one 2-vertex path
```

Registry list after this task: `[..5 existing.., "stipple", "line", "halftone"]`, and the same in both Python tests.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::tsp`
Expected: compile error.

- [ ] **Step 3: Implement `TspRenderer`**
  - **Construction:** nearest-neighbour from the point nearest the top-left of the points' bounds, marking each visited point dead in `alive`. Frame `construction`.
  - **Improvement** works on `tour: Vec<usize>` and `pos: Vec<usize>`. The candidates are each point's Delaunay neighbours sorted by distance. A queue of active points holds don't-look bits, with all points active in index order.
  - **2-opt:** for active `a` and both tour neighbours `b` of `a`, and each candidate `c` with `d(a,c) < d(a,b)`, take the matching neighbour `e` of `c`. If `gain = d(a,b) + d(c,e) - d(a,c) - d(b,e) > 1e-12`, reverse the shorter side and re-activate `a`, `b`, `c` and `e`.
  - **Or-opt:** try moving the segment of 1 to 3 points starting at `a` between a candidate `c` and its successor, in either orientation, with the same gain rule.
  - **Stop** when the queue is empty. A frame `improving: k moves` comes every 2000 points taken from the queue.
  - **Ending:** if not `closed`, open the cycle at its longest edge, with ties to the lowest tour position. If `closed`, repeat the first vertex.
  - With `optimize` off, skip improvement and treat the nearest-neighbour order as the line.

- [ ] **Step 4: Register `line`** (tone, `VoronoiPlacer`, `TspRenderer`, order) between `stipple` and `halftone`.

- [ ] **Step 5: Run the tests, `cargo test -p plotline-core`, `uv run pytest -q` and the verification set**

Expected: all pass. Any `NO_OPS` entry needs a reason.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the single-line renderer and the line style"
```

---

### Task 6: Dash renderer and the `dashes` style

**Files:**
- Create: `crates/core/src/stages/dashes.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Produces:
  - `pub struct DashRenderer;`: name `"dashes"`, label `"Dashes"`, version 1, requires `[Kind::Points, Kind::Tone]`, provides `Kind::Paths`.
  - `pub struct DashSettings { length_mm: f64, angle_deg: f64, follow_edges: bool }`.
  - `pub fn dashes_style()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema() // length_mm (0.2,10,1.5); angle_deg (0,180,0); follow_edges true
#[test] fn dashes_lie_along_a_vertical_edge() // 20x20 map, ink 1 for x<10 else 0; a point at the edge (x=30 mm, y=30 mm): dash is vertical (|dx| < 1e-6), length 1.5
#[test] fn flat_ink_uses_the_angle() // flat 0.5 map, angle 90: dash vertical
#[test] fn fixed_angle_ignores_edges() // edge map, follow_edges false, angle 0: dash horizontal
#[test] fn dashes_stay_on_the_map() // a point at the map's corner: both ends inside rect
```

Registry list: `[.., "stipple", "line", "dashes", "halftone"]`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::dashes`
Expected: compile error.

- [ ] **Step 3: Implement it as spec §3.4 says**
  - The gradient is central differences with `tone.sample` at `±length_mm / 2` along x and along y.
  - The direction angle is `math::atan2(gy, gx) + π/2 + angle_deg·π/180`.
  - The ends are clamped.
  - The run gives one frame, then the paths.

- [ ] **Step 4: Register `dashes`** (tone, voronoi, dashes, order). Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the dash renderer and the dashes style"
```

---

### Task 7: Sketch Lines

**Files:**
- Create: `crates/core/src/stages/sketch.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Produces:
  - `pub struct SketchStage;`: name `"sketch"`, label `"Sketch lines"`, version 1, requires `[Kind::Tone]`, provides `Kind::Paths`.
  - `pub struct SketchSettings { length_mm, candidates: u32, lighten, stop, max_lines: u32, seed: u32 }`.
  - `pub fn sketch_style()`, built with `line_style`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema() // all six bounds from spec §3.5
#[test] fn white_gives_no_strokes()
#[test] fn strokes_cover_a_dark_bar() // 40x40 map, ink 1 for 18<=y<22 else 0: every vertex has y within the bar's mm band ± length_mm; at least one path
#[test] fn max_lines_caps_the_segments() // flat black, max_lines 100: total segments (sum of len-1) == 100
#[test] fn strokes_stay_on_the_map()
```

Registry list: `[.., "dashes", "sketch", "halftone"]`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::sketch`
Expected: compile error.

- [ ] **Step 3: Implement it as spec §3.5 says, as amended**
  - The residual is `Vec<f32>`, a copy of the tone map, plus a 16 by 16 block-max array.
  - A candidate's end is `clamp_to(rect, ...)`. A candidate shorter than one pixel after clamping is skipped.
  - The score is the mean of the residual at the nearest pixel, sampled every half pixel. Lightening stamps pixels with the segment number so each is lowered once.
  - Recompute the maxima of the touched blocks after each segment.
  - Each step draws up to `max(max_lines / 30, 1)` segments and returns a frame of the strokes so far.

- [ ] **Step 4: Register `sketch`. Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.**

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add Sketch Lines"
```

---

### Task 8: Error-diffusion placer and the `dither` style

**Files:**
- Create: `crates/core/src/stages/dither.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Produces:
  - `pub struct DitherPlacer;`: name `"dither"`, label `"Error diffusion"`, version 1, requires `[Kind::Tone]`, provides `Kind::Points`.
  - `pub struct DitherSettings { spacing_mm: f64, serpentine: bool }`.
  - `pub fn dither_style()`: tone, `DitherPlacer`, `DotRenderer`, order.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema()
#[test] fn half_ink_dots_about_half_the_cells() // flat 0.5 on 20x20 px at 2 mm/px, spacing 1: |points - 800| <= 16
#[test] fn white_and_black() // all 0 → no points; all 1 → every cell
#[test] fn serpentine_changes_the_pattern() // flat 0.3: point sets differ between serpentine on and off
```

Registry list: `[.., "sketch", "halftone", "dither"]`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::dither`
Expected: compile error.

- [ ] **Step 3: Implement it** with the grid geometry of `GridPlacer` (cell centres at `origin + spacing * (i + 0.5)`).
  - Floyd–Steinberg weights are 7/16, 3/16, 5/16 and 1/16. The error is carried in `f64` in two row buffers.
  - With serpentine on, odd rows run right to left with the weights mirrored.
  - Frames come row by row, like the grid placer (about 50).

- [ ] **Step 4: Register `dither`. Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.**

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the error-diffusion placer and the dither style"
```

---

### Task 9: Poisson-disk placer and the `circles` style

**Files:**
- Create: `crates/core/src/stages/poisson.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Consumes: `voronoi::sample_by_density` (Task 1), with exponent 1, for darts.
- Produces:
  - `pub struct PoissonPlacer;`: name `"poisson"`, label `"Poisson disk"`, version 1, requires `[Kind::Tone]`, provides `Kind::Points`.
  - `pub struct PoissonSettings { spacing_mm: f64, seed: u32 }`.
  - `pub fn circles_style()`: tone, poisson, stipple, order, `.with_defaults("poisson", json!({"spacing_mm": 3.0})).with_defaults("stipple", json!({"size_mm": 3.0, "vary": 1.0, "shape": "circle"}))`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema()
#[test] fn no_two_points_closer_than_their_radii() // noise-like map (x*7+y*13)%10/9: for every pair, dist >= min(r(p), r(q)) - 1e-9, with r = spacing/sqrt(ink) capped at 20*spacing
#[test] fn black_packs_at_the_spacing() // flat 1, spacing 2, 20x20 px at 2 mm/px (40 mm square): count between 200 and 462 (the hexagonal packing bound 2·1600/(√3·4))
#[test] fn a_small_dark_spot_on_white_gets_points() // 50x50 map, ink 1 only in a 2x2 px spot, spacing 1: points not empty, all within the spot ± 1 mm
#[test] fn white_gives_no_points()
```

Registry list: `[.., "halftone", "dither", "circles"]`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::poisson`
Expected: compile error.

- [ ] **Step 3: Implement it as spec §4.2 says, as amended**
  - **Acceptance:** a candidate `q` is accepted if `ink(q) >= 1/400` and no stored point lies within `r(q)`. The bucket grid has cells of `spacing_mm`, searched `ceil(r(q) / spacing_mm)` rings out.
  - **Active list:** pick a random active point with `rng.random_range`. Remove it with `swap_remove` after 30 failed candidates.
  - **Frames** come every `max(expected / 30, 1)` accepted points, where `expected = rect area / spacing²`.

- [ ] **Step 4: Register `circles`. Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.**

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the Poisson-disk placer and the circles style"
```

---

### Task 10: Spanning tree, triangle mesh and Voronoi cell renderers

**Files:**
- Create: `crates/core/src/stages/edges.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Consumes: `triangulate`, `delaunay_edges`, `trails` and `clip_polyline` (Task 4).
- Produces:
  - `MstRenderer`: name `"mst"`, label `"Spanning tree"`, no settings.
  - `DelaunayRenderer`: name `"delaunay"`, label `"Triangle mesh"`, `DelaunaySettings { max_edge_mm }`.
  - `CellsRenderer`: name `"cells"`, label `"Voronoi cells"`, no settings.
  - All three are version 1. `mst` and `delaunay` require `[Kind::Points]`. `cells` requires `[Kind::Points, Kind::Tone]`, for the rect. All provide `Kind::Paths`.
  - `pub fn tree_style()`, `mesh_style()` and `cells_style()`, each tone, voronoi, renderer, order.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn the_tree_has_n_minus_one_edges() // 100 seeded random points: sum of (path len - 1) == 99
#[test] fn the_tree_is_no_longer_than_the_mesh() // total_length(mst) <= total_length(delaunay with max_edge 50)
#[test] fn long_mesh_edges_are_left_out() // two clusters 30 mm apart, max_edge 8: no segment longer than 8 + 1e-9
#[test] fn cells_stay_inside_the_rect() // 100 random points over a 40x40 mm map: assert_inside
#[test] fn cells_of_a_square_lattice() // 3x3 lattice 10 mm apart inside a 40 mm rect: total path length == 40 within 1e-6 (four 10 mm edges joining the four squares' centres; the diagonals give zero-length edges, and the hull edges are left out)
```

Registry list: `[.., "sketch", "tree", "mesh", "cells", "halftone", "dither", "circles"]`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::edges`
Expected: compile error.

- [ ] **Step 3: Implement the three renderers as spec §4.3 says**
  - **mst:** Kruskal over `delaunay_edges`, sorted by `(length, a, b)`, with union-find.
  - **delaunay:** filters edges by `max_edge_mm`.
  - **cells:** one circumcentre per triangle, with an edge between `t = e / 3` and `halfedges[e] / 3` for each `e < halfedges[e]` that is not `EMPTY`. Trails run over triangle indices, map to circumcentres, then go through `clip_polyline(tone.rect_mm(), ..)`.
  - Each renderer gives one frame, then the paths.

- [ ] **Step 4: Register the three styles. Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.**

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the spanning tree, triangle mesh and Voronoi cell renderers"
```

---

### Task 11: Contours

**Files:**
- Create: `crates/core/src/stages/contours.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Produces:
  - `pub struct ContourStage;`: name `"contours"`, label `"Contours"`, version 1, requires `[Kind::Tone]`, provides `Kind::Paths`.
  - `pub struct ContourSettings { levels: u32, smooth_mm: f64 }`.
  - `pub fn contours_style()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema()
#[test] fn a_radial_gradient_gives_nested_closed_loops() // 40x40 map, ink = max(0, 1 - r/20 px) from the centre, levels 4, smooth 0: 4 paths, each closed (first == last), radii decreasing with level
#[test] fn white_gives_nothing()
#[test] fn every_vertex_stays_on_the_map() // flat black: lines run along the padded border and are clamped inside
```

Registry list: append `"contours"`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::contours`
Expected: compile error.

- [ ] **Step 3: Implement it as spec §5.1 says**
  - Blur with `math::blur(&ink, w, h, smooth_mm / mm_per_px)`, then pad by one zero cell.
  - Call `raster::iso_lines(&padded, w + 2, h + 2, 1/(L+1), 1/(L+1), L/(L+1))`, then `raster::simplify(line, 0.5)`.
  - Padded cell `(cx, cy)` maps to `tone.to_mm([cx - 0.5, cy - 0.5])`, then `clamp_to`, as `ink.rs` does for its padded grid.
  - The work is a `ChunkRun` over the lines found.

- [ ] **Step 4: Register `contours`. Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.**

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the contours style"
```

---

### Task 12: Flow lines

**Files:**
- Create: `crates/core/src/stages/flow.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Produces:
  - `pub struct FlowStage;`: name `"flow"`, label `"Flow lines"`, version 1, requires `[Kind::Tone]`, provides `Kind::Paths`.
  - `pub struct FlowSettings { min_spacing_mm, max_spacing_mm, smooth_mm, angle_deg }`.
  - `pub fn flow_style()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema()
#[test] fn flat_image_gives_parallel_lines_at_the_angle() // flat 0.5, angle 0: every path's vertices share one y within 1e-6
#[test] fn lines_keep_their_distance() // noise-like map: any two samples of different paths are at least min_spacing/2 - 1e-9 apart
#[test] fn darker_areas_get_closer_lines() // left half ink 1, right half 0: total length on the left > 2x the right
#[test] fn every_vertex_stays_on_the_map()
```

Registry list: append `"flow"`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::flow`
Expected: compile error.

- [ ] **Step 3: Implement it as spec §5.2 says**
  - Blur the ink into a field. The direction at a point bilinearly samples central-difference gradients of the blurred field.
  - **Samples** are stored in a bucket grid with cells of `min_spacing`, each as `(point, line id, sequence)`. Backward samples take negative sequence numbers.
  - **Collisions:** a sample collides with a stored one within `d(p) / 2`, unless that sample is on the same line with `|seq difference| <= ceil(2 * max_spacing / step)`.
  - **Seed check:** a seed is valid if no sample lies within `d(seed)`.
  - **Coarse fallback:** the grid scan resumes where it left off.
  - Each step traces up to 50 lines and returns a frame.

- [ ] **Step 4: Register `flow`. Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.**

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the flow lines style"
```

---

### Task 13: Adaptive Hilbert curve

**Files:**
- Create: `crates/core/src/stages/hilbert.rs`
- Modify: `stages/mod.rs`, `registry.rs`, both Python tests

**Interfaces:**
- Produces:
  - `pub struct HilbertStage;`: name `"hilbert"`, label `"Hilbert curve"`, version 1, requires `[Kind::Tone]`, provides `Kind::Paths`.
  - `pub struct HilbertSettings { cell_mm: f64, max_cell_mm: f64 }`.
  - `pub fn hilbert_style()`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn settings_schema()
#[test] fn white_folds_only_to_the_largest_cell() // 32x32 px at 2 mm/px (64 mm square), cell 1, max_cell 8: one path of 64 vertices (8x8 leaves), consecutive vertices 8 mm apart
#[test] fn black_folds_to_the_smallest_cell() // same map black, cell 2: 1024 vertices, steps 2 mm
#[test] fn the_curve_is_one_path_on_a_covered_square() // any ink on a square rect: exactly 1 path
#[test] fn every_vertex_stays_on_the_map() // 30x20 px map (non-square, non-power-of-two)
```

Registry list: append `"hilbert"`. The final list is the eighteen styles: `["grid", "hatch", "waves", "spiral", "ink", "stipple", "line", "dashes", "sketch", "tree", "mesh", "cells", "halftone", "dither", "circles", "contours", "flow", "hilbert"]`.

- [ ] **Step 2: Run them and check they fail**

Run: `cargo test -p plotline-core --lib stages::hilbert`
Expected: compile error.

- [ ] **Step 3: Implement it as spec §5.3 says**
  - The recursion is the classic vector form: `visit(origin, i_vec, j_vec, ...)`, splitting into four quadrants in Hilbert order (H. Sagan, *Space-Filling Curves*, Springer, 1994). A leaf emits its centre.
  - The cell mean comes from a summed-area table in `f64`.
  - The four top-level quadrants are four steps, each returning a frame of the path so far.
  - Leaf centres outside the rect end the current path.

- [ ] **Step 4: Register `hilbert`. Run `cargo test -p plotline-core`, `uv run pytest -q` and the verification set.**

- [ ] **Step 5: Commit**

```bash
git add crates/core/src tests crates/core/tests
git commit -m "Add the Hilbert curve style"
```

---

### Task 14: Timings, wasm size, browser check and documentation

**Files:**
- Modify: `crates/core/src/registry.rs` (an ignored timing test)
- Modify: `README.md`, `docs/BRIEF.md`

- [ ] **Step 1: Add `#[test] #[ignore] fn default_timings()` to `registry.rs`**
  - It runs `stipple` and `line` with `run_to_drawing` on `synthetic::radial(1000, 750)` at their defaults.
  - It prints the elapsed time of each with `std::time::Instant`.

Run: `cargo test --release -p plotline-core --lib default_timings -- --ignored --nocapture`
Expected: two lines of seconds. Record them.

- [ ] **Step 2: Measure the wasm size**

Run: `cd web && npm run wasm`, then check the byte size of `web/pkg/plotline_wasm_bg.wasm`.
Expected: 2,000,000 bytes or less. Record the number.

- [ ] **Step 3: Check every new style in the browser**
  - Run `npm run dev`, or the launch config's `web` server through `preview_start`.
  - Load the sample image. Run each of the 13 new styles and check that it draws with no console error.
  - Time `stipple` and `line` from Run to the end of the run, using `javascript_tool` and `performance.now()`.
  - Take one screenshot of `stipple`.

- [ ] **Step 4: Update the documentation**
  - **README:** the styles paragraph names and describes the 13 new styles in one sentence each. "There are five styles" becomes eighteen.
  - **Brief §7.1:** marked built.
  - **Brief §13:** milestones 3 to 5 marked done.
  - **Brief §14:** gains the spec §9 rows, plus a timing row with the native and browser numbers and the new wasm size (the "Wasm size limit" row is updated if the size moved).

- [ ] **Step 5: Run the full verification set and the Python tests one last time**

Run: `cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked -p plotline-core && uv run pytest -q && (cd web && npm run check)`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/registry.rs README.md docs/BRIEF.md
git commit -m "Describe the new styles and record their timings and the module size"
```
