Verification records
====================

Each milestone and significant follow-up records what was checked, newest
first within each milestone. Add an entry when you finish one. State plainly
what was *not* run, such as rendered browser sessions or a physical Android
device.

Client scenes
-------------

**Unit visuals as Bevy scenes.** 153 workspace tests, strict workspace
Clippy, formatting, and full-workspace ``wasm32`` and Android arm64 checks.
The headless scene lifecycle test covers spawning, stable IDs and transforms,
shared mesh and material caches, unit removal, match-exit cleanup, and
preservation of the simulation hash. Rendered sessions were not rerun for
this change.

M3 — Multiplayer
----------------

**Binary setup (protocol v4).** 152 workspace tests, strict workspace Clippy,
formatting, and full-workspace ``wasm32`` and Android arm64 checks. Protocol
v4 sends raw zlib bytes with a fixed little-endian setup header and keeps RON
text for lobby and gameplay messages; the base64 setup codec and its
dependency were removed. Tests verify exact framing, both WebSocket message
types, every truncated prefix, invalid tags, versions, counts, and lengths,
corrupt and trailing streams, readiness, deterministic turns, and replay
export through real client and server sockets. Rendered browser and device
sessions were not rerun for this transport change.

**Setup compression (protocol v3).** 151 workspace tests, strict workspace
Clippy, formatting, and full-workspace ``wasm32`` and Android arm64 checks.
Protocol v3 delivered lexically minified, zlib- and base64-compressed map and
rules and compared checksums over minified text. Tests preserve decimals,
Unicode, characters, escaped, raw, and byte literals, identifiers, and RON
attributes; reject corrupt, truncated, trailing, or oversized compression
envelopes and invalid UTF-8; and verify compressed setup readiness,
deterministic turns, and plain-RON replay export through real sockets.
Rendered sessions were not rerun.

**Authoritative server.** 134 workspace tests, workspace Clippy with warnings
denied, formatting, and full-workspace ``wasm32`` and Android arm64 checks
(``cargo ndk -t arm64-v8a check --workspace``). Real-socket headless Bevy
clients loaded a server-only unit type with changed stats, checked matching
hashes through 40 turns, applied Move and Stop, and exported a replay
reproducing the final hash. Tests cover readiness, server-owned cadence
without input, ownership filtering, invalid, stale, duplicate, and future
input, command-work bounds, client backlog catch-up, large escaped setup
transfer, eight-player fan-in, and bounded scheduler stalls. Web and Android
compiled, but rendered browser and device sessions were not rerun.

**Initial M3 (relay design, before the authoritative server).** 90 workspace
tests, workspace Clippy with warnings denied, formatting, ``wasm32`` check,
Android arm64 Rust check, and Android activity and manifest compilation. Real
native and browser clients using the full client app exchanged commands
through the relay: both players queued Move at tick 4 (applied on tick 6) and
Stop at tick 12 (applied on tick 14). Both reached tick 40 with identical unit
positions and hash ``4f7f7cc668b22cbd``; the saved 40-turn replay loaded and
reproduced it. Native used NVIDIA Vulkan; the browser was headless Chromium
with software WebGPU through Playwright.

**Touch lobby and replays.** A native GPU run at 320 logical pixels wide
injected touch and window events through Bevy to type a room code, commit it,
cancel address editing, start solo play, save, leave, replay the recording,
and load the saved file. Both playback paths verified their final hash. The
keypad, lobby, and session bars were inspected visually. Android permissions
are configured, including Android 17 local network access, but the milestone
did not run on an Android device. Browser file picker and download controls
were reviewed and compiled but not exercised in the browser match.

M2 — Commands, selection, movement
----------------------------------

**Terrain speed modifiers.** 122 workspace tests, workspace Clippy with
warnings denied, formatting, and ``wasm32`` and Android arm64 checks. Tests
cover integer and decimal modifiers, zero-speed blocking, exact half, normal,
and double speeds, boundary crossings in both axes and directions, mixed
terrain within one tick, fractional progress, fastest-route choices, live
occupancy, and every-turn replay hash verification.

**Tile capacity.** 107 workspace tests, workspace Clippy with warnings
denied, formatting, and ``wasm32`` and Android arm64 checks. Tests cover exact
and excess mixed occupancy, fractional and high-speed contention, waiting at
and resuming into a blocked goal, moving and removing blockers, immediate
replacement of stale detours, and replay final-hash verification.

**Milestone.** 46 workspace tests, workspace Clippy, ``wasm32`` check, and
Android arm64 check. A native GPU check at 320 logical pixels wide injected
touch and window events through Bevy and verified selection, movement, HUD
Stop before arrival, Clear, the rendered selection rings, and the responsive
HUD. Touch drag, pinch cancellation, desktop selection, delayed orders, and
obstacle routing also have automated tests. It did not run on an Android
device.

M1 — Simulation core
--------------------

Done when the game opened on the sample map with pre-placed units on desktop
and web, and ``cargo test -p rts_base`` passed, including validation and hash
tests.

**Terrain textures.** The client loads terrain textures from the asset pack
and assembles one texture array in ``TerrainId`` order, keeping the single
draw call. Types without a texture use their color. A separate color palette
is kept for distant zoom and the future minimap, with a hysteresis switch
between 40 and 48 screen pixels per tile.
