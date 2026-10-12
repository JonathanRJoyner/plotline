# plotline spec 2: Rust core, native and browser targets

Date: 2026-10-11. Status: design written from the owner's decision to move the engine to Rust, awaiting owner review of this document.

This spec replaces the Python engine from [spec 1](2026-10-11-core-engine-design.md) with a Rust engine that compiles two ways: a native Python extension for the library, CLI and MCP server, and a WebAssembly module for the GUI. It changes the language of the engine, not its design. Every type, stage, cache rule and format from spec 1 carries over unless a section below says otherwise. It sits between milestones 2 and 3 of [the brief](../../BRIEF.md): the Python engine is complete and tested, the algorithm milestones have not started, so this is the cheapest moment to switch.

## 1. Why

The owner wants a version with all the GUI features that needs no installation. A static web page can do that only if the engine runs inside the browser. Running the Python engine in the browser means shipping a Python runtime and losing numba, which puts the relaxation stage far outside its time targets. A Rust engine compiled to WebAssembly runs near native speed, and the same crate compiled natively gives Python users the same engine through bindings. One engine, two build targets, no server.

This reverses the decision in the brief that kept Rust out of 0.x. That decision assumed the only reason for Rust was speed, and speed was unmeasured. The reason now is deployment, which does not need measuring.

## 2. Scope

Delivered by this spec:

- A Cargo workspace with three crates: the engine, the Python extension, the WebAssembly module.
- The engine: types, settings and schema, the stage contract, styles, runner, cache, session state, run driver with cancellation, the four stages and the `grid` style from spec 1, SVG and JSON export, stats, preview PNG, presets, image decoding.
- The Python package rebuilt on the extension: the same `Session` and CLI surface as spec 1, with threads, files and callbacks in Python and nothing else.
- The WebAssembly module with a smoke test that runs `grid` under Node and checks its output against the golden files.
- CI for all three, a Rust licence check, and the deployment skeleton for the static page.
- Removal of the Python engine, scipy, Pillow, pydantic and the numba extra.

Out of scope, with the spec that owns each:

- Voronoi relaxation, stipple, continuous line, dashes, endpoint merging: spec 3.
- Sketch Lines: spec 4.
- The GUI itself, the Web Worker protocol, Canvas 2D or WebGL rendering, the hosted page's layout: spec 5. This spec fixes what the worker receives from the engine, not what the page does with it.
- MCP server: spec 6. It stays Python, built on the `mcp` SDK (MIT), and calls the Python `Session`.
- A standalone Rust CLI binary, a Python stage API, JSON-declared styles, a disk cache, toolchain adapters: open, see §17.

## 3. Layout

```
Cargo.toml                       workspace: crates/*
rust-toolchain.toml              stable channel, pinned
deny.toml                        cargo-deny licence allow-list
crates/
  core/                          plotline-core: the engine, no I/O, no threads
    src/
      lib.rs
      types.rs                   Pen, Canvas, ToneMap, Points, Paths, Layer, Drawing
      settings.rs                schema helpers, generic validation
      stage.rs                   Stage, StageRun, Step, Frame, RunContext, Kind, Artifact
      style.rs                   Style and artifact resolution
      runner.rs                  Run: drives stages, cache keys
      cache.rs                   StageCache
      session.rs                 Session state, start(), result, presets, stats
      registry.rs                built-in stages and styles
      stages/{tone,grid,order}.rs
      export/{svg,json}.rs
      preview.rs                 tiny-skia rasterizer
      presets.rs                 preset format and migrations
      image.rs                   decode bytes to Image, downsize
      stats.rs
      math.rs                    libm wrappers, Gaussian blur, resize
      synthetic.rs               generated test images (feature "test-util")
    tests/                       integration and golden tests
    golden/                      tiny JSON path outputs
  py/                            plotline-py: PyO3 extension, module plotline._core
  wasm/                          plotline-wasm: wasm-bindgen module
src/plotline/                    Python package
  __init__.py                    version, re-exports
  session/                       Session wrapper: thread, callbacks, files
  cli/                           click commands, unchanged surface
tests/                           Python tests
web/                             Vite, Preact, TypeScript GUI (spec 5); wasm output lands in web/pkg
scripts/check_licenses.py        Python dependencies; Rust dependencies use cargo-deny
pyproject.toml                   maturin build backend
.github/workflows/ci.yml
.github/workflows/pages.yml      deploys web/dist; disabled while the repo is private
```

The engine crate has no filesystem, network or thread code, so it compiles unchanged to `wasm32-unknown-unknown`. Hosts add what they have: Python adds threads and files, the browser adds a Web Worker and the DOM.

## 4. Dependencies and licences

Engine crate:

| Crate | Licence | Used for |
|---|---|---|
| serde, serde_json | MIT or Apache-2.0 | settings, presets, JSON export, cache keys |
| schemars | MIT | JSON Schema from settings structs |
| rand, rand_pcg | MIT or Apache-2.0 | PCG64 generator per stage seed |
| libm | MIT or Apache-2.0 | transcendental functions, identical on every target |
| image | MIT or Apache-2.0 | PNG and JPEG decoding, resize; only those two format features |
| tiny-skia | BSD-3-Clause | preview rasterizer and PNG encoding |
| blake3 | CC0-1.0 or Apache-2.0 | cache keys |
| thiserror | MIT or Apache-2.0 | error types |

Python extension: pyo3 and numpy (rust-numpy), MIT or Apache-2.0 and BSD-2-Clause. WebAssembly module: wasm-bindgen and js-sys, MIT or Apache-2.0. Build tools: maturin, wasm-pack, cargo-deny, all MIT or Apache-2.0.

Python runtime dependencies drop to numpy and click. scipy, Pillow and pydantic go, and so does the `fast` extra: numba has no job left.

`deny.toml` allows MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, CC0-1.0, Unicode-3.0 and the MIT or Apache dual forms, and fails CI on anything else. `scripts/check_licenses.py` keeps checking the Python side.

No kd-tree crate. The ordering stage uses a uniform grid bucket index written in the crate, about sixty lines, which is deterministic and fast enough for a hundred thousand endpoints in Rust.

## 5. Types

`crates/core/src/types.rs`. All plain structs, `Clone`, `Serialize`, `Deserialize`, with the same fields and meanings as spec 1 §4.

```rust
pub struct Pen { pub name: String, pub width_mm: f64, pub color: String }   // "#rrggbb"
pub enum Fit { Fit, Fill }
pub struct Canvas { pub width_mm: f64, pub height_mm: f64, pub margin_mm: f64, pub fit: Fit }
pub struct Image { pub width: u32, pub height: u32, pub rgb: Vec<f32> }     // row-major, 0..1
pub struct ToneMap { pub width: u32, pub height: u32, pub ink: Vec<f32>,
                     pub mm_per_px: f64, pub origin_mm: [f64; 2] }
pub struct Points(pub Vec<[f64; 2]>);
pub struct Paths { coords: Vec<f64>, offsets: Vec<u32> }
pub struct Layer { pub pen: Pen, pub paths: Paths }
pub struct Drawing { pub canvas: Canvas, pub layers: Vec<Layer>, pub tone: Option<Arc<ToneMap>> }
```

`Canvas` keeps `drawing_rect`, `image_rect` and `image_transform` with spec 1's semantics, including the fill-mode rounding rule recorded in the brief. `ToneMap` keeps `to_mm`, `to_px`, bilinear `sample` clamped at the edges, and `rect_mm`.

`Paths` is one flat coordinate buffer plus offsets: path `i` is `coords[2 * offsets[i] .. 2 * offsets[i + 1]]`, and `offsets` has one more entry than there are paths. A one-vertex path is a dot, as before. This layout crosses both FFI boundaries without copying per path: the Python side exposes it as one `(M, 2)` array plus an offsets array and slices views from it, and the browser receives it as one `Float64Array` plus a `Uint32Array` transferred to the main thread. `Paths` offers `len()`, `get(i)`, `push(&[[f64; 2]])`, `iter()` and `total_length()`.

`Drawing` is spec 1's `Result`, renamed because `Result` is Rust's error type.

## 6. Settings

Each stage declares one settings struct:

```rust
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GridSettings {
    /// Distance between grid cells.
    #[schemars(range(min = 0.2, max = 20.0), extend("x-unit" = "mm", "x-step" = 0.1))]
    pub spacing_mm: f64,
    /// Ink level at or above which a cell gets a dot.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub threshold: f64,
}
impl Default for GridSettings { ... }
```

The doc comment becomes the schema `description`, which is the help text every front end shows. Ranges, choices (Rust enums become `enum` in the schema) and the `x-unit`, `x-step`, `x-label` and `x-advanced` extensions carry what spec 1's `setting()` helper carried. Stages that use randomness declare `pub seed: u64` marked `x-advanced`.

Validation is generic and lives in `settings.rs`: a settings value arrives as `serde_json::Value`, is checked against the stage's own schema for `minimum`, `maximum` and `enum`, then deserialized with `deny_unknown_fields`. Both steps report the stage, the key and the problem in one error message. serde does not check ranges on its own, so the schema check is what keeps the GUI, CLI and MCP from passing values the stage never agreed to.

The engine accepts values of the declared JSON type only. The CLI's `--set stage.key=value` strings are coerced in Python before they reach the engine: `true` and `false` to booleans, integers and floats to numbers, everything else to strings. That is the one place lax coercion is useful and the one place it lives.

The JSON Schema is the only description of settings any front end sees, as before. `style.schema()` returns the same shape as spec 1 §7.

## 7. Stage contract

Rust has no generators, and a stage that runs to completion inside one call cannot be cancelled from a browser without shared memory. So stages are step functions. The host pulls one step at a time and decides when to stop.

```rust
pub enum Kind { Image, Canvas, Pens, Tone, Points, Paths, Layers }

pub enum Artifact { Image(Arc<Image>), Canvas(Arc<Canvas>), Pens(Arc<Vec<Pen>>),
                    Tone(Arc<ToneMap>), Points(Arc<Points>), Paths(Arc<Paths>),
                    Layers(Arc<Vec<Layer>>) }

pub struct Frame { pub stage: String, pub progress: f32, pub label: String,
                   pub points: Option<Points>, pub paths: Option<Paths> }

pub enum Step { Frame(Frame), Done(Artifact) }

pub struct Inputs { ... }            // typed getters: inputs.tone(), inputs.points(), ...
pub struct RunContext { ... }        // rng(seed) -> Pcg64, cancelled() -> bool

pub trait Stage: Send + Sync {
    fn name(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn version(&self) -> u32;
    fn requires(&self) -> &'static [Kind];
    fn provides(&self) -> Kind;
    fn schema(&self) -> schemars::Schema;
    fn defaults(&self) -> serde_json::Value;
    fn start(&self, inputs: Inputs, settings: &serde_json::Value, ctx: RunContext)
        -> Result<Box<dyn StageRun>, StageError>;
}

pub trait StageRun {
    fn step(&mut self) -> Result<Step, StageError>;
}
```

`start` validates the settings (§6) and returns a state machine. Each `step` does one chunk of work and returns a frame, or the finished artifact. A chunk is whatever the algorithm's natural unit is: one grid row, one relaxation pass, a few hundred improving tour moves, a batch of sketch strokes. Chunks should take well under a second so cancellation feels immediate. A stage with an unavoidably long chunk checks `ctx.cancelled()` inside it and returns `StageError::Cancelled`.

A stage returns at least one frame before `Done`. The final frame of a stage has `progress == 1.0`. The runner fills in `Frame.stage`.

Artifacts are shared through `Arc` and are immutable by construction, which replaces spec 1's read-only flag on cached arrays.

## 8. Styles, runner and cache

`Style` keeps spec 1 §7: unique stage names, every requirement resolved to a root kind or the nearest earlier provider, last stage provides paths or layers, validated on construction, resolution stored.

The runner is a struct the host drives:

```rust
pub struct Run { ... }
impl Run {
    pub fn next_frame(&mut self) -> Option<Frame>;   // None when finished, cancelled or failed
    pub fn cancel(&self);                            // sets an AtomicBool, safe from any thread
    pub fn state(&self) -> RunState;                 // Running, Done, Cancelled, Error
    pub fn error(&self) -> Option<&StageError>;
}
```

`next_frame` works through the stages in order, exactly as spec 1 §8: compute the key, serve from cache with one `cached` frame, otherwise `start` the stage and pull `step` until `Done`, then cache the artifact. It checks the cancel flag before every step. On the last stage's `Done` it builds the `Drawing` (layers from the stage if it provided them, otherwise one layer with the first pen), commits it to the session's shared state together with the run state, and returns `None`. A cancelled or failed run commits only its state; the previous drawing stays.

Cache keys are blake3 hex digests. Root keys hash the image bytes, shape and dtype, or the canonical JSON of the canvas and pens. Stage keys hash the canonical JSON of `{"stage": name, "version": version, "settings": value, "inputs": {kind: key}}`. Canonical JSON is `serde_json` with sorted keys and no whitespace, which is what `serde_json::to_string` on a `BTreeMap` gives; floats print in shortest round-trip form, so keys are stable across runs and platforms.

`StageCache` keeps spec 1 §8: in memory, bounded by estimated bytes, least recently used, oversize values not stored, `hits`, `misses`, `bytes`, `clear()`. Size is the sum of each artifact's buffers.

## 9. Session and the host model

`Session` lives in the engine so both hosts share it. It holds the state spec 1 §11 lists: image, canvas, pens, style, per-stage settings, speeds, cache, last drawing, run state. Its methods are synchronous and plain:

```rust
impl Session {
    pub fn new() -> Session;
    pub fn set_image_bytes(&mut self, bytes: &[u8]) -> Result<(), ImageError>;   // decode, downsize
    pub fn set_image(&mut self, image: Image);
    pub fn set_canvas(&mut self, canvas: Canvas);
    pub fn set_pens(&mut self, pens: Vec<Pen>);
    pub fn set_style(&mut self, name: &str) -> Result<(), StyleError>;           // resets settings, keeps cache
    pub fn update(&mut self, stage: &str, values: serde_json::Value) -> Result<(), SettingsError>;
    pub fn settings(&self) -> serde_json::Value;
    pub fn start(&self) -> Result<Run, StartError>;                              // cancels the previous run
    pub fn drawing(&self) -> Option<Arc<Drawing>>;
    pub fn run_state(&self) -> RunState;
    pub fn svg(&self) -> Option<String>;
    pub fn json(&self) -> Option<serde_json::Value>;
    pub fn stats(&self) -> Option<Stats>;
    pub fn preview_png(&self, width_px: u32, travel: bool, source: bool) -> Option<Vec<u8>>;
    pub fn preset(&self) -> serde_json::Value;
    pub fn load_preset(&mut self, value: serde_json::Value) -> Result<Vec<String>, PresetError>; // warnings
    pub fn cache_stats(&self) -> CacheStats;
}
```

`start` snapshots the style, settings, root artifacts and a handle to the cache and the shared result slot, so a `Run` is independent of later edits to the session. Shared parts are behind `Arc<Mutex<_>>`, so a native host can drive `next_frame` on one thread while the main thread reads `drawing()` or calls `update`.

Each host supplies the loop:

- **Python.** `plotline.Session` wraps the engine session. `run()` starts a thread that loops over `run.next_frame()` with the interpreter lock released during the call, delivers frames to subscribers throttled to `max_fps` with final frames always delivered, and sets `state`. `run_blocking()` loops in the calling thread. `cancel()` calls `run.cancel()`. `load_image(path)` reads the file and calls `set_image_bytes`. `export_svg`, `export_json`, `save_preset` and `load_preset(path)` do the file half. This is the whole of `src/plotline/session`, and its public surface is spec 1 §11 unchanged, so the spec 1 tests for the session, presets and CLI keep passing.
- **Browser.** The session lives in a Web Worker. The worker loop calls `run.nextFrame()`, posts each frame to the main thread with its typed arrays transferred, and awaits a zero-delay timer before the next call. That await lets a `cancel` message arrive between steps and call `run.cancel()`, so cancellation needs no shared memory and no cross-origin isolation headers, which a static host cannot set. The message protocol beyond that is spec 5's.

Frame throttling stays in the hosts because the browser and Python have different clocks and delivery costs.

## 10. Stages, export, stats, preview, presets, images

The four stages, the `grid` style, SVG and JSON export, stats and presets follow spec 1 §9 to §11 exactly, including every default, range, formula, format string and the three-decimal rounding. Differences:

- **Tone stage.** Resizing reproduces the Python engine: average `k` by `k` blocks when the source is at least twice the target, then sample bilinearly with target pixel centres mapped to `(j + 0.5) * source / target - 0.5`. The Gaussian blur is a separable kernel in `math.rs`, truncated at three sigma. Both were scipy calls before, and keeping the arithmetic lets the tone tests carry over with their pinned values.
- **Order stage.** The nearest-endpoint search uses the in-crate uniform grid index instead of a kd-tree, with exact distance ties broken by the lowest endpoint index. Same greedy rule, same start corner, same reversal rule, same frame cadence. The kd-tree broke ties by its internal layout, so the Python and Rust orders can differ only where two candidates are at exactly equal distance.
- **Preview.** tiny-skia draws anti-aliased strokes directly, so the 4x supersample and box filter go. Everything else in the picture stays: white page, light grey drawing rect, tone at 30 percent when `source` is on, each layer at true pen width with round caps, dots as filled circles of the pen width, red travel lines when `travel` is on.
- **Images.** `image.rs` decodes PNG and JPEG bytes, converts to RGB float, scales 16-bit greyscale by 65535, and downsizes so the long edge is at most 4000 px. Any other format fails with a clear message naming the two supported formats. Float images cannot occur because no float-capable format is compiled in.
- **Presets.** Same JSON format, version 1, same migration chain and warning rules, parsed and written by serde.

The synthetic images (gradient, checkerboard, radial, seeded noise, flat grey) are generated in `synthetic.rs` and exposed through both bindings, so Python and Node tests use the engine's own generators and `tests/synthetic.py` is deleted. A hidden style named `_slow`, whose stages take `frames`, `delay_ms` and `fail_after` settings, is registered for cancellation, throttling and error tests from every host; `list_styles` hides names that start with an underscore. Both are always compiled: they are small, and the hosts' tests run against release builds.

## 11. Determinism across targets

The brief requires the same input and settings to give the same output. With two compile targets the rule becomes: bit-identical output on native and WebAssembly, on every supported OS. Four choices make that hold:

- All randomness comes from `rand_pcg::Pcg64` seeded from the stage's `seed` setting through `ctx.rng(seed)`. Random floats are produced only through `rand`'s standard `f64` distribution, which is integer arithmetic underneath.
- Transcendental functions (`sin`, `cos`, `atan2`, `exp`, `powf`, `ln`) are called through `libm`, a pure Rust port whose results do not depend on the platform's C library. `sqrt` is correctly rounded everywhere and may use `f64::sqrt`. `mul_add` is never used, because a fused multiply-add rounds differently from a multiply then an add.
- Arithmetic is `f64` for coordinates and `f32` for the tone map, as in spec 1, with no target-specific SIMD paths.
- Nothing iterates a `HashMap` in an order that affects output; ordered collections are used where order matters.

A unit test scans the crate's sources and fails on any call of the platform float methods (`sin`, `cos`, `tan`, `atan2`, `exp`, `ln`, `log`, `powf`, `mul_add`) outside `math.rs`, so the rule is checked, not remembered.

The golden tests prove it: the engine's Rust tests and the WebAssembly smoke test under Node both produce the `grid` output for the gradient and the checkerboard and compare it against the same golden JSON files within 0.001 mm.

## 12. Python package

`pyproject.toml` switches the build backend to maturin with `python-source = "src"`, `module-name = "plotline._core"` and `manifest-path = "crates/py/Cargo.toml"`. `uv sync` builds the extension, so the developer loop is unchanged apart from needing a Rust toolchain, which `rustup` installs in one step. Wheels are `abi3` for Python 3.11 and up, one per OS and architecture.

The extension exposes `_core.Session`, `_core.Run`, `_core.list_styles()`, `_core.describe(name)` and `_core.synthetic`, and converts: `Paths` to an `(M, 2)` float64 array plus a `uint32` offsets array through rust-numpy, `Points` to an `(N, 2)` array, images in as `(H, W, 3)` float32 arrays, settings and presets as `dict`, errors as `ValueError` for validation and `RuntimeError` for everything else, with the engine's message.

`plotline.Session` keeps spec 1's attribute and method names. `Layer.paths` is a list of `(N, 2)` views sliced from the flat buffer, so user code and the exporters' tests see the shape they saw before. `plotline.core` is deleted; the public import surface is `plotline.Session`, `plotline.Canvas`, `plotline.Pen`, `plotline.list_styles` and `plotline.describe`.

The CLI keeps spec 1 §12 as is. `plotline gui` now serves `web/dist` from inside the package on a localhost port with the standard library's HTTP server and opens the browser; no web framework, no WebSocket. If `web/dist/index.html` is missing from the installed package, the command fails with a message saying the package was built without the GUI bundle. Until spec 5 ships a bundle, the command fails with that message, which is the correct behaviour. `plotline mcp` keeps its stub until spec 6.

## 13. WebAssembly module

`crates/wasm` wraps the same session and run types with `wasm-bindgen`: `Session`, `Run`, `listStyles()`, `describe(name)` and `synthetic`. Frames come out as a JS object holding `stage`, `progress`, `label`, and `points` or `paths` as typed arrays ready to transfer. Settings, presets, stats and schemas cross as JSON strings, parsed on the JS side. Errors become JS `Error`s with the engine's message.

`wasm-pack build --target web --out-dir ../../web/pkg` produces the module; `web/pkg` is ignored by git. The build profile is `opt-level = 3`, `lto = true`, `codegen-units = 1`. The target for the module is under 1.5 MB before compression, checked by the CI job and printed in its log so growth is visible.

The smoke test is `wasm-pack test --node` in the wasm crate: it runs `grid` on the synthetic gradient and checkerboard and compares to the golden files. It proves the engine builds for and runs on the browser target. The page itself, the worker and the GUI are spec 5.

## 14. Build, CI and deployment

CI stays Linux only while the repo is private, with three jobs:

1. **rust:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check licenses`.
2. **python:** `uv sync --locked`, `ruff check`, `ruff format --check`, `mypy src`, `pytest -q`, `python scripts/check_licenses.py`. mypy's strict overrides move from `plotline.core` to `plotline.session`.
3. **wasm:** `wasm-pack test --node` in `crates/wasm`, then `wasm-pack build` and the size check.

Windows, macOS and Python 3.12 return when the repo goes public. Wheels for every OS are built only by the release workflow at milestone 8 with maturin-action, after `npm run build` in `web/`, so the wheel carries `web/dist`. The wheel build fails if `web/dist/index.html` does not exist. That settles the brief's old open question about shipping the GUI bundle.

`.github/workflows/pages.yml` builds the wasm module and the Vite bundle on pushes to `main` and deploys `web/dist` with the standard GitHub Pages actions. Its job is gated on the repository being public, so it is inert until then. The hosted page and `plotline gui` serve the same files.

The engine's performance targets stay the brief's: 10k points on a 1000 px image over 50 relaxation passes in under 5 s natively. The browser target gets under 10 s. Both are measured by spec 3's benchmark, not here.

## 15. Testing

Rust, in `crates/core`:

- Unit tests per module, ported from spec 1 §13: canvas rects and transforms, tone map sampling, each tone adjustment, settings validation and schema extensions, style validation errors, cache get, put, eviction and oversize values, SVG structure, JSON round trip, preset migration and warnings, stats, preview pixel checks at a few known positions.
- A spy stage counts its runs: changing an `order` setting reruns only `order`, changing `tone` reruns everything, rerunning unchanged settings runs nothing.
- Cancellation: a stage that steps 100 times is cancelled after the third frame from another thread. The run stops, the state is `Cancelled`, nothing from that stage is cached, earlier stages remain cached.
- Determinism: every registered style runs twice on a 64 by 48 synthetic image and compares bitwise.
- Golden: `grid` on the gradient and the checkerboard at 64 by 48, JSON in `crates/core/golden/`, tolerance 0.001 mm, regenerated with the environment variable `PLOTLINE_UPDATE_GOLDEN=1`.

Python, in `tests/`:

- The spec 1 tests for the session wrapper, threads, presets, stats, CLI and exports keep running against the new engine with minimal edits: imports move from `plotline.core` to `plotline`, synthetic images come from `_core.synthetic`. Tests of Python-internal mechanics that no longer exist, such as byte estimation and the Python spy stage, are deleted, since their Rust equivalents cover them.
- A determinism test and a count-only golden test remain in Python, as end-to-end checks through the bindings.

Node: the wasm smoke test in §13.

Acceptance for this spec is the whole set green in CI, the Python engine deleted, and the Rust `grid` output for the synthetic gradient holding the same set of dots as the Python engine's, within 0.001 mm and with the same count; the order may differ only at exact distance ties.

## 16. Porting order

The implementation plan builds the Rust engine beside the Python one and switches over at the end, so the repository stays green throughout:

1. Workspace scaffold, `deny.toml`, toolchain file, CI rust job.
2. Types, settings, stage contract, style, cache, runner, with the spy stage tests.
3. Tone, grid placer, dot renderer, order stage, the `grid` style, synthetic images, golden files.
4. Export, stats, preview, presets, image decoding.
5. Session and `Run`.
6. The Python extension, the Python session wrapper and CLI rewired, Python tests passing.
7. The wasm crate and smoke test, the wasm CI job, the size check.
8. Delete the Python engine and its dependencies, add the Pages workflow, update the brief and README.

## 17. Decisions recorded in the brief

This spec adds these rows to the brief's decision log and marks the rows they supersede: the Rust engine and its reason; Python as the host for the library, CLI and MCP through bindings; the GUI running the WebAssembly engine in the browser both locally and hosted, with `plotline gui` as a static file server; stages as step functions pulled by the host; cross-target determinism through libm and PCG64; flat path buffers; serde and schemars in place of pydantic; tiny-skia in place of Pillow; the `image` crate for decoding; blake3 keys; the in-crate spatial index; no web framework or WebSocket; the GUI bundle in the wheel and the Pages workflow; the Pyodide route rejected; third-party styles as Rust code in 0.x with the Python entry-point group dropped.

New open questions for the brief: a standalone Rust CLI binary for users without Python; a Python stage API through callbacks, for stages that would run natively but not in the browser; JSON-declared styles that compose existing stages without code.
