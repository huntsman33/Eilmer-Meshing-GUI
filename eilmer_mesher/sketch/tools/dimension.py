"""
Dimension tool (as in NX): click a line for its length or an arc for its sweep
angle, move to place the annotation, click. Click a line and then a second line
for the distance between them (parallel lines) or the angle between them; the
angle shown is the one in the sector the annotation is placed in. Typing a value
while placing sets it and drives the curve (the second line, for two lines).
Click an existing dimension to edit its value.
"""

import numpy as np

from ...model import dimensions, geom
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


def place_at(doc, dim, world):
    """Set dim's placement so its annotation is under world (a two-line angle may change sector)."""
    a = doc.curves[dim.curve]
    if dim.curve2 is None:
        dim.offset = offset_at(dim.kind, a, world)
    elif dim.kind == "distance":
        dim.offset = float((np.asarray(world, float) - np.asarray(a.p0, float)) @ dimensions.direction(a))
    else:
        dim.offset, dim.sector = dimensions.angle_placement(a, doc.curves[dim.curve2], world)


class DimensionTool(Tool):
    name, label = "dimension", "Dimension"
    snaps = False
    hover_curves = True

    def reset(self):
        self.dim = None           # the dimension being placed (not yet in the document)

    def busy(self):
        return self.dim is not None

    @property
    def curve_id(self):
        return self.dim.curve if self.dim is not None else None

    def _alive(self):
        return self.dim is not None and all(c in self.model.doc.curves for c in self.dim.curve_ids())

    def press(self, ev):
        doc = self.model.doc
        if self.dim is None:
            did = self.canvas.pick_dimension(ev.screen)
            if did is not None:
                self.canvas.editRequested.emit(did)
                return
            cid = self.canvas.pick_curve(ev.world)
            if cid is None:
                return
            kind = self.model.dimension_kind(doc.curves[cid])
            if kind is None:
                self.canvas.message.emit("Dimensions apply to lines (length) and arcs (angle).")
                return
            self.dim = Dimension("_preview", kind, cid)
            place_at(doc, self.dim, ev.world)
            return
        if not self._alive():
            self.reset()
            return
        if self.dim.curve2 is None and doc.curves[self.dim.curve].type == "line":
            other = self.canvas.pick_curve(ev.world, among=[c for c in doc.curves if c != self.dim.curve])
            kind = self.model.two_line_kind(self.dim.curve, other) if other is not None else None
            if kind is not None:                     # a second line: distance or angle between them
                self.dim = Dimension("_preview", kind, self.dim.curve, 0.0, other)
                place_at(doc, self.dim, ev.world)
                return
        self._place()

    def move(self, ev):
        super().move(ev)
        if self._alive():
            place_at(self.model.doc, self.dim, ev.world)

    def _place(self):
        d = self.dim
        did = self.model.add_dimension(d.curve, d.offset, d.curve2, d.sector)
        self.reset()
        return did

    def accept_text(self, text):
        if self.dim is None:
            return "click a line or arc first"
        kind, two = self.dim.kind, self.dim.curve2 is not None
        if kind in ("length", "distance"):
            value = self.canvas.parse_length(text)
            if value is None or value <= 0:
                return f"type a positive {kind}"
        elif two:
            value = single_number(text)
            if value is None or not 0 < value < 180:
                return "type an angle between 0 and 180 degrees"
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
        if self.dim is None:
            return "Line or arc to dimension (or click a dimension to edit it)"
        if self.dim.curve2 is not None:
            what = "distance" if self.dim.kind == "distance" else "angle (deg)"
            return f"Place the dimension, or type the {what}"
        if self.dim.kind == "length":
            return "Place the dimension, click a second line (distance or angle), or type the length"
        return "Place the dimension, or type the angle (deg)"

    def hint(self):
        return ("Dimension: click a line (length) or arc (angle), move to place it and click. Click a second "
                "line instead for the distance between parallel lines or the angle between two lines (place "
                "it in the sector you want). Type a value while placing to set it; the second line moves. "
                "Double-click a dimension later to change it.")

    def paint(self, painter):
        if not self._alive():
            return
        painter.save()
        painter.resetTransform()
        self.canvas.paint_dimension(painter, self.dim, COLORS["preview"])
        painter.restore()
