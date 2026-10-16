"""Looks as the registry lists them, so that a test follows a look whose values were measured
again instead of hard-coding them."""

from typing import Any

from plotline import get_style, list_styles


def style_with_looks() -> tuple[str, list[dict[str, Any]]]:
    """The first listed style with a look besides balanced: its name and its looks, as the
    style's schema lists them."""
    for style in list_styles():
        looks = get_style(style.name).schema()["looks"]
        if len(looks) > 1:
            return style.name, looks
    raise AssertionError("no style has a look besides balanced")


def off_look(name: str, looks: list[dict[str, Any]]) -> tuple[str, str, Any]:
    """``(stage, key, value)``: a number a look of style ``name`` sets, with a value the setting
    accepts that none of the style's looks has, so an edit to it leaves the settings at no look."""
    stages = {
        s["name"]: s["schema"].get("properties", {}) for s in get_style(name).schema()["stages"]
    }
    for stage, values in looks[-1]["settings"].items():
        for key in values:
            prop = stages[stage][key]
            taken = [look["settings"][stage][key] for look in looks]
            for value in (prop.get("minimum"), prop.get("maximum")):
                if value is not None and value not in taken:
                    return stage, key, value
    raise AssertionError(f"no numeric setting of {name} is free of its looks")
