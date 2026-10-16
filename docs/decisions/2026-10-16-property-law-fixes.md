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
