"""Drawing statistics and a plot time estimate.

``compute_stats`` counts the paths of a finished result, measures the distance the pen draws and
the distance it travels lifted, and turns them into seconds at the speeds of a ``Speeds``.
"""

from dataclasses import dataclass

from plotline.core.paths import total_length, travel_length
from plotline.core.types import Result


@dataclass
class Speeds:
    """How fast a plotter moves, as a front end lets the user set it.

    ``draw_mm_s`` is the speed with the pen down and ``travel_mm_s`` the speed with it up, both in
    mm per second and above zero. ``pen_lift_s`` is the time to lift the pen and set it down
    again, which every path costs once. The class is not frozen, so a front end can edit the
    values in place.
    """

    draw_mm_s: float = 50.0
    travel_mm_s: float = 100.0
    pen_lift_s: float = 0.2


@dataclass(frozen=True)
class Stats:
    """What a result costs to plot: the number of paths, the pen-down and pen-up distances in mm
    and the estimated plot time in seconds."""

    path_count: int
    pen_down_mm: float
    pen_up_mm: float
    estimated_seconds: float


def compute_stats(result: Result, speeds: Speeds) -> Stats:
    """The stats of ``result`` when plotted at ``speeds``.

    The paths of all layers are taken in order as one run, which the pen starts at the top left
    corner of the drawing rect, where the ordering stage starts. The move to the first path and
    the moves between layers count as travel. The time in seconds is the pen-down distance over
    ``draw_mm_s``, plus the pen-up distance over ``travel_mm_s``, plus ``pen_lift_s`` for each
    path. Swapping pens between layers is not timed.
    """
    paths = [path for layer in result.layers for path in layer.paths]
    x, y, _, _ = result.canvas.drawing_rect
    down = total_length(paths)
    up = travel_length(paths, (x, y))
    seconds = down / speeds.draw_mm_s + up / speeds.travel_mm_s + len(paths) * speeds.pen_lift_s
    return Stats(len(paths), down, up, seconds)
