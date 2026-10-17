# Dots at small sizes

**Decided:** 2026-10-16

**Choice:** Two changes, after the owner reported that in Try all styles, on a colour illustration with red, black and yellow pens, the Grid dots and Dither cells came out blank, and that these two styles often failed to show in the main preview.

- The style grid's pictures are still drawn 600 px wide, but every mark in them is at least 2 px wide (`preview::render_png_shrunk`, `THUMB_SHRINK` in `web/src/app.tsx`). The cells show the picture about a quarter as wide.
- The main preview fills a one-vertex path as a disc as wide as the pen. It used to stroke a 0.1 µm segment with a round cap.

A law in `crates/core/tests/laws.rs` runs every style on the colour scene with three coloured pens, in every Pen use mode. Each style must draw something, and its grid picture, shrunk to a 150 px cell, must have visible marks (pixels at least 10 % darker than the paper) on at least 3 % of the cell. A unit test pins the disc, and a browser flow checks that a dot style's marks show at the fit zoom.

**Reason:** The lanes were not starving these styles. On `synthetic::scene`, `synthetic::color_bars` and an illustration-like image, grid and dither drew thousands of dots in every lane, in every Pen use mode. The two causes were in the previews.

- **The style grid.** Grid and dither draw pen-wide dots 1 mm apart, about 7 % ink at most with a 0.3 mm pen. In a 600 px picture of A4 that pen is 0.86 px, so the one-pixel floor barely applies. Shrunk into a cell, the dots average to near white: on the scene, 0.0 % of the cell is visibly marked in one-pen mode and 1.4 % by colour. Every other style marks at least 4 %. Until the enlarged pictures of 2026-10-16 the pictures were 300 px wide, where the floor made the dots visible, so that change caused the regression. A 2 px floor at 600 px gives the same marks as before in the cell and stays sharp when enlarged. It costs nothing extra: grid then marks 28 to 33 % of its cell and dither 40 %, and the faintest style, contours, 7.5 %. Raising the floor makes every style look darker in its cell than it plots. That is the old pictures' trade-off, and the cell is for seeing the kind of mark. A floor of 4 px, one screen pixel on a display with no scaling, made the line styles look about twice as dark as they plot, and an enlarged dot 2.3 px wide.
- **The main preview.** In the desktop browser's canvas, a round-capped stroke of near-zero length drew nothing once the pen was under about a pixel wide. A dot style's page was then blank at the fit zoom (51 % on an A4-sized page), while lines still showed faintly. A filled disc is drawn with its coverage at any size. For 99,330 dots it took the same time as the strokes or less (361 against 400 ms at 4.8 px per mm). Headless Chromium, which the browser flows use, drew the old strokes, so only the unit test tells the two apart.

This amends the brief's rows "Preview renderer" and "Enlarged style pictures".
