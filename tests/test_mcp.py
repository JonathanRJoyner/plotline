import json

import pytest
from mcp import Client
from pngs import write_png

from plotline import Session, _core, inks
from plotline.mcp_server import build_server

pytestmark = pytest.mark.anyio


@pytest.fixture
def anyio_backend() -> str:
    return "asyncio"


async def call(c, name, **args):
    return await c.call_tool(name, args)


def data(r):
    return json.loads(r.content[0].text)


def text(r):
    return r.content[0].text


async def test_lists_and_describes_styles():
    async with Client(build_server(Session())) as c:
        styles = data(await call(c, "list_styles"))
        assert "grid" in [s["name"] for s in styles]
        assert set(styles[0]) == {"name", "label", "description"}
        assert "stages" in data(await call(c, "describe_style", style="grid"))
        assert (await call(c, "describe_style", style="nope")).is_error


async def test_setup_tools_and_session(tmp_path):
    path = tmp_path / "i.png"
    write_png(path, _core.synthetic_gradient(64, 48))
    async with Client(build_server(Session())) as c:
        r = await call(c, "load_image", path=str(path))
        assert data(r) == {"path": str(path), "width_px": 64, "height_px": 48}
        r = await call(c, "set_page", width_mm=100, height_mm=100, margin_mm=5)
        assert data(r) == {"width_mm": 100, "height_mm": 100, "margin_mm": 5, "fit": "fit"}
        r = await call(c, "set_page", fit="fill")
        assert data(r) == {"width_mm": 100, "height_mm": 100, "margin_mm": 5, "fit": "fill"}
        r = await call(c, "set_pens", pens=[{"name": "B", "width_mm": 0.5, "color": "Blue"}])
        assert data(r) == [{"name": "B", "width_mm": 0.5, "color": inks()["blue"]}]
        s = data(await call(c, "get_session"))
        assert s["style"] == "grid"
        assert s["image"] == {"width_px": 64, "height_px": 48}
        assert s["state"] == "idle"
        assert s["stats"] is None
        assert s["page"]["fit"] == "fill"
        assert s["pens"][0]["color"] == inks()["blue"]
        assert "tone" in s["settings"]


async def test_refused_values_are_tool_errors():
    async with Client(build_server(Session())) as c:
        r = await call(c, "set_page", width_mm=0)
        assert r.is_error
        assert "width_mm" in text(r)
        assert (await call(c, "set_pens", pens=[])).is_error


async def test_file_paths(tmp_path, monkeypatch):
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    monkeypatch.setenv("HOME", str(tmp_path))
    write_png(tmp_path / "i.png", _core.synthetic_gradient(8, 8))
    async with Client(build_server(Session())) as c:
        r = await call(c, "export", path="~/o.preset.json", format="preset")
        assert not r.is_error
        out = tmp_path / "o.preset.json"
        assert out.is_file()
        assert data(r) == {"path": str(out), "format": "preset", "bytes": out.stat().st_size}

        r = await call(c, "export", path="~/o.preset.json", format="preset")
        assert r.is_error
        assert "overwrite=true" in text(r)
        r = await call(c, "export", path="~/o.preset.json", format="preset", overwrite=True)
        assert not r.is_error

        r = await call(c, "load_image", path="i.png")
        assert r.is_error
        assert "not an absolute path" in text(r)
        r = await call(c, "load_preset", path="o.preset.json")
        assert r.is_error
        assert "not an absolute path" in text(r)
        r = await call(c, "export", path="o.svg", format="svg")
        assert r.is_error
        assert "not an absolute path" in text(r)

        r = await call(c, "export", path=str(tmp_path / "o.svg"), format="svg")
        assert r.is_error
        assert "run the style first" in text(r)
        assert not (tmp_path / "o.svg").exists()

        r = await call(
            c, "export", path=str(tmp_path / "missing" / "o.preset.json"), format="preset"
        )
        assert r.is_error
        assert not (tmp_path / "missing").exists()


async def test_preset_round_trip_and_warnings(tmp_path):
    path = tmp_path / "p.preset.json"
    async with Client(build_server(Session())) as c:
        await call(c, "export", path=str(path), format="preset")
        preset = json.loads(path.read_text(encoding="utf-8"))
        preset["settings"]["bogus"] = {"x": 1}
        path.write_text(json.dumps(preset), encoding="utf-8")
        r = data(await call(c, "load_preset", path=str(path)))
        assert r["style"] == "grid"
        assert len(r["warnings"]) == 1
        assert "bogus" in r["warnings"][0]
