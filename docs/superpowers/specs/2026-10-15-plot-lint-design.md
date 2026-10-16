# plotline spec: plot lint and the coverage raster

Date: 2026-10-15. Status: written for review; the owner delegates the taste defaults, which are recorded in the brief's decision log.

A drawing can look fine in the preview and still waste an hour, a pen or a sheet: lines laid on top of each other until the paper pills, specks that each cost a pen lift, a drawing that never gets dark, or a vertex past the edge of the paper. This spec has the engine check every finished drawing for those physical problems and return each one with a measured value and, where a setting fixes it, a patch in the same shape as the image analysis suggestions. It serves goal 1 of the brief ("no stray specks, no wasted pen-up travel") directly. It also builds the coverage raster, a count of pen passes per cell at the true pen width, which the later tone-error metric and tone-response calibration of the smart-features research (S7.1 and S7.2) will reuse.

## 1. Scope

- `crates/core/src/coverage.rs` (new): the coverage raster.
- `crates/core/src/lint.rs` (new): the measurements and the checks.
- `crates/core/src/session.rs`: `Session::lint`, which measures each drawing once and keeps the measurement until the drawing changes.
- Bindings: `lint()` in `crates/py` and `crates/wasm`; `Session.lint()` in `src/plotline/session.py`.
- Front ends: findings on stderr and `--strict` in `plotline run`; findings in the MCP `run` result; a list under the stats in the GUI with Apply buttons, and a tour step. The GUI's two hard-coded hints, the empty-drawing advice in `StatsPanel` and the one-pen note in `PensPanel`, move into the core as findings.
- Docs: the README, the brief's decision log and §9.

Out of scope: a simplification setting for the ordering stage (the needless-vertices finding is a note until one exists; it is one of the brief's additions after 0.1 (§12)); a neighbour test for isolated dots in dot styles; the tone-error metric and tone calibration that will build on the raster.

## 2. The coverage raster

`coverage::Coverage` is a grid of `u8` pass counts over a page rectangle, with square cells of `cell_mm`:

- `Coverage::new(rect_mm, cell_mm)` makes an empty grid of `ceil(w / cell)` by `ceil(h / cell)` cells.
- `coverage::cell_size(pen_mm, rect_mm)` is a third of the pen width, at least 0.1 mm, and grown when needed so the grid stays within 12 million cells, the ink stage's bound for a browser tab.
- `Coverage::stamp(&mut self, paths, pen_mm)` lays every path at the true pen width: a cell counts a pass when its centre lies within `pen_mm / 2` of the path, a capsule around each segment and a disc around a lone vertex. A per-cell index of the last path that counted it means a path that crosses itself, or a spiral dot that winds over its own centre, counts once per cell. Counts saturate at 255.

Each row of a segment's capsule is one interval, because the capsule is convex: the union of the intervals cut by its two end discs and by its swept rectangle. Stamping costs pen-down length × pen width / cell² cell visits, about 6 million for 200 m of a 0.3 mm line at 0.1 mm cells. Vertices off the grid are clipped. All arithmetic is in `f64` with `sqrt` as the only library call, and paths are stamped in drawing order, so the counts are the same on every target.

A pen thinner than the cell (a 0.05 mm pen, or a page too big for the cell bound) can miss cell centres between them. The raster then undercounts, which errs towards fewer findings.

## 3. Measuring a drawing

`lint::measure(&drawing)` depends on the drawing alone. It uses one raster over the page, with the cell size of the thinnest pen, cleared between layers:

| Field | Meaning |
|---|---|
| `cell_mm` | the raster's cell size |
| per layer: `inked`, `overdrawn`, `max_passes` | cells passed at least once, cells passed three times or more, and the largest count, for that layer's pen alone |
| per layer: `paths`, `dots`, `short`, `segments`, `fine_segments` | paths, one-vertex paths, paths shorter than the pen width (dots included), segments, and segments shorter than an eighth of the pen width |
| `laid_share` | the ink laid over the drawing area: each path's length times its pen width, plus a pen-wide disc for its ends, summed and divided by the area (amended on 2026-10-16; it replaced `inked_share`, the raster's share, which undercounts pens thinner than a cell) |
| `dark_coverage` | over the cells whose nearest tone-map pixel holds 0.75 ink or more, the share any layer inks; none without a tone map or when such cells are under 1 % of the map's area |
| `off_page`, `off_area` | vertices outside the page, and vertices outside the drawing area by more than half their pen width |

Overdraw counts one pen against itself, because pens that share a drawing overlap on purpose: their inks mix where they cross. It counts three passes and up, because two are normal where hatching crosses and where an ink fill's rings meet.

## 4. Checks

`lint::check(drawing, measure, style, settings, pens, speeds, machine) -> Vec<Finding>` runs the checks in this order over the session as it stands, so the patches start from the current settings. A finding is:

```json
{"code": "close_lines", "severity": "warning",
 "message": "Lines 0.2 mm apart are closer than the 0.3 mm pen, so they overlap.",
 "value": 0.2, "patch": {"settings": {"hatch": {"spacing_mm": 0.3}}}}
```

`severity` is `error` (the plot would go wrong: off the paper or the machine), `warning` (it would waste ink, paper or time) or `info` (worth knowing). `patch` has the shape of an analysis suggestion's, any of `style`, `settings` and `page`, or is null for a note. A patch value rounds to the setting's 0.1 step and stays inside the setting's range, which the check reads from the stage's schema. "The spacing" below is the style stage's `spacing_mm`, or `min_spacing_mm` for `flow`; "the widest pen" is the widest pen with a layer.

| `code` | Severity | When | Patch |
|---|---|---|---|
| `empty` | warning | no paths, or the laid ink is under 0.1 % of the drawing area | none for a blank tone map, a minimum length above 0, Pen use by colour or a held tone response; with no paths, a threshold setting lowered to the ink of the map's darkest pixels where enough of them lie to draw; with paths, the density setting one step denser (doubled or halved) where the marks' pixels have room; else none (amended on 2026-10-16) |
| `off_page` | error | a vertex lies outside the page | none |
| `off_machine` | error | a machine is set and the page does not fit its drawable area | the page turned, when that fits |
| `off_area` | warning | a vertex lies past the margin by more than half its pen width | none |
| `close_lines` | warning | the spacing is below the widest pen's width | the spacing set to that width |
| `overdraw` | warning | in some layer, over 5 % of the inked cells are passed three times or more; skipped when `close_lines` fired, which explains it | `hatch.levels` lowered by one when above 1; else the spacing raised by half; else none |
| `specks` | warning | at least 10 paths, and 1 % of the paths, are shorter than their pen width, in a drawing where dots are under half the paths | `order.min_length_mm` set to the widest pen's width |
| `dark` | info | the dark coverage is under 0.35, and `empty` did not fire | the spacing lowered in proportion to reach about 0.5, no closer than the widest pen, where the dark parts span room for it; else a point count (`count`, at most a quarter of the inked pixels, with at least 16 new points in the dark parts) or stroke budget (`max_lines`, only when a layer drew its limit) raised in proportion; none with a minimum length above 0 or a pen thinner than a raster cell (amended on 2026-10-16) |
| `lifts` | info | pen lifts take over half the estimated plot time | none: the message names a long-stroke style |
| `travel` | warning with `order.optimize` off, info with it on | pen-up travel takes over 30 % of the estimated plot time | `order.optimize` on when it is off and the nearest-neighbour order of each layer, worked out by the ordering stage, gives a lower travel share; else none |
| `vertices` | info | in some layer with at least 1000 segments, over half are shorter than an eighth of its pen width | `order.simplify_mm` set to an eighth of the narrowest pen, rounded down to 0.001 mm, when it is 0 and simplifying the drawing's paths to that, then dropping those under `order.min_length_mm`, would make the finding go away or lower its number; else none (amended on 2026-10-16) |
| `unused_pens` | info | Pen use is one pen and there are more pens | Pen use set to light to dark |
| `idle_pens` | info | Pen use is not one pen and some pen's layer has no paths | none |

The messages are English sentences with the measured numbers in them, written for a person and read by the assistant as well. `value` is the number the rule tests: the spacing, the overdrawn share, the dark coverage, a time share, a count.

The thresholds are taste defaults, chosen by running every style at its defaults on the still life and the scene so that they raise only what is true of the styles (grid's dots cannot reach dark; dot styles are lift-heavy), and recorded in the brief with the numbers behind them.

## 5. The session

`Session::lint() -> Option<Vec<Finding>>` runs the checks on the last finished drawing and its measurement, with the session's current style, settings, pens, speeds and machine; `None` before any drawing. The drawing is measured on the first call after it is made and the session keeps the measurement, by a weak reference to its drawing, until the drawing changes. A run nobody lints, such as the GUI's trial of every style, costs nothing. The checks are cheap, so a front end may call `lint` as often as it reads the stats.

## 6. Front ends

- **Python:** `Session.lint()` returns a list of dicts, and raises like `stats()` without a result.
- **CLI:** `plotline run` prints each finding to stderr as `<severity>: <message>` after the stats, followed by `  fix: --set stage.key=value …` (or `--width` and `--height` for a page) when it has a patch. `--quiet` still prints them. `--strict` exits with status 1 when any finding is a warning or an error, after writing the output.
- **MCP:** `run` returns the stats JSON with a `lint` key holding the findings. Each patch applies with `run(style=..., settings=...)`, as an analysis suggestion's does.
- **GUI:** the `end` event and the state carry the findings. A list under the stats shows each message, with an Apply button for a patch, reusing the suggestions list's markup and the `apply` command. The stats line keeps "No marks at these settings." and the `empty` finding gives the advice. The one-pen note leaves the pens panel; the `unused_pens` finding says it after a run. A tour step points at the list.

## 7. Tests

- Coverage: a straight line covers the expected cells; two crossing paths count two at the crossing; a path that crosses itself counts once; a dot stamps a disc; marks off the grid are clipped; the cell size keeps the bound; the same drawing gives the same counts twice.
- Lint: each check fires on a small made-up drawing or setting that should trip it and stays quiet on one that should not; grid and hatch at their defaults on a small still life raise only the expected codes, and their patches apply through the session without error; one `dark` value is pinned natively and in the WebAssembly smoke test. For every check with a patch (`empty`, `close_lines`, `specks`, `dark`, `travel`, `overdraw`, `unused_pens`, `off_machine`), a real run trips it, the patch is applied through the session, and a second run no longer finds it.
- Hosts: Python `lint()`; the CLI prints findings and `--strict` fails; the MCP `run` result has `lint`; the GUI protocol and tour tests.
