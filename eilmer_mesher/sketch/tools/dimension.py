"""
Dimension tool (as in NX): click a line for its length or an arc for its sweep
angle, move to place the annotation, click. Typing a value while placing sets it
and drives the curve. Click an existing dimension to edit its value.
"""

import numpy as np

from ...model import geom
from ...model.document import Dimension
from ..canvas import COLORS
from ..typed_input import single_number
from .base import Tool


def offset_at(kind, curve, world):
    """Annotation offset that puts a dimension of this curve under the cursor."""
    if kind == "length":
        p0, p1 = np.asarray(curve.p0, float), np.asarray(curve.p1, float)
        d = p1 - p0
        n = np.array([-d[1], d[0]]) / np.hypot(*d)
        return float((np.asarray(world, float) - p0) @ n)
    centre, r, _, _ = geom.arc_geometry(curve.p0, curve.p1, curve.centre)
    return float(np.hypot(*(np.asarray(world, float) - centre)) - r)


class DimensionTool(Tool):
    name, label = "dimension", "Dimension"
    snaps = False
    hover_curves = True

    def reset(self):
        self.curve_id = None      # curve being dimensioned (placement in progress)
        self.offset = 0.0

    def busy(self):
        return self.curve_id is not None

    def _kind(self):
        return self.model.dimension_kind(self.model.doc.curves[self.curve_id])

    def press(self, ev):
        if self.curve_id is None:
            did = self.canvas.pick_dimension(ev.screen)
            if did is not None:
                self.canvas.editRequested.emit(did)
                return
            cid = self.canvas.pick_curve(ev.world)
            if cid is None:
                return
            if self.model.dimension_kind(self.model.doc.curves[cid]) is None:
                self.canvas.message.emit("Dimensions apply to lines (length) and arcs (angle).")
                return
            self.curve_id = cid
            self.offset = offset_at(self._kind(), self.model.doc.curves[cid], ev.world)
            return
        self._place()

    def move(self, ev):
        super().move(ev)
        if self.curve_id is not None and self.curve_id in self.model.doc.curves:
            self.offset = offset_at(self._kind(), self.model.doc.curves[self.curve_id], ev.world)

    def _place(self):
        did = self.model.add_dimension(self.curve_id, self.offset)
        self.reset()
        return did

    def accept_text(self, text):
        if self.curve_id is None:
            return "click a line or arc first"
        kind = self._kind()
        if kind == "length":
            value = self.canvas.parse_length(text)
            if value is None or value <= 0:
                return "type a positive length"
        else:
            value = single_number(text)
            if value is None or not 0 < abs(value) < 360:
                return "type an angle between 0 and 360 degrees"
            value = abs(value)
        did = self._place()
        try:
            self.model.set_dimension_value(did, value)
        except ValueError as e:
            return str(e)
        return None

    def prompt(self):
        if self.curve_id is None:
            return "Line or arc to dimension (or click a dimension to edit it)"
        what = "length" if self._kind() == "length" else "angle (deg)"
        return f"Place the dimension, or type the {what}"

    def hint(self):
        return ("Dimension: click a line (length) or arc (angle), move to place it and click. Type a value "
                "while placing to set it. Double-click a dimension later to change it.")

    def paint(self, painter):
        if self.curve_id is None or self.curve_id not in self.model.doc.curves:
            return
        preview = Dimension("_preview", self._kind(), self.curve_id, self.offset)
        painter.save()
        painter.resetTransform()
        self.canvas.paint_dimension(painter, preview, COLORS["preview"])
        painter.restore()
