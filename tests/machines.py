"""A machine for tests, in the shape of a preset's ``machine``: a 200 by 174 mm drawable area, pen
heights of 4, 9 and 20 mm and a few lines of G-code around the plot.

It implies the speeds 60 mm/s drawing, 100 mm/s travelling and a pen lift of 0.6 s: two Z moves of
5 mm at 20 mm/s (0.5 s) and two 50 ms dwells (0.1 s).
"""

MACHINE = {
    "name": "Test printer",
    "area": {"x": 28, "y": 42, "width": 200, "height": 174},
    "z_down_mm": 4,
    "z_up_mm": 9,
    "z_safe_mm": 20,
    "draw_feed": 3600,
    "travel_feed": 6000,
    "z_feed": 1200,
    "settle_ms": 50,
    "start_gcode": "M104 S0\nM140 S0",
    "end_gcode": "G1 Z20 F600\nG1 Y250 F3600",
}
