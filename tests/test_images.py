"""Tests for loading image files as float RGB arrays."""

import numpy as np
import pytest
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


@pytest.mark.parametrize(
    ("dtype", "name"),
    [("<u2", "i.png"), (">u2", "i.tif"), ("<i4", "i.tif")],
    ids=["I;16", "I;16B", "I"],
)
def test_load_scales_16_bit_greyscale_to_full_range(tmp_path, dtype, name):
    ramp = np.linspace(0, 65535, 16).astype(dtype)
    a = load_image(save(tmp_path, Image.fromarray(np.tile(ramp, (4, 1))), name))
    row = a[0, :, 0]
    assert a.shape == (4, 16, 3) and a.dtype == np.float32 and (a == a[..., :1]).all()
    assert np.all(np.diff(row) >= 0) and row[-1] == 1.0 and abs(row[8] - 0.5) < 0.04


def test_load_shrunken_greyscale_stays_in_range_and_saturates_overrange_values(tmp_path):
    def step(low, high):
        edge = np.full((8, 5000), low, np.int32)
        edge[:, 2500:] = high
        return load_image(save(tmp_path, Image.fromarray(edge), f"{low}.tif"))

    exact, beyond = step(0, 65535), step(-100000, 100000)
    assert exact.shape == (6, 4000, 3) and 0.0 <= exact.min() and exact.max() <= 1.0
    assert np.array_equal(exact, beyond)


def test_load_refuses_float_images(tmp_path):
    with pytest.raises(
        ValueError, match="unsupported image mode 'F': float images have no defined white level"
    ):
        load_image(save(tmp_path, Image.fromarray(np.zeros((4, 4), np.float32)), "f.tif"))
