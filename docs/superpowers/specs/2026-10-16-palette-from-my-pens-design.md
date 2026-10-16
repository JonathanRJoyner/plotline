# plotline spec 14: a palette from your own pens

Date: 2026-10-16. Status: design approved by the owner on 2026-10-16; written for review.

Spec 13 fits two to four of the ten listed inks (`pens::INKS`) to a colour image and suggests them as pens. The GUI also keeps **My pens**, a library of the user's own pens (`plotline.pens` in the browser, exchanged as a `plotline-pens.json` file; see [the pen library decision](../../decisions/2026-10-16-pen-library.md)). The two do not meet: the palette may name a pink the user has no pen for, and it cannot use the user's real ink colours.

This spec joins them. When the user has saved pens, the palette fit picks only from those pens, and a second suggestion lists up to three listed inks the user does not own that would draw the image's colours clearly better. With no saved pens, everything works as it does today.

## 1. Method

### 1.1 Candidates

The fit of spec 13 (forward stepwise selection under the Pen use colour model, stopping at four inks or a gain under 2.3 ΔE*ab) is unchanged. Only its candidates change:

- **No library:** the ten `INKS`, in their listed order, as today.
- **A library:** the library's distinct colours, compared without case, in the order each first appears. Each colour is named after the first library pen that has it. Widths and names play no part in the fit, since the colour model sees only colour; a black owned in three widths is one candidate.

Ties keep going to the earlier candidate, so a library fit is as deterministic as the ink fit. Paper is taken as white, as in spec 13.

### 1.2 Which owned pen draws a chosen colour

A chosen colour may belong to several library pens of different widths. The suggestion uses the one whose width is closest to the session's first pen's width (0.3 mm with no pens), ties going to the earlier library pen. It keeps that pen's own name, width and colour, so Apply puts the user's real pens in the list.

### 1.3 Pens to add

With a library, the fit's set `S` of owned colours has an error `E(S)`. For each of the ten `INKS` whose colour is not a library colour (compared without case), the inks are tried one step on from `S`:

- added to `S`, when `S` has fewer than four colours;
- in place of each member of `S` in turn.

The ink's error is the lowest of those sets' errors, and its gain is `E(S)` minus that error. An ink is listed when its gain is at least 2.3, the same just noticeable difference the fit stops at (Mahy, Van Eycken and Oosterlinck, 1994). An ink close to a colour the user already owns gains less than that, so it drops out without a threshold of its own. The list is sorted by the error reached, ties in `INKS` order, and keeps the first three.

This is a one-step lookahead from the current solution, the neighbourhood move of local search (C. H. Papadimitriou and K. Steiglitz, *Combinatorial Optimization: Algorithms and Complexity*, Prentice-Hall, 1982, the chapter on local search). Two alternatives were set aside:

- Refitting over the library plus each ink from scratch would take about ten times the work, on every image in the browser.
- Fitting the ten inks alone and listing those not owned would tell a user who owns a slightly different blue to buy blue.

Only the ten listed inks are offered as additions, since they are the colours plotline can name in plain words. No maker or product is named.

### 1.4 Cost

The greedy fit over `n` distinct colours solves at most `n + (n-1) + (n-2) + (n-3)` sets. The lookahead solves at most 10 × 5 = 50 more. For a library of 20 colours that is about 124 solves against the 34 of today's fit. The fit and lookahead run when the image is set and when the library changes, never on a change to a setting. The implementation measures both on the generated scene and records the timings in the decision file.

## 2. Engine

- `analysis::measure(image, library: &[Pen])` runs the fit over the candidates of §1.1. With an empty library it gives exactly today's result.
- `ImageStats` changes as follows:
  - `palette` becomes a list of `{name, color}`, where `color` is `#rrggbb`. It was a list of ink names, and a library colour cannot be found again by name alone, since two library pens may share a name and differ in colour. Without a library the names are the ink names, as before.
  - New: `palette_additions` lists `{name, color, error}` for the inks of §1.3, where `error` is the mean ΔE*ab reached with that ink. It is empty without a library.
- `analysis::suggest` takes the library as well as the session's pens.
- `Session` gains `pen_library: Vec<Pen>` and `set_pen_library(pens)`.
  - The call checks every pen as `set_pens` does, and accepts an empty list, which clears the library.
  - With an image set, it measures the image again with the new library.
  - The library is not part of a preset, an export, the undo history or the drawing, so no drawing moves and the gallery snapshot does not change.

## 3. Suggestions

### 3.1 `colour`

The rule is the same as in spec 13: it needs colourfulness over 40 and a palette of at least two colours, and is left out once in effect. With a library:

- the pens are the owned pens of §1.2, lightest first by BT.601 luma, ties in library order;
- the reason names them as the user named them: "This is a colour image. Your pens Yellow fine, Blue 0.3 and Black 0.5 draw its colours best, with Pen use set to by colour."

Without a library, the patch and reason are unchanged.

### 3.2 `pens_to_add`

`pens_to_add` is a new note with no patch. It is made when `palette_additions` is not empty, for example:

"Pens you don't have would draw its colours closer: with an orange pen the mean colour difference falls from 12.3 to 5.1, and with a pink pen to 7.4."

It stays until the user saves a pen close enough to the ink, which happens without any extra rule. If the library's own fit has fewer than two colours, there is no `colour` suggestion, and `pens_to_add` is the only colour advice, as for a library of black pens only.

## 4. Front ends

- **GUI.**
  - The library's state moves from `PensDialog` up to `app.tsx`. The dialog gets `library` and `onLibrary` props, and its saving, removing and importing work as before.
  - The app sends the library to the worker on start and on every change, through a new `setPenLibrary` call.
  - That call does not redraw: it is left out of `REDRAWS`, but the suggestions are recomputed.
  - The tour's "Take a suggestion" step adds one sentence: with pens saved under My pens, the palette is chosen from them, and a note names inks worth adding.
- **Python.** `Session.set_pen_library(pens)`.
- **CLI.** `plotline analyse --library FILE` reads a `plotline-pens.json` file, version 1, the file the GUI exports. A bad file stops with a message in the GUI's words, such as "bad.json is not a pen library: its format must be 'plotline-pens'."
- **MCP.** A new tool, `set_pen_library(pens)`, takes the same pen specs as `set_pens`, ink names included. The `analyse_image` description and the server instructions say how the two meet.
- **Wasm.** `setPenLibrary(json)`.

## 5. Tests

- **`analysis` unit tests:**
  - with a library, every palette entry is a library colour;
  - the pen chosen for a colour is the one closest in width, with ties going to the earlier pen;
  - a library of black only on the colour bars gives no `colour` suggestion and a `pens_to_add` note, whose inks are not black and each gain at least 2.3;
  - a library holding all ten inks gives no additions;
  - an empty library gives the same `ImageStats` as before, so the existing palette tests keep passing.
- **Law** (`crates/core/tests/laws.rs`): for generated colour images and generated libraries, every pen in the `colour` patch is a library pen; every addition's colour is no library colour; and every addition's error is at least 2.3 below `palette_error`.
- **Python:** `set_pen_library` then `analysis()` gives library pens; a bad pen is refused.
- **CLI:** `analyse --library` with a good file, and with a bad one.
- **MCP:** `set_pen_library` then `analyse_image`, and the tool list gains the tool.
- **Worker:** `protocol.test.ts` checks that `setPenLibrary` updates the suggestions without a redraw.
- **Playwright:**
  1. Save two pens to My pens.
  2. Open a colour image made in the page.
  3. The `colour` suggestion's Apply sets those saved pens.
  4. A note names a pen to add.

## 6. Decisions this amends

- [Pen library](../../decisions/2026-10-16-pen-library.md): the library was the page's alone ("the engine, the CLI and the MCP never see it"). The engine now reads it, because the palette fit has a use for it.
- [Pen palette](../../decisions/2026-10-16-pen-palette.md):
  - `palette` holds `{name, color}`, not names;
  - the candidates come from the library when there is one;
  - its out-of-scope item "inks other than the ten" is settled for owned pens.

## 7. Out of scope

- Inks other than the ten as pens to add, such as a measured or arbitrary colour.
- Non-white paper in the colour model.
- A library in a config file for the CLI. The CLI reads the file each time it is given one.
- Pen widths fitted to the image.
