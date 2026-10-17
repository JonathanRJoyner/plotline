# Auto framing

Date: 2026-10-16. Status: decided by the builder under the owner's standing delegation; written for review.

When the subject fills only part of the image, the drawing spends most of the page on background. This spec adds a crop to the page and a suggestion that frames the image to its subject. The suggestion is a patch like the others: it is applied only when the user clicks Apply or the assistant calls for it, never on its own.

## 1. The crop

The brief planned a crop as `fill` plus an offset on the canvas (§14, "Canvas modes"). It goes on the canvas, not into a stage's settings, because two stages read the image's placement: the tone stage crops luminance for `fill`, and the split stage crops the colour planes the same way for Pen use by colour. Both read the canvas and neither can read the other's settings.

- `Canvas` gains `crop: [f64; 4]`, the shares of the source image cut from its left, top, right and bottom, each 0 to 0.45, default all zero. Serde gives it that default and leaves it out of the output while it is all zero, so existing presets, exports and golden files keep their bytes. `Canvas::new` keeps its signature; `Canvas::with_crop(crop)` checks the values (finite, each 0 to 0.45) and returns an error naming the bad one.
- The tone stage and the split stage cut the crop from the image first, rounding each edge to whole source pixels the same way in both, then crop to the drawing area's aspect for `fill` as now, then resize. One helper in `tone.rs` gives both stages the source rectangle, so they cannot drift apart; `map_size` and `response.rs` take the crop into account through it too.
- If the focus points have landed first, `Canvas::image_point` and `page_point` include the crop, and a test places a focus point through a crop.
- Front ends: MCP `set_page` gains `crop: [left, top, right, bottom] | None`; Python's `Canvas` gains `crop` with the same check; the CLI gains `--crop LEFT,TOP,RIGHT,BOTTOM`; presets carry it inside the canvas (no new version: the field is optional and defaults to no crop). The GUI's Page panel shows "Cropped" with a "Remove crop" button while a crop is set; the crop itself comes from the suggestion, and undo takes it back like any page change.

## 2. Saliency

The spectral residual of X. Hou and L. Zhang, "Saliency detection: a spectral residual approach", *Proc. CVPR* 2007, pp. 1-8, doi:10.1109/CVPR.2007.383267, written from the paper:

1. The image's luminance, block-averaged to 64 pixels on its longer side (the paper's scale), keeping its aspect.
2. Its 2D discrete Fourier transform, computed directly as a separable transform with twiddle factors from `math::sin` and `math::cos`. No FFT crate: the common one picks SIMD kernels at run time, which could make the native and browser builds differ in the last bit.
3. The log amplitude `L = ln(|F|)` (with a floor of 1e-12 under the magnitude) and the phase `P`.
4. The spectral residual `R = L - (L convolved with a 3 by 3 mean)`, with the spectrum's edges wrapped.
5. The saliency map `S = |inverse transform of exp(R + iP)|^2`, smoothed by a Gaussian with a sigma of 8 pixels at that scale (the paper's value), through `math::blur`.

`analysis::measure` computes `S` once per image, with its statistics, and keeps it for `suggest` without showing it in the statistics' JSON. It costs about a million complex multiply-adds, a few milliseconds.

## 3. The crop search

After R. Suh, H. Ling, B. B. Bederson and D. W. Jacobs, "Automatic thumbnail cropping and its effectiveness", *Proc. UIST* 2003, pp. 95-104, doi:10.1145/964696.964707: the smallest rectangle that holds a share λ of the total saliency, with λ taken where the area needed starts to climb fastest. Two changes for plotline: the rectangle has the drawing area's aspect, so the cropped image fills the drawing area in either fit mode, and on a 64-pixel map the search is exhaustive over a summed-area table rather than greedy.

1. The largest rectangle of the drawing area's aspect that fits inside the image is size 1. For sizes `s` from 1 down to 0.3 in steps of 0.05, and every position on the map's pixel grid, the share `f(s)` is the most saliency a rectangle of that size holds, read from a summed-area table.
2. For `λ` from 0.50 to 0.95 in steps of 0.05, `A(λ)` is the area `s^2` of the smallest size whose `f(s)` reaches `λ`.
3. `λ*` is the `λ_i` before the largest step `A(λ_{i+1}) - A(λ_i)`; ties go to the smaller `λ`. The crop is the best rectangle at `A(λ*)`'s size, ties to the position nearest the image's centre.
4. The crop is suggested only when it concentrates the subject: its area is at most 0.7 of size 1 and its saliency share over its area is at least 1.3. These two cut-offs are hand-set starting values; tune them on the generated images (§5) and record the values used.
5. The rectangle becomes `crop` shares, each rounded outward to 0.01 so the subject is never cut, then clamped to 0 to 0.45.

## 4. The suggestion

`framing`, after `orientation` in `suggest`'s fixed order, with the reason "The subject fills only part of the image. Cropping to it draws it larger." and the patch `{"page": {"crop": [left, top, right, bottom]}}`. It is left out when the canvas already has that crop (each share within 0.01). It depends on the drawing area's aspect, so it moves with the page, like `orientation`. The GUI's Apply already merges a page patch into the canvas; `plotline analyse` prints it; MCP `analyse_image` returns it, applied with `set_page(crop=...)`.

## 5. Tests and laws

- `analysis.rs`: the transform against a slow reference DFT on an 8 by 6 input; a dark disc on a white field, placed off-centre, gives a crop around the disc that holds all of it; a flat image, a linear ramp and pure noise (`synthetic::noise`) give no suggestion; the still life and the scene: record whether each suggests and where, and check the picture of the cropped drawing by eye; applying the suggestion's crop removes it; determinism.
- `types.rs`: `with_crop` refuses out-of-range and non-finite values; an all-zero crop serialises to the old bytes.
- `tone.rs` and `split.rs`: a crop gives the same source rectangle in both stages; with a crop, `fill` and `fit` give a map of the crop's aspect and placement; an all-zero crop changes nothing, bit for bit.
- `crates/core/tests/laws.rs`: the existing law that an applied suggestion goes away must cover `framing`'s page patch; make it apply page patches if it does not yet. Add a law that a crop of all zeros leaves every style's drawing unchanged.
- Python, MCP and CLI tests for `crop`.
- The golden files, the gallery and its stats snapshot do not change: the gallery applies no suggestion.

## 6. Documents

- `changelog.d/auto-framing.md`, Section: Images.
- `docs/decisions/2026-10-16-auto-framing.md`: the crop on the canvas and why there, the sources, the two changes to Suh et al., the cut-offs and the results on the test images, and that it amends the brief's "Canvas modes" row (add "amended on 2026-10-16, see docs/decisions/2026-10-16-auto-framing.md" to that row).
- README: the Suggestions bullet names framing.
- Tour: the Suggestions step mentions that a suggestion may crop the image, and the Page panel's crop line is inside the existing Page step's element.
