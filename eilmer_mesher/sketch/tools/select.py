"""
Select and move.

  click curve           select it (Shift toggles)
  drag a handle         move that point; curves sharing the endpoint follow
  drag a curve          move the selection
  drag empty space      box select: left-to-right = window (fully inside),
                        right-to-left = crossing (touching)
  right-click           context menu
"""
import math

from PySide6.QtCore import QRectF, Qt
from PySide6.QtGui import QColor
from PySide6.QtWidgets import QMenu

from ...model import geom
from ..canvas import make_pen
from .base import Tool

DRAG_START_PX = 3
CONNECT_TOL_PX = 0.5


class SelectTool(Tool):
    name, label = "select", "Select"
    hover_curves = True

    def reset(self):
        self.drag = None

    @property
    def snaps(self):
        return self.drag is not None and self.drag["kind"] in ("handle", "body") and self.drag["moved"]

    def busy(self):
        return self.drag is not None

    def anchor(self):
        if self.drag is None:
            return None
        return self.drag.get("origin")

    def snap_exclude(self):
        return self.drag["moving"] if self.drag else ()

    # --- press -------------------------------------------------------------------
    def press(self, ev):
        shift = bool(ev.modifiers & Qt.ShiftModifier)
        doc = self.model.doc
        handle = self.canvas.pick_handle(ev.screen)
        if handle is not None and not shift:
            cid, name, point = handle
            partners = []
            if name in ("start", "end"):
                tol = self.canvas.px(CONNECT_TOL_PX)
                for other in doc.curves.values():
                    if other.id == cid:
                        continue
                    for which, p in (("start", other.start()), ("end", other.end())):
                        if math.hypot(p[0] - point[0], p[1] - point[1]) <= tol:
                            partners.append((other.id, which))
            moving = {cid} | {pid for pid, _ in partners}
            self.drag = {"kind": "handle", "cid": cid, "name": name, "origin": point,
                         "partners": partners, "moving": moving, "moved": False,
                         "start_screen": ev.screen,
                         "originals": {i: doc.curves[i] for i in moving}}
            return

        cid = self.canvas.pick_curve(ev.world)
        if cid is not None:
            sel = set(self.model.selection)
            if shift:
                self.model.set_selection(sel ^ {cid})
                return
            if cid not in sel:
                self.model.set_selection({cid})
            grab, _ = geom.closest_point(doc.curves[cid], ev.world)
            moving = set(self.model.selection)
            self.drag = {"kind": "body", "origin": grab, "moving": moving, "moved": False,
                         "start_screen": ev.screen,
                         "originals": {i: doc.curves[i] for i in moving}}
            return

        if not shift:
            self.model.set_selection(set())
        self.drag = {"kind": "box", "start": ev.world, "end": ev.world, "additive": shift,
                     "moving": (), "moved": False, "start_screen": ev.screen}

    # --- move ----------------------------------------------------------------------
    def move(self, ev):
        super().move(ev)
        d = self.drag
        if d is None or not ev.dragging:
            return
        if not d["moved"]:
            s0 = d["start_screen"]
            if math.hypot(ev.screen.x() - s0.x(), ev.screen.y() - s0.y()) < DRAG_START_PX:
                return
            d["moved"] = True
            if d["kind"] in ("handle", "body"):
                self.model.begin_edit()
                # Re-snap now that snapping is active for this drag.
                ev.snap = self.canvas.snap(ev.world, self.anchor(), self.snap_exclude())
                self.canvas.last_snap = ev.snap

        doc = self.model.doc
        if d["kind"] == "handle":
            target = ev.point
            new = d["originals"][d["cid"]].with_handle(d["name"], target)
            if new is None:
                return
            doc.curves[d["cid"]] = new
            for pid, which in d["partners"]:
                moved = d["originals"][pid].with_handle(which, target)
                if moved is not None:
                    doc.curves[pid] = moved
            self.model.notify()
        elif d["kind"] == "body":
            dx = ev.point[0] - d["origin"][0]
            dy = ev.point[1] - d["origin"][1]
            for i, orig in d["originals"].items():
                doc.curves[i] = orig.map_points(lambda p: (p[0] + dx, p[1] + dy))
            self.model.notify()
        else:
            d["end"] = ev.world

    def release(self, ev):
        d, self.drag = self.drag, None
        if d is None:
            return
        if d["kind"] in ("handle", "body"):
            if d["moved"]:
                self.model.commit_edit("Edit point" if d["kind"] == "handle" else "Move")
        elif d["moved"]:
            crossing = d["end"][0] < d["start"][0]
            ids = set(self.canvas.curves_in_rect(d["start"], d["end"], crossing))
            if d["additive"]:
                ids |= self.model.selection
            self.model.set_selection(ids)

    def cancel(self):
        if self.drag is not None:
            if self.drag["kind"] in ("handle", "body") and self.drag["moved"]:
                self.model.cancel_edit()
            self.drag = None
            return True
        if self.model.selection:
            self.model.set_selection(set())
            return True
        return False

    def finish(self):
        pass

    # --- context menu -------------------------------------------------------------
    def right_click(self, ev):
        cid = self.canvas.pick_curve(ev.world)
        if cid is not None and cid not in self.model.selection:
            self.model.set_selection({cid})
        if not self.model.selection:
            return
        menu = QMenu(self.canvas)
        menu.addAction("Toggle reference (dashed)", self.model.toggle_reference)
        menu.addAction("Delete", self.model.delete_selection)
        menu.popup(self.canvas.mapToGlobal(ev.screen.toPoint()))

    # --- painting ----------------------------------------------------------------
    def paint_overlay(self, painter):
        d = self.drag
        if d is None or d["kind"] != "box" or not d["moved"]:
            return
        a = self.canvas.to_screen(d["start"])
        b = self.canvas.to_screen(d["end"])
        crossing = d["end"][0] < d["start"][0]
        color = "#2e9d57" if crossing else "#1a73d9"
        painter.setPen(make_pen(color, 1.0, dashed=crossing))
        fill = QColor(color)
        fill.setAlpha(28)
        painter.setBrush(fill)
        painter.drawRect(QRectF(a, b).normalized())
        painter.setBrush(Qt.NoBrush)

    def prompt(self):
        return ""

    def hint(self):
        return ("Click to select (Shift adds), drag points or curves to edit, drag on empty space to box select "
                "(left-to-right: inside, right-to-left: crossing). Right-click for options.")
