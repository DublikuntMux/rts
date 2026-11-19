# Agent notes

The handbook in `docs/` (reStructuredText, built with Sphinx) is the project specification. Read `docs/architecture/index.rst` first for the crate layout, `docs/architecture/determinism.rst` before touching `rts_base`, and `docs/project/roadmap.rst` for milestone status. When you finish a milestone or change a decision, update the affected architecture or reference page, `docs/project/roadmap.rst`, and `docs/project/verification.rst`. `docs/documentation.rst` explains how to build and edit the handbook.

- The root `Cargo.toml` only defines the workspace. Code lives in `crates/`: `base` (deterministic core, no Bevy), `net` (protocol, turns, replays), `server` (authoritative server, binary `rts-server`), `assets` (asset pack and data loaders), `client` (the game, binary `rts`), `android` (Android entry point). Game content is in `assets/` at the root.
- `rts_base` must stay deterministic: no float arithmetic, no `HashMap`/`HashSet`, no clocks or OS randomness. See `docs/architecture/determinism.rst`.
- Pure Rust only. Every dependency must build for native, `wasm32-unknown-unknown`, and Android.
- Put dependency versions in `[workspace.dependencies]` and use `dep.workspace = true` in crates.
- Checks: `cargo test --workspace`, `cargo clippy --workspace --all-targets`, `cargo check --target wasm32-unknown-unknown`.
- Docs check: `docs/.venv/bin/python -m sphinx -b html -n -W --keep-going docs docs/_build/html` (setup in `docs/documentation.rst`).

## Commit messages

Release notes are generated from commit subjects by `.github/scripts/changelog.sh`. It reads only the subject line, skips merge commits, and sorts each subject into a section by its first word. Every subject is published verbatim in the release notes, so write it for someone reading the changelog.

- Write one imperative sentence: capitalized, no trailing period, no `feat:`/`fix:` or scope prefixes, about 72 characters at most. Example: `Add unit sight radius to rules`.
- Start with a verb from the section you mean. The script checks sections in this order and the first match wins:

  | Section | Leading verbs |
  | --- | --- |
  | Fixed | `Fix`, `Correct`, `Prevent`, `Avoid`, `Handle`, `Repair`, `Guard` |
  | Removed | `Remove`, `Delete`, `Drop` |
  | Added | `Add`, `Implement`, `Introduce`, `Create`, `Enable`, `Support`, `Render`, `Generate`, `Define`, `Configure`, `Choose`, `Draw`, `Play`, `Show` |
  | Changed | `Refactor`, `Move`, `Rename`, `Simplify`, `Replace`, `Use`, `Make`, `Split`, `Merge`, `Extract`, `Update`, `Store`, `Cache`, `Reduce`, `Improve` |
  | Other | anything else, e.g. `Document`, `Record`, `Verify`, `Test` |

- Matching is a plain prefix match on the first characters, so the verb must come first: `Fix fog redraw after terrain edits`, not `Fog redraw fix`. Pick the verb by what the change does for the reader: a new capability is `Add`, a bug fix is `Fix` even if it adds code, a behavior change to an existing feature is `Make`/`Improve`/`Replace`, dependency bumps are `Update`.
- Keep one logical change per commit, so each subject lands in one section. Split a commit when it both adds a feature and fixes an unrelated bug.
- Put details, rationale, and follow-ups in the body after a blank line; the generator ignores the body.
- If you change the verb lists in `changelog.sh`, update the table above.
