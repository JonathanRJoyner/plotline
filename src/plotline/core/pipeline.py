"""Styles: ordered stages whose inputs are resolved by artifact kind."""

from dataclasses import dataclass, field
from typing import Any

from plotline.core.settings import Settings
from plotline.core.stage import Stage

ROOT_KINDS: frozenset[str] = frozenset({"image", "canvas", "pens"})


@dataclass(frozen=True)
class Style:
    """A named recipe: stages that run in order, each reading artifacts by kind.

    Each kind in a stage's ``requires`` resolves to the nearest earlier stage that ``provides``
    it, or else to the root kind of that name, which the caller supplies (``ROOT_KINDS``).
    ``sources`` holds the result as ``{stage name: {kind: source name}}``, where a source name is
    a stage name or a root kind name. That is why a stage may not be named like a root kind.

    Construction raises ``ValueError`` when a stage name repeats, a stage is named like a root
    kind, a requirement has no provider, or the last stage does not provide ``paths`` or
    ``layers``.
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
