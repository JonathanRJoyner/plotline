# Learned line models Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `learned` style that runs a downloaded line-drawing model in tiles and draws its output with the existing ink stage, in the browser and natively, plus a catalogue of hosted models with a licence screen before each download.

**Architecture:** The core gains a `models` module: the `ModelRuntime` trait, a `ModelStore` the session owns and runs reach through `RunContext`, the catalogue, and a stand-in runtime for tests. A new `model` stage places the image as the tone stage does, runs the model one 512-pixel tile per step and returns a `ToneMap`. A new crate, `crates/models`, implements the trait with tract natively (linked into the Python extension) and, built with SIMD as its own WebAssembly module, for the browser, where the engine calls it through a JavaScript bridge in the worker. Front ends download, show the licence and keep files; the core checks every file.

**Tech Stack:** Rust (tract-onnx 0.23, sha2, serde, schemars, wasm-bindgen), pyo3, Python (click, urllib, hashlib), TypeScript, Preact, vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-16-learned-line-models-design.md`

## Global Constraints

- Runtime dependencies must be MIT, BSD or Apache-2.0: `tract-onnx` (MIT OR Apache-2.0) only in `crates/models`; `sha2` (MIT OR Apache-2.0) in `crates/core`. `uv run python scripts/check_licenses.py` must pass after any dependency change.
- No model weights in this repository. The only ONNX file committed is the code-built test model `web/e2e/fixtures/tiny-conv.onnx` (Task 4).
- Algorithm code (resize, tiling, mirroring, scaling, polarity, placement, hash checks) lives in `crates/core`. The CLI, GUI and MCP call the session layer only.
- Core code makes no float library call outside `math.rs` (its source scan forbids `sin`, `cos`, `tan`, `atan2`, `exp`, `ln`, `log`, `powf`, `mul_add`, `sin_cos`, `atan` elsewhere).
- Tile side `TILE = 512`, margins included; the kept centre is `TILE - 2 * overlap`. Default overlap for a file of your own: 64. Tile sides are a multiple of 8.
- `model` stage settings: `model` (string, default `""`), `resolution` (integer, default 1024, 256 to 2048), `invert` (bool, default false).
- Model files: at most 200 MB (209,715,200 bytes), ONNX only. A file of your own gets id `file-` plus the first 12 lowercase hex digits of its SHA-256.
- Every model the core runs takes 1 × 3 × TILE × TILE floats, RGB in 0 to 1, and returns 1 × 1 × TILE × TILE floats in 0 to 1 where 1 is a line. Hosted models have their own scaling, channel order and polarity folded into the graph by the conversion script (Task 11); `invert` flips a file of your own that runs the other way.
- Exact user-facing copy:
  - missing model: `model {id} is not loaded; get it from Styles, then Get models, or run \`plotline models get {id}\``
  - empty model: `choose a model; get one from Styles, then Get models, or run \`plotline models list\``
  - licence screen line: `Provided as is, without warranty of any kind.`
  - Load your own line: `You're responsible for the licence of any model you load.`
- The main engine module stays at or under 2,300,000 bytes (it is 1,984,967 today, so 315 KB of headroom). `plotline_models.wasm` is built with `-C target-feature=+simd128`, never relaxed SIMD, size-tuned (`opt-level = "s"` for its crates except `tract-linalg` at 3, `wasm-opt -Os`), and stays at or under 9,000,000 bytes (8,318,424 in the spike).
- `learned` stays out of the gallery and its stats snapshot; `tests/snapshots/gallery-stats.json` must not change.
- A user-visible GUI feature adds its tour step (`STEPS` in `web/src/tour.tsx`) and its flow in `web/e2e/gui.spec.ts`. A contract adds its law to `crates/core/tests/laws.rs`.
- Cite in docstrings: Chan, Durand and Isola, CVPR 2022 (contour); Soria, Li, Rouhani and Sappa, Proc. ICCV Workshops 2023, pp. 1364-1373 (TEED); the overlap-tile strategy with mirrored borders from O. Ronneberger, P. Fischer and T. Brox, "U-Net: convolutional networks for biomedical image segmentation", MICCAI 2015, pp. 234-241.
- No product names in identifiers, settings or copy. LF line ends. `cargo fmt`, clippy `-D warnings`, ruff, mypy and `tsc` clean. Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **Tiny and extreme images.** A 1 × 1 image, or 4000 × 1, must give a map, not a panic: mirroring has to repeat when the pad is wider than the image. Pinned in Task 2.
2. **A cancelled model run.** Stop between tiles, then Run again, must recompute, never reuse a half-written map from the cache. Pinned in Task 2.
3. **A damaged download.** A truncated file or a wrong hash for a catalogue id must be refused with a plain error, and the front end must not keep it. Pinned in Tasks 1, 5 and 9.
4. **A withdrawn model.** A kept file whose id a newer release no longer lists must still load, as a file of your own. Pinned in Tasks 5 and 9.
5. **Blocked browser storage.** With Cache Storage or `localStorage` throwing, or full, a download must still work for the page's lifetime, and the page must not break. Pinned in Task 9.

---

## Part A: the engine

### Task 1: Model store, runtime trait, catalogue and stand-in

**Files:**
- Create: `crates/core/src/models/mod.rs`, `crates/core/src/models/catalogue.rs`, `crates/core/src/models/catalogue.json`, `crates/core/src/models/standin.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod models;`), `crates/core/Cargo.toml` (add `sha2 = "0.10"`), `crates/core/src/session.rs`

**Interfaces:**
- Produces:
  - `pub const TILE: usize = 512;`
  - `pub trait ModelRuntime: Send + Sync { fn load(&self, id: &str, bytes: &[u8], side: usize) -> Result<(), ModelError>; fn infer(&self, id: &str, rgb: &[f32]) -> Result<Vec<f32>, ModelError>; fn unload(&self, id: &str); }`. `load` refuses a model that does not take `1 × 3 × side × side` and return `1 × 1 × side × side`; `rgb` is channel-major, `3 * side * side` floats, and the result is `side * side` floats.
  - `#[derive(Debug, thiserror::Error)] pub enum ModelError { TooLarge(usize), Hash { id: String }, Unknown(String), NoRuntime, Invalid(String), Run(String) }`, each with a plain message.
  - `pub struct ModelInfo { pub id: String, pub label: String, pub overlap: usize, pub entry: Option<CatalogueEntry> }`, `Serialize`.
  - `pub struct ModelStore` with `pub fn new(runtime: Option<Arc<dyn ModelRuntime>>) -> ModelStore`, `pub fn get(&self, id: &str) -> Option<ModelInfo>`, `pub fn runtime(&self) -> Option<Arc<dyn ModelRuntime>>`, `pub fn list(&self) -> Vec<ModelInfo>`. `Default` is an empty store with no runtime.
  - `pub struct CatalogueEntry { pub id, label, description, url, sha256, licence, attribution, source, paper, training: String, pub bytes: u64, pub overlap: usize }`, `Serialize + Deserialize`; `pub fn catalogue() -> &'static [CatalogueEntry]` parses `catalogue.json` (embedded with `include_str!`) once.
  - `pub fn file_id(bytes: &[u8]) -> String`.
  - On `Session`: `pub fn set_model_runtime(&mut self, runtime: Arc<dyn ModelRuntime>)`, `pub fn add_model(&mut self, id: Option<&str>, bytes: &[u8]) -> Result<ModelInfo, ModelError>`, `pub fn remove_model(&mut self, id: &str)`, `pub fn models(&self) -> Vec<ModelInfo>`, `pub fn model_store(&self) -> Arc<ModelStore>`.
  - `pub struct StandIn;` implementing `ModelRuntime`: it loads `STANDIN_BYTES` and `STANDIN_HALF_BYTES` and refuses any other bytes with `Invalid`. `infer` returns the 3 × 3 Sobel gradient magnitude of the luminance, clamped to 0 to 1, and half of that for a model loaded from `STANDIN_HALF_BYTES`. `pub const STANDIN_BYTES: &[u8] = b"STANDIN";` and `pub const STANDIN_HALF_BYTES: &[u8] = b"STANDIN-HALF";`

- [ ] **Step 1: Write the failing tests in `crates/core/src/models/mod.rs`**

```rust
#[test] fn add_model_names_a_file_by_its_hash() // id == "file-" + 12 hex of sha256(STANDIN_BYTES); entry is None; overlap == 64
#[test] fn add_model_refuses_files_over_200_mb() // a 209_715_201-byte Vec gives ModelError::TooLarge, nothing stored
#[test] fn add_model_refuses_a_catalogue_id_with_the_wrong_hash() // with a test entry, other bytes give ModelError::Hash, nothing stored
#[test] fn add_model_refuses_an_unknown_id() // Some("nope-1") gives ModelError::Unknown
#[test] fn add_model_without_a_runtime_says_so() // Session::new without set_model_runtime gives ModelError::NoRuntime
#[test] fn remove_model_unloads_it() // after remove, models() is empty and StandIn saw unload
#[test] fn catalogue_entries_are_complete() // for every entry: SPDX id in {MIT, BSD-2-Clause, BSD-3-Clause, Apache-2.0, CC0-1.0, CC-BY-3.0, CC-BY-4.0}; sha256 is 64 lowercase hex; url contains a 40-hex commit; training, attribution non-empty; ids unique and end in "-<n>"
```

Catalogue-hash tests use a `#[cfg(test)]` hook that lets a test supply its own entries in place of `catalogue()`; `catalogue.json` starts as `[]` and gets its entries in Task 11.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p plotline-core models::`
Expected: compile errors (module missing).

- [ ] **Step 3: Implement the module and the session methods**

`add_model` checks in this order: size, then (for `Some(id)`) the catalogue entry exists and `bytes.len() == entry.bytes` and the SHA-256 matches, then `runtime.load(id, bytes, TILE)`, then stores the `ModelInfo`. Adding an id already present replaces it. The label of a file of your own is its id.

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo test -p plotline-core models::`
Expected: PASS.

- [ ] **Step 5: Run the licence check and commit**

Run: `uv run python scripts/check_licenses.py` (expected: exit 0).

```bash
git add crates/core Cargo.lock
git commit -m "Add the model store, runtime trait, catalogue and stand-in runtime"
```

### Task 2: The `model` stage

**Files:**
- Create: `crates/core/src/stages/model.rs`
- Modify: `crates/core/src/stages/mod.rs`, `crates/core/src/stage.rs` (`RunContext`), `crates/core/src/runner.rs` (`Run`), `crates/core/src/session.rs` (pass the store to every `Run`, trial runs included)

**Interfaces:**
- Consumes: Task 1's `ModelStore`, `TILE`, `ModelInfo::overlap`; `stages::tone::{crop_to_aspect, resize}` (both `pub(crate)`), `Canvas::image_transform`, `Canvas::drawing_rect`.
- Produces:
  - `RunContext::with_models(self, models: Arc<ModelStore>) -> RunContext` and `RunContext::models(&self) -> &ModelStore`; `RunContext::new` keeps an empty store.
  - `Run::with_models(self, models: Arc<ModelStore>) -> Run`; the runner builds each stage's context with it. Existing `Run::new` callers are unchanged.
  - `pub struct ModelStage;` with `name() == "model"`, `label() == "Model"`, `requires() == &[Kind::Image, Kind::Canvas]`, `provides() == Kind::Tone`, settings `ModelSettings { model: String, resolution: u32, invert: bool }` with the help text one line each.
  - `pub(crate) fn tiles(w: usize, h: usize, overlap: usize) -> Vec<[usize; 2]>`: the top-left corners, in row-major order, of the kept centres that cover `w × h`.

- [ ] **Step 1: Write the failing tests in `crates/core/src/stages/model.rs`**

```rust
#[test] fn tiles_cover_the_map_once() // for (w,h,overlap) in [(1,1,64),(384,384,64),(385,384,64),(1024,768,32),(4000,1,64)]: every pixel lies in exactly one kept centre
#[test] fn the_map_lines_up_with_the_tone_map() // learned and tone at resolution 300 on gradient(400,300) give equal width, height, mm_per_px and origin_mm, for Fit::Fit and Fit::Fill
#[test] fn an_empty_model_setting_is_a_settings_error() // message equals the empty-model copy
#[test] fn a_missing_model_is_a_settings_error() // model "contour-1" with an empty store; message equals the missing-model copy with id contour-1
#[test] fn one_step_runs_one_tile() // 1024 x 768 at resolution 1024, overlap 64: Frame count == tiles(1024,768,64).len(), progress rises to 1.0
#[test] fn invert_flips_the_map() // ink(invert) == 1 - ink(no invert) within 1e-6
#[test] fn seams_match_a_whole_run() // StandIn on a 600 x 400 still life: tiled map within 1e-6 of the same Sobel over the whole resized image (Sobel needs 1 pixel of context, overlap 64 gives plenty)
#[test] fn tiny_and_extreme_images_draw() // 1 x 1 and 4000 x 1 images give a map with finite ink in 0..=1
#[test] fn a_cancelled_run_caches_nothing() // set cancel after the first tile, then run again on the same session and cache with cancel clear: the map equals an uncancelled run's
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p plotline-core stages::model`
Expected: compile errors (module missing).

- [ ] **Step 3: Implement the stage**

`start` validates settings, resolves the model (empty and missing give `StageError::Settings` with the copy above) and the runtime (none gives `StageError::Failed("this build cannot run models")`), splits `image.rgb` into three planes, crops each with `crop_to_aspect` when `canvas.fit == Fit::Fill`, resizes each with `resize` to `resolution`, and computes `tiles`. Each `step` cuts one `TILE × TILE` window around the next centre, filling outside pixels by mirroring (reflect about the edge pixel, repeating while out of range), calls `runtime.infer`, writes the kept centre into the map, applies `invert`, clamps to 0 to 1, checks `ctx.cancelled()`, and reports a `Frame` with `label: "model"` and progress `done / total`. The final step returns `Artifact::Tone` with `mm_per_px` and `origin_mm` from `canvas.image_transform(w, h)`. The module docstring cites the overlap-tile strategy.

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo test -p plotline-core stages::model`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "Add the model stage, which runs a model over the image in tiles"
```

### Task 3: The `learned` style

**Files:**
- Modify: `crates/core/src/registry.rs`, `crates/core/src/test_util.rs`, `crates/core/tests/laws.rs`, `crates/core/tests/settings_matter.rs`, `crates/core/tests/empty_lanes.rs`, `crates/core/examples/gallery.rs`, `crates/core/examples/readme_images.rs`

**Interfaces:**
- Consumes: Task 2's `ModelStage`, `Run::with_models`; Task 1's `StandIn`, `STANDIN_BYTES`, `file_id`.
- Produces: `pub fn learned_style() -> Style`, name `learned`, label `Learned lines`, stages `[ModelStage, SplitStage, InkStage, OrderStage]`, `.with_defaults("ink", json!({"mode": "centreline"}))`, listed after `ink` in `builtins()`. Description: "Draws the lines a trained model finds in the image: outlines and creases, as a person would draw them. Needs a model from Get models." `test_util::standin_store() -> Arc<ModelStore>` with both stand-in models loaded, and `test_util::standin_id() -> String` and `test_util::standin_half_id() -> String`.

- [ ] **Step 1: Write the failing test in `crates/core/src/registry.rs`**

```rust
#[test] fn learned_draws_centrelines_with_the_standin() // learned on scene() with standin_store(), model = standin_id(): drawing has > 0 paths; settings ink.mode == "centreline"
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p plotline-core registry::tests::learned_draws_centrelines_with_the_standin`
Expected: FAIL, `learned_style` not found.

- [ ] **Step 3: Register the style and update every suite that iterates `list_styles()`**

The laws, `settings_matter` and `empty_lanes` suites run `learned` with `standin_store()` and `model` set to `standin_id()`. `settings_matter`'s `another` panics on a string setting; teach it that `model.model` varies to the other stand-in id (`standin_half_id()` from `standin_id()`). `gallery` and `readme_images` skip `learned` by name, with a one-line comment saying the models are not in the repository.

- [ ] **Step 4: Run the core suites and the gallery snapshot**

Run: `cargo test --locked -p plotline-core` (expected: PASS) and `cargo run --release -p plotline-core --example gallery -- target/gallery --snapshot tests/snapshots/gallery-stats.json` followed by `git diff --exit-code tests/snapshots/gallery-stats.json` (expected: no change).

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "Add the learned style: a model's lines drawn as centrelines"
```

### Task 4: `crates/models`, the tract runtime

**Files:**
- Create: `crates/models/Cargo.toml`, `crates/models/src/lib.rs`, `crates/models/src/tiny.rs`, `crates/models/examples/tiny_onnx.rs`, `web/e2e/fixtures/tiny-conv.onnx`
- Modify: `.github/workflows/ci.yml` (add `cargo test --locked -p plotline-models` after the core tests, in the same job)

**Interfaces:**
- Consumes: Task 1's `ModelRuntime`, `ModelError`, `TILE`.
- Produces: `pub fn tiny_conv_onnx_with(in_channels: usize, out_channels: usize) -> Vec<u8>` and `pub fn tiny_conv_onnx() -> Vec<u8>` (`tiny_conv_onnx_with(3, 1)`). `pub struct TractRuntime` (`Default`), implementing `ModelRuntime` with `tract-onnx = "0.23"`, its default `transformers` feature off: `load` fixes the input fact to `f32 [1, 3, side, side]`, optimises, checks the output is `[1, 1, side, side]`, and keeps the runnable plan in a `Mutex<HashMap<String, _>>`. `tiny_conv_onnx_with` builds, with `tract_onnx::pb` types and prost encoding, a model whose single 3 × 3 convolution (padding 1) maps `in_channels` to `out_channels` with the fixed weights `w[o][c][y][x] = (o + c + 1) as f32 * 0.01 * ((y * 3 + x) as f32 - 4.0)` and bias 0.5, followed by a Sigmoid. The crate has `crate-type = ["cdylib", "rlib"]` and a `web` feature that Task 7 fills.

- [ ] **Step 1: Write the failing tests in `crates/models/src/lib.rs`**

```rust
#[test] fn the_tiny_model_matches_a_direct_convolution() // TILE-sized random rgb (seeded): infer within 1e-5 of the same conv + sigmoid computed in the test
#[test] fn a_one_channel_input_is_refused() // tiny_conv_onnx_with(1, 1): load gives ModelError::Invalid
#[test] fn a_three_channel_output_is_refused() // tiny_conv_onnx_with(3, 3): Invalid
#[test] fn a_file_that_is_not_onnx_is_refused() // b"not a model": Invalid
#[test] fn a_truncated_file_is_refused() // tiny_conv_onnx()[..len/2]: Invalid
#[test] fn the_e2e_fixture_is_the_tiny_model() // std::fs::read("../../web/e2e/fixtures/tiny-conv.onnx") == tiny_conv_onnx()
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p plotline-models`
Expected: compile errors (crate missing).

- [ ] **Step 3: Implement the crate, then write the fixture**

Run: `cargo run -p plotline-models --example tiny_onnx -- web/e2e/fixtures/tiny-conv.onnx`

- [ ] **Step 4: Run the tests and the licence check**

Run: `cargo test --locked -p plotline-models` (expected: PASS) and `uv run python scripts/check_licenses.py` (expected: exit 0; tract's tree is MIT or Apache-2.0).

- [ ] **Step 5: Commit**

```bash
git add crates/models web/e2e/fixtures/tiny-conv.onnx .github/workflows/ci.yml Cargo.lock
git commit -m "Add the tract model runtime and a tiny ONNX model built in code"
```

### Task 5: Python session and model files

**Files:**
- Modify: `crates/py/Cargo.toml` (depend on `plotline-models`), `crates/py/src/session.rs`, `src/plotline/_core.pyi`, `src/plotline/session.py`
- Create: `src/plotline/models.py`, `tests/test_models.py`

**Interfaces:**
- Consumes: Task 1's `Session` model methods, Task 4's `TractRuntime`.
- Produces:
  - `_core.Session.add_model(id: str | None, data: bytes) -> dict`, `remove_model(id: str) -> None`, `models() -> list[dict]`, `catalogue() -> list[dict]` (module-level function). The native session sets `TractRuntime` in `new`.
  - `plotline.models`: `cache_dir() -> Path` (`PLOTLINE_CACHE_DIR`, else `XDG_CACHE_HOME` or `~/.cache` on Linux, `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on Windows, then `plotline/models`); `kept() -> list[tuple[str, Path]]` (id from the file name `<id>.onnx`); `accepted(entry: dict) -> bool` and `accept(entry: dict) -> None` (in `accepted.json` beside the files, keyed by id, storing the SHA-256 of `licence + "\n" + attribution`); `download(entry: dict) -> Path` (urllib, checks size and SHA-256 with hashlib, writes `<id>.onnx` only after both pass); `add_file(path: Path) -> str`; `remove(id: str) -> None`.
  - `plotline.Session.__init__` loads every kept model; a kept id the catalogue no longer lists is added as a file of your own. `Session.models() -> list[dict]`, `Session.add_model(path) -> str`.

- [ ] **Step 1: Write the failing tests in `tests/test_models.py`** (every test sets `PLOTLINE_CACHE_DIR` to `tmp_path`; the tiny model comes from `web/e2e/fixtures/tiny-conv.onnx`)

```python
def test_a_file_of_your_own_draws_with_learned(): ...  # add_model(fixture) -> "file-…"; set style learned, model id; run; paths > 0
def test_kept_models_load_with_a_new_session(): ...  # add_file(fixture); Session().models() lists it
def test_a_withdrawn_id_loads_as_a_file(): ...  # copy fixture to cache as "gone-1.onnx"; Session().models() has one "file-…" entry
def test_download_refuses_a_wrong_hash(monkeypatch): ...  # urlopen patched to serve other bytes; raises ValueError; no file in cache_dir()
def test_download_refuses_a_short_file(monkeypatch): ...  # serves the first half; raises ValueError; no file kept
def test_acceptance_is_per_licence_text(): ...  # accept(entry); accepted(entry) True; same id, changed licence: False
```

- [ ] **Step 2: Run them to verify they fail**

Run: `uv run pytest tests/test_models.py -q`
Expected: FAIL (`plotline.models` missing).

- [ ] **Step 3: Implement the binding and `plotline.models`**

- [ ] **Step 4: Run the Python suite**

Run: `uv run maturin develop --release` then `uv run pytest -q`, `uv run mypy src`, `uv run ruff check .`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/py src/plotline tests/test_models.py Cargo.lock
git commit -m "Load, keep and download models from Python"
```

### Task 6: `plotline models` and the MCP tool

**Files:**
- Modify: `src/plotline/cli/main.py`, `src/plotline/mcp_server.py`, `tests/test_cli.py`, `tests/test_mcp.py`

**Interfaces:**
- Consumes: Task 5's `plotline.models` and `Session` methods.
- Produces: a click group `models` with `list`, `get <id> [--accept-licence]`, `add <file>`, `remove <id>`; the MCP tool `list_models() -> dict` returning `{"downloaded": [...], "available": [...]}`. `get` prints the licence screen (label, licence, attribution, training, source, paper, size in MB, then `Provided as is, without warranty of any kind.`) and asks `Accept this licence and download? [y/N]`; without a yes it exits 1 and downloads nothing.

- [ ] **Step 1: Write the failing tests**

```python
# tests/test_cli.py
def test_models_add_then_list_shows_it(tmp_path, monkeypatch): ...   # PLOTLINE_CACHE_DIR; add fixture; list output contains "file-"
def test_models_get_without_yes_downloads_nothing(...): ...          # one-entry catalogue patched; input "n"; exit 1; cache empty
def test_models_get_with_accept_licence_skips_the_question(...): ... # urlopen patched to serve matching bytes; exit 0; file kept
def test_models_remove_deletes_the_file(...): ...
# tests/test_mcp.py
def test_list_models_reports_downloaded_and_available(...): ...      # fixture added; downloaded has one "file-…"; no download tool is registered
```

- [ ] **Step 2: Run them to verify they fail**

Run: `uv run pytest tests/test_cli.py tests/test_mcp.py -q -k models`
Expected: FAIL.

- [ ] **Step 3: Implement the group and the tool**

- [ ] **Step 4: Run the Python suite**

Run: `uv run pytest -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/plotline tests
git commit -m "Add plotline models and the MCP list_models tool"
```

### Task 7: The browser model module and the engine bridge

**Files:**
- Modify: `crates/models/src/lib.rs` (the `web` feature's wasm-bindgen exports), `crates/models/Cargo.toml`, `crates/wasm/src/lib.rs`, `crates/wasm/Cargo.toml`, `web/package.json`, `web/src/worker.ts`, `web/src/protocol.ts`, `.github/workflows/ci.yml`, `.github/workflows/pages.yml`, `.github/workflows/release.yml`, `.gitignore` (add `web/pkg-models/`)
- Create: `crates/wasm/js/models_bridge.js`, `web/src/modelsModule.ts`, `web/src/modelsModule.test.ts`

**Interfaces:**
- Consumes: Task 1's `ModelRuntime`, `Session::add_model`; Task 4's `TractRuntime`.
- Produces:
  - Model module exports (feature `web`): `load(id: &str, bytes: &[u8], side: usize) -> Result<(), JsError>`, `infer(id: &str, rgb: &[f32]) -> Result<Vec<f32>, JsError>`, `unload(id: &str)`, wrapping one process-wide `TractRuntime`.
  - `web/package.json` script `"wasm:models": "wasm-pack build ../crates/models --release --target web --out-dir ../../web/pkg-models --features web"` run with `RUSTFLAGS="-C target-feature=+simd128"`; `npm run wasm` runs both builds.
  - Engine: `struct BridgeRuntime;` implementing `ModelRuntime` through `#[wasm_bindgen(module = "/js/models_bridge.js")] extern "C"` functions `models_load`, `models_infer`, `models_unload`, which call `globalThis.plotlineModels` and throw `the model module is not loaded` when it is unset. `Session::new` in `crates/wasm` sets it. New wasm `Session` methods: `addModel(id: Option<String>, bytes: &[u8]) -> Result<String, JsError>` (returns the `ModelInfo` JSON), `removeModel(id: &str)`, `models() -> String`, and a free function `catalogue() -> String`.
  - `web/src/modelsModule.ts`: `simdSupported(): boolean` (validates the standard 0x0B-prefixed SIMD probe bytes with `WebAssembly.validate`) and `loadModelsModule(): Promise<void>`, which imports `../pkg-models/plotline_models.js` dynamically (Vite emits it as its own chunk), sets `globalThis.plotlineModels` once and rejects with `this browser lacks WebAssembly SIMD, which models need` when `simdSupported()` is false.
  - `worker.ts` awaits `loadModelsModule()` before handing `addModel` to the protocol. `protocol.ts` gains the methods `addModel`, `removeModel`, `models`, `catalogue`; `State` gains `models: ModelInfo[]`.

- [ ] **Step 1: Write the failing tests**

```ts
// web/src/modelsModule.test.ts
test("loadModelsModule refuses without SIMD", ...)       // WebAssembly.validate stubbed to false: rejects with the SIMD message, no fetch made
test("loadModelsModule loads once", ...)                  // two calls, one import of pkg-models
```

```rust
// crates/wasm/src/lib.rs, wasm_bindgen_test
#[wasm_bindgen_test] fn add_model_without_the_module_says_so() // addModel(None, b"x") error contains "the model module is not loaded"
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd web && npx vitest run src/modelsModule.test.ts` and `cd crates/wasm && wasm-pack test --node`
Expected: FAIL.

- [ ] **Step 3: Implement the exports, the bridge and the worker change; add the model build to the `wasm`, `web`, pages and release jobs**

On wasm32, enable tract's `getrandom-js` feature (the spike's working setup; confirm the feature name in tract 0.23's manifest). Add the size tuning as `[profile.release.package.<crate>]` entries in the workspace `Cargo.toml` and `wasm-opt = ["-Os"]` in `crates/models`' wasm-pack metadata; neither touches the engine module's crates. The size check in `ci.yml` gains a second line: `plotline_models_bg.wasm` at or under 9000000.

- [ ] **Step 4: Run the checks and the size**

Run: `cd web && npm run wasm && npm run check`; then the CI size checks by hand: `stat -c %s web/pkg/plotline_wasm_bg.wasm` (expected: at or under 2300000) and `stat -c %s web/pkg-models/plotline_models_bg.wasm` (expected: at or under 9000000).

- [ ] **Step 5: Commit**

```bash
git add crates web .github
git commit -m "Build the model runtime as its own WebAssembly module and bridge the engine to it"
```

## Part B: the library

### Task 8: Hosted model files (needs the owner)

**Gate:** the owner creates the public repository `plotline-models` under the same GitHub owner as `plotline`. Account and repository creation is the owner's step; this task prepares the contents and pushes them once it exists.

**Files (in `plotline-models`, not this repository):**
- Create: `convert.py`, `NOTICE`, `contour-1.md`, `teed-1.md`, `README.md` (what the repository holds, and how to ask for a model's removal), `contour-1.onnx`, `teed-1.onnx`

**Interfaces:**
- Produces: two ONNX files that follow the model convention in Global Constraints, at commit-pinned raw URLs, with their sizes, SHA-256s and overlaps, which Task 11 copies into the catalogue.

- [ ] **Step 1: Write `convert.py`** — downloads each original into its own new, empty directory; loads `.pth` only with `torch.load(path, weights_only=True)` and the contour weights from safetensors where offered; uses the model definitions from the two MIT repositories (read first); wraps each in a module that takes RGB in 0 to 1 and returns one channel in 0 to 1 where 1 is a line (contour: `1 - output`; TEED: its own BGR mean subtraction on 0 to 255, then the fused output's sigmoid); exports ONNX at `1 × 3 × 512 × 512`.
- [ ] **Step 2: Pick each overlap** — starting at 32 (TEED) and 64 (contour), raise in steps of 16 until a tiled run of the 1024 × 768 still life is within 1/255 of a whole-image run; record the result in the model card.
- [ ] **Step 3: Write `NOTICE` and the model cards** — each with the licence text, the attribution (the CC BY 3.0 credit to the Contour Drawings dataset for contour), the training note and the paper.
- [ ] **Step 4: Push, then record each file's raw URL at the pushed commit, its size and its SHA-256.**
  Run: `curl -sI -H "Origin: https://example.org" <url> | grep -i access-control-allow-origin` (expected: a header for each file).

### Task 9: Downloads and kept files in the browser

**Files:**
- Create: `web/src/models.ts`, `web/src/models.test.ts`

**Interfaces:**
- Consumes: Task 7's protocol methods `addModel`, `removeModel`, `models`, `catalogue`.
- Produces: `keptModels(): Promise<{ id: string; bytes: ArrayBuffer }[]>`, `keep(id: string, bytes: ArrayBuffer): Promise<boolean>` (false when storage refuses), `forget(id: string): Promise<void>`, `accepted(entry): boolean`, `accept(entry): void` (key: id plus SHA-256 of `licence + "\n" + attribution`, via `crypto.subtle`), `download(entry, onProgress?): Promise<ArrayBuffer>` (checks size and SHA-256 before resolving), and `restore(call): Promise<void>`, which adds every kept model at start and re-adds one whose id the catalogue no longer lists as a file of your own (`id: null`). Cache Storage name: `plotline-models`. Every storage call is wrapped in try/catch.

- [ ] **Step 1: Write the failing tests in `web/src/models.test.ts`**

```ts
test("download refuses a wrong hash", ...)            // fetch stubbed with other bytes: rejects, keep() never called
test("download refuses a short file", ...)            // half the bytes: rejects
test("keep reports false when Cache Storage throws", ...) // caches.open throws: resolves false, no throw
test("accepted survives a throwing localStorage", ...)    // getItem throws: accepted() false, accept() does not throw
test("restore re-adds a withdrawn id as a file", ...)     // kept "gone-1", catalogue []: addModel called with id null
test("acceptance is per licence text", ...)               // accept(entry); changed licence: accepted() false
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd web && npx vitest run src/models.test.ts`
Expected: FAIL.

- [ ] **Step 3: Implement `models.ts` and call `restore` from the app's start-up**

- [ ] **Step 4: Run them to verify they pass**

Run: `cd web && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src
git commit -m "Download, check and keep models in the browser"
```

### Task 10: Get models and Load your own in the style picker

**Gate:** the style picker redesign (its own session and PR) is merged into `main`, and this branch has merged `main`.

**Files:**
- Modify: the style picker component that PR adds, `web/src/app.tsx`, `web/src/style.css`, `web/src/tour.tsx`, `web/e2e/gui.spec.ts`, `web/e2e/` "every style" flow (skip `learned`)
- Create: `web/src/LicenceDialog.tsx`, `web/public/models/.gitkeep`

**Interfaces:**
- Consumes: Task 9's functions; Task 7's protocol methods.
- Produces: tab ids `styles`, `get-models`, `load-own`. Each loaded model is a Styles entry: style `learned` with `model` set, its label from `ModelInfo.label`, its picture `models/<id>.png` when the file exists, else a plain placeholder, and no trial run. The licence screen shows label, licence, attribution, training, source, paper, size in MB and the as-is line, and its Accept button stays disabled until the "I've read the licence" box is ticked. Load your own shows a file input (`accept=".onnx"`) and the responsibility line, and nothing else.

- [ ] **Step 1: Write the failing e2e flow in `web/e2e/gui.spec.ts`**

```ts
test("a model of your own draws in the learned style", ...) // open picker, Load your own, set input file to e2e/fixtures/tiny-conv.onnx, pick its Styles entry, run finishes, paths > 0, no console error
test("Get models shows the licence before any download", ...) // with the catalogue stubbed to one entry and fetch blocked: Download opens the licence screen; Accept disabled until ticked; no request made before Accept
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cd web && npm run build && npm run e2e -- -g "model"`
Expected: FAIL.

- [ ] **Step 3: Implement the tabs, the licence screen and the tour step**

Tour step copy: "Get models: download a trained line model, or load your own, to draw the lines a person would draw."

- [ ] **Step 4: Run the whole browser suite**

Run: `cd web && npm run check && npm run build && npm run e2e`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "Add Get models and Load your own to the style picker"
```

### Task 11: The first catalogue entries, pictures and docs

**Gate:** Task 8 is pushed.

**Files:**
- Modify: `crates/core/src/models/catalogue.json`, `docs/styles.md`, `README.md` (styles table), `CHANGELOG.md`
- Create: `crates/models/examples/model_cards.rs`, `web/public/models/contour-1.png`, `web/public/models/teed-1.png`

**Interfaces:**
- Consumes: Task 8's URLs, sizes, hashes and overlaps; Task 1's `catalogue()`; Task 4's `TractRuntime`.
- Produces: entries `contour-1` and `teed-1` with the fields of `CatalogueEntry`; descriptions and training notes from the spec's "The first entries".

- [ ] **Step 1: Fill `catalogue.json` and run the catalogue test**

Run: `cargo test -p plotline-core models::tests::catalogue_entries_are_complete`
Expected: PASS.

- [ ] **Step 2: Write `model_cards`** — it reads the two files from the user's model cache, draws `learned` on the still life (A4, 0.3 mm black pen) with each, and writes `web/public/models/<id>.png` at 300 px wide.

Run: `cargo run --release -p plotline-models --example model_cards -- web/public/models`

- [ ] **Step 3: Download both through the CLI and draw once with each**

Run: `uv run plotline models get contour-1 --accept-licence`, the same for `teed-1`, then `uv run plotline run <a test image> --style learned --set model.model=teed-1 -o target/teed.svg`
Expected: both downloads pass their hash checks; the SVG has paths.

- [ ] **Step 4: Document the style** — a `learned` section in `docs/styles.md` (what it does, when to use it, `resolution` and `invert`, that models download from Get models or `plotline models`), the README table row, and a CHANGELOG entry.

- [ ] **Step 5: Commit**

```bash
git add crates web/public/models docs README.md CHANGELOG.md
git commit -m "List the contour and TEED models and document the learned style"
```

### Task 12: Brief and final checks

**Files:**
- Modify: `docs/BRIEF.md` (§7 styles list, §11 a "Models" bullet: hosted models follow the catalogue rules in §14, "Model catalogue"; the five §14 rows' "not built" become "built on <date>"), `docs/superpowers/specs/2026-10-16-learned-line-models-design.md` (status line)

- [ ] **Step 1: Update the brief and the spec status.**
- [ ] **Step 2: Run every check CI runs**

Run: `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked -p plotline-core`, `cargo test --locked -p plotline-models`, `uv run pytest -q`, `uv run python scripts/check_licenses.py`, `cd web && npm run wasm && npm run check && npm run build && npm run e2e`, the gallery snapshot diff from Task 3, and the size check from Task 7.
Expected: all pass; the snapshot is unchanged; the engine module is at or under 2,300,000 bytes.

- [ ] **Step 3: Commit**

```bash
git add docs
git commit -m "Record the learned line models as built"
```
