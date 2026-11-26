Rendering
=========

The client draws three layers: a terrain tilemap, unit visuals, and combat
effects, plus gizmos for selection. Everything is derived from the simulation;
nothing drawn feeds back into it. Code lives in ``crates/client/src/map.rs``,
``units/mod.rs``, ``units/layout.rs``, ``units/motion.rs``, and
``effects.rs``.

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

``UnitVisuals`` stores entities, shared images, persistent movement state, and
reusable layout buffers. A flat unit buffer sorts by movement state, tile, and
stable ID, so packing retains its storage across frames instead of allocating
a vector for each occupied tile. The placement map also retains its capacity.
These presentation caches never affect the deterministic core. On match exit,
the caches reset and roots despawn through ``DespawnOnExit``.

Unit detail by zoom
-------------------

Units use the same ``ZoomDetail`` state and ``terrain_detail`` thresholds as
the map: rectangles below 40 screen pixels per tile, sprites again at 48. Unit
size and crowded-group scaling do not affect the decision. All units, including
new visuals, switch together; the shared hysteresis prevents flicker. Terrain
textures still need to finish loading before they appear.

Both unit modes share the same size, anchor, transform, and selection target;
a distant rectangle has the fitted image's shape.
Rectangles use Bevy's white default sprite image; the required image handle
remains cached for zooming back in. There are no separate unit detail settings.

Squads
------

A unit type with ``squad`` above 1 draws each soldier as a child sprite of the
unit root (``SoldierVisual``). The unit's bounds are split into the shared
``SquadGrid`` cells, row zero at the top as in the simulation, and every
soldier's image is fitted to its cell. The root keeps the whole bounds: it is
transparent at sprite detail and becomes the solid formation rectangle when
zoomed out, while the soldiers hide. A fallen soldier's sprite hides and its
cell stays empty. Soldiers inherit the root transform, so tile packing and
movement smoothing move and scale the formation as one piece, and selection
still hits the unit's bounds.

Tile groups
-----------

``units/layout.rs`` uses the optional ``visual_bounds`` from the rules: a center
offset and full width and height. When omitted, it derives a centered square
from the unit's diameter. Once the image has loaded, the sprite is scaled to
fit inside this footprint at the image's own aspect ratio and centered on it,
so non-square images are not stretched. Until then it fills the footprint. See
:doc:`../reference/rules-format` for the RON syntax.

Units sharing a tile, including different owners, occupy a compact grid in
``UnitId`` order. Cells use the group's maximum width and height; the grid
orientation minimizes its longest side. Cells have a nominal four-world-pixel
gap (scaled with the group) and four world pixels of border padding. The last
row is centered.

A stopped group fits within the tile interior. It uses the largest shared scale
that fits and stays as close as possible to its members' mean footprint center.
The mean sums offsets from one member to avoid precision loss far from the
origin. An isolated unit stays at its simulation position and keeps its size
up to one tile across.
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

Moving and stopped units in a tile share the grid. When every occupant is
traveling, the grid follows the mean footprint center across tile boundaries.
A tile with stopped occupants keeps its packed layout. Once the last movement
segment finishes in an ended session, its layout eases into the tile interior.
An active session keeps its last layout during a pause or delayed server frame.

Only the packing offset relative to the simulation position and the fit scale
use exponential easing, with ``alpha = 1 - exp(-delta_seconds / 0.1)``. Stable
groups keep their relative slot spacing across tiles. Joining or leaving a
tile eases toward the new layout. These transitions can briefly overlap
sprites. New units appear
at their initial placement. Despawn and match exit remove the movement state.
Simulation positions, occupancy, and hashes are unaffected.

Selection hit tests, box selection, and selection outlines use the drawn
positions and scaled bounds in both detail modes. Custom bounds use rectangular
click targets and selection outlines; a drag selects any custom bounds
intersecting the box. Units without custom bounds retain circle hit tests,
rings, and selection by center. Minimum mouse and touch hit areas remain in
effect. Smoothing never changes the simulation or its hash.

Combat effects
--------------

Every volley the simulation fires is drawn as pixel-art tracers and impacts by
``effects.rs``. The simulation records each tick's volleys in
``Sim::volleys``: shooter and target with their kinds and positions, and for
every shot the squad slot that fired and the slot it hit. The record is
rebuilt each tick and excluded from the state hash, so hashes and replays are
unchanged. After each turn, ``advance`` copies the record into ``Match``
with its tick, so catch-up frames that step several ticks keep every volley.

Once per frame, before units sync, ``spawn_shots`` turns the new volleys into
one effect per shot. Endpoints are fixed then: the firing soldier as drawn,
to the hit soldier as drawn, using ``soldier_point`` from the unit's root
transform and squad cell. A unit with no visual yet falls back to its
simulation position. Running before ``SyncUnits`` keeps a target destroyed
by the volley on screen long enough to be measured. Tracers therefore end
where the target stood when the shot landed, not where it later walks.

Each shot is a parent entity with five child sprites, animated in virtual
time:

1. a pale muzzle flash block at the shooter for 0.05 s;
2. a three-block tracer, bright head and amber tail, flying to the target at
   2400 world pixels per second, between 0.04 and 0.12 s;
3. an impact plus shape for 0.06 s, then one dim block for 0.06 s.

Soldiers of one volley start 18 ms apart, so a squad reads as a ripple of
shots. Colors are fixed neutral warm tones, distinct from player colors and
legible on every terrain. Blocks are untextured squares snapped to a grid
of four world pixels, the terrain's art pixel, and drawn at z = 2 above
units. When zoomed out, blocks grow to stay at least two screen pixels wide.

Effects stay cheap in large fights and catch-up. A volley more than three
ticks old when drawn is skipped, and older ticks start their shots aged so
timing stays true. At most 192 shots are live. ``Match`` keeps at most 256
undrawn volleys. Effects freeze with the pause menu's virtual time and
despawn when the match ends. Replays draw the same effects because playback
steps the simulation through the same ``advance``.

Gizmos
------

``draw_selection`` in ``PostUpdate`` draws, with Bevy gizmos:

* A ring for a default circle or a rectangle for custom bounds,
  using ``selection.color`` and ``selection.padding`` (yellow and four
  screen pixels in the bundled file). Outlines may overlap;
  they do not participate in packing.
* The drag box while box-selecting.
* A health bar above every damaged or selected unit, spanning its drawn
  footprint ``health_bar.gap`` screen pixels above it. The fill shows the
  squad's remaining share of total health.
* An order marker after a move or attack: a circle and cross using the
  configured radius, cross size, duration, and colors. Bundled defaults show
  green for accepted targets, red for blocked terrain, and orange for attacks
  for 0.6 seconds.

Gizmo sizes are multiplied by the camera scale so they keep a constant size on
screen.
