Assets and data loading
=======================

All game content lives in ``assets/`` at the repository root. The
``rts_assets`` crate (``crates/assets``) packs it into one file at build time,
opens that file on each platform, and registers Bevy loaders for the RON data
files.

Content
-------

.. kroki::
   :type: plantuml
   :caption: Content in the assets folder.
   :align: center

   @startsalt
   skinparam backgroundColor transparent
   scale 1.2
   {
       {T
           + assets/ | .
           ++ data/ | .
           +++ core.rules.ron | Terrain and unit types
           ++ maps/ | .
           +++ proving_grounds.map.ron | Two-player sample map, 48×32 tiles
           ++ textures/terrain/ | 128×128 PNG per terrain type
           ++ fonts/ | .
           +++ Exo2.ttf | Interface font
           +++ Exo2-OFL.txt | Font license
       }
   }
   @endsalt

The rules and map are simulation data and also travel over the network.
Textures and the font are presentation assets: they stay local to each
client and never affect simulation state.

The asset pack
--------------

``crates/assets/build.rs`` runs ``bevy_asset_packer``'s ``AssetBundler`` on
``assets/`` and writes ``assets.pack`` into the Cargo profile directory (for
example ``target/debug/`` or ``target/wasm32-unknown-unknown/release/``). It
reruns whenever ``assets/`` changes, so editing a map and running
``cargo run`` picks up the change.

``with_bundle`` opens the pack and then starts the app:

.. list-table::
   :header-rows: 1
   :widths: 20 80

   * - Platform
     - How the pack is opened
   * - Desktop
     - ``AssetBundle::from_file`` on ``assets.pack`` next to the running
       executable.
   * - Web
     - ``AssetBundle::from_url("assets.pack")``, awaited in
       ``wasm_bindgen_futures::spawn_local`` before the Bevy app starts. Later
       reads use HTTP range requests.
   * - Android
     - Opened through the ``AssetManager`` from the APK, where it is stored
       uncompressed, and read as one mapped buffer. The asset handle is leaked
       so the bytes stay valid for the whole process.

A missing or corrupt pack panics at startup with the path and the error.

.. kroki::
   :type: plantuml
   :caption: Startup from the asset pack to the lobby.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   hide footbox
   participant "rts_client::run" as run
   participant "rts_assets::with_bundle" as bundle
   participant "Bevy App" as app
   participant "game.rs" as game
   run -> bundle : open this platform's pack
   bundle -> app : build_app(bundle).run()
   app -> app : BundledAssetPlugin\nserves AssetServer reads
   app -> game : Startup: load_match_data
   game -> app : load data/core.rules.ron\nload maps/proving_grounds.map.ron
   app --> game : RulesAsset, MapAsset loaded
   game -> game : build MatchSetup,\nvalidate with simulation()
   game -> app : AppState::Loading → Menu
   @enduml

Data loaders
------------

``DataPlugin`` registers two asset types and one generic loader:

.. list-table::
   :header-rows: 1
   :widths: 25 20 55

   * - Asset
     - Extension
     - Content
   * - ``RulesAsset(Rules, String)``
     - ``rules.ron``
     - Parsed and validated rules, plus the original text.
   * - ``MapAsset(MapData, String)``
     - ``map.ron``
     - Parsed map structure, plus the original text.

Both keep the original text because the match setup carries text, not parsed
values. The client builds its solo ``MatchSetup`` from these strings, exactly
as the server does from files, so solo play, online play, and replays all load
the simulation through ``MatchSetup::simulation``. See
:ref:`fixed-point-data` for why the text is preserved.

The loader reads the file, checks UTF-8, and calls ``Rules::from_ron`` or
``MapData::from_ron``. A failure becomes a Bevy load error. If the bundled
setup does not build a valid simulation, the client logs
``cannot start `maps/proving_grounds.map.ron`: <error>`` and stays in the
loading state.

Textures and fonts
------------------

* Terrain textures are loaded when a match starts, with
  ``RenderAssetUsages::MAIN_WORLD`` so their pixels stay on the CPU until they
  are assembled into one texture array. See :doc:`rendering`.
* ``GameFont`` loads ``fonts/Exo2.ttf`` at startup. Bevy's built-in font is
  disabled, so every text node uses this handle.

Adding an asset
---------------

1. Put the file under ``assets/``.
2. Load it through the ``AssetServer`` with a path relative to ``assets/``.
3. Use only formats enabled in the workspace Bevy features (PNG for images).

Mods sent by a server cannot add textures; see :doc:`../guides/modding`.
