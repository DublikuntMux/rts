Fog of war and player views
===========================

Every player sees the match through fog of war:

* Terrain a player has never seen is black.
* Terrain the player has explored stays drawn, but dimmed while it is out of sight.
* Enemy units are visible only while they stand on a tile one of the player's
  units can see. They vanish when they leave sight.

The server enforces this. A client never receives terrain, units, economy,
orders, or combat that its player cannot see, so a modified client cannot
reveal them. ``rts_base`` (``crates/base/src/visibility.rs``) implements fog
once for the server, the client, the solo session, and the AI.

Pieces
------

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Type
     - Role
   * - ``FogVisibility``
     - Explored and currently visible tiles of one player. Each unit sees a
       disc of radius ``UnitDef::sight`` whole tiles around its tile (offsets
       with ``dx² + dy² ≤ r² + r``). Sight is recomputed every tick from the
       player's own units.
   * - ``PlayerKnowledge``
     - The authority's record of one player's knowledge. ``observe(&sim)``
       returns a ``ViewFrame`` with what changed since the previous
       observation. ``translate_command`` and ``revalidate`` bind that
       player's commands to authoritative IDs.
   * - ``ViewIntro``
     - What a player knows before the first frame: its slot, the player
       count, the map size, its own start, and whether the map is a skirmish.
   * - ``ViewFrame``
     - One tick of knowledge, relative to the previous frame. This is the
       wire format online and the record format of player-perspective replays.
   * - ``PlayerView``
     - The client-side knowledge rebuilt from frames. Its read API mirrors
       ``Sim``: ``units``, ``unit``, ``move_target``, ``production``,
       ``deposits``, ``resources``, ``terrain``, ``visibility``,
       ``can_place_barracks``, and so on. The renderer, HUD, input, and
       ``rts_ai`` read only a ``PlayerView``.

.. kroki::
   :type: plantuml
   :caption: One player's knowledge, from the authoritative simulation to the screen.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   skinparam componentStyle rectangle
   [Sim (authority)] as sim
   [PlayerKnowledge] as knowledge
   [ViewFrame] as frame
   [PlayerView] as view
   [Renderer, HUD, input, AI] as client
   sim --> knowledge : observe every tick
   knowledge --> frame : delta
   frame --> view : apply (validated)
   view --> client : read
   client --> knowledge : Command in view IDs
   knowledge --> sim : translate + revalidate
   @enduml

What a frame contains
---------------------

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Field
     - Content
   * - ``tick``
     - The authoritative tick. Frames start at 0 and follow every tick.
   * - ``explored``
     - ``(tile index, terrain)`` for tiles seen for the first time. Terrain is
       never sent before it is explored.
   * - ``units``
     - Units that came into sight or changed: the player's own units and enemies
       in sight. An enemy's ``cooldown`` is zero and its ``kills`` are rounded
       down to its rank threshold. Only the player's own units carry
       ``OwnUnitState``: move, attack, gather, and build targets, construction,
       cargo, and production.
   * - ``removed``
     - Units that left sight or died. The two cases look the same.
   * - ``deposits``
     - Deposits seen for the first time, or seen again with a different stock.
       A remembered deposit keeps the stock last seen.
   * - ``resources``, ``headquarters``, ``outcome``
     - The player's own stockpile and headquarters, and the match result, each
       sent only when it changes.
   * - ``volleys``
     - Volleys whose shooter and target positions are both in sight.

Hidden activity changes nothing in a frame. Two matches that differ only out
of sight produce byte-identical frames for the observing player; a test
checks this while the hidden player gathers and trains.

View IDs
--------

Unit and deposit IDs in a view are per-player aliases. They are assigned in the
order that player first observes each entity, starting at 0. They reveal
neither authoritative IDs nor how many units were created out of sight. A unit
that leaves sight and returns keeps its alias. The local player's starting
units are observed first, in ID order.

Commands
--------

Clients and the AI write commands in view IDs. The authority binds them in two
steps:

1. ``translate_command`` runs when the command is received (online) or when
   the turn is built (solo). It maps view IDs to authoritative IDs and drops
   units the player does not own. It rejects commands whose attack target,
   gather deposit, or barracks site is not in sight *now*. Remembered terrain,
   deposits, and hidden occupancy therefore never change the answer.
2. ``revalidate`` runs again on the tick the command executes, after the input
   delay. A target that left sight in between is refused, and units that died
   or changed hands are dropped.

Move orders may target unexplored ground. The simulation plans routes using
only terrain the player has revealed; see :doc:`movement`.
``PlayerView::can_place_barracks`` applies the same current-sight rule, so the
interface offers only sites the server will accept.

Fog in the simulation
---------------------

``Sim::enforce_fog`` makes combat and navigation respect each player's sight:
automatic fire and pursuit need a target in sight. Servers enable it for
every match, and solo matches enable it through the full replay version. See
:doc:`../reference/replay-format`. Older recordings replay with their original
rules.

Rendering
---------

* Terrain (``map.rs``) redraws its tile data only when
  ``PlayerView::terrain_revision`` changes (discovery or lost sight) and when a
  match starts; quiet ticks skip the redraw. Unexplored tiles use a black tint, remembered tiles a dim tint
  (``REMEMBERED_TINT``), and visible tiles no tint.
* The minimap terrain image (``hud/minimap.rs``) is regenerated on the same
  revision check. It keeps its CPU pixels (``MAIN_WORLD`` usage) so it can be
  rewritten in place. Its markers are the view's units, so hidden enemies have none.
* Deposit sprites (``skirmish.rs``) spawn when a deposit is first seen and are
  dimmed while out of sight. Only a deposit in sight now is a gather target;
  right-clicking a remembered one issues a move to explore it instead.
* Unit visuals follow ``PlayerView::units``. An enemy that leaves sight is
  despawned like a dead unit.

Solo play, replays, and the AI
------------------------------

A local match keeps a ``LocalAuthority``: the ``Sim``, the local player's
``PlayerKnowledge``, and one ``Seat`` per computer opponent. Each seat has the
opponent's own knowledge, view, and ``rts_ai::Opponent``. Every tick the
session builds the turn as follows:

1. Translate the local player's delayed orders.
2. Ask each seat for orders from its view and translate them.
3. Apply the turn, then observe for every player.

The AI never reads the ``Sim``. Resuming a saved game replays its turns in the
same order, so every knowledge and opponent memory matches the live match.

Full replays of local matches play back the whole simulation and show it from
player 1's view. Online matches record a player-perspective ``ViewReplay``.
See :doc:`sessions` and :doc:`../reference/replay-format`.

Bundled maps are public
-----------------------

Fog cannot hide a map that every client ships with. By default the server
therefore plays a fresh *Random frontier* layout for every match, generated
from a seed only the server knows (see :doc:`server`). A server started with
``--map`` plays that map, and anyone with the file knows its layout. Fog still
hides units, orders, economy, and combat.
