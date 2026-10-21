Contributing
============

This page collects the rules every change follows. Coding agents read
``AGENTS.md``, which points here.

Before you start
----------------

* Read :doc:`../architecture/index` for the crate layout and
  :doc:`../architecture/determinism` before touching ``rts_base``.
* Check :doc:`roadmap` for the current milestone and open questions.

Code rules
----------

* **Workspace layout.** The root ``Cargo.toml`` only defines the workspace.
  Code lives in ``crates/``; game content lives in ``assets/``.
* **Determinism.** ``rts_base`` must stay deterministic: no float arithmetic,
  no ``HashMap`` or ``HashSet``, no clocks or OS randomness. Every field that
  affects a later tick goes into the state hash.
* **Pure Rust.** Every dependency must build for native,
  ``wasm32-unknown-unknown``, and Android without C or C++ code.
* **Dependencies.** Put versions in ``[workspace.dependencies]`` and use
  ``dep.workspace = true`` in crates. Prefer existing dependencies; check
  their documentation before adding a crate.
* **Bevy features.** Enable a Bevy feature only when game code uses it. Avoid
  the ``2d`` and ``ui`` presets. See :doc:`../architecture/client`.
* **Simulation boundary.** The client reads ``Sim`` and submits commands. It
  never changes simulation state directly, and nothing in the simulation
  depends on Bevy entities.
* **Touch first.** Every action must be reachable by touch with targets of at
  least 48 logical pixels, and layouts must fit 320 pixels wide.
* **No placeholders.** Create crates and modules when their milestone needs
  them, not in advance.
* **Long-term design.** Update callers and remove obsolete paths in the same
  change instead of adding compatibility layers.

Checks
------

Run before every change:

.. code-block:: sh

   cargo test --workspace
   cargo clippy --workspace --all-targets
   cargo check --target wasm32-unknown-unknown

:doc:`../guides/testing` lists the additional checks for platform code and
releases.

Documentation
-------------

The handbook is the living specification. When you finish a milestone or
change a decision:

* Update the affected page in ``docs/architecture/`` or ``docs/reference/``.
* Update :doc:`roadmap` and add an entry to :doc:`verification`.
* Build the handbook with warnings as errors; see :doc:`../documentation`.

Keep constants in the documentation in step with the code. Pages name the
source file that owns each value.
