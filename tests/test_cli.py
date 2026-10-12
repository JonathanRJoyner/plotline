"""Tests for the command line interface: listing, a full run with every output, presets, bad input
and the stubs."""

import json
import xml.etree.ElementTree as ET

import click
import numpy as np
import pytest
from click.testing import CliRunner
from PIL import Image
from synthetic import gradient

from plotline.cli.main import main, parse_pen, parse_set
from plotline.core.types import Pen


@pytest.fixture
def image(tmp_path):
    p = tmp_path / "i.png"
    Image.fromarray((gradient(64, 48) * 255).astype(np.uint8)).save(p)
    return str(p)


def run(*args):
    # click 8.2: .output is the combined stream, .stdout and .stderr are separate.
    return CliRunner().invoke(main, list(args))


def test_styles_and_describe():
    assert "grid" in run("styles").output and run("styles").exit_code == 0
    out = run("describe", "grid").output
    assert "min_length_mm" in out and "Dots have zero length" in out
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
        "--pen",
        "Red 0.5:0.5:#ff0000",
        "--width",
        "100",
        "--height",
        "75",
    )
    assert r.exit_code == 0, r.output
    assert ET.parse(tmp_path / "o.svg").getroot()[0].get("stroke") == "#ff0000"
    assert (tmp_path / "o.json").exists() and (tmp_path / "o.png").exists()
    assert json.loads((tmp_path / "p.json").read_text())["canvas"]["width_mm"] == 100
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
    assert r.exit_code != 0 and "min_length_mm" in r.output
    assert run("run", "grid", image, "--out", out, "--set", "nonsense").exit_code == 2
    r = run("run", "grid", image, "--out", out, "--margin", "200")
    assert r.exit_code != 0 and "margin" in r.output


def test_gui_and_mcp_stubs():
    for cmd in ("gui", "mcp"):
        r = run(cmd)
        assert r.exit_code == 1 and "not available" in r.stderr


def test_parsers():
    assert parse_set("order.min_length_mm=1.5") == ("order", "min_length_mm", "1.5")
    # Only the last two colons split, so the name may hold colons and the colour keeps its '#'.
    assert parse_pen("Fine: red:0.5:#ff0000") == Pen("Fine: red", 0.5, "#ff0000")
    for bad in ("nonsense", "a=1", "a.b", "a.b.c=1", "a.b=c=d", ".b=1", "a.=1"):
        with pytest.raises(click.BadParameter):
            parse_set(bad)
    for bad in ("black", "0.5:#000000", "p:wide:#000000", "p:0:#000000", "p:0.5:red"):
        with pytest.raises(click.BadParameter):
            parse_pen(bad)


def test_unknown_names_and_files_are_reported(tmp_path, image):
    gone = "[Errno 2] No such file or directory"
    bad_preset = tmp_path / "bad.json"
    header = {"format": "plotline-preset", "version": 1, "style": "grid"}
    bad_preset.write_text(json.dumps({**header, "canvas": {"bogus": 1}}))
    cases = [
        (("nope", image), "unknown style 'nope'"),
        (("grid", image, "--set", "ghost.x=1"), "style 'grid' has no stage 'ghost'"),
        (("grid", str(tmp_path / "gone.png")), gone),
        (("grid", image, "--preset", str(tmp_path / "gone.json")), gone),
        (("grid", image, "--preset", str(bad_preset)), "invalid preset: "),
    ]
    for args, message in cases:
        r = run("run", *args, "--out", str(tmp_path / "o.svg"))
        # "Error: " is what click prints for a ClickException; any other exception would not show.
        assert r.exit_code == 1 and r.stderr.startswith(f"Error: {message}"), r.output
