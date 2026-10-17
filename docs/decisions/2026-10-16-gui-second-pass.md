# GUI second pass

**Decided:** 2026-10-16

**Choice:** After a second walkthrough as a first-time user, at 1366 px and 375 px in both themes, the GUI keeps the first screen to the work's main path: open an image, pick a style or look, see the drawing, export. The power features stay one click away.

- While no image is open, the preview shows a card with "Use the sample image", "Open an image" and a line about dropping or pasting. It replaces the small canvas text.
- Fit to target and Optimize sit in a "Fit and optimize" disclosure under the stats. It is closed at first and opens by itself when one of them has a result or an error.
- The Compare card stays hidden until there is a drawing to compare.
- Inputs, selects, text areas and colour inputs take a `--control-border` token with at least 3:1 against their card in both themes.
- With Focus on, Enter on the focused preview marks a focus point at its centre, under a crosshair.
- The tour shrinks from 26 steps to 14, in the order of the work. Each step groups the features of one place on the page, and it gives focus back when it closes. Every feature still appears in the tour: a new feature adds or updates a step, as the project rules say, and groups with the step for the place it lives in, rather than adding a step of its own.

This amends the "GUI tour" row of the brief's decision log.

**Reason:**
- **Empty preview.** The toolbar sheds its labels below a 1340 px preview column (see the narrow toolbar decision), so at 1366, 1440 and 1536 px windows the first step, opening an image, was two unlabelled icons. Labelled buttons in the empty preview give the first step a name without bringing back the toolbar's crowding, which the owner disliked.
- **Fit and optimize.** The target field, unit menu and two buttons showed before any drawing existed. On a phone they pushed the style menu below the second screen.
- **Compare card.** It showed a disabled chip on an empty page.
- **Control borders.** The borders measured 1.2 to 1.3:1 against their cards, under the 3:1 that WCAG 2.2 success criterion 1.4.11 asks for a control's boundary, and a white Paper swatch read as an empty box.
- **Focus from the keyboard.** Focus points could only be placed with a pointer, the one control a keyboard could not reach.
- **The tour.** At 26 steps it ran out of the order of the work: suggestions came before style, focus before run, and the test chart and presets had steps of their own. A step per feature does not scale with the feature list; a step per place on the page does.
- **What the pass found fine.** Every text passed contrast in both themes by a scripted check over each text node. Every focusable control had an accessible name and a visible focus ring. Reduced motion already stops every transition and animation.
