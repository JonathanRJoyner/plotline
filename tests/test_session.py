"""Tests for the session: defaults, state changes, blocking runs and export."""

import json
import xml.etree.ElementTree as ET

import numpy as np
import pytest
from doubles import Spy
from PIL import Image
from pydantic import ValidationError
from synthetic import gradient

from plotline.core.pipeline import Style
from plotline.core.settings import Settings
from plotline.core.stage import Cancelled
from plotline.core.types import Canvas, Pen
from plotline.session.session import RunState, Session


def failing(error):
    return Style("f", "F", "d", (Spy("a", ("image",), "paths", fail=error),))


def test_defaults():
    s = Session()
    assert s.canvas == Canvas(210, 297, 10) and s.pens == (Pen("Black 0.3", 0.3, "#000000"),)
    assert s.style.name == "grid" and set(s.settings) == {"tone", "points", "render", "order"}
    assert s.state == RunState.idle and s.result is None and s.image is None


def test_set_image_scales_integers_by_their_type_and_accepts_grey():
    s = Session()
    s.set_image(np.full((4, 4, 3), 255, np.uint8))
    assert s.image.dtype == np.float32 and s.image.max() == 1.0
    s.set_image(np.full((4, 4, 3), 65535, np.uint16))
    assert s.image.max() == 1.0  # a 16-bit white is still white, not off the scale
    s.set_image(np.zeros((4, 4), np.float32))
    assert s.image.shape == (4, 4, 3)


@pytest.mark.parametrize("shape", [(4,), (4, 4, 4), (4, 4, 3, 1)])
def test_set_image_rejects_other_shapes(shape):
    with pytest.raises(ValueError):
        Session().set_image(np.zeros(shape, np.uint8))


def test_load_image_reads_a_file(tmp_path):
    Image.new("RGB", (6, 4), (255, 255, 255)).save(tmp_path / "i.png")
    s = Session()
    s.load_image(tmp_path / "i.png")
    assert s.image.shape == (4, 6, 3) and s.image.dtype == np.float32 and s.image.min() == 1.0


def test_update_validates_and_set_style_resets():
    s = Session()
    s.update("order", min_length_mm=1.5)
    assert s.settings["order"].min_length_mm == 1.5
    with pytest.raises(ValidationError):
        s.update("order", min_length_mm="abc")
    with pytest.raises(KeyError):
        s.update("ghost", x=1)
    s.set_style("grid")
    assert s.settings["order"].min_length_mm == 0.0
    with pytest.raises(ValueError):
        s.set_pens(())


def test_set_style_takes_a_style_object_and_keeps_the_cache():
    s = Session()
    s.set_image(gradient(8, 8))
    s.set_style(Style("t", "T", "d", (Spy("a", ("image",), "paths", value=[]),)))
    s.run_blocking()
    assert s.style.name == "t" and set(s.settings) == {"a"} and len(s.cache) == 1
    s.set_style("grid")
    assert s.style.name == "grid" and len(s.cache) == 1


class NoDefaults(Settings):
    knob: int  # required, so the style cannot build its default settings


def test_a_style_whose_defaults_fail_leaves_the_session_as_it_was():
    s = Session()
    s.update("order", min_length_mm=1.5)
    style, settings = s.style, dict(s.settings)
    stage = Spy("a", ("image",), "paths", value=[])
    stage.settings_model = NoDefaults
    with pytest.raises(ValidationError):
        s.set_style(Style("broken", "Broken", "d", (stage,)))
    assert s.style is style and s.settings == settings


def test_run_blocking_requires_an_image():
    with pytest.raises(RuntimeError):
        Session().run_blocking()


def test_run_blocking_grid_and_exports(tmp_path):
    s = Session()
    s.set_image(gradient(64, 48))
    s.update("tone", resolution=64)
    r = s.run_blocking()
    assert r is s.result and s.state == RunState.done and len(r.layers[0].paths) > 0
    s.export_svg(tmp_path / "o.svg")
    s.export_json(tmp_path / "o.json")
    assert ET.parse(tmp_path / "o.svg").getroot().tag.endswith("svg")
    assert json.loads((tmp_path / "o.json").read_text())["format"] == "plotline-paths"
    assert s.svg().startswith("<svg") and s.json_dict()["units"] == "mm"


def test_exports_are_utf8_with_lf(tmp_path):
    s = Session()
    s.set_image(gradient(64, 48))
    s.set_pens([Pen("Stift \u00e9", 0.3, "#000000")])
    s.update("tone", resolution=64)
    s.run_blocking()
    s.export_svg(tmp_path / "o.svg")
    s.export_json(tmp_path / "o.json")
    svg = (tmp_path / "o.svg").read_bytes()
    assert "\u00e9".encode() in svg and b"\r" not in svg
    assert b"\r" not in (tmp_path / "o.json").read_bytes()


def test_exports_require_a_result():
    with pytest.raises(RuntimeError):
        Session().svg()


def test_every_export_requires_a_result_and_writes_nothing(tmp_path):
    s = Session()
    with pytest.raises(RuntimeError):
        s.json_dict()
    with pytest.raises(RuntimeError):
        s.export_svg(tmp_path / "o.svg")
    with pytest.raises(RuntimeError):
        s.export_json(tmp_path / "o.json")
    assert not list(tmp_path.iterdir())


def test_error_state_on_failing_stage():
    s = Session()
    s.set_image(gradient(8, 8))
    s.set_style(failing(RuntimeError("boom")))
    with pytest.raises(RuntimeError):
        s.run_blocking()
    assert s.state == RunState.error and s.result is None


def test_a_failed_run_keeps_the_previous_result():
    s = Session()
    s.set_image(gradient(8, 8))
    s.set_style(Style("ok", "Ok", "d", (Spy("ok", ("image",), "paths", value=[]),)))
    first = s.run_blocking()
    s.set_style(failing(RuntimeError("boom")))
    with pytest.raises(RuntimeError):
        s.run_blocking()
    assert s.state == RunState.error and s.result is first


def test_cancelled_state_on_cancelled_stage():
    s = Session()
    s.set_image(gradient(8, 8))
    s.set_style(failing(Cancelled()))
    with pytest.raises(Cancelled):
        s.run_blocking()
    assert s.state == RunState.cancelled and s.result is None


def test_run_blocking_delivers_every_frame_while_running():
    s = Session()
    s.set_image(gradient(8, 8))
    seen = []
    s._deliver = lambda frame: seen.append((frame.stage, s.state))
    s.run_blocking()
    assert {stage for stage, _ in seen} == {"tone", "points", "render", "order"}
    assert {state for _, state in seen} == {RunState.running}


def test_run_blocking_uses_a_snapshot_of_the_settings():
    s = Session()
    s.set_image(gradient(64, 48))
    s.update("tone", resolution=64)
    s._deliver = lambda frame: s.update("order", min_length_mm=10)
    r = s.run_blocking()
    assert s.settings["order"].min_length_mm == 10 and len(r.layers[0].paths) > 0
