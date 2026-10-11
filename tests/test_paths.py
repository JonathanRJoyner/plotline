"""Tests for the path length helpers."""

import numpy as np

from plotline.core.paths import path_length, total_length, travel_length


def test_path_lengths():
    assert path_length(np.array([[0.0, 0.0], [3.0, 4.0]])) == 5
    assert path_length(np.array([[1.0, 1.0]])) == 0
    assert (
        total_length([np.array([[0.0, 0.0], [3.0, 4.0]]), np.array([[0.0, 0.0], [0.0, 1.0]])]) == 6
    )
    assert (
        travel_length([np.array([[3.0, 4.0], [3.0, 5.0]]), np.array([[3.0, 8.0]])], (0.0, 0.0)) == 8
    )


def test_polylines_single_paths_and_no_paths():
    assert path_length(np.array([[0.0, 0.0], [3.0, 4.0], [3.0, 10.0]])) == 11
    assert travel_length([np.array([[3.0, 4.0]])], (0.0, 0.0)) == 5
    assert total_length([]) == 0
    assert travel_length([], (1.0, 1.0)) == 0
