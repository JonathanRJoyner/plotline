"""Tests for the stage cache: hit and miss counts, byte-bounded LRU eviction and size estimates."""

import sys

import numpy as np

from plotline.core.cache import StageCache, estimate_bytes
from plotline.core.types import Layer, Pen


def test_miss_then_hit():
    c = StageCache()
    assert c.get("k") is None and c.misses == 1
    c.put("k", np.zeros(10))
    assert c.get("k") is not None and c.hits == 1 and "k" in c and len(c) == 1 and c.bytes == 80


def test_lru_eviction():
    c = StageCache(max_bytes=200)
    c.put("a", np.zeros(10))
    c.put("b", np.zeros(10))
    c.get("a")
    c.put("c", np.zeros(10))
    assert "b" not in c and "a" in c and "c" in c and c.bytes == 160


def test_oversize_value_is_not_stored():
    c = StageCache(max_bytes=50)
    c.put("big", np.zeros(10))
    assert "big" not in c and c.bytes == 0


def test_estimate_bytes():
    assert estimate_bytes([np.zeros(10), np.zeros(10)]) == 160
    assert estimate_bytes(Layer(Pen("p", 0.3, "#000000"), [np.zeros((3, 2))])) >= 48
    assert estimate_bytes("abc") == sys.getsizeof("abc")


def test_estimate_bytes_counts_dataclass_fields_and_tuples():
    pen = Pen("p", 0.3, "#000000")
    assert estimate_bytes(Layer(pen, [np.zeros((3, 2))])) - estimate_bytes(Layer(pen, [])) == 48
    assert estimate_bytes((np.zeros(10), np.zeros(10))) == 160


def test_clear():
    c = StageCache()
    c.put("k", np.zeros(10))
    c.clear()
    assert len(c) == 0 and c.bytes == 0


def test_put_on_existing_key_replaces_it():
    c = StageCache(max_bytes=100)
    c.put("k", np.zeros(10))
    c.put("k", np.zeros(5))
    assert len(c) == 1 and c.bytes == 40 and c.get("k").shape == (5,)
    c.put("k", np.zeros(100))
    assert "k" not in c and c.bytes == 0
