Build and maintain the documentation
====================================

The handbook is written in reStructuredText under ``docs/`` and built with
Sphinx, the Furo theme, and ``sphinxcontrib-kroki`` for diagrams. Building
documentation does not compile the Rust workspace.

Install the documentation tools
-------------------------------

Use Python 3.11 or newer. From the repository root:

.. code-block:: sh

   python3 -m venv docs/.venv
   docs/.venv/bin/python -m pip install -r docs/requirements.txt

If you use uv, the equivalent setup is:

.. code-block:: sh

   uv venv docs/.venv --python 3.12
   uv pip install --python docs/.venv/bin/python -r docs/requirements.txt

Build and browse
----------------

.. code-block:: sh

   docs/.venv/bin/python -m sphinx -b html -n -W --keep-going docs docs/_build/html
   docs/.venv/bin/python -m http.server 8000 --directory docs/_build/html

Open ``http://localhost:8000`` in your browser. HTML output includes navigation,
search, syntax highlighting, and a light/dark theme switch. You can also open
``docs/_build/html/index.html`` directly. Generated output and the virtual
environment are ignored by Git.

The build command checks references and treats warnings as errors. Use it
before submitting a documentation change. Diagrams are rendered by the public
`Kroki <https://kroki.io>`_ service, so the build needs network access. To
check external links too:

.. code-block:: sh

   docs/.venv/bin/python -m sphinx -b linkcheck -W --keep-going docs docs/_build/linkcheck

See the official
`Sphinx build reference <https://www.sphinx-doc.org/en/master/man/sphinx-build.html>`_
for other output formats and options, and the
`Furo quickstart <https://pradyunsg.me/furo/quickstart/>`_ for theme setup.

Add or update a page
--------------------

* Put walkthroughs and task-focused help in ``docs/guides/``.
* Put formats, commands, controls, and limits in ``docs/reference/``.
* Put contributor-facing explanations of a subsystem in ``docs/architecture/``.
  One page covers one subsystem.
* Put milestones, planned subsystems, verification records, and contribution
  rules in ``docs/project/``.
* Add new pages to the appropriate toctree in ``docs/index.rst``.
* Use ``:doc:`` links between pages and ``:ref:`` links to named sections.
* Verify behavior against the source. Keep constants, defaults, and examples
  in step with the implementation, and name the source file that owns them.

Draw diagrams
-------------

Use the ``kroki`` directive with PlantUML for components, sequences, states,
and activities, and with ``rackdiag`` for byte layouts:

.. code-block:: rst

   .. kroki::
      :type: plantuml
      :caption: What the diagram shows.
      :align: center

      @startuml
      skinparam backgroundColor transparent
      Client -> Server : Create
      @enduml

Keep ``skinparam backgroundColor transparent`` so ``_static/diagrams.css`` can
invert PlantUML and rackdiag images for the dark theme. Prefer a short caption
that states what the reader should learn from the diagram.

Keep the handbook current
-------------------------

The handbook is the project's living specification. When you finish a
milestone or change a design decision, update the affected architecture
page, :doc:`project/roadmap`, and :doc:`project/verification` in the same
change. ``AGENTS.md`` points contributors and coding agents here.
