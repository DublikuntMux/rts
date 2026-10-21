Multiplayer protocol
====================

Clients and the server exchange WebSocket messages defined in ``rts_net``
(``crates/net/src/lib.rs``). The crate has no renderer or socket dependency;
the server runtime and the client session supply the transport. The current
protocol version is **5**. Older clients and replays are incompatible.

Encoding
--------

* Lobby and gameplay messages are RON text in WebSocket *text* frames. RON
  nesting is limited to 64 levels.
* The match setup is a binary frame, described in :doc:`setup-frame`.
* The server treats any binary frame from a client as invalid. Clients
  reject a text-encoded setup and unknown binary tags.
* Every message is validated when encoded and when decoded. Structures deny
  unknown fields.

Shared types
------------

.. code-block:: rust

   struct UnitId(u32);            // stable, never reused in a match
   struct PlayerId(u8);           // slot index in the map's player list

   enum Command {
       Move { units: Vec<UnitId>, target: IVec2 },  // target in world units
       Stop { units: Vec<UnitId> },
   }

   struct PlayerInput { player: PlayerId, commands: Vec<Command> }
   struct Turn { tick: u32, inputs: Vec<PlayerInput> }

A ``Turn`` contains exactly one ``PlayerInput`` per match player, in
ascending slot order. Commands keep their submission order. ``Turn::apply``
checks that the turn is for the simulation's next tick and applies it without
adding another input delay.

Client messages
---------------

.. list-table::
   :header-rows: 1
   :widths: 32 68

   * - Message
     - Meaning
   * - ``Rooms { version }``
     - List public waiting rooms while outside a room.
   * - ``Create { version, name, public }``
     - Create a room with this player display name and room visibility.
   * - ``Join { version, code, name }``
     - Join by six-character code (``A``–``Z``, ``0``–``9``) with this name.
   * - ``Loaded { checksum }``
     - Acknowledge the received setup with its checksum.
   * - ``Ready { ready }``
     - Set or clear this player's explicit readiness before the match.
   * - ``Start``
     - Host only: start a full room whose members have loaded and are ready.
   * - ``Input { tick, commands }``
     - Commands for a future tick.
   * - ``Leave``
     - Leave the room.

Clients never send rules, player slot identities, simulation state, or hashes.
The server identifies each player by the connection's slot.

Server messages
---------------

.. list-table::
   :header-rows: 1
   :widths: 32 68

   * - Message
     - Meaning
   * - ``Setup(MatchSetup)``
     - The server's rules, map, seed, and player count. Binary frame only.
   * - ``Rooms { rooms }``
     - Up to 64 public rooms that have space and have not started, sorted by code.
   * - ``Lobby { code, player, public, slots }``
     - Room visibility and named slots with readiness. ``player`` is the
       receiver's slot. Empty slots are ``None``; capacity is ``slots.len()``.
   * - ``Started { players, input_delay }``
     - The match started. ``input_delay`` is always 2.
   * - ``Turn { turn, hash }``
     - One authoritative turn and the server's state hash *after* applying it.
   * - ``Ended { reason }``
     - The room or match ended, or this client left the room.
   * - ``Error { message }``
     - A request was rejected. The connection and the match continue.

Room and slot data:

.. code-block:: rust

   struct RoomInfo {
       code: String, host: String, map: String, players: u8, capacity: u8,
   }
   struct LobbySlot { name: String, ready: bool }

Display names are non-empty and at most 24 characters. They contain no control
characters or leading or trailing whitespace. This applies to ``Create``,
``Join``, occupied lobby slots, and listed hosts. Listed map names are at most
64 characters with no control characters. Listed player counts are from 1 to
capacity; capacity is from 1 to 8. A lobby has 1 to 8 slots, and its host and
receiver slots must be occupied. These messages retain the 64 KiB size limit.

Example text frames:

.. code-block:: text

   Join(version:5,code:"K7Q2PX",name:"Ada")
   Input(tick:42,commands:[Move(units:[(0),(1)],target:(11147,14223))])
   Turn(turn:(tick:40,inputs:[(player:(0),commands:[]),(player:(1),commands:[Stop(units:[(7)])])]),hash:17134265448149945174)

Message sequence
----------------

.. kroki::
   :type: plantuml
   :caption: A client's view of one online match.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide footbox
   participant Client as C
   participant Server as S
   C -> S : Rooms { version: 5 }
   S --> C : Rooms { rooms }
   C -> S : Create { version: 5, name, public }\nor Join { version: 5, code, name }
   S --> C : Setup (binary)
   S --> C : Lobby { code, player, public, slots }
   C -> C : validate setup,\ncompute checksum
   C -> S : Loaded { checksum }
   S --> C : Lobby
   C -> S : Ready { ready: true }
   S --> C : Lobby { slots with updated readiness }
   ... other players join, load, and become ready ...
   C -> S : Start (host only)
   S --> C : Started { players, input_delay: 2 }
   loop every tick (50 ms)
     S --> C : Turn { turn, hash }
     C -> C : apply turn, compare hash
     C -> S : Input { tick: received + 2, commands }
   end
   alt a player leaves or disconnects
     S --> C : Ended { reason }
   end
   @enduml

Server rules
------------

Room membership:

* ``Rooms``, ``Create``, and ``Join`` require a matching version and a
  connection outside a room. Errors are "protocol version does not match" or
  "connection is already in a room". A ``Join`` also fails if the room does
  not exist, is full, or has started.
* Only public rooms with space and no running match appear in ``Rooms``.
  Private rooms are joinable by code. Results are sorted by code and capped
  at ``MAX_LISTED_ROOMS = 64``.
* Each creating or joining client receives ``Setup`` first, then every member
  receives ``Lobby``.
* ``Loaded`` must carry the server's checksum and is accepted only before the
  start. Clients never compare it against their bundled content.
* ``Ready { ready: true }`` requires a matching ``Loaded`` first, or returns
  "load the match setup first". Readiness can be cleared before the start.
  Readiness updates broadcast a ``Lobby`` to all members.
* ``Start`` requires slot 0, a full room, and every member loaded and ready.
  Missing explicit readiness returns "all players must be ready".
* A guest's ``Leave`` or disconnect before the start frees the slot and clears
  its loaded and ready flags. An explicit leave sends the guest
  ``Ended { reason: "left the room" }``.
* The host's departure before the start, or anyone's departure after it, ends
  the room for every member.
* Leaving keeps the connection open. After ``Ended``, members can list,
  create, or join rooms again on that connection.

Input:

* ``tick`` must be after the server's current tick, after this player's
  previous ``Input``, and at most 8 ticks ahead. Gaps are allowed.
* The batch must fit the command limits and the per-message limit.
* The server removes unit IDs the player does not own, or that no longer
  exist, and drops commands left empty.
* The combined turn with every player's input must still fit the turn message
  limit.
* A rejected ``Input`` returns ``Error`` and does not count as a submission.

Turns:

* The first turn is tick 1, due one tick (50 ms) after ``Started``.
* Turns are produced every 50 ms whether or not input arrives. A player
  without input for a tick gets an empty batch.
* The simulation validates the remaining commands: ownership, target terrain,
  and reachability. Invalid orders have no effect.

Client rules
------------

* A setup is accepted once per room membership, before the match. The client
  builds the full simulation from it before sending ``Loaded``. Explicit
  readiness is a separate player choice.
* ``Started`` must match the setup's player count and an input delay of 2.
* Each ``Turn`` must be the next tick after the last one received, with one
  input per room slot. At most 128 turns may be buffered.
* After applying a turn, the client compares its state hash with ``hash``. A
  mismatch stops the client's match; the server is unaffected.
* After each applied turn, the client sends its pending commands for the
  latest received tick plus 2, at most once per tick.

Trust model
-----------

The server is the only source of match state. Clients cannot change another
player's units, positions, or health, and cannot speed up or stall the clock.
A modified client can only change what it displays. All players currently
receive every player's commands, so there is no hidden information yet;
fog-of-war (M9) needs a visibility-aware protocol. Corrective snapshots,
reconnection, and late joining are not implemented.

Limits are listed in :doc:`limits`.
