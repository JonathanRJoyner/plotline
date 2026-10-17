# Changelog

New entries go under Unreleased until the next release.

## Unreleased

Pending entries are files in [changelog.d/](changelog.d/README.md); they move here at the next release.

## 0.2.0 (2026-10-16)

Looks inside each style, tone matching with Optimize and a budget fit, focus points and auto framing, scan flattening and edge-keeping smoothing, HPGL export, presets embedded in exports, a test chart, paper colours, your own pen library, a compare view, and a calmer GUI with plot lint whose fixes hold.

### Styles

- **Looks:** most styles offer a few measured starting points under plain names: Faithful, the lowest tone error on the test images; Quick, about half an hour on A4 where the defaults take longer; and Fine, the densest marks that still plot cleanly with a 0.3 mm pen. Balanced is the defaults. A look sets only the few settings that make it: the Look select in the GUI, `plotline run --look NAME`, the MCP `run` tool's `look` and `Session.apply_look()`.

### Pens

- **My pens:** the GUI's pens dialog keeps a library of your own pens in the browser. Save this pen adds the chosen pen, a click gives a pen a saved pen's name, width and colour, and Export and Import move the library as a `plotline-pens.json` file.

### Images

- **Auto framing:** the page can crop the image (`crop`: the shares cut from its left, top, right and bottom, each up to 0.45), and when the subject fills only part of the image the analysis suggests a crop to it, `framing`. The subject comes from a spectral-residual saliency map, and the crop is the smallest rectangle of the page's shape that holds most of it. Nothing is cropped until you apply it: Apply in the Suggestions panel, `plotline run --crop LEFT,TOP,RIGHT,BOTTOM`, `set_page(crop=...)` in the MCP server or `Canvas(crop=...)`. The Page panel shows Cropped with a Remove crop button, and Undo takes a crop back. A page with no crop is saved and exported as before.
- **Smooth, keep edges:** a tone setting, under the Image settings' Advanced, that smooths noise and paper grain but keeps edges sharp, unlike Blur, so styles that follow edges keep them while specks go. It is the guided filter of He, Sun and Tang, off by default.
- **Focus points:** mark where the picture matters and every style keeps its detail there and abstracts it further away, after DeCarlo and Santella (2002). In the GUI, turn on Focus in the toolbar and click the drawn image to add a point, or click a point to remove it; each point shows as a dashed ring of the focus radius. An assistant or a script sets the same list as `tone.focus`, `[x, y]` shares of the image from its top-left corner, for example `plotline run --set "tone.focus=[[0.5,0.3]]"` or the MCP `run` tool's `settings`. `tone.focus_mm` is the radius that keeps all its detail and `tone.focus_blur_mm` the blur furthest from every point. With no points the drawing is unchanged.
- **Flatten paper:** a tone setting for scans and phone photos of drawings that evens out grey, yellowed or unevenly lit paper, so the paper draws as white and a threshold such as the ink style's catches only the strokes. It is off by default, and the image suggestions offer it when an image looks like a drawing on uneven paper.
- **Pen palette:** for a colour image, the analysis picks two to four of the ten inks that draw its colours best under the colour model Pen use solves. Its colour suggestion, until now a note, sets those pens and Pen use to by colour with one Apply, in the GUI, `plotline analyse` and MCP `analyse_image`.

### Output

- **Pen lift by G-code lines:** a machine whose pen is lifted by a servo or a solenoid sets `pen_lift` to `gcode` and gives its own pen-down and pen-up lines, which the G-code writer puts where it would move Z. In the GUI, Settings > Pen lift > Lift by.
- **Split moves:** a machine's `max_segment_mm` cuts every move into pieces no longer than it, for firmware that would draw a long straight move as a curve, as on a hanging plotter. In the GUI, Settings > G-code > Longest move.
- Both are off by default: existing machines, presets and G-code are unchanged, and the preset version stays 3.
- **Embedded presets:** an exported SVG or JSON drawing carries the preset of the run that made it, in a `<metadata>` element in the SVG and a `preset` key in the JSON. Load preset in the GUI, `--preset` on the command line, `Session.load_preset` and the MCP `load_preset` tool read such a file as a preset, so a drawing brings its setup back.
- **HPGL export:** a drawing can be written as one HPGL file for pen plotters that read that format, with a pen selected for each layer and 40 plotter units to the millimetre unless set otherwise. The Export panel has an HPGL button and the Settings dialog an HPGL section for the units; the command line takes `--hpgl FILE` and `--hpgl-units N`, Python has `Session.hpgl` and `Session.export_hpgl`, and the MCP `export` tool takes the format `hpgl` and `units_per_mm`.
- **Paper colour:** set the paper's colour in the GUI's Page panel, with `--paper` on `plotline run` and `plotline analyse`, or with `paper` on MCP `set_page`. The preview, the style thumbnails and the PNG previews draw the ink multiplied onto that paper, so a drawing on cream or black paper looks as it will plot. Presets keep the colour; SVG and G-code leave it out.
- **Path simplification:** Path order has a new setting, `simplify_mm`, that removes the vertices lying within that distance of the simplified line (Ramer–Douglas–Peucker), keeping each path's end points. It is off at 0, so existing drawings and presets are unchanged; at an eighth of a 0.3 mm pen, 0.0375 mm, waves, spiral and flow lose 70 to 90 % of their vertices with the same drawing to within antialiasing. The plot lint's fine-vertices finding now offers it as a fix.
- **Test chart:** a page of small labelled patches for the first pen, to plot and compare by eye: lines at 0.5 to 4 pen widths apart, dots 1 to 6 pen widths wide, and a solid fill drawn 1 to 8 times over. It shows where lines merge, how big a dot comes out, how many passes the paper takes, and whether the plot takes the time the stats say. `plotline testchart`, the MCP `testchart` tool, `Session.testchart()` and the GUI's Test chart button make it, with no image needed.

### Front ends

- **Enlarged style pictures:** in Try all styles, pointing at a picture or tabbing to it shows it 2.5 times larger beside its cell. The pictures are drawn 600 px wide so the enlargement stays sharp.
- **Engine failures recover:** a panic in the browser engine shows its message, and the worker rebuilds its session, so one failure cannot stall Try all styles or the page. The `ink` style no longer panics in the browser on a large drawing with many rings.
- **Compare:** a chip on the GUI's preview swipes the current drawing against the previous run or the source image, on the same page at the same zoom, with a divider you drag or move with the keyboard.
- **Dots at small sizes:** Grid dots and Dither no longer come out blank in Try all styles, whose pictures now draw every mark at least 2 px wide. In the main preview, single dots are filled discs and still show when the pen is under a pixel wide at the current zoom; they used to vanish there in desktop browsers.
- **A calmer first screen:** an empty preview offers the sample image and opening one. Fit to target and Optimize fold into "Fit and optimize". Compare shows once there is a drawing, form fields have borders that read at 3:1, Enter places a focus point from the keyboard, and the tour is 14 steps in the order of the work.
- **Tidier details:** the stats read in metres and say "1 path". The Image settings keep Levels, Black and White together, and suggestions can be dismissed. Test chart lives with the pens, Tab reaches the toolbar first, and a phone shows the style menu in the first screen. A Fit message clears once its value is undone.
- **Plot lint fixes that work:** every Apply button in the plot lint now removes or reduces its finding. A finding shows no button where no setting is sure to help. `empty` measures the ink actually laid, so thin pens no longer read as an empty page. `travel` offers path ordering only when it cuts the travel, and `vertices` offers simplifying only when it lowers the share of needless vertices. Pens that draw nothing under light to dark or by colour now show as `idle_pens`, so `unused_pens` only means the one-pen setting. `dark` no longer offers more points or closer lines where the minimum path length would drop them.
- **Narrow windows:** the toolbar keeps to one row by showing buttons as icons alone as the window narrows, the view and app buttons first, with each name on hover. On a phone it takes two even rows instead of leaving one group on a row of its own.
- **Optimize limits:** Optimize now tries the density setting at half-octave steps from a quarter to four times its value, up to nine runs, instead of narrowing in on one dip of the tone error, and leaves out values that would plot for over twice as long as the current one (or an hour, if that is more) or that the plot lint warns about. A minutes target replaces the time limit. The suggestion says how long it takes to plot, against the current value's time, in the GUI, `plotline run --optimize`, the MCP `optimize` tool and `Session.optimize()`, whose result gains `minutes` and `current_minutes`, and whose trials gain `minutes`, `slow`, `warnings` and `failed`. A value whose run fails, such as a halftone too dense for the vertex budget, is left out instead of ending the search.

## 0.1.0 (2026-10-16)

The first release. One Rust engine runs everything: the Python package, the command line and the MCP server use it as a native extension, and the browser GUI runs it as WebAssembly.

### Styles

Twenty-one styles, each a pipeline of cached stages, so changing a late setting reruns only the stages after it:

- **Lines:** `grid`, `hatch`, `waves`, `spiral`, `truchet` and `ink`, for line art and inked drawings.
- **The Voronoi point family:** `stipple`, `line` (one unbroken line that does not cross itself), `dashes`, `tree`, `mesh`, `cells` and `scribble` (loops traced along the line). They share one placement of points, so switching between them reuses the points. Placing 10,000 points over 50 relaxation passes on a 1000-pixel image takes about 3.3 s natively.
- **Sketch Lines:** `sketch`.
- **Other placers:** `halftone`, `dither` and `circles`.
- **Tone only:** `contours`, `flow`, `hilbert` and `mosaic`. `hilbert` fills the page's own rectangle with a generalized Hilbert curve, so with one pen it is one unbroken line.

Every run ends with path ordering, which cuts pen-up travel, joins paths whose ends nearly touch and drops paths below a minimum length.

A style switch keeps the settings you changed in the stages both styles share, Pen use included, and gives the rest the new style's defaults, so a tuned placement of points carries from one Voronoi style to the next, and a value you set comes back when you switch away and back. **Reset to defaults** in the GUI, `reset` on the MCP `run` tool and `Session.reset_settings()` put every setting of the style back.

**Plot lint** checks every finished drawing for trouble on paper: marks drawn over each other, specks, lines closer than the pen, a drawing too light for the image's darks, heavy pen lifting or travel, needless vertices, unused pens, and vertices off the page, the margin or the machine. Each finding comes with the setting change that fixes it where one does. The GUI lists the findings under the stats with Apply buttons, `plotline run` prints them on stderr and fails on a warning with `--strict`, and the MCP `run` result carries them.

**Tone match** scores every drawing on how closely its tones follow the image through a 1.5 mm eye filter, as a tone error and an SSIM, shown with the stats in the GUI and the CLI and returned by the MCP `run` tool. **Optimize** searches the style's density setting for the value with the lowest tone error, within a budget if one is given, in eight runs, and suggests it without applying it: an Optimize button beside Fit to target, `plotline run --optimize` and the MCP `optimize` tool. The tone stage's **Response** setting can be `auto`, which runs the style on a small step wedge first and corrects the tone map for the coverage it measures; it is off by default, and helps most the joined-point styles `line`, `tree` and `mesh`.

### Pens

- **Pen use** shares a drawing between several pens: the first pen alone (the default), light to dark in list order, or each pen by its colour. The stages after it run once per pen that draws. By colour works in short steps, so Stop takes effect within a tenth of a second even on a 4000-pixel image with eight pens.
- **Pens kept apart:** every style but `ink` spreads its marks across the pens, so that one pen does not draw on top of another. Lines turn, grids, levels and seeds shift, and random scatters are drawn independently for each pen. The first pen draws as it would alone.
- **A pen bank** of 25 models and 121 pens in five kinds (fineliner, technical pen, gel pen, ballpoint and marker), each at the line width its maker publishes, and ten ink colours under plain names.

### Images

- PNG, JPEG and WebP input, colour or greyscale, with 16-bit greyscale for scans.
- **Image analysis** measures a new image and suggests changes: line art, levels, invert, noise, page orientation and colour. Each suggestion is a patch that is applied only on request.
- **Auto levels:** the tone stage's `levels` setting can be `auto`, which sets black and white from the image's own percentiles.

### Output

- SVG in millimetres with one group per pen layer, a plain JSON paths format, and PNG previews drawn at the real pen width.
- **A G-code adapter** writes one file per pen for a machine the user describes: its drawable area, pen heights, feeds, and start and end G-code. Nothing is built in for any particular machine.
- **Presets, version 3:** a JSON file with the style, page, pens, settings and machine. Version 1 and 2 files migrate on load.

### Front ends

- **Budgets:** give a plot time in minutes, a path count, metres of pen-down line or a dot count, and plotline finds the value of the style's density setting that meets it, in up to eight runs. The GUI has a target field with a Fit to target button, `plotline run` takes `--fit-minutes`, `--fit-paths`, `--fit-metres` or `--fit-dots`, and the MCP server has a `fit` tool. The plot time leaves out acceleration, so styles of short strokes or dots plot slower than it says.
- **Browser GUI** with a live preview that shows each stage working, true to pen width and colour, with the pen-up travel overlay, stats and plot-time estimate, a settings panel generated from the stage declarations, the pens and settings dialogs, suggestions, presets, export and a guided tour. **Try all styles** draws every style small on the current image, page and pens, and a click switches to one; **undo and redo** (Ctrl+Z, Ctrl+Shift+Z and toolbar buttons) step through changes to the style, settings, page and pens. `plotline gui` serves it locally, and the same files are the hosted page at <https://jonathanrjoyner.github.io/plotline/>. The engine runs in the visitor's browser, so no image is uploaded.
- **First-run fixes:** the GUI works at phone width, a new visitor starts on `waves`, settings show plain labels, a new machine is marked as placeholder values, a dropped file that is not an image plotline opens says so, and Fit to target names the setting it changed and says when a target is out of reach.
- **Clearer names:** exports are named after the style (`plotline-waves.svg`), a preset that will not load names its file, and the toolbar's view fit is called Fit page, apart from Fit to target and the page's fit setting.
- **G-code export buttons:** with one pen, Export shows a single G-code button; with several, one row per pen, told apart by its number and colour swatch, since pens can share a name, with long names cut short and shown in full on hover.
- **Command line:** `plotline run`, `styles`, `describe`, `analyse`, `gui` and `mcp`. `plotline run --machine FILE --gcode DIR` draws for a machine and writes one G-code file per pen.
- **MCP server** over stdio, in the `mcp` extra, with thirteen tools: `list_styles`, `describe_style`, `load_image`, `set_page`, `set_pens`, `set_machine`, `run`, `fit`, `preview`, `get_session`, `analyse_image`, `load_preset` and `export`, which also writes G-code.
- **A smaller WebAssembly module:** one PNG library instead of two, image decoding and JSON built for size, and wasm-opt at `-O3`. The module is 41 KB smaller and the Voronoi relaxation about 12 percent faster, with every drawing unchanged.
- **Python library:** `Session`, with `set_machine` and `export_gcode`, the style registry and the value types, with type information.
