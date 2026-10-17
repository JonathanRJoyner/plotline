# Ripple swing from ink length: tried, gamma 1.9 kept

**Decided:** 2026-10-17

**Choice:** `waves`, `spiral` and `mosaic` keep their tone gamma of 1.9 and their swing in proportion to the ink. This amends the brief's "Rippling gamma" row with a test of its alternative. Swinging each ripple so that the line's length grows in proportion to the ink, as amplitude-modulated line halftoning does (A. G. M. Ahmed and O. Deussen, "Amplitude modulated line-based halftoning", *Proc. Computer Graphics and Visual Computing*, Eurographics, 2016, pp. 41-43), was built and measured. It made the defaults lose contrast at every gamma and match the source's colours less closely, so it was not adopted. A law, `a_ripple_lays_more_line_where_the_tone_is_darker`, now holds the property the swing does have: on a flat page with no simplifying, a darker grey never draws a shorter line with one pen.

The test built the swing from a table of a sine's length per unit of line against its swing, 65 rows from no swing to the full swing. The length came from the midpoint rule over a quarter period. The swing for each ink was the one whose ripple is longer than a straight line by that share of the full swing's extra length. A light tone then swings about twice its share: at ink 0.1, 0.185 mm of the 0.9 mm full swing.

`cargo run --release -p plotline-core --example ripple_gamma` measured both rules. It ran the gallery's three pictures at A4 with a 0.3 mm pen, one black pen and by colour with yellow, red and black, on the by-colour lanes branch (#90). Each cell shows contrast, colour gain and darkness: the tone match's contrast, the share of blank paper's ΔE*ab against the source that the drawing takes away, and the page's mean darkness. "Now" is the swing in proportion to the ink at gamma 1.9; "length" is the length rule.

| Style, pens | Picture | Now, gamma 1.9 | Length, gamma 1 | Length, gamma 1.9 |
|---|---|---|---|---|
| waves, black | still life | 0.43, 0.61, 0.237 | 0.40, 0.51, 0.218 | 0.40, 0.47, 0.263 |
| waves, black | scene | 0.45, 0.28, 0.428 | 0.38, 0.26, 0.356 | 0.38, 0.28, 0.451 |
| waves, black | gradient | 0.43, 0.67, 0.384 | 0.39, 0.60, 0.338 | 0.39, 0.66, 0.405 |
| spiral, black | still life | 0.43, 0.59, 0.243 | 0.40, 0.50, 0.230 | 0.40, 0.47, 0.277 |
| spiral, black | scene | 0.46, 0.28, 0.424 | 0.39, 0.26, 0.362 | 0.39, 0.28, 0.444 |
| spiral, black | gradient | 0.43, 0.67, 0.378 | 0.39, 0.60, 0.338 | 0.39, 0.66, 0.400 |
| waves, by colour | still life | 0.47, 0.62, 0.239 | 0.63, 0.57, 0.169 | 0.45, 0.51, 0.275 |
| waves, by colour | scene | 0.38, 0.43, 0.434 | 0.45, 0.40, 0.382 | 0.38, 0.44, 0.453 |
| waves, by colour | gradient | 0.59, 0.71, 0.377 | 0.55, 0.62, 0.328 | 0.57, 0.71, 0.399 |
| spiral, by colour | still life | 0.47, 0.62, 0.239 | 0.63, 0.57, 0.167 | 0.46, 0.51, 0.272 |
| spiral, by colour | scene | 0.40, 0.43, 0.443 | 0.49, 0.41, 0.387 | 0.41, 0.44, 0.463 |
| spiral, by colour | gradient | 0.60, 0.71, 0.383 | 0.56, 0.62, 0.328 | 0.59, 0.71, 0.407 |

At gamma 1, the length rule is paler than now in every cell, and its colour gain is lower in all twelve. It loses contrast in every one-pen cell, by 0.03 to 0.07. At gamma 1.9 it is darker than now, but every one-pen cell still loses contrast, and the still life's colour gain falls from 0.61 to 0.47. No gamma between 1 and 1.9 recovered both. The brief allows neither paler defaults nor lost contrast.

**Reason:** Ink on paper follows the inked area, not the length of line. At the defaults, a 1 mm wavelength and a 0.3 mm pen, a full swing packs the line so tightly that it overlaps itself, so the inked area grows more slowly than the length. Making length rise in proportion to the ink therefore lifts the light tones and flattens the image's range. The tone match reads that as lost contrast. Against the source in CIELAB, the extra ink in the highlights costs the most, because lightness is most sensitive there. Today's swing grows the length with roughly the square of a small swing, which together with gamma 1.9 keeps the highlights clean. That was the measured best when "Rippling gamma" was set.

`mosaic` has no ripple: its line spacing already covers in proportion to the ink (Winkenbach and Salesin, 1994, as its module says), so the length rule does not apply to it. Its gamma was measured the same way. At gamma 1, one pen, it is a quarter to two-fifths paler and its colour gain falls: 0.75 to 0.46 on the still life, 0.27 to 0.22 on the scene, 0.65 to 0.49 on the gradient. So 1.9 stays there too, for the reason the brief gives: images store grey gamma-encoded while ink mixes linearly.

No default, drawing or snapshot changes, so the looks need no new measurement for this. A swing that follows the inked area, with the pen's width and the overlap at the crests, might still beat the gamma. It needs a coverage model of a rippled line, and that is left for later.
