Simulation core
===============

``rts_base`` (``crates/base``) is the deterministic heart of the game. It
loads rules and maps, owns the state of one match, applies commands, moves
units, and computes the state hash. It has no Bevy dependency and no I/O, so
the client, the server, replays, and tests all run the same code.

Modules
-------

.. list-table::
   :header-rows: 1
   :widths: 22 78

   * - Module
     - Responsibility
   * - ``lib.rs``
     - ``Sim``, ``Unit``, ``Player``, ``UnitId``, ``PlayerId``, setup
       validation, the tick step, and command application.
   * - ``coords.rs``
     - ``TILE``, ``tile_of``, and exact conversions from data-file units.
   * - ``rules.rs``
     - ``Rules``, ``TerrainDef``, ``UnitDef``, and their validation.
   * - ``map.rs``
     - ``MapData`` (the file format) and ``Terrain`` (the runtime grid).
   * - ``command.rs``
     - ``Command`` and ``INPUT_DELAY_TICKS``.
   * - ``movement.rs``
     - ``MoveOrder``: routes, replanning, and the per-tick movement budget.
   * - ``path.rs``
     - Travel-time A* over the tile grid.
   * - ``occupancy.rs``
     - Per-tile capacity usage, rebuilt each tick.
   * - ``hash.rs``
     - ``StateHasher``, the platform-independent FNV-1a hasher.

Match state
-----------

.. kroki::
   :type: plantuml
   :caption: What a Sim owns. Rules and terrain are fixed after setup; the rest changes every tick.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide empty members
   class Sim {
     rules: Rules
     terrain: Terrain
     players: Vec<Player>
     tick: u32
     units: BTreeMap<UnitId, Unit>
     next_unit_id: u32
     {field} commands: BTreeMap<u32, Vec<(PlayerId, Command)>>
     move_orders: BTreeMap<UnitId, MoveOrder>
   }
   class Rules {
     tile_capacity: u16
     terrain: Vec<TerrainDef>
     units: Vec<UnitDef>
   }
   class Terrain {
     size: IVec2
     tiles: Vec<TerrainId>
   }
   class Unit {
     kind: UnitKind
     owner: PlayerId
     pos: IVec2
     health: i32
   }
   class Player {
     start: IVec2
   }
   class MoveOrder {
     target: IVec2
     waypoints: VecDeque<IVec2>
     segment_start: IVec2
     segment_progress: u64
     speed_remainder: u32
   }
   Sim *-- Rules
   Sim *-- Terrain
   Sim *-- "1..8" Player
   Sim *-- "*" Unit
   Sim *-- "*" MoveOrder
   @enduml

* ``Rules`` index terrain and unit types by position: ``TerrainId(u8)`` and
  ``UnitKind(u16)``. Lookups by ID panic only for IDs that did not come from
  the same rules.
* ``Terrain`` stores one ``TerrainId`` per tile, row-major with the top row
  first. ``speed_modifier(pos)`` returns 0 outside the map, so "outside" and
  "blocked" are the same check.
* ``Unit`` has no ID field; its ID is its key in ``units``. ``health`` is set
  to ``max_health`` and is reserved for combat (M4).
* ``commands`` holds locally queued commands by application tick. Online play
  supplies commands per turn instead.
* ``move_orders`` holds at most one active order per unit.

Setup
-----

``Sim::new(rules, &map)`` builds a match and validates the map against the
rules:

1. Parse ``tiles`` into a ``Terrain`` (size 1 to 1024 per side, equal row
   lengths, known symbols).
2. Check the player count (1 to 8) and that every start is on traversable
   terrain.
3. Spawn each unit in file order after checking its kind, owner, terrain,
   and the capacity of its tile. IDs start at 0.

``rts_net::MatchSetup::simulation`` wraps this for networked setups: it
bounds document sizes, parses both RON documents, and checks the map's
player count against the setup.

The tick step
-------------

.. kroki::
   :type: plantuml
   :caption: One call to Sim::step_with_commands.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   :tick += 1;
   :take local commands queued for this tick;
   :apply queued commands, then supplied commands,\nin order;
   if (any move orders?) then (yes)
     :rebuild occupancy from all living units;
     :for each order in UnitId order:\n  remove the unit from occupancy\n  replan against live occupancy\n  advance along the route\n  add the unit back at its new position;
     :drop orders that finished\nor whose unit no longer exists;
   else (no)
   endif
   stop
   @enduml

``Sim::step()`` is ``step_with_commands`` with no supplied commands. Commands
are always applied *before* movement on the same tick, so a Stop applied on
tick N prevents any movement on tick N. :doc:`movement` describes the movement
phase in detail.

Commands
--------

.. code-block:: rust

   pub enum Command {
       Move { units: Vec<UnitId>, target: IVec2 },
       Stop { units: Vec<UnitId> },
   }

A command names units by stable ID and a target in world units. Applying a
command checks everything again, because a command may be stale or hostile:

* **Move** is ignored entirely if the target is outside the map or on blocked
  terrain. Otherwise, for each listed unit that exists and belongs to the
  issuing player, the simulation plans a route ignoring other units. If the
  target is reachable, the new order replaces the unit's current one. If not,
  the current order stays, so an invalid replacement never strands a unit.
* **Stop** removes the order of each listed unit the player owns.
* Unknown IDs, other players' units, and invalid players are skipped.

Input delay
-----------

Commands never apply on the tick they are issued. ``INPUT_DELAY_TICKS = 2``
gives every peer 100 ms to receive them before they take effect.

* ``Sim::queue_command(player, command)`` stores a command for
  ``tick + 2``. Commands for the same tick keep submission order. This is the
  simple local path used by tests.
* ``Sim::step_with_commands(commands)`` applies agreed commands on the next
  tick with no further delay. The client and server use it through
  ``rts_net::Turn::apply``; the delay is added when the client chooses the
  turn's tick. See :doc:`networking`.

Queued commands are part of the state hash, so two peers that disagree about
pending input are detected even before the input takes effect.

Unit lifecycle
--------------

* Units are created by ``Sim::new`` from the map. In-match spawning
  (production) arrives in M5.
* ``Sim::remove_unit(id)`` removes a unit and its order and returns it. IDs
  are never reused. The freed capacity is visible to pathfinding on the next
  tick. Simulation actions must call it identically on every peer; combat
  deaths (M4) will use it.
* ``Sim::units()`` iterates living units in ID order. ``Sim::unit(id)``,
  ``Sim::move_target(id)``, ``players()``, ``terrain()``, ``rules()``, and
  ``tick()`` are the read-only views the client uses for rendering and input.

Errors
------

Loading returns ``rts_base::LoadError``: ``Parse`` wraps a RON error with its
position, and ``Invalid`` carries a message naming the offending tile, unit
index, or type. Runtime operations never fail; invalid commands are ignored
by design so that a bad command cannot stop the match on one peer only.
