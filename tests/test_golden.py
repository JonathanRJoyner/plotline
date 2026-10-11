"""Golden tests: the grid style on tiny synthetic images keeps giving the same paths.

Coordinates are compared to 1e-3 mm, but the path count and every top-level key except ``layers``
must match exactly, so a rounding difference that flips one grid cell shows up as a count
mismatch. If a platform ever does that, move ``threshold`` in ``produce`` farther from every
sampled ink value instead of loosening the comparison. After a deliberate change to the geometry,
rewrite the files with ``pytest tests/test_golden.py --update-golden`` and review the diff.
"""

import json
import pathlib

import numpy as np
import pytest
from doubles import collect
from synthetic import checkerboard, gradient

from plotline.core.cache import StageCache
from plotline.core.export.paths_json import to_json_dict
from plotline.core.pipeline import run_style
from plotline.core.stage import CancelToken, RunContext
from plotline.core.stages.grid import GridSettings
from plotline.core.stages.tone import ToneSettings
from plotline.core.styles import GRID
from plotline.core.types import Canvas, Pen

CASES = {"grid_gradient": gradient(64, 48), "grid_checkerboard": checkerboard(64, 48, 8)}
GOLDEN = pathlib.Path(__file__).parent / "golden"


def produce(image):
    settings = {
        **GRID.defaults(),
        "tone": ToneSettings(resolution=64),
        "points": GridSettings(threshold=0.6),
    }
    roots = {
        "image": image,
        "canvas": Canvas(100, 75, 5),
        "pens": (Pen("Black 0.3", 0.3, "#000000"),),
    }
    _, result = collect(run_style(GRID, settings, roots, StageCache(), RunContext(CancelToken())))
    return to_json_dict(result)


@pytest.mark.parametrize("name", CASES)
def test_golden(name, update_golden):
    actual = produce(CASES[name])
    path = GOLDEN / f"{name}.json"
    if update_golden:
        GOLDEN.mkdir(exist_ok=True)
        path.write_text(json.dumps(actual, indent=0) + "\n", encoding="utf-8", newline="\n")
        return
    expected = json.loads(path.read_text(encoding="utf-8"))
    assert {k: v for k, v in actual.items() if k != "layers"} == {
        k: v for k, v in expected.items() if k != "layers"
    }
    a, e = actual["layers"][0]["paths"], expected["layers"][0]["paths"]
    assert len(a) == len(e) and len(a) > 0
    assert np.allclose(
        np.concatenate([np.array(p) for p in a]),
        np.concatenate([np.array(p) for p in e]),
        atol=1e-3,
    )
