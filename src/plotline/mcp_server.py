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
from collections.abc import Iterator
from contextlib import ExitStack, contextmanager
from dataclasses import asdict, replace
from pathlib import Path
from typing import Any, Literal, cast

import anyio
from mcp.server.mcpserver import Context, Image, MCPServer
from mcp.server.mcpserver.exceptions import ToolError
from mcp.types import TextContent
from typing_extensions import TypedDict  # pydantic refuses typing.TypedDict before Python 3.12

import plotline
from plotline import registry
from plotline.session import Session
from plotline.types import Cancelled, Canvas, Frame, Pen, RunState

INSTRUCTIONS = (
    "plotline turns an image into line art for a pen plotter. Work in this order: load_image, "
    "then set_page and set_pens if the defaults do not fit, then list_styles and describe_style "
    "to choose a style and see its settings. Call analyse_image to see the image's statistics and "
    "suggested changes; apply a suggestion with run(style=..., settings=...) or set_page. run "
    "draws the image and returns its stats, the plot lint's findings and a preview at true pen "
    "width; apply a finding's patch the same way as a suggestion's. Look at the preview, "
    "change settings with run(settings=...), and repeat; only the stages after a changed setting "
    "run again. Use preview with region_mm to inspect detail. export writes the result. All "
    "lengths are millimetres, the page origin is its top-left corner and y grows down. Paths must "
    "be absolute."
)


PREVIEW_PX = 1000
MIN_PX = 100
MAX_PX = 4000
MAX_PIXELS = 16_000_000  # bounds the memory one preview can ask for, since the input is untrusted


class PenSpec(TypedDict):
    name: str
    width_mm: float
    color: str


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
        noise and colour. Each suggestion has a kind, a reason and a patch, which holds any of
        style, settings ({stage: {setting: value}}) and page ({width_mm, height_mm}), or none for a
        note. Apply a patch with run(style=..., settings=...) or set_page. The suggestions follow
        the current style, settings and page."""
        with _errors():
            return session.analysis()

    @server.tool()
    async def set_page(
        width_mm: float | None = None,
        height_mm: float | None = None,
        margin_mm: float | None = None,
        fit: Literal["fit", "fill"] | None = None,
    ) -> dict[str, Any]:
        """Set the page size and margin in mm, and whether the image fits inside the drawing area
        or fills it. A value that is left out stays as it was."""
        given: dict[str, Any] = {
            "width_mm": width_mm,
            "height_mm": height_mm,
            "margin_mm": margin_mm,
            "fit": fit,
        }
        with _errors():
            changes = {key: value for key, value in given.items() if value is not None}
            session.set_canvas(replace(session.canvas, **changes))
            return asdict(session.canvas)

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
    async def run(
        ctx: Context,
        style: str | None = None,
        settings: dict[str, dict[str, Any]] | None = None,
        reset: bool = False,
    ) -> list[str | Image]:
        """Draw the image and return the stats as JSON and a preview at true pen width, up to 1000
        px wide. The JSON's lint key lists physical problems with the drawing as {code, severity,
        message, value, patch}; a patch applies like an analyse_image suggestion's. A style that
        differs from the current one replaces it; a stage the two styles share keeps the values
        changed from the old style's defaults, and the rest take the new style's defaults. reset
        puts every setting back to the style's defaults first. settings is {stage: {setting:
        value}} and changes only the values given; only the stages after a changed setting run
        again. When a change is refused, nothing changes and nothing runs."""
        with _errors(), ExitStack() as stack:
            old_style, old_settings = session.style.name, session.settings
            try:
                if style is not None and style != old_style:
                    session.set_style(style)
                if reset:
                    session.reset_settings()
                for stage, values in (settings or {}).items():
                    session.update(stage, **values)
            except Exception:
                session.set_style(old_style)
                for stage, values in old_settings.items():
                    session.update(stage, **values)
                raise

            latest: deque[Frame] = deque(maxlen=1)
            stack.callback(session.subscribe(latest.append))
            handle = session.run()
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
            png = session.preview_png(_page_px(session))
            result = {**asdict(session.stats()), "lint": session.lint()}
            return [json.dumps(result), Image(data=png, format="png")]

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
        """Show the style, its settings, the page in mm, the pens, the image size, the state of the
        latest run and the stats of the result, where there is one."""
        with _errors():
            stats = asdict(session.stats()) if session.result is not None else None
            return {
                "style": session.style.name,
                "settings": session.settings,
                "page": asdict(session.canvas),
                "pens": [asdict(pen) for pen in session.pens],
                "image": _image_size(session),
                "state": session.state.value,
                "stats": stats,
            }

    @server.tool()
    async def load_preset(path: str) -> dict[str, Any]:
        """Load the preset file at the absolute path, which replaces the style, page, pens and
        settings. Settings that the style does not have are dropped and named in the warnings."""
        with _errors(), warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always")
            session.load_preset(_absolute(path))
        return {"style": session.style.name, "warnings": [str(w.message) for w in caught]}

    @server.tool()
    async def export(
        path: str, format: Literal["svg", "json", "png", "preset"], overwrite: bool = False
    ) -> dict[str, Any]:
        """Write the result as svg, json (the paths) or png (a preview), or the settings as a
        preset, to the absolute path. A file that exists is kept unless overwrite is true."""
        with _errors():
            p = _absolute(path)
            if p.exists() and not overwrite:
                raise ValueError(f"{p} exists; pass overwrite=true to replace it")
            if format == "svg":
                session.export_svg(p)
            elif format == "json":
                session.export_json(p)
            elif format == "png":
                p.write_bytes(session.preview_png(_page_px(session)))
            else:
                session.save_preset(p)
            return {"path": str(p), "format": format, "bytes": p.stat().st_size}

    return server
