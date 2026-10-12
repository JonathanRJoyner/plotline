"""Tests for threaded runs: handles, cancellation, subscribers and frame throttling."""

import threading
import time

import pytest

from plotline import RunState, Session, _core


def session(frames=100, delay_ms=10, max_fps=1e9, **values):
    """A session on the hidden ``_slow`` style, whose stage ``b`` gives ``frames`` frames with a
    pause of ``delay_ms`` before each."""
    s = Session(max_fps=max_fps)
    s.set_image(_core.synthetic_gradient(8, 8))
    s.set_style("_slow")
    s.update("b", frames=frames, delay_ms=delay_ms, **values)
    return s


def test_run_completes_and_delivers_every_frame_when_unthrottled():
    s = session(frames=20, delay_ms=0)
    seen = []
    s.subscribe(seen.append)
    h = s.run()
    assert h.wait(5) == RunState.done and s.state == RunState.done and s.result is h.result
    assert sum(f.stage == "b" for f in seen) == 20


def test_throttle_always_delivers_final_frames():
    s = session(frames=20, delay_ms=0, max_fps=1e-6)
    seen = []
    s.subscribe(seen.append)
    s.run().wait(5)
    assert [(f.stage, f.progress) for f in seen] == [("a", 1.0), ("b", 1.0)]


def test_cancel_from_another_thread():
    s = session()
    third = threading.Event()
    n = [0]

    def cb(frame):
        n[0] += 1
        if n[0] == 3:
            third.set()

    s.subscribe(cb)
    h = s.run()
    assert third.wait(5)
    s.cancel()
    assert h.wait(5) == RunState.cancelled and s.state == RunState.cancelled and s.result is None
    assert s.cache_stats()["entries"] == 1  # stage a, but nothing from the cancelled stage b


def test_new_run_cancels_the_previous_one():
    s = session()
    h1 = s.run()
    time.sleep(0.05)
    h2 = s.run()
    assert h1.state == RunState.cancelled and h2.wait(5) == RunState.done


def test_error_is_stored_on_the_handle():
    s = session(fail_after=1)
    h = s.run()
    assert h.wait(5) == RunState.error and isinstance(h.error, RuntimeError)
    assert "slow failed" in str(h.error) and s.result is None


def test_unsubscribe_and_blocking_delivery():
    s = session(frames=3, delay_ms=0)
    seen = []
    unsub = s.subscribe(seen.append)
    s.run_blocking()
    assert len(seen) >= 2
    unsub()
    seen.clear()
    s.run_blocking()
    assert seen == []


def test_a_subscriber_error_ends_the_run_in_error():
    s = session(frames=3, delay_ms=0)
    s.subscribe(lambda frame: 1 / 0)
    h = s.run()
    assert h.wait(5) == RunState.error and isinstance(h.error, ZeroDivisionError)
    with pytest.raises(ZeroDivisionError):
        s.run_blocking()
    assert s.state == RunState.error and s.result is None


def test_run_blocking_stops_a_running_worker_first():
    s = session()
    started = threading.Event()
    s.subscribe(lambda frame: started.set())
    h = s.run()
    assert started.wait(5)
    s.update("b", frames=1, delay_ms=0)
    s.run_blocking()
    assert h.state == RunState.cancelled and s.state == RunState.done


def test_handle_cancel_and_wait_timeout():
    h = session().run()
    assert h.wait(0) == RunState.running
    h.cancel()
    assert h.wait(5) == RunState.cancelled


def test_run_requires_an_image():
    with pytest.raises(ValueError, match="no image"):
        Session().run()
