# plotline spec: edge-keeping smoothing

Date: 2026-10-16. Status: decided by the builder under the owner's standing delegation; written for review.

The tone stage's only smoothing is `blur_mm`, a Gaussian that softens edges as much as it softens noise, so a style that follows edges (contours, flow, ink, hatch's level boundaries) loses them as it loses the specks. This spec adds a tone setting that smooths noise and paper grain but keeps edges.

## 1. Method

The guided filter of K. He, J. Sun and X. Tang ("Guided image filtering", *ECCV 2010*, LNCS 6311, pp. 1-14, doi:10.1007/978-3-642-15549-9_1; journal version *IEEE TPAMI* 35(6), 2013, pp. 1397-1409, doi:10.1109/TPAMI.2012.213), used with the image as its own guide. In each window the output is a linear function of the input, `q = a I + b`, fitted by least squares with a penalty `eps` on `a`; where the window's variance is well above `eps` (an edge), `a` is near 1 and the edge stays, and where it is well below (noise on a flat tone), `a` is near 0 and the window's mean takes over. With box means:

1. `mean_I = box(I)`, `mean_II = box(I * I)`, both over a square of `2r + 1` pixels.
2. `var = max(0, mean_II - mean_I^2)`, `a = var / (var + eps)`, `b = (1 - a) mean_I`.
3. `mean_a = box(a)`, `mean_b = box(b)`.
4. `q = clamp(mean_a I + mean_b, 0, 1)`.

`box` is the running-sum box mean the wide blur already uses (W. M. Wells, "Efficient synthesis of Gaussian filters by cascaded uniform filters", *IEEE TPAMI* 8(2), 1986, pp. 234-239), rows then columns, samples past an edge taking the edge value, sums in f64 in a fixed order. It costs the same per pixel at any radius. The output is an average of convex combinations of a pixel and window means, so it stays within the image's own tone range.

`r = max(1, round(smooth_mm / mm_per_px))` pixels. `eps = 0.1^2`: a tone step of about a tenth of the range or more counts as an edge. That is fixed rather than a second setting; see the decision.

## 2. Setting

`tone.smooth_mm`, "Smooth, keep edges": the radius in mm, from 0 to 10 in steps of 0.1, default 0 (off), under Advanced beside Blur. The tone stage's order becomes luminance, crop, resize, flatten, **smooth**, levels, gamma, contrast, invert, blur and the focus points' falloff. With 0 the stage gives exactly the map it gave before.

## 3. Steps

When smoothing is on, it runs as four steps of the tone run, labelled `smooth`, after the resize step (`tone map`) and the flattening step (`flatten paper`) when flattening is on: the first two compute `mean_I` and `mean_II`, the third `a`, `b` and `mean_a`, the fourth `mean_b` and `q`. The last step of the run's filters also makes the rest of the map (levels to blur), as the flattening step does today, so with smoothing off the run's frames are as before. Each frame's progress is the share of the run's steps done, the last one 1. `pub fn guided(y, w, h, r) -> Vec<f32>` runs all four for tests and laws, and the steps call the same code, so both give the same bits.

## 4. GUI

The "Clean up a scan" tour step gains a sentence on Smooth. A browser flow sets it on the sample image and sees the stats change.

## 5. Checks

- A law in `laws.rs`: smoothing stays within the image's tone range. For a random picture's luminance and a radius from 1 to 40 pixels, every output lies within the input's minimum and maximum, within 1e-5.
- Unit tests: a two-tone step with noise keeps its step sharper than a Gaussian blur of the same radius while lowering the noise as much; a flat image stays flat; `smooth_mm` 0 is bit-identical to before, and with it on the run reports the four `smooth` frames.
- The golden files and `tests/snapshots/gallery-stats.json` do not change.
