# Style picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the style select, the Try all styles button and `StylesDialog` with one style picker: a compact sidebar button that opens a tabbed dialog of every style, grouped by family, each with its trial picture, label and description.

**Architecture:** A new `web/src/picker.tsx` holds the family table, a pure `groupStyles`, and the `StylePicker` dialog built on the house `Modal`. The protocol's `thumbs` learns to draw a given list of styles in order, so the app asks for pictures in the picker's order. No engine change.

**Tech Stack:** TypeScript, Preact, Vite, vitest with fast-check, Playwright.

**Spec:** [docs/superpowers/specs/2026-10-16-style-picker-design.md](../specs/2026-10-16-style-picker-design.md)

## Global Constraints

- No new dependency. Icons come from `lucide-preact`, already a dependency.
- No other product's names, UI or wording in code, copy or commits (CLAUDE.md).
- The family headings are `docs/styles.md`'s own, word for word: "Lines", "The Voronoi point family", "Sketch lines", "Other placers", "Tone only"; the catch-all group is "More styles".
- The dialog title is "Styles"; the style button's accessible name is `Style: <label>`; cards and the style button carry the style's name as `value`.
- Cards are at least 140 px wide; the description is cut to three lines on screen only.
- Pictures stay at `THUMB_PX = 600`, `THUMB_SHRINK = 2`.
- Every e2e flow fails on a console error (existing fixture); keep it that way.
- Prerequisites, once per worktree: `npm ci`, `npm run wasm` and `npx playwright install chromium` in `web/`. The dev server for this worktree runs on port 5187.

## Review Focus

1. Opening the picker before any image is loaded: no `thumbs` call, the "Open an image to see each style drawn on it" line shows, cards have no picture, and a pick still switches style (Task 3, flow "the style picker works before an image").
2. Reopening after a pick: the tick, `aria-current` and the initial focus move to the new style (Task 3, keyboard flow, second half).
3. A trial that fails shows its error in its own card and the rest still draw (Task 1, unknown-name test; the existing `ink` "unreachable" test keeps the per-card error).
4. A long label in the sidebar button at phone width stays inside the Style row and the row stays in the first screen (Task 3, phone flow).
5. A style the family table does not know still shows, under More styles (Task 2, law).

---

### Task 1: `thumbs` draws a given list of styles

**Files:**
- Modify: `web/src/protocol.ts` (the `thumbs` method near line 122, `private async thumbs` near line 208)
- Test: `web/src/protocol.test.ts`

**Interfaces:**
- Produces: the worker method `thumbs(width: number, shrink = 1, names?: string[])`. With `names`, the loop draws exactly those, in that order; without, every `listStyles()` name as now.

- [ ] **Step 1: Write the failing test** in `protocol.test.ts`, after "thumbs without an image posts each style's error":

```ts
test("thumbs with names draws those styles in that order, and an unknown name fails alone", async () => {
  const { posted, ok, ended } = setup();
  ok("setImage", ...image());
  await ended(1);
  ok("thumbs", 120, 1, ["truchet", "nope", "grid"]);
  for (let i = 0; i < 5000 && posted.filter((m) => m.type === "thumb").length < 3; i++) await macrotask();
  const events = posted.filter((m) => m.type === "thumb");
  expect(events.map((m) => m.style)).toEqual(["truchet", "nope", "grid"]);
  expect(events.map((m) => m.png === null)).toEqual([false, true, false]);
  expect(events[1].error).toMatch(/nope/);
});
```

- [ ] **Step 2: Run it and see it fail**

Run: `npx vitest run src/protocol.test.ts -t "thumbs with names"` in `web/`
Expected: FAIL, the styles come in `listStyles` order.

- [ ] **Step 3: Implement** — thread `names` from the `thumbs` method into `private async thumbs(id, width, shrink, names?: string[])` and loop over `names ?? listStyles names`. If the session's error for an unknown style does not name it, wrap it so the card's error does.

- [ ] **Step 4: Run the protocol tests**

Run: `npx vitest run src/protocol.test.ts`
Expected: all pass, the two existing thumbs tests unchanged.

- [ ] **Step 5: Commit** — `git commit -m "Let thumbs draw a given list of styles in order"`

---

### Task 2: Families and grouping

**Files:**
- Create: `web/src/picker.tsx` (this task: types, `FAMILIES`, `MORE`, `groupStyles` only)
- Create: `web/src/picker.test.ts`

**Interfaces:**
- Produces:
  - `export type Entry = { name: string; label: string; description: string; note?: string; picture?: string }`
  - `export type Thumb = { url: string | null; error: string | null }` (moved here from `panels.tsx` in Task 3)
  - `export const FAMILIES: ReadonlyArray<readonly [label: string, names: readonly string[]]>` with the spec §2 table, in order
  - `export const MORE = "More styles"`
  - `export function groupStyles<E extends { name: string }>(entries: readonly E[]): Array<[string, E[]]>`

- [ ] **Step 1: Write the failing tests** in `picker.test.ts`:

  - **"grouping lists every entry once, families first in table order, unknown ones last in input order"** — fast-check: `fc.uniqueArray(fc.oneof(fc.constantFrom(...FAMILIES.flatMap(([, n]) => n)), fc.string({ minLength: 1 })))` mapped to `{ name }`, shuffled by `fc.shuffledSubarray` or generated in any order. Assert: the flattened output's names, sorted, equal the input names sorted; every group is non-empty; group labels are distinct; the labels are a subsequence of `[...FAMILIES.map(([l]) => l), MORE]` in that order; within a family group the names follow that family's table order; the `MORE` group's names follow the input order and none of them is in `FAMILIES`.
  - **"the families match the style guide's contents list"** — read `../../docs/styles.md` with `readFileSync(new URL(..., import.meta.url))`; from each contents line `- [Label](#anchor): [\`a\`](#a), ...` take the label and the backticked names, skipping the line with no names ("Settings every style shares"); expect the result to equal `FAMILIES.map(([l, n]) => [l, [...n]])`.
  - **"every style the engine lists has a family"** — `init` the wasm module as `protocol.test.ts` does in `beforeAll`, then expect every `name` in `JSON.parse(listStyles())` to be in `FAMILIES.flatMap(([, n]) => n)`.

- [ ] **Step 2: Run them and see them fail**

Run: `npx vitest run src/picker.test.ts`
Expected: FAIL, `./picker` does not exist.

- [ ] **Step 3: Implement** `FAMILIES`, `MORE` and `groupStyles` in `picker.tsx`. One header comment cites the spec section, as the other GUI files do.

- [ ] **Step 4: Run them and see them pass**

Run: `npx vitest run src/picker.test.ts`
Expected: 3 passed.

- [ ] **Step 5: Commit** — `git commit -m "Group the styles by family for the picker"`

---

### Task 3: The picker, the style button, the tour and the browser flows

**Files:**
- Modify: `web/src/picker.tsx` (add `StylePicker`; move `ZOOM`, `FRAME`, `Thumb` and the enlarged-picture code here from `panels.tsx`)
- Modify: `web/src/panels.tsx` (delete `StylesDialog`, `StylesDialogProps`, `Thumb`, `ZOOM`, `FRAME`; `export` `Modal`; drop imports left unused)
- Modify: `web/src/app.tsx` (the Style fieldset near line 593, `tryStyles` near line 432, the `StylesDialog` element near line 658, the `Style` type near line 18)
- Modify: `web/src/style.css` (the `.thumbs` rules near line 378)
- Modify: `web/src/tour.tsx` (`STEPS` lines 14–15)
- Modify: `web/e2e/gui.spec.ts`

**Interfaces:**
- Consumes: `Entry`, `Thumb`, `FAMILIES`, `groupStyles` (Task 2); `thumbs(width, shrink, names)` (Task 1); `Modal` from `panels.tsx`; `place` from `tour.tsx`; `onPaper` from `view.ts`.
- Produces: `export function StylePicker(props: { opens: number; styles: Entry[]; style: string; canvas: Canvas; paper: string; hasImage: boolean; thumbs: Record<string, Thumb>; onPick: (name: string) => void; onClose: () => void }): JSX.Element`

- [ ] **Step 1: Rewrite the e2e helpers and flows** (they are this task's failing tests). In `gui.spec.ts`:
  - Replace `styleSelect` with `styleButton = (page) => page.getByRole("button", { name: /^Style: / })`, and add `picker = (page) => page.locator("dialog").filter({ has: page.getByRole("heading", { name: "Styles", exact: true }) })`, `card = (page, style) => picker(page).locator(\`.picker button[value="${style}"]\`)` and `currentStyle = async (page) => (await styleButton(page).getAttribute("value"))!`.
  - `pick` opens the picker with `styleButton(page).click()`, clicks `card(page, style)`, expects the picker hidden, then `ran`. `styleNames` opens the picker, reads every `.picker button[value]`'s `value`, presses Escape and expects it hidden.
  - Every `styleSelect(page).inputValue()` becomes `currentStyle(page)`; every `expect(styleSelect(page)).toHaveValue(x)` becomes `expect(styleButton(page)).toHaveAttribute("value", x)`; the focus checks at lines ~714 and ~925 and the phone flow's bounding box use `styleButton(page)`.
  - Replace "the thumbnail grid picks a style" with **"the style picker groups the styles and picks one"**: after `sample`, open the picker; `picker.locator(".picker h3")` has texts `["Lines", "The Voronoi point family", "Sketch lines", "Other placers", "Tone only"]`; `card(page, current)` has `aria-current="true"`; the first other card contains its description text (read the description from the card's `title`); its `img` becomes visible (timeout 100 000); hovering it shows `.zoom img` over twice as wide (keep the existing assertions); clicking it hides the picker, the style button's `value` is that style, and the run ends done.
  - Add **"the style picker works from the keyboard"**: after `sample`, focus the style button and press Enter; expect `card(page, current)` focused; press Tab, read the focused card's `value`, press Enter; expect the picker hidden and that style current; `ran`. Open again with Enter: the new style's card is focused and has `aria-current="true"`; Escape closes.
  - Add **"the style picker works before an image"**: `open(page)` with no sample; open the picker; it contains "Open an image to see each style drawn on it" and no `.picker img`; click `card(page, "hatch")`; the style button's `value` is `hatch`.
  - Rename "the thumbnail grid survives reopening and a run in progress" to "the style picker survives…", open with `styleButton(page)`, count `.picker img` and `.picker .paper > span`.
  - The phone flow, after its existing checks: open the picker; the first two cards' bounding boxes share a `y` and the third is lower; `picker(page).evaluate((d) => d.scrollWidth <= d.clientWidth)` is true.

- [ ] **Step 2: Build and run the changed flows to see them fail**

Run: `npm run build && npx playwright test -g "style picker"` in `web/`
Expected: FAIL, no button named `Style: …`.

- [ ] **Step 3: Implement `StylePicker`** in `picker.tsx`, following spec §4:
  - `const TABS = ["Styles"] as const`, `const [tab, setTab] = useState<(typeof TABS)[number]>("Styles")`; when `TABS.length > 1` render `<nav>` with one button per tab, `aria-current` on the open one, as `SettingsDialog` renders its sections; then the open tab's panel.
  - The Styles panel is `<div class="picker" onScroll={hide}>`: when `!hasImage`, `<p class="help">Open an image to see each style drawn on it.</p>`; then for each `[label, entries]` of `groupStyles(styles)`, a `<section>` with `<h3>{label}</h3>` and `<ul class="thumbs">`.
  - Each card is `<button type="button" value={name} title={description} aria-current={current ? "true" : undefined} autofocus={current}>` with the paper (only when `hasImage` or `picture`; `picture ?? thumbs[name]?.url`, else the error or "Drawing…"), `<span class="name">{current && <Check />}{label}{note && <span class="note">{note}</span>}</span>` and `<span class="description">{description}</span>`. Click: `onPick(name)` and close the dialog, as `StylesDialog` does. Keep its pointer and focus handlers for the enlarged picture, and its `onClose` that hides it.
  - Wrap the whole in `<Modal opens={opens} title="Styles" icon={<LayoutGrid />} wide onClose={...}>`.

- [ ] **Step 4: Wire the app** in `app.tsx`:
  - `type Style = Entry` (imported from `./picker`); import `StylePicker`, `groupStyles`, `type Thumb` from `./picker` and drop `StylesDialog` and `Thumb` from the `./panels` import.
  - Rename `tryStyles` to `openPicker`. It clears the thumbs and bumps `stylesOpens` as now, and only when `state.image` calls `call<number>("thumbs", [THUMB_PX, THUMB_SHRINK, groupStyles(styles).flatMap(([, es]) => es.filter((e) => !e.picture).map((e) => e.name))])`.
  - Replace the Style `<label>` with the select and the Try all styles button with one row: `<label>Style{" "}<button type="button" class="pick" data-tour="styles" aria-haspopup="dialog" aria-label={\`Style: ${label}\`} value={state.style} onClick={openPicker}><LayoutGrid /> <span>{label}</span></button></label>`, where `label` is the current style's label (fall back to its name).
  - Render `<StylePicker ... hasImage={state.image} />` where `<StylesDialog>` was, with the same `onPick` and `onClose`. Update the `THUMB_PX` comment and the `thumbsFor` comment to say "the style picker".

- [ ] **Step 5: Style it** in `style.css`, replacing the styles-dialog rules: `label > button.pick { margin-left: auto; min-width: 0; max-width: 65%; }` with its `span` ellipsised on one line; `dialog.modal .picker` spans `grid-column: 1 / -1` (`auto` after a `nav`), scrolls (`overflow-y: auto`) with `padding: 0 16px 16px`; `.thumbs` is the grid, `repeat(auto-fill, minmax(140px, 1fr))`, gap 10 px, no list style, no scrolling of its own; a card stacks its parts in a column, left-aligned; `.description` is muted 12 px at normal weight, clamped to three lines with `-webkit-line-clamp: 3`; `.note` is a small pill on `--accent-soft` in `--accent`; the tick is 14 px in `--accent`. Keep the `.paper`, `img` and `.zoom` rules.

- [ ] **Step 6: Rewrite the two tour steps** in `tour.tsx`: the `"style"` step's text says the Style button shows the chosen style and opens the picker, and the line under it says what the style does, then keeps its looks sentences; the `"styles"` step is retitled "Choose a style by eye" and says the picker shows every style by family, drawn small on the image, page and pens with a line on what it does; point or tab to enlarge, click or press Enter to switch, closing leaves the drawing as it was.

- [ ] **Step 7: Run the checks**

Run in `web/`: `npm run check`, then `npm run build && npx playwright test`
Expected: tsc clean, every vitest passes, every Playwright flow passes (the tour flow included).

- [ ] **Step 8: Look at it** in the browser pane on the dev server (port 5187): desktop and 375 px, light and dark, picker open with the sample and without an image; screenshot each for the PR.

- [ ] **Step 9: Commit** — `git commit -m "Choose styles in a picker grouped by family"`

---

### Task 4: Docs

**Files:**
- Modify: `docs/BRIEF.md` (§8 bullets; decision log rows "Try all styles", "Browser tests", "Enlarged style pictures"; new row "Style picker" after "Try all styles")
- Modify: `README.md` (line ~95)
- Modify: `docs/styles.md` (a sentence under the contents list)
- Create: `changelog.d/style-picker.md`

- [ ] **Step 1: Brief.** §8 gains "**Style picker:** a Style button opens a dialog of every style grouped by family, each drawn small on the current image with its label and description (§14, "Style picker")." The new "Style picker" row records, each with its reason from the spec: one picker replacing the select and Try all styles; families kept in the GUI and tested against the guide and the engine; the guide's headings word for word with More styles as the catch-all; the tab column hidden while one tab exists, and how a tab is added; pictures on every open with an image, in the picker's order; focus on the current card and Tab order, no arrow keys; the description cut to three lines on screen only; entries may carry a note and a picture for the model piece. The "Try all styles" row starts "Amended on 2026-10-16: the dialog became the style picker, see "Style picker"." The "Browser tests" row's "the styles dialog picking a style" becomes "the style picker picking a style, by mouse, by keyboard and before an image". The "Enlarged style pictures" row's "Try all styles dialog" becomes "style picker".
- [ ] **Step 2: README** — the "Try it" step says the Style button opens the style picker, which draws every style small on your image so you can choose by eye.
- [ ] **Step 3: styles.md** — under the contents list: "The GUI's style picker groups the styles under these same headings, and a test keeps the two equal, so a new style gets a section here and a family there."
- [ ] **Step 4: changelog.d/style-picker.md** — no `Section:` line (Front ends): one bullet, "**Style picker:** the Style button opens a picker of every style grouped by family, each drawn small on your image with its label and what it does; it replaces the style menu and Try all styles, and works from the keyboard and at phone width."
- [ ] **Step 5: Commit** — `git commit -m "Document the style picker"`

---

### Task 5: Ship

- [ ] **Step 1:** Run the repo's verification set from CONTRIBUTING.md for a web-only change (`npm run check`, `npm run build`, `npx playwright test` in `web/`), and confirm `git status` shows nothing stray.
- [ ] **Step 2:** Push the branch and open a PR against `main` with the screenshots from Task 3, the spec and plan links, and the follow-ups: paste while a modal is open still loads an image (existing), and the model tabs (later piece).
- [ ] **Step 3:** Bind the PR with the ccd_pr tools, read its checks, and fix any red one until all are green.
