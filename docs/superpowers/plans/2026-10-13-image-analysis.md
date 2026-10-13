# Image analysis and auto levels Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The engine measures each image once and suggests patches (line art, levels, invert, noise, orientation, colour), the tone stage gains automatic levels, and the CLI, MCP and GUI show the suggestions without applying any on their own.

**Architecture:** A new core module `analysis.rs` holds `measure` (image statistics), `levels` (percentile black and white points, shared with the tone stage) and `suggest` (rules over the statistics and the session state). The session measures on `set_image` and works out suggestions on each `analysis()` call. Bindings expose it as a dict and as JSON; the CLI prints it, the MCP returns it, and the GUI worker puts the suggestions in every state reply and applies a patch in one `apply` command.

**Tech Stack:** Rust (serde, schemars, rand_pcg in synthetic.rs), pyo3, wasm-bindgen, Python (click, mcp SDK), TypeScript, Preact, vitest.

**Spec:** `docs/superpowers/specs/2026-10-13-image-analysis-design.md`

## Global Constraints

- No new runtime dependency anywhere.
- Determinism: integer histograms with fixed bin edges, sums in row-major order, `sqrt` allowed, no other float library call outside `math.rs` (the source scan in `math.rs` forbids `sin`, `cos`, `tan`, `atan2`, `exp`, `ln`, `log`, `powf`, `mul_add`, `sin_cos`, `atan` elsewhere). Native and WebAssembly results must match bit for bit.
- Cite papers in module docstrings: Otsu 1979 (doi:10.1109/TSMC.1979.4310076), Hasler & Süsstrunk 2003 (doi:10.1117/12.477378), Immerkær 1996 (doi:10.1006/cviu.1996.0060), Limare et al. 2011 (doi:10.5201/ipol.2011.llmps-scb). The last one's reference code is GPL-3.0: write from the paper only, never read or port the code.
- Suggestion cut-offs, verbatim from spec §3: `line_art` two_level ≥ 0.85 and mid ≤ 0.15; `levels` p_high − p_low < 0.6; `invert` border < 0.3, mean < 0.5, two_level ≥ 0.85; `noise` noise > 0.03; `orientation` ratio ≥ 1.3; `colour` colourfulness > 40.
- Reason sentences, verbatim (one per kind):
  - `line_art`: "This looks like line art. The Ink style keeps its shapes and line weights."
  - `levels`: "The image uses a narrow range of tones. Auto levels stretch it over the full range."
  - `invert`: "This looks like light marks on a dark ground. Inverting it gives the marks the ink."
  - `noise`: "The image is noisy. A blur of one tone-map pixel keeps the noise from turning into specks."
  - `orientation`: "The image would draw larger with the page turned."
  - `colour`: "This is a colour image. Each pen draws one colour, and splitting colours across pens is not built yet, so it is drawn in the first pen's tones."
- Tone defaults stay as they are: `levels` `"manual"`, `clip_percent` 0.5. Golden files and existing presets must not change.
- Images in tests are generated in code. No product names anywhere. LF line ends. Rust fmt and clippy `-D warnings`, ruff, mypy and `tsc` clean.

## Review Focus

- A preset saved before this change (no `levels` key) must load and draw exactly as before. Test in Task 3.
- A flat or nearly flat image must not be stretched into ink by auto levels (`white − black` < 1/64 keeps 0 and 1). Test in Task 3.
- A tiny image (1 × 1, 2 × 2) must analyse without a panic or a division by zero (noise 0, two_level 0). Test in Task 1.
- Applying a suggestion in the GUI must redraw once, not once per call. Test in Task 6.
- `plotline analyse` on a file that is not an image must exit 1 with the engine's message, like `run`. Test in Task 5.

---

### Task 1: Image statistics

**Files:**
- Create: `crates/core/src/analysis.rs` (declare `pub mod analysis;` in `lib.rs`)
- Modify: `crates/core/src/synthetic.rs`
- Test: `crates/core/src/analysis.rs` tests module

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, PartialEq, Serialize)] pub struct ImageStats { pub width_px: u32, pub height_px: u32, pub p_low: f64, pub p_high: f64, pub mean: f64, pub otsu: f64, pub two_level: f64, pub dark: f64, pub mid: f64, pub light: f64, pub colourfulness: f64, pub noise: f64, pub border: f64 }`
  - `pub fn measure(image: &Image) -> ImageStats`
  - `pub fn levels(lum: &[f32], clip_percent: f64) -> (f64, f64)`: the black and white points of spec §4 on a 4096-bin histogram (bin `min(floor(v * 4096), 4095)`; black = lower edge of the first bin where the cumulative count from the dark end exceeds `clip_percent / 100 * n`; white = upper edge of the first bin where the count from the light end does). `p_low` and `p_high` are `levels(lum, 0.5)`.
  - synthetic helpers: `strokes(w: u32, h: u32, ink: f32, paper: f32) -> Image` (2-pixel lines every 16 pixels in both directions), `ramp(w: u32, h: u32, from: f32, to: f32) -> Image` (left-to-right), `flat_noise(w: u32, h: u32, value: f32, sigma: f32, seed: u64) -> Image` (uniform noise of half-width `sigma * sqrt(3)` around `value`, clamped to 0..1).

- [ ] **Step 1: Write the failing tests:**
  - `line_art_is_two_level`: `measure(&strokes(256, 256, 0.0, 1.0))` has `two_level >= 0.85` and `mid <= 0.15`.
  - `a_gradient_is_not`: `measure(&ramp(256, 64, 0.0, 1.0))` has `two_level < 0.85`, `mid` within 0.05 of 0.5, and `p_high - p_low > 0.9`.
  - `a_narrow_ramp_has_a_narrow_range`: `ramp(256, 64, 0.4, 0.6)` gives `p_high - p_low < 0.25`.
  - `noise_is_estimated`: `flat_noise(256, 256, 0.5, 0.1, 7)` gives `noise` within 20 % of 0.1; `flat(256, 256, 0.5)` gives `noise == 0.0` and `two_level == 0.0`.
  - `light_marks_on_dark`: `strokes(256, 256, 1.0, 0.0)` gives `border < 0.3` and `mean < 0.5`.
  - `colour_bars_are_colourful`: `color_bars(256, 64)` gives `colourfulness > 40`; `ramp(256, 64, 0.0, 1.0)` gives `colourfulness == 0.0`.
  - `tiny_images_measure`: `flat(1, 1, 0.5)` and `flat(2, 2, 0.5)` measure without panicking, with `noise == 0.0`.
  - `big_images_are_downsampled`: `measure(&ramp(3000, 10, 0.0, 1.0))` reports `width_px == 3000` and matches `measure(&ramp(1000, 10, 0.0, 1.0))` within 0.01 on `mean`.
  - `levels_finds_the_clip_points`: `levels` of 1000 values `i / 999` with clip 0.5 gives black within 1/4096 of 0.005 and white within 1/4096 of 0.995.
- [ ] **Step 2: Run** `cargo test -p plotline-core analysis`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement.** Block-average luminance (`image.luminance()`) and each colour channel with `math::block_average` at `k = max(1, ceil(long / 1024))`. Otsu on 256 bins with f64 sums in bin order; `two_level` = max σ²_between / σ²_total, 0 when σ²_total is 0; `otsu` is the threshold bin's upper edge. Noise: Immerkær's mask `[[1, -2, 1], [-2, 4, -2], [1, -2, 1]]`, `noise = 1.2533141373155003 * sum(|response|) / (6 * (w - 2) * (h - 2))` (that constant is √(π/2)), 0 when either side is under 3. Colourfulness on 0..255 values per spec §2. Border band `max(1, round(0.05 * min(w, h)))` pixels wide. Module docstring cites Otsu, Hasler & Süsstrunk and Immerkær.
- [ ] **Step 4: Run** `cargo test -p plotline-core analysis` and `cargo clippy --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit** `Measure images for analysis`.

### Task 2: Suggestions

**Files:**
- Modify: `crates/core/src/analysis.rs`
- Test: `crates/core/src/analysis.rs` tests module

**Interfaces:**
- Consumes: `ImageStats`, `measure`, the synthetic helpers from Task 1.
- Produces:
  - `#[derive(Clone, Debug, PartialEq, Serialize)] pub struct Suggestion { pub kind: &'static str, pub reason: &'static str, pub patch: Option<Value> }`; `patch` serialises as `null` when absent; a patch is an object with any of `style`, `settings`, `page` (spec §3).
  - `pub fn suggest(stats: &ImageStats, canvas: &Canvas, style: &str, settings: &Value) -> Vec<Suggestion>`, rules in the order `line_art`, `levels`, `invert`, `noise`, `orientation`, `colour`, each left out when already in effect (spec §3 table). `settings` is the session's settings value (`{stage: {key: value}}`); a missing `tone` stage or key counts as its default.
  - `#[derive(Clone, Debug, PartialEq, Serialize)] pub struct Analysis { pub stats: ImageStats, pub suggestions: Vec<Suggestion> }`.

- [ ] **Step 1: Write the failing tests** (canvas `Canvas::new(210.0, 297.0, 10.0, Fit::Fit)`, settings from `get_style(name).defaults()`):
  - `line_art_suggests_ink`: strokes under `grid` give a `line_art` with patch `{"style": "ink"}`; under `ink` none.
  - `a_narrow_range_suggests_auto_levels`: `ramp(256, 64, 0.4, 0.6)` gives `levels` with patch `{"settings": {"tone": {"levels": "auto"}}}`; with tone `levels` `"auto"` none.
  - `light_marks_suggest_invert`: `strokes(256, 256, 1.0, 0.0)` gives `invert`; with tone `invert` true none; a dark ramp `ramp(256, 64, 0.0, 0.4)` gives none (not two-level).
  - `noise_suggests_one_tone_pixel_of_blur`: `flat_noise(256, 256, 0.5, 0.1, 7)` with tone resolution 1000 gives patch blur `0.3` (277 / 1000 = 0.277, rounded up to 0.1); with `blur_mm` 0.5 none.
  - `a_landscape_image_suggests_turning_the_page`: a 400 × 200 `ramp` gives patch `{"page": {"width_mm": 297.0, "height_mm": 210.0}}`; a 200 × 400 one gives none; on a `Fit::Fill` canvas none.
  - `colour_is_a_note`: `color_bars(256, 64)` gives `colour` with `patch == None`.
  - `reasons_are_the_fixed_sentences`: each kind's `reason` equals the Global Constraints sentence.
  - `serialises_as_the_spec_shows`: `serde_json::to_value` of the line-art suggestion equals `{"kind": "line_art", "reason": <sentence>, "patch": {"style": "ink"}}`.
- [ ] **Step 2: Run** `cargo test -p plotline-core analysis`. Expected: FAIL.
- [ ] **Step 3: Implement** per spec §3. Orientation: with drawing area `dw × dh` and image `w × h`, the fit scale is `min(dw / w, dh / h)`; the turned page's area is `(H - 2m) × (W - 2m)`; suggest when `fit` is `Fit`, `(w > h) != (dw > dh)`, and the turned scale is at least 1.3 times the current one. Blur: `(max(dw, dh) / resolution * 10.0).ceil() / 10.0`.
- [ ] **Step 4: Run** the tests and clippy. Expected: PASS.
- [ ] **Step 5: Commit** `Suggest changes from the image analysis`.

### Task 3: Auto levels in the tone stage

**Files:**
- Modify: `crates/core/src/stages/tone.rs`
- Modify: `crates/core/tests/settings_matter.rs` (`NO_OPS`)
- Test: `crates/core/src/stages/tone.rs` tests module, `tests/test_presets.py`

**Interfaces:**
- Consumes: `analysis::levels` from Task 1.
- Produces: `ToneSettings` gains `pub levels: Levels` (`#[serde(rename_all = "lowercase")] pub enum Levels { Manual, Auto }`, default `Manual`) and `pub clip_percent: f64` (range 0 to 10, `x-step` 0.1, default 0.5). Help texts verbatim from spec §4: `levels`: "Manual uses Black and White. Auto sets them from the image so that Clip percent of its pixels turn full black and as many full white." `clip_percent`: "Share of pixels, in percent, that auto levels turn full black and, again, full white; used only when Levels is Auto." Place both right after `white`.

- [ ] **Step 1: Write the failing tests:**
  - `auto_levels_stretch_a_narrow_ramp`: tone map of `ramp(256, 64, 0.3, 0.7)` with `levels: auto`, `clip_percent: 0` has min ink ≤ 1/4096 + 1e-6 and max ink ≥ 1 − 1/4096 − 1e-6.
  - `auto_levels_leave_a_flat_image_flat`: `flat(64, 64, 0.5)` with `auto` gives the same map as with `manual`.
  - `manual_is_unchanged`: the default settings give the same ink as before (an existing golden or determinism test covers this; name it in the report).
  - Python, `tests/test_presets.py`: a preset dict without `levels` loads and `settings["tone"]["levels"] == "manual"`.
- [ ] **Step 2: Run** `cargo test -p plotline-core tone`. Expected: FAIL.
- [ ] **Step 3: Implement** spec §4: in `tone_map`, when `levels` is `Auto`, `let (b, w) = analysis::levels(&y, s.clip_percent)` after the resize; when `w - b < 1.0 / 64.0` use 0 and 1; otherwise use them in place of `s.black` and `s.white`. Update the module doc's pipeline order sentence and cite Limare et al. 2011. Add `NO_OPS` rows for `clip_percent` on every listed style with the reason "used only when levels is auto, which is off by default". If `levels` itself does not change a style's drawing on the noise image, add that row with its reason instead of weakening the test.
- [ ] **Step 4: Run** `cargo test -p plotline-core`, clippy, `uv sync`, `uv run pytest -q`. Expected: PASS, golden tests unchanged.
- [ ] **Step 5: Commit** `Add automatic levels to the tone stage`.

### Task 4: Session and bindings

**Files:**
- Modify: `crates/core/src/session.rs`, `crates/py/src/session.rs`, `crates/wasm/src/lib.rs`, `crates/wasm/tests/smoke.rs`
- Modify: `src/plotline/session.py`, `src/plotline/_core.pyi`
- Test: `crates/core/src/session.rs` tests, `tests/test_session.py`

**Interfaces:**
- Consumes: `measure`, `suggest`, `Analysis` from Tasks 1-2.
- Produces:
  - `Session::analysis(&self) -> Option<Analysis>`; the session keeps `image_stats: Option<ImageStats>`, set by `set_image` and `set_image_bytes`.
  - Python binding `analysis() -> dict | None`; `Session.analysis(self) -> dict[str, Any]` raising `RuntimeError("no image is loaded")` without an image.
  - WebAssembly `analysis(): string | undefined` (JSON).

- [ ] **Step 1: Write the failing tests:**
  - Rust `analysis_follows_the_image`: `None` before an image; after `set_image(strokes(..))` the suggestions hold `line_art`; after `set_image(ramp(..))` the stats differ.
  - Rust `still_life_analysis_is_pinned` and a wasm smoke test `still_life_analysis_matches_native`: `measure(&still_life(600, 875))`'s `two_level` and `p_low` equal the same two literal `f64` values in both tests (read them from the first native run and paste them into both).
  - Python `test_analysis`: after `set_image(_core.synthetic_gradient(64, 48))`, `analysis()` has keys `{"stats", "suggestions"}`; a fresh `Session().analysis()` raises `RuntimeError`.
- [ ] **Step 2: Run** them. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p plotline-core`, clippy, `uv sync`, `uv run pytest -q`, mypy, and `wasm-pack test --node` in `crates/wasm` if available. Expected: PASS.
- [ ] **Step 5: Commit** `Expose the image analysis from the session`.

### Task 5: CLI and MCP

**Files:**
- Modify: `src/plotline/cli/main.py`, `src/plotline/mcp_server.py`
- Test: `tests/test_cli.py`, `tests/test_mcp.py`

**Interfaces:**
- Consumes: `Session.analysis()` from Task 4.
- Produces: CLI command `analyse IMAGE [--width MM] [--height MM] [--margin MM] [--fit fit|fill]`; MCP tool `analyse_image()`; `INSTRUCTIONS` gains, after the sentence naming `load_image`, the spec §6 sentence verbatim.

- [ ] **Step 1: Write the failing tests:**
  - CLI `test_analyse_prints_json`: on a PNG of `strokes`-like content (write one with `write_png` from a numpy array of 2-pixel black lines on white), `analyse` exits 0 and its stdout parses as JSON with a `suggestions` list containing kind `line_art`.
  - CLI `test_analyse_turns_the_page`: a 400 × 200 gradient PNG with no options suggests `orientation`; with `--width 297 --height 210` it does not.
  - CLI `test_analyse_refuses_a_non_image`: a text file gives exit 1 and the engine's message on stderr.
  - MCP `test_analyse_image`: before `load_image` it is a tool error containing `no image is loaded`; after, `data(r)` has `stats` and `suggestions`.
  - MCP stdio test: the expected tool set gains `analyse_image` (eleven names).
- [ ] **Step 2: Run** `uv run pytest -q tests/test_cli.py tests/test_mcp.py`. Expected: FAIL.
- [ ] **Step 3: Implement.** Reuse `run`'s page-option handling; factor a small helper shared by both commands if it keeps them short. Print with `json.dumps(..., indent=2)`.
- [ ] **Step 4: Run** the full Python suite, ruff, mypy. Expected: PASS.
- [ ] **Step 5: Commit** `Add image analysis to the CLI and the MCP server`.

### Task 6: GUI, README and the brief

**Files:**
- Modify: `web/src/protocol.ts`, `web/src/panels.tsx`, `web/src/app.tsx`, `web/src/style.css` if needed
- Test: `web/src/protocol.test.ts`
- Modify: `README.md`, `docs/BRIEF.md`

**Interfaces:**
- Consumes: wasm `analysis()` from Task 4.
- Produces:
  - `export type Suggestion = { kind: string; reason: string; patch: Patch | null }`, `export type Patch = { style?: string; settings?: Record<string, Record<string, unknown>>; page?: { width_mm: number; height_mm: number } }` in `protocol.ts`.
  - `State.suggestions: Suggestion[]`, empty without an image.
  - Protocol method `apply(patch: Patch)`, in `REDRAWS`: `setStyle` when `patch.style`, then `update` per stage, then `setCanvas` with the page merged into the current canvas; one redraw.
  - `SuggestionsPanel({ suggestions, error, onApply })` in `panels.tsx`, rendered at the top of the right column only when the list is not empty.

- [ ] **Step 1: Write the failing vitest tests** in `protocol.test.ts`:
  - `suggestions come with the state`: before an image `state.suggestions` is `[]`; after `setImage` of a 400 × 200 gradient, it holds kind `orientation`.
  - `apply runs the patch and redraws once`: after `setImage`, `apply({style: "ink", settings: {tone: {levels: "auto"}}, page: {width_mm: 297, height_mm: 210}})` gives state style `ink`, tone levels `auto`, canvas 297 × 210, and exactly one run start (count the posted run events the way the existing redraw tests do).
- [ ] **Step 2: Run** `npm run wasm && npm run check` in `web/`. Expected: FAIL.
- [ ] **Step 3: Implement** the protocol changes, the panel (each item: the reason, then an Apply button when `patch` is not null; an error line under the list when `error` is set) and the app wiring (Apply calls `apply` and shows a refused patch's message in the panel).
- [ ] **Step 4: Docs.**
  - README: one or two sentences after the GUI paragraph's image sentence: the page lists suggestions for a new image (line art, levels, invert, noise, page orientation, colour) with an Apply button; `plotline analyse IMAGE` prints the same as JSON; the tone stage's Levels can be Auto. In the MCP paragraph, mention `analyse_image`.
  - Brief §9: the tool list gains `analyse_image`, "eleven tools".
  - Brief §14: append rows "Image analysis", "Auto levels" and "Suggestion cut-offs" with spec §9's decisions and reasons.
- [ ] **Step 5: Run** `npm run check`, `npm run build`, the full Rust and Python checks, and a manual check in `npm run dev`: open the sample image, then a landscape image, see the orientation suggestion, Apply it, see the page turn and the suggestion disappear.
- [ ] **Step 6: Commit** `Show image suggestions in the GUI and document the analysis`.
