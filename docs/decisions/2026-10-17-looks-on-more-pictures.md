# Looks on more pictures

**Decided:** 2026-10-17. Amends [the looks](2026-10-16-looks.md).

**Choice:** The looks were measured again with `crates/core/examples/looks.rs` and chosen anew, with three changes to how they are measured and one new rule. Each look was also drawn and looked at.

- **More pictures.** Nine code-drawn pictures instead of three: the gallery's still life, scene and gradient; three dark discs on white (flat shapes); a checkerboard; colour bars; line art lit from one side; the scene made dark (each channel to the power 2.5); and the scene made pale (35 % of its ink). Two candidates were dropped: plain noise and evenly lit line art. Their blurred tone barely varies, so blank paper beats every drawing of them, and they could not rank anything. Neither could the uniformly darkened scene, so the gamma-darkened scene replaced it.
- **The score, not the tone error.** Each drawing is scored by [`ToneMatch::score`](2026-10-17-contrast-and-colour-match.md), the tone error over blank paper's divided by the contrast to the power 0.25. A look is judged by its median over the nine pictures, not the mean of three. Faithful must beat Balanced's median score by 10 %. Fine may be up to a quarter worse. Quick may be up to twice as bad. These are the thresholds of the earlier looks, now applied to the score.
- **No look is more than twice as sparse as the default.** `style::SPARSEST_LOOK` is 2. Along a style's density setting, no look may place fewer than half the default's points, lines or dots, or space its marks more than twice as far apart. `Style::with_look` refuses a look past it, so a built-in style that breaks it fails every test run, and a unit test checks that the refusal fires. The scan's sparse side stops there, and a Quick fit that asks for more is held to it. The tone score could not see what this rule catches. Grid's Quick spaced its dots 5.6 times the default, so the still life became a few dozen discs as wide as their cells, a picture the owner reported as plainly bad.
- **Looked at.** Every look of every style was drawn on the still life and the scene and inspected. Three that passed the numbers were dropped or re-picked by eye; the table marks them.

| Style | Look | Before | After | Score vs Balanced | Time vs Balanced |
|---|---|---|---|---:|---:|
| `grid` | quick | spacing 5.6 mm | **dropped** | | |
| `grid` | fine | spacing 0.7 mm | **dropped** | | |
| `hatch` | fine | spacing 0.7 mm | **dropped** | | |
| `waves` | faithful | spacing 2.8 mm | auto response, spacing 1.7 mm | 0.64 | 1.09 |
| `waves` | quick | none | spacing 4.0 mm | 1.25 | 0.74 |
| `waves` | fine | spacing 1.0 mm | spacing 1.0 mm | 1.08 | 1.32 |
| `spiral` | faithful | none | auto response | 0.77 | 1.05 |
| `spiral` | quick | none | none (dropped by eye) | 1.14 | 0.74 |
| `spiral` | fine | spacing 1.4 mm | spacing 1.2 mm | 1.24 | 1.23 |
| `stipple` | faithful | none | 10,000 points | 0.85 | 1.91 |
| `stipple` | fine | 12,000 points | **dropped** | | |
| `line` | faithful | auto response, 20,000 | 20,000 points | 0.43 | 2.54 |
| `dashes` | faithful | 12,000 points | 8,400 points | 0.81 | 1.59 |
| `sketch` | faithful | auto response, 20,000 lines | auto response, 17,000 lines | 0.33 | 3.41 |
| `sketch` | fine | 10,000 lines | 7,100 lines | 0.90 | 1.43 |
| `mesh` | faithful | auto response, 6,000 | **dropped** | | |
| `mesh` | fine | 20,000 points | 7,100 points | 0.92 | 1.31 |
| `cells` | faithful | auto response, 8,400 | 14,000 points | 0.67 | 2.12 |
| `cells` | fine | 12,000 points | **dropped** | | |
| `dither` | faithful | spacing 0.8 mm | **dropped** | | |
| `dither` | fine | none | spacing 0.7 mm | 1.05 | 1.83 |
| `circles` | faithful | spacing 1.5 mm | spacing 2.1 mm | 0.48 | 1.94 |
| `flow` | faithful | auto response, spacing 0.3 mm | **dropped by eye** | 0.27 | 3.59 |
| `hilbert` | faithful | auto response, cell 0.3 mm | cell 0.4 mm (re-picked by eye) | 0.89 | 1.74 |
| `hilbert` | quick | none | cell 0.9 mm | 1.25 | 0.57 |

Unchanged: `scribble` Faithful (20,000), `tree` Faithful (auto response, 20,000), `mosaic` Faithful (0.3 mm) and `truchet` Faithful (0.3 mm). `ink`, `halftone` and `contours` still have no looks. "Score vs Balanced" is the look's median score over Balanced's, so lower is better. "Time vs Balanced" is the ratio of the median plot times.

**Dropped by eye:**
- **`spiral` Quick.** At 4 mm the rings take over the still life, and its spheres can barely be read.
- **`flow` Faithful.** Auto response turned the still life's box and spheres, and the scene's sky and hill, into flat bands with hard edges. It scores far better, but the drawing no longer looks like the picture. Without auto response, no spacing beats Balanced by 10 %, so `flow` has no looks.
- **`hilbert` Faithful.** Auto response at 0.4 mm lost the scene's hill. The same cell without it scores 0.416 against Balanced's 0.467, so it stays.

**Dropped by the numbers:** a look whose best value no longer beats Balanced by its margin, or whose values between it and the default stop passing, is left out. Grid, hatching, Mesh's Faithful, Cells' Fine and Stipple's Fine all fall under that. `mesh` and `cells` gained so much from [ink squared](2026-10-17-ink-squared-for-joined-points.md) that a denser Balanced is hard to beat by 10 %.

**Reason:** This settles F13 of [the colour and styles audit](../audits/2026-10-16-colour-styles-audit.md). The looks had been chosen on three smooth, mid-toned pictures by tone error alone. The score sees contrast. The wider set holds flat shapes on white, a dark picture and a pale one. Looking at each drawing catches what no number here does. The sparseness bound is the one numeric rule that does catch the reported failure.

**Not done:** an SSIM-style structural score of each look against Balanced. It would need a new comparison in the core, and on these pictures the sparseness bound and the visual check together caught every look that looked wrong.
