Match sessions
==============

A *session* decides where the match on screen comes from: a local simulation
(solo), the server (online), or a recording (playback). In every mode the
interface reads only ``Match::view``, the local player's fog-filtered
``PlayerView`` (:doc:`visibility`).

* Solo matches and full-replay playback hold the authoritative ``Sim`` in a
  ``LocalAuthority`` and observe it after each ``Turn::apply``.
* Online matches and player-perspective playback apply ``ViewFrame``\ s
  directly; they never hold the ``Sim``.

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
     - Mode, local input frames, buffered server frames, the latest received
       tick, pending ``LobbySetup``, the current ``Recording``, the last
       recording (also used during playback), the opponent difficulty, and
       timing.
   * - ``Match``
     - The local player's ``PlayerView``, pending orders in view IDs, recent
       volleys, and, for local matches, the ``LocalAuthority``: the ``Sim``,
       the local player's ``PlayerKnowledge``, and one ``Seat`` (knowledge,
       view, ``rts_ai::Opponent``) per computer opponent.
   * - ``SessionStatus``
     - Interface view: ``message``, ``error``, ``server``, ``asset_progress``,
       ``rooms``, ``lobby``, ``joining``, ``running``, ``stopped``, ``kind``,
       ``result``, and ``has_replay``.
   * - ``Network``
     - Non-send resource holding the QUIC ``Connection`` handle and its
       smoothed RTT tracker, if any.
   * - ``ServerAssets``
     - Resource holding server-synced textures (``server_assets.rs``), cleared
       on disconnect or local play.
   * - ``Library``
     - Save and replay records, backed by ``Store`` in ``storage.rs``.

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
   * - ``NewGame { map, difficulty }``, ``LoadGame { id }``, ``SaveGame``
     - Start a catalog map against an opponent of that difficulty, resume a
       save at its stored difficulty, or save a running solo match.
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

The ``SessionActions`` system set runs ``handle_actions``. Menu action
processing runs before this set.

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
   Online --> Stopped : invalid frame, stall,\ndisconnect, Ended
   Online --> Idle : Leave
   Playback --> Stopped : recording finished
   Playback --> Idle : Leave
   Stopped --> Idle : Leave
   Stopped : match stays on screen,\norders disabled
   @enduml

Connection failures in the menu set ``Server::Failed`` without creating a
stopped match. Only ``Leave``, ``Restart``, ``SaveReplay``, and ``SaveGame`` are allowed
while a match is on screen. Other actions require ``AppState::Menu`` and no
running match; otherwise they fail with *Leave the current match first.*
Saving a game also requires a running, unstopped solo match. Restarting is
solo only and preserves the setup, recording version, and difficulty.

Starting a match
----------------

* ``Session::begin`` starts a solo match from a ``MatchSetup``. It creates a
  ``Replay``, builds the ``Sim`` from it (``Replay::simulation`` enables fog
  for current recordings), and plays slot 0. On a skirmish map it seats a
  computer opponent at the session's ``difficulty`` in every other slot.
* ``Session::begin_online`` starts from the server's ``LobbySetup`` and the
  ``ViewIntro`` in ``Started``. It records a ``ViewReplay`` and creates a
  ``Match`` with an empty view. The tick 0 frame arrives next.
* ``Session::resume`` continues a saved game. See below.
* ``Session::begin_replay`` accepts a recording already verified by the
  library or recorded locally, with orders disabled. A full replay starts a
  fresh local simulation viewed by player 1. A player-perspective replay
  starts an empty view from its intro.

Each of them archives the old recording into ``last_replay`` and clears the
buffers. Starting any local match, save, or replay disconnects from the
server. Playback reads the retained last replay directly instead of cloning a
second copy of the recording.

The fixed-step loop
-------------------

``advance_session`` runs in ``FixedUpdate`` at 20 Hz while ``InGame``. Each
call advances one tick. In online mode it repeats up to eight times while
buffered frames remain, so a slow frame catches up with bounded work.

.. kroki::
   :type: plantuml
   :caption: One call to advance for each mode.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   switch (mode)
   case (Offline)
     :frames[view.tick + 2] = take pending;
     :local orders = translate frames[next]\nwith the local player's knowledge;
     :each seat: opponent.commands(seat view),\ntranslated with its knowledge;
     :turn.apply(sim); observe every player;
     :recording.push(turn, state hash);
     if (outcome?) then (yes)
       :show result, stop;
     endif
   case (Online)
     if (frame for next tick buffered?) then (no)
       if (> 15 s since last step?) then (yes)
         :fail: server stopped responding;
         stop
       elseif (> 0.25 s?) then (yes)
         :show "Waiting for the server…";
       endif
       stop
     endif
     if (received + 2 > last submitted?) then (yes)
       :send Input { tick: received + 2,\ncommands: take pending };
     endif
     :view.apply(frame) (fails on an invalid frame);
     :recording.push(frame);
   case (Playback)
     if (full replay) then (yes)
       if (turn for next tick?) then (no)
         :verify final hash;\n"Replay finished and verified.";
         stop
       endif
       :turn.apply(sim); observe;
     else (view replay)
       if (frame left?) then (no)
         :"Replay finished.";
         stop
       endif
       :view.apply(frame);
     endif
   case (Idle or Stopped)
     stop
   endswitch
   stop
   @enduml

On a fixed-step failure an online session closes its connection. The session
switches to ``Stopped``, disables orders, clears pending commands, and shows the reason. The match stays on screen until the player leaves.

Input delay
-----------

Solo play delays local input by two ticks, exactly as the server does, so
solo matches feel the same as online ones:

* Commands queued while tick *T* is current are stored in ``frames[T + 2]``.
* The turn for tick *T + 1* takes ``frames[T + 1]``, which was filled two
  steps earlier, and translates it then. Translating on the executing tick
  applies the same current-sight rules as the server's revalidation.

Online, the client sends pending commands only when ``Match::pending`` is
non-empty, targeting the latest *received* tick plus an RTT-adaptive lead
(``2..=16`` ticks). Using the received tick rather than the applied tick keeps
input ahead of the server even when the client has a backlog. ``submitted_tick``
makes sure each tick is sent at most once.

Connection flow
---------------

``Connection::connect`` parses the server address (``quic://host[:port][#sha256]``
or bare ``host[:port]``, defaulting to port ``9001``) and spawns a background
Tokio QUIC client task. After completing the TLS 1.3 handshake and opening
``STREAM_GAME``, the client runs asset sync on ``STREAM_ASSETS``, verifies
hashes against ``<data_dir>/asset_cache/``, downloads only missing textures and
translations, emits ``SyncedAssets`` to Bevy, and then sends
``Rooms { version: 12 }``. The first ``Rooms`` reply marks the server
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
     - Accept only before a match and once per join/create request; parse
       the rules and reply ``Loaded { checksum }``.
   * - ``Lobby``
     - Update code, local slot, visibility, names, and readiness. Ignore
       updates when neither joining nor already in a lobby. An update during
       a match fails the connection.
   * - ``Started``
     - Require a setup and lobby, matching player count, delay 2, a view for
       the lobby's slot, and no running match. Begin online play.
   * - ``View``
     - Require online mode, fewer than 128 buffered frames, and the next tick
       in sequence (0 first). Buffer the frame.
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

A saved game is a solo match's full ``Replay``. ``Library::load`` verifies
any ``Recording`` with ``Recording::from_ron``. ``Session::resume`` replays the
saved turns from the start, in the live order: opponent orders, then apply,
then observe. Afterwards every player's knowledge and each opponent's memory
match the live match. It checks the final hash and continues in ``Offline``
mode as player 1. Recorded turns remain in the recording and new turns append
to it. Online matches cannot be saved as games. Their player-perspective
replays can be watched, saved to the replay shelf, imported, and exported.

``library.rs`` keeps two shelves: ``saves/`` and ``replays/``. Each record
contains ``<id>.meta.ron`` and ``<id>.data.ron``. The data is replay RON;
``RecordMeta`` contains ``name``, ``map``, ``players``, ``ticks``,
``saved_at`` (Unix milliseconds), ``setup`` for previews, and, for saved
games, the opponent's ``difficulty`` (absent in older saves, which resume at
Normal). IDs use
13-digit millisecond stamps, incremented when necessary to avoid collisions.
Names use one more than the highest automatic number in use, such as
**Save 01** or **Replay 03**. Data is written before metadata. Unreadable
metadata is skipped; records sort newest first.

``storage.rs`` bounds each document to 32 MiB. Desktop storage uses
``$XDG_DATA_HOME/rts``, then ``%LOCALAPPDATA%/rts``, then
``~/.local/share/rts``. Android uses the app's internal data path. Writes
use ``<file>.tmp`` followed by rename. See :doc:`../guides/replays`.

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
