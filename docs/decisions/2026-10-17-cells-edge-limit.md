# Cells edge limit

**Decided:** 2026-10-17

**Choice:** The cells renderer has a setting, `cells.max_edge_mm`, from 0.5 to 50 mm, **8 mm** by default, the same range and default as mesh's `delaunay.max_edge_mm`. A Voronoi wall is drawn only when the two points it separates, the ends of its dual Delaunay edge, are no farther apart than the limit. A law in `crates/core/tests/laws.rs` checks that raising the limit never removes a wall: with `order.join` off, the pen-down length never falls. Tree and single line get no such guard. The tree has to join every point, and the single line is one stroke by design; a limit on either would change what the style is, so it was not measured.

**Reason:** This settles F11 of [the colour and styles audit](../audits/2026-10-16-colour-styles-audit.md). Points sit only where there is ink, so the cell of a point at the edge of an inked area is huge, and its walls ran across the white paper to the page edge. Mesh already leaves out its long edges for the same reason.

The measurement draws `cells` at its defaults on ten pictures with a 0.3 mm pen. The pictures are the gallery's three; three dark discs on white (a poster); a checkerboard; colour bars; line art; noise; and the scene darkened to 35 % and lightened to 35 % of its ink. Error, contrast and score are [the tone match's](2026-10-17-contrast-and-colour-match.md) medians over the ten. Paths, pen-down and plot time are sums.

| `max_edge_mm` | Paths | Pen-down m | Plot time min | Tone error | Contrast | Score |
|---|---:|---:|---:|---:|---:|---:|
| 50 (no limit) | 6,831 | 327.2 | 248.5 | 0.066 | 0.247 | 0.607 |
| **8** | 6,985 | 319.0 | 245.2 | **0.059** | **0.278** | **0.555** |
| 4 | 9,086 | 262.2 | 220.6 | 0.068 | 0.374 | 0.606 |

At 8 mm no picture's tone error or contrast gets worse than with no limit. The still life, scene, gradient, checkerboard, colour bars and pale scene all improve. The poster, line art, noise and dark scene stay within 0.001. The poster gains least: with ink-squared density there are no points on its white paper, so only the few walls between the discs go. At 4 mm the cells lose walls inside the inked areas too, and the error rises again.

On the regression gallery the default moves the six `cells` cells; the snapshot is regenerated.
