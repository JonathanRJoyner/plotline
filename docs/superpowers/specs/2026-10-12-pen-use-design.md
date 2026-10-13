# plotline spec 8: drawing with several pens

Date: 2026-10-12. Status: approved in conversation; written for review.

Every style draws with the first pen: its last stage gives one set of paths, and the runner puts them on `pens[0]` ([runner.rs](../../../crates/core/src/runner.rs), `Run::drawing`). Everything past that point already handles several layers: the SVG has a group per pen, G-code is one file per layer, the preview shows or hides each pen, and the stats add up every layer. What is missing is a step that decides which pen draws which part of the image. The owner asked for multi-pen drawing and left the design to the builder.

This spec adds that step once, for every style, instead of a multi-pen style family (§7.2 of the brief). A new shared stage, **Pen use**, turns the tone map into one map per pen, and the runner runs the rest of the style once per pen.

## 1. Scope

- `crates/core/src/stage.rs`: a `Tones` kind, and a lane in `RunContext`. `Frame` gains `pen`.
- `crates/core/src/style.rs`: inputs after a `Tones` stage resolve per lane.
- `crates/core/src/runner.rs`: runs the stages after the split once per pen, with their own cache keys, and builds one layer per pen.
- `crates/core/src/stages/split.rs` (new): the Pen use stage and its three modes.
- `crates/core/src/stages/tone.rs`: the crop and resize steps become `pub(crate)`, so the split stage can sample the image's colour on the same grid.
- `crates/core/src/stages/{hatch,waves,spiral,grid}.rs`: spread each pen's lines or dots so pens do not draw on top of each other.
- `crates/core/src/registry.rs`: every style except `_slow` gets the split stage right after tone.
- Hosts: the frame's pen reaches Python (`Frame.pen`), the WebAssembly handle and the worker's frame event.
- `web/`: the Pen use setting sits in the Pens panel, pens can be moved up, and the preview draws each pen's frames in that pen.
- `src/plotline/cli/main.py`: the `--pen` help stops saying the extra pens are kept for later.
- Docs: the brief (§5, §7.2, §10 and the decision log) and the README.

Out of scope:
- Choosing pen colours from the image (a palette picked by clustering).
- A per-pen switch, and per-pen settings such as a different spacing for each pen.
- Measuring real ink colours.
- Opaque inks such as paint markers, which cover rather than mix.
- A pause or pen-change prompt in G-code. Each pen is already its own file.
- Timing pen swaps in the plot-time estimate.

## 2. What the user sees

The number of pens is the list in the Pens panel, as now. Below the list, one select, **Pen use**:

| Value | Shown as | What it does |
|---|---|---|
| `one_pen` (default) | one pen | The first pen draws everything, as before this spec. The other pens stay empty. |
| `light_to_dark` | light to dark | Pens share the darkness in list order. The first pen draws from the lightest tones; each later pen adds only to darker ones. Suits one colour in several widths, or a grey pen and a black one. |
| `by_colour` | by colour | Each pen draws the part of the image its colour makes up. Works for any set of pen colours, such as cyan, magenta, yellow and black, or brown and black. |

The pen list sets the drawing order: the first pen is plotted first. The help text says to list light inks first.

Each pen row gets a "Move up" icon button, disabled on the first row, because the list order now decides the bands and the drawing order. The note under the list changes: in `one_pen` with more than one pen it reads "Pen use is set to one pen, so only the first pen draws." In the other modes there is no note.

Old presets have no `split` settings and load as `one_pen`, so they draw what they drew before. No preset version bump is needed: missing keys already take their defaults.

## 3. The Pen use stage

`split` ("Pen use") requires the image, the canvas, the pens and the tone map, and provides `Tones`: one tone map per lane, each with the tone map's size, pixel size and origin. It has one setting:

```rust
/// How the pens share the drawing: the first pen alone, light to dark in list order, or each pen by its colour.
pub mode: SplitMode, // one_pen | light_to_dark | by_colour, serde snake_case
```

It runs in one step that reports a frame labelled "pen use", then gives the maps. The GUI shows enum values with underscores as spaces; that is a one-line change in `schema.ts`'s `optionsOf`.

### 3.1 One pen

One lane holding the tone map unchanged.

### 3.2 Light to dark

`n` lanes for `n` pens. Lane `k`, counting from 0, takes ink `clamp(n t - k, 0, 1)`, where `t` is the tone map's ink. The first pen reaches full ink at a tone of `1 / n`, and pen `k` starts only where the tone passes `k / n`. This is the rule hatch levels already use, applied across pens: each stroke family adds darkness over the ones before (Praun, Hoppe, Webb and Finkelstein, "Real-time hatching", SIGGRAPH 2001, on tonal art maps). With one pen it equals one pen mode.

### 3.3 By colour

`n` lanes for `n` pens, lane `i`'s ink being pen `i`'s coverage.

**The colour on the tone grid.** The stage samples the image's red, green and blue on the tone map's grid. It uses the tone stage's own crop and resize, with the long edge read from the tone map's size. Resizing is linear, so the BT.601 luminance of the result matches the luminance the tone stage resized, up to rounding.

**Carrying the Image settings over.** The tone map holds the darkness the user asked for through levels, gamma, contrast, invert and blur. Each pixel's colour `c` is moved so that the luma of its inks becomes the tone map's ink `t`. With channel inks `ink_k = 1 - c_k` and their BT.601 luma `u` (the weights of `Image::luminance`), each ink becomes `ink'_k = t + beta (ink_k - u)`, where `beta = min(1, (1 - t) / (max_k ink_k - u), t / (u - min_k ink_k))` and a bound whose denominator is not positive is skipped. The new colour is `c'_k = 1 - ink'_k`, clamped to [0, 1] only against rounding. Because the weights sum to 1, the luma of the new inks is exactly `t`. At the default settings `t = u`, so `beta = 1` and the colour is unchanged. A grey or white pixel skips both bounds and becomes the grey `1 - t`, so white needs no special case. The rule is continuous, and it reduces the chroma only as far as needed to keep every channel in range, so a near-white pixel under invert goes to near black rather than to a saturated hue. So the Image settings act on a colour drawing the way they act on a grey one, and the hue comes from the image.

**The ink model.** Pens are treated as transparent inks laid down at independent positions. This holds once each pen's marks are turned against the others' (§5), as for rotated print screens (Amidror and Hersch, "Neugebauer and Demichel: dependence and independence in n-screen superpositions for colour printing", Color Research and Application 25(4), 2000). Under independence, the Demichel equations give the share of paper under each overlap of inks, and the Neugebauer model sums their colours. For one channel, that sum is the product

```
R_c(a) = prod_i (1 - a_i k_ic),   k_ic = 1 - p_ic
```

where `a_i` in [0, 1] is pen `i`'s coverage and `p_ic` its colour's channel `c`. Colours are used as stored, sRGB-encoded in [0, 1], not linearised. That is deliberate: the tone stage takes ink as one minus the encoded luminance, so a black pen on a grey image covers exactly what one pen mode gives it. A product of colours is also nearly the same in encoded and linear values, because the encoding is close to a power law.

**Solving for the coverages.** For a target colour `t`, the coverages minimise

```
E(a) = sum_c (R_c(a) - t_c)^2 + lambda sum_i a_i,   0 <= a_i <= 1,   lambda = 1e-3
```

The small ink penalty picks one solution when several match. A black pen reaches the same grey with less ink than cyan, magenta and yellow stacked, so the black pen takes the greys (grey component replacement falls out of the penalty).

`R_c` is linear in each `a_i` alone, so `E` is a quadratic in each coverage and exact coordinate descent applies (Wright, "Coordinate descent algorithms", Mathematical Programming 151, 2015). With `Q_c` the product over the other pens and `g_c = k_ic Q_c`, each update is

```
a_i <- clamp((sum_c g_c (Q_c - t_c) - lambda / 2) / sum_c g_c^2, 0, 1)
```

or 0 when the denominator is 0 (a white pen). No update raises `E`. The solve starts from all zeros and makes 64 sweeps, darkest pen first (by total absorption, ties in list order), in f64 with plain arithmetic, so it gives the same bits on every target. Darkest first matters: from zero, list order lets cyan, magenta and yellow claim a black before the black pen is visited, a coordinate-wise minimum the descent cannot leave.

**A lookup table.** Solving every pixel would be slow, so the stage solves the 17 × 17 × 17 grid of colours in [0, 1]³ once per run and interpolates the coverages of each pixel's rescaled colour trilinearly. A pixel's ink in lane `i` is its interpolated `a_i`, clamped to [0, 1].

A pen whose colour the image does not contain gets an empty map. Its lane draws nothing, and its layer has no paths.

## 4. Lanes in the runner

**The style.** A stage that provides `Tones` splits the style. The stages after it run once per lane, and in each lane:
- a requirement for `Tone` resolves to that lane's map;
- a requirement for `Pens` resolves to a one-pen list holding that lane's pen, so the ink stage takes its own pen's width with no change;
- other requirements resolve as now.

`Style::new` refuses a style with two `Tones` stages, or a split style whose last stage gives `Layers`; the message names the stage. A style without a split runs as now, with one lane.

**The run.** The runner runs the stages before and including the split once, as now. Then it runs the rest lane by lane: every stage of lane 0, then lane 1, and so on. The cancel flag is checked before every step, as now. `RunContext::lane()` returns `Lane { index, count }`, which is `{0, 1}` outside the lanes.

**Cache keys.** A lane's tone map is keyed by the BLAKE3 hash of its content: width, height, pixel size, origin and every ink value, in the byte layout `root_key` uses for the image. Its pen list is keyed like the pens root. A lane stage's key adds a `lane` entry, `"{index}/{count}"`, to the key's inputs, because a stage may read its lane. So:
- an unchanged lane map reruns nothing downstream, whatever else changed;
- a pen's colour or width reruns only the lanes that read it;
- changing order settings reruns only the order stage of each lane.

**The drawing.** With lanes, layer `i` is pen `i` with lane `i`'s last paths, empty or not. Keeping empty layers keeps layer `i` as pen `i`, which the preview's per-pen toggles and the G-code buttons rely on. Without lanes, the drawing is built as now.

**Frames.** `Frame` gains `pen: usize`, which the runner sets, as it sets `stage`: the lane index in a lane and 0 otherwise. The stages build their frames with `pen: 0`.

## 5. Keeping pens apart

If every pen drew the same lines, each pen would cover the one before and the inks would not mix. Following rotated print screens (Amidror, *The Theory of the Moiré Phenomenon*, 2nd ed., 2009), each style spreads its marks evenly across the pens. With one lane, every rule below gives today's output.

- **hatch.** Level `k` of pen `i` is drawn at `angle_deg + 180 (k n + i) / (levels n)`. With one level and two pens, the second pen's lines are square to the first.
- **waves.** Pen `i`'s lines run at `angle_deg + 180 i / n`.
- **spiral.** Pen `i`'s spiral is turned by `360 i / n` degrees about the centre. The arms of an Archimedean spiral turned this way interleave and never cross.
- **grid.** Pen `i`'s grid moves by `spacing i / n` across and down, and keeps the whole cells of the moved grid that fit in the tone rect. Two pens give a staggered (quincunx) lattice.
- **ink** and **order** do not change.

## 6. Hosts and GUI

- **Python.** The frame dict gains `"pen"`, and `plotline.Frame` gains a `pen: int` field. The CLI sets the mode with `--set split.mode=by_colour`, with no new flag. The `--pen` help reads: "Repeat the option to give more pens; set split.mode to share the drawing between them."
- **WebAssembly.** `FrameHandle` gains a `pen` getter, and the worker's frame event carries `pen`.
- **Settings form.** `schema.ts`'s `splitStages` leaves `split` out of the left column. The app renders its section inside the Pens panel, below the pen list.
- **Preview.** While a run is going, the preview keeps the latest frame with geometry for each pen and draws all of them, each in its own pen's colour and width. A finished pen's lines therefore stay on the page while the next pen runs. A new run clears them. Once the run ends, the result is drawn per layer, as now.

## 7. Tests

- `split.rs`:
  - one pen passes the map through unchanged;
  - light to dark with two pens on `gradient` gives `clamp(2t)` and `clamp(2t - 1)`, and with one pen equals one pen;
  - by colour with one black pen on a grey image matches the tone map within 1e-3;
  - by colour with cyan, magenta, yellow and black pens on `color_bars`:
    - the white bar takes no ink;
    - the black bar goes to the black pen, with cyan, magenta and yellow under 0.2;
    - the red bar goes to magenta and yellow, with cyan and black under 0.2;
    - the cyan bar goes to cyan;
  - a white pen takes no ink;
  - coordinate descent never raises `E`, checked on random colours with a fixed seed;
  - the output is deterministic.
- `style.rs`: lane resolution, and the two refusals named in §4.
- `runner.rs`, through `Spy` stages:
  - two lanes give two layers on pens 0 and 1;
  - lane frames carry their pen;
  - a second run with the same settings is all cached;
  - changing pen 1's colour reruns no lane stage when the split is in one pen mode;
  - empty lanes keep their layer;
  - cancel works between lanes.
- Each spreading stage: one lane gives the same paths as before; two lanes give different angles (hatch, waves), a turned start (spiral) or shifted centres (grid), and every vertex stays inside the tone rect.
- `tests/settings_matter.rs`: the session gets two pens, black and red, so `split.mode` changes the drawing. The no-op list gains nothing new unless the test finds a reason.
- `tests/golden.rs`: passes unedited. The default is one pen, so the grid output does not change.
- `registry.rs`: every listed style has the split stage after tone.
- Python: `Frame.pen`; a session with two pens and `by_colour` on the scene image gives two layers, both with paths.
- WebAssembly smoke test: the frame handle exposes `pen`.
- vitest: underscores read as spaces in enum options, and `splitStages` leaves out `split`.
- Manual, in `npm run dev`, on the sample and scene images:
  - all three modes with two and with four pens;
  - Move up reorders the pens;
  - each pen's frames draw in its own colour, and finished pens stay on the page while the next one runs;
  - G-code exports one file per pen.

## 8. Decisions recorded in the brief

- **Pen use.** Multi-pen drawing is one shared stage after tone, with three modes, rather than a style family. Every style gains it with no change to its own drawing code, and caching stays per stage. §7.2's multi-pen row and §10's input line are updated to match.
- **Colour model.** Pens are transparent inks at independent positions (Demichel and Neugebauer). Coverages are solved by exact coordinate descent with a small ink penalty, on encoded colour values, through a 17³ lookup table. Encoded values keep a black pen in colour mode equal to one pen mode on a grey image, the penalty gives the greys to a black pen, and the table keeps the cost per pixel at one trilinear lookup.
- **Pens kept apart.** Each style spreads its angles, start or grid across the pens, with no setting. That stops pens covering each other and lets the independence the colour model assumes hold.
- **Frames in the preview** (amends the earlier row). Frames carry their pen, and the preview draws each pen's latest frame in that pen.
- **Lane cache keys.** A lane's tone map is keyed by its content, so changing a pen or the mode reruns only the lanes whose map changed.
