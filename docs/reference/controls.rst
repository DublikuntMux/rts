Controls
========

Every action is reachable by touch. No feature depends on hover or on a
hotkey alone. Pointer input over the top-left pause button or bottom RTS
console never reaches the map. Desktop reserves 184 logical pixels at the
bottom; phones use floating corner controls, leaving the rest playable.
Phone mode starts below 620 logical pixels wide or 520 high. Bounds grow
with interface scale. Menus, dialogs, and expanded phone panels capture
world input.

Desktop
-------

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Input
     - Action
   * - Left-click a unit
     - Select it, replacing the selection.
   * - Left-drag
     - Box-select your units. A drag starts after 8 pixels of movement.
   * - Shift + left-click a unit
     - Add it to the selection, or remove it if already selected.
   * - Shift + left-drag
     - Add the boxed units to the selection.
   * - Left-click empty ground
     - Clear the selection.
   * - Right-click ground
     - Move the selection there.
   * - M
     - Arm Move targeting; click a destination to move the selection.
   * - X
     - Stop the selection.
   * - Escape
     - Cancel Move targeting first, then close a dialog or toggle the pause
       menu. Keep the selection. Close a phone HUD popup before opening
       the pause menu.
   * - W A S D or arrow keys
     - Pan the camera.
   * - Pointer at a world-view edge (8 pixels)
     - Pan when **Edge scrolling** is on. The downward edge is just above
       the desktop HUD; phones use the full window edge.
   * - Middle-drag
     - Pan the camera with the pointer.
   * - Mouse wheel
     - Zoom toward the pointer, 1.15× per line.

Clicking or dragging the minimap centers the camera without issuing orders.
On phones, tap the top-right thumbnail to open the map first. Tap the
selection portrait in the floating dock to inspect units.
Losing window focus cancels a pending press. Clicking a unit of another player
does not select it.

Touch
-----

.. list-table::
   :header-rows: 1
   :widths: 40 60

   * - Gesture
     - Action
   * - Tap a unit
     - Select it, replacing the selection. Hit areas are at least
       48 pixels across.
   * - Tap ground with a selection
     - Move the selection there.
   * - Tap ground without a selection
     - Nothing.
   * - One-finger drag
     - Box-select your units. A drag starts after 14 pixels of movement.
   * - Two-finger drag
     - Pan the camera.
   * - Pinch
     - Zoom around the fingers.

A second finger cancels any pending selection or order until all fingers
lift, so a pinch never turns into a tap. A canceled touch also cancels the
press.

Buttons
-------

.. list-table::
   :header-rows: 1
   :widths: 25 25 50

   * - Button
     - Where
     - Action
   * - Pause / Resume
     - Top-left button / pause menu
     - Open or close the menu; pause solo play and replays. Online continues.
   * - Move
     - Command card
     - Arm targeting; the next world click or tap chooses a destination.
   * - Stop
     - Command card
     - Stop the selection. Disabled without a selection or during playback.
   * - Clear
     - Command card
     - Clear the selection.
   * - Save game
     - Pause menu, solo only
     - Add a save for later continuation through **Singleplayer → Load game**.
   * - Save replay
     - Pause menu
     - Add the current recording to the replay library.
   * - Leave
     - Pause menu
     - Return to the menu. Live solo and online matches ask for confirmation;
       leaving online ends the match for everyone and keeps the connection.
   * - New game, Start game
     - Singleplayer
     - Open map selection, then start the selected map offline.
   * - Load save, Delete…
     - Load game
     - Continue the selected save, or confirm its deletion.
   * - Connect, Cancel, Disconnect
     - Multiplayer
     - Open or close the server connection.
   * - Refresh
     - Room browser
     - Request the public room list again.
   * - + Create room, Join room, Join by code
     - Room browser
     - Choose visibility and create a room, join the selected room, or enter
       an invite code in a dialog.
   * - Ready / Not ready
     - Room page
     - Set or clear your explicit readiness.
   * - Start match
     - Room page
     - Host only; enabled when all slots are filled and ready.
   * - Copy code
     - Room page, except Android
     - Copy the invite code to the system clipboard.
   * - Leave room
     - Room page
     - Confirm leaving; keep the server connection.
   * - Watch replay, Save to library
     - Replays
     - Watch a selected recording, or save **Last match** persistently.
   * - Import replay file, Export
     - Replays, browser only
     - Add a file to the library, or download a stored replay.

Menu keyboard and touch
-----------------------

Tab and Shift+Tab move focus between menu controls. Focused controls scroll
into view. Enter in the server address field connects; Enter in the invite
code field joins. Escape and the compact layout's **Back** button close a
dropdown first, then a dialog. They return New game or Load game to
Singleplayer, disconnect from Connecting or Rooms, or ask to leave a lobby.

Touch taps on **Server address**, **Invite code**, and **Player name** focus
the field and open the platform keyboard; edits apply immediately. Mouse
wheels and touch drags scroll pages. In-game pause and command buttons stay outside the
tab order.

**Settings** offers interface scales of 75, 100, 125, and 150 percent,
**Edge scrolling** on/off, and desktop-only **Fullscreen**. Compact mode
starts below 760 logical pixels of width after interface scaling.

Camera limits
-------------

The camera is orthographic. At scale 1, one tile is 128 logical pixels. Zoom
ranges from scale 0.25 (512 pixels per tile) to scale 4 (32 pixels per tile),
and the farthest zoom is also limited so the view never shows more than the
whole map. The usable view above the HUD is clamped to the map and centered on axes
where the map is smaller. The bottom edge stays reachable above the HUD. Keyboard and edge panning move 1200 screen pixels
per second.

Planned controls
----------------

.. list-table::
   :header-rows: 1
   :widths: 30 25 45

   * - Gesture
     - Milestone
     - Action
   * - Tap an enemy with a selection
     - M4
     - Attack it.
   * - Long-press
     - M4
     - Context menu with attack-move, patrol, and other orders.
   * - Control groups
     - M9
     - Save and recall selections.
