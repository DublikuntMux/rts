``rts-server`` command line
===========================

.. code-block:: text

   rts-server [bind-address] [--rules PATH] [--map PATH] [--seed u64]

Run it from the repository with Cargo:

.. code-block:: sh

   cargo run --release -p rts_server --bin rts-server -- 0.0.0.0:9001 --map mods/battle.map.ron

Arguments
---------

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Argument
     - Meaning
   * - ``bind-address``
     - TCP address to listen on, such as ``0.0.0.0:9001``. Defaults to the
       ``RTS_SERVER_ADDR`` environment variable, then ``127.0.0.1:9001``.
   * - ``--rules PATH``
     - Rules file sent to every client. Defaults to the bundled
       ``assets/data/core.rules.ron``.
   * - ``--map PATH``
     - Map file sent to every client. Defaults to the bundled
       ``assets/maps/proving_grounds.map.ron``. The map's number of player
       starts sets the room size.
   * - ``--seed N``
     - Unsigned 64-bit match seed. Defaults to 0. It is included in the
       setup checksum and replays, and reserved for simulation randomness.

Each option may appear once, in any order. Unknown options, a second
address, a missing value, or an invalid seed print the usage line and exit.

The bundled rules and map are compiled into the server binary, so the server
runs without the ``assets/`` folder unless you override them.

Startup validation
------------------

Before it binds the port, the server:

1. Reads each document, refusing files larger than 4 MiB.
2. Parses the map to count its player starts.
3. Builds the complete initial simulation from the rules and map, which runs
   every content check described in :doc:`rules-format` and
   :doc:`map-format`.

Any failure exits with status 1 and a message such as:

.. code-block:: text

   failed to run server: unit 0 is outside the map or on blocked terrain

Runtime defaults
----------------

The binary uses the default ``rts_server::Config``. These values are not
command-line options yet; programs that embed the server pass their own
``Config`` to ``serve_with_config``.

.. list-table::
   :header-rows: 1
   :widths: 35 20 45

   * - Field
     - Default
     - Meaning
   * - ``max_connections``
     - 1024
     - Open sockets, including handshakes in progress.
   * - ``max_rooms``
     - 256
     - Rooms in the lobby or in a match.
   * - ``event_queue_capacity``
     - 256
     - Decoded messages waiting for the coordinator.
   * - ``outbound_queue_capacity``
     - 64
     - Messages waiting to be written to one socket.
   * - ``peer_timeout``
     - 15 s
     - Silence before a socket in a match or outside a room is dropped. Also
       bounds each socket write; pings go out every third of it.
   * - ``lobby_timeout``
     - 30 min
     - Silence before a socket waiting in a room is dropped.
   * - ``handshake_timeout``
     - 5 s
     - Time to complete the WebSocket handshake.
   * - ``poll_interval``
     - 10 ms
     - Coordinator clock interval; values above 10 ms are capped at 10 ms.

All limits and timeouts must be positive. :doc:`../architecture/server`
describes how they are enforced.

TLS
---

The server accepts plain WebSockets only. For ``wss://`` clients, run it
behind a TLS reverse proxy that forwards WebSocket upgrades.
