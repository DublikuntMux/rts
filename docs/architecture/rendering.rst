Rendering
=========

The client draws two layers: a terrain tilemap and unit visuals, plus gizmos
for selection. Everything is derived from the simulation each frame; nothing
drawn feeds back into it. Code lives in ``crates/client/src/map.rs`` and
``units.rs``.

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

``update_terrain_detail`` measures screen pixels per tile (``128 / scale``)
and switches the chunk's material between tilesets with hysteresis:

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

``sync_units`` runs every frame in ``InGame`` and reconciles one visual entity
per living unit:

.. kroki::
   :type: plantuml
   :caption: Unit visual synchronization.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   while (for each living unit in ID order)
     :position = lerp(previous, current,\nfixed overstep fraction);
     if (visual exists for UnitId?) then (yes)
       :update its Transform;
     else (no)
       :get or create the circle mesh\nfor its kind (radius);
       :get or create the material\nfor its owner (player color);
       :spawn_scene(unit_scene(...));
     endif
   endwhile
   :despawn visuals whose unit is gone;
   stop
   @enduml

* Units are circles with their rules ``radius``, drawn at z = 1 above the
  terrain.
* Meshes are cached per ``UnitKind`` and materials per ``PlayerId``, so all
  units of a kind share one mesh and all units of a player share one material.
  Player colors by slot: blue, red, yellow, green, purple, orange, cyan, and
  white.
* ``UnitVisuals`` maps ``UnitId`` to entity. It is a Bevy ``HashMap`` because
  it is presentation state; determinism rules apply only to ``rts_base``.
* ``OnExit(InGame)`` resets the caches. The visual roots themselves despawn
  through ``DespawnOnExit``.

The unit scene
--------------

.. code-block:: rust

   fn unit_scene(id: UnitId, mesh: Handle<Mesh>,
                 material: Handle<ColorMaterial>, translation: Vec3) -> impl Scene {
       let visual = UnitVisual(id);
       let transform = Transform::from_translation(translation);
       bsn! {
           DespawnOnExit<AppState>(AppState::InGame)
           visual
           Mesh2d(mesh)
           MeshMaterial2d<ColorMaterial>(material)
           transform
       }
   }

Health bars (M4) will be children of this root. See :ref:`client-scenes` for
the scene rules.

Interpolation
-------------

The simulation moves in 50 ms steps, but frames arrive more often. Before each
tick, the session copies every unit's position into ``PreviousPositions``.
Each frame, ``sync_units`` draws:

.. math::

   \text{drawn} = \text{previous} + (\text{current} - \text{previous}) \times \alpha

where α is ``Time<Fixed>::overstep_fraction()``, clamped to 0..1. Units
without a previous position (new units) are drawn at their current position.
When a session stalls—no server turn, playback finished, or stopped—the
snapshot is cleared so units do not slide backwards toward an old position
on repeated frames.

Selection hit tests, box selection, and selection rings all use the drawn
positions, so what the player sees is what they click. Interpolation never
changes the simulation or its hash.

Gizmos
------

``draw_selection`` in ``PostUpdate`` draws, with Bevy gizmos:

* A yellow ring around each selected unit, 4 screen pixels outside its radius.
* The drag box while box-selecting.
* An order marker for 0.6 seconds after a move: a green circle and cross for
  an accepted target, red for blocked terrain.

Gizmo sizes are multiplied by the camera scale so they keep a constant size on
screen.
