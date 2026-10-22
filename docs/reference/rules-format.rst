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
           (id: "worker", sprite: "textures/units/worker.png",
            size: 1, max_health: 40, radius: 0.3, speed: 2.5),
           (id: "soldier", sprite: "textures/units/soldier.png",
            size: 2, max_health: 80, radius: 0.4, speed: 3.0),
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
   * - ``sprite``
     - string
     - Required image path in the asset pack. Empty and whitespace-only
       paths are rejected. Client only.
   * - ``size``
     - integer
     - Tile capacity the unit consumes. 1 to ``tile_capacity``.
   * - ``max_health``
     - integer
     - Positive starting health. Combat arrives in M4.
   * - ``radius``
     - decimal tiles
     - Positive nominal drawing radius. Crowded visual groups stay near this
       size and may spill past their tile; see :doc:`../architecture/rendering`.
   * - ``visual_bounds``
     - optional bounds
     - ``Some((offset: (x, y), size: (width, height)))`` in tiles. Used for
       visual packing, hit tests, box selection, and selection outlines.
       Omitted or ``None`` uses a centered square of diameter ``2 × radius``
       for packing and preserves circular selection.
   * - ``speed``
     - decimal tiles/second
     - Positive speed on terrain with modifier 1.0.

``radius`` and ``speed`` are converted to world units (1024 per tile) when the
file loads: ``speed: 2.5`` becomes 2560 world units per second. Values that
round to zero or exceed the 32-bit range are rejected.

The client draws ``sprite`` at the size and offset given by ``visual_bounds``.
When bounds are absent or ``None``, the image fills a centered square with
width and height ``2 × radius``. Crowded visual groups scale the sprite and
its bounds together, but only as far as a group area of one and a half tiles
requires, so stacked units keep most of their unshared size. The simulation
never reads ``sprite``; changing only its path does not affect movement or
state hashes.

Custom sprite bounds
--------------------

For a sprite whose visible body is wider than it is tall and centered above
its root, add this field to its unit entry:

.. code-block:: text

   visual_bounds: Some((
       offset: (0.0, -0.125),
       size: (0.75, 0.5),
   )),

``offset`` is the center of the bounding box relative to the unit's visual
root. Positive x is right and positive y is down, matching map coordinates.
``size`` is the full width and height, not half-extents. Both pairs use tiles
and are converted to integer world units at load time. An offset may be signed;
each size component must round to a positive world-unit value. Both fields
are required when bounds are supplied, and unknown nested fields are errors.

Bounds describe the sprite footprint and displayed dimensions; they do not
crop artwork. Images are stretched to fill these dimensions.
The client scales the footprint and its offset with the sprite, placing the
bounds center in its assigned tile slot. Rectangular groups choose the grid
orientation that best fits the group area. See :doc:`../architecture/rendering`.
These fields affect presentation only: paths, tile capacity, unit positions,
and simulation hashes are unchanged. They travel with the rules text through
server setup and replays.

Validation
----------

``Rules::from_ron`` rejects:

* A missing field, an unknown field, or a value of the wrong type.
* ``tile_capacity`` of 0, or a capacity or size outside 0..65535.
* No terrain, more than 256 terrain types, or more than 65536 unit types.
* Duplicate terrain IDs, terrain symbols, or unit IDs.
* A unit with ``size`` of 0 or larger than ``tile_capacity``.
* A unit with nonpositive ``max_health``, ``radius``, or ``speed``.
* A missing ``sprite`` path, or a path that is empty or contains only whitespace.
* Nonfinite or out-of-range ``visual_bounds`` components, or a size component
  that rounds to zero or is negative.
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

The bundled sprites are ``textures/units/worker.png`` and
``textures/units/soldier.png``, relative to the asset pack root.
