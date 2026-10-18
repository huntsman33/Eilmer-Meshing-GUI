"""
Qt-side owner of the Document: undo/redo, selection and change notification.

Every edit is recorded as a before/after snapshot of the whole document. Sketch
documents are small, so this keeps undo simple and exact.
"""
from contextlib import contextmanager

from PySide6.QtCore import QObject, Signal
from PySide6.QtGui import QUndoCommand, QUndoStack

from ..model.document import Document
from ..model.entities import GEOMETRY, REFERENCE


class SnapshotCommand(QUndoCommand):
    def __init__(self, model, before, after, text):
        super().__init__(text)
        self.model = model
        self.before = before
        self.after = after
        self._applied = True      # the edit has already been made when the command is pushed

    def redo(self):
        if self._applied:
            self._applied = False
            return
        self.model._restore(self.after)

    def undo(self):
        self.model._restore(self.before)


class SketchModel(QObject):
    changed = Signal()             # geometry changed
    selectionChanged = Signal()

    def __init__(self, doc=None, parent=None):
        super().__init__(parent)
        self.doc = doc or Document()
        self.undo_stack = QUndoStack(self)
        self.selection = set()
        self.reference_mode = False
        self.revision = 0
        self._before = None

    # --- editing -----------------------------------------------------------
    def new_kind(self):
        return REFERENCE if self.reference_mode else GEOMETRY

    def begin_edit(self):
        if self._before is None:
            self._before = self.doc.to_dict()

    def commit_edit(self, text):
        before, self._before = self._before, None
        if before is None:
            return
        after = self.doc.to_dict()
        if after != before:
            self.undo_stack.push(SnapshotCommand(self, before, after, text))
        self.notify()

    def cancel_edit(self):
        before, self._before = self._before, None
        if before is not None:
            self._restore(before)

    @contextmanager
    def edit(self, text):
        self.begin_edit()
        try:
            yield self.doc
        except Exception:
            self.cancel_edit()
            raise
        self.commit_edit(text)

    def notify(self):
        self.revision += 1
        self.changed.emit()

    def _restore(self, data):
        self.doc = Document.from_dict(data)
        self._prune_selection()
        self.notify()

    def _prune_selection(self):
        alive = self.selection & set(self.doc.curves)
        if alive != self.selection:
            self.selection = alive
            self.selectionChanged.emit()

    # --- common operations ------------------------------------------------
    def add_curves(self, curves, text):
        """Give curves fresh ids and add them in one undo step; returns their ids."""
        ids = []
        with self.edit(text) as doc:
            for c in curves:
                c.id = doc.new_id("c")
                doc.add_curve(c)
                ids.append(c.id)
        return ids

    def delete_selection(self):
        if not self.selection:
            return
        with self.edit("Delete") as doc:
            doc.remove_curves(self.selection)
        self._prune_selection()

    def toggle_reference(self, ids=None):
        ids = set(ids if ids is not None else self.selection) & set(self.doc.curves)
        if not ids:
            return
        with self.edit("Toggle reference") as doc:
            for cid in ids:
                c = doc.curves[cid]
                c.kind = GEOMETRY if c.kind == REFERENCE else REFERENCE
                if c.kind == REFERENCE:
                    c.bc_tag = ""

    def set_selection(self, ids):
        ids = set(ids) & set(self.doc.curves)
        if ids != self.selection:
            self.selection = ids
            self.selectionChanged.emit()

    # --- files ---------------------------------------------------------------
    def reset(self, doc):
        self.doc = doc
        self.undo_stack.clear()
        self.selection = set()
        self._before = None
        self.selectionChanged.emit()
        self.notify()
