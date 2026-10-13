# MCP server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `plotline mcp` serves the session over the Model Context Protocol on stdio, with ten tools and a zoomed preview, so an AI assistant can run the try, look, adjust loop.

**Architecture:** The engine's preview renderer gains an optional region in page mm, passed through `Session::preview_png`, the Python binding and `Session.preview_png`. A new module `src/plotline/mcp_server.py` builds an `MCPServer` (official SDK, version 2) whose `async` tools call one `Session`; `plotline mcp` imports it lazily and runs it on stdio. The SDK is an optional extra that the licence check covers.

**Tech Stack:** Rust (tiny-skia, thiserror, pyo3), Python 3.11+, `mcp` 2.3 (`MCPServer`, `Image`, `ToolError`, `Client`), anyio and its pytest plugin, click, pytest.

**Spec:** `docs/superpowers/specs/2026-10-12-mcp-server-design.md`

## Global Constraints

- SDK version range, verbatim: `mcp>=2.3,<3`, in `[project.optional-dependencies] mcp` and in the dev dependency group.
- SDK names, checked against 2.3.0: `from mcp.server.mcpserver import MCPServer, Context, Image`; `from mcp.server.mcpserver.exceptions import ToolError`; `from mcp import Client, StdioServerParameters`; `MCPServer(name, *, version=..., instructions=...)`; `server.run()` defaults to stdio; `Context.report_progress(progress, total=None, message=None)`; `Client.call_tool(name, arguments, progress_callback=...)`, whose callback is `async (progress, total, message)`.
- SDK behaviour, checked in a prototype: a tool returning `[str, Image(data=png, format="png")]` gives a text block then an image block; `ToolError("m")` gives `is_error=True` with text `Error executing tool <name>: m`; any other exception gives only `Error executing tool <name>`, which is why every tool maps errors itself; cancelling the awaiting client task cancels the tool's task; `async def` tools run on the event loop's thread.
- Every tool is `async def`. No session call happens off the event loop's thread except a run's stages, on the session's own worker.
- No algorithm code in `mcp_server.py` or the CLI. Validation of tool input (sizes, paths) is allowed and required.
- Don't name other software products or MCP clients anywhere: code, comments, docs, commits.
- Rust: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` stay clean. Python: `ruff check .`, `ruff format --check .` and `mypy src` stay clean.
- Files are written with LF line ends (the repo is `eol=lf`; on Windows use the Edit tool or `newline="\n"`).

## Review Focus

- Two `run` calls at once, as an assistant may send in parallel: the earlier one must end as a tool error "the run was cancelled" and the later one return the drawing. Test in Task 4.
- A path starting with `~/` must land in the user's home folder. Test in Task 3.
- Export into a folder that does not exist must be a tool error and must not leave a file. Test in Task 3.
- A preset that names a stage the style lacks must load and list the warning, not fail. Test in Task 3.
- The whole page at the default width, after `set_page` to 10 by 1000 mm, must be refused by the pixel cap instead of allocating gigabytes. Test in Task 4.

---

### Task 1: Engine: a preview region

**Files:**
- Modify: `crates/core/src/preview.rs`
- Modify: `crates/core/src/session.rs` (`preview_png` and its test call at line 413)
- Modify: `crates/py/src/session.rs` (`preview_png`)
- Modify: `crates/wasm/src/lib.rs` (`preview_png` passes `None`)
- Modify: `src/plotline/session.py`, `src/plotline/_core.pyi`
- Test: `crates/core/src/preview.rs` tests module, `tests/test_session.py`

**Interfaces:**
- Produces:
  - `PreviewError::Region([f64; 4])`, message `the preview region must have finite values and a positive width and height, got {0:?}`.
  - `pub fn render_png(d: &Drawing, width_px: u32, travel: bool, source: bool, region: Option<[f64; 4]>) -> Result<Vec<u8>, PreviewError>`
  - `pub fn render_rgba(d: &Drawing, width_px: u32, travel: bool, source: bool, region: Option<[f64; 4]>) -> (u32, u32, Vec<u8>)`; empty pixels for a refused region, as for a picture with no pixels.
  - `Session::preview_png(&self, width_px: u32, travel: bool, source: bool, region: Option<[f64; 4]>) -> Option<Result<Vec<u8>, PreviewError>>`
  - Python binding: `preview_png(width_px, travel, source, region=None)`; `PreviewError::Region` raises `ValueError`, other preview errors `RuntimeError` as now.
  - `Session.preview_png(self, width_px: int = 1000, travel: bool = False, source: bool = False, region: tuple[float, float, float, float] | None = None) -> bytes`

- [ ] **Step 1: Write the failing Rust tests** in `preview.rs`, with existing calls given `None`:
  - `a_region_zooms_and_offsets`: `drawing_100(vec![vec![[50.0, 20.0]]], 4.0, "#ff0000")`, `render_rgba(&d, 100, false, false, Some([40.0, 10.0, 20.0, 20.0]))` gives `(100, 100)`, red at `(50, 50)` and `(50, 58)`, white at `(50, 62)` (5 px per mm, dot radius 10 px).
  - `a_region_beside_the_paths_is_white`: the path and dot of `strokes_land_where_the_path_is_and_dots_are_pen_wide` with region `[60.0, 60.0, 10.0, 10.0]`: every pixel is `[255, 255, 255, 255]`.
  - `the_whole_page_as_a_region_matches_none`: on that drawing, `render_png(&d, 100, true, false, Some([0.0, 0.0, 100.0, 100.0]))` equals `render_png(&d, 100, true, false, None)`.
  - `a_bad_region_is_refused`: `[0.0, 0.0, 0.0, 10.0]`, `[0.0, 0.0, 10.0, -1.0]` and `[f64::NAN, 0.0, 10.0, 10.0]` each give `Err(PreviewError::Region(_))` from `render_png`.
- [ ] **Step 2: Run** `cargo test -p plotline-core preview`. Expected: compile errors on the new argument.
- [ ] **Step 3: Implement.** Replace `size()` with `fn view(d: &Drawing, width_px: u32, region: Option<[f64; 4]>) -> Result<View, PreviewError>`, where `View { x: f64, y: f64, px_per_mm: f64, height_px: u32 }`; `None` means `[0, 0, page width, page height]`. `render` takes the view; `to_px` subtracts `(view.x, view.y)` before scaling, and `draw_tone`'s translation does the same. Update the module doc's sentence on scale and origin. Pass the region through `Session::preview_png`, the bindings and `Session.preview_png`; update `_core.pyi`.
- [ ] **Step 4: Add the Python test** `test_preview_region` in `tests/test_session.py`: after the run in `test_run_blocking_grid_and_exports`' setup, `png = s.preview_png(100, region=(0, 0, 105, 148.5))` has `struct.unpack(">II", png[16:24]) == (100, 141)`, and `region=(0, 0, 0, 10)` raises `ValueError`.
- [ ] **Step 5: Run** `cargo test -p plotline-core`, `cargo clippy --all-targets -- -D warnings`, `uv sync`, `uv run pytest -q tests/test_session.py tests/test_core_binding.py`, and `wasm-pack test --node` in `crates/wasm` if wasm-pack is installed (CI runs it otherwise). Expected: all pass.
- [ ] **Step 6: Commit** `Add a region to the preview renderer`.

### Task 2: The SDK extra and the licence check

**Files:**
- Modify: `pyproject.toml`, `uv.lock`
- Modify: `scripts/check_licenses.py`
- Test: `tests/test_check_licenses.py`

**Interfaces:**
- Produces: `runtime_closure(root: str = "plotline", extras: tuple[str, ...] = ()) -> list[str]`; `main()` checks `runtime_closure(extras=("mcp",))`.

- [ ] **Step 1: Write the failing tests** in `tests/test_check_licenses.py`:
  - `test_closure_follows_only_the_roots_chosen_extras`: fake graph `{"plotline": ["a>=1", 'm>=2; extra == "mcp"'], "a": ['x; extra == "cli"'], "m": []}`; `runtime_closure() == ["a"]` and `runtime_closure(extras=("mcp",)) == ["a", "m"]`.
  - `test_closure_is_runtime_only` adds: `"mcp" in cl.runtime_closure(extras=("mcp",))` and `"pydantic" in` it; its existing asserts on the default closure stay.
  - `test_main_fails_on_a_denied_licence` patches `runtime_closure` with `lambda **kw: ["demo"]`.
- [ ] **Step 2: Run** `uv run pytest -q tests/test_check_licenses.py`. Expected: FAIL (unexpected keyword `extras`).
- [ ] **Step 3: Implement.** In `pyproject.toml` add `[project.optional-dependencies] mcp = ["mcp>=2.3,<3"]` and `"mcp>=2.3,<3"` to the `dev` group; run `uv lock` and `uv sync`. In `runtime_closure`, for the root distribution's requirements only, follow a marker of the form `extra == "<name>"` when `<name>` is in `extras`; skip every other extra as now. Update the module docstring: the check covers the root's shipped extras. If `uv run python scripts/check_licenses.py` denies any package, stop and report it; do not add an override.
- [ ] **Step 4: Run** `uv run pytest -q tests/test_check_licenses.py` and `uv run python scripts/check_licenses.py`. Expected: PASS; the script lists `mcp` and its dependencies, all `OK`.
- [ ] **Step 5: Commit** `Add the mcp extra and check its licences`.

### Task 3: The server and its setup, file and session tools

**Files:**
- Create: `src/plotline/mcp_server.py`
- Create: `tests/test_mcp.py`

**Interfaces:**
- Consumes: `Session.preview_png(..., region=...)` from Task 1.
- Produces:
  - `INSTRUCTIONS: str`, the spec §3 text verbatim.
  - `build_server(session: Session | None = None) -> MCPServer`, named `plotline`, `version=plotline.__version__`, `instructions=INSTRUCTIONS`; a new `Session(max_fps=10)` when none is given.
  - Module helpers used by Task 4: `_errors()`, a context manager turning `ValueError`, `OSError`, `RuntimeError` and `Cancelled` into `ToolError(str(error))`; `_absolute(path: str) -> Path`, which expands `~` and raises `ValueError(f"{path!r} is not an absolute path")` otherwise.
  - Tools, each `async def` with a one- or two-sentence docstring for the assistant naming units: `list_styles`, `describe_style(style: str)`, `load_image(path: str)`, `set_page(width_mm: float | None = None, height_mm: float | None = None, margin_mm: float | None = None, fit: Literal["fit", "fill"] | None = None)`, `set_pens(pens: list[PenSpec])` with `class PenSpec(TypedDict): name: str; width_mm: float; color: str`, `get_session()`, `load_preset(path: str)`, `export(path: str, format: Literal["svg", "json", "png", "preset"], overwrite: bool = False)`. Return shapes are spec §4's; `set_pens` maps `color` through `inks().get(color.lower(), color)`; `load_preset` records warnings with `warnings.catch_warnings(record=True)` and `simplefilter("always")`; `export` refuses an existing file with `f"{p} exists; pass overwrite=true to replace it"` and returns `{"path", "format", "bytes"}`; `get_session` gives `stats` as `asdict(session.stats())` when `session.result` is set, else `None`. Import the registry as a module so the tool `list_styles` does not shadow it.

- [ ] **Step 1: Write the failing tests** in `tests/test_mcp.py`. Module setup: an `anyio_backend` fixture returning `"asyncio"`, `pytestmark = pytest.mark.anyio`, a helper `async def call(c, name, **args)` returning the `CallToolResult`, and `def data(r)` returning `json.loads(r.content[0].text)`. Each test opens `async with Client(build_server(session)) as c` on its own `Session()`.
  - `test_lists_and_describes_styles`: `"grid"` is among the names; `describe_style("grid")` has `stages`; `describe_style("nope")` has `is_error`.
  - `test_setup_tools_and_session`: a PNG from `write_png(tmp_path / "i.png", _core.synthetic_gradient(64, 48))`; `load_image` gives `{"path": str(path), "width_px": 64, "height_px": 48}`; `set_page(width_mm=100, height_mm=100, margin_mm=5)` gives `{"width_mm": 100, "height_mm": 100, "margin_mm": 5, "fit": "fit"}`; `set_pens([{"name": "B", "width_mm": 0.5, "color": "Blue"}])` gives colour `inks()["blue"]`; `get_session` has style `grid`, `image == {"width_px": 64, "height_px": 48}`, `state == "idle"` and `stats is None`.
  - `test_file_paths`: with `USERPROFILE` and `HOME` set to `tmp_path`, `export(path="~/o.preset.json", format="preset")` writes `tmp_path / "o.preset.json"`; a second export there without `overwrite` is an error containing `overwrite=true`, and with `overwrite=True` succeeds; `load_image(path="i.png")` and `export(path="o.svg", ...)` are errors containing `not an absolute path`; `export` of `svg` before any run is an error containing `run the style first`; `export` into `tmp_path / "missing" / "o.preset.json"` is an error and the folder stays absent.
  - `test_preset_round_trip_and_warnings`: export a preset, add `"bogus": {"x": 1}` to its `settings`, write it back, `load_preset` gives `style == "grid"` and one warning naming `bogus`.
- [ ] **Step 2: Run** `uv run pytest -q tests/test_mcp.py`. Expected: FAIL (no module `plotline.mcp_server`).
- [ ] **Step 3: Implement** `mcp_server.py` with a module docstring in the style of `session.py` (what the server is, that it holds no algorithm code, the one-thread rule and why).
- [ ] **Step 4: Run** `uv run pytest -q tests/test_mcp.py` and `uv run mypy src`. Expected: PASS.
- [ ] **Step 5: Commit** `Add the MCP server's setup, file and session tools`.

### Task 4: `run` and `preview`

**Files:**
- Modify: `src/plotline/mcp_server.py`
- Modify: `tests/test_mcp.py`

**Interfaces:**
- Consumes: `build_server`, `_errors`, `PenSpec` from Task 3.
- Produces: tools `run(ctx: Context, style: str | None = None, settings: dict[str, dict[str, Any]] | None = None) -> list[str | Image]` and `preview(width_px: int = 1000, travel: bool = False, source: bool = False, region_mm: list[float] | None = None) -> Image`. Constants `PREVIEW_PX = 1000`, `MIN_PX = 100`, `MAX_PX = 4000`, `MAX_PIXELS = 16_000_000`.

- [ ] **Step 1: Write the failing tests:**
  - `test_run_and_preview`: after loading the PNG and `set_page(width_mm=100, height_mm=100)`, `run(settings={"tone": {"resolution": 64}})` gives a text block whose JSON keys are `{"path_count", "pen_down_mm", "pen_up_mm", "estimated_seconds"}` and an image block whose base64 `data` decodes to bytes starting `b"\x89PNG"`; `preview(region_mm=[0, 0, 50, 50], width_px=200)` gives an image; `export` of `svg`, `json` and `png` then writes files starting `<svg`, `{` and the PNG signature.
  - `test_refused_setting_changes_nothing`: before = `get_session`; `run(style="hatch", settings={"tone": {"resolution": 64}, "nope": {"x": 1}})` is an error; `get_session` has the same `style` and `settings` as before.
  - `test_run_without_an_image_is_an_error`: the error text contains `no image is loaded`.
  - `test_preview_limits`: after a run, `width_px=50`, `region_mm=[0, 0, 1, 100]` and `region_mm=[0, 0, 0, 10]` are errors; after `set_page(width_mm=10, height_mm=1000, margin_mm=0)`, `run` succeeds with a preview 400 px wide (`struct.unpack(">II", png[16:24])[0] == 400`), and `preview()` is an error containing `16000000` and `400`.
  - `test_progress_and_cancel`: with `session.set_image(_core.synthetic_gradient(8, 8))`, `run(style="_slow", settings={"b": {"frames": 20, "delay_ms": 10}})` through `c.call_tool(..., progress_callback=cb)` collects at least one message of the form `"b: ..."` and succeeds; then `run` with `{"b": {"frames": 200, "delay_ms": 50}}` inside `anyio.move_on_after(0.5)`, followed by polling `session.state` every 50 ms for up to 10 s, ends with `session.state == RunState.cancelled`.
  - `test_a_second_run_cancels_the_first`: two `_slow` runs (`frames` 100, `delay_ms` 20, then `frames` 5, `delay_ms` 0) started 0.2 s apart in one task group: the first result is an error containing `the run was cancelled`, the second is not an error.
- [ ] **Step 2: Run** `uv run pytest -q tests/test_mcp.py`. Expected: the new tests FAIL (unknown tool).
- [ ] **Step 3: Implement** per spec §5. `run` restores a refused change by `set_style(<previous name>)` and then `update(stage, **values)` for every stage of the previous `settings`, and re-raises. It subscribes a callback that stores the latest frame before `session.run()`, polls every 0.1 s while `running`, reports `report_progress(n, None, f"{frame.stage}: {frame.label}")` with `n` counting notifications, only when the frame object changed, and in `finally` cancels a still-running handle and unsubscribes. The result is `[json.dumps(asdict(stats)), Image(data=session.preview_png(_page_px(session)), format="png")]`, where `_page_px(session) -> int` is the largest width up to `PREVIEW_PX` whose whole-page picture stays within `MAX_PIXELS`: `min(PREVIEW_PX, int(math.sqrt(MAX_PIXELS * width_mm / height_mm)))`. `export` of `png` (Task 3) switches to `_page_px` too. `preview` refuses `width_px` outside `MIN_PX..=MAX_PX`; for the region (or the whole page when none is given) with finite `width > 0` and finite `height`, it refuses `width_px * round(height * width_px / width) > MAX_PIXELS` with the message `f"a picture {width_px} px wide would have {n} pixels, over the limit of {MAX_PIXELS}; use width_px {most} or less"`, where `most` is the same square-root bound for that area, lowered by one until it passes the check (rounding the height can push the bound a few pixels over); any other bad region reaches the engine, which refuses it.
- [ ] **Step 4: Run** `uv run pytest -q tests/test_mcp.py` and `uv run mypy src`. Expected: PASS.
- [ ] **Step 5: Commit** `Add the MCP server's run and preview tools`.

### Task 5: `plotline mcp`, docs and the brief

**Files:**
- Modify: `src/plotline/cli/main.py` (module docstring and the `mcp` command)
- Modify: `tests/test_cli.py` (`test_gui_without_a_bundle_and_the_mcp_stub`), `tests/test_mcp.py`
- Modify: `README.md`, `docs/BRIEF.md`

**Interfaces:**
- Consumes: `build_server` from Task 3.

- [ ] **Step 1: Write the failing tests:**
  - In `tests/test_cli.py`, rename the test to `test_gui_without_a_bundle_and_mcp_without_its_extra`: with `monkeypatch.setitem(sys.modules, "mcp", None)` and `monkeypatch.delitem(sys.modules, "plotline.mcp_server", raising=False)`, `run("mcp")` exits 1 and its stderr contains `pip install "plotline[mcp]"`.
  - In `tests/test_mcp.py`, `test_stdio_server_lists_the_tools`: `Client(StdioServerParameters(command=sys.executable, args=["-c", "from plotline.cli.main import main; main()", "mcp"]))` lists exactly the ten tool names of spec §4.
- [ ] **Step 2: Run** both tests. Expected: FAIL (the stub still prints "not available").
- [ ] **Step 3: Implement** the command: docstring `Serve the session to an AI assistant over MCP on stdio.`; import `build_server` inside the function; on `ModuleNotFoundError` whose `name` is `mcp` or starts with `mcp.`, print the spec §9 message to stderr and exit 1, else re-raise; then `build_server().run()`. Drop the module docstring's placeholder sentence.
- [ ] **Step 4: Docs.**
  - README: replace "and the MCP server are planned but not built yet, and `plotline mcp` prints a message saying so" with a short paragraph: install with `pip install "plotline[mcp]"`, then give an MCP client the server as `{"command": "plotline", "args": ["mcp"]}`; the assistant can load an image, run styles, look at previews (also zoomed) and export. Name no client.
  - Brief §9: replace the sketch with the ten tools and the rules from spec §§4–6, keeping the section's rules paragraph.
  - Brief §13: after the milestone 6 sentence, add that milestone 7 was built on 2026-10-12 under the MCP spec, linked like the others.
  - Brief §14: append the rows "MCP SDK", "MCP tools" and "Zoomed preview" with the decisions and reasons of spec §11.
- [ ] **Step 5: Run the full check:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test -p plotline-core`, `uv run ruff check .`, `uv run ruff format --check .`, `uv run mypy src`, `uv run pytest -q`, `uv run python scripts/check_licenses.py`. Expected: all pass.
- [ ] **Step 6: Manual check:** `uv run plotline mcp` started from a shell prints nothing to stdout before a client speaks and stops on Ctrl-C or end of input.
- [ ] **Step 7: Commit** `Serve the MCP server from plotline mcp and document it`.
