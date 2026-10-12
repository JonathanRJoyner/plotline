"""Tests for the ordering stage."""

import itertools

import numpy as np
import pytest
from doubles import collect

from plotline.core.paths import travel_length
from plotline.core.stage import CancelToken, RunContext
from plotline.core.stages.order import OrderSettings, OrderStage
from plotline.core.types import Canvas

CTX = RunContext(CancelToken())
CANVAS = Canvas(100, 100, 10)


def order(paths, **kw):
    return collect(OrderStage().run({"paths": paths, "canvas": CANVAS}, OrderSettings(**kw), CTX))


def test_greedy_order_and_reversal():
    p1 = np.array([[50.0, 50.0], [60.0, 50.0]])
    p2 = np.array([[12.0, 10.0], [20.0, 10.0]])
    p3 = np.array([[40.0, 10.0], [30.0, 10.0]])
    frames, out = order([p1, p2, p3])
    assert np.array_equal(out[0], p2)
    assert np.array_equal(out[1], p3[::-1])
    assert np.array_equal(out[2], p1)
    assert frames[-1].progress == 1 and len(frames[-1].paths) == 3


def test_min_length_drops_short_paths_but_keeps_dots_at_zero():
    dot = np.array([[5.0, 5.0]])
    short = np.array([[0.0, 0.0], [0.0, 0.5]])
    assert len(order([dot, short], min_length_mm=1.0)[1]) == 0
    assert len(order([dot, short])[1]) == 2


def test_no_length_is_measured_without_a_minimum_and_the_result_is_a_new_list(monkeypatch):
    def fail(path):
        raise AssertionError("measured a path that no minimum can drop")

    monkeypatch.setattr("plotline.core.stages.order.path_length", fail)
    paths = [np.array([[90.0, 90.0], [91.0, 90.0]]), np.array([[5.0, 5.0]])]
    out = order(paths, optimize=False)[1]
    assert out is not paths and all(a is b for a, b in zip(out, paths, strict=True))
    assert len(order(paths)[1]) == 2


def test_optimize_off_keeps_order():
    paths = [np.array([[90.0, 90.0], [91.0, 90.0]]), np.array([[10.0, 10.0], [11.0, 10.0]])]
    out = order(paths, optimize=False)[1]
    assert np.array_equal(out[0], paths[0])


def test_ordering_never_increases_travel():
    rng = np.random.default_rng(1)
    paths = [np.stack([p, p + [1.0, 0.0]]) for p in rng.uniform(10, 90, (50, 2))]
    assert travel_length(order(paths)[1], (10, 10)) <= travel_length(paths, (10, 10))


def test_empty_input():
    assert order([])[1] == []


def test_starts_at_the_drawing_rect_corner_not_the_page_corner():
    # The corner is (10, 10): (12, 12) lies 2.8 mm from it and (2, 2) lies 11.3 mm from it, while
    # from the page corner at (0, 0) the distances are 17.0 mm and 2.8 mm.
    near_corner = np.array([[12.0, 12.0]])
    near_page_corner = np.array([[2.0, 2.0]])
    out = order([near_page_corner, near_corner])[1]
    assert np.array_equal(out[0], near_corner)


@pytest.mark.parametrize("flip", [False, True], ids=["forwards", "backwards"])
@pytest.mark.parametrize("perm", list(itertools.permutations(range(3))))
def test_a_path_is_left_at_its_far_end_in_any_direction_and_input_order(perm, flip):
    # From (10, 10) the long path is entered at (30, 10) and left at (40, 10), so the dot at
    # (45, 10) follows it and the dot at (31, 12), close to where it was entered, comes last.
    long = np.array([[30.0, 10.0], [40.0, 10.0]])
    ahead = np.array([[45.0, 10.0]])
    behind = np.array([[31.0, 12.0]])
    given = [long[::-1] if flip else long, ahead, behind]
    out = order([given[i] for i in perm])[1]
    assert len(out) == 3
    assert np.array_equal(out[0], long)
    assert np.array_equal(out[1], ahead)
    assert np.array_equal(out[2], behind)


def test_a_far_path_is_found_when_the_nearest_endpoints_are_all_drawn():
    # Once the cluster is drawn, its ten endpoints are all nearer to the pen than the far path's
    # two, and ten is more than the first lookup lists.
    cluster = [np.array([[20.0 + i, 20.0], [20.0 + i, 21.0]]) for i in range(5)]
    far = np.array([[80.0, 80.0], [85.0, 80.0]])
    out = order([far, *cluster])[1]
    assert len(out) == 6 and np.array_equal(out[-1], far)


def test_every_path_comes_out_once_even_with_equal_distances():
    # Two dots on every point of a lattice give many ties, and both endpoints of a dot coincide.
    lattice = [np.array([[10.0 + 5 * i, 10.0 + 5 * j]]) for i in range(5) for j in range(8)] * 2
    out = order(lattice)[1]
    assert sorted(map(tuple, np.concatenate(out))) == sorted(map(tuple, np.concatenate(lattice)))


@pytest.mark.parametrize(
    ("n", "sizes"),
    [
        (0, [0]),
        (1, [1]),
        (3, [1, 2, 3]),
        (39, list(range(1, 40))),
        (40, list(range(2, 41, 2))),
        (100, list(range(5, 101, 5))),
    ],
)
def test_frames_hold_the_paths_ordered_so_far_about_every_twentieth_path(n, sizes):
    dots = [np.array([[10.0 + i, 10.0]]) for i in range(n)]
    frames, out = order(dots)
    assert [len(f.paths) for f in frames] == sizes
    assert [f.progress for f in frames[:-1]] == [size / n for size in sizes[:-1]]
    assert frames[-1].progress == 1
    # The dots lie on a line that starts at the corner, so they come out in input order, and each
    # frame holds the first dots of the result.
    assert all(np.array_equal(f.paths[i], dots[i]) for f in frames for i in range(len(f.paths)))
    assert len(out) == n


def test_without_optimize_short_paths_are_dropped_and_one_frame_holds_the_rest():
    a = np.array([[90.0, 90.0], [95.0, 90.0]])
    dot = np.array([[5.0, 5.0]])
    b = np.array([[10.0, 10.0], [15.0, 10.0]])
    frames, out = order([a, dot, b], optimize=False, min_length_mm=1.0)
    assert len(out) == 2 and np.array_equal(out[0], a) and np.array_equal(out[1], b)
    assert len(frames) == 1 and frames[0].progress == 1 and len(frames[0].paths) == 2


def test_inputs_are_only_read():
    paths = [np.array([[90.0, 90.0], [91.0, 90.0]]), np.array([[12.0, 10.0], [11.0, 10.0]])]
    for p in paths:
        p.flags.writeable = False  # an in-place write would raise ValueError
    given = list(paths)
    out = order(paths)[1]
    assert len(paths) == 2 and all(a is b for a, b in zip(paths, given, strict=True))
    assert np.array_equal(out[0], paths[1][::-1]) and np.array_equal(out[1], paths[0])
