"""The tone stage: turn the RGB image into a tone map on the canvas.

Luminance uses the ITU-R BT.601 weights, Y = 0.299 R + 0.587 G + 0.114 B (Recommendation
ITU-R BT.601, "Studio encoding parameters of digital television for standard 4:3 and wide-screen
16:9 aspect ratios"). A source much larger than the tone map is averaged in blocks before it is
sampled bilinearly, so fine detail is averaged rather than skipped.
"""

from collections.abc import Generator, Mapping
from typing import Any

import numpy as np
from numpy.typing import NDArray
from scipy.ndimage import gaussian_filter, map_coordinates

from plotline.core.settings import Settings, setting
from plotline.core.stage import Frame, RunContext, Stage
from plotline.core.types import Canvas, ToneMap


class ToneSettings(Settings):
    """Settings for the tone stage."""

    resolution: int = setting(
        1000,
        min=64,
        max=4000,
        unit="px",
        help="Length of the tone map's longer edge in pixels; more pixels follow the image more "
        "closely but run slower.",
    )
    invert: bool = setting(False, help="Swap light and dark, so that light areas get the ink.")
    black: float = setting(
        0.0,
        min=0,
        max=1,
        step=0.01,
        help="Luminance at or below this level becomes black, which takes full ink.",
    )
    white: float = setting(
        1.0,
        min=0,
        max=1,
        step=0.01,
        help="Luminance at or above this level becomes white, which takes no ink.",
    )
    gamma: float = setting(
        1.0,
        min=0.1,
        max=5,
        step=0.05,
        help="Midtone curve applied to the luminance: above 1 darkens the midtones, below 1 "
        "lightens them.",
    )
    contrast: float = setting(
        0.0,
        min=-1,
        max=1,
        step=0.05,
        help="Contrast around mid-grey: positive values push tones apart, negative values "
        "squeeze them towards mid-grey.",
    )
    blur_mm: float = setting(
        0.0,
        min=0,
        max=10,
        step=0.1,
        unit="mm",
        advanced=True,
        help="Gaussian blur, given as sigma in mm, that smooths the tone map; 0 turns it off.",
    )


def _crop_to_aspect(lum: NDArray[np.float32], aspect: float) -> NDArray[np.float32]:
    """Centre-crop to a width over height of ``aspect``, to the nearest pixel. Returns a view."""
    h, w = lum.shape
    ch = min(h, max(1, round(w / aspect)))
    cw = min(w, max(1, round(h * aspect)))
    y0, x0 = (h - ch) // 2, (w - cw) // 2
    return lum[y0 : y0 + ch, x0 : x0 + cw]


def _resize(lum: NDArray[np.float32], long_edge: int) -> NDArray[np.float32]:
    """Resample to ``long_edge`` pixels on the longer side, keeping the aspect ratio and at least
    one pixel on the other side."""
    h, w = lum.shape
    if h >= w:
        th, tw = long_edge, max(1, round(w * long_edge / h))
    else:
        th, tw = max(1, round(h * long_edge / w)), long_edge
    k = max(h, w) // long_edge
    if k >= 2:
        # Average k x k blocks (a side shorter than k becomes one block), dropping the ragged edge.
        by, bx = min(k, h), min(k, w)
        nh, nw = h // by, w // bx
        lum = lum[: nh * by, : nw * bx].reshape(nh, by, nw, bx).mean(axis=(1, 3), dtype=np.float32)
    # Target pixel j has its centre at (j + 0.5) * source / target - 0.5 in source index space.
    sh, sw = lum.shape
    rows = (np.arange(th) + 0.5) * sh / th - 0.5
    cols = (np.arange(tw) + 0.5) * sw / tw - 0.5
    grid = np.meshgrid(rows, cols, indexing="ij")
    return np.asarray(map_coordinates(lum, grid, order=1, mode="nearest"), dtype=np.float32)


class ToneStage(Stage):
    """Turn the image into a tone map. In order: luminance, a centre crop for ``fill``, resize,
    levels, gamma, contrast, invert and blur."""

    name = "tone"
    label = "Tone"
    requires = ("image", "canvas")
    provides = "tone"
    settings_model = ToneSettings

    def run(
        self, inputs: Mapping[str, Any], settings: Settings, ctx: RunContext
    ) -> Generator[Frame, None, ToneMap]:
        assert isinstance(settings, ToneSettings)
        canvas: Canvas = inputs["canvas"]
        rgb = np.asarray(inputs["image"], dtype=np.float32)
        lum = 0.299 * rgb[..., 0] + 0.587 * rgb[..., 1] + 0.114 * rgb[..., 2]
        if canvas.fit == "fill":
            _, _, w, h = canvas.drawing_rect
            lum = _crop_to_aspect(lum, w / h)
        y = _resize(lum, settings.resolution)
        # white <= black makes the levels a hard threshold at black instead of dividing by zero.
        y = np.clip((y - settings.black) / max(settings.white - settings.black, 1e-6), 0, 1)
        y = y**settings.gamma
        y = np.clip(0.5 + (y - 0.5) * (1 + settings.contrast), 0, 1)
        ink = 1 - y
        if settings.invert:
            ink = 1 - ink
        th, tw = y.shape
        mm_per_px, origin_mm = canvas.image_transform(tw, th)
        if settings.blur_mm > 0:
            ink = np.asarray(gaussian_filter(ink, settings.blur_mm / mm_per_px), dtype=np.float32)
        yield Frame(1.0, "tone map")
        return ToneMap(np.asarray(np.clip(ink, 0, 1), dtype=np.float32), mm_per_px, origin_mm)
