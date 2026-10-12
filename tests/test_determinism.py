"""Determinism: the same settings on the same input give identical paths through the bindings,
and the grid style keeps the path count of the engine's golden file."""

import numpy as np

from plotline import Canvas, Session, _core

# The path count of crates/core/golden/grid_gradient.json.
EXPECTED_COUNT = 544


def test_grid_is_deterministic_through_the_bindings():
    def run():
        s = Session()
        s.set_image(_core.synthetic_gradient(64, 48))
        s.update("tone", resolution=64)
        return s.run_blocking().layers[0].paths

    a, b = run(), run()
    assert len(a) == len(b) and all(np.array_equal(x, y) for x, y in zip(a, b, strict=True))


def test_grid_golden_counts():
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.set_canvas(Canvas(100, 75, 5))
    s.update("tone", resolution=64)
    s.update("points", threshold=0.6, spacing_mm=2.0)
    assert len(s.run_blocking().layers[0].paths) == EXPECTED_COUNT
