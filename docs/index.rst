RTS documentation
=================

RTS is a top-down 2D real-time strategy game in the classic style: gather,
build, and fight. One pure-Rust codebase runs on desktop, in the browser, and
on Android, with first-class touch controls and online multiplayer. A
server-authoritative deterministic simulation keeps every client in step;
Lua mission scripting and a built-in map editor are planned.

Start with :doc:`getting-started` to build the game, play a solo match, and
host a multiplayer room.

Find what you need
------------------

* **Play and host:** :doc:`guides/playing`, :doc:`guides/multiplayer`, and
  :doc:`guides/replays`.
* **Change game content:** :doc:`guides/modding`,
  :doc:`reference/rules-format`, and :doc:`reference/map-format`.
* **Build for other platforms:** :doc:`guides/web` and :doc:`guides/android`.
* **Look up a format or limit:** :doc:`reference/protocol`,
  :doc:`reference/setup-frame`, :doc:`reference/replay-format`, and
  :doc:`reference/limits`.
* **Work on the code:** :doc:`architecture/index`,
  :doc:`architecture/determinism`, and :doc:`project/contributing`.
* **See what comes next:** :doc:`project/roadmap`.
* **Edit this handbook:** :doc:`documentation`.

.. toctree::
   :maxdepth: 1
   :caption: Get started

   getting-started

.. toctree::
   :maxdepth: 1
   :caption: Play and host

   guides/playing
   guides/multiplayer
   guides/replays
   guides/modding

.. toctree::
   :maxdepth: 1
   :caption: Build and test

   guides/web
   guides/android
   guides/testing
   guides/releasing

.. toctree::
   :maxdepth: 1
   :caption: Reference

   reference/controls
   reference/server-cli
   reference/rules-format
   reference/map-format
   reference/graphics-format
   reference/protocol
   reference/setup-frame
   reference/replay-format
   reference/limits

.. toctree::
   :maxdepth: 1
   :caption: Architecture

   architecture/index
   architecture/determinism
   architecture/coordinates
   architecture/simulation
   architecture/movement
   architecture/assets
   architecture/client
   architecture/rendering
   architecture/input
   architecture/interface
   architecture/sessions
   architecture/server
   architecture/networking
   architecture/platforms

.. toctree::
   :maxdepth: 1
   :caption: Project

   project/roadmap
   project/scripting
   project/map-editor
   project/skirmish-ai
   project/verification
   project/contributing
   documentation
