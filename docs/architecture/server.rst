Authoritative server
====================

``rts_server`` (``crates/server``) hosts rooms and runs the authoritative
simulation of every match. It has two layers:

* ``Coordinator`` (``coordinator.rs``): pure room and match logic. It has no
  sockets, no clock, and no OS randomness; callers pass the elapsed time and
  a seeded random generator for room codes. Tests drive it directly with
  exact timestamps and fixed seeds.
* The runtime (``runtime.rs``, native only): a Tokio task that accepts
  WebSocket connections and feeds the coordinator.

The binary (``main.rs``) parses the command line, validates the setup, binds
the port, and calls ``serve_with_config``. On ``wasm32`` the crate builds as a
stub so ``cargo check --workspace`` works for the web target.

Runtime tasks
-------------

.. kroki::
   :type: plantuml
   :caption: Server tasks and queues. The coordinator is owned by a single task.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   skinparam componentStyle rectangle
   node "serve_with_config task" {
     [TcpListener accept] as accept
     [Coordinator] as coord
     [10 ms clock] as clock
     [dispatch] as dispatch
   }
   node "connection task (one per socket)" as conn1 {
     [WebSocket] as ws1
     [heartbeat ping] as hb1
   }
   queue "events\n(capacity 256, shared)" as events
   queue "outbound\n(capacity 64, per socket)" as out1
   actor Client
   Client <--> ws1
   accept --> conn1 : spawn
   ws1 --> events : Message / Invalid / Activity
   events --> coord : receive / touch
   clock --> coord : expire, advance
   coord --> dispatch : Deliveries
   dispatch --> out1 : encoded frames
   out1 --> ws1
   @enduml

The main task runs a ``select!`` loop over four sources:

1. **Accept.** Refuse the socket if there are already ``max_connections``
   tasks. Otherwise allocate a ``ClientId``, register it with the
   coordinator, create its outbound queue, and spawn its connection task.
2. **Events.** Pass decoded messages to ``Coordinator::receive``; answer
   invalid frames with ``Error``; record activity with ``touch``.
3. **Task completion.** A finished connection task disconnects its client.
4. **Clock.** Every ``min(poll_interval, 10 ms)``, expire silent clients and
   advance every match.

Every coordinator call returns a list of ``Delivery { client, message }``.
``dispatch`` encodes each one (binary for setup, text otherwise) and
``try_send``\ s it to the client's outbound queue. A full queue or an encoding
failure disconnects that client immediately, and the resulting deliveries
(such as *a player disconnected*) are dispatched in the same pass. A slow
client therefore cannot make the server buffer without limit or block others.

Connection tasks
----------------

Each connection task:

* Completes the WebSocket handshake within ``handshake_timeout`` (5 s), with
  a 64 KiB maximum incoming message and frame size.
* Sends a ping every third of ``peer_timeout`` (every 5 s by default). Pongs
  and pings count as activity, so idle but healthy sockets stay alive.
  Browsers answer pings automatically.
* Decodes text frames with ``decode_client``. Binary frames and decoding
  failures become ``Invalid`` events.
* Forwards events with ``try_send``. If the shared event queue is full, the
  producing connection is closed rather than blocking every socket.
* Writes outbound messages with a ``peer_timeout`` deadline.

The task ends on close, error, or timeout; the main loop then disconnects the
client.

Rooms
-----

.. kroki::
   :type: plantuml
   :caption: Room lifecycle in the coordinator.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Waiting : Create (host takes slot 0)
   Waiting : public: bool\nslots: Vec<Option<Slot>>
   Waiting : Slot: client, name, loaded, ready
   Waiting --> Waiting : Join / Loaded / Ready /\nguest Leave or disconnect
   Waiting --> Running : host Start,\nroom full, all loaded and ready
   Running : Game { sim, last_input,\ninputs, next_turn }
   Running --> Running : Input, 20 Hz turns
   Waiting --> [*] : host leaves or disconnects
   Running --> [*] : anyone leaves or disconnects,\nsilent socket, or tick overflow
   @enduml

* The room size is the server map's player count. Occupied slots store the
  connection, display name, loaded flag, and ready flag.
* Connections outside a room can request ``Rooms``. The coordinator walks
  its ``BTreeMap`` in code order and returns at most 64 public rooms with
  space that have not started. The listed map name is read from ``MapData``
  at startup, truncated to 64 characters, with controls replaced by spaces.
* ``Loaded`` checks the setup checksum. ``Ready { ready: true }`` requires
  that acknowledgment; players may clear readiness before the start.
  Lobby updates contain each occupied slot's name and ready flag.
* Only the host can start a full room where every slot is loaded and ready.
* A waiting guest's departure frees the slot and clears both flags. An
  explicit leave sends that guest ``Ended`` and keeps their connection open.
* Room codes are six characters from ``A``–``Z`` and ``0``–``9``, about
  2.2 billion possibilities. Each character is drawn uniformly from a
  ChaCha-based ``StdRng`` that the runtime seeds from OS entropy at startup,
  so codes cannot be predicted from codes seen earlier. A code that a live
  room already uses is drawn again, so no two rooms share a code. A closed
  room's code may be reused later.
* Public rooms appear in room lists. Private rooms are joinable only by code,
  so share a private code only with the players you want.
* Every member of a room receives ``Ended`` with the reason when the room
  closes, and their connection becomes free to list, create, or join rooms again.
* Requests that fail (wrong version, unknown room, not the host, room not
  full, wrong checksum) return ``Error`` and leave state unchanged, so the
  client can retry.

Peers and timeouts
------------------

The coordinator tracks each client's room and the time it was last seen.
``expire`` disconnects clients that have been silent for:

* ``lobby_timeout`` (30 minutes) while waiting in a room that has not started.
* ``peer_timeout`` (15 seconds) otherwise.

Match progress never depends on input or hash reports, so a silent player
cannot stall the clock; it only ends the match when its socket is dropped.
When a match starts, every member's last-seen time is refreshed.

The match clock
---------------

``Game::advance`` produces turns on a fixed 50 ms schedule per room:

* The first turn (tick 1) is due 50 ms after ``Start``.
* Each turn applies the collected input for its tick—an empty batch for any
  player who sent nothing—to the room's ``Sim`` and broadcasts
  ``Turn { turn, hash }``.
* After a stall (for example a suspended process), at most
  ``MAX_CATCH_UP_TURNS = 4`` turns are produced in one call. If the schedule is
  still behind, the next deadline resets to one tick from now instead of
  bursting the backlog.

The tick rate never changes in response to input, so clients cannot speed up
or slow down a match.

Input handling
--------------

.. kroki::
   :type: plantuml
   :caption: Validation of Input { tick, commands } from a player's slot.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   if (tick > sim tick\nand tick > this slot's last input?) then (no)
     :Error: past, duplicate, or out of order;
     stop
   endif
   if (tick ≤ sim tick + 8?) then (no)
     :Error: exceeds future lead;
     stop
   endif
   if (batch fits command limits\nand the 64 KiB message limit?) then (no)
     :Error;
     stop
   endif
   :remove unit IDs the player does not own\nor that do not exist;
   :drop commands with no units left;
   :place batch in this slot of the tick's inputs;
   if (combined turn fits the\nturn message limit?) then (no)
     :Error;
     stop
   endif
   :store inputs; last_input[slot] = tick;
   stop
   @enduml

The server stores at most one batch per player per tick, in slot order. The
simulation repeats ownership checks when applying, and validates target
terrain and reachability. Rejected input does not count as a submission, so a
client can resend.

Configuration
-------------

``Config`` carries the ``MatchSetup`` (rules, map, seed, players) and all
resource limits; :doc:`../reference/server-cli` lists the defaults. The
default setup is the bundled rules and map, compiled into the binary with
``include_str!``. ``serve_with_config`` refuses zero limits and an invalid
setup before accepting connections; ``Coordinator::connect`` also refuses
clients if the setup cannot build a simulation.

Each room clones one validated initial ``Sim`` when its match starts, so
setup parsing happens once per server, not once per room.
