import base64
import json
import struct
import sys
import threading
import time
from functools import partial

import anyio
import pytest
from looks import off_look, style_with_looks
from machines import MACHINE
from mcp import Client, StdioServerParameters
from pngs import write_png

from plotline import RunState, Session, _core, inks, list_styles
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


async def until(condition, seconds=10.0):
    """Yield to the event loop until `condition()` holds, and fail rather than hang after
    `seconds`."""
    with anyio.fail_after(seconds):
        while not condition():
            await anyio.sleep(0.005)


async def test_lists_and_describes_styles():
    async with Client(build_server(Session())) as c:
        styles = data(await call(c, "list_styles"))
        assert "grid" in [s["name"] for s in styles]
        assert set(styles[0]) == {"name", "label", "description"}
        assert "stages" in data(await call(c, "describe_style", style="grid"))
        looks = data(await call(c, "describe_style", style=style_with_looks()[0]))["looks"]
        assert looks[0]["name"] == "balanced" and len(looks) > 1
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


async def test_set_machine_sets_the_page_and_clears():
    async with Client(build_server(Session())) as c:
        await call(c, "set_page", width_mm=300, height_mm=300, margin_mm=7, fit="fill")
        r = await call(c, "set_machine", machine=MACHINE)
        assert not r.is_error, text(r)
        page = {"width_mm": 200, "height_mm": 174, "margin_mm": 7, "fit": "fill"}
        assert data(r) == {"machine": MACHINE, "page": page}
        s = data(await call(c, "get_session"))
        assert (s["machine"], s["page"]) == (MACHINE, page)

        r = await call(c, "set_machine", machine=None)
        assert data(r) == {"machine": None, "page": page}  # the page stays
        assert data(await call(c, "get_session"))["machine"] is None


async def test_a_refused_machine_changes_nothing():
    session = Session()
    async with Client(build_server(session)) as c:
        before = data(await call(c, "get_session"))
        assert before["machine"] is None

        # The machine is refused, with no machine before and with one.
        bad = {**MACHINE, "z_up_mm": 2}
        for first in (None, MACHINE):
            await call(c, "set_machine", machine=first)
            before = data(await call(c, "get_session"))
            r = await call(c, "set_machine", machine=bad)
            assert r.is_error
            assert "z_up_mm" in text(r)
            assert data(await call(c, "get_session")) == before

        # The page is refused: a margin of 90 mm leaves no height in the machine's 174 mm.
        await call(c, "set_machine", machine=None)
        await call(c, "set_page", width_mm=300, height_mm=300, margin_mm=90)
        before, speeds = data(await call(c, "get_session")), session.speeds
        r = await call(c, "set_machine", machine=MACHINE)
        assert r.is_error
        assert data(await call(c, "get_session")) == before
        assert session.speeds == speeds


async def test_export_gcode(tmp_path):
    session = Session()
    session.set_image(_core.synthetic_gradient(64, 48))
    out = tmp_path / "gcode"
    async with Client(build_server(session)) as c:
        r = await call(c, "export", path=str(out), format="gcode")
        assert r.is_error
        assert "run the style first" in text(r)

        settings = {"tone": {"resolution": 64}, "points": {"spacing_mm": 5}}
        assert not (await call(c, "run", settings=settings)).is_error
        r = await call(c, "export", path=str(out), format="gcode")
        assert r.is_error
        assert "machine" in text(r)
        assert not out.exists()

        # The result keeps the page it was drawn on, so it must run again on the machine's.
        assert not (await call(c, "set_machine", machine=MACHINE)).is_error
        r = await call(c, "export", path=str(out), format="gcode")
        assert r.is_error
        assert "larger than the machine" in text(r)
        assert not (await call(c, "run")).is_error
        r = await call(c, "export", path=str(out), format="gcode")
        assert not r.is_error, text(r)
        file = out / "plot_1_Black_0.3.gcode"
        assert "G1 Z4 F1200" in file.read_text(encoding="utf-8")
        files = [{"path": str(file), "bytes": file.stat().st_size}]
        assert data(r) == {"path": str(out), "format": "gcode", "files": files}

        # The folder exists now, and the file with it.
        file.write_text("kept", encoding="utf-8")
        r = await call(c, "export", path=str(out), format="gcode")
        assert r.is_error
        assert "plot_1_Black_0.3.gcode" in text(r)
        assert file.read_text(encoding="utf-8") == "kept"
        r = await call(c, "export", path=str(out), format="gcode", overwrite=True)
        assert not r.is_error, text(r)
        assert "G1 Z4 F1200" in file.read_text(encoding="utf-8")


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
        keys = {"path_count", "pen_down_mm", "pen_up_mm", "estimated_seconds", "tone", "lint"}
        assert set(stats) == keys and isinstance(stats["lint"], list)
        assert set(stats["tone"]) == {"error", "ssim", "floor", "reach"}
        assert png(r.content[1]).startswith(b"\x89PNG")

        r = await call(c, "preview", region_mm=[0, 0, 50, 50], width_px=200)
        assert not r.is_error
        assert struct.unpack(">II", png(r.content[0])[16:24]) == (200, 200)
        r = await call(c, "preview", travel=True, source=True)
        assert not r.is_error
        assert struct.unpack(">II", png(r.content[0])[16:24]) == (1000, 1000)

        for format, start in [("svg", b"<svg"), ("json", b"{"), ("png", b"\x89PNG")]:
            out = tmp_path / f"o.{format}"
            assert not (await call(c, "export", path=str(out), format=format)).is_error
            assert out.read_bytes().startswith(start)


async def test_fit_returns_the_value_and_the_curve():
    session = Session()
    session.set_image(_core.synthetic_gradient(64, 48))
    async with Client(build_server(session)) as c:
        await call(c, "set_page", width_mm=100, height_mm=75, margin_mm=5)
        settings = {"tone": {"resolution": 64}}
        r = await call(c, "fit", metric="metres", target=10, style="waves", settings=settings)
        assert not r.is_error, text(r)
        fit = json.loads(r.content[0].text)
        assert (fit["stage"], fit["key"], fit["value"]) == ("waves", "spacing_mm", 0.843)
        assert [x["value"] for x in fit["samples"]] == [2.0, 1.472, 0.655, 0.883, 0.843]
        assert json.loads(r.content[1].text)["pen_down_mm"] / 1000 == fit["achieved"]
        assert png(r.content[2]).startswith(b"\x89PNG")
        assert data(await call(c, "get_session"))["settings"]["waves"]["spacing_mm"] == 0.843
        r = await call(c, "fit", metric="dots", target=10)
        assert r.is_error and "places no dots" in text(r)


async def test_optimize_suggests_a_density_and_changes_nothing():
    session = Session()
    session.set_image(_core.synthetic_gradient(64, 48))
    async with Client(build_server(session)) as c:
        await call(c, "set_page", width_mm=100, height_mm=75, margin_mm=5)
        settings = {"tone": {"resolution": 64}}
        r = await call(c, "optimize", style="hatch", settings=settings)
        assert not r.is_error, text(r)
        found = json.loads(r.content[0].text)
        assert (found["stage"], found["key"], found["current"]) == ("hatch", "spacing_mm", 1.0)
        assert len(found["trials"]) == 9 and "tone" in json.loads(r.content[1].text)
        assert data(await call(c, "get_session"))["settings"]["hatch"]["spacing_mm"] == 1.0
        r = await call(c, "optimize", metric="minutes")
        assert r.is_error and "both a metric and a target" in text(r)


async def test_refused_setting_changes_nothing():
    session = Session()
    session.set_image(_core.synthetic_gradient(8, 8))
    async with Client(build_server(session)) as c:
        # A value that is not the default shows that the old settings come back.
        assert not (await call(c, "run", settings={"tone": {"resolution": 64}})).is_error
        before = data(await call(c, "get_session"))
        assert before["settings"]["tone"]["resolution"] == 64
        refused = {"tone": {"resolution": 100}, "nope": {"x": 1}}
        for style in ("hatch", "grid"):
            r = await call(c, "run", style=style, settings=refused)
            assert r.is_error
            assert f'style "{style}" has no stage "nope"' in text(r)
            after = data(await call(c, "get_session"))
            assert (after["style"], after["settings"]) == (before["style"], before["settings"])


def small_page(c):
    """A page that keeps a run short, whatever the look."""
    return call(c, "set_page", width_mm=40, height_mm=30, margin_mm=2)


async def test_run_starts_from_a_look_and_get_session_reports_it():
    name, looks = style_with_looks()
    look = looks[-1]
    session = Session()
    session.set_image(_core.synthetic_gradient(64, 48))
    async with Client(build_server(session)) as c:
        await small_page(c)
        settings = {"tone": {"resolution": 64}}
        r = await call(c, "run", style=name, look=look["name"], settings=settings)
        assert not r.is_error, text(r)
        after = data(await call(c, "get_session"))
        assert after["look"] == look["name"]
        for stage, values in look["settings"].items():
            for key, value in values.items():
                assert after["settings"][stage][key] == value
        # settings apply after the look, so they adjust it, and the settings are then at no look.
        stage, key, value = off_look(name, looks)
        r = await call(c, "run", look=looks[0]["name"], settings={stage: {key: value}})
        assert not r.is_error, text(r)
        after = data(await call(c, "get_session"))
        assert after["settings"][stage][key] == value and after["look"] is None
        # reset comes before the look, and with no look puts the defaults back.
        r = await call(c, "run", reset=True, settings=settings)
        assert not r.is_error and data(await call(c, "get_session"))["look"] == "balanced"


async def test_a_refused_look_changes_nothing_and_runs_nothing():
    name, looks = style_with_looks()
    other = next(s.name for s in list_styles() if s.name != name)
    session = Session()
    session.set_image(_core.synthetic_gradient(64, 48))
    async with Client(build_server(session)) as c:
        await small_page(c)
        r = await call(
            c, "run", style=name, look=looks[-1]["name"], settings={"tone": {"resolution": 64}}
        )
        assert not r.is_error, text(r)
        before = data(await call(c, "get_session"))
        # A switch to another style comes first, so it has to be put back too.
        for tool, args in (
            ("run", {}),
            ("fit", {"metric": "minutes", "target": 1}),
            ("optimize", {}),
        ):
            r = await call(c, tool, style=other, look="nope", **args)
            assert r.is_error and 'has no look "nope"' in text(r)
            assert data(await call(c, "get_session")) == before


async def test_fit_and_optimize_start_from_a_look():
    name, looks = style_with_looks()
    look = looks[-1]
    session = Session()
    session.set_image(_core.synthetic_gradient(64, 48))
    async with Client(build_server(session)) as c:
        await small_page(c)
        settings = {"tone": {"resolution": 64}}
        r = await call(c, "optimize", style=name, look=look["name"], settings=settings)
        assert not r.is_error, text(r)
        # Optimize changes nothing more, so the look it started from still holds.
        assert data(await call(c, "get_session"))["look"] == look["name"]
        r = await call(c, "fit", metric="minutes", target=1, look=looks[0]["name"])
        assert not r.is_error, text(r)


async def test_a_new_style_keeps_changed_settings():
    session = Session()
    session.set_image(_core.synthetic_gradient(8, 8))
    async with Client(build_server(session)) as c:
        changed = {"tone": {"resolution": 64}, "split": {"mode": "light_to_dark"}}
        assert not (await call(c, "run", settings=changed)).is_error
        assert not (await call(c, "run", style="waves")).is_error
        after = data(await call(c, "get_session"))
        assert after["style"] == "waves"
        assert after["settings"]["tone"]["resolution"] == 64
        assert after["settings"]["split"]["mode"] == "light_to_dark"
        assert after["settings"]["tone"]["gamma"] == 1.9  # still at grid's default, so waves'
        r = await call(c, "run", reset=True, settings={"order": {"min_length_mm": 1.0}})
        assert not r.is_error
        after = data(await call(c, "get_session"))
        assert after["settings"]["tone"]["resolution"] == 1000
        assert after["settings"]["split"]["mode"] == "one_pen"
        assert after["settings"]["order"]["min_length_mm"] == 1.0  # applied after the reset


async def test_run_without_an_image_is_an_error():
    async with Client(build_server(Session())) as c:
        r = await call(c, "run")
        assert r.is_error
        assert "no image is loaded" in text(r)


async def test_analyse_image(tmp_path):
    path = tmp_path / "i.png"
    write_png(path, _core.synthetic_gradient(64, 48))
    async with Client(build_server(Session())) as c:
        r = await call(c, "analyse_image")
        assert r.is_error
        assert "no image is loaded" in text(r)
        await call(c, "load_image", path=str(path))
        r = await call(c, "analyse_image")
        assert not r.is_error
        assert set(data(r)) == {"stats", "suggestions"}


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


async def test_preview_waits_for_a_run_to_finish():
    session = Session()
    session.set_image(_core.synthetic_gradient(8, 8))
    async with Client(build_server(session)) as c:
        r = await call(c, "run", style="_slow", settings={"b": {"frames": 1, "delay_ms": 0}})
        assert not r.is_error
        long = {"b": {"frames": 200, "delay_ms": 50}}
        async with anyio.create_task_group() as group:
            group.start_soon(partial(call, c, "run", style="_slow", settings=long))
            await until(lambda: session.state is RunState.running)
            r = await call(c, "preview")
            assert r.is_error
            assert "wait for the run to finish" in text(r)
            group.cancel_scope.cancel()


async def test_a_superseded_run_returns_no_drawing():
    session = Session()
    session.set_image(_core.synthetic_gradient(8, 8))
    results = []
    # A latch, not the run state: a loaded runner can let the older run end before this task
    # looks, and a wait for `running` would then never end.
    started = threading.Event()
    session.subscribe(lambda frame: started.set())
    async with Client(build_server(session)) as c:

        async def older(frames):
            settings = {"b": {"frames": frames, "delay_ms": 10}}
            results.append(await call(c, "run", style="_slow", settings=settings))

        # The newer run has finished, or is still under way, when the older call looks again.
        # Each older run has its own frame count, so the stage cache cannot finish it at once.
        for older_frames, newer_frames in ((20, 2), (21, 200)):
            started.clear()
            async with anyio.create_task_group() as group:
                group.start_soon(older, older_frames)
                await until(started.is_set)
                # The call now sleeps between polls. Blocking the event loop keeps it asleep while
                # its run ends and a newer one starts.
                deadline = time.monotonic() + 10
                while session.state is RunState.running:
                    assert time.monotonic() < deadline, "the older run did not end"
                    time.sleep(0.005)
                session.update("b", frames=newer_frames)
                newer = session.run()
                if newer_frames == 2:
                    newer.wait()
            newer.cancel()
            newer.wait()
    assert [r.is_error for r in results] == [True, True]
    assert all("the run was cancelled" in text(r) for r in results)


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
        "analyse_image",
        "set_page",
        "set_pens",
        "set_machine",
        "run",
        "fit",
        "optimize",
        "preview",
        "get_session",
        "load_preset",
        "export",
    }
