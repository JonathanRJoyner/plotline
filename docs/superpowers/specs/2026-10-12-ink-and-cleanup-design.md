# plotline spec 6: ink style, cleaner hatching and a tidier GUI

Date: 2026-10-12. Status: built from the owner's request to test the tool on many images, find the problem areas, cater to ordinary pen plotters as well as the owner's pen-fitted 3D printer, and add the outline-and-fill method that gave the owner's best plots so far; open to review.

## 1. What the test pass found

Every built-in style ran at its defaults on 34 images (public-domain photographs, engravings, paintings and woodcuts from Wikimedia Commons, and the owner's own line art and scans, none of them committed) on A4 with a 0.3 mm pen, and the GUI was driven in a browser at 1440 by 900.

- **Inked line art has no faithful style.** Comics, logos and pen drawings come out as threshold hatching broken into stubs (`hatch`), or smeared into ripples (`waves`, `spiral`). The owner's best plots came from outlining every ink shape and filling it with nested rings, with a centreline for strokes thinner than the pen; plotline has nothing like it.
- **`hatch` and `grid` are binary at their defaults.** One level at threshold 0.5 turns a photograph into a silhouette.
- **`hatch` shatters on fine detail.** A run ends at the first light sample, so engravings and noisy scans give 12,000 to 44,000 stubs, with as much pen-up travel as pen-down (one Doré engraving: 25 m down, 18 m up, 79 minutes).
- **`waves` and `spiral` give the best tone on photographs** and need no change here.
- **WebP is refused.** Two of the owner's eight images were WebP files, the commonest format for images saved from the web.
- **The GUI buries the style's own settings.** Every control shows its help text, so the tone stage alone fills the left column and the style's settings sit below the fold. The right column opens with the 3D-printer machine form, which an ordinary plotter user, who exports SVG, never needs. The pen bank's select overflows its column, export sits at the bottom of the right column, and the zoom has no readout.

## 2. Ink style

`ink` "Ink" runs tone, ink, order. It draws a drawing's own marks rather than a tone: every ink shape is outlined and filled with rings, and strokes too thin to hold a ring are drawn once down their centre.

**Working raster.** The tone map is resampled bilinearly by an integer factor `k` from 1 to 4 so that the pen spans at least six working pixels, as long as the raster stays under 16 million pixels. Bilinear upsampling before thresholding gives sub-pixel edges, which keeps rings smooth on a coarse tone map. A pixel is ink where the resampled ink reaches `threshold`. Ink shapes smaller than `min_area_mm2` (8-connected components) are dropped.

**Rings.** The Euclidean distance transform of the mask (Felzenszwalb and Huttenlocher, "Distance Transforms of Sampled Functions", Theory of Computing 8, 2012) gives each ink pixel its distance to the nearest background pixel. Ring `j` is the iso-line of that distance at an inset of `pen (0.5 - overhang) + j * spacing * pen`, traced with marching squares (Lorensen and Cline, "Marching Cubes", SIGGRAPH 1987, in its two-dimensional form) with linear interpolation along cell edges. `overhang` is fixed at a quarter pen: the outer ring's ink reaches a quarter pen past the shape's edge, which matches the source's weight once ink spreads into paper. `spacing` (share of the pen width, default 0.8) leaves a fifth of a pen of overlap between rings, so a filled shape reads solid. Mode `outline` keeps ring 0 alone.

**Centrelines.** Ink further than 0.45 pen from every ring is missed ink: whole strokes thinner than half a pen, and the slivers along the middle of a shape whose depth falls between two rings. The missed pixels are thinned to a one-pixel skeleton (Zhang and Suen, "A Fast Parallel Algorithm for Thinning Digital Patterns", CACM 27(3), 1984), the skeleton is walked into polylines between its end and junction pixels, closed loops included, and branches shorter than a pen width that end in a free end are dropped. Mode `centreline` skips the rings and traces the skeleton of the whole mask, which draws clean line art in one pass per stroke.

**Smoothing and simplification.** Every polyline is smoothed with a Gaussian of sigma one working pixel along its vertices (end points fixed, loops wrapped), mapped to page mm, and simplified with Ramer–Douglas–Peucker (Douglas and Peucker, "Algorithms for the reduction of the number of points required to represent a digitized line or its caricature", The Canadian Cartographer 10(2), 1973) at a twentieth of the pen width.

**Settings.** `mode` fill, outline or centreline, default fill; `threshold` 0.05 to 0.95, default 0.5; `spacing` 0.3 to 1.5 of the pen width, default 0.8; `min_area_mm2` 0 to 4, default 0.02. The pen width comes from the first pen.

**Frames.** One frame after the mask, one per ring level with the paths so far, one after the centrelines.

**Style defaults.** A style may override the defaults of its stages. `ink` sets `tone.resolution` to 2500, because a 1000-pixel tone map loses the hairlines that ink drawings are made of.

## 3. Hatching that holds together

A hatch run now survives a light gap shorter than the line spacing: a gap that narrow cannot be seen between neighbouring lines, and bridging it removes most stubs and most of the pen lifts. `levels` defaults to 3 and `threshold` to 0.25, so a photograph keeps three tones of cross-hatching rather than a silhouette.

## 4. WebP input

The image crate's `webp` feature (the `image-webp` decoder, MIT or Apache-2.0) is enabled, and the GUI's file picker accepts WebP.

## 5. GUI

- The left column shows the style's description under its select. The style's own stages come first, then the shared ones (path order, then tone under the name "Image"), with the shared ones collapsed.
- Help text moves into tooltips, with a "Show help" switch at the top of the column that the page remembers.
- A slider row puts its label and number on one line and the slider under them.
- The toolbar gains the zoom readout and the SVG export, so the commonest end of a session is one click.
- The right column runs page, pens, export and plot time, then presets, with the machine form last and collapsed as "G-code (optional)". (Spec 7 later moved plot time and the machine form into a settings dialog.) A page has an orientation select in place of the swap button.
- Pens are one compact row each, and the bank's select fits its column.

## 6. Not built here, for the owner

- **Servo pen lifts in G-code.** Many hobby plotters lift the pen with a servo through spindle commands rather than a Z move. A machine field for pen-up and pen-down command text would cover them; the strict checker in the G-code tests would need to learn it.
- **Layer attributes for drawing programs.** Plotter software often reads layers from a vendor namespace in the SVG. That is open question 1 of the brief.
- **Auto levels.** Scans on grey paper put ink under every line of `waves` and `spiral`; `tone.white` fixes it by hand today.
- **Tone-modulated hatch spacing** and the planned Voronoi family remain the next tonal styles.

## 7. Tests

- `ink.rs`: schema bounds and defaults; a filled disc gives nested closed rings whose outer ring sits a quarter pen inside the edge and whose count matches the disc's radius; a stroke thinner than half a pen gives one centreline and no ring; `outline` gives one ring per shape; `centreline` gives no closed ring on a straight bar; specks below `min_area_mm2` vanish; every vertex inside the tone rect; deterministic.
- The distance transform matches a brute-force one on a random mask; marching squares closes the contour of a square; thinning a 5-pixel bar gives a 1-pixel line; Ramer–Douglas–Peucker keeps the corners of an L.
- `hatch.rs`: a gap shorter than the spacing is bridged and a longer one is not.
- `registry.rs`: the list holds five styles; `style.rs`: an override changes the style's defaults and a preset's missing key takes the override.
- WebP: a tiny encoded image decodes.
