# Contributing

Thanks for helping. Read the [design brief](docs/BRIEF.md) first: it holds the goals, the architecture, every decision so far with its reason, and the open questions. A change that settles an open question records the decision and the reason in the brief's decision log.

## Setup

You need a Rust toolchain from [rustup](https://rustup.rs), which reads the version from `rust-toolchain.toml`, and [uv](https://docs.astral.sh/uv/). The GUI also needs Node 22 and wasm-pack, and the Rust licence check needs cargo-deny: `cargo install wasm-pack cargo-deny` installs both.

## Verification

CI runs these on every pull request, and a pull request merges only when all of them pass. Run them before you push.

Python, from the repository root (`uv sync` builds the Rust extension):

```sh
uv sync --locked
uv run ruff check .
uv run ruff format --check .
uv run mypy src
uv run pytest -q
uv run python scripts/check_licenses.py
```

Rust:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -p plotline-core
cargo deny check licenses
```

WebAssembly, in `crates/wasm`. The built `web/pkg/plotline_wasm_bg.wasm` must stay at or under 2,000,000 bytes:

```sh
wasm-pack test --node
wasm-pack build --release --target web --out-dir ../../web/pkg
```

GUI, in `web` (`npm run check` runs the type check and vitest):

```sh
npm run wasm
npm ci
npm run check
npm run build
```

The browser tests in `web/e2e` run in Chromium against the bundle `npm run build` wrote, one flow per GUI feature. `npx playwright install chromium` fetches the browser once:

```sh
npx playwright install chromium
npm run e2e
```

## Regression gallery

When a change can move a drawing (a stage, a style's defaults, the tone map, the pens or the preview), compare the gallery before and after:

```sh
git switch main
cargo run --release -p plotline-core --example gallery -- target/gallery-main
git switch your-branch
cargo run --release -p plotline-core --example gallery -- target/gallery --baseline target/gallery-main
```

Open `target/gallery/index.html`. The engine is deterministic, so every outlined cell is a real change. Say in the pull request which cells changed and why. If a README picture changed too, redraw them with `cargo run --release -p plotline-core --example readme_images`.

`tests/snapshots/gallery-stats.json` keeps the gallery's numbers: path count, pen-down and pen-up length and plot time for each cell. CI draws the gallery again and fails when the numbers moved but the file did not, and it uploads the gallery page, compared with the base branch, as an artifact. When the change is meant, regenerate the file and commit it, so the review shows the moved numbers:

```sh
cargo run --release -p plotline-core --example gallery -- target/gallery --snapshot tests/snapshots/gallery-stats.json
```

## Rules

The repository is public, so every commit is public.

- **Licences.** Every runtime dependency must be permissively licensed (MIT, BSD, Apache-2.0 or similar). Check before adding one. The licence checks above enforce it.
- **Original implementations.** Write algorithms from published descriptions and cite the source in the docstring. Don't copy code from another project unless its licence allows it, and credit it if you do.
- **Naming.** Cite papers and published algorithm descriptions, but don't borrow names, UI, presets or marketing claims from other products, and don't compare plotline with them. Naming a product to state its maker's published specs, as the pen bank does with line widths, is fine. Our own settings never go under a product's name.
- **Images.** No third-party images. Test images are generated in code or made for this project.
- **No personal details.** No personal paths, credentials or private notes.
- **Layers.** Algorithm code lives in the core crate. The CLI, GUI and MCP only call the session layer.
- **Tour.** A user-visible GUI feature adds or updates its step in the tour (`STEPS` in `web/src/tour.tsx`).
- **Browser flows.** A user-visible GUI feature also adds its flow in `web/e2e/gui.spec.ts`, which runs it in a real browser.
