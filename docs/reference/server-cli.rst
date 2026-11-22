``rts-server`` and ``rts server`` command line
==============================================

Both the standalone ``rts-server`` binary and the ``server`` subcommand of the
game binary ``rts`` share the same ``clap`` CLI definition:

.. code-block:: text

   rts-server [BIND_ADDRESS] [--rules PATH] [--map PATH] [--seed N] [--assets DIR]
   rts server [BIND_ADDRESS] [--rules PATH] [--map PATH] [--seed N] [--assets DIR]

Run either from the repository with Cargo:

.. code-block:: sh

   cargo run --release -p rts_server --bin rts-server -- 0.0.0.0:9001 --map mods/battle.map.ron --assets mods/my-mod
   cargo run --release -p rts_client --bin rts -- server 0.0.0.0:9001 --assets mods/my-mod

Pass ``--help`` to either binary to print the generated usage summary. Running
``rts`` with no subcommand starts the graphical game client.

Arguments
---------

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Argument
     - Meaning
   * - ``bind-address``
     - UDP address to listen on, such as ``0.0.0.0:9001``. Defaults to the
       ``RTS_SERVER_ADDR`` environment variable, then ``127.0.0.1:9001``.
   * - ``--rules PATH``
     - Rules file sent to every client. Defaults to the bundled
       ``assets/data/core.rules.ron``.
   * - ``--map PATH``
     - Play this map in every room instead of a private random layout. The
       map's number of player starts sets the room size. Clients receive
       only its name; anyone with the file knows its layout.
   * - ``--seed N``
     - Unsigned 64-bit match seed for a fixed map. Defaults to 0. It stays on
       the server and is reserved for simulation randomness.
   * - ``--assets DIR``
     - Directory containing optional ``textures/**/*.png`` and
       ``locales/*.lang.ron`` client assets served to connecting clients over
       ``STREAM_ASSETS``.

Without ``--map``, every match plays a fresh two-player *Random frontier*
layout generated from a seed drawn from OS entropy at the start of that
match. The seed is never sent to clients, so the layout is known only through
each player's fog of war. See :doc:`../architecture/server`.

Each option may appear once, in any order. Unknown options, a second
address, a missing value, or an invalid seed print the usage line and exit.

The bundled rules and the Frontier map are compiled into the server binary,
so the server runs without the ``assets/`` folder unless you override them.

Startup validation
------------------

Before it binds the port, the server:

1. Reads each document, refusing files larger than 4 MiB.
2. Parses the map to count its player starts.
3. Builds the complete initial simulation from the rules and map, which runs
   every content check described in :doc:`rules-format` and
   :doc:`map-format`.
4. With random layouts, also generates and validates a layout, so rules that
   lack the standard terrain symbols or skirmish units are refused.

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
     - Time to complete the QUIC/TLS 1.3 handshake and open ``STREAM_GAME``.
   * - ``poll_interval``
     - 10 ms
     - Coordinator clock interval; values above 10 ms are capped at 10 ms.

All limits and timeouts must be positive. :doc:`../architecture/server`
describes how they are enforced.

TLS and certificate pinning
---------------------------

The server always encrypts traffic with TLS 1.3 over QUIC using a pure-Rust
Ed25519 self-signed certificate generated at startup. Its 64-hex SHA-256
fingerprint is printed on startup; clients can pin it by appending
``#<sha256>`` to the ``quic://host:port`` address.
