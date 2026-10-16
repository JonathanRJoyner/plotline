Section: Output

- **Path simplification:** Path order has a new setting, `simplify_mm`, that removes the vertices lying within that distance of the simplified line (Ramer–Douglas–Peucker), keeping each path's end points. It is off at 0, so existing drawings and presets are unchanged; at an eighth of a 0.3 mm pen, 0.0375 mm, waves, spiral and flow lose 70 to 90 % of their vertices with the same drawing to within antialiasing. The plot lint's fine-vertices finding now offers it as a fix.
