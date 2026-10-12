# plotline spec 4: machines and G-code export

Date: 2026-10-12. Status: built from the owner's approved plan; open to review.

The brief keeps the core machine-agnostic and puts generic G-code in an adapter (sections 4 and 14). This spec adds that adapter: a `Machine` the session can hold, a G-code writer for one pen layer at a time, the machine in the preset, and a machine panel with per-pen G-code export in the GUI. The example machine below is the owner's test rig, a 3D printer with a clip-on pen holder, whose values come from the owner's own toolchain. It shipped as a built-in at first and was removed on the same day: see Built-ins.

## 1. Scope

- `crates/core/src/machine.rs`: the `Machine` type, its validation, the page-to-machine mapping, and the plot speeds it implies.
- `crates/core/src/export/gcode.rs`: one layer of a drawing as G-code for a machine.
- The session holds an optional machine; setting one also sets the speeds the stats use. `Session::gcode(layer)` writes a layer.
- Preset version 2 carries `machine` (an object or null); version 1 presets migrate with null.
- WebAssembly: `Session.setMachine(json)`, `Session.machine()`, `Session.gcode(layer)`.
- GUI: a machine panel (a select to start a new machine or drop it, the machine's fields editable) and one G-code button per pen in the export panel.

Out of scope: machine options in the Python CLI and the MCP (the binding stays as it is; a later spec adds them), any dialect other than the strict one below, one file for several pens with a pause between them, arcs, and per-machine pen banks.

## 2. The machine

Superseded in part by [spec 7](2026-10-12-settings-dialog-design.md): the area is now a corner and a size, and there is no bed.

```json
{"name": "Test printer",
 "bed_mm": [256, 256],
 "area": {"x_min": 28, "x_max": 228, "y_min": 42, "y_max": 216},
 "z_down_mm": 4, "z_up_mm": 9, "z_safe_mm": 20,
 "draw_feed": 3600, "travel_feed": 6000, "z_feed": 1200,
 "settle_ms": 50,
 "start_gcode": "M104 S0\nM140 S0",
 "end_gcode": "G1 Z20 F600\nG1 Y250 F3600"}
```

- `area` is where the pen may be below `z_safe_mm`, in machine millimetres, y up as printers count it. It must sit inside the bed with min below max on both axes.
- `0 <= z_down_mm < z_up_mm < z_safe_mm`. Feeds are mm per minute and positive. `settle_ms` is the dwell after each Z move.
- `start_gcode` and `end_gcode` are lines written as they are, after the mode lines and before the final lift respectively. The machine's own checker, not plotline, decides whether they are safe.
- `Machine::validate` names the failing field in its message.

**Mapping.** The page (origin top-left, y down) is centred in the area: `ox = x_min + (W - w) / 2`, `oy = y_max - (H - h) / 2`, and a page point `(x, y)` lands at `(ox + x, oy - y)`. A page wider or taller than the area is refused with both sizes in the message, so no move can leave the area.

**Speeds.** `draw_mm_s = draw_feed / 60`, `travel_mm_s = travel_feed / 60` and `pen_lift_s = 2 (z_up - z_down) / (z_feed / 60) + 2 settle_ms / 1000`, which for the machine above is 0.6 s. Setting a machine sets these on the session so the estimate is honest; the user can still change them afterwards.

**Built-ins.** None. A machine's area and heights are measured on one rig, so a built-in is unsafe for anyone else, and the owner's rig is their own preset. The GUI's select shows "None (SVG only)", the current machine, and "New machine", which starts from placeholder values the user replaces with their machine's own.

## 3. The G-code

Strict dialect, so a downstream checker never has to guess: upper case, one command per line, `;` comments only, G21/G90 before any move, no G28, no arcs, no combined X/Y and Z moves, every number with at most three decimals and no trailing zeros. One file per layer, no pause: the owner's printer pauses by reheating, so each pen is its own job.

```
; plotline: <pen name>, <n> paths, pen <i> of <k>
; Home the machine before mounting the pen: this file has no G28.
G21
G90
<start_gcode lines>
G1 Z<safe> F<z_feed>
G0 X<x> Y<y> F<travel_feed>          to the first vertex of the first path
G1 Z<down> F<z_feed>                 pen down: Z, dwell, draw feed
G4 P<settle_ms>
G1 F<draw_feed>
G1 X<x> Y<y>                         one per further vertex; none for a dot
G1 Z<up> F<z_feed>                   pen up: Z, dwell, draw feed
G4 P<settle_ms>
G1 F<draw_feed>
G0 X<x> Y<y> F<travel_feed>          to the next path, left out when the pen is already there
...
G1 Z<safe> F<z_feed>
<end_gcode lines>
```

A dot (a one-vertex path) is a travel, pen down, pen up at one point. `Session::gcode(i)` fails without a machine, without a drawing, for a layer the drawing lacks, and when the page does not fit the area.

## 4. Preset version 2

`PRESET_VERSION` becomes 2 and the preset gains `"machine"`: the machine object or null. The migration from 1 adds `machine: null`. `parse` accepts a missing or null machine and validates an object through `Machine::validate`, naming the failing field. The Python tests that pin the version move with it.

## 5. GUI

- The state carries `machine` (from the preset). `setMachine` is not a redraw: the drawing does not depend on the machine.
- Machine panel: a select (None, the built-ins, custom), then, with a machine set, its fields: name, the four area bounds, the three heights, the three feeds, the settle time, and start and end G-code as text areas. Picking a built-in sends `setMachine` and then `setCanvas` with the area's size and the current margin, so the page is the drawable area. Editing a field sends `setMachine` with the changed machine; a refusal shows under the panel and the fields restore from the reply.
- Export panel: with a drawing and a machine, one button per pen, "G-code: <pen name>", which asks `gcode(i)` and downloads `plot_<i+1>_<pen name>.gcode` with the name made file-safe.

## 6. Tests

- `machine.rs`: validation names the field; for the machine in section 2 the speeds are 60, 100 and 0.6; the mapping puts the page's top-left corner at `(x_min, y_max)` when the page fills the area, centres a smaller page, and refuses a larger one.
- `gcode.rs`: a walk of the output in the style of the owner's checker: only the allowed commands, G21 and G90 before the first move, the first X/Y move at `z_safe_mm`, every X/Y inside the area while Z is below `z_safe_mm`, Z never below `z_down_mm`, no line that moves X/Y and Z together; a dot gives the down/up pair at one point; a two-vertex path gives one `G1 X Y`; the layer header names the pen and counts the paths; a page larger than the area is refused.
- `presets.rs`: a version 1 preset parses with no machine; version 2 round-trips the machine; a bad machine names the field.
- `session.rs`: `set_machine` sets the speeds; `gcode` fails without a machine or a drawing.
- WebAssembly smoke test: `setMachine` with the machine in section 2, then `gcode(0)` on the gradient drawing contains `G1 Z4 F1200` and `G1 Z20 F1200`.
- vitest `protocol.test.ts`: `setMachine` changes the state's machine and speeds without a run, `gcode` before a drawing is refused.
- Acceptance, outside the repo: the owner's `plot.py check` passes on an exported file.

## 7. Decisions recorded in the brief

The G-code adapter lives in the core crate so the browser and Python share it; the machine is part of the preset; one file per pen; the dialect is the strict one above.
