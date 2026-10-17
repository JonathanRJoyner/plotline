Section: Output

- **Tone match at 45 degrees:** the tone error and plot lint sample each raster cell at a point of its own instead of its centre. Hatching at 0, 45 or 90 degrees no longer jumps in tone error between neighbouring spacings that line up with the 0.1 mm raster, so Optimize compares those spacings on their tones. Tone errors move by about 3 %, and dots on a regular grid now count their full area towards `dark`.
