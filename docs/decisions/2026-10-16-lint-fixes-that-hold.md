# Lint fixes that hold

**Decided:** 2026-10-16. Amends the brief's rows "Plot lint", "Lint thresholds" and "Pen use", and closes issue #32.

**Choice:** Every plot lint patch now keeps the property law: applied, it removes its finding or moves the finding's number the right way. A finding offers no patch when a rule cannot tell that its patch acts. The law no longer skips any code.

- **`unused_pens`** fires only when Pen use is one pen and there are more pens, and its patch, light to dark, always removes it. Pens that draw nothing in another mode get their own code, `idle_pens`: info, no patch, same message.
- **`travel`** offers `order.optimize` only when it cuts the travel. Lint runs the ordering stage on each layer and measures the result. That is exactly the drawing the setting would make, because with `optimize` off the stage keeps a layer's paths in input order.
- **`empty`** reads the ink laid, each path's length times its pen width plus a pen-wide disc, over the drawing area, instead of the coverage raster's share. Its patch acts only where marks can appear:
  - with no paths, a threshold lowered to the ink of the map's darkest pixels where enough of them lie to draw;
  - with paths, the density setting one step denser where the marks' pixels have room.
  It offers none for a blank tone map, a minimum length above 0, Pen use by colour or a held tone response. Auto levels is no longer offered: the drawing keeps the tone map after levels, so nothing in it shows whether auto levels would act.
- **`dark`** offers closer spacing only where the dark parts have room for it. It caps a raised `voronoi.count` at a quarter of the inked pixels, with at least 16 new points in the dark parts, and raises `sketch.max_lines` only when a layer drew its limit. It offers no patch with a minimum length above 0, or with a pen thinner than a raster cell.

**Reason:** Issue #32 listed the misses the law found:
- **`empty`:** the raster undercounts pens under 0.3 mm, and dots from pens under about 0.1 mm count as no ink at all. Its threshold patch did nothing when every grid point already drew.
- **`unused_pens`:** after light to dark, the idle pens reported the same code with an equal or higher number.
- **`dark`:** a minimum length drops the shorter marks more points make. Voronoi points cannot be closer than a tone-map pixel, so more `count` adds nothing on a coarse map.
- **`travel`:** a nearest-neighbour order can equal the input order, such as grid dots emitted row by row.

Each rule above came from a shrunk counterexample. Examples: `poisson` at 7.1 mm spacing, `scribble` at 361 points, `waves` with two lines on a page, `truchet` at 40 mm tiles, `sketch`, `dither` and `halftone`. The room rule asks for 16 spacings squared of dark area, measured at the density stage's largest `_mm` setting, because a `truchet` tile or a `scribble` loop is wider than its spacing. Four was too few: a `waves` page with two lines missed. The laid ink moved six gallery cells, all with a 0.1 mm pen drawing dots only. Five of them, with 11,282 to 37,523 dots laying 0.17 % to 0.56 %, now report `dark` instead of `empty`. `grid` on the still life, at 0.093 %, stays empty.

**Verification:** The lint law passed at 10 times the pull request's cases on the default seed and on 38855027096. A further 3000 cases per seed found no miss. The patches offered are fewer than before: about 30 `empty` and 14 to 21 `dark` patches in 3000 cases, on the law's deliberately hostile pages, pens and minimum lengths.
