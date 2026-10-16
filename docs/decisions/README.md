# Decisions

From 2026-10-16, each decision is its own file here instead of a new row at the end of the table in [the brief's decision log](../BRIEF.md#14-decision-log). Pull requests that land side by side then never conflict over the same lines. This is the decision-record practice described by M. Nygard, "Documenting architecture decisions" (2011).

Name a file `YYYY-MM-DD-short-title.md` and give it three parts, like a row of the old table:

```markdown
# Short title

**Decided:** YYYY-MM-DD

**Choice:** what was decided.

**Reason:** why, with the measurements or sources behind it.
```

To amend an earlier decision, edit its file and say what changed and when. For one in the brief's table, add a file here that names the row it amends, and add "amended on YYYY-MM-DD, see docs/decisions/..." to that row.
