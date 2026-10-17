# Mesh drops its loose edges

**Decided:** 2026-10-17

**Choice:** After leaving out the Delaunay edges longer than `delaunay.max_edge_mm`, the mesh renderer drops any edge with an end that no other kept edge reaches. It repeats until none is left, which keeps the graph's 2-core (S. B. Seidman, "Network structure and minimum degree", *Social Networks* 5(3), 1983, pp. 269–287). The 2-core is unique, so it does not depend on the order edges are dropped in. Points that all lie on one line have no triangle, and they keep their chain whole. The limit stays at 8 mm.

A unit test in `stages/edges.rs` checks that no drawn vertex has one edge. A law in `crates/core/tests/laws.rs` checks that raising the limit never removes an edge: with `order.join` off, the pen-down length never falls. The 2-core only grows with the graph.

**Reason:** This settles A10 of the roadmap. With [ink squared](2026-10-17-ink-squared-for-joined-points.md), the points in pale parts of a picture sit near or past 8 mm apart. Some of their edges went and some stayed, so the still life's pale top was ringed with stubs and loose strokes. That is the problem [the cells edge limit](2026-10-17-cells-edge-limit.md) met.

A higher limit does not fix it for mesh. The table draws `mesh` at its defaults on the nine pictures of [looks on more pictures](2026-10-17-looks-on-more-pictures.md), with a 0.3 mm pen; error, contrast and score are medians.

| Limit | Loose edges | Paths | Pen-down m | Plot time min | Tone error | Contrast | Score |
|---|---|---:|---:|---:|---:|---:|---:|
| 8 mm | kept | 1,732 | 392.7 | 217.4 | 0.071 | 0.473 | 0.292 |
| 12 mm | kept | 1,727 | 407.4 | 223.0 | 0.069 | 0.451 | 0.359 |
| 16 mm | kept | 1,729 | 416.9 | 226.5 | 0.070 | 0.433 | 0.460 |
| 25 mm | kept | 1,725 | 429.0 | 231.1 | 0.068 | 0.429 | 0.435 |
| 8 mm, whole triangles only | dropped | 1,716 | 389.5 | 216.0 | 0.074 | 0.504 | 0.302 |
| **8 mm, 2-core** | **dropped** | 1,715 | 392.1 | 216.9 | 0.072 | 0.484 | 0.296 |

- **A higher limit** stops the pale top fragmenting by filling it with large triangles. That lays ink where the picture is light, and every limit above 8 mm scores worse: as its white squares fill, the checkerboard's score goes from 0.25 to 0.89.
- **Whole triangles only:** keeping only the sides of triangles whose three sides pass cleared the stubs. It also broke the still life's floor into isolated triangles, because a quad loses all four sides when its diagonal is too long.
- **The 2-core** clears the stubs and keeps the floor whole, as the drawings show. Its median score is within 0.004 of the old one, and its contrast is higher. Gradient and colour bars worsen most, each by under 0.005 in score; no other picture moves by more than 0.003.

On the regression gallery the change moves the `mesh` cells; the snapshot is regenerated.
