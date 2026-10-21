# Eilmer Mesher in Rust

The native desktop app and command-line tool, ported from the Python reference implementation
(`../eilmer_mesher`). Features are prototyped in Python first, frozen as golden files, then
ported here; the Rust crates must reproduce every golden file.

## Crates
| Crate | What | Ported from |
|---|---|---|
| `emesh-core` | document model (same `.emesh.json`), curve geometry with Eilmer's parameterisation, topology, blocking, sub-divide, boundary layers, offset, merge/mirror, dimensions, point-file import, snapping, units | `model/`, `sketch/importers.py`, `sketch/snapping.py`, `units.py` |
| `emesh-lua` | `mesh.lua`, `bcs.lua`, starter `sim.lua`; BC classes and tag rules. Feature `check`: compile-check generated Lua with an embedded Lua 5.4 (mlua) | `lua/codegen.py`, `model/bcs.py` |
| `emesh-mesh` | the preview grid (Eilmer's construction), VTK reader, cell quality, the `lmr` runner, and the background live mesher | `mesh/` |
| `emesh-app` | the `emesh` program: the GUI (egui/eframe) with canvas, tools, dialogs, settings, shortcuts and the editing model with undo; `emesh export ...` runs the command line | `app.py`, `sketch/` |
| `emesh-cli` | the command-line tools (export, Lua, checks, preview) as a library, and `emesh-cli`, a console-only binary | new |

## Build and run
```
cargo run --release -p emesh-app -- [project.emesh.json]     # the app
cargo run --release -p emesh-app -- export project.emesh.json sim/
cargo install --path crates/emesh-app                          # `emesh` on PATH (~/.cargo/bin)
cargo test --workspace
cargo test -p emesh-lua --features check     # generated Lua compiles as Lua 5.4
```
On Linux the binary links only glibc; X11, Wayland, OpenGL and Vulkan are loaded at run time.

## Parity with Python
Generated Lua is byte-for-byte identical to the Python version's, and project files saved by
either are identical. Getting there means doing arithmetic exactly as the Python stack does
(`emesh-core/src/num.rs`):
- `math.hypot`/`math.dist` are CPython's compensated vector norm, not libm's `hypot`;
- `np.sum` is numpy's pairwise summation; `np.dot` on 1-D arrays is OpenBLAS's ddot (the Haswell
  kernel, or its strided loop for column views); a 2x2 matrix-vector product is a fused multiply-add;
- scipy's `minimize_scalar(method="bounded")`, `brentq` and `solve_banded` are ported line by line;
- shapely's `representative_point`, `centroid`, `contains` and `is_simple` follow GEOS's algorithms
  (`poly.rs`);
- `repr(float)` and the `%g` formats are reproduced (`pyfmt.rs`).

`golden/` holds the reference outputs. The tests compare to 1e-12 by default; set
`EMESH_GOLDEN_STRICT=1` for bit-for-bit (true on x86_64 Linux with glibc 2.35, where the goldens
are made). Other platforms' libm can differ in the last bit, so CI checks parity on Linux and
skips it elsewhere (`EMESH_GOLDEN_SKIP=1`).

## Testing the GUI
`emesh_app::headless::Headless` runs the whole window without a display: tests send pointer and
key events and check the model (`crates/emesh-app/tests/gui.rs`). It can also render screenshots
in software, from a script of commands (`src/script.rs`):
```
cargo run --release -p emesh-app --example shots -- script.txt
```
The same script runs in a real window with `emesh --script script.txt`.

## Releases
Push a tag `v*` and `.github/workflows/release.yml` builds Linux x86_64/aarch64 (tarball and
AppImage), Windows x86_64 (zip) and macOS universal (`.app` in a `.dmg`) and attaches them to a
GitHub release (a pre-release when the version has a `-` suffix). The binaries are not signed yet:
that needs an Apple Developer ID and a Windows code-signing certificate as repository secrets.
