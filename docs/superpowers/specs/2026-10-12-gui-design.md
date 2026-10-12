# plotline spec 3: GUI and live preview in the browser

Date: 2026-10-12. Status: design awaiting owner review.

This spec covers milestone 6 of [the brief](../../BRIEF.md): the GUI with the live preview, running the WebAssembly engine from [the Rust core spec](2026-10-11-rust-core-design.md) inside the browser. It is built before milestones 3 to 5 (the Voronoi family and Sketch Lines) because the owner wants to test the engine through the GUI, and because the GUI is generated from the stage declarations, so the styles that come later need no GUI changes. The only style it can show today is `grid`. The same bundle is served locally by `plotline gui` and, later, as the hosted page.

## 1. Scope

Delivered by this spec:

- The web app under `web/`: TypeScript, Vite and Preact, with the engine in a Web Worker.
- The worker protocol: commands in, state back, and frames, results and run ends as events.
- The preview of §8 of the brief: progressive drawing, true pen width and colour, zoom and pan, source overlay, per-pen visibility, pen-up travel overlay, stats and stage progress.
- A settings panel generated from the style schema, grouped by stage, advanced settings collapsed.
- Panels for the page, the pens and the plot speeds, a style picker, preset download and upload, SVG and JSON export.
- Three small additions to the WebAssembly binding (§4).
- The bundle built into the Python package for `plotline gui`, a CI job for the web app, and the Pages workflow pointed at the bundle.
- Tests for the worker driver, the pure frontend logic and the binding additions.

Out of scope, with the owner of each:

- The MCP server: a later spec. The Voronoi family and Sketch Lines: the next specs.
- A WebGL renderer for very large path counts: decided against for now, see §6.2.
- More than one session or tab, a disk cache, a persisted pen library, a compare view, the canvas crop offset: later, as the brief's decision log says.
- Deploying the hosted page while the repository is private: the Pages job stays gated on the repository being public.

## 2. Dependencies

No Python or Rust dependency is added. The web app's only runtime dependency is preact (MIT), which is the only third-party code in the bundle besides the project's own WebAssembly module. vite (MIT), typescript (Apache-2.0), `@preact/preset-vite` (MIT) and vitest (MIT) are build and test tools. The build copies Preact's `LICENSE` file into the bundle directory, as its licence asks of copies. Node 22 is the version CI uses.

## 3. Layout

```
web/
  package.json  package-lock.json  tsconfig.json  vite.config.ts  index.html
  pkg/                  the wasm-pack output; ignored by git
  src/
    main.tsx            mounts the app
    app.tsx             layout, the state store, the command calls
    engine.ts           the main-thread client: starts the worker, one promise per command, events out
    worker.ts           loads the module, owns the Session, answers commands, runs the driver
    driver.ts           the run loop of §5, a pure function of a Run and a post callback
    schema.ts           settings schema to controls, pure
    paths.ts            iterates paths over a coordinate buffer and its offsets, pure
    preview.tsx         the canvas: page, tone image, paths, travel, zoom and pan
    settings.tsx        the schema-driven settings form
    panels.tsx          page, pens, speeds, stats, presets and export
    style.css
    *.test.ts           vitest
src/plotline/web/       the built bundle: ignored by git, shipped in the wheel, served by `plotline gui`
```

`vite build` writes to `src/plotline/web` with `base` set to `./`, so the same files work at the root of a local server and under a path on the hosted site.

## 4. The engine in the browser

The WebAssembly module from `crates/wasm` is used as it is, with three additions, each a thin wrapper over a session or drawing method that already exists in the engine:

- `Session.setSpeeds(json)` and `Session.speeds()`, so the stats panel can change the plot speeds.
- `DrawingHandle.canvas()`, the page the drawing was made on, as JSON.
- `DrawingHandle.toneRect()`, `{"x", "y", "w", "h", "width_px", "height_px"}` in mm and pixels, and `DrawingHandle.toneInk()`, the tone map as a `Uint8Array` of grey levels where 255 is no ink, for the source overlay; both `undefined` when the drawing has no tone map.

Every handle the module hands out (`Run`, `FrameHandle`, `DrawingHandle`) is freed as soon as its data has been copied out, because the module's memory is not collected by the browser.

## 5. Worker protocol

The worker owns the one `Session`. The main thread never touches the module. Messages are plain objects; typed arrays travel in the transfer list, so nothing is copied between threads.

**Commands,** main to worker, each `{id, method, args}` with a number `id`. The methods are `listStyles()`, `describe(name)`, `setImageBytes(bytes)`, `setCanvas(canvas)`, `setPens(pens)`, `setStyle(name)`, `update(stage, values)`, `setSpeeds(speeds)`, `run()`, `cancel()`, `preset()`, `loadPreset(preset)`, `svg()` and `json()`. The worker answers `{id, ok: true, state, value}` or `{id, ok: false, error}` with the engine's message. `value` is the styles list, a schema, the preset, the warnings of `loadPreset`, or the export text; otherwise it is absent. Every command that changes what a run would read (image, settings, style, canvas, pens, preset) starts a run when an image is set, and only changes the state when none is. The reply's state is taken after that run has started, so its `run.state` is `running`.

**The state,** in every reply:

```json
{"image": true,
 "style": "grid",
 "canvas": {"width_mm": 210, "height_mm": 297, "margin_mm": 10, "fit": "fit"},
 "pens": [{"name": "Black 0.3", "width_mm": 0.3, "color": "#000000"}],
 "settings": {"tone": {"resolution": 1000}, "points": {}},
 "speeds": {"draw_mm_s": 50, "travel_mm_s": 100, "pen_lift_s": 0.2},
 "run": {"id": 3, "state": "running", "error": null},
 "stats": {"path_count": 1200, "pen_down_mm": 0, "pen_up_mm": 2310.5, "estimated_seconds": 263.1}}
```

`stats` is null without a drawing and `run.error` is the message of the error that ended the last run. The worker builds it from `preset()`, `speeds()`, `state()`, `error()` and `stats()`.

**Events,** worker to main, each with a `type`:

- `{type: "run", id}` when a run starts. Runs are numbered from 1 in the worker.
- `{type: "frame", id, stage, progress, label, points?, coords?, offsets?}` while it runs. `points` is a `Float64Array` of x, y pairs in mm. `coords` and `offsets` are the paths made so far in the engine's layout: a `Float64Array` of x, y pairs and a `Uint32Array` with the start of each path in those pairs, then the total count, so path `i` is the pairs from `offsets[i]` to `offsets[i + 1]`. A frame from the tone stage carries neither.
- `{type: "result", id, canvas, layers: [{pen, coords, offsets}], tone: {x, y, w, h, width_px, height_px, ink} | null}` when the run ends in `done`.
- `{type: "end", id, state, error, stats}` with `state` one of `done`, `cancelled` or `error`, the error message when there is one, and the stats of the session's drawing, which after a cancelled or failed run is still the previous one.

A fresh worker has no image, so a loaded page starts empty and the first run starts when an image is opened.

**The driver** is the run loop:

1. `run()` calls `session.start()`, which cancels the run before it inside the engine, and posts `run`.
2. The loop calls `run.nextFrame()`. For each frame it reads `stage`, `progress` and `label`, and copies the geometry out only when the frame will be posted: at most every 33 ms, and always when `progress` is 1 or more, so a throttled preview still ends on the finished state of every stage. The handle is freed either way.
3. Every 50 ms of work the loop yields to the worker's event loop with a zero-delay macrotask (a `MessageChannel` message, not `setTimeout`, which browsers clamp to 4 ms once nested). That is when `cancel` and the other commands get in, so cancellation needs no shared memory and no cross-origin isolation headers, which a static host cannot set.
4. When `nextFrame()` returns `undefined` the run has ended. If its state is `done`, the driver reads the drawing's layers, canvas and tone map, posts `result`, then posts `end` with the stats. Otherwise it posts `end` with the state and error. A run that was cancelled by a later `start` ends this way too, so the main thread sees every run end exactly once.

A command that arrives while the driver is between steps runs at once. `start()` from such a command makes the old loop's next `nextFrame()` return `undefined` with state `cancelled`, and the new loop begins on the next turn.

## 6. Frontend

TypeScript, Vite and Preact with hooks. No state library, no CSS framework and no runtime dependency beyond Preact.

### 6.1 Layout

Three columns. Left: the style picker and the settings form. Centre: a toolbar (open image, run and stop, the source and travel toggles, fit to window), the preview, and a status bar with the stages of the run and the stats. Right: the page, the pens, the speeds, presets and export. Every control is a native input with a label, so it works from the keyboard. The columns have fixed widths and the preview takes the rest; this is a desktop tool.

While the module loads, the page shows one line saying so; if loading fails, the error with its message.

### 6.2 Preview

A Canvas 2D element in page millimetres under a view transform: a scale in pixels per mm and an offset. The view fits the page to the element at start, on resize and on "fit"; the wheel zooms about the cursor and dragging pans. The device pixel ratio is honoured so strokes stay crisp.

Drawn in order: the page in white with a faint shadow, the drawing rect outlined in light grey, the tone image at its rect at 30 percent opacity when the source toggle is on, the paths, then the travel overlay. The tone image is an `ImageBitmap` made once from the `ink` of the latest result.

Paths are drawn from the latest `result`: one `Path2D` per layer from its `coords` and `offsets`, stroked at the pen's width in mm with round caps and joins in the pen's colour, so one stroke call draws a whole layer. A one-vertex path is a zero-length segment, which a round cap draws as a dot of the pen's width. The checkbox of each pen hides its layer. While a run is under way the latest `frame` is drawn instead: its points as dots and its paths as lines, in the first pen's width and colour, since frames carry no pen. The travel overlay is a thin red line, one screen pixel wide at any zoom, from the last vertex of each path to the first vertex of the next, across the layers in drawing order. Redraws are coalesced with `requestAnimationFrame`.

Canvas 2D is the renderer, which settles open question 3 of the brief for now: a layer of a hundred thousand vertices strokes in tens of milliseconds, which is enough for the styles planned, and it needs no shaders or buffer management. The threshold for revisiting WebGL is a drawing whose redraw takes over 100 ms.

Before an image is loaded the page is drawn with a hint to open one.

### 6.3 Settings form

Built from `describe(style)` for the current style, which the worker exposes as the command `describe(name)`. Each stage is a section headed by its label, with its settings and then an "Advanced" disclosure, collapsed, for those flagged `x-advanced`. A stage with no settings shows its label alone. The control for a property comes from its schema:

| Schema | Control |
|---|---|
| `boolean` | checkbox |
| `integer` or `number` with `minimum` and `maximum` | range slider with a number input beside it; the step is `x-step`, or 1 for an integer, or a hundredth of the range |
| `integer` or `number` without both bounds | number input |
| `enum`, directly or through a `$ref` into the schema's `definitions` or `$defs`, also when wrapped in a one-element `allOf` | select, with each variant's description as its title |
| anything else | text input |

The label is `x-label` or the key with underscores as spaces, `x-unit` follows the input, and `description` is the help text under the control. A change is sent as `update(stage, {key: value})` after a 150 ms debounce so a slider does not start a run per pixel. A refused value shows the message under the stage and restores the control from the state in the reply.

### 6.4 Other panels

- Page: width, height and margin in mm, the fit mode, a select of standard sizes (A5, A4, A3, Letter, Tabloid, and "custom" when the dimensions match none), and a button that swaps width and height.
- Pens: one row per pen with name, width in mm, a colour input, a visibility checkbox (client-side only) and a remove button; an "add pen" button. The last pen cannot be removed, since the session refuses an empty list.
- Speeds: the draw and travel speeds in mm/s and the pen lift time in s. Changing one sends `setSpeeds` and the reply carries the new estimate.
- Stats: path count, pen-down and pen-up distances in mm, and the estimated time as minutes and seconds.
- Status bar: the stages of the current style in order, each marked done, running with its label and a progress bar, or waiting; and the run state with the error message when there is one.
- Presets: "save preset" downloads the JSON from `preset()`; "load preset" reads a file and sends `loadPreset`, then shows the warnings.
- Export: SVG and JSON buttons that ask the worker for the text and download it, disabled without a drawing.
- Open image: a file input for PNG and JPEG; the bytes go to `setImageBytes` with the buffer transferred.

### 6.5 State flow

The worker is the source of truth. The client keeps the styles list from `listStyles()`, the schema of the current style, and the last state it received. A command updates the control at once, sends the request, and replaces the state with the reply; on an error it shows the message and asks for the state again. Events drive the preview and the status bar. Replies and events can interleave, so the client keeps the run id of the latest `run` event and ignores frames and results of older runs.

## 7. Build, packaging and CI

- Development: `wasm-pack build --release --target web --out-dir ../../web/pkg` in `crates/wasm`, then `npm run dev` in `web/`. The page imports the module from `./pkg/plotline_wasm.js` and Vite serves the `.wasm` file as an asset, in the worker too.
- Bundle: `npm run build` writes `src/plotline/web`, which `.gitignore` lists. `plotline gui`, already in the CLI, serves that directory on a loopback port and opens the browser, and exits with a message when it is missing. maturin's `include` list adds the directory to the wheel and the sdist; a release build fails before `maturin build` when `src/plotline/web/index.html` is missing, as the Rust core spec decided.
- CI gains a `web` job: Node 22, the wasm-pack build, `npm ci`, `npm run check` (type check and vitest) and `npm run build`. The matrix stays Linux-only as the brief decided.
- The Pages workflow builds the same way and uploads `src/plotline/web`. It stays gated on the repository being public, since Pages on a private repository needs a paid plan; until then the owner tests with `plotline gui` or `npm run dev`.

## 8. Testing

Rust: the wasm smoke test gains checks for `setSpeeds`, `canvas()`, `toneRect()` and `toneInk()` on the `grid` drawing.

Vitest, under Node:

- `driver.test.ts` loads the real module from `web/pkg` with its bytes, runs `grid` on the module's synthetic gradient with a fake `post`, and checks the sequence: one `run`, at least one frame per stage with the final frame of each stage present, one `result` with one layer and a tone map of the expected size, one `end` of state `done` with stats; then that a second `start` during a run ends the first with `cancelled`.
- `schema.test.ts`: the control mapping from a schema fixture with a boolean, a bounded number with `x-step`, an integer, an enum through a `$ref`, and an advanced setting.
- `paths.test.ts`: iterating a buffer with a dot, a two-vertex path and an empty buffer.

Python: the existing `plotline gui` test, which checks the message when the bundle is missing, stays.

Before the branch is finished, the GUI is opened with a photo to check the progressive drawing, a setting change with the cached stages skipped, cancel, the toggles, zoom and pan, a preset round trip and an export.

## 9. Decisions recorded in the brief

This spec settles open question 3 of the brief (Canvas 2D) and records: the worker protocol with transferred typed arrays; the driver's yield cadence; the bundle at `src/plotline/web` as the one location for `plotline gui`, the wheel and the hosted page; frames drawn with the first pen and results per layer.
