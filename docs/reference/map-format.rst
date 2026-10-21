Map file format
===============

A map file (``*.map.ron``) describes the terrain grid, player starts, and
starting units of one match. Bundled maps live under ``assets/maps/``:
``proving_grounds.map.ron`` has two players on 48×32 tiles, and
``crossroads.map.ron`` has four on 40×40 tiles. Valid ``*.map.ron`` files in
that folder appear in **Singleplayer → New game** automatically.
A map is parsed by
``rts_base::MapData::from_ron`` and validated against a rules file by
``Sim::new`` (``crates/base/src``). The map editor (M7) will write the same
format.

Example
-------

.. code-block:: rust

   // Positions are in tiles; (0, 0) is the top-left corner.
   (
       name: "Small Valley",
       players: [
           (start: (1.5, 1.5)),
           (start: (8.5, 4.5)),
       ],
       tiles: [
           "..........",
           "...##.....",
           "...##..~~.",
           "......~~~.",
           "..........",
       ],
       units: [
           (kind: "worker", owner: 0, pos: (1.5, 2.5)),
           (kind: "soldier", owner: 1, pos: (7.5, 4.5)),
       ],
   )

Fields
------

.. list-table::
   :header-rows: 1
   :widths: 18 22 60

   * - Field
     - Type
     - Meaning
   * - ``name``
     - string
     - Display name.
   * - ``players``
     - list of ``(start: (x, y))``
     - Player slots, 1 to 8. The list order defines player IDs: the first
       entry is player 0 (shown as player 1). The count sets the online room
       size.
   * - ``tiles``
     - list of strings
     - Terrain rows from top to bottom. Each character is a terrain
       ``symbol`` from the rules.
   * - ``units``
     - list, optional
     - Units present at the start. Defaults to empty.

Each ``units`` entry has:

.. list-table::
   :header-rows: 1
   :widths: 18 22 60

   * - Field
     - Type
     - Meaning
   * - ``kind``
     - string
     - Unit type ``id`` from the rules.
   * - ``owner``
     - integer
     - Index into ``players``.
   * - ``pos``
     - ``(x, y)`` tiles
     - Center position. Decimals allowed.

Coordinates
-----------

Positions are written in tiles and converted to world units (1024 per tile)
on load. x grows to the right and y grows *down*: tile ``(0, 0)`` is the
top-left corner, and the first ``tiles`` row is the top of the map, so the file
looks like the map. ``(1.5, 2.5)`` is the center of the tile in column 1,
row 2. See :doc:`../architecture/coordinates`.

Validation
----------

``MapData::from_ron`` checks only the RON structure. ``Sim::new`` then
rejects:

* A map with no rows, an empty first row, or a side longer than 1024 tiles.
* Rows of different lengths, counted in characters.
* An unknown terrain symbol.
* No player starts, or more than eight.
* A player start outside the map or on blocked terrain.
* A unit of an unknown kind or for a missing player.
* A unit outside the map or on blocked terrain.
* Units whose total size exceeds ``tile_capacity`` in any tile. Capacity counts
  the tile that contains each unit's center.

Units spawn in file order and receive IDs 0, 1, 2, and so on, with full
health. That order also sets the default selection and contention order, so
reordering ``units`` changes the match.

Over the network
----------------

The server sends the map text itself, not a reserialized copy. It removes
comments and unnecessary whitespace without changing any literal, so decimal
positions are parsed identically on every client. The setup checksum covers
the minified text, so comments and formatting never affect readiness. See
:doc:`setup-frame`.

Planned fields
--------------

M6 adds named regions and a ``script`` path for mission scripts. See
:doc:`../project/scripting`.
