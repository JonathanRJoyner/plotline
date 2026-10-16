# The scribble style

Date: 2026-10-15. Status: decided by the builder under the owner's standing delegation; written for review.

Brief §7.2 lists circular scribbles among the renderers that later point placers could feed. This spec adds them as the `scribble` style: loops traced along the single line through the Voronoi points, small and close where the image is dark and wide and loose where it is light.

## 1. Source

C.-C. Chiu, Y.-H. Lo, R.-R. Lee and H.-K. Chu, "Tone- and Feature-Aware Circular Scribble Art", *Computer Graphics Forum* 34(7), 2015 (Proc. Pacific Graphics), pp. 225-234, doi:10.1111/cgf.12761. The paper builds a tracing path through points sampled by tone (§4), then draws a "rolling disk": a point on a disk whose centre moves along the path (§5.1, a trochoid). The disk radius and the centre's speed per turn follow the luminance `L` at the centre (equations 5 and 6):

- `r = Rmin + L (Rmax - Rmin)`
- `v = Vmin + L (Vmax - Vmin)`, with the turning speed held constant, so the loops stay loops while `v` is under one turn's length.

The disk's normal tilts the loops into ellipses: an azimuth turns them and a polar angle narrows them (§5.1, Figure 8).

What plotline takes and what it leaves:

- **The path.** The paper links its points with a Kernighan-Lin tour inside each SLIC superpixel and merges the tours. plotline already has a tone-adaptive placement (weighted Voronoi, Secord 2002) and a tour through it (the `tsp` stage, Johnson and McGeoch 1997), so the scribble follows that tour. The style then reuses both the cached points and the cached tour of `line`.
- **The model.** Equations 5 and 6 as written, with `L = 1 - ink`. The tilt is one fixed ellipse for the whole drawing: a `flatten` share for the polar angle and a `tilt_deg` direction for the azimuth.
- **Left out.** The feature-preserving radius cut near edges (§5.2, equation 7) needs an edge map that plotline does not compute; the tone curve fitted by blurring (§6) is the tone stage's job; the small random jitter of the normal is left out because the tour already breaks up any regular pattern.

## 2. Style

`scribble`, label "Scribble", description "Loops traced along one line through the points, small and close where the image is dark, wide and loose where it is light."

Stages: `tone`, `split`, `voronoi`, `tsp`, `scribble`, `order`. The `tone`, `voronoi` and `tsp` defaults are those of `line`, so a switch between `stipple`, `line` and `scribble` at the defaults reuses the points and the tour (brief §14, "Shared Voronoi defaults"). The registry test `the_voronoi_styles_share_tone_and_point_defaults` gains `scribble`.

The density setting for the budget solver stays `voronoi.count`, already marked `x-density`; the `scribble` stage marks none, so the style keeps exactly one mark.

## 3. The `scribble` stage

New file `crates/core/src/stages/scribble.rs`. Name `scribble`, label "Scribble", version 1. Requires `Paths`, `Tone`, `Pens`; provides `Paths`. Inside a style the `Paths` input resolves to the `tsp` output and the `Pens` input to the lane's pens, whose first pen is the lane's.

### 3.1 Settings

Every help text is one line.

| Key | Help | Range | Default |
|---|---|---|---|
| `radius_mm` | Radius of a loop over full ink. | 0.1 to 10, step 0.1, mm | 1.0 |
| `white_radius_mm` | Radius of a loop over white. | 0.1 to 20, step 0.1, mm | 3.0 |
| `spacing_mm` | How far the loops move along the line in one turn over full ink; closer loops draw darker. | 0.05 to 10, step 0.05, mm | 1.0 |
| `white_spacing_mm` | How far the loops move along the line in one turn over white. | 0.05 to 20, step 0.05, mm | 4.0 |
| `flatten` | How far the loops flatten into ellipses; 0 draws circles. | 0 to 0.9, step 0.05 | 0 |
| `tilt_deg` | Direction of the flattened loops' long axis, counter-clockwise from horizontal. | 0 to 180, step 1, deg | 0 |

The defaults are starting values. The builder tunes them on the README still life with a 0.3 mm pen: the drawing should read clearly, and plot lint should report no `overdraw` (lint counts a spot passed three or more times). Record the final values in the brief row. `spacing_mm` is named so that plot lint's spacing rules apply to it: loops closer than the pen overlap (`close_lines`), overdraw widens it, and a dark drawing that is too light narrows it.

### 3.2 Tracing

For each input path, in order, the stage draws one output path:

1. The centre `c` starts at the path's first vertex, at arc length `s = 0`, with the turn angle `phi = 0`.
2. At each output vertex, `ink = tone.sample(c)` and `L = 1 - ink`, clamped to 0..1. `r = radius_mm + L (white_radius_mm - radius_mm)` and `v = spacing_mm + L (white_spacing_mm - spacing_mm)`.
3. The vertex is `c + rotate(tilt_deg) (r cos phi, -(1 - flatten) r sin phi)`, then clamped to the tone map's rect, as `waves` clamps (`lines::clamp_to`). The minus sign makes the loops turn counter-clockwise as seen on the page (brief §14, "Angles").
4. The angle step is `dphi = 2 pi / k`, with `k = clamp(ceil(2 pi r / step), 8, 64)` and `step = max(pen / 4, 2 pi r / 64)`, the arc rule of the stipple dots (brief §14, "Dot geometry"). Then `phi += dphi` and `s += v dphi / (2 pi)`, and `c` moves to arc length `s` along the path by linear interpolation between its vertices.
5. The path ends once `s` reaches the path's length, with one last vertex at `phi` there, so the loops end on the line's last point.

An input path of one vertex, or of zero length, gives one full loop around that vertex at the `r` there, closed.

The stage keeps a running vertex count and fails once it passes 20 million vertices, with the message "the scribble needs over 20 million vertices; use wider spacing, fewer points or a wider pen" (the stipple's budget, brief §14 "Dot geometry").

### 3.3 Steps and frames

The run is a `lines::ChunkRun` over the input segments of all paths, with the phase and the arc length carried from one item to the next in the closure. A segment's scribble carries on into the next segment through the run's open path; the last segment of an input path closes its output path. ChunkRun reports about 30 frames that hold the paths so far, labelled with the segments done. The tour is one path, so the frames show one growing scribble.

### 3.4 Pens kept apart

No rule of its own. Each lane's points come from its own random stream (`RunContext::rng`), so the tours and the loops on them differ. The registry test `every_style_but_ink_keeps_its_pens_apart` covers it.

### 3.5 Empty lanes

No lift rule. The points exist only where the lane has ink ("Empty Voronoi points dropped"), and the loops follow the tour wherever it goes, as `line` draws its edges.

## 4. Tests

In `scribble.rs`:

- Settings schema: ranges and defaults, name and label.
- A straight two-vertex path over white gives one path whose vertices lie within `white_radius_mm` of the line (plus clamping) and whose centre advances `white_spacing_mm` per turn: count the turns by sign changes of `y - y_line` and compare with `length / white_spacing_mm` within one.
- Over full ink the vertices lie within `radius_mm` of the line.
- `flatten` 0.5 with `tilt_deg` 0 halves the loops' extent across a horizontal line.
- A one-vertex path gives one closed loop.
- Every vertex stays inside the tone rect.
- Determinism: the same inputs and settings give the same coords twice.
- The vertex budget: a tiny `spacing_mm` and `white_spacing_mm` over a long path fails with the message.

Elsewhere:

- `registry.rs`: `lists_builtins_and_hides_test_styles` lists `scribble` after `hilbert`; `the_voronoi_styles_share_tone_and_point_defaults` covers it. The existing tests over every listed style (determinism, colour scene, tiny images, pens kept apart, one density setting) then cover it too.
- `crates/core/tests/golden.rs`: a case `scribble_gradient` on `gradient(64, 48)` with `tone.resolution` 64, `voronoi.count` small and `voronoi.iterations` 10, and spacings wide enough that the file stays under 100 KB.
- `crates/core/tests/settings_matter.rs`: `tilt_deg` matters only when `flatten` is above 0; add the exception with that reason if the test needs it, as for `clip_percent`.
- The Python tests that list the styles (`tests/test_core_binding.py`, `tests/test_session.py`).

## 5. Documents

- README: the style count, a row in the style table, and a picture cell in a third row of the picture table, from `cargo run --release -p plotline-core --example readme_images` (commit only `docs/images/scribble.png`, and check that the committed pictures did not change).
- CHANGELOG 0.1.0: the style count and the new style in the Voronoi point family line.
- Brief §14: a row "Scribble style" with the decision, the source and the tuned defaults.
