"""The command line interface: list the styles, describe one, analyse an image, run a style on it,
and serve the web interface and the MCP server.

``run`` sets a ``Session`` up from its options, runs the style and writes the outputs. ``analyse``
prints the session's analysis of an image. ``styles`` and ``describe`` read the style registry.
``gui`` serves the web bundle that a release wheel carries. ``mcp`` serves a session to an AI
assistant on stdio. The work all happens in the session layer, so this module holds no algorithm
code.
"""

import http.server
import importlib.util
import json
import sys
import webbrowser
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from dataclasses import replace
from functools import partial
from importlib import resources
from pathlib import Path
from typing import Any

import click

from plotline import inks
from plotline.registry import get_style, list_styles
from plotline.session import Session
from plotline.types import Pen


def coerce(text: str) -> bool | int | float | str:
    """Read a ``--set`` value: ``true`` and ``false`` as booleans, then an integer, then a
    float, and anything else as the text itself. The engine checks the value against the setting."""
    if text in ("true", "false"):
        return text == "true"
    for kind in (int, float):
        try:
            return kind(text)
        except ValueError:
            pass
    return text


def parse_set(value: str) -> tuple[str, str, bool | int | float | str]:
    """Split ``STAGE.KEY=VALUE`` into ``(stage, key, value)``, with the value read by ``coerce``.

    There must be exactly one ``.`` before the ``=`` and exactly one ``=``, and the stage and the
    key must not be empty. Anything else raises ``click.BadParameter``.
    """
    target, equals, text = value.partition("=")
    stage, _, key = target.partition(".")
    if not (equals and stage and key) or "." in key or "=" in text:
        raise click.BadParameter(f"{value!r} must look like STAGE.KEY=VALUE")
    return stage, key, coerce(text)


def parse_pen(value: str) -> Pen:
    """Split ``NAME:WIDTH_MM:COLOR`` into a ``Pen``, as in ``Red 0.5:0.5:#ff0000``.

    Only the last two colons separate the fields, so the name may hold colons and the colour keeps
    its ``#``. The colour may also be the name of one of the ``inks()``, in any case. A wrong
    shape or a width that is not a number raise ``click.BadParameter``; the engine checks the
    width and colour when the session is given the pen.
    """
    parts = value.rsplit(":", 2)
    if len(parts) != 3:
        raise click.BadParameter(f"{value!r} must look like NAME:WIDTH_MM:COLOR")
    name, width, color = parts
    color = inks().get(color.lower(), color)
    try:
        return Pen(name, float(width), color)
    except ValueError as error:
        raise click.BadParameter(f"{value!r}: {error}") from error


@contextmanager
def _errors() -> Iterator[None]:
    """Turn the errors that bad input causes in the session into a one-line CLI error. The engine
    reports a refused value as ``ValueError``, and a file that cannot be read is an
    ``OSError``."""
    try:
        yield
    except (ValueError, OSError) as error:
        raise click.ClickException(str(error)) from error


@click.group()
def main() -> None:
    """Turn images into line art for pen plotters."""


@main.command()
def styles() -> None:
    """List the styles that can be run."""
    for style in list_styles():
        click.echo(f"{style.name}  {style.label}  {style.description}")


@main.command()
@click.argument("style")
@click.option("--json", "as_json", is_flag=True, help="Print the settings as JSON Schema.")
def describe(style: str, as_json: bool) -> None:
    """Show the stages of STYLE and the settings of each."""
    with _errors():
        schema = get_style(style).schema()
    if as_json:
        click.echo(json.dumps(schema, indent=2))
        return
    for i, stage in enumerate(schema["stages"]):
        if i:
            click.echo()
        click.echo(f"{stage['name']} ({stage['label']})")
        # A stage without settings has no properties in its schema.
        for name, prop in stage["schema"].get("properties", {}).items():
            fields = [name, prop.get("type", "any"), f"default {json.dumps(prop.get('default'))}"]
            for word, key in (("min", "minimum"), ("max", "maximum")):
                if key in prop:
                    fields.append(f"{word} {prop[key]}")
            fields.append(prop.get("description", ""))
            click.echo("  " + "  ".join(fields))


def page_options(command: Callable[..., Any]) -> Callable[..., Any]:
    """Give ``command`` the options that set the page, which ``set_page`` then applies."""
    for option in (
        click.option(
            "--fit",
            type=click.Choice(["fit", "fill"]),
            help="Keep the whole image on the page (fit) or fill the drawing area with it (fill).",
        ),
        click.option("--margin", type=float, metavar="MM", help="Margin around the drawing area."),
        click.option("--height", type=float, metavar="MM", help="Page height."),
        click.option("--width", type=float, metavar="MM", help="Page width."),
    ):  # the last one applied is listed first
        command = option(command)
    return command


# The budget metrics, each with its option's help and the unit its result prints with.
FITS = {
    "minutes": ("MINUTES", "the estimated plot time", "min"),
    "paths": ("N", "the path count", "paths"),
    "metres": ("METRES", "the pen-down length", "m"),
    "dots": ("N", "the number of dots, for a style that places points", "dots"),
}


def fit_options(command: Callable[..., Any]) -> Callable[..., Any]:
    """Give ``command`` one ``--fit-METRIC`` option per budget metric, which ``run`` passes to
    ``Session.fit``."""
    for metric, (metavar, what, _) in reversed(FITS.items()):
        command = click.option(
            f"--fit-{metric}",
            type=float,
            metavar=metavar,
            help=(
                f"Fit the style's density setting so that {what} comes near {metavar}, in up to "
                "eight runs, instead of running once."
                + (
                    " The estimate leaves out acceleration, so a style of short strokes or dots "
                    "plots slower than it says."
                    if metric == "minutes"
                    else ""
                )
            ),
        )(command)
    return command


def set_page(
    session: Session,
    width: float | None,
    height: float | None,
    margin: float | None,
    fit: str | None,
) -> None:
    """Change the page of ``session`` by the options that were given; the rest stay as they are."""
    page: dict[str, Any] = {
        "width_mm": width,
        "height_mm": height,
        "margin_mm": margin,
        "fit": fit,
    }
    changes = {key: value for key, value in page.items() if value is not None}
    if changes:
        session.set_canvas(replace(session.canvas, **changes))


@main.command()
@click.argument("image", type=click.Path())
@page_options
def analyse(
    image: str, width: float | None, height: float | None, margin: float | None, fit: str | None
) -> None:
    """Print what IMAGE looks like and the changes that would suit it, as JSON."""
    session = Session()
    with _errors():
        session.load_image(image)
        set_page(session, width, height, margin, fit)
        click.echo(json.dumps(session.analysis(), indent=2))


@main.command()
@click.argument("style")
@click.argument("image", type=click.Path())
@click.option("--out", required=True, type=click.Path(), help="Where to write the SVG.")
@click.option("--json", "json_out", type=click.Path(), help="Also write the paths as JSON.")
@click.option("--preview", type=click.Path(), help="Also write a PNG preview.")
@page_options
@click.option(
    "--pen",
    "pens",
    multiple=True,
    callback=lambda ctx, param, values: [parse_pen(v) for v in values],
    metavar="NAME:WIDTH_MM:COLOR",
    help=(
        "A pen, such as 'Red:0.5:#ff0000' or 'Fine:0.3:blue'. The colour is #rrggbb or one of "
        f"the inks {', '.join(inks())}. "
        "Repeat the option to give more pens. By default only the first pen draws; "
        "set split.mode to share the drawing between them."
    ),
)
@click.option(
    "--set",
    "sets",
    multiple=True,
    callback=lambda ctx, param, values: [parse_set(v) for v in values],
    metavar="STAGE.KEY=VALUE",
    help="Change a setting, such as tone.resolution=500. Repeat for more. See 'describe'.",
)
@click.option("--preset", type=click.Path(), help="Start from a saved preset.")
@click.option(
    "--save-preset",
    type=click.Path(),
    help="Write the style, page, pens and settings used to a preset file.",
)
@fit_options
@click.option("--quiet", is_flag=True, help="Do not print progress.")
def run(
    style: str,
    image: str,
    out: str,
    json_out: str | None,
    preview: str | None,
    width: float | None,
    height: float | None,
    margin: float | None,
    fit: str | None,
    pens: list[Pen],
    sets: list[tuple[str, str, bool | int | float | str]],
    preset: str | None,
    save_preset: str | None,
    fit_minutes: float | None,
    fit_paths: float | None,
    fit_metres: float | None,
    fit_dots: float | None,
    quiet: bool,
) -> None:
    """Run STYLE on IMAGE and write the drawing as an SVG."""
    given = zip(FITS, (fit_minutes, fit_paths, fit_metres, fit_dots), strict=True)
    fits = [(metric, target) for metric, target in given if target is not None]
    if len(fits) > 1:
        raise click.UsageError("give at most one --fit option")
    session = Session(max_fps=4)
    if not quiet:
        session.subscribe(lambda frame: click.echo(f"{frame.stage}: {frame.label}", err=True))
    with _errors():
        session.load_image(image)
        if preset is not None:
            session.load_preset(preset)
        # The preset's changed settings carry into STYLE where it has the same stages.
        session.set_style(style)
        set_page(session, width, height, margin, fit)
        if pens:
            session.set_pens(pens)
        for stage, key, value in sets:
            session.update(stage, **{key: value})
        if fits:
            [(metric, target)] = fits
            handle = session.fit(metric, target)
            handle.wait()
            if handle.error is not None:
                raise handle.error
            chosen = handle.fit
            assert chosen is not None  # a fit that ended without an error is done
            unit = FITS[metric][2]
            click.echo(
                f"fit: {chosen['stage']}.{chosen['key']} = {chosen['value']} "
                f"({chosen['achieved']:.4g} {unit} for a target of {target:g} {unit}, "
                f"{len(chosen['samples'])} trials)"
            )
        else:
            session.run_blocking()
        session.export_svg(out)
        if json_out is not None:
            session.export_json(json_out)
        if preview is not None:
            Path(preview).write_bytes(session.preview_png())
        if save_preset is not None:
            session.save_preset(save_preset)
    stats = session.stats()
    click.echo(
        f"paths: {stats.path_count}  pen down: {stats.pen_down_mm:.1f} mm  "
        f"pen up: {stats.pen_up_mm:.1f} mm  estimated: {stats.estimated_seconds:.1f} s"
    )
    if len(pens) > 1 and session.settings["split"]["mode"] == "one_pen":
        click.echo(
            "Pen use is set to one pen, so only the first pen draws. Add "
            "--set split.mode=light_to_dark or --set split.mode=by_colour to use the others.",
            err=True,
        )


@main.command()
@click.option("--port", type=int, default=8765, show_default=True, help="Port to serve on.")
def gui(port: int) -> None:
    """Serve the web interface on this computer and open it in the browser."""
    web = resources.files("plotline") / "web"
    if not (web / "index.html").is_file():
        message = "This build has no GUI bundle; install a release wheel or build the web app."
        click.echo(message, err=True)
        sys.exit(1)
    handler = partial(http.server.SimpleHTTPRequestHandler, directory=str(web))
    with _errors():  # most often the port is taken
        server = http.server.ThreadingHTTPServer(("127.0.0.1", port), handler)
    with server:
        url = f"http://127.0.0.1:{server.server_address[1]}/"
        click.echo(f"Serving the GUI at {url}; press Ctrl-C to stop.")
        webbrowser.open(url)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass


@main.command()
def mcp() -> None:
    """Serve the session to an AI assistant over MCP on stdio."""
    # Asked first because the server imports other modules of the extra before the SDK; any other
    # missing module is a bug, not a missing extra.
    if importlib.util.find_spec("mcp") is None:
        click.echo('The MCP server needs the mcp extra: pip install "plotline[mcp]"', err=True)
        sys.exit(1)
    # Imported here so the other commands do not load the SDK.
    from plotline.mcp_server import build_server

    build_server().run()
