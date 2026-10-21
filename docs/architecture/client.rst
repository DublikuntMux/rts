Client application
==================

``rts_client`` (``crates/client``) is the game. It is a Bevy application that
loads content, shows the lobby, runs match sessions, draws the simulation, and
turns input into commands. The binary ``rts`` and the Android library both
call ``rts_client::run()``.

The client never changes simulation state directly. It reads ``Sim`` to draw
and sends **commands** that the simulation applies at a tick boundary.

Plugins
-------

``build_app`` adds the asset pack, Bevy's ``DefaultPlugins`` with a window
titled *RTS* that fits its canvas to the browser page, and the game plugins:

.. list-table::
   :header-rows: 1
   :widths: 22 18 60

   * - Plugin
     - Module
     - Responsibility
   * - ``DataPlugin``
     - ``rts_assets``
     - Loaders for rules and map files.
   * - ``GamePlugin``
     - ``game.rs``
     - ``AppState``, the 20 Hz fixed clock, loading the bundled setup, and
       the ``Match`` resource.
   * - ``MapPlugin``
     - ``map.rs``
     - Terrain tilemap, texture array, detail switching, map bounds, and
       coordinate conversion.
   * - ``UnitsPlugin``
     - ``units.rs``
     - Unit scenes synchronized from the simulation, with interpolation.
   * - ``RtsCameraPlugin``
     - ``camera.rs``
     - Orthographic camera: pan, zoom, pinch, and clamping.
   * - ``SelectionPlugin``
     - ``input.rs``
     - Selection, move and stop orders, and selection gizmos.
   * - ``HudPlugin``
     - ``hud.rs``
     - Bottom command bar.
   * - ``SessionPlugin``
     - ``session.rs``
     - Solo, online, and playback sessions; networking; replay recording.
   * - ``LobbyPlugin``
     - ``lobby.rs``
     - Lobby screen, touch keypad, and top session bar.

``replay_io.rs`` saves and loads replay files and ``font.rs`` holds the
``GameFont`` resource.

Application states
------------------

.. kroki::
   :type: plantuml
   :caption: AppState transitions.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Loading
   Loading : rules and map loading
   Loading --> Menu : bundled setup is valid
   Menu : lobby
   Menu --> InGame : Play solo, Started from server,\nReplay last, loaded replay
   InGame : terrain, units, HUD, session bar
   InGame --> Menu : Leave
   @enduml

Each screen spawns its entities on ``OnEnter`` and tags them with
``DespawnOnExit`` for the state, so leaving a state removes everything it
created. ``OnExit(InGame)`` also removes the ``Match`` resource, clears the
selection, terrain resources, unit caches, and interpolation snapshots.

A failed or stopped online session stays in ``InGame`` with orders disabled
and a message in the session bar, so the player can still save the replay
before choosing **Leave**.

Schedules
---------

.. kroki::
   :type: plantuml
   :caption: Where client systems run in one frame.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   :PreUpdate\npoll_network: drain socket events,\nhandle server messages;
   :Update\nhandle_actions, finish_replay_load\ncontrol_camera (CameraControl)\nsync_units (SyncUnits)\ncontrol_units after both\nlobby and HUD updates;
   while (fixed clock owes a 50 ms step?) is (yes)
     :FixedUpdate\nadvance_session: apply up to 8 turns;
   endwhile (no)
   :PostUpdate\nupdate_terrain_detail\ndraw_selection gizmos;
   stop
   @enduml

``Time<Fixed>`` runs at ``TICK_RATE`` (20 Hz). Bevy runs ``FixedUpdate`` zero
or more times per frame to keep up with real time; between ticks, unit
transforms are interpolated with the fixed clock's overstep fraction.

Selection runs after camera control, so a tap and a pan in the same frame use
the same camera transform, and after unit synchronization, so hit tests use
this frame's displayed positions.

The match resource
------------------

.. code-block:: rust

   pub struct Match {
       pub sim: Sim,
       pub local_player: PlayerId,
       pub accepts_orders: bool,
       pub pending: Vec<Command>,
   }

* ``sim`` is the local replica. Only the session advances it.
* ``local_player`` is the slot this client controls (0 for solo and
  playback).
* ``accepts_orders`` is false during playback and after a session stops.
* ``pending`` collects commands for the next turn. ``queue_command`` refuses
  a command when orders are disabled, when 64 commands are already pending,
  or when adding it would break the per-turn command limits. Commands are
  never applied optimistically.

``MatchData`` holds the bundled ``MatchSetup`` for solo play.
``PreviousPositions`` holds unit positions from just before the latest tick,
for interpolation only.

Bevy features
-------------

Bevy's default features are disabled in the workspace ``Cargo.toml``. The
enabled features are:

.. list-table::
   :header-rows: 1
   :widths: 35 65

   * - Features
     - Used for
   * - ``std``, ``async_executor``, ``multi_threaded``
     - Task pools and asset loading.
   * - ``bevy_asset``, ``png``
     - Asset server and terrain textures.
   * - ``bevy_scene``
     - Scene composition for unit visuals.
   * - ``bevy_log``, ``bevy_state``
     - Logging and ``AppState``.
   * - ``bevy_winit``, ``x11``, ``wayland``, ``webgl2``
     - Windows on desktop and the browser canvas.
   * - ``bevy_sprite_render``
     - 2D meshes and the terrain tilemap.
   * - ``bevy_ui_render``, ``bevy_ui_widgets``, ``ui_picking``
     - Lobby, HUD, buttons, text input, and pointer picking.
   * - ``bevy_gizmos_render``
     - Selection rings, box, and order markers.
   * - ``keyboard``, ``mouse``, ``touch``
     - Input.

``crates/android`` adds ``android-game-activity``. Bevy's ``2d`` and ``ui``
convenience presets are avoided because they enable unused modules. The
client does not use PBR or glTF, animation, world serialization,
post-processing or extra anti-aliasing, HDR or compressed-texture codecs,
Bevy's built-in font, audio, gamepads, custom cursors, system diagnostics, or
reflection auto-registration. Shared renderer dependencies such as
``bevy_light`` (through ``bevy_core_pipeline``) still compile and cannot be
removed with features. Enable a new feature only when game code uses it.

.. _client-scenes:

Scenes
------

The client uses Bevy 0.20 scene composition (``bsn!`` and functions returning
``impl Scene``) for reusable presentation objects. ``DefaultPlugins`` installs
``ScenePlugin``; do not register it twice.

``units.rs::unit_scene`` is the first scene. It spawns one unit visual with
its stable ``UnitId``, shared mesh and material handles, its interpolated
transform, and ``DespawnOnExit(AppState::InGame)``. See :doc:`rendering`.

Rules for scenes:

* Simulation state stays in ``rts_base``; scenes only present it. Synchronize
  scene roots from stable simulation IDs and update visual children from
  simulation data.
* Scene observers for UI actions submit ordinary commands; they never mutate
  the simulation. Movement, combat, ownership, hashes, networking, and replays
  stay independent of Bevy entities.
* Keep ownership and cleanup on the scene root so future ``Children`` are
  removed with it. Keep ``UnitVisual`` and ``Transform`` on the root for
  selection and interpolation.
* Use ``Commands::spawn_scene`` when the scene's assets are already loaded,
  as with the cached unit meshes and materials. Use ``queue_spawn_scene`` when
  a scene must wait for assets, and account for pending scenes in
  synchronization and match cleanup so they cannot outlive their owner.
* Author scenes in Rust. Bevy 0.20 has no built-in ``.bsn`` asset loader;
  rules, maps, and replays stay in RON. Scenes need neither world
  serialization nor reflection auto-registration.
* Prefer scenes for reusable hierarchies. Simple entities and procedural
  terrain can keep ordinary spawning.

Planned uses: health bars as unit scene children (M4), building visuals,
construction indicators, and command-card widgets (M5), and decorative groups
in the editor (M7).
