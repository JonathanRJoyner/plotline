"""The built-in styles, read from the engine's registry."""

from plotline import _core
from plotline.types import Style


def list_styles() -> list[Style]:
    """The styles a user can pick, in the engine's order; hidden test styles are left out."""
    return [Style(**s) for s in _core.list_styles()]


def get_style(name: str) -> Style:
    """The style called ``name``, hidden ones included. Raises ``ValueError`` for an unknown
    name."""
    schema = _core.describe(name)
    return Style(schema["name"], schema["label"], schema["description"])
