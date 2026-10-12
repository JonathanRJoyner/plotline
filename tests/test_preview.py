"""Tests for the preview PNG: size and format, true pen width, the overlays and the session."""

import io

import numpy as np
import pytest
from PIL import Image
from synthetic import gradient

from plotline.core.types import Canvas, Layer, Pen, Result, ToneMap
from plotline.session.preview import render_png
from plotline.session.session import Session

PEN = Pen("p", 2.0, "#000000")


def png(result, **kw):
    return Image.open(io.BytesIO(render_png(result, **kw))).convert("RGB")


def test_size_and_format():
    im = Image.open(
        io.BytesIO(render_png(Result(Canvas(210, 297, 10), [Layer(PEN, [])]), width_px=200))
    )
    assert im.format == "PNG" and im.size == (200, 283)


def test_dot_is_drawn_at_true_width():
    im = png(Result(Canvas(100, 100), [Layer(PEN, [np.array([[50.0, 50.0]])])]), width_px=200)
    assert im.getpixel((100, 100))[0] < 128 and im.getpixel((110, 100))[0] > 200


def test_line_is_drawn_at_true_width_with_round_caps():
    # A 2 mm pen at 2 px per mm: a stroke 4 px thick and 120 px long, with a round cap at each end.
    line = np.array([[20.0, 50.0], [80.0, 50.0]])
    im = png(Result(Canvas(100, 100), [Layer(PEN, [line])]), width_px=200)
    dark = (255 - np.asarray(im.crop((10, 10, 190, 190)).convert("L"), float)) / 255  # no outline
    assert dark[:, 90].sum() == pytest.approx(4, abs=0.05)  # the column at x = 100
    assert dark.sum() == pytest.approx(120 * 4 + np.pi * 2**2, abs=1)  # the stroke and the caps


def test_a_dot_survives_a_pen_thinner_than_a_subpixel():
    thin = Pen("t", 0.1, "#000000")
    im = png(Result(Canvas(100, 100), [Layer(thin, [np.array([[50.0, 50.0]])])]), width_px=100)
    assert im.getpixel((50, 50))[0] < 255


def test_travel_overlay_is_red_only_when_asked():
    paths = [np.array([[10.0, 50.0], [20.0, 50.0]]), np.array([[80.0, 50.0], [90.0, 50.0]])]
    r = Result(Canvas(100, 100), [Layer(PEN, paths)])

    def reddish(im):
        a = np.asarray(im, int)
        return bool((a[..., 0] > a[..., 1] + 50).any())

    assert reddish(png(r, travel=True, width_px=200)) and not reddish(png(r, width_px=200))


def test_source_overlay_darkens_the_image_area():
    tone = ToneMap(np.ones((10, 10), np.float32), 5.0, (25.0, 25.0))
    r = Result(Canvas(100, 100), [Layer(PEN, [])], tone=tone)
    assert png(r, source=True).getpixel((500, 500))[0] < png(r).getpixel((500, 500))[0]


def test_empty_result_renders():
    assert png(Result(Canvas(100, 100), [Layer(PEN, [])])).size == (1000, 1000)


def test_session_preview():
    s = Session()
    s.set_image(gradient(64, 48))
    s.update("tone", resolution=64)
    s.run_blocking()
    assert s.preview_png(width_px=100)[:8] == b"\x89PNG\r\n\x1a\n"
    with pytest.raises(RuntimeError):
        Session().preview_png()
