# Cells edge limit

**Decided:** 2026-10-17

**Choice:** The cells renderer has a setting, `cells.max_edge_mm`, from 0.5 to 50 mm, **16 mm** by default. A Voronoi wall is drawn only when the two points it separates, the ends of its dual Delaunay edge, are no farther apart than the limit. This is how mesh's `delaunay.max_edge_mm` leaves out its long edges. A law in `crates/core/tests/laws.rs` checks that raising the limit never removes a wall: with `order.join` off, the pen-down length never falls. Tree and single line get no such guard. The tree has to join every point and the single line is one stroke by design; a limit would change what either style is, so it was not measured.

**Reason:** This settles F11 of [the colour and styles audit](../audits/2026-10-16-colour-styles-audit.md). Points sit only where there is ink, so the cell of a point at the edge of an inked area is huge. Its walls ran across the white paper to the page edge.

The measurement draws `cells` at its defaults on the nine pictures of [looks on more pictures](2026-10-17-looks-on-more-pictures.md), with a 0.3 mm pen. Error, contrast and score are [the tone match's](2026-10-17-contrast-and-colour-match.md) medians over the nine. Paths, pen-down and plot time are sums.

| `max_edge_mm` | Paths | Pen-down m | Plot time min | Tone error | Contrast | Score |
|---|---:|---:|---:|---:|---:|---:|
| 50 (no limit) | 6,162 | 288.4 | 221.3 | 0.087 | 0.254 | 0.532 |
| 8 | 6,317 | 280.1 | 217.7 | 0.067 | 0.282 | 0.530 |
| **16** | 6,199 | 288.6 | 221.3 | 0.086 | 0.256 | 0.530 |
| 25 | 6,174 | 288.7 | 221.3 | 0.087 | 0.254 | 0.530 |

The median score is the same from 8 mm up. 8 mm, mesh's default, has the lowest tone error, but it gets it by breaking up the cells of pale backgrounds. Where the ink is light, neighbouring points sit more than 8 mm apart. Some of their walls go and others stay, so the still life's pale top filled with loose dashes and a starburst of spokes. 16 mm keeps the pale cells whole and still leaves out the long walls across white. Those walls are the ones that cross the checkerboard's white squares, whose score improves from 1.25 to 1.05, and the walls between the poster's discs. No picture's score is worse at 16 mm than with no limit.

On the regression gallery the default moves the `cells` cells; the snapshot is regenerated.
