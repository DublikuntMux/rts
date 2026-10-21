Controls
========

Every action is reachable by touch. No feature depends on hover or on a
hotkey alone. Pointer input over the session bar (top, 56 logical pixels) or
the command bar (bottom, 88 logical pixels) never reaches the map.

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
   * - X
     - Stop the selection.
   * - Escape
     - Clear the selection and cancel a pending press.
   * - W A S D or arrow keys
     - Pan the camera.
   * - Pointer at a window edge (8 pixels)
     - Pan the camera.
   * - Middle-drag
     - Pan the camera with the pointer.
   * - Mouse wheel
     - Zoom toward the pointer, 1.15× per line.

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
   * - Stop
     - Command bar
     - Stop the selection. Disabled without a selection or during playback.
   * - Clear
     - Command bar
     - Clear the selection.
   * - Save replay
     - Session bar, lobby
     - Save the current or last recording.
   * - Leave
     - Session bar
     - Return to the lobby; ends an online match for everyone.
   * - Play solo
     - Lobby
     - Start an offline match.
   * - Create room, Join room
     - Lobby
     - Connect to the server address.
   * - Start match
     - Lobby
     - Host only; available when the room is full and ready.
   * - Leave room
     - Lobby
     - Leave the room or cancel connecting.
   * - Replay last, Load saved replay / Load replay
     - Lobby
     - Watch a recording.

In the lobby, Tab and Shift+Tab move keyboard focus between controls, and
Enter in the room code field joins the room. In-game bar buttons are not
focusable, so gameplay keys never activate them.

Camera limits
-------------

The camera is orthographic. At scale 1, one tile is 128 logical pixels. Zoom
ranges from scale 0.25 (512 pixels per tile) to scale 4 (32 pixels per tile),
and the farthest zoom is also limited so the view never shows more than the
whole map. The view is clamped to the map and centered on axes where the map
is smaller than the window. Keyboard and edge panning move 1200 screen pixels
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
   * - Minimap tap or drag
     - M9
     - Jump the camera.
   * - Control groups
     - M9
     - Save and recall selections.
