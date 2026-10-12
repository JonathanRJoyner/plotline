# plotline Core Engine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the plotline package scaffold and core engine: core types, declared settings, the staged cached pipeline, the `grid` style, SVG and JSON export, a session layer with threaded runs, preview, stats and presets, and a CLI, with tests and CI.

**Architecture:** Three packages under `src/plotline` that depend only downward. `core` is a pure library of types, stages and a cached generator pipeline. `session` owns state, the worker thread, files and preview rendering. `cli` only calls `session`. Stages declare the artifact kinds they consume and produce, and the runner caches each stage's output under a content hash of its name, version, settings and input keys.

**Tech Stack:** Python 3.11+, numpy, scipy, Pillow, pydantic 2, click; uv, hatchling, ruff, mypy, pytest; GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-11-core-engine-design.md`

## Global Constraints

- Python `>=3.11`. Runtime dependencies are exactly numpy, scipy, Pillow, pydantic 2 and click. The optional extra `fast` declares numba, and nothing imports it.
- `plotline.core` imports nothing from `session` or `cli` and does no file, thread or network I/O. `plotline.cli` calls `session`, constructs `core.types` data classes and reads `core.registry` to list and describe styles; nothing else from `core`.
- Public units are millimetres. Pixels appear only inside the tone stage and the preview rasterizer.
- Every module that implements an algorithm names its published source in the module docstring. No code is copied from other projects.
- No other tools or products are named or compared against in code, docs or commit messages.
- No image files are committed. Tests generate images in code.
- Arrays are annotated with `NDArray[...]` from `numpy.typing`, never bare `np.ndarray`, because `plotline.core` is checked with mypy strict. Wrap scipy results in `np.asarray(..., dtype=...)` so strict mode does not see `Any`.
- Every setting is declared with `setting()` and has one-sentence help text.
- Same inputs and settings give bitwise-identical output. Randomness only through `ctx.rng(seed)`.
- Before every commit, these pass from the repo root: `uv run ruff format .`, `uv run ruff check .`, `uv run mypy src`, `uv run pytest -q`. This set is called "the verification set" below.
- Commit messages are plain imperative sentences like the existing history and end with the trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`, passed as a second `-m`.

## Review Focus

1. A photo carrying an EXIF orientation tag loads rotated as the camera intended, not sideways. Test in Task 15.
2. A PNG with transparency loads with transparent pixels as white, not as full ink. Test in Task 15.
3. A margin of half the page or more, or a non-positive page size, fails with a clear `ValueError` at `Canvas` construction and as a CLI error, not as a crash inside numpy. Tests in Tasks 3 and 20.
4. An all-white image yields zero paths and still exports valid SVG and JSON, renders a preview and reports zero stats. Tests in Tasks 11, 12, 13, 17 and 18.
5. An image with an extreme aspect ratio still produces a tone map at least one pixel on its short edge. Test in Task 9.

---

### Task 1: Scaffold

**Files:**
- Create: `pyproject.toml`, `uv.lock` (generated), `.python-version`, `.gitattributes`, `src/plotline/__init__.py`, `src/plotline/core/__init__.py`, `src/plotline/core/stages/__init__.py`, `src/plotline/core/export/__init__.py`, `src/plotline/session/__init__.py`, `src/plotline/cli/__init__.py`
- Test: `tests/test_version.py`

**Interfaces:**
- Produces: `plotline.__version__ == "0.1.0.dev0"`; the verification set runs.

- [x] **Step 1: Write the failing test**

```python
import plotline

def test_version_is_dev_zero_one():
    assert plotline.__version__ == "0.1.0.dev0"
```

- [x] **Step 2: Write `pyproject.toml` exactly as follows**

```toml
[project]
name = "plotline"
version = "0.1.0.dev0"
description = "Turn images into drawings for pen plotters."
readme = "README.md"
license = "MIT"
license-files = ["LICENSE"]
requires-python = ">=3.11"
dependencies = [
  "numpy>=1.26",
  "scipy>=1.11",
  "pillow>=10.1",
  "pydantic>=2.6",
  "click>=8.2",
]

[project.optional-dependencies]
fast = ["numba>=0.59"]

[dependency-groups]
dev = ["pytest>=8.0", "ruff>=0.6", "mypy>=1.11"]

[build-system]
requires = ["hatchling>=1.27"]
build-backend = "hatchling.build"

[tool.hatch.build.targets.wheel]
packages = ["src/plotline"]

[tool.ruff]
line-length = 100
target-version = "py311"

[tool.ruff.lint]
select = ["E", "W", "F", "I", "UP", "B"]

[tool.ruff.lint.per-file-ignores]
"tests/*" = ["E501"]

[tool.mypy]
python_version = "3.11"
warn_unused_ignores = true
warn_redundant_casts = true

[[tool.mypy.overrides]]
module = "plotline.core.*"
check_untyped_defs = true
disallow_any_generics = true
disallow_incomplete_defs = true
disallow_subclassing_any = true
disallow_untyped_calls = true
disallow_untyped_decorators = true
disallow_untyped_defs = true
extra_checks = true
implicit_reexport = false
strict_equality = true
warn_return_any = true

[[tool.mypy.overrides]]
module = ["scipy", "scipy.*"]
ignore_missing_imports = true

[tool.pytest.ini_options]
testpaths = ["tests"]
pythonpath = ["tests"]
```

- [x] **Step 3: Write the other scaffold files**

`.python-version` contains `3.11`. `.gitattributes` contains `* text=auto eol=lf`. `src/plotline/__init__.py` contains `__version__ = "0.1.0.dev0"`. The other `__init__.py` files are empty.

- [x] **Step 4: Create the environment and lock file**

Run: `uv sync`
Expected: `uv.lock` is created, numpy, scipy, pillow, pydantic, click, pytest, ruff and mypy are installed, and a Python 3.11 interpreter is downloaded if none is present.

- [x] **Step 5: Run the verification set**

Expected: `1 passed`; ruff and mypy report nothing.

- [x] **Step 6: Commit**

```bash
git add pyproject.toml uv.lock .python-version .gitattributes src tests
git commit -m "Scaffold the package with uv, ruff, mypy and pytest" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 2: Licence check and CI

**Files:**
- Create: `scripts/check_licenses.py`, `.github/workflows/ci.yml`
- Test: `tests/test_check_licenses.py`

**Interfaces:**
- Produces: in `scripts/check_licenses.py`: `ALLOWED: tuple[str, ...]`, `DENIED: tuple[str, ...]`, `OVERRIDES: dict[str, tuple[str, str]]` mapping a normalised package name to `(licence, url_checked)`; `license_text(dist: importlib.metadata.Distribution) -> str`; `is_allowed(text: str) -> bool`; `runtime_closure(root: str = "plotline") -> list[str]`; `main() -> int`.

- [x] **Step 1: Write the failing tests**

```python
import sys
sys.path.insert(0, "scripts")
import check_licenses as cl

def test_allows_permissive_texts():
    for t in ["MIT", "License :: OSI Approved :: BSD License", "Apache-2.0", "MIT-CMU",
              "Historical Permission Notice and Disclaimer (HPND)", "PSF-2.0", "ISC"]:
        assert cl.is_allowed(t), t

def test_denies_copyleft_and_empty():
    for t in ["GPL-3.0-only", "LGPL-2.1", "MIT OR LGPL-3.0", "MPL-2.0", ""]:
        assert not cl.is_allowed(t), t

def test_closure_is_runtime_only():
    names = cl.runtime_closure()
    assert "numpy" in names and "pydantic" in names and "click" in names
    assert "pytest" not in names and "numba" not in names and "plotline" not in names

def test_environment_passes():
    assert cl.main() == 0

def test_tokens_match_whole_words_only():
    assert cl.is_allowed("THIS SOFTWARE IS PROVIDED WITHOUT IMPLIED WARRANTIES. License :: OSI Approved :: BSD License")
    assert not cl.is_allowed("PERMITTED USE ONLY")

def test_glued_copyleft_spellings_are_denied():
    for t in ["MIT OR LGPLv3", "MIT/GPLv2", "License :: OSI Approved :: GNU General Public License v3 (GPLv3)"]:
        assert not cl.is_allowed(t), t
    assert cl.is_allowed("THIS SOFTWARE IS PROVIDED WITHOUT IMPLIED WARRANTIES. BSD-3-Clause")
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_check_licenses.py -v`
Expected: FAIL with `ModuleNotFoundError: No module named 'check_licenses'`

- [x] **Step 3: Implement `scripts/check_licenses.py` with the standard library only**

`ALLOWED = ("MIT", "BSD", "Apache", "ISC", "HPND", "Historical Permission", "PSF", "Python Software Foundation", "Python-2.0")`. `DENIED = ("GPL", "LGPL", "AGPL", "MPL", "EPL", "CDDL", "SSPL")`. `OVERRIDES = {}`.

`license_text` joins the `License-Expression` field, the `License` field and every `Classifier` starting with `License ::`. `is_allowed` is true when some `ALLOWED` token appears as a whole word and no `DENIED` token appears at a word start, case-sensitive. `runtime_closure` walks `Requires-Dist` breadth-first from `root`, skipping requirements whose marker contains `extra ==`, skipping names that raise `PackageNotFoundError` (platform-only dependencies), normalising names with `re.sub(r"[-_.]+", "-", name).lower()`, and returns the sorted names without `root`. `main` prints one line per package (`name  licence  OK|DENIED`), applies `OVERRIDES` by name, prints the offenders, and returns `1` when any exist.

- [x] **Step 4: Write `.github/workflows/ci.yml` exactly as follows**

```yaml
name: CI

on:
  push:
  pull_request:

jobs:
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
        python: ["3.11", "3.12", "3.13"]
    steps:
      - uses: actions/checkout@v4
      - uses: astral-sh/setup-uv@v5
        with:
          python-version: ${{ matrix.python }}
      - run: uv sync --locked
      - run: uv run ruff check .
      - run: uv run ruff format --check .
      - run: uv run mypy src
      - run: uv run pytest -q
      - run: uv run python scripts/check_licenses.py
```

- [x] **Step 5: Verify**

Run: `uv run pytest tests/test_check_licenses.py -v`
Expected: 6 passed.
Run: `uv run --with pyyaml python -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml')); print('ok')"`
Expected: `ok`.
Run the verification set.

- [x] **Step 6: Commit**

```bash
git add scripts tests/test_check_licenses.py .github
git commit -m "Add the dependency licence check and the CI workflow" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 3: Core types

**Files:**
- Create: `src/plotline/core/types.py`
- Test: `tests/test_types.py`

**Interfaces:**
- Produces, all in `plotline.core.types`:
  - `@dataclass(frozen=True) class Pen: name: str; width_mm: float; color: str`. `__post_init__` raises `ValueError` unless `width_mm > 0` and `color` matches `^#[0-9a-fA-F]{6}$`.
  - `@dataclass(frozen=True) class Canvas: width_mm: float; height_mm: float; margin_mm: float = 0.0; fit: Literal["fit", "fill"] = "fit"`. `__post_init__` raises `ValueError` unless both sizes are positive, `margin_mm >= 0` and `2 * margin_mm < min(width_mm, height_mm)`; each message names the offending field, for example `margin_mm must leave a positive drawing area`. `drawing_rect -> tuple[float, float, float, float]` property. `image_rect(width_px: int, height_px: int) -> tuple[float, float, float, float]`. `image_transform(width_px: int, height_px: int) -> tuple[float, tuple[float, float]]` returning `(mm_per_px, origin_mm)` where `mm_per_px = rect_w / width_px` and the origin is the rect's top-left.
  - `@dataclass(frozen=True, eq=False) class ToneMap: ink: NDArray[np.float32]; mm_per_px: float; origin_mm: tuple[float, float]`. `rect_mm` property; `to_mm(px: NDArray[np.float64]) -> NDArray[np.float64]`; `to_px(mm: NDArray[np.float64]) -> NDArray[np.float64]`; `sample(points_mm: NDArray[np.float64]) -> NDArray[np.float32]`. Pixel centres sit at half-integer pixel coordinates; `sample` is bilinear at those coordinates and clamps to the edge pixels.
  - `@dataclass(frozen=True, eq=False) class Layer: pen: Pen; paths: list[NDArray[np.float64]]`.
  - `@dataclass(frozen=True, eq=False) class Result: canvas: Canvas; layers: list[Layer]; tone: ToneMap | None = None`.
- `eq=False` on the array-holding classes, because array equality is ambiguous.

- [x] **Step 1: Write the failing tests**

```python
def test_drawing_rect_and_fit():
    c = Canvas(210, 297, 10)
    assert c.drawing_rect == (10, 10, 190, 277)
    assert c.image_rect(1000, 500) == pytest.approx((10, 101, 190, 95))
    assert c.image_transform(1000, 500) == pytest.approx((0.19, (10, 101)))

def test_fill_covers_drawing_rect():
    c = Canvas(100, 50, 0, "fill")
    assert c.image_rect(64, 64) == (0, 0, 100, 50)
    assert c.image_transform(64, 32) == pytest.approx((1.5625, (0, 0)))

def test_canvas_rejects_bad_dimensions():
    for args in [(0, 10), (100, -1), (100, 100, 50), (100, 100, -1)]:
        with pytest.raises(ValueError):
            Canvas(*args)

def test_pen_validation():
    with pytest.raises(ValueError):
        Pen("x", 0.3, "000000")
    with pytest.raises(ValueError):
        Pen("x", 0, "#000000")

def test_tone_map_geometry_and_sampling():
    t = ToneMap(np.array([[0, 1], [0, 1]], np.float32), 10.0, (5.0, 5.0))
    assert t.rect_mm == (5, 5, 20, 20)
    assert t.to_px(np.array([[10.0, 10.0]])) == pytest.approx([[0.5, 0.5]])
    p = np.array([[12.0, 17.0]])
    assert t.to_mm(t.to_px(p)) == pytest.approx(p)
    s = t.sample(np.array([[10, 10], [20, 10], [15, 10], [100, 100], [-50, -50]], float))
    assert s.dtype == np.float32
    assert s == pytest.approx([0, 1, 0.5, 1, 0])
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_types.py -v`
Expected: FAIL with `ImportError` on `plotline.core.types`.

- [x] **Step 3: Implement `types.py` to the interfaces above**

`fit`: scale is `min(rect_w / width_px, rect_h / height_px)`, the image is centred in the drawing rect. `fill`: the rect is the drawing rect.

- [x] **Step 4: Run the tests and the verification set**

Run: `uv run pytest tests/test_types.py -v`
Expected: 5 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/types.py tests/test_types.py
git commit -m "Add the core types: pen, canvas, tone map, layer and result" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 4: Settings

**Files:**
- Create: `src/plotline/core/settings.py`
- Test: `tests/test_settings.py`

**Interfaces:**
- Produces: `class Settings(pydantic.BaseModel)` with `model_config = ConfigDict(frozen=True, extra="forbid")`; `def setting(default: Any, *, help: str, min: float | None = None, max: float | None = None, step: float | None = None, unit: str | None = None, advanced: bool = False, label: str | None = None) -> Any`, returning `pydantic.Field(default, description=help, ge=min, le=max, json_schema_extra=extra)` where `extra` always holds `advanced` and holds `step`, `unit` and `label` only when given.

- [x] **Step 1: Write the failing tests**

```python
class M(Settings):
    x: float = setting(2.0, help="Spacing.", min=0.2, max=20, step=0.1, unit="mm", advanced=True)
    y: int = setting(3, help="Count.")

def test_schema_carries_extras():
    p = M.model_json_schema()["properties"]["x"]
    assert (p["description"], p["minimum"], p["maximum"], p["default"]) == ("Spacing.", 0.2, 20, 2.0)
    assert (p["step"], p["unit"], p["advanced"]) == (0.1, "mm", True)

def test_plain_setting_is_not_advanced_and_has_no_step():
    p = M.model_json_schema()["properties"]["y"]
    assert p["advanced"] is False and "step" not in p

def test_unknown_key_rejected():
    with pytest.raises(ValidationError):
        M(z=1)

def test_frozen():
    with pytest.raises(ValidationError):
        M().x = 1.0

def test_lax_coercion_and_range():
    assert M.model_validate({"x": "0.5"}).x == 0.5
    with pytest.raises(ValidationError):
        M(x=100)
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_settings.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `settings.py`**

- [x] **Step 4: Run the tests and the verification set**

Expected: 5 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/settings.py tests/test_settings.py
git commit -m "Add the settings base model and the setting helper" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 5: Stage contract

**Files:**
- Create: `src/plotline/core/stage.py`, `tests/doubles.py`
- Test: `tests/test_stage.py`

**Interfaces:**
- Produces, in `plotline.core.stage`:
  - `@dataclass(frozen=True, eq=False) class Frame: progress: float; label: str; points: NDArray[np.float64] | None = None; paths: list[NDArray[np.float64]] | None = None; stage: str = ""`.
  - `class Cancelled(Exception)`.
  - `class CancelToken` with `set() -> None` and `is_set() -> bool`, backed by `threading.Event`.
  - `class RunContext` with `__init__(self, token: CancelToken)`, `check() -> None` raising `Cancelled` when the token is set, and `rng(seed: int) -> np.random.Generator` returning `np.random.default_rng(seed)`.
  - `class Stage(ABC)` with class-level annotations `name: str`, `label: str`, `version: str = "1"`, `requires: tuple[str, ...]`, `provides: str`, `settings_model: type[Settings]`, and `@abstractmethod run(self, inputs: Mapping[str, Any], settings: Settings, ctx: RunContext) -> Generator[Frame, None, Any]`.
- Produces, in `tests/doubles.py`: `collect(gen) -> tuple[list[Frame], Any]`, which drains a stage generator and returns its frames and its return value (from `StopIteration.value`).

- [x] **Step 1: Write the failing tests**

```python
class Two(Stage):
    name, label, requires, provides, settings_model = "two", "Two", (), "n", Settings
    def run(self, inputs, settings, ctx):
        yield Frame(0.5, "half")
        yield Frame(1.0, "done")
        return 2

def test_stage_yields_frames_then_returns_artifact():
    frames, value = collect(Two().run({}, Settings(), RunContext(CancelToken())))
    assert [f.label for f in frames] == ["half", "done"] and value == 2
    assert frames[0].points is None and frames[0].paths is None and frames[0].stage == ""

def test_check_raises_only_when_cancelled():
    token = CancelToken(); ctx = RunContext(token)
    ctx.check()
    token.set()
    assert token.is_set()
    with pytest.raises(Cancelled):
        ctx.check()

def test_rng_is_seeded():
    ctx = RunContext(CancelToken())
    assert ctx.rng(7).random() == ctx.rng(7).random()
    assert ctx.rng(7).random() != ctx.rng(8).random()

def test_stage_is_abstract():
    with pytest.raises(TypeError):
        Stage()
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_stage.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `stage.py` and `collect` in `tests/doubles.py`**

- [x] **Step 4: Run the tests and the verification set**

Expected: 4 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/stage.py tests/doubles.py tests/test_stage.py
git commit -m "Add the stage contract: frames, cancellation and run context" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 6: Style and artifact resolution

**Files:**
- Create: `src/plotline/core/pipeline.py`
- Modify: `tests/doubles.py`
- Test: `tests/test_style.py`

**Interfaces:**
- Produces, in `plotline.core.pipeline`:
  - `ROOT_KINDS: frozenset[str] = frozenset({"image", "canvas", "pens"})`.
  - `@dataclass(frozen=True) class Style: name: str; label: str; description: str; stages: tuple[Stage, ...]; sources: dict[str, dict[str, str]] = field(init=False, repr=False, compare=False)`. `__post_init__` fills `sources[stage.name][kind]` with either the root kind name or the name of the nearest earlier stage providing that kind, and raises `ValueError` when stage names repeat, a stage name equals a root kind, a requirement cannot be resolved, or the last stage does not provide `paths` or `layers`.
  - `Style.schema() -> dict[str, Any]` returning `{"name", "label", "description", "stages": [{"name", "label", "schema": stage.settings_model.model_json_schema()}, ...]}`.
  - `Style.defaults() -> dict[str, Settings]` returning `{stage.name: stage.settings_model()}`.
- Produces, in `tests/doubles.py`:
  - `class SpySettings(Settings): knob: int = setting(0, help="Test knob.")`.
  - `class Spy(Stage)` with `settings_model = SpySettings` and `__init__(self, name: str, requires: tuple[str, ...], provides: str, value: Any = None, frames: int = 1, delay_s: float = 0.0, fail: Exception | None = None)`. `label` is `name.title()`. `runs: int` counts calls to `run`. `run` sleeps `delay_s` before each of `frames` frames, yields `Frame((i + 1) / frames, f"{i + 1}/{frames}")`, raises `fail` after the first frame when given, and returns `value(inputs)` when `value` is callable, else `value`.

- [x] **Step 1: Write the failing tests**

```python
def spy(name, req, prov): return Spy(name, req, prov)

def test_resolves_nearest_upstream_provider():
    st = Style("t", "T", "d", (spy("a", ("image",), "paths"), spy("b", ("paths",), "paths"), spy("c", ("paths", "canvas"), "paths")))
    assert st.sources == {"a": {"image": "image"}, "b": {"paths": "a"}, "c": {"paths": "b", "canvas": "canvas"}}

@pytest.mark.parametrize("stages", [
    (spy("a", ("image",), "paths"), spy("a", ("paths",), "paths")),      # duplicate name
    (spy("canvas", ("image",), "paths"),),                                # root kind as name
    (spy("a", ("points",), "paths"),),                                    # unresolved
    (spy("a", ("image",), "points"),),                                    # wrong final kind
])
def test_invalid_styles_are_rejected(stages):
    with pytest.raises(ValueError):
        Style("t", "T", "d", stages)

def test_schema_and_defaults():
    st = Style("t", "T", "d", (spy("a", ("image",), "paths"),))
    sch = st.schema()
    assert sch["name"] == "t" and sch["stages"][0]["name"] == "a"
    assert "knob" in sch["stages"][0]["schema"]["properties"]
    assert st.defaults() == {"a": SpySettings()}
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_style.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `Style` in `pipeline.py` and `Spy` in `tests/doubles.py`**

Use `object.__setattr__(self, "sources", ...)` inside `__post_init__` because the dataclass is frozen.

- [x] **Step 4: Run the tests and the verification set**

Expected: 6 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/pipeline.py tests/doubles.py tests/test_style.py
git commit -m "Add styles with declared artifact kinds and nearest-provider resolution" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 7: Stage cache

**Files:**
- Create: `src/plotline/core/cache.py`
- Test: `tests/test_cache.py`

**Interfaces:**
- Produces, in `plotline.core.cache`:
  - `estimate_bytes(value: Any) -> int`: `nbytes` for arrays, the sum over items for lists and tuples, the sum over fields for dataclass instances, `sys.getsizeof` otherwise.
  - `class StageCache` with `__init__(self, max_bytes: int = 512 * 2**20)`, `get(key: str) -> Any | None` (counts a hit or a miss and marks the key most recently used), `put(key: str, value: Any) -> None` (skips values larger than `max_bytes`, evicts least recently used until under budget), `__contains__(key: str) -> bool`, `__len__() -> int`, `clear() -> None`, and attributes `hits: int`, `misses: int`, `bytes: int`.

- [x] **Step 1: Write the failing tests**

```python
def test_miss_then_hit():
    c = StageCache()
    assert c.get("k") is None and c.misses == 1
    c.put("k", np.zeros(10))
    assert c.get("k") is not None and c.hits == 1 and "k" in c and len(c) == 1 and c.bytes == 80

def test_lru_eviction():
    c = StageCache(max_bytes=200)
    c.put("a", np.zeros(10)); c.put("b", np.zeros(10)); c.get("a"); c.put("c", np.zeros(10))
    assert "b" not in c and "a" in c and "c" in c and c.bytes == 160

def test_oversize_value_is_not_stored():
    c = StageCache(max_bytes=50)
    c.put("big", np.zeros(10))
    assert "big" not in c and c.bytes == 0

def test_estimate_bytes():
    assert estimate_bytes([np.zeros(10), np.zeros(10)]) == 160
    assert estimate_bytes(Layer(Pen("p", 0.3, "#000000"), [np.zeros((3, 2))])) >= 48
    assert estimate_bytes("abc") == sys.getsizeof("abc")

def test_clear():
    c = StageCache(); c.put("k", np.zeros(10)); c.clear()
    assert len(c) == 0 and c.bytes == 0
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_cache.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `cache.py`** with an `OrderedDict` for recency.

- [x] **Step 4: Run the tests and the verification set**

Expected: 5 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/cache.py tests/test_cache.py
git commit -m "Add the in-memory stage cache with byte-bounded LRU eviction" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 8: Runner

**Files:**
- Modify: `src/plotline/core/pipeline.py`
- Test: `tests/test_runner.py`

**Interfaces:**
- Consumes: `Style`, `Stage`, `Frame`, `RunContext`, `Cancelled`, `StageCache`, `Settings`, `Canvas`, `Pen`, `Layer`, `Result`, `ToneMap`.
- Produces, in `plotline.core.pipeline`:
  - `canonical_json(obj: Any) -> str`: `json.dumps(obj, sort_keys=True, separators=(",", ":"))`.
  - `root_key(kind: str, value: Any) -> str`: for `image`, the BLAKE2b (digest size 16) hex of the dtype string, the shape string and the contiguous array bytes; for `canvas` and `pens`, the BLAKE2b of the canonical JSON of `dataclasses.asdict` (a list of dicts for pens).
  - `stage_key(stage: Stage, settings: Settings, input_keys: dict[str, str]) -> str`: BLAKE2b hex of the canonical JSON of `{"stage": f"{type(stage).__module__}.{type(stage).__qualname__}", "name": stage.name, "version": stage.version, "settings": settings.model_dump(mode="json"), "inputs": input_keys}`.
  - `run_style(style: Style, settings: Mapping[str, Settings], roots: Mapping[str, Any], cache: StageCache, ctx: RunContext) -> Generator[Frame, None, Result]`. Raises `KeyError` when a root kind or a stage's settings are missing. For each stage: compute the key; on a cache hit yield `Frame(1.0, "cached", stage=stage.name)`; otherwise drive `stage.run`, re-yield each frame with `stage` set via `dataclasses.replace`, call `ctx.check()` after each frame, and `cache.put` the returned artifact. Builds the `Result` from the last stage's artifact (`layers` used directly, `paths` wrapped in `Layer(roots["pens"][0], paths)`) and sets `tone` to the artifact of the last stage providing `tone`, if any.

- [x] **Step 1: Write the failing tests**

```python
ROOTS = {"image": np.zeros((4, 4, 3), np.float32), "canvas": Canvas(100, 100), "pens": (Pen("p", 0.3, "#000000"),)}
PATHS = [np.array([[1.0, 1.0], [2.0, 2.0]])]

def make(frames_b=1):
    a = Spy("a", ("image",), "points", value=np.zeros((3, 2)))
    b = Spy("b", ("points",), "paths", value=PATHS, frames=frames_b)
    return Style("t", "T", "d", (a, b)), a, b

def run(style, settings, cache, roots=ROOTS, token=None):
    return collect(run_style(style, settings, roots, cache, RunContext(token or CancelToken())))

def test_runs_every_stage_and_wraps_paths_in_a_layer():
    style, a, b = make(); frames, result = run(style, style.defaults(), StageCache())
    assert (a.runs, b.runs) == (1, 1) and [f.stage for f in frames] == ["a", "b"]
    assert result.layers[0].pen == ROOTS["pens"][0] and result.layers[0].paths == PATHS
    assert result.canvas == ROOTS["canvas"] and result.tone is None

def test_second_run_is_served_from_cache():
    style, a, b = make(); cache = StageCache(); run(style, style.defaults(), cache)
    frames, _ = run(style, style.defaults(), cache)
    assert (a.runs, b.runs) == (1, 1) and [f.label for f in frames] == ["cached", "cached"]

def test_late_change_reruns_late_stages_only():
    style, a, b = make(); cache = StageCache(); run(style, style.defaults(), cache)
    run(style, {**style.defaults(), "b": SpySettings(knob=1)}, cache)
    assert (a.runs, b.runs) == (1, 2)

def test_early_change_and_root_change_rerun_everything():
    style, a, b = make(); cache = StageCache(); run(style, style.defaults(), cache)
    run(style, {**style.defaults(), "a": SpySettings(knob=1)}, cache)
    run(style, style.defaults(), cache, roots={**ROOTS, "image": np.ones((4, 4, 3), np.float32)})
    assert (a.runs, b.runs) == (3, 3)

def test_equivalent_stage_in_another_style_hits_the_cache():
    style, a, b = make(); cache = StageCache(); run(style, style.defaults(), cache)
    a2 = Spy("a", ("image",), "points", value=np.zeros((3, 2)))
    other = Style("u", "U", "d", (a2, Spy("b2", ("points",), "paths", value=PATHS)))
    run(other, other.defaults(), cache)
    assert a2.runs == 0

def test_cancel_discards_partial_output_and_keeps_earlier_stages():
    style, a, b = make(frames_b=5); cache = StageCache(); token = CancelToken()
    gen = run_style(style, style.defaults(), ROOTS, cache, RunContext(token))
    seen = 0
    with pytest.raises(Cancelled):
        for frame in gen:
            if frame.stage == "b":
                seen += 1
                if seen == 2:
                    token.set()
    assert len(cache) == 1 and seen == 2

def test_layers_provider_and_tone_are_used_directly():
    tone = ToneMap(np.zeros((2, 2), np.float32), 1.0, (0.0, 0.0))
    layers = [Layer(Pen("q", 0.5, "#ff0000"), PATHS)]
    style = Style("t", "T", "d", (Spy("t", ("image",), "tone", value=tone), Spy("l", ("tone",), "layers", value=layers)))
    _, result = run(style, style.defaults(), StageCache())
    assert result.layers is layers and result.tone is tone

def test_missing_root_raises():
    style, _, _ = make()
    with pytest.raises(KeyError):
        run(style, style.defaults(), StageCache(), roots={"canvas": ROOTS["canvas"]})
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_runner.py -v`
Expected: FAIL with `ImportError: cannot import name 'run_style'`.

- [x] **Step 3: Implement `canonical_json`, `root_key`, `stage_key` and `run_style` in `pipeline.py`**

Keep a `keys: dict[str, str]` and an `artifacts: dict[str, Any]` indexed by source name (root kinds and stage names); a stage's `input_keys` is `{kind: keys[source] for kind, source in style.sources[stage.name].items()}`.

- [x] **Step 4: Run the tests and the verification set**

Expected: 8 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/pipeline.py tests/test_runner.py
git commit -m "Add the cached pipeline runner with content-hash keys" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 9: Tone stage and synthetic images

**Files:**
- Create: `src/plotline/core/stages/tone.py`, `tests/synthetic.py`
- Test: `tests/test_tone.py`

**Interfaces:**
- Produces, in `tests/synthetic.py`, each returning float32 RGB arrays of shape `(h, w, 3)` in 0..1: `gradient(w, h)` with luminance `x / (w - 1)` (black at the left edge, white at the right); `checkerboard(w, h, cells)` with square cells of side `w // cells` alternating 0 and 1; `radial(w, h)` with luminance equal to the distance from the centre over half the diagonal, clipped; `noise(w, h, seed)` from `np.random.default_rng(seed).random((h, w))`; `flat(w, h, value)`.
- Produces, in `plotline.core.stages.tone`:
  - `class ToneSettings(Settings)` with `resolution: int = setting(1000, min=64, max=4000, unit="px", ...)`, `invert: bool = setting(False, ...)`, `black: float = setting(0.0, min=0, max=1, step=0.01, ...)`, `white: float = setting(1.0, min=0, max=1, step=0.01, ...)`, `gamma: float = setting(1.0, min=0.1, max=5, step=0.05, ...)`, `contrast: float = setting(0.0, min=-1, max=1, step=0.05, ...)`, `blur_mm: float = setting(0.0, min=0, max=10, step=0.1, unit="mm", advanced=True, ...)`.
  - `class ToneStage(Stage)` with `name = "tone"`, `label = "Tone"`, `requires = ("image", "canvas")`, `provides = "tone"`, `settings_model = ToneSettings`. `run` narrows `settings` with an `isinstance` assert, yields one `Frame(1.0, "tone map")` and returns a `ToneMap`.
- Module docstring cites ITU-R BT.601 for the luminance weights.

- [x] **Step 1: Write the failing tests**

```python
def run_tone(image, canvas=Canvas(100, 100), **kw):
    kw.setdefault("resolution", 64)
    frames, tone = collect(ToneStage().run({"image": image, "canvas": canvas}, ToneSettings(**kw), RunContext(CancelToken())))
    return frames, tone

def test_shape_dtype_and_transform():
    frames, t = run_tone(gradient(64, 48))
    assert t.ink.shape == (48, 64) and t.ink.dtype == np.float32
    assert t.mm_per_px == pytest.approx(1.5625) and t.origin_mm == pytest.approx((0, 12.5))
    assert len(frames) == 1 and frames[0].progress == 1

def test_gradient_ink_falls_left_to_right():
    _, t = run_tone(gradient(64, 48))
    assert t.ink[12, 0] > 0.9 and t.ink[12, -1] < 0.1 and np.all(np.diff(t.ink[12]) <= 1e-6)

def test_invert_levels_gamma_contrast():
    _, a = run_tone(gradient(64, 48)); _, b = run_tone(gradient(64, 48), invert=True)
    assert np.allclose(a.ink + b.ink, 1, atol=1e-6)
    assert run_tone(gradient(64, 48), black=0.5)[1].ink[12, :14].min() > 0.99
    assert run_tone(flat(16, 16, 0.5), gamma=2.0)[1].ink == pytest.approx(0.75, abs=1e-6)
    assert run_tone(flat(16, 16, 0.75), contrast=1.0)[1].ink == pytest.approx(0.0, abs=1e-6)
    assert run_tone(flat(16, 16, 0.75), contrast=-1.0)[1].ink == pytest.approx(0.5, abs=1e-6)

def test_blur_lowers_variation():
    cb = checkerboard(64, 64, 8)
    assert run_tone(cb, blur_mm=3.0)[1].ink.std() < run_tone(cb)[1].ink.std()

def test_fill_crops_to_the_drawing_aspect():
    _, t = run_tone(flat(64, 64, 0.5), Canvas(100, 50, 0, "fill"))
    assert t.ink.shape == (32, 64) and t.rect_mm == pytest.approx((0, 0, 100, 50))

def test_short_edge_is_at_least_one_pixel():
    _, t = run_tone(flat(4000, 1, 0.5), resolution=100)
    assert t.ink.shape == (1, 100)

def test_downscaling_averages_rather_than_skips():
    _, t = run_tone(checkerboard(256, 256, 128))
    assert abs(float(t.ink.mean()) - 0.5) < 0.05 and float(t.ink.std()) < 0.1
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_tone.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `tests/synthetic.py` and `tone.py`**

Processing order in `run`: luminance `Y = 0.299 R + 0.587 G + 0.114 B` as float32; when `canvas.fit == "fill"`, centre-crop `Y` to the drawing rect's aspect ratio; target size has long edge `resolution` and short edge `max(1, round(short * resolution / long))`; when the source long edge is at least twice the target, block-mean by `k = long // resolution` (crop to a multiple of `k` first), then resample to the exact target shape with `scipy.ndimage.map_coordinates(order=1, mode="nearest")` at target pixel centres; levels `(Y - black) / (white - black)` clipped to 0..1; `Y ** gamma`; `clip(0.5 + (Y - 0.5) * (1 + contrast), 0, 1)`; `ink = 1 - Y`; invert as `1 - ink`; `mm_per_px, origin = canvas.image_transform(tw, th)`; when `blur_mm > 0`, `scipy.ndimage.gaussian_filter(ink, blur_mm / mm_per_px)`; clip and cast to float32.

- [x] **Step 4: Run the tests and the verification set**

Expected: 7 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/stages/tone.py tests/synthetic.py tests/test_tone.py
git commit -m "Add the tone map stage and synthetic test images" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 10: Grid placer and dot renderer

**Files:**
- Create: `src/plotline/core/stages/grid.py`
- Test: `tests/test_grid.py`

**Interfaces:**
- Produces, in `plotline.core.stages.grid`:
  - `class GridSettings(Settings)` with `spacing_mm: float = setting(2.0, min=0.2, max=20, step=0.1, unit="mm", ...)` and `threshold: float = setting(0.5, min=0, max=1, step=0.01, ...)`.
  - `class GridPlacer(Stage)` with `name = "points"`, `label = "Grid"`, `requires = ("tone",)`, `provides = "points"`, `settings_model = GridSettings`. Cell centres are at `x0 + spacing * (i + 0.5)` for `i < floor(w / spacing)` and likewise for rows, over `tone.rect_mm`. A centre is kept when `tone.sample` there is at least `threshold`. Yields one frame per row with `points` set to the points placed so far; returns a float64 array of shape `(N, 2)`, `(0, 2)` when empty.
  - `class DotSettings(Settings)` with no fields.
  - `class DotRenderer(Stage)` with `name = "render"`, `label = "Dots"`, `requires = ("points",)`, `provides = "paths"`, `settings_model = DotSettings`. Returns `[p.reshape(1, 2) for p in points]` and yields one final frame carrying those paths.

- [x] **Step 1: Write the failing tests**

```python
CTX = RunContext(CancelToken())
def tone(ink): return ToneMap(np.asarray(ink, np.float32), 1.0, (0.0, 0.0))

def test_full_grid():
    frames, pts = collect(GridPlacer().run({"tone": tone(np.ones((10, 20)))}, GridSettings(spacing_mm=2.0), CTX))
    assert pts.shape == (50, 2) and pts.dtype == np.float64 and pts[0] == pytest.approx([1, 1])
    assert len(frames) == 5 and frames[-1].progress == 1 and frames[-1].points.shape == (50, 2)

def test_threshold_keeps_dark_cells_only():
    ink = np.zeros((10, 20)); ink[:, 10:] = 1
    _, pts = collect(GridPlacer().run({"tone": tone(ink)}, GridSettings(spacing_mm=2.0), CTX))
    assert pts.shape == (25, 2) and np.all(pts[:, 0] > 10)

def test_white_tone_gives_no_points():
    _, pts = collect(GridPlacer().run({"tone": tone(np.zeros((10, 20)))}, GridSettings(), CTX))
    assert pts.shape == (0, 2)

def test_dots_are_one_vertex_paths():
    frames, paths = collect(DotRenderer().run({"points": np.array([[1.0, 2.0], [3.0, 4.0]])}, DotSettings(), CTX))
    assert [p.shape for p in paths] == [(1, 2), (1, 2)] and paths[1][0] == pytest.approx([3, 4])
    assert frames[-1].progress == 1 and len(frames[-1].paths) == 2
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_grid.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `grid.py`**

- [x] **Step 4: Run the tests and the verification set**

Expected: 4 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/stages/grid.py tests/test_grid.py
git commit -m "Add the grid placer and the dot renderer" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 11: Path helpers and the ordering stage

**Files:**
- Create: `src/plotline/core/paths.py`, `src/plotline/core/stages/order.py`
- Test: `tests/test_paths.py`, `tests/test_order.py`

**Interfaces:**
- Produces, in `plotline.core.paths`: `path_length(path: NDArray[np.float64]) -> float` (0 for one vertex); `total_length(paths: Iterable[NDArray[np.float64]]) -> float`; `travel_length(paths: Sequence[NDArray[np.float64]], start: tuple[float, float]) -> float`, the distance from `start` to the first path's first vertex plus each gap from a path's last vertex to the next path's first vertex.
- Produces, in `plotline.core.stages.order`:
  - `class OrderSettings(Settings)` with `optimize: bool = setting(True, help="Reorder paths to cut pen-up travel.")` and `min_length_mm: float = setting(0.0, min=0, max=10, step=0.1, unit="mm", help="Drop paths shorter than this. Dots have zero length, so any positive value removes them.")`.
  - `class OrderStage(Stage)` with `name = "order"`, `label = "Path order"`, `requires = ("paths", "canvas")`, `provides = "paths"`, `settings_model = OrderSettings`. Drops paths with `path_length < min_length_mm`. When `optimize` is on: start at the drawing rect's top-left corner, repeatedly take the unvisited path whose nearer endpoint is closest, reversing it when its last vertex is the nearer one. Yields a frame with the ordered paths so far every `max(1, n // 20)` paths and a final frame at progress 1. Returns `[]` for no paths without touching scipy.
- Module docstring cites the nearest-neighbour heuristic analysis by Rosenkrantz, Stearns and Lewis (1977).

- [x] **Step 1: Write the failing tests**

```python
def test_path_lengths():
    assert path_length(np.array([[0.0, 0.0], [3.0, 4.0]])) == 5
    assert path_length(np.array([[1.0, 1.0]])) == 0
    assert total_length([np.array([[0.0, 0.0], [3.0, 4.0]]), np.array([[0.0, 0.0], [0.0, 1.0]])]) == 6
    assert travel_length([np.array([[3.0, 4.0], [3.0, 5.0]]), np.array([[3.0, 8.0]])], (0.0, 0.0)) == 8
```

```python
CTX = RunContext(CancelToken()); CANVAS = Canvas(100, 100, 10)
def order(paths, **kw): return collect(OrderStage().run({"paths": paths, "canvas": CANVAS}, OrderSettings(**kw), CTX))

def test_greedy_order_and_reversal():
    p1 = np.array([[50.0, 50.0], [60.0, 50.0]]); p2 = np.array([[12.0, 10.0], [20.0, 10.0]]); p3 = np.array([[40.0, 10.0], [30.0, 10.0]])
    frames, out = order([p1, p2, p3])
    assert np.array_equal(out[0], p2) and np.array_equal(out[1], p3[::-1]) and np.array_equal(out[2], p1)
    assert frames[-1].progress == 1 and len(frames[-1].paths) == 3

def test_min_length_drops_short_paths_but_keeps_dots_at_zero():
    dot = np.array([[5.0, 5.0]]); short = np.array([[0.0, 0.0], [0.0, 0.5]])
    assert len(order([dot, short], min_length_mm=1.0)[1]) == 0
    assert len(order([dot, short])[1]) == 2

def test_optimize_off_keeps_order():
    paths = [np.array([[90.0, 90.0], [91.0, 90.0]]), np.array([[10.0, 10.0], [11.0, 10.0]])]
    out = order(paths, optimize=False)[1]
    assert np.array_equal(out[0], paths[0])

def test_ordering_never_increases_travel():
    rng = np.random.default_rng(1)
    paths = [np.stack([p, p + [1.0, 0.0]]) for p in rng.uniform(10, 90, (50, 2))]
    assert travel_length(order(paths)[1], (10, 10)) <= travel_length(paths, (10, 10))

def test_empty_input():
    assert order([])[1] == []
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_paths.py tests/test_order.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `paths.py` and `order.py`**

Greedy search: build a `scipy.spatial.cKDTree` over all `2n` endpoints (starts then ends), query the nearest `k` from the current position with `k` doubling until an unvisited endpoint appears, map index `j` to path `j % n` and reversal `j >= n`.

- [x] **Step 4: Run the tests and the verification set**

Expected: 6 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/paths.py src/plotline/core/stages/order.py tests/test_paths.py tests/test_order.py
git commit -m "Add path length helpers and the greedy path ordering stage" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 12: Registry, the grid style, integration and determinism

**Files:**
- Create: `src/plotline/core/styles.py`, `src/plotline/core/registry.py`
- Test: `tests/test_registry.py`, `tests/test_integration.py`, `tests/test_determinism.py`

**Interfaces:**
- Produces, in `plotline.core.styles`: `GRID: Style` with `name="grid"`, `label="Grid dots"`, a one-sentence description, and stages `(ToneStage(), GridPlacer(), DotRenderer(), OrderStage())`; `BUILTIN: tuple[Style, ...] = (GRID,)`.
- Produces, in `plotline.core.registry`: `ENTRY_POINT_GROUP = "plotline.styles"`; `_entry_points() -> list[importlib.metadata.EntryPoint]` wrapping `importlib.metadata.entry_points(group=ENTRY_POINT_GROUP)`; `list_styles() -> list[Style]` returning the built-ins followed by every entry point whose `load()` returns a `Style`, warning with `warnings.warn` and skipping anything else; `get_style(name: str) -> Style` raising `KeyError(f"unknown style {name!r}")`.

- [x] **Step 1: Write the failing tests**

```python
def test_builtin_grid_is_listed():
    assert [s.name for s in list_styles()] == ["grid"] and get_style("grid") is GRID

def test_unknown_style_raises():
    with pytest.raises(KeyError):
        get_style("nope")

def test_entry_points_are_discovered_and_bad_ones_skipped(monkeypatch):
    extra = Style("x", "X", "d", (ToneStage(), GridPlacer(), DotRenderer()))
    eps = [SimpleNamespace(name="x", load=lambda: extra), SimpleNamespace(name="bad", load=lambda: 42)]
    monkeypatch.setattr(registry, "_entry_points", lambda: eps)
    with pytest.warns(UserWarning):
        names = [s.name for s in list_styles()]
    assert names == ["grid", "x"]
```

```python
ROOTS = {"canvas": Canvas(100, 75, 5), "pens": (Pen("Black 0.3", 0.3, "#000000"),)}
def run(image, **tone):
    s = {**GRID.defaults(), "tone": ToneSettings(resolution=64, **tone)}
    return collect(run_style(GRID, s, {**ROOTS, "image": image}, StageCache(), RunContext(CancelToken())))

def test_grid_on_gradient():
    frames, result = run(gradient(64, 48))
    paths = result.layers[0].paths
    assert len(paths) > 100 and all(p.shape == (1, 2) for p in paths)
    x, y, w, h = ROOTS["canvas"].drawing_rect
    pts = np.concatenate(paths)
    assert pts[:, 0].min() >= x and pts[:, 0].max() <= x + w and pts[:, 1].min() >= y and pts[:, 1].max() <= y + h
    assert {f.stage for f in frames} == {"tone", "points", "render", "order"} and result.tone is not None

def test_grid_on_white_image_has_no_paths():
    _, result = run(flat(64, 48, 1.0))
    assert result.layers[0].paths == [] and result.layers[0].pen == ROOTS["pens"][0]
```

```python
@pytest.mark.parametrize("style", list_styles(), ids=lambda s: s.name)
def test_same_settings_give_identical_output(style):
    settings = style.defaults()
    if "tone" in settings:
        settings["tone"] = ToneSettings(resolution=64)
    roots = {"image": gradient(64, 48), "canvas": Canvas(100, 75, 5), "pens": (Pen("p", 0.3, "#000000"),)}
    outs = [collect(run_style(style, settings, roots, StageCache(), RunContext(CancelToken())))[1] for _ in range(2)]
    a, b = (o.layers[0].paths for o in outs)
    assert len(a) == len(b) and all(np.array_equal(p, q) for p, q in zip(a, b))
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_registry.py tests/test_integration.py tests/test_determinism.py -v`
Expected: FAIL with `ImportError`.

- [x] **Step 3: Implement `styles.py` and `registry.py`**

- [x] **Step 4: Run the tests and the verification set**

Expected: 6 passed.

- [x] **Step 5: Commit**

```bash
git add src/plotline/core/styles.py src/plotline/core/registry.py tests/test_registry.py tests/test_integration.py tests/test_determinism.py
git commit -m "Add the style registry, the grid style and determinism tests" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 13: SVG and JSON export

**Files:**
- Create: `src/plotline/core/export/svg.py`, `src/plotline/core/export/paths_json.py`
- Test: `tests/test_export.py`

**Interfaces:**
- Produces: `to_svg(result: Result) -> str` and `to_json_dict(result: Result) -> dict[str, Any]`, both pure. SVG: root `<svg xmlns="http://www.w3.org/2000/svg" width="{W}mm" height="{H}mm" viewBox="0 0 {W} {H}">` with `W` and `H` formatted with `:g`; one `<g id="pen-{i}" data-pen="{name}" fill="none" stroke="{color}" stroke-width="{width:g}" stroke-linecap="round" stroke-linejoin="round">` per layer, 1-based; one `<path d="M x y L x y ..."/>` per path with `:.3f` coordinates, and `M x y L x y` for a one-vertex path; the pen name is attribute-escaped. JSON: `{"format": "plotline-paths", "version": 1, "units": "mm", "canvas": {"width_mm", "height_mm", "margin_mm", "fit"}, "layers": [{"pen": {"name", "width_mm", "color"}, "paths": [[[x, y], ...], ...]}]}` with coordinates as floats rounded to 3 decimals.

- [ ] **Step 1: Write the failing tests**

```python
SVG = "{http://www.w3.org/2000/svg}"
def result(name="Black 0.3"):
    return Result(Canvas(210, 297, 10), [Layer(Pen(name, 0.3, "#000000"), [np.array([[10.0, 10.0], [20.0, 15.5]]), np.array([[30.0, 30.0]])])])

def test_svg_structure():
    root = ET.fromstring(to_svg(result()))
    assert root.tag == SVG + "svg" and (root.get("width"), root.get("height"), root.get("viewBox")) == ("210mm", "297mm", "0 0 210 297")
    g = root[0]
    assert {k: g.get(k) for k in ("id", "data-pen", "fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin")} == \
        {"id": "pen-1", "data-pen": "Black 0.3", "fill": "none", "stroke": "#000000", "stroke-width": "0.3", "stroke-linecap": "round", "stroke-linejoin": "round"}
    assert [p.get("d") for p in g] == ["M 10.000 10.000 L 20.000 15.500", "M 30.000 30.000 L 30.000 30.000"]

def test_svg_escapes_pen_name_and_handles_empty_layers():
    root = ET.fromstring(to_svg(result('a"b<c&d')))
    assert root[0].get("data-pen") == 'a"b<c&d'
    empty = ET.fromstring(to_svg(Result(Canvas(100, 100), [Layer(Pen("p", 0.3, "#000000"), [])])))
    assert len(empty[0]) == 0

def test_json_matches_spec_example():
    assert to_json_dict(result()) == {
        "format": "plotline-paths", "version": 1, "units": "mm",
        "canvas": {"width_mm": 210, "height_mm": 297, "margin_mm": 10, "fit": "fit"},
        "layers": [{"pen": {"name": "Black 0.3", "width_mm": 0.3, "color": "#000000"},
                    "paths": [[[10.0, 10.0], [20.0, 15.5]], [[30.0, 30.0]]]}]}

def test_json_rounds_to_three_decimals_and_serialises():
    r = Result(Canvas(100, 100), [Layer(Pen("p", 0.3, "#000000"), [np.array([[1.23456, 2.0]])])])
    d = to_json_dict(r)
    assert d["layers"][0]["paths"] == [[[1.235, 2.0]]] and json.dumps(d)
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_export.py -v`
Expected: FAIL with `ImportError`.

- [ ] **Step 3: Implement both exporters**

Escape the pen name with `xml.sax.saxutils.escape(name, {'"': "&quot;"})`.

- [ ] **Step 4: Run the tests and the verification set**

Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/plotline/core/export tests/test_export.py
git commit -m "Add SVG and JSON exporters" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 14: Golden tests

**Files:**
- Create: `tests/conftest.py`, `tests/golden/grid_gradient.json`, `tests/golden/grid_checkerboard.json` (generated)
- Test: `tests/test_golden.py`

**Interfaces:**
- Consumes: `run_style`, `GRID`, `to_json_dict`, `gradient`, `checkerboard`.
- Produces: a pytest option `--update-golden` that rewrites the golden files instead of comparing.

- [ ] **Step 1: Write `tests/conftest.py` and the failing test**

`conftest.py` adds the `--update-golden` boolean option with `pytest_addoption` and an `update_golden` fixture returning its value.

```python
CASES = {"grid_gradient": gradient(64, 48), "grid_checkerboard": checkerboard(64, 48, 8)}
GOLDEN = pathlib.Path(__file__).parent / "golden"

def produce(image):
    settings = {**GRID.defaults(), "tone": ToneSettings(resolution=64), "points": GridSettings(threshold=0.6)}
    roots = {"image": image, "canvas": Canvas(100, 75, 5), "pens": (Pen("Black 0.3", 0.3, "#000000"),)}
    return to_json_dict(collect(run_style(GRID, settings, roots, StageCache(), RunContext(CancelToken())))[1])

@pytest.mark.parametrize("name", CASES)
def test_golden(name, update_golden):
    actual = produce(CASES[name]); path = GOLDEN / f"{name}.json"
    if update_golden:
        path.write_text(json.dumps(actual, indent=0) + "\n"); return
    expected = json.loads(path.read_text())
    assert {k: v for k, v in actual.items() if k != "layers"} == {k: v for k, v in expected.items() if k != "layers"}
    a, e = actual["layers"][0]["paths"], expected["layers"][0]["paths"]
    assert len(a) == len(e) and len(a) > 0
    assert np.allclose(np.concatenate([np.array(p) for p in a]), np.concatenate([np.array(p) for p in e]), atol=1e-3)
```

- [ ] **Step 2: Run to verify it fails for the missing files**

Run: `uv run pytest tests/test_golden.py -v`
Expected: FAIL with `FileNotFoundError`.

- [ ] **Step 3: Generate the golden files**

Run: `uv run pytest tests/test_golden.py --update-golden -q`
Expected: 2 passed, two files under `tests/golden/`, each under 100 KB.

- [ ] **Step 4: Run the tests and the verification set**

Run: `uv run pytest tests/test_golden.py -v`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add tests/conftest.py tests/test_golden.py tests/golden
git commit -m "Add golden output tests for the grid style" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

If a CI platform later flips a single cell in a golden file, move `threshold` in `produce` to a value farther from any sampled ink value rather than loosening the comparison.

### Task 15: Session state, image loading, blocking runs and export

**Files:**
- Create: `src/plotline/session/session.py`, `src/plotline/session/images.py`
- Test: `tests/test_session.py`, `tests/test_images.py`

**Interfaces:**
- Produces, in `plotline.session.images`: `MAX_EDGE_PX = 4000`; `load_image(path: str | os.PathLike[str]) -> NDArray[np.float32]` returning `(H, W, 3)` in 0..1. It applies `PIL.ImageOps.exif_transpose`, composites images with an alpha channel (modes `RGBA`, `LA`, or `P` with `transparency` in `info`) over white, converts to `RGB`, and shrinks with `Image.thumbnail((MAX_EDGE_PX, MAX_EDGE_PX), Image.Resampling.LANCZOS)`.
- Produces, in `plotline.session.session`:
  - `class RunState(str, Enum)` with `idle`, `running`, `done`, `cancelled`, `error`.
  - `class Session` with `__init__(self, cache_bytes: int = 512 * 2**20, max_fps: float = 15.0)`. Attributes: `image: NDArray[np.float32] | None`, `canvas: Canvas` (default `Canvas(210, 297, 10)`), `pens: tuple[Pen, ...]` (default `(Pen("Black 0.3", 0.3, "#000000"),)`), `style: Style` (default `get_style("grid")`), `settings: dict[str, Settings]`, `cache: StageCache`, `result: Result | None`, `state: RunState`, `max_fps: float`. Methods: `load_image(path) -> None`; `set_image(image: NDArray[Any]) -> None` accepting `(H, W, 3)` or `(H, W)` arrays, uint8 scaled by 255, stored as float32 RGB; `set_canvas(canvas: Canvas) -> None`; `set_pens(pens: Sequence[Pen]) -> None` raising `ValueError` when empty; `set_style(style: str | Style) -> None` resetting `settings` to the style's defaults and keeping the cache; `update(stage: str, **values: Any) -> None` raising `KeyError` for an unknown stage and validating with `type(current).model_validate({**current.model_dump(), **values})`; `run_blocking() -> Result` raising `RuntimeError` when no image is loaded, driving `run_style` in the calling thread through `self._deliver(frame)` (a no-op until Task 16), setting `state` to `done` and `result`, re-raising stage exceptions after setting `state` to `error`, and re-raising `Cancelled` after setting `state` to `cancelled`; `svg() -> str`; `json_dict() -> dict[str, Any]`; `export_svg(path) -> None`; `export_json(path) -> None`, all four raising `RuntimeError` when `result` is `None`.

- [ ] **Step 1: Write the failing tests**

```python
def test_defaults():
    s = Session()
    assert s.canvas == Canvas(210, 297, 10) and s.pens == (Pen("Black 0.3", 0.3, "#000000"),)
    assert s.style.name == "grid" and set(s.settings) == {"tone", "points", "render", "order"}
    assert s.state == RunState.idle and s.result is None and s.image is None

def test_set_image_accepts_uint8_and_grey():
    s = Session(); s.set_image(np.full((4, 4, 3), 255, np.uint8))
    assert s.image.dtype == np.float32 and s.image.max() == 1.0
    s.set_image(np.zeros((4, 4), np.float32))
    assert s.image.shape == (4, 4, 3)

def test_update_validates_and_set_style_resets():
    s = Session(); s.update("order", min_length_mm=1.5)
    assert s.settings["order"].min_length_mm == 1.5
    with pytest.raises(ValidationError):
        s.update("order", min_length_mm="abc")
    with pytest.raises(KeyError):
        s.update("ghost", x=1)
    s.set_style("grid")
    assert s.settings["order"].min_length_mm == 0.0
    with pytest.raises(ValueError):
        s.set_pens(())

def test_run_blocking_requires_an_image():
    with pytest.raises(RuntimeError):
        Session().run_blocking()

def test_run_blocking_grid_and_exports(tmp_path):
    s = Session(); s.set_image(gradient(64, 48)); s.update("tone", resolution=64)
    r = s.run_blocking()
    assert r is s.result and s.state == RunState.done and len(r.layers[0].paths) > 0
    s.export_svg(tmp_path / "o.svg"); s.export_json(tmp_path / "o.json")
    assert ET.parse(tmp_path / "o.svg").getroot().tag.endswith("svg")
    assert json.loads((tmp_path / "o.json").read_text())["format"] == "plotline-paths"
    assert s.svg().startswith("<svg") and s.json_dict()["units"] == "mm"

def test_exports_require_a_result():
    with pytest.raises(RuntimeError):
        Session().svg()

def test_error_state_on_failing_stage():
    s = Session(); s.set_image(gradient(8, 8))
    s.set_style(Style("f", "F", "d", (Spy("a", ("image",), "paths", fail=RuntimeError("boom")),)))
    with pytest.raises(RuntimeError):
        s.run_blocking()
    assert s.state == RunState.error and s.result is None
```

```python
def save(tmp_path, img, name="i.png", **kw):
    p = tmp_path / name; img.save(p, **kw); return p

def test_load_png_shape_dtype_range(tmp_path):
    a = load_image(save(tmp_path, Image.fromarray((gradient(20, 10) * 255).astype(np.uint8))))
    assert a.shape == (10, 20, 3) and a.dtype == np.float32 and 0 <= a.min() and a.max() <= 1

def test_load_downsizes_long_edge(tmp_path):
    a = load_image(save(tmp_path, Image.new("RGB", (5000, 100))))
    assert a.shape == (80, 4000, 3)

def test_load_applies_exif_orientation(tmp_path):
    exif = Image.Exif(); exif[0x0112] = 6
    a = load_image(save(tmp_path, Image.new("RGB", (20, 10)), "i.jpg", exif=exif.tobytes()))
    assert a.shape == (20, 10, 3)

def test_load_composites_alpha_on_white(tmp_path):
    a = load_image(save(tmp_path, Image.new("RGBA", (4, 4), (0, 0, 0, 0))))
    assert a.min() == 1.0
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_session.py tests/test_images.py -v`
Expected: FAIL with `ImportError`.

- [ ] **Step 3: Implement `images.py` and `session.py`**

- [ ] **Step 4: Run the tests and the verification set**

Expected: 11 passed.

- [ ] **Step 5: Commit**

```bash
git add src/plotline/session tests/test_session.py tests/test_images.py
git commit -m "Add the session with image loading, blocking runs and export" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 16: Threaded runs, cancellation and frame delivery

**Files:**
- Modify: `src/plotline/session/session.py`
- Test: `tests/test_session_threads.py`

**Interfaces:**
- Produces, in `plotline.session.session`:
  - `class RunHandle` with `wait(timeout: float | None = None) -> RunState`, `cancel() -> None`, and read-only properties `state: RunState`, `result: Result | None`, `error: BaseException | None`.
  - `Session.run() -> RunHandle`: cancels and joins any running handle, snapshots `dict(self.settings)`, the roots and the style, then starts a daemon `threading.Thread` that drives `run_style` and updates the handle and the session as `run_blocking` does. `Session.state` reflects the latest handle. `Session.result` is set only on `done`.
  - `Session.cancel() -> None` sets the current handle's token.
  - `Session.subscribe(callback: Callable[[Frame], None]) -> Callable[[], None]` returning an unsubscribe function.
  - `Session._deliver(frame: Frame) -> None`: delivers to every subscriber when `frame.progress >= 1` or at least `1 / max_fps` seconds (by `time.monotonic()`) have passed since the last delivery; the first frame of a run is always delivered. `run_blocking` delivers through the same method.

- [ ] **Step 1: Write the failing tests**

```python
PATHS = [np.array([[1.0, 1.0], [2.0, 2.0]])]
def slow(frames=100, delay=0.01):
    a = Spy("a", ("image",), "points", value=np.zeros((1, 2)))
    b = Spy("b", ("points",), "paths", value=PATHS, frames=frames, delay_s=delay)
    return Style("slow", "Slow", "d", (a, b)), a, b

def session(style, max_fps=1e9):
    s = Session(max_fps=max_fps); s.set_image(gradient(8, 8)); s.set_style(style); return s

def test_run_completes_and_delivers_every_frame_when_unthrottled():
    style, a, b = slow(frames=20, delay=0); s = session(style); seen = []; s.subscribe(seen.append)
    h = s.run()
    assert h.wait(5) == RunState.done and s.state == RunState.done and s.result is h.result
    assert sum(f.stage == "b" for f in seen) == 20

def test_throttle_always_delivers_final_frames():
    style, _, _ = slow(frames=20, delay=0); s = session(style, max_fps=1e-6); seen = []; s.subscribe(seen.append)
    s.run().wait(5)
    assert [(f.stage, f.progress) for f in seen] == [("a", 1.0), ("b", 1.0)]

def test_cancel_from_another_thread():
    style, a, b = slow(); s = session(style); third = threading.Event(); n = [0]
    def cb(frame):
        n[0] += 1
        if n[0] == 3: third.set()
    s.subscribe(cb); h = s.run(); assert third.wait(5); s.cancel()
    assert h.wait(5) == RunState.cancelled and s.state == RunState.cancelled and s.result is None
    assert len(s.cache) == 1 and b.runs == 1

def test_new_run_cancels_the_previous_one():
    style, _, _ = slow(); s = session(style)
    h1 = s.run(); time.sleep(0.05); h2 = s.run()
    assert h1.state == RunState.cancelled and h2.wait(5) == RunState.done

def test_error_is_stored_on_the_handle():
    style = Style("f", "F", "d", (Spy("a", ("image",), "paths", fail=RuntimeError("boom")),))
    s = session(style); h = s.run()
    assert h.wait(5) == RunState.error and isinstance(h.error, RuntimeError) and s.result is None

def test_unsubscribe_and_blocking_delivery():
    style, _, _ = slow(frames=3, delay=0); s = session(style); seen = []
    unsub = s.subscribe(seen.append); s.run_blocking(); assert len(seen) >= 2
    unsub(); seen.clear(); s.run_blocking(); assert seen == []
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_session_threads.py -v`
Expected: FAIL with `AttributeError` on `subscribe` or `run`.

- [ ] **Step 3: Implement `RunHandle`, `run`, `cancel`, `subscribe` and `_deliver`**

Keep one private `_execute(handle, style, settings, roots)` used by both `run_blocking` and the worker thread so the state transitions live in one place. Initialise the throttle clock to `-inf` at the start of each run.

- [ ] **Step 4: Run the tests and the verification set**

Expected: 6 passed, plus the Task 15 tests still passing.

- [ ] **Step 5: Commit**

```bash
git add src/plotline/session/session.py tests/test_session_threads.py
git commit -m "Run pipelines in a worker thread with cancellation and throttled frames" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 17: Stats

**Files:**
- Create: `src/plotline/session/stats.py`
- Modify: `src/plotline/session/session.py`
- Test: `tests/test_stats.py`

**Interfaces:**
- Produces, in `plotline.session.stats`: `@dataclass class Speeds: draw_mm_s: float = 50.0; travel_mm_s: float = 100.0; pen_lift_s: float = 0.2`; `@dataclass(frozen=True) class Stats: path_count: int; pen_down_mm: float; pen_up_mm: float; estimated_seconds: float`; `compute_stats(result: Result, speeds: Speeds) -> Stats` using `total_length` and `travel_length` from `plotline.core.paths` over all layers' paths in order, starting at the drawing rect's top-left corner, with `estimated_seconds = pen_down_mm / draw_mm_s + pen_up_mm / travel_mm_s + path_count * pen_lift_s`.
- Produces, in `Session`: attribute `speeds: Speeds` (default `Speeds()`) and `stats() -> Stats` raising `RuntimeError` without a result.

- [ ] **Step 1: Write the failing tests**

```python
def test_compute_stats_by_hand():
    r = Result(Canvas(100, 100, 10), [Layer(Pen("p", 0.3, "#000000"), [np.array([[0.0, 0.0], [10.0, 0.0]]), np.array([[20.0, 0.0], [20.0, 5.0]])])])
    st = compute_stats(r, Speeds())
    assert st.path_count == 2 and st.pen_down_mm == pytest.approx(15)
    assert st.pen_up_mm == pytest.approx(math.hypot(10, 10) + 10)
    assert st.estimated_seconds == pytest.approx(15 / 50 + (math.hypot(10, 10) + 10) / 100 + 0.4)

def test_empty_result_has_zero_stats():
    st = compute_stats(Result(Canvas(100, 100), [Layer(Pen("p", 0.3, "#000000"), [])]), Speeds())
    assert (st.path_count, st.pen_down_mm, st.pen_up_mm, st.estimated_seconds) == (0, 0, 0, 0)

def test_session_stats():
    s = Session(); s.set_image(gradient(64, 48)); s.update("tone", resolution=64); s.run_blocking()
    assert s.stats().path_count == len(s.result.layers[0].paths) > 0
    with pytest.raises(RuntimeError):
        Session().stats()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_stats.py -v`
Expected: FAIL with `ImportError`.

- [ ] **Step 3: Implement `stats.py` and `Session.stats`**

- [ ] **Step 4: Run the tests and the verification set**

Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/plotline/session/stats.py src/plotline/session/session.py tests/test_stats.py
git commit -m "Add drawing stats and a plot time estimate" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 18: Preview PNG

**Files:**
- Create: `src/plotline/session/preview.py`
- Modify: `src/plotline/session/session.py`
- Test: `tests/test_preview.py`

**Interfaces:**
- Produces, in `plotline.session.preview`: `SUPERSAMPLE = 4`; `render_png(result: Result, width_px: int = 1000, travel: bool = False, source: bool = False) -> bytes`. Output height is `round(canvas.height_mm * width_px / canvas.width_mm)`. Draws, at `SUPERSAMPLE` times the size and then `Image.reduce(SUPERSAMPLE)`: a white page; the drawing rect outlined in `#dddddd`; when `source` and `result.tone` is not `None`, the tone map as grey `255 * (1 - ink)` resized to its `rect_mm` and blended at 30 percent; each layer's paths in the pen colour at width `max(1, pen.width_mm * px_per_mm * SUPERSAMPLE)` with `joint="curve"` and filled circles of that diameter at both ends for round caps, a one-vertex path as one such circle; when `travel`, a `#ff8080` line of width `SUPERSAMPLE` from each path's last vertex to the next path's first vertex. Returns PNG bytes.
- Produces, in `Session`: `preview_png(width_px: int = 1000, travel: bool = False, source: bool = False) -> bytes` raising `RuntimeError` without a result.

- [ ] **Step 1: Write the failing tests**

```python
PEN = Pen("p", 2.0, "#000000")
def png(result, **kw): return Image.open(io.BytesIO(render_png(result, **kw))).convert("RGB")

def test_size_and_format():
    im = Image.open(io.BytesIO(render_png(Result(Canvas(210, 297, 10), [Layer(PEN, [])]), width_px=200)))
    assert im.format == "PNG" and im.size == (200, 283)

def test_dot_is_drawn_at_true_width():
    im = png(Result(Canvas(100, 100), [Layer(PEN, [np.array([[50.0, 50.0]])])]), width_px=200)
    assert im.getpixel((100, 100))[0] < 128 and im.getpixel((110, 100))[0] > 200

def test_travel_overlay_is_red_only_when_asked():
    r = Result(Canvas(100, 100), [Layer(PEN, [np.array([[10.0, 50.0], [20.0, 50.0]]), np.array([[80.0, 50.0], [90.0, 50.0]])])])
    def reddish(im): return any(p[0] > p[1] + 50 for p in im.getdata())
    assert reddish(png(r, travel=True, width_px=200)) and not reddish(png(r, width_px=200))

def test_source_overlay_darkens_the_image_area():
    tone = ToneMap(np.ones((10, 10), np.float32), 5.0, (25.0, 25.0))
    r = Result(Canvas(100, 100), [Layer(PEN, [])], tone=tone)
    assert png(r, source=True).getpixel((500, 500))[0] < png(r).getpixel((500, 500))[0]

def test_empty_result_renders():
    assert png(Result(Canvas(100, 100), [Layer(PEN, [])])).size == (1000, 1000)

def test_session_preview():
    s = Session(); s.set_image(gradient(64, 48)); s.update("tone", resolution=64); s.run_blocking()
    assert s.preview_png(width_px=100)[:8] == b"\x89PNG\r\n\x1a\n"
    with pytest.raises(RuntimeError):
        Session().preview_png()
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_preview.py -v`
Expected: FAIL with `ImportError`.

- [ ] **Step 3: Implement `preview.py` and `Session.preview_png`**

- [ ] **Step 4: Run the tests and the verification set**

Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add src/plotline/session/preview.py src/plotline/session/session.py tests/test_preview.py
git commit -m "Render preview PNGs at true pen width with travel and source overlays" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 19: Presets

**Files:**
- Create: `src/plotline/session/presets.py`
- Modify: `src/plotline/session/session.py`
- Test: `tests/test_presets.py`

**Interfaces:**
- Produces, in `plotline.session.presets`: `PRESET_FORMAT = "plotline-preset"`; `PRESET_VERSION = 1`; `MIGRATIONS: dict[int, Callable[[dict[str, Any]], dict[str, Any]]] = {}` mapping a version to the function that upgrades a preset of that version to the next; `preset_dict(session: Session) -> dict[str, Any]` returning `{"format", "version", "style", "canvas": asdict(canvas), "pens": [asdict(p)...], "settings": {name: s.model_dump(mode="json")}}`; `apply_preset(session: Session, data: dict[str, Any]) -> None` raising `ValueError` on a wrong `format` or a `version` above `PRESET_VERSION`, applying migrations from the file's version up to the current one, then `set_style`, `set_canvas`, `set_pens`, and per stage `update` with the keys that exist, emitting `warnings.warn(..., UserWarning)` once per unknown stage and once per unknown key; `save_preset(session, path) -> None`; `load_preset(session, path) -> None`.
- Produces, in `Session`: `save_preset(path) -> None` and `load_preset(path) -> None` delegating to the module.

- [ ] **Step 1: Write the failing tests**

```python
def test_round_trip(tmp_path):
    s = Session(); s.update("order", min_length_mm=2.0); s.set_canvas(Canvas(100, 100, 5, "fill")); s.set_pens([Pen("Red", 0.5, "#ff0000")])
    p = tmp_path / "p.json"; s.save_preset(p); d = json.loads(p.read_text())
    assert (d["format"], d["version"], d["style"], d["settings"]["order"]["min_length_mm"]) == ("plotline-preset", 1, "grid", 2.0)
    t = Session(); t.load_preset(p)
    assert t.canvas == s.canvas and t.pens == s.pens and t.settings == s.settings and t.style is s.style

def test_unknown_stage_and_key_warn_and_are_dropped():
    s = Session(); d = preset_dict(s); d["settings"]["order"]["nope"] = 1; d["settings"]["ghost"] = {}; d["settings"]["order"]["min_length_mm"] = 3.0
    with pytest.warns(UserWarning) as w:
        apply_preset(s, d)
    assert len(w) == 2 and s.settings["order"].min_length_mm == 3.0

def test_missing_keys_take_defaults():
    s = Session(); s.update("order", min_length_mm=3.0); d = preset_dict(s); del d["settings"]["order"]["min_length_mm"]
    apply_preset(s, d)
    assert s.settings["order"].min_length_mm == 0.0

def test_bad_format_and_future_version_raise():
    s = Session()
    with pytest.raises(ValueError):
        apply_preset(s, {**preset_dict(s), "format": "other"})
    with pytest.raises(ValueError):
        apply_preset(s, {**preset_dict(s), "version": PRESET_VERSION + 1})

def test_migration_is_invoked(monkeypatch):
    calls = []
    def up(d): calls.append(d["version"]); return {**d, "version": 1}
    monkeypatch.setattr(presets, "MIGRATIONS", {0: up})
    s = Session(); apply_preset(s, {**preset_dict(s), "version": 0})
    assert calls == [0]
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_presets.py -v`
Expected: FAIL with `ImportError`.

- [ ] **Step 3: Implement `presets.py` and the two `Session` methods**

- [ ] **Step 4: Run the tests and the verification set**

Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
git add src/plotline/session/presets.py src/plotline/session/session.py tests/test_presets.py
git commit -m "Add versioned JSON presets" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 20: CLI

**Files:**
- Create: `src/plotline/cli/main.py`
- Modify: `pyproject.toml` (add `[project.scripts] plotline = "plotline.cli.main:main"`)
- Test: `tests/test_cli.py`

**Interfaces:**
- Consumes: `Session`, `list_styles`, `get_style`, `Canvas`, `Pen`.
- Produces: a click group `main` with commands:
  - `styles`: one line per style, `name`, `label` and description separated by two spaces.
  - `describe STYLE [--json]`: for each stage a heading `name (label)` then one line per setting with the name, the JSON Schema type, the default, the range when present and the help text; `--json` prints `style.schema()` as JSON. Unknown style: `ClickException`.
  - `run STYLE IMAGE --out PATH [--json PATH] [--preview PATH] [--width MM] [--height MM] [--margin MM] [--fit fit|fill] [--pen NAME:WIDTH_MM:COLOR]... [--set STAGE.KEY=VALUE]... [--preset PATH] [--save-preset PATH] [--quiet]`. Order: `Session(max_fps=4)`, `load_image`, `load_preset` if given, `set_style(STYLE)` only when the session's style name differs, canvas rebuilt from the current canvas with any given `--width`, `--height`, `--margin`, `--fit`, pens from `--pen` when given, then each `--set`. A `--set` without exactly one `.` before `=` and one `=` is a `click.BadParameter`. `ValueError`, `KeyError` and `pydantic.ValidationError` from the session become `click.ClickException` with the message. Progress frames print `stage: label` to stderr unless `--quiet`. Writes `--out` SVG and the optional JSON, preview and preset, then prints `paths: N  pen down: X mm  pen up: Y mm  estimated: Z s` with one decimal.
  - `gui` and `mcp`: print `The GUI is not available yet.` / `The MCP server is not available yet.` to stderr and exit with status 1.

- [ ] **Step 1: Write the failing tests**

```python
@pytest.fixture
def image(tmp_path):
    p = tmp_path / "i.png"; Image.fromarray((gradient(64, 48) * 255).astype(np.uint8)).save(p); return str(p)

def run(*args): return CliRunner().invoke(main, list(args))   # click 8.2: .output is combined, .stderr separate

def test_styles_and_describe():
    assert "grid" in run("styles").output and run("styles").exit_code == 0
    out = run("describe", "grid").output
    assert "min_length_mm" in out and "Dots have zero length" in out
    assert json.loads(run("describe", "grid", "--json").output)["stages"][0]["name"] == "tone"
    assert run("describe", "nope").exit_code != 0

def test_run_writes_all_outputs(tmp_path, image):
    r = run("run", "grid", image, "--out", str(tmp_path / "o.svg"), "--json", str(tmp_path / "o.json"),
            "--preview", str(tmp_path / "o.png"), "--save-preset", str(tmp_path / "p.json"),
            "--set", "tone.resolution=64", "--pen", "Red 0.5:0.5:#ff0000", "--width", "100", "--height", "75")
    assert r.exit_code == 0, r.output
    assert ET.parse(tmp_path / "o.svg").getroot()[0].get("stroke") == "#ff0000"
    assert (tmp_path / "o.json").exists() and (tmp_path / "o.png").exists()
    assert json.loads((tmp_path / "p.json").read_text())["canvas"]["width_mm"] == 100
    assert "paths:" in r.output and "tone:" in r.stderr

def test_preset_is_applied_and_style_kept(tmp_path, image):
    run("run", "grid", image, "--out", str(tmp_path / "a.svg"), "--set", "tone.resolution=64", "--set", "points.spacing_mm=4", "--save-preset", str(tmp_path / "p.json"))
    r = run("run", "grid", image, "--out", str(tmp_path / "b.svg"), "--preset", str(tmp_path / "p.json"), "--quiet")
    assert r.exit_code == 0 and r.stderr == ""
    assert (tmp_path / "a.svg").read_text() == (tmp_path / "b.svg").read_text()

def test_run_rejects_bad_inputs(tmp_path, image):
    out = str(tmp_path / "o.svg")
    assert run("run", "grid", image, "--out", out, "--set", "order.min_length_mm=abc").exit_code != 0
    assert "min_length_mm" in run("run", "grid", image, "--out", out, "--set", "order.min_length_mm=abc").output
    assert run("run", "grid", image, "--out", out, "--set", "nonsense").exit_code == 2
    r = run("run", "grid", image, "--out", out, "--margin", "200")
    assert r.exit_code != 0 and "margin" in r.output

def test_gui_and_mcp_stubs():
    for cmd in ("gui", "mcp"):
        r = run(cmd)
        assert r.exit_code == 1 and "not available" in r.stderr
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `uv run pytest tests/test_cli.py -v`
Expected: FAIL with `ImportError`.

- [ ] **Step 3: Implement `main.py` and add the script entry to `pyproject.toml`, then `uv sync`**

Define `parse_set(value: str) -> tuple[str, str, str]` and `parse_pen(value: str) -> Pen` as module functions so the error messages stay in one place.

- [ ] **Step 4: Run the tests and the verification set, then try the real entry point**

Expected: 5 passed. Run `uv run plotline styles` and expect a line starting with `grid`.

- [ ] **Step 5: Commit**

```bash
git add src/plotline/cli/main.py pyproject.toml uv.lock tests/test_cli.py
git commit -m "Add the command line interface" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 21: README and final verification

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update the README**

Replace the status paragraph with: what works now (the `grid` style via the CLI, SVG and JSON export, preview PNGs, presets), how to install from a clone (`uv sync`, then `uv run plotline --help`), one `plotline run grid your-photo.jpg --out drawing.svg --preview preview.png` example, and a pointer to the brief for everything that is still planned. Name no other tools. Reference no third-party images.

- [ ] **Step 2: Run the full verification set and the licence check**

Run: `uv run ruff format --check .`, `uv run ruff check .`, `uv run mypy src`, `uv run pytest -q`, `uv run python scripts/check_licenses.py`
Expected: all clean, every test passing, licence check exit 0.

- [ ] **Step 3: Build the wheel once**

Run: `uv build`
Expected: `dist/plotline-0.1.0.dev0-py3-none-any.whl` is produced. `dist/` is ignored by git.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "Describe what works in the README" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
