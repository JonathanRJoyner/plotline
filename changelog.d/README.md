# Changelog entries

Each change adds its own file here instead of editing `CHANGELOG.md`, so pull requests that land side by side never conflict over the same lines.

- Name the file after the change, such as `pen-palette.md`.
- Write one or more bullets in the style of `CHANGELOG.md`, for example `- **Pen palette:** ...`. A line `Section: Styles` at the top (Styles, Pens, Images, Output or Front ends) says where it goes; without one it goes under Front ends.

At a release, the entries move into `CHANGELOG.md` under the new version's heading, grouped by section, and their files are deleted in the same commit that bumps the version.
