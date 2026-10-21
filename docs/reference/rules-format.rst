Rules file format
=================

A rules file (``*.rules.ron``) defines terrain and unit types shared by every
match. The bundled file is ``assets/data/core.rules.ron``. It is parsed by
``rts_base::Rules::from_ron`` (``crates/base/src/rules.rs``) and written in
`RON <https://github.com/ron-rs/ron>`_.

Example
-------

.. code-block:: rust

   // Core game data. Distances are in tiles; speeds are in tiles per second.
   (
       tile_capacity: 4,
       terrain: [
           (id: "grass", symbol: '.', speed_modifier: 1.0, color: "#4a7a3e",
            texture: Some("textures/terrain/grass.png")),
           (id: "water", symbol: '~', speed_modifier: 0.0, color: "#2e5d8c",
            texture: Some("textures/terrain/water.png")),
       ],
       units: [
           (id: "worker", size: 1, max_health: 40, radius: 0.3, speed: 2.5),
           (id: "soldier", size: 2, max_health: 80, radius: 0.4, speed: 3.0),
       ],
   )

Top level
---------

.. list-table::
   :header-rows: 1
   :widths: 22 18 60

   * - Field
     - Type
     - Meaning
   * - ``tile_capacity``
     - integer
     - Required. Total unit ``size`` one tile can hold, shared by all
       players. 1 to 65535.
   * - ``terrain``
     - list
     - Required. At least 1 and at most 256 terrain types. A type's position
       in the list is its ``TerrainId``.
   * - ``units``
     - list
     - Required. At most 65536 unit types. A type's position is its
       ``UnitKind``.

Unknown fields are errors in every structure, so a typo never silently falls
back to a default.

Terrain types
-------------

.. list-table::
   :header-rows: 1
   :widths: 22 18 60

   * - Field
     - Type
     - Meaning
   * - ``id``
     - string
     - Unique name.
   * - ``symbol``
     - character
     - Unique character that stands for this terrain in map ``tiles``.
   * - ``speed_modifier``
     - decimal
     - Required movement multiplier. ``0.0`` blocks movement, ``1.0`` is
       normal speed, ``0.5`` is half, ``2.0`` is double.
   * - ``color``
     - string
     - ``#rrggbb`` color for distant zoom and the future minimap, and for
       terrain without a texture. Client only.
   * - ``texture``
     - optional string
     - ``Some("path")`` to a square image in the asset pack, or ``None``
       (the default). Client only.

``speed_modifier`` must be finite and nonnegative. It is rounded to Q10 fixed
point (1024 represents 1.0), so the smallest positive value is
``0.0009765625`` and the largest is ``2097151.9990234375``. A positive value
that rounds to zero is rejected rather than silently blocking movement. See
:ref:`fixed-point-data`.

All textures must have the same square size. The bundled textures are
128×128 PNG images in ``assets/textures/terrain/``. The simulation never reads
``color`` or ``texture``; they do not affect state hashes.

Unit types
----------

.. list-table::
   :header-rows: 1
   :widths: 22 18 60

   * - Field
     - Type
     - Meaning
   * - ``id``
     - string
     - Unique name, used by map ``units`` entries.
   * - ``size``
     - integer
     - Tile capacity the unit consumes. 1 to ``tile_capacity``.
   * - ``max_health``
     - integer
     - Positive starting health. Combat arrives in M4.
   * - ``radius``
     - decimal tiles
     - Positive collision and drawing radius.
   * - ``speed``
     - decimal tiles/second
     - Positive speed on terrain with modifier 1.0.

``radius`` and ``speed`` are converted to world units (1024 per tile) when the
file loads: ``speed: 2.5`` becomes 2560 world units per second. Values that
round to zero or exceed the 32-bit range are rejected.

Validation
----------

``Rules::from_ron`` rejects:

* A missing field, an unknown field, or a value of the wrong type.
* ``tile_capacity`` of 0, or a capacity or size outside 0..65535.
* No terrain, more than 256 terrain types, or more than 65536 unit types.
* Duplicate terrain IDs, terrain symbols, or unit IDs.
* A unit with ``size`` of 0 or larger than ``tile_capacity``.
* A unit with nonpositive ``max_health``, ``radius``, or ``speed``.
* An invalid ``speed_modifier`` as described above.

The legacy ``walkable`` field is no longer accepted; use ``speed_modifier``.

A malformed ``color`` is not a load error. The client logs a warning and draws
that terrain magenta.

Bundled content
---------------

.. list-table::
   :header-rows: 1
   :widths: 15 10 15 60

   * - Terrain
     - Symbol
     - Modifier
     - Color
   * - ``grass``
     - ``.``
     - 1.0
     - ``#4a7a3e``
   * - ``dirt``
     - ``,``
     - 1.0
     - ``#8a7350``
   * - ``sand``
     - ``:``
     - 1.0
     - ``#c9b27a``
   * - ``water``
     - ``~``
     - 0.0
     - ``#2e5d8c``
   * - ``rock``
     - ``#``
     - 0.0
     - ``#5a5852``

.. list-table::
   :header-rows: 1
   :widths: 20 10 20 15 35

   * - Unit
     - Size
     - Max health
     - Radius
     - Speed
   * - ``worker``
     - 1
     - 40
     - 0.3
     - 2.5 tiles/s
   * - ``soldier``
     - 2
     - 80
     - 0.4
     - 3.0 tiles/s

With ``tile_capacity: 4``, a tile holds four workers, two soldiers, or two
workers and one soldier.
