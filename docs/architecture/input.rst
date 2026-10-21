Camera and input
================

Two systems handle world input: ``control_camera`` (``camera.rs``) moves the
view, and ``control_units`` (``input.rs``) selects units and issues orders.
Both treat mouse and touch as first-class. Input only ever *queues* commands;
the simulation applies them later through the session.

Camera
------

The camera is a ``Camera2d`` entity with the ``RtsCamera`` component. Its
settings:

.. list-table::
   :header-rows: 1
   :widths: 25 15 60

   * - Field
     - Default
     - Meaning
   * - ``min_scale``
     - 0.25
     - Closest zoom.
   * - ``max_scale``
     - 4.0
     - Farthest zoom, further limited so the view never exceeds the map.
   * - ``pan_speed``
     - 1200
     - Keyboard and edge-scroll speed in screen pixels per second.
   * - ``edge_margin``
     - 8
     - Width of the window border that scrolls, in logical pixels.
   * - ``wheel_zoom_step``
     - 1.15
     - Scale multiplier per wheel line.

On entering a match the camera centers on the local player's start position.
Each frame, ``control_camera`` builds a ``View`` (center, scale, window size,
limits) and applies, in order:

1. **Keyboard and edge pan.** WASD and arrows, plus the window edges when the
   window is focused and no finger is down. The direction is normalized, so
   diagonals are not faster.
2. **Middle-drag.** The world follows the pointer.
3. **Wheel zoom.** Zoom toward the cursor: the world point under the cursor
   stays fixed on screen.
4. **Touch gesture.** With two or more fingers, pinch zooms around the
   previous centroid and the centroid's movement pans.
5. **Clamp.** The view stays inside the map; on an axis where the map is
   smaller than the view, the view centers on it.

The camera tracks finger positions itself instead of using
``Touch::previous_position``, which only refreshes on frames that carry
touch events. A gesture is computed only when the same fingers were down on
the previous frame, so adding or lifting a finger never makes the view jump.
Pinch zoom is the ratio of the average finger distance from the centroid
before and after.

Pointer areas
-------------

The top 56 pixels (session bar) and bottom 88 pixels (command bar) belong to
the interface. ``ScreenView::playable`` rejects presses that start or end
there, so tapping a button never also selects or orders.

Screen positions convert to the world with the camera's ``Transform`` and
``Projection`` from this frame rather than ``GlobalTransform``, which is only
propagated later in ``PostUpdate``. See :doc:`coordinates`.

Selection state
---------------

``Selection`` holds a ``BTreeSet<UnitId>`` and a ``touch_mode`` flag that
picks which hint the HUD shows. Every frame the selection drops units that no
longer exist or are not the local player's.

``PointerState`` tracks one pending press:

* ``mouse``: a left-button press, with start position, current position,
  whether it has become a drag, and whether Shift was held.
* ``finger``: the one finger that may become a tap or box.
* ``touch_blocked``: set when a second finger or a cancel appears; cleared
  when all fingers lift.
* ``suppress_mouse``: ignores the emulated mouse events some platforms send
  after touches, until the mouse buttons are released.

Touch input
-----------

.. kroki::
   :type: plantuml
   :caption: Touch press state machine.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Idle
   Idle --> Pressed : first finger down\nin the playable area
   Pressed --> Pressed : finger moves\n(≥ 14 px marks a drag)
   Pressed --> Completed : finger lifts\nin the playable area
   Pressed --> Idle : finger lifts outside
   Pressed --> Blocked : second finger,\nor touch canceled
   Idle --> Blocked : second finger,\nor touch canceled
   Blocked --> Idle : all fingers lifted
   Completed --> Idle : finish_press
   @enduml

When counting fingers, the system includes fingers that were pressed and
released within the same frame, so a fast second tap during a pinch still
blocks. While blocked, two-finger gestures belong to the camera and no
selection or order is issued.

Mouse input
-----------

* Left press in the playable area starts a ``Press``. Moving 8 pixels turns
  it into a drag; returning to the start does not undo that.
* Left release in the playable area finishes the press. Releasing outside
  cancels it.
* Right press in the playable area issues a move immediately.
* X queues a Stop for the selection. Escape clears the selection and any
  pending press. Losing window focus cancels presses.

Finishing a press
-----------------

.. kroki::
   :type: plantuml
   :caption: finish_press, shared by mouse and touch.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   if (dragged?) then (yes)
     if (Shift held?) then (no)
       :clear selection;
     endif
     :add own units whose displayed center\nis inside, or custom bounds intersect the box;
   else (no)
     :find the nearest own unit hit by its displayed\ncircle or custom bounds (with minimum hit area);
     if (unit hit?) then (yes)
       if (Shift held and already selected?) then (yes)
         :remove it;
       else (no)
         if (Shift held?) then (no)
           :clear selection;
         endif
         :add it;
       endif
     elseif (touch with a selection?) then (yes)
       :issue move to the tap position;
     elseif (Shift held?) then (no)
       :clear selection;
     endif
   endif
   stop
   @enduml

Hit tests use displayed (interpolated) positions, visual scale, and bounds
offsets. Custom ``visual_bounds`` use rectangular click targets and box
intersection; default circle visuals retain radius tests and selection by
center. Minimum half-extents are 8 screen pixels for the mouse and 24 for
touch, so the touch target is at least 48 pixels across. Ties go to the
nearer bounds center, then the lower ``UnitId``. See :doc:`rendering` and
:doc:`../reference/rules-format`.

Issuing orders
--------------

``issue_move`` converts the world position to simulation units, rounds it, and
checks the target terrain:

* Blocked or outside the map: show a red marker and send nothing.
* Otherwise: show a green marker and call ``Match::queue_command`` with a
  ``Move`` for the selected IDs in ascending order.

``Selection::stop`` queues a ``Stop`` for the selection. Both the X key and the
HUD's Stop button use it.

``Match::queue_command`` silently drops the command when orders are disabled
(playback or a stopped session) or when it would exceed the per-turn limits.
The simulation still validates ownership and reachability when the command is
applied.
