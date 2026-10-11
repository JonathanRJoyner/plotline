"""Core data types: pen, canvas, tone map, layer and result.

Coordinates are page coordinates in millimetres: the origin is the top-left corner of the page,
x grows to the right and y grows down. The only pixel coordinates are the tone map's own.
"""

import re
from dataclasses import dataclass
from typing import Literal

import numpy as np
from numpy.typing import NDArray
from scipy.ndimage import map_coordinates

_COLOR = re.compile(r"#[0-9a-fA-F]{6}")


@dataclass(frozen=True)
class Pen:
    """A physical pen: a name, a stroke width in mm and a colour written as ``#rrggbb``."""

    name: str
    width_mm: float
    color: str

    def __post_init__(self) -> None:
        if not self.width_mm > 0:
            raise ValueError(f"width_mm must be positive, got {self.width_mm}")
        if not _COLOR.fullmatch(self.color):
            raise ValueError(f"color must be written as #rrggbb, got {self.color!r}")


@dataclass(frozen=True)
class Canvas:
    """The page in mm, a margin around the drawing area, and how an image fits that area.

    ``fit`` scales the whole image to sit inside the drawing area, centred. ``fill`` makes the
    image rectangle the drawing area itself; the caller crops the image to that aspect ratio.
    """

    width_mm: float
    height_mm: float
    margin_mm: float = 0.0
    fit: Literal["fit", "fill"] = "fit"

    def __post_init__(self) -> None:
        if not self.width_mm > 0:
            raise ValueError(f"width_mm must be positive, got {self.width_mm}")
        if not self.height_mm > 0:
            raise ValueError(f"height_mm must be positive, got {self.height_mm}")
        if not self.margin_mm >= 0:
            raise ValueError(f"margin_mm must not be negative, got {self.margin_mm}")
        if not 2 * self.margin_mm < min(self.width_mm, self.height_mm):
            raise ValueError(
                f"margin_mm must leave a positive drawing area, got {self.margin_mm} "
                f"for a {self.width_mm} x {self.height_mm} mm page"
            )

    @property
    def drawing_rect(self) -> tuple[float, float, float, float]:
        """The page minus the margin, as ``(x, y, w, h)`` in mm."""
        m = self.margin_mm
        return (m, m, self.width_mm - 2 * m, self.height_mm - 2 * m)

    def image_rect(self, width_px: int, height_px: int) -> tuple[float, float, float, float]:
        """Where an image of this pixel size sits on the page, as ``(x, y, w, h)`` in mm."""
        x, y, w, h = self.drawing_rect
        if self.fit == "fill":
            return (x, y, w, h)
        scale = min(w / width_px, h / height_px)
        iw, ih = width_px * scale, height_px * scale
        return (x + (w - iw) / 2, y + (h - ih) / 2, iw, ih)

    def image_transform(self, width_px: int, height_px: int) -> tuple[float, tuple[float, float]]:
        """The tone map placement for an image of this pixel size: ``(mm_per_px, origin_mm)``."""
        x, y, w, _ = self.image_rect(width_px, height_px)
        return (w / width_px, (x, y))


@dataclass(frozen=True, eq=False)
class ToneMap:
    """Ink density per pixel (0 is white, 1 is full ink) and where the pixels sit on the page.

    Pixel ``(row, col)`` covers ``[col, col + 1) x [row, row + 1)`` in pixel coordinates, so its
    centre is at ``(col + 0.5, row + 0.5)``.
    """

    ink: NDArray[np.float32]
    mm_per_px: float
    origin_mm: tuple[float, float]

    @property
    def rect_mm(self) -> tuple[float, float, float, float]:
        """The page area the whole map covers, as ``(x, y, w, h)`` in mm."""
        x, y = self.origin_mm
        h, w = self.ink.shape
        return (x, y, w * self.mm_per_px, h * self.mm_per_px)

    def to_mm(self, px: NDArray[np.float64]) -> NDArray[np.float64]:
        """Convert ``(N, 2)`` pixel coordinates ``(x, y)`` to page mm."""
        return np.asarray(px * self.mm_per_px + self.origin_mm, dtype=np.float64)

    def to_px(self, mm: NDArray[np.float64]) -> NDArray[np.float64]:
        """Convert ``(N, 2)`` page mm ``(x, y)`` to pixel coordinates."""
        return np.asarray((mm - self.origin_mm) / self.mm_per_px, dtype=np.float64)

    def sample(self, points_mm: NDArray[np.float64]) -> NDArray[np.float32]:
        """Bilinear ink at ``(N, 2)`` page points; points off the map clamp to the edge pixels."""
        # Index space has pixel centres at integers; map_coordinates wants (row, col) as (2, N).
        rows_cols = (self.to_px(points_mm) - 0.5)[:, ::-1].T
        return np.asarray(
            map_coordinates(self.ink, rows_cols, order=1, mode="nearest"), dtype=np.float32
        )


@dataclass(frozen=True, eq=False)
class Layer:
    """The paths drawn with one pen. Each path is an ``(N, 2)`` array of mm, ``N >= 1``."""

    pen: Pen
    paths: list[NDArray[np.float64]]


@dataclass(frozen=True, eq=False)
class Result:
    """A finished drawing: the canvas, the layers in drawing order, and the tone map if any."""

    canvas: Canvas
    layers: list[Layer]
    tone: ToneMap | None = None
