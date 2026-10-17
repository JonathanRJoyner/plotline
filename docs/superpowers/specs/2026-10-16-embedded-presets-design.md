# plotline spec: the run's settings embedded in exports

Date: 2026-10-16. Status: written for review. The owner delegates the choices, which are recorded in the brief's decision log.

An exported drawing does not record how it was made, so its look comes back only if someone also saved a preset. This spec embeds the preset of the run that made a drawing in its SVG and JSON exports, and lets the preset loaders read such a file. Brief §12, question 8 ("Additions after 0.1") lists this.

## 1. Scope

- `crates/core/src/types.rs`: `Drawing` gains `preset: Option<Value>`.
- `crates/core/src/runner.rs`: a run's drawing carries the preset of the run's own snapshot.
- `crates/core/src/export/svg.rs` and `export/json.rs`: an optional preset block.
- `crates/core/src/presets.rs`: `from_text`, which reads a preset, an exported SVG or an exported JSON drawing.
- `crates/core/src/session.rs`: `svg()` and `json()` embed the drawing's preset with the session's machine.
- Bindings: `preset_from_text` in `crates/py`, `presetFromText` in `crates/wasm`. `Session.load_preset` reads all three kinds of file, and so do the CLI's `--preset` and the MCP `load_preset` tool, which call it.
- GUI: Load preset takes `.json` and `.svg`; the "Keep a look" tour step's text and a Playwright flow.
- Docs: brief §9, §12 and §14, the README, and a CHANGELOG line under a new "Unreleased" heading.

Out of scope: G-code and PNG exports, which carry no preset, and a setting to leave the block out.

## 2. Which settings

A drawing's preset is the snapshot its run started from, never the session's current state. Settings can change while a run is under way, and a budget fit leaves the settings alone while its drawing comes from a trial value.

- The runner builds `Drawing.preset` when it assembles the drawing: `presets::to_value(&style.name, canvas, pens, None, &settings)` from the run's own style, settings and canvas and pens roots. The run holds no machine, so `machine` is null there.
- A fit's trials are ordinary runs, so the drawing a fit keeps carries the fitted value.
- Drawings made any other way, such as by test helpers, have `preset: None`.

## 3. Exports

`svg::to_svg(d, preset: Option<&Value>)` and `json::to_value(d, preset: Option<&Value>)` take the block to embed. With `None` they write exactly what they write today, so the golden files do not change. `Session::svg()` and `Session::json()` pass the drawing's preset with `machine` set to `json!(self.machine)`, so loading the file later keeps the user's machine rather than clearing it.

- **SVG:** on its own line right after the opening `<svg>` tag, `<metadata id="plotline-preset">` holds the preset as compact JSON (`serde_json::to_string`). `&`, `<` and `>` are written as entities, and `</metadata>` closes it. `<metadata>` is plain SVG with no vendor namespace, so it keeps the naming rule (§12, question 1).
- **JSON:** the `plotline-paths` object gains a top-level `preset` key. `version` stays 1, because a reader that ignores unknown keys reads the file as before.

## 4. Reading a file as a preset

`presets::from_text(text: &str) -> Result<Value, PresetError>` returns the preset a file holds, and `Session::load_preset` applies it as today.

- **SVG,** when the first non-whitespace character is `<`:
  - It finds `id="plotline-preset"`, the `>` that closes that tag, and the next `</metadata>`.
  - It strips a `<![CDATA[ … ]]>` wrapper, if an editor added one.
  - It unescapes `&lt;`, `&gt;`, `&quot;`, `&apos;` and decimal and hex character references, then `&amp;` last.
  - It parses the JSON. Without the element it refuses: "this SVG carries no plotline preset".
- **JSON,** otherwise. It parses the text, and refuses text that is neither JSON nor SVG: "not a preset: it is neither JSON nor an SVG".
  - A value whose `format` is `plotline-paths` gives its `preset` key, or refuses: "this drawing carries no preset".
  - Any other value is returned as it is, and `presets::parse` checks it.

The refusals are new `PresetError` variants, so the hosts report them as refused values (`ValueError` in Python).

## 5. Front ends

- **Python:** `_core.preset_from_text(text) -> dict`. `Session.load_preset(path)` reads the file as UTF-8 text, passes it through `preset_from_text`, and its docstring names the three kinds of file.
- **CLI:** the `--preset` help says it takes a preset, an exported SVG or an exported JSON drawing.
- **MCP:** the `load_preset` docstring and brief §9 say the same.
- **GUI:** the Load preset input accepts `application/json,.json,image/svg+xml,.svg`.
  - The page sends the file's text to a new worker command, `loadPresetText(text)`, which runs `s.loadPreset(presetFromText(text))`.
  - The remembered setup still uses `loadPreset` with an object.
  - A refusal shows in the presets panel, and the "is not valid JSON" message goes.

## 6. Tests

- **Core:**
  - The SVG and JSON exports carry the block when given one, and are unchanged without it.
  - `from_text` reads a preset, an exported SVG, an exported JSON drawing and a CDATA-wrapped block. It refuses an SVG without the block, a paths file without one, and text that is neither.
  - Round trip: a session runs a style with a changed setting and exports both formats. A fresh session loads each, and the style, page, pens and settings match.
  - A pen named `A & <B>` survives the round trip.
  - After a budget fit, the SVG's preset holds the fitted value.
- **Python:** `load_preset` on an exported SVG and on an exported JSON drawing.
- **CLI:** `--preset drawing.svg` gives the same SVG as the run that wrote it.
- **MCP:** `load_preset` on an exported SVG.
- **GUI:**
  - A protocol test for `loadPresetText`.
  - A Playwright flow: change a setting, export the SVG, reset to defaults, load the SVG in the presets panel, and find the setting back.
