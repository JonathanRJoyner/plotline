# plotline: project brief

This brief is the starting point for building plotline. It says what the project is for, how it should work, what is already decided and why, and what is left open. If you are picking up this project, read all of it before writing code. If something here turns out to be wrong once you are building, raise it rather than quietly working around it.

## 1. What plotline is

plotline turns raster images (photos, renders, scans) into vector line art that a pen plotter can draw. Tone becomes line: dark areas get dense marks and light areas get sparse ones. The marks can be dots, one continuous line, dashes, sketched strokes, and more styles over time.

It has four parts:

- **A Python library.** Image in, pen paths out. It knows nothing about any particular machine.
- **A live preview GUI.** A local web app that shows each style working as it runs, so you can see what a setting does.
- **An MCP server.** It lets an AI assistant run styles, look at previews, adjust settings and export results.
- **A CLI.** For scripting and batch work.

## 2. Who it's for

- People with pen plotters (purpose-built plotters, converted 3D printers, drawing robots) who want good tonal drawings from images.
- Generative and plotter artists who want styles they can combine and extend in code.
- People who work with an AI assistant and want it to handle the "try settings, look, adjust" loop.

## 3. Goals and success criteria

1. **Quality.** A style's output should look deliberate and plot cleanly. That means no stray specks, no wasted pen-up travel, and the drawing fills its area.
2. **Seeing is understanding.** The preview shows the algorithm progressing (points settling, a path being built). It draws at the real pen width and colour, so what you see is what you plot.
3. **Fast iteration.** Changing a late-stage setting, such as dot size or switching stipple to a continuous line, should update the preview in about a second. It should not rerun the slow stages.
4. **One engine, many front ends.** The GUI, MCP, CLI and toolchain adapters all call the same core API. No algorithm code lives in a front end.
5. **Extensible.** A new style is built mostly from existing stages. A third party could add one without changing the core.
6. **Ready to be public.** Clean history, permissive licensing all the way down, documentation and tests from day one. See §11.

**The first release succeeds when** a user can run `plotline gui` and load a photo, then:
- watch a Voronoi stipple converge in the preview
- switch the rendering between stipple, continuous line and dashes without the points being recomputed
- try Sketch Lines
- export a layered SVG at a chosen paper size

The same workflow must also work through the CLI and the MCP.

## 4. Non-goals (for now)

- **Machine-specific output**, such as G-code for one particular printer or firmware quirks. plotline outputs plain vector data, and adapting it to a machine is the downstream user's job. Generic G-code export is an open question (§12).
- **Driving plotters directly** (serial control, job queues).
- **Image editing** beyond what tone mapping needs (crop, levels, contrast, gamma, blur, invert).
- **Generative art from nothing.** plotline is image-driven.
- **A hosted service.** Everything runs locally. A browser-only demo may come later.

## 5. Core concepts

- **Source image:** the raster input. Internally it becomes a **tone map**, a 2D float array where 0 is white and 1 is full ink, after the user's adjustments.
- **Canvas:** the physical drawing area in millimetres, plus how the image fits it (fit, fill or crop).
- **Pen:** a named physical pen with a stroke width in mm and a colour. Pens affect both the preview and how styles space marks.
- **Layer:** the paths for one pen. A result is an ordered list of layers.
- **Path:** a polyline, an N×2 array of mm coordinates. A single dot is a very short path or a small circle, decided per style. Pen-down along the path, pen-up between paths.
- **Style:** a named recipe, defined as a pipeline of stages with default settings (e.g. "Voronoi Stipple").
- **Stage:** one step in a pipeline (tone mapping, point placement, rendering, path ordering…). Each stage declares its settings, takes the previous stage's output and produces its own.
- **Frame:** an intermediate snapshot a stage produces while it runs, for the preview.
- **Session:** one working context: a source image, a canvas, pens, a chosen style, settings, cached stage outputs and the latest result.

## 6. Architecture

```
            ┌────────────────────────────────────────────┐
            │  core: tone mapping, stages, styles, paths   │
            │  (pure library, no I/O, machine-agnostic)    │
            └──────────────────────┬─────────────────────┘
                                   │
            ┌──────────────────────┴─────────────────────┐
            │  session: load image, run/rerun pipeline,   │
            │  stage cache, preview rendering, export     │
            └──┬────────────┬──────────────┬─────────┬──┘
               │            │              │         │
              CLI     local web GUI       MCP     toolchain
                     (server + browser)            adapters
```

### 6.1 Staged, cached pipeline (decided)

A style is a short chain of stages:

```
image → tone map → point placement → renderer → path ordering → result
         (levels,    (slow: iterative  (stipple,   (cut pen-up
          gamma)      relaxation)       line, dash)  travel)
```

- **Stages are generators.** Each stage yields frames as it works (e.g. "points after relaxation pass 12 of 50") and finishes with its final output. The GUI streams frames; the CLI and MCP can ignore them and wait for the end.
- **Settings are declared, not implicit.** Each stage declares its settings: name, type, range, default, help text and whether it is "advanced". The GUI's controls, the CLI flags and the MCP tool schemas are all generated from that one declaration.
- **Caching.** The session stores each stage's output, keyed by a hash of that stage's settings plus the upstream key. Changing a setting reruns only that stage and the ones after it. Switching between styles that share their early stages (e.g. Voronoi stipple to Voronoi continuous line) reuses the shared outputs.
- **Cancellation.** A new run cancels an unfinished one. Stages must check for cancellation between iterations.
- **Determinism.** Any randomness comes from a seed in the settings. The same input and settings must give the same output, which tests rely on.
- **Styles that don't split into stages** (e.g. Sketch Lines) are a single stage. That's fine.

**Why:** the preview is only useful if adjusting settings is fast. Point placement is the slow part and many styles share it, so caching per stage is what makes tweaking feel immediate. It also matches how the styles relate to each other (§7): new styles are mostly new combinations of existing stages. Retrofitting caching onto monolithic style functions later would mean rewriting every style.

### 6.2 Session layer

The session owns state and side effects: loading images, managing the canvas and pens, running pipelines in a worker, holding the cache, rendering preview images (PNG at true pen width), computing stats and exporting. Every front end goes through it.

### 6.3 Front ends

- **CLI:** `plotline run <style> image.png --out drawing.svg --set key=value …`, `plotline styles`, `plotline describe <style>`, `plotline gui`, `plotline mcp`.
- **GUI:** see §8.
- **MCP:** see §9.
- **Toolchain adapters:** small optional extras that convert plotline results into the data formats of existing plotter toolchains, so plotline can slot into workflows people already use. Which ones is an open question (§12).

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

`plotline gui` starts a local Python server and opens the browser. The browser UI renders the canvas and settings, and receives frames over a WebSocket (or similar) while a pipeline runs.

**Why a local web app:** the GUI and MCP share the same server and session code. It is identical on Windows, macOS and Linux with nothing to install beyond Python. And it leaves the door open to a browser-only demo later.

What the preview should do:

- **Progressive drawing.** Frames stream in while stages run, and each stage's progress is visible (e.g. "relaxation 18/50").
- **True to scale.** Strokes drawn at the real pen width and colour on the chosen paper size, with zoom and pan.
- **Toggles:** source image versus drawing (or overlaid), and per-pen layer visibility.
- **Pen-up travel overlay** as faint lines, to judge path ordering.
- **Stats:** path count, total pen-down length, pen-up travel and estimated plot time (from user-set speeds).
- **Settings panel** generated from stage declarations, grouped by stage, with advanced settings collapsed. Changing a setting reruns from that stage.
- **Presets:** save and load full settings as a small file. They should be shareable.
- **Export:** layered SVG, plus the other formats in §10.

The frontend stack (vanilla TypeScript vs a framework, and canvas vs WebGL for very large path counts) is open (§12). The built assets must ship inside the Python package so `pip install plotline` is all a user needs.

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

- **Input:** common raster formats (PNG, JPEG, …). Colour images are converted to tone unless a multi-pen colour style is used.
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
- **Other products:** don't borrow names, UI, presets or marketing claims from other products, and don't compare against or disparage them. This avoids trademark use and implied affiliation. Citing papers and published descriptions is expected.
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

1. **Frontend stack** for the GUI, and how assets are built and bundled into the Python package.
2. **Performance strategy** for large point counts (tens of thousands): pure numpy/scipy, an optional JIT, or a compiled extension. Set target timings, e.g. 10k-point relaxation on a 1000-px image in a few seconds on a laptop.
3. **The continuous-line solver:** which construction heuristic and improvement passes, and how to stream intermediate tours to the preview.
4. **Dot geometry:** how a dot is drawn (tiny circle, short tick or spiral), and whether that is a renderer setting.
5. **Settings schema format:** a dataclass, a schema library or a custom declaration. It must generate the GUI controls, CLI flags and MCP schema.
6. **The cache:** in memory only, or optionally on disk across sessions, and how large.
7. **Toolchain adapters:** which existing plotter toolchains to support first, and whether generic G-code export belongs in core or in an adapter.
8. **Third-party styles:** an entry-point plugin mechanism, and when.
9. **Python version floor and packaging tooling.** The working assumption is Python 3.11+ with `uv`, `pytest` and `ruff`; confirm or change.
10. **Preset file format** and versioning, so old presets keep working.

## 13. Suggested milestones

1. **Scaffold:** package layout, tooling, CI, licence checks, a synthetic test image set.
2. **Core:** tone map, canvas, pens, path and layer types, the stage/pipeline/cache machinery, SVG and JSON export, a minimal CLI. Prove caching and cancellation with a trivial test style.
3. **Voronoi stipple:** the point placement stage plus the stipple renderer, with path ordering.
4. **Continuous line and dashes:** the remaining renderers, reusing the cached points.
5. **Sketch Lines.**
6. **GUI:** server, frame streaming, preview features (§8), presets.
7. **MCP server.**
8. **0.1 release prep:** docs, examples with project-owned images, a check before making the repo public.

## 14. Decision log

| Decision | Choice | Reason |
|---|---|---|
| Name | plotline | Plain and memorable. Free on PyPI with no clashing repos. |
| License | MIT | Widest adoption among plotter users. The algorithms are published, so there is little to protect with copyleft. Contributing is easy. |
| Language | Python (core and server) | Strong numeric and image libraries, and familiar to plotter and creative-coding users. |
| Engine model | Staged, cached generator pipeline | A fast, visible preview, and shared stages across style families (§6.1). |
| GUI | Local web app | Shares server and session code with the MCP, cross-platform, possible browser demo later (§8). |
| MCP | Yes, a thin layer over the session | Lets an assistant run the try, look, adjust loop. No logic duplicated. |
| First styles | Voronoi stipple, continuous line, dashes, plus Sketch Lines | Point placement underpins the largest group of future styles. Sketch Lines is a contrasting, simple baseline. |
| Scope boundary | Machine-agnostic | Machine-specific output belongs downstream or in adapters, which keeps the core reusable. |
| Other products | Cite published sources; never borrow product names, UI or presets; no comparisons | Avoids trademark use, implied affiliation and disparagement, and keeps a later paid tier from looking hypocritical. |
