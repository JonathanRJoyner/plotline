# The dark finding fires only with a fix

**Decided:** 2026-10-17

**Choice:** The plot lint's `dark` finding says the darkest parts of the picture are too lightly covered. It still measures the same: tone at least 0.75 covered under 35 %. But it now fires only when it can offer a fix whose setting moves at most four times from where it is, up or down. That is as far as Optimize scans ([optimize limits](2026-10-16-optimize-limits.md)). Without such a fix it stays quiet. A law in `crates/core/tests/laws.rs` checks that whenever `dark` fires, it carries a patch within that reach. The existing law that applied fixes shrink their findings still holds, so every patch it offers still helps. The lint test that pinned the cross-target raster now pins the coverage itself, in both `lint.rs` and the wasm smoke test, since the finding no longer fires on that drawing.

**Reason:** This settles A9 of the roadmap. Ink squared, the looks, disc dots and the colour lanes all landed. After them, `dark` still fired on 70 of the gallery's 126 cells. Nine of those had no fix at all. Many more asked for one no one would apply: 50,000 points against 5,000, or a spacing of 0.1 mm against 0.4. A finding that names no practical fix is noise, and it buries the ones that do.

Fires in the regression gallery, cells of 6 per style, before and after:

| Style | Before | After |
|---|---:|---:|
| `cells` | 4 | 2 |
| `circles` | 6 | 3 |
| `contours` | 6 | 0 |
| `dashes` | 6 | 3 |
| `flow` | 5 | 5 |
| `hatch` | 3 | 3 |
| `hilbert` | 3 | 0 |
| `line` | 5 | 2 |
| `mesh` | 3 | 3 |
| `mosaic` | 3 | 3 |
| `scribble` | 3 | 2 |
| `sketch` | 3 | 1 |
| `spiral` | 3 | 3 |
| `stipple` | 6 | 2 |
| `tree` | 5 | 2 |
| `truchet` | 3 | 3 |
| `waves` | 3 | 3 |
| **Total** | **70** | **40** |

`contours` and `hilbert` had no fix, and the rest of the drop is fixes past four times. The 40 that remain each offer a setting within Optimize's range that lifts the dark coverage, such as `voronoi.count` from 5,000 to between 7,200 and 19,900, or a hatch spacing from 1 to 0.5 mm.
