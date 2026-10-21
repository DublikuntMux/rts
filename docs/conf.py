"""Sphinx configuration for the RTS handbook."""

import os

project = "RTS"
author = "RTS contributors"
release = os.environ.get("RTS_DOCS_VERSION", "development")
version = release
extensions = ["sphinxcontrib.kroki"]
root_doc = "index"
source_suffix = ".rst"
language = "en"
exclude_patterns = ["_build", ".venv", "Thumbs.db", ".DS_Store"]
nitpicky = True

html_theme = "furo"
html_title = "RTS documentation"
html_theme_options = {"navigation_with_keys": True}
html_static_path = ["_static"]
html_css_files = ["diagrams.css"]
html_show_copyright = False

linkcheck_timeout = 15
linkcheck_retries = 2
