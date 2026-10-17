# Contrast and colour in the tone match

**Decided:** 2026-10-17

**Choice:** The tone match keeps its tone error as the measure of shape and reports two more numbers beside it, and Optimize ranks on a score that counts contrast. This settles F2 and the scoring half of F3 in the [colour and styles audit](../audits/2026-10-16-colour-styles-audit.md).

- **Contrast** (`ToneMatch::contrast`): the fitted line's rise from white to full ink, `reach - floor`, and 0 when it does not rise. It measures, unscaled, how much of the tone map's range the drawing reaches: 1 inks as much more where the image is darkest as the tone map says, and the dot styles sit at 0.05 to 0.2.
- **Blank paper's error** (`ToneMatch::blank`): the RMS of the blurred tone map about its mean. The tone error is capped at it, because a drawing whose ink does not follow the tones can do no better than blank paper's guess, the mean, and should do no worse. Below the gain floor (`MIN_GAIN`, 0.01) the error is blank paper's instead of no match. `contours` read 2 to 12 before, from dividing by a gain near the floor; it now reads blank paper's error on every gallery picture, which is what its outlines tell the tone match.
- **Colour** (`ToneMatch::colour`, for a drawing in more than one pen colour): the drawing is rendered as the preview draws its ink, without the drawing area's outline, at 4 px per mm. It is averaged to 2 px per mm over the tone map's rectangle and compared with the part of the source image the page shows. Both go through the same eye filter, and the mean CIE 1976 ΔE*ab is reported (CIE 15:2004, *Colorimetry*, 3rd ed.), with blank paper's ΔE and the share of it the drawing takes away (`gain`). This is the audit harness's method. The CIELAB conversion is the palette analysis's (`analysis::lab`, sRGB per IEC 61966-2-1). The session works it out once per drawing and paper. It is not used by Optimize yet: the trials do not carry the source image, and a by-colour score is left for the looks work (A7).
- **Score** (`ToneMatch::score`): `(error / blank) / min(contrast, 1)^w` with `w = 0.25` (`CONTRAST_WEIGHT`). It is infinite for a drawing with no contrast or no better than blank paper, so blank paper never wins and such a trial leaves Optimize nothing to suggest. Optimize keeps its rule: the quickest trial within a tenth of the lowest score. Weight 0 is the old ranking.
- The stats JSON, the GUI's stats line, the CLI's line and the MCP run result show the contrast, and the colour ΔE when there is one. The gallery snapshot keeps its fields. Its tone errors moved only where they were above blank paper's: `contours` everywhere, and on the scene `flow`, `line`, `sketch`, `tree` and `grid`. They were also missing where the gain was under the floor: `dither` and `grid` with the 0.1 mm pen, and `contours`. All of these now read blank paper's error.
- Two laws in `crates/core/tests/laws.rs`, under `blank_paper_bounds_the_tone_match_and_never_wins`, cover the contract. The tone error is never above blank paper's, and a blank drawing scores blank paper's error with no contrast and an infinite score. In colour, blank paper's gain is 0 and no drawing's gain is above 1.

**Reason:** The audit showed Optimize, the Faithful looks and the stats line rewarding near-blank drawings. The tone match divides out how dark a drawing gets, so a grid at 1 % coverage scored like one at 20 %, and Optimize moved grid dots sparser on all three gallery pictures. The weight was measured on main at 484e7fc, before the trail join (#88) landed, with `cargo run --release -p plotline-core --example contrast_weight`. It runs every density style's Optimize once on the gallery's still life, scene and gradient (600 by 875 px, A4, 0.3 mm pen, default speeds) and ranks the same trials again for each weight. Medians over 58 searches, the pick's contrast and plot time against the current value's:

| Weight | Contrast | Plot time |
|---|---|---|
| 0 (before) | ×1.14 | ×1.30 |
| 0.25 | ×1.43 | ×1.70 |
| 0.5 | ×1.47 | ×1.81 |
| 0.75 | ×1.48 | ×1.83 |
| 1 | ×1.75 | ×1.86 |

0.25 is the smallest weight that turns every faint pick around: grid, flow, truchet and tree no longer go sparser toward blank. It gets almost all of 0.5's contrast for less time. Higher weights mostly add plot time for small gains, for example `hilbert` on the scene at 116 minutes instead of 71. The owner leaves taste calls like this to us.

The picks that moved on the gallery pictures, as value (contrast, plot time). The other 32 searches picked the same value:

| Style | Picture | Current | Before | After |
|---|---|---|---|---|
| grid | still life | 1.0 | 2.828 (1 %, 4 min) | 0.707 (18 %, 53 min) |
| grid | scene | 1.0 | 4 (1 %, 13 min) | 0.707 (30 %, 317 min) |
| grid | gradient | 1.0 | 2 (3 %, 32 min) | 0.707 (21 %, 222 min) |
| hatch | still life | 1.0 | 4 (26 %, 3 min) | 1.414 (70 %, 8 min) |
| hatch | scene | 1.0 | 4 (31 %, 9 min) | 1.414 (79 %, 26 min) |
| hatch | gradient | 1.0 | 4 (26 %, 8 min) | 2 (48 %, 15 min) |
| stipple | still life | 5000 | 3536 (13 %, 25 min) | 7071 (26 %, 46 min) |
| stipple | scene | 5000 | 5000 (8 %, 35 min) | 7071 (12 %, 48 min) |
| stipple | gradient | 5000 | 3536 (6 %, 25 min) | 7071 (13 %, 48 min) |
| line | still life | 5000 | 14142 (29 %, 15 min) | 20000 (35 %, 18 min) |
| dashes | still life | 5000 | 7071 (31 %, 42 min) | 10000 (43 %, 58 min) |
| dashes | scene | 5000 | 7071 (12 %, 43 min) | 10000 (18 %, 58 min) |
| tree | scene | 5000 | 1250 (5 %, 5 min) | 10000 (13 %, 26 min) |
| tree | gradient | 5000 | 14142 (16 %, 34 min) | 20000 (19 %, 45 min) |
| mesh | scene | 5000 | 7071 (26 %, 40 min) | 10000 (31 %, 52 min) |
| cells | still life | 5000 | 7071 (37 %, 56 min) | 10000 (43 %, 75 min) |
| cells | scene | 5000 | 7071 (17 %, 58 min) | 10000 (20 %, 77 min) |
| cells | gradient | 5000 | 7071 (21 %, 57 min) | 10000 (25 %, 76 min) |
| flow | still life | 0.8 | 0.8 (13 %, 5 min) | 0.566 (15 %, 5 min) |
| flow | scene | 0.8 | 3.2 (3 %, 5 min) | 0.4 (20 %, 8 min) |
| flow | gradient | 0.8 | 1.131 (13 %, 8 min) | 0.566 (31 %, 12 min) |
| hilbert | scene | 0.6 | 0.849 (39 %, 44 min) | 0.6 (57 %, 71 min) |
| hilbert | gradient | 0.6 | 1.2 (26 %, 21 min) | 0.424 (72 %, 90 min) |
| truchet | scene | 0.4 | 1.131 (8 %, 9 min) | 0.4 (38 %, 23 min) |
| contours | still life, gradient | 12 | 3 levels | no suggestion |

`contours` never beats blank paper's error on these pictures, so Optimize now says to judge it by eye instead of suggesting 3 levels. Grid's denser picks plot for a long time because each dot costs a lift and a short hop at the default speeds. They stay within Optimize's time limit, twice the current value's time, which this decision leaves alone. "Before" is weight 0 on the same trials, with the error capped. The old engine left out trials below the gain floor, which changes only `contours`.

Left for later: the looks were measured with the old score and are not re-measured here (A7). Nor are the lint's `dark` fixes, F1's dot geometry or the by-colour Optimize score.
