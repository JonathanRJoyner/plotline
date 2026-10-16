# Machines and G-code from the CLI and MCP

Status: built on 2026-10-15. Decisions are recorded in the brief's decision log under "Machines outside the GUI".

## Goal

Until now only the GUI could set a machine and export G-code. The MCP spec left both out ("a later spec adds them to the CLI and the MCP together"). This spec adds them. The engine already has everything: `Session::set_machine`, `Session::machine`, `Session::gcode(layer)` and `export::gcode`, which writes one pen layer as strict G-code. The work is the binding, the Python session and the two front ends. No algorithm code goes into a front end.

## Python

The binding gains `set_machine(machine | None)`, `machine()` and `gcode(layer)` over the engine's methods. A machine crosses the binding as a dict in the preset's `machine` shape: `name`, `area: {x, y, width, height}`, `z_down_mm`, `z_up_mm`, `z_safe_mm`, `draw_feed`, `travel_feed`, `z_feed`, `settle_ms`, `start_gcode`, `end_gcode`. The engine checks it, and a refused value raises `ValueError` with the engine's message, which names the field.

`plotline.Session` gains:

- `machine`: the machine as a dict, or `None`.
- `set_machine(machine)`: use the machine, or `None` for none. Like the engine, this also sets `speeds` to the machine's.
- `gcode(layer)`: one layer of the result as G-code text.
- `export_gcode(directory, overwrite=True)`: writes one file per layer of the result and returns the paths. The directory is created if missing. With `overwrite` false, a file that already exists raises `FileExistsError` naming it, and nothing is written. Files are named as the GUI names them, `plot_{n}_{pen}.gcode`, where `n` counts from 1 and every run of characters outside `[A-Za-z0-9_.-]` in the pen's name becomes `_`.

Each of the three raises `RuntimeError` without a machine or without a result. `gcode` and `export_gcode` raise `ValueError` when the page does not fit the machine's area.

Fixed on the way: `stats()` and `lint()` push `Session.speeds` into the engine. So a preset that carried a machine had its speeds overwritten by the old ones. `set_machine`, and `load_preset` when the preset has a machine, now copy the engine's speeds into `speeds`.

## CLI

`plotline run` gains two options.

- `--machine FILE`: a JSON file holding either a machine or a preset that has one. It applies after `--preset` and before the page options. Like the GUI's machine pick, it makes the page the machine's drawable area while keeping the margin and fit mode. `--width` and `--height` then override that.
- `--gcode DIR`: after the run, writes one G-code file per pen layer into `DIR` and prints how many it wrote. Without a machine, from `--machine` or the preset, the command refuses before running, as a usage error.

## MCP

- **New tool `set_machine(machine)`** takes a machine object, or `null` to clear it. It also makes the page the machine's drawable area while keeping the margin and fit mode, as the GUI's pick does, so that the next export fits. It returns the machine and the page. This makes thirteen tools. A separate tool matches `set_page` and `set_pens`, one tool per session setter. A field on `set_page` would hide the machine's feeds and pen heights in a page call.
- **`export(path, format="gcode")`** takes `path` as a directory and writes one file per pen layer into it. It refuses if any of those files exists, unless `overwrite` is set, through `export_gcode(overwrite=False)`. It returns each file's path and size.
- `get_session()` also returns the machine.

## Tests

- **Python:** set, read and clear a machine; check that the speeds follow it and that a refused machine changes nothing; check G-code text and file names; check the errors without a machine or result and with a page off the machine; check that a preset's machine speeds reach `speeds`.
- **CLI:** `--machine` with a machine file and with a preset file, and the page following the machine; `--gcode` file count and contents; `--gcode` without a machine is a usage error.
- **MCP:** `set_machine` sets the page and appears in `get_session`; `export` with `gcode` writes the files and refuses existing ones; a refused machine is a tool error; the tool list has thirteen tools.
