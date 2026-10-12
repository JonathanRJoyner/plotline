"""Tests for the SVG and JSON exporters."""

import json
import xml.etree.ElementTree as ET

import numpy as np

from plotline.core.export.paths_json import to_json_dict
from plotline.core.export.svg import to_svg
from plotline.core.types import Canvas, Layer, Pen, Result

SVG = "{http://www.w3.org/2000/svg}"


def result(name="Black 0.3"):
    return Result(
        Canvas(210, 297, 10),
        [
            Layer(
                Pen(name, 0.3, "#000000"),
                [np.array([[10.0, 10.0], [20.0, 15.5]]), np.array([[30.0, 30.0]])],
            )
        ],
    )


def test_svg_structure():
    root = ET.fromstring(to_svg(result()))
    assert root.tag == SVG + "svg"
    assert (root.get("width"), root.get("height"), root.get("viewBox")) == (
        "210mm",
        "297mm",
        "0 0 210 297",
    )
    g = root[0]
    keys = ("id", "data-pen", "fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin")
    assert {k: g.get(k) for k in keys} == {
        "id": "pen-1",
        "data-pen": "Black 0.3",
        "fill": "none",
        "stroke": "#000000",
        "stroke-width": "0.3",
        "stroke-linecap": "round",
        "stroke-linejoin": "round",
    }
    assert [p.get("d") for p in g] == [
        "M 10.000 10.000 L 20.000 15.500",
        "M 30.000 30.000 L 30.000 30.000",
    ]


def test_svg_escapes_pen_name_and_handles_empty_layers():
    root = ET.fromstring(to_svg(result('a"b<c&d')))
    assert root[0].get("data-pen") == 'a"b<c&d'
    empty = ET.fromstring(to_svg(Result(Canvas(100, 100), [Layer(Pen("p", 0.3, "#000000"), [])])))
    assert len(empty[0]) == 0


def test_json_matches_spec_example():
    assert to_json_dict(result()) == {
        "format": "plotline-paths",
        "version": 1,
        "units": "mm",
        "canvas": {"width_mm": 210, "height_mm": 297, "margin_mm": 10, "fit": "fit"},
        "layers": [
            {
                "pen": {"name": "Black 0.3", "width_mm": 0.3, "color": "#000000"},
                "paths": [[[10.0, 10.0], [20.0, 15.5]], [[30.0, 30.0]]],
            }
        ],
    }


def test_json_rounds_to_three_decimals_and_serialises():
    r = Result(Canvas(100, 100), [Layer(Pen("p", 0.3, "#000000"), [np.array([[1.23456, 2.0]])])])
    d = to_json_dict(r)
    assert d["layers"][0]["paths"] == [[[1.235, 2.0]]]
    assert json.dumps(d)
