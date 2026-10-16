# plotline spec: the Hilbert curve on rectangles

Date: 2026-10-15. Status: decided by the builder under the owner's standing delegation; written for review.

The `hilbert` stage draws its adaptive curve over one square of side `cell_mm * 2^k`, anchored at the tone map's top-left corner. On a page that is not square the square overhangs the map, and every leaf off the map ends the line. On A4 at the defaults over full ink that gives about 200 pen lifts, many of them shorter than 5 mm (brief §14, "Hilbert curve"). Each lift is a Z move on the owner's printer. This spec replaces the square with a curve native to rectangles.

## 1. The curve

The grid is the tone map cut into cells of `cell_mm`, anchored at the map's top-left corner: `W = ceil(width / cell_mm)` columns and `H = ceil(height / cell_mm)` rows. The last column and row may overhang the map by less than a cell. The curve through the grid is J. Červený's generalized Hilbert curve ("gilbert", <https://github.com/jakubcerveny/gilbert>, BSD-2-Clause), written here from the description in its README. A block of cells is a corner cell and two axis vectors in cells, `a` along the curve's travel and `b` across it, with lengths `w` and `h`. Its curve starts in the corner cell and, when `w` is even or both lengths are odd, ends in the cell at the far end of `a`.

- A block one cell wide or high is walked cell by cell along its length.
- A block with `2w > 3h` is cut across `a` into two blocks with the same `b`, the first `w / 2` long, rounded up to an even length when that is odd and `w > 2`.
- Any other block is cut into three: a block of `h / 2` (made even the same way when `h > 2`) along `b` and `w / 2` across, a block of the rest of `b` along `a`, and a block coming back along `-b` across the far end of `a`.

The root block runs along the longer side. A curve over the whole grid visits every cell once in unit steps. When the longer side has an odd number of cells and the shorter an even number, one diagonal step may be needed; the README shows it cannot be avoided there. Powers of two give the classic Hilbert curve.

## 2. What stays

- **Tone.** A block becomes a leaf, and the line joins the leaves' centres in curve order, unless it is wider than `max_cell_mm` or wider than both `cell_mm` and `cell_mm / max(ink, 1/64)`, where width is the block's longer side in mm and `ink` its mean over the map, as before (L. Velho and J. de M. Gomes, 1991). A block of one cell is always a leaf. Sizes are now whole cells rather than powers of two, so `max_cell_mm` acts in finer steps.
- **Lanes.** Pen `i` of `n` moves the grid up and left by `cell_mm * i / n`, and the grid grows to cover the map from there (brief §14, "Pens kept apart").
- **Empty lanes.** Split between pens, a leaf whose lane ink is under `LANE_MIN_INK` ends the path (brief §14, "Empty lanes").

## 3. What changes

- A leaf's centre that lies off the map, in the overhanging column or row or the strip a lane's move uncovers, is clamped onto the map's edge instead of ending the line. Every cell overlaps the map, so the clamp moves a centre by under half a cell. With one pen the drawing is one path.
- The run reports about 30 frames: the root is split by the rule above, level by level, until there are at least 30 blocks or none splits, and each frame draws the next share of them. The split rule is the same as inside a block, so the frames do not change the drawing.
- The stage's version goes from 2 to 3.

## 4. Checks

- A unit test walks the curve over every grid from 1 by 1 to 40 by 40: each cell once, unit steps, at most one diagonal step and none unless the longer side is odd and the shorter even, the start in the first corner, and the end at the far end of the longer side when there is no diagonal case.
- A black A4 page at the defaults draws one path of 317 by 462 cells. A map half black and half white, split between two pens, draws the first lane as one path over the black half.
- A new golden file, `hilbert_gradient.json`, pins the curve. There was no hilbert golden file before.
- In the regression gallery against `main`, only the `hilbert` cells change.
