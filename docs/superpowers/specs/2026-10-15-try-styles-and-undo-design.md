# plotline spec: try all styles, and undo

Date: 2026-10-15. Status: approved in conversation; written for review.

The registry lists eighteen styles, and picking one by name means knowing what the name draws. This spec adds two GUI features that make exploring cheaper: a grid that draws every style on the current image at once, so a user chooses by eye, and undo and redo for the setup, so a user can try a change without fear of losing what they had.

## 1. Scope

- `crates/core/src/session.rs`: `Session::trial`, a run that writes a result slot of its own.
- `crates/wasm/src/lib.rs`: `Session.trial(name)`, returning a `Trial` with `nextFrame`, `error` and `previewPng`; the worker drops a trial to stop it.
- `web/src/protocol.ts`: `thumbs` and `cancelThumbs` commands and a `thumb` event; a `restore` command.
- `web/src/history.ts` (new): the undo history.
- `web/src/app.tsx`, `web/src/panels.tsx`, `web/src/style.css`: the styles dialog, the toolbar buttons and the keys.
- `web/src/tour.tsx`: one step for each feature.
- The brief's decision log.

Out of scope: thumbnails in the Python library, CLI or MCP (an assistant can already run each style), undo of the image, the machine's speeds or the view, and a history that survives a reload.

## 2. Try all styles

A "Try all styles" button under the style menu opens a modal dialog with one cell per listed style, in registry order. Each cell shows the style's label and, once drawn, a picture of that style on the current image, page and pens. Clicking a cell switches to that style and closes the dialog. Closing the dialog, by its button or Esc, cancels the drawing of the cells still to come. The button is disabled until an image is open.

### 2.1 A trial run, not a second session

Each cell is drawn by `Session::trial(name)`, which returns a run of the style `name` at the settings `set_style(name)` would give, over the session's image, page and pens, and the slot that run writes. The run shares the session's stage cache but has its own result slot and cancel flag, so it neither cancels the user's run nor replaces the user's drawing, stats or settings.

A second `Session` in the worker was the other choice and is rejected. It would need its own copy of the image and its own cache, so it could not reuse the tone map or Voronoi points the user's runs have cached, and the style the user then picks would be drawn a second time from nothing (2.3 s for the relaxation at the defaults). Sharing the cache makes the grid cheap, since the six Voronoi styles share their points and most others take 50 to 60 ms over a cached tone map (brief, "Timings and size"), and it makes the click cheap, since the picked style's stages are already in the cache. The cost is that eighteen styles' outputs can push older entries out of the 512 MB cache.

`trial` computes the settings by calling `set_style` and putting the style and settings back, so the cells follow whatever a style switch does, including a later change to carry settings across a switch (brief §12, question 8). A trial without an image fails as `start` does.

### 2.2 In the worker

`thumbs(width)` starts drawing every listed style in turn and replies with an id. The worker drives each trial run to its end without posting frames, yielding to its event loop every 50 ms of work as a run does, so a command such as `cancelThumbs` or the user's own run gets its turn. After each run it posts one event, `{type: "thumb", id, style, png, error}`: `png` is the drawing as PNG bytes `width` pixels wide, through the same preview renderer as `previewPng`, and `error` the run's message when it failed. `cancelThumbs` stops the drawing, and so does a new `thumbs`. The page keeps only the events of the id it was given, so a cell drawn just before a cancel cannot land in a later grid.

The cells are 300 pixels wide, about twice their size on screen. A full-size preview of every style would cost more to rasterise than most of the runs themselves, and the cells only need to show the kind of mark.

## 3. Undo and redo

The page keeps a history of whole setups in the preset's shape: `{style, canvas, pens, machine, settings}`. Every reply carries the state, and when its setup differs from the last one the page saw, the last one goes on the undo stack and the redo stack empties. The undo stack keeps the latest 100 setups. Undo restores the setup on top of the undo stack and puts the current one on the redo stack; redo does the reverse. The image, the plot speeds, the view and the hidden pens are not part of a setup.

A restore is one `restore(setup)` command, which applies the setup over the current preset with `loadPreset`, so the format and version stay the worker's, and redraws like any other change. The reply's setup becomes the current one without a new step, so a restore never empties the redo stack.

Ctrl+Z undoes and Ctrl+Shift+Z redoes, with Cmd in place of Ctrl on a Mac. The keys do nothing while focus is in a text field, a number field or a text area, where the browser's own text undo applies, or while a modal dialog is open. They do not touch the preview's keys: the arrows, plus, minus and 0. Undo and Redo buttons sit in the toolbar and are disabled when their stack is empty.

Setting edits are already sent 150 ms after the last change, so dragging a slider or typing a number makes one step. Picking a machine sets the machine and then the page, which makes two steps.

## 4. Tour

Two steps join `STEPS` in `web/src/tour.tsx`: "Try every style" on the button under the style menu, after "Pick a style", and "Undo a change" on the toolbar buttons, after "Run it".

## 5. Tests

- Rust: a trial draws without touching the session's state, drawing or latest run, and reuses the cache; a trial without an image fails.
- Web: the history's push, undo, redo, limit and restore-without-step rules; the protocol's `thumbs` posts one event per style and `cancelThumbs` stops it; `restore` applies a setup.
- In the browser: the grid fills, a click switches the style, closing cancels, and the keys and buttons undo and redo a style, a setting, a page and a pen change.
