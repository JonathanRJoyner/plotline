# plotline spec 1: scaffold and core engine

Date: 2026-10-11. Status: approved design, awaiting owner review of this document.

This spec covers the first of six sub-projects: the package scaffold, the core engine, a minimal session layer and a minimal CLI. It implements milestones 1 and 2 of [the brief](../../BRIEF.md), plus a minimal path-ordering stage, a preview rasterizer, stats and presets, because those make the pipeline realistic and let results be checked visually before the GUI exists. Later specs build on the stage API defined here, so this API is the part to get right.

## 1. Scope

Delivered by this spec:

- Repository scaffold: package layout, build and dev tooling, CI, licence check, synthetic test images.
- Core types: canvas, pen, tone map, paths, layers, result.
- Settings declarations and the schema they generate.
- The stage contract, frames, run context, cancellation.
- Styles as ordered stages with declared artifact kinds, and the cached runner.
- Four stages: tone map, grid placer, dot renderer, path ordering.
- One built-in style, `grid`, which exercises the whole pipeline.
- SVG and JSON export.
- Session: state, worker thread, frame delivery, export, stats, preview PNG, presets.
- CLI: `styles`, `describe`, `run`, and stubs for `gui` and `mcp`.
- Tests and CI.

Out of scope, with the spec that owns each:

- Voronoi relaxation, stipple, continuous line, dashes, endpoint merging in ordering: spec 2.
- Sketch Lines: spec 3.
- GUI server and frontend, WebSocket frame streaming: spec 4.
- MCP server: spec 5.
- numba acceleration, disk cache, toolchain adapters, G-code, multi-pen colour, crop offset for the canvas: later.

## 2. Package layout

A `src` layout with three packages that depend only downward. `core` knows nothing about files, threads or networks. `session` owns state and side effects. `cli` only calls `session`.

```
pyproject.toml              hatchling build; uv, ruff, mypy, pytest config
uv.lock
README.md  LICENSE  CLAUDE.md
docs/BRIEF.md
docs/superpowers/specs/
src/plotline/
  __init__.py               version string
  core/
    __init__.py
    types.py                Canvas, Pen, ToneMap, Layer, Result
    settings.py             Settings base model, setting() helper
    stage.py                Stage, Frame, RunContext, CancelToken, Cancelled
    pipeline.py             Style, artifact resolution, run_style()
    cache.py                StageCache
    registry.py             built-in and entry-point style discovery
    stages/
      __init__.py
      tone.py               ToneStage
      grid.py               GridPlacer, DotRenderer
      order.py              OrderStage
    export/
      __init__.py
      svg.py                to_svg()
      paths_json.py         to_json_dict()
    styles.py               built-in Style definitions
  session/
    __init__.py
    session.py              Session, RunHandle, RunState
    images.py               load_image(): file to float32 RGB array
    preview.py              render_png()
    presets.py              save_preset(), load_preset(), migrations
    stats.py                compute_stats()
  cli/
    __init__.py
    main.py                 click group and commands
tests/
  conftest.py               fixtures, --update-golden flag
  synthetic.py              generated test images
  golden/                   tiny JSON path outputs
  test_*.py
scripts/
  check_licenses.py         CI licence allow-list check
.github/workflows/ci.yml
```

## 3. Dependencies and tooling

Runtime dependencies, all permissively licensed:

| Package | Licence | Used for |
|---|---|---|
| numpy | BSD-3 | arrays everywhere |
| scipy | BSD-3 | image resize and blur, KD-tree for ordering |
| Pillow | HPND (permissive) | image loading, preview rasterizer |
| pydantic 2 | MIT | settings models and JSON Schema |
| click | BSD-3 | CLI |

Optional extra `fast` declares numba (BSD-2) for later specs. Nothing in this spec imports it.

Dev dependencies: pytest, ruff, mypy. The licence check script uses the standard library only.

Tooling decisions: Python 3.11 or newer; uv for environments and locking, with `uv.lock` committed; hatchling as the build backend; ruff for both linting and formatting; mypy in strict mode on `src/plotline/core`, standard mode elsewhere; pytest.

Vector rasterizers commonly used from Python are LGPL and are excluded by the licence rule, which is why the preview uses Pillow.

## 4. Core types

All in `core/types.py`, as frozen dataclasses unless stated.

**Pen:** `name: str`, `width_mm: float`, `color: str` as `#rrggbb`.

**Canvas:** `width_mm`, `height_mm`, `margin_mm` (default 0), `fit: Literal["fit", "fill"]`.

- `drawing_rect` returns `(x, y, w, h)` in mm: the page minus the margin.
- `image_rect(width_px, height_px)` returns the mm rectangle the image occupies. With `fit` the whole image is scaled to sit inside the drawing rect, centred, preserving aspect ratio. With `fill` the image is scaled to cover the drawing rect and the overflow is cropped equally on both sides, so the rectangle equals the drawing rect.
- `image_transform(width_px, height_px)` returns `(mm_per_px, origin_mm)`.

The brief also lists `crop`. It is `fill` plus an offset and is deferred, recorded in the brief.

**ToneMap:** `ink: np.ndarray` float32 of shape `(H, W)`, 0 is white and 1 is full ink; `mm_per_px: float`; `origin_mm: tuple[float, float]`. Methods: `to_mm(px)`, `to_px(mm)` for `(N, 2)` arrays, `sample(points_mm)` bilinear sampling clamped at the edges, `rect_mm`.

**Paths:** a path is a plain `np.ndarray` of shape `(N, 2)`, float64, in mm, with `N >= 1`. A path with `N == 1` is a dot. There is no Path class, so every algorithm works on arrays directly.

**Layer:** `pen: Pen`, `paths: list[np.ndarray]`.

**Result:** `canvas: Canvas`, `layers: list[Layer]`.

Coordinates are page coordinates: origin at the top-left corner of the page, x to the right, y down, in mm. SVG uses the same convention, so export needs no flip.

## 5. Settings

`core/settings.py` defines:

```python
class Settings(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

def setting(default, *, help: str, min=None, max=None, step=None,
            unit: str | None = None, advanced: bool = False,
            label: str | None = None): ...
```

`setting()` returns a pydantic `Field` with `description=help`, `ge=min`, `le=max`, and `json_schema_extra` holding `step`, `unit`, `advanced` and `label`. Choices are declared with `Literal[...]`. Stages that use randomness declare `seed: int = setting(0, help=..., advanced=True)`.

Each stage has one `Settings` subclass. `extra="forbid"` makes typos from the CLI and MCP fail loudly. Frozen models are hashable and safe to use in cache keys. Validation uses pydantic's default lax mode so CLI strings such as `"0.5"` coerce to floats.

The JSON Schema from `model_json_schema()` is the only description of settings that any front end sees.

## 6. Stage contract

`core/stage.py`:

```python
@dataclass(frozen=True)
class Frame:
    progress: float                     # 0..1 within the stage
    label: str                          # e.g. "row 12/40"
    points: np.ndarray | None = None    # (N, 2) mm
    paths: list[np.ndarray] | None = None
    stage: str = ""                     # filled in by the runner

class Cancelled(Exception): ...

class CancelToken:
    def set(self) -> None
    def is_set(self) -> bool

class RunContext:
    def __init__(self, token: CancelToken)
    def check(self) -> None             # raises Cancelled if token is set
    def rng(self, seed: int) -> np.random.Generator   # np.random.default_rng(seed)

class Stage(ABC):
    name: str                           # unique within a style
    label: str                          # shown in UIs
    version: str = "1"                  # bump when the algorithm changes
    requires: tuple[str, ...]           # artifact kinds consumed
    provides: str                       # artifact kind produced
    settings_model: type[Settings]

    @abstractmethod
    def run(self, inputs: Mapping[str, Any], settings: Settings,
            ctx: RunContext) -> Generator[Frame, None, Any]: ...
```

`run` is a generator. It yields `Frame`s while working and returns the artifact as the generator's return value. The runner reads it with `yield from`.

A stage must yield at least one frame. The runner calls `ctx.check()` after every frame it receives, so a stage that yields regularly needs no explicit cancellation checks. A stage with a long loop that does not yield must call `ctx.check()` itself.

Artifact kinds in this spec:

| Kind | Type | Provided by |
|---|---|---|
| `image` | float32 `(H, W, 3)` RGB in 0..1 | caller (root) |
| `canvas` | `Canvas` | caller (root) |
| `pens` | `tuple[Pen, ...]` | caller (root) |
| `tone` | `ToneMap` | ToneStage |
| `points` | float64 `(N, 2)` mm | GridPlacer |
| `paths` | `list[np.ndarray]` | DotRenderer, OrderStage |
| `layers` | `list[Layer]` | no stage in this spec; reserved for multi-pen styles |

## 7. Styles and artifact resolution

`core/pipeline.py`:

```python
@dataclass(frozen=True)
class Style:
    name: str
    label: str
    description: str
    stages: tuple[Stage, ...]
```

On construction the style validates itself and raises `ValueError` otherwise:

- stage names are unique;
- every `requires` kind of every stage resolves to a root kind or to the `provides` of an earlier stage, choosing the nearest earlier provider when several exist;
- the last stage provides `paths` or `layers`.

The resolution is stored so the runner does not recompute it. Because the nearest provider wins, an ordering stage that requires and provides `paths` reads the renderer's output.

`style.schema()` returns `{"name", "label", "description", "stages": [{"name", "label", "schema": <JSON Schema>}, ...]}`. `style.defaults()` returns `{stage.name: stage.settings_model()}`.

This diverges from the brief, which says a stage takes the previous stage's output. Renderers need both points and the tone map, and future hatching styles need only the tone map, so declared kinds are the general form. Recorded in the brief.

## 8. Runner and cache

```python
def run_style(style: Style, settings: Mapping[str, Settings],
              roots: Mapping[str, Any], cache: StageCache,
              ctx: RunContext) -> Generator[Frame, None, Result]: ...
```

Root keys: `image` is a BLAKE2b hash of the array bytes, shape and dtype; `canvas` and `pens` are hashes of their canonical JSON.

Stage key: BLAKE2b hex digest of canonical JSON (sorted keys, no whitespace) of `{"stage": <module.qualname>, "name", "version", "settings": settings.model_dump(), "inputs": {kind: key_of_resolved_source}}`. Two styles that share a stage with equal settings and equal inputs therefore share the cached output.

For each stage in order the runner:

1. Computes the key.
2. If the key is cached, takes the artifact and yields one `Frame(progress=1, label="cached")`.
3. Otherwise drives `stage.run()`, forwards each frame with `stage` set to the stage name, calls `ctx.check()` after each, and stores the returned artifact in the cache.

After the last stage, if it provided `layers` the result uses them; otherwise the result is one `Layer(pens[0], paths)`. The runner returns `Result(canvas, layers)`.

`core/cache.py`:

```python
class StageCache:
    def __init__(self, max_bytes: int = 512 * 2**20)
    def get(self, key: str) -> Any | None     # marks as recently used
    def put(self, key: str, value: Any) -> None
    def __contains__(self, key: str) -> bool
    def clear(self) -> None
    hits: int; misses: int; bytes: int
```

In memory only, least-recently-used eviction until under `max_bytes`. Size is estimated as `nbytes` for arrays, the sum for lists, tuples and `Layer`s, and `sys.getsizeof` otherwise. A value larger than `max_bytes` is not stored. Keys are content hashes, so a disk cache can be added later without changing them.

Cancellation: the runner propagates `Cancelled`. A cancelled stage stores nothing, so partial output is discarded, but every earlier completed stage stays cached.

Determinism: all randomness comes from `ctx.rng(settings.seed)`. No module-level random state. The same inputs and settings give bitwise-identical output on the same machine.

## 9. Stages in this spec

**ToneStage** (`name="tone"`, requires `image`, `canvas`; provides `tone`). Settings:

| Setting | Default | Range | Notes |
|---|---|---|---|
| `resolution` | 1000 | 64..4000 | working long-edge size in px |
| `invert` | false | | |
| `black` | 0.0 | 0..1 | levels: luminance at or below this becomes black |
| `white` | 1.0 | 0..1 | levels: luminance at or above this becomes white |
| `gamma` | 1.0 | 0.1..5 | |
| `contrast` | 0.0 | -1..1 | |
| `blur_mm` | 0.0 | 0..10 | advanced; Gaussian sigma in mm |

Processing order: luminance from RGB with the ITU-R BT.601 weights 0.299, 0.587, 0.114; crop to the drawing rect aspect when `fit` is `fill`; resize so the long edge is `resolution` px; levels `(Y - black) / (white - black)` clipped to 0..1; gamma `Y ** gamma`; contrast `Y = clip(0.5 + (Y - 0.5) * (1 + contrast), 0, 1)`; `ink = 1 - Y`; invert flips ink; blur with sigma `blur_mm / mm_per_px` when positive. Output is a `ToneMap` whose `mm_per_px` and `origin_mm` come from `canvas.image_transform()`.

**GridPlacer** (`name="points"`, requires `tone`; provides `points`). Settings: `spacing_mm` default 2.0, range 0.2..20; `threshold` default 0.5, range 0..1. Emits one point at every grid cell centre inside the tone map rectangle whose sampled ink is at least `threshold`. Yields one frame per grid row with the points placed so far. Trivial on purpose: it exists to exercise caching, cancellation and determinism, and ships as a usable style.

**DotRenderer** (`name="render"`, requires `points`; provides `paths`). No settings beyond the empty model. Each point becomes a one-vertex path. Yields a frame at the end. Spec 2 extends it in place into the stipple renderer with dot size and dot shape settings.

**OrderStage** (`name="order"`, requires `paths`; provides `paths`). Settings: `optimize` default true; `min_length_mm` default 0.0, range 0..10, with help text saying dots have zero length and are dropped by any positive value. Drops paths shorter than `min_length_mm`. When `optimize` is on, orders paths greedily: start at the drawing rect's top-left corner, repeatedly move to the nearest unvisited path endpoint, reversing the path when its end is nearer than its start. A KD-tree over all endpoints keeps this near `O(n log n)`. Yields a frame about every 5 percent of paths with the ordered paths so far. Endpoint merging is spec 2.

**The `grid` style:** `tone`, `points`, `render`, `order`. Label "Grid dots". Registered as a built-in.

## 10. Export

Both exporters are pure functions in `core/export`. The session writes files.

**SVG** (`to_svg(result) -> str`):

```xml
<svg xmlns="http://www.w3.org/2000/svg" width="210mm" height="297mm" viewBox="0 0 210 297">
  <g id="pen-1" data-pen="Black 0.3" fill="none" stroke="#000000" stroke-width="0.3"
     stroke-linecap="round" stroke-linejoin="round">
    <path d="M 10.000 10.000 L 20.000 15.500"/>
    <path d="M 30.000 30.000 L 30.000 30.000"/>
  </g>
</svg>
```

One group per layer, numbered in order, carrying the pen name in `data-pen`. Coordinates have three decimals. A one-vertex path is written as `M x y L x y`, which SVG renders as a dot when the line cap is round and which plotter software treats as a pen-down at a point. No vendor-namespaced attributes.

**JSON** (`to_json_dict(result) -> dict`):

```json
{"format": "plotline-paths", "version": 1, "units": "mm",
 "canvas": {"width_mm": 210, "height_mm": 297, "margin_mm": 10, "fit": "fit"},
 "layers": [{"pen": {"name": "Black 0.3", "width_mm": 0.3, "color": "#000000"},
             "paths": [[[10.0, 10.0], [20.0, 15.5]], [[30.0, 30.0]]]}]}
```

Coordinates rounded to three decimals so golden files are stable.

## 11. Session

`session/session.py`. One `Session` holds one working context. It is not thread-safe for concurrent mutation; front ends call it from one thread.

State:

- `image: np.ndarray | None`, float32 RGB. `load_image(path)` uses Pillow, converts to RGB, and downsizes so the long edge is at most 4000 px. `set_image(array)` accepts an array directly.
- `canvas: Canvas`, default 210 by 297 mm with a 10 mm margin and `fit`.
- `pens: tuple[Pen, ...]`, default one pen named "Black 0.3", 0.3 mm, `#000000`.
- `style: Style`, `settings: dict[str, Settings]`. `set_style(name)` resets settings to that style's defaults and keeps the cache. `update(stage, **values)` validates and replaces that stage's settings.
- `speeds`: `draw_mm_s` 50, `travel_mm_s` 100, `pen_lift_s` 0.2, used by stats only.
- `cache: StageCache`, `result: Result | None`, `state: RunState` in `idle`, `running`, `done`, `cancelled`, `error`.

Running:

- `run() -> RunHandle` cancels any run in progress, then starts a worker thread that drives `run_style()`. numpy releases the interpreter lock, so a thread is sufficient. The handle exposes `wait()`, `cancel()`, `state`, `result`, `error`.
- `run_blocking() -> Result` runs in the calling thread and re-raises stage errors. The CLI and tests use it.
- `cancel()` sets the token of the current run.
- `subscribe(callback) -> unsubscribe` registers a frame callback. Frames are delivered from the worker thread at most `max_fps` times per second, default 15, except that a frame with `progress == 1` is always delivered. Callbacks must return quickly; the GUI spec will wrap one in a queue.
- A stage exception sets `state` to `error`, stores the exception on the handle, and leaves `result` unchanged.

Outputs:

- `svg() -> str`, `json_dict() -> dict`, `export_svg(path)`, `export_json(path)`.
- `stats() -> Stats` with `path_count`, `pen_down_mm`, `pen_up_mm`, `estimated_seconds`. Pen-up travel sums the gaps between consecutive paths across layers, starting from the drawing rect's top-left corner. The estimate is `pen_down_mm / draw_mm_s + pen_up_mm / travel_mm_s + path_count * pen_lift_s`.
- `preview_png(width_px=1000, travel=False, source=False) -> bytes`. Draws the page in white with the drawing rect outlined in light grey, the tone map at 30 percent opacity when `source` is on, each layer in its pen colour at the true pen width with round caps, a one-vertex path as a filled circle of the pen width, and pen-up travel as thin red lines when `travel` is on. Renders at 4 times the size and reduces with a box filter.
- `save_preset(path)`, `load_preset(path)`.

Preset format:

```json
{"format": "plotline-preset", "version": 1, "style": "grid",
 "canvas": {...}, "pens": [...],
 "settings": {"tone": {...}, "points": {...}, "render": {}, "order": {...}}}
```

Loading checks `format`, runs a migration chain from the file's `version` to the current one, sets the style, canvas and pens, then validates settings per stage. An unknown stage or key produces a warning and is dropped. A missing key takes the default.

## 12. CLI

`cli/main.py` with click. Every command calls the session; no algorithm code.

```
plotline styles
plotline describe STYLE [--json]
plotline run STYLE IMAGE --out OUT.svg
    [--json OUT.json] [--preview OUT.png]
    [--width MM] [--height MM] [--margin MM] [--fit fit|fill]
    [--pen NAME:WIDTH_MM:COLOR]...
    [--set STAGE.KEY=VALUE]...
    [--preset FILE] [--save-preset FILE]
    [--quiet]
plotline gui
plotline mcp
```

`describe` prints each stage's settings with type, range, default and help, or the raw schema with `--json`. `run` applies the preset first, then explicit options, then `--set` values, validates, runs blocking, writes the requested outputs and prints stats. Progress labels go to stderr unless `--quiet`. `gui` and `mcp` print that they are not available yet and exit with status 1.

## 13. Testing

`tests/synthetic.py` generates float32 RGB images in code: a horizontal gradient, a checkerboard, a radial falloff and seeded uniform noise, each at a requested size. No image files are committed except tiny golden JSON outputs.

Tests:

- Unit tests per module: canvas rects and transforms for both fit modes, tone map sampling, each tone adjustment in isolation, setting helper and schema extras, style validation errors, cache get, put, eviction and oversize values, SVG structure parsed with ElementTree, JSON round trip.
- Determinism: every registered style runs twice on a 64 by 48 synthetic image and the results compare bitwise.
- Golden: `grid` on the gradient and the checkerboard at 64 by 48, stored as JSON in `tests/golden/`, compared with a tolerance of 0.001 mm. `pytest --update-golden` rewrites them.
- Cache: a spy stage counts its runs. Changing an `order` setting reruns only `order`. Changing a `tone` setting reruns everything. Rerunning unchanged settings runs nothing.
- Cancellation: a stage that yields 100 frames is cancelled from another thread after the third frame. The stage stops, `state` is `cancelled`, nothing from that stage is cached, and earlier stages remain cached.
- Session: subscribe receives frames with throttling, final frames always arrive, `run()` cancels a previous run, errors set `state` to `error`.
- Presets: save and load round trip, unknown keys warn and drop, version migration is invoked.
- CLI: click's runner covers `styles`, `describe`, `run` with each output, `--set` validation errors, and the `gui` and `mcp` stubs.

## 14. CI and repository hygiene

`.github/workflows/ci.yml` runs on push and pull request with a matrix of Windows, macOS and Linux against Python 3.11, 3.12 and 3.13:

1. `uv sync`
2. `ruff check` and `ruff format --check`
3. `mypy`
4. `pytest`
5. `python scripts/check_licenses.py`

The licence check reads installed distribution metadata for every runtime dependency and fails on any licence outside an allow-list of MIT, BSD, Apache-2.0, ISC, HPND, PSF and Python-2.0 variants. Packages whose metadata is non-standard are listed in the script with their verified licence and the URL where it was checked. It is written against the standard library only.

Repository rules from the brief apply: no third-party images, no personal paths, no naming of other tools, every algorithm cites its source in the module docstring.

## 15. Decisions recorded in the brief

This spec settles brief open questions 2, 5, 6, 9 and 10 for the core, and records the choices for 1, 3, 4, 7 and 8 made during the design discussion so later specs inherit them. The decision log in the brief carries each with its reason. Three divergences from the brief are recorded there too: declared artifact kinds instead of linear stage input, `fit` and `fill` only for the canvas, and the inclusion of ordering, preview, stats and presets in this first sub-project.
