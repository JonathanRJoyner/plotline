"""The session: one working context that owns the state and the side effects.

A ``Session`` holds the source image, the canvas, the pens, the chosen style with its settings,
the stage cache and the latest result. It reads image files, runs the style and writes exports,
which the pure core leaves out. The CLI, the GUI and the MCP server call this layer and nothing
below it.
"""

import json
import math
import os
import threading
import time
from collections.abc import Callable, Mapping, Sequence
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
from plotline.session.preview import render_png
from plotline.session.stats import Speeds, Stats, compute_stats


class RunState(StrEnum):
    """Where the latest run stands: ``idle`` before any run, ``running`` while one is under way,
    then ``done``, ``cancelled`` or ``error`` for how it ended."""

    idle = "idle"
    running = "running"
    done = "done"
    cancelled = "cancelled"
    error = "error"


class RunHandle:
    """One run of a style, as ``Session.run`` or ``Session.run_blocking`` makes it.

    ``state`` starts as ``running`` and ends as ``done``, ``cancelled`` or ``error``. ``result``
    is set when the run is ``done`` and ``error`` holds the exception when it ended in ``error``.
    All of them can be read from any thread. A handle from ``run_blocking`` has no thread, so it
    is finished by the time the caller has it.
    """

    def __init__(self) -> None:
        self._token = CancelToken()
        self._thread: threading.Thread | None = None
        self._state = RunState.running
        self._result: Result | None = None
        self._error: BaseException | None = None

    @property
    def state(self) -> RunState:
        return self._state

    @property
    def result(self) -> Result | None:
        return self._result

    @property
    def error(self) -> BaseException | None:
        return self._error

    def cancel(self) -> None:
        """Ask the run to stop at its next frame. A run that has already finished stays as it
        ended."""
        self._token.set()

    def wait(self, timeout: float | None = None) -> RunState:
        """Block until the run has ended, or for ``timeout`` seconds at most, and return
        ``state``, which is still ``running`` after a timeout. A subscriber runs on the worker
        thread, which cannot wait for itself, so it must not call this."""
        if self._thread is not None:
            self._thread.join(timeout)
        return self._state

    def _end(
        self,
        state: RunState,
        result: Result | None = None,
        error: BaseException | None = None,
    ) -> None:
        # The state goes last, so a reader that sees the end state sees the result or error too.
        self._result = result
        self._error = error
        self._state = state


class Session:
    """One working context: an image, a canvas, pens, a style with its settings, the stage cache
    and the latest result.

    The attributes can be read at any time. Change them through the ``set_*`` methods and
    ``update``, which check their input. ``cache_bytes`` bounds the stage cache and ``max_fps``
    caps how often frames are delivered to the subscribers. ``run`` works on a worker thread and
    ``run_blocking`` in the caller's. Starting either one stops the run before it, so at most one
    is active. A session is not safe for concurrent changes, so a front end should call it from
    one thread. ``speeds``, which ``stats`` reads, has no setter and is edited in place.
    """

    def __init__(self, cache_bytes: int = 512 * 2**20, max_fps: float = 15.0) -> None:
        self.image: NDArray[np.float32] | None = None
        self.canvas = Canvas(210, 297, 10)
        self.pens: tuple[Pen, ...] = (Pen("Black 0.3", 0.3, "#000000"),)
        self.style: Style = get_style("grid")
        self.settings: dict[str, Settings] = self.style.defaults()
        self.cache = StageCache(cache_bytes)
        self.result: Result | None = None
        self.speeds = Speeds()
        self.max_fps = max_fps
        self._handle: RunHandle | None = None
        self._subscribers: dict[object, Callable[[Frame], None]] = {}
        self._last_delivery = -math.inf

    @property
    def state(self) -> RunState:
        """The state of the latest run, or ``idle`` before any run."""
        return RunState.idle if self._handle is None else self._handle.state

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

        A run under way on a worker is cancelled and waited for first. Each frame goes through
        ``_deliver`` as it arrives. A stage error sets ``state`` to ``error`` and ``Cancelled``
        sets it to ``cancelled``, and both are raised again with ``result`` as it was. Raises
        ``RuntimeError`` when no image is loaded.
        """
        return self._execute(*self._start())

    def run(self) -> RunHandle:
        """Run the style on a worker thread and return its handle at once.

        A run under way is cancelled and waited for first, so two runs never use the cache at
        the same time. The style, settings, canvas, pens and image are taken now, so later
        changes do not reach the run. The subscribers get its frames on the worker thread. Unlike
        ``run_blocking`` it raises no stage error: the run ends in ``error`` and the handle keeps
        the exception. Raises ``RuntimeError`` when no image is loaded.
        """
        handle, style, settings, roots = self._start()
        thread = threading.Thread(
            target=self._work, args=(handle, style, settings, roots), daemon=True
        )
        handle._thread = thread
        thread.start()
        return handle

    def cancel(self) -> None:
        """Ask the latest run to stop at its next frame, which ends it in ``cancelled``. Nothing
        happens when there has been no run, and a run that has ended stays as it ended."""
        if self._handle is not None:
            self._handle.cancel()

    def subscribe(self, callback: Callable[[Frame], None]) -> Callable[[], None]:
        """Call ``callback(frame)`` for the frames of every later run, and return a function that
        removes it again.

        ``max_fps`` caps the rate, see ``_deliver``. The callback runs on the thread that drives
        the style, the worker for ``run`` and the caller for ``run_blocking``, so it should
        return quickly. An exception from it ends the run in ``error``, as one from a stage does.
        """
        key = object()
        self._subscribers[key] = callback

        def unsubscribe() -> None:
            self._subscribers.pop(key, None)

        return unsubscribe

    def _start(self) -> tuple[RunHandle, Style, dict[str, Settings], dict[str, Any]]:
        """Stop the latest run and make the handle of the next one, along with the snapshot it
        reads: the style, a copy of the settings and the roots. Raises ``RuntimeError`` when no
        image is loaded."""
        if self.image is None:
            raise RuntimeError("no image is loaded")
        if self._handle is not None:
            self._handle.cancel()
            self._handle.wait()
        roots = {"image": self.image, "canvas": self.canvas, "pens": self.pens}
        self._handle = handle = RunHandle()
        return handle, self.style, dict(self.settings), roots

    def _work(
        self,
        handle: RunHandle,
        style: Style,
        settings: Mapping[str, Settings],
        roots: Mapping[str, Any],
    ) -> None:
        """The body of a worker thread."""
        try:
            self._execute(handle, style, settings, roots)
        except Exception:
            pass  # the handle keeps it, and a thread has no caller to raise to

    def _execute(
        self,
        handle: RunHandle,
        style: Style,
        settings: Mapping[str, Settings],
        roots: Mapping[str, Any],
    ) -> Result:
        """Drive the style to its end in the current thread, passing each frame to ``_deliver``,
        and record how it ended on ``handle``. This is the one place that moves a run through its
        states. ``Cancelled`` and any other error are raised again, with ``result`` as it was."""
        self._last_delivery = -math.inf
        frames = run_style(style, settings, roots, self.cache, RunContext(handle._token))
        try:
            while True:
                try:
                    frame = next(frames)
                except StopIteration as stop:
                    result: Result = stop.value
                    break
                self._deliver(frame)
        except Cancelled:
            handle._end(RunState.cancelled)
            raise
        except BaseException as error:
            handle._end(RunState.error, error=error)
            raise
        self.result = result
        handle._end(RunState.done, result)
        return result

    def _deliver(self, frame: Frame) -> None:
        """Pass ``frame`` to the subscribers, unless less than ``1 / max_fps`` seconds have
        passed since the last delivery. The first frame of a run and every frame with ``progress``
        of 1 or more are always passed on, so a throttled preview still ends on the finished
        state of each stage."""
        # Not time.monotonic: before Python 3.13 it ticks every 15.6 ms on Windows, so a fast stage
        # would lose frames even when max_fps asks for all of them.
        now = time.perf_counter()
        if frame.progress < 1 and now - self._last_delivery < 1 / self.max_fps:
            return
        self._last_delivery = now
        for callback in list(self._subscribers.values()):
            callback(frame)

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

    def stats(self) -> Stats:
        """The path count, pen-down and pen-up distances and estimated plot time of the result at
        ``speeds``. Raises ``RuntimeError`` when there is no result."""
        return compute_stats(self._require_result(), self.speeds)

    def preview_png(
        self, width_px: int = 1000, travel: bool = False, source: bool = False
    ) -> bytes:
        """The result drawn at true pen width as PNG bytes that are ``width_px`` wide, with the
        pen-up moves when ``travel`` is set and the tone map under the paths when ``source`` is.
        See ``preview.render_png``. Raises ``RuntimeError`` when there is no result."""
        return render_png(self._require_result(), width_px, travel, source)
