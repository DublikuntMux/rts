Skirmish AI
===========

The solo skirmish opponent reads ``rts_base::Sim`` and submits ordinary
player commands through the offline session. ``rts_ai`` depends on the
simulation crate and has no rendering or networking dependency.

The opponent gathers wood and gold, builds a barracks, trains workers and
rifle squads, and sends its squads against the enemy headquarters. The
simulation validates its commands and applies the same prices, orders, and
combat rules for both players.

Solo saves and replays record both players' commands. Playback verifies the
recorded turns without running the AI again. A resumed solo skirmish begins
issuing fresh AI orders from the restored simulation state.
