"""The command line interface: list the styles, describe one, analyse an image, run a style on it,
keep learned line models, and serve the web interface and the MCP server.

``run`` sets a ``Session`` up from its options, runs the style and writes the outputs. ``testchart``
writes a calibration chart for a pen. ``analyse`` prints the session's analysis of an image, with
the palette picked from a pen library file when one is given.
``styles`` and ``describe`` read the style registry.
``models`` lists, downloads, adds and removes the models the ``learned`` style draws with; a
download shows its licence and asks first. ``gui`` serves the web bundle that a release wheel
carries. ``mcp`` serves a session to an AI assistant on stdio. The work all happens in the session
layer, so this module holds no algorithm code.
"""

import http.server
import importlib.util
import json
import re
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

from plotline import _core, inks, models
from plotline.registry import get_style, list_styles
from plotline.session import Session
from plotline.types import Pen

Setting = bool | int | float | str | list[Any] | dict[str, Any]


def coerce(text: str) -> Setting:
    """Read a ``--set`` value: ``true`` and ``false`` as booleans, text that starts with ``[`` or
    ``{`` as JSON (so ``[[0.5,0.3]]`` is a list of points; bad JSON raises ``ValueError``), then an
    integer, then a float, and anything else as the text itself. The engine checks the value
    against the setting."""
    if text in ("true", "false"):
        return text == "true"
    if text.startswith(("[", "{")):
        return json.loads(text)
    for kind in (int, float):
        try:
            return kind(text)
        except ValueError:
            pass
    return text


def parse_set(value: str) -> tuple[str, str, Setting]:
    """Split ``STAGE.KEY=VALUE`` into ``(stage, key, value)``, with the value read by ``coerce``.

    There must be exactly one ``.`` before the ``=`` and exactly one ``=``, and the stage and the
    key must not be empty. Anything else raises ``click.BadParameter``.
    """
    target, equals, text = value.partition("=")
    stage, _, key = target.partition(".")
    if not (equals and stage and key) or "." in key or "=" in text:
        raise click.BadParameter(f"{value!r} must look like STAGE.KEY=VALUE")
    try:
        return stage, key, coerce(text)
    except ValueError as error:
        raise click.BadParameter(f"{value!r}: {error}") from error


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


# The engine's advice for a model that is missing names the browser's menus before the command, and
# a command line has only the command.
_BROWSER_ADVICE = re.compile(
    r"(?:get (?:it|one) from Styles, then Get models|load it again with Load your own), or run "
)


@contextmanager
def _errors() -> Iterator[None]:
    """Turn the errors that bad input causes in the session into a one-line CLI error. The engine
    reports a refused value as ``ValueError``, and a file that cannot be read is an ``OSError``.
    A failed run, a model that is not loaded and a missing home folder are ``RuntimeError``."""
    try:
        yield
    except (ValueError, OSError, RuntimeError) as error:
        raise click.ClickException(_BROWSER_ADVICE.sub("run ", str(error))) from error


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
    click.echo()
    click.echo("Looks")
    for look in schema["looks"]:
        pairs = " ".join(
            f"{stage}.{key}={_text(value)}"
            for stage, values in look["settings"].items()
            for key, value in values.items()
        )
        fields = [look["name"], look["label"], look["description"], pairs]
        click.echo("  " + "  ".join(f for f in fields if f))


def page_options(command: Callable[..., Any]) -> Callable[..., Any]:
    """Give ``command`` the options that set the page, which ``set_page`` then applies."""
    for option in (
        click.option(
            "--paper",
            metavar="COLOR",
            help=(
                "Paper colour for the preview, #rrggbb or an ink name; the drawing itself is "
                "unchanged."
            ),
        ),
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


def parse_crop(value: str) -> tuple[float, float, float, float]:
    """Split ``LEFT,TOP,RIGHT,BOTTOM`` into four numbers. A wrong shape or a share that is not a
    number raise ``click.BadParameter``; the engine checks each share when the session is given
    the page."""
    try:
        left, top, right, bottom = (float(part) for part in value.split(","))
    except ValueError as error:
        message = f"{value!r} must look like LEFT,TOP,RIGHT,BOTTOM, four numbers"
        raise click.BadParameter(message) from error
    return left, top, right, bottom


crop_option = click.option(
    "--crop",
    metavar="LEFT,TOP,RIGHT,BOTTOM",
    callback=lambda ctx, param, value: None if value is None else parse_crop(value),
    help=(
        "Cut these shares of the image from its left, top, right and bottom, each 0 to 0.45, "
        "before it is fitted to the page; 0,0,0,0 removes a crop."
    ),
)


def set_page(
    session: Session,
    width: float | None,
    height: float | None,
    margin: float | None,
    fit: str | None,
    paper: str | None,
    crop: tuple[float, float, float, float] | None = None,
) -> None:
    """Change the page and the paper colour of ``session`` by the options that were given; the
    rest stay as they are. The paper may be the name of one of the ``inks()``, in any case."""
    page: dict[str, Any] = {
        "width_mm": width,
        "height_mm": height,
        "margin_mm": margin,
        "fit": fit,
        "crop": crop,
    }
    changes = {key: value for key, value in page.items() if value is not None}
    if changes:
        session.set_canvas(replace(session.canvas, **changes))
    if paper is not None:
        session.set_paper(inks().get(paper.lower(), paper))


def read_library(path: str) -> list[Pen]:
    """The pens of the pen library file at ``path``: ``{"format": "plotline-pens", "version": 1,
    "pens": [...]}``, the file the web interface's My pens exports. Raises ``ValueError`` saying
    what is wrong, in the web interface's words, for a file that is not one, and ``OSError`` for a
    file that cannot be read. The engine checks each pen's width and colour."""
    name = Path(path).name
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        raise ValueError(f"{name} is not a pen library: it is not valid JSON.") from None
    file = value if isinstance(value, dict) else {}
    pens = file.get("pens")
    fields = {"name", "width_mm", "color"}
    if file.get("format") != "plotline-pens":
        reason = "not a pen library: its format must be 'plotline-pens'"
    elif file.get("version") != 1:
        reason = f"unsupported pen library version {file.get('version')}"
    elif not isinstance(pens, list):
        reason = "not a pen library: it has no list of pens"
    elif not all(isinstance(p, dict) and fields <= p.keys() for p in pens):
        reason = "not a pen library: every pen needs a name, a width_mm and a color"
    else:
        return [Pen(p["name"], p["width_mm"], p["color"]) for p in pens]
    raise ValueError(f"{name}: {reason}.")


@main.command()
@click.argument("image", type=click.Path())
@page_options
@crop_option
@click.option(
    "--library",
    type=click.Path(),
    help=(
        "A pen library file, as the web interface's My pens exports it. A colour image's "
        "palette is then picked from those pens, and inks worth adding are named."
    ),
)
def analyse(
    image: str,
    width: float | None,
    height: float | None,
    margin: float | None,
    fit: str | None,
    paper: str | None,
    crop: tuple[float, float, float, float] | None,
    library: str | None,
) -> None:
    """Print what IMAGE looks like and the changes that would suit it, as JSON."""
    session = Session()
    with _errors():
        if library is not None:
            session.set_pen_library(read_library(library))
        session.load_image(image)
        set_page(session, width, height, margin, fit, paper, crop)
        click.echo(json.dumps(session.analysis(), indent=2))


pen_option = click.option(
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

machine_option = click.option(
    "--machine",
    type=click.Path(),
    help=(
        "A JSON file with a machine, or a preset that has one, which sets the plot speeds and "
        "makes the page the machine's drawable area. --width and --height still override that."
    ),
)


def read_machine(path: str) -> dict[str, Any]:
    """Read the machine in the JSON file at ``path``, which holds either a machine or a preset
    (``"format": "plotline-preset"``) whose ``machine`` is used. A preset with no machine raises
    ``click.BadParameter``; the engine checks the machine when the session is given it."""
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if isinstance(data, dict) and data.get("format") == "plotline-preset":
        data = data.get("machine")
        if data is None:
            raise click.BadParameter(f"{path} is a preset with no machine", param_hint="--machine")
    return data


def use_machine(session: Session, path: str) -> None:
    """Give ``session`` the machine in the file at ``path``, which also sets the speeds, and make
    the page the machine's drawable area."""
    machine = read_machine(path)
    session.set_machine(machine)
    area = machine["area"]
    session.set_canvas(replace(session.canvas, width_mm=area["width"], height_mm=area["height"]))


def echo_stats(session: Session) -> None:
    """Print the path count, pen-down and pen-up distances and estimated time of the result."""
    stats = session.stats()
    click.echo(
        f"paths: {stats.path_count}  pen down: {stats.pen_down_mm:.1f} mm  "
        f"pen up: {stats.pen_up_mm:.1f} mm  estimated: {stats.estimated_seconds:.1f} s"
    )


@main.command()
@click.argument("style")
@click.argument("image", type=click.Path())
@click.option("--out", required=True, type=click.Path(), help="Where to write the SVG.")
@click.option("--json", "json_out", type=click.Path(), help="Also write the paths as JSON.")
@click.option("--preview", type=click.Path(), help="Also write a PNG preview.")
@page_options
@crop_option
@pen_option
@click.option(
    "--set",
    "sets",
    multiple=True,
    callback=lambda ctx, param, values: [parse_set(v) for v in values],
    metavar="STAGE.KEY=VALUE",
    help="Change a setting, such as tone.resolution=500. Repeat for more. See 'describe'.",
)
@click.option(
    "--preset",
    type=click.Path(),
    help="Start from a saved preset, or from an exported SVG or JSON drawing.",
)
@click.option(
    "--look",
    metavar="NAME",
    help=(
        "Start from one of the style's looks, such as faithful, quick or fine; "
        "plotline describe STYLE lists them."
    ),
)
@click.option(
    "--save-preset",
    type=click.Path(),
    help="Write the style, page, pens and settings used to a preset file.",
)
@machine_option
@click.option(
    "--gcode",
    type=click.Path(),
    metavar="DIR",
    help=(
        "Also write one G-code file per pen into DIR. Needs a machine, from --machine or --preset."
    ),
)
@click.option(
    "--hpgl",
    type=click.Path(),
    metavar="FILE",
    help="Also write the drawing as one HPGL file, with a pen selected for each layer.",
)
@click.option(
    "--hpgl-units",
    type=click.FloatRange(min=0, min_open=True, max=1000),
    default=40.0,
    show_default=True,
    help="Plotter units per mm in the HPGL file, above 0 and at most 1000.",
)
@fit_options
@click.option(
    "--optimize",
    is_flag=True,
    help=(
        "Also search the style's density setting for the quickest value whose drawing follows "
        "the image's tones within a tenth of the best, in up to nine runs, and print it and its "
        "plot time with the --set option that applies it. Values that plot for over twice as "
        "long, or an hour if that is more, or that the plot lint warns about, are left out, and "
        "the sentence says when one left out for its time would follow the tones more closely. "
        "With a --fit option, every trial stays within its target instead of fitting it, and "
        "--fit-minutes replaces the time limit. "
        "The drawing written is the one the given settings make."
    ),
)
@click.option("--quiet", is_flag=True, help="Do not print progress.")
@click.option(
    "--strict",
    is_flag=True,
    help="Exit with status 1 when the plot lint finds a warning or an error.",
)
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
    paper: str | None,
    crop: tuple[float, float, float, float] | None,
    pens: list[Pen],
    sets: list[tuple[str, str, Setting]],
    preset: str | None,
    look: str | None,
    save_preset: str | None,
    machine: str | None,
    gcode: str | None,
    hpgl: str | None,
    hpgl_units: float,
    fit_minutes: float | None,
    fit_paths: float | None,
    fit_metres: float | None,
    fit_dots: float | None,
    optimize: bool,
    quiet: bool,
    strict: bool,
) -> None:
    """Run STYLE on IMAGE and write the drawing as an SVG. The tone match follows the stats,
    and the plot lint's findings follow on stderr, each with the --set options that fix it where
    a setting does."""
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
        if look is not None:
            try:
                session.apply_look(look)
            except ValueError as error:
                raise click.BadParameter(str(error), param_hint=["--look"]) from error
        if machine is not None:
            use_machine(session, machine)
        set_page(session, width, height, margin, fit, paper, crop)
        if pens:
            session.set_pens(pens)
        for stage, key, value in sets:
            session.update(stage, **{key: value})
        if gcode is not None and session.machine is None:
            raise click.UsageError(
                "--gcode needs a machine: give --machine or a --preset that has one"
            )
        if optimize:
            handle = session.optimize(*(fits[0] if fits else (None, None)))
            handle.wait()
            if handle.error is not None:
                raise handle.error
            found = handle.optimum
            assert found is not None  # a search that ended without an error is done
            # With no trial scored, the reason says why and there is no patch.
            click.echo(f"optimize: {found['reason']}")
            if fix := _options(found["patch"]):
                click.echo(f"  apply: {fix}")
        elif fits:
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
        if gcode is not None:
            files = session.export_gcode(gcode)
            click.echo(f"gcode: {len(files)} {'file' if len(files) == 1 else 'files'} in {gcode}")
        if hpgl is not None:
            session.export_hpgl(hpgl, hpgl_units)
    echo_stats(session)
    if (match := session.tone_match()) is not None:
        click.echo(f"tone error: {match['error']:.3f}  ssim: {match['ssim']:.3f}")
    findings = session.lint()
    for finding in findings:
        click.echo(f"{finding['severity']}: {finding['message']}", err=True)
        if fix := _options(finding["patch"]):
            click.echo(f"  fix: {fix}", err=True)
    if strict and any(f["severity"] != "info" for f in findings):
        sys.exit(1)


@main.command()
@click.option("--out", required=True, type=click.Path(), help="Where to write the SVG.")
@click.option("--json", "json_out", type=click.Path(), help="Also write the paths as JSON.")
@click.option("--preview", type=click.Path(), help="Also write a PNG preview.")
@page_options
@pen_option
@machine_option
def testchart(
    out: str,
    json_out: str | None,
    preview: str | None,
    width: float | None,
    height: float | None,
    margin: float | None,
    fit: str | None,
    paper: str | None,
    pens: list[Pen],
    machine: str | None,
) -> None:
    """Write a test chart for the first pen: small labelled patches of line spacings, dot sizes
    and repeated passes, scaled to the pen. Plot it and compare by eye where lines merge, how big
    dots come out and how many passes the paper takes, and time the plot against the estimate."""
    session = Session()
    with _errors():
        if machine is not None:
            use_machine(session, machine)
        set_page(session, width, height, margin, fit, paper)
        if pens:
            session.set_pens(pens)
        session.testchart()
        session.export_svg(out)
        if json_out is not None:
            session.export_json(json_out)
        if preview is not None:
            Path(preview).write_bytes(session.preview_png())
    echo_stats(session)


def _text(value: Any) -> str:
    """A setting's value as ``coerce`` reads it back: a boolean as ``true`` or ``false``, numbers
    and text as they are."""
    return str(value).lower() if isinstance(value, bool) else str(value)


def _options(patch: dict[str, Any] | None) -> str:
    """The ``run`` options that apply a lint or optimize patch: ``--set`` for each setting,
    then ``--width`` and ``--height`` for a page."""
    if patch is None:
        return ""
    sets = [
        f"--set {stage}.{key}={_text(value)}"
        for stage, values in patch.get("settings", {}).items()
        for key, value in values.items()
    ]
    if page := patch.get("page"):
        sets.append(f"--width {page['width_mm']:g} --height {page['height_mm']:g}")
    return " ".join(sets)


@main.group("models")
def models_group() -> None:
    """Keep the learned line models that the learned style draws with."""


@models_group.command("list")
def models_list() -> None:
    """List the models that can be downloaded and the ones kept on this computer."""
    with _errors():  # an unreadable cache folder
        held = dict(models.kept())
        catalogue = {entry["id"]: entry for entry in _core.catalogue()}

        def status(e: dict[str, Any]) -> str:
            if e["id"] not in held:
                return f"{e['bytes'] / 1_000_000:.1f} MB"
            return "downloaded" if models.intact(e) else "damaged"

        # The licence of a file of your own is not known, so its cell stays blank.
        rows = [("ID", "NAME", "LICENCE", "STATUS")]
        rows += [(e["id"], e["label"], e["licence"], status(e)) for e in catalogue.values()]
        rows += [(i, "your own", "", "downloaded") for i in held if i not in catalogue]
    widths = [max(len(row[col]) for row in rows) for col in range(3)]
    for row in rows:
        cells = [row[col].ljust(widths[col]) for col in range(3)]
        click.echo("  ".join([*cells, row[3]]).rstrip())


@models_group.command("get")
@click.argument("id")
@click.option(
    "--accept-licence", is_flag=True, help="Accept the licence without asking, for scripts."
)
def models_get(id: str, accept_licence: bool) -> None:
    """Show the licence of the model ID, ask to accept it, and download the model."""
    entry = next((e for e in _core.catalogue() if e["id"] == id), None)
    if entry is None:
        click.echo(f"Error: {id!r} is not in the catalogue; run `plotline models list`", err=True)
        sys.exit(2)
    with _errors():
        if models.intact(entry):  # nothing to accept or fetch
            click.echo(f"{id} is already downloaded")
            return
    for line in (
        entry["label"],
        f"Licence: {entry['licence']}",
        f"Attribution: {entry['attribution']}",
        f"Training: {entry['training']}",
        f"Source: {entry['source']}",
        f"Paper: {entry['paper']}",
        f"Size: {entry['bytes'] / 1_000_000:.1f} MB",
        "Provided as is, without warranty of any kind.",
    ):
        click.echo(line)
    with _errors():
        # A licence accepted before is not asked again; a changed one is (models.accepted).
        asked = not (accept_licence or models.accepted(entry))
    if asked:  # outside `_errors`, because the abort of a "no" is a RuntimeError too
        click.confirm("Accept this licence and download?", abort=True)
    with _errors():
        path = models.download(entry)
        models.accept(entry)  # after the download, so a failed one keeps nothing at all
    click.echo(f"Downloaded {id} to {path}")


@models_group.command("add")
@click.argument("file", type=click.Path())
def models_add(file: str) -> None:
    """Copy the ONNX model FILE, a model of your own, into the cache and print its id."""
    with _errors():
        click.echo(models.add_file(Path(file)))


@models_group.command("remove")
@click.argument("id")
def models_remove(id: str) -> None:
    """Delete the kept model ID."""
    with _errors():
        if id not in dict(models.kept()):
            raise click.ClickException(f"{id!r} is not kept; run `plotline models list`")
        models.remove(id)
    click.echo(f"removed {id}")


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
