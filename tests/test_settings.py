import pytest
from pydantic import ValidationError

from plotline.core.settings import Settings, setting


class M(Settings):
    x: float = setting(2.0, help="Spacing.", min=0.2, max=20, step=0.1, unit="mm", advanced=True)
    y: int = setting(3, help="Count.")


def test_schema_carries_extras():
    p = M.model_json_schema()["properties"]["x"]
    assert (p["description"], p["minimum"], p["maximum"], p["default"]) == (
        "Spacing.",
        0.2,
        20,
        2.0,
    )
    assert (p["step"], p["unit"], p["advanced"]) == (0.1, "mm", True)


def test_plain_setting_is_not_advanced_and_has_no_step():
    p = M.model_json_schema()["properties"]["y"]
    assert p["advanced"] is False and "step" not in p


def test_unknown_key_rejected():
    with pytest.raises(ValidationError):
        M(z=1)


def test_frozen():
    with pytest.raises(ValidationError):
        M().x = 1.0


def test_lax_coercion_and_range():
    assert M.model_validate({"x": "0.5"}).x == 0.5
    with pytest.raises(ValidationError):
        M(x=100)
