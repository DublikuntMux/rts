Play a match
============

This page explains what you see in a match and the rules that decide how
units move. For a compact list of inputs, see :doc:`../reference/controls`.

The main menu
-------------

The game opens on **Singleplayer**. The sidebar also offers **Multiplayer**,
**Replays**, **Settings**, and **Quit**. Browsers hide **Quit**.

* **Singleplayer → New game** lists bundled maps with previews. Choose
  *Crossroads* (four players, 40×40 tiles) or *Proving Grounds* (two players,
  48×32 tiles), then **Start game**. You control player 1; other players'
  units stand still.
* **Singleplayer → Load game** lists saved solo matches. Filter by map or
  choose **Most recent** / **Oldest first**, select a save, then **Load save**.
  **Delete…** asks for confirmation.
* **Multiplayer** connects to a server, browses rooms, and creates or joins
  one. See :doc:`multiplayer`.
* **Replays** browses recorded matches. See :doc:`replays`.
* **Settings** changes the player name, interface scale, edge scrolling,
  and desktop fullscreen. Changes persist automatically. An invalid typed
  name stays visible; rooms use the last valid name until it is corrected.

The title bar shows your location. Content scrolls, and details wrap below
lists on narrow screens. Below 760 logical pixels of width, the sidebar
uses short labels and a **Back** button appears when needed. Interface scale
also affects this threshold. Escape goes back or closes a dialog or dropdown.

On touch screens, tapping **Server address**, **Invite code**, or
**Player name** opens the in-game keypad with **Done** and **Cancel**.

The match screen
----------------

.. kroki::
   :type: plantuml
   :caption: The match screen: minimap, selected units, and command card below the battlefield.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.2
   {+
     { [Pause] | . | . | . }
     {
       Map: terrain and units
       Drag to select; right-click or tap ground to move
     }
     ==
     { Minimap | 3 selected / portraits / health | [Move] | [Stop] }
     { Camera outline | Right-click ground to move | . | [Clear] }
   }
   @endsalt

* The top-left **pause button**, or Escape, opens the menu with **Resume**,
  **Save game** (solo only), **Save replay**, and **Leave**. The current
  session message also appears there. Solo games and replays pause while
  the menu is open. Online matches continue; world controls are blocked.
* The bottom-left **minimap** shows terrain, player-colored units, and the
  camera outline. Click, tap, or drag it to move the camera.
* The center shows the selection's name or count, health, order status, and
  portraits. Click a group portrait to select that unit alone.
* The right **command card** has **Move**, **Stop**, and **Clear**. Commands
  are disabled without a selection or during replay playback.

On phones, the bottom bar is replaced by floating controls. Tap the top-right
map thumbnail to open the minimap. Selecting units shows a small command dock
in the bottom-right corner: two rows in portrait, one row in landscape. Tap
its unit portrait to see selection details. With no selection, the command
dock disappears. The rest of the battlefield stays playable. This layout
also applies to short landscape windows and accounts for interface scale.

Buttons are at least 48 logical pixels tall before interface scaling.

Select units
------------

You can select only your own units. Each player has a color by slot:
player 1 is blue and player 2 is red.

* Click or tap a unit to select it. Touch uses a larger hit area (at least
  48 pixels across) than the mouse.
* Drag a box to select every unit of yours whose center is inside it.
* Hold Shift while clicking to add a unit to the selection or remove it;
  Shift with a box adds to the selection.
* Click empty ground or choose **Clear** to deselect. Escape retains the
  selection and opens the pause menu.

Selected units show a yellow ring.

Give orders
-----------

* **Move:** right-click ground with the mouse, or tap ground with a selection
  on a touch screen. A green marker confirms the order. A red marker means the
  target is water, rock, or outside the map, and no order is sent.
  Alternatively choose **Move** or press M, then click or tap a destination.
  Escape cancels destination targeting before opening the menu.
* **Stop:** press X or choose **Stop**. Units drop their current order.

Orders are never applied instantly. Your client sends them for a tick two
ticks (100 ms) ahead, and every client applies them at that tick in the same
order. :doc:`../architecture/networking` explains why.

Terrain
-------

Each terrain type has a speed modifier. On the bundled maps:

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

Open the pause menu and choose **Leave** to return to the main menu. Live solo and online matches ask for
confirmation. Leaving solo play loses progress since the last **Save game**;
leaving an online match ends it for everyone and keeps the server connection
open. Replays and stopped matches leave at once.

The recording remains available under **Replays → Last match**, with
**Watch replay** and **Save to library**. Save it to keep it across restarts.
