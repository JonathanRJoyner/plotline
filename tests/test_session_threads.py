"""Tests for threaded runs: handles, cancellation, subscribers and frame throttling."""

import threading
import time

import numpy as np
import pytest
from doubles import Spy
from synthetic import gradient

from plotline.core.pipeline import Style
from plotline.session.session import RunState, Session

PATHS = [np.array([[1.0, 1.0], [2.0, 2.0]])]


def slow(frames=100, delay=0.01):
    a = Spy("a", ("image",), "points", value=np.zeros((1, 2)))
    b = Spy("b", ("points",), "paths", value=PATHS, frames=frames, delay_s=delay)
    return Style("slow", "Slow", "d", (a, b)), a, b


def session(style, max_fps=1e9):
    s = Session(max_fps=max_fps)
    s.set_image(gradient(8, 8))
    s.set_style(style)
    return s


def test_run_completes_and_delivers_every_frame_when_unthrottled():
    style, a, b = slow(frames=20, delay=0)
    s = session(style)
    seen = []
    s.subscribe(seen.append)
    h = s.run()
    assert h.wait(5) == RunState.done and s.state == RunState.done and s.result is h.result
    assert sum(f.stage == "b" for f in seen) == 20


def test_throttle_always_delivers_final_frames():
    style, _, _ = slow(frames=20, delay=0)
    s = session(style, max_fps=1e-6)
    seen = []
    s.subscribe(seen.append)
    s.run().wait(5)
    assert [(f.stage, f.progress) for f in seen] == [("a", 1.0), ("b", 1.0)]


def test_cancel_from_another_thread():
    style, a, b = slow()
    s = session(style)
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
    assert len(s.cache) == 1 and b.runs == 1


def test_new_run_cancels_the_previous_one():
    style, _, _ = slow()
    s = session(style)
    h1 = s.run()
    time.sleep(0.05)
    h2 = s.run()
    assert h1.state == RunState.cancelled and h2.wait(5) == RunState.done


def test_error_is_stored_on_the_handle():
    style = Style("f", "F", "d", (Spy("a", ("image",), "paths", fail=RuntimeError("boom")),))
    s = session(style)
    h = s.run()
    assert h.wait(5) == RunState.error and isinstance(h.error, RuntimeError) and s.result is None


def test_unsubscribe_and_blocking_delivery():
    style, _, _ = slow(frames=3, delay=0)
    s = session(style)
    seen = []
    unsub = s.subscribe(seen.append)
    s.run_blocking()
    assert len(seen) >= 2
    unsub()
    seen.clear()
    s.run_blocking()
    assert seen == []


def test_a_subscriber_error_ends_the_run_in_error():
    style, _, _ = slow(frames=3, delay=0)
    s = session(style)
    s.subscribe(lambda frame: 1 / 0)
    h = s.run()
    assert h.wait(5) == RunState.error and isinstance(h.error, ZeroDivisionError)
    with pytest.raises(ZeroDivisionError):
        s.run_blocking()
    assert s.state == RunState.error and s.result is None


def test_run_blocking_stops_a_running_worker_first():
    style, _, _ = slow()
    s = session(style)
    started = threading.Event()
    s.subscribe(lambda frame: started.set())
    h = s.run()
    assert started.wait(5)
    s.set_style(Style("q", "Q", "d", (Spy("q", ("image",), "paths", value=PATHS),)))
    s.run_blocking()
    assert h.state == RunState.cancelled and s.state == RunState.done


def test_handle_cancel_and_wait_timeout():
    style, _, _ = slow()
    h = session(style).run()
    assert h.wait(0) == RunState.running
    h.cancel()
    assert h.wait(5) == RunState.cancelled


def test_a_subscriber_that_starts_a_run_gets_an_error_instead_of_a_hang():
    style, _, _ = slow(frames=3, delay=0)
    s = session(style)
    s.subscribe(lambda frame: s.run())  # would wait for the worker it runs on
    h = s.run()
    assert h.wait(5) == RunState.error and isinstance(h.error, RuntimeError)
    assert "own thread" in str(h.error)


def test_run_requires_an_image():
    with pytest.raises(RuntimeError):
        Session().run()
