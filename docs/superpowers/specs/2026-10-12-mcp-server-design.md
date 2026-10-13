# plotline spec 8: the MCP server

Date: 2026-10-12. Status: approved in conversation; written for review.

This spec builds milestone 7: `plotline mcp` serves the session over the Model Context Protocol, so an AI assistant can run the "try settings, look, adjust" loop the brief describes in §9. The server is a thin layer: every tool calls `Session` methods and holds no algorithm code. One engine change comes with it, a zoomed preview, so the assistant can look at fine detail.

## 1. Scope

- `src/plotline/mcp_server.py`: the server and its tools.
- `src/plotline/cli/main.py`: `plotline mcp` starts the server on stdio.
- `pyproject.toml`: an optional extra, `plotline[mcp]`, holding the MCP SDK; the dev group gets it too.
- `scripts/check_licenses.py`: the check covers the `mcp` extra.
- `crates/core/src/preview.rs`, `crates/core/src/session.rs`, `crates/py/src/session.rs`, `crates/wasm/src/lib.rs`, `src/plotline/session.py`, `src/plotline/_core.pyi`: an optional preview region.
- Docs: the README's MCP sentence, the brief's §9 and §13, and three rows in its decision log.

Out of scope: machines and G-code (spec 4 left them out of the Python binding; a later spec adds them to the CLI and the MCP together), plot speeds, the pen bank, image analysis (its own spec, after the smart-features research), network transports, MCP resources and prompts, and images passed as bytes instead of a file path.

## 2. The dependency

The server uses the official MCP Python SDK, `mcp`, version 2 (MIT), with its high-level `MCPServer` class. It goes in an optional extra:

```toml
[project.optional-dependencies]
mcp = ["mcp>=2.3,<3"]
```

`pip install plotline` stays numpy and click, so a GUI user does not download the SDK's two dozen packages. The dev dependency group lists `mcp>=2.3,<3` as well, so `uv sync` gives every contributor and every CI job the SDK without a flag.

The SDK beats a hand-written JSON-RPC server because it carries version negotiation, progress and cancellation, which plotline would otherwise have to track as the protocol changes. It beats a core dependency because most users never start the server.

**Licence check.** `runtime_closure` gains an `extras` argument. For the root distribution it also follows requirements whose marker names one of those extras; for every other distribution it still skips extras. `main()` checks the closure with `extras=("mcp",)`, so every package the server needs passes the same allow-list as the core. If the installed closure has a package the check denies, the work stops and the owner decides; the check is not loosened and no override is added without that decision.

## 3. The server

`mcp_server.py` has one public function:

```python
def build_server(session: Session | None = None) -> MCPServer
```

It makes an `MCPServer` named `plotline` with the package version and the instructions below, and registers the tools of §4 as closures over one `Session` (a new one when none is given, created with `max_fps=10`). One session per server process follows the brief's "Session model" row. Tests build a server per test with its own session.

**One thread for the session.** `Session` is not safe for concurrent changes, and SDK version 2 runs plain `def` tools on worker threads. Every tool is therefore `async def`, so every session call happens on the event loop's thread. Only a run's stages leave it, on the session's own worker thread, as `Session.run` already does.

**Instructions.** The server sends these to the client on initialisation:

> plotline turns an image into line art for a pen plotter. Work in this order: load_image, then set_page and set_pens if the defaults do not fit, then list_styles and describe_style to choose a style and see its settings. run draws the image and returns its stats and a preview at true pen width. Look at the preview, change settings with run(settings=...), and repeat; only the stages after a changed setting run again. Use preview with region_mm to inspect detail. export writes the result. All lengths are millimetres, the page origin is its top-left corner and y grows down. Paths must be absolute.

## 4. Tools

Each tool's input schema comes from its type hints. Lengths are in mm.

| Tool | Arguments | Does | Returns |
|---|---|---|---|
| `list_styles` | none | `list_styles()` | `[{name, label, description}]` |
| `describe_style` | `style: str` | `get_style(style).schema()` | the style's stages with each setting's type, range, default and help text |
| `load_image` | `path: str` | `load_image` | `{path, width_px, height_px}` |
| `set_page` | `width_mm`, `height_mm`, `margin_mm: float \| None`, `fit: "fit" \| "fill" \| None` | `set_canvas` with only the values given changed, as the CLI does | the page as `{width_mm, height_mm, margin_mm, fit}` |
| `set_pens` | `pens: [{name, width_mm, color}]` | `set_pens`; `color` is `#rrggbb` or an ink name from `inks()`, in any case, as `--pen` takes it | the pens with colours as `#rrggbb` |
| `run` | `style: str \| None`, `settings: {stage: {key: value}} \| None` | see §5 | a text block with the stats as JSON, and a 1000 px preview as a PNG image block |
| `preview` | `width_px: int = 1000`, `travel: bool = false`, `source: bool = false`, `region_mm: [x, y, width, height] \| None` | `preview_png` | a PNG image block |
| `get_session` | none | reads the session | `{style, settings, page, pens, image: {width_px, height_px} \| null, state, stats \| null}` |
| `load_preset` | `path: str` | `load_preset`, with its warnings caught | `{style, warnings: [str]}` |
| `export` | `path: str`, `format: "svg" \| "json" \| "png" \| "preset"`, `overwrite: bool = false` | `export_svg`, `export_json`, `preview_png()` written to the file, or `save_preset` | `{path, format, bytes}` |

The brief's sketch had a separate `update(settings)`. `run(settings=...)` covers it: the cache already reruns only the stages after a changed setting, and one tool is easier for an assistant to use than two that differ only in when they run.

`preview` refuses a `width_px` outside 100 to 4000, and a region whose picture would be more than 4000 pixels high. Those checks bound the memory a tool call can ask for, since the tool's input comes from outside.

## 5. Running

`run(style, settings)` does this, in order:

1. **Changes.** When `style` is given and differs from the current one, `set_style(style)`, which resets the settings to that style's defaults, as in the CLI. Then `update(stage, **values)` for each stage in `settings`, in the order given. If any change is refused, the style and settings are put back as they were before the call and the tool fails with the engine's message. Nothing runs.
2. **Start.** `Session.run()` starts the run on the session's worker thread. It raises `RuntimeError` when no image is loaded, which becomes a tool error.
3. **Progress.** A subscriber keeps the latest frame. Every 100 ms, while the run is `running`, the tool sends a progress notification when the frame has changed since the last one: the progress value counts the notifications sent, so it always grows as the protocol requires, and the message is `"<stage>: <label>"`, for example `tone: tone map`. Clients that reset their timeout on progress therefore wait for a long run.
4. **Cancellation.** When the client cancels the request, the tool's task is cancelled; a `finally` block cancels the run if it is still running and removes the subscriber either way.
5. **End.** `done` returns the stats and the preview. `error` fails with the run's error message. `cancelled` fails with "the run was cancelled".

## 6. Files

- `load_image`, `load_preset` and `export` take a path after `~` expansion and refuse one that is not absolute, naming it. A stdio server's working directory is wherever the client started it, so a relative path would land somewhere the user did not mean.
- `export` refuses a path that exists unless `overwrite` is true: "<path> exists; pass overwrite=true to replace it". An assistant cannot replace a user's file by accident. The parent directory must exist; the `OSError` otherwise becomes a tool error.
- Every result that names a file gives its absolute path.

## 7. Errors

Each tool runs its session calls inside one helper that turns `ValueError`, `OSError`, `RuntimeError` and `Cancelled` into the SDK's `ToolError` with the original message. The assistant sees a tool result marked as an error with the engine's words, such as `width_mm must be positive, got 0.0`, and can correct itself. Any other exception is a bug and propagates.

## 8. The zoomed preview

`preview::render_png` and `render_rgba` gain `region: Option<[f64; 4]>`, a rectangle `[x, y, width, height]` in page mm. With `None` the picture is the whole page, exactly as now. With a region:

- One mm is `width_px / width` pixels, and the picture is `round(height * width_px / width)` pixels high.
- Page point `(px, py)` is drawn at `((px - x) * scale, (py - y) * scale)`, for the drawing-rect outline, the tone overlay, the paths and the travel lines alike. Pens keep their true width at that scale, at least one pixel, so a zoomed preview shows what the pen will lay down.
- The region may reach past the page; the part outside is white.
- A region with a value that is not finite, or a width or height that is not positive, is refused with a new `PreviewError::Region` naming the value.

`Session::preview_png` passes the region through. The Python binding takes `region=None` as a keyword argument, and `Session.preview_png` gains `region: tuple[float, float, float, float] | None = None`. The WebAssembly binding passes `None`, so the GUI is unchanged.

## 9. The CLI

`plotline mcp` imports `plotline.mcp_server` when it runs, not at start-up, so the other commands do not load the SDK. When that import fails because `mcp` is missing, it prints `The MCP server needs the mcp extra: pip install "plotline[mcp]"` to stderr and exits with status 1. Otherwise it calls `build_server().run()` on stdio and prints nothing to stdout, which carries the protocol.

The README shows the client configuration as plain JSON, `{"command": "plotline", "args": ["mcp"]}`, and names no client.

## 10. Tests

- **`tests/test_mcp.py`**, through the SDK's in-memory `Client` with the anyio pytest plugin, which the SDK already depends on:
  - `list_styles` lists `grid`; `describe_style("grid")` has stages; an unknown style is a tool error.
  - A full flow on a PNG from `tests/pngs.py`: `load_image`, `set_page`, `set_pens` with an ink name, `run`, `preview` with and without a region, `export` in all four formats, then `load_preset` of the saved preset.
  - `run` returns a text block whose JSON has the four stats and an image block that starts with the PNG signature.
  - `run` with a refused setting is a tool error, and `get_session` shows the style and settings from before the call.
  - `run` with no image is a tool error.
  - `export` refuses an existing file without `overwrite` and replaces it with; a relative path is refused for all three file tools.
  - `preview` refuses `width_px` 50 and a region 1 mm wide and 100 mm high at 1000 px.
  - A run of the hidden `_slow` style sends at least one progress notification.
  - Cancelling the task that awaits a `_slow` run leaves the session's run `cancelled`.
- **`tests/test_cli.py`**: with the `mcp` module made unimportable, `plotline mcp` exits 1 with the install message.
- **`tests/test_check_licenses.py`**: the closure without extras still leaves out `pydantic`; the closure with `extras=("mcp",)` holds `mcp` and `pydantic`; a fake graph shows the root's extra followed and a dependency's extra skipped.
- **`preview.rs`**: a dot at a known point lands at the expected pixel in a region around it; a region beside it is all white; `None` gives the same bytes as before; a zero-width region is refused.
- **`tests/test_session.py`**: `preview_png(region=...)` returns a PNG of the expected height.

## 11. Decisions recorded in the brief

- **MCP SDK.** The official MCP Python SDK, version 2, in an optional `mcp` extra, checked by the licence script like a core dependency. It carries the protocol's negotiation, progress and cancellation, and the extra keeps the default install small.
- **MCP tools.** Ten tools over one session; `run(settings=...)` replaces the sketched `update`; every tool is `async` so the session is used from one thread; paths must be absolute and `export` does not overwrite unless asked. These keep the server thin and safe to drive from an assistant.
- **Zoomed preview.** The preview renderer takes an optional region in page mm, so the assistant can judge detail that a whole-page picture cannot show.

§9 of the brief is rewritten to list the final tools, and §13 marks milestone 7 as built under this spec.
