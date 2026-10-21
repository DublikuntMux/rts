Change units, terrain, and maps
===============================

Game content lives in two RON files: a *rules* file with terrain and unit
types, and a *map* file with terrain tiles, player starts, and starting units.
A server can run modified copies of both, and clients receive them when they
join, so content mods need no client rebuild.

What you can change
-------------------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Content
     - Where
   * - Tile capacity
     - ``tile_capacity`` in the rules file.
   * - Terrain types: symbol, speed, color, texture
     - ``terrain`` in the rules file.
   * - Unit types: size, health, radius, speed
     - ``units`` in the rules file.
   * - Map name, size, and terrain layout
     - ``name`` and ``tiles`` in the map file.
   * - Player count and start positions
     - ``players`` in the map file.
   * - Starting units
     - ``units`` in the map file.

Executable game code, textures, fonts, and the interface are part of the
client and are not downloaded. Lua scripting for missions and triggers is
planned; see :doc:`../project/scripting`.

Create a mod
------------

1. Copy the bundled files:

   .. code-block:: sh

      mkdir -p mods/my-mod
      cp assets/data/core.rules.ron mods/my-mod/
      cp assets/maps/proving_grounds.map.ron mods/my-mod/battle.map.ron

2. Edit them. For example, make soldiers faster and add a road terrain:

   .. code-block:: rust

      terrain: [
          // ...existing types...
          (id: "road", symbol: '=', speed_modifier: 2.0, color: "#9a9a90"),
      ],
      units: [
          (id: "worker", size: 1, max_health: 40, radius: 0.3, speed: 2.5),
          (id: "soldier", size: 2, max_health: 80, radius: 0.4, speed: 4.0),
      ],

   Then use ``=`` in the map's ``tiles`` rows.

3. Start the server with your files:

   .. code-block:: sh

      cargo run --release -p rts_server --bin rts-server -- 0.0.0.0:9001 \
        --rules mods/my-mod/core.rules.ron \
        --map mods/my-mod/battle.map.ron

Each option is independent; an omitted file uses the bundled default. The
server validates both files at startup and exits with an error if they are
invalid, before it opens the port. Restart the server after editing.

Textures in mods
----------------

Clients load textures from their own asset pack. A mod can reuse a bundled
path such as ``Some("textures/terrain/grass.png")``, or use ``texture: None``
to draw the terrain with its ``color``. A path that the client does not have
fails to load and leaves every terrain drawn with flat colors.

Test a map locally
------------------

Solo play always uses the bundled files in ``assets/``. To try a map in solo
play, edit ``assets/maps/proving_grounds.map.ron`` or
``assets/data/core.rules.ron`` and run ``cargo run``; the asset pack rebuilds
automatically. To try a mod without touching the bundled files, run a local
server with the mod and create a room. A map with a single player start makes
a one-player room that the host can start immediately.

Validation errors
-----------------

The server and client reject content that could make peers disagree or
crash. Common errors:

.. list-table::
   :header-rows: 1
   :widths: 45 55

   * - Error
     - Fix
   * - ``unknown terrain symbol `x` at tile (3, 7)``
     - Define the symbol in the rules or fix the map row.
   * - ``row 5 has 47 tiles; expected 48``
     - Every row must be the same length.
   * - ``unit 3 exceeds tile capacity at (4, 8)``
     - Too many starting units share one tile. Spread them out.
   * - ``unit 0 is outside the map or on blocked terrain``
     - Move the unit onto terrain with a positive speed modifier.
   * - ``player 1 starts outside the map or on blocked terrain``
     - Move the start position.
   * - ``unit `soldier` size must be positive and at most tile_capacity``
     - Lower the size or raise ``tile_capacity``.
   * - ``positive speed_modifier is below Q10 resolution``
     - Use ``0.0`` to block, or at least ``0.0009765625`` (1/1024).

:doc:`../reference/rules-format` and :doc:`../reference/map-format` list every
field and rule.

Share a mod with players
------------------------

Players only need the server address. When they create or join a room, the
server sends its minified, compressed map and rules in one binary message; each
client validates the files and acknowledges a checksum before the host can
start. Replays of the match embed the same files, so they play back anywhere.
