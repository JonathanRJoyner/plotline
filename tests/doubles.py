"""Test helpers shared by the test modules. Import them as ``from doubles import ...``."""

import time
from collections.abc import Generator, Mapping
from typing import Any

from plotline.core.settings import Settings, setting
from plotline.core.stage import Frame, RunContext, Stage


def collect(gen: Generator[Frame, None, Any]) -> tuple[list[Frame], Any]:
    """Drain a stage generator and return its frames and its return value."""
    frames: list[Frame] = []
    try:
        while True:
            frames.append(next(gen))
    except StopIteration as stop:
        return frames, stop.value


class SpySettings(Settings):
    knob: int = setting(0, help="Test knob.")


class Spy(Stage):
    """A stage for tests. It yields ``frames`` frames, sleeping ``delay_s`` before each, and
    returns ``value``, which is called with the inputs when it is callable. ``fail`` is raised
    after the first frame, and ``runs`` counts how often the stage ran."""

    settings_model = SpySettings

    def __init__(
        self,
        name: str,
        requires: tuple[str, ...],
        provides: str,
        value: Any = None,
        frames: int = 1,
        delay_s: float = 0.0,
        fail: Exception | None = None,
    ) -> None:
        self.name = name
        self.label = name.title()
        self.requires = requires
        self.provides = provides
        self.value = value
        self.frames = frames
        self.delay_s = delay_s
        self.fail = fail
        self.runs = 0

    def run(
        self, inputs: Mapping[str, Any], settings: Settings, ctx: RunContext
    ) -> Generator[Frame, None, Any]:
        self.runs += 1
        for i in range(self.frames):
            if self.delay_s > 0:
                time.sleep(self.delay_s)
            yield Frame((i + 1) / self.frames, f"{i + 1}/{self.frames}")
            if self.fail is not None:
                raise self.fail
        return self.value(inputs) if callable(self.value) else self.value
