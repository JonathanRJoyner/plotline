"""Declared settings: the base model for a stage's settings and the ``setting()`` helper."""

from typing import Any

from pydantic import BaseModel, ConfigDict, Field


class Settings(BaseModel):
    """Base class for a stage's settings. Frozen so instances can serve as cache keys, and extra
    keys are an error so that a typo fails loudly."""

    model_config = ConfigDict(frozen=True, extra="forbid")


def setting(
    default: Any,
    *,
    help: str,
    min: float | None = None,
    max: float | None = None,
    step: float | None = None,
    unit: str | None = None,
    advanced: bool = False,
    label: str | None = None,
) -> Any:
    """Declare one setting as a pydantic field. ``min`` and ``max`` are inclusive bounds, and
    ``None`` leaves that side open. The help text and UI hints go into the JSON Schema."""
    extra: dict[str, Any] = {"advanced": advanced}
    if step is not None:
        extra["step"] = step
    if unit is not None:
        extra["unit"] = unit
    if label is not None:
        extra["label"] = label
    return Field(default, description=help, ge=min, le=max, json_schema_extra=extra)
