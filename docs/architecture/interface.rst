Interface: menus, HUD, and keypad
=================================

The interface uses Bevy UI widgets and observers. It reads ``SessionStatus``
and sends ``SessionAction`` messages. Gameplay controls read ``Selection``
and queue commands. Interface code never advances the simulation. The main
menu lives in ``crates/client/src/menu/``, the pause menu in
``pause_menu.rs``, and the bottom console in ``hud.rs``.

Design rules
-------------

* Buttons and text fields have a minimum target size of 48 logical pixels.
* The plotting-table palette uses graphite and slate surfaces, chalk text,
  and ocher for primary actions, invite codes, selection stripes, map-preview
  corners, and keyboard focus. Exo 2 uses semibold page headings and normal
  body text. Errors use red; status tags pair a square lamp with a word.
* Selection uses a left stripe; keyboard focus uses a full outline. Terrain
  previews supply the main color and visual emphasis. Menu and command-bar
  buttons share the same palette and interaction-state function.
* Primary actions are filled, secondary actions are outlined, and destructive
  actions use red outlines. Disabled controls carry ``InteractionDisabled``
  and ignore activation.
* Content scrolls vertically. Button rows wrap, and list/detail splits put
  the detail below the list when space is narrow.

Menu layout
------------

.. kroki::
   :type: plantuml
   :caption: Main menu with the New game page selected.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.1
   {+
     { <b>RTS | . | Menu / Singleplayer / New game }
     ==
     {
       {
         [Singleplayer]
         [Multiplayer]
         [Replays]
         [Settings]
         [Quit]
       } |
       {
         <b>Select map
         {
           { [Crossroads] | Four players, 40x40 tiles
             [Proving Grounds] | Two players, 48x32 tiles } |
           { Map preview
             Selected: Crossroads
             [Start game] }
         }
         ==
         Latest session message
       }
     }
   }
   @endsalt

The title bar shows **RTS** and a breadcrumb. The sidebar selects
**Singleplayer**, **Multiplayer**, **Replays**, or **Settings**. **Quit** is
hidden in browsers. A scrollable host holds one page. A message line below
it shows session results and errors; navigation clears the message. The
message line takes no space while empty.
A separate layer holds dialogs.

Compact mode starts below 760 logical pixels of available width, calculated
as window width divided by ``UiScale``. The 216-pixel sidebar becomes a
horizontal navigation row with **Solo**, **Online**, **Replays**, **Settings**,
and native-only **Quit** labels. The navigation row wraps when needed to keep
48-pixel targets. Content uses the full screen width below navigation;
list/detail columns wrap without a fixed minimum width. Page padding changes
from 40 to 16 pixels, and the title bar minimum from 72 to 56 pixels.
A **Back** button appears in the compact title bar when there is somewhere
to go. Changing interface scale can change the layout mode.

Scenes and widgets
-------------------

``pages.rs`` builds scenes as functions of ``View``. This model gathers the
menu, form, settings, catalog, library, and session status. ``PageKey``
captures the data a page shows. The page respawns only when its key changes.
``Bind`` updates text and ``ShowIf`` updates visibility in place.
``Action::enabled`` controls availability; ``Action::selected`` controls
selection highlights. ``update_looks`` combines those states with hover,
press, and focus using ``Tone``: ``Primary``, ``Secondary``, ``Danger``,
``Nav``, or ``Item``.

The renderer spawns pages and dialogs with ``Commands::spawn_scene`` under
``PageHost`` and ``DialogHost``. It restores focus to a button with the same
``Action`` after replacing a page. Rendering pauses while an ``AppState``
transition is pending, so a new page cannot outlive the menu being left.
Menu roots use ``DespawnOnExit(AppState::Menu)``.

``widgets.rs`` supplies reusable scenes:

* ``text``, ``heading``, ``hint``, and ``bound_text``.
* ``plain_button``, ``button``, ``wide_button``, and ``list_item``.
* ``row``, ``column``, ``panel``, and ``split``.
* ``map_image``, ``empty_preview``, ``badge``, ``text_field``, ``spinner``,
  and ``dialog``.

Fonts resolve through ``template(...)`` and the asset server when a scene
spawns. ``plain_button`` stays outside the tab order for gameplay controls.
Map images carry ``FitPreview(aspect)``. ``fit_previews`` sizes them from
their container's width in the previous layout, at most 300 pixels tall, so the
actions below a preview stay in view without stretching the image. Generated
map previews use nearest-neighbor sampling so terrain edges and player start
markers remain crisp when enlarged.

Pages and actions
------------------

.. list-table::
   :header-rows: 1
   :widths: 24 76

   * - Page
     - Content and actions
   * - Singleplayer
     - **Your next move**, **New game**, **Load game**, and a named preview
       of the latest save or a catalog map.
   * - New game
     - Catalog map list, selected map preview, player count, dimensions,
       and **Start game**. The local player controls player 1.
   * - Load game
     - Save browser with **All maps** or a map filter, **Most recent** or
       **Oldest first** order, a preview, **Load save**, and **Delete…**.
   * - Multiplayer: Connect
     - Server address, address hint, **Recent: <url>**, and **Connect**.
       Failure shows a reason and **Retry**. The typed address is kept.
   * - Multiplayer: Connecting
     - Server URL, spinner, and **Cancel**.
   * - Multiplayer: Rooms
     - Server host, **Connected** badge, **Refresh**, **Disconnect**, and
       public rooms. Each row shows the host's name, occupied/capacity counts,
       and map. Detail offers **Join room**; footer actions are
       **Join by code** and **+ Create room**.
   * - Multiplayer: Lobby
     - **Room: <host>'s room**, invite code, visibility, named slots,
       readiness toggle, **Start match**, and **Leave room**.
   * - Replays
     - Replay browser with the same filters and ordering as saves. A
       **Last match** row appears when a recording exists and no map filter
       is set. It offers **Watch replay** and **Save to library**. Stored
       records offer **Watch replay**, **Delete…**, and browser-only
       **Export**. Browsers also offer **Import replay file**.
   * - Settings
     - **Player name**, **Interface scale** (75, 100, 125, or 150 percent),
       **Edge scrolling**, and desktop-only **Fullscreen**.

The room page marks slots with **(host)** and **(you)** and badges for
**Ready**, **Not ready**, or **Open slot**. **Copy code** uses the system
clipboard and is hidden on Android. The readiness button toggles between
**Ready** and **Not ready**. Only the host can activate **Start match**, once
all slots are filled and ready. Guests see a disabled **Start (host only)**
button. A hint explains whether players, readiness, or the host are awaited.

The server address accepts at most 256 characters, without whitespace or
control characters. **Connect** requires a ``ws://`` or ``wss://`` prefix
and a nonempty remainder. Invite codes contain six ASCII letters or digits
and normalize to uppercase. Join and create actions wait for a connection;
repeated join requests are disabled while one is pending.

Player names use ``rts_net::validate_name``. Invalid typed names remain
visible with a hint, while rooms use the last valid name. Settings persist
in ``settings.ron``. ``SettingsPlugin`` applies ``UiScale`` and desktop
borderless fullscreen. Disabling edge scrolling sets the camera's
``edge_margin`` to zero. World-input exclusion for the HUD and pause button scales with
``UiScale`` too.

Dialogs and navigation
-----------------------

**Join room with invite code** contains a code field, hint, inline error,
and **Cancel** / **Join room** buttons. **Create room** offers **Public**
and **Private**, a visibility hint, inline errors, and confirmation buttons.
Both dialogs close when a lobby arrives or the connection drops. Deleting a
record, leaving a room, leaving a live match, and quitting while connected
use confirmation dialogs. Dialogs dim the screen and use a modal tab group.

Tab and Shift+Tab move focus. Focused controls scroll into view. Enter in
the address field connects; Enter in the invite field joins. Mouse wheels
and touch drags scroll content. Escape or **Back** follows this order:

1. Close an open dropdown.
2. Close an open dialog.
3. From New game or Load game, return to the Singleplayer entry page.
4. From Connecting or Rooms, disconnect.
5. From a lobby, open the leave-room confirmation.

Touch keypad
-------------

A completed touch tap on a text field opens the in-game keypad. Mouse and
physical keyboards use the field directly. Opening after the finger lifts
prevents the same touch from pressing a key. Opening clears text-field focus.

.. kroki::
   :type: plantuml
   :caption: Keypad edits stay in a draft until Done.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   [*] --> Closed
   Closed --> Open : touch tap on a text field
   Open : overlay blocks the menu
   Open : KeypadDraft holds field, value, shift, page
   Open --> Open : character, Delete, Clear, Shift, page
   Open --> Closed : Done: commit to field and MenuForm\nupdate settings for a valid name
   Open --> Closed : Cancel: discard draft
   Open --> Closed : field disappears or menu exits
   @enduml

* **Invite code** shows uppercase letters and digits, up to six characters.
* **Server address** has letters and symbols pages. It includes digits,
  Shift, and URL punctuation (``: / . - _ ? & = % + [ ] #``), up to 256
  characters.
* **Player name** has letters, digits, space, ``- _ . '`` and Shift, up to
  the protocol's name limit.

Keys use six columns. A preview shows the draft or a placeholder and a
character count. **Done** commits; **Cancel** discards.

RTS console
-----------

The HUD (``hud.rs``) spawns on ``OnEnter(AppState::InGame)``. Its three
sections follow the conventional RTS layout: minimap left, selected units
center, command card right. The graphite and slate console has a thin top
border; ocher marks selected minimap units and an armed Move command.

* The terrain minimap (``hud/minimap.rs``) builds one nearest-sampled image
  from the current match, including server-provided maps. Live player-colored
  unit dots and a camera outline update as UI nodes. Click, tap, or drag it
  to center the camera without issuing orders.
* The center shows unit names, health totals, order status, and the current
  mouse or touch hint. A single unit has a sprite portrait; a group shows
  up to twelve 48-pixel portraits. Activating a portrait narrows the selection
  to that unit.
* The command card uses fixed 48-pixel slots. **Move** (M) arms destination
  targeting, **Stop** (X) queues a Stop, and **Clear** empties the selection.
  Icons load as PNG images from ``assets/textures/ui/commands/`` (``move.png``,
  ``stop.png``, ``clear.png``), bundled in the normal asset pack. Their white
  glyphs and transparent backgrounds allow tinting for hover, pressed, and
  disabled states. Replace these files to change the command artwork.
  Empty slots are noninteractive. Controls retain the shared button states
  and are disabled without a selection, while orders are disabled, or while
  the pause menu is open.

The console is 184 logical pixels high. Below 620 logical pixels of width,
its height becomes 152 and the command card changes from three rows to two.
Below 560 the center panel hides, with a selection count below the command
card instead. The minimap shrinks with available width and hides below 240 logical pixels
to preserve touch targets. Layout and input
exclusion use window width divided by ``UiScale``.

Pause menu
----------

``PauseMenuPlugin`` spawns the top-left 48-pixel pause button at (12, 12)
and a modal from ``pause_menu.rs``. The rest of the top edge remains playable.
The button or Escape toggles the menu without clearing selected units.
Escape first cancels Move targeting, then closes a confirmation dialog,
then toggles the menu.

**Resume**, **Save game** (solo only), **Save replay**, and **Leave** are
stacked in the menu. The latest session message appears there as well.
Solo simulation and replay playback pause while the menu or its confirmation
is open. Online matches keep receiving and applying authoritative turns;
the menu explicitly says the match continues. All world controls are blocked
by menus and dialogs, including gestures that cross an overlay boundary.

**Leave** asks for confirmation during live solo and online matches. Stopped
matches and replays leave at once. The confirmation host shares the main
menu's dialog scenes and renders above the pause menu. Dialog content
scrolls when a short window cannot fit all actions. Pause and command
buttons stay outside the tab order; menu actions use a modal tab group.
