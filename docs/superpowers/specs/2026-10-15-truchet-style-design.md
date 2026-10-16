# The Truchet style

Date: 2026-10-15. Status: decided by the builder under the owner's standing delegation; written for review.

Brief §7.2 lists regions and tiles among the later families. This spec adds the `truchet` style: square tiles, each holding quarter-circle arcs in one of two turns chosen at random, so that the arcs join from tile to tile into winding curves. Tone sets how many nested arcs a tile holds.

## 1. Source

- S. Truchet, "Mémoire sur les combinaisons", *Mémoires de l'Académie Royale des Sciences*, 1704, pp. 363-372: square tiles split by a diagonal, laid in different turns.
- C. S. Smith and P. Boucher, "The tiling patterns of Sebastien Truchet and the topology of structural hierarchy", *Leonardo* 20(4), 1987, pp. 373-385. At its end Smith gives the variant used here: each tile holds two quarter-circle arcs, centred on two opposite corners, that join the midpoints of adjacent sides. Laid in random turns, the arcs connect into continuous curves.
- C. Carlson, "Multi-scale Truchet patterns", *Proc. Bridges* 2018, pp. 39-44, was read and not followed: its winged tiles complete their motif by painting smaller tiles over larger ones, which a pen cannot do.

Nesting several arcs about each corner, and choosing their number from the tone, is this project's own rule. Smith's arcs have radius `s / 2` on a tile of side `s`. Arcs of radius `s / 2 + j g` about the same corner, for `j` from `-m` to `m`, meet each side at `j g` either side of its midpoint. Every tile's arcs meet a side at the same places, so two neighbouring tiles with the same count join all their arcs, and where the counts differ the extra arcs end at the side.

## 2. Style

`truchet`, label "Truchet tiles", description "Quarter-circle arcs on square tiles in random turns, joining into winding curves, with more nested arcs where the image is darker."

Stages: `tone`, `split`, `truchet`, `order`. Like `waves` and `spiral`, the style sets `tone.gamma` to 1.9 (the registry's `RIPPLE_GAMMA`, brief §14 "Rippling gamma"), because its ink grows in proportion to the tone value: the arc count is proportional to the ink.

## 3. The `truchet` stage

New file `crates/core/src/stages/truchet.rs`. Name `truchet`, label "Truchet tiles", version 1. Requires `Tone`; provides `Paths`.

### 3.1 Settings

Every help text is one line.

| Key | Help | Range | Default |
|---|---|---|---|
| `tile_mm` | Side of one square tile. | 2 to 40, step 0.5, mm | 8 |
| `spacing_mm` | Distance between nested arcs; closer arcs fit more into a tile and draw darker. | 0.1 to 5, step 0.05, mm, `"x-density": "inverse"` | 0.4 |
| `seed` | Turns of the tiles. | 0 to 9999 | 0 |

The defaults are starting values. The builder tunes them on the README still life with a 0.3 mm pen so that the drawing reads clearly, and records the final values in the brief row. `spacing_mm` is the density setting for the budget solver, and its name lets plot lint's spacing rules apply.

### 3.2 Tiles

Write `s = tile_mm` and `g = spacing_mm`. The tiles cover the tone map's rect from its top-left corner, so the last column and row may overhang the map (see §3.5 for other lanes).

The most arcs a corner can nest without meeting the opposite corner's arcs: `m_max = floor(((sqrt 2 - 1) s / g - 1) / 2)`, at least 0. Then the outermost arcs of the two corners stay at least `g` apart along the tile's diagonal.

For each tile, in row-major order over the covering grid:

1. Draw its turn from the stage's generator, `ctx.rng(seed)`, for every tile whether or not it has ink, so that a change of tone never reshuffles the turns. Turn A puts the arcs about the top-left and bottom-right corners, turn B about the top-right and bottom-left.
2. Its ink `t` is the mean of `tone.sample` at the centres of a 4 by 4 grid of sub-squares of the tile.
3. With `n = t (2 m_max + 1)`, the tile holds `k = 0` arcs per corner when `n < 0.5`, else the odd number nearest `n`: `k = 2 floor((n - 1) / 2 + 0.5) + 1`, at most `2 m_max + 1`. Write `k = 2 m + 1`.
4. About each of its two corners it holds the quarter arcs of radius `s / 2 + j g`, `j = -m..m`, each from one side to the other. A quarter arc is 16 segments, a 64th of a turn each, the cap of the stipple dots (brief §14, "Dot geometry").

### 3.3 Joining

An arc ends on a side at a point the side's two tiles share. Each such point gets one id from the side (which grid edge, horizontal or vertical, by its integer tile coordinates) and its offset index from the side's midpoint, so both tiles name it alike without comparing floats. The arcs are the edges of a graph on those points. Each point has at most one arc from each of its two tiles, so the graph's trails (`graph::trails`) are its curves: chains between loose ends and closed loops. Each trail is drawn as one path through its arcs' vertices, each arc reversed where the trail runs it backwards, with a shared endpoint written once. The paths are then clipped to the tone map's rect with `graph::clip_polyline`, which cuts the overhanging tiles.

### 3.4 Steps and frames

The run computes the tiles in chunks of rows, then the trails, with about 30 frames. A frame while tiles are being built can hold the arcs so far as separate paths; the last frame and the artifact hold the joined paths. Any split into steps that keeps each step short on A3 at the smallest tile and spacing is fine.

### 3.5 Pens kept apart

Lane `i` of `n` turns its tile grid by `90 i / n` degrees about the tone map's centre, and draws its turns from its own random stream (`RunContext::rng`). A square grid repeats every quarter turn, so the lanes' grids are spread evenly over it, as `hatch` spreads its levels over the half turn. Lane 0 keeps the unturned grid, so one pen draws as it would alone. In a turned lane the covering grid is the one whose turned tiles cover the map's rect; tiles whose 16 samples all fall off the map are still given their turn, and their arcs are dropped. The registry test `every_style_but_ink_keeps_its_pens_apart` covers the rule.

### 3.6 Empty lanes

No lift rule: a tile with too little ink holds no arcs.

## 4. Tests

In `truchet.rs`:

- Settings schema: ranges, defaults, the `x-density` mark, name and label.
- `m_max`: for `s = 8, g = 0.4` it is 3, and the outermost arcs of a tile's two corners stay at least `g` apart.
- White gives no paths; full ink gives `2 m_max + 1` arcs per corner in every tile.
- Over full ink on a map a whole number of tiles wide and high, every curve is a closed loop or ends on the map's edge: no path ends inside the map.
- Two tiles with different counts: the matching arcs join, the extra ones end at the shared side.
- A change of tone does not change any tile's turn (compare the turns of two tone maps with the same seed).
- A second lane's grid is turned: with one tile size, its arcs' centres are not on lane 0's grid.
- Every vertex stays inside the tone rect.
- Determinism: same inputs, same coords.

Elsewhere:

- `registry.rs`: `lists_builtins_and_hides_test_styles` lists `truchet` after `hilbert`, and `rippling_styles_darken_their_midtones` checks its gamma of 1.9. The tests over every listed style then cover it.
- `crates/core/tests/golden.rs`: a case `truchet_gradient` on `gradient(64, 48)` with `tone.resolution` 64.
- `crates/core/tests/settings_matter.rs`: add exceptions only with a reason, as the existing ones have.
- The Python tests that list the styles (`tests/test_core_binding.py`, `tests/test_session.py`).

## 5. Documents

- README: the style count, a row in the style table, and a picture cell in a third row of the picture table, from `cargo run --release -p plotline-core --example readme_images` (commit only `docs/images/truchet.png`, and check that the committed pictures did not change).
- CHANGELOG 0.1.0: the style count, and `truchet` in the Lines line.
- Brief §14: a row "Truchet style" with the decision, the sources and the tuned defaults; amend "Rippling gamma" to name `truchet`.
