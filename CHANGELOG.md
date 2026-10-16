# Changelog

## 0.1.0

The first release. One Rust engine runs everything: the Python package, the command line and the MCP server use it as a native extension, and the browser GUI runs it as WebAssembly.

### Styles

Eighteen styles, each a pipeline of cached stages, so changing a late setting reruns only the stages after it:

- **Lines:** `grid`, `hatch`, `waves`, `spiral` and `ink`, for line art and inked drawings.
- **The Voronoi point family:** `stipple`, `line` (one unbroken line that does not cross itself), `dashes`, `tree`, `mesh` and `cells`. They share one placement of points, so switching between them at their default settings reuses the points.
- **Sketch Lines:** `sketch`.
- **Other placers:** `halftone`, `dither` and `circles`.
- **Tone only:** `contours`, `flow` and `hilbert`.

Every run ends with path ordering, which cuts pen-up travel, joins paths whose ends nearly touch and drops paths below a minimum length.

### Pens

- **Pen use** shares a drawing between several pens: the first pen alone (the default), light to dark in list order, or each pen by its colour. The stages after it run once per pen that draws.
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
- **Command line:** `plotline run`, `styles`, `describe`, `analyse`, `gui` and `mcp`.
- **MCP server** over stdio, in the `mcp` extra, with twelve tools: `list_styles`, `describe_style`, `load_image`, `set_page`, `set_pens`, `run`, `fit`, `preview`, `get_session`, `analyse_image`, `load_preset` and `export`.
- **Python library:** `Session`, the style registry and the value types, with type information.
