# Path simplification

**Decided:** 2026-10-16

**Choice:** Added on 2026-10-16 under [the path simplification spec](../superpowers/specs/2026-10-16-path-simplification-design.md). The `order` stage has a setting, `simplify_mm`, from 0 to 1 mm in steps of 0.01 with a default of **0**, which is off. Before its length filter the stage passes each path of three or more vertices through `raster::simplify`, which is Ramer–Douglas–Peucker (D. H. Douglas and T. K. Peucker, "Algorithms for the reduction of the number of points required to represent a digitized line or its caricature", *The Canadian Cartographer* 10(2), 1973, pp. 112–122). It keeps the end points, so a closed path stays closed, and at 0 the output is bit for bit what it was. The plot lint's `vertices` finding gains the patch `{"settings": {"order": {"simplify_mm": t}}}`, where `t` is the narrowest pen's width over 8 rounded down to 0.001 mm and never under 0.001; it has no patch once `simplify_mm` is at least `t`. A law in `crates/core/tests/laws.rs` checks that `raster::simplify` keeps the end points, returns a subsequence of its input, keeps every input vertex within the tolerance of its output, and drops nothing from a line with no collinear vertices at a tolerance of 0.

**Reason:** Some styles sample curves far finer than a pen can show, and the extra vertices cost file size, plotter buffer and, on some firmware, speed. The tolerance of an eighth of the pen width is the one the `vertices` finding already calls finer than the pen can show. The default stays 0 because a preset saved before this change has no `simplify_mm` and takes the default, so any other default would change every drawing made from an old preset, every golden file and the gallery's committed stats snapshot. The measurements below show the size of that change: 16 of the 21 styles would draw differently.

The measurement draws each listed style at its defaults on `synthetic::still_life(600, 875)` on A4 with a 0.3 mm pen, with `order.simplify_mm` at 0 and at 0.0375, which is the pen width over 8. The vertices are those of all layers. The SVG is `export::svg::to_svg` without the embedded preset, so the setting's own text does not count. The previews are 1000 by 1414 px, 1,414,000 pixels, about 5 px per mm. "Max diff" is the largest difference in any channel of any pixel, out of 255; "differ" counts the pixels where any channel differs, and "over 32" those where a channel differs by more than 32. The program was a throwaway release-build example and is not kept.

| Style | Vertices at 0 | Vertices at 0.0375 | SVG bytes at 0 | SVG bytes at 0.0375 | Max diff | Pixels that differ | Pixels over 32 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `grid` | 6,215 | 6,215 | 319,961 | 319,961 | 0 | 0 | 0 |
| `hatch` | 1,136 | 1,136 | 29,268 | 29,268 | 0 | 0 | 0 |
| `waves` | 314,778 | 85,543 | 5,416,766 | 1,488,431 | 64 | 256,813 | 13,418 |
| `spiral` | 315,838 | 86,172 | 5,434,748 | 1,498,937 | 112 | 231,817 | 21,203 |
| `ink` | 22,093 | 13,520 | 396,809 | 244,675 | 64 | 2,026 | 177 |
| `stipple` | 129,015 | 55,055 | 2,345,006 | 1,046,403 | 64 | 26,935 | 1,561 |
| `line` | 5,000 | 4,896 | 87,797 | 85,964 | 64 | 2,557 | 258 |
| `dashes` | 10,000 | 10,000 | 255,329 | 255,329 | 0 | 0 | 0 |
| `sketch` | 5,001 | 4,960 | 88,671 | 87,947 | 64 | 1,021 | 59 |
| `tree` | 6,282 | 6,167 | 130,761 | 128,731 | 64 | 2,639 | 307 |
| `mesh` | 15,727 | 15,533 | 290,990 | 287,575 | 64 | 4,177 | 343 |
| `cells` | 19,800 | 19,747 | 426,912 | 425,976 | 64 | 537 | 35 |
| `halftone` | 604,335 | 207,664 | 10,777,041 | 3,821,660 | 112 | 92,319 | 5,408 |
| `dither` | 11,282 | 11,282 | 575,735 | 575,735 | 0 | 0 | 0 |
| `circles` | 42,972 | 11,873 | 769,342 | 221,790 | 80 | 25,869 | 600 |
| `contours` | 1,476 | 1,476 | 26,631 | 26,631 | 0 | 0 | 0 |
| `flow` | 30,779 | 2,883 | 534,884 | 53,069 | 64 | 35,292 | 9,134 |
| `hilbert` | 29,925 | 23,443 | 527,965 | 413,464 | 48 | 1,750 | 121 |
| `scribble` | 197,970 | 66,957 | 3,462,368 | 1,168,085 | 112 | 164,766 | 14,403 |
| `mosaic` | 7,948 | 7,936 | 190,684 | 190,477 | 16 | 11 | 0 |
| `truchet` | 51,843 | 23,526 | 926,084 | 430,072 | 112 | 72,680 | 6,210 |
| **Total** | **1,829,415** | **665,984** | **33,013,752** | **12,800,180** | **112** | | |

Over the 21 styles the vertices fall by 63.6 % and the SVG by 61.2 %. `flow` loses 90.6 % of its vertices, `waves`, `spiral` and `circles` about 72 %, `scribble` 66 %, `halftone` 66 %, `stipple` 57 % and `truchet` 55 %. `grid`, `hatch`, `dashes` and `dither` draw dots and two-vertex strokes, so a path of fewer than three vertices leaves them alone, and `contours` and `ink` already simplify their own lines.

The pixel differences are the size of antialiasing. At this width the tolerance is 0.19 px, so by the Douglas–Peucker bound no mark moves by more than a fifth of a pixel, and no channel of any pixel differs by more than 112 of 255. The most pixels that differ are in `waves` (18 % of the page) and `spiral` (16 %), the styles with the longest lines of the most vertices; at most 1.5 % of the page (`spiral`) differs by more than 32, and in the styles that change little, such as `mosaic` and `cells`, a few hundred pixels or fewer differ.

The `vertices` finding fires on none of the 21 drawings: at a 0.3 mm pen it needs over half of at least 1,000 segments of a layer to be shorter than 0.0375 mm, and no style's defaults sample that finely. It fires for the same drawings under a wider pen, as the tests show for `waves` with a 1 mm pen, and its patch then turns simplification on. Applying the patch is the supported way to use the setting; the measurement shows what a user gets when the tolerance is the pen over 8.

**Settled in the build, not in the spec:** the finding's message adds " Path order's simplify setting removes them." only when it offers the patch, as the `travel` finding does with its optimize sentence, so a message does not tell the user to turn on a setting that is already on. `crates/core/tests/settings_matter.rs` lists `simplify_mm` as a no-op for `grid`, `dither`, `hatch` and `dashes`, whose paths have fewer than three vertices at their defaults; every other style changes at the test's value of 0.5 mm.

**Amended on 2026-10-16:** the `vertices` patch is offered only when `order.simplify_mm` is 0 and the finding's number, predicted by simplifying the drawing's own paths, would fall or the finding would go away. The property laws found a drawing where simplifying to an eighth of the narrowest pen raised the number from 0.67 to 0.74: a wide pen's layer counts its fine segments against its own width, so the patch removed its near-collinear long segments more than its short ones. See [lint fixes that hold](2026-10-16-lint-fixes-that-hold.md).
