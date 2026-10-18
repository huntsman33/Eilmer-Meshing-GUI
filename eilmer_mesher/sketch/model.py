"""
Qt-side owner of the Document: undo/redo, selection and change notification.

Every edit is recorded as a before/after snapshot of the whole document. Sketch
documents are small, so this keeps undo simple and exact.
"""
import math
from contextlib import contextmanager

from PySide6.QtCore import QObject, Signal
from PySide6.QtGui import QUndoCommand, QUndoStack

from ..model.document import Dimension, Document
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

    def entity_ids(self):
        return set(self.doc.curves) | set(self.doc.points) | set(self.doc.dimensions)

    def _prune_selection(self):
        alive = self.selection & self.entity_ids()
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

    def add_point(self, p, text="Point"):
        with self.edit(text) as doc:
            pid = doc.new_id("pt")
            doc.add_point(pid, p)
        return pid

    def connected_ends(self, point, exclude=(), tol=None):
        """(curve id, "start"/"end") of curve ends at point, excluding curves in exclude."""
        if tol is None:
            tol = 1e-9 * max(1.0, abs(point[0]), abs(point[1]))
        out = []
        for c in self.doc.curves.values():
            if c.id in exclude:
                continue
            for which, q in (("start", c.start()), ("end", c.end())):
                if which in dict(c.handles()) and math.dist(q, point) <= tol:
                    out.append((c.id, which))
        return out

    # --- dimension edits (double-click dialogs) -------------------------------------
    @staticmethod
    def _move_ends(doc, partners, new_point):
        """Move the listed (curve id, "start"/"end") curve ends to new_point."""
        for cid, which in partners:
            moved = doc.curves[cid].with_handle(which, new_point)
            if moved is not None:
                doc.curves[cid] = moved

    def set_point(self, pid, p):
        with self.edit("Edit point") as doc:
            doc.points[pid] = (float(p[0]), float(p[1]))

    def set_line(self, cid, length, angle_deg):
        """Keep the start, put the end at length along angle_deg; connected curves follow."""
        if length <= 0:
            raise ValueError("length must be positive")
        c = self.doc.curves[cid]
        a = math.radians(angle_deg)
        end = (c.p0[0] + length*math.cos(a), c.p0[1] + length*math.sin(a))
        old_end = c.p1
        partners = self.connected_ends(old_end, exclude={cid})
        with self.edit("Edit line") as doc:
            doc.curves[cid] = c.copy(p1=end)
            self._move_ends(doc, partners, end)

    def set_arc(self, cid, radius, sweep_deg):
        """Keep the centre and start direction; set radius and sweep (sign keeps the direction)."""
        from ..model import geom
        if radius <= 0 or not 0 < abs(sweep_deg) < 360:
            raise ValueError("radius must be positive and the sweep between 0 and 360 degrees")
        c = self.doc.curves[cid]
        centre, _, a0, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
        new_sweep = math.copysign(math.radians(abs(sweep_deg)), sweep)
        pieces = geom.arc_pieces(centre, radius, a0, new_sweep)
        old_start, old_end = c.p0, c.p1
        new_start, new_end = pieces[0][0], pieces[-1][1]
        at_start = self.connected_ends(old_start, {cid})
        at_end = self.connected_ends(old_end, {cid})
        with self.edit("Edit arc") as doc:
            if len(pieces) > 1:            # the face would now need several curves
                doc.remove_blocks_using([cid])
            first = pieces[0]
            doc.curves[cid] = c.copy(p0=first[0], p1=first[1], centre=first[2])
            for p0, p1, cen in pieces[1:]:
                doc.add_curve(c.copy(id=doc.new_id("c"), p0=p0, p1=p1, centre=cen))
            self._move_ends(doc, at_start, new_start)
            self._move_ends(doc, at_end, new_end)

    def translate(self, ids, dx, dy, text="Move"):
        """Move curves and points by (dx, dy); dimensions follow their curves."""
        with self.edit(text) as doc:
            for i in ids:
                if i in doc.curves:
                    doc.curves[i] = doc.curves[i].map_points(lambda p: (p[0] + dx, p[1] + dy))
                elif i in doc.points:
                    x, y = doc.points[i]
                    doc.points[i] = (x + dx, y + dy)

    # --- dimensions ------------------------------------------------------------------------
    @staticmethod
    def dimension_kind(curve):
        return {"line": "length", "arc": "angle"}.get(curve.type)

    def add_dimension(self, cid, offset):
        """Dimension a line (length) or arc (angle); re-placing an existing one moves it."""
        kind = self.dimension_kind(self.doc.curves[cid])
        if kind is None:
            raise ValueError("only lines and arcs can be dimensioned")
        existing = next((d for d in self.doc.dimensions.values()
                         if d.curve == cid and d.kind == kind), None)
        with self.edit("Dimension") as doc:
            if existing is not None:
                existing.offset = float(offset)
                return existing.id
            did = doc.new_id("dim")
            doc.add_dimension(Dimension(did, kind, cid, float(offset)))
        return did

    def dimension_value(self, did):
        """Length (metres) or sweep angle (degrees) the dimension currently shows."""
        from ..model import geom
        d = self.doc.dimensions[did]
        c = self.doc.curves[d.curve]
        if d.kind == "length":
            return math.dist(c.p0, c.p1)
        _, _, _, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
        return math.degrees(abs(sweep))

    def set_dimension_value(self, did, value):
        """Drive the curve to a new length (metres) or sweep angle (degrees)."""
        from ..model import geom
        d = self.doc.dimensions[did]
        c = self.doc.curves[d.curve]
        if d.kind == "length":
            angle = math.degrees(math.atan2(c.p1[1] - c.p0[1], c.p1[0] - c.p0[0]))
            self.set_line(d.curve, value, angle)
        else:
            _, r, _, _ = geom.arc_geometry(c.p0, c.p1, c.centre)
            self.set_arc(d.curve, r, value)

    # --- merge and mirror --------------------------------------------------------------
    def merge(self, ids):
        """Join connected collinear lines, concentric arcs and pieces of one spline.
        Returns (surviving ids, number of joins)."""
        from ..model.operations import merge_curves
        ids = set(ids)
        curves = [c for cid, c in self.doc.curves.items() if cid in ids]   # document order
        result, removed = merge_curves(curves)
        if not removed:
            return [c.id for c in curves], 0
        with self.edit("Merge") as doc:
            doc.remove_blocks_using([c.id for c in curves])
            doc.remove_curves(removed)
            for c in result:
                doc.curves[c.id] = c
        survivors = [c.id for c in result]
        self.set_selection(survivors)
        return survivors, len(removed)

    def mirror(self, ids, a, b):
        """Mirrored copies of curves and points across the line through a and b; returns new ids."""
        from ..model.operations import lies_on, mirror_curve, reflect
        tol = 1e-9 * max(1.0, abs(a[0]), abs(a[1]), abs(b[0]), abs(b[1]))
        new_ids = []
        with self.edit("Mirror") as doc:
            for i in ids:
                if i in doc.curves:
                    c = doc.curves[i]
                    if lies_on(c, a, b, tol):
                        continue                  # its mirror image is itself
                    m = mirror_curve(c, a, b).copy(id=doc.new_id("c"))
                    doc.add_curve(m)
                    new_ids.append(m.id)
                elif i in doc.points:
                    q = reflect(doc.points[i], a, b)
                    if math.dist(q, doc.points[i]) > tol:
                        new_ids.append(doc.add_point(doc.new_id("pt"), q))
        return new_ids

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
        ids = set(ids) & self.entity_ids()
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
