"""The MCP server: one ``Session`` served to an AI assistant over the Model Context Protocol.

``build_server`` registers the tools, which draw an image with a style, show the result as a
picture and write it out. Every tool calls the session layer and nothing below it, so this module
holds no algorithm code.

``Session`` is not safe for concurrent changes, and the SDK runs a plain ``def`` tool on a worker
thread. Every tool is therefore ``async def``, which keeps each session call on the event loop's
thread. Only the stages of a run leave it, on the session's own worker thread.

Lengths are in mm, the page origin is its top-left corner and y grows down. A path must be
absolute: a stdio server starts in whatever folder its client was in, so a relative path would land
somewhere the user did not mean.
"""

import json
import math
import warnings
from collections import deque
from collections.abc import Callable, Iterator
from contextlib import ExitStack, contextmanager
from dataclasses import asdict, replace
from pathlib import Path
from typing import Any, Literal, NotRequired, cast

import anyio
from mcp.server.mcpserver import Context, Image, MCPServer
from mcp.server.mcpserver.exceptions import ToolError
from mcp.types import TextContent
from typing_extensions import TypedDict  # pydantic refuses typing.TypedDict before Python 3.12

import plotline
from plotline import registry
from plotline.session import RunHandle, Session
from plotline.types import Cancelled, Canvas, Frame, Pen, RunState

INSTRUCTIONS = (
    "plotline turns an image into line art for a pen plotter. Work in this order: load_image, "
    "then set_page and set_pens if the defaults do not fit, then list_styles and describe_style "
    "to choose a style and see its settings and looks. Call analyse_image to see the image's "
    "statistics and suggested changes; apply a suggestion with run(style=..., settings=...) or "
    "set_page, and set_pens first when its patch holds pens. run draws the image and returns its "
    "stats, the tone match, the plot lint's findings and a preview at true pen width; apply a "
    "finding's patch the same way as a suggestion's. Look at the preview, change settings with "
    "run(settings=...), and repeat; only the stages after a changed setting run again. To meet a "
    "budget of plot time, paths, pen-down metres or dots, call fit, which sets the style's "
    "density setting. To make the drawing's tones follow the image more closely, call optimize, "
    "which suggests a density, or set tone.response to auto. Use preview with region_mm to "
    "inspect detail. export writes the result. testchart draws a calibration chart for the "
    "first pen, to plot and compare by eye. set_machine describes a pen plotter or "
    "pen-fitted printer; export with gcode then writes one file per pen. All lengths are "
    "millimetres, the page origin is its top-left corner and y grows down. Paths must be "
    "absolute."
)


PREVIEW_PX = 1000
MIN_PX = 100
MAX_PX = 4000
MAX_PIXELS = 16_000_000  # bounds the memory one preview can ask for, since the input is untrusted


class PenSpec(TypedDict):
    name: str
    width_mm: float
    color: str


class AreaSpec(TypedDict):
    x: float
    y: float
    width: float
    height: float


class MachineSpec(TypedDict):
    name: str
    area: AreaSpec
    z_down_mm: float
    z_up_mm: float
    z_safe_mm: float
    draw_feed: float
    travel_feed: float
    z_feed: float
    settle_ms: int
    start_gcode: str
    end_gcode: str
    pen_lift: NotRequired[Literal["z", "gcode"]]
    pen_down_gcode: NotRequired[str]
    pen_up_gcode: NotRequired[str]
    max_segment_mm: NotRequired[float | None]


@contextmanager
def _errors() -> Iterator[None]:
    """Turn the errors that bad input causes into a ``ToolError`` with the original message, which
    the assistant sees as a failed call it can correct. Any other exception is a bug."""
    try:
        yield
    except (ValueError, OSError, RuntimeError, Cancelled) as error:
        raise ToolError(str(error)) from error


def _absolute(path: str) -> Path:
    """``path`` with ``~`` expanded. Raises ``ValueError`` when the result is not absolute."""
    expanded = Path(path).expanduser()
    if not expanded.is_absolute():
        raise ValueError(f"{path!r} is not an absolute path")
    return expanded


def _array(items: list[Any]) -> TextContent:
    # Returning the list itself would send one block per item; one JSON array reads better.
    return TextContent(type="text", text=json.dumps(items, indent=2))


def _image_size(session: Session) -> dict[str, int] | None:
    # shortcut: session.image copies the pixels, only the shape is used; upgrade when a call gets
    # slow on a huge image, with a size accessor in the engine.
    image = session.image
    return None if image is None else {"width_px": image.shape[1], "height_px": image.shape[0]}


def _page_dict(canvas: Canvas) -> dict[str, Any]:
    """The page as the tools show it. A crop of zeros is left out, as the engine leaves it out of
    a preset."""
    page = asdict(canvas)
    if not any(canvas.crop):
        del page["crop"]
    return page


def _page(session: Session) -> Canvas:
    """The result's page: the picture shows the result, and the page may have changed since the
    run. Raises ``RuntimeError`` while a run is under way, since the engine holds the new drawing
    a moment before ``session.result`` does, and the picture would then be checked against the
    wrong page."""
    if session.state is RunState.running:
        raise RuntimeError("a run is under way; wait for the run to finish, then try again")
    return session.canvas if session.result is None else session.result.canvas


def _pixels(width_px: int, width: float, height: float) -> float:
    """The pixels of a picture ``width_px`` wide of an area ``width`` by ``height`` mm, or ``inf``
    when its height in pixels is too large for a float. The height rounds half away from zero, as
    the engine's does."""
    rows = height * (width_px / width)
    return width_px * math.floor(rows + 0.5) if math.isfinite(rows) else math.inf


def _widest(width: float, height: float, limit: int) -> int:
    """The largest ``width_px`` up to ``limit`` whose picture of an area ``width`` by ``height`` mm
    has at most ``MAX_PIXELS``. Rounding the height can push the square-root bound a few pixels
    over."""
    most = min(limit, int(math.sqrt(MAX_PIXELS * (width / height))))
    while _pixels(most, width, height) > MAX_PIXELS:
        most -= 1
    return most


def _page_px(session: Session) -> int:
    """The width of the previews that run returns and export writes: the largest up to
    ``PREVIEW_PX`` whose picture of the whole page stays within ``MAX_PIXELS``."""
    page = _page(session)
    return _widest(page.width_mm, page.height_mm, PREVIEW_PX)


def build_server(session: Session | None = None) -> MCPServer:
    """The server, with its tools working on ``session``, or on a new one when none is given."""
    session = Session(max_fps=10) if session is None else session
    server = MCPServer("plotline", version=plotline.__version__, instructions=INSTRUCTIONS)

    @server.tool()
    async def list_styles() -> TextContent:
        """List the styles that can be run, each with a name, a label and a description."""
        return _array([asdict(style) for style in registry.list_styles()])

    @server.tool()
    async def describe_style(style: str) -> dict[str, Any]:
        """Describe a style: its stages, and for each setting the type, range, default and help
        text. Change a setting by passing it to run."""
        with _errors():
            return registry.get_style(style).schema()

    @server.tool()
    async def load_image(path: str) -> dict[str, Any]:
        """Load the PNG, JPEG or WebP file at the absolute path as the source image."""
        with _errors():
            p = _absolute(path)
            session.load_image(p)
            return {"path": str(p), **(_image_size(session) or {})}

    @server.tool()
    async def analyse_image() -> dict[str, Any]:
        """Measure the loaded image and suggest changes for it. stats are numbers about its tones,
        noise and colour; for a colour image, palette lists the inks that draw it best. Each
        suggestion has a kind, a reason and a patch, which holds any of style, pens (a list of
        {name, width_mm, color}), settings ({stage: {setting: value}}) and page ({width_mm,
        height_mm}, or {crop}, the shares to cut from the image's left, top, right and bottom),
        or none for a note. Apply a patch with set_pens for its pens, then run(style=...,
        settings=...) or set_page. The suggestions follow the current style, pens, settings and
        page."""
        with _errors():
            return session.analysis()

    @server.tool()
    async def set_page(
        width_mm: float | None = None,
        height_mm: float | None = None,
        margin_mm: float | None = None,
        fit: Literal["fit", "fill"] | None = None,
        paper: str | None = None,
        crop: list[float] | None = None,
    ) -> dict[str, Any]:
        """Set the page size and margin in mm, whether the image fits inside the drawing area or
        fills it, the paper colour, which is #rrggbb or an ink name such as blue, and the crop:
        [left, top, right, bottom], the shares of the image to cut from each side before it is
        fitted, each 0 to 0.45 (analyse_image's framing suggestion gives one; [0, 0, 0, 0] removes
        it). The paper changes only the previews and the saved preset: they show the ink on that
        paper, and the drawing, its SVG and its G-code stay as they are. A value that is left out
        stays as it was; when any value is refused, nothing changes. Returns the page, with its
        crop while there is one, and the paper."""
        given: dict[str, Any] = {
            "width_mm": width_mm,
            "height_mm": height_mm,
            "margin_mm": margin_mm,
            "fit": fit,
            "crop": crop,
        }
        inks = plotline.inks()
        with _errors():
            changes = {key: value for key, value in given.items() if value is not None}
            old_page = session.canvas
            session.set_canvas(replace(old_page, **changes))
            if paper is not None:
                try:
                    session.set_paper(inks.get(paper.lower(), paper))
                except ValueError:
                    session.set_canvas(old_page)
                    raise
            return {**_page_dict(session.canvas), "paper": session.paper}

    @server.tool()
    async def set_pens(pens: list[PenSpec]) -> TextContent:
        """Set the pens, in drawing order. A pen has a name, a line width in mm and a colour, which
        is #rrggbb or an ink name such as blue."""
        inks = plotline.inks()
        with _errors():
            session.set_pens(
                [
                    Pen(p["name"], p["width_mm"], inks.get(p["color"].lower(), p["color"]))
                    for p in pens
                ]
            )
            return _array([asdict(pen) for pen in session.pens])

    @server.tool()
    async def set_machine(machine: MachineSpec | None) -> dict[str, Any]:
        """Set the machine that G-code is written for, a pen plotter or a printer fitted with a
        pen, or clear it with null. area is the part of the bed the pen may reach, {x, y, width,
        height} in mm. z_down_mm is the height of the pen when it draws, z_up_mm its height when
        it travels between paths and z_safe_mm the height it lifts to before the first move and
        before end_gcode, all in mm. draw_feed, travel_feed and z_feed are the speeds for drawing,
        travelling and Z moves, in mm per minute. settle_ms is the pause after each pen move.
        start_gcode and end_gcode are lines written as they are after the setup and after the last
        lift. For a pen lifted by a servo or a solenoid, set pen_lift to "gcode" and give
        pen_down_gcode and pen_up_gcode, the lines written as they are in place of each Z move;
        the Z heights and z_feed are then unused. max_segment_mm, for a machine whose firmware
        draws a long straight move as a curve (a hanging plotter, say), cuts every move into
        pieces no longer than it; leave it out to keep moves whole. The page becomes the area's
        width and height, with the margin and fit kept, so that the drawing fits the machine;
        set_page can change it afterwards. The machine also sets the plot speeds that the stats
        and the lint use. export with format gcode needs a machine, and a result drawn before the
        page changed must run again first. When any part is refused, nothing changes. Returns the
        machine and the page."""
        with _errors():
            old_machine, old_speeds = session.machine, session.speeds
            try:
                session.set_machine(cast("dict[str, Any] | None", machine))
                if machine is not None:
                    area = machine["area"]
                    page = replace(session.canvas, width_mm=area["width"], height_mm=area["height"])
                    session.set_canvas(page)
            except Exception:
                # A refused machine or page changes nothing in the engine; only the machine, which
                # went in first, and the speeds that came with it need to be put back.
                session.set_machine(old_machine)
                session.speeds = old_speeds
                raise
            return {"machine": session.machine, "page": _page_dict(session.canvas)}

    def change(
        style: str | None,
        settings: dict[str, dict[str, Any]] | None,
        reset: bool = False,
        look: str | None = None,
    ) -> None:
        """Switch to ``style``, reset the settings when ``reset`` is set, start from ``look``, and
        apply ``settings``; when a change is refused, put the style and settings back and
        raise."""
        old_style, old_settings = session.style.name, session.settings
        try:
            if style is not None and style != old_style:
                session.set_style(style)
            if reset:
                session.reset_settings()
            if look is not None:
                session.apply_look(look)
            for stage, values in (settings or {}).items():
                session.update(stage, **values)
        except Exception:
            session.set_style(old_style)
            for stage, values in old_settings.items():
                session.update(stage, **values)
            raise

    async def follow(ctx: Context, start: Callable[[], RunHandle]) -> RunHandle:
        """Start a run with ``start``, report its frames as progress at most every 100 ms, and
        wait for it to end. Raises the run's error, or ``Cancelled`` when it was cancelled or a
        newer run took the drawing. A client cancel cancels the run."""
        with ExitStack() as stack:
            latest: deque[Frame] = deque(maxlen=1)
            stack.callback(session.subscribe(latest.append))
            handle = start()
            stack.callback(handle.cancel)  # a run that has ended stays as it ended
            sent: Frame | None = None
            count = 0
            while handle.state is RunState.running:
                await anyio.sleep(0.1)
                if latest and latest[-1] is not sent:
                    sent = latest[-1]
                    count += 1
                    await ctx.report_progress(count, None, f"{sent.stage}: {sent.label}")
            if handle.state is not RunState.done:
                raise handle.error or Cancelled("the run was cancelled")
            # A newer run that started while this call slept owns the engine's drawing now.
            if session.state is RunState.running or session.result is not handle.result:
                raise Cancelled("the run was cancelled")
            return handle

    def shown() -> list[str | Image]:
        """The result's stats, with the tone match under tone and the plot lint's findings under
        lint, as JSON and a preview of it."""
        png = session.preview_png(_page_px(session))
        result = {**asdict(session.stats()), "tone": session.tone_match(), "lint": session.lint()}
        return [json.dumps(result), Image(data=png, format="png")]

    @server.tool()
    async def run(
        ctx: Context,
        style: str | None = None,
        settings: dict[str, dict[str, Any]] | None = None,
        reset: bool = False,
        look: str | None = None,
    ) -> list[str | Image]:
        """Draw the image and return the stats as JSON and a preview at true pen width, up to 1000
        px wide. The JSON's tone key is {error, ssim, floor, reach}: how the drawing's tones,
        seen through a 1.5 mm eye filter, follow the tone map, as the RMS error in tone-map ink
        once the coverage is put on the tone map's scale (lower is closer) and the structural
        similarity (1 is identical), or null when there is nothing to compare. Its lint key lists
        physical problems with the drawing as {code, severity, message, value, patch}; a patch
        applies like an analyse_image suggestion's. A style that
        differs from the current one replaces it; a stage the two styles share keeps the values
        changed from the old style's defaults, and the rest take the new style's defaults. reset
        puts every setting back to the style's defaults first. look starts from one of the style's
        looks, such as faithful, quick or fine, after reset and before settings; describe_style
        lists them. settings is {stage: {setting: value}} and changes only the values given; only
        the stages after a changed setting run again. An assistant that sees a face or a subject
        can keep detail there and abstract the rest with {"tone": {"focus": [[x, y]]}}, each point
        a share of the image's width and height from its top-left corner. When a change is
        refused, nothing changes and nothing runs."""
        with _errors():
            change(style, settings, reset, look)
            await follow(ctx, session.run)
            return shown()

    @server.tool()
    async def testchart() -> list[str | Image]:
        """Draw a test chart for the page and the first pen and return the stats as JSON, with
        the lint, and a preview, as run does. It needs no image and replaces the result. Plot it
        and compare by eye. SPACING MM shows lines at 0.5 to 4 pen widths apart, to see where
        they merge. DOT MM shows grids of dots 1 to 6 pen widths wide, to see how big a dot
        really comes out. PASSES draws a solid fill 1 to 8 times over, to see how many passes the
        paper takes; the lint reports overdraw on it by design. The stats' plot time is the
        estimate to check against a stopwatch. Fails when the page is too small."""
        with _errors():
            session.testchart()
            return shown()

    @server.tool()
    async def fit(
        ctx: Context,
        metric: Literal["minutes", "paths", "metres", "dots"],
        target: float,
        style: str | None = None,
        settings: dict[str, dict[str, Any]] | None = None,
        look: str | None = None,
    ) -> list[str | Image]:
        """Find the value of the style's density setting that brings a metric of the drawing near
        target: minutes of estimated plot time, paths, metres of pen-down length, or dots for a
        style that places points. style, look and settings apply first, as in run. The style reruns
        with the setting changed, up to eight times, and keeps the run nearest the target; the
        setting keeps its value. Returns the stage, key, chosen value and the value and metric of
        every trial as JSON, then the stats and a preview as run does. The time estimate leaves
        out acceleration, so a time target runs optimistic on styles of short strokes or dots."""
        with _errors():
            change(style, settings, look=look)
            handle = await follow(ctx, lambda: session.fit(metric, target))
            return [json.dumps(handle.fit), *shown()]

    @server.tool()
    async def optimize(
        ctx: Context,
        metric: Literal["minutes", "paths", "metres", "dots"] | None = None,
        target: float | None = None,
        style: str | None = None,
        settings: dict[str, dict[str, Any]] | None = None,
        look: str | None = None,
    ) -> list[str | Image]:
        """Search the style's density setting for the value whose drawing's tones follow the
        image most closely, by the tone match's error, in up to nine runs: the current value, then
        half-octave steps from a quarter to four times it. A trial does not count when it plots for
        over twice the current value's time or an hour, whichever is more, or has a plot lint
        warning or error the current value does not. Given metric and target, as for fit, a trial
        over the target does not count either, and a minutes target replaces the time limit.
        style, look and settings apply first, as in run.
        Nothing else changes: returns {stage, key, value, error, minutes, current, current_error,
        current_minutes, budget, trials, reason, patch} as JSON, with plot times in minutes, or
        null when no trial stayed within the target, then the
        stats and a preview of the current settings as run does. Apply the patch with
        run(settings=patch["settings"]) if it suits; fidelity to the image is not always the
        aim."""
        with _errors():
            if (metric is None) != (target is None):
                raise ValueError("give both a metric and a target, or neither")
            change(style, settings, look=look)
            handle = await follow(ctx, lambda: session.optimize(metric, target))
            return [json.dumps(handle.optimum), *shown()]

    @server.tool()
    async def preview(
        width_px: int = PREVIEW_PX,
        travel: bool = False,
        source: bool = False,
        region_mm: list[float] | None = None,
    ) -> Image:
        """Draw the result at true pen width as a PNG width_px wide, from 100 to 4000. travel adds
        the pen-up moves and source lays the tone map under the paths. region_mm is [x, y, width,
        height] in mm to show only that part of the page, which may reach past it. A picture has
        at most 16 million pixels."""
        with _errors():
            if not MIN_PX <= width_px <= MAX_PX:
                raise ValueError(f"width_px must be from {MIN_PX} to {MAX_PX}, got {width_px}")
            page = _page(session)
            area = [page.width_mm, page.height_mm] if region_mm is None else region_mm[2:]
            # Any other area is refused by the engine, which names the bad value.
            if len(area) == 2 and all(math.isfinite(v) and v > 0 for v in area):
                width, height = area
                if (n := _pixels(width_px, width, height)) > MAX_PIXELS:
                    raise ValueError(
                        f"a picture {width_px} px wide would have {n} pixels, over the limit of "
                        f"{MAX_PIXELS}; use width_px {_widest(width, height, width_px)} or less"
                    )
            # The engine refuses a region that is not four numbers.
            region = cast(Any, None if region_mm is None else tuple(region_mm))
            return Image(data=session.preview_png(width_px, travel, source, region), format="png")

    @server.tool()
    async def get_session() -> dict[str, Any]:
        """Show the style, its settings, the look they are at (null for none), the page in mm, the
        paper colour, the pens, the image size, the state of the latest run and the stats of the
        result, where there is one."""
        with _errors():
            stats = asdict(session.stats()) if session.result is not None else None
            return {
                "style": session.style.name,
                "settings": session.settings,
                "look": session.look,
                "page": _page_dict(session.canvas),
                "paper": session.paper,
                "pens": [asdict(pen) for pen in session.pens],
                "machine": session.machine,
                "image": _image_size(session),
                "state": session.state.value,
                "stats": stats,
            }

    @server.tool()
    async def load_preset(path: str) -> dict[str, Any]:
        """Load the preset at the absolute path, which replaces the style, page, paper, pens and
        settings. The file is a saved preset, an exported SVG or an exported JSON drawing. Settings
        that the style does not have are dropped and named in the warnings."""
        with _errors(), warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            session.load_preset(_absolute(path))
        return {"style": session.style.name, "warnings": [str(w.message) for w in caught]}

    @server.tool()
    async def export(
        path: str,
        format: Literal["svg", "json", "png", "preset", "gcode", "hpgl"],
        overwrite: bool = False,
        units_per_mm: float = 40.0,
    ) -> dict[str, Any]:
        """Write the result as svg, json (the paths), png (a preview) or hpgl (one plotter file,
        with a pen selected for each layer), or the settings as a preset, to the absolute path.
        units_per_mm is the plotter units to the millimetre of hpgl, above 0 and at most 1000,
        40 for most plotters, and no other format reads it. A file that exists is kept unless
        overwrite is true. With format gcode the path is a folder, made if it is missing, and the
        result is written into it as one file per pen, plot_1_<pen>.gcode and so on, for the
        machine from set_machine; if any of those files exists, none is written unless overwrite
        is true. Returns the path and size of each file for gcode, and the size of the file for
        the other formats."""
        with _errors():
            p = _absolute(path)
            if format == "gcode":
                files = session.export_gcode(p, overwrite=overwrite)
                sizes = [{"path": str(f), "bytes": f.stat().st_size} for f in files]
                return {"path": str(p), "format": format, "files": sizes}
            if p.exists() and not overwrite:
                raise ValueError(f"{p} exists; pass overwrite=true to replace it")
            if format == "svg":
                session.export_svg(p)
            elif format == "json":
                session.export_json(p)
            elif format == "hpgl":
                session.export_hpgl(p, units_per_mm)
            elif format == "png":
                p.write_bytes(session.preview_png(_page_px(session)))
            else:
                session.save_preset(p)
            return {"path": str(p), "format": format, "bytes": p.stat().st_size}

    return server
