"""The ordering stage: drop short paths, then reorder and reverse the rest to cut pen-up travel.

The order comes from the nearest-neighbour heuristic for the travelling salesman problem, which
Rosenkrantz, Stearns and Lewis analyse in "An analysis of several heuristics for the traveling
salesman problem" (SIAM Journal on Computing 6(3), 1977). Here it runs on paths, which have two
endpoints: the pen starts at the top-left corner of the drawing area and repeatedly moves to the
closest endpoint of a path not yet drawn, then draws that path to its other end. A k-d tree over
the endpoints finds the closest one without comparing against every path.
"""

from collections.abc import Generator, Mapping
from typing import Any

import numpy as np
from numpy.typing import NDArray
from scipy.spatial import cKDTree

from plotline.core.paths import path_length
from plotline.core.settings import Settings, setting
from plotline.core.stage import Frame, RunContext, Stage
from plotline.core.types import Canvas


class OrderSettings(Settings):
    """Settings for the ordering stage."""

    optimize: bool = setting(True, help="Reorder paths to cut pen-up travel.")
    min_length_mm: float = setting(
        0.0,
        min=0,
        max=10,
        step=0.1,
        unit="mm",
        help="Drop paths shorter than this. Dots have zero length, so any positive value "
        "removes them.",
    )


class OrderStage(Stage):
    """Drop the paths shorter than ``min_length_mm``, then, with ``optimize`` on, put the rest in
    nearest-neighbour order from the top-left corner of the drawing area.

    Each path is entered at its nearer endpoint, so one whose last vertex is the nearer is
    reversed, as a view. The result is a new list and no input array is written to. Without
    ``optimize`` the paths that remain keep their order and direction.

    About every twentieth of the paths a frame holds the paths ordered so far, and the last frame,
    at progress 1, holds all of them.
    """

    name = "order"
    label = "Path order"
    requires = ("paths", "canvas")
    provides = "paths"
    settings_model = OrderSettings

    def run(
        self, inputs: Mapping[str, Any], settings: Settings, ctx: RunContext
    ) -> Generator[Frame, None, list[NDArray[np.float64]]]:
        assert isinstance(settings, OrderSettings)
        canvas: Canvas = inputs["canvas"]
        kept: list[NDArray[np.float64]] = list(inputs["paths"])
        if settings.min_length_mm > 0:  # at 0 no path is shorter, so skip measuring every one
            kept = [p for p in kept if path_length(p) >= settings.min_length_mm]
        n = len(kept)
        if n == 0 or not settings.optimize:
            yield Frame(1.0, f"{n}/{n} paths", paths=kept)
            return kept
        # All starts, then all ends: index j is an endpoint of path j % n, its last when j >= n.
        tree = cKDTree(np.array([p[0] for p in kept] + [p[-1] for p in kept]))
        drawn = np.zeros(n, dtype=bool)
        pen: NDArray[np.float64] = np.array(canvas.drawing_rect[:2])
        ordered: list[NDArray[np.float64]] = []
        every = max(1, n // 20)
        for count in range(1, n + 1):
            # List the k endpoints nearest the pen and double k until one is on an undrawn path,
            # which happens at the latest when k reaches 2n and lists every endpoint.
            k = 8
            # shortcut: k-doubling goes quadratic when many endpoints coincide, and the greedy
            # loop costs ~100 us per path in Python; revisit with the stipple renderer in spec 2
            # (dedupe coincident endpoints or move the loop to a numba kernel).
            while True:
                near = tree.query(pen, min(k, 2 * n))[1]
                free = near[~drawn[near % n]]
                if free.size:
                    break
                k *= 2
            j = int(free[0])
            drawn[j % n] = True
            path = kept[j % n]
            ordered.append(path[::-1] if j >= n else path)
            pen = ordered[-1][-1]
            if count < n and count % every == 0:
                # A copy, so the frame does not change as the list grows.
                yield Frame(count / n, f"{count}/{n} paths", paths=ordered[:])
        yield Frame(1.0, f"{n}/{n} paths", paths=ordered)
        return ordered
