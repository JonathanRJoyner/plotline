"""The session: one working context that owns the state and the side effects.

A ``Session`` holds the source image, the canvas, the pens, the chosen style with its settings,
the stage cache and the latest result. It reads image files, runs the style and writes exports,
which the pure core leaves out. The CLI, the GUI and the MCP server call this layer and nothing
below it.
"""

import json
import os
from collections.abc import Sequence
from enum import StrEnum
from pathlib import Path
from typing import Any

import numpy as np
from numpy.typing import NDArray

from plotline.core.cache import StageCache
from plotline.core.export.paths_json import to_json_dict
from plotline.core.export.svg import to_svg
from plotline.core.pipeline import Style, run_style
from plotline.core.registry import get_style
from plotline.core.settings import Settings
from plotline.core.stage import Cancelled, CancelToken, Frame, RunContext
from plotline.core.types import Canvas, Pen, Result
from plotline.session import images


class RunState(StrEnum):
    """Where the latest run stands: ``idle`` before any run, ``running`` while one is under way,
    then ``done``, ``cancelled`` or ``error`` for how it ended."""

    idle = "idle"
    running = "running"
    done = "done"
    cancelled = "cancelled"
    error = "error"


class Session:
    """One working context: an image, a canvas, pens, a style with its settings, the stage cache
    and the latest result.

    The attributes can be read at any time. Change them through the ``set_*`` methods and
    ``update``, which check their input. ``cache_bytes`` bounds the stage cache and ``max_fps``
    caps how often frames are delivered. A session is not safe for concurrent changes, so a front
    end should call it from one thread.
    """

    def __init__(self, cache_bytes: int = 512 * 2**20, max_fps: float = 15.0) -> None:
        self.image: NDArray[np.float32] | None = None
        self.canvas = Canvas(210, 297, 10)
        self.pens: tuple[Pen, ...] = (Pen("Black 0.3", 0.3, "#000000"),)
        self.style: Style = get_style("grid")
        self.settings: dict[str, Settings] = self.style.defaults()
        self.cache = StageCache(cache_bytes)
        self.result: Result | None = None
        self.state = RunState.idle
        self.max_fps = max_fps

    def load_image(self, path: str | os.PathLike[str]) -> None:
        """Read the image file at ``path`` as the source image. See ``images.load_image``."""
        self.image = images.load_image(path)

    def set_image(self, image: NDArray[Any]) -> None:
        """Use ``image`` as the source image. It must have shape ``(H, W, 3)`` or ``(H, W)``,
        where a grey image is repeated into three channels, else ``ValueError`` is raised. A
        ``uint8`` array is divided by 255; any other dtype is taken to be in 0..1 already. A
        ``float32`` array of shape ``(H, W, 3)`` is kept as it is, not copied, so the caller must
        not change it afterwards."""
        pixels = np.asarray(image)
        if pixels.ndim == 2:
            pixels = np.repeat(pixels[:, :, None], 3, axis=2)
        elif pixels.ndim != 3 or pixels.shape[2] != 3:
            raise ValueError(f"image must have shape (H, W) or (H, W, 3), got {pixels.shape}")
        rgb = np.asarray(pixels, dtype=np.float32)
        if pixels.dtype == np.uint8:
            rgb /= 255  # a new array, since the dtype changed, so the caller's is untouched
        self.image = rgb

    def set_canvas(self, canvas: Canvas) -> None:
        """Use ``canvas`` as the page."""
        self.canvas = canvas

    def set_pens(self, pens: Sequence[Pen]) -> None:
        """Use ``pens``, in drawing order. Raises ``ValueError`` when there are none, because
        every result needs a pen."""
        if not pens:
            raise ValueError("a session needs at least one pen")
        self.pens = tuple(pens)

    def set_style(self, style: str | Style) -> None:
        """Switch to ``style``, a registered name or a ``Style``, and reset the settings to its
        defaults. The cache is kept, so stages that the styles share are not run again. Raises
        ``KeyError`` for an unknown name."""
        self.style = get_style(style) if isinstance(style, str) else style
        self.settings = self.style.defaults()

    def update(self, stage: str, **values: Any) -> None:
        """Change settings of one stage, leaving the rest as they were.

        The whole stage is validated again, so a bad value raises ``pydantic.ValidationError`` and
        changes nothing. An unknown stage raises ``KeyError``.
        """
        current = self.settings[stage]
        self.settings[stage] = type(current).model_validate({**current.model_dump(), **values})

    def run_blocking(self) -> Result:
        """Run the style in the calling thread and return the result, which is also kept in
        ``result``.

        Each frame goes through ``_deliver`` as it arrives. A stage error sets ``state`` to
        ``error`` and ``Cancelled`` sets it to ``cancelled``, and both are raised again with
        ``result`` as it was. Raises ``RuntimeError`` when no image is loaded.
        """
        if self.image is None:
            raise RuntimeError("no image is loaded")
        roots = {"image": self.image, "canvas": self.canvas, "pens": self.pens}
        frames = run_style(
            self.style, dict(self.settings), roots, self.cache, RunContext(CancelToken())
        )
        self.state = RunState.running
        try:
            while True:
                try:
                    frame = next(frames)
                except StopIteration as stop:
                    result: Result = stop.value
                    break
                self._deliver(frame)
        except Cancelled:
            self.state = RunState.cancelled
            raise
        except BaseException:
            self.state = RunState.error
            raise
        self.result = result
        self.state = RunState.done
        return result

    def _deliver(self, frame: Frame) -> None:
        """Pass ``frame`` on to whoever watches the run. Nothing receives frames yet."""

    def _require_result(self) -> Result:
        if self.result is None:
            raise RuntimeError("there is no result yet; run the style first")
        return self.result

    def svg(self) -> str:
        """The result as SVG text. Raises ``RuntimeError`` when there is no result."""
        return to_svg(self._require_result())

    def json_dict(self) -> dict[str, Any]:
        """The result as the dict of the JSON paths format. Raises ``RuntimeError`` when there is
        no result."""
        return to_json_dict(self._require_result())

    def export_svg(self, path: str | os.PathLike[str]) -> None:
        """Write the SVG to ``path`` as UTF-8 with LF line ends. Raises ``RuntimeError`` when
        there is no result."""
        Path(path).write_text(self.svg(), encoding="utf-8", newline="\n")

    def export_json(self, path: str | os.PathLike[str]) -> None:
        """Write the JSON paths format to ``path`` as UTF-8 with LF line ends, one value per line
        and no indentation, which diffs well and is a quarter the size of indenting by two.
        Raises ``RuntimeError`` when there is no result."""
        text = json.dumps(self.json_dict(), indent=0) + "\n"
        Path(path).write_text(text, encoding="utf-8", newline="\n")
