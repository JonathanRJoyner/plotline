"""The grid placer and the dot renderer: the simplest point stage and the simplest renderer.

The placer lays a regular grid over the tone map and keeps the cell centres where the ink density
reaches a threshold, so dark areas get points and light areas stay empty. The renderer turns each
point into a path of one vertex, which is how a dot is stored. With the tone and ordering stages
they make up the first built-in style.
"""

import math
from collections.abc import Generator, Mapping
from typing import Any

import numpy as np
from numpy.typing import NDArray

from plotline.core.settings import Settings, setting
from plotline.core.stage import Frame, RunContext, Stage
from plotline.core.types import ToneMap


class GridSettings(Settings):
    """Settings for the grid placer."""

    spacing_mm: float = setting(
        2.0,
        min=0.2,
        max=20,
        step=0.1,
        unit="mm",
        help="Distance between neighbouring grid points, along the rows and along the columns.",
    )
    threshold: float = setting(
        0.5,
        min=0,
        max=1,
        step=0.01,
        help="Least ink density that gets a point: 0 keeps every grid position and 1 keeps only "
        "full ink.",
    )


class GridPlacer(Stage):
    """Place a point at the centre of every grid cell whose ink density is at least the threshold.

    The grid covers the tone map's rectangle. Cell centres sit at ``x0 + spacing * (i + 0.5)``
    across and likewise down, for as many whole cells as fit. Points come out row by row, left to
    right, as an ``(N, 2)`` float64 array in mm, with shape ``(0, 2)`` when nothing is dark enough.
    """

    name = "points"
    label = "Grid"
    requires = ("tone",)
    provides = "points"
    settings_model = GridSettings

    def run(
        self, inputs: Mapping[str, Any], settings: Settings, ctx: RunContext
    ) -> Generator[Frame, None, NDArray[np.float64]]:
        assert isinstance(settings, GridSettings)
        tone: ToneMap = inputs["tone"]
        x0, y0, w, h = tone.rect_mm
        step = settings.spacing_mm
        # The tolerance keeps the last cell when w / step is a whole number that rounds just below.
        cols = math.floor(w / step + 1e-9)
        rows = math.floor(h / step + 1e-9)
        if rows == 0 or cols == 0:
            empty = np.empty((0, 2))
            yield Frame(1.0, "no grid cells", empty)
            return empty
        xs = x0 + step * (np.arange(cols) + 0.5)
        # Each row appends to the buffer, so a frame is a prefix of it that later rows never touch.
        buf = np.empty((rows * cols, 2))
        n = 0
        for j in range(rows):
            centres = np.column_stack((xs, np.full(cols, y0 + step * (j + 0.5))))
            kept = centres[tone.sample(centres) >= settings.threshold]
            buf[n : n + len(kept)] = kept
            n += len(kept)
            yield Frame((j + 1) / rows, f"row {j + 1}/{rows}", buf[:n])
        # A copy, so the result does not hold the whole buffer and is not a view the frames share.
        return buf[:n].copy()


class DotSettings(Settings):
    """Settings for the dot renderer, which takes none."""


class DotRenderer(Stage):
    """Turn each point into a path of one vertex, which is a dot."""

    name = "render"
    label = "Dots"
    requires = ("points",)
    provides = "paths"
    settings_model = DotSettings

    def run(
        self, inputs: Mapping[str, Any], settings: Settings, ctx: RunContext
    ) -> Generator[Frame, None, list[NDArray[np.float64]]]:
        assert isinstance(settings, DotSettings)
        points: NDArray[np.float64] = inputs["points"]
        # Views of the points, which are shared with the cache: nothing here writes to them.
        paths = [p.reshape(1, 2) for p in points]
        yield Frame(1.0, f"{len(paths)} dots", paths=paths)
        return paths
