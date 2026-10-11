"""Tests for styles: validation, artifact resolution by kind, schema and defaults."""

import pytest
from doubles import Spy, SpySettings

from plotline.core.pipeline import Style


def spy(name, req, prov):
    return Spy(name, req, prov)


def test_resolves_nearest_upstream_provider():
    st = Style(
        "t",
        "T",
        "d",
        (
            spy("a", ("image",), "paths"),
            spy("b", ("paths",), "paths"),
            spy("c", ("paths", "canvas"), "paths"),
        ),
    )
    assert st.sources == {
        "a": {"image": "image"},
        "b": {"paths": "a"},
        "c": {"paths": "b", "canvas": "canvas"},
    }


def test_stage_providing_a_root_kind_shadows_it():
    st = Style(
        "t", "T", "d", (spy("crop", ("image",), "image"), spy("b", ("image", "pens"), "paths"))
    )
    assert st.sources == {"crop": {"image": "image"}, "b": {"image": "crop", "pens": "pens"}}


@pytest.mark.parametrize(
    "stages",
    [
        (spy("a", ("image",), "paths"), spy("a", ("paths",), "paths")),  # duplicate name
        (spy("canvas", ("image",), "paths"),),  # root kind as name
        (spy("a", ("points",), "paths"),),  # unresolved
        (spy("a", ("image",), "points"),),  # wrong final kind
    ],
)
def test_invalid_styles_are_rejected(stages):
    with pytest.raises(ValueError):
        Style("t", "T", "d", stages)


def test_style_without_stages_is_rejected():
    with pytest.raises(ValueError, match="no stages"):
        Style("t", "T", "d", ())


def test_schema_and_defaults():
    st = Style("t", "T", "d", (spy("a", ("image",), "paths"),))
    sch = st.schema()
    assert sch["name"] == "t" and sch["stages"][0]["name"] == "a"
    assert "knob" in sch["stages"][0]["schema"]["properties"]
    assert st.defaults() == {"a": SpySettings()}
