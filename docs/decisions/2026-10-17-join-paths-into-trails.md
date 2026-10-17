# Joining paths into trails

**Decided:** 2026-10-17

**Choice:** The `order` stage has a new setting, `join`, with three values. `off` leaves the paths as the style drew them. `on` snaps the vertices of the paths that lie within 0.001 mm of each other into one graph, draws each distinct edge once, and splits the graph into the fewest trails it allows: for a connected part with `k > 0` odd vertices that is `k / 2`, the bound J. Edmonds and E. L. Johnson start the postman problem from ("Matching, Euler tours and the Chinese postman", *Mathematical Programming* 5, 1973, pp. 88–124). The trails come from Euler circuits found as C. Hierholzer describes (*Mathematische Annalen* 6, 1873, pp. 30–32), after pairing the odd vertices of each part with virtual edges, and cut at those edges. `retrace` does the same after drawing some edges twice: the edges whose two ends are both odd, shortest first, while their part still has four or more odd vertices. Each edge so doubled saves one trail. Edmonds and Johnson double the shortest set of paths that pairs all the odd vertices, found by weighted matching; the greedy pass is cheaper and takes only single edges. The join runs first in the stage, before simplification, the length filter and the ordering, and the code is `crates/core/src/stages/join.rs`.

The default is `off`, except through the style defaults: **`retrace` for `cells`, `mesh` and `tree`**.

If the trails would be more paths than the input, which can only happen when the input draws an edge twice on purpose, the stage keeps the input. Dots, and paths whose vertices all snap to one point, pass through unchanged. A law in `crates/core/tests/laws.rs` checks that both modes never give more paths than the input, keep every segment and every dot, add no segment, and that `retrace` never gives more paths than `on`.

**Reason:** The request behind this change said `cells` drew every shared edge twice, one closed loop per cell. It does not: the cells renderer already walks the Voronoi graph with `graph::trails` and draws each edge once. Every corner of a Voronoi cell joins three edges, so every vertex is odd and about 5,000 paths per drawing is already the Edmonds–Johnson bound. `on` cannot lower it, and the gallery shows that. Only drawing some edges twice can, which is why `retrace` exists and why the three graph styles take it.

The gallery draws each style on three images at two pen widths, six cells per style. The table sums those six cells at `off` (the drawings before this change), `on` and `retrace`; tone error is the mean, and "lifts" counts the cells where the plot lint's `lifts` finding fires. Plot time uses the default speeds: 50 mm/s drawing, 100 mm/s travel, 1000 mm/s² acceleration and 0.2 s per pen lift. The numbers are from main at 484e7fc, after the plot time began to count acceleration ([acceleration in the plot time](2026-10-17-acceleration-in-the-plot-time.md)) and contour levels began to end at the image edge ([contours end at the image edge](2026-10-17-contours-open-at-edge.md)).

| Style | `join` | Paths | Pen-down m | Pen-up m | Plot time min | Tone error | `lifts` |
|---|---|---:|---:|---:|---:|---:|---:|
| `cells` | off | 29,962 | 172.9 | 67.3 | 255.5 | 0.105 | 0/6 |
| `cells` | on | 29,960 | 172.9 | 68.7 | 255.9 | 0.105 | 0/6 |
| `cells` | **retrace** | **4,170** | 205.6 | 27.9 | **157.3** | 0.105 | 0/6 |
| `mesh` | off | 5,650 | 296.3 | 31.0 | 179.5 | 0.114 | 0/6 |
| `mesh` | on | 5,430 | 296.3 | 30.0 | 179.3 | 0.114 | 0/6 |
| `mesh` | **retrace** | **1,096** | 309.6 | 13.3 | **162.0** | 0.114 | 0/6 |
| `tree` | off | 7,816 | 89.4 | 39.3 | 88.7 | 0.249 | 0/6 |
| `tree` | on | 7,816 | 89.4 | 38.5 | 88.6 | 0.249 | 0/6 |
| `tree` | **retrace** | **4,246** | 99.8 | 27.3 | **75.1** | 0.249 | 0/6 |
| `truchet` | off | 8,880 | 175.1 | 36.6 | 111.9 | 0.129 | 0/6 |
| `truchet` | on | 8,852 | 175.1 | 36.6 | 111.8 | 0.129 | 0/6 |
| `truchet` | retrace | 8,852 | 175.1 | 36.6 | 111.8 | 0.129 | 0/6 |
| `mosaic` | off | 13,910 | 325.1 | 33.0 | 215.9 | 0.050 | 0/6 |
| `mosaic` | on | 13,592 | 325.1 | 33.8 | 214.3 | 0.050 | 0/6 |
| `mosaic` | retrace | 13,130 | 328.8 | 32.2 | 213.5 | 0.050 | 0/6 |

- **`cells`:** `retrace` cuts the paths by 86 % and the plot time by 38 %, about 4,300 fewer pen lifts per drawing. The cost is 19 % more pen-down length: those edges are drawn twice, over a line already there. Under the plot time before acceleration was counted, `retrace` cut the time by 48 % and cleared the `lifts` finding, which fired in all six cells; with acceleration counted, drawing takes longer, the lifts' share falls under the finding's half, and it fires in none of them either way.
- **`mesh`:** `retrace` cuts the paths by 81 % and the plot time by 10 %, for 4.5 % more pen-down. `on` alone saves 4 % of the paths.
- **`tree`:** a tree's greedy trails are already the fewest, so `on` changes nothing; `retrace` cuts the paths by 46 % and the plot time by 15 %, for 12 % more pen-down.
- **`truchet` and `mosaic`:** both modes save at most 1.1 % of the plot time, so they stay `off` and their drawings do not move.
- **`contours`:** before contour levels ended at the image edge, every level inked at the border ran along it, and `on` drew those shared stretches once: 12.5 % less pen-down over the six cells. Now that the levels end at the edge there is nothing left to share, all three modes draw the same, and `contours` stays `off`.
- No other style moves by more than a few paths with `on` or `retrace`, since its marks do not meet: dots, packed circles, parallel or spaced lines, or one long path.

The task that asked for this change set the bar as fewer paths and less pen-down with no loss of tone. `on` gives `mesh` fewer paths at the same pen-down and changes nothing for `cells` and `tree`; `retrace` passes the bar on paths and tone but draws more pen-down. It is the default for the three graph styles anyway, because the extra length is ink laid on a line already drawn while the time saved is far larger: over a third of the plot time of `cells`, from about 4,300 fewer pen lifts and less travel per drawing. Fewer lifts also mean fewer landings, which can leave a blot on wet ink. The tone error cannot see the second pass, since the coverage raster counts one pass per path, and neither can the `overdraw` finding. With a pen whose second pass darkens a line, set `order.join` to `on`.

Cells that moved in the gallery: every cell of `cells`, `mesh` and `tree`, 18 of 126. No other cell moved.

**Settled in the build:** the new setting reshuffled the cases the property laws draw from their fixed seed, and the fit law (`a_fit_lands_near_its_target_or_says_how_near_it_can`) found `flow` cases it had not drawn before, with `join` off, so the stage ran exactly as before this change. When a target is out of reach, the law wants the fit no farther from it than the end of the setting's range. `flow`'s minutes are not monotone in its minimum spacing: one case fell from 0.0642 to 0.0639 minutes as the spacing closed from 0.8 to 0.7 mm, the search took that for a cliff and stayed near 0.75 mm, and 0.3 mm gave 0.0700. The law now allows the fit's tolerance or one of the end drawing's average paths, the slack the density law already gives line styles because their lines land on whole steps ([property law fixes](2026-10-16-property-law-fixes.md)). The fit itself is unchanged. The snap distance, 0.001 mm, is fixed rather than a setting: far under any pen's width and far over the rounding between two computations of one point. The plot lint's `lifts` finding offers the patch `{"settings": {"order": {"join": "retrace"}}}` when `join` is not already `retrace` and running the drawing's own paths through the ordering stage again at `retrace` cuts the lifts' share of the plot time by at least a tenth. The tenth is a margin: the patch joins the renderer's paths before simplification and the length filter, so its drawing can differ a little from the finished drawing joined again. Dots, and lines that never meet, get no patch, since joining cannot cut their lifts. `on` is never offered, because `retrace` never gives more paths than `on` and so always a lower lift share. The brief and the changelog said path ordering "merges paths whose ends nearly touch"; it did not until now, and only with `join` on. A preset saved before this change has no `join`, so it takes its style's default, and a `cells`, `mesh` or `tree` preset draws differently from when it was saved.
