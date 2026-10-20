# RTS — Project Spec

Living document. Update it when a decision changes or a milestone lands.

## Vision

A top-down 2D real-time strategy game in the classic style (gather, build, fight) with:

- Desktop, web, and Android from one codebase, with first-class touch controls.
- Online multiplayer.
- Lua scripting for missions, triggers, and AI.
- A built-in map editor.

## Hard requirements

| Requirement | Consequence |
|---|---|
| Pure Rust | No C/C++ dependencies. Scripting uses `omnilua` (pure-Rust Lua), not `mlua`/Luau. |
| Desktop + web (`wasm32-unknown-unknown`) + Android | Every dependency must build for all three. Networking must work in browsers (WebSocket or WebRTC). |
| Multiplayer | Deterministic lockstep simulation (see below). This constrains all simulation and script code. |
| Mobile controls | Every action reachable by touch; no hover or hotkey-only features. |

## Tech stack

- Engine/client: Bevy 0.20 (nightly toolchain, edition 2024).
- Simulation: `rts_base` crate, plain Rust, no Bevy dependency.
- Data: RON files via `serde` + `ron`.
- Assets: `bevy_asset_packer` bundles `assets/` into `assets.pack` at build time (`crates/assets/build.rs`).
- Scripting (M6): `omnilua` built with `default-features = false` (only base/string/table/math), run through `Lua::sandboxed` with an instruction limit.
- Web build: Trunk (`index.html`, `Trunk.toml` at the root). Android: Gradle project in `android/`, which builds `crates/android` with `cargo ndk`.

## Architecture

### Crates

The root `Cargo.toml` only defines the workspace (shared package fields, dependency versions, lints, profiles). All Rust code lives in `crates/`. Non-code content stays at the root: `assets/` (game content), `android/` (Gradle app), `index.html` + `Trunk.toml` (web shell), `docs/`.

| Crate | Path | Role | Depends on |
|---|---|---|---|
| `rts_base` | `crates/base` | Deterministic game core: coordinates, IDs, rules and map formats, simulation, state hash. No Bevy. Shared by every other crate. | — |
| `rts_assets` | `crates/assets` | Builds `assets.pack` (`build.rs`), opens it per platform, Bevy loaders for `*.rules.ron` / `*.map.ron`. | base |
| `rts_client` | `crates/client` | The game: rendering, input, camera, match flow. Binary `rts`; `cargo run` starts it. | base, assets |
| `rts_android` | `crates/android` | Android entry point (cdylib). | client |

Planned crates, created when their milestone starts (no empty placeholders):

| Crate | Path | Milestone | Role |
|---|---|---|---|
| `rts_net` | `crates/net` | M3 | Wire protocol (commands, turn bundles, lobby messages) shared by client and server. |
| `rts_server` | `crates/server` | M3 | Relay server binary: lobbies, turn ordering, desync checks. Doesn't run the simulation. |
| `rts_script` | `crates/script` | M6 | omnilua integration: sandbox, deterministic math, game API bindings. Scripts run inside the sim step, so base and script can't simply depend on each other one way. Decide the split at M6 (for example, base calls a trait that script implements). |
| `rts_editor` | `crates/editor` | M7 | Map editor mode, plugged into the client. |
| `rts_ai` | `crates/ai` | M8 | Skirmish AI player that reads sim state and emits commands. |

Dependency direction: base ← assets ← client ← android. Server and AI never depend on the client. Base never depends on Bevy.

The client never changes simulation state directly. It reads `Sim` to draw, and from M2 on it sends **commands** that the sim applies at a tick boundary.

### Deterministic lockstep

All peers run the same simulation from the same start state and the same ordered command stream, so their states stay identical. Only commands go over the network.

Rules for code in `rts_base` (enforced partly by lints, see `crates/base/src/lib.rs` and `crates/base/clippy.toml`):

1. **No floating-point arithmetic.** Positions, speeds, and distances are integers in world units (`TILE = 1024` units per tile). Use `i64` for intermediates such as squared distances and `isqrt` for lengths. Floats appear only when parsing data files; decimal parsing and scaling by a power of two are exact, so the conversion is deterministic.
2. **No `HashMap`/`HashSet`** or anything else with unspecified iteration order. Use `BTreeMap`, `Vec`, or sorted keys.
3. **No wall clock, OS randomness, or thread timing.** Randomness comes from a seeded PRNG stored in sim state (added when first needed).
4. **Iteration order is defined by stable IDs.** Units have monotonically increasing `UnitId`s.
5. **Fixed tick rate:** `TICK_RATE = 20` ticks per second. Rendering interpolates between ticks (M2).
6. **State hash:** `Sim::state_hash()` uses FNV-1a with explicit little-endian, fixed-width encoding (`usize` is hashed as `u64`), so native and wasm32 peers agree. Peers compare hashes to detect desyncs (M3).

### Coordinates

- Sim: integer world units, `TILE = 1024` per tile. x grows right, **y grows down**. Tile `(0, 0)` is the top-left; map file rows are listed top to bottom, so the file looks like the map.
- Client: Bevy world space, y up. `map::to_world` converts: one tile is `TILE_PX = 128` pixels, and sim `y` is negated. The map spans from `(0, 0)` to `(width_px, -height_px)`.

### Networking plan (M3)

- Model: deterministic lockstep with an input delay. A command issued on tick `t` is scheduled for tick `t + delay` (around 100–200 ms). The sim only advances when the commands for the next tick have arrived from every player.
- Transport: browsers allow only WebSocket/WebRTC/WebTransport, so peers connect to a small **relay server** (pure-Rust workspace crate) over WebSocket. The relay orders commands into per-tick bundles, broadcasts them, and compares state hashes. It doesn't simulate. Alternative to evaluate: `matchbox` (WebRTC P2P).
- Replays are the same thing: start setup plus the command log.
- Match setup (rules + map + seed + player list) is checksummed so peers with different data refuse to start.

### AI

AI is a virtual player that issues ordinary commands. It runs on one machine (the host) and its commands travel through lockstep like a human's, so AI code does **not** need to be deterministic.

### Scripting (M6)

- Mission/trigger scripts run **inside** the sim step, on every peer, so they must be deterministic:
  - Replace `math.random` with the sim's PRNG.
  - Remove or replace platform-dependent math (`sin`, `cos`, `exp`, `log`, …) with deterministic implementations.
  - Verify `pairs` iteration order is deterministic in omnilua (string-hash seed must be fixed), or forbid relying on it.
  - No `os`, `io`, `package`, or `debug` libraries.
- API shape: events (`on_unit_died`, `on_enter_region`, `every(ticks)`), queries (units in region, player resources), and actions (issue command, spawn, set objective, show message, victory/defeat).
- Each script call gets an instruction budget (`SandboxConfig::instruction_limit`).
- Unit and building stats stay in RON data files, not scripts.

### Data formats

- Rules: `assets/data/core.rules.ron`, with terrain types (`id`, map `symbol`, `walkable`, strategic/minimap `color`, optional `texture` asset path) and unit types (`id`, `max_health`, `radius` in tiles). Terrain textures are equally sized square images; the bundled tiles are 128×128 PNGs in `assets/textures/terrain/`. Textures are presentation data only.
- Maps: `assets/maps/*.map.ron`, with `name`, `players` (start positions), `tiles` (rows of terrain symbols, top to bottom), and `units` (pre-placed units: kind, owner index, position in tiles). M6 adds regions and a `script` path. M7 makes the editor write the same format.
- Distances and positions in data files are written in tiles (decimals allowed) and converted to world units on load.
- Data is validated on load: unknown symbols/kinds, ragged rows, out-of-bounds or unwalkable placements, and duplicate IDs are errors.

### Mobile controls (M2 onward)

| Gesture | Action |
|---|---|
| Tap unit | Select it |
| Tap ground/enemy with a selection | Move/attack order |
| One-finger drag | Box select |
| Two-finger drag | Pan camera |
| Pinch | Zoom |
| Long-press | Context menu (attack-move, patrol, …) |
| Minimap tap/drag | Jump camera |

Desktop keeps mouse plus keyboard: left-click/box select, right-click order, WASD/edge/middle-drag pan, wheel zoom.

The current camera (M0) pans with one-finger drag. M2 moves panning to two fingers.

### Map editor (M7)

- An editor mode inside the game binary (desktop first), reusing the renderer and data formats. UI with `bevy_egui` if it supports the Bevy version at the time.
- Tools: terrain brush/fill, player starts, unit placement with owner, regions, map properties, script assignment, undo/redo, save/load `*.map.ron`, play-test.

## Milestones

Status: ✅ done · 🚧 in progress · ⬜ not started

### ✅ M0 — Skeleton
Bevy app for desktop/web/Android, asset packing, RTS camera (keyboard/edge/drag pan, wheel and pinch zoom).

### ✅ M1 — Simulation core and map format
- `rts_base` crate: world units, rules and map RON formats with validation, terrain grid, units with stable IDs, fixed tick, deterministic state hash, determinism lints.
- Client: RON asset loaders, loading → in-game state, terrain rendered from map data (`TilemapChunk`), units rendered from sim state, camera bounded by the map and starting at the local player's start.
- Sample content: `core.rules.ron`, `proving_grounds.map.ron`.
- Done when: the game opens on the sample map with pre-placed units on desktop and web, and `cargo test -p rts_base` passes (including validation and hash tests).
- Terrain rendering: the client loads terrain textures from the asset pack and assembles one texture array in `TerrainId` order, preserving the single-draw-call tilemap. Types without a texture use their color. A separate 1×1-per-layer palette keeps the original colors for the future minimap and distant zoom. Below 40 logical screen pixels per tile, terrain switches to flat colors; above 48 it switches back to textures (hysteresis). The minimap remains M9.
- Notes: terrain colors live in a generated tileset (`map::terrain_colors`). Bevy's tilemap shader treats `TileData::color` tints as linear, which washes colors out, so don't use tints for terrain color.

### 🚧 M2 — Commands, selection, movement (next)
- `Command` type (move/stop), a per-tick command queue with input delay (local loopback for now).
- Selection: click/tap, box select (mouse and one-finger drag), two-finger pan. Selection markers.
- Grid A* pathfinding on walkable tiles, unit movement with speeds in rules, render interpolation between ticks.
- Done when: units can be selected and moved around obstacles on mouse and touch.

### ⬜ M3 — Multiplayer
- Relay server crate (WebSocket), lobby (create/join by code), lockstep turn exchange, input delay, setup checksum, desync detection via state hash, disconnect handling.
- Replay recording and playback.
- Done when: two clients (native and web) play the same match in sync, and a recorded replay reproduces the final hash.

### ⬜ M4 — Combat
Health, weapons (range, damage, cooldown), attack and attack-move orders, auto-targeting, death, health bars.

### ⬜ M5 — Economy and production
Resources and gathering, buildings with placement validation, construction, production queues, rally points, supply cap. HUD with resources and a command card (touch buttons).

### ⬜ M6 — Scripting
omnilua in the sim, deterministic math and RNG, map script loading, regions, triggers, objectives, victory/defeat conditions.

### ⬜ M7 — Map editor
See "Map editor" above.

### ⬜ M8 — Skirmish AI
A host-side AI player using the command API.

### ⬜ M9 — Fog of war, minimap, UX polish
Per-player visibility grid, minimap, control groups, mobile UX pass.

### ⬜ M10 — Scale
Flow-field pathfinding for groups, local avoidance, spatial index, chunked tilemap rendering, performance targets (for example 500 units at 60 FPS on mid-range Android).

## Open questions

- Relay server vs. `matchbox` P2P (decide at M3 start).
- Whether omnilua's table iteration order is deterministic (verify at M6 start).
- Factions/art direction (not needed before M5).
