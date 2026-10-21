Build and play your first match
===============================

This walkthrough starts in the RTS repository and ends with a solo match and a
two-player online match on your machine. For browser and Android builds, see
:doc:`guides/web` and :doc:`guides/android`.

Install build prerequisites
---------------------------

You need:

* A Rust toolchain installed through rustup. The repository's
  ``rust-toolchain.toml`` selects nightly; rustup installs it on first use.
* On Linux, clang and the mold linker. ``.cargo/config.toml`` links Linux
  builds with ``clang -fuse-ld=/usr/bin/mold``.
* On Linux, Bevy's windowing and graphics libraries (Wayland, X11, and
  libxkbcommon development packages, plus a Vulkan driver). See Bevy's
  `Linux dependency list <https://github.com/bevyengine/bevy/blob/main/docs/linux_dependencies.md>`_.
  The game disables Bevy audio and gamepads, so ALSA and udev are not needed.

Every Rust dependency is pure Rust, so the build compiles no C or C++ code.

Run the game
------------

From the repository root:

.. code-block:: sh

   cargo run

The workspace's default member is the client crate, so this builds and starts
the ``rts`` binary. The first build also packs ``assets/`` into
``target/debug/assets.pack``; the game reads that file beside its executable.
Dependencies are optimized even in debug builds (``opt-level = 3``), so the
first build takes a while and later builds are fast.

The game opens on the lobby. Choose **Play solo** to start a match on the
bundled *Proving Grounds* map as player 1.

Play a few orders
-----------------

* Click one of your blue units, or drag a box around several of them.
* Right-click open ground to move them. Water and rock block movement.
* Press **X** or the **Stop** button to cancel their orders.
* Pan with WASD, the arrow keys, the screen edges, or a middle-mouse drag.
  Zoom with the wheel.

On a touch screen, tap or drag with one finger to select, tap ground to
move, and drag or pinch with two fingers to move the camera.
:doc:`guides/playing` explains the rules behind movement, and
:doc:`reference/controls` lists every control.

Host a multiplayer match
------------------------

Start the authoritative server in a second terminal:

.. code-block:: sh

   cargo run -p rts_server --bin rts-server

It listens on ``ws://127.0.0.1:9001``. Then start two clients:

1. In the first client, keep the server address ``ws://127.0.0.1:9001`` and
   choose **Create room**. The lobby shows a six-character room code.
2. In the second client, enter the same address and the room code, then
   choose **Join room**.
3. When both players have loaded the server's map and rules, the host's
   **Start match** button becomes available.

The server simulates the match at 20 ticks per second and sends every client
the same ordered commands. :doc:`guides/multiplayer` covers LAN and Internet
hosting, and :doc:`architecture/networking` explains how clients stay in sync.

Save and watch a replay
-----------------------

During or after a match, choose **Save replay**. Back in the lobby,
**Replay last** plays the most recent recording, and **Load saved replay**
opens the saved file. Playback re-simulates every turn and verifies the final
state. See :doc:`guides/replays`.

Run the checks
--------------

Before submitting a change, run:

.. code-block:: sh

   cargo test --workspace
   cargo clippy --workspace --all-targets
   cargo check --target wasm32-unknown-unknown

:doc:`guides/testing` describes what the test suites cover and the additional
platform checks.

Where to go next
----------------

* :doc:`guides/modding` to change units, terrain, and maps without
  rebuilding clients.
* :doc:`architecture/index` for the crate layout and how a match runs.
* :doc:`project/roadmap` for milestone status.
