# plotline spec 11: keeping every style's pens apart

Date: 2026-10-15. Status: decided by the builder under the owner's standing delegation; written for review.

[Spec 8 on pen use](2026-10-12-pen-use-design.md) runs the stages after the split once per pen lane, and its §5 spreads `hatch`, `waves`, `spiral` and `grid` across the lanes, so that two pens do not draw on top of each other. The point family and more styles ([the other spec 8](2026-10-12-point-family-and-more-styles-design.md)) were built at the same time on another branch, so their thirteen styles got no spreading rule of their own; only `halftone` spread, through the grid placer it shares with `grid`. Where two lanes of the others both hold ink, their marks land on each other. The brief's "Pens kept apart" row records the gap. This spec closes it.

## 1. Rules

Every rule reads the lane, `index / count`, which is already in a lane stage's cache key. No rule adds a setting. In the first lane, and so with one pen, every rule gives exactly the output it gave before, so no golden file changes.

| Stage | Styles | Lane `i` of `n` |
|---|---|---|
| `voronoi` | `stipple`, `line`, `dashes`, `tree`, `mesh`, `cells` | draws its scatter from its own random stream |
| `poisson` | `circles` | draws its packing from its own random stream |
| `sketch` | `sketch` | draws its stroke angles from its own random stream |
| `dither` | `dither` | moves its grid by `spacing * i / n` across and down, as `grid` does |
| `points` (the grid placer) | `halftone` | no change: it already moves its grid, see §2 |
| `contours` | `contours` | traces level `j` at `(j + i / n) / (levels + 1)` |
| `flow` | `flow` | starts each line that has no neighbour yet `i / n` of its separation to the side |
| `hilbert` | `hilbert` | moves the root square up and left by `cell_mm * i / n` on both axes |

The renderers (`stipple`, `tsp`, `dashes`, `mst`, `delaunay`, `cells`, `render`) do not change: they draw the points their lane was given, and those now differ.

`Stage::version` is bumped whenever the output for the same inputs and settings changes, and a lane is one of a lane stage's inputs. So `voronoi`, `poisson`, `sketch`, `dither`, `contours`, `flow` and `hilbert` go to version 2. The grid placer's output does not change, so it keeps version 1.

### 1.1 Random streams

`RunContext::rng(seed)` seeds lane `i` with `seed + i * 2^32`. Lane 0 keeps `seed`. Every seed setting is a `u32`, so no other lane ever repeats a scatter that some seed gives the first lane. The change sits in the one function every stage draws randomness from, so `voronoi`, `poisson` and `sketch` need no edit of their own, and a later random stage spreads too.

Independent random patterns, one per ink, are what the Demichel equations of the colour model assume: I. Amidror and R. D. Hersch, "Neugebauer and Demichel: dependence and independence in n-screen superpositions for colour printing", *Color Research and Application* 25(4), 2000. Relaxation keeps each lane's points evenly spread, but it does not tie them to another lane's, so where two lanes hold ink their marks fall independently.

`sketch` starts each lane's first stroke at the same darkest pixel, because the start is chosen by tone and not at random. From the second vertex the angles differ.

### 1.2 Moved lattices

`dither` scans the grid placer's cells, so it takes the grid placer's rule: the grid moves along the diagonal by its share of a cell and keeps the whole cells that still fit. One helper in `grid.rs` gives both stages their cells. Two lattices of the same period and angle superpose with no moiré, whatever their offset (I. Amidror, *The Theory of the Moiré Phenomenon*, 2nd ed., Springer, 2009). The offset keeps each lane's dots off the other lanes' dots.

`hilbert` moves its root square the same way, by a share of `cell_mm`. The leaves of full ink are `cell_mm` wide, so two lanes' finest folds interleave, and the coarser folds are offset by the same distance. The root square is grown until it covers the map from its moved corner, so no strip at the right or the bottom is left out.

### 1.3 Shifted levels

Over a smooth change of tone, equally spaced contour levels draw a family of near-parallel lines, a line grating. Two lanes over similar tone draw two gratings with the same period, and shifting one lane's levels by its share of the level step puts its lines between the other lane's instead of on them, the same reasoning as for lattices (Amidror, 2009). The top level of every lane stays under full ink.

### 1.4 Sideways seeds

The evenly spaced streamlines of B. Jobard and W. Lefer ("Creating evenly-spaced streamlines of arbitrary density", 1997) seed each new line one separation to the side of a line already drawn. A lane's lines therefore follow from the first line and from the coarse-grid points used when the seeds run out. Lane `i` moves each of those starting points `i / n` of the separation there to the side, along the normal of the field. The lines seeded from them keep the usual separation, so where two lanes' fields agree the lanes' lines interleave. A move that would leave the rect is not made, and neither is one where the field has no direction. A moved coarse-grid point that lands too close to a line is skipped, as any seed is.

## 2. Halftone needs no new rule

`halftone` places its dots with the grid placer, the `points` stage that `grid` uses, and that stage already moves its grid per lane. Its dots therefore already sit apart. The brief's row said otherwise because it was written before the two branches met. A test pins it now, like every other style.

Turning each lane's screen, as colour printing does, was considered and left out. The `points` stage is shared with `grid` by name, so a rotation for `halftone` alone would need a second placer under a new stage name, which moves `halftone`'s settings to a new preset key, or a rotation for `grid` as well, which changes a rule already settled. Rotated screens make the colour robust when the inks are printed out of register. Pens on one plotter register well, and a constant offset between two screens of the same period and angle shifts the whole page evenly instead of making a pattern.

## 3. Tests

- `stage.rs`: lane 0's generator equals the generator outside a split, and lane 1's differs.
- `grid.rs`: the existing test of the staggered grid covers the helper.
- `dither.rs`, `contours.rs`, `flow.rs`, `hilbert.rs`: one lane gives the old output, two lanes give different marks, and every vertex stays inside the tone rect.
- `registry.rs`: for every listed style except `ink`, the stages between the split and path order, run at the style's defaults over one tone map, give the same paths in lane 0 of 2 as with one pen. In lane 1 of 2, at most one in fifty of the vertices off the map's edge lands exactly on one of lane 0's. The map is a cone of ink over a floor of 0.15, so the lift over empty lanes plays no part, at 0.2 mm a pixel, about 24 pixels to each Voronoi point as on real maps. On a map coarser than the points, relaxation puts many points on the same pixel centres whatever the seed. Vertices on the map's edge are left out because lines clamped onto it meet there in any lane. Turned lines cross, and a crossing can be a vertex of both lanes: about one in a hundred of the waves' vertices are.
- The golden files do not change.
- The regression gallery, run against a baseline from `main`, shows no changed cell, since it draws with one pen.

## 4. Out of scope

- A setting to turn spreading off, or to choose the rule.
- Rotated screens for `halftone` (§2).
- `ink` and `order`, which keep no spreading rule, as spec 8 decided.
- Lines clamped onto the map's edge, such as a contour level that reaches it. Every level that reaches the edge runs along it in one pen as well.
