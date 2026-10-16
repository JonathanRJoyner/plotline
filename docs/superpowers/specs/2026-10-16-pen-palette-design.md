# plotline spec 13: pen and palette suggestions

Date: 2026-10-16. Status: decided by the builder under the owner's standing delegation; written for review. Not built yet.

For a colour image, the analysis suggests a set of two to four pens from the ink list (`pens::INKS`), and Pen use set to by colour, as one suggestion with a patch. Today the `colour` suggestion is a note with no patch ("add pens in those colours"). This spec turns it into a change the user applies with one click, through the GUI's Apply, `plotline analyse` and MCP `analyse_image`.

## 1. Method: greedy fit under the colour model

Dominant colours (k-means or median cut) matched to inks were considered and left out: the pens are transparent inks that mix, so a green area may be best drawn by yellow over blue, and the inks that reproduce an image best are not its most common colours. The choice instead runs the colour model the Pen use stage already uses (`stages/split.rs`: Demichel and Neugebauer, solved by coordinate descent) and picks the inks whose reproduction is closest to the image.

- **Samples.** The block-averaged red, green and blue planes `measure` already makes are binned 8 × 8 × 8 on their encoded values. Each non-empty bin gives one sample, the mean colour of its pixels, weighted by their count. At most 512 samples.
- **Error of an ink set.** For each sample, `split::coverages` solves the set's coverages for the sample's colour as the target (the colour as stored, so `rescale` is not needed), and the reproduced colour is `R_c = prod_i (1 - a_i k_ic)`. The error is the CIE 1976 colour difference ΔE*ab between the sample and `R`, after decoding sRGB (IEC 61966-2-1) to CIE XYZ under D65 and then to CIELAB (CIE 15:2004, *Colorimetry*, 3rd ed.). The set's error is the count-weighted mean.
- **Selection.** Forward stepwise selection (T. Hastie, R. Tibshirani and J. Friedman, *The Elements of Statistical Learning*, 2nd ed., Springer, 2009, §3.3.2): start from no ink, and at each step add the ink from the ten that lowers the error most, ties to the earlier ink in `INKS`. Stop at four inks, or when the best addition lowers the mean ΔE*ab by less than 2.3, about one just noticeable difference (M. Mahy, L. Van Eycken and A. Oosterlinck, "Evaluation of uniform color spaces developed after the adoption of CIELAB and CIELUV", *Color Research and Application* 19(2), 1994). Exhaustive best-subset selection would solve 385 sets of up to four, an estimated ten times the work of the greedy fit's 34, on every image in the browser; it is not measured.
- **Gate.** The fit runs only when `colourfulness` passes 40, the existing colour cut-off. A grey image gets an empty palette.
- **Determinism.** `cbrt` and `pow` for the sRGB decode and CIELAB go through new `math::cbrt` and `math::powf` wrappers over `libm`, as the source-scan test requires.

`ImageStats` gains `palette` (ink names, lightest first) and `palette_error` (its mean ΔE*ab), both empty or 0 for a grey image.

## 2. The suggestion

Kind `colour`, as before. It is made when `colourfulness > 40` and the palette has at least two inks, and is left out when it is already in effect: Pen use is by colour and the pens' colours, as a set and ignoring case, are the palette's.

```json
{"kind": "colour",
 "reason": "This is a colour image. Yellow, red, blue and black pens draw its colours best, with Pen use set to by colour.",
 "patch": {"pens": [{"name": "Yellow 0.3", "width_mm": 0.3, "color": "#f5c400"}, ...],
           "settings": {"split": {"mode": "by_colour"}}}}
```

- The pens come lightest first by BT.601 luma, ties in `INKS` order, since pens plot in list order and light inks go first.
- Each pen takes the width of the session's first pen and is named after its ink and that width, like the default "Black 0.3".
- `Suggestion::reason` becomes a `String`; the JSON is unchanged. `suggest` takes the session's pens.

## 3. Front ends

- **GUI.** `Patch` gains `pens`; the worker's `apply` sets them after the style and before the settings. The pens tour step mentions the suggestion. A Playwright flow loads a colour image made in the page with a canvas, clicks the colour suggestion's Apply, and checks the pen list and Pen use.
- **MCP.** `analyse_image`'s description and the instructions say a patch's `pens` apply with `set_pens`.
- **CLI.** `plotline analyse` prints the new patch with no change.

## 4. Tests

- `math`: CIELAB of white, black and sRGB red against the CIE values.
- `analysis`: colour bars give a palette of two to four inks with an error lower than black alone; a grey ramp gives none; the patch's pens are lightest first, take the first pen's width and come with by colour; the suggestion is left out once in effect; the reason names the inks.
- Python: `analysis()` on a colour image carries a pens patch that `set_pens` accepts.
- `protocol.test.ts`: `apply` with `pens` sets them.
- The Playwright flow above. The regression gallery does not change, since no drawing does.

## 5. Out of scope

- Inks other than the ten in `INKS`, and pens from the bank (all black).
- Measured ink colours.
- Pen widths fitted to the image.
