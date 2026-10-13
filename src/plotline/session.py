"""The session: one working context over the engine's session, with the side effects the engine
leaves to its host.

The engine (``plotline._core``) keeps the source image, the canvas, the pens, the chosen style with
its settings, the plot speeds, the stage cache and the latest drawing, and checks every change. This
module adds what a Python host does: it reads and writes files, drives a run on a worker thread or
in the caller's, passes frames to subscribers at a capped rate, and turns the engine's dicts and
flat path buffers into the dataclasses of ``plotline.types``. The CLI, the GUI and the MCP server
call this layer and nothing below it.
"""

import json
import math
import os
import threading
import time
import warnings
from collections.abc import Callable, Sequence
from dataclasses import asdict
from itertools import pairwise
from pathlib import Path
from typing import Any, TypeVar

import numpy as np
from numpy.typing import NDArray

from plotline import _core
from plotline.registry import get_style
from plotline.types import (
    Cancelled,
    Canvas,
    Drawing,
    Frame,
    Layer,
    Pen,
    RunState,
    Speeds,
    Stats,
    Style,
    ToneMap,
)

T = TypeVar("T")


def _split(coords: NDArray[np.float64], offsets: NDArray[np.uint32]) -> list[NDArray[np.float64]]:
    """The paths of a flat buffer as ``(n, 2)`` views: path ``i`` is
    ``coords[offsets[i]:offsets[i + 1]]``."""
    return [coords[a:b] for a, b in pairwise(offsets.tolist())]


def _frame(f: dict[str, Any]) -> Frame:
    coords = f["coords"]
    paths = None if coords is None else _split(coords, f["offsets"])
    return Frame(f["stage"], f["progress"], f["label"], f["points"], paths, f["pen"])


def _drawing(d: dict[str, Any]) -> Drawing:
    layers = [Layer(Pen(**x["pen"]), _split(x["coords"], x["offsets"])) for x in d["layers"]]
    tone = None if d["tone"] is None else ToneMap(**d["tone"])
    return Drawing(Canvas(**d["canvas"]), layers, tone)


def _have(value: T | None) -> T:
    if value is None:
        raise RuntimeError("there is no result yet; run the style first")
    return value


class RunHandle:
    """One run of a style, as ``Session.run`` or ``Session.run_blocking`` makes it.

    ``state`` starts as ``running`` and ends as ``done``, ``cancelled`` or ``error``. ``result``
    is set when the run is ``done`` and ``error`` holds the exception when it ended in ``error``:
    a ``RuntimeError`` with the engine's message for a failed stage, or what a subscriber raised.
    All of them can be read from any thread. A handle from ``run_blocking`` has no thread, so it
    is finished by the time the caller has it.
    """

    def __init__(self, run: _core.Run) -> None:
        self._run = run
        self._thread: threading.Thread | None = None
        self._state = RunState.running
        self._result: Drawing | None = None
        self._error: BaseException | None = None

    @property
    def state(self) -> RunState:
        return self._state

    @property
    def result(self) -> Drawing | None:
        return self._result

    @property
    def error(self) -> BaseException | None:
        return self._error

    def cancel(self) -> None:
        """Ask the run to stop; the stage under way sees it at its next check. A run that has
        already finished stays as it ended."""
        self._run.cancel()

    def wait(self, timeout: float | None = None) -> RunState:
        """Block until the run has ended, or for ``timeout`` seconds at most, and return
        ``state``, which is still ``running`` after a timeout. A subscriber runs on the worker
        thread, which cannot wait for itself, so a call from there raises ``RuntimeError``
        instead of hanging."""
        if self._thread is threading.current_thread():
            raise RuntimeError("a run cannot wait for itself: wait() was called on its own thread")
        if self._thread is not None:
            self._thread.join(timeout)
        return self._state

    def _end(
        self,
        state: RunState,
        result: Drawing | None = None,
        error: BaseException | None = None,
    ) -> None:
        # The state goes last, so a reader that sees the end state sees the result or error too.
        self._result = result
        self._error = error
        self._state = state


class Session:
    """One working context: an image, a canvas, pens, a style with its settings, plot speeds, the
    stage cache and the latest result.

    The attributes can be read at any time; ``image``, ``canvas``, ``pens``, ``style`` and
    ``settings`` are copies of what the engine holds. Change them through the ``set_*`` methods and
    ``update``, which the engine checks: a refused value raises ``ValueError`` and changes
    nothing. ``cache_bytes`` bounds the stage cache and ``max_fps`` caps how often frames are
    delivered to the subscribers. ``run`` works on a worker thread and ``run_blocking`` in the
    caller's. Starting either one stops the run before it, so at most one is active. A session is
    not safe for concurrent changes, so a front end should call it from one thread. ``speeds``,
    which ``stats`` reads, has no setter and is edited in place.
    """

    def __init__(self, cache_bytes: int = 512 * 2**20, max_fps: float = 15.0) -> None:
        self._core = _core.Session(cache_bytes)
        self.result: Drawing | None = None
        self.speeds = Speeds()
        self.max_fps = max_fps
        self._handle: RunHandle | None = None
        self._subscribers: dict[object, Callable[[Frame], None]] = {}
        self._last_delivery = -math.inf

    @property
    def image(self) -> NDArray[np.float32] | None:
        """The source image as an ``(H, W, 3)`` float32 array in 0..1, or ``None``."""
        return self._core.image()

    @property
    def canvas(self) -> Canvas:
        return Canvas(**self._core.canvas())

    @property
    def pens(self) -> tuple[Pen, ...]:
        return tuple(Pen(**pen) for pen in self._core.pens())

    @property
    def style(self) -> Style:
        return get_style(self._core.style_name())

    @property
    def settings(self) -> dict[str, dict[str, Any]]:
        """Every stage's settings, keyed by stage name."""
        return self._core.settings()

    @property
    def state(self) -> RunState:
        """The state of the latest run, or ``idle`` before any run."""
        return RunState.idle if self._handle is None else self._handle.state

    def load_image(self, path: str | os.PathLike[str]) -> None:
        """Read the PNG, JPEG or WebP file at ``path`` as the source image. Raises ``OSError``
        when the file cannot be read and ``ValueError`` when the engine cannot decode it."""
        self._core.set_image_bytes(Path(path).read_bytes())

    def set_image(self, image: NDArray[Any]) -> None:
        """Use ``image`` as the source image. It must have shape ``(H, W, 3)`` or ``(H, W)``,
        where a grey image is repeated into three channels, else ``ValueError`` is raised. An
        integer array is divided by the largest value of its type, so ``uint8`` by 255 and
        ``uint16`` by 65535; a float array is taken to be in 0..1 already. The engine keeps a
        copy, so the caller may change the array afterwards."""
        pixels = np.asarray(image)
        if pixels.ndim == 2:
            pixels = np.repeat(pixels[:, :, None], 3, axis=2)
        elif pixels.ndim != 3 or pixels.shape[2] != 3:
            raise ValueError(f"image must have shape (H, W) or (H, W, 3), got {pixels.shape}")
        rgb = np.asarray(pixels, dtype=np.float32)
        if np.issubdtype(pixels.dtype, np.integer):
            # rgb is a new array, since the dtype changed, so the caller's is untouched.
            rgb /= np.iinfo(pixels.dtype).max
        self._core.set_image(rgb)

    def set_canvas(self, canvas: Canvas) -> None:
        """Use ``canvas`` as the page. Raises ``ValueError`` for a page or margin that leaves no
        drawing area."""
        self._core.set_canvas(asdict(canvas))

    def set_pens(self, pens: Sequence[Pen]) -> None:
        """Use ``pens``, in drawing order. Raises ``ValueError`` when there are none, because
        every result needs a pen, and for a pen the engine refuses."""
        self._core.set_pens([asdict(pen) for pen in pens])

    def set_style(self, name: str) -> None:
        """Switch to the built-in style called ``name`` and reset the settings to its defaults.
        The cache is kept, so a stage that the styles share, with the same settings and
        inputs, is not run again. Raises
        ``ValueError`` for an unknown name."""
        self._core.set_style(name)

    def update(self, stage: str, **values: Any) -> None:
        """Change settings of one stage, leaving the rest as they were. The whole stage is
        checked again, so a bad value or an unknown stage raises ``ValueError`` and changes
        nothing."""
        self._core.update(stage, values)

    def save_preset(self, path: str | os.PathLike[str]) -> None:
        """Write the style, canvas, pens and settings to ``path`` as a JSON preset, UTF-8 with LF
        line ends and indented by two spaces so that it reads and diffs well."""
        text = json.dumps(self._core.preset(), indent=2) + "\n"
        Path(path).write_text(text, encoding="utf-8", newline="\n")

    def load_preset(self, path: str | os.PathLike[str]) -> None:
        """Read the preset file at ``path`` and apply it, so its style, canvas, pens and settings
        replace the current ones. A key the file leaves out keeps its default, and each stage or
        setting the style does not have is dropped with a ``UserWarning``. Raises ``ValueError``
        for a file that is not a preset or that the engine refuses."""
        data = json.loads(Path(path).read_text(encoding="utf-8"))
        for message in self._core.load_preset(data):
            warnings.warn(message, UserWarning, stacklevel=2)

    def run_blocking(self) -> Drawing:
        """Run the style in the calling thread and return the drawing, which is also kept in
        ``result``.

        A run under way on a worker is cancelled and waited for first. Each frame goes through
        ``_deliver`` as it arrives. A failed stage raises ``RuntimeError`` with the engine's
        message and sets ``state`` to ``error``; a cancelled run raises ``Cancelled`` and sets it
        to ``cancelled``. Either way ``result`` stays as it was. Raises ``RuntimeError`` when no
        image is loaded.
        """
        return self._execute(self._start())

    def run(self) -> RunHandle:
        """Run the style on a worker thread and return its handle at once.

        A run under way is cancelled and waited for first. The style, settings, canvas, pens and
        image are taken now, so later changes do not reach the run. The subscribers get its
        frames on the worker thread. Unlike ``run_blocking`` it raises no stage error: the run
        ends in ``error`` and the handle keeps the exception. Raises ``RuntimeError`` when no
        image is loaded.
        """
        handle = self._start()
        thread = threading.Thread(target=self._work, args=(handle,), daemon=True)
        handle._thread = thread
        thread.start()
        return handle

    def cancel(self) -> None:
        """Ask the latest run to stop, which ends it in ``cancelled``. Nothing happens when there
        has been no run, and a run that has ended stays as it ended."""
        if self._handle is not None:
            self._handle.cancel()

    def subscribe(self, callback: Callable[[Frame], None]) -> Callable[[], None]:
        """Call ``callback(frame)`` for the frames of every later run, and return a function that
        removes it again.

        ``max_fps`` caps the rate, see ``_deliver``. The callback runs on the thread that drives
        the run, the worker for ``run`` and the caller for ``run_blocking``, so it should return
        quickly. An exception from it ends the run in ``error``, as a failed stage does.
        """
        key = object()
        self._subscribers[key] = callback

        def unsubscribe() -> None:
            self._subscribers.pop(key, None)

        return unsubscribe

    def _start(self) -> RunHandle:
        """Stop the latest run, wait for it, and start the next one in the engine, which takes a
        snapshot of the session."""
        if self._handle is not None:
            self._handle.cancel()
            self._handle.wait()
        self._handle = handle = RunHandle(self._core.start())
        return handle

    def _work(self, handle: RunHandle) -> None:
        """The body of a worker thread."""
        try:
            self._execute(handle)
        except Exception:
            pass  # the handle keeps it, and a thread has no caller to raise to

    def _execute(self, handle: RunHandle) -> Drawing:
        """Drive the run to its end in the current thread, passing each frame to ``_deliver``,
        and record how it ended on ``handle``. This is the one place that moves a run through its
        states. ``Cancelled`` and any error are raised, with ``result`` as it was."""
        self._last_delivery = -math.inf
        run = handle._run
        try:
            while (frame := run.next_frame()) is not None:
                self._deliver(_frame(frame))
            state = RunState(run.state())
            if state is RunState.done:
                # No newer run has started, so the engine's latest drawing is this run's.
                self.result = result = _drawing(_have(self._core.drawing()))
                handle._end(state, result)
                return result
            if state is RunState.cancelled:
                handle._end(state)
                raise Cancelled("the run was cancelled")
            failure = RuntimeError(run.error())
            handle._end(RunState.error, error=failure)
            raise failure
        except BaseException as error:
            # Anything raised before the handle ended, such as a subscriber's error, ends it here.
            if handle.state is RunState.running:
                handle._end(RunState.error, error=error)
            raise

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

    def svg(self) -> str:
        """The result as SVG text. Raises ``RuntimeError`` when there is no result."""
        return _have(self._core.svg())

    def json_dict(self) -> dict[str, Any]:
        """The result as the dict of the JSON paths format. Raises ``RuntimeError`` when there is
        no result."""
        return _have(self._core.json())

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
        self._core.set_speeds(asdict(self.speeds))
        return Stats(**_have(self._core.stats()))

    def preview_png(
        self,
        width_px: int = 1000,
        travel: bool = False,
        source: bool = False,
        region: tuple[float, float, float, float] | None = None,
    ) -> bytes:
        """The result drawn at true pen width as PNG bytes that are ``width_px`` wide, with the
        pen-up moves when ``travel`` is set and the tone map under the paths when ``source`` is.
        ``region`` is ``(x, y, width, height)`` in page mm, origin top-left; the picture then shows
        only that part, which may reach past the page, and is as high as the region's shape needs.
        Raises ``ValueError`` for a region with a value that is not finite or a width or height
        that is not positive, and ``RuntimeError`` when there is no result."""
        return _have(self._core.preview_png(width_px, travel, source, region))

    def cache_stats(self) -> dict[str, int]:
        """What the stage cache holds and how often it was asked: ``hits``, ``misses``,
        ``bytes`` and ``entries``."""
        return self._core.cache_stats()
