Deterministic simulation
========================

Every peer that runs a match must compute bit-identical results from the same
setup and the same turns: the server, native clients, browser clients on
``wasm32``, Android clients, and replay playback. This page lists the rules
that guarantee it and how they are enforced. They apply to all code in
``rts_base`` and to future mission scripts.

Authority model
---------------

The server owns the rules, map, tick clock, and simulation. It sends the
complete match configuration when a player creates or joins a room, then one
ordered turn and its resulting state hash every tick. Clients run
deterministic replicas to render the match and record replays. They verify
every tick against the server and stop locally on divergence. Client state and
hashes never influence the server.

Determinism is therefore needed for *agreement*, not for authority. A
nondeterministic bug cannot corrupt the server's match, but it stops every
client whose replica disagrees.

Rules
-----

1. **No floating-point arithmetic.** Positions, speeds, and distances are
   integers in world units (``TILE = 1024`` per tile). Use ``i64`` or ``u64``
   for intermediates such as squared distances, and ``isqrt`` for lengths.
   Floats appear only when parsing data files; see :ref:`fixed-point-data`.
2. **No hashed collections.** ``HashMap``, ``HashSet``, and anything else with
   unspecified iteration order are forbidden. Use ``BTreeMap``, ``BTreeSet``,
   ``Vec``, or sorted keys.
3. **No wall clock, OS randomness, or thread timing.** Time advances only
   through ``Sim::step``. Randomness will come from a seeded PRNG stored in
   simulation state when it is first needed; the setup already carries a
   ``seed``.
4. **Iteration order is defined by stable IDs.** Units get monotonically
   increasing ``UnitId`` values that are never reused, and are stored in a
   ``BTreeMap``. Contention between units is resolved in ID order.
5. **Fixed tick rate.** ``TICK_RATE = 20`` ticks per second. Rendering
   interpolates between ticks; the simulation never sees frame time.
6. **Explicit hash encoding.** The state hash uses fixed-width little-endian
   integers, so 64-bit and 32-bit targets agree.

Enforcement
-----------

``crates/base/src/lib.rs`` denies two Clippy lints for the whole crate:

.. code-block:: rust

   #![deny(clippy::float_arithmetic, clippy::disallowed_types)]

``crates/base/clippy.toml`` lists ``std::collections::HashMap`` and
``HashSet`` as disallowed types. The only float arithmetic is the data-file
conversion in ``coords.rs``, which carries an ``#[expect]`` with its reason.
Integer conversions use ``try_from`` with explicit failure rather than
``as`` casts where a value could overflow.

.. _fixed-point-data:

Fixed-point data
----------------

Data files are written in human units, so the loaders convert them once:

.. list-table::
   :header-rows: 1
   :widths: 30 25 45

   * - Field
     - File unit
     - Stored as
   * - Positions, ``radius``
     - tiles
     - ``i32`` world units: ``round(tiles × 1024)``.
   * - Unit ``speed``
     - tiles/second
     - ``i32`` world units per second.
   * - Terrain ``speed_modifier``
     - multiplier
     - Q10 ``i32``: ``round(modifier × 1024)``.

Parsing a decimal string into ``f64`` is correctly rounded on every platform,
and multiplying by a power of two is exact. The result is rounded and
range-checked before the cast, so every peer stores the same integer. After
loading, no float is touched again.

The server sends the original RON text rather than reserializing the parsed
values. Reserializing could print a different decimal and, after rounding,
produce a different integer on the client. The lexical minifier
(:doc:`../reference/setup-frame`) preserves every number's spelling for the
same reason.

State hash
----------

``Sim::state_hash`` hashes everything that changes during a match with
``StateHasher`` (``crates/base/src/hash.rs``), a 64-bit FNV-1a implementation
of ``std::hash::Hasher``:

* Integers are written as fixed-width little-endian bytes.
* ``usize`` (collection lengths) is written as ``u64``, so ``wasm32`` matches
  64-bit targets.
* ``Command`` implements ``Hash`` by hand with explicit ``u8`` tags, because a
  derived enum discriminant may be hashed with a platform-sized integer.

.. list-table:: Hashed state, in order
   :header-rows: 1
   :widths: 30 70

   * - Field
     - Content
   * - ``tick``
     - Ticks simulated so far.
   * - ``next_unit_id``
     - The next ID to allocate.
   * - ``units``
     - Every living unit in ID order: kind, owner, position, health.
   * - ``commands``
     - Locally queued commands by tick, in submission order.
   * - ``move_orders``
     - Every active order: target, remaining waypoints, segment start,
       segment progress, and speed remainder.

Rules and terrain are fixed at setup and are not hashed; the setup checksum
covers them instead. Fractional movement state is hashed, so two peers that
disagree about sub-unit progress are detected before their positions differ.

Desync detection
----------------

.. kroki::
   :type: plantuml
   :caption: How a client detects divergence from the server.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   :receive Turn N and server hash H;
   :apply Turn N to the local Sim;
   :h = Sim::state_hash();
   if (h == H?) then (yes)
     :record Turn N and h in the replay;
     :render;
   else (no)
     :stop the local match;
     :show "Match state differs from\nthe authoritative server";
     :close the connection;
   endif
   stop
   @enduml

There is no resynchronization yet. Corrective snapshots that let a diverged or
late client recover are future work.

Writing deterministic code
--------------------------

* Prefer integer division with explicit rounding (``div_ceil``,
  ``div_euclid``), and keep remainders when a quantity accumulates over ticks.
* Make every tie-break explicit: by ``UnitId``, by tile index, or by a fixed
  neighbor order.
* Add every field that affects a later tick to ``state_hash``.
* Never derive behavior from Bevy entities, frame timing, or presentation
  state. The client may read the simulation but must only change it through
  turns.
* Test with exact integer assertions and replay hash checks; see
  :doc:`../guides/testing`.
