Interface: menu, bars, and keypad
==================================

The interface uses Bevy UI widgets and observers. It reads ``SessionStatus``
and sends ``SessionAction`` messages. Gameplay controls read ``Selection``
and queue commands. Interface code never advances the simulation. The main
menu lives in ``crates/client/src/menu/``, the session bar in
``session_bar.rs``, and the command bar in ``hud.rs``.

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
spawns. ``plain_button`` stays outside the tab order for the in-game bar.
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
``edge_margin`` to zero. World-input exclusion for both bars scales with
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

Command bar
------------

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
------------

``SessionBarPlugin`` spawns a scene from ``session_bar.rs`` on
``OnEnter(AppState::InGame)``. The bar is 56 logical pixels high before
interface scaling. It shows **Solo**, **Online**, or **Replay**, plus the
latest session message. Short titles leave room for save controls on narrow
screens.

**Save** appears for solo play and is enabled while the match runs; it saves
the game. **Save replay** stores the current recording in the replay library.
The short **Save** label keeps the controls reachable on narrow screens.
**Leave** asks for confirmation during live solo and online matches. Stopped matches and
replays leave at once. Bar buttons stay outside the tab order, so gameplay
keys never activate them. Its dialog host shares the menu's dialog scenes.
Both bars are excluded from world input; see :doc:`input`.
