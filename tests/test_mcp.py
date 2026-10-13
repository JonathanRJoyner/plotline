import base64
import json
import struct
import sys

import anyio
import pytest
from mcp import Client, StdioServerParameters
from pngs import write_png

from plotline import RunState, Session, _core, inks
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


def png(block):
    return base64.b64decode(block.data)


async def test_run_and_preview(tmp_path):
    path = tmp_path / "i.png"
    write_png(path, _core.synthetic_gradient(64, 48))
    async with Client(build_server(Session())) as c:
        await call(c, "load_image", path=str(path))
        await call(c, "set_page", width_mm=100, height_mm=100)
        r = await call(c, "run", settings={"tone": {"resolution": 64}})
        assert not r.is_error
        stats = json.loads(r.content[0].text)
        assert set(stats) == {"path_count", "pen_down_mm", "pen_up_mm", "estimated_seconds"}
        assert png(r.content[1]).startswith(b"\x89PNG")

        r = await call(c, "preview", region_mm=[0, 0, 50, 50], width_px=200)
        assert not r.is_error
        assert struct.unpack(">II", png(r.content[0])[16:24]) == (200, 200)

        for format, start in [("svg", b"<svg"), ("json", b"{"), ("png", b"\x89PNG")]:
            out = tmp_path / f"o.{format}"
            assert not (await call(c, "export", path=str(out), format=format)).is_error
            assert out.read_bytes().startswith(start)


async def test_refused_setting_changes_nothing():
    async with Client(build_server(Session())) as c:
        before = data(await call(c, "get_session"))
        r = await call(
            c, "run", style="hatch", settings={"tone": {"resolution": 64}, "nope": {"x": 1}}
        )
        assert r.is_error
        assert 'style "hatch" has no stage "nope"' in text(r)
        after = data(await call(c, "get_session"))
        assert (after["style"], after["settings"]) == (before["style"], before["settings"])


async def test_run_without_an_image_is_an_error():
    async with Client(build_server(Session())) as c:
        r = await call(c, "run")
        assert r.is_error
        assert "no image is loaded" in text(r)


async def test_preview_limits(tmp_path):
    path = tmp_path / "i.png"
    write_png(path, _core.synthetic_gradient(64, 48))
    async with Client(build_server(Session())) as c:
        await call(c, "load_image", path=str(path))
        assert not (await call(c, "run")).is_error
        assert (await call(c, "preview", width_px=50)).is_error
        assert (await call(c, "preview", region_mm=[0, 0, 1, 100])).is_error
        assert (await call(c, "preview", region_mm=[0, 0, 0, 10])).is_error
        assert (await call(c, "preview", region_mm=[0, 0, 1e-300, 1])).is_error  # no overflow

        await call(c, "set_page", width_mm=10, height_mm=1000, margin_mm=0)
        r = await call(c, "run")
        assert not r.is_error
        assert struct.unpack(">II", png(r.content[1])[16:24])[0] == 400
        r = await call(c, "preview")
        assert r.is_error
        assert "16000000" in text(r)
        assert "400" in text(r)


async def test_progress_and_cancel():
    session = Session()
    session.set_image(_core.synthetic_gradient(8, 8))
    messages = []

    async def progress(value, total, message):
        messages.append(message)

    async with Client(build_server(session)) as c:
        args = {"style": "_slow", "settings": {"b": {"frames": 20, "delay_ms": 10}}}
        r = await c.call_tool("run", args, progress_callback=progress)
        assert not r.is_error
        assert any(m.startswith("b: ") for m in messages)

        with anyio.move_on_after(0.5):
            await call(c, "run", style="_slow", settings={"b": {"frames": 200, "delay_ms": 50}})
        for _ in range(200):
            if session.state is RunState.cancelled:
                break
            await anyio.sleep(0.05)
        assert session.state == RunState.cancelled


async def test_a_second_run_cancels_the_first():
    session = Session()
    session.set_image(_core.synthetic_gradient(8, 8))
    results = {}
    async with Client(build_server(session)) as c:

        async def run(key, frames, delay_ms):
            settings = {"b": {"frames": frames, "delay_ms": delay_ms}}
            results[key] = await call(c, "run", style="_slow", settings=settings)

        async with anyio.create_task_group() as group:
            group.start_soon(run, "first", 100, 20)
            await anyio.sleep(0.2)
            group.start_soon(run, "second", 5, 0)
    assert results["first"].is_error
    assert "the run was cancelled" in text(results["first"])
    assert not results["second"].is_error


async def test_stdio_server_lists_the_tools():
    # The real thing: `plotline mcp` as a subprocess speaking the protocol over its pipes.
    command = ["-c", "from plotline.cli.main import main; main()", "mcp"]
    # A child that hangs at start-up must fail the test, not hang the suite.
    with anyio.fail_after(30):
        async with Client(StdioServerParameters(command=sys.executable, args=command)) as c:
            names = {tool.name for tool in (await c.list_tools()).tools}
    assert names == {
        "list_styles",
        "describe_style",
        "load_image",
        "set_page",
        "set_pens",
        "run",
        "preview",
        "get_session",
        "load_preset",
        "export",
    }
