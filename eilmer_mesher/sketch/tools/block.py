"""
Block tool.

  click a region          make a block over it (corners: its four sharpest turns)
  click a block           select it
  double-click an edge    cells and clustering along that edge
  right-click             make block / patch (Laplace, AOPatch) / sub-divide / set corners / delete
"""
import math

import numpy as np
from PySide6.QtCore import QPointF, Qt
from PySide6.QtWidgets import QApplication, QMenu

from ...mesh.preview import FaceEval
from ...model import blocking
from ...model.geom import project_polyline
from ...model.document import FACES, BlockError
from ..canvas import COLORS, PICK_PX, make_pen
from ..bc_dialogs import add_bc_menu
from .base import Tool


class BlockTool(Tool):
    name, label = "block", "Block"
    snaps = False

    def reset(self):
        self.hover = None             # ("face", bid, face) | ("block", bid) | ("region", region)
        self.selected = None          # block id
        self.corner_pick = None       # (block id, [points]) while re-picking corners
        self._faces = (None, {})      # (revision, {(block id, face): world points})

    def busy(self):
        return self.corner_pick is not None

    # --- what is under the cursor ---------------------------------------------------------
    def face_points(self):
        rev, faces = self._faces
        if rev != self.model.revision:
            res = self.canvas.resolution()
            md = self.model.doc.with_blocks(res.blocks)
            t = np.linspace(0.0, 1.0, 48)
            faces = {(b.id, f): FaceEval(md, b.sides[f])(t) for b in res.blocks for f in FACES}
            self._faces = (self.model.revision, faces)
        return faces

    def face_at(self, screen):
        best = None
        c = self.canvas
        cursor = np.array(c.to_world(screen))
        for (bid, face), pts in self.face_points().items():
            d = project_polyline(pts, cursor)[1] * c.scale          # distance to the face, in pixels
            if d <= PICK_PX and (best is None or d < best[0]):
                best = (d, bid, face)
        return best[1:] if best else None

    def thing_at(self, ev):
        face = self.face_at(ev.screen)
        if face is not None:
            return ("face",) + face
        bid = self.model.block_at(ev.world, res=self.canvas.resolution())
        if bid is not None:
            return ("block", bid)
        region = self.canvas.topology().region_at(ev.world)
        if region is not None:
            return ("region", region)
        return None

    def move(self, ev):
        super().move(ev)
        self.hover = self.thing_at(ev)

    # --- actions ----------------------------------------------------------------------------
    def press(self, ev):
        if self.corner_pick is not None:
            bid, pts = self.corner_pick
            pts.append(self._nearest_vertex(bid, ev.world))
            if len(pts) == 4:
                self.corner_pick = None
                self._try(lambda: self.model.set_block_corners(bid, pts), "Corners set.")
            return
        thing = self.thing_at(ev)
        if thing is None:
            self.selected = None
        elif thing[0] in ("face", "block"):
            self.selected = thing[1]
            self._report(thing[1])
        else:
            self._try(lambda: self.model.make_block(ev.world), "Block made.")

    def double_click(self, ev):
        thing = self.thing_at(ev)
        if thing is not None and thing[0] == "face":
            self.canvas.edgeRequested.emit(thing[1], thing[2])

    def _nearest_vertex(self, bid, world):
        topo = self.canvas.topology()
        block = next(b for b in self.model.doc.blocks if b.id == bid)
        region, _ = blocking.find_region(topo, block.region)
        verts = blocking.loop_vertices(topo, region.loop) if region else [np.asarray(world)]
        return tuple(min(verts, key=lambda v: math.dist(v, world)))

    def _try(self, action, done):
        try:
            action()
            self.canvas.message.emit(done)
        except (BlockError, ValueError) as e:
            self.canvas.message.emit(f"Cannot do that: {e}.")

    def _report(self, bid):
        res = self.canvas.resolution()
        if bid in res.errors:
            self.canvas.message.emit(f"{bid}: {res.errors[bid]}.")
        elif bid in res.warnings:
            self.canvas.message.emit(f"{bid}: " + "; ".join(res.warnings[bid]) + ".")

    def right_click(self, ev):
        thing = self.thing_at(ev)
        if thing is None:
            if self.canvas.hidden_ids():
                menu = QMenu(self.canvas)
                self.canvas.block_visibility_menu(menu, self.model.block_at(ev.world, res=self.canvas.resolution()))
                menu.popup(self.canvas.mapToGlobal(ev.screen.toPoint()))
            return
        menu = QMenu(self.canvas)
        point = ev.world
        self.canvas.block_visibility_menu(menu, self.model.block_at(point, res=self.canvas.resolution()))
        menu.addSeparator()
        if thing[0] == "face":
            menu.addAction("Edge cells...", lambda: self.canvas.edgeRequested.emit(thing[1], thing[2]))
            block = self.canvas.resolution().by_id()[thing[1]]
            add_bc_menu(menu, self.canvas, [r.id for r in block.sides[thing[2]]])
        if thing[0] == "region":
            menu.addAction("Make block", lambda: self._try(lambda: self.model.make_block(point), "Block made."))
        if thing[0] in ("face", "block"):
            bid = thing[1]
            self._patch_menu(menu, bid)
            menu.addAction("Set corners...", lambda: self._start_corners(bid))
            menu.addAction("Delete block", lambda: self.model.delete_block(bid))
        region = self.canvas.topology().region_at(point)
        if region is not None:
            sub = menu.addMenu("Sub-divide")
            sub.addAction("Split N x M...", lambda: self.canvas.splitRequested.emit(point))
            sub.addAction("O-grid", lambda: self._subdivide(point, "ogrid"))
            act = sub.addAction("O-grid around hole", lambda: self._subdivide(point, "ogrid_hole"))
            act.setEnabled(len(region.holes) == 1)
            sub.addAction("Centroid split", lambda: self._subdivide(point, "centroid"))
        menu.popup(self.canvas.mapToGlobal(ev.screen.toPoint()))

    def _patch_menu(self, menu, bid):
        """Patch type and smoothing for a block (the refinement options)."""
        from ...model.document import DEFAULT_SMOOTHING
        block = next((b for b in self.model.doc.blocks if b.id == bid), None)
        if block is None:
            return
        sub = menu.addMenu("Patch")
        options = [("CoonsPatch", "CoonsPatch", None),
                   ("CoonsPatch + Laplace smoothing", "CoonsPatch", DEFAULT_SMOOTHING)]
        for text, patch, smoothing in options:
            act = sub.addAction(text, lambda p=patch, sm=smoothing: self.model.set_block_patch(
                [bid], p, {}, sm))
            act.setCheckable(True)
            act.setChecked(block.patch == patch and bool(block.smoothing) == bool(smoothing))
        act = sub.addAction("AOPatch...", lambda: self._aopatch(bid))
        act.setCheckable(True)
        act.setChecked(block.patch == "AOPatch")

    def _aopatch(self, bid):
        from ..block_dialogs import AOPatchDialog
        block = next(b for b in self.model.doc.blocks if b.id == bid)
        dlg = AOPatchDialog(block.patch_opts.get("nx", 10), block.patch_opts.get("ny", 10), self.canvas)
        if dlg.exec():
            self.model.set_block_patch([bid], "AOPatch", dlg.values(), smoothing=None)

    def _subdivide(self, point, kind, **opts):
        self._try(lambda: self.model.subdivide(point, kind, **opts), "Sub-divided.")

    def _start_corners(self, bid):
        self.corner_pick = (bid, [])
        self.canvas.message.emit("Click the block's four corners, going round it.")

    def cancel(self):
        if self.corner_pick is not None:
            self.corner_pick = None
            return True
        if self.selected is not None:
            self.selected = None
            return True
        return False

    def finish(self):
        pass

    def claims_key(self, key):
        return key == Qt.Key_H

    def claims_hold(self, key):
        return key == Qt.Key_H

    def key(self, key):
        if key != Qt.Key_H:
            return False
        if QApplication.keyboardModifiers() & Qt.ShiftModifier:
            self.canvas.toggle_blocks_hidden(self.canvas.all_block_ids())
            return True
        bid = self.selected
        if bid is None:
            bid = self._cursor_block()
        if bid is not None:
            self.canvas.toggle_blocks_hidden({bid})
        return True

    def _cursor_block(self):
        pos = self.canvas.last_screen
        if pos is None:
            return None
        return self.model.block_at(tuple(self.canvas.to_world(pos)), res=self.canvas.resolution())

    # --- text ---------------------------------------------------------------------------------
    def prompt(self):
        if self.corner_pick is not None:
            return f"Corner {len(self.corner_pick[1]) + 1} of 4"
        return "Click a region to block it; double-click an edge for its cells"

    def hint(self):
        return ("Block: click a closed region to make a block, double-click a block edge to set its cells, "
                "right-click to sub-divide, set corners or delete. H hides / shows the selected block, Shift+H all blocks.")

    # --- painting -----------------------------------------------------------------------------
    def paint(self, painter):
        res = self.canvas.resolution()
        if self.selected is not None and self.selected in res.polygons:
            self._outline(painter, res.polygons[self.selected], COLORS["selected"], 2.5)
        if self.hover is None:
            return
        kind = self.hover[0]
        if kind == "region":
            self._outline(painter, self.hover[1].polygon, COLORS["hover"], 2.0)
        elif kind == "block" and self.hover[1] in res.polygons:
            self._outline(painter, res.polygons[self.hover[1]], COLORS["hover"], 2.0)

    def paint_overlay(self, painter):
        if self.corner_pick is not None:
            painter.setPen(make_pen(COLORS["selected"], 2.0))
            for p in self.corner_pick[1]:
                painter.drawEllipse(self.canvas.to_screen(p), 6.0, 6.0)
        if self.hover is None or self.hover[0] != "face":
            return
        _, bid, face = self.hover
        res = self.canvas.resolution()
        block = res.by_id().get(bid)
        if block is None:
            return
        md = self.model.doc.with_blocks(res.blocks)
        pts = FaceEval(md, block.sides[face])(np.linspace(0.0, 1.0, 64))
        scr = [self.canvas.to_screen(p) for p in pts]
        painter.setPen(make_pen(COLORS["selected"], 3.5))
        for a, b in zip(scr, scr[1:]):
            painter.drawLine(a, b)
        painter.drawEllipse(scr[0], 5.0, 5.0)                       # start: o
        d = scr[-1] - scr[-4]
        n = math.hypot(d.x(), d.y()) or 1.0
        d = QPointF(d.x() / n, d.y() / n)
        perp = QPointF(-d.y(), d.x())
        tip = scr[-1]
        painter.drawLine(tip, tip - d * 10 + perp * 5)              # end: >
        painter.drawLine(tip, tip - d * 10 - perp * 5)
        painter.drawText(tip + QPointF(8, -8), f"{face} ({block.niv - 1 if face in ('south', 'north') else block.njv - 1} cells)")

    def _outline(self, painter, poly, color, width):
        painter.setPen(make_pen(color, width))
        pts = [QPointF(x, y) for x, y in poly]
        for a, b in zip(pts, pts[1:] + pts[:1]):
            painter.drawLine(a, b)
