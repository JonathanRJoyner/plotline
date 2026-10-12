"""Preview PNG: a result drawn at true pen width, for the front ends that show a picture.

Pillow draws the page at ``SUPERSAMPLE`` times the output size and the picture is then reduced by
that factor with a box filter. Pillow fills whole pixels, so averaging each block of them gives
every output pixel the share of it that the strokes cover. One mm on the page is
``width_px / canvas.width_mm`` output pixels in both directions. The page origin is the top left
corner and y grows down, as it does in the engine, so nothing is flipped.
"""

import io

import numpy as np
from numpy.typing import NDArray
from PIL import Image, ImageDraw

from plotline.core.types import Layer, Result, ToneMap

SUPERSAMPLE = 4
_OUTLINE = "#dddddd"
_TRAVEL = "#ff8080"
_SOURCE_OPACITY = 0.3


def render_png(
    result: Result, width_px: int = 1000, travel: bool = False, source: bool = False
) -> bytes:
    """``result`` as PNG bytes that are ``width_px`` wide, as tall as the page's aspect ratio asks.

    The page is white, with the drawing rect outlined in light grey. Each layer's paths are drawn
    in the pen's colour at the pen's width, with round caps and joins, and a path of one vertex is
    a dot as wide as the pen. The width is rounded to whole supersampled pixels, and to one pixel
    at the least so that a thin pen still shows. ``source`` lays the tone map under the paths at
    30 percent opacity, as the grey the pen would ink, when the result has one. ``travel`` adds the
    pen-up moves as thin light red lines, from the last vertex of each path to the first of the
    next one, across the layers in drawing order.
    """
    canvas = result.canvas
    px_per_mm = width_px / canvas.width_mm
    scale = px_per_mm * SUPERSAMPLE  # supersampled pixels per mm
    size = (width_px * SUPERSAMPLE, round(canvas.height_mm * px_per_mm) * SUPERSAMPLE)
    img = Image.new("RGB", size, "white")
    draw = ImageDraw.Draw(img)
    x, y, w, h = canvas.drawing_rect
    # Pillow draws the last pixel of a box too, so the far edges are one pixel in.
    draw.rectangle(
        [x * scale, y * scale, (x + w) * scale - 1, (y + h) * scale - 1],
        outline=_OUTLINE,
        width=SUPERSAMPLE,
    )
    if source and result.tone is not None:
        _blend_tone(img, result.tone, scale)
    ends = [_draw_layer(draw, layer, scale) for layer in result.layers]
    if travel:
        # The pen goes first0 last0 first1 last1 ..., so the moves are (last0, first1), (last1,
        # first2) and so on: the route without its first and last points, taken in pairs. The
        # empty array at the front is for a result without layers.
        route = np.concatenate([np.empty((0, 2, 2)), *ends]).reshape(-1, 2)
        for move in route[1:-1].reshape(-1, 4).tolist():
            draw.line(move, fill=_TRAVEL, width=SUPERSAMPLE)
    out = io.BytesIO()
    img.reduce(SUPERSAMPLE).save(out, format="PNG")
    return out.getvalue()


def _blend_tone(img: Image.Image, tone: ToneMap, scale: float) -> None:
    """Blend ``tone`` into ``img`` over the area of the page it covers, as the grey the pen would
    ink: white where the ink is 0 and black where it is 1."""
    x, y, w, h = tone.rect_mm
    left, top = round(x * scale), round(y * scale)
    right, bottom = round((x + w) * scale), round((y + h) * scale)
    grey = Image.fromarray(np.clip(255 * (1 - tone.ink), 0, 255).astype(np.uint8))
    grey = grey.resize((right - left, bottom - top)).convert("RGB")
    blended = Image.blend(img.crop((left, top, right, bottom)), grey, _SOURCE_OPACITY)
    img.paste(blended, (left, top))


def _draw_layer(draw: ImageDraw.ImageDraw, layer: Layer, scale: float) -> NDArray[np.float64]:
    """Draw the paths of ``layer`` and return the first and last vertex of each of them, in
    pixels, as an array of shape ``(n, 2, 2)``.

    All the vertices are scaled at once and each path is a slice of them, so a dot costs one
    Pillow call and no array work.
    """
    paths = layer.paths
    if not paths:
        return np.empty((0, 2, 2))
    color = layer.pen.color
    width = max(1, round(layer.pen.width_mm * scale))
    sizes = np.fromiter(map(len, paths), np.intp, len(paths))
    stop = np.cumsum(sizes)
    start = stop - sizes
    points = np.concatenate(paths) * scale
    lines = sizes > 1
    # A round cap is a circle on the first vertex of every path, and on the last one of each path
    # that is not a dot. Around the pixel v that holds a vertex, a horizontal or vertical line
    # takes the pixels from v - (width - 1) // 2 to v + width // 2, and the circle is given the
    # same box, so the caps sit flush with the strokes and a dot is exactly as wide as the pen.
    caps = np.floor(np.concatenate((points[start], points[stop[lines] - 1])))
    stamp = draw.ellipse if width > 1 else draw.rectangle  # Pillow draws no ellipse of 1 pixel
    for box in np.hstack((caps - (width - 1) // 2, caps + width // 2)).tolist():
        stamp(box, fill=color)
    for a, b in zip(start[lines].tolist(), stop[lines].tolist(), strict=True):
        draw.line(points[a:b].ravel().tolist(), fill=color, width=width, joint="curve")
    return points[np.stack((start, stop - 1), axis=1)]
