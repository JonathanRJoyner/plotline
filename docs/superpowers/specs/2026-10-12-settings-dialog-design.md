# plotline spec 7: a simpler machine and a settings dialog

Date: 2026-10-12. Status: approved in conversation; written for review.

The owner found the machine panel of [spec 4](2026-10-12-machines-gcode-design.md) confusing: it asks for a bed size that does nothing once the drawable area is known, and the area is four bounds instead of a size and a position. The machine, its G-code and the plot speeds also crowd the right column, although they are set once per setup, not per drawing. This spec simplifies the machine and moves those settings into a dialog opened from the toolbar.

## 1. Scope

- `crates/core/src/machine.rs`: drop `bed_mm`; `area` becomes a corner and a size.
- `crates/core/src/presets.rs`: preset version 3, with a migration from version 2.
- `crates/core/src/export/gcode.rs`: its tests stop checking against the bed. The writer is unchanged.
- `web/`: a Settings button in the toolbar opens a dialog with four sections: Machine, Pen lift, G-code and Plot time. The machine and speeds panels leave the right column.
- Docs: three rows in the brief's decision log, the README's GUI paragraph, and a pointer from spec 4 section 2 to this spec.

Out of scope: pen-up and pen-down G-code snippets (servo plotters), units other than mm, a reset button per field, named machine presets the user saves, and machine options in the Python CLI and the MCP.

## 2. The machine

```json
{"name": "Bambu A1 with clip-on pen holder",
 "area": {"x": 28, "y": 42, "width": 200, "height": 174},
 "z_down_mm": 4, "z_up_mm": 9, "z_safe_mm": 20,
 "draw_feed": 3600, "travel_feed": 6000, "z_feed": 1200,
 "settle_ms": 50,
 "start_gcode": "M104 S0\nM140 S0",
 "end_gcode": "G1 Z20 F600\nG1 Y250 F3600"}
```

- `area` is where the pen may be below `z_safe_mm`, in machine millimetres with y up. `x` and `y` are the machine position of its corner with the smallest X and Y. `width` and `height` are its size. The built-in values describe the same rectangle as before, so the A1's G-code does not change.
- `x` and `y` must be finite and may be negative, so a machine whose zero is the middle of its bed works. `width` and `height` must be positive and finite.
- There is no bed size. The bed only checked that the area fits on it, which the area alone now states.
- Validation names the failing field, for example `area.width must be positive, got 0`. The other rules are unchanged from spec 4.
- `Area::contains` and `Machine::mapping` keep their meaning. The page is centred in the area: `ox = x + (width - w) / 2` and `oy = y + (height + h) / 2`, and a page point `(px, py)` lands at `(ox + px, oy - py)`. A page larger than the area is still refused, naming both sizes.

## 3. Preset version 3

`PRESET_VERSION` becomes 3. The migration from 2 changes only an object `machine`: it removes `bed_mm` and, when `area` holds the four numbers `x_min`, `x_max`, `y_min` and `y_max`, replaces it with `{x: x_min, y: y_min, width: x_max - x_min, height: y_max - y_min}`. Anything else is left as it is, so `parse` names the bad field as before. A version 1 preset still goes through the migration to 2 first, which adds a null machine.

So the preset the browser remembers, and preset files saved with spec 4, keep loading. The Rust and Python tests that pin the current and the unsupported version move up by one.

## 4. The settings dialog

**Opening it.** A Settings button with a gear icon sits in the toolbar before the theme button. It opens a native `<dialog>` with `showModal()`, which brings focus trapping and Esc to close. A Close button sits in the header. Changes apply as they are made, through the same `setMachine` and `setSpeeds` calls as now, so there is no OK or Cancel.

**Layout.** A list of sections on the left and the open section's fields on the right, at most 720 px wide and 70% of the window high, with the fields scrolling inside. At phone width the list sits above the fields. The dialog opens on Machine. A machine refusal shows at the top of the fields, whichever section caused it, and the fields restore from the reply as before.

**Sections.**

- **Machine.**
  - The machine select: "None (export SVG only)", the built-ins, and "custom" when the machine matches no built-in.
  - With a machine set: Name.
  - Drawable area: Width and Height, then Offset X and Offset Y, all in mm. A help line follows: "Offset is the machine position of the area's corner with the smallest X and Y. The page is centred in the area."
  - Feeds: Draw and Travel, in mm/min.
- **Pen lift.** Pen down Z, Pen up Z and Safe Z in mm, Z feed in mm/min, and Settle in ms. Each has a one-line help text:
  - Pen down: the Z the pen draws at.
  - Pen up: the Z it travels at between paths.
  - Safe: the Z each file lifts to before its first move and before its end lines.
  - Settle: the pause after each Z move.
- **G-code.**
  - Start and End as monospace text areas of six rows.
  - A help line: "plotline writes G21 and G90 before the start lines, and lifts to safe Z before the end lines. Home the machine before mounting the pen: the files have no G28."
- **Plot time.** Draw and Travel in mm/s and Pen lift in s, which the time estimate uses.
  - With a machine set, they are read-only, with the line "From the machine's feeds and pen lift." The session already resets them on every `setMachine`, so they could only drift from the machine through this form.
  - Without a machine, they are editable, as the speeds panel is now.

Without a machine, Pen lift and G-code show "Choose a machine first" instead of their fields.

**Picking a machine.** Picking a built-in still sets the page to the area's width and height and keeps the current margin. Editing the area does not change the page. If the page then no longer fits, the G-code export refuses it with the message that names both sizes, as now.

**The right column** keeps what changes per drawing: Page, Pens, Presets and Export. `MachinePanel` and `SpeedsPanel` become sections of a `SettingsDialog` in `panels.tsx`, and `events.ts` takes the new `Area` and drops `bed_mm`.

## 5. Tests

- `machine.rs`:
  - The A1 built-in matches section 2.
  - Validation names `area.width` and `area.x`.
  - A negative offset is accepted.
  - The existing mapping tests keep their numbers: a full page starts at `(28, 216)`, a smaller one is centred, and a larger one is refused.
- `gcode.rs`: the checker walk drops its bed check and keeps the area check.
- `presets.rs`:
  - A version 2 preset carrying the spec 4 A1 parses to the new A1.
  - A version 1 preset still parses with no machine.
  - The round trip asserts `area.x`.
  - The version pins move to 3, with 4 as the unsupported version.
- `tests/test_presets.py`: the unsupported version moves from 3 to 4.
- vitest and the WebAssembly smoke test pass unchanged, since they only use the built-in by name and the Z lines.
- Manual, in `npm run dev`:
  - The dialog opens from the toolbar and closes with Esc and with Close.
  - Picking the A1 sets the page to 200 by 174 mm.
  - Editing a feed updates the read-only plot time.
  - A refused value, such as Pen up Z below Pen down Z, shows its message.
  - A G-code file exported for the sample image is byte for byte the same as on `main`.

## 6. Decisions recorded in the brief

- **Machine area.** The area is a corner and a size, and there is no bed size. The owner found the bed and the four bounds confusing, the bed only checked the area, and a size and an offset are what one measures on the machine.
- **Settings dialog.** The machine, pen lift, G-code and plot speeds live in a modal dialog opened from the toolbar, and the right column keeps the per-drawing panels. They are set once per setup, and the owner asked for a pop-up settings menu. Changes apply at once, so the dialog needs no undo.
- **Preset version 3.** The machine's new shape needs a migration, so saved and remembered setups keep loading.
