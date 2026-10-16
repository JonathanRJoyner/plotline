# plotline spec 12: settings across a style switch

Date: 2026-10-15. Status: decided under the owner's standing delegation; written for review.

This settles the brief's open question 8. Until now `Session::set_style` reset every stage to the new style's defaults. The six Voronoi styles therefore reused each other's cached points only at the default settings: change the relaxation on `stipple`, switch to `line`, and the slow stage ran again. A switch also put Pen use back to one pen.

## 1. Rule

On a style switch, each stage of the new style starts from the new style's defaults. Then, for a stage the old style also has, every value the user changed from the old style's default is kept. A value still at the old style's default takes the new style's default.

- **Shared** means the same stage name. The registry test "Unique stage names" already guarantees that stages sharing a name share their schema, so a kept value is always valid in the new style.
- **Changed** compares numbers by value, so `1` and `1.0` are the same. A front end may send either: JavaScript writes `1.0` as `1`.
- **Style overrides keep working.** `waves` and `spiral` set `tone.gamma` to 1.9 and `ink` sets `tone.resolution` to 2500. A gamma left at `hatch`'s default of 1 is not a change, so a switch from `hatch` to `waves` gives 1.9. A gamma the user set to 1.4 stays 1.4.
- **Pen use carries.** Every style has the `split` stage, so its mode survives every switch.
- Switching to the style already chosen changes nothing.

There is no memory per style: settings of a stage the new style lacks are dropped, as before. A switch from `stipple` to `line` and back gives `stipple`'s dot settings their defaults again.

## 2. What does not change

- **Presets load as before.** `load_preset` sets the preset's style with every stage at its defaults, then applies the preset's settings, so a key the preset leaves out takes its default whatever the session held.
- **The cache is untouched.** The points are reused because the kept settings give the same cache key, not because of any new cache code.

## 3. Where it reaches

- **Core.** `Session::set_style` in `crates/core/src/session.rs` applies the rule; `load_preset` resets to the defaults after setting the style. The bindings in `crates/py` and `crates/wasm` pass through, with their doc comments updated.
- **Python.** `Session.set_style` in `src/plotline/session.py` documents the rule.
- **CLI.** `plotline run STYLE --preset FILE` used to skip `set_style` when the preset's style matched, because it reset the settings. A switch to the same style is now a no-op, so the check goes. With a preset of another style, the preset's changed shared settings now carry into STYLE.
- **MCP.** A `run` with a different style carries the changed settings instead of resetting them. When a change is refused, the server still puts back the old style and settings exactly: it switches back, then reapplies every stage's old values. The tool's docstring and the brief's §9 say so.
- **GUI.** The style select calls the engine's `setStyle`, so it gains the rule with no code change. The tour's "Pick a style" step says that changed settings carry over.
- **Docs.** The README's style section, the brief's §3 first-release criterion, §6.1, §9, §12 (question 8 removed), the "Shared Voronoi defaults" row, and a new decision-log row.

## 4. Tests

- Core: set the relaxation's iterations on `stipple`, run, switch to `line`, run, and assert the `voronoi` stage's only frame is `cached`.
- Core: from `hatch` at its defaults, switch to `waves` and assert `tone.gamma` is 1.9; a gamma changed on `hatch` stays.
- Core: a changed Pen use mode survives a switch; an integer `1` for a default of `1.0` counts as unchanged.
- Core: `load_preset` still gives the defaults for keys the preset leaves out, after a switch that carried changes.
- `crates/core/tests/settings_matter.rs` compares each setting's drawing against the style's defaults, so its helper now resets every stage after the switch.
- Python: `tests/test_session.py` asserts the carry through the binding. MCP: `tests/test_mcp.py` asserts that a `run` with a new style keeps a changed setting.
