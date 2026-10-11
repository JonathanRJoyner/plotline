"""Test helpers shared by the test modules. Import them as ``from doubles import ...``."""

from collections.abc import Generator
from typing import Any

from plotline.core.stage import Frame


def collect(gen: Generator[Frame, None, Any]) -> tuple[list[Frame], Any]:
    """Drain a stage generator and return its frames and its return value."""
    frames: list[Frame] = []
    try:
        while True:
            frames.append(next(gen))
    except StopIteration as stop:
        return frames, stop.value
