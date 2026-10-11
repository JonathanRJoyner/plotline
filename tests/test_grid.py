"""Tests for the grid placer and the dot renderer."""

import numpy as np
import pytest
from doubles import collect

from plotline.core.stage import CancelToken, RunContext
from plotline.core.stages.grid import DotRenderer, DotSettings, GridPlacer, GridSettings
from plotline.core.types import ToneMap

CTX = RunContext(CancelToken())


def tone(ink, mm_per_px=1.0, origin_mm=(0.0, 0.0)):
    return ToneMap(np.asarray(ink, np.float32), mm_per_px, origin_mm)


def place(tone_map, **kw):
    return collect(GridPlacer().run({"tone": tone_map}, GridSettings(**kw), CTX))


def test_full_grid():
    frames, pts = collect(
        GridPlacer().run({"tone": tone(np.ones((10, 20)))}, GridSettings(spacing_mm=2.0), CTX)
    )
    assert pts.shape == (50, 2) and pts.dtype == np.float64 and pts[0] == pytest.approx([1, 1])
    assert len(frames) == 5 and frames[-1].progress == 1 and frames[-1].points.shape == (50, 2)


def test_threshold_keeps_dark_cells_only():
    ink = np.zeros((10, 20))
    ink[:, 10:] = 1
    _, pts = collect(GridPlacer().run({"tone": tone(ink)}, GridSettings(spacing_mm=2.0), CTX))
    assert pts.shape == (25, 2) and np.all(pts[:, 0] > 10)


def test_white_tone_gives_no_points():
    _, pts = collect(GridPlacer().run({"tone": tone(np.zeros((10, 20)))}, GridSettings(), CTX))
    assert pts.shape == (0, 2)


def test_dots_are_one_vertex_paths():
    frames, paths = collect(
        DotRenderer().run({"points": np.array([[1.0, 2.0], [3.0, 4.0]])}, DotSettings(), CTX)
    )
    assert [p.shape for p in paths] == [(1, 2), (1, 2)] and paths[1][0] == pytest.approx([3, 4])
    assert frames[-1].progress == 1 and len(frames[-1].paths) == 2


def test_threshold_is_inclusive():
    # 0.5 is exact in float32, so every cell sits exactly at the threshold.
    half = tone(np.full((10, 20), 0.5))
    assert place(half, threshold=0.5)[1].shape == (50, 2)
    assert place(half, threshold=0.51)[1].shape == (0, 2)


def test_frames_hold_the_points_placed_so_far():
    ink = np.zeros((10, 20))
    ink[:, 10:] = 1
    frames, pts = place(tone(ink), spacing_mm=2.0)
    assert [f.points.shape for f in frames] == [(5 * i, 2) for i in range(1, 6)]
    assert [f.progress for f in frames] == pytest.approx([0.2, 0.4, 0.6, 0.8, 1.0])
    assert all(np.array_equal(f.points, pts[: len(f.points)]) for f in frames)


def test_grid_follows_origin_and_scale_row_by_row():
    # 12 x 8 px at 0.5 mm/px cover (10, 20, 6, 4) mm: 3 columns and 2 rows of 2 mm cells.
    _, pts = place(tone(np.ones((8, 12)), 0.5, (10.0, 20.0)), spacing_mm=2.0)
    expected = [[11, 21], [13, 21], [15, 21], [11, 23], [13, 23], [15, 23]]
    assert pts == pytest.approx(np.array(expected))


def test_exact_multiples_keep_their_last_cell():
    # 2 px at 0.3 mm/px are 0.6 mm, and 0.6 / 0.2 is 2.9999999999999996 in floating point.
    _, pts = place(tone(np.ones((2, 2)), 0.3), spacing_mm=0.2)
    assert pts.shape == (9, 2)


@pytest.mark.parametrize("shape", [(3, 20), (10, 3)], ids=["zero rows", "zero columns"])
def test_a_map_without_cells_gives_one_empty_frame(shape):
    frames, pts = place(tone(np.ones(shape)), spacing_mm=4.0)
    assert len(frames) == 1 and frames[0].progress == 1 and frames[0].points.shape == (0, 2)
    assert pts.shape == (0, 2) and pts.dtype == np.float64


def test_no_points_give_no_dots_and_one_frame():
    frames, paths = collect(DotRenderer().run({"points": np.empty((0, 2))}, DotSettings(), CTX))
    assert paths == [] and len(frames) == 1 and frames[0].progress == 1 and frames[0].paths == []


def test_inputs_are_only_read():
    ink = np.ones((10, 20), np.float32)
    ink.flags.writeable = False  # an in-place write would raise ValueError
    points = np.array([[1.0, 2.0], [3.0, 4.0]])
    points.flags.writeable = False
    _, pts = place(tone(ink), spacing_mm=2.0)
    _, paths = collect(DotRenderer().run({"points": points}, DotSettings(), CTX))
    assert pts.shape == (50, 2) and np.concatenate(paths) == pytest.approx(points)
