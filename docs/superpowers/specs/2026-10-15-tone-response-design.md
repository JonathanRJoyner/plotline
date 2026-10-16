# plotline spec: tone match, auto tone response and optimize

Date: 2026-10-15, revised 2026-10-16 with the measurements. Status: built. The owner delegates the taste defaults, which are recorded in the brief's decision log.

The owner asked for an optimize button that makes drawings match the image within a line count. The plan has three layers: the budget solver (fit a drawing to minutes, paths, metres or dots), plot lint with its coverage raster, and this spec, which builds on both. A first optimize button was tried on 2026-10-12 and shelved (brief §14, "Tone matching"): it chased the image's black, which no style reaches, and each press took 10 to 42 seconds, mostly rasterising. This spec measures how well a drawing's tones follow its tone map, offers a correction for each style's own tone response, and offers a search as an opt-in suggestion.

It follows section 7 of the smart-features research: S7.1 (a blur-and-compare tone error and SSIM), S7.2 (a simulated tone response, inverted into a lookup) and S7.3 (a setting search on the tone error).

## 1. Scope

- `crates/core/src/tone_match.rs` (new): the tone error and SSIM of a drawing against its tone map.
- `crates/core/src/response.rs` (new): the step wedge, its measurement, the fit, pool-adjacent-violators and the inverse lookup.
- `crates/core/src/optimize.rs` (new): the search and the job hosts drive.
- `crates/core/src/stages/tone.rs`: the `response` setting.
- `crates/core/src/runner.rs`: a run whose tone stage is set to `auto` first runs the wedge, then reads the tone map through the lookup.
- `crates/core/src/lint.rs`: the measurement also takes the tone match, from the same raster.
- `crates/core/src/session.rs`: `Session::tone_match` and `Session::optimize`.
- Bindings, CLI, MCP and GUI, with a tour step.
- Docs: the brief's decision log and §9, the README, the CHANGELOG.

## 2. Tone match (S7.1)

`tone_match::compare(coverage, tone) -> Option<ToneMatch>` compares where the pens put ink with the tone map the drawing was made from:

1. **Coverage per tone pixel.** Lint's raster marks each cell any layer inks. A tone-map pixel's coverage is the share of inked cells among the cells whose centres fall inside it; a pixel smaller than a cell takes the cell under its centre. This is the box average of the coverage at the tone map's scale (`pixel_coverage`).
2. **Eye filter.** Both maps are blurred by the same Gaussian, sigma `EYE_SIGMA_MM` = 1.5 mm, with `math::blur`. This is the blur-and-compare of model-based halftoning (T. N. Pappas and D. L. Neuhoff, "Least-squares model-based halftoning", *IEEE Trans. Image Processing* 8(8), 1999, pp. 1102-1116), with a Gaussian standing in for the eye's contrast sensitivity (R. Näsänen, "Visibility of halftone dot textures", *IEEE Trans. SMC* 14(6), 1984, pp. 920-924). 1.5 mm is the blur the 2026-10-12 spike scored with: line texture at the default spacings of 1 to 2 mm fades under it, as it does when a plot is seen from a metre or two. It is a taste default.
3. **Fit.** No style reaches full black, and some, such as `waves`, ink their white too. The blurred coverage `c` is fitted to the blurred tone `t` by the least-squares line `c = floor + gain * t`, and the coverage is put on the tone map's scale as `(c - floor) / gain`. So a drawing is judged on how its tones follow the image, not on the darkest tone it can lay down, which was the spike's lesson. Dividing by the gain also means a faint drawing whose ink barely follows the tone scores worse than a strong one with the same residue.
4. **Tone error** is the root mean square of `(c - floor) / gain - t` over the map, in tone-map ink: 0.05 means the tones are off by 5 % of the drawing's range on average. **SSIM** is the mean structural similarity of the same two maps (Z. Wang, A. C. Bovik, H. R. Sheikh and E. P. Simoncelli, "Image quality assessment: from error visibility to structural similarity", *IEEE Trans. Image Processing* 13(4), 2004, pp. 600-612), with Gaussian windows as wide as the eye filter and the paper's constants `C1 = (0.01)^2` and `C2 = (0.03)^2`. They are reported separately, with `floor` and `reach` (`floor + gain`), the coverage the fit puts at white and at full ink.

There is no tone match without a tone map, for a flat tone map, or when the gain is under 0.01 (a drawing whose ink does not follow its tone, such as an empty one).

The measure runs in lint's `measure`, on the raster lint already stamps, so a drawing is rasterised once for both, and the session keeps it with lint's measurement. `tone_match::error(drawing)` gives the tone error alone, without lint's other counts or the SSIM, for the search, which scores many drawings.

## 3. Auto tone response (S7.2)

### 3.1 The setting

The tone stage gains `response: off | auto`, default `off` for every style (section 6). With `auto`, the stages after the tone stage read the tone map corrected for the style's own tone response: how much of the paper the style covers at each ink, with the current settings and pen. The correction comes last, after levels, gamma, contrast, invert and blur, so it corrects what the style does with the tone map and leaves the user's own adjustments as they are.

### 3.2 The wedge

A run whose tone settings say `auto` first runs the wedge: a run of the same style over a synthetic step wedge, with

- an image of 4 by 4 grey patches at inks `i / 15` for `i` from 0 to 15, in snake order so that neighbours along a row differ by one step;
- the tone stage at its plain defaults (`response` off, gamma 1, manual levels) and a resolution equal to the wedge's width in pixels, so the wedge's tone map is the wedge itself;
- the patches at the real tone map's pixel size, which the run works out from the image size, the page and the resolution (`tone::map_size`), because some stages work in pixels;
- patches 12 mm wide, or 8 spacings when the style's density setting is a spacing and that is wider, at least 16 pixels, and at most 40 mm and 256 pixels, so a wide spacing or a fine map keeps the wedge's run short (the plot laws found a 20 mm spacing making a 4000-pixel wedge);
- the stage that splits between the pens at its defaults, one pen, and the first pen;
- every other stage at the run's own settings.

Then for each patch, inside a square inset by a tenth of the patch on each side, the mean tone-map ink is the patch's tone and the weighted share of coverage-raster cells any path inks is its coverage. Two measures keep the patches' coverage within a few thousandths of a raster four times finer:

- **A smooth window.** Each cell is weighted by `(4 u (1 - u))^2` along each axis, `u` its place across the window, so a line cut by the window's edge barely counts.
- **Cells off the style's grid.** The wedge's marks are as regular as the style makes them: a 2 mm spacing on 0.1 mm cells covers the same cells in every patch, and the raster's quantisation shows as steps in the response. The cells are widened by `1 + (sqrt(5) - 1) / 20`, a factor no short ratio of whole numbers comes near.

### 3.3 The lookup

The 16 coverages are fitted by a least-squares polynomial of degree 5. The patches are still off by a few thousandths, and straight lines between them give the lookup a kink at every step, which on a smooth gradient prints as bands; a fifth degree smooths that and still follows the steep start of the connected renderers' response. The fit, sampled at 257 tones, is made non-decreasing with the pool-adjacent-violators algorithm (M. Ayer, H. D. Brunk, G. M. Ewing, W. T. Reid and E. Silverman, "An empirical distribution function for sampling with incomplete information", *Ann. Math. Statist.* 26(4), 1955, pp. 641-647), giving the response `c(s)` from `c_min` at white to `c_max` at full ink.

The target is a straight line over the range the style can reach: tone `t` should cover `c_min + t (c_max - c_min)`. Aiming at the style's own darkest tone rather than full black is the spike's lesson. The lookup takes each tone to the least `s` with `c(s)` at or above its target, sampled at 257 tones and interpolated linearly. Inverting a device's measured tone curve to linearise it is standard print calibration (R. Bala, "Device characterization", in G. Sharma (ed.), *Digital Color Imaging Handbook*, CRC Press, 2003).

A response that rises less than 0.01 of coverage from white to black cannot be inverted, and the tone map is left as it is; so is one whose fit is not finite. Level lines (`contours`) are such a style.

### 3.4 Running it

The runner drives the wedge run before the first stage, sharing the cache and the cancel flag and writing a slot of its own, as a trial does. Its frames pass through as frames of the tone stage, labelled `response: <stage> <label>` and stripped of geometry, which is in the wedge's coordinates. A wedge that fails fails the run with its message; a cancelled wedge cancels it.

When the tone stage's output lands, the runner reads it through the lookup and files the corrected map under a key of its own, the BLAKE3 hash of the tone stage's key and the lookup's; the stages after it read that map. The drawing keeps the map as the tone stage made it, so the tone match, lint's `dark` check and the preview's source view all see the image's own tones. A run with `response` off has no lookup, so its keys and output are unchanged.

The wedge's stages are cached under their own keys, which hold the wedge, the pen and every stage's settings, so a run whose style settings and pen have not changed finds the whole wedge cached; stamping its paths and inverting the curve take a few milliseconds. That is the caching S7.2 asks for: the response is reused while the style stage's key and the pen width stay the same.

Every run gets it: `Session::start`, the trials of "Try all styles", the budget fit's trials and the optimize trials, so each trial is corrected for its own settings. The cost: a setting after the tone stage now changes the tone map the earlier stages read, so with `auto` on a Voronoi style a renderer setting reruns the relaxation.

Limits, recorded in the brief:

- **One pen.** The wedge measures the first pen alone. With Pen use set to light to dark or by colour, the pens' combined response is not measured.
- **Count styles.** Styles that place a fixed number of marks over the whole page, the Voronoi family and `sketch`, spread them over the wedge rather than the image, so the wedge gives the shape of their response, not its level. The lookup uses only the shape.
- **Threshold styles.** `grid`, `ink` and `hatch`'s levels respond in steps; the lookup can only move the steps.
- **Paper.** Pen bleed and paper are not simulated; a scanned wedge per pen is still needed for physical accuracy.

## 4. Optimize (S7.3)

The tone error has a minimum inside the range of a style's density setting: lines too far apart cannot show the tones, and lines too close run together until the midtones turn black. On the test images the best spacing moved with the image (section 6), so a search per image earns its place, which "Tone matching" had asked for.

- `Session::optimize(budget)` returns an `OptimizeRun`, driven like a run or a fit through the `Job` trait. It searches the style's density setting, the one the budget solver fits, for the lowest tone error. The first trial is the current value; then golden-section search (J. Kiefer, "Sequential minimax search for a maximum", *Proc. AMS* 4(3), 1953, pp. 502-506) on the logarithm of the value, from a quarter to four times the current value within the schema's range, for eight trials in all. Values round as the fit's do. The stop rule counts trials, never time.
- With a budget (a fit metric and target), a trial whose drawing goes over the target scores worse than any trial within it, as does a trial with nothing to compare. When both inner trials score so, the search moves towards sparser values if either went over the budget, and towards denser ones otherwise.
- Each trial scores with `tone_match::error`, so it skips lint's other counts and the SSIM's five blurs.
- The result, `Optimum`, holds the best value and its tone error, the current value and its own, every trial, a sentence and a patch in the shape of an analysis suggestion's, or no patch when the current value is best. Nothing is applied: the search ends with the first trial's drawing in the session's slot, which is the drawing of the current settings.
- `Job::fit` became `Job::result`, the job's result as JSON, since a job now has one of two kinds of result.

## 5. Front ends

- **Python:** `Session.tone_match()` returns `{"error", "ssim", "floor", "reach"}` or `None`, and raises without a result like `stats()`. `Session.optimize(metric=None, target=None)` returns a handle whose `optimum` holds the result; unlike a fit's, it is never applied.
- **CLI:** `plotline run` prints `tone error: … ssim: …` after the stats. `--optimize` also runs the search and prints its sentence and, when it suggests a change, the `--set` option that applies it; with a `--fit-…` option every trial stays within that target instead of fitting it. `--set tone.response=auto` works as for any setting.
- **MCP:** the `run` result gains a `tone` key with the tone match, and an `optimize(metric, target, style, settings)` tool returns the result as JSON, then the stats and a preview. That makes fourteen tools.
- **GUI:** the stats line shows the tone error and SSIM, with a tooltip. An Optimize button beside Fit to target runs the search, within the typed target if there is one, and shows what it found in the settings' own labels, with an Apply button. `response` appears in the Image section from the schema. A tour step, "Match the image", explains the three.

## 6. Measurements and defaults

An example kept outside the repository ran every listed style at its defaults on the still life, the scene and the gradient, A4 with a 0.3 mm pen, with `response` off and `auto`; the brief's "Auto tone response" row records the numbers. In short:

- The connected renderers, `line`, `tree`, `mesh` and `cells`, gain on every image: their coverage follows the square root of their density, the clearest bend any style has. A density exponent of 2, which their help text already names (Ahmed, 2015), gains more, and `auto` on top of it lowers the error of `line` and `tree` further on every image and of `mesh` on two.
- `waves`, `spiral`, `halftone` and `dither` already cover the paper close to in proportion to the tone map, so `auto` changes their tone error by about 1 % (`halftone`, `dither`) or up to a fifth either way (`waves`, `spiral`).
- The threshold styles, `grid`, `ink` and `contours`, cannot follow tone, and `hatch`, `circles`, `flow`, `hilbert`, `sketch`, `stipple` and `dashes` are mixed.

So `auto` is the default for no style. For the Voronoi family, where it pays, a default of `auto` would also give each renderer its own corrected tone map and stop a switch of renderer from reusing the points, the reason the six share a density exponent of 1 (brief, "Shared Voronoi defaults"). The golden files therefore did not change. The gamma of 1.9 for `waves` and `spiral` ("Rippling gamma") still holds: their response is close to linear in the tone map, so the gamma corrects the image's encoding, a different thing that `auto` leaves alone.

## 7. Tests

- Tone match: coverage equal to the tone map scores 0 and SSIM 1; a scale and a floor are not errors; a bent response scores worse; a blank or flat drawing has none; cells average into pixels, and a pixel smaller than a cell takes the cell under it.
- Response: pool-adjacent-violators on known sequences; the inverse of a known curve; an S-shaped response is straightened; noise on a straight response leaves a smooth rising lookup; a flat or falling response gives none; the wedge's layout; `map_size` is the size the tone stage makes.
- Runs: `auto` runs the wedge first as frames of the tone stage, keeps the uncorrected tone map in the drawing and changes the marks; a second run is all cached; a renderer setting reruns the placer under `auto` and not under `off`; a cancel during the wedge cancels the run.
- Law: on a gradient, `auto` lowers the tone error against `off` for `line`, `tree` and `mesh`. There is no property suite on `main` yet, so the law is a core test.
- Optimize: golden-section finds a minimum inside the bracket, stays inside a quarter to four times the current value, is pushed towards sparser values by a budget, and finds nothing when no trial is within it; through the session it leaves the settings alone, ends with the current settings' drawing and keeps the chosen trial within the budget.
- Pinned: the tone match and the `auto` tone error of `waves` on the gradient, natively and in the WebAssembly smoke test.
- Hosts: Python `tone_match()` and `optimize()`, the CLI's tone line and `--optimize`, the MCP `run` result's `tone` and the `optimize` tool, the GUI protocol and tour tests. `settings_matter` lists `contours`' `response` as a setting that changes nothing.
