Match sessions
==============

A *session* decides where each turn comes from: the local player (solo), the
server (online), or a recording (playback). ``crates/client/src/session.rs``
implements all three with the same turn format, so solo play, online play,
and replays go through one code path: ``Turn::apply``, a hash, and a replay
recording.

Components
----------

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Item
     - Role
   * - ``SessionAction``
     - Interface messages for local games, connections, rooms, and the library.
   * - ``Session``
     - Mode, buffered turns, local input frames, pending online setup,
       current recording, last replay, playback, and timing.
   * - ``SessionStatus``
     - Interface view: ``message``, ``error``, ``server``, ``rooms``,
       ``lobby``, ``joining``, ``running``, ``stopped``, ``kind``,
       ``has_replay``, and ``importing``.
   * - ``Network``
     - Non-send resource holding the WebSocket ``Connection``, if any.
   * - ``Library``
     - Save and replay records, backed by ``Store`` in ``storage.rs``.
   * - ``ReplayIo``
     - Browser file picker and download bridge in ``replay_io.rs``.

``SessionStatus`` is a projection; ``Session::mode`` is the match state
machine. Server state is separate: ``Offline``, ``Connecting { url }``,
``Connected { url }``, or ``Failed { url, reason }``. ``Lobby`` holds the
``code``, local ``player`` slot, ``public`` visibility, and optional named
``slots``. Its helpers derive readiness and host start availability.
``MatchKind`` is ``Solo``, ``Online``, or ``Playback``.

Actions
-------

.. list-table::
   :header-rows: 1
   :widths: 45 55

   * - Action
     - Effect
   * - ``NewGame { map }``, ``LoadGame { id }``, ``SaveGame``
     - Start a catalog map, resume a save, or save a running solo match.
   * - ``Connect { url }``, ``Disconnect``, ``RefreshRooms``
     - Open or close the server connection, or request its public rooms.
   * - ``CreateRoom { name, public }``, ``JoinRoom { name, code }``
     - Create or join on the current connection with a display name.
   * - ``SetReady(bool)``, ``Start``, ``LeaveRoom``
     - Change explicit readiness, start as host, or leave the waiting room.
   * - ``Leave``
     - Return from the current match to the menu.
   * - ``WatchReplay { id }``, ``WatchLast``, ``SaveReplay``
     - Watch a library record or the last match, or add a recording to the
       replay shelf.
   * - ``Delete { shelf, id }``
     - Remove a save or replay record.
   * - ``ImportReplay``, ``ExportReplay { id }``
     - Import into the browser library or download a stored replay.

The ``SessionActions`` system set runs ``handle_actions`` followed by
``finish_replay_import``. Menu action processing runs before this set.

Modes
-----

.. kroki::
   :type: plantuml
   :caption: Session modes.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Idle
   Idle --> Offline : NewGame, LoadGame
   Idle --> Online : Started from server
   Idle --> Playback : WatchLast, WatchReplay
   Offline --> Idle : Leave
   Online --> Stopped : hash mismatch, stall,\ndisconnect, Ended
   Online --> Idle : Leave
   Playback --> Stopped : last turn verified
   Playback --> Idle : Leave
   Stopped --> Idle : Leave
   Stopped : match stays on screen,\norders disabled
   @enduml

Connection failures in the menu set ``Server::Failed`` without creating a
stopped match. Only ``Leave``, ``SaveReplay``, and ``SaveGame`` are allowed
while a match is on screen. Other actions require ``AppState::Menu`` and no
running match; otherwise they fail with *Leave the current match first.*
Saving a game also requires a running, unstopped solo match.

Starting a match
----------------

``Session::begin`` is shared by new solo and online matches:

1. Build the ``Sim`` and a new ``Replay`` from the ``MatchSetup``.
2. Archive the old recording into ``last_replay`` and clear buffers.
3. Install the new recording, mode, and step time.
4. Return a ``Match`` for the local player. The caller inserts it and
   switches to ``InGame``.

Solo play selects a setup from ``MapCatalog`` and controls slot 0. Online
play uses the server setup and the local slot from ``Lobby``.
``Session::begin_replay`` accepts a recording already verified by the library
or recorded locally. It starts a fresh simulation in ``Playback`` with
orders disabled. Starting any local match, save, or replay disconnects from
the server.

The fixed-step loop
-------------------

``advance_session`` runs in ``FixedUpdate`` at 20 Hz while ``InGame``. Each
call applies one turn, and in online mode repeats up to eight times while
buffered turns remain, so a slow frame catches up with bounded work.

.. kroki::
   :type: plantuml
   :caption: One call to advance for each mode.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   :next = sim.tick + 1;
   switch (mode)
   case (Offline)
     :frames[sim.tick + 2] = take pending;
     :turn = frames[next] for the local player,\nempty for others;
   case (Online)
     if (turn for next buffered?) then (no)
       if (> 15 s since last step?) then (yes)
         :fail: server stopped responding;
         stop
       elseif (> 0.25 s?) then (yes)
         :show "Waiting for the server…";
       endif
       stop
     endif
     :take turn and server hash;
     if (received + 2 > last submitted?) then (yes)
       :send Input { tick: received + 2,\ncommands: take pending };
     endif
   case (Playback)
     if (turn for next in replay?) then (no)
       if (hash == final_hash?) then (yes)
         :"Replay finished and verified.";
       else (no)
         :fail;
       endif
       stop
     endif
   case (Idle or Stopped)
     stop
   endswitch
   :turn.apply(sim);
   :hash = sim.state_hash();
   if (Online and hash != server hash?) then (yes)
     :fail: state differs from server;
     stop
   endif
   :recording.push(turn, hash);
   stop
   @enduml

On a fixed-step failure an online session closes its connection. The session
switches to ``Stopped``, disables orders, clears pending commands, and shows the reason. The match stays on screen until the player leaves.

Input delay
-----------

Solo play delays local input by two ticks, exactly as the server does, so
solo matches feel the same as online ones and record the same kind of turns:

* Commands queued while tick *T* is current are stored in ``frames[T + 2]``.
* The turn for tick *T + 1* takes ``frames[T + 1]``, which was filled two
  steps earlier.

Online, the client sends its pending commands once per applied turn, for the
latest *received* tick plus two. Using the received tick rather than the
applied tick keeps input ahead of the server even when the client has a
backlog. ``submitted_tick`` makes sure each tick is sent at most once.

Connection flow
---------------

``Connection::connect`` validates the address (``ws://`` or ``wss://``, at most
2048 bytes; native builds refuse ``wss://``) and opens an ``ewebsock``
socket. Socket events enter a bounded channel of 128 events; if it fills,
the socket stops reading. On open, the client sends
``Rooms { version: 5 }``. The first ``Rooms`` reply marks the server
``Connected``. Create and join use this same connection.

``poll_network`` runs in ``PreUpdate`` and handles up to 128 events per frame:

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Server message
     - Client reaction
   * - ``Rooms``
     - Mark a connecting server connected and replace the public room list.
   * - ``Setup``
     - Accept only before a match and once per join/create request; validate
       through ``simulation()`` and reply ``Loaded { checksum }``.
   * - ``Lobby``
     - Update code, local slot, visibility, names, and readiness. Ignore
       updates when neither joining nor already in a lobby. An update during
       a match fails the connection.
   * - ``Started``
     - Require a setup and lobby, matching player count, delay 2, and no
       running match. Begin online play with the assigned slot.
   * - ``Turn``
     - Require online mode, fewer than 128 buffered turns, and the next tick
       in sequence. Buffer the turn with its hash.
   * - ``Ended``
     - During online play, stop the match and show the reason; keep the
       connection. In a lobby, clear the lobby and setup, show the reason,
       and request ``Rooms`` again.
   * - ``Error``
     - While connecting, fail the connection with the refusal reason.
       Otherwise clear ``joining`` and show the error. The match continues.

A socket error, close, decode failure, unexpected message, or socket that
fails to open within 10 seconds closes the connection and sets
``Server::Failed``. A running online match stops with orders disabled.
Loading the setup and choosing **Ready** are separate steps; see
:doc:`../reference/protocol`.

Saved games and library
-----------------------

A saved game is a solo match's ``Replay``. ``Library::load`` calls
``Replay::from_ron_with_sim`` to verify it once and return both the recording
and its final ``Sim``. ``Session::resume`` continues from that simulation in
``Offline`` mode as player 1. Recorded turns remain in the recording and new
turns append to it. Online matches cannot be saved as games.

``library.rs`` keeps two shelves: ``saves/`` and ``replays/``. Each record
contains ``<id>.meta.ron`` and ``<id>.data.ron``. The data is replay RON;
``RecordMeta`` contains ``name``, ``map``, ``players``, ``ticks``,
``saved_at`` (Unix milliseconds), and ``setup`` for previews. IDs use
13-digit millisecond stamps, incremented when necessary to avoid collisions.
Names use one more than the highest automatic number in use, such as
**Save 01** or **Replay 03**. Data is written before metadata. Unreadable
metadata is skipped; records sort newest first.

``storage.rs`` bounds each document to 32 MiB. Native storage uses
``$XDG_DATA_HOME/rts``, then ``%LOCALAPPDATA%/rts``, then
``~/.local/share/rts``. Android uses the app's internal data path. Writes
use ``<file>.tmp`` followed by rename. Browsers use ``localStorage`` keys
``rts/<path>``. Values use ``miniz_oxide`` deflate and encode each byte as
one character in U+0100 through U+01FF.

``ReplayIo`` handles browser import and export only. Import reads a chosen
file asynchronously through a one-slot channel. ``finish_replay_import``
validates it and adds it to the replay shelf. Export downloads a stored
record as ``match.replay.ron``. Native builds expose neither button; their
records live in the data folder. See :doc:`../guides/replays`.

Leaving
-------

The interface confirms leaving a live solo or online match. Stopped matches
and replay playback leave at once. ``Leave`` archives the recording, sets
``Idle``, and returns to the menu. The **Last match** replay row offers
**Watch replay** and **Save to library**.

Leaving an online match sends ``Leave`` while it is live and refreshes
``Rooms`` if still connected. The server connection stays open. ``LeaveRoom``
also sends ``Leave`` and requests rooms without disconnecting. Host departure
closes a waiting room; any departure ends a live match for everyone.
``Disconnect`` closes the connection. Starting a local game or replay also
closes it.
