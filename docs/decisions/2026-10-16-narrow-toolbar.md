# Narrow toolbar

**Decided:** 2026-10-16

**Choice:** the toolbar sheds labels as the preview column narrows, measured with a container query on that column. Below 1240 px, Source, Travel, Fit page, Export SVG, Settings and Tour show their icons alone. Below 880 px every button but Run does. From 340 px to 620 px the toolbar is a grid of two even rows: open, run and undo above, view, fit and the app buttons below. A shed label stays in the button as visually hidden text, so the button keeps its accessible name, and a title shows the name on hover.

**Reason:** the owner disliked how the toolbar stacked on smaller screens. With every label it needs 1194 px, so a 1730 px window wrapped it and left Export SVG, Settings, Tour and the theme button alone on a second row, pushed to the right. The two tiers need 829 px and 596 px, so the toolbar stays on one row from a 620 px preview column up, which covers a 1280 px laptop. A wrapped flex row cannot be balanced in CSS, so the narrowest widths use a fixed two-row grid instead. Below 340 px that grid would overflow, so the toolbar wraps there as before. The thresholds leave about 5 % for fonts wider than the Windows system font the widths were measured in.
