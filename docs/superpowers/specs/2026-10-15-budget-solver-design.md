# Budget solver design

Status: built on 2026-10-15. Decisions are recorded in the brief's decision log under "Budget solver".

## Goal

The user sets a target and plotline finds the value of the style's density setting that meets it. A target is one of:

| Metric | Unit | Measured as |
|---|---|---|
| `minutes` | min | the estimated plot time at the session's speeds (`Stats::estimated_seconds / 60`) |
| `paths` | paths | the path count over all layers |
| `metres` | m | the pen-down length over all layers |
| `dots` | dots | the points the style placed, summed over the pens' lanes; only for a style with a stage that places points |

This is step 1 of the "optimize" button the owner asked for: meeting a budget. Matching tone was tried and shelved (brief, "Tone matching"); a budget has one clear number to hit, so it avoids that search's failure mode of trading the drawing's look for a score.

The time estimate ignores acceleration, so on short-segment styles (dots, tight waves) a time target runs optimistic. The help text in every front end says so.

## Which setting

A stage marks its density setting in its schema with the extension `"x-density"`:

- `"inverse"`: the metric falls as the value rises, as with a spacing.
- `"direct"`: the metric rises with the value, as with a count.

Marked settings:

| Stage | Setting | Direction | Styles |
|---|---|---|---|
| `points` (grid placer) | `spacing_mm` | inverse | grid, halftone |
| `hatch` | `spacing_mm` | inverse | hatch |
| `waves` | `spacing_mm` | inverse | waves |
| `spiral` | `spacing_mm` | inverse | spiral |
| `dither` | `spacing_mm` | inverse | dither |
| `poisson` | `spacing_mm` | inverse | circles |
| `voronoi` | `count` | direct | stipple, line, dashes, tree, mesh, cells |
| `sketch` | `max_lines` | direct | sketch |
| `hilbert` | `cell_mm` | inverse | hilbert |
| `contours` | `levels` | direct | contours |
| `flow` | `min_spacing_mm` | inverse | flow |

`ink` has none: it draws the marks the image has. A registry test checks that every listed style other than `ink` has exactly one marked setting, and that each mark is `inverse` or `direct` on a ranged number or integer. The solver reads the mark and the schema's `minimum`, `maximum` and `type`; it has no per-style code.

## Algorithm

Let `u = ln(value)` and `f(u) = ln(metric / target)`. The metric of most density settings follows a power of the value (pen-down length goes with 1/spacing, dots with 1/spacing²), so `f` is close to a straight line in `u` and a root finder on it converges in a few steps.

1. **Start** at the setting's current value, which is often near the answer and is never wasted: its run may already be cached.
2. **Bracket.** While every sample lies on one side of the target, step from the sample nearest the target along the secant through the two nearest samples: `du = -f / s`. The slope `s` is the measured secant when it has the mark's sign and a magnitude from 0.25 to 4, and otherwise the prior, +1 for `direct` and -1 for `inverse`. The step is clamped to the schema range. A sample whose metric is 0 has `f = -inf`; from it the next value is halfway, in `u`, to the range end that raises the metric. If the clamped value has been sampled already, the target is out of reach and the solver stops. When a sample already lies past the nearest one in the direction that should bring the metric closer, the metric fell off a cliff between them, which a monotone curve cannot do, so the next value is halfway to that sample instead. `hatch` needs this: it drops its lines once the spacing is under about half a tone-map pixel.
3. **Refine.** Once two samples bracket the target, the next value is the Illinois variant of regula falsi (M. Dowell and P. Jarratt, "A modified regula falsi method for computing the root of an equation", *BIT* 11(2), 1971, pp. 168-174): the false-position point of the bracket, where an end that is kept twice in a row has its `f` halved, so the bracket shrinks from both sides.
4. **Bisect** in `u` instead when the false-position point is undefined (an end with `f = -inf`) or when the newest sample broke monotonicity: when the samples sorted by value do not have their metrics in the mark's order. Threshold styles step and the minimum-length filter adds small jumps, so this happens.
5. **Round** each value to the setting's grain before it runs: whole numbers for an integer, 0.001 for a number. A value that rounds to one already sampled ends the search, since nothing new can be learnt.
6. **Stop** when a sample is within 2 % of the target (`|f| <= ln 1.02`), after 8 trials, or by rule 2 or 5. The result is the sample nearest the target in ratio, the smallest `|f|`; ties go to the earlier trial.

All arithmetic is plain `f64` with `ln` and `exp` from the crate's `math` module (libm), and the stop rules count trials, never time, so the native and WebAssembly builds sample the same values and pick the same answer. A test pins the samples of one fit in both builds.

## Engine

`Session::fit(metric, target) -> Result<FitRun, SessionError>` fails, changing nothing, without an image, for a target that is not finite and above zero, for a style with no marked setting, and for `dots` on a style with no point stage. Otherwise it cancels the latest run and claims the result slot as `start` does, and returns a `FitRun` over a snapshot of the session, its speeds included. (The name keeps clear of `types::Fit`, the canvas's fit mode.)

`FitRun` is driven like a `Run`: `next_frame`, `cancel`, `cancel_flag`, `state` and `error` behave the same, and the `Job` trait, which both implement, lets each host drive either with the code that drives a run. Each trial is a `Run` with the setting changed, sharing the session's cache and cancel flag but writing a private slot, so the session's slot shows the fit running until it ends. The tone and split stages stay cached across trials; only the stages from the density setting's stage on run again.

- A trial's frames pass through with their label prefixed, as `trial 3 of 8: relaxation 18/30`.
- After each trial the fit gives a frame of stage `fit` with no geometry, labelled `trial 3 of 8: 72.4 min`, at progress 1, so a host that throttles frames still shows it.
- A trial that is cancelled or fails ends the fit the same way, with the trial's message.
- At the end the fit writes the nearest trial's drawing to the session's slot as `done`.

`FitRun::result()` gives, once the fit is done, `{stage, key, metric, target, value, achieved, samples: [{value, metric}]}`. The fit does not change the session's settings, because the session is not reachable from the thread that drives a fit; each host applies the value with `update(stage, {key: value})` when the fit ends `done`. That rerun is all cache hits.

## Front ends

- **Python.** `_core.Session.fit(metric, target)` returns the same `_core.Run` class a plain run does, holding the job as a `Job`; its `result()` is `None` for a plain run. `plotline.Session.fit(metric, target)` runs it on the worker thread like `run` and returns the `RunHandle`, whose `fit` holds the result once it is done; the session applies the value on the worker before the handle reports `done`, so a reader that sees `done` sees the new setting.
- **CLI.** `plotline run` takes one of `--fit-minutes`, `--fit-paths`, `--fit-metres` and `--fit-dots` in place of a plain run, after the other options are applied, and prints the chosen value: `fit: points.spacing_mm = 1.234 (60.4 min for a target of 60 min, 5 trials)`.
- **MCP.** A twelfth tool, `fit(metric, target, style, settings)`, applies the optional style and settings as `run` does, fits, and returns the result with the sampled curve, the stats and a preview.
- **GUI.** Beside the stats in the status bar: a number field, a unit menu (min, paths, m, dots) and a Fit to target button, disabled without an image and named apart from the toolbar's Fit, which fits the view. The worker drives the fit as a run; its end event carries the result, and the page applies the value through `update`, which redraws from the cache. The status bar shows the fit's latest trial. A tour step names the field and the time caveat.

## Tests

- Solver: on functions with known roots (inverse and direct power laws, a step function, a non-monotone bump, an unreachable target, an integer setting) it lands within 2 % where it can, within 8 trials, and stops early where it cannot.
- Fit: on generated images, every metric lands within tolerance (`minutes` on hatch, `paths` on contours within 20 %, since levels are whole, `metres` on waves, `dots` on stipple), cached tone stages are not rerun, a cancel ends the fit cancelled, and errors are refused before anything changes.
- Determinism: a test pins the sampled values of one fit, and the WebAssembly smoke test checks the same literals.
- Hosts: the Python binding, the CLI flags, the MCP tool and the GUI's worker protocol each get a test.

## Not built

- Lift budgets over `order.min_length_mm` (research S5.2): the same solver over another setting, later.
- Fitting any setting by name: only the marked one is fitted, which keeps the front ends to a metric and a number.
- An acceleration-aware time estimate.
