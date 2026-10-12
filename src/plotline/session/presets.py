"""Presets: the style, canvas, pens and settings of a session as a small, versioned JSON file.

A preset holds the format name ``plotline-preset``, an integer ``version``, the name of the style,
the canvas, the pens and the settings of each stage. ``preset_dict`` and ``apply_preset`` work on
that dict, which is also what a front end sends over the wire, and ``save_preset`` and
``load_preset`` put it in a file.

Loading checks the format, brings a file of an older version up to ``PRESET_VERSION`` through the
``MIGRATIONS`` chain, then sets the style, canvas and pens and validates the settings stage by
stage. A stage or key that the style does not have is dropped with a ``UserWarning``, and a key
that the file leaves out keeps the default.
"""

from __future__ import annotations

import json
import os
import warnings
from collections.abc import Callable
from dataclasses import asdict
from pathlib import Path
from typing import TYPE_CHECKING, Any

from plotline.core.types import Canvas, Pen

if TYPE_CHECKING:
    from plotline.session.session import Session

PRESET_FORMAT = "plotline-preset"
PRESET_VERSION = 1

# Maps a version to the function that upgrades a preset of that version to the next one. Whoever
# raises PRESET_VERSION adds the entry for the version it leaves behind.
MIGRATIONS: dict[int, Callable[[dict[str, Any]], dict[str, Any]]] = {}


def preset_dict(session: Session) -> dict[str, Any]:
    """The preset of ``session``, as a dict of plain values that ``json.dumps`` accepts."""
    return {
        "format": PRESET_FORMAT,
        "version": PRESET_VERSION,
        "style": session.style.name,
        "canvas": asdict(session.canvas),
        "pens": [asdict(pen) for pen in session.pens],
        "settings": {name: s.model_dump(mode="json") for name, s in session.settings.items()},
    }


def apply_preset(session: Session, data: dict[str, Any]) -> None:
    """Set the style, canvas, pens and settings of ``session`` from the preset ``data``.

    A file of an older version is brought up to date first, by ``MIGRATIONS[v]`` for each ``v``
    from its version up to ``PRESET_VERSION``. Setting the style resets every stage to its
    defaults, so a key that the preset leaves out keeps the default. A stage or key that the style
    does not have is dropped with one ``UserWarning`` each.

    Raises ``ValueError``, before anything is changed, when ``format`` is not ``plotline-preset``
    or ``version`` is not an integer, is above ``PRESET_VERSION`` (a newer build wrote the file)
    or has no migration. Other bad data raises where it is found, after the earlier steps have
    been applied: ``KeyError`` for an unknown style or a missing section, ``TypeError`` or
    ``ValueError`` for a bad canvas or pen and ``pydantic.ValidationError`` for a bad setting.
    """
    if not isinstance(data, dict) or data.get("format") != PRESET_FORMAT:
        raise ValueError(f"not a preset: its format must be {PRESET_FORMAT!r}")
    version = data.get("version")
    if type(version) is not int or version > PRESET_VERSION:
        raise ValueError(
            f"unsupported preset version {version!r}: this build reads up to {PRESET_VERSION}"
        )
    for v in range(version, PRESET_VERSION):
        if v not in MIGRATIONS:
            raise ValueError(f"cannot read preset version {version}: no migration from {v}")
        data = MIGRATIONS[v](data)
    session.set_style(data["style"])
    session.set_canvas(Canvas(**data["canvas"]))
    session.set_pens([Pen(**pen) for pen in data["pens"]])
    for stage, values in data["settings"].items():
        if stage not in session.settings:
            warnings.warn(f"ignoring unknown stage {stage!r}", UserWarning, stacklevel=2)
            continue
        known = type(session.settings[stage]).model_fields
        kept = {}
        for key, value in values.items():
            if key in known:
                kept[key] = value
            else:
                message = f"ignoring unknown setting {key!r} of stage {stage!r}"
                warnings.warn(message, UserWarning, stacklevel=2)
        session.update(stage, **kept)


def save_preset(session: Session, path: str | os.PathLike[str]) -> None:
    """Write the preset of ``session`` to ``path`` as UTF-8 JSON with LF line ends, indented by
    two spaces so that it reads and diffs well."""
    text = json.dumps(preset_dict(session), indent=2) + "\n"
    Path(path).write_text(text, encoding="utf-8", newline="\n")


def load_preset(session: Session, path: str | os.PathLike[str]) -> None:
    """Read the preset file at ``path`` and apply it to ``session``. See ``apply_preset``."""
    apply_preset(session, json.loads(Path(path).read_text(encoding="utf-8")))
