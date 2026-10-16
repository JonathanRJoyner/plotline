# plotline spec: looks

Date: 2026-10-16. Status: built. The owner delegates the taste defaults, which are recorded in the brief's decision log ("Looks").

The owner suggested presets within each style, built from what we see as optimal settings. A preset file holds a whole setup (style, page, pens, machine, settings), which is the wrong size for this: a starting point inside one style should change the few settings that set its character and leave the image, the page, the pens and the user's own adjustments alone. This spec calls such a starting point a **look**: a named settings patch declared with the style, like the style's own default overrides, and chosen by measurement rather than by eye.

## 1. Scope

- `crates/core/src/style.rs`: looks on `Style`, declared with `with_look`, listed in `schema()`.
- `crates/core/src/registry.rs`: each style's looks, with the measured values.
- `crates/core/src/session.rs`: `Session::apply_look` and `Session::look`.
- `crates/core/examples/looks.rs` (new): the measurement that chose the values, so it can be run again.
- `crates/core/tests/laws.rs`: the look law.
- Bindings, CLI, MCP and GUI, with a tour step and a browser flow.
- Docs: the brief's decision log, §6.3, §8 and §9, the README, the CHANGELOG.

## 2. Looks

Every style has the look **Balanced**, its defaults. A style may declare up to three more, always from this list and in this order:

| Name | Label | What it is |
|---|---|---|
| `balanced` | Balanced | The style's defaults. |
| `faithful` | Faithful | The settings whose drawing follows the image's tones most closely on the test images. |
| `quick` | Quick | Sparser marks that plot in about half an hour on A4 at the default speeds. |
| `fine` | Fine | The densest marks that still plot cleanly with a 0.3 mm pen and keep the tones. |

The names are plain words of our own. Each look has a one-sentence description, the same for every style, which the GUI shows as help and the MCP and CLI list.

A look's **keys** are the settings any look of the style sets. A look's settings are its own values for its keys and the style's defaults for the rest of the keys, so every look of a style sets the same keys, and Balanced sets them all to their defaults.

### 2.1 Declaring

`Style::with_look(name, patch)` adds the look `name` with `patch` (`{stage: {key: value}}`) over the style's defaults. Like `with_defaults`, it panics for an unknown look name, a look declared twice or out of order, an unknown stage, or a value the stage's schema refuses, since built-in styles are fixed at compile time. A look whose settings equal another's is refused too.

`Style::looks()` lists every look, Balanced first, as `Look { name, label, description, settings }` with `settings` over all of the style's keys. `Style::schema()` gains `"looks": [{"name", "label", "description", "settings"}]`, so `describe_style`, `plotline describe` and the GUI all read them from one place.

### 2.2 Applying

`Session::apply_look(name)` sets each key to the look's value through `Session::update`, so a key set to its default stops counting as changed and any other counts as the user's change. Every other setting stays as it is: the image adjustments, Pen use, seeds and any setting no look of the style sets. An unknown look fails with `SessionError::UnknownLook`, naming the style's looks, and changes nothing.

`Session::look()` is the name of the first look whose settings all equal the current ones, numbers by value, or `None` when none does ("Custom" in the GUI). It is worked out from the settings on each call, so nothing new is stored: undo, a preset, a reset and a style switch all show the right look with no extra state.

Consequences:

- **Reset to defaults** puts every key back to its default, so the look becomes Balanced.
- **Undo.** A look is one command, so it is one step in the GUI's history; Undo restores the setup before it, and the look shown follows.
- **Style switch.** A look's values are the user's changes, so a switch keeps those of stages both styles share, as for any change ("Settings across a style switch"). Switching from `stipple` at Fine to `line` keeps the point count, so the points are reused; the new style's look then shows as whichever of its looks matches, often none.
- **Presets** need no change: a preset holds the settings, and the look follows from them.

Why a patch over the current settings rather than a reset first: a reset would throw away the user's image adjustments and Pen use, which have nothing to do with how dense the marks are. Why work the look out rather than store it: a stored name would go stale on every edit, undo and preset load, each of which would need to clear or set it.

## 3. Choosing the values

The values come from `cargo run --release -p plotline-core --example looks`, which measures every listed style with a density setting (the one the budget fit and Optimize change) on the regression gallery's three test images (the still life, the scene and the gradient, 600 by 875 pixels), on A4 with the default 10 mm margin, the default 0.3 mm pen and the default speeds. The tone error is the one Optimize minimises ([tone response spec](2026-10-15-tone-response-design.md)), averaged over the three images.

It scans the density on a grid from a quarter to four times the default, steps of about 1.19 rounded to the setting's grain (the 0.1 mm step of a length, two significant figures of a count), with the tone stage's `response` off and `auto`. Two limits hold for every look:

- **Clean.** No plot lint finding of severity warning or error on any image. This is the law of section 5.
- **Practical.** On each image, no longer to plot than twice Balanced or an hour, whichever is longer.

Then:

- **Faithful** takes the scan's lowest mean tone error within the limits. `auto` is used only where it beats `off` by 5 %, since it is one more key and makes the Voronoi styles relax again on a renderer change. The look exists only where it beats Balanced by 10 %.
- **Quick** exists where Balanced's median plot time is over 40 minutes. The budget fit runs from the defaults for 30 minutes on each image, and the look takes the median, rounded, if that is sparser than the defaults and clean.
- **Fine** takes the densest value of the `off` scan that is denser than the default, within the limits, with a mean tone error at most a quarter above Balanced's, and with every value between it and the default passing too.

Where Fine lands within one step of the scan from Faithful, the style keeps Faithful alone. The two then draw about as densely, Faithful's tones are no further from the image, and on some styles they differ only in `response`; two entries that look alike would make the user choose for nothing.

The first plan took Faithful as the median of what Optimize finds on each image, and Fine as the densest clean value up to four times the default. A first measurement showed both fail: Optimize's eight golden-section trials land in different local minima of a bumpy error (for `flow` with `auto`, 0.65, 1.05 and 1.29 mm on the three images), Faithful for `dither` went to the pen's width at ten times Balanced's plot time (7 to 18 hours), and Fine for `waves` reached 0.5 mm, where the ripples run together and the tone error doubles. A scan scores every candidate on all three images at once, and the two limits and Fine's tone bound keep the looks to drawings someone would plot.

A style skips a look the rules do not give it. Section 7 records what each style got and why.

## 4. Front ends

- **Python:** `Session.apply_look(name)` and `Session.look`; `get_style(name).schema()` lists the looks.
- **CLI:** `plotline run --look NAME` applies the look after `--preset` and the style and before `--set`, so `--set` adjusts a look. An unknown name exits 2 with the style's looks. `plotline describe STYLE` lists them.
- **MCP:** `run(look=...)` applies the look after `reset` and before `settings`; `describe_style` lists the looks through the schema; `get_session` gains `look`. Still fourteen tools.
- **GUI:** a Look select under the style select, showing the current look or "Custom", with the look's description as help, hidden for a style with no look but Balanced. Choosing a look sends one `applyLook` command. The tour gains a "Pick a look" step and `gui.spec.ts` a flow: choosing a look changes the density setting's field and redraws, Undo goes back to Balanced, and Reset to defaults shows Balanced again.
- **Words.** The tour and the README called a preset a way to "keep a look". A preset now keeps a setup, so the word means one thing.

## 5. Law

`every_look_plots_clean` in `crates/core/tests/laws.rs` draws a style and one of its looks other than Balanced from the registry, and one of the three test images, and runs it on A4 at the default pen: the plot lint finds no warning or error. Balanced is left out because it is the defaults, which this work does not change and some of which warn (`contours` on the scene overdraws). A pull request runs a fixed number of cases; the weekly run's 25 times covers every look on every image with high probability.

A registry test checks every style's looks: each applies, is then the session's look, and no two of a style's looks have equal settings.

## 6. Tests

- Style: `with_look` merges over the defaults, fills the other keys with defaults, keeps the declared order, and panics on a bad name, stage or value; `schema()` lists the looks.
- Session: `apply_look` sets the keys and nothing else, marks them changed or not as `update` does, fails on an unknown name without changing anything; `look()` follows an edit (to `None`), a reset (Balanced) and a switch there and back.
- Hosts: Python `apply_look` and `look`, the CLI's `--look` and its error, the MCP `run(look=...)` and `get_session`'s `look`, the GUI protocol, tour and browser flow.
- The default drawings do not change: Balanced is the defaults, so the golden files and the stats snapshot stay as they are.

The regression gallery gets no cells for the looks. Its grid is a row per style and a column per image and pen (0.1 and 0.5 mm), while the looks are measured at 0.3 mm; a row per look at those pens would double the gallery for drawings whose settings the registry diff already shows. A change to a stage moves the Balanced cells, the law catches a look that starts to warn, and `--example looks` measures them all again.

## 7. Measurements

Measured on 2026-10-16 on main after the tone-response merge (00561ad), with `cargo run --release -p plotline-core --example looks` (5 minutes on a 20-thread desktop). The tone error is the mean over the three images; no look below warns on any image. Balanced is listed for comparison; its warnings are the defaults' own.

| Style | Look | Settings | Tone error | Minutes (still life, scene, gradient) |
|---|---|---|---|---|
| `grid` | Balanced | defaults | 0.151 | 22, 131, 92 |
| `grid` | Quick | `points.spacing_mm=1.8` | 0.151 | 7, 42, 30 |
| `grid` | Fine | `points.spacing_mm=0.8` | 0.149 | 34, 204, 143 |
| `hatch` | Balanced | defaults | 0.069 | 10, 35, 29 |
| `hatch` | Fine | `hatch.spacing_mm=0.7` | 0.077 | 14, 50, 41 |
| `waves` | Balanced | defaults | 0.043 | 15, 26, 24 |
| `waves` | Faithful | `waves.spacing_mm=2.8` | 0.021 | 14, 26, 23 |
| `waves` | Fine | `waves.spacing_mm=1.0` | 0.050 | 23, 32, 30 |
| `spiral` | Balanced | defaults | 0.026 | 15, 26, 24 |
| `spiral` | Fine | `spiral.spacing_mm=1.4` | 0.030 | 18, 28, 26 |
| `ink` | Balanced | defaults | 0.144 | 10, 54, 37 (specks) |
| `stipple` | Balanced | defaults | 0.065 | 22, 23, 23 |
| `stipple` | Fine | `voronoi.count=12000` | 0.065 | 51, 52, 52 |
| `line` | Balanced | defaults | 0.149 | 5, 5, 5 |
| `line` | Faithful | `tone.response=auto, voronoi.count=20000` | 0.056 | 9, 10, 9 |
| `dashes` | Balanced | defaults | 0.075 | 21, 21, 21 |
| `dashes` | Faithful | `voronoi.count=12000` | 0.066 | 49, 49, 48 |
| `sketch` | Balanced | defaults | 0.301 | 10, 10, 10 |
| `sketch` | Faithful | `sketch.max_lines=20000, tone.response=auto` | 0.072 | 20, 54, 51 |
| `sketch` | Fine | `sketch.max_lines=10000` | 0.136 | 20, 20, 20 |
| `tree` | Balanced | defaults | 0.246 | 10, 11, 10 |
| `tree` | Faithful | `tone.response=auto, voronoi.count=20000` | 0.088 | 28, 29, 28 |
| `mesh` | Balanced | defaults | 0.112 | 20, 21, 20 |
| `mesh` | Faithful | `tone.response=auto, voronoi.count=6000` | 0.070 | 21, 24, 21 |
| `mesh` | Fine | `voronoi.count=20000` | 0.091 | 49, 52, 50 |
| `cells` | Balanced | defaults | 0.100 | 28, 29, 28 |
| `cells` | Faithful | `tone.response=auto, voronoi.count=8400` | 0.061 | 42, 43, 42 |
| `cells` | Fine | `voronoi.count=12000` | 0.078 | 57, 58, 58 |
| `halftone` | Balanced | defaults | 0.023 | 60, 80, 76 (specks) |
| `dither` | Balanced | defaults | 0.048 | 42, 103, 93 |
| `dither` | Faithful | `dither.spacing_mm=0.8` | 0.030 | 64, 160, 145 |
| `dither` | Quick | `dither.spacing_mm=1.8` | 0.077 | 14, 34, 30 |
| `circles` | Balanced | defaults | 0.121 | 5, 13, 12 |
| `circles` | Faithful | `poisson.spacing_mm=1.5` | 0.046 | 18, 48, 44 |
| `contours` | Balanced | defaults | none on the scene | 2, 4, 3 (overdraw) |
| `flow` | Balanced | defaults | 0.177 | 5, 7, 10 |
| `flow` | Faithful | `flow.min_spacing_mm=0.3, tone.response=auto` | 0.112 | 14, 27, 32 |
| `hilbert` | Balanced | defaults | 0.098 | 10, 26, 21 |
| `hilbert` | Faithful | `hilbert.cell_mm=0.3, tone.response=auto` | 0.081 | 14, 32, 29 |
| `scribble` | Balanced | defaults | 0.089 | 12, 15, 18 |
| `scribble` | Faithful | `voronoi.count=20000` | 0.059 | 24, 30, 35 |
| `mosaic` | Balanced | defaults | 0.044 | 22, 32, 26 |
| `mosaic` | Faithful | `mosaic.spacing_mm=0.3` | 0.017 | 42, 60, 48 |
| `truchet` | Balanced | defaults | 0.126 | 10, 20, 18 |
| `truchet` | Faithful | `truchet.spacing_mm=0.3` | 0.104 | 12, 25, 23 |

What the table says, style by style:

- **The connected Voronoi renderers** (`line`, `tree`, `mesh`, `cells`) and `sketch` gain most, from `auto` and more points or lines: `line`'s error falls from 0.149 to 0.056, `tree`'s from 0.246 to 0.088. Their coverage follows the square root of the density, which `auto` straightens (the tone response spec, section 6). `line` and `tree` hit the scan's reach of four times the default count; the error was still falling there.
- **Spacing styles** that ink close to in proportion, `waves` and `mosaic`, gain from their spacing alone. `waves` is most faithful at 2.8 mm, sparser than its 2 mm default, at the same plot time. `mosaic` and `truchet` are most faithful at the pen's width, where their lines fill the darks.
- **`dither` and `circles`** get closer as they get denser almost to the pen's width: `dither` reaches 0.006 at 0.3 mm in ten times Balanced's plot time, `circles` 0.030 at 0.9 mm. So the time limit set Faithful, and Fine came out within a step of it and was dropped.
- **Quick** exists for `grid` and `dither`, whose Balanced takes 92 and 93 minutes on the median image. Every other style plots in under 40 minutes on the median image. `halftone`'s Quick (3.2 mm) leaves specks on the gradient, as its Balanced does.
- **Fine** survives where it is a real step denser than Faithful or where there is no Faithful: `grid`, `hatch`, `waves`, `spiral`, `stipple`, `sketch`, `mesh` and `cells`. `hatch` stops at 0.7 mm, since 0.6 mm passes cells three times on the gradient (`overdraw`); `waves` at 1.0 mm and `spiral` at 1.4 mm, where the tone error bound stops them.
- **No looks:** `ink` has no density setting; `halftone` leaves specks on the gradient at every spacing from 5.7 mm down, its 2 mm default included, and the clean ones, 6.8 mm and wider, lose the picture (tone error 0.36 and more); `contours` overdraws on the scene and the gradient at its defaults and has no tone match on the scene. A look for them needs another setting than the density, which is left for later.
- **Faithful skipped:** `grid`, `hatch`, `spiral` and `stipple` come within 10 % of Balanced's tone error at their best (`hatch`'s best is 0.0625 at 2 mm, 9.7 % better), so they offer only Quick or Fine.

