"""Tests for the drawing stats: numbers worked out by hand, an empty result and the session."""

import math

import numpy as np
import pytest
from synthetic import gradient

from plotline.core.types import Canvas, Layer, Pen, Result
from plotline.session.session import Session
from plotline.session.stats import Speeds, compute_stats


def test_compute_stats_by_hand():
    paths = [np.array([[0.0, 0.0], [10.0, 0.0]]), np.array([[20.0, 0.0], [20.0, 5.0]])]
    r = Result(Canvas(100, 100, 10), [Layer(Pen("p", 0.3, "#000000"), paths)])
    st = compute_stats(r, Speeds())
    assert st.path_count == 2 and st.pen_down_mm == pytest.approx(15)
    assert st.pen_up_mm == pytest.approx(math.hypot(10, 10) + 10)
    assert st.estimated_seconds == pytest.approx(15 / 50 + (math.hypot(10, 10) + 10) / 100 + 0.4)


def test_empty_result_has_zero_stats():
    st = compute_stats(Result(Canvas(100, 100), [Layer(Pen("p", 0.3, "#000000"), [])]), Speeds())
    assert (st.path_count, st.pen_down_mm, st.pen_up_mm, st.estimated_seconds) == (0, 0, 0, 0)


def test_session_stats():
    s = Session()
    s.set_image(gradient(64, 48))
    s.update("tone", resolution=64)
    s.run_blocking()
    assert s.stats().path_count == len(s.result.layers[0].paths) > 0
    with pytest.raises(RuntimeError):
        Session().stats()
