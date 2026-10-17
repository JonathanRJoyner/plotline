# What a density setting drives

**Decided:** 2026-10-16. Amends [the property law fixes](2026-10-16-property-law-fixes.md).

**Choice:** A density setting may declare in its schema what it drives, as `"x-density-of"` with the value `points`, `lines` or `length`, and the density law judges that measure:

- `points`: the points placed;
- `lines`: the path count, within 1 %;
- `length`: the pen-down length, within one of the sparser drawing's average paths.

A setting without the mark keeps the earlier rule: points for a style that places points, length otherwise. `waves`, `spiral` and `mosaic` declare `lines` for their `spacing_mm`.

The plot lint's `dark` and `empty` patches offer `truchet` a closer `spacing_mm` only when it nests more arcs in a tile. Its `overdraw` patch raises a spacing only where `stipple.size_mm`, the widest dot, fits in the new spacing.

**Reason:** A survey of the density law on seven seeds at 10 times the pull request's cases, on main after the tone stage's new settings, found six breaks. All were pen-down length, and in every one the path count rose or held:

- **`waves` and `spiral` (five breaks).** Their ripple swings by `amplitude × spacing_mm / 2`, so closer lines carry smaller ripples. Where the ripple dominates (amplitude near 1, a 0.2 mm wavelength, dark tone), the length per line falls as fast as the line count rises, and the total stops following the spacing. One waves drawing went from 0.99 m in 4 paths to 0.53 m in 18. Another spiral went from 10.72 m in 82 paths to 9.39 m in 209.
- **`mosaic` (one break).** It joins a patch's lines into one path when the gap is within twice `spacing_mm`, an absolute reach. A sparser spacing joins with longer connectors: 2.53 m against 2.17 m, in 112 paths either way.

None of these is a defect. In all three the spacing sets how many lines are drawn, which the mark now says. A setting carries the mark itself, so a new style declares its own measure and the law needs no list of styles.

`truchet` nests its arcs `spacing_mm` apart within half a tile. With 2 mm tiles every spacing of 1 mm or more gives one arc a corner, so the `dark` patch from 5 mm to 2.7 mm changed nothing, at seeds 1 and 38855027096. Truchet's own count of the arcs that fit decides the patch now.

At seed 1 the lint law then found `circles` with 10 mm dots at a 0.2 mm spacing. The dots overlap at any spacing, so raising the spacing by half left the overdraw at 1. With all these rules, the density and lint laws passed at 10 times the cases on seeds 1, 3 and 38855027096 and on the pull request's own seed.
