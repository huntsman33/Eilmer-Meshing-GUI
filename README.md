# Eilmer Meshing GUI

Sketch 2D geometry and build block-structured grids for the
[Eilmer](https://gdtk.uqcloud.net/) compressible-flow solver. The GUI writes a
`geometry.lua` that Eilmer runs directly, so the mesh you see is the one Eilmer builds.

Status: the Document → Lua → Eilmer pipeline is working and tested. The GUI is in progress.

## Requirements
- Python 3.10+ with `pip install -r requirements.txt`
- Eilmer 5 (`lmr`) from the [GDTk](https://github.com/gdtk-uq/gdtk) install. `lmr`
  is found on `PATH`, through `$LMR`, or at `~/gdtkinst/bin/lmr`.

## How it works
- A project (`*.emesh.json`) holds curves (line, arc, Bezier, spline), blocks
  (four faces, each an ordered list of curves) and BC tags. It is the source of truth.
- `eilmer_mesher.lua.codegen` turns it into one `geometry.lua` that runs under both:
  - `lmr prep-grids -j geometry.lua`: registers the grids and calls `identifyGridConnections()`
  - `lmr custom-script -j geometry.lua`: writes `preview/blk-NNNN.vtk` for the GUI
- Dashed (reference) curves are internal block interfaces and are never BC-tagged.
  Untagged solid faces use Eilmer's default, `WallBC_WithSlip`.
- `bcs.lua` is a `bcDict` stub to paste into the job script.
- Block options: `CoonsPatch` (default) or `AOPatch`, plus optional Laplace smoothing
  (Jacobi four-point averaging with fixed boundaries, run inside Eilmer).

## Use from Python
```python
from eilmer_mesher.model.document import Document
from eilmer_mesher.mesh.runner import preview_mesh, export_grids

doc = Document.load("project.emesh.json")
result = preview_mesh(doc, ".emesh")      # result.grids: list of (niv, njv, 3) arrays
export_grids(doc, "path/to/sim")          # writes geometry.lua, bcs.lua; runs lmr prep-grids
```

## Tests
```
python3 -m pytest tests
```
Tests that need `lmr` are skipped when it is not installed.

## Credits
`gridgen/path.py` and `gridgen/vector3.py` are Python ports of the GDTk geometry
classes by Peter Jacobs and Rowan Gollan. They are included so the project is self-contained.
