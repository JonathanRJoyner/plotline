"""Styles and the runner.

A ``Style`` is ordered stages whose inputs are resolved by artifact kind. ``run_style`` drives a
style's stages through the stage cache, under keys that hash the content each stage depends on.
"""

import hashlib
import json
from collections.abc import Generator, Mapping
from dataclasses import asdict, dataclass, field, replace
from typing import Any

import numpy as np

from plotline.core.cache import StageCache
from plotline.core.settings import Settings
from plotline.core.stage import Frame, RunContext, Stage
from plotline.core.types import Layer, Result

ROOT_KINDS: frozenset[str] = frozenset({"image", "canvas", "pens"})


@dataclass(frozen=True)
class Style:
    """A named recipe: stages that run in order, each reading artifacts by kind.

    Each kind in a stage's ``requires`` resolves to the nearest earlier stage that ``provides``
    it, or else to the root kind of that name, which the caller supplies (``ROOT_KINDS``). A
    stage that provides a root kind, such as ``image``, replaces the root as the source of that
    kind for every later stage. ``sources`` holds the result as
    ``{stage name: {kind: source name}}``, where a source name is a stage name or a root kind
    name. That is why a stage may not be named like a root kind.

    Construction raises ``ValueError`` when a stage name repeats, a stage is named like a root
    kind, a requirement has no provider, there are no stages, or the last stage does not provide
    ``paths`` or ``layers``.
    """

    name: str
    label: str
    description: str
    stages: tuple[Stage, ...]
    sources: dict[str, dict[str, str]] = field(init=False, repr=False, compare=False)

    def __post_init__(self) -> None:
        names = [stage.name for stage in self.stages]
        for name in names:
            if names.count(name) > 1:
                raise ValueError(f"style {self.name!r}: stage name {name!r} is used more than once")
        for name in names:
            if name in ROOT_KINDS:
                raise ValueError(f"style {self.name!r}: stage {name!r} is named like a root kind")
        providers = {kind: kind for kind in ROOT_KINDS}
        sources: dict[str, dict[str, str]] = {}
        for stage in self.stages:
            sources[stage.name] = {}
            for kind in stage.requires:
                if kind not in providers:
                    raise ValueError(
                        f"style {self.name!r}: stage {stage.name!r} requires {kind!r}, "
                        "which no root kind or earlier stage provides"
                    )
                sources[stage.name][kind] = providers[kind]
            providers[stage.provides] = stage.name
        if not self.stages:
            raise ValueError(f"style {self.name!r} has no stages")
        last = self.stages[-1]
        if last.provides not in ("paths", "layers"):
            raise ValueError(
                f"style {self.name!r}: last stage {last.name!r} provides {last.provides!r}, "
                "not 'paths' or 'layers'"
            )
        object.__setattr__(self, "sources", sources)

    def schema(self) -> dict[str, Any]:
        """The style and each stage's settings as JSON Schema, for UIs and tool descriptions."""
        return {
            "name": self.name,
            "label": self.label,
            "description": self.description,
            "stages": [
                {
                    "name": stage.name,
                    "label": stage.label,
                    "schema": stage.settings_model.model_json_schema(),
                }
                for stage in self.stages
            ],
        }

    def defaults(self) -> dict[str, Settings]:
        """Each stage's settings at their defaults, keyed by stage name."""
        return {stage.name: stage.settings_model() for stage in self.stages}


def canonical_json(obj: Any) -> str:
    """``obj`` as JSON with sorted keys and no whitespace, so equal data gives equal text."""
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def _digest(*parts: Any) -> str:
    """The BLAKE2b hex digest (16 bytes) of the byte-like ``parts``, taken in order."""
    h = hashlib.blake2b(digest_size=16)
    for part in parts:
        h.update(part)
    return h.hexdigest()


def root_key(kind: str, value: Any) -> str:
    """The content hash of a root artifact, so equal content gives an equal key.

    An ``image`` hashes its dtype, its shape and its bytes. A ``canvas`` and ``pens`` hash their
    canonical JSON, with the pens as a list in order. Any other kind raises ``ValueError``.
    """
    if kind == "image":
        return _digest(
            str(value.dtype).encode(), str(value.shape).encode(), np.ascontiguousarray(value)
        )
    if kind == "canvas":
        return _digest(canonical_json(asdict(value)).encode())
    if kind == "pens":
        return _digest(canonical_json([asdict(pen) for pen in value]).encode())
    raise ValueError(f"{kind!r} is not a root kind")


def stage_key(stage: Stage, settings: Settings, input_keys: dict[str, str]) -> str:
    """The cache key of one stage run: the stage's class, name and version, its settings, and the
    keys of its inputs by kind. A change to any of them gives a new key. Any configuration that
    changes the output of a stage must live in its name, version or settings, because instance
    attributes are not part of the key."""
    return _digest(
        canonical_json(
            {
                "stage": f"{type(stage).__module__}.{type(stage).__qualname__}",
                "name": stage.name,
                "version": stage.version,
                "settings": settings.model_dump(mode="json"),
                "inputs": input_keys,
            }
        ).encode()
    )


def _freeze(value: Any) -> None:
    """Mark ``value`` read-only if it is an array, or each array in it if it is a list or tuple.

    Anything else is left as it is. A cached artifact is shared, so an in-place write to it should
    fail at once instead of corrupting later runs.
    """
    if isinstance(value, np.ndarray):
        value.flags.writeable = False
    elif isinstance(value, list | tuple):
        for item in value:
            if isinstance(item, np.ndarray):
                item.flags.writeable = False


def run_style(
    style: Style,
    settings: Mapping[str, Settings],
    roots: Mapping[str, Any],
    cache: StageCache,
    ctx: RunContext,
) -> Generator[Frame, None, Result]:
    """Run the stages of ``style`` in order, reusing cached outputs, and return the ``Result``.

    ``roots`` holds the caller's ``image``, ``canvas`` and ``pens`` and ``settings`` holds one
    ``Settings`` per stage name. A missing root or missing settings raises ``KeyError``.

    A stage whose key is in ``cache`` is not run and yields one ``Frame(1.0, "cached")``. Any other
    stage runs, yields its frames and stores its output. Each frame carries the stage's name, and
    ``ctx.check()`` runs after each one. A cancelled run therefore stops at a frame, stores
    nothing for the stage in progress and keeps the stages before it.

    The result is built from the last stage's output: layers as they are, or the paths, as a
    new list of the same arrays, on one layer of the first pen. Its tone map is the output of the
    last stage that provides ``tone``, if any.

    Everything else is shared, not copied: the inputs a stage receives and the artifact it
    returns are the objects held by ``cache``, handed to later runs and placed in the ``Result``.
    Stages and callers must never modify them in place, and must copy before changing anything.
    Before it stores an artifact the runner marks it read-only if it is an array, and each array
    in it if it is a list or tuple, so an in-place write to one of those raises ``ValueError``.
    That marking covers arrays and lists or tuples of arrays only, so other artifact types rely
    on the rule alone. The roots belong to the caller and are not marked, but the same rule holds
    for them.
    """
    artifacts: dict[str, Any] = {kind: roots[kind] for kind in sorted(ROOT_KINDS)}
    keys = {kind: root_key(kind, value) for kind, value in artifacts.items()}
    for stage in style.stages:
        sources = style.sources[stage.name]
        stage_settings = settings[stage.name]
        key = stage_key(stage, stage_settings, {kind: keys[src] for kind, src in sources.items()})
        value = cache.get(key)
        if value is None and key not in cache:
            inputs = {kind: artifacts[src] for kind, src in sources.items()}
            frames = stage.run(inputs, stage_settings, ctx)
            while True:
                try:
                    frame = next(frames)
                except StopIteration as stop:
                    value = stop.value
                    break
                yield replace(frame, stage=stage.name)
                ctx.check()
            _freeze(value)
            cache.put(key, value)
        else:
            yield Frame(1.0, "cached", stage=stage.name)
            ctx.check()
        keys[stage.name] = key
        artifacts[stage.name] = value
    last = style.stages[-1]
    if last.provides == "layers":
        layers = artifacts[last.name]
    else:
        layers = [Layer(roots["pens"][0], list(artifacts[last.name]))]
    tone = next((artifacts[s.name] for s in reversed(style.stages) if s.provides == "tone"), None)
    return Result(roots["canvas"], layers, tone)
