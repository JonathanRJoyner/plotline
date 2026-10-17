# plotline spec: paper flattening

Date: 2026-10-16. Status: decided by the builder under the owner's standing delegation; written for review.

Scans and phone photos of drawings have grey, yellowed or unevenly lit paper. The tone stage maps luminance to ink with one global curve, so where the paper is darker it takes ink, and a threshold such as the ink style's catches the shadow of the light along with the strokes. This spec adds a tone setting that divides the paper's own tone out before tone mapping.

## 1. Method

The paper is estimated as a grey-level closing of the luminance, a maximum filter then a minimum filter with a window wider than the widest stroke, and the luminance is divided by it. This is background subtraction by a morphological closing, after S. R. Sternberg, "Biomedical image processing", *IEEE Computer* 16(1), 1983, pp. 22-34, doi:10.1109/MC.1983.1654163, whose rolling ball the flat square window stands in for; the closing removes dark marks narrower than the window and keeps the slowly varying paper. Document binarisation uses the same estimate-and-divide step: S. Lu, B. Su and C. L. Tan, "Document image binarization using background estimation and stroke edges", *IJDAR* 13(4), 2010, pp. 303-314, doi:10.1007/s10032-010-0130-8.

On the tone stage's resized luminance `Y` (`w` by `h` pixels, `mm_per_px` from the canvas):

1. **Window.** `n = flatten_mm / mm_per_px` pixels.
2. **Coarse grid.** The paper is smooth, so it is estimated on a coarser grid of `k` by `k` blocks, `k = max(1, floor(n / 8), ceil(max(w, h) / 1024))`, each block holding the largest `Y` in it (blocks at the right and bottom edges are partial). A block maximum is itself a maximum filter, so this is part of the closing.
3. **Closing.** On the coarse grid, a maximum filter then a minimum filter, each over a square of `2r + 1` blocks with `r = max(1, ceil(n / (2k)))`, row pass then column pass, the window cut off at the grid's edges.
4. **Smoothing.** `math::blur` of the coarse grid with a sigma of `r` blocks, which rounds the closing's plateaus off.
5. **Upsampling.** `math::resample` of the coarse grid to `cw * k` by `ch * k`, keeping its top-left `w` by `h`, so block centres line up with pixel centres.
6. **Division.** `Y' = min(1, Y / B)` where `B > 0`, and 0 where `B` is 0 (a pixel whose whole neighbourhood is black).

Everything is comparisons, the crate's own blur and resample, and one f32 division, so native and WebAssembly builds give the same bits. Every pass costs the same per pixel whatever the window, and the coarse grid is at most 1024 pixels on its longer side, so the filter's cost does not grow with the window: about 140 ms at a 4000 by 2800 map with a 250-pixel window in a release build, most of it the upsampling.

A dark area wider than the window reads as paper and turns white: the window must be wider than the widest stroke, and flattening suits drawings and text, not photographs. It assumes light paper, so it runs before `invert`.

## 2. Setting

`tone.flatten_mm`, "Flatten paper": the window in mm, from 0 to 50 in steps of 0.5, default 0 (off). The tone stage's order becomes luminance, crop for `fill`, resize, **flatten**, levels (auto levels measure the flattened map), gamma, contrast, invert and blur. With 0 the stage gives exactly the map it gave before, so the golden files and the gallery snapshot do not change. The stage keeps version 1: the new key is part of the cache key.

## 3. Steps

The tone stage did its work in one step. It now reports the resize as one step (label `tone map`) and, when flattening is on, the flattening and the rest of the map as a second (label `flatten paper`), so Stop can act between them. With flattening off it is one step as before.

## 4. Suggestion

`analysis::measure` also reports `flat_two_level`: Otsu's separability of the analysis grid's luminance after flattening with a window of a sixteenth of its longer side. `analysis::suggest` adds `paper` ("The paper's tone is uneven. Flatten paper evens it out, so the paper draws as white.") when `flat_two_level` is at least 0.85 and at least 0.05 above `two_level`, the image is not a colour image (colourfulness at most 40, the colour suggestion's cut), and `tone.flatten_mm` is 0. Its patch sets `tone.flatten_mm` to a sixteenth of the drawing area's longer side, rounded to 0.5 mm and kept within the schema's range (17.5 mm on A4 with a 10 mm margin). Flattening that turns an image two-level is what a drawing on uneven paper shows; on a photograph it does not, so photographs get no suggestion.

## 5. GUI

The control sits in the Image settings, not under Advanced. A new tour step, "Clean up a scan", rings the Image section (`data-tour="image-settings"` on the tone stage's section). A browser flow sets Flatten paper on the sample image with the ink style and sees the stats change.

## 6. Checks

- A law in `laws.rs`: flattening ignores uniform lighting. For a random picture's luminance, a window from 1 to 50 pixels and a gain from 0.25 to 1, flattening the luminance times the gain matches flattening the luminance within 1e-4.
- Unit tests: line art under a lighting ramp flattens to white paper and dark strokes; a flat image flattens to white; a black image stays black; the tone stage with `flatten_mm` 0 is bit-identical to before, and with it on reports two frames.
- Analysis: the line art under a lighting ramp gets the `paper` suggestion with its patch, the still life and the scene do not, and the suggestion goes away once `flatten_mm` is set (the law `an_applied_suggestion_goes_away` covers it too).
- The golden files and `tests/snapshots/gallery-stats.json` do not change.
