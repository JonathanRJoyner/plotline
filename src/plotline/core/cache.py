"""In-memory stage cache, bounded by estimated bytes with least-recently-used eviction."""

import sys
from collections import OrderedDict
from dataclasses import fields, is_dataclass
from typing import Any

import numpy as np


def estimate_bytes(value: Any) -> int:
    """Estimate the memory a cached value holds.

    Arrays count their ``nbytes``, lists and tuples the sum over their items, dataclass instances
    the sum over their fields, and anything else ``sys.getsizeof``. Object and container overhead
    is not counted, so values made of many tiny arrays are undercounted.
    """
    if isinstance(value, np.ndarray):
        return value.nbytes
    if isinstance(value, list | tuple):
        return sum(estimate_bytes(item) for item in value)
    if is_dataclass(value) and not isinstance(value, type):
        return sum(estimate_bytes(getattr(value, f.name)) for f in fields(value))
    return sys.getsizeof(value)


class StageCache:
    """Stage outputs by key, held in memory within a budget of ``max_bytes`` of estimated size.

    ``bytes`` is the sum of the stored estimates. ``get`` and ``put`` mark a key as most recently
    used; ``in`` does not. ``hits`` and ``misses`` count ``get`` calls and survive ``clear``. Not
    thread safe.
    """

    def __init__(self, max_bytes: int = 512 * 2**20) -> None:
        self._max_bytes = max_bytes
        self._items: OrderedDict[str, tuple[Any, int]] = OrderedDict()
        self.hits = 0
        self.misses = 0
        self.bytes = 0

    def get(self, key: str) -> Any | None:
        """Return the value stored under ``key``, or ``None`` if there is none.

        A stored ``None`` also reads as ``None``, so use ``in`` to tell it from a miss.
        """
        entry = self._items.get(key)
        if entry is None:
            self.misses += 1
            return None
        self.hits += 1
        self._items.move_to_end(key)
        return entry[0]

    def put(self, key: str, value: Any) -> None:
        """Store ``value`` under ``key``, replacing any old value, then evict to fit the budget.

        A value larger than ``max_bytes`` is not stored. The old value goes either way, so ``get``
        never returns a value that ``put`` replaced.
        """
        old = self._items.pop(key, None)
        if old is not None:
            self.bytes -= old[1]
        size = estimate_bytes(value)
        if size > self._max_bytes:
            return
        self._items[key] = (value, size)
        self.bytes += size
        while self.bytes > self._max_bytes:
            _, (_, evicted) = self._items.popitem(last=False)
            self.bytes -= evicted

    def __contains__(self, key: str) -> bool:
        return key in self._items

    def __len__(self) -> int:
        return len(self._items)

    def clear(self) -> None:
        """Drop every entry."""
        self._items.clear()
        self.bytes = 0
