Build and host the browser version
==================================

The browser client is the same ``rts`` binary compiled to
``wasm32-unknown-unknown`` and packaged by `Trunk <https://github.com/trunk-rs/trunk>`_. It
renders with WebGL2 and connects to servers through the browser's WebSocket.

Install the tools
-----------------

.. code-block:: sh

   rustup target add wasm32-unknown-unknown
   cargo install --locked trunk

Trunk downloads matching ``wasm-bindgen`` and ``wasm-opt`` tools on first use.

Run a development build
-----------------------

From the repository root:

.. code-block:: sh

   trunk serve

Open the address Trunk prints, normally ``http://127.0.0.1:8080``. Trunk
rebuilds and reloads on source changes; ``Trunk.toml`` ignores the
``android/`` directory.

How the build is assembled
--------------------------

.. kroki::
   :type: plantuml
   :caption: Files involved in a browser build.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   file "index.html" as html
   file "crates/client" as client
   folder "assets/" as assets
   node "trunk build" as trunk
   node "rts_assets build.rs" as build
   file "target/wasm32-unknown-unknown/<profile>/assets.pack" as pack
   folder "dist/" as dist {
     file "index.html" as out_html
     file "rts-<hash>.js" as js
     file "rts-<hash>_bg.wasm" as wasm
     file "assets.pack" as out_pack
   }
   html --> trunk : data-trunk link
   client --> trunk
   assets --> build
   trunk --> build : cargo build
   build --> pack
   trunk --> js
   trunk --> wasm : wasm-opt -Oz
   trunk --> out_html
   pack --> out_pack : post_build hook copies
   @enduml

* ``index.html`` tells Trunk to build ``crates/client`` with the ``rts``
  binary and to optimize the WebAssembly for size (``data-wasm-opt="z"``). It
  makes the canvas fill the page.
* The ``rts_assets`` build script packs ``assets/`` into
  ``assets.pack`` in the Cargo profile directory.
* A ``post_build`` hook in ``Trunk.toml`` copies that pack into Trunk's
  staging directory, so it ships beside the generated files.

At startup the client fetches ``assets.pack`` with an asynchronous request and
starts Bevy only after the pack's index has loaded. See
:doc:`../architecture/assets`.

Build for release
-----------------

.. code-block:: sh

   trunk build --release

The output in ``dist/`` is a static site: ``index.html``, the JavaScript glue,
the ``.wasm`` module, and ``assets.pack``. The release profile uses fat LTO
and one codegen unit.

Host the files
--------------

Any static file server works if it meets two requirements:

* It serves ``.wasm`` with the ``application/wasm`` content type.
* It supports HTTP byte-range requests and answers them with
  ``206 Partial Content``. The asset pack loader reads the index and then
  individual assets on demand. A server that always returns the whole file
  is not sufficient.

To test the release output locally, use a server that supports ranges, such as
``trunk serve --release`` or Caddy's ``file_server``.

Connect to a game server
------------------------

A page served over plain HTTP can connect to ``ws://`` servers. A page served
over HTTPS can only connect to ``wss://``, so put ``rts-server`` behind a TLS
reverse proxy. See :doc:`multiplayer` for hosting options.

.. list-table::
   :header-rows: 1
   :widths: 35 65

   * - Page origin
     - Server address
   * - ``http://localhost:8080``
     - ``ws://127.0.0.1:9001`` or a LAN ``ws://`` address.
   * - ``https://example.com``
     - ``wss://example.com/rts`` through a reverse proxy to the server.

Browser differences
-------------------

* **Save replay** downloads ``match.replay.ron`` instead of writing a file.
* **Load replay** opens a file picker instead of reading a fixed path.
* The browser handles WebSocket ping and pong frames itself.
* The server crate compiles for ``wasm32`` as a stub only; there is no
  in-browser server.
