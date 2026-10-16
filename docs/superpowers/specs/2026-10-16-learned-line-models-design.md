# Learned line models and the model library

Status: designed on 2026-10-16, not built. Decisions are recorded in the brief's decision log under "Learned line models", "Model runtime", "Model catalogue", "Model hosting" and "Model downloads".

## Goal

Users asked for a drawing that follows the lines a person would draw from a photo, not the photo's tone. A classical line style, built under its own spec, finds those lines with filters. This spec adds a second way: models trained to turn a photo into a line drawing, which the user downloads when they want one. Nothing runs on a server: the models run in the visitor's browser, and the CLI, Python and MCP use the same models natively.

The work has two parts, built in this order:

1. **The engine.** A model runs inside a style, behind a small interface, and is tested against a stand-in model written in Rust.
2. **The library.** The models plotline offers, how a download reaches the user, and loading a model file of your own.

## Measurements

A throwaway spike on 2026-10-16 converted two published models to ONNX and ran them with tract 0.23 (MIT or Apache-2.0) in release builds, natively and in WebAssembly under Node. The input was the still life rendered in code.

| Model, input | WebAssembly with SIMD | WebAssembly without SIMD | Peak memory |
|---|---|---|---|
| TEED, 512 × 384 | 0.17 s | 0.69 s | 79 MB |
| TEED, 1024 × 768 | 0.75 s | 2.7 s | 294 MB |
| TEED, 2048 × 1536 | 3.0 s | | 1.45 GB |
| contour, 512 × 384 | 3.8 s | 11.2 s | 282 MB |
| contour, 1024 × 768 | 16.4 s (7.4 s native) | | 887 MB |
| contour, 2048 × 1536 | 66 s | | 3.3 GB |

| File | Raw | Gzipped |
|---|---|---|
| Engine module today | 1.98 MB | 0.75 MB |
| Engine module with tract inside | 14.9 MB | 4.1 MB |
| Second engine build with tract, size-tuned, SIMD | 10.1 MB | 3.1 MB |
| tract alone as its own module, size-tuned, SIMD | 8.3 MB | 2.4 MB |
| contour model, fp32 | 17.2 MB | 15.9 MB |
| TEED model, fp32 | 0.25 MB | 0.22 MB |

What follows from them:

- tract runs both models, every operator supported. Its output is within 1.4e-3 of PyTorch's, no pixel off by more than 1/255, and native and WebAssembly outputs differ by at most 5.1e-6. After rounding to 8 bits that still flips 1 to 11 pixels per image, so a learned style's output is not bit-identical across platforms.
- A whole 2048-pixel image takes 3.3 GB, close to WebAssembly's 4 GB ceiling, so models run in tiles (see "Tiles").
- SIMD makes the models about four times faster, and every current browser supports WebAssembly SIMD, so the model module is built with it.
- tract inside the engine module would take every visitor's download from 0.75 MB to 4.1 MB gzipped, against a CI limit of 2.3 MB raw. tract therefore goes in a module of its own, fetched only when a model first runs. A module holding tract alone is 0.7 MB smaller gzipped than a second engine build with tract, and keeps one engine in memory.
- "Size-tuned" is `opt-level = "s"` for every crate but `tract-linalg`, which stays at 3, and `wasm-opt -Os`; it runs as fast as `-O3`. Relaxed SIMD is not used, because its results can differ between machines.
- tract's own NNEF format would save another 0.55 MB gzipped for a fixed tile size, but a file of your own arrives as ONNX, so the module keeps the ONNX loader.

## The style

A new style, `learned` ("Learned lines"), runs four stages: `model`, `split`, `ink` and `order`. Only `model` is new.

The `model` stage requires `Image` and `Canvas` and provides `Tone`, as the tone stage does. It places the image on the page with the tone stage's own code (`map_size` and the crop in `stages/tone.rs`), so a learned drawing lines up with every other style on the same image. It then runs the chosen model and writes the result into a `ToneMap` in which ink is line darkness, 0 for paper and 1 for a full line. From there the existing stages draw it: `split` shares it between pens, `ink` traces it, and `order` orders the paths. The style sets `ink` to `centreline` by default, so a line becomes one stroke down its middle.

Settings of the `model` stage:

- `model`: the id of a loaded model (see "Model ids"). The default is empty, which is a settings error that says how to load a model; the picker always sets it.
- `resolution`: the long side, in pixels, of the image the model sees. Default 1024, range 256 to 2048. Higher finds finer lines and takes longer; the table above gives the cost.
- `invert`: whether the model's output is flipped before use. Default false. Catalogue entries already say which way their output runs, so this is only for a model file of your own that draws white lines on black.

The ink stage's existing `threshold` decides how dark a line must be to draw.

In the style picker, each loaded model is its own entry: the `learned` style with `model` already set. The picker's tabs for getting and loading models are described under "In the GUI".

## The engine

### Model ids

A session keeps a model store: a map from model id to a loaded model. A catalogue model's id is its catalogue id, which carries a version (`contour-1`, `teed-1`); a new conversion of the same model gets a new id. A model file of your own gets `file-` and the first 12 hex digits of its SHA-256.

The id is an ordinary setting, so the stage cache, presets, the page's undo history, trial runs and MCP runs need nothing new. Two different files never share an id, so a cached tone map always belongs to the model its key names.

A setting that names a model the session has not loaded is a settings error that says how to load it: "model contour-1 is not loaded; get it from Styles, then Get models, or run `plotline models get contour-1`". Loading a preset that names a missing model gives the same message.

### The runtime interface

The core defines one trait, implemented once per platform:

```rust
pub trait ModelRuntime: Send + Sync {
    /// Parse `bytes` as a model for square tiles of `side` pixels and keep it under `id`.
    fn load(&self, id: &str, bytes: &[u8], side: usize) -> Result<(), ModelError>;
    /// Run the model `id` on one tile: `rgb` is 3 × side × side floats, channel-major.
    /// Returns side × side floats.
    fn infer(&self, id: &str, rgb: &[f32]) -> Result<Vec<f32>, ModelError>;
    /// Drop the model `id`.
    fn unload(&self, id: &str);
}
```

Every model takes RGB in 0 to 1, channel-first, and returns one channel in 0 to 1 where 1 is a line. `load` fixes the input to 1 × 3 × side × side, so tract optimises once per model, and refuses a model that does not take that shape or return 1 × 1 × side × side. Everything that is algorithm stays in the core: resizing to `resolution`, tiling, mirroring, `invert`, and placing the result on the page.

`Session::add_model(id, bytes)` checks the file's size and, for a catalogue id, its size and SHA-256 against the entry (see "Downloads"), calls `ModelRuntime::load`, and keeps the model's id and its catalogue entry, or the defaults for a file of your own, in the store. The runtime holds the parsed model. The `model` stage reaches the store through `RunContext`, which gains it, and looks its model up in `start`.

Each platform implements the trait like this:

- **Native (CLI, Python, MCP):** a new crate, `crates/models`, wraps `tract-onnx`. It is linked into the Python extension.
- **Browser:** the same crate is built as a separate WebAssembly module with SIMD (`plotline_models.wasm`). The engine module calls it through a JavaScript import that the worker provides: it copies a tile from the engine's memory into the model module's, calls `infer`, and copies the result back. Both modules live in the one worker, so the call is synchronous and a stage's `step` stays as it is.
- **Tests:** a stand-in written in Rust (see "Tests").

### Loading the model module

The worker fetches `plotline_models.wasm` before the first model is added to the session: at start, when the browser keeps downloaded models, or when the user first downloads or loads one. After that the browser's HTTP cache holds it. Adding a model awaits the module, so a run never starts and then waits for it. If the browser lacks WebAssembly SIMD, the fetch is not attempted and adding a model reports a plain error naming the missing browser feature.

The module is part of the hosted page's build, served next to the engine module, so it needs no other host.

### Tiles

The model sees the image in tiles of 512 × 512 pixels, margins included: each tile keeps its centre, `512 - 2 × overlap` pixels on a side, and its margin of `overlap` pixels gives that centre context. The catalogue entry gives the overlap. It starts at 32 for TEED and 64 for contour, and the conversion script checks that a tiled run of each model on the still life stays within 1/255 of a whole-image run, raising the overlap until it does. A model file of your own uses 64. The margin is cut away before the tile's centre is written into the line map, so seams fall where every pixel still saw `overlap` pixels of context. Pixels past the image's edge are filled by mirroring it. Tile sides are padded up to a multiple of 8, which the contour model needs.

Each `step` of the `model` stage runs one tile. The progress bar moves per tile, Stop takes effect between tiles, and memory stays near the 512-pixel rows of the table, scaled to a 512 × 512 tile: about 400 MB for contour, whatever the resolution.

The model's output for an image is cached like any other stage's, so changing the ink threshold, the pens or the order does not run the model again.

## The library

### The catalogue

The catalogue is a file in the core, `crates/core/src/models/catalogue.json`, built into every front end. It changes only with a plotline release, and nothing fetches a catalogue at run time.

Each entry holds:

- `id`, `label`, and `description`: what the model draws, in one or two sentences.
- `url` to the model file, pinned to a commit; `sha256`; and `bytes`.
- `licence` (an SPDX id), `attribution` (the text the licence asks to be shown), and `source` and `paper` (links).
- `training`: one or two sentences on what the model was trained on, and under which licences.
- `overlap`: the tile margin in pixels.

The conversion script folds each model's own input scaling, channel order and polarity into the ONNX graph, so every hosted model follows the convention above and a new model needs a catalogue entry, not code.

The entries make no claim about what a user may do with a drawing. They show the licence, the attribution and the training note, and the user reads them.

### What the catalogue may hold

A model is listed only when all of these hold:

1. Whoever owns the weights releases them under a permissive licence: MIT, BSD, Apache-2.0, CC0 or CC BY.
2. Its training data is documented, and the licence of each dataset is known and written into `training`. A model whose training data cannot be traced is not listed.
3. That licence lets plotline host its own converted copy.

The first entries:

- `contour-1`: the contour style of Chan, Durand and Isola, "Learning to generate line drawings that convey geometry and semantics", CVPR 2022. The repository, which publishes the weights, is MIT. Its drawings come from the Contour Drawings dataset (CC BY 3.0), and its photos from a subset of COCO, whose images keep their Flickr licences. Clean, closed lines; slow (16 s at 1024 pixels in the browser).
- `teed-1`: TEED, X. Soria, Y. Li, M. Rouhani and A. D. Sappa, "Tiny and efficient model for the edge detection generalization", Proc. ICCV Workshops, 2023, pp. 1364-1373. MIT, and the repository ships the weights trained on BIPED, a dataset by the same first author. Fast (under a second at 1024 pixels); it finds edges, texture included, rather than an artist's selection of lines.

The same paper's anime style is not listed, because the provenance of its training sketches cannot be checked.

### Hosting

plotline hosts its own converted copies in a separate public repository, `plotline-models`, under the same GitHub owner. It holds only the ONNX files, a `NOTICE` with each model's licence and attribution, a model card per model, and the conversion script, which loads the original weights with `torch.load(weights_only=True)` or from safetensors. The files are served from raw.githubusercontent.com, which sends the CORS header a browser needs, and every catalogue URL names a commit, so a file never changes under its hash.

To withdraw a model, the next release drops its entry and the file is deleted from `plotline-models`. Old commit URLs would still serve it, so a full withdrawal rewrites that repository's history; it holds nothing else, so that is cheap. A model a user already downloaded stays on their machine.

The models stay out of the main repository, which keeps its rule of no third-party files.

### Downloads

A download goes through the same steps in every front end:

1. Show the licence screen: the label, the licence text, the attribution, the training note, the source and paper links, the size, and "Provided as is, without warranty of any kind."
2. Wait for the user to accept. Acceptance is kept per model id and per hash of the licence text, so a changed licence asks again.
3. Fetch the file from the catalogue URL.
4. Check `bytes` and `sha256`. On a mismatch, discard the file and report a plain error.
5. Keep the file, then load it into the session's store.

A model file of your own skips steps 1 to 4. The front end reads the file and loads it under its `file-` id. It never leaves the machine.

Every model file, downloaded or your own, must:

- be at most 200 MB;
- parse as ONNX (anything else is refused, and PyTorch `.pth` files, which can run code when unpickled, are never opened);
- pass `ModelRuntime::load`'s shape check.

A model runs inside the worker's WebAssembly sandbox in the browser and inside tract natively, so a hostile file can fail or run long, and Stop still works between tiles.

### In the GUI

The style picker (built under its own spec) gains two tabs:

- **Get models:** one card per catalogue entry, with the label, the description, a picture of the model's drawing of the still life, the size and the licence. A card's button downloads (through the licence screen), or removes a downloaded model. A downloaded model appears in the Styles tab.
- **Load your own:** a file button for an `.onnx` file, with one line beneath it: "You're responsible for the licence of any model you load." No list of other models.

Downloaded files are kept in the browser's Cache Storage under the page's origin, and the accepted licences in `localStorage`. Every read and write is wrapped so the page works, without remembering, where storage is blocked. At start the worker loads every kept model into the store, so a reload keeps the user's models.

The cards' pictures are drawn by plotline from the still life with each downloaded model, by a new example, `model_cards`, run by hand when an entry is added, and committed under `web/public/models/`. The picker asks for no trial picture of a `learned` entry, since a contour run takes seconds, and shows the card's picture instead.

### In the CLI, Python and MCP

- `plotline models list` prints the catalogue, marking downloaded models, and any models of your own.
- `plotline models get <id>` prints the licence screen and asks; `--accept-licence` accepts without asking, for scripts.
- `plotline models add <file.onnx>` copies a file of your own into the cache.
- `plotline models remove <id>` deletes a model.

Files live in `plotline/models` under the user's cache folder: `XDG_CACHE_HOME` or `~/.cache` on Linux, `~/Library/Caches` on macOS, `%LOCALAPPDATA%` on Windows. Accepted licences are kept in a JSON file beside them. The download uses Python's standard library. The Python `Session` loads every kept model into its store when it starts, and gains `models()` and `add_model(path)`.

MCP gains one tool, `list_models`. A run may use any downloaded model. There is no download tool: a person accepts each licence, in the GUI or the CLI.

## Tests

- **Core:** the stand-in runtime runs a fixed 3 × 3 edge filter on the luminance. With it, unit tests cover the `model` stage (tiles, seams, mirroring, padding, polarity, `invert`, cancellation between tiles), and the property laws include the `learned` style.
- **`crates/models`:** a tiny ONNX model built in code, a single 3 × 3 convolution with fixed weights, is loaded and run, and compared with the same convolution computed directly. The shape check, the size cap, a non-ONNX file and a truncated file each have a test.
- **Gallery snapshot:** `learned` stays out. The models are not in the repository, and their output differs across platforms (see "Measurements").
- **Catalogue:** a test checks that every entry parses, has a permissive SPDX id, a 64-hex `sha256` and a commit-pinned `url`, and names `training`.
- **Browser:** an e2e flow loads the tiny ONNX model through "Load your own" and draws with it. CI never downloads a catalogue model.
- **Tour:** a step for the Get models tab.

## Out of scope

- Other learned styles, such as hatching or stippling driven by a model. The line map only feeds `ink`.
- Fine-tuning or training models in plotline.
- fp16 model files, until tract's fp16 output is checked against fp32 for both models.
- A catalogue fetched at run time.
