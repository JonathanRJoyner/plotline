"""Style discovery: the built-in styles, then any that other packages register.

A package adds a style by exposing a ``Style`` through an entry point in the ``plotline.styles``
group, for example in its ``pyproject.toml``::

    [project.entry-points."plotline.styles"]
    my-style = "my_package.styles:MY_STYLE"

The stage API such a style is built from is unstable until 0.2.
"""

import warnings
from importlib import metadata
from typing import Any

from plotline.core.pipeline import Style
from plotline.core.styles import BUILTIN

ENTRY_POINT_GROUP = "plotline.styles"


def _entry_points() -> list[Any]:
    """The entry points in ``ENTRY_POINT_GROUP``, each with a ``name`` and a ``load()``. It is a
    function of its own so that tests can stand in for installed packages."""
    return list(metadata.entry_points(group=ENTRY_POINT_GROUP))


def list_styles() -> list[Style]:
    """The built-in styles, then the styles from entry points in the order they are found.

    The list is new on every call. An entry point whose ``load()`` raises, or returns anything
    but a ``Style``, is skipped with a ``UserWarning`` that names it. Names are not checked for
    uniqueness: ``get_style`` finds the first style of a name, so a built-in always wins.
    """
    styles = list(BUILTIN)
    for ep in _entry_points():
        try:
            style = ep.load()
            if not isinstance(style, Style):
                raise TypeError(f"expected a Style, got {type(style).__name__}")
        except Exception as exc:
            warnings.warn(
                f"skipping style entry point {ep.name!r}: {exc!r}", UserWarning, stacklevel=2
            )
        else:
            styles.append(style)
    return styles


def get_style(name: str) -> Style:
    """The style called ``name``. Raises ``KeyError`` when there is none."""
    for style in list_styles():
        if style.name == name:
            return style
    raise KeyError(f"unknown style {name!r}")
