Movement and pathfinding
========================

Movement turns a ``Move`` order into integer position changes, one tick at a
time. It must be exact (no drift between platforms), fair (no free movement
through slow terrain), and aware of other units (tile capacity). The code is
in ``crates/base/src/movement.rs``, ``path.rs``, and ``occupancy.rs``.

Overview
--------

.. kroki::
   :type: plantuml
   :caption: The life of a move order.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Planned : Move applied and\nstatically reachable
   Planned : route ignores other units
   Planned --> Active
   Active : each tick: replan against live occupancy,\nthen spend the tick's movement budget
   Active --> Active : advanced, or waiting\nfor capacity
   Active --> [*] : reached the target
   Active --> [*] : Stop, or unit removed
   Active --> Planned : replaced by a\nreachable Move
   @enduml

Each tick the simulation:

1. Rebuilds tile occupancy from every living unit.
2. Visits move orders in ``UnitId`` order. For each unit it removes the unit
   from occupancy, replans its route against the remaining units, spends the
   unit's movement budget along the route, and adds the unit back at its new
   position.
3. Drops orders that reached their target or whose unit no longer exists.

Because each unit updates occupancy before the next one plans, two units can
never both take the last space in a tile. The lower ID wins.

Terrain speed
-------------

Each terrain type has a Q10 ``speed_modifier`` (1024 is normal speed; 0
blocks). Movement uses its reciprocal, the **travel cost** per world unit:

.. math::

   \text{cost} = \left\lceil \frac{2^{32} \times 1024}{\text{modifier}} \right\rceil

At normal speed the cost is exactly 2\ :sup:`32` effort per world unit
(``TRAVEL_SCALE``); at double speed it is 2\ :sup:`31`; at half speed
2\ :sup:`33`. Rounding up means a fast terrain is never faster than declared.
Blocked terrain has no cost and is never entered.

Pathfinding
-----------

``path::find_path`` runs A* over the tile grid with four neighbors and
minimizes travel *time*, not distance:

* **Edge cost.** Moving between adjacent tile centers spends half a tile in
  each tile, so the edge cost is ``cost(a) + cost(b)``; the common factor of
  half a tile is dropped from every path.
* **Heuristic.** Manhattan distance × 2 × the cheapest terrain cost in the
  rules. Using the *fastest* terrain keeps the heuristic admissible, so a
  route over fast terrain is never overestimated.
* **Ordering.** The open set is a min-heap ordered by estimated total cost,
  then remaining estimate, then row-major tile index. Neighbors are expanded
  north, west, east, south. Every tie is therefore resolved identically on
  every peer.
* **Traversable tiles.** A tile must have a positive speed modifier. While
  replanning, intermediate tiles must also have free capacity. The start and
  goal tiles are exempt, so a unit can approach a full destination and wait
  beside it.

The result is a list of tile centers from the start tile to the goal tile.

Routes
------

``MoveOrder::route`` turns a path into waypoints:

.. kroki::
   :type: plantuml
   :caption: Waypoints for a unit at an arbitrary position moving two tiles to the right. Numbers give the visiting order.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.1
   {#
     <b>row 0 | <b>column 0 | <b>column 1 | <b>column 2
     start | unit at (300, 200) | . | .
     waypoints | 1. center (512, 512) | 2. center (1536, 512) | 3. center (2560, 512)
     . | . | . | 4. target (2900, 700)
   }
   @endsalt

* If the target is in the start tile, the only waypoint is the target.
* Otherwise the waypoints are every tile center on the path, *including the
  start tile's center*, followed by the exact target.

Visiting the start center first keeps every leg either inside one tile or
between two adjacent centers. A unit at an arbitrary position that headed
straight for a neighbor's center could cut across the corner of a blocked
tile.

Segments and the movement budget
--------------------------------

A **segment** is the leg from ``segment_start`` to the next waypoint. Its
length is the Euclidean distance rounded up with an integer square root.
A segment between two adjacent centers is exactly one tile long and is split
at the tile boundary: the first half costs the start tile's rate and the
second half the end tile's rate. So a unit cannot coast through slow terrain
at fast-terrain speed.

Each tick a unit gets a budget of effort:

.. math::

   \text{budget} = \left\lfloor \frac{\text{speed} \times 2^{32} + \text{remainder}}{20} \right\rfloor

``speed`` is in world units per second at normal terrain, and the division
remainder is kept in ``speed_remainder`` for the next tick. The unit spends
the budget along its waypoints:

.. kroki::
   :type: plantuml
   :caption: MoveOrder::advance for one tick.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   :compute budget, keep remainder;
   while (next waypoint exists?) is (yes)
     if (waypoint tile has capacity?) then (no)
       :wait;
       break
     endif
     :cost = effort to finish the segment;
     :spend min(budget, cost - progress);
     if (segment finished?) then (yes)
       :pos = waypoint;
       :next segment starts here,\nprogress = 0;
     else (no)
       :pos = point along segment\nfor the spent effort;
       break
     endif
   endwhile (no)
   :order is finished if no waypoints remain;
   stop
   @enduml

* Progress is stored as effort, not distance, so fractional movement
  accumulates exactly. A unit at 1 world unit per second still finishes a
  short diagonal, and slow units do not drift.
* The position inside a segment is derived from progress with integer
  division, using the rate of the half the unit is in.
* A fast unit may cross several waypoints in one tick. Every waypoint it
  enters is checked for capacity first, so high speed cannot skip a full
  tile.

Tile capacity
-------------

``Rules::tile_capacity`` limits the total unit ``size`` in one tile, shared by
all players. A unit occupies only the tile that contains its center.

``Occupancy`` is a dense ``Vec<u32>`` of used capacity per tile. It is derived
data: rebuilt from living units at the start of every movement phase and never
stored in ``Sim`` or hashed. Map loading uses the same structure to reject
over-capacity starting placements.

Replanning
----------

Every active order replans every tick against live occupancy
(``MoveOrder::refresh``):

* If the unit is partway along a segment whose destination tile still has
  room, that segment is kept as an *anchor* and the route is replanned from
  its end. Keeping the anchor prevents the unit from turning back toward its
  previous center and preserves fractional progress.
* Otherwise the unit starts a fresh segment from its current position.
* If no route exists right now, the order stays active and the unit waits.
  It tries again on the next tick.

An order is created only if the target is reachable when units are ignored,
so a waiting order always has a chance to resume. A full goal can be
approached but not entered until space frees; moving or removing a blocker
frees its capacity on the next tick.

Limits and future work
----------------------

* Capacity limits how many units share a tile; it does not separate unit
  circles inside a tile.
* Replanning runs one A* per moving unit per tick. That is fine for current
  unit counts; M10 adds flow fields for groups, local avoidance, and a spatial
  index. Optimizations must keep the live-occupancy behavior and the exact
  tie-breaking above.
* Radius-aware terrain clearance, physical separation, formations, and
  simultaneous swaps between adjacent full tiles are not implemented.

Everything that affects a later tick—waypoints, segment start, progress, and
speed remainder—is part of the state hash.
