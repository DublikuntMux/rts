Test and check changes
======================

Every change must pass the workspace checks below. Simulation changes also
need determinism tests that compare state hashes, because a desync between
platforms is the most expensive kind of bug in this project.

Required checks
---------------

The ``CI`` workflow runs the required checks on every push and pull request;
see :doc:`releasing` for the jobs and their caches. To run them locally, from
the repository root:

.. code-block:: sh

   cargo fmt --all --check
   cargo test --workspace
   cargo clippy --workspace --all-targets -- -D warnings
   cargo check --target wasm32-unknown-unknown

For changes that touch platform code, dependencies, or Bevy features, also
check the whole workspace for the web and Android:

.. code-block:: sh

   cargo check --workspace --target wasm32-unknown-unknown
   cargo ndk -t arm64-v8a check --workspace

The workspace enables Clippy's ``pedantic`` and ``nursery`` groups as warnings.
``rts_base`` additionally denies float arithmetic and hashed collections; see
:doc:`../architecture/determinism`.

What the tests cover
--------------------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Location
     - Coverage
   * - ``crates/base/src/*`` unit tests
     - Exact tile conversion, Q10 speed parsing, travel-cost rounding, rules
       validation, FNV-1a reference values, and pointer-width-independent
       hashing.
   * - ``crates/base/tests/setup.rs``
     - Bundled map loading, spawn order, invalid maps, identical hashes for
       identical setups.
   * - ``crates/base/tests/commands.rs``
     - Input delay, submission order, ownership, hostile and invalid
       commands, and command queues in the hash.
   * - ``crates/base/tests/movement.rs``
     - A* routes and tie-breaking, exact targets, corner cutting, stop and
       replacement timing, fractional speed remainders, high speeds.
   * - ``crates/base/tests/occupancy.rs``
     - Tile capacity, contention by unit ID, blocked goals and corridors,
       blocker removal, monotonic slow movement.
   * - ``crates/base/tests/terrain_speed.rs``
     - Half, normal, and double speed; boundary crossings in both axes and
       directions; mixed terrain within a tick; fastest-route choice.
   * - ``crates/net/src/*`` unit tests
     - RON recursion bound, bounded inflate, lexical minifier literals and
       separators.
   * - ``crates/net/tests/protocol.rs``
     - Message round trips, setup checksums, binary setup framing and forged
       frames, turn validation, command limits, replay continuity and
       corruption.
   * - ``crates/net/tests/*_replay.rs``
     - Replays that reproduce every hash through capacity contention and
       mixed terrain.
   * - ``crates/server/tests/coordinator.rs``
     - Rooms, readiness, the 20 Hz clock, input authentication and bounds,
       forged ownership, bounded catch-up, connection limits.
   * - ``crates/server/tests/websocket.rs``
     - The real socket runtime: handshake timeouts, heartbeats, large setup
       transfer, oversized input, disconnects.
   * - ``crates/client/src/*`` unit tests
     - Camera math, touch gestures, selection and order input, terrain
       texture arrays, menu pages, dialogs, and text fields, and unit scene
       lifecycle.
   * - ``crates/client/src/session/tests.rs``
     - Session modes, input delay, stalls, divergent hashes, backlog
       catch-up, and headless Bevy clients playing a modded match against
       a real server.

Write a determinism test
------------------------

Simulation tests build a ``Sim`` from inline RON, issue commands, step, and
compare positions and ``state_hash()``. A typical pattern:

.. code-block:: rust

   let rules = Rules::from_ron(include_str!("../../../assets/data/core.rules.ron"))?;
   let map = MapData::from_ron(r#"(name: "t", players: [(start: (0.5, 0.5))],
       tiles: ["...."], units: [(kind: "worker", owner: 0, pos: (0.5, 0.5))])"#)?;
   let mut sim = Sim::new(rules, &map)?;
   sim.queue_command(PlayerId(0), Command::Move {
       units: vec![UnitId(0)],
       target: IVec2::new(3 * TILE + TILE / 2, TILE / 2),
   });
   for _ in 0..40 {
       sim.step();
   }
   assert_eq!(sim.unit(UnitId(0)).unwrap().pos, IVec2::new(3 * TILE + TILE / 2, TILE / 2));

Good determinism tests:

* Assert exact integer positions, not approximate ones.
* Check that every field that affects later ticks changes the hash, even
  before positions differ.
* Record a ``Replay`` and verify that ``Replay::playback`` reproduces the
  final hash.

Verify rendered behavior
------------------------

Automated tests use headless Bevy apps and do not open windows. Before
closing a milestone, also run the game:

* Natively, at a small window width (320 logical pixels) with touch input.
* In a browser with ``trunk serve``, ideally with a native client in the same
  online match.
* On an Android device when Android-specific code changes.

Record what you ran in :doc:`../project/verification`.
