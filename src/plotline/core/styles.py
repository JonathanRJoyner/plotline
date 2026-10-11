"""The built-in styles, which ``registry`` lists. This module must not import ``registry``."""

from plotline.core.pipeline import Style
from plotline.core.stages.grid import DotRenderer, GridPlacer
from plotline.core.stages.order import OrderStage
from plotline.core.stages.tone import ToneStage

GRID = Style(
    name="grid",
    label="Grid dots",
    description="Dots on a regular grid, one wherever the image is dark enough, drawn in an "
    "order that keeps the pen's travel short.",
    stages=(ToneStage(), GridPlacer(), DotRenderer(), OrderStage()),
)

BUILTIN: tuple[Style, ...] = (GRID,)
