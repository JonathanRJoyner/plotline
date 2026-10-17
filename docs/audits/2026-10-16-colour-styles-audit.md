# Audit: colour, optimize, faithfulness and Try all styles

**Date:** 2026-10-16, on `main` at 07776a9 (0.3.0.dev0).

**Why:** the owner kept finding problems with colour selection, Optimize, faithful reproduction, and styles (mostly Grid dots and Dither) that seem not to show up. This audit ran every style and every colour, optimize and preview feature over a varied set of pictures, measured what a person would judge, and traced each problem to its cause. Each finding lists the evidence, the cause in the code, and the fixes, best first. Nothing here is fixed yet.

## Contents

- [Summary](#summary)
- [Method](#method)
- [Findings](#findings)
- [Measurements](#measurements)
- [How to run the audit again](#how-to-run-the-audit-again)

## Summary

The reported problems are real and have a few shared causes, not one bug per feature.

| What you see | Findings |
|---|---|
| Grid dots and Dither don't show up, in the preview or in Try all styles | [F1](#f1-grid-dots-and-dither-draw-at-6-to-9--coverage-so-they-look-blank-confirmed) (they really are 6 to 9 % grey at most), [F9](#f9-try-all-styles-shows-dot-styles-darker-than-they-plot-confirmed) (the Try all cells overstate them, so the page then looks broken), [F8](#f8-the-colour-suggestion-puts-the-lightest-pen-first-so-a-reset-draws-everything-in-yellow-confirmed) (after a reset, everything draws in yellow) |
| Colours come out wrong | [F4](#f4-waves-spiral-and-mosaic-muddy-every-colour-by-colour-confirmed) (Waves, the style the GUI opens on, adds black to every colour), [F5](#f5-line-styles-ink-areas-that-should-stay-white-worst-with-several-pens-confirmed) (line styles ink white and other pens' areas), [F6](#f6-every-pen-gets-the-same-point-budget-whatever-share-of-the-colour-it-draws-confirmed) (minor colours get as many dots as major ones), [F8](#f8-the-colour-suggestion-puts-the-lightest-pen-first-so-a-reset-draws-everything-in-yellow-confirmed), [F12](#f12-the-colour-palette-picks-miss-accent-colours-and-misjudge-the-pen-library-confirmed) (palette picks) |
| Optimize suggests odd values or nothing | [F2](#f2-the-tone-match-cannot-see-contrast-or-colour-so-faithful-can-mean-invisible-confirmed) (its score ignores contrast and colour), [F3](#f3-optimize-fit-and-the-lint-fixes-pull-dot-styles-the-wrong-way-confirmed), [F10](#f10-optimize-can-finish-with-no-result-and-the-gui-then-says-nothing-confirmed) |
| The drawing doesn't look like the picture | [F1](#f1-grid-dots-and-dither-draw-at-6-to-9--coverage-so-they-look-blank-confirmed), [F2](#f2-the-tone-match-cannot-see-contrast-or-colour-so-faithful-can-mean-invisible-confirmed), [F5](#f5-line-styles-ink-areas-that-should-stay-white-worst-with-several-pens-confirmed), [F7](#f7-fixed-point-budgets-cannot-draw-dark-pictures-confirmed) (dark pictures vanish in the point styles), [F11](#f11-cells-tree-and-single-line-draw-across-white-paper-confirmed) (Cells draws over white paper), [F13](#f13-the-looks-were-chosen-on-code-drawn-pictures-and-do-not-all-hold-on-art-confirmed) |

The three causes under most of it:

1. **A dot is one pen touch.** Grid and Dither put a 0.3 mm dot in a 1 mm cell, so even solid black is a 7 % grey. Drawing each dot as a filled disc the size of its cell, with the stipple renderer the engine already has, took reach from 0.06 to 0.62 and SSIM from 0.67 to 0.90 at the same dot count for about a quarter more plot time (F1).
2. **The faithfulness score ignores contrast and colour.** The tone match divides out how dark a drawing gets and compares luminance only, so Optimize, the Faithful look and the stats line reward near-blank drawings and cannot see wrong colours (F2, F3).
3. **Colour mode inherits single-pen settings that don't carry over.** The rippling styles' gamma becomes black ink in every colour (F4), line styles ink every pen's lane wherever it has 2 % of its colour (F5), and each pen gets the whole point budget (F6).

What works well: Halftone, Hatching, Waves, Spiral, Hilbert, Mosaic and Scribble get closest to the pictures with one pen (a median 40 to 58 % of the way from blank paper to the picture by the colour measure), and Halftone, Ink, Hatching, Hilbert, Scribble and Mosaic by colour (62 to 79 %); Hatching and Halftone keep each colour to its own pen. Every style ran without an error on all twenty pictures in all four pen setups, and Try all styles worked on a 4000 x 5812 photo in the browser. Every Try-all trial drew exactly the drawing a real switch to that style draws. Light-to-dark Pen use showed no problem of its own.

### Suggested order of work

| Step | Findings | Size | Moves drawings |
|---|---|---|---|
| 1. Disc dots for Grid and Dither | F1, most of F9 | small: a renderer swap and a size rule | yes: gallery, looks, style guide |
| 2. Reset keeps Pen use; warn when a light pen draws alone | F8 | small | no |
| 3. Optimize: say why when nothing scored; clear stale sentences and chips | F10, F14 | small | no |
| 4. Colour: gamma after the colour solve; lanes lift below a bare line's coverage; point budget shared by ink | F4, F5, F6 | medium | yes |
| 5. Contrast and colour error beside the tone error, used by Optimize and the looks | F2, F3 | medium | Optimize's picks and the looks |
| 6. Re-measure the looks on a wider set of pictures | F13 | medium (compute) | the looks |
| 7. Cells edge limit; palette fixes; point count from the picture | F11, F12, F7 | small to medium | yes |

## Method

**Pictures.** Twenty, chosen to span the kinds of art people bring:

- 16 made for this audit with a local image model: a portrait photo, a flat vector fox (red, orange, black), an anime character on black, a watercolour landscape, an ink comic street, a pop-art poster, an oil still life, a black and white cathedral photo, pixel art, a two-colour geometric logo, a pencil rose, a sunset photo, a high-key white cat, a night street with neon, a stained-glass window and a pastel children's book page.
- 2 drawn in code for the audit: flat red, yellow and black shapes on white (the owner's failing case), and pale pastel blobs on cream.
- The engine's own `synthetic::still_life` and `synthetic::scene`.

None of them is in the repository: the harness reads pictures from a folder, so the audit can be run again on any set.

**Runs.** For each picture, on A4 with the default 10 mm margin and 0.3 mm pens, `crates/core/examples/audit.rs` drew every listed style at its defaults in four pen setups:

| Setup | Pens | Pen use |
|---|---|---|
| `mono` | Black | one pen |
| `ryb` | Yellow, Red, Black (the owner's report) | by colour |
| `greys` | Grey, Black | light to dark |
| `palette` | the analysis's colour suggestion, when it makes one | by colour |

It then ran Try all styles (`Session::trial`, from Waves as the GUI does) in every setup and compared each trial with the real run, ran Optimize with no budget and with a 60-minute budget and applied each suggestion, and drew every look.

**Measures.** For each drawing: path count, plot time, the plot lint findings, the tone match (error, SSIM, floor, reach), the share of a 150 px Try-all cell with visible marks (the law's measure), and a colour comparison the engine does not have: the drawing rendered at 8 px per mm, averaged to 2 px per mm, blurred with the tone match's 1.5 mm eye filter, and compared with the source image in CIE Lab. `gain` is how much closer to the image the drawing is than blank paper, as a share of the paper's ΔE: 0 % means the drawing looks no more like the picture than an empty page, 100 % a perfect match.

**GUI.** The same flows by hand in the browser build (`npm run dev` with a fresh `npm run wasm`): loading each kind of picture, applying the colour suggestion, switching styles, Reset to defaults, Try all styles with one pen and three, the lint fixes, Optimize, the pens dialog, and a 4000 x 5812 photo.

**Probes.** Each suspected cause was checked with a single run that changes one thing (`crates/core/examples/probe.rs`), or by fitting palettes with chosen libraries (`crates/core/examples/palette.rs`). The disc-dot fix in F1 was measured with a temporary style that was not kept.

## Findings

Numbered by priority. Each one says how sure we are: **confirmed** means measured and reproduced, **likely** means measured once or reasoned from the code.

### F1. Grid dots and Dither draw at 6 to 9 % coverage, so they look blank (confirmed)

**What a person sees.** Pick Grid dots or Dither on any picture and the page stays almost white, in the main preview and on paper. On a flat poster of a red disc, a yellow square and a black triangle, the black triangle comes out as a 7 % grey. In colour, the red disc is a faint pink and the yellow square almost vanishes. On photos the subject is barely there (see the measurements: every image, both styles).

**Cause.** Both styles put one pen touch, a dot as wide as the pen, in each grid cell (`DotRenderer` in `crates/core/src/stages/grid.rs`). A cell is `spacing_mm` wide, 1 mm by default, and the default pen is 0.3 mm, so a cell at full ink is covered `π (0.15)² / 1² = 7 %`. Error diffusion (`stages/dither.rs`) decides *where* dots go as though a dot inked its whole cell, so it reproduces the tones faithfully in shape but compressed into 0 to 7 %. Grid is worse in one respect: it thresholds at 0.5, so a light colour such as yellow (luma ink about 0.2) is dropped completely in one-pen mode.

| Spacing with a 0.3 mm pen | Coverage at full ink | Dots on A4 at full ink | Plot time at full ink |
|---|---|---|---|
| 1.0 mm (default) | 7 % | 52,600 | about 3 h |
| 0.707 mm (where Optimize stops) | 14 % | 105,000 | about 6 h |
| 0.5 mm | 28 % | 210,000 | about 12 h |
| 0.3 mm (what the lint's Apply sets) | 78 % | 585,000 | about 33 h |

A denser grid can never fix it at a sane plot time, because every dot costs a pen lift.

**Fix (recommended): draw each dot as a filled disc about as wide as its cell.** The stipple renderer already draws filled discs (`StippleRenderer`, shape `auto` gives a spiral fill when the dot is wider than the pen). Measured by registering `DitherPlacer` and `GridPlacer` with `StippleRenderer` at `size_mm` 0.9 and `vary` 0, on A4 with a 0.3 mm black pen:

| Image | Style | Dots | Plot time | Reach | Tone error | SSIM |
|---|---|---|---|---|---|---|
| portrait photo | dither now | 25,760 | 92 min | 0.06 | 0.057 | 0.67 |
| | dither, disc dots | 25,760 | 116 min | 0.62 | 0.024 | 0.90 |
| | grid now | 30,004 | 105 min | 0.08 | 0.128 | 0.49 |
| | grid, disc dots | 30,004 | 133 min | 0.84 | 0.119 | 0.63 |
| children's book | dither now | 30,817 | 109 min | 0.06 | 0.063 | 0.60 |
| | dither, disc dots | 30,817 | 139 min | 0.62 | 0.024 | 0.88 |
| flat fox | dither now | 16,547 | 60 min | 0.06 | 0.043 | 0.78 |
| | dither, disc dots | 16,547 | 75 min | 0.62 | 0.019 | 0.91 |

Reach went up tenfold, the tone error halved, the dot count stayed the same, and the plot time grew by about a quarter, since a 0.9 mm spiral is about 2 mm of drawing against a 0.2 s lift. By colour with the fox's palette, disc dither reached 0.89 against 0.11. The audit's contact sheets stay outside the repository with its pictures.

How to build it:
- Give `grid` and `dither` the `stipple` renderer in place of `render`, with `size_mm` tied to the spacing. The simplest rule is a new stipple setting, `size_of_cell` (a share of the placer's spacing, default 0.9), so changing the spacing keeps the coverage; or keep `size_mm` and have the look and Optimize move both.
- Keep `vary` at 0 for dither, since error diffusion already encodes the tone in where dots go. Grid could take `vary` 1, but that is the existing `halftone` style; consider merging grid into halftone and keeping grid's look as a halftone preset with a threshold.
- This moves drawings, so it needs the gallery snapshot, the looks re-measured (`--example looks`), the style guide's pictures, and a decision file.

**Other fixes, not recommended alone:**
- Default spacing tied to the pen width (spacing = 1.2 x pen) gives the coverage but at 400,000 lifts on A4.
- Raising the `dark` lint finding from Info to Warning (F3) only tells the user.

### F2. The tone match cannot see contrast or colour, so "faithful" can mean invisible (confirmed)

**What a person sees.** The stats line's tone error and SSIM praise drawings that are plainly wrong. Dither's Faithful look on the flat poster scores SSIM 0.93 and tone error 0.017 while its darkest area is a 9 % grey. Applying the lint's fix to colour Dither made the picture far more faithful by eye and the tone error *worse* (0.195 to 0.235). Waves by colour at its default gamma, with black lines all over the red disc (F4), scores better (0.182) than at gamma 1 with clean colours (0.193). Disc dither by colour, plainly better, scores 0.134 against 0.130.

**Cause.** `crates/core/src/tone_match.rs` fits `coverage = floor + gain * tone` and scores `(coverage - floor) / gain` against the tone. Dividing by the gain throws away how dark the drawing gets: a drawing at 6 % reach with the right shape scores as well as one at 80 %. The comparison is also with the tone map, which is luminance only, so a hue error (black over red) does not register. The decision that added it chose this on purpose ("no style reaches black ... judging against the tone map itself chases black"), but Optimize, the looks and the GUI now treat it as *the* faithfulness score, which it is not.

**Fix (recommended): report and use three numbers, not one.**
1. **Shape**: today's tone error, unchanged.
2. **Contrast**: `reach - floor`, already computed. Show it in the stats ("reaches 7 % at black") and make Optimize and the looks reject a value whose contrast is under, say, half the best trial's, the way they already reject new lint warnings.
3. **Colour** (by colour only): the mean ΔE*ab between the drawing seen through the same eye filter and the source image, over the image's area. The audit harness (`crates/core/examples/audit.rs`, `colour_match`) computes it in about 60 ms at 2 px per mm. Use it as Optimize's score when Pen use is by colour.

Also cap or hide the tone error when the gain is small: `contours` reports tone errors up to 12.4, which mean nothing to a user (see F14).

### F3. Optimize, Fit and the lint fixes pull dot styles the wrong way (confirmed)

**What a person sees.**
- On Grid dots the tone error barely moves with the spacing (F2), so Optimize drifts: on the poster it suggests a *sparser* grid (1.0 to 1.414 mm), halving the already faint coverage (reach 0.07 to 0.04); on most photos it goes denser to the time limit for a 1 % better score.
- On Dither, Optimize goes denser until the time limit stops it at 0.707 mm (reach 0.06 to 0.14): still near-blank, now twice as long.
- By colour on Dither the GUI says "Spacing at 1 mm already follows the image's tones best, with a tone error of 0.195 and 84.6 min to plot" about a page that is nearly white.
- The `dark` lint finding is only Info ("Where the image is darkest, the drawing covers only 7 % of the paper."), and its Apply sets dither to 0.3 mm: 265,879 dots and about 15 hours, on the poster, by colour.
- For most styles Optimize is really "plot for longer". Fifteen of the nineteen density styles went denser on nearly every picture, at a median of 76 % to 299 % more plot time. For Grid, Dither, Cells, Circles and Mosaic the value was picked by the limit, not by a minimum: on 17 to 20 of 20 pictures the next denser value was over the time limit (twice the current time, or an hour) or added a lint warning. Grid paid 97 % more time for a 2 % better score. Only Hatching, Waves and Contours tended to go sparser, and Halftone and Truchet stayed put (see [Measurements](#measurements)).

**Cause.** The score (F2) and the dot geometry (F1). `more_dark` in `crates/core/src/lint.rs` only knows how to shrink a spacing or raise a count, and caps nothing by time. Optimize's limits (`docs/decisions/2026-10-16-optimize-limits.md`) stop the time from running away, but cannot find what the style cannot draw.

**Fix.** F1 and F2 first; after them Optimize has something to find. Then:
- Make `dark` a Warning when the reach is under 15 %, and let its patch prefer a dot size (after F1) over a spacing, and never exceed the Optimize time limit (twice the current time or an hour).
- Pick the cheapest value whose score is within a few per cent of the best, rather than the best itself, so a search that only gains 1 % for twice the time keeps the current value. Grid's median gain was 1 % for 97 % more time.
- When a style's best trial still reaches under 15 %, say so in Optimize's sentence ("Dither cannot get darker than 7 % with a 0.3 mm pen at a sane plot time; try a wider pen or Halftone").

### F4. Waves, Spiral and Mosaic muddy every colour by colour (confirmed)

**Status:** fixed on 2026-10-17 with the recommended fix ([decision](../decisions/2026-10-17-colour-lanes.md)).

**What a person sees.** With pens chosen by the colour suggestion and Pen use by colour, Waves (the style the GUI opens on), Spiral and Mosaic draw black lines across red and yellow areas and red and yellow lines across black. The red disc of the poster comes out brown, the yellow square mustard. Hatching, at the same pens, is clean.

**Cause.** These styles default to a tone gamma of 1.9 ("Rippling gamma" in the brief). In `by_colour` mode the split stage carries the Image settings over to colour by the luminance rule `ink'_k = t + beta (ink_k - u)` (`crates/core/src/stages/split.rs`): a darker tone `t` than the colour's own luma `u` adds the same ink to every channel, which is grey, which the solver gives to the black pen. Gamma 1.9 darkens every mid-tone, so every colour gains black. Verified on the poster with Yellow, Red and Black pens: at gamma 1.9 the black pen draws 29.0 m, at gamma 1.0 15.3 m, and the red disc and yellow square are clean only at 1.0.

**Fix (recommended).** In `by_colour` mode, solve the colours from the untouched image and apply the style's tone curve to each lane's coverage afterwards, `a' = 1 - (1 - a)^gamma` (the curve the tone stage applies to ink, `crates/core/src/stages/tone.rs`), rather than darkening the colour before the solve. That keeps the reason for the rippling gamma (the lines ink in proportion, so mid-tones need more) without inventing grey. Simpler stop-gap: a style default of gamma 1 when the split is by colour, which the GUI's style switch and the colour suggestion's patch can carry (`settings.tone.gamma` in the suggestion).

### F5. Line styles ink areas that should stay white, worst with several pens (confirmed)

**Status:** the lane rule fixed on 2026-10-17 for Waves, Spiral and Hilbert, lifting below half of a bare line's cover rather than all of it ([decision](../decisions/2026-10-17-colour-lanes.md)); Flow keeps 2 % until it inks in proportion, and one pen and the backgrounds are open.

**What a person sees.** Waves and Spiral cover the whole page, white areas included, with a 15 % grey of straight lines; with several pens, every pen draws its own lines over every area that has 2 % of its ink, so colours mix where the image has one colour. Halftone draws an even field of pen-wide dots over the fox's light grey background (ink about 0.06): true to the grey, but it reads as texture where the artist meant white.

**Cause.**
- `waves.rs`, `spiral.rs` (and `flow.rs`, `hilbert.rs`) lift the pen only below `LANE_MIN_INK = 0.02`, and only when the drawing is split between pens. A bare line at 2 mm with a 0.3 mm pen already covers 15 %, so any lane ink from 0.02 to about 0.15 gets far more than it asked for.
- With one pen they never lift, by design (the brief: "some, such as `waves`, ink their white").
- `halftone` defaults its threshold to 0.02, and the stipple renderer never draws a dot smaller than the pen, so any off-white background becomes a dot field.

**Fix.**
- Lift where the lane's ink is below what a bare line already covers, `pen_width / spacing`, not 0.02, for every pen count. Offer a "Skip white" setting (default on with several pens) if one pen should keep drawing whole lines.
- For backgrounds, let the analysis suggest auto levels (or a white point) when the border tone is near white but not white, as for the fox, so the paper stays paper in every style. Raising `halftone`'s threshold to about 0.05 is the style-local alternative.

### F6. Every pen gets the same point budget, whatever share of the colour it draws (confirmed)

**Status:** fixed on 2026-10-17 by colour, keeping `count` per pen so the total stays as before ([decision](../decisions/2026-10-17-colour-lanes.md)); light to dark keeps a whole budget per pen, which measured better there.

**What a person sees.** By colour, Stipple, Single line, Dashes, Tree, Mesh, Cells and Scribble draw 5,000 points for each pen. A pen whose colour covers a tenth of the image crowds 5,000 dots into it, so minor colours come out too strong, and the pen that carries most of the picture is too sparse.

**Cause.** The Voronoi placer reads `count` from its settings for each lane (`crates/core/src/stages/voronoi.rs`, `sample_by_density(&tone, s.exponent, s.count as usize, ...)`), and each lane's density is normalised to its own map. The poster drew [5000, 5000, 5000] for yellow, red and black whatever their areas.

**Fix.** Share the count between lanes in proportion to each lane's total ink (the sum of its map), with the total equal to `count`. The split stage already has every lane's map, so the share can be passed in the lane context or worked out by the placer from its own map and a total the split stage provides. `sketch`'s `max_lines` is per pen too, but a sketch lane stops when its ink is used up, so it matters less there.

### F7. The point styles ignore how much ink a picture needs (confirmed)

**What a person sees.**
- Dark pictures come out light. On the night street, Stipple is a uniform field of dots and Single line a uniform maze: the picture is gone. Across all twenty pictures Stipple's darkest areas reach a median of 8 % coverage, Dashes 9 %, Single line 13 % and Tree 14 %; the `dark` finding fires on 17 of 20.
- Pale pictures come out dark. On the pastel blobs, Mesh, Scribble, Cells, Waves and Spiral draw a page further from the picture than blank paper (Mesh by colour: 310 % worse than paper by the colour measure; one black pen: 108 % worse), and the white cat suffers the same way in Mesh.

**Cause.** The Voronoi placer's `count` is an absolute number of points, 5,000 by default, and it spreads them by the tone map's *relative* density (`sample_by_density`, then weighted Lloyd relaxation). So every picture gets about the same amount of ink: too little for a dark one, too much for a pale one. The renderers that join the points (Mesh, Cells, Scribble) then draw lines whose length depends on the point spacing, not on the tone.

**Fix.** Start the count from the picture: the total ink of the tone map times the page area, over the area one mark covers, at a target coverage, capped by the time limit. Then a pale picture gets few points and a dark one many. As a cheaper step, the analysis could suggest auto levels for pale pictures and a higher count for dark ones (mean ink over 0.6), and the `dark` finding (F3) would fire as a Warning.

### F8. The colour suggestion puts the lightest pen first, so a reset draws everything in yellow (confirmed)

**What a person sees.** Apply "This is a colour image. Yellow, red and black pens draw its colours best..." and it works. Press **Reset to defaults** and every style draws in yellow only, in the main preview and in Try all styles. Setting Pen use back to one pen by hand does the same.

**Cause.** The palette is listed lightest first (`palette_of` in `crates/core/src/analysis.rs`), as the Pens dialog advises ("List light inks first"), because pens plot in list order. In one-pen mode the *first* pen draws everything. `Session::reset_settings` resets Pen use with the rest. So Reset swaps a three-colour drawing for the lightest pen alone. The note "Pen use is set to one pen, so only the first pen draws." is there, but nothing says that pen is yellow.

**Fix (recommended).** Treat Pen use as part of the pen setup, like the pens themselves: Reset to defaults leaves it alone when there is more than one pen. Also make one-pen mode use the darkest pen, or warn when the first pen's luma is above 0.6 ("Only Yellow draws, and a light ink alone shows little").

### F9. Try all styles shows dot styles darker than they plot (confirmed)

**What a person sees.** In Try all styles, Grid dots and Dither look like proper pictures. Pick one and the page is near-blank (F1). The grid promises what the style does not deliver: Dither's cells have visible marks on a median 64 % of their pixels and Grid's on 48 %, while their drawings reach 6 and 7 % at black.

**Cause.** The cells are drawn with every mark at least 2 px wide in a 600 px picture (`render_png_shrunk`, `THUMB_SHRINK` in `web/src/app.tsx`, from [dots at small sizes](../decisions/2026-10-16-dots-at-small-sizes.md)). That is 0.7 mm on A4, so a 0.3 mm dot is drawn 5 times its area. It fixed the blank cells, but now the cell and the plot disagree.

**Fix.** F1 removes most of the gap. Then either draw the cells at true coverage (area-weighted, the way the main preview's disc does) and accept faint cells for faint styles, or keep the floor and add a "very light" badge to a cell whose drawing covers under 15 % at its darkest. The coverage needs no lint measure: dots times pen area over the inked area is enough.

### F10. Optimize can finish with no result, and the GUI then says nothing (confirmed)

**What a person sees.** Press Optimize on a drawing with no marks (Grid dots or Hatching on a pale picture, where the thresholds keep everything out) and the search runs, ends, and shows no sentence at all. The stage chips under the preview then read "waiting" for every stage although nothing is running.

It also happens on drawings with marks: Contours' search came back with no result on 7 of the 20 pictures, because its outlines follow the tone too little for the tone match (its gain stays under `MIN_GAIN`).

**Cause.** `OptimizeRun::result` returns `None` when no trial has a tone error within the limits: when every trial draws nothing, or when no trial's drawing follows the tone enough to have a tone error (`crates/core/src/optimize.rs`). The run still ends in `Done`. The GUI only shows a result when one comes (`if (e.optimum) setOptimum(e.optimum)` in `web/src/app.tsx`), and keeps the previous one hidden. The CLI prints "optimize: no trial stayed within the target" (`src/plotline/cli/main.py`), which is wrong when no target was given, and the MCP `optimize` tool returns a bare `null`. The audit harness assumed a `Done` search has a result and panicked on it, so any new caller has the same trap.

The chips: a search's frames carry the stage `optimize`, which replaces the progress map (`setProgress({ [e.stage]: ... })`), and the search ends on the cached first trial, which sends no stage frames, so every stage is left "waiting".

**Fix.**
- Return a result with no patch and a reason when nothing scored: "No value of Spacing draws anything here; lower the threshold or use auto levels first." Better still, refuse to start on an empty drawing with the `empty` lint finding's message.
- In the GUI, clear the old sentence when a search starts, and show the reason when one comes back empty.
- At the end of a fit or a search, set every stage chip to done.

### F11. Cells, Tree and Single line draw across white paper (confirmed)

**What a person sees.** Cells (Voronoi cell outlines) draws long straight lines fanning out over the white paper around every inked area: on the poster, red, yellow and black lines radiate from each shape to the page edge. Tree and Single line draw long jumps between distant clusters.

**Cause.** `CellsRenderer` (`crates/core/src/stages/edges.rs`) draws every Voronoi edge with two ends, clipped to the tone map's rectangle. Points sit only where there is ink, so the cells on the edge of an inked area are huge and reach across the empty paper. `DelaunayRenderer` (Mesh) has `max_edge_mm` for exactly this; Cells has no setting. Tree must connect every point, so it crosses gaps by design.

**Fix.** Give Cells a `max_edge_mm` like Mesh: drop a Voronoi edge when the Delaunay edge it crosses is longer than the limit (8 mm by default), which removes the cells of points with no near neighbour on that side. For Tree, the same limit turns the tree into a forest (more pen lifts, no lines over white); offer it as a setting, default off, or on when Pen use splits the drawing.

### F12. The colour palette picks miss accent colours and misjudge the pen library (confirmed)

**What a person sees.**
- With a pen library of Black, Red, Blue (#1f4fb4), Yellow and Green, the logo of dark green mountains and an orange sun gets Yellow, Red and Black, and then the note "Pens you don't have would draw its colours closer: with a blue pen the mean colour difference falls from 3.0 to 0.6". The user owns a blue pen.
- Small but vivid areas, such as eyes or a red accent, never earn a pen.

**Cause.**
- `additions` (`crates/core/src/analysis.rs`) decides that the library owns an ink by an exact hex match. Any pen whose colour was picked by eye, or taken from a maker's swatch, differs from the built-in ink by a few units, so the note suggests buying a pen the user has. The library fit itself stopped one pen short because adding the user's own blue gained just under the 2.3 ΔE threshold, while the built-in blue, two units away, gained just over it.
- `mean_error` weights each colour by its pixel count, white paper included. On a picture with a large white background the mean ΔE is small whatever the inks, and a colour covering 3 % of the pixels moves the mean by too little to pass the 2.3 gain, however wrong it is drawn.

**Fix.**
- Count an ink as owned when a library pen is within a small ΔE of it (5 is a safe start), and fit the library's own near-duplicate in its place.
- Leave the paper out of the mean (weight only the pixels with ink above a small threshold), or score the palette by a high percentile of ΔE rather than the mean, so accents count.
- After the forward selection, try one round of swaps among the library's own pens, as `additions` already does for the built-in inks.

### F13. The looks were chosen on code-drawn pictures and do not all hold on art (confirmed)

**What a person sees.** Across the twenty pictures most looks do what they say: every Faithful look lowers the median tone error, by 7 to 64 %. Some do not hold:
- Waves' Fine has 1.6 times Balanced's median tone error and Hatching's Fine 1.4 times, past the 1.25 the looks decision allows a Fine look. At 1 mm Waves' bare lines cover 30 % of the page, white included (F5).
- Waves' Faithful barely differs from Balanced on art (0.93 times the tone error, 1 point less gain).
- Dither's Quick has 1.5 times Balanced's tone error at a third of the time, and Dither's Faithful is still near-blank (F1).
- Several Faithful looks cost 2.7 to 4 times Balanced's plot time (Tree, Circles, Flow, Sketch), within the rule only because an hour is always allowed.

**Cause.** The looks were measured on `still_life`, `scene` and `gradient` only ([the looks decision](../decisions/2026-10-16-looks.md)), all smooth and mid-toned, and scored by the tone error alone (F2).

**Fix.** After F2's contrast and colour numbers exist, re-run `--example looks` on a wider set: the twenty pictures here, or a set of code-drawn ones that cover the same ground (flat colour shapes, a dark picture, a pale one, fine line art, a busy texture). A look should beat Balanced on the median picture by every measure it claims, not on the mean of three.

### F14. Smaller problems

- **Contours' tone error is meaningless.** It reports values up to 12.4 (median 0.67 with one pen, 1.97 by colour), where the styles that follow the picture well score 0.02 to 0.1, because its outlines barely follow the tone and the fit divides by a small gain (`MIN_GAIN = 0.01` in `tone_match.rs`). Hide the tone error, or say "does not follow tone", when the gain is under about 0.15, and leave such styles out of Optimize.
- **The line-art suggestion fires on pale pictures.** On a pastel picture (two flat pale tones) the analysis suggests the Ink style, and Ink then draws nothing. The rule (`two_level >= 0.85 && mid <= 0.15` in `analysis::suggest`) does not check that the dark class is dark. Add `p_low < 0.5`, or fold auto levels into the line-art patch.
- **A stale Optimize sentence stays after loading a new image.** "Spacing at 1 mm already follows the image's tones best..." from the previous picture stays under Fit and optimize when the next picture is loaded with the same settings. The panel compares settings only (`arrival` in `FitPanel`, `web/src/panels.tsx`); key it on the image too.
- **Dot styles say "pen down 0.0 mm".** A one-vertex path has no length. Say "dots" instead, or count a dot as one pen width of drawing.
- **Defaults that warn.** At its defaults Hatching has a specks warning on 17 of the 20 pictures and Ink on 19; by colour, Halftone, Waves and Spiral have it on 18. The lint's own patch (`order.min_length_mm` at the pen's width) clears it, so it could be the default for these styles, which the brief's goal of drawings that "plot cleanly" asks for.
- **The invert suggestion fires on art with a black ground.** The anime character on black and the stained-glass window (black leading) both get "This looks like light marks on a dark ground. Inverting it gives the marks the ink." Inverting either is wrong. The rule (`border < 0.3 && mean < 0.5 && two_level >= 0.85` in `analysis::suggest`) fits chalk on a board but also any illustration on black; leaving out colourful pictures (colourfulness over 40) would keep the first and drop both of these.
- **The grid's threshold drops whole colours in one-pen mode.** At the default 0.5, a yellow (ink about 0.2) or any light colour is left out entirely (the poster's yellow square vanishes). After F1, consider a lower default (0.35) or `auto` from Otsu's threshold, which the analysis already measures.

## Measurements

### Every style at its defaults (medians)

All twenty pictures, A4, 0.3 mm pens, every style at its defaults. "By colour" is the analysis's own palette (11 pictures); "Y+R+B" is Yellow, Red and Black by colour on all twenty. The per-picture JSON has every number.

`reach` is the coverage the fitted tone line gives at full ink (1 would be solid). `gain` is how much closer to the picture than blank paper (F2's colour comparison). `cell` is the median share of a Try-all cell with visible marks. `warn` counts pictures with a lint warning or error.

| Style | Reach, black | Gain, black | Tone error, black | Minutes, black | Reach, by colour | Gain, by colour | Gain, Y+R+B | Cell | Warn, black |
|---|---|---|---|---|---|---|---|---|---|
| grid | 0.07 | 5 % | 0.125 | 101 | 0.09 | 7 % | 7 % | 48 % | 1 |
| hatch | 0.77 | 50 % | 0.057 | 39 | 0.86 | 69 % | 68 % | 73 % | 17 |
| waves | 0.53 | 47 % | 0.031 | 24 | 0.77 | 57 % | 58 % | 85 % | 0 |
| spiral | 0.54 | 46 % | 0.031 | 24 | 0.80 | 58 % | 57 % | 85 % | 0 |
| ink | 1.22 | 39 % | 0.114 | 49 | 1.09 | 75 % | 60 % | 58 % | 19 |
| stipple | 0.08 | 6 % | 0.083 | 23 | 0.25 | 15 % | 12 % | 42 % | 0 |
| line | 0.13 | 12 % | 0.169 | 5 | 0.32 | 24 % | 22 % | 66 % | 0 |
| dashes | 0.09 | 8 % | 0.093 | 21 | 0.27 | 19 % | 16 % | 56 % | 0 |
| sketch | 0.42 | 21 % | 0.285 | 10 | 0.72 | 38 % | 31 % | 28 % | 1 |
| tree | 0.14 | 12 % | 0.218 | 10 | 0.31 | 23 % | 21 % | 62 % | 0 |
| mesh | 0.40 | 30 % | 0.131 | 21 | 0.74 | 58 % | 45 % | 82 % | 0 |
| cells | 0.23 | 20 % | 0.139 | 28 | 0.55 | 37 % | 30 % | 79 % | 0 |
| halftone | 0.78 | 58 % | 0.024 | 76 | 0.97 | 79 % | 76 % | 82 % | 4 |
| dither | 0.06 | 5 % | 0.060 | 101 | 0.10 | 7 % | 7 % | 64 % | 0 |
| circles | 0.16 | 10 % | 0.140 | 13 | 0.24 | 16 % | 14 % | 46 % | 0 |
| contours | 0.19 | 13 % | 0.666 | 12 | 0.37 | 23 % | 23 % | 37 % | 5 |
| flow | 0.22 | 20 % | 0.173 | 9 | 0.40 | 29 % | 27 % | 66 % | 0 |
| hilbert | 0.65 | 45 % | 0.092 | 21 | 0.74 | 64 % | 65 % | 78 % | 0 |
| scribble | 0.51 | 40 % | 0.094 | 17 | 0.89 | 63 % | 51 % | 80 % | 0 |
| mosaic | 0.50 | 44 % | 0.045 | 39 | 0.70 | 62 % | 62 % | 78 % | 0 |
| truchet | 0.39 | 30 % | 0.147 | 19 | 0.51 | 39 % | 38 % | 64 % | 0 |

### Palettes

11 of 20 pictures were colourful enough for a colour suggestion. With the five-pen library (Black, Red, Blue, Yellow, Green):

| Picture | Suggested inks | ΔE | From the library | Pens to add |
|---|---|---|---|---|
| anime-character | blue, black | 4.4 | Blue, Black | - |
| childrens-book | yellow, red, black | 2.2 | Yellow, Red, Black | - |
| flat-shapes-ryb | yellow, red, black | 1.0 | Yellow, Red, Black | - |
| flat-vector-fox | orange, red, black | 1.1 | Yellow, Red, Black | - |
| geometric-logo | orange, green, black | 2.4 | Yellow, Red, Black | with a blue pen the mean colour difference falls from 3.0 to 0.6. |
| neon-night | pink, purple, black | 13.4 | Red, Blue, Black | with a pink pen the mean colour difference falls from 21.2 to 14.5 and with a purple pen to 15.7. |
| pixel-art | yellow, blue, purple, black | 9.9 | Yellow, Red, Blue, Black | - |
| pop-art | yellow, red, blue, black | 8.4 | Yellow, Red, Blue, Black | - |
| stained-glass | yellow, red, black | 6.7 | Yellow, Red, Black | - |
| sunset-photo | yellow, red, purple, black | 1.6 | Yellow, Red, Blue, Black | - |
| synthetic-scene | yellow, green, blue, black | 3.1 | Yellow, Green, Blue, Black | - |

### Optimize, no budget, one black pen

| Style | Moved denser / sparser / stayed / no result | Median tone error change | Median plot time change |
|---|---|---|---|
| grid | 17 / 2 / 0 / 1 | -2 % | +97 % |
| hatch | 0 / 16 / 3 / 1 | -4 % | -50 % |
| waves | 0 / 13 / 7 / 0 | -15 % | -4 % |
| spiral | 7 / 4 / 9 / 0 | -3 % | +0 % |
| stipple | 18 / 1 / 1 / 0 | -4 % | +92 % |
| line | 20 / 0 / 0 / 0 | -47 % | +96 % |
| dashes | 20 / 0 / 0 / 0 | -12 % | +167 % |
| sketch | 19 / 0 / 0 / 1 | -62 % | +299 % |
| tree | 20 / 0 / 0 / 0 | -40 % | +180 % |
| mesh | 19 / 1 / 0 / 0 | -25 % | +141 % |
| cells | 20 / 0 / 0 / 0 | -23 % | +76 % |
| halftone | 1 / 0 / 19 / 0 | +0 % | +0 % |
| dither | 20 / 0 / 0 / 0 | -18 % | +96 % |
| circles | 20 / 0 / 0 / 0 | -62 % | +262 % |
| contours | 3 / 8 / 2 / 7 | -16 % | -53 % |
| flow | 15 / 3 / 2 / 0 | -7 % | +11 % |
| hilbert | 19 / 0 / 1 / 0 | -14 % | +97 % |
| scribble | 20 / 0 / 0 / 0 | -34 % | +95 % |
| mosaic | 20 / 0 / 0 / 0 | -29 % | +92 % |
| truchet | 0 / 0 / 20 / 0 | +0 % | +0 % |

### Looks against Balanced (medians of the ratio)

| Style | Look | Tone error ratio | Plot time ratio | Gain change |
|---|---|---|---|---|
| grid | quick | 1.04 | 0.32 | -4 points |
| grid | fine | 0.95 | 1.55 | +3 points |
| hatch | fine | 1.36 | 1.44 | +7 points |
| waves | faithful | 0.93 | 0.95 | -1 points |
| waves | fine | 1.62 | 1.24 | +1 points |
| spiral | fine | 1.03 | 1.09 | +0 points |
| stipple | fine | 0.97 | 2.28 | +9 points |
| line | faithful | 0.38 | 1.87 | +10 points |
| dashes | faithful | 0.88 | 2.28 | +10 points |
| sketch | faithful | 0.36 | 3.99 | +26 points |
| sketch | fine | 0.69 | 2.00 | +13 points |
| tree | faithful | 0.41 | 2.71 | +9 points |
| mesh | faithful | 0.71 | 1.08 | +2 points |
| mesh | fine | 0.75 | 2.42 | +13 points |
| cells | faithful | 0.68 | 1.51 | +5 points |
| cells | fine | 0.74 | 2.05 | +8 points |
| dither | faithful | 0.57 | 1.54 | +3 points |
| dither | quick | 1.46 | 0.32 | -4 points |
| circles | faithful | 0.38 | 3.58 | +26 points |
| flow | faithful | 0.60 | 3.00 | +39 points |
| hilbert | faithful | 0.60 | 1.41 | +15 points |
| scribble | faithful | 0.66 | 1.95 | +14 points |
| mosaic | faithful | 0.71 | 1.92 | +8 points |
| truchet | faithful | 0.89 | 1.34 | +11 points |

## How to run the audit again

The three tools are examples in the core crate, so they need nothing installed beyond Rust. Put the pictures in a folder outside the repository (PNG, JPEG or WebP; the two synthetic pictures are always added).

```bash
cargo run --release -p plotline-core --example audit -- <pictures-dir> <out-dir> [workers]
```

It writes `<out-dir>/<picture>.json` with every run, trial, Optimize search and look, and the previews and Try-all cells under `<out-dir>/png/<picture>/`. A picture whose JSON exists is skipped, so an interrupted run picks up where it stopped. `AUDIT_ONLY=fox,logo` limits it to pictures whose names contain those words. A picture takes 5 to 10 minutes on one core; most of it is the Voronoi styles' Optimize searches.

To check one cause, change one thing and compare:

```bash
cargo run --release -p plotline-core --example probe -- <picture> waves "Yellow:0.3:#f5c400,Red:0.3:#d0202e,Black:0.3:#000000" "{\"split\":{\"mode\":\"by_colour\"},\"tone\":{\"gamma\":1.0}}" out.png
```

```bash
cargo run --release -p plotline-core --example palette -- <picture> "Black:#000000,Red:#d0202e,Blue:#1f4fb4"
```

A fix for a finding here should come with the numbers before and after from the same pictures, the way F1's table does.
