Rendering
=========

The client draws two layers: a terrain tilemap and unit visuals, plus gizmos
for selection. Everything is derived from the simulation each frame; nothing
drawn feeds back into it. Code lives in ``crates/client/src/map.rs`` and
``units/mod.rs``, ``units/layout.rs``, and ``units/motion.rs``.

Local presentation settings live in ``assets/data/client.graphics.ron`` and
are installed as the ``Graphics`` resource before the main menu opens. Player
colors, selection and order markers, and terrain detail thresholds come from
this file. See :doc:`../reference/graphics-format` for fields and validation.

Terrain
-------

When a match starts, ``spawn_terrain`` draws the whole map as **one**
``TilemapChunk``: the chunk size is the map size, each tile shows the layer
of its ``TerrainId``, and tiles are 128 pixels. A single chunk is one draw
call. Chunked rendering for large maps is planned for M10.

.. kroki::
   :type: plantuml
   :caption: Building the two terrain tilesets.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   :OnEnter(InGame): spawn_terrain;
   fork
     :build color palette\n1×1 layer per TerrainId\nfrom each type's color;
     :use the palette as the tileset now;
   fork again
     :start loading each texture\n(CPU-side pixels kept);
     :finish_terrain_textures waits\nuntil all have loaded;
     if (any failed or mismatched?) then (yes)
       :log the error,\nkeep the palette only;
     else (no)
       :assemble one texture array\nin TerrainId order, filling\nuntextured types with their color;
     endif
   end fork
   :update_terrain_detail picks a tileset\nfrom the zoom level every frame;
   stop
   @enduml

Two tilesets exist side by side in ``TerrainTilesets``:

* **Colors**: one 1×1 layer per terrain type, from the rules' ``color``. It is
  available immediately and is kept for distant zoom and the future minimap.
* **Detailed**: the textures assembled into one ``Rgba8UnormSrgb`` array
  texture, one layer per ``TerrainId``. All textures must be square and the
  same size. Types without a texture get a layer filled with their color.

Colors go into the texture rather than ``TileData::color`` tints because the
tilemap shader multiplies tint sRGB bytes as if they were linear, which washes
colors out. Array textures need at least two layers, so a single-terrain rules
file is padded.

Detail by zoom
--------------

``update_zoom_detail`` measures screen pixels per tile (``128 / scale``)
and updates one shared ``ZoomDetail`` state after camera control. Terrain and
units consume that state, so their zoom transitions occur together.
``update_terrain_detail`` switches the chunk's material between tilesets. The
following thresholds are the bundled defaults, configurable through
``terrain_detail`` in the graphics file:

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Currently showing
     - Switch when
   * - Textures
     - Fewer than 40 pixels per tile: switch to colors.
   * - Colors
     - At least 48 pixels per tile: switch to textures.

The gap keeps small wheel or pinch changes near the threshold from flickering
between materials.

Units
-----

``sync_units`` runs every frame in ``InGame``, after camera control. It
interpolates simulation travel, eases packing offsets and scale, updates sprite
detail, spawns missing visuals, and despawns removed units.

Every unit type requires a ``sprite`` image path in its rules. The client loads
it through ``AssetServer`` and shares its handle per ``UnitKind``. Missing
images produce Bevy asset loading errors. Sprites are tinted with their owner's
player color and drawn at z = 1, above terrain. Player slots wrap through the
configured palette.

``unit_scene`` spawns ``UnitVisual``, ``Sprite``, ``Anchor``, detail state,
``Transform``, and ``DespawnOnExit`` on one root. The sprite template resolves
the image handle. Health bars (M4) will be children of this root. See
:ref:`client-scenes` for scene rules.

``UnitVisuals`` stores entities, shared images, and persistent movement state.
These presentation caches never affect the deterministic core. On match exit,
the caches reset and roots despawn through ``DespawnOnExit``.

Unit detail by zoom
-------------------

Units use the same ``ZoomDetail`` state and ``terrain_detail`` thresholds as
the map: rectangles below 40 screen pixels per tile, sprites again at 48. Unit
size and crowded-group scaling do not affect the decision. All units, including
new visuals, switch together; the shared hysteresis prevents flicker. Terrain
textures still need to finish loading before they appear.

Both unit modes share the same size, anchor, transform, and selection target.
Rectangles use Bevy's white default sprite image; the required image handle
remains cached for zooming back in. There are no separate unit detail settings.

Tile groups
-----------

``units/layout.rs`` uses the optional ``visual_bounds`` from the rules: a center
offset and full width and height. When omitted, it derives a centered square
from the unit's diameter. The sprite is sized to this footprint. See
:doc:`../reference/rules-format` for the RON syntax.

Stopped units sharing a tile, including different owners, occupy a compact grid in
``UnitId`` order. Cells use the group's maximum width and height; the grid
orientation minimizes its longest side. Cells have a nominal four-world-pixel
gap (scaled with the group) and four world pixels of border padding. The last
row is centered.

A packed group may fill one and a half tiles across. Crowded sprites therefore
grow past the tile edge instead of shrinking into it: a full tile of workers
keeps its unshared size, and two soldiers still draw over 85% of theirs. The
group is anchored at its members' mean bounds center — the point the group was
ordered to — and is never pulled toward the tile center, so units stop where
the player clicked even at a tile edge, and a lone unit stands exactly on its
ordered point. A group near an edge may overlap a neighbouring tile's group;
that overlap is a presentation detail. If the grid is larger still, a common
visual scale fits it to the area, preserving relative unit sizes. An isolated
unit keeps its original size unless its footprint exceeds the group area.
Offsets and scale live in the root ``Transform``. The root is placed at the
slot center minus the scaled bounds offset; the sprite anchor and selection add
that offset back exactly once. Tile capacity, paths, simulation positions, and
state hashes are unaffected.

Movement smoothing
------------------

Travel and packing are separate. ``UnitMotion`` in ``units/motion.rs`` keeps
the two latest observed simulation positions and linearly interpolates between
them over one 50 ms simulation tick. On uniform terrain, equal frame durations
produce equal travel distances, including at tile centers and boundaries.
Interpolation samples update only when the simulation tick changes. During a
network stall, visuals finish the last segment and hold there; continuing fixed
clock ticks cannot rewind the segment.

Units with active move orders form their own tile groups. Their grid uses the
same slots, fit scale, and mean-position anchor as stopped groups, so a group
that arrives where it was sent has no layout shift to undo. Moving sprites can
straddle tile edges.

Only the packing offset relative to the simulation position and the fit scale
use exponential easing, with ``alpha = 1 - exp(-delta_seconds / 0.1)``. Stable
groups keep constant offsets across tiles. Joining, leaving, and Stop ease
toward the new layout; stopped groups settle exactly on their members' mean
position. These layout transitions can briefly overlap sprites. New units appear
at their initial placement. Despawn and match exit remove the movement state.
Simulation positions, occupancy, and hashes are unaffected.

Selection hit tests, box selection, and selection outlines use the drawn
positions and scaled bounds in both detail modes. Custom bounds use rectangular
click targets and selection outlines; a drag selects any custom bounds
intersecting the box. Units without custom bounds retain circle hit tests,
rings, and selection by center. Minimum mouse and touch hit areas remain in
effect. Smoothing never changes the simulation or its hash.

Gizmos
------

``draw_selection`` in ``PostUpdate`` draws, with Bevy gizmos:

* A ring for a default circle or a rectangle for custom bounds,
  using ``selection.color`` and ``selection.padding`` (yellow and four
  screen pixels in the bundled file). Outlines may overlap;
  they do not participate in packing.
* The drag box while box-selecting.
* An order marker after a move: a circle and cross using the configured
  radius, cross size, duration, and accepted/blocked colors. Bundled defaults
  show green for accepted targets and red for blocked terrain for 0.6 seconds.

Gizmo sizes are multiplied by the camera scale so they keep a constant size on
screen.
