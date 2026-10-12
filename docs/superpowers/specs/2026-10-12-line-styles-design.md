# plotline spec 5: line styles (hatch, waves, spiral)

Date: 2026-10-12. Status: built from the owner's request for more styles to test; open to review.

The owner tests on a 3D printer fitted with a pen, where every pen lift is a Z move that wears the machine. Grid dots lift once per dot. These three styles draw long lines with few lifts, so they come before the Voronoi family. Each is one new stage between the existing tone and path-order stages, so caching, the settings form, presets and every export work unchanged.

## 1. Styles

| Style | Stages | Look |
|---|---|---|
| `hatch` "Hatching" | tone, hatch, order | Straight parallel lines at an angle, drawn only where the image is dark enough; more levels add cross-hatching in darker areas. |
| `waves` "Waves" | tone, waves, order | Parallel lines that ripple, the ripple growing with the ink; one path per line. |
| `spiral` "Spiral" | tone, spiral, order | One spiral from the centre out, rippling with the ink; broken only where it leaves the drawing. |

## 2. Shared geometry

`stages::lines::scanlines(rect, angle_deg, spacing)` returns the lines of a family of parallel lines `spacing` apart, at `angle_deg` counter-clockwise from horizontal as seen on the page, clipped to `rect`, each as its two end points, in order across the rect. The first line sits half a spacing in from the rect's edge. Angles go through `math::sin` and `math::cos`.

Every vertex a style writes lies inside the tone map's rect: the wobble of waves and spiral is clamped to it. The G-code writer also refuses any vertex off the page, naming the path, so a future style cannot send the pen outside the machine's area.

## 3. Settings

**hatch:** `spacing_mm` 0.2 to 10, default 1; `angle_deg` 0 to 180, default 45; `threshold` 0 to 1, default 0.5; `levels` 1 to 4, default 1. Level `k` (from 0) is drawn at `angle_deg + 180 k / levels` where the ink reaches `threshold + (1 - threshold) k / levels`. Each line is sampled every half tone pixel; a run of samples at or above the level's threshold becomes one two-vertex path, and a run of one sample is dropped.

**waves:** `spacing_mm` 0.5 to 10, default 2; `angle_deg` 0 to 180, default 0; `wavelength_mm` 0.2 to 10, default 1; `amplitude` 0 to 1, default 0.9. A line is sampled every twelfth of a wavelength and each sample moves along the line's normal by `amplitude * spacing / 2 * ink * sin(2 pi t / wavelength)`, where `t` is the distance along the line's direction measured from one origin for all lines, so neighbouring lines ripple in phase.

**spiral:** `spacing_mm` 0.5 to 10, default 2 (distance between turns); `wavelength_mm` and `amplitude` as for waves, the ripple moving each sample along the radius. The spiral is Archimedean, `r = spacing * theta / (2 pi)`, centred on the tone rect, sampled every twelfth of a wavelength of arc, and runs until the radius reaches the rect's half diagonal. A sample whose unrippled point lies outside the rect ends the current path; paths of one vertex are dropped.

## 4. Frames

Each stage reports about 30 frames, each holding the paths made so far, with a label of lines done (hatch, waves) or turns done (spiral).

## 5. Tests

- `lines.rs`: horizontal lines on a 10 by 4 mm rect at spacing 1 give four lines at y 0.5 to 3.5 spanning the width; a 90 degree family is vertical; a 45 degree family clips every line to the rect.
- Each stage: its schema bounds and defaults; every vertex inside the tone rect; output deterministic; a flat white map gives no hatch paths and straight waves; full ink with `levels` 2 gives two hatch directions; the spiral's first path starts at the rect's centre.
- The existing every-setting-matters test covers all three styles; its no-op list gains what the test finds, each with a reason.
- `gcode.rs`: a vertex off the page is refused.
- `registry.rs`: the list holds the four styles.

## 6. Decisions recorded in the brief

Line styles come before the Voronoi family because the owner's test machine pays for every lift. The waves and spiral wobble is clamped to the tone rect, and the G-code writer checks every vertex against the page.
