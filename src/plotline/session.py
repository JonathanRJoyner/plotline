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
import re
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
    """One run of a style, as ``Session.run``, ``Session.run_blocking``, ``Session.fit`` or
    ``Session.optimize`` makes it.

    ``state`` starts as ``running`` and ends as ``done``, ``cancelled`` or ``error``. ``result``
    is set when the run is ``done`` and ``error`` holds the exception when it ended in ``error``:
    a ``RuntimeError`` with the engine's message for a failed stage, or what a subscriber raised.
    ``fit`` holds what a fit chose once it is ``done``, and ``optimum`` what a search found; both
    are ``None`` for a plain run. All of them can be read from any thread. A handle from
    ``run_blocking`` has no thread, so it is finished by the time the caller has it.
    """

    def __init__(self, run: _core.Run, applies: bool = False) -> None:
        self._run = run
        # A fit's value goes into the settings when it ends; a search's suggestion never does.
        self._applies = applies
        self._thread: threading.Thread | None = None
        self._state = RunState.running
        self._result: Drawing | None = None
        self._error: BaseException | None = None
        self._fit: dict[str, Any] | None = None
        self._optimum: dict[str, Any] | None = None

    @property
    def state(self) -> RunState:
        return self._state

    @property
    def result(self) -> Drawing | None:
        return self._result

    @property
    def error(self) -> BaseException | None:
        return self._error

    @property
    def fit(self) -> dict[str, Any] | None:
        """``{stage, key, metric, target, value, achieved, samples}``, where ``samples`` lists
        every trial as ``{value, metric}`` in order."""
        return self._fit

    @property
    def optimum(self) -> dict[str, Any] | None:
        """``{stage, key, value, error, minutes, current, current_error, current_minutes, budget,
        limit_minutes, beyond, trials, reason, patch}``: the best value found with its tone error
        and plot time, the current value with its own, the time limit in minutes (``None`` when a
        minutes target replaced it), the trial left out for its time that follows the tones more
        than a tenth more closely, if any, ``trials`` as ``{value, error, minutes, metric, over,
        slow, warnings, failed}`` in order, a sentence, and ``patch``, which
        ``{"settings": {stage: {key: value}}}`` applies, or ``None`` when the current value is
        best. Nothing is applied."""
        return self._optimum

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
    """One working context: an image, a canvas, pens, a style with its settings, plot speeds, an
    optional machine, the stage cache and the latest result.

    The attributes can be read at any time; ``image``, ``canvas``, ``pens``, ``style``,
    ``settings`` and ``machine`` are copies of what the engine holds. Change them through the
    ``set_*`` methods and ``update``, which the engine checks: a refused value raises
    ``ValueError`` and changes nothing. ``cache_bytes`` bounds the stage cache and ``max_fps``
    caps how often frames are delivered to the subscribers. ``run`` works on a worker thread and
    ``run_blocking`` in the caller's. Starting either one stops the run before it, so at most one
    is active. A session is not safe for concurrent changes, so a front end should call it from one
    thread. ``speeds``, which ``stats`` reads, has no setter and is edited in place, but
    ``set_machine`` and ``load_preset`` replace it with a machine's.
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
    def look(self) -> str | None:
        """The name of the style's look that the settings are at, such as ``"balanced"``, or
        ``None`` when they are at none, as after an edit of a setting that the looks set. It is
        worked out from the settings on each call, so a reset, a preset or a style switch show
        the right one. ``style.schema()["looks"]`` lists them."""
        return self._core.look()

    @property
    def state(self) -> RunState:
        """The state of the latest run, ``done`` after a test chart, or ``idle`` before any run."""
        # With no handle the engine's slot is all there is: idle, or the chart's done.
        return RunState(self._core.state()) if self._handle is None else self._handle.state

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

    def analysis(self) -> dict[str, Any]:
        """What the image looks like and what would suit it, as ``{"stats", "suggestions"}``.
        ``stats`` holds numbers measured when the image was set, among them ``palette`` (the
        ``{name, color}`` inks that draw a colour image best, lightest first, empty for a grey
        one, and taken from the pen library when one is set with :meth:`set_pen_library`),
        ``palette_error`` (its mean colour difference) and ``palette_additions`` (``{name,
        color, error}`` inks the library lacks that would lower it, which a ``pens_to_add``
        suggestion names). Each suggestion is
        ``{"kind", "reason", "patch"}``, where ``patch`` is ``None`` or any of ``style``, ``pens``
        (a list of ``{name, width_mm, color}``, for :meth:`set_pens`), ``settings``
        (``{stage: {key: value}}``) and ``page`` (``{width_mm, height_mm}`` or ``{crop}``, the
        shares to cut from the image's left, top, right and bottom), which the caller applies in
        that order; the list follows the current style, pens, settings and page. Raises
        ``RuntimeError`` when no image is loaded."""
        analysis = self._core.analysis()
        if analysis is None:
            raise RuntimeError("no image is loaded")
        return analysis

    def set_canvas(self, canvas: Canvas) -> None:
        """Use ``canvas`` as the page. Raises ``ValueError`` for a page or margin that leaves no
        drawing area and for a crop share that is not from 0 to 0.45."""
        self._core.set_canvas(asdict(canvas))

    @property
    def paper(self) -> str:
        """The colour of the paper, as ``#rrggbb``; ``#ffffff`` until it is set."""
        return self._core.paper()

    def set_paper(self, color: str) -> None:
        """Use ``color``, ``#rrggbb``, as the paper. Only the previews and the preset read it: the
        previews fill the page with it and multiply the ink onto it, and the preset keeps it. No
        stage reruns, and the SVG, JSON paths and G-code leave it out. Raises ``ValueError``
        for anything but ``#rrggbb``, and changes nothing."""
        self._core.set_paper(color)

    def set_pens(self, pens: Sequence[Pen]) -> None:
        """Use ``pens``, in drawing order. Raises ``ValueError`` when there are none, because
        every result needs a pen, and for a pen the engine refuses."""
        self._core.set_pens([asdict(pen) for pen in pens])

    def set_pen_library(self, pens: Sequence[Pen]) -> None:
        """Use ``pens`` as the user's own pens. With any, :meth:`analysis` picks a colour image's
        palette from their colours and lists inks worth adding; an empty list clears them. They
        never change a drawing or a preset. Raises ``ValueError`` for a pen the engine refuses."""
        self._core.set_pen_library([asdict(pen) for pen in pens])

    @property
    def machine(self) -> dict[str, Any] | None:
        """The machine that ``gcode`` writes for, as the dict of a preset's ``machine`` (``name``,
        ``area``, the heights and feeds, ``start_gcode`` and ``end_gcode``, and when used
        ``pen_lift``, ``pen_down_gcode``, ``pen_up_gcode``, ``max_segment_mm`` and
        ``accel_mm_s2``), or ``None``."""
        return self._core.machine()

    def set_machine(self, machine: dict[str, Any] | None) -> None:
        """Use ``machine``, a dict in the shape of a preset's ``machine``, or ``None`` for no
        machine. A machine also sets ``speeds`` to the ones it implies; ``None`` leaves them as
        they are. Raises ``ValueError`` for a machine the engine refuses, with a message that
        names the field, and changes nothing."""
        self._core.set_machine(machine)
        if machine is not None:
            self.speeds = Speeds(**self._core.speeds())

    def set_style(self, name: str) -> None:
        """Switch to the built-in style called ``name``. A stage that the old style also has
        keeps each value changed from the old style's default; every other value takes the new
        style's default. The cache is kept, so a shared stage with the same settings and inputs,
        such as the Voronoi points, is not run again. Raises ``ValueError`` for an unknown
        name."""
        self._core.set_style(name)

    def reset_settings(self) -> None:
        """Put every stage of the style, Pen use included, back to the style's defaults."""
        self._core.reset_settings()

    def update(self, stage: str, **values: Any) -> None:
        """Change settings of one stage, leaving the rest as they were. The whole stage is
        checked again, so a bad value or an unknown stage raises ``ValueError`` and changes
        nothing."""
        self._core.update(stage, values)

    def apply_look(self, name: str) -> None:
        """Start from the style's look ``name``, a named set of values for the few settings that
        set how the style draws, as ``update`` would set each one. Every other setting stays as
        it is, the image adjustments and Pen use among them. Raises ``ValueError`` for a name
        that the style has no look of, naming the ones it has, and changes nothing."""
        self._core.apply_look(name)

    def save_preset(self, path: str | os.PathLike[str]) -> None:
        """Write the style, canvas, paper, pens and settings to ``path`` as a JSON preset, UTF-8
        with LF line ends and indented by two spaces so that it reads and diffs well."""
        text = json.dumps(self._core.preset(), indent=2) + "\n"
        Path(path).write_text(text, encoding="utf-8", newline="\n")

    def load_preset(self, path: str | os.PathLike[str]) -> None:
        """Read the file at ``path`` and apply the preset it holds, so its style, canvas, paper,
        pens, machine and settings replace the current ones. The file is a preset, an SVG exported
        with its preset embedded or a JSON drawing exported with one. A key the file leaves out
        keeps its default, and each stage or setting the style does not have is dropped with a
        ``UserWarning``. A preset with a machine also sets ``speeds`` to the machine's. Raises
        ``ValueError`` for a file that holds no preset or that the engine refuses."""
        data = _core.preset_from_text(Path(path).read_text(encoding="utf-8"))
        for message in self._core.load_preset(data):
            warnings.warn(message, UserWarning, stacklevel=2)
        if self._core.machine() is not None:
            # The engine took the machine's speeds, and ``stats`` would push the old ones back.
            self.speeds = Speeds(**self._core.speeds())

    def run_blocking(self) -> Drawing:
        """Run the style in the calling thread and return the drawing, which is also kept in
        ``result``.

        A run under way on a worker is cancelled and waited for first. Each frame goes through
        ``_deliver`` as it arrives. A failed stage raises ``RuntimeError`` with the engine's
        message and sets ``state`` to ``error``; a cancelled run raises ``Cancelled`` and sets it
        to ``cancelled``. Either way ``result`` stays as it was. Raises ``RuntimeError`` when no
        image is loaded.
        """
        return self._execute(self._start(self._core.start))

    def run(self) -> RunHandle:
        """Run the style on a worker thread and return its handle at once.

        A run under way is cancelled and waited for first. The style, settings, canvas, pens and
        image are taken now, so later changes do not reach the run. The subscribers get its
        frames on the worker thread. Unlike ``run_blocking`` it raises no stage error: the run
        ends in ``error`` and the handle keeps the exception. Raises ``RuntimeError`` when no
        image is loaded.
        """
        return self._spawn(self._start(self._core.start))

    def fit(self, metric: str, target: float) -> RunHandle:
        """Find the value of the style's density setting that brings ``metric`` of the drawing
        near ``target``, on a worker thread as ``run`` does, and return its handle at once.

        ``metric`` is ``minutes`` (the estimated plot time at ``speeds``), ``paths``, ``metres``
        of pen-down length, or ``dots`` for a style that places points. The style reruns with the
        setting changed, up to eight times, until the metric is within 2 % of the target, and the
        trial nearest it is kept: when the handle is ``done``, the setting holds its value,
        ``result`` its drawing and the handle's ``fit`` the search. Raises ``RuntimeError`` when
        no image is loaded and ``ValueError`` for an unknown metric, a target that is not above 0,
        a style with no density setting, or ``dots`` for a style that places none.
        """
        self._core.set_speeds(asdict(self.speeds))
        return self._spawn(self._start(lambda: self._core.fit(metric, target), applies=True))

    def optimize(self, metric: str | None = None, target: float | None = None) -> RunHandle:
        """Search the style's density setting for the value whose drawing follows the tone map
        most closely for its plot time, on a worker thread as ``run`` does, and return its handle
        at once.

        It runs the style up to nine times: at the current value, then at half-octave steps from
        a quarter to four times it, and suggests the quickest trial whose tone error is within a
        tenth of the lowest. A trial does not count when its run fails, or when its drawing
        plots for over twice the current value's time, or an hour if that is more, or has a plot
        lint warning or error the current value's drawing does not. Given ``metric`` and
        ``target``, as for ``fit``, a trial whose drawing goes over the target does not count
        either, and a ``minutes`` target replaces the time limit. The settings are left alone:
        when the handle is ``done``, ``result`` is the drawing of the current settings and the
        handle's ``optimum`` the suggestion, which ``update`` applies. Raises ``RuntimeError``
        when no image is loaded and ``ValueError`` as ``fit`` does, or for only one of ``metric``
        and ``target``.
        """
        self._core.set_speeds(asdict(self.speeds))
        return self._spawn(self._start(lambda: self._core.optimize(metric, target)))

    def testchart(self) -> Drawing:
        """Make the test chart for the canvas and the first pen and return it; it is also kept in
        ``result``, so the exports, ``stats`` and ``lint`` work on it as on any drawing. The chart
        is a page of small labelled patches: line spacings, dot sizes and repeated passes, all
        scaled to the pen. Plot it to see where lines merge, how big dots come out, how many
        passes the paper takes, and whether ``stats`` gives the plot time. It needs no image. A
        run under way is cancelled and waited for, and ``state`` is ``done``. Raises ``ValueError``
        when the page is too small for the chart, leaving ``result`` and the run as they were.
        """
        self._core.testchart()  # cancels the engine's run, and refuses before it does
        if self._handle is not None:
            self._handle.cancel()
            self._handle.wait()
            self._handle = None
        self.result = result = _drawing(_have(self._core.drawing()))
        return result

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

    def _start(self, begin: Callable[[], _core.Run], applies: bool = False) -> RunHandle:
        """Stop the latest run, wait for it, and start the next one in the engine with ``begin``,
        which takes a snapshot of the session. ``applies`` is for a fit, whose value goes into
        the settings when it ends."""
        if self._handle is not None:
            self._handle.cancel()
            self._handle.wait()
        self._handle = handle = RunHandle(begin(), applies)
        return handle

    def _spawn(self, handle: RunHandle) -> RunHandle:
        """Drive ``handle`` on a new worker thread and return it."""
        thread = threading.Thread(target=self._work, args=(handle,), daemon=True)
        handle._thread = thread
        thread.start()
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
                # Applied before the handle ends, so a reader that sees done sees the new value.
                found = run.result()
                if found is not None and handle._applies:
                    self._core.update(found["stage"], {found["key"]: found["value"]})
                    handle._fit = found
                elif found is not None:
                    handle._optimum = found
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

    def gcode(self, layer: int) -> str:
        """Layer ``layer`` of the result, counted from 0, as G-code text for the machine. Raises
        ``RuntimeError`` when there is no machine or no result, and ``ValueError`` for a layer the
        result lacks or a page larger than the machine's drawable area."""
        return self._core.gcode(layer)

    def export_gcode(self, directory: str | os.PathLike[str], overwrite: bool = True) -> list[Path]:
        """Write one G-code file per layer of the result into ``directory``, which is created with
        its parents if missing, and return the paths in layer order. A file is named
        ``plot_{n}_{pen}.gcode``, where ``n`` counts from 1 and every run of characters outside
        ``[A-Za-z0-9_.-]`` in the pen's name becomes ``_``. The text is UTF-8 with LF line ends.
        With ``overwrite`` false, a file that already exists raises ``FileExistsError`` naming it,
        before anything is written. Raises ``RuntimeError`` when there is no machine or no result
        and ``ValueError`` for a page larger than the machine's drawable area; in both cases
        nothing is written."""
        layers = _have(self.result).layers
        # All the text first, so that a refusal from the engine leaves the directory untouched.
        texts = [self.gcode(i) for i in range(len(layers))]
        files = []
        for n, layer in enumerate(layers, 1):
            pen = re.sub(r"[^\w.-]+", "_", layer.pen.name, flags=re.ASCII)
            files.append(Path(directory) / f"plot_{n}_{pen}.gcode")
        if not overwrite and (clash := next((f for f in files if f.exists()), None)):
            raise FileExistsError(f"{clash} already exists")
        Path(directory).mkdir(parents=True, exist_ok=True)
        for file, text in zip(files, texts, strict=True):
            file.write_text(text, encoding="utf-8", newline="\n")
        return files

    def hpgl(self, units_per_mm: float = 40.0) -> str:
        """The result as one HPGL program, with ``units_per_mm`` plotter units to the millimetre;
        40, HPGL's plotter unit of 0.025 mm, suits most plotters. Each layer that has paths
        selects its own pen, numbered from 1. Raises ``RuntimeError`` when there is no result and
        ``ValueError`` for a resolution that is not above 0 and at most 1000."""
        return self._core.hpgl(units_per_mm)

    def export_hpgl(self, path: str | os.PathLike[str], units_per_mm: float = 40.0) -> None:
        """Write the HPGL program to ``path`` as ASCII with LF line ends. Raises
        ``RuntimeError`` when there is no result and ``ValueError`` for a resolution that is not
        above 0 and at most 1000; in both cases nothing is written."""
        Path(path).write_text(self.hpgl(units_per_mm), encoding="ascii", newline="\n")

    def stats(self) -> Stats:
        """The path count, pen-down and pen-up distances and estimated plot time of the result at
        ``speeds``. Raises ``RuntimeError`` when there is no result."""
        self._core.set_speeds(asdict(self.speeds))
        return Stats(**_have(self._core.stats()))

    def lint(self) -> list[dict[str, Any]]:
        """Physical problems with the result, as ``{"code", "severity", "message", "value",
        "patch"}``: ``severity`` is ``"error"``, ``"warning"`` or ``"info"``, ``value`` the number
        the check tested, and ``patch`` ``None`` or any of ``style``, ``settings`` and ``page``, as
        in :meth:`analysis`. The result was measured when its run ended; the findings follow the
        current style, settings, pens, ``speeds`` and machine. Raises ``RuntimeError`` when there
        is no result."""
        self._core.set_speeds(asdict(self.speeds))
        return _have(self._core.lint())

    def tone_match(self) -> dict[str, float] | None:
        """How the result's tones follow its tone map, seen through a 1.5 mm eye filter, as
        ``{"error", "ssim", "floor", "reach"}``: ``error`` is the RMS difference in tone-map ink
        after the coverage is put on the tone map's scale by the line that fits them best (0 is a
        perfect match), ``ssim`` their structural similarity (1 is identical), and ``floor`` and
        ``reach`` the coverage that line puts at white and at full ink. ``None`` when there is
        nothing to compare, as with an empty drawing. Raises ``RuntimeError`` when there is no
        result."""
        _have(self._core.stats())
        return self._core.tone_match()

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
