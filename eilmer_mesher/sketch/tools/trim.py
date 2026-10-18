"""
Quick trim (as in NX): hover a curve to highlight the piece between the
nearest points where it meets other curves (or its ends); click to delete it.
"""
from ...model import geom
from ...model.topology import curve_params
from ..canvas import COLORS
from .base import Tool


class TrimTool(Tool):
    name, label = "trim", "Quick trim"
    snaps = False

    def reset(self):
        self.target = None
        self._cache = (None, {})

    def _params(self, cid):
        rev, cache = self._cache
        if rev != self.model.revision:
            cache = {}
            self._cache = (self.model.revision, cache)
        if cid not in cache:
            cache[cid] = curve_params(self.model.doc, cid)
        return cache[cid]

    def piece_at(self, world):
        """(curve id, ta, tb) of the piece under the cursor, or None."""
        cid = self.canvas.pick_curve(world)
        if cid is None:
            return None
        ts = self._params(cid)
        t, _, _ = geom.closest_param(self.model.doc.curves[cid], world)
        k = max(i for i in range(len(ts) - 1) if ts[i] <= t)
        return cid, ts[k], ts[k + 1]

    def move(self, ev):
        super().move(ev)
        self.target = self.piece_at(ev.world)

    def press(self, ev):
        piece = self.piece_at(ev.world)
        if piece is not None:
            self.trim(*piece)
        self.target = self.piece_at(ev.world)

    def trim(self, cid, ta, tb):
        with self.model.edit("Trim") as doc:
            c = doc.curves[cid]
            keep = []
            if ta > 0.0:
                keep.append(geom.subcurve(c, 0.0, ta))
            if tb < 1.0:
                keep.append(geom.subcurve(c, tb, 1.0))
            doc.remove_blocks_using([cid])
            if not keep:
                doc.remove_curves([cid])
                return
            doc.curves[cid] = keep[0]                       # first piece keeps the id
            if len(keep) > 1:
                doc.add_curve(keep[1].copy(id=doc.new_id("c")))
        self.model.set_selection(self.model.selection)      # prune deleted ids

    def busy(self):
        return False

    def prompt(self):
        return ""

    def hint(self):
        return "Quick trim: click the piece of a curve to delete it, up to where it meets other curves."

    def paint(self, painter):
        if self.target is None:
            return
        cid, ta, tb = self.target
        c = self.model.doc.curves.get(cid)
        if c is None:
            return
        piece = geom.subcurve(c, ta, tb)
        self.canvas.draw_curve(painter, piece, COLORS["trim"], 3.0, dashed=False)
