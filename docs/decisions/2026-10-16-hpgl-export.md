# HPGL export

**Decided:** 2026-10-16

**Choice:** Added on 2026-10-16 under [the HPGL export spec](../superpowers/specs/2026-10-16-hpgl-export-design.md), as the first use of [the standard format names decision](2026-10-16-standard-format-names.md). `export::hpgl::to_hpgl(drawing, units_per_mm)` in the core writes the whole drawing as one program, in the instruction set of Hewlett-Packard's *The HP-GL/2 and HP RTL Reference Guide* (Addison-Wesley, 1997), using only `IN`, `SP`, `PU` and `PD`, which HP-GL/2 kept from the original HP-GL. The writer's rules:
- The program starts with `IN;`. Each layer `i` that has paths selects pen `i + 1` with `SP{i+1};`, and an empty layer writes nothing. It ends with `PU;` and `SP0;`.
- Each path is `PU x,y;` to its first vertex, then `PD` with the rest of its vertices as one comma-separated list. A dot, a path of one vertex, is `PD;` with no coordinates.
- Coordinates are whole plotter units: `x = round(x_mm * units_per_mm)` and `y = round((page_height_mm - y_mm) * units_per_mm)`, because HPGL's y axis points up from the page's lower-left corner and plotline's points down from the top-left.
- `units_per_mm` defaults to 40. A value that is not above 0 and at most 1000, or is not a number, is refused.
- One instruction goes on each line, every line ends with `;`, and the text is ASCII with LF line ends.
- No velocity, force or other plotter-specific instruction is written.

`Session::hpgl(units_per_mm)` fails with `NoDrawing` without a result, and with a new `Hpgl` variant for a refused resolution, which Python reports as `ValueError`. It needs no machine. The front ends:
- **Python:** `Session.hpgl(units_per_mm=40.0)` and `Session.export_hpgl(path, units_per_mm=40.0)`.
- **CLI:** `plotline run` takes `--hpgl FILE` and `--hpgl-units N`.
- **MCP:** `export` takes the format `hpgl` and a `units_per_mm` parameter that only `hpgl` reads.
- **GUI:** an HPGL button in the Export panel downloads `plotline-<style>.hpgl`, named like the SVG and JSON exports; the Settings dialog has an HPGL section with "Plotter units per mm", kept in the browser's storage like the theme; the tour's export step mentions HPGL.

A property law in `crates/core/tests/laws.rs` checks, for any drawing and resolution, that every coordinate lies within the page scaled by the resolution and that the vertices of the `PU` and `PD` lines equal the drawing's vertex count.

**Reason:** Many pen plotters, older ones especially, read HPGL and not SVG or G-code, and a file in the format they read needs no converter between plotline and the machine. The standard format names decision lets the format be named, and nothing in the file is a vendor extension.

One file with `SP` per layer, because `SP` is how HPGL changes pens: a multi-pen plotter then draws every layer from one file, and a single-pen plotter pauses or ignores the command, depending on the model. The G-code writer makes a file per pen because a printer pauses by reheating, so each pen is its own job; HPGL has no such problem, so one file is the standard way.

40 units to the millimetre because HPGL's plotter unit is 0.025 mm, which is 40 to the mm, and most plotters work in it. The writer takes the number as a parameter, since some plotters work in other units, and the user sets it where their plotter's manual says; nothing guesses it from the page.

No plotter-specific instructions, because plotters differ and a file of the four basic instructions is read by all of them, while an instruction that one maker's machines understand would keep a file from being plain. The cost is that a pen number above a plotter's carousel and coordinates past its hard-clip limits are the user's to check, as the machine's own lines are for G-code. The resolution is refused above 1000 units to the millimetre, 25 times finer than HPGL's unit, because no plotter draws at that grain and a far larger value would saturate the 64-bit coordinates and write numbers that mean nothing; the property law draws its resolutions from the accepted range.
