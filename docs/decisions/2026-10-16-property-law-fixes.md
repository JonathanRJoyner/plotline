# Property law fixes

**Decided:** 2026-10-16. Amends the brief's "Property laws" row.

**Choice:** The density law now does three more things:

- It turns `tone.response` off, as it already sets `order.min_length_mm` to 0.
- It judges a style that places points on the number of points alone, not on its pen-down length.
- It judges a style that places no points on its length. The denser drawing may fall short of the sparser one by 1 % or by one of the sparser drawing's average paths, whichever is more. This replaces the 0.25 m floor set earlier the same day.

**Reason:** A weekly-scale run (seed 38855027096, 10 times the pull request's cases) on main at 01e136f found three cases where a denser setting drew less. None of them was a wrong `x-density` mark.

- **`stipple` with two pens, light to dark, `tone.response` auto.** The response correction holds the drawing's tone to the image's. As more points darken the style, it lightens the tone map. At 3346 points the map's darkest ink fell below the second pen's band, and that pen drew nothing ("no ink"). So 4065 points placed 4065 dots over both pens while 516 points placed 1032, and the length fell from 0.25 m to 0 m. A density setting cannot promise more marks while the response is held, by design.
- **`mesh` on a 64 by 6 tone map.** Voronoi points cannot be closer than a tone-map pixel, because relaxation drops a point whose cell holds no pixel. Asking for 443, 837 and 2000 points placed 321, 367 and 384 of them, packed onto the pixel lattice. A mesh over a lattice with holes has longer edges than over a fuller one: 321 points drew 3.17 m of line and 367 points drew 2.74 m. The points still rose, and the length of a point style depends on how its renderer joins the points. A rule on the map's pixel count could miss points crowding a small dark area.
- **`waves` with about six lines.** Lines land on whole steps of the spacing, so where they fall moves the length by about one line in six. A spacing of 6.3 mm drew 3.8 % less line than 7.7 mm. The flow case behind the old floor fits the same rule. A flow line ends within one tracing step of the edge, and that step is half the minimum spacing, so a one-line flow drawing came out 0.46 mm longer at the sparser setting. A slack of one average path covers both cases. It still catches a setting that draws much less, such as the hatch cliff the laws found first.

**Any seed:** A new setting reshuffles the cases of the fixed seed, so the density law has to hold for any seed, not only the pull request's. With the rules above, a survey that ran the law's density check at 10 times the cases on eight seeds (1 to 7 and 38855027096), 1,920 cases in all, found no break. Per style, from the registry on main at 01e136f:

- **Judged on the points they place:** `grid`, `halftone`, `dither`, `circles`, `stipple`, `line`, `dashes`, `tree`, `mesh`, `cells` and `scribble`. Their density setting sets the points, and the points rise with it. Their length follows the renderer: a nearest-neighbour `line` tour wobbles with the points, a mesh over packed points has holes, and small dots are single vertices. That is how `line` and `circles` broke the law under other seeds in the paper flattening work.
- **Judged on their length, within one average path:** `hatch`, `waves`, `spiral`, `sketch`, `contours`, `flow`, `hilbert`, `mosaic` and `truchet`. Their lines land on whole steps, so a denser setting can lose about one line where they fall. This covers `sketch`, whose strokes end where the tone runs out.
- **`ink`** has no density setting.

No style needed a declared "approximately monotone" mark: each rule follows from what the setting controls, and no break pointed to a defect in a style.

The pull request seed goes back to `0x706c_6f74_6c69_6e65`, after the paper flattening work moved it to 1 (docs/decisions/2026-10-16-paper-flattening.md). The cases recorded there all fall under the rules above. A nearest-neighbour `line` and `circles` are judged on their points. `sketch` is judged on its length within one average stroke.
