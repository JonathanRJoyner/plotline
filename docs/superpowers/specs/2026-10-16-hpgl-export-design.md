# HPGL export

Status: built on 2026-10-16. Decisions: [standard format names](../../decisions/2026-10-16-standard-format-names.md) and [HPGL export](../../decisions/2026-10-16-hpgl-export.md).

## Goal

Many pen plotters, older ones especially, take HPGL rather than SVG or G-code. HPGL is a standard format name, as G-code is. So once brief §12 question 1 is settled, which the first decision file does, the naming rule allows it. This spec adds an HPGL writer to the core and wires it to every front end.

## Writer

The writer is `crates/core/src/export/hpgl.rs`, `to_hpgl(d: &Drawing, units_per_mm: f64) -> Result<String, HpglError>`. It writes the whole drawing as one program, in the instruction set described in Hewlett-Packard's *The HP-GL/2 and HP RTL Reference Guide* (Addison-Wesley, 1997). It uses only instructions from the original HP-GL set that HP-GL/2 kept, so both kinds of plotter accept the file.

```
IN;
SP1;
PU800,11080;
PD840,11080,880,11120;
PU1200,9600;
PD;
SP2;
...
PU;
SP0;
```

- `IN;` initializes the plotter. Each layer `i` that has paths selects pen `i + 1` with `SP{i+1};`, and an empty layer writes nothing. At the end, `PU;` and `SP0;` lift the pen and put it away.
- Each path is `PU x,y;` to its first vertex, then `PD` with the rest of its vertices as one comma-separated list. A path of one vertex, a dot, is `PD;` with no coordinates, which lowers the pen where it stands.
- Coordinates are whole plotter units: `x = round(x_mm * units_per_mm)` and `y = round((page_height_mm - y_mm) * units_per_mm)`. HPGL's y axis points up from the page's lower-left corner, while plotline's points down from the top-left.
- `units_per_mm` is the plotter's resolution. It defaults to 40, which is HPGL's plotter unit of 0.025 mm. A value that is not finite, or not above 0, is refused.
- One instruction goes on each line, and every line ends with `;`. The text is ASCII with LF line ends.
- No velocity, force or plotter-specific instructions are written. Plotters differ, so a pen number above a plotter's carousel and coordinates past its hard-clip limits are the user's to check.

`Session::hpgl(units_per_mm) -> Result<String, SessionError>` fails with `NoDrawing` when there is no result, and with a new `Hpgl` variant for a refused resolution.

## Why one file with SP per layer

`SP` is how HPGL changes pens. A multi-pen plotter then draws every layer from one file, and a single-pen plotter pauses or ignores the command, depending on the model. The G-code adapter writes one file per pen because a printer pauses by reheating; HPGL has no such problem, so one file is the standard way.

## Front ends

- **Python.** The binding gains `hpgl(units_per_mm)`. `plotline.Session` gains `hpgl(units_per_mm=40.0) -> str` and `export_hpgl(path, units_per_mm=40.0)`, which writes ASCII with LF line ends.
- **CLI.** `plotline run` gains `--hpgl FILE` and `--hpgl-units N` (plotter units per mm, default 40).
- **MCP.** `export` gains the format `"hpgl"` and a `units_per_mm` parameter, default 40, which only `hpgl` reads.
- **wasm and GUI.**
  - wasm gets `Session.hpgl(unitsPerMm)`, and the worker protocol gets a method for it.
  - The Export panel gains an HPGL button that downloads `drawing.hpgl`.
  - The Settings dialog gains an "HPGL" section with "Plotter units per mm", default 40. It is kept in the browser's storage like the theme, wrapped in try/catch because storage may be missing.
  - The tour's export step mentions HPGL.
  - `web/e2e/gui.spec.ts` gains a flow: export HPGL and check that the download starts with `IN;` and contains `SP1;`.

## Tests

- **Core unit tests:**
  - the shape of the program on a small drawing;
  - the y flip;
  - rounding;
  - dots;
  - two layers giving `SP1` and `SP2`;
  - an empty layer skipped;
  - a refused resolution.
- **A law in `crates/core/tests/laws.rs`:** for any drawing and resolution, every coordinate lies within the page scaled by the resolution, and the count of `PU` and `PD` vertices equals the drawing's vertex count.
- **Hosts:** a wasm smoke test, plus Python, CLI and MCP tests.

## Docs

- **Decision files:** the two linked at the top.
- **Brief:** mark §12 question 1 settled, pointing to its decision file; add HPGL to §10's output list; name HPGL beside G-code among §6.3's adapters.
- **Changelog:** `changelog.d/hpgl-export.md`, with `Section: Output`.
- **README:** the outputs feature line and the CLI table.
