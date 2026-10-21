Platform support
================

One codebase targets desktop, the browser, and Android. Platform differences
are isolated behind ``cfg`` attributes in a few places; everything else is
shared. This page lists every difference so you can keep them contained.

Targets
-------

.. list-table::
   :header-rows: 1
   :widths: 18 27 27 28

   * - Concern
     - Desktop
     - Web (``wasm32``)
     - Android
   * - Entry point
     - ``crates/client/src/main.rs``
     - Same binary via Trunk
     - ``crates/android`` ``#[bevy_main]``
   * - Window
     - winit, X11 or Wayland on Linux
     - Canvas that fits the page
     - ``GameActivity``, landscape
   * - Graphics
     - wgpu native backend
     - WebGL2
     - Vulkan or GLES via wgpu
   * - Asset pack
     - File next to the executable
     - Fetched, then range requests
     - Mapped from the APK
   * - WebSocket
     - ``ewebsock`` native, ``ws://``
     - Browser WebSocket, ``ws://`` and ``wss://``
     - ``ewebsock`` native, ``ws://``
   * - Replay save
     - ``latest.replay.ron`` in the data directory
     - Download ``match.replay.ron``
     - Internal app storage
   * - Replay load
     - Read the saved file
     - File picker
     - Read the saved file
   * - Server
     - ``rts-server`` binary
     - Stub only
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
   * - ``session.rs`` ``Connection::connect``
     - Native builds refuse ``wss://``.
   * - ``replay_io.rs``
     - Native file paths, Android internal storage, and browser ``Blob``
       download and file input via ``web-sys``.
   * - ``lobby.rs``
     - The replay load button label.
   * - ``rts_server`` ``lib.rs`` and ``main.rs``
     - Runtime and binary compiled only for non-``wasm32`` targets.
   * - ``crates/android``
     - The only crate with ``android-game-activity``.

Target-specific dependencies are declared in each crate's
``[target.'cfg(...)'.dependencies]``: ``wasm-bindgen``, ``web-sys``,
``js-sys``, and ``wasm-bindgen-futures`` for the browser; Tokio and
``tokio-tungstenite`` for the native server.

Pure Rust
---------

Every dependency must build for native, ``wasm32-unknown-unknown``, and
Android without a C toolchain. Consequences so far:

* Lua will be ``omnilua``, not ``mlua`` or Luau.
* Setup compression uses ``miniz_oxide``, not zlib bindings.
* Native TLS is disabled. Standard Rustls crypto providers (``ring``,
  ``aws-lc-rs``) include C and assembly. Browsers get ``wss://`` from the
  platform; native clients and the server use plain ``ws://``.
* Bevy audio and gamepad support are disabled; they would pull in platform
  libraries and are not used yet.

Build profiles
--------------

* Dependencies are built with ``opt-level = 3`` even in debug builds, so the
  game runs at full speed while game crates compile quickly.
* Release builds use ``lto = "fat"`` and ``codegen-units = 1``. The web build
  also runs ``wasm-opt -Oz``.
* The workspace uses the nightly toolchain. On Linux, ``.cargo/config.toml``
  links with clang and mold; all targets use ``-Zshare-generics=y``.

Verification status
-------------------

Native rendering and browser sessions have been exercised; Android builds and
activity compile but have not yet run on a device. See
:doc:`../project/verification`.
