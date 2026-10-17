Section: Pens

- **Palette from My pens:** with pens saved under My pens, a colour image's suggested pens are picked from them, and a new note names up to three of the ten inks you don't have that would draw the image's colours clearly closer. `plotline analyse --library` reads a `plotline-pens.json` file, and MCP gains `set_pen_library`. The analysis's `palette` now lists `{name, color}` objects instead of ink names, and gains `palette_additions`.
