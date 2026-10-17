# plotline spec: style picker

Date: 2026-10-16. Status: proposed. The owner delegates the taste decisions; each is recorded in the brief's decision log under "Style picker", and the "Try all styles" row is amended to point there.

Twenty-one styles in a dropdown is already a long list, and the only way to see them drawn was a second button, Try all styles, which opened its own dialog. This spec puts both into one **style picker**: a dialog that shows every style grouped by family, each with a picture of it drawn on the current image, its label and its one-line description. The sidebar keeps a compact button that shows the current style and opens the picker.

The picker is the second of four pieces planned on 2026-10-16. A later piece adds two tabs to it, "Get models" and "Load your own", and learned line models appear as entries in its Styles tab once downloaded. This spec builds only the Styles tab, lays the dialog out so that adding a tab is a small change, and lets an entry carry an optional note and picture. It builds no model code.

## 1. Scope

- `web/src/picker.tsx` (new): the families, the grouping, and the picker dialog, which takes over from `StylesDialog` in `web/src/panels.tsx`.
- `web/src/picker.test.ts` (new): the grouping law, and a check that the families match `docs/styles.md` and the engine's styles.
- `web/src/panels.tsx`: `StylesDialog` removed; `Modal` exported for the picker.
- `web/src/app.tsx`: the style select and the Try all styles button replaced by the style button; the pictures asked for in the picker's order.
- `web/src/protocol.ts`: `thumbs` takes the styles to draw, in order.
- `web/src/style.css`: the picker's layout.
- `web/src/tour.tsx`: the two style steps rewritten.
- `web/e2e/gui.spec.ts`: the style helpers and the two picker flows rewritten, plus a keyboard flow.
- Docs: the brief (§8 and the decision log), the README, `docs/styles.md`'s contents note, `changelog.d/style-picker.md`.

No engine, CLI or MCP change. No drawing moves, so the gallery stats snapshot stays as it is.

## 2. Families

The picker groups the styles as `docs/styles.md` and the CHANGELOG do, in the guide's order:

| Family | Styles |
|---|---|
| Lines | `grid`, `hatch`, `waves`, `spiral`, `truchet`, `ink` |
| The Voronoi point family | `stipple`, `line`, `dashes`, `tree`, `mesh`, `cells`, `scribble` |
| Sketch lines | `sketch` |
| Other placers | `halftone`, `dither`, `circles` |
| Tone only | `contours`, `flow`, `hilbert`, `mosaic` |

The table lives in `picker.tsx` as `FAMILIES`, a list of `[label, names]`. The headings are the guide's own, word for word, so the guide and the picker never disagree.

`groupStyles(entries)` returns the families in that order, each with its entries in the table's order, and drops a family with no entry. An entry whose name is in no family goes into a last group, **More styles**, in the order it came, so a style the table does not know yet still shows. Nothing is lost or listed twice.

The family is a front-end grouping, so it lives in the GUI rather than as a field on the engine's `Style`. The engine lists styles in the order they were built, not by family, and the CLI and MCP have no use for the grouping yet. A test keeps the GUI's table, the guide and the engine's list in step (§7).

## 3. The sidebar

The Style fieldset keeps its place and its `data-tour="style"`. Its select and its Try all styles button become one row:

- **The style button:** `Style` on the left, as the other rows have their label, and on the right a button with the grid icon and the current style's label, such as "Waves". Its accessible name is "Style: Waves", it has `aria-haspopup="dialog"`, and it carries the style's name as its `value`. It is never disabled: without an image it is still the way to choose a style. It takes `data-tour="styles"`.
- The help line under it, the Look select and Reset to defaults stay as they are.

A long label is cut short with an ellipsis, and the button keeps the select's width limit, so the phone layout keeps the Style row in the first screen (the existing phone flow checks it).

## 4. The picker

The picker is a wide modal on the house `Modal` (the native `<dialog>`: focus trap, Escape to close), titled **Styles** with the grid icon.

### 4.1 Tabs

The picker holds a list of tabs, `TABS = ["Styles"]`, and a panel for each. When the list has more than one tab, the left column shows them as the Settings dialog shows its sections: buttons, the open one marked with `aria-current`. With one tab the column is not drawn and the panel takes the dialog's whole width. Adding a tab is one name in `TABS` and one panel; the phone layout, which turns the column into a row across the top, comes from the existing dialog CSS.

The tab column is hidden while there is one tab because a single "Styles" entry beside a dialog already titled Styles says nothing, and it costs a fifth of the width the pictures need.

### 4.2 The Styles tab

A scrolling panel with one section per family: the family's heading (the dialogs' small uppercase heading) and a grid of cards. With no image open, a line at the top says "Open an image to see each style drawn on it", and the cards show no picture.

Each card is one button, carrying the style's name as its `value`:

- **The picture:** the style drawn on the current image, page, pens and paper colour, from the existing trial runs (§5), on paper of the page's shape; "Drawing…" until it arrives, or the error if the trial failed. An entry with its own `picture` URL shows that instead and is not drawn.
- **The label**, with a tick before it on the current style.
- **The note**, when the entry has one: a small pill after the label. No built-in style has one; a downloaded model will.
- **The description**, in the muted help text, cut to three lines on screen. The whole sentence stays in the button's text, so a screen reader reads all of it, and in its tooltip.

The current style's card also has `aria-current="true"` and the accent border, so it is marked by more than colour. The grid fits as many cards of at least 140 px as the width allows: five on a wide window, two on a phone.

Pointing at a card, or tabbing to it, shows its picture enlarged beside it, as the Try all styles dialog does now. A click, or Enter or Space on the focused card, switches to that style and closes the picker. Closing it in any way stops the trial runs, and the user's own drawing stays as it was.

### 4.3 Keyboard

The dialog opens with focus on the current style's card, so the user starts where they are. Tab and Shift+Tab move through the cards in reading order, family by family, and Escape closes. Arrow keys are left to scrolling: a grid that wraps at a width the window decides gives arrow keys no reliable meaning.

## 5. Pictures

The pictures are the trial runs `StylesDialog` already draws through `Session::trial`, 600 px wide with marks at least 2 px wide, unchanged. Two things change:

- They start every time the picker opens with an image loaded, instead of only from Try all styles. The trials share the stage cache, so the current style and its family's shared stages draw at once, and a closed picker stops them.
- The protocol's `thumbs(width, shrink, names)` takes the styles to draw. The app passes the picker's order, so the pictures fill in from the top of the dialog down; before, they came in the order the engine lists its styles, which put `truchet`, in the first family, last. A name the engine does not know comes back as that card's error. Without `names` it draws every listed style, as now.

Opening the picker on a large image starts up to twenty-one trial runs where Try all styles did the same on request. Each yields to the worker every 50 ms, so the user's own run keeps drawing, and a pick or a close cancels the rest within one step.

## 6. Entries for later

An entry is `{ name, label, description, note?, picture? }`. The engine's styles fill the first three. The later model piece adds entries with a note, such as the model's licence, and a picture of its own, shown without a trial run. Nothing in this spec sets either field; the picker draws them when present.

## 7. Tests

- **Grouping law** (fast-check, `picker.test.ts`): for any list of entries with distinct names, some from the families and some not, in any order, `groupStyles` lists every entry exactly once; the groups come in `FAMILIES` order with More styles last; no group is empty; within a family the entries follow the table, and within More styles they follow the input.
- **The families match the guide:** the test reads `docs/styles.md`'s contents list and expects its family headings and style names, in order, to equal `FAMILIES`.
- **Every style has a family:** the test loads the WebAssembly engine, as `protocol.test.ts` does, and expects every name `listStyles` returns to be in `FAMILIES`. A new style then fails until it is given a family here and a section in the guide.
- **Protocol:** `thumbs` with names draws those styles in that order, and an unknown name gives that one an error.
- **Browser flows** (`gui.spec.ts`):
  - The helpers that chose a style through the select open the picker and click the card with that `value`; the current style is the style button's `value`.
  - "The style picker groups the styles and picks one" replaces the thumbnail grid flow: the headings are the five families in order; the current style's card is marked; a card shows its description; its picture arrives, pointing at it shows it enlarged, and a click picks it and closes the picker.
  - "The style picker works from the keyboard": Enter on the style button opens it with focus on the current card; Tab then Enter picks the next card's style.
  - "The style picker survives reopening and a run in progress" keeps the existing flow through the style button.
  - The phone flow also opens the picker at 375 px and expects two cards a row and no sideways scroll.
- **Tour:** the "style" step's text says the button opens the picker; the "styles" step, renamed "Choose a style by eye", rings the style button and says what the picker shows. The tour flow checks both from `STEPS`.

## 8. Docs

- The brief: a bullet in §8 for the picker; a decision row "Style picker" with each choice above and its reason; the "Try all styles" row amended to say the dialog became the picker; the "Browser tests" and "Enlarged style pictures" rows' mentions updated.
- The README's "Try it" step names the style picker in place of Try all styles.
- `docs/styles.md` says, under its contents list, that the GUI's picker uses these headings and a test keeps them equal.
- `changelog.d/style-picker.md` under Front ends.

## 9. Not in this spec

- The Get models and Load your own tabs, and any model code (the later piece).
- A search or filter over the styles. With five families on one scrolling page there is no list long enough to need one yet.
- Keeping the pictures between openings of the picker. The stage cache already makes a second opening fast, and a kept picture could show an old image or setting.
- The family in the engine, the CLI's `plotline styles` or the MCP's `list_styles`.
