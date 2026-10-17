"""Tests for the session: defaults, state changes, blocking runs and export."""

import json
import struct
import xml.etree.ElementTree as ET

import numpy as np
import pytest
from looks import off_look, style_with_looks
from machines import MACHINE
from pngs import decode_png, write_png

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
    assert s.style.name == "grid"
    assert set(s.settings) == {"tone", "split", "points", "render", "order"}
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
        "tree",
        "mesh",
        "cells",
        "halftone",
        "dither",
        "circles",
        "contours",
        "flow",
        "hilbert",
        "scribble",
        "mosaic",
        "truchet",
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


def test_update_validates_and_set_style_keeps_changes():
    s = Session()
    s.update("order", min_length_mm=1.5)
    assert s.settings["order"]["min_length_mm"] == 1.5
    with pytest.raises(ValueError, match="min_length_mm"):
        s.update("order", min_length_mm="abc")
    with pytest.raises(ValueError, match="ghost"):
        s.update("ghost", x=1)
    # A changed value carries; one at grid's default takes the new style's default.
    s.set_style("waves")
    assert s.settings["order"]["min_length_mm"] == 1.5
    assert s.settings["tone"]["gamma"] == 1.9
    s.reset_settings()
    assert s.settings["order"]["min_length_mm"] == 0.0
    assert s.settings["tone"]["gamma"] == 1.9
    with pytest.raises(ValueError):
        s.set_pens(())


def test_apply_look_sets_the_looks_settings_and_look_reports_it():
    name, looks = style_with_looks()
    s = Session()
    s.set_style(name)
    assert s.look == "balanced"
    for look in looks:
        s.apply_look(look["name"])
        assert s.look == look["name"]
        for stage, values in look["settings"].items():
            for key, value in values.items():
                assert s.settings[stage][key] == value


def test_a_look_leaves_the_settings_no_look_sets_alone():
    name, looks = style_with_looks()
    s = Session()
    s.set_style(name)
    s.update("tone", invert=True)
    s.update("split", mode="light_to_dark")
    before = s.settings
    s.apply_look(looks[-1]["name"])
    assert s.settings != before
    sets = {(stage, key) for look in looks for stage, v in look["settings"].items() for key in v}
    for stage, values in before.items():
        for key, value in values.items():
            if (stage, key) not in sets:
                assert s.settings[stage][key] == value


def test_a_reset_gives_balanced_an_edit_gives_none_and_a_missing_look_is_refused():
    name, looks = style_with_looks()
    s = Session()
    s.set_style(name)
    s.apply_look(looks[-1]["name"])
    s.reset_settings()
    assert s.look == "balanced"
    s.apply_look(looks[-1]["name"])
    stage, key, value = off_look(name, looks)
    s.update(stage, **{key: value})
    assert s.look is None
    # Only a look of this style applies, whatever the other styles have.
    have = [look["name"] for look in looks]
    missing = next(n for n in ("faithful", "quick", "fine", "nope") if n not in have)
    before = s.settings
    with pytest.raises(ValueError, match=f'no look "{missing}"') as refused:
        s.apply_look(missing)
    assert ", ".join(have) in str(refused.value)
    assert s.settings == before and s.look is None


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


def test_the_page_takes_a_crop_that_reaches_the_drawing_and_the_preset(tmp_path):
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", resolution=64)
    s.set_canvas(Canvas(100, 100, 0))
    s.run_blocking()
    assert s.result.tone.ink.shape == (48, 64) and s.result.canvas.crop == (0.0,) * 4
    plain = tmp_path / "plain.json"
    s.save_preset(plain)
    assert "crop" not in json.loads(plain.read_text())["canvas"]
    # The middle half of the width, 32 x 48 pixels, drawn as a map 64 pixels tall.
    s.set_canvas(Canvas(100, 100, 0, crop=(0.25, 0, 0.25, 0)))
    assert s.canvas == Canvas(100, 100, 0, crop=[0.25, 0, 0.25, 0])
    s.run_blocking()
    assert s.result.tone.ink.shape == (64, 43) and s.result.canvas.crop == (0.25, 0, 0.25, 0)
    cropped = tmp_path / "cropped.json"
    s.save_preset(cropped)
    assert json.loads(cropped.read_text())["canvas"]["crop"] == [0.25, 0, 0.25, 0]
    t = Session()
    t.load_preset(cropped)
    assert t.canvas == s.canvas


def test_the_engine_checks_the_crop_and_a_refusal_changes_nothing():
    s = Session()
    s.set_canvas(Canvas(100, 75, 5, crop=(0.1, 0.1, 0.1, 0.1)))
    for crop, name in [
        ((0.5, 0, 0, 0), "left"),
        ((0, 0, 0, -0.1), "bottom"),
    ]:
        with pytest.raises(ValueError, match=f"crop {name} must be from 0 to 0.45"):
            s.set_canvas(Canvas(100, 75, 5, crop=crop))
    for crop in [(0, 0, float("nan"), 0), (0.1, 0.1)]:  # not finite, not four shares
        with pytest.raises(ValueError):
            s.set_canvas(Canvas(100, 75, 5, crop=crop))  # type: ignore[arg-type]
    assert s.canvas == Canvas(100, 75, 5, crop=(0.1, 0.1, 0.1, 0.1))


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


def test_analysis():
    s = Session()
    with pytest.raises(RuntimeError, match="no image is loaded"):
        s.analysis()
    s.set_image(_core.synthetic_gradient(64, 48))
    a = s.analysis()
    assert set(a) == {"stats", "suggestions"}
    assert (a["stats"]["width_px"], a["stats"]["height_px"]) == (64, 48)
    assert isinstance(a["suggestions"], list)


def test_lint_follows_the_speeds():
    s = Session()
    with pytest.raises(RuntimeError, match="no result"):
        s.lint()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", resolution=64)
    s.run_blocking()
    for f in s.lint():
        assert set(f) == {"code", "severity", "message", "value", "patch"}
    s.speeds.pen_lift_s = 1000.0
    assert "lifts" in [f["code"] for f in s.lint()]


def test_tone_match_and_auto_response():
    s = Session()
    with pytest.raises(RuntimeError, match="no result"):
        s.tone_match()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.set_canvas(Canvas(100, 75, 5))
    s.set_style("waves")
    s.update("tone", resolution=64)
    s.run_blocking()
    # The engine's pinned values, since the image, page and style are the same.
    match = s.tone_match()
    assert (match["error"], match["ssim"]) == (0.08197368769144689, 0.7488977156057808)
    assert set(match) == {"error", "ssim", "floor", "reach", "contrast", "blank", "colour"}
    s.update("tone", response="auto")
    frames = []
    s.subscribe(frames.append)
    s.run_blocking()
    assert s.tone_match()["error"] == 0.08880655753709105
    assert any(f.stage == "tone" and f.label.startswith("response: ") for f in frames)


def test_optimize_suggests_and_applies_nothing():
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.set_canvas(Canvas(100, 75, 5))
    s.set_style("hatch")
    s.update("tone", resolution=64)
    before = s.settings
    handle = s.optimize()
    assert handle.wait() is RunState.done
    found = handle.optimum
    assert found is not None and handle.fit is None
    assert (found["stage"], found["key"], found["current"]) == ("hatch", "spacing_mm", 1.0)
    # The quickest within a tenth of the lowest tone error, which is at most the current one's.
    assert len(found["trials"]) == 9 and found["error"] <= 1.1 * found["current_error"]
    assert found["current_minutes"] == s.stats().estimated_seconds / 60
    assert found["limit_minutes"] == max(2 * found["current_minutes"], 60)
    assert found["beyond"] is None or found["beyond"]["slow"]
    assert "min to plot" in found["reason"]
    assert s.settings == before and s.result is handle.result
    if found["patch"] is not None:
        assert found["patch"] == {"settings": {"hatch": {"spacing_mm": found["value"]}}}
    minutes = s.stats().estimated_seconds / 60
    within = s.optimize("minutes", minutes / 2)
    assert within.wait() is RunState.done
    assert all(t["over"] == (t["metric"] > minutes / 2) for t in within.optimum["trials"])
    with pytest.raises(ValueError, match="both a metric and a target"):
        s.optimize("minutes")


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


def test_preview_region():
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", resolution=64)
    s.run_blocking()
    png = s.preview_png(100, region=(0, 0, 105, 148.5))  # the top-left quarter of the A4 page
    assert struct.unpack(">II", png[16:24]) == (100, 141)
    with pytest.raises(ValueError, match="preview region"):
        s.preview_png(100, region=(0, 0, 0, 10))


def test_the_paper_is_white_until_set_and_takes_only_rrggbb():
    s = Session()
    assert s.paper == "#ffffff"
    s.set_paper("#f5f0e1")
    assert s.paper == "#f5f0e1"
    for bad in ("cream", "#fff", "f5f0e1"):
        with pytest.raises(ValueError, match="paper"):
            s.set_paper(bad)
    assert s.paper == "#f5f0e1"


def test_the_preview_shows_the_paper_in_the_margin_and_a_new_paper_needs_no_run():
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.set_canvas(Canvas(100, 75, 10))
    s.update("tone", resolution=64)
    s.run_blocking()
    assert decode_png(s.preview_png(100))[0, 0].tolist() == [255, 255, 255]
    s.set_paper("#f5f0e1")
    assert decode_png(s.preview_png(100))[0, 0].tolist() == [245, 240, 225]


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


def test_a_lower_acceleration_lengthens_the_plot_time_and_a_machine_sets_it():
    s = slow(Session(), frames=1)
    s.run_blocking()
    assert s.speeds.accel_mm_s2 == 1000
    before = s.stats().estimated_seconds
    s.speeds.accel_mm_s2 = 1.0
    assert s.stats().estimated_seconds > before
    s.set_machine({**MACHINE, "accel_mm_s2": 250})
    assert s.speeds.accel_mm_s2 == 250 and s.machine["accel_mm_s2"] == 250
    # At its default the machine leaves it out, as a preset saved before it existed has it.
    s.set_machine(MACHINE)
    assert s.speeds.accel_mm_s2 == 1000 and "accel_mm_s2" not in s.machine
    with pytest.raises(ValueError, match="accel_mm_s2 must be positive"):
        s.set_machine({**MACHINE, "accel_mm_s2": 0})


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


def test_testchart_is_the_result_and_needs_no_image():
    s = Session()
    assert s.state is RunState.idle
    drawing = s.testchart()
    assert s.state is RunState.done
    assert s.result is drawing and len(drawing.layers) == 1 and drawing.tone is None
    assert drawing.layers[0].pen == s.pens[0]
    assert s.stats().path_count == len(drawing.layers[0].paths) > 0
    assert s.svg().startswith("<svg")
    assert "overdraw" in [f["code"] for f in s.lint()]
    assert s.preview_png(200).startswith(b"\x89PNG")


def test_testchart_cancels_a_run_and_a_small_page_leaves_the_result():
    s = slow(Session(), frames=1000)
    s.update("b", delay_ms=5)  # slow enough that the run is still going when the chart is made
    handle = s.run()
    assert s.state is RunState.running
    chart = s.testchart()
    assert handle.wait() is RunState.cancelled and s.result is chart
    assert s.state is RunState.done
    s.set_canvas(Canvas(74, 105, 10))
    s.set_pens([Pen("Fat", 2.0, "#000000")])
    with pytest.raises(ValueError, match="the page is too small for the test chart"):
        s.testchart()
    assert s.result is chart and s.state is RunState.done
    # A later run takes the state back, and a refused chart leaves a run under way alone.
    s.update("b", frames=1000)
    handle = s.run()
    with pytest.raises(ValueError, match="the page is too small for the test chart"):
        s.testchart()
    assert s.state is RunState.running and handle.state is RunState.running
    handle.cancel()
    assert handle.wait() is RunState.cancelled and s.state is RunState.cancelled
    s.update("b", frames=2)
    s.run_blocking()
    assert s.state is RunState.done and s.result is not chart


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
    assert {stage for stage, _ in seen} == {"tone", "split", "points", "render", "order"}
    assert {state for _, state in seen} == {RunState.running}


def test_run_blocking_uses_a_snapshot_of_the_settings():
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", resolution=64)
    s._deliver = lambda frame: s.update("order", min_length_mm=10)
    r = s.run_blocking()
    assert s.settings["order"]["min_length_mm"] == 10 and len(r.layers[0].paths) > 0


def test_two_pens_by_colour_draw_one_layer_each():
    img = np.ones((48, 64, 3))
    img[:, :32] = [1.0, 0.0, 0.0]
    img[:, 32:] = [0.0, 0.0, 1.0]
    s = Session(max_fps=1e9)
    s.set_image(img)
    s.set_pens([Pen("Red", 0.3, "#ff0000"), Pen("Blue", 0.3, "#0000ff")])
    s.set_style("hatch")
    s.update("tone", resolution=64)
    s.update("split", mode="by_colour")
    frames = []
    s.subscribe(frames.append)
    r = s.run_blocking()
    assert [layer.pen.name for layer in r.layers] == ["Red", "Blue"]
    assert all(len(layer.paths) > 0 for layer in r.layers)
    assert {f.pen for f in frames if f.stage == "hatch"} == {0, 1}


def test_a_colour_image_suggests_pens_that_set_pens_accepts():
    img = np.ones((48, 64, 3))
    for i, rgb in enumerate([[1.0, 0.0, 0.0], [0.0, 0.7, 0.0], [0.0, 0.0, 1.0], [1.0, 0.9, 0.0]]):
        img[:, i * 16 : (i + 1) * 16] = rgb
    s = Session()
    s.set_image(img)
    analysis = s.analysis()
    assert 2 <= len(analysis["stats"]["palette"]) <= 4
    assert analysis["stats"]["palette_error"] >= 0
    patch = next(x["patch"] for x in analysis["suggestions"] if x["kind"] == "colour")
    assert patch["settings"] == {"split": {"mode": "by_colour"}}
    s.set_pens([Pen(p["name"], p["width_mm"], p["color"]) for p in patch["pens"]])
    assert [p.name for p in s.pens] == [p["name"] for p in patch["pens"]]


def test_a_pen_library_chooses_the_palette():
    img = np.ones((48, 64, 3))
    for i, rgb in enumerate([[1.0, 0.0, 0.0], [0.0, 0.7, 0.0], [0.0, 0.0, 1.0], [1.0, 0.9, 0.0]]):
        img[:, i * 16 : (i + 1) * 16] = rgb
    s = Session()
    s.set_image(img)
    library = [Pen("Ink", 0.3, "#000000"), Pen("Lemon 0.5", 0.5, "#f5c400")]
    s.set_pen_library(library)
    analysis = s.analysis()
    owned = {(p.name, p.width_mm, p.color) for p in library}
    for suggestion in analysis["suggestions"]:
        for p in (suggestion["patch"] or {}).get("pens", []):
            assert (p["name"], p["width_mm"], p["color"]) in owned
    assert "pens_to_add" in [x["kind"] for x in analysis["suggestions"]]
    assert all(
        a["error"] < analysis["stats"]["palette_error"]
        for a in analysis["stats"]["palette_additions"]
    )
    with pytest.raises(ValueError):
        s.set_pen_library([Pen("Bad", 0.3, "red")])


def test_fit_meets_a_budget_and_applies_the_value():
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.set_canvas(Canvas(100, 75, 5))
    s.set_style("waves")
    s.update("tone", resolution=64)
    frames = []
    s.subscribe(frames.append)
    handle = s.fit("metres", 10)
    assert handle.wait() is RunState.done
    fit = handle.fit
    assert (fit["stage"], fit["key"], fit["metric"], fit["target"]) == (
        "waves",
        "spacing_mm",
        "metres",
        10,
    )
    # The same samples as the engine's pinned fit, since the image and page are the same.
    assert [x["value"] for x in fit["samples"]] == [2.0, 1.472, 0.655, 0.883, 0.843]
    assert s.settings["waves"]["spacing_mm"] == fit["value"] == 0.843
    assert s.result is handle.result
    assert s.stats().pen_down_mm / 1000 == fit["achieved"]
    assert [f.label for f in frames if f.stage == "fit"][0] == "trial 1 of 8: 7.36 m"
    assert s.run().wait() is RunState.done and s._handle.fit is None
    with pytest.raises(ValueError, match="places no dots"):
        s.fit("dots", 10)
    with pytest.raises(ValueError, match="unknown metric"):
        s.fit("hours", 10)
    with pytest.raises(RuntimeError, match="no image"):
        Session().fit("paths", 10)


def drawn(*pens, page=(200, 174), **split):
    """A session with ``MACHINE`` that has run a coarse grid on a page of size ``page``, which is
    the machine's drawable area unless given."""
    s = Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.set_canvas(Canvas(*page, 10))
    if pens:
        s.set_pens(pens)
    s.update("tone", resolution=64)
    s.update("points", spacing_mm=5)
    if split:
        s.update("split", **split)
    s.set_machine(MACHINE)
    s.run_blocking()
    return s


def test_a_machine_is_set_read_and_cleared_and_sets_the_speeds():
    s = Session()
    assert s.machine is None
    s.set_machine(MACHINE)
    assert s.machine == MACHINE
    assert (s.speeds.draw_mm_s, s.speeds.travel_mm_s) == (60, 100)
    assert s.speeds.pen_lift_s == pytest.approx(0.6)
    # No machine has no speeds to put back, so clearing leaves them as they are.
    s.speeds.draw_mm_s = 30.0
    s.set_machine(None)
    assert s.machine is None and s.speeds.draw_mm_s == 30


def test_a_refused_machine_raises_and_changes_nothing():
    s = Session()
    s.set_machine(MACHINE)
    refused = (({"z_up_mm": 3}, "z_up_mm"), ({"draw_feed": 0}, "draw_feed"), ({"x": 1}, "x"))
    for change, word in refused:
        # The travel feed is changed too, so that a speed taken from the refused machine would show.
        with pytest.raises(ValueError, match=word):
            s.set_machine({**MACHINE, "travel_feed": 1, **change})
    assert s.machine == MACHINE and s.speeds.travel_mm_s == 100
    t = Session()
    with pytest.raises(ValueError, match="z_up_mm"):
        t.set_machine({**MACHINE, "z_up_mm": 3})
    assert t.machine is None and t.speeds.draw_mm_s == 50


def test_gcode_is_the_text_of_one_layer():
    s = drawn()
    text = s.gcode(0)
    assert "G1 Z4 F1200" in text and "M104 S0\nM140 S0" in text
    with pytest.raises(ValueError, match="no layer 1"):
        s.gcode(1)


def test_a_pen_lifted_by_gcode_lines_and_split_moves_reach_the_file():
    s = drawn()
    servo = {
        **MACHINE,
        "end_gcode": "M400",
        "pen_lift": "gcode",
        "pen_down_gcode": "M280 P0 S30",
        "pen_up_gcode": "M280 P0 S90",
        "max_segment_mm": 1,
    }
    s.set_machine(servo)
    assert s.machine == servo
    # The lift is the two 50 ms dwells alone.
    assert s.speeds.pen_lift_s == pytest.approx(0.1)
    text = s.gcode(0)
    assert "M280 P0 S30\nG4 P50\nG1 F3600\n" in text and text.endswith("G4 P50\nG1 F3600\nM400\n")
    assert not any("Z" in line for line in text.splitlines() if not line.startswith(";"))
    with pytest.raises(ValueError, match="pen_up_gcode must have a line"):
        s.set_machine({**servo, "pen_up_gcode": ""})


def test_export_gcode_writes_a_file_for_each_pen_layer(tmp_path):
    pens = (Pen("Light 0.3", 0.3, "#808080"), Pen("Stift é/2", 0.3, "#000000"))
    s = drawn(*pens, mode="light_to_dark")
    out = tmp_path / "a" / "b"
    files = s.export_gcode(out)
    # Each run of characters outside [A-Za-z0-9_.-] is one underscore.
    assert [f.name for f in files] == ["plot_1_Light_0.3.gcode", "plot_2_Stift_2.gcode"]
    assert sorted(out.iterdir()) == files
    for i, file in enumerate(files):
        raw = file.read_bytes()
        assert raw.decode("utf-8") == s.gcode(i) and b"\r" not in raw


def test_export_gcode_refuses_an_existing_file_before_writing_any(tmp_path):
    s = drawn(Pen("A", 0.3, "#808080"), Pen("B", 0.3, "#000000"), mode="light_to_dark")
    assert len(s.export_gcode(tmp_path / "new", overwrite=False)) == 2
    first, second = s.export_gcode(tmp_path)
    first.unlink()
    second.write_text("keep")
    with pytest.raises(FileExistsError, match=second.name):
        s.export_gcode(tmp_path, overwrite=False)
    assert not first.exists() and second.read_text() == "keep"
    assert s.export_gcode(tmp_path) == [first, second] and second.read_text() != "keep"


def test_gcode_needs_a_machine_a_result_and_a_page_that_fits(tmp_path):
    out = tmp_path / "g"
    s = Session()
    with pytest.raises(RuntimeError, match="no machine"):
        s.gcode(0)
    with pytest.raises(RuntimeError, match="no result"):
        s.export_gcode(out)
    s.set_machine(MACHINE)
    with pytest.raises(RuntimeError, match="no drawing"):
        s.gcode(0)
    s = drawn()
    s.set_machine(None)
    with pytest.raises(RuntimeError, match="no machine"):
        s.gcode(0)
    with pytest.raises(RuntimeError, match="no machine"):
        s.export_gcode(out)
    s = drawn(page=(210, 297))  # the default A4 page, which is larger than 200 by 174 mm
    with pytest.raises(ValueError, match="larger than the machine's"):
        s.gcode(0)
    with pytest.raises(ValueError, match="larger than the machine's"):
        s.export_gcode(out)
    assert not out.exists()


def test_hpgl_is_one_program_with_a_pen_for_each_layer(tmp_path):
    pens = (Pen("Light 0.3", 0.3, "#808080"), Pen("Dark 0.3", 0.3, "#000000"))
    s = drawn(*pens, mode="light_to_dark")
    text = s.hpgl()
    lines = text.splitlines()
    assert lines[:2] == ["IN;", "SP1;"] and lines[-2:] == ["PU;", "SP0;"] and "SP2;" in lines
    assert text == s.hpgl(40.0) and s.hpgl(10) != text
    # No machine is needed, and the file is ASCII with LF line ends.
    s.set_machine(None)
    file = tmp_path / "o.hpgl"
    s.export_hpgl(file, 20)
    raw = file.read_bytes()
    assert raw.decode("ascii") == s.hpgl(20) and b"\r" not in raw


def test_hpgl_needs_a_result_and_a_resolution_above_zero(tmp_path):
    file = tmp_path / "o.hpgl"
    with pytest.raises(RuntimeError, match="no drawing"):
        Session().hpgl()
    with pytest.raises(RuntimeError, match="no drawing"):
        Session().export_hpgl(file)
    s = drawn()
    for bad in (0, -1, 1000.5, float("nan"), float("inf")):
        with pytest.raises(ValueError, match="above 0 and at most 1000"):
            s.hpgl(bad)
        with pytest.raises(ValueError, match="above 0 and at most 1000"):
            s.export_hpgl(file, bad)
    assert s.hpgl(1000)
    assert not file.exists()


def test_a_preset_with_a_machine_brings_its_speeds(tmp_path):
    s = drawn()
    s.save_preset(tmp_path / "p.json")
    t = Session()
    t.set_image(_core.synthetic_gradient(64, 48))
    t.load_preset(tmp_path / "p.json")
    assert t.machine == MACHINE
    assert t.speeds.draw_mm_s == 60 and t.speeds.pen_lift_s == pytest.approx(0.6)
    # stats pushes the session's speeds into the engine, so they must be the machine's already.
    t.run_blocking()
    assert t.stats() == s.stats()
    Session().save_preset(tmp_path / "none.json")
    t.load_preset(tmp_path / "none.json")
    assert t.machine is None and t.speeds.draw_mm_s == 60
