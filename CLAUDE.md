# plotline

Read [docs/BRIEF.md](docs/BRIEF.md) before doing anything. It holds the goals, architecture, decisions and open questions. When you settle an open question, record the decision and the reason in the brief's decision log.

Rules (the repo will become public, so treat every commit as public):
- Every runtime dependency must be permissively licensed (MIT/BSD/Apache-2.0). Check before adding one.
- Write algorithms from published descriptions and cite them in docstrings. Don't copy code from other projects unless their license allows it.
- Cite papers and published algorithm descriptions. Don't borrow names, UI, presets or marketing claims from other products, and don't compare against or disparage them. This avoids trademark use and implied affiliation. Naming a product to state its maker's published specs (the pen bank's widths) is fine; our own settings never go under a product's name.
- No third-party images in the repo. Test images are generated in code or made for this project.
- No personal paths, credentials or private notes.
- Keep algorithm code in the core. The CLI, GUI and MCP only call the session layer.
- A user-visible GUI feature adds or updates its step in the tour (`STEPS` in `web/src/tour.tsx`).
