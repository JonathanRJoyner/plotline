"""Tests for presets: the saved file, the round trip, unknown and missing keys, and versions."""

import json

import pytest

from plotline import Canvas, Pen, Session


@pytest.fixture
def files(tmp_path):
    """``(read, apply)``: the preset of a session as a dict, and a dict applied as a preset file."""

    def read(s):
        s.save_preset(tmp_path / "saved.json")
        return json.loads((tmp_path / "saved.json").read_text(encoding="utf-8"))

    def apply(s, data):
        (tmp_path / "applied.json").write_text(json.dumps(data), encoding="utf-8")
        s.load_preset(tmp_path / "applied.json")

    return read, apply


def test_round_trip(tmp_path):
    s = Session()
    s.update("order", min_length_mm=2.0)
    s.set_canvas(Canvas(100, 100, 5, "fill"))
    s.set_pens([Pen("Red", 0.5, "#ff0000")])
    p = tmp_path / "p.json"
    s.save_preset(p)
    text = p.read_bytes()
    assert text.endswith(b"}\n") and b"\r" not in text and b'\n  "canvas"' in text
    d = json.loads(text)
    assert (d["format"], d["version"], d["style"], d["settings"]["order"]["min_length_mm"]) == (
        "plotline-preset",
        2,
        "grid",
        2.0,
    )
    t = Session()
    t.set_style("_slow")
    t.load_preset(p)
    assert t.canvas == s.canvas and t.pens == s.pens and t.settings == s.settings
    assert t.style == s.style


def test_unknown_stage_and_key_warn_and_are_dropped(files):
    read, apply = files
    s = Session()
    d = read(s)
    d["settings"]["order"]["nope"] = 1
    d["settings"]["ghost"] = {}
    d["settings"]["order"]["min_length_mm"] = 3.0
    with pytest.warns(UserWarning) as w:
        apply(s, d)
    assert len(w) == 2 and s.settings["order"]["min_length_mm"] == 3.0
    assert "nope" in str(w[0].message) and "ghost" in str(w[1].message)


def test_warnings_point_at_the_caller(tmp_path):
    s = Session()
    s.save_preset(tmp_path / "p.json")
    d = json.loads((tmp_path / "p.json").read_text())
    d["settings"]["ghost"] = {}
    (tmp_path / "p.json").write_text(json.dumps(d))
    with pytest.warns(UserWarning) as w:
        s.load_preset(tmp_path / "p.json")
    assert w[0].filename == __file__


def test_missing_keys_take_defaults(files):
    read, apply = files
    s = Session()
    s.update("order", min_length_mm=3.0)
    d = read(s)
    del d["settings"]["order"]["min_length_mm"]
    apply(s, d)
    assert s.settings["order"]["min_length_mm"] == 0.0


def test_bad_format_and_future_version_raise(files):
    read, apply = files
    s = Session()
    with pytest.raises(ValueError, match="not a preset"):
        apply(s, {**read(s), "format": "other"})
    with pytest.raises(ValueError, match="unsupported preset version"):
        apply(s, {**read(s), "version": 3})


def test_malformed_header_raises_value_error(files):
    _, apply = files
    s = Session()
    header = {"format": "plotline-preset"}
    for bad in ([], header, {**header, "version": 1.0}, {**header, "version": 0}):
        with pytest.raises(ValueError):
            apply(s, bad)


MISSING = object()


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("style", 5),
        ("canvas", MISSING),
        ("canvas", [100, 100]),
        ("pens", "Black"),
        ("pens", ["Black"]),
        ("settings", []),
        ("settings", {"order": []}),
    ],
    ids=[
        "style-not-a-string",
        "canvas-missing",
        "canvas-not-an-object",
        "pens-not-a-list",
        "pen-not-an-object",
        "settings-not-an-object",
        "stage-settings-not-an-object",
    ],
)
def test_a_misshapen_section_raises_a_value_error_naming_it_and_changes_nothing(
    files, field, value
):
    read, apply = files
    s = Session()
    s.update("order", min_length_mm=2.0)
    d = read(s)
    if value is MISSING:
        del d[field]
    else:
        d[field] = value
    with pytest.raises(ValueError, match=f"preset field '{field}' must be"):
        apply(s, d)
    # Setting the style would have reset the stage settings to their defaults.
    assert s.style.name == "grid" and s.settings["order"]["min_length_mm"] == 2.0
