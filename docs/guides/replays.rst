Save games and watch replays
=============================

Every solo and online match records its setup and turns automatically.
Playback re-simulates those turns and verifies the final state hash. Online
recordings include only turns whose hash matches the server's.

Save and load a game
---------------------

Choose **Save** in the top bar of a running solo match. Each save adds
a record such as **Save 01**. From the menu, choose **Singleplayer → Load
game**, select a record, and choose **Load save**. Loading verifies the
recorded turns and continues from their final state. The recording keeps
growing. Online games cannot be saved for continuation.

Save a replay
--------------

Choose **Save replay** in the session bar to add the current recording to
the replay library. After leaving, open **Replays**, select **Last match**,
and choose **Save to library**. Automatic names such as **Replay 01** use
one more than the highest number in use.

**Last match** holds the current or last recording in memory. It is not a
persistent library entry. Save it before closing the app or recording another
match.

Browse and watch
-----------------

**Replays** lists saved recordings. **All maps** can filter by map;
**Most recent** and **Oldest first** change the order. **Last match** appears
when a recording exists and no map filter is active. Select a row and choose
**Watch replay**. Stored records show a map preview and duration;
**Delete…** asks for confirmation.

Loading a record checks its version, setup, turn sequence, limits, and final
hash before starting playback. Playback runs at 20 ticks per second. The
session bar shows *Watching replay*, and order buttons stay disabled. At the
end it shows *Replay finished and verified.* **Leave** returns at once.

Storage
--------

Saves and replays use separate ``saves/`` and ``replays/`` shelves. Each
record has two documents: ``<id>.meta.ron`` for listing and previews, and
``<id>.data.ron`` for replay data. IDs are 13-digit millisecond stamps. Each
save creates a record; it does not replace the previous save.

.. list-table::
   :header-rows: 1
   :widths: 25 75

   * - Platform
     - Storage root
   * - Linux
     - ``$XDG_DATA_HOME/rts``, falling back to ``~/.local/share/rts``.
   * - Windows
     - ``%LOCALAPPDATA%/rts``, unless ``XDG_DATA_HOME`` is set.
   * - Android
     - The app's internal data directory.
   * - Other native platforms
     - ``$XDG_DATA_HOME``, then ``LOCALAPPDATA``, then ``~/.local/share``,
       followed by ``rts/``.
   * - Browser
     - This site's ``localStorage``, with keys prefixed by ``rts/``.

Native writes use a temporary ``<file>.tmp`` and rename it to finish.
Browser documents use deflate compression. Storage refuses documents over
32 MiB and reports unavailable or full storage. Browser storage belongs to
the site and browser profile; clearing site data removes saves, replays,
and settings.

Import and export
------------------

In browsers, **Import replay file** opens a picker and adds a verified RON
recording to the library. Select a stored replay and choose **Export** to
download ``match.replay.ron``. **Save replay** stores locally; use **Export**
to keep a separate file.

Native and Android builds have no import or export buttons. Native records
are in the storage folder above. Keep both metadata and data documents when
copying a library record. The data document is plain replay RON.

Errors
-------

.. list-table::
   :header-rows: 1
   :widths: 45 55

   * - Message
     - Meaning
   * - *There is no recorded match yet.*
     - Play a match first.
   * - *Cannot load: final replay hash mismatch*
     - The recording is edited or uses an incompatible simulation.
   * - *Cannot load: unsupported protocol version*
     - The recording uses a different protocol version. The current version
       is 5.
   * - *Replay file is too large.*
     - The imported file exceeds 32 MiB.

Inspect a replay
-----------------

Replay data is plain RON text. Online replays embed the server's minified map
and rules as strings. Textures stay local. See
:doc:`../reference/replay-format` for the format and
:doc:`../architecture/sessions` for library storage details.
