# Path simplification

Status: built on 2026-10-16. The decision is in [docs/decisions/2026-10-16-path-simplification.md](../../decisions/2026-10-16-path-simplification.md).

## Goal

Some styles sample their curves far finer than any pen can show: waves, spiral, flow, contours and ink. A pen-width finding of the plot lint, `vertices`, already flags a layer where over half of at least 1,000 segments are shorter than an eighth of the pen. Those vertices cost file size, plotter buffer and, on some firmware, speed, and they show nothing. Brief §12 question 9 listed Ramer–Douglas–Peucker simplification in the ordering stage as a candidate after 0.1. This spec builds it.

## Setting

The `order` stage gains `simplify_mm`: a number from 0 to 1 mm, step 0.01, unit mm, default **0**, which is off. Its help text:

> Remove vertices that lie within this distance of the simplified line; 0 keeps every vertex. An eighth of the pen width cannot be seen at the pen's own width.

**Why the default stays 0:** a preset saved before this change has no `simplify_mm`, so it takes the default. Any other default would change every drawing made from an old preset, every golden file and the stats snapshot. The setting exists so that a user, or the lint fix, can turn it on. The decision file records the measurements behind the choice.

## Stage

Before the length filter, the order stage passes each path of three or more vertices through `raster::simplify(path, simplify_mm)`. That function is Ramer–Douglas–Peucker (D. H. Douglas and T. K. Peucker, "Algorithms for the reduction of the number of points required to represent a digitized line or its caricature", *The Canadian Cartographer* 10(2), 1973, pp. 112–122), which `raster.rs` already cites. It keeps the end points, so a closed path stays closed, and a path of one or two vertices passes through unchanged. Ordering and the length filter then work on the simplified paths. At 0 nothing is simplified and the output is bit for bit what it was.

## Lint

The `vertices` finding gains a patch, `{"settings": {"order": {"simplify_mm": t}}}`, where `t` is the narrowest pen's width divided by 8 and rounded down to 0.001 mm, and never less than 0.001. Its message adds " Path order's simplify setting removes them." When `simplify_mm` is already at least `t`, the finding has no patch, as other findings do once their fix is in effect. The severity stays `info`. The test that every lint patch fixes its finding must cover the new patch.

## Law

`crates/core/tests/laws.rs` gains a law for `raster::simplify` over random polylines and tolerances:
- the output keeps the first and last vertices;
- it is a subsequence of the input, with no more vertices;
- every input vertex lies within the tolerance (plus 1e-9) of the output polyline;
- a tolerance of 0 drops only vertices exactly on the line between their neighbours, so a line with no collinear vertices comes back unchanged.

## Measurement

The style guide's case is the still life at 600 by 875 px on A4 with a 0.3 mm pen. For each listed style, draw it at the defaults with `simplify_mm` at 0 and at 0.0375, which is the pen width over 8. Record in the decision file:
- the vertex count;
- the SVG size in bytes;
- the largest per-pixel difference between the two previews at the gallery's width.

A throwaway program or ignored test can do the measuring; do not commit it. The gallery's committed snapshot must not change, because the default does not.

## Docs

- `changelog.d/path-simplification.md` with `Section: Output`.
- The decision file.
- In the brief, §12 question 9 notes that the simplification candidate was built on 2026-10-16 and points to the decision file, and §7.1's path ordering paragraph mentions simplification.
