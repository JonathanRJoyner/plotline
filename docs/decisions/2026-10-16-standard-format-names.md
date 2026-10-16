# Standard format names

**Decided:** 2026-10-16

**Choice:** This settles brief §12, question 1, the scope of the naming rule. The rule against naming other products covers product and program names in prose, identifiers, settings, presets, commit messages and the UI. Standard or de facto file formats, named for what they are, may appear anywhere: in code, file names, options, the UI and the docs. That covers SVG, JSON, PNG, G-code and HPGL, even where the format began at one company. What stays out:
- vendor-specific extensions and namespaces, such as a plotter program's own SVG layer attributes;
- instructions that only one maker's machines understand;
- any setting, preset or adapter named after a product rather than a format.

An adapter that targets a named toolchain still needs its own decision.

**Reason:** A format name tells the user what a file contains, which they need in order to pick the right export, and it does not suggest affiliation with or comparison to any product. HPGL is the format many pen plotters read, and G-code was already written under this reading. Keeping vendor extensions out keeps every file plain, so any program that reads the standard can read it.
