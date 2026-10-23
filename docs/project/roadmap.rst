Roadmap
=======

The game is built in milestones. Each one ends with a playable, verified
increment. When you finish a milestone or change a decision, update this page,
the affected architecture page, and :doc:`verification`.

Status
------

.. list-table::
   :header-rows: 1
   :widths: 8 30 14 48

   * - ID
     - Milestone
     - Status
     - Scope
   * - M0
     - Skeleton
     - Done
     - App on three platforms, asset pack, camera.
   * - M1
     - Simulation core and map format
     - Done
     - ``rts_base``, data formats, terrain and unit rendering.
   * - M2
     - Commands, selection, movement
     - Done
     - Orders, pathfinding, tile capacity, touch selection.
   * - M3
     - Multiplayer
     - Done
     - Authoritative server, lobby, replays.
   * - M4
     - Combat
     - Not started
     - Health, weapons, attack orders, death.
   * - M5
     - Economy and production
     - Not started
     - Resources, buildings, production, command card.
   * - M6
     - Scripting
     - Not started
     - Lua missions, triggers, objectives.
   * - M7
     - Map editor
     - Not started
     - In-game editor for ``*.map.ron``.
   * - M8
     - Skirmish AI
     - Not started
     - Server-side AI player.
   * - M9
     - Fog of war, minimap, UX polish
     - Partial
     - Minimap done; visibility, control groups, and further UX pending.
   * - M10
     - Scale
     - Not started
     - Flow fields, avoidance, performance targets.

Completed
---------

M0 — Skeleton
   Bevy app for desktop, web, and Android; asset packing; RTS camera with
   keyboard, edge, and drag pan, and wheel and pinch zoom. macOS releases keep
   the asset pack in the app bundle's Resources directory for code signing.

M1 — Simulation core and map format
   ``rts_base`` with world units, validated rules and map RON formats, the
   terrain grid, units with stable IDs, the fixed tick, the deterministic state
   hash, and determinism lints. Client RON loaders, the loading → menu → game
   flow, terrain rendered from map data as one tilemap chunk with a texture
   array and a color palette for distant zoom, units rendered from simulation
   state, and a camera bounded by the map that starts at the local player's
   start. Sample content: ``core.rules.ron`` and ``proving_grounds.map.ron``.

M2 — Commands, selection, movement
   Move and Stop commands with a per-tick queue, input delay, ownership
   validation, and hashing. Deterministic travel-time A* with terrain speed
   modifiers and integer movement. Tile capacity with live replanning.
   Click, tap, and box selection, two-finger pan, selection rings, the touch
   command bar, and smooth rendered movement between ticks. Units sharing a tile
   use configurable rectangular visual bounds (or radius-derived defaults)
   and stable grid slots, keeping crowded groups large by letting them spill
   past the tile while keeping movement smooth and simulation state unchanged.

M3 — Multiplayer
   The authoritative WebSocket server with create/join by code, server-owned
   map and rules delivery as a compressed binary setup, readiness handshake,
   fixed-rate simulation, input delay, input validation, per-turn hashes, and
   disconnect handling. Replay recording, playback, native persistence, and
   browser import and export. Menu text fields use the platform keyboard. Unit
   visuals as Bevy scenes.

Local diagnostics
   Panic capture starts before asset loading and plugin construction. Reports
   remain local until exported or discarded; app-storage exports live separately
   from pending reports, and log rotation failures still keep the log bounded.

Unit movement and sprites
   Linear simulation-position interpolation keeps travel speed steady through
   tile centers and boundaries. Packed groups are anchored at the point they
   were ordered to, never at the tile center, so arriving never shifts them and
   a lone unit stands exactly on its ordered point. Packing offsets and scale
   ease independently when groups change or units stop. Unit rules require sprite
   paths; bundled worker and soldier sprites use transparent backgrounds and
   player-color tinting. Distant units switch to their solid bounding
   rectangles at the same zoom as terrain colors, sharing the map's configurable
   thresholds and hysteresis.

Client presentation data
   Player palettes, selection outlines, move feedback, and terrain detail
   thresholds are configured in a validated local ``client.graphics.ron``
   asset. Startup loads it before opening the main menu. Graphics stay outside
   simulation state, network setup, and replays.

Interface redesign
   Plotting-table visual direction with graphite and slate surfaces, ocher
   actions, terrain previews with corner marks, selection stripes, and square
   status lamps. Compact navigation moves above the page. The menu and
   in-game command bar share their control palette and interaction states.

In-game console and pause menu
   A top-left pause button and Escape open a modal with Resume, Save game,
   Save replay, and Leave. Solo and replay sessions pause; online matches
   continue syncing with world controls blocked. The bottom console has an
   interactive terrain minimap with unit markers and camera outline,
   selected-unit portraits and details, and a spatial Move/Stop/Clear command
   card. Command icons load from PNG assets in ``textures/ui/commands`` and
   follow the button-state tint. Phone layouts use a floating contextual dock
   with 48-pixel commands, a top-right map thumbnail, and on-demand map/details
   panels. Width and height thresholds cover both phone orientations. Shared
   geometry keeps unused space playable, and camera limits allow edge units
   to move out from under corner controls.

Main menu
   Scene-based menu with a sidebar, pages, dialogs, and a compact layout.
   Map selection includes crisp, nearest-neighbor previews and a second map,
   *Crossroads* (four players, 40×40 tiles). Solo saved games continue
   recorded matches.
   A replay library supports browsing, filtering, deletion, and browser
   import/export. Persistent settings cover player name, interface scale,
   edge scrolling, and desktop fullscreen. Protocol v5 adds a room list,
   player names, and explicit readiness separate from setup loading.

Upcoming
--------

M4 — Combat
   Health, weapons (range, damage, cooldown), attack and attack-move orders,
   auto-targeting, death through ``Sim::remove_unit``, and health bars as unit
   scene children. Tapping an enemy attacks it; long-press opens a context
   menu with attack-move and patrol.

M5 — Economy and production
   Resources and gathering, buildings with placement validation,
   construction, production queues, rally points, and a supply cap. A HUD with
   resources and a touch command card. Building visuals and command-card
   widgets use scene composition.

M6 — Scripting
   omnilua in the simulation, deterministic math and random numbers, map
   script loading, regions, triggers, objectives, and victory and defeat
   conditions. See :doc:`scripting`.

M7 — Map editor
   An editor mode in the game binary that writes ``*.map.ron``. See
   :doc:`map-editor`.

M8 — Skirmish AI
   A server-side AI player that uses the command API. See
   :doc:`skirmish-ai`.

M9 — Fog of war, minimap, UX polish
   The terrain minimap with tap and drag camera navigation is implemented in
   the in-game console. Remaining work: a per-player visibility grid,
   visibility-aware minimap markers, control groups, and a mobile UX pass. Fog of war also needs a visibility-aware protocol so clients do not
   receive hidden information.

M10 — Scale
   Flow-field pathfinding for groups, local avoidance, a spatial index,
   chunked tilemap rendering, and performance targets such as 500 units at
   60 FPS on a mid-range Android device. Optimizations must preserve live
   occupancy behavior and determinism.

Open questions
--------------

* Whether omnilua's table iteration order is deterministic. Verify at the
  start of M6.
* Factions and art direction. Not needed before M5.
