# plotline

Turn images into drawings for pen plotters.

plotline converts raster images into vector line art, such as stippling, single continuous lines, dashes and sketched strokes. It comes with a live preview that shows the algorithm at work, so you can see what a setting does before you commit ink to paper.

## What works now

plotline is early software. The engine is written in Rust and compiled twice: to a native extension for the Python package, and to WebAssembly for the browser. The Python package and the command line are thin layers over it. There are five styles, and every run orders its paths to keep the pen's travel short. `grid` puts a dot on a regular grid wherever the image is dark enough. `hatch` draws straight parallel lines over the dark areas, in three levels that cross over darker tones. `waves` draws parallel lines that ripple more where the image is darker, one pen stroke per line. `spiral` draws one spiral from the centre out that ripples the same way. `ink` is for line art, lettering and inked drawings: it outlines every ink shape and fills it with rings, and draws strokes thinner than the pen once down the middle, so the drawing keeps its own shapes and weights. The line styles lift the pen far less often than `grid`. A run writes an SVG in millimetres that holds the drawing in one layer, labelled with the name of its pen. It can also write the paths as JSON and a PNG preview that draws each mark at its real pen width. You can set the page size and margin, choose the pen (`--pen 'Fine:0.3:blue'` takes an ink name or a `#rrggbb` colour), and change a setting with `--set stage.key=value`, such as `--set points.spacing_mm=3`. `plotline describe grid` lists the settings.

`plotline gui` opens the live preview in the browser. It runs the engine inside the page, as WebAssembly. Drop or paste a PNG, JPEG or WebP image to open it, pick pens from a bank of common fineliners and colour them from a list of common inks, and the page remembers your setup between visits. Export SVG sits in the toolbar, ready for a pen plotter's own software. For G-code, the Settings dialog holds the machine: the drawable area, pen heights, feeds and start and end G-code of a pen-fitted 3D printer, which you enter for your own machine, and the export panel then writes one G-code file per pen for it. The preset carries the machine, so save a preset to keep it. Building it needs Node, wasm-pack and the `wasm32-unknown-unknown` Rust target: run `npm run wasm`, `npm ci` and `npm run build` in `web/`. The Voronoi styles, Sketch Lines and the MCP server are planned but not built yet, and `plotline mcp` prints a message saying so. The goals, architecture, decisions and everything still planned are in [docs/BRIEF.md](docs/BRIEF.md).

## Try it

There is no release yet, so work from a clone of this repository. Building the Python extension needs a Rust toolchain: install one with [rustup](https://rustup.rs). With [uv](https://docs.astral.sh/uv/) installed as well, run this in the clone, where `uv sync` builds the extension and installs the package:

```
uv sync
uv run plotline --help
```

Then draw an image of your own:

```
uv run plotline run grid your-photo.jpg --out drawing.svg --preview preview.png
```

This writes `drawing.svg` and `preview.png`, and prints a one-line summary with the path count and an estimated plot time. A preset is a small JSON file that holds the style, page, pens and settings of a run. `--save-preset my-look.json` writes one and `--preset my-look.json` starts a later run from it, so you can share a look.

## License

[MIT](LICENSE)
