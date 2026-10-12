# plotline

Turn images into drawings for pen plotters.

plotline converts raster images into vector line art, such as stippling, single continuous lines, dashes and sketched strokes. It comes with a live preview that shows the algorithm at work, so you can see what a setting does before you commit ink to paper.

## What works now

plotline is early software. The engine is written in Rust and compiled twice: to a native extension for the Python package, and to WebAssembly for the browser. The Python package and the command line are thin layers over it. The command line runs one style, `grid`: dots on a regular grid, one wherever the image is dark enough, ordered to keep the pen's travel short. A run writes an SVG in millimetres that holds the drawing in one layer, labelled with the name of its pen. It can also write the paths as JSON and a PNG preview that draws each mark at its real pen width. You can set the page size and margin, choose the pen, and change a setting with `--set stage.key=value`, such as `--set points.spacing_mm=3`. `plotline describe grid` lists the settings.

The WebAssembly module builds and passes its tests under Node, but the browser interface that will use it is still to come, so `plotline gui` says so and exits. The Voronoi styles, Sketch Lines and the MCP server are planned but not built yet, and `plotline mcp` also prints a message saying so. The goals, architecture, decisions and everything still planned are in [docs/BRIEF.md](docs/BRIEF.md).

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
