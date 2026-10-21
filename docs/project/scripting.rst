Planned: mission scripting (M6)
===============================

Missions, triggers, and objectives will be written in Lua. This page records
the design decisions made so far. Nothing here is implemented yet.

Runtime
-------

* Use ``omnilua``, a pure-Rust Lua, built with ``default-features = false``
  so only the ``base``, ``string``, ``table``, and ``math`` libraries are
  included.
* Run scripts through ``Lua::sandboxed`` with an instruction budget per call
  (``SandboxConfig::instruction_limit``), so a runaway script cannot hang a
  tick.
* No ``os``, ``io``, ``package``, or ``debug`` libraries.

Determinism
-----------

Mission scripts run **inside** the simulation step on every peer, so they must
follow :doc:`../architecture/determinism`:

* Replace ``math.random`` with the simulation's seeded PRNG. The ``seed`` in
  ``MatchSetup`` is reserved for this.
* Remove or replace platform-dependent math (``sin``, ``cos``, ``exp``,
  ``log``, and similar) with deterministic implementations.
* Verify that ``pairs`` iteration order is deterministic in omnilua, which
  requires a fixed string-hash seed, or forbid relying on it. This is an open
  question.
* Script state that affects later ticks must be included in the state hash.

Crate placement
---------------

Scripts run inside the simulation step, so ``rts_base`` and a new
``rts_script`` crate cannot simply depend on each other in one direction.
Decide the split at the start of M6. One option: ``rts_base`` defines a trait
for script hooks and ``rts_script`` implements it.

.. kroki::
   :type: plantuml
   :caption: One possible split, with the simulation calling scripts through a trait.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   skinparam componentStyle rectangle
   component rts_base {
     [Sim::step] as step
     interface "ScriptHooks" as hooks
   }
   component rts_script {
     [omnilua sandbox] as lua
     [game API bindings] as api
   }
   step --> hooks : events each tick
   lua ..|> hooks : implements
   api --> step : queries and actions
   @enduml

API shape
---------

.. list-table::
   :header-rows: 1
   :widths: 20 80

   * - Kind
     - Examples
   * - Events
     - ``on_unit_died``, ``on_enter_region``, ``every(ticks)``.
   * - Queries
     - Units in a region, player resources.
   * - Actions
     - Issue a command, spawn a unit, set an objective, show a message,
       declare victory or defeat.

Unit and building statistics stay in RON data files, not scripts.

Map format additions
--------------------

Maps gain named regions and a ``script`` path. The map editor (M7) will edit
both. See :doc:`../reference/map-format`.
