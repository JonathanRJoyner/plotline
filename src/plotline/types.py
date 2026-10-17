"""The values the Python package hands out and takes in: page, pens, speeds, stats, drawings,
frames, styles and run states.

They are plain dataclasses with no checks of their own. The engine checks a canvas and pens when a
session is given them and raises ``ValueError`` for one it refuses.
"""

from dataclasses import dataclass
from enum import StrEnum
from typing import Any, Literal

import numpy as np
from numpy.typing import NDArray

from plotline import _core


@dataclass(frozen=True)
class Canvas:
    """The page in mm, the margin on every side, whether the image fits inside the drawing area
    (``fit``) or fills it (``fill``), and the shares of the image cut from its left, top, right
    and bottom before it is fitted (``crop``), each 0 to 0.45."""

    width_mm: float
    height_mm: float
    margin_mm: float = 0.0
    fit: Literal["fit", "fill"] = "fit"
    crop: tuple[float, float, float, float] = (0.0, 0.0, 0.0, 0.0)

    def __post_init__(self) -> None:
        # The engine hands a crop back as a list; a tuple keeps equal pages equal.
        object.__setattr__(self, "crop", tuple(self.crop))


@dataclass(frozen=True)
class Pen:
    """A pen: a name, the line width in mm and the colour as ``#rrggbb``."""

    name: str
    width_mm: float
    color: str


@dataclass
class Speeds:
    """How fast a plotter moves, as a front end lets the user set it.

    ``draw_mm_s`` is the speed with the pen down and ``travel_mm_s`` the speed with it up, both in
    mm per second. ``pen_lift_s`` is the time to lift the pen and set it down again, which every
    path costs once. The class is not frozen, so a front end can edit the values in place.
    """

    draw_mm_s: float = 50.0
    travel_mm_s: float = 100.0
    pen_lift_s: float = 0.2


@dataclass(frozen=True)
class Stats:
    """What a drawing costs to plot: the number of paths, the pen-down and pen-up distances in mm
    and the estimated plot time in seconds."""

    path_count: int
    pen_down_mm: float
    pen_up_mm: float
    estimated_seconds: float


@dataclass(frozen=True)
class ToneMap:
    """The ink density in 0..1 that a drawing was made from, one value per pixel of ``ink``, with
    the size of a pixel in mm and the page position of the top left pixel."""

    ink: NDArray[np.float32]
    mm_per_px: float
    origin_mm: tuple[float, float]


@dataclass(frozen=True)
class Layer:
    """The paths drawn with one pen, each an ``(n, 2)`` array of points in mm; a path of one
    point is a dot."""

    pen: Pen
    paths: list[NDArray[np.float64]]


@dataclass(frozen=True)
class Drawing:
    """A finished drawing: the page, a layer per pen and the tone map, if the style made one."""

    canvas: Canvas
    layers: list[Layer]
    tone: ToneMap | None


@dataclass(frozen=True)
class Frame:
    """One step of a run as the preview sees it: the stage, its progress in 0..1, a label, the
    points or paths it has so far, if any, and the pen whose lane gave the frame (0 outside
    lanes)."""

    stage: str
    progress: float
    label: str
    points: NDArray[np.float64] | None
    paths: list[NDArray[np.float64]] | None
    pen: int


@dataclass(frozen=True)
class Style:
    """A built-in style: its name, a short label and a description."""

    name: str
    label: str
    description: str

    def schema(self) -> dict[str, Any]:
        """The style with each stage's name, label and settings as JSON Schema."""
        return _core.describe(self.name)


class RunState(StrEnum):
    """Where the latest run stands: ``idle`` before any run, ``running`` while one is under way,
    then ``done``, ``cancelled`` or ``error`` for how it ended."""

    idle = "idle"
    running = "running"
    done = "done"
    cancelled = "cancelled"
    error = "error"


class Cancelled(Exception):
    """Raised by ``Session.run_blocking`` when the run was cancelled before it finished."""
