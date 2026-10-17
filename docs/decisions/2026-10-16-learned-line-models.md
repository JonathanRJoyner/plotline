# Learned line models

**Decided:** 2026-10-16

**Choice:** Designed on 2026-10-16 under [the learned line models spec](../superpowers/specs/2026-10-16-learned-line-models-design.md), and built on 2026-10-17: a `learned` style (model, split, ink, order). The `model` stage places the image on the page as the tone stage does, runs a downloaded model in 512-pixel tiles, and writes line darkness into a `ToneMap` that the existing stages draw, `ink` in `centreline` mode by default. Its settings are `model`, `resolution` (1024 by default) and `invert`.

**Reason:** The owner asked for the lines a person would draw from a photo. A model's output has the shape of a tone map, so only one stage is new. Tiles keep TEED near 100 MB, where a whole 2048-pixel contour image took 3.3 GB of WebAssembly's 4 GB.
