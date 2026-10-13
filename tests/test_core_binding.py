import threading
import time

import numpy as np
import pytest

from plotline import _core


def test_session_runs_grid_and_exposes_drawing():
    s = _core.Session()
    s.set_image(_core.synthetic_gradient(64, 48))
    s.update("tone", {"resolution": 64})
    run = s.start()
    frames = []
    while (f := run.next_frame()) is not None:
        frames.append(f)
    assert [f["stage"] for f in frames][-1] == "order" and s.state() == "done"
    d = s.drawing()
    assert d["layers"][0]["coords"].dtype == np.float64 and d["layers"][0]["offsets"][0] == 0
    assert s.svg().startswith("<svg") and s.json()["format"] == "plotline-paths"
    assert s.preview_png(100, False, False)[:4] == b"\x89PNG"


def test_validation_errors_are_value_errors():
    s = _core.Session()
    with pytest.raises(ValueError, match="resolution"):
        s.update("tone", {"resolution": "500"})
    with pytest.raises(ValueError):
        s.set_canvas({"width_mm": 0, "height_mm": 10, "margin_mm": 0, "fit": "fit"})


def test_styles_and_schema():
    assert [s["name"] for s in _core.list_styles()] == [
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
        "circles",
    ]
    assert (
        _core.describe("grid")["stages"][0]["schema"]["properties"]["resolution"]["default"] == 1000
    )


def test_inputs_convert_and_bad_ones_are_value_errors():
    s = _core.Session(1 << 20)
    img = _core.synthetic_radial(8, 6)
    s.set_image(img[:, ::-1])  # a strided view is copied in logical order
    assert np.array_equal(s.image(), img[:, ::-1])
    for bad in [img.astype(np.float64), img[:, :, 0], np.zeros((4, 4, 4), np.float32)]:
        with pytest.raises(ValueError):
            s.set_image(bad)
    s.update("tone", {"resolution": np.int64(64)})
    assert s.settings()["tone"]["resolution"] == 64
    for patch in [{"resolution": float("nan")}, {1: 2}, {"resolution": object()}]:
        with pytest.raises(ValueError):
            s.update("tone", patch)
    cyclic: dict[str, object] = {"resolution": 64}
    cyclic["self"] = cyclic
    with pytest.raises(ValueError, match="nest too deeply"):
        s.update("tone", cyclic)
    s.set_pens([{"name": "Red", "width_mm": 0.5, "color": "#ff0000"}])
    t = _core.Session()
    assert t.load_preset(s.preset()) == [] and t.pens() == s.pens()


def test_cancel_from_the_main_thread_does_not_wait_for_a_busy_run():
    s = _core.Session(1 << 20)
    s.set_image(_core.synthetic_flat(8, 8, 0.5))
    s.set_style("_slow")
    s.update("b", {"frames": 1000, "delay_ms": 5})
    run = s.start()
    frames: list[dict[str, object]] = []
    errors: list[BaseException] = []
    first = threading.Event()

    def drive() -> None:
        try:
            while (f := run.next_frame()) is not None:
                frames.append(f)
                first.set()
        except BaseException as e:  # re-raised in the test thread after the join
            errors.append(e)
        finally:
            first.set()

    worker = threading.Thread(target=drive)
    worker.start()
    assert first.wait(5)
    started = time.monotonic()
    run.cancel()
    assert time.monotonic() - started < 1
    worker.join(5)
    if errors:
        raise errors[0]
    assert not worker.is_alive()
    assert len(frames) < 1000 and run.state() == "cancelled"
