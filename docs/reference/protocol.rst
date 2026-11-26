Multiplayer protocol
====================

Clients and the server exchange compact binary messages over QUIC, defined in
``rts_net`` (``crates/net/src/lib.rs``, ``wire.rs``, ``assets.rs``, and
``quic.rs``). The current protocol version is **11**. Older clients are
incompatible. Version 8 and 9 replays still load; see :doc:`replay-format`.

Encoding and streams
--------------------

* Connections use pure-Rust QUIC + TLS 1.3 (``quinn``, ``rustls``, and
  ``glomar-rustls``) with two bidirectional stream kinds:

  * ``STREAM_GAME = 0`` (priority ``10``): ordered lobby, setup, input, and
    view-frame messages.
  * ``STREAM_ASSETS = 1`` (priority ``-10``): optional manifest and file
    download for server-provided textures (``textures/**/*.png``) and
    translations (``locales/*.lang.ron``), verified by SHA-256.

* Every frame on both streams is length-prefixed with an unsigned LEB128
  varint, followed by a single header byte (bits ``0..7`` = message tag,
  bit ``7`` = raw-deflate compressed whenever compression shrinks payloads
  of 64 bytes or more).
* Quiet ``View`` frames are 3 bytes on the wire; ``Start`` and ``Leave`` are
  1 byte. Clients skip sending empty ``Input`` batches when no commands are
  queued.
* ``Setup(LobbySetup)`` uses the same compact binary envelope with minified
  rules; see :doc:`setup-frame`.
* Every message is validated when encoded and when decoded, with strict
  length bounds checked before allocating buffers.

Shared types
------------

.. code-block:: rust

   struct UnitId(u32);            // a view ID on the wire (see below)
   struct PlayerId(u8);           // slot index in the map's player list

   enum Command {
       Move { units: Vec<UnitId>, target: IVec2 },  // target in world units
       Stop { units: Vec<UnitId> },
       Attack { units: Vec<UnitId>, target: UnitId },
       Gather { units: Vec<UnitId>, deposit: DepositId },
       BuildBarracks { units: Vec<UnitId>, target: IVec2 },
       TrainWorker { units: Vec<UnitId> },
       TrainRifleSquad { units: Vec<UnitId> },
       TrainScout { units: Vec<UnitId> },
   }

   struct LobbySetup { rules: String, map: String, players: u8 }
   struct ViewIntro {
       player: PlayerId, players: u8, size: IVec2, start: IVec2,
       skirmish: bool, fog_disabled: bool,
   }
   struct ViewFrame {
       tick: u32,
       explored: Vec<(u32, TerrainId)>,
       units: Vec<ViewUnit>, removed: Vec<UnitId>,
       deposits: Vec<ViewDeposit>,
       resources: Option<Resources>, headquarters: Option<UnitId>,
       outcome: Option<MatchOutcome>, volleys: Vec<Volley>,
   }

Unit and deposit IDs on the wire are the receiving player's *view IDs*. They
are aliases assigned in the order that player first observed each entity, so
they never reveal authoritative IDs. ``ViewFrame`` and its parts are described
in :doc:`../architecture/visibility`. ``Turn`` (one ``PlayerInput`` per player
in slot order) no longer crosses the network. It remains the record format of
full replays (:doc:`replay-format`).

The ``ViewIntro`` flags byte carries skirmish in bit 0 and the server's
fog-disabled setting in bit 1. With fog disabled, the initial view frame
contains every terrain tile in row-major order and all current units.

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
     - Acknowledge the received lobby setup with its checksum.
   * - ``Ready { ready }``
     - Set or clear this player's explicit readiness before the match.
   * - ``Start``
     - Host only: start a full room whose members have loaded and are ready.
   * - ``Input { tick, commands }``
     - Commands in this player's view IDs for a future tick.
   * - ``Leave``
     - Leave the room.

Clients never send rules, player slot identities, or simulation state.
The server identifies each player by the connection's slot.

Server messages
---------------

.. list-table::
   :header-rows: 1
   :widths: 32 68

   * - Message
     - Meaning
   * - ``Setup(LobbySetup)``
     - The server's rules text, the room's map name, and the player count.
       No terrain, units, starts, deposits, or seed. Binary frame only.
   * - ``Rooms { rooms }``
     - Up to 64 public rooms that have space and have not started, sorted by code.
   * - ``Lobby { code, player, public, slots }``
     - Room visibility and named slots with readiness. ``player`` is the
       receiver's slot. Empty slots are ``None``; capacity is ``slots.len()``.
   * - ``Started { players, input_delay, view }``
     - The match started. ``input_delay`` is always 2. ``view`` is the
       receiver's own ``ViewIntro``; it differs per player.
   * - ``View { frame }``
     - The receiver's knowledge after one authoritative tick, starting at
       tick 0. Sent only to that player.
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
receiver slots must be occupied. These messages retain the 64 KiB size limit;
``View`` may use up to 4 MiB.

Example text frames:

.. code-block:: text

   Join(version:9,code:"K7Q2PX",name:"Ada")
   Input(tick:42,commands:[Move(units:[(0),(1)],target:(11147,14223))])
   View(frame:(tick:40,explored:[],units:[],removed:[],deposits:[],resources:None,headquarters:None,outcome:None,volleys:[]))

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
   C -> S : Rooms { version: 10 }
   S --> C : Rooms { rooms }
   C -> S : Create { version: 10, name, public }\nor Join { version: 10, code, name }
   S --> C : Setup (compact binary + deflate)
   S --> C : Lobby { code, player, public, slots }
   C -> C : parse rules,\ncompute checksum
   C -> S : Loaded { checksum }
   S --> C : Lobby
   C -> S : Ready { ready: true }
   S --> C : Lobby { slots with updated readiness }
   ... other players join, load, and become ready ...
   C -> S : Start (host only)
   S --> C : Started { players, input_delay: 2, view }
   S --> C : View { frame for tick 0 }
   loop every tick (50 ms)
     S --> C : View { frame }
     C -> C : validate and apply frame
     opt pending commands non-empty
       C -> S : Input { tick: received + lead, commands }
     end
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
  previous ``Input``, and at most 16 ticks ahead. Gaps are allowed.
* The batch must fit the command limits and the per-message limit.
* The server translates each command with the sender's knowledge. View IDs
  that do not resolve, or name units the player does not own, are removed.
  ``Attack`` needs an enemy in sight now, ``Gather`` a deposit in sight now,
  and ``BuildBarracks`` a site in sight now; otherwise the command is
  dropped. Commands left with no units are dropped.
* A rejected ``Input`` returns ``Error`` and does not count as a submission.
  Dropped commands are silent: the answer must not depend on hidden state.

Ticks:

* ``Started`` is followed at once by each player's frame for tick 0. The
  first simulated tick is 1, due 50 ms after ``Started``.
* Ticks run every 50 ms whether or not input arrives. A player without input
  for a tick gets an empty batch.
* On the tick a command executes, the server revalidates it against the
  player's current knowledge, so a target that left sight during the input
  delay is refused. The simulation then validates ownership, target terrain,
  and reachability. Invalid orders have no effect.
* After each tick every player receives its own ``View`` frame.

Client rules
------------

* A setup is accepted once per room membership, before the match. The client
  parses its rules before sending ``Loaded``. Explicit readiness is a
  separate player choice.
* ``Started`` must match the setup's player count, an input delay of 2, and
  the client's lobby slot.
* ``View`` frames must arrive in tick order, starting at 0. At most 128 may
  be buffered. ``PlayerView::apply`` rejects frames that are out of sequence,
  explore a tile twice, remove unknown units, or carry units, deposits,
  volleys, or economy the view cannot hold. A rejected frame stops the
  client's match; the server is unaffected.
* After each applied frame, the client sends its pending commands for the
  latest received tick plus 2, at most once per tick.

Trust model
-----------

The server is the only source of match state. Clients cannot change another
player's units, positions, or health, cannot command units by guessing IDs,
cannot target what they cannot see, and cannot speed up or stall the clock.
A client receives only its own view: terrain it has explored, units in sight,
its own orders and economy, and volleys between visible positions. Activity
out of sight produces the same frames as no activity. By default every match
uses a fresh layout from a server-private seed; see
:doc:`../architecture/server`. Reconnection and late joining are not
implemented.

Limits are listed in :doc:`limits`.
