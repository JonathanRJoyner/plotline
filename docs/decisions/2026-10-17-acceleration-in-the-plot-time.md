# Acceleration in the plot time

**Decided:** 2026-10-17

**Choice:** The plot time counts acceleration. `Speeds` gains `accel_mm_s2`, 1000 mm/s² by default, and a machine gains the same field, which `Machine::speeds` passes on. `stats::compute` times every straight move on a trapezoidal velocity profile (L. Biagiotti and C. Melchiorri, "Trajectory Planning for Automatic Machines and Robots", Springer, 2008, section 3.2): the pen speeds up at the acceleration, cruises, and slows down at it, so a move too short to reach its speed takes 2·sqrt(d/a) from rest to rest instead of d/v. Each travel move runs from rest to rest. Each path runs from rest to rest too, but does not stop at every vertex: the pen passes a vertex at the speed of S. Jeon's cornering rule ("Improving Grbl: Cornering Algorithm", 2011), the speed at which the circle that passes 0.01 mm from the vertex can be taken at the acceleration, then a backward and a forward pass over the path lower each vertex's speed to one the pen can slow down from and reach, as a look-ahead planner does. The 0.01 mm junction deviation is a constant, the default that description gives, and not a setting. The pen lift is timed as before. The lint's travel share is now travel seconds over the plot time, from the same timing, rather than the pen-up length over the travel speed.

The acceleration is exposed where the speeds are: the GUI's Settings dialog has an Acceleration field under Plot time (read-only while a machine is set, as the other speeds are) and one under Machine for the machine's own, both with help text; the CLI has `--accel MM_S2` on `run` and `testchart`, which overrides the machine's; the MCP `set_machine` and machine files and presets take `accel_mm_s2`; and Python's `Speeds` has `accel_mm_s2`. The G-code is unchanged: plotline does not write the acceleration into the files, which leave it to the firmware.

The preset version stays 3, as for the pen-lift fields ("New machine fields in presets" in the brief's table): a machine without `accel_mm_s2` reads it as 1000, and a machine at 1000 is written without it, so every existing preset loads and saves exactly as before. Speeds JSON without the field, such as the GUI's saved speeds from an older page, also reads it as 1000.

Before and after on the regression gallery's A4 page, 0.5 mm pen, at the default speeds (50 mm/s drawing, 100 mm/s travel, 0.2 s lift), from `tests/snapshots/gallery-stats.json`:

| Cell | Before | After | Ratio |
|---|---|---|---|
| `grid` on the still life | 21.8 min | 27.4 min | 1.25 |
| `grid` on the scene | 131.4 min | 164.7 min | 1.25 |
| `stipple` on the still life | 20.5 min | 32.2 min | 1.57 |
| `stipple` on the gradient | 21.0 min | 33.7 min | 1.60 |
| `waves` on the still life | 15.2 min | 33.1 min | 2.17 |
| `waves` on the scene | 26.5 min | 60.1 min | 2.27 |
| `hilbert` on the scene | 25.5 min | 71.2 min | 2.80 |
| `contours` on the gradient | 3.0 min | 3.0 min | 1.01 |

Every cell's time rose and nothing else moved: the path counts, lengths and tone errors are the same. Dots and short strokes gain the most per mark, and so do paths that turn sharply at many vertices: `waves` and `spiral` wiggle at every vertex where the image is dark, and `hilbert` turns 90° at each one. Long smooth strokes, as in `contours`, barely change. Because the time grew while the lift time did not, the lint's `lifts` finding left 19 cells of `cells`, `circles`, `halftone` and `stipple`, and its `travel` finding joined the four `circles` and `dither` cells on the still life, where the hops between marks now cost what they do on a machine.

**Reason:** Fit to minutes, Optimize's time limit and the lint all trust the plot time, and the old one, distance over speed, ran optimistic by up to a factor of three on the styles above, which a roadmap review on 2026-10-17 ranked as a top trust fix. A trapezoidal profile with look-ahead is how the usual firmware of stepper-driven machines plans its moves, so the estimate follows what such a machine does. Counting the cornering speed as well was needed for the result to hold on curved and turning paths: with each path as one move, `hilbert` and tight `waves` would have stayed near their old, too-short times, and with a stop at every vertex, a smooth curve drawn as many short segments would have taken many times its real time. 1000 mm/s² is a middle value for small plotters and converted printers: it reaches the default 50 mm/s in 1.25 mm, so it changes long strokes little and moves the estimate only where a machine's acceleration matters. Users who know their firmware's value set it. A law in `crates/core/tests/laws.rs`, `acceleration_never_shortens_the_plot_time`, checks for any style's drawing, speeds and acceleration that the new time is never below the old one, and that it comes to the old one as the acceleration grows without limit.

Left for later: the looks were measured with the old time, so the Quick looks of `grid` and `dither` ("plot in about half an hour on A4 at the default speeds") now take longer than that. Re-measuring them with `cargo run --release -p plotline-core --example looks` is a follow-up, held until the sessions editing the grid and dither renderers land.
