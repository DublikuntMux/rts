Replay file format
===================

A replay file (``*.replay.ron``) is a RON document in one of two forms
(``crates/net/src/replay.rs``), wrapped by ``rts_net::Recording``:

* A **full replay** (``Replay``) of a local match: the setup and every
  player's turns. Playback re-simulates the match and verifies its hash.
  Saved games use this form.
* A **player-perspective replay** (``ViewReplay``) of an online match: the
  rules and the view frames one player received. It contains only what that
  player saw, never the map, the seed, or other players' commands.

``Recording::from_ron`` accepts either form and verifies it.

Full replay structure
---------------------

.. code-block:: rust

   struct Replay {
       version: u32,          // 9 for new recordings
       setup: MatchSetup,     // rules and map text, seed, players
       turns: Vec<Turn>,      // tick 1, 2, 3, ... in order
       final_hash: u64,       // state hash after the last turn
   }

   struct MatchSetup { rules: String, map: String, seed: u64, players: u8 }

The ``Turn`` type is the same as in :doc:`protocol`. A shortened example,
wrapped for reading:

.. code-block:: text

   (version:9,
    setup:(rules:"(tile_capacity:4,terrain:[...],units:[...])",
           map:"(name:\"Proving Grounds\",players:[...],tiles:[...],units:[...])",
           seed:0,players:2),
    turns:[(tick:1,inputs:[(player:(0),commands:[]),(player:(1),commands:[])]),
           (tick:2,inputs:[(player:(0),commands:[Move(units:[(0),(1)],target:(11147,14223))]),
                           (player:(1),commands:[])])],
    final_hash:17134265448149945174)

Solo replays embed the bundled files with their original comments and
formatting as plain RON strings, so you can read the exact content that was
played.

Version 9 matches enforce fog in the simulation (``Sim::enforce_fog``): combat
and navigation respect each player's sight. Playback enables it for version 9
and replays version 8 and 7 recordings with their original rules.

Validation
-----------

``Replay::from_ron``, ``Replay::from_ron_with_sim``, and ``Replay::to_ron``
run a full playback, which checks:

* The text is at most 32 MiB and RON nesting is at most 64 levels.
* ``version`` is 9 for current recordings or 8 for earlier ones. Original
  version 7 sandbox replays
  are accepted when they are not skirmish matches and contain only ``Move``,
  ``Stop``, and ``Attack`` commands.
* There are at most 1,000,000 turns (about 13.9 hours at 20 ticks per second).
* The setup is valid and the map has exactly ``players`` starts.
* Each turn is the next tick, with one ascending input per player, and
  satisfies the command limits.
* The state hash after the last turn equals ``final_hash``.

Any failure rejects the whole file. A single edited command or position
changes the final hash.

Recording
----------

``Replay::new`` starts with no turns and the initial state's hash.
``Replay::push`` appends a turn after it is applied and stores the resulting
hash as ``final_hash``, so a partial recording is always valid and can be
saved mid-match.

Playback in the client applies one turn per fixed tick, shows the match from
player 1's view, then compares the final state with ``final_hash`` and
reports *Replay finished and verified.*

Player-perspective replays
--------------------------

.. code-block:: rust

   struct ViewReplay {
       version: u32,          // 9
       map: String,           // the room's map name
       rules: String,         // the server's minified rules
       intro: ViewIntro,      // the recording player's perspective
       frames: Vec<ViewFrame> // tick 0, 1, 2, ... in order
   }

An online client starts the recording when the match starts and pushes every
frame it applies. Playback applies one frame per fixed tick to a fresh
``PlayerView`` and reports *Replay finished.* Verification replays every
frame through ``PlayerView::apply``, so an out-of-order, duplicated, or
malformed frame rejects the file. There is no hash: the recording is exactly
what the server sent, and a client never has the state to hash. Frame and
setup types are described in :doc:`protocol` and
:doc:`../architecture/visibility`. At most 1,000,000 frames are accepted.

Library records and saved games
--------------------------------

The client stores replay RON in ``replays/<id>.data.ron`` and a companion
``<id>.meta.ron`` with the name, map, player count, tick count, save time in
Unix milliseconds, and setup. The setup is omitted for player-perspective
replays, which never hold the map; their previews are blank. Saved solo games use the same pair under
``saves/``; their metadata also records the opponent's ``difficulty``
(``Easy``, ``Normal``, or ``Hard``; Normal when absent). Metadata supports listing and previews; the data document is the
replay format above. Browsers store compressed documents in ``localStorage``
under ``rts/<path>`` keys and export plain RON as ``match.replay.ron``.
See :doc:`../guides/replays` for platform paths.

Only full replays can be saved games. Loading one replays its turns from the
start so the local player's knowledge, each computer opponent's knowledge,
and the opponents' memory match the live match. The final state is checked
against ``final_hash``, then new turns are appended to the same recording.
