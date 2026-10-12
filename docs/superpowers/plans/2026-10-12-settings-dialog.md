# Simpler machine and settings dialog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the machine's bed and four area bounds with a corner and a size, keep old presets loading, and move the machine, pen lift, G-code and plot speeds into a settings dialog opened from the toolbar.

**Architecture:** The engine change is confined to `Machine`/`Area` in `crates/core/src/machine.rs` and a preset migration from version 2 to 3. The G-code writer reads the area only through `Machine::mapping`, so its output is unchanged. In the GUI, `MachinePanel` and `SpeedsPanel` become sections of one `SettingsDialog`, a native `<dialog>` that sends the same `setMachine` and `setSpeeds` calls as before.

**Tech Stack:** Rust (serde, serde_json), TypeScript, Preact, lucide-preact, Vite, vitest.

**Spec:** `docs/superpowers/specs/2026-10-12-settings-dialog-design.md`

## Global Constraints

- No new runtime dependency. `<dialog>` is native, and the gear icon comes from the installed `lucide-preact`.
- Copy follows spec section 4 word for word: the help lines and the select's "None (export SVG only)".
- The A1 G-code is unchanged. The existing exact-string tests in `gcode.rs` must pass without being edited.
- Don't name other plotter programs anywhere: not in code, comments, docs or commits.
- Rust: `cargo fmt` and `cargo clippy --all-targets -- -D warnings` stay clean.

## Review Focus

- A preset file saved before this change, with the A1, must load and give the new A1. The migration test pins this.
- If the browser remembers an old preset, a reload must keep the setup and not fall back to defaults. This is the same migration path, covered by the same test.
- An area with a negative offset must validate and map correctly. A test pins this in Task 1.
- An area of NaN or zero width must be refused with the field named. A test pins this in Task 1.
- With a machine set, Plot time must not be editable, so it cannot drift from the feeds. A manual check covers this in Task 2.

---

### Task 1: Engine: area as a corner and a size, preset version 3

**Files:**
- Modify: `crates/core/src/machine.rs`
- Modify: `crates/core/src/export/gcode.rs` (tests module only)
- Modify: `crates/core/src/presets.rs`
- Modify: `tests/test_presets.py`

**Interfaces:**
- Produces:
  - `pub struct Area { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }`, with `deny_unknown_fields` and `Area::contains([f64; 2], eps) -> bool`.
  - The methods `Area::width()` and `Area::height()` are removed. Use the fields instead.
  - `Machine` has no `bed_mm`.
  - `PRESET_VERSION == 3`.

- [ ] **Step 1: Update the tests in `machine.rs`.**
  - `the_a1_is_built_in_with_the_owners_values` asserts `(m.area.x, m.area.y, m.area.width, m.area.height) == (28.0, 42.0, 200.0, 174.0)` and drops the `bed_mm` assert.
  - `validation_names_the_field` replaces the `x_max = 300` case with two cases: `m.area.width = 0.0` gives an error containing `area.width`, and `m.area.x = f64::NAN` gives one containing `area.x`.
  - Add `a_negative_offset_is_accepted`: `m.area.x = -100.0; m.area.y = -100.0`, then `m.validate().unwrap()` and `m.mapping(200.0, 174.0).unwrap().to_machine([0.0, 0.0]) == [-100.0, 74.0]`.
  - The mapping tests keep their numbers.
- [ ] **Step 2: Update the tests in `presets.rs` and `tests/test_presets.py`.**
  - `round_trip` asserts `v["version"] == 3`.
  - `rejects_bad_shapes_with_named_fields` uses `json!(4)` and expects `reads up to 3`.
  - `the_machine_round_trips` asserts `v["machine"]["area"]["x"] == 28.0`.
  - Add `a_version_2_preset_migrates_the_machine`. It parses a version 2 preset whose machine is the spec 4 A1: `"bed_mm": [256, 256]`, `"area": {"x_min": 28, "x_max": 228, "y_min": 42, "y_max": 216}` and the other A1 fields. It asserts `parse(..).unwrap().machine == Some(builtin().remove(0))`.
  - Python: the saved version moves to `3` and the unsupported version to `4`.
- [ ] **Step 3: Run the tests and check they fail.**
  - Run: `cargo test -p plotline-core machine presets`
  - Expected: compile errors on `area.x` and the other new fields.
- [ ] **Step 4: Implement `Area` and `Machine` in `machine.rs`.**
  - Change `Area` to the fields above and remove `bed_mm`.
  - `contains` uses `x..=x + width` and `y..=y + height`, each with `eps` of slack.
  - `validate` replaces the bed and area check with these, in this order:
    - `area.x` and then `area.y` must be finite: `"area.x must be finite, got {v}"`.
    - `area.width` and then `area.height` must be positive and finite: `"area.width must be positive, got {v}"`.
  - `mapping`: `ox = x + (width - w) / 2`, `oy = y + (height + h) / 2`. The refusal message is unchanged.
  - Update the module doc and the field docs. `Area`'s doc says "`x` and `y` are the corner with the smallest X and Y".
  - Update the built-in A1 to `Area { x: 28.0, y: 42.0, width: 200.0, height: 174.0 }`.
- [ ] **Step 5: In the `gcode.rs` test `check`, drop the "outside the bed" assert and its mention in the doc comment.**
- [ ] **Step 6: Add the migration in `presets.rs`.**
  - Set `PRESET_VERSION = 3` and `MIGRATIONS = &[None, Some(v1_to_v2), Some(v2_to_v3)]`.
  - Write `fn v2_to_v3(v: Value) -> Value`. When `v["machine"]` is an object, it removes `bed_mm`. When `area` holds all four numbers `x_min`, `x_max`, `y_min` and `y_max`, it replaces `area` with `{x: x_min, y: y_min, width: x_max - x_min, height: y_max - y_min}`. Anything else stays as it is.
  - Give it a doc line in the style of `v1_to_v2`: "Version 3 gave the machine's area a corner and a size and dropped the bed".
- [ ] **Step 7: Run the tests and check they pass.**
  - Run: `cargo test --workspace`. Expected: all pass, and the `gcode.rs` exact-string tests are untouched.
  - Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`. Expected: clean.
  - Run: `uv sync --reinstall-package plotline` then `uv run pytest`. Expected: all pass.
- [ ] **Step 8: Commit.** Message: `Give the machine's area a corner and a size, drop the bed, and migrate presets to version 3`.

### Task 2: GUI: settings dialog

**Files:**
- Modify: `web/src/events.ts`
- Modify: `web/src/panels.tsx` (replace `MachinePanel` and `SpeedsPanel`)
- Modify: `web/src/app.tsx`
- Modify: `web/src/style.css`

**Interfaces:**
- Consumes: the Task 1 machine JSON, `{name, area: {x, y, width, height}, z_down_mm, …}`.
- Produces:
  ```ts
  export type Area = { x: number; y: number; width: number; height: number };
  // Machine: as before without bed_mm
  type SettingsProps = {
    open: boolean; onClose: () => void;
    machine: Machine | null; builtins: Machine[]; error?: string; speeds: Speeds;
    onPick: (machine: Machine | null) => void; onEdit: (machine: Machine) => void; onSpeeds: (speeds: Speeds) => void;
  };
  export function SettingsDialog(props: SettingsProps): JSX.Element
  ```

- [ ] **Step 1: In `events.ts`, change `Area` as above and remove `bed_mm` from `Machine`.**
- [ ] **Step 2: Write `SettingsDialog` in `panels.tsx`.** It replaces `MachinePanel` and `SpeedsPanel` and keeps their helpers: `num`, `same`, `SPEEDS` and the speed guard.
  - A `ref` to a `<dialog>`. An effect on `open` calls `showModal()` or `close()` when the element's state differs. The dialog's `onClose` event calls `onClose`, so Esc keeps the app's state in sync.
  - Header: a gear icon, "Settings", and a Close button with `aria-label="Close"` that calls `onClose`.
  - Body: a `<nav>` of four buttons for the sections in `const SECTIONS = ["Machine", "Pen lift", "G-code", "Plot time"] as const`. The open one has `aria-current="true"`. Local state holds the open section and starts on "Machine".
  - Then `error` as `<p class="error">`, then the fields of the open section, as spec section 4 lists them.
  - Area inputs edit `machine.area.{width,height,x,y}`. Their labels are Width, Height, Offset X and Offset Y.
  - Pen lift and G-code without a machine show `<p>Choose a machine first</p>`.
  - Plot time with a machine shows the three values as disabled inputs and the line "From the machine's feeds and pen lift."
  - Text areas get `class="code"` and `rows={6}`.
- [ ] **Step 3: Wire it up in `app.tsx`.**
  - Import `Settings as Gear` from `lucide-preact`, since `Settings` is already the stage form.
  - Add `const [settingsOpen, setSettingsOpen] = useState(false)`.
  - Add a toolbar button after `<span class="spacer" />` and before `<ThemeButton />`: `<button type="button" onClick={() => setSettingsOpen(true)}><Gear /> Settings</button>`.
  - Remove `MachinePanel` and `SpeedsPanel` from the right `<aside>`.
  - Render `<SettingsDialog …/>` as the last child of `.app`, passing `pickMachine`, `call("setMachine", [m], "machine")` and `call("setSpeeds", [s])`.
  - In `pickMachine`, set the page from `machine.area.width` and `machine.area.height`.
- [ ] **Step 4: Add dialog styles to `style.css`, using the existing tokens.**
  - `dialog.settings`: width `min(720px, 100vw - 32px)`, max-height `70vh`, the `--surface` background, the `--border` border and `--radius`, padding 0, and flex in a column.
  - `::backdrop`: `rgb(0 0 0 / 0.4)`.
  - Body: a grid of `140px minmax(0, 1fr)`, with the fields pane at `overflow-y: auto`.
  - `nav button[aria-current]`: the `--accent-soft` background.
  - `textarea.code`: `ui-monospace, monospace`, at full width.
  - `@media (max-width: 600px)`: the body becomes one column.
- [ ] **Step 5: Run the checks and the build.**
  - Run in `web/`: `npm ci`, `npm run wasm`, `npm run check`, `npm run build`.
  - Expected: the type check is clean, all vitest tests pass, and the build writes `src/plotline/web/index.html`.
- [ ] **Step 6: Check it by hand in `npm run dev`.**
  - The dialog opens from the toolbar, and Esc and Close both close it. Reopening works after either.
  - Picking the A1 sets the page to 200 by 174 mm.
  - Editing Draw feed changes Plot time's Draw value, and Plot time is disabled.
  - Pen up Z set below Pen down Z shows the `z_up_mm` message.
  - With None, Pen lift and G-code say "Choose a machine first" and Plot time is editable.
  - The A1 G-code export of the sample image downloads, and its first lines match the `gcode.rs` expectations.
  - At 375 px wide, nothing scrolls sideways.
- [ ] **Step 7: Commit.** Message: `GUI: machine, pen lift, G-code and plot time in a settings dialog`.

### Task 3: Docs

**Files:**
- Modify: `docs/BRIEF.md` (decision log, after "GUI look")
- Modify: `README.md:11`
- Modify: `docs/superpowers/specs/2026-10-12-machines-gcode-design.md` (section 2)

- [ ] **Step 1: In the brief, add three rows: "Machine area", "Settings dialog" and "Preset version 3".** Take the wording from spec section 6 and give the date 2026-10-12.
  - Amend the "Machine in the preset" row with "Amended on 2026-10-12, see "Preset version 3"".
- [ ] **Step 2: Update README line 11.** "A machine panel holds the drawable area, pen heights and feeds" becomes "A settings dialog holds the machine: its drawable area, pen heights, feeds and start and end G-code". The rest of the sentence stays.
- [ ] **Step 3: Add a note under spec 4's section 2 heading.** It reads: "Superseded in part by [spec 7](2026-10-12-settings-dialog-design.md): the area is now a corner and a size, and there is no bed."
- [ ] **Step 4: Commit.** Message: `Record the machine area, settings dialog and preset version 3 in the brief`.
