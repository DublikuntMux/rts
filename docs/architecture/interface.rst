Interface: lobby, bars, and keypad
==================================

The interface uses Bevy UI with the standard widgets (``Button``,
``TextInput``, ``ScrollArea``) and observers. Interface code never runs
sessions or touches the simulation. It reads ``SessionStatus`` or
``Selection`` and sends ``SessionAction`` messages or queues commands. Code
lives in ``crates/client/src/lobby.rs`` and ``hud.rs``.

Design rules
------------

* Every interactive element is at least 48 logical pixels on its short side.
* Layouts fit a 320-pixel-wide screen. The lobby column is at most 440 pixels
  wide and scrolls vertically; button rows wrap.
* Primary buttons are filled; secondary buttons are outlined. Hover, pressed,
  and disabled states change fill, border, and text color. A shared ``look``
  function computes them, so state never depends on hover alone.
* Text uses the Exo 2 font. The room code and keyboard focus use an amber
  signal color.
* Disabled controls carry ``InteractionDisabled`` and ignore activation.

Lobby
-----

.. kroki::
   :type: plantuml
   :caption: Lobby layout.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.1
   {
     <b>RTS
     Play solo, or create a room and share its code.
     [Play solo]
     {+
       <b>Online match
       Server address
       "ws://127.0.0.1:9001     "
       [Create room]
       Room code
       { "K7Q2PX  " | [Join room] }
       Share the room code. Start becomes available when the room is full.
       {+
         Room code
         <b>K7Q2PX
         You are the host, player 1.
         1 of 2 players joined
       }
       { [Start match] | [Leave room] }
     }
     {+
       <b>Replays
       { [Replay last] | [Save replay] | [Load saved replay] }
     }
   }
   @endsalt

The lobby spawns on ``OnEnter(AppState::Menu)``. ``LobbyForm`` keeps the
typed address and code across visits. The room code field accepts only
letters and digits and uppercases them in place; the address field rejects
whitespace and control characters and holds at most 256 characters.

Each button carries a ``Control`` value. ``Control::enabled`` decides
availability from ``SessionStatus`` and the form:

.. list-table::
   :header-rows: 1
   :widths: 30 70

   * - Control
     - Enabled when
   * - Play solo, Load replay
     - Not connecting.
   * - Create room
     - Not connecting, not in a room, and the address starts with ``ws://``
       or ``wss://`` followed by more text.
   * - Join room
     - As Create room, and the code is six uppercase letters or digits.
   * - Start match
     - This client is player 1 (the host), and the room is full and ready.
   * - Leave room
     - Connecting, in a room, or in a match.
   * - Replay last, Save replay
     - A recording exists.

Activating an enabled control writes its ``SessionAction`` (see
:doc:`sessions`). Fields are read-only while connecting or in a room. The hint
under the online fields explains the next step: enter an address, share the
code, wait for the host, and so on.

Keyboard support: the lobby is a tab group with Play solo focused first, Tab
moves focus, focused controls scroll into view, and Enter in the code field
activates Join room. Touch drags scroll the column; mouse wheels scroll
through ``ScrollArea``.

Touch keypad
------------

Bevy's text input cannot open the platform's on-screen keyboard yet, so a
completed *touch* tap on an editable field opens an in-game keypad instead.
Mouse clicks and physical keyboards keep using the text field directly.

.. kroki::
   :type: plantuml
   :caption: Keypad lifecycle. Edits stay in a draft until Done.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Closed
   Closed --> Open : touch tap on an editable field
   Open : full-screen overlay blocks the lobby
   Open : KeypadDraft holds field, value, shift, page
   Open --> Open : key pressed\n(Char, Delete, Clear, Shift, page)
   Open --> Closed : Done → commit draft\nto field and LobbyForm
   Open --> Closed : Cancel → discard draft
   Open --> Closed : fields lock\n(connecting or joined)
   Open --> Closed : leave the Menu state
   @enduml

* The keypad opens on click, after the finger lifts, so the same touch cannot
  press a key underneath.
* Room code keypads show letters and digits, typed uppercase, up to six
  characters.
* Server address keypads have a letters page (letters, digits, ``: / . -``,
  Shift, and a symbols key) and a symbols page with digits and all URL
  punctuation (``: / . - _ ? & = % + [ ] #``). Shift types capitals. The limit is
  256 characters.
* Keys are laid out six per row so they fit 320 pixels. A preview shows the
  draft or a placeholder, with a character count.
* Opening the keypad clears focus from the text field.

Command bar
-----------

The HUD (``hud.rs``) spawns on ``OnEnter(AppState::InGame)``: an 88-pixel bar
at the bottom with the selection count, a hint, and two buttons.

* The hint depends on ``touch_mode`` and the selection: *Click or drag to
  select units.*, *Right-click ground to move.*, *Tap or drag to select
  units.*, or *Tap ground to move. Two fingers pan.* It wraps to two lines at
  320 pixels.
* **Stop** (primary, with a square glyph drawn as a node so it does not depend
  on font coverage) calls ``Selection::stop``.
* **Clear** (secondary) empties the selection.
* Both are disabled when nothing is selected or the match does not accept
  orders.

Session bar
-----------

The session bar (``lobby.rs``) spawns on ``OnEnter(AppState::InGame)``: a
56-pixel bar at the top.

* Title: *Watching replay*, *Room CODE, player N*, or *Solo match*.
* Message: the latest ``SessionStatus::message``, such as *Waiting for the
  server…* or a failure reason.
* **Save replay** and **Leave** buttons. They are not tab-focusable, so
  gameplay keys never activate them.

Both bars are excluded from world input; see :doc:`input`.
