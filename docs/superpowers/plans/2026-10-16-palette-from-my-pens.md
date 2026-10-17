# A palette from your own pens: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The palette fit picks from the user's saved pens when there are any, and a `pens_to_add` note lists up to three listed inks the user lacks that would draw the image clearly better.

**Architecture:** The engine changes in `crates/core/src/analysis.rs`: the fit takes its candidates as a parameter, and a one-step lookahead finds the additions. `Session` holds the library and measures the image again when the library changes. The wasm, PyO3, Python, CLI and MCP layers each pass a list of pens through. The GUI lifts its existing library state into `app.tsx` and sends it to the worker.

**Tech Stack:** Rust (core, wasm-bindgen, PyO3), Python (click, MCP SDK), TypeScript and Preact (web), Playwright.

**Spec:** `docs/superpowers/specs/2026-10-16-palette-from-my-pens-design.md`

## Global Constraints

- With an empty library, every result is unchanged, the palette's JSON shape aside. The gallery stats snapshot must not move.
- Stop rule and gain: `PALETTE_GAIN = 2.3`, at most `PALETTE_MAX = 4` colours, at most 3 additions.
- Library colours are compared without case. Ties go to the earlier candidate, in library order or `INKS` order.
- Ships to wasm: use ASCII case methods only, and add no new collection types (from the spec 13 size traps).
- No product names in copy. Additions are named after `INKS` names only.
- Persisted text (code comments, docs, changelog) is plain English, not terse.

## Review Focus

1. **A library whose colours all match the inks exactly** gives no additions and a normal `colour` suggestion. Owned by Task 1, test `a_library_of_every_ink_has_no_additions`.
2. **A library of only black pens on a colour image** gives no `colour` suggestion and a `pens_to_add` note. Owned by Task 1, test `black_pens_only_get_pens_to_add`.
3. **The library changes after the image is loaded**: the suggestions follow without a reload. Owned by Task 2, test `a_new_library_measures_the_image_again`.
4. **A worker trap rebuilds the session**: the library survives the rebuild. Owned by Task 5, test in `protocol.test.ts`.
5. **A bad library file on the CLI** gives a one-line error and no traceback. Owned by Task 4, test `test_analyse_refuses_a_bad_library`.

---

### Task 1: Engine fit over candidates, additions, suggestions

**Files:**
- Modify: `crates/core/src/analysis.rs`, which covers `ImageStats`, `fit_palette`, `measure`, `suggest`, its tests, and the module and field docs.

**Interfaces:**
- Produces:
  - `pub struct PaletteInk { pub name: String, pub color: String }` (Serialize, Clone, Debug, PartialEq).
  - `pub struct PaletteAddition { pub name: String, pub color: String, pub error: f64 }`.
  - `ImageStats.palette: Vec<PaletteInk>` and `ImageStats.palette_additions: Vec<PaletteAddition>`.
  - `pub fn measure_with(image: &Image, library: &[Pen]) -> ImageStats`. `measure(image)` stays as `measure_with(image, &[])`, so the 20-odd test callers and `lint.rs` keep working.
  - `pub fn suggest(stats, canvas, style, settings, pens: &[Pen], library: &[Pen]) -> Vec<Suggestion>`, with the new parameter last.

- [ ] **Step 1: Write the failing tests** in `analysis.rs` `mod tests`. Add a helper, `fn lib(specs: &[(&str, f64, &str)]) -> Vec<Pen>`. The tests are:
  - `a_library_palette_uses_only_library_colours`: on `color_bars(256, 64)` with a library of yellow, blue, pink and black (`INKS` hexes, names like "My yellow"), every `palette` entry's `color` is a library colour and its `name` is that pen's name.
  - `the_closest_width_draws_a_colour`: the library has black at 0.1 named "B1", black at 0.5 named "B5", and yellow at 0.3. With session pens at width 0.45, the `colour` patch's black pen is "B5". With a library holding two blacks at 0.2 and 0.4 and session width 0.3, it is the earlier one.
  - `black_pens_only_get_pens_to_add`: a library of black only, on the colour bars, gives no `colour` suggestion. It gives a `pens_to_add` suggestion with `patch == None`. `palette_additions` has 1 to 3 entries, none `#000000`, each with `palette_error - error >= 2.3`, sorted by `error` ascending.
  - `a_library_of_every_ink_has_no_additions`: with all ten `INKS` as the library, `palette_additions` is empty.
  - `no_library_keeps_the_ink_palette`: `measure(&color_bars(256, 64))` gives palette names equal to the existing expected list. Update the existing tests at about lines 913 and 932, which compare names, to map `.name`.
  - `the_reason_names_owned_pens`: the library `colour` reason contains "Your pens " and each chosen pen's name.

- [ ] **Step 2: Run them and check they fail**

  Run: `cargo test -p plotline-core --lib analysis`

  Expected: compile errors (`measure_with`, `palette_additions` and the 6-argument `suggest` do not exist yet).

- [ ] **Step 3: Implement**
  - `fit_palette(samples: &[Sample], candidates: &[(&str, &str)]) -> (Vec<usize>, f64)` returns the chosen candidate indices in candidate order, plus the error. It keeps today's loop and tie rule, with `INKS` replaced by `candidates`.
  - Lightest-first sorting moves to the caller and is stable, so ties stay in candidate order.
  - `measure_with` builds `candidates`:
    - `INKS.to_vec()` when the library is empty;
    - otherwise, the library's distinct colours in order of first appearance (compared with `eq_ignore_ascii_case`), each named after the first pen with that colour.
  - It runs `additions` only when the library is not empty.
  - Add `fn additions(samples, candidates, chosen: &[usize], error: f64, library: &[Pen]) -> Vec<PaletteAddition>`, written to spec §1.3:
    - for each `INKS` entry whose hex matches no library colour, try `chosen` plus the ink (if fewer than 4), and `chosen` with each member replaced by the ink;
    - the ink's error is the lowest of those;
    - keep it if the gain is at least `PALETTE_GAIN`;
    - sort by error with `total_cmp` and a stable sort, so ties stay in `INKS` order;
    - keep the first 3.
  - In `suggest`, change the `colour` branch:
    - the palette colours come from `PaletteInk.color`, not an `INKS` lookup;
    - with a library, each pen is the library pen of that colour (ignoring case) whose `width_mm` is closest to `pens.first()`'s width (0.3 with none), with ties to the earlier pen;
    - it keeps that pen's own name, width and colour;
    - without a library, the pens are as today;
    - the reason with a library reads `"This is a colour image. Your pens {a, b and c} draw its colours best, with Pen use set to by colour."`, joining names as the current code does but without capitalising them.
  - Add `pens_to_add` after `colour`, made when `palette_additions` is not empty. Its reason, with the numbers written `{:.1}`:
    - with one ink: `"Pens you don't have would draw its colours closer: with {an|a} {ink} pen the mean colour difference falls from {e0:.1} to {e1:.1}."`
    - each further ink adds `", and with a {ink} pen to {e:.1}"` before the full stop;
    - "an" goes before orange only, since no other ink name starts with a vowel.
  - Update the doc comments: `ImageStats::palette`, `palette_additions`, `suggest`'s new parameter, and the module doc's last sentence.

- [ ] **Step 4: Run the tests and check they pass**

  Run: `cargo test -p plotline-core --lib analysis`. Expected: all pass.

  Then run `cargo test -p plotline-core` and fix the callers it breaks (`session.rs` `suggest` call, `session.rs:755` test).

- [ ] **Step 5: Commit**

  `git commit -am "Fit the palette to the pen library and list inks worth adding"`

### Task 2: Session holds the library

**Files:**
- Modify: `crates/core/src/session.rs` (field, `set_pen_library`, `set_image`, `analysis`, tests).

**Interfaces:**
- Consumes: `measure_with` and the 6-argument `suggest` from Task 1.
- Produces:
  - `pub fn set_pen_library(&mut self, pens: Vec<Pen>)`. It cannot fail: the pens are already valid.
  - `pub fn pen_library(&self) -> &[Pen]`.

- [ ] **Step 1: Write the failing tests**
  - `a_new_library_measures_the_image_again`: set `color_bars` with no library, and keep the analysis's palette names. Then call `set_pen_library` with a black pen and a yellow pen named "Lemon". The palette is now drawn only from those colours, and `pens_to_add` appears.
  - `the_library_is_not_in_the_preset`: after `set_pen_library`, `preset()` has no library key and no added pens.

- [ ] **Step 2: Run them and check they fail**

  Run: `cargo test -p plotline-core --lib session::tests::a_new_library`. Expected: compile error.

- [ ] **Step 3: Implement**
  - Add the field `pen_library: Vec<Pen>`, with a doc comment saying no stage, preset or undo history reads it, and `Vec::new()` in `new`.
  - `set_image` uses `measure_with(&image, &self.pen_library)`.
  - `set_pen_library` stores the pens and, when there is an image, sets `image_stats = Some(measure_with(image, &self.pen_library))`.
  - `analysis` passes `&self.pen_library`.

- [ ] **Step 4: Run the tests and check they pass**

  Run: `cargo test -p plotline-core`. Expected: all pass.

- [ ] **Step 5: Commit**

  `git commit -am "Keep the pen library in the session"`

### Task 3: The law

**Files:**
- Modify: `crates/core/tests/laws.rs`.

- [ ] **Step 1: Write the law** `a_library_palette_uses_owned_pens`:
  - Use `check("palette library", 128, (picture(), library()), ...)`.
  - `library()` is 1 to 6 pens: the width drawn as in `pens()`, and the colour either an `INKS` hex (half the time) or any `[u8; 3]`, named `"P{i}"`.
  - Build a `Session` with `set_pen_library` and then `set_image`, and check three things:
    - every pen in the `colour` patch equals a library pen (name, width, colour ignoring case);
    - every `palette_additions` colour matches no library colour;
    - every addition has `palette_error - error >= 2.3`.
  - Then apply the `colour` patch with the existing `apply` helper and check that `colour` is gone.

- [ ] **Step 2: Run it**

  Run: `cargo test -p plotline-core --test laws a_library_palette`.

  Expected: PASS. A failure is a real bug in Task 1: fix the engine, not the law.

- [ ] **Step 3: Commit**

  `git commit -am "Add the library palette law"`

### Task 4: Wasm, Python binding, Python session, CLI, MCP

**Files:**
- Modify:
  - `crates/wasm/src/lib.rs`: `setPenLibrary`, beside `setPens`.
  - `crates/py/src/session.rs`: `set_pen_library`, beside `set_pens`.
  - `src/plotline/session.py`: `set_pen_library`; the `analysis` docstring describes `palette`, `palette_additions` and `pens_to_add`.
  - `src/plotline/cli/main.py`: `analyse --library FILE`, plus `read_library(path) -> list[Pen]`.
  - `src/plotline/mcp_server.py`: the `set_pen_library` tool, and the `analyse_image` docstring and `INSTRUCTIONS`.
  - `docs/BRIEF.md`: the MCP tool list near line 167, and the "MCP tools" row count.
- Test: `tests/test_session.py`, `tests/test_cli.py`, `tests/test_mcp.py`.

**Interfaces:**
- Consumes: `Session::set_pen_library(Vec<Pen>)`.
- Produces:
  - wasm `setPenLibrary(json: &str) -> Result<(), JsError>`, which validates each pen with `Pen::new` like `setPens` and accepts `[]`.
  - Python `Session.set_pen_library(pens: Sequence[Pen]) -> None`, which raises `ValueError` for a bad pen.
  - MCP `set_pen_library(pens: list[PenSpec]) -> TextContent`, which takes ink names like `set_pens` and returns the library as a JSON array.

- [ ] **Step 1: Write the failing tests**
  - Python, `test_a_pen_library_chooses_the_palette`: on a colour image, after `set_pen_library` with black and a yellow named "Lemon 0.5", every pen in the `colour` patch is one of the two. A bad colour raises `ValueError`.
  - CLI, `test_analyse_reads_a_library`: write `{"format": "plotline-pens", "version": 1, "pens": [black, yellow]}` to `tmp_path`, run `analyse IMG --library FILE`, and check the JSON's palette names come from the file.
  - CLI, `test_analyse_refuses_a_bad_library`: `{"format": "x"}` exits non-zero with `"could not be imported (not a pen library: its format must be 'plotline-pens')"`. Invalid JSON gives `"is not a pen library: it is not valid JSON"`.
  - MCP: `set_pen_library` then `analyse_image` uses library pens, and `test_stdio_server_lists_the_tools` gains `"set_pen_library"`.

- [ ] **Step 2: Run them and check they fail**

  Run: `maturin develop` (or the project's build command from CONTRIBUTING.md), then `pytest tests/test_session.py tests/test_cli.py tests/test_mcp.py -k "library or lists_the_tools"`. Expected: FAIL.

- [ ] **Step 3: Implement** the four layers.

  `read_library` uses the GUI's messages (see `web/src/library.ts`):
  - `"{name} is not a pen library: it is not valid JSON."`
  - `"{name} could not be imported ({reason})."`, where the reason is one of:
    - `not a pen library: its format must be 'plotline-pens'`
    - `unsupported pen library version {v}`
    - `not a pen library: it has no list of pens`

  It raises `ValueError` so that `_errors()` turns it into a `ClickException`. Pen fields are checked by the engine through `set_pen_library`.

- [ ] **Step 4: Run the tests and check they pass**

  Run the same command as Step 2 and then the whole of `pytest`. Expected: all pass.

- [ ] **Step 5: Commit**

  `git commit -am "Take a pen library in wasm, Python, the CLI and MCP"`

### Task 5: GUI

**Files:**
- Modify:
  - `web/src/protocol.ts`: the `setPenLibrary` method. It is left out of `REDRAWS`. The protocol keeps `private library = "[]"`, and `recover` re-applies it after `loadPreset`.
  - `web/src/app.tsx`: `const [library, setLibrary] = useState(loadLibrary)`. A `useEffect` on `library` calls `call("setPenLibrary", [library], "pens")`. `PensDialog` gets `library` and `onLibrary={(l) => { setLibrary(l); saveLibrary(l); }}`.
  - `web/src/panels.tsx`: `PensDialog` drops its own `useState(loadLibrary)` and `saveLibrary` and uses the props. `keep` becomes `onLibrary`.
  - `web/src/tour.tsx`: one sentence is appended to the "Take a suggestion" step: `"With pens saved under My pens it picks from those, and a note names any ink worth adding."`
- Test: `web/src/protocol.test.ts`, `web/e2e/gui.spec.ts`.

- [ ] **Step 1: Write the failing tests**
  - `protocol.test.ts`:
    - `setPenLibrary` changes `state.suggestions` (a colour image gains `pens_to_add` with a black-only library) and starts no run;
    - after a forced `recover`, the library still holds. Follow the file's existing pattern for recovery tests.
  - `gui.spec.ts`, `"pens saved under My pens choose a colour image's palette"`:
    1. Open the pens dialog.
    2. Set pen 1 to yellow and save it.
    3. Set it to blue and save it.
    4. Close the dialog.
    5. Load the colour image the existing palette flow makes.
    6. Click the `colour` suggestion's Apply.
    7. Check that the pen list holds only the saved pens' names.
    8. Check that a suggestion containing "Pens you don't have" is visible.

- [ ] **Step 2: Run them and check they fail**

  Run: `npm run wasm && npm test -- protocol` in `web/`. Expected: FAIL.

- [ ] **Step 3: Implement** the files listed above.

- [ ] **Step 4: Run the tests and check they pass**

  Run: `npm run check`, `npm run build` and `npm run e2e` in `web/`. Expected: all pass.

  Then check by hand in the preview: save two pens, open a colour image, and confirm the suggestions read right.

- [ ] **Step 5: Commit**

  `git commit -am "Send My pens to the engine so suggestions use them"`

### Task 6: Records, sizes and timings, PR

**Files:**
- Create:
  - `docs/decisions/2026-10-16-palette-from-my-pens.md`, in the format of the existing decision files, with an Amends line for `2026-10-16-pen-library.md` and `2026-10-16-pen-palette.md`;
  - `changelog.d/palette-from-my-pens.md`, in the format of `changelog.d/README.md`.
- Modify: the spec's §2, where the signature becomes `measure_with`, and its §4 CLI message, which takes the GUI's wording.

- [ ] **Step 1: Measure**
  - Time `measure` natively on `synthetic::scene(1024, 768)`, the median of 7 release runs, with no library and with a 20-colour library.
  - Measure the wasm size with `npm run wasm` against an `origin/main` build.
  - Put the numbers in the decision file.

- [ ] **Step 2: Check the full suite**
  - `cargo test --workspace`
  - `cargo clippy --workspace -- -D warnings`
  - `cargo fmt --check`
  - `pytest`
  - the web checks
  - the gallery stats snapshot check from CONTRIBUTING.md

  The snapshot must not change.

- [ ] **Step 3: Commit, push, and open the PR** against `main`. Then bind it with the ccd_pr tools.
