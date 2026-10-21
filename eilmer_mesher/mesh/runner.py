"""
Run lmr on generated Lua and collect results.

This is the synchronous core used by tests and export; the GUI wraps the same
steps in a debounced QProcess so it never blocks the canvas.
"""
import glob
import os
import shutil
import subprocess
from dataclasses import dataclass, field

from ..lua.codegen import PREVIEW_DIR, generate_bcs_lua, generate_geometry_lua
from .vtkio import read_structured_vtk

MESH_FILE = "mesh.lua"
BCS_FILE = "bcs.lua"


def find_lmr():
    exe = os.environ.get("LMR") or shutil.which("lmr")
    if exe is None:
        candidate = os.path.expanduser("~/gdtkinst/bin/lmr")
        if os.path.exists(candidate):
            exe = candidate
    if exe is None:
        raise FileNotFoundError("lmr not found: put ~/gdtkinst/bin on PATH or set LMR")
    return exe


@dataclass
class LmrResult:
    command: str
    returncode: int
    output: str
    grids: list = field(default_factory=list)   # (niv, njv, 3) arrays, one per block

    @property
    def ok(self):
        return self.returncode == 0

    @property
    def error_summary(self):
        """First meaningful error line, without the D stack trace (empty when ok)."""
        if self.ok:
            return ""
        for line in self.output.splitlines():
            line = line.strip()
            if not line or line.startswith(("----", "??:?")):
                continue
            # "object.Error@(0): msg" / "lmr...Exception@file(97): msg" -> "msg"
            if "@" in line.split(":", 1)[0] and ": " in line:
                return line.split(": ", 1)[1]
            return line
        return f"lmr {self.command} failed with exit code {self.returncode}"


def run_lmr(command, script, workdir, timeout=120):
    proc = subprocess.run([find_lmr(), command, "-j", script], cwd=workdir,
                          capture_output=True, text=True, timeout=timeout)
    return LmrResult(command, proc.returncode, proc.stdout + proc.stderr)


def write_scripts(doc, workdir):
    os.makedirs(workdir, exist_ok=True)
    with open(os.path.join(workdir, MESH_FILE), "w") as f:
        f.write(generate_geometry_lua(doc))
    with open(os.path.join(workdir, BCS_FILE), "w") as f:
        f.write(generate_bcs_lua(doc))


def preview_mesh(doc, workdir):
    """Generate mesh.lua, run lmr custom-script, and load the block grids it writes."""
    write_scripts(doc, workdir)
    preview = os.path.join(workdir, PREVIEW_DIR)
    for old in glob.glob(os.path.join(preview, "blk-*.vtk")):
        os.remove(old)
    result = run_lmr("custom-script", MESH_FILE, workdir)
    if result.ok:
        files = sorted(glob.glob(os.path.join(preview, "blk-*.vtk")))
        result.grids = [read_structured_vtk(p) for p in files]
    return result


SIM_FILE = "sim.lua"
GAS_INPUT_FILE = "ideal-air.lua"


def write_starter_files(doc, simdir):
    """sim.lua and a gas-model input, only where they do not exist yet. Returns the files written."""
    from ..lua.codegen import used_bc_tags
    from ..model.bcs import GAS_TEMPLATE, sim_template
    written = []
    for name, text in ((SIM_FILE, sim_template(doc, used_bc_tags(doc))), (GAS_INPUT_FILE, GAS_TEMPLATE)):
        path = os.path.join(simdir, name)
        if not os.path.exists(path):
            with open(path, "w") as f:
                f.write(text)
            written.append(name)
    return written


def export_grids(doc, simdir, starter_files=False):
    """Write mesh.lua and bcs.lua (and optionally starter sim files) into a folder; run lmr prep-grids."""
    write_scripts(doc, simdir)
    written = write_starter_files(doc, simdir) if starter_files else []
    result = run_lmr("prep-grids", MESH_FILE, simdir)
    result.written = [MESH_FILE, BCS_FILE] + written
    return result
