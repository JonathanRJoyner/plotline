# Focus points

Date: 2026-10-16. Status: decided by the builder under the owner's standing delegation; written for review.

The user, or an assistant that has looked at the image, marks one or more focus points. Every style then keeps the image's detail near them and abstracts it further away. There is no detector and no trained model: the person or the assistant chooses where to look.

## 1. Source and the choice of method

D. DeCarlo and A. Santella, "Stylization and abstraction of photographs", *ACM Transactions on Graphics* 21(3), 2002 (Proc. SIGGRAPH), pp. 769-776, doi:10.1145/566654.566650. They record where a viewer looks and keep detail only near those fixations, removing more of it the further a region lies from them. Their abstraction is a region hierarchy pruned by a contrast-sensitivity model of the eye. Here the fixations are the focus points, and the abstraction is a blur whose strength grows with the distance from the nearest point: the same falloff of detail in a form that runs on the tone map every style already reads.

Other routes were considered and left out:
- Weighting the tone by importance, or the point density of the placers by it. Multiplying an importance map into tone changes how dark a region draws, not how much detail it keeps (the research report, §3), and a per-placer weight would be per-style code.
- A saliency detector to place the focus automatically. Auto framing (its own spec) uses saliency for a crop; for focus, the person's or the assistant's choice is the point.

Blur keeps the local mean of the ink, so a region away from the focus draws about as dark as before, with its fine structure gone. A style's density setting then spends the same marks there on smoother tone, so the drawing's detail sits near the focus.

## 2. Settings

Three settings join the `tone` stage. With `focus` empty, which is the default, the stage's output is unchanged bit for bit, so no golden file, gallery cell or snapshot line moves.

| Key | Help | Range | Default |
|---|---|---|---|
| `focus` | Points to keep detail around, as `[x, y]` shares of the image's width and height from its top-left corner; set by clicking the preview. | a list of at most 16 points, each coordinate 0 to 1; `"x-hidden": true` | `[]` |
| `focus_mm` | Radius around each focus point that keeps all its detail; used only with focus points. | 1 to 300, step 1, mm | 40 |
| `focus_blur_mm` | Blur where the image is furthest from every focus point; detail fades into it beyond the focus radius. Used only with focus points. | 0.5 to 10, step 0.1, mm | 3 |

Coordinates are shares of the source image, not of the page, so they keep their place through a change of page, fit or crop, and they ride in presets with the rest of the tone settings. The stage refuses a point that is not two finite numbers within 0 to 1, and more than 16 points, with a message naming the point.

`"x-hidden": true` is a new schema extension: the GUI's generated settings form leaves such a key out (`web/src/schema.ts`), and the GUI sets it through its own control (§4). Other front ends see the key like any other.

## 3. The falloff

After the existing global blur, and only when `focus` holds a point:

1. Each focus point is placed on the tone map in pixels, through the same transform that placed the image: the share of the source image, less the `fill` crop the stage cut (and any canvas crop, should one land first), scaled to the map. A point outside the drawn part of the image still counts; it simply lies off the map.
2. For each map pixel, `d` is the distance in mm from its centre to the nearest focus point, and `w = clamp((d - focus_mm) / (2 focus_mm), 0, 1)`: no blur within the radius, rising to full blur at three radii.
3. The stage blurs the map at `k / 4` of `focus_blur_mm` for `k = 1..4` with `math::blur` (the same Gaussian and box blur the `blur_mm` setting uses), and each pixel takes its ink from the levels on either side of `4 w`, interpolated linearly (level 0 is the unblurred map).
4. The ink is clamped to 0 to 1 as before.

Pixels within `focus_mm` of a point keep exactly the ink they had without focus. The cost is four blurs of the map, a few tens of milliseconds at the default resolution.

With several pens, the falloff applies to the tone map the split shares out, so every lane is abstracted the same way. By colour, the lanes' chroma comes from the image, so only the luminance part of each lane is blurred; the decision file says so.

## 4. Front ends

- **Core.** `Canvas` gains the two halves of the placement the GUI needs, `image_point(page_mm, image_w, image_h) -> Option<[f64; 2]>` (the share of the source image under a page point, `None` off the image) and `page_point(share, image_w, image_h) -> [f64; 2]`, written once and used by the tone stage's step 1 as well, so a later crop extends one place. `Session` exposes both for the current image, and the wasm `Session` passes them through.
- **GUI.** A toolbar toggle, "Focus" (a Lucide crosshair icon, `data-tour="focus"`), puts the preview into focus mode. In focus mode a click on the drawn image adds a focus point there, and a click within 3 mm (on screen, at least 8 px) of a point removes it; dragging still pans. Each point shows as a dashed ring of radius `focus_mm` on the page, in the theme's accent colour, also outside focus mode while points exist. Every change is one `update` of `tone.focus`, so it reruns from the tone stage, joins the undo history and saves with the setup. The Image section shows a line "N focus points" with a Clear button when there are any, beside `focus_mm` and `focus_blur_mm`.
- **Tour.** A step "Mark the focus" after the Image settings step: what a click does, and that detail stays near the points.
- **CLI.** `--set tone.focus=[[0.5,0.3]]` works: `coerce` parses a value that starts with `[` or `{` as JSON.
- **MCP.** `run(settings={"tone": {"focus": [[x, y]]}})` already works. The `run` docstring gains one sentence: an assistant that sees a face or a subject can set focus points there.

## 5. Tests and laws

- `tone.rs`: an empty `focus` gives the same map as before, bit for bit; pixels within `focus_mm` of a point are unchanged; far pixels equal the map blurred at `focus_blur_mm` within rounding; the mean ink over the map moves by under 2 % on the still life; bad points and a 17th point are refused with the messages; determinism.
- `types.rs`: `image_point` and `page_point` invert each other on `fit` and `fill` canvases and give `None` off the image.
- `crates/core/tests/laws.rs`: the case generator gives `focus` 0 to 3 random points (it must not treat the hidden list as a free-text setting); a new law: with focus points, every tone-map pixel within `focus_mm` of one keeps the ink it has without them.
- `crates/core/tests/settings_matter.rs`: `focus_mm` and `focus_blur_mm` matter only with focus points; list them with that reason, as `clip_percent` is listed.
- The golden files, the gallery and its stats snapshot do not change.
- Web: `schema.ts` leaves out `x-hidden` keys (a vitest case); a Playwright flow in `web/e2e/` loads the sample image, turns on Focus, clicks the preview, sees one ring and "1 focus point", clicks the ring, sees none, and undoes back to one.
- Python: a CLI test that `--set tone.focus=[[0.5,0.5]]` reaches the session as a list.

## 6. Documents

- `changelog.d/focus-points.md`, Section: Images.
- `docs/decisions/2026-10-16-focus-points.md`: the choice of falloff over density weighting, the source, the hidden-key extension, and the colour-mode note.
- `docs/styles.md`, "Settings every style shares": one sentence on focus.
- README Features: one bullet sentence, in the Suggestions bullet or its own.
