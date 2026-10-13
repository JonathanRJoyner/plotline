# plotline spec 8: the point family and more styles

Date: 2026-10-12. Status: written after the owner asked for as many new algorithms as possible once the repository went public; open to review.

The brief's first-release styles (§7.1: weighted Voronoi stipple, a continuous line, dashes and Sketch Lines) are still missing. This spec builds them, then widens the catalogue with more point placers, graph renderers and styles that need only the tone map. Every new piece is a stage behind the existing `Stage` trait, so caching, the settings form, presets, the CLI, the MCP and every export work unchanged. Nothing in the GUI changes: it is generated from the stage schemas.

The work lands in three waves, each usable on its own:

1. The Voronoi family and Sketch Lines (the brief's first release).
2. More placers and the graph renderers.
3. Styles that need only the tone map.

## 1. Styles

Every style starts with the existing `tone` stage and ends with the existing `order` stage.

| Wave | Style | Stages between tone and order | Look |
|---|---|---|---|
| 1 | `stipple` "Stipple" | voronoi, stipple | Evenly spread dots, dense in dark areas, each dot sized by the ink under it |
| 1 | `line` "Single line" | voronoi, tsp | One unbroken, non-crossing line through every point |
| 1 | `dashes` "Dashes" | voronoi, dashes | Short strokes that follow the image's edges |
| 1 | `sketch` "Sketch lines" | sketch | Straight strokes chained from dark to dark, like a quick pencil sketch |
| 2 | `tree` "Tree" | voronoi, mst | The shortest tree joining every point |
| 2 | `mesh` "Mesh" | voronoi, delaunay | A triangle mesh over the points |
| 2 | `cells` "Cells" | voronoi, cells | The outlines of the points' Voronoi cells |
| 2 | `halftone` "Halftone" | points (grid), stipple | Dots on a regular grid, each sized by the ink, like a printed halftone |
| 2 | `dither` "Dither" | dither, render (dots) | Dots where error diffusion puts them |
| 2 | `circles` "Circles" | poisson, stipple | Circles packed without overlapping, larger and closer where it is darker |
| 3 | `contours` "Contours" | contours | Level lines of the image, like a topographic map |
| 3 | `flow` "Flow lines" | flow | Evenly spaced lines that follow the image's edges, closer together where it is darker |
| 3 | `hilbert` "Hilbert curve" | hilbert | One space-filling line that folds tighter where it is darker |

`halftone` reuses the grid placer (stage `points`) and `dither` reuses the dot renderer (stage `render`). `halftone` and `circles` are compositions with their own defaults through `Style::with_defaults`. They need no stage of their own.

The registry lists the existing five styles first, then these in the order of the table.

## 2. Rules every new stage follows

- **Unique stage names.** The cache key is the stage's name, version, settings and input keys, so two different stages that shared a name, a version and a settings object would hand each other their outputs. Every new stage has its own name. A registry test checks that stages sharing a name across all built-in styles have the same label and schema, which holds today for `tone` and `order`.
- **Shared points.** All six Voronoi styles use the one `voronoi` stage with the same defaults. Switching between them reuses the cached points, which is a first-release success criterion.
- **Determinism.** Randomness comes only from `RunContext::rng(seed)`, with a `seed` setting from 0 to 9999, default 1. Transcendental functions go through `math`. Ties go to the lowest index. Sums run in a fixed order.
- **On the map.** Every vertex lies inside the tone map's rect. A stage clamps or clips to it, as the line styles do.
- **Frames.** A stage reports progress at least every 50 ms of work on a typical image, so Stop stays responsive. Frames hold the points or paths made so far.
- **Help text.** Every setting has one line of help, and every module docstring cites its source, or says it follows no published algorithm.

## 3. Wave 1: the Voronoi family and Sketch Lines

### 3.1 `voronoi` "Weighted Voronoi" (tone → points)

Weighted Voronoi stippling: A. Secord, "Weighted Voronoi stippling", *Proc. NPAR*, 2002, pp. 37-43. It uses Lloyd's method: S. P. Lloyd, "Least squares quantization in PCM", *IEEE Trans. Information Theory* 28(2), 1982.

- **Density** is `ink ^ exponent` per tone pixel.
- **Initial points** are drawn from the density by inverse-CDF sampling over the pixels in row-major order, each jittered uniformly inside its pixel, from the seeded generator.
- **Each iteration** assigns every tone pixel to its nearest point through `spatial::GridIndex` (cell side `sqrt(area / count)`) and moves each point to the density-weighted centroid of its pixels. A point whose pixels carry no density stays put.
- **After the last iteration**, points whose cells carry no density are dropped, so no stray dots sit in white areas.
- **A step** covers a band of rows, so an iteration on a large map spans several steps. A frame after each step holds the current points, labelled `relaxation k/N`.

| Setting | Range | Default | Help |
|---|---|---|---|
| `count` | 100 to 50000 | 5000 | Points to place; more points give finer detail and a longer plot |
| `iterations` | 0 to 200 | 30 | Relaxation passes; more passes spread the points more evenly |
| `exponent` | 0.5 to 3 | 1 | Density follows ink to this power; 2 suits the line, tree, mesh and cells renderers, which put more ink between close points |
| `seed` | 0 to 9999 | 1 | Starting scatter |

### 3.2 `stipple` "Stipple dots" (points, tone, pens → paths)

Each point becomes a dot of diameter `d = size_mm * sqrt(1 - vary + vary * ink)`, so a dot's area follows the ink.

- **`auto` shape:** a dot no wider than the first pen becomes a one-vertex path. A wider dot becomes a spiral fill: an Archimedean spiral from the centre with a pitch of one pen width, out to radius `(d - w) / 2`, closed by one full circle at that radius. The brief's "Dot geometry" decision already fixes this.
- **`point`** always gives one vertex.
- **`circle`** gives that outer circle alone.
- **Sampling:** arcs are sampled at most a quarter pen width apart.
- **Frames:** one frame, then the paths.

| Setting | Range | Default | Help |
|---|---|---|---|
| `size_mm` | 0.1 to 10 | 0.8 | Dot diameter over full ink |
| `vary` | 0 to 1 | 0.5 | How much lighter ink shrinks a dot: 0 keeps every dot the same size |
| `shape` | auto, point, circle, spiral | auto | How a dot is drawn |

### 3.3 `tsp` "Single line" (points → paths)

The tour starts with nearest-neighbour construction from the point nearest the top-left corner of the points' bounding box, using `GridIndex::nearest`. It is then improved by 2-opt and Or-opt (segments of 1 to 3 points) over Delaunay-neighbour candidate lists with don't-look bits, following D. S. Johnson and L. A. McGeoch, "The traveling salesman problem: a case study in local optimization", in *Local Search in Combinatorial Optimization*, Wiley, 1997, pp. 215-310. The brief's "Continuous-line solver" decision already fixes this.

- **Open line:** the tour is improved as a cycle, then opened at its longest edge unless `closed` is on.
- **Segment reversal** reverses the shorter side.
- **No crossings:** a 2-opt local optimum in the plane has none. Note, 2026-10-13: over neighbour lists the optimum can keep crossings, as probes found, so the build adds a crossing sweep. Once the queue is empty, a scan over a grid uncrosses any remaining crossings and improvement resumes, until a scan finds none.
- **Frames:** one after construction, then one per step of about 2000 node checks, each holding the current tour, labelled `construction` or `improving: k moves`.

| Setting | Range | Default | Help |
|---|---|---|---|
| `optimize` | bool | true | Shorten the line and remove its crossings; off leaves the nearest-neighbour line |
| `closed` | bool | false | End the line where it started |

### 3.4 `dashes` "Dashes" (points, tone → paths)

Each point becomes one straight dash centred on it. The direction runs along the image's edges: perpendicular to the tone gradient, measured by central differences half a dash length either side, then turned by `angle_deg`. Where the gradient is under 1e-3 ink per mm, the dash lies at `angle_deg` from horizontal. With `follow_edges` off, every dash lies at `angle_deg`. Both ends are clamped to the tone rect.

Strokes oriented by the image gradient follow A. Hertzmann, "Painterly rendering with curved brush strokes of multiple sizes", *Proc. SIGGRAPH*, 1998, pp. 453-460.

| Setting | Range | Default | Help |
|---|---|---|---|
| `length_mm` | 0.2 to 10 | 1.5 | Dash length |
| `angle_deg` | 0 to 180 | 0 | Turn from the edge direction, or the fixed direction when edges are not followed |
| `follow_edges` | bool | true | Lay dashes along the image's edges; off lays every dash at the fixed angle |

### 3.5 `sketch` "Sketch lines" (tone → paths)

This is the greedy loop of the brief's §7.1. It keeps a residual copy of the tone map. From the pen's position it tries `candidates` segments of length `length_mm` at angles from the seeded generator, and scores each by the mean residual ink along it, sampled every half pixel. The best segment is drawn as the next vertex of the current stroke, and the residual under it is lowered by `lighten`, clamped at 0. Each pixel is lowered at most once per segment.

- **When the best score falls under `stop`**, the stroke ends and the pen moves to the darkest residual pixel. It is found through the maxima of 16 by 16 pixel blocks, kept up to date for the blocks a segment touches, so a restart does not scan the whole map. Ties go to the first block in row-major order, then the first pixel in it.
- **The run ends** when that pixel is under `stop` too, or after `max_lines` segments.
- **Frames** come every 1/30 of `max_lines`, holding the strokes so far.

The greedy residual-subtraction loop also drives string-art methods: M. Birsak, F. Rist, P. Wonka and P. Musialski, "String art: towards computational fabrication of string images", *Computer Graphics Forum* 37(2), 2018. The citation is to be checked against the paper before it goes into the docstring.

| Setting | Range | Default | Help |
|---|---|---|---|
| `length_mm` | 1 to 50 | 6 | Length of each straight segment |
| `candidates` | 4 to 256 | 48 | Directions tried per segment; more find darker paths but run slower |
| `lighten` | 0.05 to 1 | 0.3 | Ink removed under each drawn segment; less gives more, denser strokes |
| `stop` | 0 to 1 | 0.2 | Ink left under which no stroke is drawn |
| `max_lines` | 100 to 50000 | 5000 | Most segments to draw |
| `seed` | 0 to 9999 | 1 | Directions tried |

## 4. Wave 2: more placers and the graph renderers

### 4.1 `dither` "Error diffusion" (tone → points)

Floyd–Steinberg error diffusion on a grid of `spacing_mm` cells, sampling the tone at cell centres. A cell whose value plus carried error reaches 0.5 gets a point at its centre. Sources: R. W. Floyd and L. Steinberg, "An adaptive algorithm for spatial greyscale", *Proc. SID* 17(2), 1976, pp. 75-77. Serpentine scanning follows R. Ulichney, *Digital Halftoning*, MIT Press, 1987. Frames come by rows, like the grid placer.

| Setting | Range | Default | Help |
|---|---|---|---|
| `spacing_mm` | 0.2 to 10 | 1 | Grid cell size; one dot at most per cell |
| `serpentine` | bool | true | Scan alternate rows in opposite directions, which breaks up diagonal patterns |

### 4.2 `poisson` "Poisson disk" (tone → points)

Variable-radius Poisson-disk sampling, after R. Bridson, "Fast Poisson disk sampling in arbitrary dimensions", *SIGGRAPH Sketches*, 2007.

- **Radius:** a point at `p` needs every other point at least `r(p) = spacing_mm / sqrt(ink(p))` away, capped at `20 * spacing_mm`. Ink under 1/400 takes no point.
- **Growth** runs from an active list, with 30 candidates per active point in the annulus `[r, 2r]`.
- **Seeds:** when the active list empties, the next seed is a dart drawn by ink with the voronoi stage's sampler, so a small dark area on a white page still gets one. Seeding stops after 30 darts in a row fail.
- **Neighbour checks** use a bucket grid with cells of `spacing_mm`.
- **Frames** come about every 1/30 of the points expected.

| Setting | Range | Default | Help |
|---|---|---|---|
| `spacing_mm` | 0.2 to 20 | 1 | Distance between points over full ink; lighter areas space them further apart |
| `seed` | 0 to 9999 | 1 | Where the packing starts |

### 4.3 Graph renderers (points → paths)

A new module `stages/graph.rs` holds three shared pieces:

- **Delaunay triangulation** of the points through the `delaunator` crate.
- **`trails(n, edges)`**, which splits a graph's edges into as few pen-down trails as a greedy walk finds. Start at the lowest-index vertex of odd remaining degree, or else the lowest with any remaining edge, and walk unused edges (lowest neighbour first) until stuck. Each trail from an odd vertex ends at another odd vertex, so a connected graph with `k` odd vertices gives `k / 2` trails, plus closed trails for any edges left. Paths are joined where they meet rather than lifted between.
- **`clip_segment(rect, a, b)`**, a Liang–Barsky clip of a segment to a rect, with the same citation as `lines.rs`.

| Stage | What it draws | Settings |
|---|---|---|
| `mst` "Spanning tree" | The Euclidean minimum spanning tree: Kruskal's algorithm over the Delaunay edges, sorted by length with ties by index, using union-find. The Euclidean MST is a subgraph of the Delaunay triangulation (M. I. Shamos and D. Hoey, "Closest-point problems", *Proc. FOCS*, 1975). J. B. Kruskal, *Proc. AMS* 7(1), 1956. | none |
| `delaunay` "Triangle mesh" | Every Delaunay edge no longer than `max_edge_mm`. B. Delaunay, "Sur la sphère vide", 1934, via the delaunator crate. | `max_edge_mm` 0.5 to 50, default 8: "Leave out edges longer than this, such as those across light areas" |
| `cells` "Voronoi cells" | Each Delaunay edge shared by two triangles gives the Voronoi edge joining their circumcentres, clipped to the tone rect. Hull edges, whose Voronoi edges are unbounded, are left out. The trails run over circumcentre vertices, and each trail is clipped and split where it leaves the rect. | none |

Each graph renderer reports one frame, then the paths.

### 4.4 The compositions

- **`halftone`:** grid placer `{"spacing_mm": 2, "threshold": 0.02}` and stipple `{"size_mm": 2, "vary": 1}`.
- **`dither`:** the dither placer and the existing dot renderer.
- **`circles`:** poisson `{"spacing_mm": 3}` and stipple `{"size_mm": 3, "vary": 1, "shape": "circle"}`.

## 5. Wave 3: styles from the tone map alone

### 5.1 `contours` "Contours" (tone → paths)

The tone map is blurred by `smooth_mm` with `math::blur` and padded by one zero pixel on every side. Its iso-lines at `levels` evenly spaced values in `(0, 1)` come from `raster::iso_lines`, and each one is simplified with `raster::simplify` at a tolerance of half a pixel, then mapped to mm. Lines crowd where the tone changes fast, which draws shapes rather than shading. The iso-line tracer already cites its source. Equally spaced levels of a smoothed field follow no published plotting algorithm, so the stage cites nothing else.

| Setting | Range | Default | Help |
|---|---|---|---|
| `levels` | 1 to 64 | 12 | Contour levels between white and full ink |
| `smooth_mm` | 0 to 10 | 0.5 | Blur before tracing; more gives rounder, calmer lines |

### 5.2 `flow` "Flow lines" (tone → paths)

Evenly spaced streamlines, after B. Jobard and W. Lefer, "Creating evenly-spaced streamlines of arbitrary density", *Proc. Visualization in Scientific Computing*, 1997, pp. 43-55.

- **Field:** the tone map is blurred by `smooth_mm` and its gradient `g` is taken by central differences. The direction is the unit vector of `rot90(g) + 0.05 * (cos a, -sin a)`, where `a` is `angle_deg`. Lines therefore follow the image's edges and drift at `angle_deg` where the image is flat.
- **Spacing:** the separation at a point is `d(p) = max_spacing - (max_spacing - min_spacing) * ink(p)`.
- **Tracing:** a line is traced both ways from its seed with second-order Runge–Kutta steps of `min_spacing / 2`. It stops at the rect's edge, after 20000 steps, or within `d(p) / 2` of another line's samples.
- **Seeds:** new seeds sit `d(p)` to either side of each traced sample, taken in FIFO order. The first seed is the rect's centre. When the queue empties, a scan of a coarse grid at `max_spacing` adds any seed that is free.
- **Short lines:** lines shorter than `2 * min_spacing` are dropped.
- **Neighbour checks** use a bucket grid with cells of `min_spacing`.
- **Frames** come about every 50 lines.

| Setting | Range | Default | Help |
|---|---|---|---|
| `min_spacing_mm` | 0.3 to 10 | 0.8 | Line spacing over full ink |
| `max_spacing_mm` | 1 to 30 | 6 | Line spacing over white |
| `smooth_mm` | 0 to 10 | 2 | Blur before the edge directions are measured; more gives longer, smoother lines |
| `angle_deg` | 0 to 180 | 0 | Direction the lines take where the image is flat |

### 5.3 `hilbert` "Hilbert curve" (tone → paths)

An adaptive Hilbert curve, after L. Velho and J. de M. Gomes, "Digital halftoning with space filling curves", *Proc. SIGGRAPH*, 1991, pp. 81-90. The roadmap lists their patent, US5602943, as expired. That status is to be confirmed on the patent record before the style ships.

- **The square:** the smallest square of side `cell_mm * 2^k` that covers the tone rect, anchored at the rect's top-left corner.
- **Subdivision:** a cell is split into four, in Hilbert order and orientation, while it is wider than `max_cell_mm`, or while it is wider than `cell_mm / max(ink, 1/64)` and wider than `cell_mm`. The ink is the cell's mean, from a summed-area table over the tone map.
- **The line** joins the centres of the leaf cells in curve order. Leaves whose centres fall outside the rect break it, so it is one path wherever the rect is covered.
- **Line density** follows ink because a curve through cells of side `s` lays down about one `s` of line per `s²` of area.
- **Frames:** one per top-level quadrant.

| Setting | Range | Default | Help |
|---|---|---|---|
| `cell_mm` | 0.3 to 5 | 0.6 | Smallest fold, used over full ink |
| `max_cell_mm` | 1 to 40 | 8 | Largest fold, used over white |

## 6. Dependency

`delaunator` 1.1.0 (ISC, no dependency but `robust` 1.2.0, MIT or Apache-2.0) gives the Delaunay triangulation that `tsp`, `mst`, `delaunay` and `cells` share. Both licences are on the cargo-deny allow-list. Both crates compute in plain `f64` with no fused multiply-add and no transcendental calls, so output stays bit-identical between native and WebAssembly builds. A triangulation is about 600 lines to write and test, so the crate is cheaper. The wasm size check (2,000,000 bytes) must still pass, and the log records the new size.

## 7. Tests

- **Each stage:** its schema bounds and defaults; every vertex inside the tone rect; white gives nothing where nothing is expected (dither, poisson, sketch, contours); a bad setting is refused at `start`. Plus one behaviour test each:
  - voronoi: points are denser in the dark half of a split map, and none sit in a white area
  - stipple: a point at full ink with `vary` 1 becomes a spiral whose outer radius is `(size - w) / 2`
  - tsp: one path through every point, with no two segments crossing on a random set
  - dashes: on a vertical edge, the dashes are vertical
  - sketch: strokes cover a dark bar and stop at `max_lines`
  - dither: a flat 0.5 map gives about half the cells
  - poisson: no two points closer than the smaller of their radii
  - mst: `n - 1` edges for `n` points, and the total length is at most the Delaunay mesh's
  - trails: a path graph gives one trail, and a star with four arms gives two
  - cells: every vertex lies inside the rect
  - contours: a radial gradient gives nested closed loops
  - flow: no two lines' samples closer than `min_spacing / 2` (less 1e-9)
  - hilbert: a white map at `max_cell_mm` gives one path through every coarse cell
- **`registry.rs`:** the list holds the eighteen styles in order; stages sharing a name share a label and schema; every style is deterministic, which the existing test covers once the list grows.
- **`settings_matter.rs`:** every new setting changes the drawing. Settings that cannot change it on the noise image are listed as no-ops, each with a reason.
- **Golden:** `golden/stipple_gradient.json` pins the `stipple` style on the 64 by 48 gradient at `{"count": 300, "iterations": 10}`. The native golden test and the WebAssembly smoke test both check it, because relaxation is the longest chain of float sums in the engine.
- **Python:** `tests/test_core_binding.py` and `tests/test_session.py` list the eighteen styles.
- **Timing:** the voronoi and tsp stages are timed at their defaults on the sample image, native and in the browser. The numbers go into the brief rather than into a test, as milestone 3 asks.

## 8. Documentation

- **README:** the styles paragraph describes the new styles.
- **Brief §7.1:** marked built.
- **Brief §13:** milestones 3 to 5 marked done.
- **Brief decision log**, new rows:
  - the point family and more styles
  - unique stage names
  - the shared Voronoi defaults, with the exponent trade-off below
  - the `delaunator` dependency
  - dropping empty Voronoi points

## 9. Decisions to record in the brief

- **The Voronoi exponent defaults to 1 for every Voronoi style.** Connected renderers reproduce tone best with density following ink squared (A. G. M. Ahmed, *Proc. Bridges* 2015, pp. 267 onward, https://archive.bridgesmathart.org/2015/bridges2015-267.html; the title and claim to be checked against the paper). A default of 2 for them would break the first-release promise that switching renderer keeps the points. The help text names 2 for those renderers instead.
- **Stage names are unique per stage type,** and a test enforces it, because the cache key does not hold the stage's type.
- **Empty Voronoi points are dropped** after relaxation, so white areas get no stray dots. This serves the brief's "no stray specks" quality goal.
- **Delaunay comes from the `delaunator` crate,** for the reasons in §6.
