Play a match
============

This page explains what you see in a match and the rules that decide how
units move. For a compact list of inputs, see :doc:`../reference/controls`.

The lobby
---------

The game opens on the lobby. It has three parts:

* **Play solo** starts an offline match on the bundled map. You control
  player 1; the other player's units stand still.
* **Online match** creates or joins a room on a server. See
  :doc:`multiplayer`.
* **Replays** plays, saves, and loads recordings. See :doc:`replays`.

On short screens the lobby scrolls. On touch screens, tapping the
**Server address** or **Room code** field opens an in-game keypad with
**Done** and **Cancel**, because Bevy cannot open the platform keyboard yet.

The match screen
----------------

.. kroki::
   :type: plantuml
   :caption: The in-game screen. Pointer input in the top and bottom bars never reaches the map.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.2
   {+
     { <b>Solo match | . | . | . | [Save replay] | [Leave] }
     { Latest session message }
     ==
     {
       Map: terrain and units
       Drag to select; right-click or tap ground to move
     }
     ==
     { <b>3 selected | . | . | [■ Stop] | [Clear] }
     { Right-click ground to move. }
   }
   @endsalt

* The **session bar** at the top shows what is running (*Solo match*,
  *Room K7Q2PX, player 1*, or *Watching replay*) and the latest message, such
  as *Waiting for the server…*. It has **Save replay** and **Leave**.
* The **command bar** at the bottom shows how many units are selected, a hint
  for the next action, and the **Stop** and **Clear** buttons. They are
  disabled without a selection, and during replay playback.

Every button is at least 48 logical pixels tall, so the interface works on a
320-pixel-wide phone screen.

Select units
------------

You can select only your own units. Each player has a color by slot:
player 1 is blue and player 2 is red.

* Click or tap a unit to select it. Touch uses a larger hit area (at least
  48 pixels across) than the mouse.
* Drag a box to select every unit of yours whose center is inside it.
* Hold Shift while clicking to add a unit to the selection or remove it;
  Shift with a box adds to the selection.
* Click empty ground, press Escape, or choose **Clear** to deselect.

Selected units show a yellow ring.

Give orders
-----------

* **Move:** right-click ground with the mouse, or tap ground with a selection
  on a touch screen. A green marker confirms the order. A red marker means the
  target is water, rock, or outside the map, and no order is sent.
* **Stop:** press X or choose **Stop**. Units drop their current order.

Orders are never applied instantly. Your client sends them for a tick two
ticks (100 ms) ahead, and every client applies them at that tick in the same
order. :doc:`../architecture/networking` explains why.

Terrain
-------

Each terrain type has a speed modifier. On the bundled map:

.. list-table::
   :header-rows: 1
   :widths: 20 20 60

   * - Terrain
     - Symbol
     - Movement
   * - Grass
     - ``.``
     - Normal speed.
   * - Dirt
     - ``,``
     - Normal speed.
   * - Sand
     - ``:``
     - Normal speed.
   * - Water
     - ``~``
     - Blocked.
   * - Rock
     - ``#``
     - Blocked.

Mods can add faster or slower terrain; a modifier of ``2.0`` doubles speed and
``0.5`` halves it. Units choose the *fastest* route, not the shortest, so a
longer road can beat a short trip through mud. See
:doc:`../reference/rules-format`.

Tile capacity
-------------

Each tile holds a limited total unit *size*, shared by all players. In the
bundled rules each tile holds 4 size points; a worker uses 1 and a soldier
uses 2. A tile can hold four workers, two soldiers, or two workers and one
soldier.

Units route around full tiles. If a route or the destination stays full, they
wait next to it and retry automatically every tick as space opens. Moving or
stopping the blocking units frees the space on the next tick.

Units within the same tile can overlap visually; capacity only limits how many
share a tile. Physical separation and formations are planned for a later
milestone (see :doc:`../project/roadmap`).

Leave a match
-------------

Choose **Leave** to return to the lobby. In an online match, leaving ends the
match for every player. The recording stays available for **Replay last** and
**Save replay** until you record another match.
