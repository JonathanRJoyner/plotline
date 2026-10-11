"""Tests for the style registry: the built-in style, unknown names and entry-point discovery."""

from types import SimpleNamespace

import pytest

from plotline.core import registry
from plotline.core.pipeline import Style
from plotline.core.registry import get_style, list_styles
from plotline.core.stages.grid import DotRenderer, GridPlacer
from plotline.core.stages.tone import ToneStage
from plotline.core.styles import GRID


def other_style():
    return Style("x", "X", "d", (ToneStage(), GridPlacer(), DotRenderer()))


def broken():
    raise RuntimeError("the plugin failed to start")


def test_builtin_grid_is_listed():
    assert [s.name for s in list_styles()] == ["grid"] and get_style("grid") is GRID


def test_unknown_style_raises():
    with pytest.raises(KeyError):
        get_style("nope")


def test_entry_points_are_discovered_and_bad_ones_skipped(monkeypatch):
    extra = other_style()
    eps = [
        SimpleNamespace(name="x", load=lambda: extra),
        SimpleNamespace(name="bad", load=lambda: 42),
    ]
    monkeypatch.setattr(registry, "_entry_points", lambda: eps)
    with pytest.warns(UserWarning):
        names = [s.name for s in list_styles()]
    assert names == ["grid", "x"]


def test_unknown_style_error_names_the_style():
    with pytest.raises(KeyError, match="unknown style 'nope'"):
        get_style("nope")


def test_list_styles_returns_a_new_list_each_call():
    first = list_styles()
    first.clear()
    assert [s.name for s in list_styles()] == ["grid"]


def test_get_style_finds_entry_point_styles(monkeypatch):
    extra = other_style()
    eps = [SimpleNamespace(name="x", load=lambda: extra)]
    monkeypatch.setattr(registry, "_entry_points", lambda: eps)
    assert get_style("x") is extra and get_style("grid") is GRID


@pytest.mark.parametrize("load", [broken, lambda: 42], ids=["raises", "not a style"])
def test_a_bad_entry_point_is_skipped_and_named_in_the_warning(monkeypatch, load):
    eps = [SimpleNamespace(name="bad-ep", load=load)]
    monkeypatch.setattr(registry, "_entry_points", lambda: eps)
    with pytest.warns(UserWarning, match="bad-ep"):
        names = [s.name for s in list_styles()]
    assert names == ["grid"]


def test_entry_points_are_read_from_package_metadata_in_the_plotline_group(tmp_path, monkeypatch):
    # A package on sys.path with one entry point in our group and one in another group, both
    # pointing at the grid style, so a wrong group name gives one style too few or too many.
    info = tmp_path / "fake_dist-0.1.dist-info"
    info.mkdir()
    (info / "METADATA").write_text("Metadata-Version: 2.1\nName: fake-dist\nVersion: 0.1\n")
    (info / "entry_points.txt").write_text(
        "[plotline.styles]\nalias = plotline.core.styles:GRID\n\n"
        "[other.group]\nignored = plotline.core.styles:GRID\n"
    )
    monkeypatch.syspath_prepend(tmp_path)
    styles = list_styles()
    assert len(styles) == 2 and all(s is GRID for s in styles)
