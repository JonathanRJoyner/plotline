"""JSON paths export: a result as a plain dict that other programs can read.

The format is named ``plotline-paths`` and has an integer ``version``. Coordinates are page
coordinates in mm (origin at the top-left corner, y down) rounded to three decimals, so the same
result always gives the same text and golden files stay stable. A path of one vertex is a dot.
"""

from dataclasses import asdict
from typing import Any

from plotline.core.types import Result


def to_json_dict(result: Result) -> dict[str, Any]:
    """``result`` as a dict that ``json.dumps`` accepts.

    ``{"format", "version", "units", "canvas", "layers"}``, where each layer is the pen and its
    paths as lists of ``[x, y]`` pairs of Python floats.
    """
    return {
        "format": "plotline-paths",
        "version": 1,
        "units": "mm",
        "canvas": asdict(result.canvas),
        "layers": [
            {
                "pen": asdict(layer.pen),
                "paths": [[[round(float(v), 3) for v in pt] for pt in p] for p in layer.paths],
            }
            for layer in result.layers
        ],
    }
