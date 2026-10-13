"""Tests for the command line interface: listing, a full run with every output, presets, bad input,
the GUI server and the MCP command without its extra."""

import http.server
import json
import sys
import webbrowser
import xml.etree.ElementTree as ET
from importlib import resources

import click
import pytest
from click.testing import CliRunner
from pngs import write_png

from plotline import Pen, _core, inks
from plotline.cli.main import main, parse_pen, parse_set


@pytest.fixture
def image(tmp_path):
    p = tmp_path / "i.png"
    write_png(p, _core.synthetic_gradient(64, 48))
    return str(p)


def run(*args):
    # click 8.2: .output is the combined stream, .stdout and .stderr are separate.
    return CliRunner().invoke(main, list(args))


def test_styles_and_describe():
    assert "grid" in run("styles").output and run("styles").exit_code == 0
    assert "_slow" not in run("styles").output
    out = run("describe", "grid").output
    assert "min_length_mm" in out and "Dots have zero length" in out
    assert "resolution  integer  default 1000  min 64  max 4000" in out
    assert json.loads(run("describe", "grid", "--json").output)["stages"][0]["name"] == "tone"
    assert run("describe", "nope").exit_code != 0


def test_run_writes_all_outputs(tmp_path, image):
    r = run(
        "run",
        "grid",
        image,
        "--out",
        str(tmp_path / "o.svg"),
        "--json",
        str(tmp_path / "o.json"),
        "--preview",
        str(tmp_path / "o.png"),
        "--save-preset",
        str(tmp_path / "p.json"),
        "--set",
        "tone.resolution=64",
        "--set",
        "tone.invert=false",
        "--pen",
        "Red 0.5:0.5:#ff0000",
        "--width",
        "100",
        "--height",
        "75",
        "--fit",
        "fill",
    )
    assert r.exit_code == 0, r.output
    assert ET.parse(tmp_path / "o.svg").getroot()[0].get("stroke") == "#ff0000"
    assert (tmp_path / "o.json").exists() and (tmp_path / "o.png").read_bytes()[:4] == b"\x89PNG"
    preset = json.loads((tmp_path / "p.json").read_text())
    assert preset["canvas"]["width_mm"] == 100 and preset["canvas"]["fit"] == "fill"
    assert preset["settings"]["tone"]["resolution"] == 64
    assert "paths:" in r.output and "tone:" in r.stderr


def test_preset_is_applied_and_style_kept(tmp_path, image):
    run(
        "run",
        "grid",
        image,
        "--out",
        str(tmp_path / "a.svg"),
        "--set",
        "tone.resolution=64",
        "--set",
        "points.spacing_mm=4",
        "--save-preset",
        str(tmp_path / "p.json"),
    )
    r = run(
        "run",
        "grid",
        image,
        "--out",
        str(tmp_path / "b.svg"),
        "--preset",
        str(tmp_path / "p.json"),
        "--quiet",
    )
    assert r.exit_code == 0 and r.stderr == ""
    # Compared outside the assert, because pytest would spend minutes diffing two long SVGs.
    same = (tmp_path / "a.svg").read_text() == (tmp_path / "b.svg").read_text()
    assert same


def test_run_rejects_bad_inputs(tmp_path, image):
    out = str(tmp_path / "o.svg")
    r = run("run", "grid", image, "--out", out, "--set", "order.min_length_mm=abc")
    assert r.exit_code == 1 and "min_length_mm" in r.output
    assert run("run", "grid", image, "--out", out, "--set", "nonsense").exit_code == 2
    r = run("run", "grid", image, "--out", out, "--margin", "200")
    assert r.exit_code == 1 and "margin" in r.output
    for pen, word in (("p:0:#000000", "width_mm"), ("p:0.5:crimson", "color")):
        r = run("run", "grid", image, "--out", out, "--pen", pen)
        assert r.exit_code == 1 and word in r.stderr, r.output


def test_gui_without_a_bundle_and_mcp_without_its_extra(monkeypatch, tmp_path):
    monkeypatch.setattr(resources, "files", lambda package: tmp_path)
    r = run("gui")
    assert r.exit_code == 1 and "This build has no GUI bundle" in r.stderr
    # A None entry makes "import mcp..." raise ModuleNotFoundError, as when the extra is absent,
    # but only for modules not imported yet. Other tests have loaded the SDK, so clear its cache.
    for name in [n for n in sys.modules if n.startswith("mcp.")]:
        monkeypatch.delitem(sys.modules, name)
    monkeypatch.setitem(sys.modules, "mcp", None)
    monkeypatch.delitem(sys.modules, "plotline.mcp_server", raising=False)
    r = run("mcp")
    assert r.exit_code == 1 and 'pip install "plotline[mcp]"' in r.stderr


def test_gui_serves_the_bundle_on_localhost(monkeypatch, tmp_path):
    (tmp_path / "web").mkdir()
    (tmp_path / "web" / "index.html").write_text("<!doctype html>")
    monkeypatch.setattr(resources, "files", lambda package: tmp_path)
    opened, served = [], []
    monkeypatch.setattr(webbrowser, "open", opened.append)

    def serve_forever(server, *args):
        served.append((server.server_address, server.RequestHandlerClass.keywords["directory"]))
        raise KeyboardInterrupt

    monkeypatch.setattr(http.server.ThreadingHTTPServer, "serve_forever", serve_forever)
    r = run("gui", "--port", "0")
    assert r.exit_code == 0, r.output
    (host, port), directory = served[0]
    assert host == "127.0.0.1" and directory == str(tmp_path / "web")
    assert opened == [f"http://127.0.0.1:{port}/"]


def test_parsers():
    assert parse_set("order.min_length_mm=1.5") == ("order", "min_length_mm", 1.5)
    values = [parse_set(f"a.b={text}")[2] for text in ("true", "false", "64", "1e3", "abc", "")]
    assert values == [True, False, 64, 1000.0, "abc", ""]
    assert [type(v) for v in values[:4]] == [bool, bool, int, float]
    # Only the last two colons split, so the name may hold colons and the colour keeps its '#'.
    assert parse_pen("Fine: red:0.5:#ff0000") == Pen("Fine: red", 0.5, "#ff0000")
    # An ink name in any case stands for its colour.
    assert parse_pen("Fine:0.3:Blue") == Pen("Fine", 0.3, inks()["blue"])
    for bad in ("nonsense", "a=1", "a.b", "a.b.c=1", "a.b=c=d", ".b=1", "a.=1"):
        with pytest.raises(click.BadParameter):
            parse_set(bad)
    for bad in ("black", "0.5:#000000", "p:wide:#000000"):
        with pytest.raises(click.BadParameter):
            parse_pen(bad)


def test_unknown_names_and_files_are_reported(tmp_path, image):
    gone = "[Errno 2] No such file or directory"
    bad_preset = tmp_path / "bad.json"
    header = {"format": "plotline-preset", "version": 1, "style": "grid"}
    sections = {"pens": [], "settings": {}}
    bad_preset.write_text(json.dumps({**header, **sections, "canvas": {"bogus": 1}}))
    no_canvas = tmp_path / "no_canvas.json"
    no_canvas.write_text(json.dumps({**header, **sections}))
    cases = [
        (("nope", image), 'unknown style "nope"'),
        (("grid", image, "--set", "ghost.x=1"), 'style "grid" has no stage "ghost"'),
        (("grid", str(tmp_path / "gone.png")), gone),
        (("grid", image, "--preset", str(tmp_path / "gone.json")), gone),
        (("grid", image, "--preset", str(bad_preset)), "canvas: unknown field `bogus`"),
        (("grid", image, "--preset", str(no_canvas)), "preset field 'canvas' must be an object"),
    ]
    for args, message in cases:
        r = run("run", *args, "--out", str(tmp_path / "o.svg"))
        # "Error: " is what click prints for a ClickException; any other exception would not show.
        assert r.exit_code == 1 and r.stderr.startswith(f"Error: {message}"), r.output
