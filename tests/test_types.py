"""Tests for the core data types: pen, canvas, tone map, layer and result."""

import numpy as np
import pytest

from plotline.core.types import Canvas, Pen, ToneMap


def test_drawing_rect_and_fit():
    c = Canvas(210, 297, 10)
    assert c.drawing_rect == (10, 10, 190, 277)
    assert c.image_rect(1000, 500) == pytest.approx((10, 101, 190, 95))
    assert c.image_transform(1000, 500) == (pytest.approx(0.19), pytest.approx((10, 101)))


def test_fill_covers_drawing_rect():
    c = Canvas(100, 50, 0, "fill")
    assert c.image_rect(64, 64) == (0, 0, 100, 50)
    assert c.image_transform(64, 32) == (pytest.approx(1.5625), pytest.approx((0, 0)))


def test_canvas_rejects_bad_dimensions():
    for args in [(0, 10), (100, -1), (100, 100, 50), (100, 100, -1)]:
        with pytest.raises(ValueError):
            Canvas(*args)


def test_pen_validation():
    with pytest.raises(ValueError):
        Pen("x", 0.3, "000000")
    with pytest.raises(ValueError):
        Pen("x", 0, "#000000")


def test_tone_map_geometry_and_sampling():
    t = ToneMap(np.array([[0, 1], [0, 1]], np.float32), 10.0, (5.0, 5.0))
    assert t.rect_mm == (5, 5, 20, 20)
    assert t.to_px(np.array([[10.0, 10.0]])) == pytest.approx(np.array([[0.5, 0.5]]))
    p = np.array([[12.0, 17.0]])
    assert t.to_mm(t.to_px(p)) == pytest.approx(p)
    s = t.sample(np.array([[10, 10], [20, 10], [15, 10], [100, 100], [-50, -50]], float))
    assert s.dtype == np.float32
    assert s == pytest.approx([0, 1, 0.5, 1, 0])
