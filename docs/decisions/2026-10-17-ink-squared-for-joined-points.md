# Ink squared for the styles that join points

**Decided:** 2026-10-17

**Choice:** `line`, `scribble`, `tree`, `mesh` and `cells` default `voronoi.exponent` to **2**, so their points' density follows ink squared. `stipple` and `dashes` keep 1. The style defaults go through one helper, `joined` in `crates/core/src/registry.rs`. The placement stays a single Voronoi stage. Switching between a style at 1 and one at 2 now places the points again; switches within either group still reuse them. A law in `crates/core/tests/laws.rs` checks the five defaults, and that on a ramp a higher exponent puts a larger share of the points in the dark half.

**Reason:** A pen line of fixed width through stipple points darkens with the square root of the points' density, so a part twice as dark needs four times the points (Ahmed, "Variable-density stippling with connected points", *Bridges*, 2015). C. S. Kaplan and R. Bosch reached the same rule for their line drawings ("TSP Art", *Bridges*, 2005). With density following ink, the five styles drew their darks too light, which was the main finding of the roadmap review.

Two ways to get there were measured:

- **Exponent 2:** the placer itself places points by ink squared.
- **Thinning:** place points by ink as before, so `stipple`'s placement can be shared, then keep each point with a probability equal to its ink, seeded and deterministic.

The probe drew each style on the gallery's three images at the 0.1 and 0.5 mm pens. It reported the plot time with acceleration counted, and the tone match error and contrast of #91. The probe was a throwaway example and is not kept; thinning was a temporary change to the placer. The table sums paths, pen-down and plot time over the six drawings and averages error and contrast.

| Style | Density | Paths | Pen-down m | Plot time min | Tone error | Contrast |
|---|---|---:|---:|---:|---:|---:|
| `line` | ink | 6 | 93.4 | 45.3 | 0.148 | 0.120 |
| `line` | **ink²** | 6 | 84.8 | 42.5 | **0.105** | **0.198** |
| `line` | ink, thinned | 6 | 55.0 | 26.7 | 0.176 | 0.138 |
| `scribble` | ink | 6 | 271.1 | 93.0 | 0.092 | 0.431 |
| `scribble` | **ink²** | 6 | 260.6 | 89.2 | **0.081** | **0.583** |
| `scribble` | ink, thinned | 6 | 174.9 | 59.7 | 0.141 | 0.455 |
| `tree` | ink | 4,246 | 99.8 | 75.1 | 0.195 | 0.123 |
| `tree` | **ink²** | 4,158 | 89.9 | 70.5 | **0.141** | **0.196** |
| `tree` | ink, thinned | 2,108 | 55.0 | 41.2 | 0.196 | 0.135 |
| `mesh` | ink | 1,096 | 309.6 | 162.0 | 0.114 | 0.320 |
| `mesh` | **ink²** | 1,116 | 264.8 | 145.8 | **0.065** | **0.583** |
| `mesh` | ink, thinned | 862 | 175.3 | 92.1 | 0.111 | 0.506 |
| `cells` | ink | 4,170 | 205.6 | 157.3 | 0.105 | 0.199 |
| `cells` | **ink²** | 4,108 | 187.6 | 147.5 | **0.074** | **0.323** |
| `cells` | ink, thinned | 2,304 | 140.8 | 93.9 | 0.123 | 0.251 |
| `stipple` | **ink** | 30,000 | 82.8 | 214.5 | **0.068** | 0.119 |
| `stipple` | ink² | 30,000 | 87.8 | 213.1 | 0.089 | 0.192 |

- **Ink squared** lowers the tone error of all five styles by 12 to 43 % and raises their contrast by 35 to 82 %. Their plot time falls by 4 to 10 %.
- **Their pen-down falls by 4 to 14 %.** The same number of points moves from the light parts to the dark ones, so the darks get darker and the lights lighter, which is what the higher contrast measures.
- **Thinning** keeps only as many points as the mean ink allows, so it draws 32 to 45 % less line. The error rises for four of the five styles, and the contrast gains less than with ink squared. Raising the count to make up for it would change the placer's settings anyway, so the placement could not be shared, and the relaxation would cost more. Independent thinning also breaks up the even spacing that the relaxation builds, most where the ink is light.
- **`stipple`** is the control. Its dots darken with their density, not its square root, and ink squared raises its error from 0.068 to 0.089. So the dot styles keep 1.

The cost is the shared cache between the two groups. A switch from `stipple` to `line` relaxes the points again; a switch from `line` to `scribble`, or among `tree`, `mesh` and `cells`, still reuses them. This amends the first-release criterion in the brief's §3 that switching between stipple, continuous line and dashes does not recompute the points: it still holds between `stipple` and `dashes`, and between any two styles when `voronoi.exponent` is set alike, since a switch keeps a changed shared setting. The session tests that pinned the shared points now switch within a group.

**Gallery:** on main at 3798f12 the regression gallery moved all 30 cells of the five styles and no other cell, with the same totals as the table above. The plot lint's `dark` finding, which says the darkest parts are too lightly covered, fires on 4 of the 6 `cells` drawings instead of 5, 5 of 6 `line` drawings instead of 6, and 5 of 6 `tree` drawings instead of 6; `mesh` and `scribble` stay at 3. The golden file `scribble_gradient.json` is regenerated. The faithful looks, whose point counts were chosen by a density scan at an exponent of 1, keep those counts; a new scan can follow.
