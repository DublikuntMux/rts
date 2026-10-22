Build for Android
=================

The Android app is a Gradle project in ``android/``. Gradle builds the
``rts_android`` Rust library with
`cargo-ndk <https://github.com/bbqsrc/cargo-ndk>`_, packages it with the asset
pack, and launches it through Android's ``GameActivity``.

Install the tools
-----------------

You need:

* The Android SDK with platform 37.2 and build tools 37.0.0, and NDK
  30.0.16248370. Android Studio installs them, or use ``sdkmanager``.
* A JDK 17 or newer for Gradle.
* The Rust Android targets and cargo-ndk:

  .. code-block:: sh

     rustup target add aarch64-linux-android x86_64-linux-android
     cargo install cargo-ndk

Set ``sdk.dir`` in ``android/local.properties`` or ``ANDROID_HOME`` so Gradle
can find the SDK. Gradle sets ``ANDROID_NDK_HOME`` for cargo-ndk itself.

Build and install
-----------------

With a device connected through ``adb`` or an emulator running:

.. code-block:: sh

   cd android
   ./gradlew installDebug

Use ``./gradlew assembleRelease`` for an optimized APK. The release build is
signed with the debug key for now. The release workflow overrides the version
with ``-PversionName`` and ``-PversionCode``; without them Gradle uses the
defaults in ``app/build.gradle.kts``.

How the build works
-------------------

.. kroki::
   :type: plantuml
   :caption: Android build pipeline for one variant.
   :align: center

   @startuml
   skinparam backgroundColor transparent
   start
   :Gradle variant (debug or release);
   :cargoBuild<Variant> task;
   :cargo ndk -t arm64-v8a -t x86_64 -P 31\nbuild -p rts_android [--release];
   fork
     :librts_android.so per ABI\n→ build/rustJniLibs/<variant>;
   fork again
     :copy target/aarch64-linux-android/<profile>/assets.pack\n→ build/rustAssets/<variant>;
   end fork
   :package APK\n(assets.pack stored uncompressed);
   :MainActivity loads librts_android;
   :bevy_main → rts_client::run();
   stop
   @enduml

* ``app/build.gradle.kts`` registers a ``cargoBuild<Variant>`` task for each
  variant. It always runs and lets Cargo decide what to rebuild.
* The task builds for ``arm64-v8a`` and ``x86_64`` with minimum API 31 and adds
  the libraries and ``assets.pack`` as generated sources.
* ``noCompress += "pack"`` stores the asset pack uncompressed so the game can
  memory-map it directly from the APK.
* ``crates/android`` is a ``cdylib`` with a ``#[bevy_main]`` entry point that
  calls ``rts_client::run()``. It is the only crate that enables Bevy's
  ``android-game-activity`` feature.

The activity
------------

``MainActivity`` extends ``GameActivity`` from
``androidx.games:games-activity`` 4.4.x, the line supported by the
``android-activity`` crate that Bevy uses. It:

* Loads ``librts_android.so``.
* Hides the system bars and restores immersive mode when focus returns.
* Runs in sensor landscape and handles configuration changes itself, so
  rotation and keyboard changes do not restart the game.
* On Android 17 (API 37) and newer, requests the *Nearby devices* local
  network permission (``ACCESS_LOCAL_NETWORK``).

Network access
--------------

The manifest requests ``INTERNET`` and ``ACCESS_LOCAL_NETWORK``. On Android 17
and newer, allow *Nearby devices* access when asked to connect to servers on
your LAN. If you deny it, solo play and Internet servers still work; grant the
permission in the system app settings before retrying a LAN server.

Like other native clients, Android supports ``ws://`` servers only.

Storage
-------

Saved games use ``saves/`` and the replay library uses ``replays/`` in the
app's internal data directory. Each record has ``<id>.meta.ron`` and
``<id>.data.ron`` documents. Settings persist in ``settings.ron`` there too.
Uninstalling the app deletes this data. See :doc:`replays`.

Menu controls
-------------

The menu shares the desktop pages and uses a compact rail on narrow screens.
Tapping the server address, invite code, or player name focuses the field and
opens the system keyboard. **Settings**
offers interface scale and edge scrolling; fullscreen is controlled by the
activity. **Copy code** is hidden because Android has no system clipboard
backend. Replay import and export buttons are browser-only.

Check without a device
----------------------

To confirm the Rust code compiles for Android without Gradle:

.. code-block:: sh

   cargo ndk -t arm64-v8a check --workspace

.. note::

   Milestone verification so far has compiled the Android build but has not
   run on a physical device. See :doc:`../project/verification`.
