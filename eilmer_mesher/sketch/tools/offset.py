"""
Offset tool (as NX's Offset Curve).

  click curves        pick them (click a picked one again to drop it); anything
                      selected beforehand is included
  Enter               the offset dialog: distance and side, previewed live; OK makes
                      the offset curves (the originals stay), in one undo step

Connected curves are offset as chains: closed shapes grow (Reverse shrinks them),
open chains go to the left of their direction (Reverse: the right). An arrow on
each chain shows the side.
"""
import math

from PySide6.QtCore import QPointF
from PySide6.QtWidgets import QDialog

from ...model.offset import find_chains, offset_chains, side_markers
from ..canvas import COLORS, make_pen
from ..offset_dialog import OffsetDialog
from .modify import _TargetTool

ARROW_PX = 26


class OffsetTool(_TargetTool):
    name, label = "offset", "Offset"

    def __init__(self, canvas):
        self.last_distance = None          # metres; remembered for the session
        self.last_source_ref = False       # the 'sources to reference' box, likewise
        super().__init__(canvas)

    def reset(self):
        self.preview = []                  # offset curves shown while the dialog is open
        self.markers = []                  # [(point, unit normal)] arrows showing the side
        self.dialog = None
        self.source_ref = False            # preview the sources dashed
        self.source_curves = []

    def busy(self):
        return self.dialog is not None

    def picked(self):
        """Picked curve ids, in document order."""
        sel = self.model.selection
        return [cid for cid in self.model.doc.curves if cid in sel]

    def press(self, ev):
        cid = self.canvas.pick_curve(ev.world)
        if cid is not None:
            self.model.set_selection(self.model.selection ^ {cid})

    def finish(self):
        ids = self.picked()
        if not ids:
            self.canvas.message.emit("Offset: click the curves to offset first, then press Enter.")
            return
        curves = [self.model.doc.curves[i] for i in ids]
        if self.last_distance is None:
            self.last_distance = self.canvas.grid_step()
        chains = find_chains(curves)
        closed = sum(c.closed for c in chains)
        summary = (f"{len(ids)} curve(s) in {len(chains)} chain(s): {closed} closed (grow by default), "
                   f"{len(chains) - closed} open (to the left by default).")
        dlg = OffsetDialog(self.canvas.units, self.last_distance, False, summary, self.canvas,
                           self.last_source_ref)
        dlg.valuesChanged.connect(lambda d, rev, sr: self._update_preview(curves, d, rev, sr))
        self.dialog = dlg
        dlg.refresh()
        accepted = dlg.exec() == QDialog.Accepted
        self.reset()
        self.canvas.update()
        dlg.deleteLater()
        if not accepted:
            return
        d, rev, sref = dlg.distance(), dlg.reverse(), dlg.source_to_reference()
        self.last_distance = d
        self.last_source_ref = sref
        try:
            new_ids = self.model.offset(ids, d, rev, sref)
        except ValueError as e:
            self.canvas.message.emit(f"Cannot offset: {e}.")
            return
        self.model.set_selection(new_ids)
        self.canvas.message.emit(f"Offset {len(ids)} curve(s) by {self.canvas.units.format(d)}: "
                                 f"{len(new_ids)} new curve(s).")

    def _update_preview(self, curves, distance, reverse, source_ref=False):
        self.source_ref = source_ref
        self.source_curves = curves if source_ref else []
        self.markers = side_markers(curves, reverse)
        try:
            self.preview = offset_chains(curves, distance, reverse)
            if self.dialog is not None:
                self.dialog.set_error("")
        except ValueError as e:
            self.preview = []
            if self.dialog is not None:
                self.dialog.set_error(f"{str(e)[0].upper()}{str(e)[1:]}.")
        self.canvas.update()

    def cancel(self):
        if self.model.selection:
            self.model.set_selection(set())
            return True
        return False

    # --- text ---------------------------------------------------------------------------
    def prompt(self):
        return f"Curves to offset ({len(self.picked())} picked): then Enter"

    def hint(self):
        return ("Offset: click curves (connected ones are offset together), then press Enter to set the "
                "distance and side. Closed shapes grow, or shrink with Reverse.")

    # --- painting -----------------------------------------------------------------------
    def paint(self, painter):
        for c in self.source_curves:                   # the sources, drawn dashed over the solid ones
            self.canvas.draw_curve(painter, c, COLORS["preview"], 1.0, dashed=True)
        self.draw_preview(painter, self.preview)

    def paint_overlay(self, painter):
        painter.setPen(make_pen(COLORS["preview"], 2.0))
        for p, (nx, ny) in self.markers:
            q = self.canvas.to_screen(p)
            d = QPointF(nx, -ny)                       # screen y points down
            tip = q + d * ARROW_PX
            painter.drawLine(q, tip)
            for ang in (math.radians(150), math.radians(-150)):
                c, s = math.cos(ang), math.sin(ang)
                w = QPointF(d.x() * c - d.y() * s, d.x() * s + d.y() * c)
                painter.drawLine(tip, tip + w * 8.0)
