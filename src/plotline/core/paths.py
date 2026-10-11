"""Path geometry: the length of a path and the pen-up travel between paths.

A path is a float64 ``(N, 2)`` array in mm with ``N >= 1``. A path of one vertex is a dot and has
zero length. Everything here is a pure function of its arguments and reads them only.
"""

import math
from collections.abc import Iterable, Sequence

import numpy as np
from numpy.typing import NDArray


def path_length(path: NDArray[np.float64]) -> float:
    """The length of the polyline through the vertices of ``path`` in mm; 0 for one vertex."""
    return float(np.linalg.norm(np.diff(path, axis=0), axis=1).sum())


def total_length(paths: Iterable[NDArray[np.float64]]) -> float:
    """The sum of ``path_length`` over ``paths`` in mm, which is the pen-down distance."""
    return math.fsum(path_length(p) for p in paths)


def travel_length(paths: Sequence[NDArray[np.float64]], start: tuple[float, float]) -> float:
    """The pen-up distance in mm when ``paths`` are drawn in order with the pen starting at
    ``start``: the move from ``start`` to the first vertex of the first path, plus the gap from
    each path's last vertex to the next path's first vertex. 0 for no paths."""
    if not paths:
        return 0.0
    firsts = np.array([p[0] for p in paths])
    lasts = np.array([p[-1] for p in paths])
    return float(np.linalg.norm(firsts - np.vstack((start, lasts[:-1])), axis=1).sum())
