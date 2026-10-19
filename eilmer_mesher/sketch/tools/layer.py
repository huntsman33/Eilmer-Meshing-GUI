"""
Boundary-layer tool.

  click block walls        pick them (click again to drop one)
  Enter                    cut a boundary layer along the picked walls and re-block
  double-click a wall      the layer's settings (first cell / y+, growth, cells);
                           on a wall without a layer, the settings for the next one
  right-click              apply / settings / remove layer
"""
import math

import numpy as np
from PySide6.QtCore import QPointF
from PySide6.QtWidgets import QMenu

from ... import settings
from ...mesh.preview import FaceEval
from ...model.document import BlockError, CurveRef
from ...model.entities import REFERENCE
from ..canvas import COLORS, make_pen
from .block import BlockTool


class LayerTool(BlockTool):
    name, label = "layer", "Boundary layer"

    def reset(self):
        super().reset()
        self.walls = []               # picked (block id, face)

    def busy(self):
        return bool(self.walls)

    # --- picking ----------------------------------------------------------------------------
    def _face_refs(self, bid, face):
        block = self.canvas.resolution().by_id().get(bid)
        return block.sides[face] if block is not None else []

    def press(self, ev):
        face = self.face_at(ev.screen)
        if face is None:
            return
        bid, f = face
        if face in self.walls:
            self.walls.remove(face)
        elif any(self.model.doc.curves[r.id].kind == REFERENCE for r in self._face_refs(bid, f)):
            self.canvas.message.emit("That is a dashed reference line, not a wall.")
            return
        elif self.model.layer_at_face(bid, f) is not None:
            self.canvas.message.emit("This wall already has a boundary layer: double-click it to change it.")
            return
        else:
            self.walls.append(face)
        self.canvas.message.emit(f"{len(self.walls)} wall(s) picked: Enter cuts the boundary layer.")

    def double_click(self, ev):
        face = self.face_at(ev.screen)
        if face is not None:
            self.canvas.layerRequested.emit(*face)

    def finish(self):
        res = self.canvas.resolution()
        walls = [(bid, f) for bid, f in self.walls if bid in res.by_id()]
        self.walls = []
        if not walls:
            return
        d = settings.layer_defaults()
        try:
            self.model.add_boundary_layer(walls, d["first"], d["growth"], d["cells"], d["transition"])
            self.canvas.message.emit(f"Boundary layer cut along {len(walls)} wall(s), tagged as no-slip walls; "
                                     "its blocks were re-meshed.")
        except (BlockError, ValueError) as e:
            self.canvas.message.emit(f"Cannot cut the boundary layer: {e}.")

    def cancel(self):
        if self.walls:
            self.walls = []
            return True
        return False

    def right_click(self, ev):
        face = self.face_at(ev.screen)
        menu = QMenu(self.canvas)
        act = menu.addAction(f"Cut boundary layer on {len(self.walls)} wall(s)", self.finish)
        act.setEnabled(bool(self.walls))
        if face is not None:
            lid = self.model.layer_at_face(*face)
            menu.addAction("Layer settings..." if lid else "Settings for new layers...",
                           lambda: self.canvas.layerRequested.emit(*face))
            if lid:
                menu.addAction("Remove boundary layer", lambda: self._remove(lid))
        else:
            menu.addAction("Settings for new layers...", lambda: self.canvas.layerRequested.emit("", ""))
        if self.walls:
            menu.addAction("Clear picked walls", lambda: self.walls.clear())
        menu.popup(self.canvas.mapToGlobal(ev.screen.toPoint()))

    def _remove(self, lid):
        try:
            self.model.remove_boundary_layer(lid)
            self.canvas.message.emit("Boundary layer removed.")
        except BlockError as e:
            self.canvas.message.emit(f"Cannot remove it: {e}.")

    # --- text ---------------------------------------------------------------------------------
    def prompt(self):
        if self.walls:
            return f"{len(self.walls)} wall(s) picked; Enter cuts the layer"
        return "Click block walls, then Enter; double-click a wall for settings"

    def hint(self):
        d = settings.layer_defaults()
        return (f"Boundary layer: pick walls and press Enter (next layer: first cell {d['first']:.3g} m, "
                f"growth {d['growth']:.3g}, {d['cells']} cells). Double-click a wall for its settings.")

    # --- painting -----------------------------------------------------------------------------
    def paint(self, painter):
        pass

    def paint_overlay(self, painter):
        doc = self.model.doc
        for layer in doc.layers:
            for w in layer.walls:
                refs = [CurveRef.from_dict(r) for r in w["refs"]]
                if all(r.id in doc.curves for r in refs):
                    self._ticks(painter, refs, w.get("inside"))
        for bid, f in self.walls:
            refs = self._face_refs(bid, f)
            if refs:
                self._face(painter, refs, COLORS["selected"], 4.0)
        if self.hover is not None and self.hover[0] == "face":
            _, bid, f = self.hover
            refs = self._face_refs(bid, f)
            if refs:
                self._face(painter, refs, COLORS["hover"], 3.0)
                lid = self.model.layer_at_face(bid, f)
                text = "boundary layer (double-click to edit)" if lid else (
                    "dashed: not a wall" if any(doc.curves[r.id].kind == REFERENCE for r in refs)
                    else "wall: click to pick")
                pts = FaceEval(doc, refs)([0.5])
                painter.drawText(self.canvas.to_screen(pts[0]) + QPointF(10, -10), text)

    def _face(self, painter, refs, color, width):
        pts = FaceEval(self.model.doc, refs)(np.linspace(0.0, 1.0, 64))
        scr = [self.canvas.to_screen(p) for p in pts]
        painter.setPen(make_pen(color, width))
        for a, b in zip(scr, scr[1:]):
            painter.drawLine(a, b)

    def _ticks(self, painter, refs, inside):
        """The wall drawn with short ticks on the side the layer grows into."""
        t = np.linspace(0.0, 1.0, 25)
        fe = FaceEval(self.model.doc, refs)
        pts = fe(t)
        scr = [self.canvas.to_screen(p) for p in pts]
        painter.setPen(make_pen(COLORS["layer"], 2.5))
        for a, b in zip(scr, scr[1:]):
            painter.drawLine(a, b)
        side = 1.0
        if inside is not None:
            mid, d = pts[12], pts[13] - pts[11]
            side = 1.0 if (-d[1]) * (inside[0] - mid[0]) + d[0] * (inside[1] - mid[1]) > 0 else -1.0
        painter.setPen(make_pen(COLORS["layer"], 1.5))
        for k in range(1, len(scr) - 1, 2):
            d = scr[k + 1] - scr[k - 1]
            n = math.hypot(d.x(), d.y()) or 1.0
            # screen y points down, so the world-left normal (-dy, dx) is (dy, -dx) on screen
            normal = QPointF(d.y() / n, -d.x() / n) * side
            painter.drawLine(scr[k], scr[k] + normal * 7.0)
