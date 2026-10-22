Release and continuous integration
===================================

Two workflows in ``.github/workflows`` automate checks and distributions.
Both install the nightly toolchain requested by ``rust-toolchain.toml``.

Continuous integration
----------------------

``ci.yml`` runs on every branch push, every pull request, and on demand. It
has four independent jobs:

.. list-table::
   :header-rows: 1
   :widths: 20 80

   * - Job
     - What it runs
   * - Format
     - ``cargo fmt --all --check``.
   * - Test and lint
     - ``cargo test --locked --workspace`` and
       ``cargo clippy --locked --workspace --all-targets -- -D warnings`` on
       Linux. The job installs the X11, Wayland, ALSA, and udev development
       packages needed to link Bevy.
   * - WebAssembly check
     - ``cargo check --locked --workspace --target wasm32-unknown-unknown``.
   * - Handbook
     - The strict Sphinx build from :doc:`../documentation`, with warnings
       treated as errors.

Runs on the same branch cancel older runs. Cargo dependencies and build
artifacts are cached with a single shared key per job; only runs on ``master``
write the cache, so pull requests reuse the default branch's cache instead of
creating a copy for every branch. The Sphinx build needs network access for
Kroki diagrams, and the Python packages are cached by
``actions/setup-python``.

Release
-------

``release.yml`` is started manually from **Actions → Release → Run workflow**.
It takes three inputs:

.. list-table::
   :header-rows: 1
   :widths: 20 80

   * - Input
     - Meaning
   * - ``version``
     - Required. ``0.2.0`` or ``0.2.0-rc.1``; a leading ``v`` is accepted.
   * - ``prerelease``
     - Marks the GitHub release as a pre-release.
   * - ``deploy_web``
     - Deploy the browser build to GitHub Pages. Enabled by default.

The workflow builds and publishes:

.. list-table::
   :header-rows: 1
   :widths: 28 72

   * - Asset
     - Contents
   * - ``rts-linux-x86_64-<tag>.tar.gz``
     - ``rts``, ``rts-server``, and ``assets.pack``. Built on Ubuntu 22.04
       for the widest glibc compatibility.
   * - ``rts-windows-x86_64-<tag>.zip``
     - ``rts.exe``, ``rts-server.exe``, and ``assets.pack``.
   * - ``rts-macos-universal-<tag>.zip``
     - Ad-hoc-signed ``RTS.app`` with a universal binary, ``rts-server``, and
       ``assets.pack``.
   * - ``rts-android-<tag>.apk``
     - Release APK, signed with the debug key.
   * - ``rts-web-<tag>.zip``
     - Trunk output for any static host.
   * - ``CHANGELOG.md`` and ``SHA256SUMS``
     - Generated release notes and checksums for every other asset.

The game and server open ``assets.pack`` from the directory containing the
executable, so keep the files together. macOS builds are ad-hoc signed
because the project has no Apple developer certificate: the first launch needs
right-click **Open**. Android release builds still use the debug signing key,
so they install for testing but cannot be uploaded to Google Play. Replace the
signing configuration in ``android/app/build.gradle.kts`` before a store
release.

Versioning and changelogs
-------------------------

The version input becomes the tag ``v<version>``. The ``prepare`` job finds the
newest earlier ``v*`` tag reachable from the current commit, then generates the
changelog from ``previous..HEAD``. Merge commits are skipped, and subjects are
grouped into *Added*, *Fixed*, *Changed*, *Removed*, and *Other* by their
leading verb (including conventional prefixes such as ``feat:`` and ``fix:``).
The first release lists the complete history. The notes are the GitHub release
body, attached as ``CHANGELOG.md``, and include a comparison link.

If the tag already points at the current commit, re-running the workflow
refreshes the release's assets and notes. A tag that points at a different
commit is rejected before any build starts.

GitHub Pages
------------

Enable Pages once under **Settings → Pages → Build and deployment**, with
**Source: GitHub Actions**. The ``deploy-web`` job publishes the ``dist``
output through ``actions/deploy-pages``. For a project repository the site is
``https://<owner>.github.io/<repo>/``. The web job derives that base path from
the repository name and passes it to ``trunk build --public-url``, so the
hashed JavaScript, WebAssembly, and ``assets.pack`` URLs resolve under the
subdirectory.

A custom domain or a ``<owner>.github.io`` repository needs its own base path.
Adjust the *Choose the GitHub Pages base path* step in
``.github/workflows/release.yml`` when the default project-site layout no
longer applies. Pages serves WebAssembly with the required content type and
supports byte-range requests, which the asset pack loader needs.

Caching
-------

Every Rust job restores Cargo's registry and target directory with
``Swatinem/rust-cache``. Cache keys are shared per workload (native CI, wasm
CI, and one key per release platform) rather than per run, and nightly Rust
versions are part of the key, so a toolchain update starts a fresh cache.
Release jobs also cache Trunk's downloaded ``wasm-bindgen`` and ``wasm-opt``
tools, ``gradle/actions/setup-gradle`` caches the Gradle user home, and the
Android job caches the SDK platform, build tools, and NDK.

GitHub limits each repository to 10 GB of caches and evicts least recently
used entries. The caches are an acceleration layer: a missing or evicted cache
only makes a run rebuild dependencies.

Local equivalents
-----------------

The commands each job runs are the ones in :doc:`testing`. Run the release
packaging locally with:

.. code-block:: sh

   cargo build --locked --release --workspace
   cargo ndk -t arm64-v8a -t x86_64 -P 31 build -p rts_android --release
   cd android && ./gradlew assembleRelease
