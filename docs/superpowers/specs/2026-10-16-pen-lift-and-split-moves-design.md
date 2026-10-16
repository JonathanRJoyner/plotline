# Pen lift by G-code lines, and moves split for curved-axis machines

Status: built on 2026-10-16. Decisions are recorded in the brief's decision log under "Pen lift by G-code lines" and "Split moves".

## Goal

The generic G-code writer (`export::gcode`) lifts the pen with Z moves, which suits a printer fitted with a pen. Many plotters lift the pen with a servo or a solenoid instead, driven by a command, and some (hanging plotters with two cords, arms with two joints) move along curves unless the firmware cuts a straight move into short ones. This spec lets a machine the user describes cover both, without naming any machine or firmware and without changing a byte of today's output.

## The machine

`Machine` gains four fields, each with a default that gives today's behaviour:

- `pen_lift`: `"z"` (the default) or `"gcode"`.
- `pen_down_gcode` and `pen_up_gcode`: lines written as they are, in place of the Z move, when `pen_lift` is `"gcode"`. Empty by default.
- `max_segment_mm`: the longest straight move, or null (the default) to keep every move whole.

Serde fills a missing field with its default, and leaves a field out when it holds its default. So a machine file or preset written before this change loads as it was, and a machine that uses none of the new fields is written exactly as before. The preset version stays 3 (see "Presets" below).

`validate`:

- In `"z"` mode it checks what it checks today.
- In `"gcode"` mode the Z heights and `z_feed` are unused, so they are not checked; `pen_down_gcode` and `pen_up_gcode` must each hold a line that is not blank. A Z value a user left behind when switching to G-code lines then never blocks the machine.
- `max_segment_mm`, when set, must be at least 0.5 mm and finite. The floor bounds the file: 0.5 mm turns the longest pen-down length a dense drawing reaches (under 1 km) into under two million lines, which the browser can hold, and firmware that segments moves uses pieces of about a millimetre, so nothing finer is needed.

`speeds()` in `"gcode"` mode takes the pen lift as the two dwells alone, `2 × settle_ms / 1000` seconds: the dwell is the time the user gives the servo to turn, and no Z move is made.

## The writer

In `"z"` mode the output is unchanged, byte for byte.

In `"gcode"` mode:

- A pen move is the machine's lines for it, then the dwell, then the draw feed, in the place where the Z move, dwell and draw feed go today.
- The lift at the start of the file becomes the pen-up lines and the dwell, so the pen is up before the first travel even if it was down when the file began.
- There is no safe height, so the file has no lift before the end lines: every path already ends with the pen up, and an empty layer still has the lift at the start.

With `max_segment_mm` set, every straight move whose start the file knows is cut into `n = ceil(length / max)` equal pieces, each written as the move would be (`G1 X Y` for drawing, `G0 X Y F` for travel). That is every drawing move and every travel but the first, whose start is wherever the start lines left the machine. Travel is cut too: on a machine that curves a long move, an uncut travel can bow outside the drawable area, which the writer otherwise promises it never leaves. The last piece ends on the original vertex exactly.

The dialect comment says what the rules cover: the lines plotline writes. The machine's own lines (start, end, pen down and pen up) are written as they are and are the user's to check, as start and end already were.

## Presets

The preset version stays 3, and nothing migrates. Adding the fields with defaults, left out when unused, means:

- every preset and machine file written before loads, through the fields' defaults;
- a preset whose machine uses neither new feature is written exactly as before, so an older build, such as the package on PyPI, still reads it, including every preset the hosted page saves for a Z-lifted machine;
- a preset that uses a new feature fails in an older build with serde's "unknown field `pen_lift`", which names the field.

A version bump would have made every preset the hosted page saves unreadable by the released package, for the sake of a clearer message in the one case that cannot work anyway. Version 2 was bumped because it added a key every preset carries; these keys appear only when used.

## Front ends

- **GUI**: the Settings dialog's "Pen lift" section starts with "Lift by", Z moves or G-code lines. Z shows today's fields. G-code lines shows two text boxes, pen down and pen up, and Settle. Switching to G-code lines fills empty boxes with a servo command as a placeholder (`M280 P0 S30` down, `M280 P0 S90` up), so the switch is not refused for empty lines, and the help says to replace them. The "G-code" section gains "Longest move" in mm, empty for whole moves. The tour's machine step says the pen lifts by Z or by the machine's own lines.
- **Python, CLI, MCP**: the machine crosses as the same dict, so nothing changes in code but the MCP `MachineSpec` type (four optional keys) and the docs that list a machine's fields.

## Tests

- Rust unit tests: G-code mode writes the lines with no Z anywhere, lifts at the start, has no end lift, and passes the strict `check` (which now tracks the pen-up lines as the lift in G-code mode, and in that mode allows no X/Y outside the area at all); splitting cuts a long move into equal pieces ending on the vertex, leaves short moves alone, cuts travels after the first; validation names `pen_down_gcode` and `max_segment_mm`; speeds in G-code mode.
- Law (`crates/core/tests/laws.rs`): for any style's drawing on a machine whose area is the page, with either lift and any `max_segment_mm`, the split file differs from the whole one only in its X/Y moves, every piece lies on the move it was cut from, no piece is longer than the maximum, every vertex of the whole file appears in order, and every point is inside the area.
- Python: a G-code-lift machine round-trips through `set_machine` and its file has the lines and no Z.
- Browser (`web/e2e/gui.spec.ts`): set a new machine to G-code lines with a longest move, draw the sample, export G-code, and find the pen lines, no Z, and no move longer than the maximum.
