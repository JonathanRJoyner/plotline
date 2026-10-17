"""plotline: turn images into drawings for pen plotters.

The engine is the extension module ``plotline._core``. This package is the Python host around it:
``Session`` for working with an image, the values it takes and gives, and the style registry.
"""

from plotline._core import describe, inks
from plotline.registry import get_style, list_styles
from plotline.session import RunHandle, Session
from plotline.types import Cancelled, Canvas, Drawing, Frame, Pen, RunState, Speeds, Stats, Style

__all__ = [
    "Cancelled",
    "Canvas",
    "Drawing",
    "Frame",
    "Pen",
    "RunHandle",
    "RunState",
    "Session",
    "Speeds",
    "Stats",
    "Style",
    "describe",
    "get_style",
    "inks",
    "list_styles",
]

__version__ = "0.2.0"
