# plotline spec: the test chart

Date: 2026-10-16. Status: written for review. The owner delegates the choices, which are recorded in the brief's decision log.

The engine's physical numbers are guesses until someone checks them on paper. The plot lint calls three or more passes overdraw, the plot time assumes the speeds the user gave, and a dot's ink is assumed to reach its drawn diameter. This spec adds a test chart: a page of small labelled patches for the current pen. Plotted and compared by eye, it shows where lines merge, how dots of each size come out, how many passes the paper takes, and whether the plot time estimate holds. Brief §12, question 8 ("Additions after 0.1") lists `plotline testchart`.

## 1. Scope

- `crates/core/src/text.rs` (new): a single-stroke font drawn as paths.
- `crates/core/src/testchart.rs` (new): the chart for a canvas and a pen.
- `crates/core/src/session.rs`: `Session::testchart`, which makes the chart the session's latest drawing.
- Bindings: `testchart()` in `crates/py` and `crates/wasm`; `Session.testchart()` in `src/plotline/session.py`.
- Front ends: a `plotline testchart` command, a `testchart` MCP tool, and a Test chart button in the GUI's export panel, with a tour step and a Playwright flow.
- Docs: brief §6.3, §9, §12 and §14, the README, and a CHANGELOG line under an "Unreleased" heading above 0.1.0. Add the heading if it does not exist.

Out of scope: a chart for every pen at once, a speed or acceleration ladder, and any setting the chart would feed automatically. The user reads the plotted chart and adjusts the settings, or the lint constant, by hand.

## 2. The font

`text::paths(s: &str, origin: [f64; 2], height: f64) -> Vec<Vec<[f64; 2]>>` draws `s` as polylines.

- `origin` is the top-left corner of the first glyph, in page mm with y down.
- Each glyph is designed for this project on a grid 4 units wide and 6 high, with y down and the baseline at 6.
- The glyphs are original polylines of a few strokes each: `A`–`Z`, `0`–`9`, `.`, `-`, `:`, `/` and space. Lowercase letters draw as their capitals; any other character draws as a space.
- One unit is `height / 6` mm. Each glyph advances 5 units, its 4 plus a gap of 1.
- `text::width(s, height)` gives the advance of the whole string.

No font file or third-party glyph data is used, so the licence and naming rules hold. The module docstring says the glyphs were drawn for plotline.

## 3. The chart

`testchart::chart(canvas: &Canvas, pen: &Pen) -> Result<Drawing, String>` gives one layer for `pen` and no tone map; `preset` is `None` once that field exists. It lays out from the top-left corner of the drawing area, with `w` the pen width:

- **Sizes.** Patch side `P = max(10, 8 w)` mm, gap `G = P / 2`, and text height `T = max(2.5, 5 w)` mm. Each patch has its label under it, `T / 2` below its bottom edge.
- **Header.** `TEST CHART PEN {w} MM`, with the width formatted as `{:.2}`.
- **Sections.** Each section starts a new row with its title at height `T`. Its patches follow left to right `G` apart, and wrap to a new row when the next would pass the drawing area's right edge.
  1. `SPACING MM`: spacings `s = f w` for `f` in 0.5, 0.75, 1, 1.25, 1.5, 2, 3 and 4. Each patch is horizontal lines across its full width, the first `s / 2` below its top and then every `s` while inside the patch. Lines alternate direction and each is its own path. The label is `s` as `{:.2}`.
  2. `DOT MM`: diameters `d = f w` for `f` in 1, 1.5, 2, 3, 4 and 6. Each patch is a grid of dots with pitch `p = max(2 d, d + w)`, `n = max(1, floor(P / p))` per side, centred in the patch. Each dot is `stipple::dot_path(centre, d, w, DotShape::Auto)`: a point when it is no wider than the pen, a filled spiral above. The label is `d` as `{:.2}`.
  3. `PASSES`: `n` in 1, 2, 3, 4, 6 and 8. Each patch is the solid fill of the spacing section at `s = w`, drawn `n` times over, each time as its own paths. The label is `X{n}`.
- **Page too small.** When the content needs more height than the drawing area has, `chart` fails with: "the page is too small for the test chart: it needs {h:.0} mm of the drawing area's height and has {a:.0} mm".

Arithmetic goes through `crate::math`, as everywhere, so the chart is the same on every target.

## 4. The session

`Session::testchart(&mut self) -> Result<(), SessionError>` builds the chart from the session's canvas and first pen.

- It claims the result slot as a run or a fit does (`self.claim()`), so the latest run is cancelled. It then writes the chart as the slot's drawing with state `Done` and no error.
- A failure from `chart` becomes a new `SessionError::Testchart(String)`.
- The chart needs no image.
- `stats()`, `lint()`, `preview_png()`, `svg()`, `json()` and `gcode()` work on it as on any drawing. The stats' plot time is the estimate a user checks against a stopwatch, and the lint finds overdraw on the `PASSES` section by design.

## 5. Front ends

- **Python:** `Session.testchart()` returns the `Drawing` and keeps it as `result`, as `run_blocking` does. It refuses with `ValueError` when the page is too small.
- **CLI:** `plotline testchart --out chart.svg [--preview chart.png] [--json chart.json] [--pen NAME:WIDTH_MM:COLOR] [page options] [--machine FILE]` writes the files and prints the stats line `run` prints. Reuse the page, pen and machine options and helpers that `run` has. It does not print lint findings.
- **MCP:** a `testchart()` tool draws the chart for the session's page and first pen. It returns the same stats JSON (with `lint`) and preview as `run`, through the same helper. Its docstring says what each section is for.
- **GUI:**
  - A "Test chart" button in the export panel, `data-tour="testchart"`, enabled with or without an image.
  - It calls a new worker command, `testchart`. The command posts `run`, `result` and `end` events like a finished run (reuse the driver's result posting), so the preview, stats, lint list and export work unchanged. The next run or setting change replaces the chart.
  - A tour step after the export step: "Calibrate on paper". It says what to look for: where spaced lines merge, how big each dot really comes out, at how many passes the paper scuffs, and whether the plot takes the time the stats say.
  - A Playwright flow in `web/e2e/gui.spec.ts` clicks the button and checks the run ends done with a path count above zero.

## 6. Tests

- **Text:** every character the chart draws has a glyph; `width` matches the advance; a string of unknown characters draws nothing.
- **Chart:**
  - It fits A4 and A5 with a 0.3 mm pen, and A4 with a 1 mm pen.
  - It refuses an A7 page with a 2 mm pen, and the message names both heights.
  - Every vertex lies inside the drawing area.
  - The same inputs give the same paths.
  - The `PASSES` patches are drawn their number of times.
- **Session:**
  - `testchart` works without an image.
  - The chart becomes the drawing, and `stats` and `svg` work on it.
  - `lint` finds `overdraw`.
  - A refused page leaves the old drawing in place.
- **Hosts:** a Python test; a CLI test that writes the SVG; an MCP test of the tool; a protocol test of the `testchart` command; and the Playwright flow.
