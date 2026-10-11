"""Synthetic test images, generated in code so the repo holds no third-party pictures.

Import them as ``from synthetic import gradient``. Each function returns a float32 RGB array of
shape ``(h, w, 3)`` with values in 0..1.
"""

import numpy as np
from numpy.typing import ArrayLike, NDArray


def _grey(luminance: ArrayLike) -> NDArray[np.float32]:
    """Repeat an ``(h, w)`` luminance into three equal RGB channels."""
    return np.repeat(np.asarray(luminance, np.float32)[:, :, None], 3, axis=2)


def gradient(w: int, h: int) -> NDArray[np.float32]:
    """Luminance ``x / (w - 1)``: black at the left edge, white at the right."""
    return _grey(np.broadcast_to(np.linspace(0, 1, w), (h, w)))


def checkerboard(w: int, h: int, cells: int) -> NDArray[np.float32]:
    """Square cells of side ``w // cells`` alternating 0 and 1, with 0 at the top left."""
    side = w // cells
    y, x = np.indices((h, w))
    return _grey((x // side + y // side) % 2)


def radial(w: int, h: int) -> NDArray[np.float32]:
    """The distance from the centre over half the diagonal, clipped: 0 in the middle, near 1 at
    the corners."""
    y, x = np.indices((h, w))
    distance = np.hypot(x - (w - 1) / 2, y - (h - 1) / 2) / (np.hypot(w, h) / 2)
    return _grey(np.clip(distance, 0, 1))


def noise(w: int, h: int, seed: int) -> NDArray[np.float32]:
    """Uniform random luminance from ``np.random.default_rng(seed)``, equal in all channels."""
    return _grey(np.random.default_rng(seed).random((h, w)))


def flat(w: int, h: int, value: float) -> NDArray[np.float32]:
    """The grey level ``value`` everywhere."""
    return np.full((h, w, 3), value, np.float32)
