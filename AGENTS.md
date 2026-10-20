# Agent notes

Read `docs/SPEC.md` first. It holds the design decisions, crate layout, and milestone status. Update it when you finish a milestone or change a decision.

- The root `Cargo.toml` only defines the workspace. Code lives in `crates/`: `base` (deterministic core, no Bevy), `assets` (asset pack and data loaders), `client` (the game, binary `rts`), `android` (Android entry point). Game content is in `assets/` at the root.
- `rts_base` must stay deterministic: no float arithmetic, no `HashMap`/`HashSet`, no clocks or OS randomness. See "Deterministic simulation" in the spec.
- Pure Rust only. Every dependency must build for native, `wasm32-unknown-unknown`, and Android.
- Put dependency versions in `[workspace.dependencies]` and use `dep.workspace = true` in crates.
- Checks: `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check --target wasm32-unknown-unknown`.
