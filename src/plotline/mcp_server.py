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
import warnings
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import asdict, replace
from pathlib import Path
from typing import Any, Literal

from mcp.server.mcpserver import MCPServer
from mcp.server.mcpserver.exceptions import ToolError
from mcp.types import TextContent
from typing_extensions import TypedDict  # pydantic refuses typing.TypedDict before Python 3.12

import plotline
from plotline import registry
from plotline.session import Session
from plotline.types import Cancelled, Pen

INSTRUCTIONS = (
    "plotline turns an image into line art for a pen plotter. Work in this order: load_image, "
    "then set_page and set_pens if the defaults do not fit, then list_styles and describe_style "
    "to choose a style and see its settings. run draws the image and returns its stats and a "
    "preview at true pen width. Look at the preview, change settings with run(settings=...), and "
    "repeat; only the stages after a changed setting run again. Use preview with region_mm to "
    "inspect detail. export writes the result. All lengths are millimetres, the page origin is "
    "its top-left corner and y grows down. Paths must be absolute."
)


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
                p.write_bytes(session.preview_png())
            else:
                session.save_preset(p)
            return {"path": str(p), "format": format, "bytes": p.stat().st_size}

    return server
