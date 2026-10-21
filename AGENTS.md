# Agent notes

The handbook in `docs/` (reStructuredText, built with Sphinx) is the project specification. Read `docs/architecture/index.rst` first for the crate layout, `docs/architecture/determinism.rst` before touching `rts_base`, and `docs/project/roadmap.rst` for milestone status. When you finish a milestone or change a decision, update the affected architecture or reference page, `docs/project/roadmap.rst`, and `docs/project/verification.rst`. `docs/documentation.rst` explains how to build and edit the handbook.

- The root `Cargo.toml` only defines the workspace. Code lives in `crates/`: `base` (deterministic core, no Bevy), `net` (protocol, turns, replays), `server` (authoritative server, binary `rts-server`), `assets` (asset pack and data loaders), `client` (the game, binary `rts`), `android` (Android entry point). Game content is in `assets/` at the root.
- `rts_base` must stay deterministic: no float arithmetic, no `HashMap`/`HashSet`, no clocks or OS randomness. See `docs/architecture/determinism.rst`.
- Pure Rust only. Every dependency must build for native, `wasm32-unknown-unknown`, and Android.
- Put dependency versions in `[workspace.dependencies]` and use `dep.workspace = true` in crates.
- Checks: `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check --target wasm32-unknown-unknown`.
- Docs check: `docs/.venv/bin/python -m sphinx -b html -n -W --keep-going docs docs/_build/html` (setup in `docs/documentation.rst`).
