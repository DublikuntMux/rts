Record and watch replays
========================

Every match is recorded automatically. A replay stores the match setup (map,
rules, seed, and player count) and every turn applied, so playback
re-simulates the whole match rather than storing positions. Playback checks
that it reaches the same final state hash as the original match.

Record a match
--------------

Recording starts when a solo or online match starts. Each applied turn is
appended together with the resulting state hash. Online clients record only
turns whose hash matched the server's.

Leaving a match keeps the recording in memory as the *last replay* until
another match starts. Starting a new match replaces it.

Save a replay
-------------

Choose **Save replay** in the session bar during a match, or in the lobby
afterwards. The game re-simulates the recording to verify it before writing.

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Platform
     - Saved file
   * - Linux
     - ``$XDG_DATA_HOME/rts/replays/latest.replay.ron``, falling back to
       ``~/.local/share/rts/replays/latest.replay.ron``.
   * - Windows
     - ``%LOCALAPPDATA%/rts/replays/latest.replay.ron``, unless
       ``XDG_DATA_HOME`` is set.
   * - Android
     - ``replays/latest.replay.ron`` inside the app's internal data
       directory.
   * - Other native platforms
     - ``$XDG_DATA_HOME``, then ``LOCALAPPDATA``, then ``~/.local/share``,
       followed by ``rts/replays/latest.replay.ron``.
   * - Browser
     - A download named ``match.replay.ron``.

Native saves write a temporary ``latest.replay.ron.tmp`` file and rename it
over the previous replay, so a failed write never leaves a truncated file.
Each save replaces the previous one; copy the file elsewhere to keep it.

Watch a replay
--------------

From the lobby:

* **Replay last** plays the in-memory recording.
* **Load saved replay** (native) opens ``latest.replay.ron``.
* **Load replay** (browser) opens a file picker for a ``.ron`` file.

The game validates a loaded file completely before it replaces the current
session: it checks the format version, setup, turn sequence, and limits, then
re-simulates every turn and compares the final hash. Playback then runs at 20
ticks per second. The session bar shows *Watching replay*, and order buttons
stay disabled. When the last turn is applied, the bar shows
*Replay finished and verified.*

Errors
------

.. list-table::
   :header-rows: 1
   :widths: 45 55

   * - Message
     - Meaning
   * - *There is no recorded match yet.*
     - Play a match first.
   * - *Cannot load replay: final replay hash mismatch*
     - The file was edited or recorded by an incompatible simulation.
   * - *Cannot load replay: unsupported protocol version*
     - The replay comes from an older or newer protocol version. Replays are
       tied to the protocol version (currently 4).
   * - *Replay file is too large.*
     - The file exceeds 32 MiB.

Inspect a replay
----------------

Replays are plain RON text. Online replays contain the server's minified map
and rules as strings, so you can see exactly what was played. Terrain textures
are not included; they are local presentation assets.
:doc:`../reference/replay-format` documents the file layout.
