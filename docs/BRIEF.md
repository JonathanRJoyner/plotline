# plotline: project brief

This brief is the starting point for building plotline. It says what the project is for, how it should work, what is already decided and why, and what is left open. If you are picking up this project, read all of it before writing code. If something here turns out to be wrong once you are building, raise it rather than quietly working around it.

## 1. What plotline is

plotline turns raster images (photos, renders, scans) into vector line art that a pen plotter can draw. Tone becomes line: dark areas get dense marks and light areas get sparse ones. The marks can be dots, one continuous line, dashes, sketched strokes, and more styles over time.

It has four parts:

- **A Python library.** Image in, pen paths out. It knows nothing about any particular machine.
- **A live preview GUI.** A local web app that shows each style working as it runs, so you can see what a setting does.
- **An MCP server.** It lets an AI assistant run styles, look at previews, adjust settings and export results.
- **A CLI.** For scripting and batch work.

The engine is one Rust crate compiled two ways: a native Python extension behind the library, CLI and MCP, and a WebAssembly module that runs the GUI inside the browser. The GUI therefore also works as a static web page with nothing to install.

## 2. Who it's for

- People with pen plotters (purpose-built plotters, converted 3D printers, drawing robots) who want good tonal drawings from images.
- Generative and plotter artists who want styles they can combine and extend in code.
- People who work with an AI assistant and want it to handle the "try settings, look, adjust" loop.

## 3. Goals and success criteria

1. **Quality.** A style's output should look deliberate and plot cleanly. That means no stray specks, no wasted pen-up travel, and the drawing fills its area.
2. **Seeing is understanding.** The preview shows the algorithm progressing (points settling, a path being built). It draws at the real pen width and colour, so what you see is what you plot.
3. **Fast iteration.** Changing a late-stage setting, such as dot size or switching stipple to a continuous line, should update the preview in about a second. It should not rerun the slow stages.
4. **One engine, many front ends.** The GUI, MCP, CLI and toolchain adapters all call the same core API, whether it runs natively or in the browser. No algorithm code lives in a front end.
5. **Extensible.** A new style is built mostly from existing stages. A third party could add one without changing the core.
6. **Ready to be public.** Clean history, permissive licensing all the way down, documentation and tests from day one. See §11.

**The first release succeeds when** a user can run `plotline gui` or open the hosted page, load a photo, then:
- watch a Voronoi stipple converge in the preview
- switch the rendering between stipple, continuous line and dashes without the points being recomputed
- try Sketch Lines
- export a layered SVG at a chosen paper size

The same workflow must also work through the CLI and the MCP.

## 4. Non-goals (for now)

- **Machine-specific output**, such as G-code for one particular printer or firmware quirks. plotline outputs plain vector data, and adapting it to a machine is the downstream user's job. Generic G-code export belongs in an adapter, not the core (§14).
- **Driving plotters directly** (serial control, job queues).
- **Image editing** beyond what tone mapping needs (crop, levels, contrast, gamma, blur, invert).
- **Generative art from nothing.** plotline is image-driven.
- **A hosted service.** Nothing runs on a server. The hosted page is static files, and the engine runs in the visitor's browser (§8).

## 5. Core concepts

- **Source image:** the raster input. Internally it becomes a **tone map**, a 2D float array where 0 is white and 1 is full ink, after the user's adjustments.
- **Canvas:** the physical drawing area in millimetres, plus how the image fits it (fit or fill; a crop offset is planned, see §14).
- **Pen:** a named physical pen with a stroke width in mm and a colour. Pens affect both the preview and how styles space marks.
- **Layer:** the paths for one pen. A result is an ordered list of layers.
- **Path:** a polyline, an N×2 array of mm coordinates. A single dot is a very short path or a small circle, decided per style. Pen-down along the path, pen-up between paths.
- **Style:** a named recipe, defined as a pipeline of stages with default settings (e.g. "Voronoi Stipple").
- **Stage:** one step in a pipeline (tone mapping, point placement, rendering, path ordering…). Each stage declares its settings, the kinds of artifact it requires (such as the tone map and the points) and the kind it produces.
- **Frame:** an intermediate snapshot a stage produces while it runs, for the preview.
- **Session:** one working context: a source image, a canvas, pens, a chosen style, settings, cached stage outputs and the latest result.

## 6. Architecture

```
            ┌────────────────────────────────────────────┐
            │  engine (Rust): tone mapping, stages,       │
            │  styles, cache, session, export, preview    │
            │  (pure library, no I/O, machine-agnostic)   │
            └──────────┬──────────────────────┬──────────┘
                       │                      │
              Python extension         WebAssembly module
              (thread, files)          (Web Worker)
                       │                      │
            ┌──────────┴──────────┐           │
           CLI    library     MCP      GUI in the browser
               toolchain adapters    (installed or hosted page)
```

### 6.1 Staged, cached pipeline (decided)

A style is a short chain of stages:

```
image → tone map → point placement → renderer → path ordering → result
         (levels,    (slow: iterative  (stipple,   (cut pen-up
          gamma)      relaxation)       line, dash)  travel)
```

- **Stages are step functions.** The host pulls one step at a time, and each step returns a frame (e.g. "points after relaxation pass 12 of 50") or the final output. The GUI shows the frames; the CLI and MCP can ignore them and wait for the end.
- **Stages declare their inputs by kind.** A stage names the artifact kinds it requires (image, canvas, pens, tone, points, paths…) and the kind it provides. Each requirement resolves to the nearest upstream provider, so a renderer can read both the points and the tone map, and a hatching style can read the tone map alone.
- **Settings are declared, not implicit.** Each stage declares its settings: name, type, range, default, help text and whether it is "advanced". The GUI's controls, the CLI flags and the MCP tool schemas are all generated from that one declaration.
- **Caching.** The session stores each stage's output, keyed by a hash of that stage's name, version and settings plus the keys of its inputs. Changing a setting reruns only that stage and the ones after it. Switching between styles that share their early stages (e.g. Voronoi stipple to Voronoi continuous line) reuses the shared outputs.
- **Cancellation.** A new run cancels an unfinished one. The runner checks a cancel flag between steps, so the browser can cancel without shared memory.
- **Determinism.** Any randomness comes from a seed in the settings. The same input and settings must give the same output on every platform and in the browser, which tests rely on.
- **Styles that don't split into stages** (e.g. Sketch Lines) are a single stage. That's fine.

**Why:** the preview is only useful if adjusting settings is fast. Point placement is the slow part and many styles share it, so caching per stage is what makes tweaking feel immediate. It also matches how the styles relate to each other (§7): new styles are mostly new combinations of existing stages. Retrofitting caching onto monolithic style functions later would mean rewriting every style.

### 6.2 Session layer

The session holds the state: the image, canvas and pens, the chosen style and settings, the cache, the latest result, stats, preview rendering (PNG at true pen width) and export. It lives in the engine so both targets share it. Each host adds its own side effects: the Python package adds files and a worker thread, the browser adds a Web Worker. Every front end goes through it.

### 6.3 Front ends

- **CLI:** `plotline run <style> image.png --out drawing.svg --set stage.key=value …`, `plotline styles`, `plotline describe <style>`, `plotline gui`, `plotline mcp`.
- **GUI:** see §8.
- **MCP:** see §9.
- **Toolchain adapters:** small optional extras that convert plotline results into the data formats of existing plotter toolchains, so plotline can slot into workflows people already use. None ship in 0.1; which come first is open (§12).

## 7. Styles

### 7.1 First release (decided)

**The Voronoi point family:** one point-placement stage and three renderers.

- **Point placement: weighted Voronoi relaxation.** Scatter points with density proportional to tone, then repeatedly move each point to the tone-weighted centroid of its Voronoi cell (weighted Lloyd relaxation), so points spread evenly while dark areas stay dense. This is the slow stage, and its frames are the "points settling" animation.
- **Renderer: stipple.** Each point becomes a dot. Dot size is optionally scaled by local tone.
- **Renderer: continuous line (TSP).** All points joined into one non-crossing path through a travelling-salesman tour (a construction heuristic plus local improvement such as 2-opt). The result is a single-stroke drawing.
- **Renderer: dashes.** Each point becomes a short stroke, oriented by the local image gradient or a set angle.

**Sketch Lines:** a greedy loop that repeatedly finds the darkest line segment from the current position, draws it, lightens the tone map under it, and continues until it hits a target darkness or line budget. It runs as a single stage, and its frames show strokes accumulating.

**Shared final stage: path ordering.** Reorder and optionally reverse paths to cut pen-up travel, merge paths whose ends nearly touch, and drop paths below a minimum length.

### 7.2 Later families (for planning; not committed)

These are listed so the stage design leaves room for them. Most are new combinations of stages rather than new machinery.

| Family | Idea | Reuses |
|---|---|---|
| More point placers | Adaptive sampling, other quantisation and relaxation schemes | All renderers |
| More renderers | Triangulation, spanning tree, Voronoi cell outlines, shapes or glyphs at points, circular scribbles | All point placers |
| Sketch variants | Same greedy loop with curves, beziers, waves or shapes instead of straight segments | The sketch loop |
| Flow and streamlines | Evenly spaced lines along an edge, gradient or noise field, with spacing or width set by tone | Tone map |
| Modulated paths | A spiral or hatch whose wobble or zig-zag size follows tone | Tone map |
| Hatching | Parallel hatch lines whose spacing follows tone, with optional cross-hatch layers | Tone map |
| Regions and mosaic | Split the image into regions or tiles and render each with its own style | Any style |
| Multi-pen colour | Split colour images into per-pen layers (e.g. CMYK or a custom palette) and run a style per pen | Any style |

## 8. GUI and preview (decided: local web app)

`plotline gui` serves the GUI's static files on localhost and opens the browser. The same files are the hosted page. The engine runs inside the browser as WebAssembly in a Web Worker, which posts frames to the page while a pipeline runs. No server takes part.

**Why in the browser:** one GUI code path serves both the installed and the hosted version, it is identical on every OS, and the hosted version needs nothing installed at all. The MCP shares the engine through the Python extension instead of a server.

What the preview should do:

- **Progressive drawing.** Frames stream in while stages run, and each stage's progress is visible (e.g. "relaxation 18/50").
- **True to scale.** Strokes drawn at the real pen width and colour on the chosen paper size, with zoom and pan.
- **Toggles:** source image versus drawing (or overlaid), and per-pen layer visibility.
- **Pen-up travel overlay** as faint lines, to judge path ordering.
- **Stats:** path count, total pen-down length, pen-up travel and estimated plot time (from user-set speeds).
- **Settings panel** generated from stage declarations, grouped by stage, with advanced settings collapsed. Changing a setting reruns from that stage.
- **Presets:** save and load full settings as a small file. They should be shareable.
- **Export:** layered SVG, plus the other formats in §10.

The frontend stack is TypeScript, Vite and Preact (§14). Canvas 2D was chosen over WebGL on 2026-10-12, see the decision log. The built assets must ship inside the Python package so `pip install plotline` is all a user needs, and the same assets are deployed as the hosted page.

## 9. MCP server (decided: yes, kept thin)

`plotline mcp` exposes the session through MCP so an assistant can work the same way a person does in the GUI. Sketch of the tools (names and shapes to be finalised):

- `load_image(path)`, `set_canvas(...)`, `set_pens(...)`
- `list_styles()`, `describe_style(style)`, which returns the generated settings schema with help text
- `run(style, settings)`, which returns stats plus a preview PNG
- `update(settings)`, which reruns only from the earliest changed stage
- `render_preview(options)`, e.g. zoomed region, travel overlay, source overlay
- `export(path, format)`

Rules: no algorithm code in the MCP layer, every tool maps to a session method, and previews are returned as images so the assistant can judge the result visually.

## 10. Inputs and outputs

- **Input:** PNG and JPEG. Colour images are converted to tone unless a multi-pen colour style is used.
- **Output:**
  - SVG in millimetres, with one group per pen layer and pen metadata
  - a plain JSON paths format, so other programs can read results
  - PNG previews
  - presets (settings files)
- **Units:** millimetres everywhere in the public API. Pixels exist only inside tone mapping.

## 11. Project rules (public-readiness)

The repo starts private and will go public, so treat every commit as public from day one.

- **License:** MIT. Every runtime dependency must be permissively licensed (MIT, BSD, Apache-2.0 or similar). Check before adding one, and do not add copyleft dependencies.
- **Original implementations:** build algorithms from published descriptions (papers, textbooks, well-known techniques). Cite the source in the module docstring. Do not copy code from other projects unless their license is compatible, and credit them if you do.
- **Other products:** don't borrow names, UI, presets or marketing claims from other products, and don't compare against or disparage them. This avoids trademark use and implied affiliation. Citing papers and published descriptions is expected. Naming a product to state a fact its maker publishes, such as the pen bank's widths, is a reference, not a borrowing; our own settings or measurements never go under a product's name.
- **No third-party images:** tests, examples and docs use images that are generated in code, made for this project, or clearly public domain or CC0, with the source noted.
- **No personal details:** no machine-specific paths, credentials or private notes in the repo.
- **Tests and CI from the first commit:**
  - unit tests per stage
  - determinism tests (same seed, same output)
  - small "golden" output tests on tiny synthetic images
  - lint and format checks
- **Documentation:** every style and setting has help text, which also feeds the GUI and MCP. The README stays accurate about what works.

## 12. Open questions for the builder

Decide these during design, write the decision and reason into this brief, or ask the project owner if it's a matter of taste.

The ten questions the brief opened with (frontend stack, performance strategy, continuous-line solver, dot geometry, settings schema, cache, toolchain adapters, third-party styles, Python and tooling, preset format) were settled on 2026-10-11. Each decision and its reason is in §14, and the core ones are detailed in [the spec for the first sub-project](superpowers/specs/2026-10-11-core-engine-design.md). Still open:

1. **Scope of the naming rule.** The rule not to name other tools conflicts with vendor-namespaced SVG layer attributes that some plotter software reads, and with adapters that by nature target a named toolchain. The working reading is that the rule covers prose, identifiers and commit messages, and that standard format names are fine. The owner should confirm before the adapter milestone. Until then SVG export uses plain groups only.
2. **Which toolchains to adapt first.** Driven by the owner's own setup.
3. **Canvas 2D or WebGL** was settled on 2026-10-12 in [the GUI spec](superpowers/specs/2026-10-12-gui-design.md): Canvas 2D, revisited if a redraw takes over 100 ms.
4. **Disk cache.** Whether and when. Cache keys already support it.
5. **Standalone Rust CLI binary.** Users without Python could get the CLI as one executable. Not before the Python surface settles.
6. **Python stage API.** Stages written in Python through callbacks would run natively but not in the browser. Decide when someone asks for one.
7. **JSON-declared styles.** Compositions of existing stages as data, so a new style needs no code. Decide after the Voronoi family shows which compositions people want.

## 13. Suggested milestones

1. **Scaffold:** package layout, tooling, CI, licence checks, a synthetic test image set.
2. **Core:** tone map, canvas, pens, path and layer types, the stage/pipeline/cache machinery, SVG and JSON export, a minimal CLI. Prove caching and cancellation with a trivial test style.
3. **Voronoi stipple:** the point placement stage plus the stipple renderer, with path ordering. Includes a benchmark script that measures the performance targets in §14, so the numbers are checked before they are published.
4. **Continuous line and dashes:** the remaining renderers, reusing the cached points.
5. **Sketch Lines.**
6. **GUI:** Web Worker, frame transfer, preview features (§8), presets, the hosted page.
7. **MCP server.**
8. **0.1 release prep:** docs, examples with project-owned images, and this checklist before the repo goes public:
   - add `src/plotline/py.typed` so third-party style authors get type information (done on 2026-10-12, see "Release workflow")
   - add `[project.urls]` and classifiers to `pyproject.toml` (done on 2026-10-12)
   - add a CHANGELOG and a short CONTRIBUTING (the verification set, the licence rule, the naming rule)
   - scan the full git history for personal paths and secrets (done on 2026-10-12, see "Repository history")
   - restore the full CI matrix (Windows, macOS, Python 3.12) (done when the repo went public)
   - add the release workflow, which builds the Vite bundle before the wheel and fails if `src/plotline/web/index.html` is missing (done on 2026-10-12, see "Release workflow"), and register the trusted publisher on PyPI (owner action)
   - bump the version from `0.1.0.dev0` to `0.1.0` and tag `v0.1.0`

Each milestone, or pair of milestones, gets its own spec under `docs/superpowers/specs/` before it is built. Milestones 1 and 2 share the first spec, which also pulls in a minimal ordering stage, the preview PNG, stats and presets so results can be checked visually before the GUI exists. The Rust port, decided on 2026-10-11, sits between milestones 2 and 3 under [its own spec](superpowers/specs/2026-10-11-rust-core-design.md): the Python engine is rewritten in Rust with the same design, so the algorithm milestones are written once, in Rust.

Milestone 2 was completed in Python and then ported to Rust on 2026-10-12 under the Rust core spec. The Python engine is gone: the Python package is now a thin layer over the Rust extension, and the algorithm milestones that follow proceed in Rust. Milestone 6 is built before 3 to 5, see the decision log. It was built on 2026-10-12 under [the GUI spec](superpowers/specs/2026-10-12-gui-design.md).

## 14. Decision log

| Decision | Choice | Reason |
|---|---|---|
| Name | plotline | Plain and memorable. Free on PyPI with no clashing repos. |
| License | MIT | Widest adoption among plotter users. The algorithms are published, so there is little to protect with copyleft. Contributing is easy. |
| Language | Python (core and server). Superseded on 2026-10-11, see "Engine language" | Strong numeric and image libraries, and familiar to plotter and creative-coding users. |
| Engine model | Staged, cached generator pipeline | A fast, visible preview, and shared stages across style families (§6.1). |
| GUI | Local web app. Amended on 2026-10-11, see "GUI engine" | Shares server and session code with the MCP, cross-platform, possible browser demo later (§8). |
| MCP | Yes, a thin layer over the session | Lets an assistant run the try, look, adjust loop. No logic duplicated. |
| First styles | Voronoi stipple, continuous line, dashes, plus Sketch Lines | Point placement underpins the largest group of future styles. Sketch Lines is a contrasting, simple baseline. |
| Scope boundary | Machine-agnostic | Machine-specific output belongs downstream or in adapters, which keeps the core reusable. |
| Other products | Cite published sources; never borrow product names, UI or presets; no comparisons | Avoids trademark use, implied affiliation and disparagement, and keeps a later paid tier from looking hypocritical. |
| Stage inputs | Stages declare the artifact kinds they require and provide; a requirement resolves to the nearest upstream provider | Renderers need both the points and the tone map, and future styles need only the tone map, so "previous stage's output" was too narrow. |
| Settings schema | Superseded on 2026-10-11, see "Settings schema (Rust)". One frozen pydantic v2 model per stage, declared through a `setting()` helper; its JSON Schema feeds the GUI, CLI and MCP | Validation and JSON Schema come free, the licence is MIT, and the MCP SDK already uses it. Unknown keys are rejected, so typos fail loudly. |
| Cache | In memory, bounded by estimated bytes, least-recently-used; keys are content hashes of stage name, version, settings and input keys | Enough for one session. Content-hash keys let a disk cache be added later without changing them. |
| Concurrency | Amended on 2026-10-11, see "Stage contract (Rust)". The pipeline runs in a worker thread; cancellation is a flag checked after every frame | numpy and numba release the interpreter lock, so a thread suffices. Frame granularity is fine enough for the preview. |
| Determinism | Amended on 2026-10-11, see "Cross-target determinism". Every stage that uses randomness has its own `seed` setting feeding a numpy generator; no global random state | Per-stage seeds keep cache keys correct and make tests simple. |
| Python and tooling | Amended on 2026-10-11, see "Rust tooling". Python 3.11+, uv, hatchling, ruff for lint and format, mypy, pytest; CI on Windows, macOS and Linux for 3.11 to 3.13; a licence allow-list check in CI | Confirms the working assumption. uv installs the interpreter, so the developer's system Python does not matter. |
| Performance | Superseded on 2026-10-11, see "Engine language". numpy and scipy baseline, optional numba extra (`plotline[fast]`) for hot kernels, no compiled extension in 0.x; relaxation runs on the raster, coarse to fine; targets for 10k points on a 1000 px image over 50 passes: under 5 s with numba, under 30 s without | The baseline always installs. numba is BSD and speeds up the few hot loops. Multi-platform wheels for an extension cost more than this project can carry. |
| Compiled extension (Rust) | Superseded on 2026-10-11, see "Engine language". Not in 0.x. The hot kernels (relaxation, 2-opt) live in their own modules behind a pure numpy fallback, so a Rust extension built with maturin and abi3 wheels can replace them in 0.2 or later if the numba path misses the targets | The targets are not measured yet, so optimising now is guesswork. A Rust extension adds a toolchain for every contributor and a wheel matrix across three OSes, which the project cannot carry before it is public and the slow kernels are known. |
| Continuous-line solver | Nearest-neighbour construction, then 2-opt and Or-opt with candidate neighbour lists and don't-look bits (Johnson and McGeoch, 1997); frames after construction and then every few hundred improving moves | Standard and well documented. A 2-opt local optimum in the plane has no crossings, so the single line is non-crossing by construction. |
| Dot geometry | A stipple renderer setting: point (one-vertex path), circle outline or spiral fill; default automatic, point when the dot is no wider than the pen, spiral above | Dots stay paths, so export and preview need no special case. |
| Canvas modes | `fit` and `fill` in the first release; `crop` is `fill` plus an offset and comes later | Keeps the canvas type small until someone needs the offset. |
| Presets | JSON with a format name and an integer version, holding style, canvas, pens and per-stage settings; a migration chain per version bump | Same wire format as the GUI and MCP. Unknown keys warn and are dropped; missing keys take defaults. |
| Third-party styles | Superseded on 2026-10-11, see "Third-party styles (Rust)". The registry discovers an entry-point group from day one; the stage API is documented as unstable until 0.2 | Discovery is cheap. Promising stability before the Voronoi family and GUI exist would be premature. |
| Toolchain adapters | None in 0.1; output is SVG and JSON. Generic G-code goes in an adapter module later, not the core | Keeps the core machine-agnostic. The owner's own toolchain decides which adapters come first. |
| Frontend stack | TypeScript, Vite and Preact; the bundle is built in CI and shipped in the wheel; during frontend work the Vite dev server serves the page directly (amended on 2026-10-11, the proxy flag is gone with the server) | Type checking suits a UI generated from a schema. Preact is tiny, MIT and familiar. Python-only contributors get the GUI from the wheel; only frontend work needs Node. |
| Frame transport (GUI) | Superseded on 2026-10-11, see "GUI engine". HTTP for commands, a WebSocket carrying frames as binary float32 arrays, throttled server-side | Path data for tens of thousands of points is too heavy as JSON at preview frame rates. |
| Preview rasterizer | Superseded on 2026-10-11, see "Preview rasterizer (Rust)". Pillow with 4x supersampling | The usual vector rasterizers for Python are LGPL. Pillow is permissive and already a dependency. |
| Session model | One session per server process; the API carries a session id | Simplest GUI and MCP. Tabs can come later without a rewrite. |
| First sub-project scope | Spec 1 covers milestones 1 and 2 plus a minimal ordering stage, preview PNG, stats and presets | Results can be checked visually before the GUI exists, and the pipeline is realistic enough to test ordering. |
| Licence check policy | A dependency's own licence governs. A binary wheel's notice for a bundled runtime library covered by the GCC Runtime Library Exception does not make the dependency copyleft; such packages are recorded as overrides in the check script with the verified licence and URL. | Keeps the check honest without blocking on toolchain runtime notices that every compiled wheel carries. |
| Artifact sharing | Stage inputs and results are shared with the cache by reference and are never modified in place; the runner marks cached arrays read-only (in Rust, `Arc` makes them immutable by construction) | Copying every artifact would defeat the cache, and a read-only flag turns a silent corruption into an immediate error. |
| CI runs | Runs on pushes to `main` and on pull requests, on Linux, Windows and macOS for Python 3.11 to 3.13. While the repo was private the matrix was Linux on Python 3.11 and 3.13; the full matrix returned when it went public | Private repos get 2,000 free Actions minutes a month, Windows bills at 2x and macOS at 10x, so the full matrix would cap out after about 25 pushes. Public repos get unlimited standard runners. The owner develops and tests on Windows, so Linux is the run that adds information. |
| Release publishing | None until milestone 8. Then a workflow on `v*` tags builds the wheel (with the GUI bundle) and publishes to PyPI through trusted publishing | Nothing to publish before 0.1, and trusted publishing needs no stored token. |
| Engine language | Rust, one crate compiled to a native Python extension (PyO3, maturin, abi3 wheels) and to WebAssembly (wasm-bindgen); the design from spec 1 is unchanged | The owner wants a version with every GUI feature and nothing to install. Only an engine that runs in the browser gives that, and Rust compiles to WebAssembly at near native speed. The same crate serves Python users through bindings, so there is still one engine. Decided while the Python engine is small and before any algorithm milestone, which is the cheapest moment. |
| Pyodide route | Rejected | Running the Python engine in the browser means downloading a Python runtime and losing numba, which puts relaxation far outside its targets. |
| Hosts | Python stays the host for the library, CLI and MCP; the browser is the host for the GUI; the engine has no thread, file or network code | Keeps the thin layers thin and the engine portable. A standalone Rust CLI is an open question. |
| GUI engine | The GUI runs the WebAssembly engine in a Web Worker, both installed and hosted; `plotline gui` serves the static bundle on localhost with the standard library and opens the browser; no web framework, no WebSocket | One GUI code path. Settles the old open question about the server stack: there is none. |
| Stage contract (Rust) | Stages are step functions: `start` returns a state machine and each `step` returns a frame or the artifact; the runner checks a cancel flag between steps | Rust has no generators, and a browser host cannot interrupt a running call without shared memory, which a static host cannot enable. Pulling steps lets the worker yield to its event loop between them. |
| Cross-target determinism | PCG64 per stage seed, transcendental functions through the `libm` crate, no fused multiply-add, no SIMD paths; a source-scan test forbids the platform float methods outside one module; the golden files are checked by both the native tests and the WebAssembly smoke test | Output must match bit for bit between native and browser builds, or the goldens and the users' presets would mean different things on different targets. |
| Path layout | One flat coordinate buffer with offsets per layer | Crosses both FFI boundaries as two typed arrays with no per-path copies. |
| Settings schema (Rust) | serde structs with `deny_unknown_fields` and schemars for the JSON Schema; help text from doc comments; ranges and `x-` extensions in schemars attributes; a generic check of values against the schema before deserializing; the CLI coerces strings in Python | Keeps the one-declaration rule without pydantic. |
| Preview rasterizer (Rust) | tiny-skia (BSD-3), anti-aliased strokes, no supersampling | Permissive, pure Rust, builds for WebAssembly. |
| Image decoding | The `image` crate with PNG and JPEG only; 16-bit greyscale scaled by 65535; other formats refused with a message | Two formats cover photos and scans, and fewer formats keep the WebAssembly module small. |
| Cache keys (Rust) | blake3 of canonical JSON | Fast and stable across platforms; keys keep supporting a disk cache. |
| Spatial index | A uniform grid bucket index in the crate; no kd-tree dependency | About sixty lines, deterministic, fast enough in Rust for a hundred thousand endpoints. |
| Third-party styles (Rust) | New stages and styles are Rust code behind the `Stage` trait in 0.x; the Python entry-point group is dropped | Stages must run in the browser, which Python stages cannot. A Python stage API and JSON-declared styles are open questions. |
| GUI bundle shipping | Amended on 2026-10-12, see "Bundle location". The release workflow builds the Vite bundle before the wheel and fails if `web/dist/index.html` is missing; `plotline gui` fails with a clear message when the installed package has no bundle; a Pages workflow deploys the same bundle, gated on the repository being public | Settles the old open question. The wheel and the hosted page are built from the same files. |
| Rust tooling | Stable toolchain pinned in `rust-toolchain.toml`, rustfmt, clippy with warnings denied, cargo-deny for licences, wasm-pack for the browser build, maturin for the extension; CI gains a rust job and a wasm job | Standard tools, all permissively licensed. |
| LLVM exception (Rust) | cargo-deny allows `Apache-2.0 WITH LLVM-exception` for `target-lexicon` alone, a build-time dependency of the PyO3 build script | The exception only adds a permission to Apache-2.0, so the licence is at least as permissive; scoping it to one crate keeps any other use visible. |
| Run generations | Each run carries a generation number; a run writes its terminal state and drawing to the session only while it is still the newest run, and `start` shows the new run as running at once | A cancelled older run that finishes late must never overwrite a newer run's state; the session has no threads, so the guard lives in the slot the hosts share. |
| Order-stage ties | Exact distance ties go to the lowest endpoint index, with a uniform grid index in the crate | The kd-tree's tie order was not reproducible; the lowest index is deterministic on every target, and the greedy result is otherwise unchanged. |
| Cross-engine check | The Rust `grid` output was compared with the Python engine's golden files before the Python engine was removed: equal dot sets within 0.001 mm and equal counts (544 and 669), with order differing only at exact ties | The goldens that remain pin Rust behaviour; the comparison is recorded here because the Python side no longer exists to repeat it. |
| No-image error | Starting a run without an image raises `RuntimeError` in Python; refused values raise `ValueError` | Spec 1 named `RuntimeError` for the missing-image case and spec 2 keeps that split between state errors and validation errors. |
| CLI error surface | The engine validates pens and canvases when they are set, so a bad `--pen` exits with status 1 like other input errors, and preset errors carry the engine's message without an `invalid preset:` prefix | One validation path in the engine instead of two; the exit code and prefix were implementation details of the Python engine. |
| Hidden test style | A `_slow` style with configurable frames, delay and failure is always compiled and hidden from `list_styles` | Host tests for cancellation, throttling and errors need a slow, controllable style, and release builds are what the hosts test. |
| Image bit depth | 16-bit integer greyscale loads scaled by 65535; integer arrays given to the session are scaled by the largest value of their type; only PNG and JPEG decode, so a float image cannot occur | Scans are a named input and 16-bit greyscale is their common form; guessing a white level would plot the wrong tones silently. |
| Fill-mode rounding | In `fill` the tone map height may differ from the drawing rect by up to one source pixel's worth of mm, because a tone map has one scale | A second scale would complicate every stage for a sub-millimetre effect that only shows when a tiny source is upsampled; placers may clamp to the drawing rect. |
| Gamma direction | `gamma` applies as `Y ** gamma`, so values above 1 darken midtones; the help text states the direction | Matches the formula fixed in the core spec; a convention users can read from the help text. |
| Image size limit | The image library's decode memory limit stays in force; a too-large file is reported as a plain error | The whole image is decoded before downsizing, so the limit protects memory on ordinary machines; users can pre-resize. |
| Engine route, reconfirmed | On 2026-10-12 the owner chose the Rust branch over running the Python engine in the browser with Pyodide, and it was merged into `main` | For a hosted page the live preview of the relaxation stage decides it: near-native speed in WebAssembly against minutes under Pyodide, a 1.2 MB module against a 15 to 20 MB runtime, and no copyleft exception, since Pyodide is MPL-2.0. The cost, a Rust toolchain for contributors and a wheel matrix, was accepted. |
| Milestone order | The GUI (milestone 6) is built right after the engine port, before the Voronoi family and Sketch Lines | The owner wants to test the engine through the GUI, and the GUI is generated from the stage declarations, so the styles that come later need no GUI changes. |
| Preview renderer | Canvas 2D, one path object per pen layer; WebGL is revisited when a redraw takes over 100 ms | A layer of a hundred thousand vertices strokes in tens of milliseconds, which covers the planned styles, and Canvas 2D needs no shaders or buffer management. |
| Worker protocol | Commands as `{id, method, args}` answered with the whole state; frames, results and run ends as events; geometry as typed arrays in the transfer list | One reply shape keeps the page simple, and transferring the arrays means no copy between the worker and the page. |
| Driver cadence | The worker posts at most 30 frames a second plus every stage's final frame, and yields to its event loop every 50 ms of work through a zero-delay macrotask | Fine-grained stages would otherwise pay a timer clamp per frame, and 50 ms bounds how long a cancel waits. |
| Bundle location | `vite build` writes `src/plotline/web`, git-ignored; `plotline gui`, the wheel and the hosted page all use that one directory | One location for all three means they cannot drift apart. |
| Frames in the preview | A frame is drawn with the first pen's width and colour; the result is drawn per layer with each pen | Frames carry no pen, and the styles planned draw with one pen until the multi-pen family exists. |
| Pen bank | Amended on 2026-10-12, see "Pens dialog". A built-in list of common fineliners with the widths their makers publish, named by maker and size, shared by every front end | Picking a pen by name beats typing a width. The names are the makers' own for their hardware, which is nominative use, not a preset borrowed from another plotter program. Reconfirmed on 2026-10-12 before going public: a size code such as "005" only means a width next to its maker's name, and every width is the maker's own figure. The built-in printer was different because it put the owner's own measurements under a printer's name. |
| G-code adapter | `export::gcode` in the core crate writes one pen layer as strict G-code for a `Machine` the session holds, one file per pen with no pause; built on 2026-10-12 under [the machines spec](superpowers/specs/2026-10-12-machines-gcode-design.md) | The browser and Python share one writer, the strict dialect lets a downstream checker verify every file, and a printer that pauses by reheating makes each pen its own job. The core stays machine-agnostic: only the exporter, the preset and the speeds read the machine. |
| Machine in the preset | Amended on 2026-10-12, see "Preset version 3". Preset version 2 carries the machine or null; a version 1 preset migrates with null | A preset is the whole setup, and the machine decides the page size and the plot speeds. |
| Built-in machines | None, decided on 2026-10-12. The GUI's "New machine" starts from placeholder values, and a user keeps their machine in their own preset | A machine's area and pen heights are measured on one rig, so a built-in is unsafe on anyone else's, and the owner's test rig was meant as a personal preset. Naming a built-in after a commercial printer also broke the rule on other products. |
| Line styles first | `hatch`, `waves` and `spiral` were built on 2026-10-12 under [the line styles spec](superpowers/specs/2026-10-12-line-styles-design.md), before the Voronoi family; their wobble is clamped to the tone map and the G-code writer refuses a vertex off the page | The owner tests on a pen-fitted 3D printer, where every pen lift is a Z move that wears the machine, and long strokes give tone with few lifts. The clamp and the check keep a style from sending the pen outside the machine's area. |
| GUI look | Hand-written CSS on colour tokens declared with `light-dark()`; light and dark themes that follow the system, with a toolbar button to pin either one, kept in the browser; Lucide icons (ISC) through `lucide-preact`, their licence shipped beside the bundle like preact's; no CSS framework or component library; the paper stays white in both themes | The owner asked on 2026-10-12 for a more modern look with a dark mode. The form is generated from the stage schemas, so a few dozen rules on the native controls restyle every panel at once, and `color-scheme` switches the native controls and scrollbars too. A utility-class framework would rewrite every component's markup, and the common component libraries target React and need a compatibility layer. ISC is permissive and the icons are tree-shaken to the dozen in use. The paper is the real sheet, so only the backdrop around it changes with the theme. |
| Ink style | `ink` (tone, ink, order) outlines every ink shape and fills it with rings from the mask's distance transform, and draws ink too thin for a ring down its middle from a thinned skeleton; built on 2026-10-12 under [the ink and cleanup spec](superpowers/specs/2026-10-12-ink-and-cleanup-design.md) | The owner's best plots came from this method in a separate script. A test pass on 34 images showed that no other style keeps the shapes and weights of line art. |
| Style defaults | A style may override the defaults of its stages; `ink` sets `tone.resolution` to 2500, and `waves` and `spiral` set `tone.gamma` to 1.9 (amended on 2026-10-12, see "Rippling gamma") | Averaging a drawing into 1000 pixels greys out its hairlines, and the other styles need no finer map. |
| Hatch defaults | Three levels from threshold 0.25, and a light gap narrower than the spacing does not end a line | At one level the default turned a photograph into a silhouette, and on engravings unbridged gaps gave up to six times more strokes and as much pen-up travel as pen-down. |
| WebP input | The image crate's `webp` decoder (`image-webp`, MIT or Apache-2.0) is enabled | Images saved from the web are often WebP; two of the owner's eight test images were. |
| Test build | `cargo test` builds at opt-level 1 | Whole-style tests on real-sized rasters took nearly three minutes unoptimised and under thirty seconds at opt-level 1. |
| Wasm size limit | The CI size check allows 2,000,000 bytes, up from the 1.5 MB of the Rust core spec | The ink style and the WebP decoder took the module from 1,299,070 to 1,500,838 bytes (574 KB gzipped), just over the old limit. Both are wanted, and shrinking with `wasm-opt -Os` would slow the live preview. The check stays to catch an accidental jump, such as a heavy dependency, and the log still prints the size. |
| Repository history | Rewritten on 2026-10-12 before going public: every commit carries the owner's GitHub noreply address, and a printer brand was taken out of three old places. The history was also scanned for personal paths, secrets and images, and none were found. The earlier private repository is kept, renamed, as a private archive with the first five pull requests | The owner's personal address was on every commit. GitHub keeps a pull request's commits even after a force-push, so only a new repository removes them. |
| GUI layout | Amended on 2026-10-12, see "Settings dialog". The style's own settings first and open, path order and tone (as "Image") collapsed, help in tooltips behind a "Show help" switch; SVG export and a zoom readout in the toolbar; the right column runs page, pens, export, plot time, presets, then G-code collapsed | Most plotter owners export SVG to their machine's own software, so the 3D-printer form no longer leads, and the controls that shape the drawing are the first thing in view. |
| Wide blur | From a sigma of 4 pixels up, the tone blur is three running-sum box filters per direction with Kovesi's widths (Wells 1986; Kovesi 2010); below it, the true Gaussian; the GUI shows a run as "stopping…" from the moment Stop is pressed | A direct Gaussian costs six sigma taps a pixel, so a 10 mm blur on a 4000-pixel map took over five seconds in one step that Stop could not interrupt. The boxes cost the same at any sigma and stay within a few percent of the Gaussian; small blurs, such as the ink style's softening, keep their exact output. |
| GUI feedback | An outside usability audit on 2026-10-12 led to these: errors show in the panel that caused them (image, page, pens, presets); a rejected image says the previous one is still open; the pens panel says when the drawing uses fewer pens than are listed, and that a hidden pen still exports; an empty drawing suggests which settings to try; the preview says when the pen is under a device pixel wide at the current zoom; a skip link reaches the toolbar, and the focused preview pans with the arrow keys, zooms with plus and minus and fits with 0 | Each was a place where the GUI worked as built but a person read it wrongly, such as a grid of 0.3 mm dots looking like a blank page at the fit zoom. The unused-pens note comes from the result's layers, so it stays right once multi-pen styles exist. |
| Grid spacing and empty stages | The grid's default spacing is 1 mm, down from 2 mm, the same as the hatch spacing; the golden files pin 2 mm, the spacing they were checked against the Python engine at; the GUI leaves out the section of a stage with no settings, such as the dot renderer, unless it has an error to show | At 2 mm the sample image gave 3,408 dots on A4, too sparse to read as the picture. 1 mm gives about four times as many and roughly four times the plot time. The dot renderer's section opened on nothing, which read as broken. |
| Machine area | On 2026-10-12, under [spec 7](superpowers/specs/2026-10-12-settings-dialog-design.md), the machine's `area` became a corner and a size, `{x, y, width, height}`, and the bed size was dropped; the offset may be negative | The owner found the bed and the four bounds confusing. The bed only checked that the area fits on it, and a size and an offset are what one measures on the machine. A negative offset serves a machine whose zero is the middle of its bed. |
| Settings dialog | The machine, pen lift, G-code and plot speeds moved on 2026-10-12 from the right column into a modal dialog opened from the toolbar, built on the native `<dialog>`; changes apply at once; the plot speeds are read-only while a machine is set; the right column keeps Page, Pens, Presets and Export | They are set once per setup, not per drawing, and the owner asked for a pop-up settings menu. Applying at once matches the panels and needs no undo. The session already resets the speeds from the machine on every change, so editing them beside a machine only let them drift. |
| Preset version 3 | Version 3 carries the machine with the new area; the migration from 2 drops `bed_mm` and turns the four bounds into a corner and a size, leaving anything else for `parse` to name | Saved preset files and the setup the browser remembers keep loading after the change. |
| Going public | The repository went public on 2026-10-12, at milestone 6 and before the 0.1 release, and the Pages workflow deploys the hosted page from `main`. The rest of the milestone 8 checklist stays with the release | The history rewrite and the naming checks were done, and a hosted page needs a public repository on the free plan. Type information and project URLs only matter once a package is on PyPI, and the CHANGELOG and CONTRIBUTING come with the first release. |
| CI after going public | Set on 2026-10-12: a ruleset on `main` requires a pull request and every CI job, and blocks force-pushes and deletion; admins may bypass only on a pull request. Secret scanning with push protection and private vulnerability reporting are on, and merged branches are deleted. Workflow actions are pinned by commit SHA and Dependabot bumps them monthly; the toolchain file pins the Rust version and the jobs install it with `rustup toolchain install`, and `taiki-e/install-action` replaces the unmaintained wasm-pack action; every job has a 30-minute timeout and a new push to a pull request cancels its older run | Pages deploys every push to `main`, so only green pull requests may reach it. Push protection enforces the no-credentials rule at push time. A pinned SHA cannot be moved under the Pages job, which can deploy. With clippy denying warnings, an unpinned stable toolchain could turn `main` red on a Rust release day. The old actions targeted Node 20, which the runners are dropping. Dependency bumps beyond security fixes stay deliberate so each passes the licence check. |
| Sample image | The GUI's "Sample image" is a still life rendered in code (`synthetic::still_life`): three balls and a cube on a floor under a soft key light, 600 x 875 pixels, the shape of the default A4 drawing area; it replaced a 400 x 300 left-to-right gradient on 2026-10-12 | The gradient showed tone and nothing else, so no style could show shapes, edges or shading on it, and as a landscape image it filled half the default page. A rendered scene has the full tonal range, curved and flat shading, hard and soft edges and cast shadows, and keeps the rule against third-party images with no file in the repository. It renders in about 0.4 s in the browser's worker and matches the native build bit for bit. A bundled photograph made for the project would show more texture, at the cost of a binary in the repository and the bundle. |
| Ink colours | A shared list of ten common ink colours under plain names (black, grey, brown, red, orange, yellow, green, blue, purple, pink) in `pens::INKS`, added on 2026-10-12 (amended the same day, see "Pens dialog"); the GUI offers them as swatches in each pen's colour picker through a `<datalist>`, `plotline.inks()` returns them, and `--pen` takes an ink name where it takes a colour; the bank stays black | The owner asked for coloured pen options. Most fineliners come in several inks, so colour is a choice per pen rather than more bank rows, which would multiply every size by every ink. The values are this project's own screen colours, so they go under plain names and never under a maker's, and a pen whose exact colour matters is given its own. A datalist adds no control to the crowded pen row; a browser that ignores it on colour inputs still opens its usual picker. Drawings still use one pen until the multi-pen colour family exists. |
| Colour test images | `synthetic::color_bars` (the eight corners of the RGB cube, light to dark) and `synthetic::scene` (a made-up landscape built from sums and products only), added on 2026-10-12; the tone stage is tested on the bars, and every style is tested for determinism and for drawing something on the scene | The owner asked for colour photos in the test suite. Every earlier test image was grey, so nothing checked the colour-to-tone conversion past the luminance unit test. Generated in code under the image rule; with no `sin` or `sqrt` the scene is the same on every platform. Colour file decoding was already covered in `image.rs`. |
| GUI tour | Added on 2026-10-12: a card that steps through the page and rings the element each step names by its `data-tour` attribute. The steps are one list in `web/src/tour.tsx`. The tour opens once on a browser's first visit and again from Tour in the toolbar. A test fails when a step names an element that does not exist, and the project rules say a user-visible GUI feature adds or updates its step | The owner asked for a tutorial that can grow with the features. Pointing at the real controls teaches the layout faster than a page of text, and a list of steps makes a new feature one more entry. The test stops a renamed element from silently breaking a step. The card is not modal, so the user can try what a step shows. CLI users keep the README's "Try it". No dependency is needed: the card is about a hundred lines of Preact. |
| Pens dialog | On 2026-10-12 the pens moved into a modal dialog, opened by an "Edit pens" button in the Pens panel and built on the same `<dialog>` shell as Settings. The panel keeps one row per pen: its swatch, name, width and preview toggle. The dialog lists the pens in drawing order, with add, duplicate, move and remove. It edits the chosen pen's name, line width and colour, showing the inks as swatch buttons in place of the `<datalist>`, with a picker for any other colour. It also browses the bank by kind and by search. Choosing a size sets the pen's name and width and keeps its colour. The bank grew from 5 fineliner models to 25 models and 121 pens in five kinds (fineliner, technical pen, gel pen, ballpoint, marker), and each entry carries its kind and model. Every width is the width of the line as the maker publishes it, with the source noted above each model. Pens whose makers give only a ball or tip size are left out, among them several popular gel pens and ballpoints. So are sizes given only as a range, and brush pens. | The owner asked for the pen settings to pop out like Settings, and for a robust menu with many common plotting pens. A row of five controls per pen crowded the right column, and a select of 40 names was hard to search, let alone one of 121. Pens change per drawing, so their summary and preview toggles stay in the column. A gel pen's or ballpoint's ball is wider than the line it draws: Pilot's Juice 0.5 draws a 0.31 mm line. Using the ball size would make the preview and the styles' spacing wrong, so only a published line width counts. Swatch buttons fit the dialog, which has room for them, and work in every browser, unlike a `<datalist>` on a colour input. |
| Regression gallery | Added on 2026-10-12: `cargo run --release -p plotline-core --example gallery -- <out> [--baseline <dir>]` draws every listed style on the still life, the scene and the gradient with a 0.1 mm and a 0.5 mm pen, and writes the previews, `stats.json` and an `index.html` grid. With a baseline it compares each preview byte for byte, outlines the changed and new cells with their stat changes, and switches them between new, baseline and difference views. It always exits 0 and writes under `target/`. No plotter simulation was built | The owner wanted to judge by eye how a change to the styles or pens looks, without waiting on a plot. The engine is deterministic, so equal bytes mean an unchanged drawing and no image diff code is needed; the difference view is CSS blending. A simulation of ink and motion was considered and left out: with no real plot to calibrate against, its numbers would be guesses. The tool runs only on demand, so CI stays as fast, and many changes are deliberate, so a change is reported rather than failed. The pictures stay out of the repository under the image rule. |
| Rippling gamma | Set on 2026-10-12: `waves` and `spiral` start from a tone gamma of 1.9; `hatch` and `grid` keep 1 | An image stores its grey values gamma-encoded, but ink and paper mix linearly to the eye. So a style that inks in proportion to the grey value draws the midtones too light, and at a gamma of 1 both styles did. The test used eight public-domain and CC0 photos and the three test images, all kept outside the repository. Each drawing was compared with its image as "Tone matching" describes. The best gamma lay between 1.6 and 2.0 for every image. At 1.9 the error was between a half (the still life) and a hundredth of the error at 1, and close to each image's own best. At 2.0 the still life and the bridge photo got worse. In the regression gallery only the waves and spiral cells changed: the shapes read more clearly, and pen-down distance and plot time rose by 21 to 29 percent. Hatch's best gamma ranged from 0.9 to 1.4 between photos, so no single value suits it. Grid at its defaults cannot draw darker than about 7 percent ink. A browser that remembers an earlier setup keeps its old gamma until the style is chosen again. |
| Tone matching | An "optimize" button that would search the settings for the drawing closest to the image was tried on 2026-10-12 and not built | A throwaway search over the tone levels, gamma, contrast and one style setting used the simplex method of J. A. Nelder and R. Mead ("A simplex method for function minimization", *The Computer Journal* 7(4), 1965, pp. 308-313). It scored each try by blurring the drawing's reflectance and the image's linear light by the same 1.5 mm Gaussian, as model-based halftoning does (M. Analoui and J. P. Allebach, "Model-based halftoning using direct binary search", *Proc. SPIE* 1666, 1992), then took the mean squared difference in sRGB. Aimed at the image's own black, the search made every drawing worse to look at: no style reaches black, so it darkened everything and lost the shapes. Aimed at the darkest tone each style can draw, it kept the shapes. But on waves and spiral a fixed gamma gave almost all of the gain ("Rippling gamma"). On hatch the results came closer in tone but looked heavier, a matter of taste, and grid cannot draw tone at its defaults. One press would take 10 to 20 seconds for hatch and 16 to 42 for waves and spiral in the browser, mostly spent rasterising each try. Worth revisiting for a style whose best settings vary from image to image, with a cheaper coverage estimate than rasterising. |
| Release workflow | Added on 2026-10-12 as `.github/workflows/release.yml`. One job builds the GUI bundle once and fails without `index.html`; abi3 wheels for Python 3.11 and later are built with maturin for Linux x86_64 and aarch64 (manylinux), Windows x64 and macOS universal2, each installed and run before upload, plus an sdist that carries the bundle. A `v*` tag must match the package version, and only a tag publishes, through a trusted publisher bound to the `pypi` environment. A pull request that changes the workflow runs the build without publishing. The first upload is the current `0.1.0.dev0`, ahead of the 0.1 release | PyPI cannot reserve a name: a pending trusted publisher does not hold it, and a placeholder upload can be reclaimed under PEP 541, so a working pre-release is the only way to secure `plotline` before 0.1. It also proves the wheel build before the release depends on it. pip takes a pre-release only with `--pre` or when no final release exists, so until 0.1 `pip install plotline` installs the dev build. The aarch64 Linux wheel serves plotters driven from a Raspberry Pi. musllinux and Windows on Arm users build from the sdist, which needs Rust. |
| Third-party notices | Added on 2026-10-12: the release and Pages workflows write `THIRD-PARTY-LICENSES.txt` into the GUI bundle with cargo-about (`about.toml`, `about.hbs`), beside the preact and lucide licences, so it ships in every wheel, every sdist and on the hosted page. It gives each licence text once, with the crates it covers: 38 texts over 79 crates at the time. Build and test dependencies are left out, and so are the workspace's own crates, which are all marked `publish = false` | MIT, BSD and Apache-2.0 ask for their notice in every copy, binaries included, and the wheels and the WebAssembly module are binaries built from those crates. Generating the file in CI keeps it in step with `Cargo.lock`, with no file to update by hand. One file covers both targets, so it also names a few crates that only one of them uses. Local builds skip it, because they are not distributed and cargo-about would be one more tool for every contributor. The accepted licences match `deny.toml`. |
