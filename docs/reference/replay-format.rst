Replay file format
===================

A replay file (``*.replay.ron``) is a RON document produced by
``rts_net::Replay::to_ron`` (``crates/net/src/replay.rs``). It contains the
match setup and every turn, not positions: playback re-simulates the match.

Structure
----------

.. code-block:: rust

   struct Replay {
       version: u32,          // protocol version, currently 5
       setup: MatchSetup,     // rules and map text, seed, players
       turns: Vec<Turn>,      // tick 1, 2, 3, ... in order
       final_hash: u64,       // state hash after the last turn
   }

   struct MatchSetup { rules: String, map: String, seed: u64, players: u8 }

The ``Turn`` type is the same as in :doc:`protocol`. A shortened example,
wrapped for reading:

.. code-block:: text

   (version:5,
    setup:(rules:"(tile_capacity:4,terrain:[...],units:[...])",
           map:"(name:\"Proving Grounds\",players:[...],tiles:[...],units:[...])",
           seed:0,players:2),
    turns:[(tick:1,inputs:[(player:(0),commands:[]),(player:(1),commands:[])]),
           (tick:2,inputs:[(player:(0),commands:[Move(units:[(0),(1)],target:(11147,14223))]),
                           (player:(1),commands:[])])],
    final_hash:17134265448149945174)

Online replays embed the server's minified documents. Solo replays embed the
bundled files with their original comments and formatting. Both stay plain RON
strings rather than the compressed transport format, so you can read the
exact content that was played.

Validation
-----------

``Replay::from_ron``, ``Replay::from_ron_with_sim``, and ``Replay::to_ron``
run a full playback, which checks:

* The text is at most 32 MiB and RON nesting is at most 64 levels.
* ``version`` equals the current protocol version.
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

Playback in the client applies one turn per fixed tick, then compares the
final state with ``final_hash`` and reports *Replay finished and verified.*

Library records and saved games
--------------------------------

The client stores replay RON in ``replays/<id>.data.ron`` and a companion
``<id>.meta.ron`` with the name, map, player count, tick count, save time in
Unix milliseconds, and setup. Saved solo games use the same pair under
``saves/``. Metadata supports listing and previews; the data document is the
replay format above. Browsers store compressed documents in ``localStorage``
under ``rts/<path>`` keys and export plain RON as ``match.replay.ron``.
See :doc:`../guides/replays` for platform paths.

``Replay::from_ron_with_sim`` returns ``(Replay, Sim)`` after verification.
Saved-game loading continues from this final simulation and appends new
turns to the recording, without replaying the history a second time.
