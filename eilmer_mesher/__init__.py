"""Eilmer Mesher: sketch 2D geometry and generate block-structured Eilmer grids."""
import os
import sys

# gridgen sits next to this package at the repository root.
REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..'))
if REPO_ROOT not in sys.path:
    sys.path.insert(0, REPO_ROOT)
