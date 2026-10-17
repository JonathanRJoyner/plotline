"""Tests for the command line interface: listing, a full run with every output, presets, bad input,
the GUI server and the MCP command without its extra."""

import hashlib
import http.server
import io
import json
import sys
import urllib.request
import webbrowser
import xml.etree.ElementTree as ET
from importlib import resources
from pathlib import Path

import click
import numpy as np
import pytest
from click.testing import CliRunner
from machines import MACHINE
from pngs import write_png

from plotline import Pen, _core, inks, models
from plotline.cli.main import main, parse_pen, parse_set


@pytest.fixture
def image(tmp_path):
    p = tmp_path / "i.png"
    write_png(p, _core.synthetic_gradient(64, 48))
    return str(p)


def run(*args, input=None):
    # click 8.2: .output is the combined stream, .stdout and .stderr are separate.
    return CliRunner().invoke(main, list(args), input=input)


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
    assert r.exit_code == 0 and "tone:" not in r.stderr
    # Compared outside the assert, because pytest would spend minutes diffing two long SVGs.
    same = (tmp_path / "a.svg").read_text() == (tmp_path / "b.svg").read_text()
    assert same


def test_a_preset_of_another_style_carries_its_changes(tmp_path, image):
    base = ["--out", str(tmp_path / "o.svg"), "--quiet"]
    p, q = str(tmp_path / "p.json"), str(tmp_path / "q.json")
    run("run", "grid", image, *base, "--set", "tone.resolution=64", "--save-preset", p)
    r = run("run", "waves", image, *base, "--preset", p, "--save-preset", q)
    assert r.exit_code == 0, r.output
    saved = json.loads((tmp_path / "q.json").read_text())
    assert saved["style"] == "waves" and saved["settings"]["tone"]["resolution"] == 64
    assert saved["settings"]["tone"]["gamma"] == 1.9


def test_two_pens_in_one_pen_mode_get_a_note(tmp_path, image):
    note = "Pen use is set to one pen, so only the first pen draws."
    args = ["run", "grid", image, "--out", str(tmp_path / "o.svg"), "--quiet"]
    args += ["--set", "tone.resolution=64", "--pen", "A:0.3:black", "--pen", "B:0.3:red"]
    r = run(*args)
    assert r.exit_code == 0 and note in r.stderr, r.output
    r = run(*args, "--set", "split.mode=light_to_dark")
    assert r.exit_code == 0 and note not in r.stderr, r.output


def test_lint_findings_go_to_stderr_and_strict_fails_on_a_warning(tmp_path, image):
    # Waves at their defaults find nothing on the gradient; lines closer than the pen are a warning.
    args = ["run", "waves", image, "--out", str(tmp_path / "o.svg"), "--quiet"]
    args += ["--set", "tone.resolution=64", "--pen", "Wide:1:black"]
    args += ["--set", "waves.spacing_mm=0.5"]
    r = run(*args)
    assert r.exit_code == 0, r.output
    assert "warning: Marks 0.5 mm apart are closer than the 1 mm pen" in r.stderr
    assert "  fix: --set waves.spacing_mm=1" in r.stderr
    r = run(*args, "--strict")
    assert r.exit_code == 1 and (tmp_path / "o.svg").exists()
    assert run(*args[:-2], "--strict").exit_code == 0


def test_run_prints_the_tone_match_and_an_optimize_suggestion(tmp_path, image):
    args = ["run", "hatch", image, "--out", str(tmp_path / "o.svg"), "--quiet"]
    args += ["--width", "100", "--height", "75", "--set", "tone.resolution=64"]
    r = run(*args)
    assert r.exit_code == 0, r.output
    assert any(x.startswith("tone error: ") and "ssim: " in x for x in r.stdout.splitlines())
    r = run(*args, "--optimize")
    assert r.exit_code == 0, r.output
    line = next(x for x in r.stdout.splitlines() if x.startswith("optimize: "))
    assert "spacing_mm" in line
    lines = r.stdout.splitlines()
    apply = [x for x in lines if x.startswith("  apply: --set hatch.spacing_mm=")]
    assert len(apply) == ("already follows" not in line)


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


def test_run_fits_a_budget(tmp_path, image):
    out, preset = str(tmp_path / "o.svg"), tmp_path / "p.json"
    page = ["--width", "100", "--height", "75", "--set", "tone.resolution=64"]
    r = run(
        "run",
        "waves",
        image,
        "--out",
        out,
        *page,
        "--save-preset",
        str(preset),
        "--fit-metres",
        "10",
    )
    assert r.exit_code == 0, r.output
    line = next(x for x in r.stdout.splitlines() if x.startswith("fit: "))
    value = json.loads(preset.read_text())["settings"]["waves"]["spacing_mm"]
    assert line.startswith(f"fit: waves.spacing_mm = {value} (") and "for a target of 10 m" in line
    assert "trial 1 of 8" in r.stderr
    r = run("run", "waves", image, "--out", out, "--fit-paths", "5", "--fit-metres", "5")
    assert r.exit_code == 2 and "at most one --fit option" in r.output
    r = run("run", "waves", image, "--out", out, "--fit-dots", "500")
    assert r.exit_code == 1 and "places no dots" in r.output


def kinds(path, *options):
    r = run("analyse", str(path), *options)
    assert r.exit_code == 0, r.output
    return [s["kind"] for s in json.loads(r.stdout)["suggestions"]]


def test_analyse_prints_json(tmp_path):
    lines = np.ones((128, 128, 3))
    lines[np.arange(128) % 16 < 2] = 0  # 2-pixel black lines on white
    write_png(tmp_path / "lines.png", lines)
    r = run("analyse", str(tmp_path / "lines.png"))
    assert r.exit_code == 0, r.output
    assert set(json.loads(r.stdout)) == {"stats", "suggestions"}
    assert "line_art" in kinds(tmp_path / "lines.png")


def test_analyse_turns_the_page(tmp_path):
    write_png(tmp_path / "wide.png", _core.synthetic_gradient(400, 200))
    assert "orientation" in kinds(tmp_path / "wide.png")
    assert "orientation" not in kinds(tmp_path / "wide.png", "--width", "297", "--height", "210")


def test_analyse_refuses_a_non_image(tmp_path):
    (tmp_path / "t.txt").write_text("not an image")
    r = run("analyse", str(tmp_path / "t.txt"))
    assert r.exit_code == 1 and r.stderr.startswith("Error: unsupported image"), r.output


def test_gui_without_a_bundle_and_mcp_without_its_extra(monkeypatch, tmp_path):
    monkeypatch.setattr(resources, "files", lambda package: tmp_path)
    r = run("gui")
    assert r.exit_code == 1 and "This build has no GUI bundle" in r.stderr
    # A None entry makes "import mcp..." raise ModuleNotFoundError, as when the extra is absent,
    # but only for modules not imported yet. Other tests have loaded the SDK and anyio, which only
    # the extra brings, so clear their caches.
    for name in [n for n in sys.modules if n.startswith(("mcp.", "anyio."))]:
        monkeypatch.delitem(sys.modules, name)
    monkeypatch.setitem(sys.modules, "mcp", None)
    monkeypatch.setitem(sys.modules, "anyio", None)
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
        (("grid", image, "--machine", str(tmp_path / "gone.json")), gone),
    ]
    for args, message in cases:
        r = run("run", *args, "--out", str(tmp_path / "o.svg"))
        # "Error: " is what click prints for a ClickException; any other exception would not show.
        assert r.exit_code == 1 and r.stderr.startswith(f"Error: {message}"), r.output


def coarse(tmp_path, image, *args):
    """A run of a coarse grid that writes its SVG into ``tmp_path``."""
    args += ("--set", "tone.resolution=64", "--set", "points.spacing_mm=5", "--quiet")
    return run("run", "grid", image, "--out", str(tmp_path / "o.svg"), *args)


def test_a_machine_file_sets_the_page_and_gcode_is_written(tmp_path, image):
    machine, saved, out = tmp_path / "m.json", tmp_path / "p.json", tmp_path / "g"
    machine.write_text(json.dumps(MACHINE))
    args = ["--machine", str(machine), "--save-preset", str(saved)]
    r = coarse(tmp_path, image, *args, "--gcode", str(out))
    assert r.exit_code == 0, r.output
    assert f"gcode: 1 file in {out}" in r.stdout
    # The page is the machine's drawable area, and the margin and fit mode stay as they were.
    preset = json.loads(saved.read_text())
    assert preset["canvas"] == {"width_mm": 200, "height_mm": 174, "margin_mm": 10, "fit": "fit"}
    assert preset["machine"] == MACHINE
    [file] = out.iterdir()
    assert file.name == "plot_1_Black_0.3.gcode" and "G1 Z4 F1200" in file.read_text()
    # --width and --height come after the machine and win.
    r = coarse(tmp_path, image, *args, "--width", "100", "--height", "80", "--gcode", str(out))
    assert r.exit_code == 0, r.output
    assert json.loads(saved.read_text())["canvas"]["width_mm"] == 100
    assert json.loads(saved.read_text())["canvas"]["height_mm"] == 80


def test_a_preset_file_gives_its_machine_and_two_pens_give_two_files(tmp_path, image):
    machine, saved, out = tmp_path / "m.json", tmp_path / "p.json", tmp_path / "g"
    machine.write_text(json.dumps(MACHINE))
    r = coarse(tmp_path, image, "--machine", str(machine), "--save-preset", str(saved))
    assert r.exit_code == 0, r.output
    # A preset that has a machine works as the machine file, and also needs no --machine.
    for option in ("--machine", "--preset"):
        r = coarse(tmp_path, image, option, str(saved), "--gcode", str(out))
        assert r.exit_code == 0 and f"gcode: 1 file in {out}" in r.stdout, r.output
    pens = ["--pen", "A:0.3:#808080", "--pen", "B:0.3:black", "--set", "split.mode=light_to_dark"]
    r = coarse(tmp_path, image, "--preset", str(saved), "--gcode", str(out / "two"), *pens)
    assert r.exit_code == 0 and f"gcode: 2 files in {out / 'two'}" in r.stdout, r.output
    assert sorted(f.name for f in (out / "two").iterdir()) == ["plot_1_A.gcode", "plot_2_B.gcode"]


def test_a_gcode_run_needs_a_machine_that_works(tmp_path, image):
    out = tmp_path / "g"
    r = coarse(tmp_path, image, "--gcode", str(out))
    message = "--gcode needs a machine: give --machine or a --preset that has one"
    assert r.exit_code == 2 and message in r.output
    assert not (tmp_path / "o.svg").exists() and not out.exists()  # refused before the run
    # A preset saved without a machine has none to give.
    none = str(tmp_path / "none.json")
    assert coarse(tmp_path, image, "--save-preset", none).exit_code == 0
    r = coarse(tmp_path, image, "--machine", none)
    assert r.exit_code == 2 and "is a preset with no machine" in r.output
    r = coarse(tmp_path, image, "--preset", none, "--gcode", str(out))
    assert r.exit_code == 2 and message in r.output
    bad = tmp_path / "bad.json"
    bad.write_text(json.dumps({**MACHINE, "z_up_mm": 3}))
    r = coarse(tmp_path, image, "--machine", str(bad))
    assert r.exit_code == 1 and "z_up_mm must be above z_down_mm" in r.stderr, r.output
    # A page larger than the machine's area is refused when the G-code is written.
    good = tmp_path / "good.json"
    good.write_text(json.dumps(MACHINE))
    r = coarse(tmp_path, image, "--machine", str(good), "--width", "300", "--gcode", str(out))
    assert r.exit_code == 1 and "larger than the machine's" in r.stderr, r.output
    assert not out.exists()


TINY_MODEL = Path(__file__).parent.parent / "web" / "e2e" / "fixtures" / "tiny-conv.onnx"
DEMO_BYTES = b"demo model bytes"


def with_catalogue(monkeypatch, data=DEMO_BYTES, **changes):
    """List one catalogue entry for a model whose file is ``data``, and make ``urlopen`` serve it."""
    entry = {
        "id": "demo-1",
        "label": "Demo lines",
        "description": "Draws demo lines.",
        "url": "https://example.test/demo-1.onnx",
        "sha256": hashlib.sha256(data).hexdigest(),
        "licence": "MIT",
        "attribution": "Demo authors",
        "source": "https://example.test/source",
        "paper": "https://example.test/paper",
        "training": "Trained on demo photos.",
        "bytes": len(data),
        "overlap": 64,
    } | changes
    monkeypatch.setattr(_core, "catalogue", lambda: [entry])
    monkeypatch.setattr(urllib.request, "urlopen", lambda url, **kw: io.BytesIO(data))
    return entry


def test_models_add_then_list_shows_it(monkeypatch, tmp_path):
    with_catalogue(monkeypatch)
    r = run("models", "add", str(TINY_MODEL))
    assert r.exit_code == 0, r.output
    model_id = r.stdout.strip()
    assert model_id.startswith("file-")
    out = run("models", "list").stdout.splitlines()
    assert out[0].split() == ["demo-1", "Demo", "lines", "0.0", "MB"]
    assert out[1].split() == [model_id, "your", "own", "downloaded"]
    # A file that is missing or not a model is an error, and nothing more is kept.
    (tmp_path / "bad.onnx").write_bytes(b"not a model")
    for name in ("missing.onnx", "bad.onnx"):
        r = run("models", "add", str(tmp_path / name))
        assert r.exit_code == 1 and r.stderr.startswith("Error:"), r.output
    assert [i for i, _ in models.kept()] == [model_id]


def test_models_get_without_yes_downloads_nothing(monkeypatch):
    with_catalogue(monkeypatch, bytes=2_500_000)
    r = run("models", "get", "demo-1", input="n\n")
    assert r.exit_code == 1
    lines = r.stdout.splitlines()
    assert lines[:8] == [
        "Demo lines",
        "Licence: MIT",
        "Attribution: Demo authors",
        "Training: Trained on demo photos.",
        "Source: https://example.test/source",
        "Paper: https://example.test/paper",
        "Size: 2.5 MB",
        "Provided as is, without warranty of any kind.",
    ]
    assert lines[8].startswith("Accept this licence and download? [y/N]")
    assert models.kept() == [] and not models.cache_dir().exists()
    # No answer at all is not a yes either.
    assert run("models", "get", "demo-1").exit_code == 1 and models.kept() == []


def test_models_get_with_accept_licence_skips_the_question(monkeypatch):
    entry = with_catalogue(monkeypatch)
    r = run("models", "get", "demo-1", "--accept-licence")
    assert r.exit_code == 0, r.output
    assert "Accept this licence" not in r.output
    assert [i for i, _ in models.kept()] == ["demo-1"]
    assert models.kept()[0][1].read_bytes() == DEMO_BYTES and models.accepted(entry)


def test_models_get_asks_only_until_the_licence_is_accepted(monkeypatch):
    with_catalogue(monkeypatch)
    r = run("models", "get", "demo-1", input="y\n")
    assert r.exit_code == 0 and "Accept this licence and download? [y/N]" in r.output
    assert run("models", "remove", "demo-1").exit_code == 0 and models.kept() == []
    r = run("models", "get", "demo-1")
    assert r.exit_code == 0 and "Accept this licence" not in r.output
    assert [i for i, _ in models.kept()] == ["demo-1"]
    assert "demo-1  Demo lines  downloaded" in run("models", "list").stdout


def test_models_get_refuses_an_unknown_id_and_keeps_nothing_of_a_bad_download(monkeypatch):
    with_catalogue(monkeypatch)
    r = run("models", "get", "nope-1")
    assert r.exit_code == 2 and "nope-1" in r.stderr and "plotline models list" in r.stderr
    with_catalogue(monkeypatch, sha256="0" * 64)
    r = run("models", "get", "demo-1", "--accept-licence")
    assert r.exit_code == 1 and "checksum" in r.stderr, r.output
    assert models.kept() == [] and not models.cache_dir().exists()  # no model, no accepted.json


def test_models_remove_deletes_the_file():
    model_id = run("models", "add", str(TINY_MODEL)).stdout.strip()
    r = run("models", "remove", model_id)
    assert r.exit_code == 0 and models.kept() == []
    r = run("models", "remove", model_id)
    assert r.exit_code == 1 and "plotline models list" in r.stderr
