"""The stage contract: frames, cancellation, the run context and the ``Stage`` base class."""

import threading
from abc import ABC, abstractmethod
from collections.abc import Generator, Mapping
from dataclasses import dataclass
from typing import Any

import numpy as np
from numpy.typing import NDArray

from plotline.core.settings import Settings


@dataclass(frozen=True, eq=False)
class Frame:
    """A snapshot a stage yields while it runs, for the preview.

    ``progress`` runs from 0 to 1 within the stage, and ``label`` says where the stage is, such as
    ``"row 12/40"``. ``points`` is an ``(N, 2)`` array and ``paths`` a list of such arrays, all in
    mm. The runner fills in ``stage`` with the name of the stage that yielded the frame. Arrays
    make equality ambiguous, so frames compare by identity.
    """

    progress: float
    label: str
    points: NDArray[np.float64] | None = None
    paths: list[NDArray[np.float64]] | None = None
    stage: str = ""


class Cancelled(Exception):
    """Raised by ``RunContext.check`` when the run was cancelled."""


class CancelToken:
    """A flag that asks a run to stop. It is safe to set from any thread."""

    def __init__(self) -> None:
        self._event = threading.Event()

    def set(self) -> None:
        self._event.set()

    def is_set(self) -> bool:
        return self._event.is_set()


class RunContext:
    """What the runner hands each stage: the cancellation check and seeded random numbers."""

    def __init__(self, token: CancelToken) -> None:
        self._token = token

    def check(self) -> None:
        """Raise ``Cancelled`` if the run was cancelled, and return otherwise."""
        if self._token.is_set():
            raise Cancelled

    def rng(self, seed: int) -> np.random.Generator:
        """A new generator seeded with ``seed``. Stages draw all randomness from here, so no
        global random state is involved and equal seeds give equal streams."""
        return np.random.default_rng(seed)


class Stage(ABC):
    """One step of a pipeline.

    A subclass sets the attributes below as class attributes or in ``__init__``. ``name`` is
    unique within a style and ``label`` is what UIs show. ``version`` is part of the cache key, so
    bump it when the algorithm changes. ``requires`` lists the artifact kinds the stage reads,
    ``provides`` is the kind it produces, and ``settings_model`` is the ``Settings`` subclass that
    declares its settings.
    """

    name: str
    label: str
    version: str = "1"
    requires: tuple[str, ...]
    provides: str
    settings_model: type[Settings]

    @abstractmethod
    def run(
        self, inputs: Mapping[str, Any], settings: Settings, ctx: RunContext
    ) -> Generator[Frame, None, Any]:
        """Yield a ``Frame`` as work progresses, then return the artifact. ``inputs`` maps each
        required kind to its artifact. The runner checks for cancellation after every frame, so
        yield at least one; a long loop that does not yield must call ``ctx.check()`` itself.

        ``inputs`` and the returned artifact are shared with the cache and with later runs. Never
        modify them in place: copy before changing anything. The runner marks a returned array,
        each array in a returned list or tuple, and the arrays held by the fields of a returned
        dataclass read-only, so an in-place write to one raises ``ValueError``. The root inputs
        belong to the caller and are not marked, but the same rule holds for them."""
