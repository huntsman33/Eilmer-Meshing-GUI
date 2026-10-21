# Eilmer Meshing GUI

Sketch 2D geometry and build block-structured grids for the
[Eilmer](https://gdtk.uqcloud.net/) compressible-flow solver. The GUI writes a
`mesh.lua` that Eilmer runs directly, so the mesh you see is the one Eilmer builds.

Status: **0.7.0-pre.1**, the first pre-release of the native desktop app (Rust). Sketching,
blocking, boundary layers, the live Eilmer mesh, cell quality and refinement, boundary
conditions and export all work. The Python version remains the reference implementation and
the place new features are prototyped.

## Install
- **Eilmer 5** (`lmr`) from the [GDTk](https://github.com/gdtk-uq/gdtk) install. `lmr` is found on
  `PATH`, through `$LMR`, or at `~/gdtkinst/bin/lmr`. Without it the app still works, showing its
  built-in preview mesh (identical to Eilmer's for CoonsPatch blocks), but cannot export.
- **The desktop app**: download the archive for your system from the
  [releases](https://github.com/huntsman33/Eilmer-Meshing-GUI/releases) page and unpack it:
  - Linux (x86_64, aarch64): `eilmer-mesher-*-linux-*.tar.gz`, or the `.AppImage`
    (`chmod +x` it and run it). Needs glibc 2.35+ (Ubuntu 22.04 or newer) and a desktop with
    OpenGL or Vulkan. Works under WSLg.
  - Windows (x86_64): `eilmer-mesher-*-windows-x86_64.zip`.
  - macOS (Apple silicon and Intel): `eilmer-mesher-*-macos-universal.dmg`. The pre-release is not
    signed: right-click the app and choose Open the first time.

  Each download holds `emesh` (the app, and the command-line tools) and `emesh-cli` (the
  command-line tools alone, for scripts). Nothing else needs installing: no Python, Qt or runtime.
  Put the folder on your `PATH` to start it from any terminal.
- **From source** (any platform, needs [Rust](https://rustup.rs)):
  ```
  cd rust
  cargo install --path crates/emesh-app     # installs `emesh` into ~/.cargo/bin (on PATH after rustup)
  ```

## Run
```
emesh [project.emesh.json]
```
opens the app, from any folder.
- `--renderer glow` uses OpenGL instead of the default (wgpu, which falls back to OpenGL
  by itself where no GPU device is available).
- On Linux the app uses X11 under WSL (`EMESH_WAYLAND=1` keeps Wayland). It sets its own window
  cursor, so the pointer shows even where no cursor theme is installed (WSLg, minimal systems); there
  it uses built-in cursors written to `~/.cache/eilmer-mesher/cursors` (`EMESH_BUILTIN_CURSORS=1`
  forces them).
- View → Theme switches between Light, Dark and Follow system; the sun/moon button at the right of
  the toolbar toggles dark mode.
- Open, Save, Import and Export use the app's own file dialog (Look in, Back/Forward/Up, New Folder,
  Places and Recent folders, sortable columns, file-type filter); it works the same on every platform.
- Settings (units, shortcuts, layer defaults...) are kept in the system's settings folder
  (`~/.config/eilmer-mesher/settings.json` on Linux); Help > About shows where. Set
  `EMESH_SETTINGS` to use another file.
- Projects are the same `.emesh.json` files the Python version reads and writes; either opens the
  other's projects.

Command line, without the window:
```
emesh export project.emesh.json sim/      # mesh.lua, bcs.lua, sim.lua, ideal-air.lua; runs lmr prep-grids
emesh lua project.emesh.json [mesh|bcs|sim]
emesh check project.emesh.json            # blocks needing attention, cell quality
emesh preview project.emesh.json          # runs lmr custom-script, compares with the built-in preview
```

### The Python version
Python 3.10+ with `pip install -r requirements.txt`, then from the repository root:
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
| D | Dimension | Click a line (length) or arc (angle), move to place, click. Click a line and then a second line for the distance between parallel lines, or the angle between non-parallel ones (about their common end or crossing point; the sector you place it in picks the angle or its supplement); editing these moves the second line. Type a value while placing to set it. Double-click a dimension to change it, which drives the curve; drag its label to move it. Dimensions show the current value and follow the geometry (no constraint solver yet). |
| M | Move | Click objects to collect them, then drag from any point to its destination (snaps apply to both). Type `dx, dy` to move exactly. |
| Q | Quick trim | Click the piece of a curve to delete it, up to where it meets other curves |
| J | Merge | Click connected pieces, then Enter: joins collinear lines, arcs of one circle (under 180° in total) and pieces of one spline |
| Shift+M | Mirror | Click objects, Enter, click the symmetry axis (any line, or the x / y axis), Enter. Makes mirrored copies. |
| F | Offset | Click curves, Enter: set the distance and side (previewed, with an arrow) and OK. Makes offset copies; connected curves are offset together with exact joins (lines mitred, arcs concentric, other corners rounded). Closed shapes grow, or shrink with Reverse; open curves go to the left of their direction. |
| Ctrl+Shift+X | Reference mode | New curves are dashed reference lines: drawing aids (axes, construction) that the mesher ignores. Shift+X converts the selected curves between solid and reference. |
| Ctrl+X | Split-line mode | New curves are dash-dot block split lines: the interfaces between blocks. X converts the selected curves between solid and split lines. |
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
| H / Shift+H | Hide / show the selected block (or the one under the cursor) / all blocks (view only; hidden blocks are still meshed and exported). Right-click a block for Hide / Show block and Hide / Show all blocks. |
| Shift+K | Block all regions (also replaces blocks broken by sketch changes) |
| Ctrl+R | Refine: smooth (or AOPatch) the blocks the quality check flags |
| Shift+Q | Show cell quality (heat-map) |
| F5 | Run Eilmer now |
| Shift+B / Shift+N | Show blocks / show mesh |
| Shift+G | Show or hide the background grid |

- Dash-dot **split lines** divide regions, so they become the interfaces between blocks. Dashed
  **reference lines** are drawing aids only: the mesher ignores them. (Projects saved before this
  change had dashed block splitters; they are converted to split lines when opened.)
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

## Mesh quality and refinement
- Every new mesh (the preview, then Eilmer's) is checked cell by cell:
  - **equiangle skewness**: 0 for rectangles, 1 for degenerate cells;
  - **aspect ratio**: longest edge over shortest;
  - **inverted cells**: folded or tangled.
- Blocks with boundary-layer spacing are thin by design, so their aspect ratio isn't checked.
- Problems show in a banner above the canvas. The limits are under View > Quality thresholds
  (defaults: warn above skewness 0.6, refine above 0.85, warn above aspect ratio 20).
- **Refine** (banner button, Block > Refine flagged blocks, Ctrl+R) applies Laplace smoothing
  to the flagged blocks, as in the wk7 tutorial. The smoothing runs in the generated Lua, and the
  preview reproduces it exactly.
  - Where smoothing would fold cells, as it does beside a concave side, Refine gives that block
    Eilmer's AOPatch instead.
  - If smoothing leaves bad cells, the banner offers **Try AOPatch**; after that it suggests
    sub-dividing the block.
  - Boundary-layer blocks are never smoothed.
- **Show cells** (Shift+Q) colours each cell by skewness or aspect ratio (View > Quality metric),
  with inverted cells in purple.
- The Block tool's right-click **Patch** menu sets any block to CoonsPatch, CoonsPatch with
  Laplace smoothing, or AOPatch (nx, ny). Block labels show "smoothed" or "AO".

## Boundary layers
- With the boundary-layer tool (Y), click the block walls to resolve, then press Enter. Each
  wall's block is cut by a split line parallel to the wall (a concentric arc for arcs, a line
  for lines, otherwise a spline) at the layer thickness
  `delta = h (r^N - 1) / (r - 1)`: first cell height h, growth ratio r, N cells across.
- The thin block gets N cells growing from h at the wall. The outer block keeps the cells the
  block had; the status bar says how much bigger they are than the last layer cell, and how
  many more layer cells would meet them smoothly. Optionally ("grade the outer blocks") the outer
  block grows from the last layer cell, at no more than the layer's ratio, up to its original
  cell size and stays uniform beyond, adding cells as needed; with a large size jump that
  growth fills most of the block. Counts and clustering along the wall are kept.
- The cut continues through neighbouring blocks across split lines, so their faces still
  match. All the blocks it touches are re-blocked automatically.
- The walls are tagged as no-slip walls. Untagged and slip-wall curves get an existing no-slip
  tag, or a new `wall` tag (`WallBC_NoSlip_Adiabatic`; change it to `WallBC_NoSlip_FixedT`
  with Ctrl+B). Other tags are kept. Removing the layer puts the old tags back.
- Double-click a wall to change its layer's settings (the blocks are cut again) or remove the layer.
  Double-click a wall without a layer to set the defaults for the next one. The dialog estimates h
  from a target y+ (turbulent flat plate, Cf = 0.026 Re_x^-1/7).
- The same "first cell height" spacing is available for any edge in the Block tool's edge dialog.
  These spacings go to Eilmer through `LuaFnClustering` (Eilmer's own GeometricFunction can fail
  to construct for some inputs).

## Boundary conditions and export
- Right-click solid curves (Select tool) or a block edge (Block tool), then
  **Boundary condition**: pick an existing tag, or **New boundary condition...** to choose an
  Eilmer class (inflow, outflow, walls; with suitable default arguments) and a tag name.
  Untagged curves are inviscid slip walls (Eilmer's default). Block → Boundary conditions
  (Ctrl+B) lists, edits, renames and deletes tags. Tagged curves are coloured, with a legend.
- Blocks are joined only across split lines. A solid curve between two blocks
  stays a wall on both sides (a splitter plate, for example).
- **File → Export to Eilmer** (Ctrl+E) writes into a folder you choose:
  - `mesh.lua`: the grids, their BC tags and connections
  - `bcs.lua`: `bcDict`, mapping each tag to its Eilmer BC
  - `sim.lua` and `ideal-air.lua`: a starter simulation script (with a placeholder flow state for
    each name the BCs use) and gas-model input, written only if they do not exist yet

  then runs `lmr prep-grids -j mesh.lua` there and reports the result. Next:
  ```
  lmr prep-gas -i ideal-air.lua -o ideal-air.gas
  lmr prep-sim -j sim.lua
  lmr run
  ```

## How it works
- A project (`*.emesh.json`) holds curves (line, arc, Bezier, spline), construction points, blocks
  (four faces, each an ordered list of curves) and BC tags. It is the source of truth.
- `eilmer_mesher.lua.codegen` turns it into one `mesh.lua` that runs under both:
  - `lmr prep-grids -j mesh.lua`: registers the grids and connects them across split lines
  - `lmr custom-script -j mesh.lua`: writes `preview/blk-NNNN.vtk` for the GUI
- Curve parameters match Eilmer's, so a trimmed spline is written as a `SubRangedPath`
  that ends exactly where the sketch shows it.
- Split lines are internal block interfaces and reference lines are drawing aids; neither is BC-tagged.
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
export_grids(mesh_doc, "path/to/sim")        # writes mesh.lua, bcs.lua; runs lmr prep-grids
```

## Tests
```
python3 -m pytest tests          # the Python reference (GUI tests run offscreen)
cd rust && cargo test --workspace # the Rust port, checked against the Python reference
```
Tests that need `lmr` are skipped when it is not installed.

The Rust port must reproduce the Python version: `golden/make_goldens.py` writes the reference
outputs (generated Lua, topology, block resolution, preview grids, cell quality, and 44 editing
operations) for a corpus of projects, and the Rust tests compare against them. After changing
Python behaviour, regenerate them with `OPENBLAS_CORETYPE=Haswell python3 golden/make_goldens.py`.
See [rust/README.md](rust/README.md).

## Credits
`gridgen/path.py` and `gridgen/vector3.py` are Python ports of the GDTk geometry
classes by Peter Jacobs and Rowan Gollan. They are included so the project is self-contained.
