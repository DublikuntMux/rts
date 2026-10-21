Architecture overview
=====================

RTS separates a deterministic simulation core from everything that presents
or transports it. The core is plain Rust with no engine dependency; the
client is a Bevy application that draws the simulation and turns input into
commands; the server runs the authoritative copy of each match.

Hard requirements
-----------------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Requirement
     - Consequence
   * - Pure Rust
     - No C or C++ dependencies. Scripting will use ``omnilua`` (pure-Rust
       Lua), not ``mlua`` or Luau. Native TLS is not built in because standard
       Rustls providers include C and assembly.
   * - Desktop, web (``wasm32-unknown-unknown``), and Android
     - Every dependency must build for all three. Networking must work in
       browsers, so it uses WebSockets.
   * - Multiplayer
     - A server-authoritative deterministic simulation with verified client
       replicas. This constrains all simulation and future script code; see
       :doc:`determinism`.
   * - Mobile controls
     - Every action is reachable by touch; nothing depends on hover or on a
       hotkey alone.

Technology
----------

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Area
     - Choice
   * - Engine and client
     - Bevy 0.20 on the nightly toolchain, edition 2024.
   * - Simulation
     - ``rts_base``: plain Rust, integer math, ordered collections.
   * - Data
     - RON files through ``serde`` and ``ron``.
   * - Networking
     - Authoritative WebSocket server on Tokio and ``tokio-tungstenite``;
       ``ewebsock`` clients; shared protocol in ``rts_net``; ``miniz_oxide``
       for setup compression.
   * - Assets
     - ``bevy_asset_packer`` bundles ``assets/`` into ``assets.pack`` at build
       time.
   * - Scripting (M6)
     - ``omnilua`` with only the base, string, table, and math libraries, run
       through ``Lua::sandboxed`` with an instruction limit.
   * - Web build
     - Trunk, with ``index.html`` and ``Trunk.toml`` at the repository root.
   * - Android build
     - Gradle project in ``android/`` that builds ``crates/android`` with
       ``cargo ndk``.

Bevy's default features are disabled workspace-wide; :doc:`client` lists the
features in use and why.

Repository layout
-----------------

.. kroki::
   :type: plantuml
   :caption: Top-level repository layout.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.2
   {
       {T
           + rts/ | .
           ++ Cargo.toml | Workspace only: members, dependency versions, lints, profiles
           ++ crates/ | All Rust code
           +++ base/ | rts_base: deterministic core
           +++ net/ | rts_net: protocol, turns, replays
           +++ server/ | rts_server: authoritative server, binary rts-server
           +++ assets/ | rts_assets: asset pack and data loaders
           +++ client/ | rts_client: the game, binary rts
           +++ android/ | rts_android: Android entry point
           ++ assets/ | Game content: rules, maps, textures, font
           ++ android/ | Gradle app
           ++ docs/ | This handbook
           ++ index.html | Web shell for Trunk
           ++ Trunk.toml | Web build configuration
       }
   }
   @endsalt

Crates
------

.. list-table::
   :header-rows: 1
   :widths: 18 52 30

   * - Crate
     - Role
     - Depends on
   * - ``rts_base``
     - Deterministic game core: coordinates, IDs, rules and map formats,
       simulation, pathfinding, state hash. No Bevy.
     - —
   * - ``rts_net``
     - Bounded RON protocol, binary setup framing, setup checksums, ordered
       turns, and verified replays. No renderer or sockets.
     - base
   * - ``rts_server``
     - Socket-free room coordinator and native WebSocket runtime: lobbies,
       setup delivery, fixed-rate simulation, input validation, state hashes.
       Binary ``rts-server``.
     - base, net
   * - ``rts_assets``
     - Builds ``assets.pack`` in ``build.rs``, opens it per platform, and
       registers Bevy loaders for ``*.rules.ron``, ``*.map.ron``, and
       ``*.graphics.ron``.
     - base
   * - ``rts_client``
     - The game: rendering, input, camera, scene-based menu, match sessions,
       saved games, replay library, and settings. Binary ``rts``; ``cargo run`` starts it.
     - base, assets, net
   * - ``rts_android``
     - Android ``cdylib`` entry point.
     - client

.. kroki::
   :type: plantuml
   :caption: Crate dependencies. Arrows point from a crate to what it uses.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   skinparam componentStyle rectangle
   component rts_android
   component rts_client
   component rts_server
   component rts_assets
   component rts_net
   component rts_base
   component Bevy
   component "Tokio\ntokio-tungstenite" as tokio
   component ewebsock
   rts_android --> rts_client
   rts_client --> rts_assets
   rts_client --> rts_net
   rts_client --> rts_base
   rts_client --> Bevy
   rts_client --> ewebsock
   rts_assets --> rts_base
   rts_assets --> Bevy
   rts_server --> rts_net
   rts_server --> rts_base
   rts_server --> tokio
   rts_net --> rts_base
   note right of rts_base
     No Bevy, no floats in arithmetic,
     no hashed collections, no clocks.
   end note
   @enduml

Dependency rules:

* ``rts_base`` never depends on Bevy or any other workspace crate.
* The server and the future AI never depend on the client.
* Protocol code in ``rts_net`` has no socket or renderer dependency, so the
  same turn and replay logic runs in the server, the client, and tests.
* Dependency versions live in ``[workspace.dependencies]``; crates use
  ``dep.workspace = true``.

Planned crates are created when their milestone starts, never as empty
placeholders:

.. list-table::
   :header-rows: 1
   :widths: 18 12 70

   * - Crate
     - Milestone
     - Role
   * - ``rts_script``
     - M6
     - omnilua integration: sandbox, deterministic math, game API. See
       :doc:`../project/scripting`.
   * - ``rts_editor``
     - M7
     - Map editor mode plugged into the client. See
       :doc:`../project/map-editor`.
   * - ``rts_ai``
     - M8
     - Skirmish AI player that reads state and emits commands. See
       :doc:`../project/skirmish-ai`.

How a match runs
----------------

.. kroki::
   :type: plantuml
   :caption: Data flow during an online match. Solo play uses the same turn format without a socket.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   skinparam componentStyle rectangle
   package "Client (rts_client)" {
     [Input and HUD] as input
     [Match.pending\ncommand list] as pending
     [Session] as session
     [Client Sim replica] as csim
     [Rendering\n(terrain, unit scenes)] as render
     [Replay recording] as rec
   }
   package "Server (rts_server)" {
     [Coordinator\nroom and clock] as coord
     [Authoritative Sim] as ssim
   }
   input --> pending : Command
   pending --> session
   session --> coord : Input { tick + 2 }
   coord --> ssim : Turn every 50 ms
   ssim --> coord : state hash
   coord --> session : Turn + hash
   session --> csim : apply Turn
   csim --> session : hash to verify
   session --> rec : Turn + hash
   csim --> render : read positions
   @enduml

1. Input systems never change the simulation. They append
   ``rts_base::Command`` values to the match's pending list.
2. The session sends pending commands to the server for a future tick.
3. The server's coordinator collects every player's input into one
   ``Turn`` per tick, applies it to the authoritative ``Sim``, and broadcasts
   the turn with the resulting state hash.
4. Each client applies the same turn to its replica, checks the hash, and
   records the turn.
5. Rendering reads the replica's unit positions and smooths movement between
   ticks.

Read on
-------

* :doc:`determinism` — the rules that keep every peer identical.
* :doc:`simulation` and :doc:`movement` — the core.
* :doc:`client`, :doc:`rendering`, :doc:`input`, :doc:`interface`, and
  :doc:`sessions` — the Bevy client.
* :doc:`server` and :doc:`networking` — multiplayer.
* :doc:`assets` and :doc:`platforms` — content loading and platform
  differences.
