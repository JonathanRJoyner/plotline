# plotline

Read [docs/BRIEF.md](docs/BRIEF.md) before doing anything. It holds the goals, architecture, decisions and open questions. When you settle an open question, record the decision and the reason in the brief's decision log.

Rules (the repo will become public, so treat every commit as public):
- Every runtime dependency must be permissively licensed (MIT/BSD/Apache-2.0). Check before adding one.
- Write algorithms from published descriptions and cite them in docstrings. Don't copy code from other projects unless their license allows it.
- Don't name or compare against other specific tools or products in code, docs or commit messages.
- No third-party images in the repo. Test images are generated in code or made for this project.
- No personal paths, credentials or private notes.
- Keep algorithm code in the core. The CLI, GUI and MCP only call the session layer.
