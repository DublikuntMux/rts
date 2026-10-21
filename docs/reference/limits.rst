Limits and constants
====================

This page collects the fixed numbers that shape the game. Each entry names the
constant and the file that owns it. Change a value there, then update this
page.

Simulation
----------

.. list-table::
   :header-rows: 1
   :widths: 32 18 50

   * - Constant
     - Value
     - Meaning
   * - ``TICK_RATE``
     - 20
     - Simulation ticks per second (``crates/base/src/lib.rs``).
   * - ``TILE``
     - 1024
     - World units per tile (``coords.rs``).
   * - ``SPEED_SCALE``
     - 1024
     - Q10 fixed-point value of a terrain speed modifier of 1.0
       (``rules.rs``).
   * - ``TRAVEL_SCALE``
     - 2\ :sup:`32`
     - Movement effort per world unit at normal speed (``rules.rs``).
   * - ``MAX_PLAYERS``
     - 8
     - Players per match (``lib.rs``).
   * - ``MAX_MAP_SIDE``
     - 1024
     - Tiles per map side (``map.rs``).
   * - Terrain types
     - 256
     - ``TerrainId`` is a ``u8``.
   * - Unit types
     - 65536
     - ``UnitKind`` is a ``u16``.
   * - ``tile_capacity``, ``size``
     - 65535
     - Both are ``u16``.
   * - ``INPUT_DELAY_TICKS``
     - 2
     - Ticks between issuing and applying a command (``command.rs``).

Protocol
--------

Constants are in ``crates/net/src/lib.rs`` unless noted.

.. list-table::
   :header-rows: 1
   :widths: 32 18 50

   * - Constant
     - Value
     - Meaning
   * - ``PROTOCOL_VERSION``
     - 4
     - Wire, setup, and replay version.
   * - ``MAX_MESSAGE_BYTES``
     - 64 KiB
     - Client messages and small server messages.
   * - ``MAX_TURN_MESSAGE_BYTES``
     - 576 KiB
     - ``Turn`` messages: 64 KiB for each of up to eight players plus
       framing.
   * - ``MAX_SERVER_MESSAGE_BYTES``
     - 16 MiB + 33 B
     - Binary setup frame (``setup/wire.rs``).
   * - ``MAX_SETUP_BYTES``
     - 4 MiB
     - Each rules or map document, original or minified (``setup/mod.rs``).
   * - Compressed document
     - 8 MiB
     - Each zlib stream in the setup frame.
   * - ``MAX_COMMANDS_PER_TURN``
     - 64
     - Commands in one player's batch for a tick.
   * - ``MAX_UNITS_PER_COMMAND``
     - 1024
     - Unit IDs in one command; no duplicates within a command.
   * - ``MAX_UNIT_SELECTIONS_PER_TURN``
     - 1024
     - Unit IDs across one player's batch.
   * - ``MAX_ORDERS_PER_UNIT_PER_TURN``
     - 4
     - Commands naming the same unit in one batch, such as Move then Stop.
   * - RON recursion
     - 64 levels
     - Wire messages and replays. Embedded rules and maps use RON's
       default of 128.
   * - ``MAX_REPLAY_BYTES``
     - 32 MiB
     - Replay file size (``replay.rs``).
   * - ``MAX_REPLAY_TURNS``
     - 1,000,000
     - Turns per replay.

Server
------

Constants are in ``crates/server/src/coordinator.rs``. Configurable defaults
are listed in :doc:`server-cli`.

.. list-table::
   :header-rows: 1
   :widths: 32 18 50

   * - Constant
     - Value
     - Meaning
   * - ``MAX_FUTURE_TICKS``
     - 8
     - How far ahead of the server's tick input may be scheduled.
   * - ``MAX_CATCH_UP_TURNS``
     - 4
     - Turns produced per clock check after a stall, before the deadline
       resets.
   * - Room code
     - 6 characters
     - ``A``–``Z`` and ``0``–``9``.
   * - Inbound WebSocket frame
     - 64 KiB
     - Larger client messages close the socket.

Client
------

.. list-table::
   :header-rows: 1
   :widths: 32 18 50

   * - Value
     - Size
     - Meaning
   * - Buffered server turns
     - 128
     - More unapplied turns stop the session (``session.rs``).
   * - Turns applied per fixed update
     - 8
     - Catch-up work after a slow frame.
   * - Socket events per frame
     - 128
     - Incoming events drained in ``PreUpdate``; the channel holds 128.
   * - Connection timeout
     - 10 s
     - Time for the socket to open.
   * - Waiting message
     - 0.25 s
     - Missing-turn time before *Waiting for the server…*.
   * - Stall timeout
     - 15 s
     - Missing-turn time before the client stops.
   * - Server address
     - 256 characters
     - Lobby field length; the session accepts up to 2048.
   * - ``TILE_PX``
     - 128
     - Screen pixels per tile at camera scale 1 (``map.rs``).
   * - ``HUD_HEIGHT``
     - 88 px
     - Bottom command bar (``hud.rs``).
   * - ``SESSION_BAR_HEIGHT``
     - 56 px
     - Top session bar (``lobby.rs``).
   * - Touch target
     - 48 px
     - Smallest button side and touch hit diameter.
