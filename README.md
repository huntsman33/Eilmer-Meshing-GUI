# Eilmer Meshing GUI

Sketch 2D geometry and build block-structured grids for the
[Eilmer](https://gdtk.uqcloud.net/) compressible-flow solver. The GUI writes a
`geometry.lua` that Eilmer runs directly, so the mesh you see is the one Eilmer builds.

Status: the sketcher (with trim and region detection) and the Document → Lua →
Eilmer pipeline work. Blocking, boundary-condition tagging and mesh refinement
are in progress.

## Requirements
- Python 3.10+ with `pip install -r requirements.txt`
- Eilmer 5 (`lmr`) from the [GDTk](https://github.com/gdtk-uq/gdtk) install. `lmr`
  is found on `PATH`, through `$LMR`, or at `~/gdtkinst/bin/lmr`.

## Run
From the repository root:
```
python3 mesher [project.emesh.json]
```
(`./mesher` and `python3 -m eilmer_mesher` also work.)

## Sketcher
Default keys (change them under Edit → Keyboard shortcuts; they are saved):

| Key | Tool | Notes |
|---|---|---|
| S | Select | Ctrl+click adds or removes. Double-click a point, line or arc to edit its dimensions. Drag handles to edit (shared endpoints follow) or drag the selection to move it. Box select: left-to-right = inside, right-to-left = crossing. Right-click for a menu. |
| O | Point | Construction point (+): a snap target, not geometry |
| L | Line | Type a number while drawing to fix the length; the cursor sets the direction |
| P | Profile | Click to chain lines (typed lengths work too). Drag from the last point, or press A, for a tangent arc. Right-click or Esc to finish. |
| A | Arc | Selected arcs and circles show their centre. Centre, then start (type a radius first to fix it), then end (type a sweep angle to fix it). Move round the centre for arcs past 180°. M switches to 3-point. |
| C | Circle | Type a radius or click. Stored as four quarter arcs. |
| R | Rectangle | |
| T | Triangle | |
| B | Bezier | Click control points; right-click or Enter to finish |
| N | Spline | Click points to pass through; right-click or Enter to finish |
| D | Dimension | Click a line (length) or arc (angle), move to place, click. Type a value while placing to set it. Double-click a dimension to change it, which drives the curve; drag its label to move it. Dimensions show the current value and follow the geometry (no constraint solver yet). |
| M | Move | Click objects to collect them, then drag from any point to its destination (snaps apply to both). Type `dx, dy` to move exactly. |
| Q | Quick trim | Click the piece of a curve to delete it, up to where it meets other curves |
| J | Merge | Click connected pieces, then Enter: joins collinear lines, arcs of one circle (under 180° in total) and pieces of one spline |
| Shift+M | Mirror | Click objects, Enter, click the symmetry axis (a solid or dashed line, or the x / y axis), Enter. Makes mirrored copies. |
| X | Reference mode | New curves are dashed (block splitters, not geometry). Shift+X converts the selection. |
| hold H / V | | Force the line horizontal / vertical while held |
| G | Grid snap | Other snaps (point, endpoint, intersection, centre, midpoint, on curve, horizontal/vertical) are in the snap menu |
| Shift+R | Show regions | Shades each closed region (what blocking will work from) |
| Space | Zoom to fit | Mouse wheel zooms about the cursor; middle-drag pans |

Typing while a tool is active goes to the input box: `x, y`, `@dx, dy` (relative),
`@L<deg` (polar), or a single length, radius or angle.

View → Units sets the display units (m, mm, in, ft). Everything typed and shown uses
them; the project, the generated Lua and Eilmer always use metres.

File → Import points (Ctrl+I) reads Selig, Lednicer or plain xy files (e.g. UIUC
airfoils) as splines. You can set scale, angle of attack, translation, a split at the
leading edge (so it can be a block corner) and a trailing-edge closing line.

## How it works
- A project (`*.emesh.json`) holds curves (line, arc, Bezier, spline), construction points, blocks
  (four faces, each an ordered list of curves) and BC tags. It is the source of truth.
- `eilmer_mesher.lua.codegen` turns it into one `geometry.lua` that runs under both:
  - `lmr prep-grids -j geometry.lua`: registers the grids and calls `identifyGridConnections()`
  - `lmr custom-script -j geometry.lua`: writes `preview/blk-NNNN.vtk` for the GUI
- Curve parameters match Eilmer's, so a trimmed spline is written as a `SubRangedPath`
  that ends exactly where the sketch shows it.
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
Tests that need `lmr` are skipped when it is not installed. GUI tests run offscreen.

## Credits
`gridgen/path.py` and `gridgen/vector3.py` are Python ports of the GDTk geometry
classes by Peter Jacobs and Rowan Gollan. They are included so the project is self-contained.
