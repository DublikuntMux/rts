Coordinates and units
=====================

The simulation and the client use different coordinate systems. The
simulation needs integers that every platform computes identically; Bevy
needs floating-point world space with y pointing up. Conversions happen only
in the client and only in one direction for rendering.

Simulation space
----------------

* Positions are ``glam::IVec2`` values in integer **world units**.
* One tile is ``TILE = 1024`` world units on each side.
* x grows to the right and **y grows down**. Tile ``(0, 0)`` is the top-left
  corner of the map.
* Map files list rows from top to bottom, so a map file looks like the map.
* ``tile_of(pos)`` returns the tile that contains a position, using Euclidean
  division so negative positions round toward negative infinity.
* A tile's center is ``tile × 1024 + 512``.

.. kroki::
   :type: plantuml
   :caption: Simulation coordinates for the top-left corner of a map. Unit centers usually sit at tile centers but may be anywhere.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.1
   {#
     . | x = 0 … 1023 | x = 1024 … 2047 | x = 2048 … 3071
     y = 0 … 1023 | tile (0, 0), center (512, 512) | tile (1, 0), center (1536, 512) | tile (2, 0)
     y = 1024 … 2047 | tile (0, 1), center (512, 1536) | tile (1, 1) | tile (2, 1)
     y = 2048 … 3071 | tile (0, 2) | tile (1, 2) | tile (2, 2), center (2560, 2560)
   }
   @endsalt

World units give sub-tile precision without fractions: a unit moving at
2.5 tiles per second advances 128 world units per tick. Squared distances use
``i64`` and lengths use integer square roots. Maps are at most 1024 tiles per
side, so every position fits comfortably in ``i32``.

Data-file units
---------------

Data files use tiles because they are easier to write. Loaders convert them
to world units exactly once (see :ref:`fixed-point-data`):

.. list-table::
   :header-rows: 1
   :widths: 35 30 35

   * - Written
     - Meaning
     - Stored
   * - ``pos: (4.5, 8.0)``
     - Column 4.5, row 8.0
     - ``IVec2(4608, 8192)``
   * - ``radius: 0.3``
     - 0.3 tiles
     - 307 world units
   * - ``speed: 2.5``
     - 2.5 tiles per second
     - 2560 world units per second
   * - ``speed_modifier: 0.5``
     - Half speed
     - 512 (Q10)

Client space
------------

The client draws in Bevy's 2D world space, measured in pixels with y pointing
**up**:

* One tile is ``TILE_PX = 128`` pixels at camera scale 1.
* ``map::to_world(pos)`` converts a simulation position by scaling with
  ``128 / 1024`` and negating y.
* The map spans from ``(0, 0)`` at its top-left corner to
  ``(width_px, -height_px)`` at its bottom-right corner.
* ``map::to_world_distance`` converts a length, such as a unit radius.

.. kroki::
   :type: plantuml
   :caption: Matching points in both spaces for a map w tiles wide and h tiles tall.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.1
   {#
     <b>Point | <b>Simulation (world units, y down) | <b>Bevy world (pixels, y up)
     Map top-left corner | (0, 0) | (0, 0)
     One tile right | (1024, 0) | (128, 0)
     One tile down | (0, 1024) | (0, -128)
     Center of tile (0, 0) | (512, 512) | (64, -64)
     Map bottom-right corner | (w × 1024, h × 1024) | (w × 128, -h × 128)
   }
   @endsalt

Screen to simulation
--------------------

Input works in the other direction. Cursor and touch positions are logical
pixels from the window's top-left corner. The input system converts them in
two steps:

1. **Screen to Bevy world:** subtract the window center, flip y, multiply by
   the camera's orthographic scale, and add the camera position. The
   conversion uses the camera transform from the current frame, so a pan
   and a tap in the same frame agree.
2. **Bevy world to simulation:** flip y again, multiply by ``1024 / 128``,
   and round to the nearest integer.

The rounded target is the only float-to-integer conversion that feeds the
simulation. It happens once, on the issuing client, and the integer travels in
the ``Command``, so every peer sees the same target.

Camera space
------------

The camera uses Bevy's default ``ScalingMode::WindowSize``: one world pixel
spans one logical screen pixel at scale 1. Larger scales zoom out. The
terrain renderer measures detail as *screen pixels per tile*, which is
``128 / scale``; see :doc:`rendering`.
