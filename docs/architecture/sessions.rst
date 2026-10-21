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
     - Bevy message sent by the interface: ``Solo``, ``Host { url }``,
       ``Join { url, code }``, ``Start``, ``Leave``, ``ReplayLast``,
       ``SaveReplay``, ``LoadReplay``.
   * - ``Session``
     - Resource with the mode, buffered turns, local input frames, the
       pending online setup, the current recording, the last replay, and the
       replay being played.
   * - ``SessionStatus``
     - Resource the interface reads: message, room code, slot, player
       counts, and availability flags (``can_start``, ``has_replay``,
       ``running``, ``playback``, ``busy``).
   * - ``Network``
     - Non-send resource holding the WebSocket ``Connection``, if any.
   * - ``ReplayIo``
     - Saves and loads replay text; see :doc:`../guides/replays`.

``SessionStatus`` is a projection for the interface; ``Session::mode`` is the
real state machine.

Modes
-----

.. kroki::
   :type: plantuml
   :caption: Session modes.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Idle
   Idle --> Offline : Solo
   Idle --> Online : Started from server
   Idle --> Playback : ReplayLast, or a loaded file
   Offline --> Idle : Leave
   Online --> Stopped : hash mismatch, stall,\ndisconnect, Ended
   Online --> Idle : Leave
   Playback --> Stopped : last turn verified
   Playback --> Idle : Leave
   Stopped --> Idle : Leave
   Stopped : match stays on screen,\norders disabled
   @enduml

A connection failure while still in the lobby also sets ``Stopped``; starting
any new match from the lobby leaves it. New matches can only start from the
lobby with no match running; otherwise
the action is refused with *Leave the current match before starting
another.* ``Leave`` and ``SaveReplay`` work at any time.

Starting a match
----------------

``Session::begin`` is shared by solo and online play:

1. Archive the previous session: move the current recording to
   ``last_replay`` and clear buffers.
2. Build the ``Sim`` from the ``MatchSetup``.
3. Start a new ``Replay`` from the same setup.
4. Insert a ``Match`` for the local player and switch to ``InGame``.

Solo play uses the bundled ``MatchData`` setup and slot 0. Online play uses
the setup received from the server and the slot from the last ``Lobby``
message. ``Session::begin_replay`` validates a complete replay by playing it
back *before* replacing anything, then starts in ``Playback`` with orders
disabled.

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
       :clear interpolation snapshot;
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
     :clear interpolation snapshot;
     stop
   endswitch
   :snapshot positions for interpolation;
   :turn.apply(sim);
   :hash = sim.state_hash();
   if (Online and hash != server hash?) then (yes)
     :fail: state differs from server;
     stop
   endif
   :recording.push(turn, hash);
   stop
   @enduml

On any failure the session leaves the network, switches to ``Stopped``,
disables orders, clears pending commands and the interpolation snapshot, and
shows the reason. The match stays on screen until the player leaves.

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

Networking
----------

``Connection::connect`` validates the address (``ws://`` or ``wss://``, at most
2048 characters; native builds refuse ``wss://``) and opens an ``ewebsock``
socket. Socket events go into a bounded channel of 128 events; if it fills,
the socket stops reading. The first message, ``Create`` or ``Join``, is sent
when the socket opens.

``poll_network`` runs in ``PreUpdate`` and handles up to 128 events per frame:

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Server message
     - Client reaction
   * - ``Setup``
     - Accept only before a match and only once; build the simulation to
       validate it; reply ``Ready { checksum }``.
   * - ``Lobby``
     - Update the room code, slot, counts, and ``can_start``.
   * - ``Started``
     - Check it matches the setup's player count and a delay of 2; begin the
       online match.
   * - ``Turn``
     - Require online mode, at most 128 buffered turns, and the next tick in
       sequence; buffer it with its hash.
   * - ``Ended``
     - Stop the session with the reason.
   * - ``Error``
     - Show the message. The match continues; for example, a late input was
       rejected.

A socket error, close, decoding failure, unexpected message, or a socket that
does not open within 10 seconds stops the session the same way.

Replay files
------------

``ReplayIo`` keeps platform file access out of the session:

* ``save`` writes synchronously: native writes a temporary file and renames
  it; the browser creates a ``Blob`` and clicks a download link.
* ``load`` delivers text through a one-slot channel. Native reads the saved
  file immediately; the browser opens a file picker and reads the chosen file
  asynchronously. ``finish_replay_load`` picks up the result on a later frame.

A loaded replay is refused if a match started in the meantime. Files larger
than 32 MiB are rejected before they are read.

Leaving
-------

``Leave`` sends ``Leave`` to the server if the socket is open, closes it,
archives the recording, sets the mode to ``Idle``, and returns to the lobby.
The last recording remains available for **Replay last** and **Save replay**.
