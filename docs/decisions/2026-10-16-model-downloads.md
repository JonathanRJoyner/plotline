# Model downloads

**Decided:** 2026-10-16

**Choice:** Designed on 2026-10-16 with [learned line models](2026-10-16-learned-line-models.md) and built on 2026-10-17: every front end shows the licence, the attribution, the training note, the size and an as-is notice, and waits for acceptance before a download; acceptance is kept per model and per licence text. A model file of your own loads locally: ONNX only, at most 200 MB, with a shape check. MCP lists models and runs downloaded ones but has no download tool. This amends the brief's "MCP tools" row: `list_models` makes seventeen tools.

**Reason:** A person reads and accepts each licence. PyTorch `.pth` files can run code when opened, so only ONNX is accepted.
