# GUI third pass

**Decided:** 2026-10-16

**Choice:** These close the items the second pass left (see the [second pass](2026-10-16-gui-second-pass.md)):

- **Stats:** they read "1 path" and group thousands. Lengths show in millimetres below a metre and in metres from there.
- **Fit and Optimize messages:** a Fit to target message shows only while its setting still holds the value it chose. An Optimize suggestion shows only while the settings are as they were when it came.
- **Suggestions:** each has a Dismiss button, and the dismissals last until the next image.
- **Test chart:** moves from Export to the Pens panel.
- **Focus and reading order:** the page's document order is now the preview column first, then the style column, then the page column. The columns stay where they were on screen, so Tab and a screen reader reach the toolbar first. The skip link now jumps to the style settings.
- **Phone layout:** on a phone the preview is shorter, so the style menu shows in the first screen.
- **Image settings order:** a stage's schema may declare its display order as `x-order`. The tone stage uses it to list Levels, Black, White and Clip percent together, then Gamma, Contrast and Invert, then Flatten paper, Response and Resolution. Without it a form keeps the schema's order.
- **Pens dialog:** it builds each edit on the last list it sent, so edits made faster than the replies are no longer lost.
- **Compare divider:** it repaints when the theme changes.
- **Browser tests:** they take their port from `PLOTLINE_E2E_PORT`, or a free one, so two worktrees can run them at once.

**Reason:**
- **Image settings order:** schemars writes a schema's properties in sorted order, so Levels, the switch that decides whether Black and White apply, sat between Invert and Resolution, away from them. Keeping the declared order in serde_json would change the key order of every export and preset, so the order is declared rather than inferred.
- **Document order:** visual order and document order now agree on a phone and on a desktop. A keyboard user meets the toolbar's actions before every setting, and the skip link covers the way back.
- **Test chart:** it calibrates the first pen, so it belongs with the pens rather than with the files a drawing exports.
- **Fit and Optimize messages:** a message about a value the drawing no longer uses misleads.
- **Suggestions:** a suggestion the user has decided against should not stay in view.
