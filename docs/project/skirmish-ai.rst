Skirmish AI
===========

The solo skirmish opponent is ``rts_ai::Opponent``. It depends only on
``rts_base`` and has no rendering or networking dependency.

Knowledge
---------

The opponent reads nothing but its own player's ``PlayerView`` (see
:doc:`../architecture/simulation`): terrain it has explored, deposits it has
seen with the stock it last saw, its own units and their orders, its own
stockpile, and enemy units currently in sight. It never looks up the enemy
headquarters, unexplored terrain, or unseen deposits in the authoritative
``Sim``. Two worlds that differ only in what the player has not seen produce
identical decisions.

``Opponent::new(&view, difficulty)`` creates it, and
``opponent.commands(&view)`` is called once per tick, after the view applies
that tick's frame. Commands use the view's unit and deposit IDs; the session
translates them with ``PlayerKnowledge::translate_command`` before they enter
the turn, so the authority validates them like a human player's orders.

The opponent keeps a small deterministic memory, built only from the views it
was given: enemy buildings it has seen (forgotten once their site is in sight
and empty), scouting cells it has already chosen, and whether its army has
launched its attack. A resumed save rebuilds this memory by replaying the
recorded turns and calling ``commands`` before each one, as during live play.

Behavior
--------

* Workers gather the nearest known wood and gold, balancing the two. One
  worker builds a barracks on known open ground in front of the
  headquarters.
* The barracks trains a scout (``Command::TrainScout``) whenever the player
  has none alive or queued, then rifle squads. The headquarters trains workers
  up to eight.
* Idle scouts move to the center of an unexplored 4×4-tile cell. The
  simulation routes them through explored terrain toward the fog frontier.
  Each cell is chosen once; when every unexplored cell has been tried, the
  choices are forgotten and tried again.
* Squads defend against armed enemies in sight within seven tiles of the
  headquarters. Otherwise they rally in front of the base until the attack
  starts. Attacking squads engage armed enemies in sight first, then any
  enemy in sight, then move to the nearest remembered enemy building. With
  nothing known, they explore unexplored cells starting from the mirror image
  of their own start, which is derived from the start and map size only.

Difficulty
----------

``rts_ai::Difficulty`` is ``Easy``, ``Normal`` (the default), or ``Hard``.
All levels share the same sight, prices, income, and economic logic. They
differ only in scouting, reaction cadence, and attack timing:

.. list-table::
   :header-rows: 1
   :widths: 16 34 16 34

   * - Level
     - Scout destination
     - Decides every
     - First attack
   * - Easy
     - Unexplored cell nearest the scout
     - 40 ticks (2 s)
     - 4 squads, not before 4:00
   * - Normal
     - Unexplored cell nearest the map center
     - 20 ticks (1 s)
     - 3 squads, not before 2:30
   * - Hard
     - Unexplored cell nearest the mirror of its start
     - 8 ticks (0.4 s)
     - 2 squads, not before 1:30

Once launched, the attack continues with every squad until the army is lost;
the next wave waits for the threshold again. Between decisions the opponent
issues no orders but still updates its memory every tick.

The player chooses the level on the skirmish **New game** page. ``Session``
keeps it for **Restart**, and saved games store it in their metadata, so a
loaded save resumes at its own level. Saves without the field resume at
Normal.

Solo saves and replays record both players' commands. Playback verifies the
recorded turns without running the opponent.
