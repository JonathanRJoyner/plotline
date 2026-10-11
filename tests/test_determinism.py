"""Determinism: the same settings on the same input give identical paths, for every style."""

import numpy as np
import pytest
from doubles import collect
from synthetic import gradient

from plotline.core.cache import StageCache
from plotline.core.pipeline import run_style
from plotline.core.registry import list_styles
from plotline.core.stage import CancelToken, RunContext
from plotline.core.stages.tone import ToneSettings
from plotline.core.types import Canvas, Pen


@pytest.mark.parametrize("style", list_styles(), ids=lambda s: s.name)
def test_same_settings_give_identical_output(style):
    settings = style.defaults()
    if "tone" in settings:
        settings["tone"] = ToneSettings(resolution=64)
    roots = {
        "image": gradient(64, 48),
        "canvas": Canvas(100, 75, 5),
        "pens": (Pen("p", 0.3, "#000000"),),
    }
    outs = [
        collect(run_style(style, settings, roots, StageCache(), RunContext(CancelToken())))[1]
        for _ in range(2)
    ]
    a, b = (o.layers[0].paths for o in outs)
    assert len(a) == len(b) and all(np.array_equal(p, q) for p, q in zip(a, b, strict=True))
