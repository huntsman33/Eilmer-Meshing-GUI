"""
Tools that act on a set of existing objects: merge and mirror.

Both use the selection as their target set (anything selected beforehand is
included); a click toggles an object in or out.
"""
from PySide6.QtCore import QPointF

from ...model.operations import lies_on, mirror_curve, reflect
from ..canvas import COLORS, PICK_PX, make_pen
from .base import Tool

X_AXIS = ((0.0, 0.0), (1.0, 0.0))
Y_AXIS = ((0.0, 0.0), (0.0, 1.0))


class _TargetTool(Tool):
    snaps = False
    hover_curves = True

    def toggle_target(self, ev):
        eid = self.canvas.pick_entity(ev.screen, ev.world)
        if eid is not None:
            self.model.set_selection(self.model.selection ^ {eid})


class MergeTool(_TargetTool):
    """Click the pieces to join, then Enter (or right-click)."""
    name, label = "merge", "Merge"

    def press(self, ev):
        self.toggle_target(ev)

    def finish(self):
        ids = [i for i in self.model.selection if i in self.model.doc.curves]
        if len(ids) < 2:
            self.canvas.message.emit("Merge: select at least two connected curves.")
            return
        _, joins = self.model.merge(ids)
        if joins:
            self.canvas.message.emit(f"Merged {len(ids)} curves into {len(ids) - joins}.")
        else:
            self.canvas.message.emit("Nothing to merge: pieces must share an end and be collinear lines, "
                                     "arcs of one circle (under 180 degrees in total) or parts of one spline.")

    def busy(self):
        return False

    def prompt(self):
        n = len(self.model.selection & set(self.model.doc.curves))
        return f"Pieces to merge ({n} selected): Enter or right-click to merge"

    def hint(self):
        return ("Merge: click connected pieces (collinear lines, arcs of one circle, parts of one spline), "
                "then press Enter.")


class MirrorTool(_TargetTool):
    """
    Step 1: click objects to mirror, then Enter (or right-click).
    Step 2: click the symmetry axis: any line (solid or dashed) or the x / y axis.
    Enter creates mirrored copies.
    """
    name, label = "mirror", "Mirror"

    def reset(self):
        self.step = 1
        self.axis = None          # (a, b, label)

    def busy(self):
        return self.step == 2

    def pick_axis(self, ev):
        doc = self.model.doc
        lines = [cid for cid, c in doc.curves.items()
                 if c.type == "line" and cid not in self.model.selection]
        cid = self.canvas.pick_curve(ev.world, among=lines)
        if cid is not None:
            c = doc.curves[cid]
            return c.p0, c.p1, cid
        origin = self.canvas.to_screen((0.0, 0.0))
        if abs(ev.screen.y() - origin.y()) <= PICK_PX:
            return X_AXIS + ("x axis",)
        if abs(ev.screen.x() - origin.x()) <= PICK_PX:
            return Y_AXIS + ("y axis",)
        return None

    def press(self, ev):
        if self.step == 1:
            self.toggle_target(ev)
        else:
            axis = self.pick_axis(ev)
            if axis is not None:
                self.axis = axis

    def finish(self):
        if self.step == 1:
            if not self.model.selection:
                self.canvas.message.emit("Mirror: select the objects to mirror first.")
                return
            self.step = 2
            return
        if self.axis is None:
            self.canvas.message.emit("Mirror: click a line, or the x or y axis, as the symmetry axis.")
            return
        a, b, _ = self.axis
        new_ids = self.model.mirror(sorted(self.model.selection), a, b)
        self.canvas.message.emit(f"Mirrored {len(new_ids)} object(s).")
        self.model.set_selection(new_ids)
        self.reset()

    def right_click(self, ev):
        self.finish()

    def cancel(self):
        if self.step == 2:
            self.reset()                 # back to choosing objects
            return True
        if self.model.selection:
            self.model.set_selection(set())
            return True
        return False

    def prompt(self):
        if self.step == 1:
            return f"Objects to mirror ({len(self.model.selection)} selected): then Enter"
        if self.axis is None:
            return "Symmetry axis: click a line or the x / y axis"
        name = self.axis[2] if self.axis[2] in ("x axis", "y axis") else "line"
        return f"Axis: {name}. Enter to mirror"

    def hint(self):
        return ("Mirror: click objects, press Enter, click the symmetry axis (a solid or dashed line, "
                "or the x / y axis), then press Enter.")

    def paint(self, painter):
        if self.step != 2 or self.axis is None:
            return
        a, b, label = self.axis
        if label in ("x axis", "y axis"):
            w, h = self.canvas.width(), self.canvas.height()
            lo = self.canvas.to_world(QPointF(0, h))
            hi = self.canvas.to_world(QPointF(w, 0))
            ends = [(lo[0], 0.0), (hi[0], 0.0)] if label == "x axis" else [(0.0, lo[1]), (0.0, hi[1])]
            self.canvas.draw_curve(painter, self.line(*ends), COLORS["snap"], 2.0, dashed=True)
        else:
            self.canvas.draw_curve(painter, self.model.doc.curves[label], COLORS["snap"], 3.0)
        doc = self.model.doc
        tol = 1e-9 * max(1.0, abs(a[0]), abs(a[1]), abs(b[0]), abs(b[1]))
        previews = [mirror_curve(doc.curves[i], a, b) for i in self.model.selection
                    if i in doc.curves and not lies_on(doc.curves[i], a, b, tol)]
        self.draw_preview(painter, previews)
        painter.save()
        painter.resetTransform()
        painter.setPen(make_pen(COLORS["preview"], 1.5))
        for i in self.model.selection & set(doc.points):
            q = self.canvas.to_screen(reflect(doc.points[i], a, b))
            painter.drawLine(q + QPointF(-5, 0), q + QPointF(5, 0))
            painter.drawLine(q + QPointF(0, -5), q + QPointF(0, 5))
        painter.restore()
