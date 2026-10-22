Verification records
====================

Each milestone and significant follow-up records what was checked, newest
first within each milestone. Add an entry when you finish one. State plainly
what was *not* run, such as rendered browser sessions or a physical Android
device.

Tooling
-------

**CI and release workflows.** Added continuous integration and a manual
release pipeline. `actionlint` 1.7.12 with ShellCheck 0.11.0 accepted both
workflows, and the extracted run blocks passed `bash -n`. The changelog script
was exercised against an empty
tag history, a later release, and a re-run of the latest tag, and the release
asset checks and download table were generated locally. Gradle configuration
with `-PversionName` and `-PversionCode` succeeded. Formatting, all workspace
tests, strict all-target Clippy, the full-workspace ``wasm32`` check, and the
strict Sphinx build passed. The workflows have not yet run on GitHub-hosted
runners, and the Windows, macOS, Linux, Android, and Pages packaging and
deployment steps were not executed end to end here.

Client scenes
-------------

**Phone HUD with floating controls.** All 241 workspace tests, strict
all-target Clippy, formatting, the client WebAssembly check, a Trunk build,
and the strict Sphinx build passed. Shared-geometry regressions cover phone
portrait and landscape at 100 and 150 percent scale, visible-only pointer
exclusion, hidden docks with no selection, popup capture and focus cleanup,
full-window camera limits, corner-control clearance, small-map centering, and
whole-panel scrolling on short screens at 150 percent scale.
Headless Chromium with software WebGL rendered 320×640 portrait and 844×390
landscape, selected five units, and opened the map and unit-details panels.
At 568×320 and 150 percent UI scale, mouse-wheel and emulated touch drags
revealed complete portrait targets while the Close control stayed fixed.
Physical-device gameplay and an Android build were not run.


**Command image assets.** Bottom-HUD Move, Stop, and Clear icons load from
transparent PNG files through the existing asset server and pack. A regression
checks the actual HUD image paths and decodes each asset, including transparent
backgrounds and white glyph pixels. Labels retain their transparent backgrounds.
Workspace tests, strict all-target Clippy, formatting, the client WebAssembly
check, and the strict Sphinx build passed. A rendered session and Android
build were not rerun for this asset-only follow-up.


**RTS console and pause menu.** All 227 workspace tests, strict all-target
Clippy, formatting, the client WebAssembly check, and the strict Sphinx build
passed. Regressions cover retained selection across Escape, pause/resume of
solo and replay turns while online keeps advancing, modal capture for mouse
and touch, Move targeting, command availability, portrait selection, narrow
HUD targets, transparent button labels, portrait narrowing, focus cleanup
when closing the menu, and camera limits and downward edge pan above the HUD.
Headless Chromium with software WebGL rendered the console at desktop and
320-pixel widths, box-selected five units, and opened the pause menu using
Escape and its button. Android builds and physical device sessions were not
rerun.


**Shared map and unit zoom detail.** Unit sprites and terrain now consume one
zoom state using the map's existing 40/48 screen-pixels-per-tile thresholds.
The separate unit-size thresholds and configuration were removed. The scene
regression checks differently sized, crowded units at both boundaries and in
the hysteresis band, including unchanged bounds, anchors, and simulation hash.
All 211 workspace tests, strict Clippy with all targets, formatting, WebAssembly
and Android arm64 checks, and the strict Sphinx build passed. Browser and
Android device sessions were not rerun for this threshold change.


**Continuous travel through tile centers.** Render travel now interpolates
simulation positions separately from packing offsets and scale. Travelling groups
keep their slots without clamping to tile interiors. A regression drives the
actual simulation and unit synchronization at 30, 60, 120, and 144 FPS, checking
every frame's travel distance through multiple tile centers and boundaries.
Additional regressions cover constant group offsets across four tiles and
stalled/resumed simulation segments without rewinding. All 211 workspace tests, strict Clippy with all targets, formatting,
full-workspace WebAssembly and Android arm64 checks, and the strict Sphinx build
passed. Rendered browser and
Android device sessions were not rerun for this correction.


**Smooth unit movement and sprite detail.** 208 workspace tests, strict
workspace Clippy with all targets, formatting, full-workspace ``wasm32`` and
Android arm64 checks (through ``cargo ndk``), a Trunk debug build, and the
strict Sphinx build passed. Tests cover tile-boundary continuity, regrouping
position and scale, 30–144 FPS response, exact settling, required sprite paths,
transparent PNG decoding, shared image handles and player tinting, configurable
zoom hysteresis, matching sprite and rectangle bounds/anchors, and unchanged
simulation hashes. The obsolete previous-tick presentation snapshots were
removed; repeated stalled turns preserve input, simulation state, and replay
turns. A headless Chromium session with SwiftShader rendered team-tinted
sprites and distant bounding rectangles and exercised box and single-unit
selection, then issued a move order and checked travel and arrival. No Android
device session was run.


**Crisp map previews.** Generated catalog, save, and replay previews now use
nearest-neighbor sampling. Workspace tests, strict Clippy, formatting, the
full-workspace ``wasm32`` check, and the strict Sphinx build passed. Rendered
browser and device sessions were not rerun for this sampler-only change.

**Plotting-table interface redesign.** 199 workspace tests, strict workspace
Clippy, formatting, and full-workspace ``wasm32`` checks. A Trunk debug build
rendered in Chromium with SwiftShader WebGL at desktop and phone widths;
visual checks cover the home page, compact navigation, map previews,
scrolling to Start game, and the in-game bars at 320 pixels wide.
The existing scene smoke test still covers every page and dialog in both
layout modes. No Android device session was run.

**Scene-based main menu.** 199 workspace tests, strict workspace Clippy on
native and the client on ``wasm32``, formatting, full-workspace ``wasm32``
and Android arm64 checks, and the strict Sphinx build. Tests cover storage
round trips and the browser codec; library numbering, reload, and corrupt
records; settings repair; catalog loading with an invalid map skipped and
no valid map staying in ``Loading``; map previews; and the keypad for all
three fields. A headless menu smoke test spawns every page and dialog in
wide and compact layouts and checks that no entities remain after cleanup.
Menu tests cover action availability, Back, dialog closure, recent server,
leave confirmation, browse filters, and selection.

Session tests cover saved games continuing with matching hashes, replay
import, lobby readiness, room closure, and refusal while connecting. A
real-socket two-client match covers the room list, create/join with names,
explicit Ready, Start, 40 verified turns, leaving with connections kept,
and replay saving. Protocol and server tests cover v5. A rendered native
Wayland session captures every page and dialog in desktop and compact
layouts, using 150 percent interface scale for compact mode, against a live
server with a second client hosting a public room. A Trunk debug build in
headless Chromium (SwiftShader WebGL) rendered the menu without Quit, stored
settings and a saved solo game in compressed ``localStorage`` entries, and
listed the save with its preview after a page reload. Multiplayer, replay
import and export, and touch input were not run in a browser. No physical
Android device session was run.

**Graphics settings in RON.** 178 workspace tests, strict workspace Clippy,
formatting, full-workspace ``wasm32`` and Android arm64 checks, and the strict
Sphinx build. Tests cover the bundled palette, slot wrapping, invalid colors,
empty palettes, unknown and missing fields, nonfinite or invalid dimensions
and durations, and ordered terrain detail thresholds. Headless asset loading
checks graphics installation before the menu and startup rejection of missing
or malformed graphics. Scene tests verify custom colors in actual shared
unit materials and unchanged simulation hashes; zoom tests cover configurable
hysteresis boundaries. Rendered browser and physical device sessions were
not rerun for this follow-up.

**Configurable unit visual bounds.** 173 workspace tests, strict workspace
Clippy, formatting, full-workspace ``wasm32`` and Android arm64 checks, and
the strict Sphinx build. Rules tests cover
optional and explicit bounds, signed offsets, fixed-point conversion,
positive dimensions, malformed or missing fields, nonfinite and out-of-range
values, and unchanged movement and hashes over 80 ticks. Client tests cover
rectangular footprints with different offsets at tile edges, grid orientation,
root-to-bounds conversion, actual scene placement and scale, rectangular click
and box selection, and server-provided bounds through a real match and replay.
Rendered browser and physical device sessions were not rerun for this
follow-up.

**Tile-local unit groups.** 161 workspace tests, strict workspace Clippy,
formatting, the client ``wasm32`` check, and the strict Sphinx build. Tests
cover crowded and mixed radius groups at tile centers and edges, stable ID
ordering, independent tiles, oversized custom units, interpolation of unchanged groups, actual
scene offsets and scale, removal and scale restoration, scaled selection,
and unchanged simulation hashes. Smooth transitions may briefly overlap;
settled layouts keep unit bodies separate and inside their tile. A rendered
Chromium session using software WebGL box-selected four workers, moved them
to one tile, and selected one worker at its displayed offset. Android builds
and physical device sessions were not rerun.

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
