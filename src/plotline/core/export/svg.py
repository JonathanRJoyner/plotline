"""SVG export: a result as SVG text with one group per pen layer.

The page is in millimetres. ``width`` and ``height`` carry the unit and the ``viewBox`` has the
same numbers, so one user unit is one mm and coordinates are written as they are. The origin is
the top-left corner of the page and y grows down, as it does in the engine, so nothing is flipped.
Only plain SVG is written: no vendor-namespaced attributes.
"""

from xml.sax.saxutils import escape

import numpy as np
from numpy.typing import NDArray

from plotline.core.types import Result

_QUOT = {'"': "&quot;"}


def _path_data(path: NDArray[np.float64]) -> str:
    """The ``d`` attribute of one path: ``M x y L x y ...`` with three decimals."""
    points = [f"{x:.3f} {y:.3f}" for x, y in path.tolist()]
    if len(points) == 1:
        # A dot is a zero-length line, which a round line cap paints as a dot of the pen width.
        points *= 2
    return "M " + " L ".join(points)


def to_svg(result: Result) -> str:
    """The SVG text for ``result``.

    Each layer is a ``<g>`` numbered from 1 in drawing order (``pen-1``, ``pen-2``, ...) that
    carries the pen's name in ``data-pen``, its colour as the stroke and its width as the stroke
    width, with round caps and joins. Each path in the layer is one ``<path>``.
    """
    w, h = f"{result.canvas.width_mm:g}", f"{result.canvas.height_mm:g}"
    lines = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}mm" height="{h}mm" '
        f'viewBox="0 0 {w} {h}">'
    ]
    for i, layer in enumerate(result.layers, start=1):
        pen = layer.pen
        name = escape(pen.name, _QUOT)
        lines.append(
            f'  <g id="pen-{i}" data-pen="{name}" fill="none" stroke="{pen.color}" '
            f'stroke-width="{pen.width_mm:g}" stroke-linecap="round" stroke-linejoin="round">'
        )
        lines.extend(f'    <path d="{_path_data(p)}"/>' for p in layer.paths)
        lines.append("  </g>")
    lines.append("</svg>")
    return "\n".join(lines) + "\n"
