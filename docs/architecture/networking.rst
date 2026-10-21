Networking model
================

RTS uses **deterministic command replication with server authority**. Instead
of sending positions, the server sends the ordered commands of every tick plus
a hash of its resulting state. Each client applies the same commands to its
own copy of the simulation and checks the hash. This keeps bandwidth tiny and
independent of unit count, and gives replays for free.

This page follows one command end to end. The message formats are in
:doc:`../reference/protocol`; the server internals are in :doc:`server`; the
client internals are in :doc:`sessions`.

Why this model
--------------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Property
     - How it is achieved
   * - Same game on every screen
     - Every peer runs the deterministic ``rts_base`` simulation on the same
       setup and the same turns. See :doc:`determinism`.
   * - Cheating is limited
     - Only the server's simulation is authoritative. It checks ownership,
       targets, reachability, and capacity, and owns the clock.
   * - Divergence is detected
     - The server sends its state hash with every turn; clients compare.
   * - Slow players cannot stall others
     - The server advances at 20 Hz whether or not input arrives.
   * - Browsers can play
     - WebSockets work everywhere; the protocol has no UDP or WebRTC
       dependency.
   * - Replays are exact
     - A replay is the setup plus the turns already exchanged.

The cost is input latency: a command takes effect two ticks after the client
schedules it, plus network delay.

A command's journey
-------------------

.. kroki::
   :type: plantuml
   :caption: A Move issued by player 1 while the client has received turn 40.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide footbox
   actor "Player 1" as P
   participant "Client 1\n(Match, Session)" as C1
   participant "Server\n(Coordinator, Game)" as S
   participant "Client 2" as C2
   P -> C1 : right-click ground
   C1 -> C1 : queue Move in Match.pending
   S --> C1 : Turn 41 + hash
   C1 -> C1 : apply 41, verify hash
   C1 -> S : Input { tick: 43, [Move] }
   S -> S : validate, filter ownership,\nstore in inputs[43][slot 0]
   S --> C1 : Turn 42 + hash
   S --> C2 : Turn 42 + hash
   S -> S : tick 43 due:\nTurn 43 = { slot 0: [Move], slot 1: [] }\napply, hash = H43
   S --> C1 : Turn 43 + H43
   S --> C2 : Turn 43 + H43
   C1 -> C1 : apply 43 → units start moving\nhash == H43 ✓
   C2 -> C2 : apply 43 → same result\nhash == H43 ✓
   @enduml

1. Input appends the command to ``Match::pending``. Nothing moves yet.
2. When the client applies its next turn, it sends all pending commands for
   *latest received tick + 2*.
3. The server validates the input, removes units the player does not own,
   and stores it for that tick.
4. When the tick is due, the server builds one ``Turn`` with every player's
   batch in slot order, applies it, hashes the state, and broadcasts both.
5. Every client applies the turn and compares hashes before recording it.

Input delay timeline
--------------------

.. kroki::
   :type: plantuml
   :caption: Ticks on the server and one client, with time in milliseconds. The client always schedules input two ticks past the latest turn it has received.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   concise "Server tick" as S
   concise "Client received" as R
   concise "Input sent for" as I
   @0
   S is 40
   R is 39
   I is 41
   @50
   S is 41
   R is 40
   I is 42
   @100
   S is 42
   R is 41
   I is 43
   @150
   S is 43
   R is 42
   I is 44
   @200
   S is 44
   R is 43
   I is 45
   @enduml

Input sent after receiving turn *N* targets tick *N + 2*, which the server
simulates 100 ms after turn *N*. Input therefore arrives in time when the
round trip plus the wait for the client's next fixed step stays under 100 ms.
If input arrives after its tick has already been simulated, the server rejects
it with ``Error`` and the client keeps playing; the command is lost. Inputs
more than eight ticks ahead are also rejected. Adaptive input delay is not
implemented.

Shared clock, separate pacing
-----------------------------

The server is the only clock. Clients run ``FixedUpdate`` at 20 Hz too, but a
client's fixed step only *consumes* turns:

* If the next turn has not arrived, the client waits without advancing,
  freezing interpolation, and shows *Waiting for the server…* after 0.25 s.
* If several turns are buffered after a slow frame, it applies up to eight
  per fixed step to catch up.
* After 15 seconds without progress it stops.

The server bounds its own catch-up after stalls to four turns per clock
check, so a suspended server cannot flood clients with a burst.

Setup delivery
--------------

Before a match, every client receives the server's exact rules and map in a
compressed binary frame (:doc:`../reference/setup-frame`), builds the
simulation from it, and acknowledges with ``Loaded { checksum }``. Clients never compare
the setup with their bundled files, so a server can run content mods without
client rebuilds. The client must still run compatible game code: the protocol
version must match, and the simulation code must be the same.

.. kroki::
   :type: plantuml
   :caption: Setup acknowledgment.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide footbox
   participant Client as C
   participant Server as S
   S -> S : startup: read files,\nbuild Sim, compute checksum K
   C -> S : Join { version: 5, code, name }
   S --> C : Setup (minified + zlib)
   C -> C : inflate with bounds,\nparse, build Sim
   C -> C : k = checksum(setup)
   C -> S : Loaded { checksum: k }
   alt k == K
     S --> C : Lobby (setup loaded)
   else
     S --> C : Error "setup checksum does not match"
   end
   @enduml

Room browsing and readiness
---------------------------

A connection can request ``Rooms { version: 5 }`` before creating or joining
a room. The response lists at most 64 public rooms with space that have not
started, sorted by code. Private rooms require their code. Creating or
joining supplies a display name. Lobby updates carry room visibility and
an optional named slot for each player start.

Loading the setup and choosing readiness are separate steps. After a matching
``Loaded``, the player can send ``Ready { ready: true }`` or clear readiness
with ``Ready { ready: false }``. Only the host can start, and the room must be
full with every player loaded and ready. Leaving a room keeps the connection
open for browsing, creating, or joining again.

Solo play
---------

Solo play needs no socket but uses the same machinery. The session builds a
``MatchSetup`` from the bundled files, delays local input by two ticks, and
creates turns with an input for every player (empty except the local one).
The result records to the same ``Replay`` type, so solo and online replays are
interchangeable.

What is not implemented
-----------------------

* **Hidden information.** Every client receives every player's commands and
  simulates the whole map. Fog of war (M9) needs a visibility-aware protocol
  where the server sends only what each player may see.
* **Recovery.** There are no corrective snapshots. A diverged client stops;
  reconnecting and joining a running match are not supported.
* **Encryption.** The server speaks plain WebSockets; use a TLS reverse proxy.
* **Adaptive delay.** The input delay is fixed at two ticks.

A modified client can change its own display but cannot change the server's
simulation.
