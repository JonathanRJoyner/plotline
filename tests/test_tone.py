"""Tests for the tone stage: shape and transform, tone adjustments, blur, fill crop and resizing."""

import numpy as np
import pytest
from doubles import collect
from synthetic import checkerboard, flat, gradient, noise, radial

from plotline.core.stage import CancelToken, RunContext
from plotline.core.stages.tone import ToneSettings, ToneStage
from plotline.core.types import Canvas

SQUARE = Canvas(100, 100)


def run_tone(image, canvas=SQUARE, **kw):
    kw.setdefault("resolution", 64)
    frames, tone = collect(
        ToneStage().run(
            {"image": image, "canvas": canvas}, ToneSettings(**kw), RunContext(CancelToken())
        )
    )
    return frames, tone


def test_shape_dtype_and_transform():
    frames, t = run_tone(gradient(64, 48))
    assert t.ink.shape == (48, 64) and t.ink.dtype == np.float32
    assert t.mm_per_px == pytest.approx(1.5625) and t.origin_mm == pytest.approx((0, 12.5))
    assert len(frames) == 1 and frames[0].progress == 1


def test_gradient_ink_falls_left_to_right():
    _, t = run_tone(gradient(64, 48))
    assert t.ink[12, 0] > 0.9 and t.ink[12, -1] < 0.1 and np.all(np.diff(t.ink[12]) <= 1e-6)


def test_invert_levels_gamma_contrast():
    _, a = run_tone(gradient(64, 48))
    _, b = run_tone(gradient(64, 48), invert=True)
    assert np.allclose(a.ink + b.ink, 1, atol=1e-6)
    assert run_tone(gradient(64, 48), black=0.5)[1].ink[12, :14].min() > 0.99
    assert run_tone(flat(16, 16, 0.5), gamma=2.0)[1].ink == pytest.approx(0.75, abs=1e-6)
    assert run_tone(flat(16, 16, 0.75), contrast=1.0)[1].ink == pytest.approx(0.0, abs=1e-6)
    assert run_tone(flat(16, 16, 0.75), contrast=-1.0)[1].ink == pytest.approx(0.5, abs=1e-6)


def test_blur_lowers_variation():
    cb = checkerboard(64, 64, 8)
    assert run_tone(cb, blur_mm=3.0)[1].ink.std() < run_tone(cb)[1].ink.std()


def test_fill_crops_to_the_drawing_aspect():
    _, t = run_tone(flat(64, 64, 0.5), Canvas(100, 50, 0, "fill"))
    assert t.ink.shape == (32, 64) and t.rect_mm == pytest.approx((0, 0, 100, 50))


def test_short_edge_is_at_least_one_pixel():
    _, t = run_tone(flat(4000, 1, 0.5), resolution=100)
    assert t.ink.shape == (1, 100)


def test_downscaling_averages_rather_than_skips():
    _, t = run_tone(checkerboard(256, 256, 128))
    assert abs(float(t.ink.mean()) - 0.5) < 0.05 and float(t.ink.std()) < 0.1


def test_big_downscale_averages_blocks():
    # Averaging 4 x 4 blocks of noise leaves a quarter of its spread; blending 2 x 2 pixels, half.
    _, t = run_tone(noise(256, 256, 1))
    assert float(t.ink.std()) < 0.09


def test_resampling_keeps_pixel_centres_aligned():
    # Shrinking 96 px to 64 puts target pixel j at source index 1.5 j + 0.25; ink is 1 - x / 95.
    expected = 1 - (1.5 * np.arange(64) + 0.25) / 95
    ramp = gradient(96, 72)
    assert run_tone(ramp)[1].ink[0] == pytest.approx(expected, abs=1e-5)
    assert run_tone(ramp.transpose(1, 0, 2))[1].ink[:, 0] == pytest.approx(expected, abs=1e-5)


def test_adjustments_apply_in_the_documented_order():
    # Levels give 2 / 3, gamma 2 gives 4 / 9, contrast 0.5 gives 5 / 12, so the ink is 7 / 12.
    kw = {"black": 0.2, "white": 0.8, "gamma": 2.0, "contrast": 0.5}
    assert run_tone(flat(16, 16, 0.6), **kw)[1].ink == pytest.approx(7 / 12, abs=1e-5)
    assert run_tone(flat(16, 16, 0.6), invert=True, **kw)[1].ink == pytest.approx(5 / 12, abs=1e-5)
    # Levels clip before gamma, so luminance below black is plain black, not (-0.5) ** 2.
    assert run_tone(flat(16, 16, 0.25), black=0.5, gamma=2.0)[1].ink == pytest.approx(1, abs=1e-6)


@pytest.mark.filterwarnings("error")
@pytest.mark.parametrize("white", [0.5, 0.25])
def test_white_not_above_black_is_a_hard_threshold(white):
    ink = run_tone(gradient(64, 48), black=0.5, white=white)[1].ink
    assert np.all(ink[12, :32] == 1) and np.all(ink[12, 32:] == 0)


def test_blur_is_measured_in_mm():
    cb = checkerboard(64, 64, 8)
    on_a_small_page = run_tone(cb, Canvas(100, 100), blur_mm=3.0)[1].ink.std()
    on_a_big_page = run_tone(cb, Canvas(200, 200), blur_mm=3.0)[1].ink.std()
    assert on_a_big_page > on_a_small_page  # the same mm span covers fewer pixels


def test_extreme_aspect_ratios_keep_their_values():
    # A one pixel high ramp averages 40 px blocks: pixel j has x = 40 j + 19.5 of 3999.
    _, t = run_tone(gradient(4000, 1), resolution=100)
    assert t.ink[0] == pytest.approx(1 - (40 * np.arange(100) + 19.5) / 3999, abs=1e-5)
    # The short edge rounds to the nearest pixel: 200 * 100 / 300 is 66.67.
    assert run_tone(flat(300, 200, 0.5), resolution=100)[1].ink.shape == (67, 100)
    assert run_tone(flat(200, 300, 0.5), resolution=100)[1].ink.shape == (100, 67)


def test_fill_keeps_the_centre_of_a_wide_image_for_a_tall_page():
    _, t = run_tone(gradient(100, 50), Canvas(50, 100, 0, "fill"))
    assert t.ink.shape == (64, 32) and t.rect_mm == pytest.approx((0, 0, 50, 100))
    # Columns 37 to 61 of the 100 survive the crop, and ink is 1 - x / 99 there.
    assert t.ink[0, 0] == pytest.approx(1 - 37 / 99, abs=1e-3)
    assert t.ink[0, -1] == pytest.approx(1 - 61 / 99, abs=1e-3)


def test_the_input_image_is_only_read():
    image = gradient(64, 48)
    before = image.copy()
    image.flags.writeable = False  # an in-place write would raise ValueError
    run_tone(image, Canvas(50, 100, 0, "fill"), invert=True, blur_mm=1.0)
    assert np.array_equal(image, before)


def test_synthetic_images_are_float32_rgb_in_range():
    images = [gradient(30, 20), checkerboard(30, 20, 5), radial(30, 20), noise(30, 20, 1)]
    for image in [*images, flat(30, 20, 0.25)]:
        assert image.shape == (20, 30, 3) and image.dtype == np.float32
        assert image.min() >= 0 and image.max() <= 1
    assert gradient(30, 20)[0, 0, 0] == 0 and gradient(30, 20)[0, -1, 0] == 1
    assert checkerboard(30, 20, 5)[0, 0, 0] == 0 and checkerboard(30, 20, 5)[0, 6, 0] == 1
    assert radial(31, 21)[10, 15, 0] == 0 and radial(31, 21)[0, 0, 0] > 0.9
    assert np.array_equal(noise(30, 20, 1), noise(30, 20, 1))
    assert not np.array_equal(noise(30, 20, 1), noise(30, 20, 2))
