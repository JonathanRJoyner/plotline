"""Tests for presets: the saved file, the round trip, unknown and missing keys, and versions."""

import json

import pytest

from plotline.core.types import Canvas, Pen
from plotline.session import presets
from plotline.session.presets import PRESET_VERSION, apply_preset, preset_dict
from plotline.session.session import Session


def test_round_trip(tmp_path):
    s = Session()
    s.update("order", min_length_mm=2.0)
    s.set_canvas(Canvas(100, 100, 5, "fill"))
    s.set_pens([Pen("Red", 0.5, "#ff0000")])
    p = tmp_path / "p.json"
    s.save_preset(p)
    d = json.loads(p.read_text())
    assert (d["format"], d["version"], d["style"], d["settings"]["order"]["min_length_mm"]) == (
        "plotline-preset",
        1,
        "grid",
        2.0,
    )
    t = Session()
    t.load_preset(p)
    assert t.canvas == s.canvas and t.pens == s.pens and t.settings == s.settings
    assert t.style is s.style


def test_unknown_stage_and_key_warn_and_are_dropped():
    s = Session()
    d = preset_dict(s)
    d["settings"]["order"]["nope"] = 1
    d["settings"]["ghost"] = {}
    d["settings"]["order"]["min_length_mm"] = 3.0
    with pytest.warns(UserWarning) as w:
        apply_preset(s, d)
    assert len(w) == 2 and s.settings["order"].min_length_mm == 3.0


def test_missing_keys_take_defaults():
    s = Session()
    s.update("order", min_length_mm=3.0)
    d = preset_dict(s)
    del d["settings"]["order"]["min_length_mm"]
    apply_preset(s, d)
    assert s.settings["order"].min_length_mm == 0.0


def test_bad_format_and_future_version_raise():
    s = Session()
    with pytest.raises(ValueError):
        apply_preset(s, {**preset_dict(s), "format": "other"})
    with pytest.raises(ValueError):
        apply_preset(s, {**preset_dict(s), "version": PRESET_VERSION + 1})


def test_malformed_header_raises_value_error():
    s = Session()
    header = {"format": "plotline-preset"}
    for bad in ([], header, {**header, "version": 1.0}, {**header, "version": 0}):
        with pytest.raises(ValueError):
            apply_preset(s, bad)


def test_migration_is_invoked(monkeypatch):
    calls = []

    def up(d):
        calls.append(d["version"])
        return {**d, "version": 1}

    monkeypatch.setattr(presets, "MIGRATIONS", {0: up})
    s = Session()
    apply_preset(s, {**preset_dict(s), "version": 0})
    assert calls == [0]
