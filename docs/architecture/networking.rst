Networking model
================

RTS uses **server authority with private per-player views**. Only the server
runs the match. Every tick it sends each player a *view frame*: the changes to
what that player can see. Clients never receive the full map, other players'
commands, the seed, or a state hash. A client therefore cannot reveal fogged
terrain, hidden units, enemy orders, or enemy economy, even if it is modified.

This page follows one command end to end. The message formats are in
:doc:`../reference/protocol`. The server internals are in :doc:`server`, the
client internals in :doc:`sessions`, and the fog rules in :doc:`visibility`.

Why this model
--------------

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Property
     - How it is achieved
   * - Hidden information stays hidden
     - The server sends each player only its own ``ViewFrame`` messages
       (:doc:`visibility`). Lobby setup carries the public rules and the
       map's name, nothing else.
   * - Cheating is limited
     - Only the server's simulation exists. It binds each command to the
       sender's knowledge: own units only, and targets in sight. It checks
       again when the command executes, and it owns the clock.
   * - Slow players cannot stall others
     - The server advances at 20 Hz whether or not input arrives.
   * - Browsers can play
     - WebSockets work everywhere; the protocol has no UDP or WebRTC
       dependency.
   * - Replays are possible
     - Each client records the frames it received as a player-perspective
       replay. Local matches record full deterministic turns.

The cost is bandwidth. A frame lists every visible unit that changed, so it
grows with the number of moving units in sight. Quiet frames are only a few
bytes. Commands still take effect two ticks after the client schedules them,
plus network delay.

A command's journey
-------------------

.. kroki::
   :type: plantuml
   :caption: A Move issued by player 1 while the client has received the frame for tick 40.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide footbox
   actor "Player 1" as P
   participant "Client 1\n(Match, Session)" as C1
   participant "Server\n(Coordinator, Game)" as S
   participant "Client 2" as C2
   P -> C1 : right-click ground
   C1 -> C1 : queue Move (view IDs) in Match.pending
   S --> C1 : View { frame 41 }
   C1 -> C1 : apply frame 41 to PlayerView
   C1 -> S : Input { tick: 43, [Move] }
   S -> S : translate with player 1's knowledge:\nown units, targets in sight;\nstore in inputs[43][slot 0]
   S --> C1 : View { frame 42 }
   S --> C2 : View { frame 42 }
   S -> S : tick 43 due:\nrevalidate, apply turn,\nobserve each player
   S --> C1 : View { frame 43: own units moving }
   S --> C2 : View { frame 43: only what player 2 sees }
   @enduml

1. Input appends the command, written in view IDs, to ``Match::pending``.
   Nothing moves yet.
2. When the client applies its next frame, it sends all pending commands for
   *latest received tick + 2*.
3. The server validates the batch. It translates each command with the
   sender's ``PlayerKnowledge``: view IDs become authoritative IDs, foreign
   units are dropped, and commands that target something out of sight are
   refused. The result is stored for that tick.
4. When the tick is due, the server revalidates each stored command against
   the player's current knowledge, applies the turn, and observes the match
   once per player.
5. Each client receives only its own frame and applies it to its
   ``PlayerView``.

Input delay timeline
--------------------

.. kroki::
   :type: plantuml
   :caption: Ticks on the server and one client, with time in milliseconds. The client always schedules input two ticks past the latest frame it has received.
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
   @enduml

Input sent after receiving frame *N* targets tick *N + 2*, which the server
simulates 100 ms after tick *N*. If input arrives after its tick has been
simulated, the server rejects it with ``Error`` and the client keeps playing;
the command is lost. Inputs more than eight ticks ahead are also rejected.
Adaptive input delay is not implemented.

Shared clock, separate pacing
-----------------------------

The server is the only clock. Clients run ``FixedUpdate`` at 20 Hz too, but a
client's fixed step only *consumes* frames:

* If the next frame has not arrived, the client waits without advancing,
  letting visuals settle, and shows *Waiting for the server…* after 0.25 s.
* If several frames are buffered after a slow frame, it applies up to eight
  per fixed step to catch up.
* After 15 seconds without progress it stops.

The server bounds its own catch-up after stalls to four ticks per clock
check, so a suspended server cannot flood clients with a burst.

Setup delivery
--------------

When a player creates or joins a room, the server sends a ``LobbySetup``: the
exact rules text, the room's map name, and the player count. It is sent as a
compressed binary frame (:doc:`../reference/setup-frame`). The client parses
the rules and acknowledges with ``Loaded { checksum }``. The setup never
includes the map's terrain, units, deposits, starts, or the seed. A server
can run rules mods without client rebuilds. The protocol version and the
simulation code must still match.

When the match starts, each player receives
``Started { players, input_delay, view }``. ``view`` is that player's
``ViewIntro``: its slot, the map size, and its own start. Its first
``View`` frame for tick 0 follows immediately.

.. kroki::
   :type: plantuml
   :caption: Setup acknowledgment and start.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide footbox
   participant Client as C
   participant Server as S
   S -> S : startup: validate rules and map\n(or the random layout generator),\ncompute LobbySetup checksum K
   C -> S : Join { version: 9, code, name }
   S --> C : Setup (rules + map name, zlib)
   C -> C : inflate with bounds, parse rules
   C -> S : Loaded { checksum: k }
   alt k == K
     S --> C : Lobby (setup loaded)
   else
     S --> C : Error "setup checksum does not match"
   end
   ... all ready, host starts ...
   S -> S : build the match (fresh private layout by default)
   S --> C : Started { players, input_delay: 2, view }
   S --> C : View { frame for tick 0 }
   @enduml

Room browsing and readiness
---------------------------

A connection can request ``Rooms { version: 9 }`` before creating or joining
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

Solo play needs no socket. The session builds the ``Sim`` from the bundled
files and keeps it in a ``LocalAuthority``. It delays local input by two ticks
and builds one ``Turn`` per tick from the local player's orders and the
computer opponents' orders. The local player still sees only its own
``PlayerView``. Solo matches record full ``Replay``\ s of turns, so they can
be saved, resumed, and verified by hash.

What is not implemented
-----------------------

* **Recovery.** A client that falls behind or loses its connection stops;
  reconnecting and joining a running match are not supported.
* **Encryption.** The server speaks plain WebSockets; use a TLS reverse proxy.
* **Adaptive delay.** The input delay is fixed at two ticks.
* **Compact frames.** Frames are RON text. Binary or compressed frames would
  cut bandwidth for large battles.

A modified client can change its own display, but it cannot change the server's
simulation or see what its player has not seen.
