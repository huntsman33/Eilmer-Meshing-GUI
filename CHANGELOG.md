# Changelog

## 0.7.0-pre.1 (pre-release)
The first native build: the whole application is ported to Rust (Phase 7 of the plan).
- One self-contained executable per platform (Linux x86_64/aarch64, Windows x86_64, macOS
  universal); no Python, Qt or runtime to install. Eilmer is still driven through generated Lua
  and `lmr`, as before.
- Same projects: `.emesh.json` files open in either version, and saved files are identical.
- Same output: generated `mesh.lua`, `bcs.lua` and `sim.lua` are byte-for-byte identical to the
  Python version's, checked against golden files on every change.
- Faster: topology, blocking and the preview mesh rebuild in about a millisecond on a 14-block
  project; meshing runs on a background thread, and stale Eilmer runs are cancelled.
- One command, `emesh`: `emesh [project]` opens the app; `emesh export|lua|check|preview` runs the
  command-line tools (also as the console-only `emesh-cli`).
- Linux: X11 under WSL. The app sets its own window cursor (the pointer was invisible on WSLg
  without a system cursor theme) and falls back to built-in cursors where no theme is installed.
- A crisper light theme and a dark theme (View → Theme: Light, Dark, Follow system; toolbar button).
- A file dialog laid out like the Python version's: Look in, Back/Forward/Up, New Folder, Places
  and Recent folders, sortable Name/Size/Type/Date columns, file-type filter, rename (F2), and a
  confirmation before overwriting.
- Fixed: Ctrl+X (split-line mode) and Ctrl+Shift+X (reference mode) did nothing from the keyboard,
  because the windowing layer reports them as Cut.
- File > Open recent; Help > About (version, Eilmer location, settings file).
- A first-run notice when Eilmer (`lmr`) cannot be found; crash logs next to the settings.
- The mesh-quality banner floats over the canvas instead of pushing the drawing down.
- Fixed (also in the Python version): block faces could not be picked between their sample points
  on long faces at high zoom, so double-clicking such an edge did nothing.
