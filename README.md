# Eilmer Meshing GUI

Sketch 2D geometry and build block-structured grids for the
[Eilmer](https://gdtk.uqcloud.net/) compressible-flow solver. The GUI writes a
`geometry.lua` that Eilmer runs directly, so the mesh you see is the one Eilmer builds.

Status: sketching, blocking, the live Eilmer mesh, boundary conditions and export
work. Mesh-quality checks and refinement are next.

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

## Blocking
| Key | Action |
|---|---|
| K | Block tool: click a closed region to make a block over it (its four sharpest corners). Double-click a block edge to set its cells and clustering. Right-click to sub-divide, set corners or delete. |
| Y | Boundary-layer tool: click block walls, Enter cuts a thin wall-clustered layer along them. Double-click a wall for the settings (first cell height or y+, growth ratio, cells). |
| Shift+K | Block all regions (also replaces blocks broken by sketch changes) |
| F5 | Run Eilmer now |
| Shift+B / Shift+N | Show blocks / show mesh |

- Dashed reference lines split regions, so they become the interfaces between blocks.
  Blocks follow the sketch: drag geometry and they re-mesh.
- Sub-divide: **Split N × M** (iso-parameter lines of the region), **O-grid** (an inner
  block plus four around it, e.g. a disc), **O-grid around hole** (four blocks around an
  island such as a cylinder), **Centroid split** (triangles and other N-sided regions).
- Cell counts are shared along linked edges: opposite edges of a block, and any edge
  shared with another block, so Eilmer can join them. Clustering uses Eilmer's
  RobertsFunction; it can also apply to the opposite edge, and is mirrored onto the
  neighbouring block where its edge runs the other way.
- The mesh updates live: a numpy preview while you edit (built the same way Eilmer
  builds it), then `lmr custom-script` runs in the background and its mesh replaces
  the preview. Runs go in `.emesh/<project>/` next to a saved project.
- Blocks needing attention are shown in red with the reason in the status bar
  (for example a region that no longer exists, or a face that only partly matches its
  neighbour's).

## Boundary layers
- With the boundary-layer tool (Y), click the block walls to resolve, then press Enter. Each
  wall's block is cut by a dashed curve parallel to the wall (a concentric arc for arcs, a line
  for lines, otherwise a spline) at the layer thickness
  `delta = h (r^N - 1) / (r - 1)`: first cell height h, growth ratio r, N cells across.
- The thin block gets N cells growing from h at the wall. The outer block grades on from the
  last layer cell, taking more cells if it needs them to grow no faster than the layer.
  Counts and clustering along the wall are kept.
- The cut continues through neighbouring blocks across dashed lines, so their faces still
  match. All the blocks it touches are re-blocked automatically.
- The walls are tagged as no-slip walls. Untagged and slip-wall curves get an existing no-slip
  tag, or a new `wall` tag (`WallBC_NoSlip_Adiabatic`; change it to `WallBC_NoSlip_FixedT`
  with Ctrl+B). Other tags are kept. Removing the layer puts the old tags back.
- Double-click a wall to change its layer's settings (the blocks are cut again) or remove the layer.
  Double-click a wall without a layer to set the defaults for the next one. The dialog estimates h
  from a target y+ (turbulent flat plate, Cf = 0.026 Re_x^-1/7).
- The same "first cell height" spacing is available for any edge in the Block tool's edge dialog.
  It goes to Eilmer as a `LuaFnClustering` (Eilmer's own GeometricFunction can fail to construct
  for some inputs).

## Boundary conditions and export
- Right-click solid curves (Select tool) or a block edge (Block tool), then
  **Boundary condition**: pick an existing tag, or **New boundary condition...** to choose an
  Eilmer class (inflow, outflow, walls; with suitable default arguments) and a tag name.
  Untagged curves are inviscid slip walls (Eilmer's default). Block → Boundary conditions
  (Ctrl+B) lists, edits, renames and deletes tags. Tagged curves are coloured, with a legend.
- Blocks are joined only across dashed reference lines. A solid curve between two blocks
  stays a wall on both sides (a splitter plate, for example).
- **File → Export to Eilmer** (Ctrl+E) writes into a folder you choose:
  - `geometry.lua`: the grids, their BC tags and connections
  - `bcs.lua`: `bcDict`, mapping each tag to its Eilmer BC
  - `sim.lua` and `ideal-air.lua`: a starter simulation script (with a placeholder flow state for
    each name the BCs use) and gas-model input, written only if they do not exist yet

  then runs `lmr prep-grids -j geometry.lua` there and reports the result. Next:
  ```
  lmr prep-gas -i ideal-air.lua -o ideal-air.gas
  lmr prep-sim -j sim.lua
  lmr run
  ```

## How it works
- A project (`*.emesh.json`) holds curves (line, arc, Bezier, spline), construction points, blocks
  (four faces, each an ordered list of curves) and BC tags. It is the source of truth.
- `eilmer_mesher.lua.codegen` turns it into one `geometry.lua` that runs under both:
  - `lmr prep-grids -j geometry.lua`: registers the grids and connects them across reference lines
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

from eilmer_mesher.model import blocking
from eilmer_mesher.model.topology import Topology

doc = Document.load("project.emesh.json")
res = blocking.resolve(doc, Topology(doc))   # region blocks -> concrete faces; res.errors explains failures
mesh_doc = doc.with_blocks(res.blocks)
result = preview_mesh(mesh_doc, ".emesh")    # result.grids: list of (niv, njv, 3) arrays
export_grids(mesh_doc, "path/to/sim")        # writes geometry.lua, bcs.lua; runs lmr prep-grids
```

## Tests
```
python3 -m pytest tests
```
Tests that need `lmr` are skipped when it is not installed. GUI tests run offscreen.

## Credits
`gridgen/path.py` and `gridgen/vector3.py` are Python ports of the GDTk geometry
classes by Peter Jacobs and Rowan Gollan. They are included so the project is self-contained.
