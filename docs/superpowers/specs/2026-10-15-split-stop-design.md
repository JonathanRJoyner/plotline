# plotline spec: Stop during Pen use by colour

Date: 2026-10-15. Status: decided by the builder under the owner's standing delegation; written for review.

The Pen use stage (`split`) does all its work in its first step. In `by_colour` mode that step samples the image's colour on the tone map's grid, solves the colour table and then looks up every pixel for every pen. At a 4000-pixel tone map with eight pens the pixel loop alone takes about 0.9 s, and the runner checks Stop only between steps, so Stop waits for it. The wide blur (brief §14, "Wide blur") and the contour levels ("Contour levels") were split up for the same reason.

## 1. Steps

In `by_colour` mode the run becomes a short state machine:

1. **Prepare.** Sample the red, green and blue planes on the tone map's grid, one plane per step, then solve the table in a fourth step, and report a frame labelled `pen use: colour` after each. Done in one step, the preparation took 180 to 290 ms at a 4000-pixel map with eight pens; a plane takes 60 to 70 ms and the table 40 to 65 ms.
2. **Rows.** Each step looks up a band of whole rows, at most `PIXELS_PER_STEP` = 200,000 pixels (at least one row), the bound the Voronoi relaxation uses, and reports a frame labelled `pen use: <rows done>/<height> rows` whose progress is the share of rows done.
3. **Done.** The next step gives the maps.

`one_pen` and `light_to_dark` keep their one frame, labelled `pen use`, then the maps.

## 2. What stays

Each pixel's lookup is the same code on the same values in the same order, so the maps are bit-identical. The stage's version and cache keys do not change, and no golden file or gallery cell moves.

## 3. Checks

- The existing `by_colour` tests pass unchanged.
- A run over a map with several bands gives more than one frame, with rising progress ending at 1, and maps equal bit for bit to a run done in one band.
- An ignored timing test, `by_colour_step_timings`, reports the longest step at a 4000-pixel map with eight pens in a release build; it is recorded in the brief.
