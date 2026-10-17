# Contours end at the image edge

**Decided:** 2026-10-17

**Choice:** A contour level that reaches the edge of the tone map is cut open there. The stage still pads the map by one empty pixel so every level closes for the tracer. It then drops each line's runs through the padding and keeps the stretches inside the map as open paths. Each stretch ends on the edge, at the padding vertex next to it. A line that never touches the padding stays one closed path, as before. `contours` moves to stage version 3. This amends the brief's "Contour levels" row.

**Reason:** The padding closed every level that was inked at the border by running it along the edge. All of those levels then ran within half a pixel of one another, so a thin pen drew the same line many times over. The plot lint's `overdraw` finding fired on four of the six `contours` cells of the regression gallery, and a 0.2 mm pen made it worse. The lines along the edge followed the padding, not the image, so dropping them loses nothing the picture holds.

Gallery, `contours` cells, before and after:

| Cell | Pen-down mm | Plot time s | Lint |
|---|---|---|---|
| gradient | 8,921 → 3,323 | 181 → 71 | overdraw, dark → dark |
| scene | 10,135 → 3,209 | 213 → 73 | overdraw, dark → dark |
| still life | 6,991 → 6,205 | 152 → 137 | dark → dark |

No other style's cells moved. The tone error of the gradient and scene cells becomes empty: what remains of their ink follows tone too little for the tone match to fit a line, which was already true of one scene cell. The still life's error falls from 0.60 to 0.44.
