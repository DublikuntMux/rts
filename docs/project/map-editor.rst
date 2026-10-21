Planned: map editor (M7)
========================

The map editor will be a mode inside the game binary, desktop first, reusing
the renderer and the data formats. Nothing here is implemented yet.

Design
------

* A new ``rts_editor`` crate plugged into the client, created when M7 starts.
* Interface with ``bevy_egui`` if it supports the Bevy version at the time.
  Otherwise use Bevy UI, like the lobby.
* The editor reads and writes ``*.map.ron`` in the existing format
  (:doc:`../reference/map-format`); maps stay RON, not Bevy scene files.
* Decorative groups can use scene composition; see
  :ref:`client-scenes`.

Tools
-----

* Terrain brush and fill.
* Player starts.
* Unit placement with owner.
* Regions and script assignment (from M6).
* Map properties: name and size.
* Undo and redo.
* Save and load ``*.map.ron``.
* Play-test the current map.

Validation
----------

The editor should validate with the same ``Sim::new`` checks the game and
server use, so a saved map always loads. Play-testing can build a
``MatchSetup`` from the editor's text and start a solo session directly.
