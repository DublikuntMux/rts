Host a multiplayer match
========================

Online matches run on ``rts-server``, an authoritative WebSocket server. It
owns the map, rules, clock, and simulation. Clients send orders and replay the
server's turns locally to draw the match. This page covers running the server
and playing through it; :doc:`../architecture/server` and
:doc:`../architecture/networking` explain the design.

Start the server
----------------

From the repository root:

.. code-block:: sh

   cargo run --release -p rts_server --bin rts-server -- 127.0.0.1:9001

The bind address is optional. Without it the server uses the
``RTS_SERVER_ADDR`` environment variable, then ``127.0.0.1:9001``. The server
validates its map and rules before it opens the port and prints:

.. code-block:: text

   RTS server listening on ws://127.0.0.1:9001 (use a reverse proxy for TLS)

:doc:`../reference/server-cli` lists every option, including ``--rules``,
``--map``, and ``--seed`` for content mods.

Choose where to listen
----------------------

.. list-table::
   :header-rows: 1
   :widths: 30 35 35

   * - Players
     - Bind address
     - Clients enter
   * - Same machine
     - ``127.0.0.1:9001``
     - ``ws://127.0.0.1:9001``
   * - Local network
     - ``0.0.0.0:9001``
     - ``ws://<server LAN IP>:9001``
   * - Internet, native clients
     - ``0.0.0.0:9001`` with the port forwarded
     - ``ws://<public address>:9001``
   * - Internet, HTTPS browser clients
     - ``127.0.0.1:9001`` behind a TLS reverse proxy
     - ``wss://<domain>/<path>``

Native clients support ``ws://`` only. Browser clients support ``ws://`` and
``wss://`` through the browser's WebSocket, but a page served over HTTPS may
only open ``wss://``. The server speaks plain WebSockets, so terminate TLS in a
reverse proxy such as Caddy or nginx and forward the upgraded connection to the
server. Native TLS is not built in because the standard Rustls crypto providers
contain C and assembly, which the pure-Rust requirement excludes.

Play through a room
-------------------

.. kroki::
   :type: plantuml
   :caption: Lobby flow for a two-player room.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide footbox
   actor Host
   participant "rts-server" as S
   actor Guest
   Host -> S : Browse public rooms
   S --> Host : Available rooms
   Host -> S : Create room (name, visibility)
   S --> Host : map and rules (binary setup)
   S --> Host : Room K7Q2PX, named player slots
   Host -> S : Loaded (setup checksum)
   Host -> S : Ready
   Host ->> Guest : shares code K7Q2PX
   Guest -> S : Join room K7Q2PX (name)
   S --> Guest : map and rules
   Guest -> S : Loaded (setup checksum)
   Guest -> S : Ready
   S --> Host : Both players ready
   S --> Guest : Both players ready
   Host -> S : Start match
   S --> Host : Started
   S --> Guest : Started
   loop every 50 ms
     S --> Host : Turn + state hash
     S --> Guest : Turn + state hash
   end
   @enduml

1. Every player sets **Player name** under **Settings**, then opens
   **Multiplayer**, enters the same server address, and chooses **Connect**.
2. In the room browser, the host chooses **+ Create room**, chooses
   **Public** or **Private**, then **Create room**. Guests select a public
   room and choose **Join room**, or choose **Join by code** and enter the
   six-character invite code in the dialog. Private rooms require that code.
3. Each client loads the server's map and rules, validates them, and confirms
   with a checksum. Room size comes from the number of player starts in the
   server's map (two on *Proving Grounds*).
4. Each player chooses **Ready** after loading. **Not ready** clears
   readiness while waiting. The room shows each player's name and readiness.
5. The host chooses **Start match** when the room is full and every player has loaded and
   is ready.

The match runs at 20 ticks per second whether or not anyone sends orders.
Each order is scheduled two ticks after the latest turn the client has
received, so orders take effect about 100 ms later plus network latency.

Clients can have different bundled content from the server. The server sends
its own map and rules, and clients use them for the match. Clients only need
compatible game code and the same protocol version (currently 5).

Leaving and disconnects
-----------------------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Event
     - Result
   * - A guest leaves before the start
     - Their slot frees and load and readiness flags clear; others see the update.
   * - The host leaves before the start
     - The room closes for everyone.
   * - Anyone leaves or disconnects during a match
     - The match ends for every player.
   * - A socket stays silent for 15 seconds in a match
     - The server drops it, which ends the match.
   * - A lobby member stays silent for 30 minutes
     - The server drops it.

Leaving keeps the server connection open. Players can browse, create, or
join another room on that connection. Recovering a running match after a
disconnect and joining a running match are not supported yet.

Messages you may see
--------------------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Message
     - Meaning
   * - *Enter a server address beginning with ws:// or wss://.*
     - The address has no WebSocket scheme.
   * - *This native build uses ws:// servers.*
     - Native clients cannot open ``wss://``. Use ``ws://`` or the browser.
   * - *Server connection timed out.*
     - The socket did not open within 10 seconds.
   * - *room does not exist*, *room is full*, *match has already started*
     - The join was refused. Check the code or create a new room.
   * - *protocol version does not match*
     - Client and server come from incompatible builds.
   * - *Waiting for the server…*
     - No turn has arrived for a quarter of a second. Play resumes when turns
       arrive.
   * - *The server stopped responding. Play has stopped.*
     - No turn arrived for 15 seconds. The client stops its replica.
   * - *Match state differs from the authoritative server.*
     - The client's state hash differs from the server's for the same turn.
       This indicates a determinism bug; see
       :doc:`../architecture/determinism`.
   * - *a player disconnected*, *a player left the room*
     - Another player left; the server ended the room.

Limits
------

The default server accepts up to 1024 connections and 256 rooms, and a room
holds up to eight players. Each player may send up to 64 commands per tick.
:doc:`../reference/limits` lists every bound.
