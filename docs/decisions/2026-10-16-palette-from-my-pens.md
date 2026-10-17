# Palette from my pens

**Decided:** 2026-10-16

**Choice:** Added on 2026-10-16 under [spec 14](../superpowers/specs/2026-10-16-palette-from-my-pens-design.md).

When the session has a pen library, the palette fit of [the pen palette](2026-10-16-pen-palette.md) picks from the library's distinct colours instead of the ten `INKS`. Colours are compared without case, in the order each first appears, and each is named after the first pen with it. The `colour` suggestion then sets the user's own pens: of each chosen colour, the one whose width is closest to the first pen's, ties to the earlier pen.

A new note, `pens_to_add`, lists up to three of the ten inks the library lacks. It tries each ink one step on from the library's palette: added to it, when it has fewer than four colours, and in place of each of its colours. An ink is kept when it lowers the mean ΔE*ab by at least 2.3, the same just noticeable step the fit stops at. This is the neighbourhood move of local search (Papadimitriou and Steiglitz, *Combinatorial Optimization*, 1982). The list is best first.

The engine changes as follows:
- `Session::set_pen_library` holds the library and measures the image again;
- `analysis::measure_with` takes the library;
- `ImageStats.palette` lists `{name, color}` objects, and `ImageStats.palette_additions` lists `{name, color, error}`.

The front ends pass the library through:
- the GUI sends its My pens library to the worker at start and on every change;
- `plotline analyse --library FILE` reads the GUI's `plotline-pens.json`;
- MCP gains `set_pen_library`.

The library is in no preset, export, undo step or drawing, so the regression gallery does not change.

**Reason:**
- **Why it matters.** The palette could suggest an ink the user has no pen for, and it could not use the real colours of the pens they own. The pen library already held those pens, so the fit reads it instead of asking for a second list.
- **Why the lookahead.** It is a cheap, local answer to "which pen would help most next". It solves at most fifty more ink sets.
- **Rejected: refit from scratch.** Refitting over the library plus each ink, once per ink, was set aside. It would cost about ten times as much, on every image in the browser.
- **Rejected: fit the inks and list the missing ones.** Fitting the ten inks alone and listing those the library lacks would tell someone who owns a slightly different blue to buy blue. Under the lookahead, an ink close to an owned colour gains less than one just noticeable difference, so it drops out without a threshold of its own.
- **Why `{name, color}`.** The palette had to change shape because a library may hold two pens with one name in different colours, and a name alone could not find the colour again.
- **Measured.** On `synthetic::scene` at 1024 by 768, with a library of black alone, the palette is black at a mean ΔE*ab of 50.7. The note names blue (27.0), green (36.8) and purple (39.3). With no library, the fit picks yellow, green and blue at 3.4, as before. `measure_with` on that image, as the median of seven runs of a release build on Windows, took the following:
  - with no library: 37.4 ms;
  - with black alone: 36.6 ms;
  - with the ten inks as a library: 36.8 ms;
  - with a library of twenty distinct colours: 41.1 ms.

  A second pass gave 38.2, 37.7, 37.9 and 40.5 ms, so a library costs a few milliseconds at most. The WebAssembly module, built on Windows, grows by 2,382 bytes, from 2,102,701 on `origin/main` to 2,105,083, under the 2,300,000-byte limit.

**Amends:**
- [Pen library](2026-10-16-pen-library.md), which said the engine, CLI and MCP never see the library;
- [Pen palette](2026-10-16-pen-palette.md), for the palette's shape and its candidates.
