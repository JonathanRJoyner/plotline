"""Tests for loading image files as float RGB arrays."""

import numpy as np
from PIL import Image
from synthetic import gradient

from plotline.session.images import load_image


def save(tmp_path, img, name="i.png", **kw):
    p = tmp_path / name
    img.save(p, **kw)
    return p


def test_load_png_shape_dtype_range(tmp_path):
    a = load_image(save(tmp_path, Image.fromarray((gradient(20, 10) * 255).astype(np.uint8))))
    assert a.shape == (10, 20, 3) and a.dtype == np.float32 and 0 <= a.min() and a.max() <= 1


def test_load_downsizes_long_edge(tmp_path):
    a = load_image(save(tmp_path, Image.new("RGB", (5000, 100))))
    assert a.shape == (80, 4000, 3)


def test_load_applies_exif_orientation(tmp_path):
    exif = Image.Exif()
    exif[0x0112] = 6
    a = load_image(save(tmp_path, Image.new("RGB", (20, 10)), "i.jpg", exif=exif.tobytes()))
    assert a.shape == (20, 10, 3)


def test_load_composites_alpha_on_white(tmp_path):
    a = load_image(save(tmp_path, Image.new("RGBA", (4, 4), (0, 0, 0, 0))))
    assert a.min() == 1.0


def test_load_composites_the_other_transparency_forms_on_white(tmp_path):
    palette = Image.new("P", (4, 4), 0)
    palette.putpalette([0, 0, 0, 255, 255, 255])
    forms = {
        "la": (Image.new("LA", (4, 4), (0, 0)), {}),
        "palette": (palette, {"transparency": 0}),
        "colour_key": (Image.new("RGB", (4, 4), (0, 0, 0)), {"transparency": (0, 0, 0)}),
    }
    for name, (img, kw) in forms.items():
        assert load_image(save(tmp_path, img, f"{name}.png", **kw)).min() == 1.0, name
