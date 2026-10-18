"""
Select and move.

  click curve or point   select it (Ctrl toggles)
  double-click           edit its dimensions (point, line, arc)
  drag a handle          move that point; curves sharing the endpoint follow
  drag a selection       move it
  drag empty space       box select: left-to-right = window (fully inside),
                         right-to-left = crossing (touching)
  right-click            context menu
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
EDITABLE = ("point", "line", "arc")


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
        return self.drag.get("origin") if self.drag else None

    def snap_exclude(self):
        return self.drag["moving"] if self.drag else ()

    def entity_kind(self, eid):
        doc = self.model.doc
        if eid in doc.points:
            return "point"
        return doc.curves[eid].type if eid in doc.curves else None

    # --- press -------------------------------------------------------------------
    def press(self, ev):
        add = bool(ev.modifiers & Qt.ControlModifier)       # Ctrl+click adds / removes
        doc = self.model.doc
        handle = self.canvas.pick_handle(ev.screen)
        if handle is not None and not add:
            cid, name, point = handle
            partners = []
            if name in ("start", "end"):
                partners = self.model.connected_ends(point, exclude={cid},
                                                     tol=self.canvas.px(CONNECT_TOL_PX))
            moving = {cid} | {pid for pid, _ in partners}
            self.drag = {"kind": "handle", "cid": cid, "name": name, "origin": point,
                         "partners": partners, "moving": moving, "moved": False,
                         "start_screen": ev.screen,
                         "originals": {i: doc.curves[i] for i in moving}}
            return

        eid = self.canvas.pick_entity(ev.screen, ev.world)
        if eid is not None:
            sel = set(self.model.selection)
            if add:
                self.model.set_selection(sel ^ {eid})
                return
            if eid not in sel:
                self.model.set_selection({eid})
            if eid in doc.points:
                grab = doc.points[eid]
            else:
                grab, _ = geom.closest_point(doc.curves[eid], ev.world)
            moving = set(self.model.selection)
            self.drag = {"kind": "body", "origin": grab, "moving": moving, "moved": False,
                         "start_screen": ev.screen,
                         "originals": {i: doc.curves[i] for i in moving if i in doc.curves},
                         "point_originals": {i: doc.points[i] for i in moving if i in doc.points}}
            return

        if not add:
            self.model.set_selection(set())
        self.drag = {"kind": "box", "start": ev.world, "end": ev.world, "additive": add,
                     "moving": (), "moved": False, "start_screen": ev.screen}

    def double_click(self, ev):
        self.drag = None
        eid = self.canvas.pick_entity(ev.screen, ev.world)
        if eid is not None and self.entity_kind(eid) in EDITABLE:
            self.model.set_selection({eid})
            self.canvas.editRequested.emit(eid)

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
            for i, (x, y) in d["point_originals"].items():
                doc.points[i] = (x + dx, y + dy)
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
        eid = self.canvas.pick_entity(ev.screen, ev.world)
        if eid is not None and eid not in self.model.selection:
            self.model.set_selection({eid})
        if not self.model.selection:
            return
        menu = QMenu(self.canvas)
        sel = self.model.selection
        if len(sel) == 1:
            (only,) = sel
            if self.entity_kind(only) in EDITABLE:
                menu.addAction("Edit dimensions...", lambda: self.canvas.editRequested.emit(only))
        curves = sel & set(self.model.doc.curves)
        if len(curves) >= 2:
            menu.addAction("Merge", lambda: self._merge(curves))
        if curves:
            menu.addAction("Toggle reference (dashed)", self.model.toggle_reference)
        menu.addAction("Delete", self.model.delete_selection)
        menu.popup(self.canvas.mapToGlobal(ev.screen.toPoint()))

    def _merge(self, ids):
        _, joins = self.model.merge(ids)
        self.canvas.message.emit(f"Merged {len(ids)} curves into {len(ids) - joins}." if joins else
                                 "Nothing to merge: pieces must share an end and be collinear lines, "
                                 "arcs of one circle (under 180 degrees in total) or parts of one spline.")

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
        return ("Click to select (Ctrl adds), double-click to edit dimensions, drag points or curves to edit, "
                "drag on empty space to box select (left-to-right: inside, right-to-left: crossing).")
