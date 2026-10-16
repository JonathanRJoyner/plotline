Section: Output

- **Embedded presets:** an exported SVG or JSON drawing carries the preset of the run that made it, in a `<metadata>` element in the SVG and a `preset` key in the JSON. Load preset in the GUI, `--preset` on the command line, `Session.load_preset` and the MCP `load_preset` tool read such a file as a preset, so a drawing brings its setup back.
