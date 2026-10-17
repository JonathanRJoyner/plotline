# Point count from the picture's ink: not adopted

**Decided:** 2026-10-17

**Choice:** `voronoi.count` stays an absolute number of points per pen. F7 of [the colour and styles audit](../audits/2026-10-16-colour-styles-audit.md) proposed deriving it from the picture's total ink, so that a pale picture gets fewer points and a dark one more. That was measured and not adopted.

**Reason:** A temporary change to the placer multiplied each run's count by the tone map's mean ink over a reference ink. It drew `stipple` and `line` on the ten pictures of [the cells edge limit measurement](2026-10-17-cells-edge-limit.md), at a 0.3 mm pen. Their mean ink runs from 0.08 (the poster) to 0.84 (the dark scene); today every one gets the same 5,000 points. The table gives medians over the ten.

| Style | Count | Tone error | Contrast | Score | Plot time min (sum) |
|---|---|---:|---:|---:|---:|
| `stipple` | 5,000 | 0.048 | 0.094 | 0.680 | 340.9 |
| `stipple` | × ink / 0.35 | 0.051 | 0.130 | 0.596 | 398.7 |
| `stipple` | × ink / 0.5 | 0.058 | 0.090 | 0.706 | 286.7 |
| `line` | 5,000 | 0.067 | 0.172 | 0.767 | 68.6 |
| `line` | × ink / 0.35 | 0.064 | 0.181 | 0.755 | 77.1 |
| `line` | × ink / 0.5 | 0.074 | 0.153 | 1.009 | 61.3 |

- **It doesn't help where it was meant to.** The dark scene's tone error stayed at 0.053 at every count. Its tone map has so little spread that its score is infinite (no better than blank paper) whatever the count. The audit's dark night street, where the point styles vanish, is not reproduced by code-drawn pictures, and those pictures are not in the repository.
- **The pictures that lose ink lose fidelity.** At a reference of 0.35 the median score improves for `stipple` and barely moves for `line`. But the still life, the poster and the pale scene all get worse for both styles; the pale scene goes from 0.86 to no better than paper for `line`. Those pictures get paler, which no default may do.
- **The cost is wide.** `count` is the density setting that the budget fit, Optimize, the looks and the per-pen sharing by colour all read. An ink-derived count would change what each of them scans.

The cheaper route the audit names is still open: the image analysis could suggest a higher count for a dark picture (mean ink over 0.6), leaving the default alone.
