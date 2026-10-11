"""Integration tests: the grid style run end to end through the runner."""

import numpy as np
from doubles import collect
from synthetic import flat, gradient

from plotline.core.cache import StageCache
from plotline.core.pipeline import run_style
from plotline.core.stage import CancelToken, RunContext
from plotline.core.stages.tone import ToneSettings
from plotline.core.styles import GRID
from plotline.core.types import Canvas, Pen

ROOTS = {"canvas": Canvas(100, 75, 5), "pens": (Pen("Black 0.3", 0.3, "#000000"),)}


def run(image, **tone):
    s = {**GRID.defaults(), "tone": ToneSettings(resolution=64, **tone)}
    return collect(
        run_style(GRID, s, {**ROOTS, "image": image}, StageCache(), RunContext(CancelToken()))
    )


def test_grid_on_gradient():
    frames, result = run(gradient(64, 48))
    paths = result.layers[0].paths
    assert len(paths) > 100 and all(p.shape == (1, 2) for p in paths)
    x, y, w, h = ROOTS["canvas"].drawing_rect
    pts = np.concatenate(paths)
    assert pts[:, 0].min() >= x and pts[:, 0].max() <= x + w
    assert pts[:, 1].min() >= y and pts[:, 1].max() <= y + h
    assert {f.stage for f in frames} == {"tone", "points", "render", "order"}
    assert result.tone is not None


def test_grid_on_white_image_has_no_paths():
    _, result = run(flat(64, 48, 1.0))
    assert result.layers[0].paths == [] and result.layers[0].pen == ROOTS["pens"][0]
