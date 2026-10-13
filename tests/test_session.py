"""Tests for the session: defaults, state changes, blocking runs and export."""

import json
import xml.etree.ElementTree as ET

import numpy as np
import pytest
from pngs import write_png

from plotline import (
    Cancelled,
    Canvas,
    Pen,
    RunState,
    Session,
    _core,
    describe,
    get_style,
    list_styles,
)


def slow(s, **values):
    """Set ``s`` up to run the hidden ``_slow`` style with no pauses."""
    s.set_image(_core.synthetic_gradient(8, 8))
    s.set_style("_slow")
    s.update("b", delay_ms=0, **values)
    return s


def test_defaults():
    s = Session()
    assert s.canvas == Canvas(210, 297, 10) and s.pens == (Pen("Black 0.3", 0.3, "#000000"),)
    assert s.style.name == "grid" and set(s.settings) == {"tone", "points", "render", "order"}
    assert s.state == RunState.idle and s.result is None and s.image is None


def test_registry_lists_public_styles_and_finds_hidden_ones():
    assert [s.name for s in list_styles()] == [
        "grid",
        "hatch",
        "waves",
        "spiral",
        "ink",
        "stipple",
        "line",
        "dashes",
        "sketch",
        "halftone",
        "dither",
    ]
    assert get_style("_slow").label == "Slow" and get_style("grid").schema() == describe("grid")
    with pytest.raises(ValueError, match="unknown style"):
        get_style("nope")


def test_set_image_scales_integers_by_their_type_and_accepts_grey():
    s = Session()
    s.set_image(np.full((4, 4, 3), 255, np.uint8))
    assert s.image.dtype == np.float32 and s.image.max() == 1.0
    s.set_image(np.full((4, 4, 3), 65535, np.uint16))
    assert s.image.max() == 1.0
    s.set_image(np.zeros((4, 4), np.float32))
    assert s.image.shape == (4, 4, 3)


@pytest.mark.parametrize("shape", [(4,), (4, 4, 4), (4, 4, 3, 1)])
def test_set_image_rejects_other_shapes(shape):
    with pytest.raises(ValueError, match="image must have shape"):
        Session().set_image(np.zeros(shape, np.uint8))


def test_load_image_reads_a_file(tmp_path):
    write_png(tmp_path / "i.png", np.ones((4, 6, 3)))
    s = Session()
    s.load_image(tmp_path / "i.png")
    assert s.image.shape == (4, 6, 3) and s.image.dtype == np.float32 and s.image.min() == 1.0


def test_update_validates_and_set_style_resets():
    s = Session()
    s.update("order", min_length_mm=1.5)
    assert s.settings["order"]["min_length_mm"] == 1.5
    with pytest.raises(ValueError, match="min_length_mm"):
        s.update("order", min_length_mm="abc")
    with pytest.raises(ValueError, match="ghost"):
        s.update("ghost", x=1)
    s.set_style("grid")
    assert s.settings["order"]["min_length_mm"] == 0.0
    with pytest.raises(ValueError):
        s.set_pens(())


def test_the_engine_checks_canvas_and_pens_and_a_refusal_changes_nothing():
    s = Session()
    with pytest.raises(ValueError, match="width_mm"):
        s.set_canvas(Canvas(0, 10))
    with pytest.raises(ValueError, match="color"):
        s.set_pens([Pen("p", 0.5, "red")])
    assert s.canvas == Canvas(210, 297, 10) and s.pens == (Pen("Black 0.3", 0.3, "#000000"),)
    s.set_canvas(Canvas(100, 75, 5, "fill"))
    s.set_pens([Pen("Red", 0.5, "#ff0000")])
    assert s.canvas == Canvas(100, 75, 5, "fill") and s.pens == (Pen("Red", 0.5, "#ff0000"),)


def test_set_style_keeps_the_cache():
    s = slow(Session(), frames=1)
    s.run_blocking()
    assert s.style.name == "_slow" and set(s.settings) == {"a", "b"}
    assert s.cache_stats()["entries"] == 2
    s.set_style("grid")
    assert s.style.name == "grid" and s.cache_stats()["entries"] == 2
    with pytest.raises(ValueError, match="unknown style"):
        s.set_style("nope")
    assert s.style.name == "grid"


def test_run_blocking_requires_an_image():
    with pytest.raises(RuntimeError, match="no image"):
        Session().run_blocking()


def test_run_blocking_grid_and_exports(tmp_path):
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", resolution=64)
    r = s.run_blocking()
    assert r is s.result and s.state == RunState.done and len(r.layers[0].paths) > 0
    assert all(p.ndim == 2 and p.shape[1] == 2 for p in r.layers[0].paths)
    assert r.layers[0].pen == s.pens[0] and r.canvas == s.canvas and r.tone.ink.ndim == 2
    s.export_svg(tmp_path / "o.svg")
    s.export_json(tmp_path / "o.json")
    assert ET.parse(tmp_path / "o.svg").getroot().tag.endswith("svg")
    assert json.loads((tmp_path / "o.json").read_text())["format"] == "plotline-paths"
    assert s.svg().startswith("<svg") and s.json_dict()["units"] == "mm"
    assert s.preview_png(100)[:4] == b"\x89PNG"


def test_frames_carry_points_and_paths_as_arrays():
    s = Session(max_fps=1e9)
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", resolution=64)
    frames = []
    s.subscribe(frames.append)
    s.run_blocking()
    assert any(f.points is not None and f.points.shape[1] == 2 for f in frames)
    last = frames[-1]
    assert last.stage == "order" and last.progress == 1.0 and last.label
    assert last.paths and all(p.ndim == 2 and p.shape[1] == 2 for p in last.paths)


def test_stats_use_the_speeds_edited_in_place():
    s = slow(Session(), frames=1)
    s.run_blocking()
    before = s.stats()
    assert before.path_count == 1
    s.speeds.pen_lift_s += 1.0
    assert s.stats().estimated_seconds == pytest.approx(before.estimated_seconds + 1.0)


def test_an_error_after_the_last_frame_ends_the_run_in_error(monkeypatch):
    def broken(self):
        raise OSError("drawing lost")

    monkeypatch.setattr(_core.Session, "drawing", broken)
    s = slow(Session(), frames=1)
    with pytest.raises(OSError, match="drawing lost"):
        s.run_blocking()
    assert s.state == RunState.error and s.result is None
    h = s.run()
    assert h.wait(5) == RunState.error and isinstance(h.error, OSError) and h.result is None


def test_exports_are_utf8_with_lf(tmp_path):
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.set_pens([Pen("Stift é", 0.3, "#000000")])
    s.update("tone", resolution=64)
    s.run_blocking()
    s.export_svg(tmp_path / "o.svg")
    s.export_json(tmp_path / "o.json")
    svg = (tmp_path / "o.svg").read_bytes()
    assert "é".encode() in svg and b"\r" not in svg
    assert b"\r" not in (tmp_path / "o.json").read_bytes()


def test_exports_require_a_result():
    with pytest.raises(RuntimeError, match="no result"):
        Session().svg()


def test_every_export_requires_a_result_and_writes_nothing(tmp_path):
    s = Session()
    for output in (s.json_dict, s.stats, s.preview_png):
        with pytest.raises(RuntimeError):
            output()
    with pytest.raises(RuntimeError):
        s.export_svg(tmp_path / "o.svg")
    with pytest.raises(RuntimeError):
        s.export_json(tmp_path / "o.json")
    assert not list(tmp_path.iterdir())


def test_error_state_on_failing_stage():
    s = slow(Session(), fail_after=1)
    with pytest.raises(RuntimeError, match="slow failed"):
        s.run_blocking()
    assert s.state == RunState.error and s.result is None


def test_a_failed_run_keeps_the_previous_result():
    s = slow(Session(), frames=1)
    first = s.run_blocking()
    s.update("b", fail_after=1)
    with pytest.raises(RuntimeError, match="slow failed"):
        s.run_blocking()
    assert s.state == RunState.error and s.result is first


def test_cancelled_state_on_cancelled_run():
    s = slow(Session(), frames=5)
    s.subscribe(lambda frame: s.cancel())
    with pytest.raises(Cancelled):
        s.run_blocking()
    assert s.state == RunState.cancelled and s.result is None


def test_run_blocking_delivers_every_frame_while_running():
    s = Session()
    s.set_image(_core.synthetic_gradient(8, 8))
    seen = []
    s._deliver = lambda frame: seen.append((frame.stage, s.state))
    s.run_blocking()
    assert {stage for stage, _ in seen} == {"tone", "points", "render", "order"}
    assert {state for _, state in seen} == {RunState.running}


def test_run_blocking_uses_a_snapshot_of_the_settings():
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", resolution=64)
    s._deliver = lambda frame: s.update("order", min_length_mm=10)
    r = s.run_blocking()
    assert s.settings["order"]["min_length_mm"] == 10 and len(r.layers[0].paths) > 0
