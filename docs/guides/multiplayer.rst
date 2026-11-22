Host a multiplayer match
========================

Online matches run on ``rts-server``, an authoritative QUIC server with built-in
TLS 1.3 encryption. It owns the map, rules, clock, and simulation, and can
optionally send custom textures and translations to connecting clients. Clients
send orders and draw the match from private view frames: each player receives
only what its own units can see. This page covers running the server and
playing through it; :doc:`../architecture/server` and
:doc:`../architecture/networking` explain the design.

Start the server
----------------

From the repository root:

.. code-block:: sh

   cargo run --release -p rts_server --bin rts-server -- 127.0.0.1:9001

The bind address is optional. Without it the server uses the
``RTS_SERVER_ADDR`` environment variable, then ``127.0.0.1:9001``. The server
validates its rules, map generator, and optional ``--assets`` directory before
opening the UDP port and prints:

.. code-block:: text

   RTS QUIC server listening on quic://127.0.0.1:9001
   Certificate SHA-256: <64-hex-fingerprint>

By default every match plays a fresh *Random frontier* layout that only the
server knows, so fog of war hides the map too. ``--map`` plays a fixed map
instead; anyone with that map file knows its layout. ``--assets DIR`` serves
custom ``textures/**/*.png`` and ``locales/*.lang.ron`` files to connecting
clients. :doc:`../reference/server-cli` lists every option.

Choose where to listen
----------------------

.. list-table::
   :header-rows: 1
   :widths: 30 35 35

   * - Players
     - Bind address (UDP)
     - Clients enter
   * - Same machine
     - ``127.0.0.1:9001``
     - ``quic://127.0.0.1:9001``
   * - Local network
     - ``0.0.0.0:9001``
     - ``quic://<server LAN IP>:9001``
   * - Internet
     - ``0.0.0.0:9001`` with UDP port 9001 forwarded
     - ``quic://<public address>:9001#<sha256>``

Connections are always encrypted with TLS 1.3 using pure-Rust cryptography
(``quinn``, ``rustls``, and ``glomar-rustls``). Appending ``#<sha256>`` pins
the server's certificate fingerprint printed at startup so clients reject
mismatched certificates.

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
   S --> Host : rules and map name (binary setup)
   S --> Host : Room K7Q2PX, named player slots
   Host -> S : Loaded (setup checksum)
   Host -> S : Ready
   Host ->> Guest : shares code K7Q2PX
   Guest -> S : Join room K7Q2PX (name)
   S --> Guest : rules and map name
   Guest -> S : Loaded (setup checksum)
   Guest -> S : Ready
   S --> Host : Both players ready
   S --> Guest : Both players ready
   Host -> S : Start match
   S --> Host : Started (host's perspective)
   S --> Guest : Started (guest's perspective)
   loop every 50 ms
     S --> Host : host's view frame
     S --> Guest : guest's view frame
   end
   @enduml

1. Every player sets **Player name** under **Settings**, then opens
   **Multiplayer**, enters the same server address, and chooses **Connect**.
2. In the room browser, the host chooses **+ Create room**, chooses
   **Public** or **Private**, then **Create room**. Guests select a public
   room and choose **Join room**, or choose **Join by code** and enter the
   six-character invite code in the dialog. Private rooms require that code.
3. Each client loads the server's rules, validates them, and confirms with a
   checksum. The map itself is never sent. Room size comes from the number
   of player starts in the server's map (two for random layouts).
4. Each player chooses **Ready** after loading. **Not ready** clears
   readiness while waiting. The room shows each player's name and readiness.
5. The host chooses **Start match** when the room is full and every player has loaded and
   is ready.

The match runs at 20 ticks per second whether or not anyone sends orders.
Each order is scheduled two ticks after the latest frame the client has
received, so orders take effect about 100 ms later plus network latency.
Unexplored terrain is black, explored terrain out of sight is dimmed, and
enemy units disappear when they leave sight.

Clients can have different bundled content from the server. The server sends
its own rules (plus any custom textures and translations configured with
``--assets``), and clients use them for the match. Clients only need
compatible game code and the same protocol version (currently 10).

Each client records the match as it saw it. **Watch replay** and **Save to
library** on the last match keep that private replay; it never contains the
opponent's hidden units or the map beyond what you explored.

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
   * - *Invalid server address* / *Certificate fingerprint must be 64 hex characters*
     - Check the ``quic://host:port`` address and optional ``#<sha256>`` pin.
   * - *Server connection timed out.*
     - The QUIC handshake did not complete within 10 seconds (check UDP port).
   * - *room does not exist*, *room is full*, *match has already started*
     - The join was refused. Check the code or create a new room.
   * - *protocol version does not match*
     - Client and server come from incompatible builds.
   * - *Waiting for the server…*
     - No frame has arrived for a quarter of a second. Play resumes when
       frames arrive.
   * - *The server stopped responding. Play has stopped.*
     - No frame arrived for 15 seconds. The client stops the match.
   * - *Invalid server view*
     - A frame failed validation, such as one out of order. This indicates an
       incompatible or broken server.
   * - *a player disconnected*, *a player left the room*
     - Another player left; the server ended the room.

Limits
------

The default server accepts up to 1024 connections and 256 rooms, and a room
holds up to eight players. Each player may send up to 64 commands per tick.
:doc:`../reference/limits` lists every bound.
