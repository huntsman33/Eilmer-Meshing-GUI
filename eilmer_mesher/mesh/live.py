"""
Live mesh: an instant numpy preview while editing, replaced by Eilmer's own
mesh once `lmr custom-script` (run in the background after each committed
edit) has written it.
"""
import glob
import os
import tempfile

from PySide6.QtCore import QObject, QProcess, QTimer, Signal

from ..lua.codegen import PREVIEW_DIR
from ..model.document import BlockError
from . import preview
from .runner import MESH_FILE, LmrResult, find_lmr, write_scripts
from .vtkio import read_structured_vtk


class Mesh:
    """Grids for one sketch revision: [(block id, (niv, njv, 3) array)]."""

    def __init__(self, revision, grids, source):
        self.revision = revision
        self.grids = grids
        self.source = source          # "eilmer" or "preview"

    def cells(self):
        return sum((g.shape[0] - 1) * (g.shape[1] - 1) for _, g in self.grids)


class LiveMesher(QObject):
    meshChanged = Signal(object)      # Mesh (or None when there are no blocks)
    status = Signal(str)              # one-line state for the status bar

    def __init__(self, model, parent=None):
        super().__init__(parent)
        self.model = model
        self.enabled = True
        self.workdir = None
        self._tmpdir = None
        self.mesh = None
        self.proc = None
        self._pending = False
        self._run = None              # (revision, block ids)
        self._preview_timer = QTimer(self, singleShot=True, interval=30)
        self._preview_timer.timeout.connect(self._preview)
        self._eilmer_timer = QTimer(self, singleShot=True, interval=150)
        self._eilmer_timer.timeout.connect(self._start)
        model.changed.connect(self._preview_timer.start)
        model.committed.connect(self.schedule)

    # --- preview -------------------------------------------------------------------------
    def _resolution(self):
        return self.model.doc, self.model.resolution()          # cached per revision

    def _preview(self):
        if not self.enabled:
            return
        rev = self.model.revision
        if self.mesh is not None and self.mesh.revision == rev and self.mesh.source == "eilmer":
            return
        doc, res = self._resolution()
        md = doc.with_blocks(res.blocks)
        grids = []
        for b in res.blocks:
            try:
                grids.append((b.id, preview.block_grid(md, b)))
            except Exception:          # a half-dragged shape can be degenerate; Eilmer will say
                pass
        self._set(Mesh(rev, grids, "preview") if grids else None)

    def _set(self, mesh):
        self.mesh = mesh
        self.meshChanged.emit(mesh)

    # --- Eilmer --------------------------------------------------------------------------
    def working_directory(self):
        if self.workdir:
            os.makedirs(self.workdir, exist_ok=True)
            return self.workdir
        if self._tmpdir is None:
            self._tmpdir = tempfile.mkdtemp(prefix="emesh-live-")
        return self._tmpdir

    def schedule(self):
        if self.enabled:
            self._eilmer_timer.start()

    def busy(self):
        return self.proc is not None or self._eilmer_timer.isActive()

    def _start(self):
        if self.proc is not None:
            self._pending = True
            return
        rev = self.model.revision
        doc, res = self._resolution()
        if not res.blocks:
            self._set(None)
            self.status.emit("No blocks" if not doc.blocks else "Blocks need attention")
            return
        workdir = self.working_directory()
        try:
            write_scripts(doc.with_blocks(res.blocks), workdir)
            lmr = find_lmr()
        except (BlockError, FileNotFoundError) as e:
            self.status.emit(f"Mesh: {e}")
            return
        for old in glob.glob(os.path.join(workdir, PREVIEW_DIR, "blk-*.vtk")):
            os.remove(old)
        self._run = (rev, [b.id for b in res.blocks])
        self.proc = QProcess(self)
        self.proc.setWorkingDirectory(workdir)
        self.proc.setProcessChannelMode(QProcess.MergedChannels)
        self.proc.finished.connect(self._finished)
        self.status.emit("Mesh: running Eilmer...")
        self.proc.start(lmr, ["custom-script", "-j", MESH_FILE])

    def _finished(self, code, _status):
        proc, self.proc = self.proc, None
        output = bytes(proc.readAll()).decode(errors="replace")
        proc.deleteLater()
        rev, ids = self._run
        if code == 0:
            files = sorted(glob.glob(os.path.join(self.working_directory(), PREVIEW_DIR, "blk-*.vtk")))
            mesh = Mesh(rev, list(zip(ids, [read_structured_vtk(f) for f in files])), "eilmer")
            if rev == self.model.revision:
                self._set(mesh)
                self.status.emit(f"Mesh: Eilmer, {len(ids)} block(s), {mesh.cells()} cells")
        else:
            self.status.emit("Mesh: Eilmer error: " + LmrResult("custom-script", code, output).error_summary)
        if self._pending or rev != self.model.revision:
            self._pending = False
            self.schedule()

    def run_now(self):
        self._eilmer_timer.stop()
        self._start()
