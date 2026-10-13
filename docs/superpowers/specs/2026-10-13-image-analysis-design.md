# plotline spec 9: image analysis and auto levels

Date: 2026-10-13. Status: approved in conversation; written for review.

The first thing a user does with a new image is fiddle: stretch the levels of a grey scan, find the style that keeps line art, invert a chalkboard photo, turn the page for a landscape picture. This spec has the engine measure the image once and suggest those changes, and adds automatic levels to the tone stage. Every suggestion is a patch the user or the assistant applies; nothing changes on its own. All of it is plain arithmetic over the image, with no trained model, following the smart-features research of 2026-10-12.

## 1. Scope

- `crates/core/src/analysis.rs` (new): image statistics and suggestions.
- `crates/core/src/stages/tone.rs`: `levels` and `clip_percent` settings.
- `crates/core/src/session.rs`: `Session::analysis`, with the statistics cached per image.
- Bindings: `analysis()` in `crates/py` and `crates/wasm`; `Session.analysis()` in `src/plotline/session.py`.
- Front ends: `plotline analyse IMAGE` in the CLI; an `analyse_image` MCP tool; a suggestions list in the GUI.
- Docs: the README, the brief's decision log, and §9's tool list.

Out of scope: an automatic threshold as a stage setting, histogram equalisation and its local form, paper flattening (which needs the brief's §4 widened first), saliency maps, plot lint and the budget solver. Each is a later spec.

This work is stacked on the MCP server branch, because §6 adds an MCP tool. Its pull request opens once the MCP server's has merged.

## 2. Statistics

`analysis::measure(image: &Image) -> ImageStats` works on the image's luminance and colour after a block average to at most 1024 pixels on the longer edge (`math::block_average` with `k = ceil(long / 1024)`, at least 1). The values, all `f64` unless noted:

| Field | Meaning | Source |
|---|---|---|
| `width_px`, `height_px` (`u32`) | the image's own size | |
| `p_low`, `p_high` | the 0.5th and 99.5th luminance percentiles, from a 4096-bin histogram | |
| `mean` | mean luminance | |
| `otsu` | the luminance threshold that maximises between-class variance on a 256-bin histogram | Otsu 1979, doi:10.1109/TSMC.1979.4310076 |
| `two_level` | Otsu's separability η = σ²_between / σ²_total, 0 to 1; 1 for a pure two-tone image, 0 for a flat one | Otsu 1979 |
| `dark`, `mid`, `light` | shares of pixels with luminance below 0.25, from 0.25 to 0.75, and above 0.75 | |
| `colourfulness` | √(σ²_rg + σ²_yb) + 0.3·√(μ²_rg + μ²_yb) over rg = R − G and yb = ½(R + G) − B, on 0..255 values | Hasler & Süsstrunk 2003, doi:10.1117/12.477378 |
| `noise` | the noise standard deviation on 0..1 luminance at the analysis scale, which is close to the default tone map's, from the mean absolute response of a 3×3 Laplacian-difference mask | Immerkær 1996, doi:10.1006/cviu.1996.0060 |
| `border` | mean luminance of the outer band, 5 % of the shorter edge wide | |

Every value is a deterministic function of the pixels: integer histograms with fixed bin edges, sums in row-major order, `sqrt` (exact in IEEE 754) and no other floating-point library call. A flat image gives `two_level` 0 rather than dividing by zero. The module docstring cites the three papers.

## 3. Suggestions

`analysis::suggest(stats: &ImageStats, canvas: &Canvas, style: &str, settings: &Value) -> Vec<Suggestion>` turns the statistics into suggestions for the session as it stands. A suggestion is:

```json
{"kind": "line_art", "reason": "This looks like line art. The Ink style keeps its shapes and line weights.",
 "patch": {"style": "ink"}}
```

`patch` holds any of `style` (a style name), `settings` (`{stage: {key: value}}`, applied after the style) and `page` (`{width_mm, height_mm}`). A suggestion already in effect is left out, so applying one makes it disappear. The rules, in this order:

| `kind` | When | Patch |
|---|---|---|
| `line_art` | `two_level` ≥ 0.85 and `mid` ≤ 0.15, and the style is not `ink` | `{"style": "ink"}` |
| `levels` | `p_high − p_low` < 0.6 and at least 1/64, and `tone.levels` is `manual` | `{"settings": {"tone": {"levels": "auto"}}}` |
| `invert` | `border` < 0.3, `mean` < 0.5 and `two_level` ≥ 0.85 (light marks on a dark ground, not a dark photograph), and `tone.invert` is false | `{"settings": {"tone": {"invert": true}}}` |
| `noise` | `noise` > 0.03, and `tone.blur_mm` is 0 | `{"settings": {"tone": {"blur_mm": b}}}`, `b` = one tone-map pixel, the drawing area's longer side over `tone.resolution`, in mm rounded up to 0.1 mm |
| `orientation` | `canvas.fit` is `fit`, the image and the drawing area are on opposite sides of square, and swapping the page would draw the image at least 1.3 times larger | `{"page": {"width_mm": h, "height_mm": w}}` |
| `colour` | `colourfulness` > 40 | none: the reason says each pen draws one colour and colour separation is not built yet |

The reasons are fixed English sentences in the module, one per kind, written for a person and read by the assistant as well. The cut-offs are hand-set starting values, checked on the generated test images of §8 and recorded in the brief; they are a rule, not a fitted model.

## 4. Auto levels

The tone stage gains two settings:

- `levels`: `"manual"` (default) or `"auto"`. Help: "Manual uses Black and White. Auto sets them from the image so that Clip percent of its pixels turn full black and as many full white."
- `clip_percent`: 0 to 10, default 0.5, step 0.1. Used only when `levels` is `auto`.

In `auto`, after the crop and the resize and before the levels, the stage builds a 4096-bin histogram of the resized luminance `y`. `black` is the lower edge of the first bin at which the cumulative count from the dark end exceeds `clip_percent / 100` of the pixels, and `white` the upper edge of the first bin at which the count from the light end does. When `white − black` is under 1/64 the image is close to flat, and the levels stay at 0 and 1 so noise is not stretched into ink. The rest of the pipeline is unchanged. Measuring the resized map, not the whole image, means `fill` mode stretches the part it draws.

This is the single-channel case of the published "Simplest Color Balance" method (Limare, Lisani, Morel, Petro, Sbert 2011, doi:10.5201/ipol.2011.llmps-scb). Its reference code is GPL-3.0, so the stage is written from the paper's description only.

The default stays `manual`, so every existing output, golden file and preset is unchanged. A preset without `levels` takes the default, so no preset version bump is needed.

## 5. The session and bindings

- `Session::analysis(&self) -> Option<Analysis>`, where `Analysis { stats: ImageStats, suggestions: Vec<Suggestion> }`, both `Serialize`. `None` without an image. The statistics are measured when an image is set, a few milliseconds at most, and kept with it; the suggestions are worked out on every call from the current canvas, style and settings.
- Python binding `analysis() -> dict | None`; `Session.analysis()` in `session.py` returns the dict, or raises `RuntimeError("no image is loaded")` without an image, as `run` does.
- WebAssembly binding `analysis() -> string | undefined`, the JSON.

## 6. Front ends

- **CLI:** `plotline analyse IMAGE` loads the image into a default session and prints the analysis as indented JSON. It takes the same `--width`, `--height`, `--margin` and `--fit` options as `run`, since the orientation suggestion depends on the page. Errors follow `run`'s rules.
- **MCP:** a tool `analyse_image()` returns the analysis for the loaded image, or a tool error without one. The server instructions gain one sentence after `load_image`: "Call analyse_image to see the image's statistics and suggested changes; apply a suggestion with run(style=..., settings=...) or set_page." The tool count becomes eleven, and the stdio test lists it.
- **GUI:** the worker's `State` gains `suggestions: Suggestion[]`, empty without an image, so every reply carries the current list. A Suggestions panel sits at the top of the right column while the list is not empty. Each item shows its reason and, when it has a patch, an Apply button that sends the patch's `setStyle`, `update` and `setCanvas` calls in that order and redraws once. Nothing is applied without a click. The tour gains no step.

## 7. Errors

Analysis never fails on an image the session accepted. The CLI and MCP report a missing image as their other commands do. A patch the engine refuses when applied, which only a stale GUI could send, shows its message in the Suggestions panel.

## 8. Tests

All on images generated in code (`synthetic.rs` gains helpers as needed):

- `analysis.rs`:
  - thin dark strokes on white: `two_level` ≥ 0.85, a `line_art` suggestion while the style is `grid`, none while it is `ink`;
  - a full left-to-right gradient: no `line_art`, no `levels`;
  - a gradient from 0.4 to 0.6: a `levels` suggestion, gone once `tone.levels` is `auto`;
  - light strokes on black: an `invert` suggestion;
  - a 256 × 256 flat image plus seeded noise of σ 0.1: `noise` within 20 % of 0.1 and a `noise` suggestion; the flat image alone: `noise` 0 and `two_level` 0;
  - a 400 × 200 image on the default A4 portrait page: an `orientation` suggestion whose patch swaps 210 and 297;
  - the colour bars: a `colour` suggestion with no patch;
  - each suggestion carries its exact reason sentence.
- `tone.rs`: auto levels map a 0.3 to 0.7 gradient onto the full 0 to 1 range within one histogram bin; a flat image stays flat; `manual` output is byte-identical to before.
- `session.rs`: `analysis()` is `None` without an image, and its statistics change when a new image is set.
- WebAssembly smoke test and a native test pin the same `two_level` and `p_low` of the still life to the last bit.
- Python: `Session.analysis()` returns the two keys, and raises without an image. CLI: `plotline analyse` prints JSON with a `suggestions` list. MCP: `analyse_image` returns the analysis and fails without an image; the stdio test lists eleven tools.
- GUI (vitest): a state with suggestions renders the panel; Apply sends the patch's calls in order.

## 9. Decisions recorded in the brief

- **Image analysis.** The engine measures each image once (percentiles, Otsu's two-level score, tone shares, colourfulness, noise, border tone) and suggests patches by hand-set rules; nothing is applied without the user or the assistant. Suggestions remove the first fiddles every user makes, and patches keep the user in charge.
- **Auto levels.** A `levels` setting with `manual` as the default and `auto` from percentiles of the tone map itself. The default keeps every existing drawing and preset unchanged, and `auto` in a preset adapts to each image.
- **Suggestion cut-offs.** The starting values of §3, with the reason that they separate the generated test images and are easy to retune.
