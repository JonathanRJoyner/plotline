# Model hosting

**Decided:** 2026-10-16

**Choice:** Designed on 2026-10-16 with [learned line models](2026-10-16-learned-line-models.md) and built on 2026-10-17: plotline's converted ONNX copies live in a separate public repository, `plotline-models`, served from raw.githubusercontent.com at commit-pinned URLs and checked by SHA-256. Withdrawing a model drops its entry from the next release and rewrites that repository's history.

**Reason:** Browsers can fetch from that host, which sends CORS headers; release assets and the authors' own download links do not. The main repository keeps its rule of no third-party files, and a pinned URL never changes under its hash.
