# The mosaic style

Date: 2026-10-15. Status: decided by the builder under the owner's standing delegation; written for review.

Brief §7.2 lists "regions and mosaic": split the image into regions and render each its own way. This spec adds the `mosaic` style. It cuts the image into patches that follow its shapes and fills each patch with parallel lines at the patch's own angle, spaced by the patch's tone. The result reads as a patchwork of hatching.

## 1. Sources

- **Regions.** SLIC superpixels: R. Achanta, A. Shaji, K. Smith, A. Lucchi, P. Fua and S. Süsstrunk, "SLIC superpixels compared to state-of-the-art superpixel methods", *IEEE Transactions on Pattern Analysis and Machine Intelligence* 34(11), 2012, pp. 2274-2282, doi:10.1109/TPAMI.2012.120. Cluster centres start on a grid of interval `S`, move to the lowest gradient in their 3 by 3 neighbourhood, then local k-means runs: each pixel joins the nearest centre among those whose `2S` by `2S` window holds it, by the distance `D = sqrt(dc^2 + (ds / S)^2 m^2)`, and each centre moves to the mean of its pixels. Ten passes suffice. A last step relabels disjoint segments with the label of the largest neighbouring cluster, so every region is connected. The paper works in CIELAB with `m` from 1 to 40; plotline has one channel, the ink, and measures `dc` as `100 |ink_a - ink_b|`, the range of CIELAB lightness, so `m` keeps the paper's meaning.
- **Direction.** Each patch's lines run along its long axis, the principal axis of its pixels' second central moments: `theta = atan2(2 mu11, mu20 - mu02) / 2`. The moments are those of M.-K. Hu, "Visual pattern recognition by moment invariants", *IRE Transactions on Information Theory* 8(2), 1962, pp. 179-187.
- **Tone.** Tone is shown by the spacing of the lines, the way stroke textures show it in G. Winkenbach and D. H. Salesin, "Computer-generated pen-and-ink illustration", *Proc. SIGGRAPH* 1994, pp. 91-100. Lines of width `w` at spacing `d` cover `w / d` of the paper, so spacing `spacing_mm / t` over ink `t` gives a cover in proportion to the ink.

## 2. Style

`mosaic`, label "Mosaic", description "The image cut into patches that follow its shapes, each filled with parallel lines at its own angle, closer together where it is darker."

Stages: `tone`, `split`, `mosaic`, `order`. Like `waves` and `spiral`, the style sets `tone.gamma` to 1.9 (the registry's `RIPPLE_GAMMA`, brief §14 "Rippling gamma"), because its cover grows in proportion to the tone value.

## 3. The `mosaic` stage

New file `crates/core/src/stages/mosaic.rs`. Name `mosaic`, label "Mosaic", version 1. Requires `Tone`; provides `Paths`.

### 3.1 Settings

Every help text is one line.

| Key | Help | Range | Default |
|---|---|---|---|
| `region_mm` | About how wide one patch is. | 2 to 50, step 0.5, mm | 8 |
| `compactness` | Higher keeps the patches round; lower lets them follow the image's shapes. | 1 to 40, step 1 | 10 |
| `spacing_mm` | Distance between lines over full ink; lighter patches space theirs wider. | 0.2 to 10, step 0.1, mm, `"x-density": "inverse"` | 0.6 |

The defaults are starting values. The builder tunes them on the README still life with a 0.3 mm pen so that the drawing reads clearly, and records the final values in the brief row. `spacing_mm` is the density setting for the budget solver, and its name lets plot lint's spacing rules apply.

### 3.2 Regions

On the lane's tone map, in pixels, with `S = max(2, region_mm / mm_per_px)` and `m = compactness`:

1. **Seeds.** `nx = max(1, round(width / S))` by `ny = max(1, round(height / S))` centres, centre `(i, j)` at `((i + 0.5) width / nx, (j + 0.5) height / ny)`, rounded down to a pixel, in row-major order. Each moves to the pixel of lowest `G(x, y) = (I(x+1, y) - I(x-1, y))^2 + (I(x, y+1) - I(x, y-1))^2` in its 3 by 3 neighbourhood, reading clamped at the map's edge; ties go to the first in row-major order.
2. **Ten passes.** Each pass labels every pixel with the nearest centre whose window of `2S` by `2S` about it holds the pixel, by `D^2 = (100 dc)^2 + (ds / S)^2 m^2`, ties to the lower centre index; a pixel in no window keeps its label. Then each centre moves to the mean `x`, `y` and ink of its pixels; a centre with none stays where it is.
3. **Connectivity.** The 4-connected components of the labels, numbered in row-major order of their first pixel, become the regions. In component order, each component under `S^2 / 4` pixels joins the neighbouring component with the most pixels at that moment (ties to the lower number), whose size grows by its own. The `S^2 / 4` floor is this project's choice; the paper does not give one. The regions are renumbered in the order of their first pixel.

### 3.3 Lines

For each region `r`, in order:

1. Its ink `t` is the mean ink of its pixels, its centre `c` the mean of its pixel centres in mm, and its angle the principal axis from §1, turned into the page's convention (counter-clockwise as seen on the page, brief §14 "Angles"; the page's y axis points down), plus the lane's turn (§3.5).
2. With `t > 0`, the spacing is `d = spacing_mm / t`. Lines at that angle lie at offsets `(k + 1/2) d` from `c` along the normal, for every whole `k` that meets the region's bounding box. A region narrower than `d` across its lines gets none, so a light patch is drawn light on average instead of always getting one line.
3. Each line is sampled every half pixel across the region's bounding box, and every run of samples whose pixel belongs to `r` becomes a segment from its first sample to its last.
4. The segments are drawn back and forth: in offset order, every second line reversed. A segment joins the path before it when its start is within `2 d` of that path's end and the midpoint of the join lies in `r`; otherwise it starts a new path. One patch then usually costs one pen lift.

Every vertex is a sample of the map, so it lies inside the map's rect.

### 3.4 Steps and frames

The seeds are one step, each pass of §3.2 is one step whose frame holds the centres as points in mm, labelled `pass k of 10`, connectivity is one step, and the lines are made a chunk of regions at a time with about 30 frames holding the paths so far. On a map of the default resolution no step should take much over 100 ms natively; split a pass into chunks of centres if it does.

### 3.5 Pens kept apart

Lane `i` of `n` turns every patch's lines by `180 i / n` degrees, as `hatch` spreads its levels over the half turn, so two pens over the same tone draw crossing lines instead of the same lines. Lane 0 keeps the patches' own angles, so one pen draws as it would alone. The registry test `every_style_but_ink_keeps_its_pens_apart` covers the rule.

### 3.6 Empty lanes

No lift rule: a patch with little ink gets lines too far apart to fit, so it stays empty.

## 4. Tests

In `mosaic.rs`:

- Settings schema: ranges, defaults, the `x-density` mark, name and label.
- Regions: on a map whose left half is black and right half white, no region holds pixels of both halves. Every region is 4-connected, and no region is under `S^2 / 4` pixels unless it is the only one.
- A flat map gives about `(width / S) (height / S)` regions.
- White gives no paths. Full ink with `spacing_mm` 1 gives lines 1 mm apart within a region.
- A region elongated along x (a map with a dark horizontal band, `compactness` low) draws its lines along x.
- A second lane's lines are turned by 90 degrees in two lanes.
- Every vertex stays inside the tone rect.
- Determinism: same inputs, same coords.

Elsewhere:

- `registry.rs`: `lists_builtins_and_hides_test_styles` lists `mosaic` after `hilbert`, and `rippling_styles_darken_their_midtones` checks its gamma of 1.9. The tests over every listed style then cover it.
- `crates/core/tests/golden.rs`: a case `mosaic_gradient` on `gradient(64, 48)` with `tone.resolution` 64.
- `crates/core/tests/settings_matter.rs`: add exceptions only with a reason, as the existing ones have.
- The Python tests that list the styles (`tests/test_core_binding.py`, `tests/test_session.py`).

## 5. Documents

- README: the style count, a row in the style table, and a picture cell in a third row of the picture table, from `cargo run --release -p plotline-core --example readme_images` (commit only `docs/images/mosaic.png`, and check that the committed pictures did not change).
- CHANGELOG 0.1.0: the style count, and `mosaic` in the list of styles.
- Brief §14: a row "Mosaic style" with the decision, the sources and the tuned defaults; amend "Rippling gamma" to name `mosaic`.
