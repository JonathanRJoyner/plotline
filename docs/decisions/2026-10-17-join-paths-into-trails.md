# Joining paths into trails

**Decided:** 2026-10-17

**Choice:** The `order` stage has a new setting, `join`, with three values. `off` leaves the paths as the style drew them. `on` snaps the vertices of the paths that lie within 0.001 mm of each other into one graph, draws each distinct edge once, and splits the graph into the fewest trails it allows: for a connected part with `k > 0` odd vertices that is `k / 2`, the bound J. Edmonds and E. L. Johnson start the postman problem from ("Matching, Euler tours and the Chinese postman", *Mathematical Programming* 5, 1973, pp. 88–124). The trails come from Euler circuits found as C. Hierholzer describes (*Mathematische Annalen* 6, 1873, pp. 30–32), after pairing the odd vertices of each part with virtual edges, and cut at those edges. `retrace` does the same after drawing some edges twice: the edges whose two ends are both odd, shortest first, while their part still has four or more odd vertices. Each edge so doubled saves one trail. Edmonds and Johnson double the shortest set of paths that pairs all the odd vertices, found by weighted matching; the greedy pass is cheaper and takes only single edges. The join runs first in the stage, before simplification, the length filter and the ordering, and the code is `crates/core/src/stages/join.rs`.

The default is `off`, except through the style defaults: **`retrace` for `cells`, `mesh` and `tree`, and `on` for `contours`**.

If the trails would be more paths than the input, which can only happen when the input draws an edge twice on purpose, the stage keeps the input. Dots, and paths whose vertices all snap to one point, pass through unchanged. A law in `crates/core/tests/laws.rs` checks that both modes never give more paths than the input, keep every segment and every dot, add no segment, and that `retrace` never gives more paths than `on`.

**Reason:** The request behind this change said `cells` drew every shared edge twice, one closed loop per cell. It does not: the cells renderer already walks the Voronoi graph with `graph::trails` and draws each edge once. Every corner of a Voronoi cell joins three edges, so every vertex is odd and about 5,000 paths per drawing is already the Edmonds–Johnson bound. `on` cannot lower it, and the gallery shows that. Only drawing some edges twice can, which is why `retrace` exists and why the three graph styles take it.

The gallery draws each style on three images at two pen widths, six cells per style. The table sums those six cells at `off` (the drawings before this change), `on` and `retrace`; tone error is the mean, and "lifts" counts the cells where the plot lint's `lifts` finding fires. Plot time uses the default speeds: 50 mm/s drawing, 100 mm/s travel and 0.2 s per pen lift.

| Style | `join` | Paths | Pen-down m | Pen-up m | Plot time min | Tone error | `lifts` |
|---|---|---:|---:|---:|---:|---:|---:|
| `cells` | off | 29,962 | 172.9 | 67.3 | 168.7 | 0.105 | 6/6 |
| `cells` | on | 29,960 | 172.9 | 68.7 | 169.0 | 0.105 | 6/6 |
| `cells` | **retrace** | **4,170** | 205.6 | 27.9 | **87.1** | 0.105 | **0/6** |
| `mesh` | off | 5,650 | 296.3 | 31.0 | 122.8 | 0.114 | 0/6 |
| `mesh` | on | 5,430 | 296.3 | 30.0 | 121.9 | 0.114 | 0/6 |
| `mesh` | **retrace** | **1,096** | 309.6 | 13.3 | **109.1** | 0.114 | 0/6 |
| `tree` | off | 7,816 | 89.4 | 39.3 | 62.4 | 0.249 | 0/6 |
| `tree` | on | 7,816 | 89.4 | 38.5 | 62.3 | 0.249 | 0/6 |
| `tree` | **retrace** | **4,246** | 99.8 | 27.3 | **52.0** | 0.249 | 0/6 |
| `contours` | off | 124 | 52.1 | 2.5 | 18.2 | 1.852 | 0/6 |
| `contours` | **on** | **108** | **45.6** | 3.0 | **16.1** | 1.852 | 0/6 |
| `contours` | retrace | 106 | 46.0 | 3.2 | 16.2 | 1.852 | 0/6 |
| `truchet` | off | 8,880 | 175.1 | 36.6 | 94.1 | 0.129 | 0/6 |
| `truchet` | on | 8,852 | 175.1 | 36.6 | 94.0 | 0.129 | 0/6 |
| `truchet` | retrace | 8,852 | 175.1 | 36.6 | 94.0 | 0.129 | 0/6 |
| `mosaic` | off | 13,910 | 325.1 | 33.0 | 160.2 | 0.050 | 0/6 |
| `mosaic` | on | 13,592 | 325.1 | 33.8 | 159.3 | 0.050 | 0/6 |
| `mosaic` | retrace | 13,130 | 328.8 | 32.2 | 158.7 | 0.050 | 0/6 |

- **`cells`:** `retrace` cuts the paths by 86 % and the plot time by 48 %, and the `lifts` finding clears in all six cells. The cost is 19 % more pen-down length: those edges are drawn twice, over a line already there.
- **`mesh`:** `retrace` cuts the paths by 81 % and the plot time by 11 %, for 4.5 % more pen-down. `on` alone saves 4 % of the paths.
- **`tree`:** a tree's greedy trails are already the fewest, so `on` changes nothing; `retrace` cuts the paths by 46 % and the plot time by 17 %, for 12 % more pen-down.
- **`contours`:** some stretches of line are drawn more than once, by several levels. `on` draws them once: 12.5 % less pen-down and 12 % less plot time over the six cells, and 15 to 18 % less pen-down in the four that move (the gradient and the scene at both widths). The still life does not move. `retrace` gains two paths for more ink, so `on` is the better default.
- **`truchet` and `mosaic`:** both modes save under 1 % of the plot time, so they stay `off` and their drawings do not move.

The task that asked for this change set the bar as fewer paths and less pen-down with no loss of tone. `on` meets it for `contours`, gives `mesh` fewer paths at the same pen-down and changes nothing for `cells` and `tree`; `retrace` passes the bar on paths and tone but draws more pen-down. It is the default for the three graph styles anyway, because the extra length is ink laid on a line already drawn while the time saved is far larger: about half the plot time of `cells`, from about 4,300 fewer pen lifts and less travel per drawing. Fewer lifts also mean fewer landings, which can leave a blot on wet ink. The tone error cannot see the second pass, since the coverage raster counts one pass per path, and neither can the `overdraw` finding. With a pen whose second pass darkens a line, set `order.join` to `on`.

Cells that moved in the gallery: every cell of `cells`, `mesh` and `tree` (18), and `contours-gradient` and `contours-scene` at both widths (4), 22 of 126. No other cell's stats moved. The pictures of 20 changed: those two `contours` cells at the 0.5 mm pen draw the same pixels, since a line drawn once looks like the same line drawn twice.

**Settled in the build:** the new setting reshuffled the cases the property laws draw from their fixed seed, and the fit law (`a_fit_lands_near_its_target_or_says_how_near_it_can`) found `flow` cases it had not drawn before, with `join` off, so the stage ran exactly as before this change. When a target is out of reach, the law wants the fit no farther from it than the end of the setting's range. `flow`'s minutes are not monotone in its minimum spacing: one case fell from 0.0642 to 0.0639 minutes as the spacing closed from 0.8 to 0.7 mm, the search took that for a cliff and stayed near 0.75 mm, and 0.3 mm gave 0.0700. The law now allows the fit's tolerance or one of the end drawing's average paths, the slack the density law already gives line styles because their lines land on whole steps ([property law fixes](2026-10-16-property-law-fixes.md)). The fit itself is unchanged. The snap distance, 0.001 mm, is fixed rather than a setting: far under any pen's width and far over the rounding between two computations of one point. The setting is not offered as a patch by the `lifts` finding yet. The brief and the changelog said path ordering "merges paths whose ends nearly touch"; it did not until now, and only with `join` on. A preset saved before this change has no `join`, so it takes its style's default, and a `cells`, `mesh`, `tree` or `contours` preset draws differently from when it was saved.
