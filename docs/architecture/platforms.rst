Platform support
================

One pure-Rust codebase targets desktop (Linux, Windows) and Android. Platform
differences are isolated behind ``cfg`` attributes in a few places; everything
else is shared. This page lists every difference so you can keep them
contained.

Targets
-------

.. list-table::
   :header-rows: 1
   :widths: 24 38 38

   * - Concern
     - Desktop (Linux, Windows)
     - Android
   * - Entry point
     - ``crates/client/src/main.rs``
     - ``crates/android`` ``#[bevy_main]``
   * - Window
     - winit, X11 or Wayland on Linux
     - ``GameActivity``, landscape
   * - Graphics
     - wgpu native backend
     - Vulkan or GLES via wgpu
   * - Asset pack
     - File next to the executable
     - Mapped from the APK
   * - Networking
     - Pure-Rust QUIC (``quinn`` + ``rustls`` + ``glomar-rustls``), ``quic://``
     - Pure-Rust QUIC (``quinn`` + ``rustls`` + ``glomar-rustls``), ``quic://``
   * - Saves, replays, locales, asset cache, settings
     - Documents in the platform data directory
     - Internal app storage
   * - Copy invite code
     - System clipboard
     - Hidden; no system clipboard backend
   * - Fullscreen setting / Quit
     - Both available
     - Fullscreen hidden; Quit available
   * - Server
     - ``rts-server`` binary
     - Not shipped

Where platform code lives
-------------------------

.. list-table::
   :header-rows: 1
   :widths: 35 65

   * - Location
     - Platform-specific behavior
   * - ``rts_assets::with_bundle`` and ``open_bundle``
     - How ``assets.pack`` is opened.
   * - ``storage.rs``
     - Desktop data paths (``XDG_DATA_HOME`` / ``LOCALAPPDATA``) and Android
       internal app storage.
   * - ``crash.rs``
     - Panic hook, rotating ``game.log``, stored crash reports, and manual
       export to ``~/Downloads`` or app-private ``exports/``.
   * - ``menu/pages.rs`` and ``settings.rs``
     - Platform visibility for clipboard and fullscreen.
   * - ``crates/android``
     - The only crate with ``android-game-activity``.

Crash reports and logs
----------------------

Diagnostics stay on the player's machine; nothing is uploaded. ``crash.rs``
installs Bevy's platform panic hook first, then chains its Rust panic hook
before opening the asset pack or building the remaining plugins. Startup
failures are captured too, and Bevy still prints the panic. The hook writes
``crash-report.txt`` holding the package version, platform, build profile, UTC
time, panic message, location, backtrace, and the end of the log. The next
launch reads that report, the menu offers it once, and **Export report** or
**Discard** settles it. Nothing removes a stored report at startup, and a run
that ends normally leaves none behind.

``LogPlugin::custom_layer`` adds a tracing layer that appends every event to
``game.log`` in the data folder. The file rotates to ``game.log.1`` once it
passes 512 KiB, so it cannot grow without bound. If rotation fails, the active
log is truncated before appending again.

Export is a separate step, done while the game runs normally, so it can differ
by platform:

.. list-table::
   :header-rows: 1
   :widths: 20 80

   * - Platform
     - Export
   * - Desktop
     - A copy in the downloads folder, or ``exports/`` in the data folder when
       there is none. The menu names the path.
   * - Android
     - A text file in the device's shared Downloads collection. The log and
       pending report stay in app-private storage until export succeeds.

A panic hook sees only Rust panics. Segfaults, out-of-memory kills, and Android
ANRs leave no report; the log still shows what the game was doing. Release
builds keep ``debug = "line-tables-only"`` so backtraces can name source lines.

Pure Rust
---------

Every dependency must build for desktop and Android without a C toolchain.
Consequences so far:

* QUIC and TLS 1.3 use ``quinn`` and ``rustls`` with default crypto features
  disabled, wired to the pure-Rust ``glomar`` and ``glomar-rustls`` provider
  (no ``ring`` or ``aws-lc-rs``).
* Lua will be ``omnilua``, not ``mlua`` or Luau.
* Wire compression uses ``miniz_oxide``, not zlib bindings.
* Bevy audio and gamepad support are disabled; they would pull in platform
  libraries and are not used yet.

Build profiles
--------------

* Dependencies are built with ``opt-level = 3`` even in debug builds, so the
  game runs at full speed while game crates compile quickly.
* Release builds use ``lto = "fat"`` and ``codegen-units = 1``.
* The workspace uses the nightly toolchain. On Linux, ``.cargo/config.toml``
  links with clang and mold; all targets use ``-Zshare-generics=y``.

Verification status
-------------------

Native rendering and QUIC multiplayer sessions have been exercised; Android
builds and activity compile but have not yet run on a device. See
:doc:`../project/verification`.
