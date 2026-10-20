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
| Multiplayer | Server-authoritative deterministic simulation with verified client replicas (see below). This constrains all simulation and script code. |
| Mobile controls | Every action reachable by touch; no hover or hotkey-only features. |

## Tech stack

- Engine/client: Bevy 0.20 (nightly toolchain, edition 2024).
- Simulation: `rts_base` crate, plain Rust, no Bevy dependency.
- Data: RON files via `serde` + `ron`.
- Networking: Authoritative WebSocket server with `ewebsock` clients and Tokio/`tokio-tungstenite` server; shared protocol in `rts_net`.
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
| `rts_client` | `crates/client` | The game: rendering, input, camera, lobby, match sessions and replay files. Binary `rts`; `cargo run` starts it. | base, assets, net |
| `rts_net` | `crates/net` | Bounded RON protocol, setup checksums, ordered turns and verified replays. No renderer or sockets. | base |
| `rts_server` | `crates/server` | Authoritative WebSocket server: configuration delivery, lobbies, fixed-rate simulation, validated commands and state hashes. Binary `rts-server`. | base, net |
| `rts_android` | `crates/android` | Android entry point (cdylib). | client |

Planned crates, created when their milestone starts (no empty placeholders):

| Crate | Path | Milestone | Role |
|---|---|---|---|
| `rts_script` | `crates/script` | M6 | omnilua integration: sandbox, deterministic math, game API bindings. Scripts run inside the sim step, so base and script can't simply depend on each other one way. Decide the split at M6 (for example, base calls a trait that script implements). |
| `rts_editor` | `crates/editor` | M7 | Map editor mode, plugged into the client. |
| `rts_ai` | `crates/ai` | M8 | Skirmish AI player that reads sim state and emits commands. |

Dependency direction: base ← assets/net ← client ← android; net ← server. Server and AI never depend on the client. Base never depends on Bevy.

The client never changes simulation state directly. It reads `Sim` to draw, and from M2 on it sends **commands** that the sim applies at a tick boundary.

### Deterministic simulation

The server owns the rules, map, tick clock and simulation. It sends the complete match configuration on room creation/join, then ordered commands and its resulting state hash every tick. Clients run deterministic replicas for rendering and replays, verify each tick against the server, and stop locally on divergence. Client state and hashes never determine server state. Replication currently sends all commands to all players; visibility filtering and snapshot resynchronization are not implemented.

Rules for code in `rts_base` (enforced partly by lints, see `crates/base/src/lib.rs` and `crates/base/clippy.toml`):

1. **No floating-point arithmetic.** Positions, speeds, and distances are integers in world units (`TILE = 1024` units per tile). Use `i64` for intermediates such as squared distances and `isqrt` for lengths. Floats appear only when parsing data files; decimal parsing and scaling by a power of two are exact, so the conversion is deterministic.
2. **No `HashMap`/`HashSet`** or anything else with unspecified iteration order. Use `BTreeMap`, `Vec`, or sorted keys.
3. **No wall clock, OS randomness, or thread timing.** Randomness comes from a seeded PRNG stored in sim state (added when first needed).
4. **Iteration order is defined by stable IDs.** Units have monotonically increasing `UnitId`s.
5. **Fixed tick rate:** `TICK_RATE = 20` ticks per second. Rendering interpolates between ticks (M2).
6. **State hash:** `Sim::state_hash()` uses FNV-1a with explicit little-endian, fixed-width encoding (`usize` is hashed as `u64`), so native and wasm32 peers agree. Clients compare their hashes with the server to detect desyncs (M3).

### Coordinates

- Sim: integer world units, `TILE = 1024` per tile. x grows right, **y grows down**. Tile `(0, 0)` is the top-left; map file rows are listed top to bottom, so the file looks like the map.
- Client: Bevy world space, y up. `map::to_world` converts: one tile is `TILE_PX = 128` pixels, and sim `y` is negated. The map spans from `(0, 0)` to `(width_px, -height_px)`.

### Commands and movement (M2)

- `Command::Move` and `Command::Stop` contain stable unit IDs. The local loopback queues commands for the current tick plus 2 (100 ms), preserving submission order. Each step advances the tick, applies that tick's commands, then moves units. M3 replaces local submission with agreed tick bundles.
- Only a unit's owner may command it. Missing IDs, invalid players, blocked/out-of-map targets, and unreachable destinations are ignored; an invalid replacement preserves the existing order.
- Four-neighbor grid A* minimizes travel time using integer reciprocal terrain-speed costs, with ties resolved by estimated remaining time and row-major tile index. Each center-to-center edge includes half a tile at each endpoint's speed. The heuristic uses the fastest defined terrain so faster routes are never overestimated. Routes visit tile centers, including the starting tile center, then the precise destination. This prevents arbitrary starts from cutting blocked corners.
- Terrain uses required `speed_modifier`: 0 blocks movement, 1 is normal, 0.5 is half speed, and 2 is double speed. RON decimals are rounded to Q10 fixed point (1024 represents 1); negative/nonfinite/out-of-range values and positive values rounding to zero are rejected. Runtime movement and pathfinding use integers only. The bundled grass/dirt/sand use 1 and water/rock use 0.
- Units consume integer `size` from the tile containing their center. Every tile has the rules' `tile_capacity`; the bundled capacity is 4, workers use 1, soldiers use 2. This allows four workers, two soldiers, or two workers and one soldier. All players share the same capacity, and map loading rejects over-capacity placements.
- Movement rebuilds occupancy from living units each tick and replans every active order against it. Stable unit ID order resolves contention, updating capacity after each move. Every traversed waypoint checks capacity, including high-speed movement; a full goal can be approached but cannot be entered until space is free. Orders wait and retry when units block a route. The current unobstructed partial segment is preserved to avoid reversing toward the previous center or losing fractional progress; later route segments are recomputed. Moving or removing a blocker frees capacity on the next movement tick.
- Unit `speed` is required, positive, and written in tiles/second at terrain modifier 1. Movement spends a shared integer effort budget per tick, using Q32 reciprocal terrain costs and retaining division remainders and fixed segment origins. Crossing a boundary spends effort at both terrains' speeds, even across multiple tiles in one tick. Slow units continue making progress without truncation drift. Queued commands, routes, weighted segment progress, and speed remainders are included in the state hash.
- The client snapshots unit positions before each simulation step and interpolates the latest two snapshots using Bevy's fixed-clock overstep fraction. Rendering, selection hit tests, and selection rings use the displayed positions. Presentation state never changes the deterministic simulation or its hash.
- Capacity limits overlap within a tile; it does not separate unit circles inside that allowed capacity. Radius-aware terrain clearance, physical separation, formations, simultaneous traffic swaps and large-group pathfinding remain M10. Replanning currently runs A* per moving unit per tick; scale optimizations must preserve live occupancy behavior.
- Capacity follow-up verified with 107 workspace tests, workspace Clippy with warnings denied, formatting, wasm32 and Android arm64 checks. Tests cover exact and excess mixed occupancy, fractional and high-speed contention, blocked-goal waiting/resumption, moving/removing blockers, immediate replacement of stale detours, and replay final-hash verification.
- Terrain-speed follow-up verified with 122 workspace tests, workspace Clippy with warnings denied, formatting, wasm32 and Android arm64 checks. Tests cover integer/decimal modifiers, zero-speed blocking, exact half/normal/double speeds, boundary crossings in both axes and directions, mixed terrain within one tick, fractional progress, fastest-route choices, live occupancy, and every-turn replay hash verification.

### Networking (M3)

- The server advances each match at 20 Hz independently of client submissions; absent input becomes an empty batch. Clients send commands for the latest received server tick plus two (100 ms). Delayed input cannot slow the game; past, duplicate, out-of-order and excessively future submissions are rejected. Offline play uses the same turn format and two-tick delay.
- Each server room owns a `Sim`. Connections are bound to player slots; ownership, targets, path validity, movement and tile capacity are checked by the server simulation. The server publishes a turn and its resulting hash each tick. Clients apply only server turns, verify the hash before recording, and stop locally on divergence. No client hash votes, client state uploads or client-controlled tick advancement exist.
- Six-character room codes, slot-zero host, map-defined capacity (two on the sample map). The server sends `MatchSetup` containing minified map/rules RON, seed and player count to each new room member. Protocol v4 strips comments/extra whitespace without changing literal bytes, zlib-compresses each document with pure-Rust `miniz_oxide`, and sends raw compressed bytes in a binary WebSocket frame. A 33-byte header identifies setup/version, seed/player count, and each document's compressed/decompressed lengths; lobby and gameplay messages remain RON text. Clients check compressed/decompressed limits and stream integrity before accepting the setup. Clients validate and acknowledge the setup checksum; only the host may start a full room after every member is ready. Clients need compatible game code/protocol, but their bundled map and rules may differ.
- The server defaults to bundled map/rules and supports `--rules PATH`, `--map PATH`, and `--seed N` for server-side content mods without rebuilding clients. The checksum covers minified rules/map RON, seed and player count, so source formatting/comments do not affect readiness. Lexical minification preserves string/character/raw literals and decimal spelling, avoiding reserialization of positions/speeds already converted to world units. The seed remains reserved for future randomness. Terrain texture files remain local presentation assets; mods can use bundled texture paths or colors (`texture: None`).
- An in-game departure or connection timeout ends the room; a guest leaving before start frees its slot. Host departure closes the room. Simulation catch-up and per-connection queues are bounded.
- Native clients use `ws://`. Browsers also support `wss://` through their platform WebSocket; HTTPS hosting requires a TLS reverse proxy. Native TLS is disabled to preserve the pure-Rust dependency requirement: standard Rustls providers add C/assembly. The server runtime is native-only, with a wasm build stub.
- Replays store the setup and every applied turn, and verify the final hash when loaded, exported or played back. Playback accepts no orders. Wire/setup/replay sizes and nesting are bounded; see [protocol details](M3_PROTOCOL.md).
- The menu exposes solo play, server address, create/join/start/leave, replay last and replay save/load. Touching an editable lobby field opens an in-game keypad because Bevy's platform keyboard support is incomplete; desktop typing uses built-in text input. Menu and keypad scroll on small screens.
- Native Save replay replaces `latest.replay.ron` in the app data directory; Load saved replay opens that file. Browser save downloads a RON file and load opens a file picker. Android uses internal app storage. [Run instructions and storage paths](PLAYING.md).

### AI

AI is a virtual player that issues ordinary commands. It will run on the server and submit commands through the same authoritative validation as human input, so AI decision code does **not** need to be deterministic.

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

- Rules: `assets/data/core.rules.ron`, with required positive `tile_capacity`, terrain types (`id`, map `symbol`, `speed_modifier`, strategic/minimap `color`, optional `texture` asset path) and unit types (`id`, `size`, `max_health`, `radius` in tiles, `speed` in tiles/second). `size` is required, positive, and at most `tile_capacity`; both are integer values up to 65535. Terrain textures are equally sized square images; the bundled tiles are 128×128 PNGs in `assets/textures/terrain/`. Textures are presentation data only.
- Maps: `assets/maps/*.map.ron`, with `name`, `players` (start positions), `tiles` (rows of terrain symbols, top to bottom), and `units` (pre-placed units: kind, owner index, position in tiles). M6 adds regions and a `script` path. M7 makes the editor write the same format.
- Distances and positions in data files are written in tiles (decimals allowed) and converted to world units on load.
- Data is validated on load: unknown symbols/kinds, ragged rows, out-of-bounds or zero-speed terrain placements, over-capacity tiles, and duplicate IDs are errors.

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

Desktop: left-click/box select, Shift to add/toggle selection, right-click to move, X to stop, Escape to clear selection, WASD/arrows/edge/middle-drag pan, wheel zoom. The command bar has touch-accessible Stop and Clear buttons; HUD input never issues world orders.

One-finger drag selects; two-finger drag/pinch controls the camera and cancels any pending selection/order until all fingers lift. Touch cancellation and loss of window focus also cancel pending input. Touch picking has at least a 48-pixel diameter. Long-press context orders arrive with combat, and the minimap remains M9.

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

### ✅ M2 — Commands, selection, movement
- ✅ Move/stop commands, per-tick queue with local input delay, ownership validation and deterministic hashing.
- ✅ Deterministic grid A* on walkable terrain and integer movement with required speeds in rules.
- ✅ Selection: click/tap, box select, two-finger pan, selection markers, touch command bar.
- ✅ Render interpolation between ticks and end-to-end verification.
- Done when: units can be selected and moved around obstacles on mouse and touch.
- Verified: 46 workspace tests, workspace Clippy, wasm32 check, Android arm64 check. A native GPU check at 320 logical pixels wide injected touch/window events through Bevy and verified selection, movement, HUD Stop before arrival, Clear, and the rendered selection rings and responsive HUD. Touch drag, pinch cancellation, desktop selection, delayed orders, and obstacle routing also have automated tests. This did not run on an Android device.

### ✅ M3 — Multiplayer
- ✅ Authoritative server crate (WebSocket), lobby (create/join by code), server-owned map/rules delivery, readiness handshake, fixed-rate simulation, input delay, command ownership validation, per-turn server hashes and disconnect handling.
- ✅ Replay recording, playback, native persistence and browser import/export controls; touch keypad for lobby fields.
- Done when: two clients (native and web) play the server's match in sync, and a recorded replay reproduces the final hash.
- Binary setup follow-up verified with 152 workspace tests, strict workspace Clippy, formatting, full-workspace wasm32 and Android arm64 checks. Protocol v4 sends raw zlib bytes with a fixed little-endian setup header and retains RON text for lobby/gameplay. The base64 setup codec and direct dependency were removed. Tests verify exact framing, both WebSocket message types, all truncated prefixes, invalid tags/versions/counts/lengths, corrupt/trailing streams, readiness, deterministic turns and replay export through real client/server sockets. Rendered browser/device sessions were not rerun for this transport change.
- Earlier setup compression follow-up verified with 151 workspace tests, strict workspace Clippy, formatting, full-workspace wasm32 and Android arm64 checks. Protocol v3 delivers lexically minified, zlib/base64-compressed map/rules and compares checksums over minified text. Tests preserve decimals, Unicode, characters, escaped/raw/byte literals, identifiers and RON attributes; reject corrupt/truncated/trailing/oversized compression envelopes and invalid UTF-8; and verify compressed setup readiness, deterministic turns and plain-RON replay export through real client/server sockets. Rendered browser/device sessions were not rerun for this transport change.
- Authoritative-server follow-up verified with 134 workspace tests, workspace Clippy with warnings denied, formatting, full-workspace wasm32 and Android arm64 checks (`cargo ndk -t arm64-v8a check --workspace`). Real-socket headless Bevy clients loaded a server-only unit type and changed stats, checked matching hashes through 40 turns, applied Move/Stop and exported a replay reproducing the final hash. Tests cover readiness, server-owned cadence without input, ownership filtering, invalid/stale/duplicate/future input, command-work bounds, client backlog catch-up, large escaped setup transfer, eight-player wire fan-in and bounded scheduler stalls. This follow-up compiled web/Android but did not rerun rendered browser or device sessions.
- Initial M3 verification (before the authoritative-server follow-up): 90 workspace tests, workspace Clippy with warnings denied, formatting, wasm32 check, Android arm64 Rust check, and Android activity/manifest compilation. Real native and browser clients using the full client app and session plugin exchanged commands through the relay: both players queued Move at tick 4 (applied on 6) and Stop at tick 12 (applied on 14). Both reached tick 40 with identical unit positions and hash `4f7f7cc668b22cbd`; the saved 40-turn replay loaded and reproduced the same hash. Native used NVIDIA Vulkan; browser used headless Chromium with software WebGPU via Playwright after T3 preview snapshots timed out twice.
- A native GPU run at 320 logical pixels wide injected touch/window events through Bevy to type a room code, commit it, cancel address editing, start solo play, save, leave, replay the recording, and load the saved file. Both playback paths verified their final hash. Keypad, lobby and session bars were visually inspected. Android permissions are configured, including Android 17 LAN access; this milestone did not run on an Android device. Browser file-picker/download controls were source-reviewed and compiled, but not exercised in the browser match.

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

- Whether omnilua's table iteration order is deterministic (verify at M6 start).
- Factions/art direction (not needed before M5).
