"""
The sketch canvas: a QWidget with its own world<->screen transform.

A custom widget (rather than QGraphicsView) keeps full double precision at any
zoom: Eilmer geometries are often millimetre-sized, and QGraphicsView's integer
scrollbars overflow at the scales needed. Picking and snapping use the cached
numpy samples in model.geom.
"""
import math
from dataclasses import dataclass

import numpy as np
from PySide6.QtCore import QEvent, QPointF, QRectF, Qt, Signal
from PySide6.QtGui import QColor, QFontMetricsF, QPainter, QPainterPath, QPen, QPolygonF, QTransform
from PySide6.QtWidgets import QWidget

from ..model import geom
from ..model.bcs import tag_color
from ..model.entities import GEOMETRY, REFERENCE, SPLIT
from ..units import Units
from . import snapping
from .snapping import Snap, Snapper, nice_step
from .typed_input import parse_point, single_number

PICK_PX = 8
SNAP_PX = 10
HANDLE_PX = 4

COLORS = {
    "background": "#f5f6f8",
    "grid_minor": "#e6e9ed",
    "grid_major": "#d2d7dd",
    "axes": "#a3abb5",
    "geometry": "#1d2733",
    "reference": "#8a929c",            # drawing aids (dashed): ignored by the mesher
    "split": "#9c6b30",                # block split lines (dash-dot)
    "selected": "#f28c28",
    "hover": "#1a9fd9",
    "preview": "#1a9fd9",
    "snap": "#d43f3a",
    "handle": "#ffffff",
    "trim": "#d43f3a",
    "dimension": "#2a6fb0",
    "block": "#1a9fd9",
    "block_error": "#d43f3a",
    "mesh": "#3d5a73",
    "layer": "#6b8e23",
}
REGION_FILLS = ["#1a9fd9", "#2e9d57", "#f28c28", "#8e5bd0", "#d43f3a", "#c9a227"]
POINT_PX = 5


def make_pen(color, width=1.5, dashed=False):
    """dashed: False, True (dashed) or "dashdot"."""
    pen = QPen(QColor(color), width)
    pen.setCosmetic(True)
    pen.setCapStyle(Qt.RoundCap)
    pen.setJoinStyle(Qt.RoundJoin)
    if dashed:
        pen.setStyle(Qt.CustomDashLine)
        pen.setDashPattern([9.0, 3.0, 1.5, 3.0] if dashed == "dashdot" else [6.0, 4.0])
    return pen


def kind_style(kind):
    """Pen dash style for a curve kind: split lines dash-dot, reference (drawing aid) lines dashed."""
    return {REFERENCE: True, SPLIT: "dashdot"}.get(kind, False)


@dataclass
class ToolEvent:
    world: tuple          # raw cursor position
    snap: Snap
    screen: QPointF
    button: object = Qt.NoButton
    modifiers: object = Qt.NoModifier
    dragging: bool = False

    @property
    def point(self):
        return self.snap.point


class SketchCanvas(QWidget):
    cursorMoved = Signal(object)          # Snap
    typedText = Signal(str)               # printable key typed while the canvas has focus
    escapeIdle = Signal()                 # Esc with nothing to cancel
    toolStateChanged = Signal()           # tool prompt/hint may have changed
    clearTyped = Signal()                 # the tool used the typed value; clear the input box
    editRequested = Signal(str)           # double-click on an entity: open its edit dialog
    message = Signal(str)                 # short status message for the user
    edgeRequested = Signal(str, str)      # (block id, face): open the edge cells dialog
    splitRequested = Signal(object)       # world point of a region to split N x M
    bcRequested = Signal(object)          # curve ids to give a new boundary condition
    layerRequested = Signal(str, str)     # (block id, face) to set boundary-layer settings for ("" for defaults)

    def __init__(self, model, parent=None):
        super().__init__(parent)
        self.model = model
        self.snapper = Snapper()
        self.scale = 200.0                # pixels per world unit
        self.cx = 0.0
        self.cy = 0.0
        self.tool = None
        self.last_snap = None
        self.last_screen = None
        self.hover_id = None
        self._pan = None
        self._pressed = set()
        self._path_cache = {}
        self.mesh = None                  # live mesh (mesh.live.Mesh) to draw
        self._mesh_path = (None, None)
        self.hidden_blocks = set()     # view only: block ids not drawn (still meshed and exported)
        self.show_blocks = True
        self.show_mesh = True
        self.quality = None            # MeshQuality of the mesh shown (mesh/quality.py)
        self.show_quality = False      # colour the cells by quality (heat-map)
        self.quality_metric = "skew"   # "skew" or "ar"
        self._quality_paths = (None, [])
        self.units = Units()
        self.show_regions = False
        self.show_grid = True
        self.hold_keys = {"horizontal": Qt.Key_H, "vertical": Qt.Key_V}
        self.held = set()
        self.setMouseTracking(True)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setMinimumSize(400, 300)
        self.setCursor(Qt.CrossCursor)
        model.changed.connect(self._on_model_changed)
        model.selectionChanged.connect(self.update)

    # --- tools ---------------------------------------------------------------
    def set_tool(self, tool):
        if self.tool is not None:
            self.tool.deactivate()
        self.tool = tool
        tool.activate()
        self.toolStateChanged.emit()
        self.update()

    def _on_model_changed(self):
        if len(self._path_cache) > 4000:
            self._path_cache.clear()
        if self.hover_id not in self.model.doc.curves:
            self.hover_id = None
        self.update()

    # --- transforms ------------------------------------------------------------
    def transform(self):
        s = self.scale
        return QTransform(s, 0.0, 0.0, -s, self.width()/2 - s*self.cx, self.height()/2 + s*self.cy)

    def to_screen(self, p):
        return QPointF(self.width()/2 + (p[0] - self.cx)*self.scale,
                       self.height()/2 - (p[1] - self.cy)*self.scale)

    def to_world(self, q):
        return (self.cx + (q.x() - self.width()/2)/self.scale,
                self.cy - (q.y() - self.height()/2)/self.scale)

    def px(self, n):
        """World length of n pixels."""
        return n / self.scale

    def grid_step(self):
        """Minor grid spacing in metres: a round number in the display units."""
        f = self.units.factor
        return nice_step(self.px(14) / f) * f

    def topology(self):
        return self.model.topology()          # cached per revision by the model

    def resolution(self):
        return self.model.resolution()

    def set_mesh(self, mesh):
        self.mesh = mesh
        self.update()

    # --- typed input (display units in, metres out) -----------------------------
    def parse_point(self, text, anchor=None):
        a = self.units.point_to_display(anchor) if anchor is not None else None
        return self.units.point_from_display(parse_point(text, a))

    def parse_length(self, text):
        v = single_number(text)
        return None if v is None else self.units.from_display(v)

    def zoom_at(self, factor, screen_pos=None):
        if screen_pos is None:
            screen_pos = QPointF(self.width()/2, self.height()/2)
        before = self.to_world(screen_pos)
        self.scale = min(max(self.scale * factor, 1e-6), 1e12)
        after = self.to_world(screen_pos)
        self.cx += before[0] - after[0]
        self.cy += before[1] - after[1]
        self.update()

    def zoom_to_fit(self):
        curves = list(self.model.doc.curves.values())
        if not curves:
            self.cx = self.cy = 0.0
            self.scale = 200.0
            self.update()
            return
        lo = [math.inf, math.inf]
        hi = [-math.inf, -math.inf]
        for c in curves:
            a, b = geom.bbox(c)
            lo = [min(lo[0], a[0]), min(lo[1], a[1])]
            hi = [max(hi[0], b[0]), max(hi[1], b[1])]
        w = max(hi[0] - lo[0], 1e-12)
        h = max(hi[1] - lo[1], 1e-12)
        self.cx = (lo[0] + hi[0]) / 2
        self.cy = (lo[1] + hi[1]) / 2
        self.scale = 0.85 * min(self.width() / w, self.height() / h)
        self.update()

    # --- snapping and picking ------------------------------------------------
    def snap(self, world, anchor=None, exclude=()):
        return self.snapper.snap(self.model.doc, self.model.revision, world, self.px(SNAP_PX),
                                 self.grid_step(), anchor, exclude)

    def pick_curve(self, world, tol_px=PICK_PX, among=None):
        tol = self.px(tol_px)
        best = None
        ids = among if among is not None else self.model.doc.curves.keys()
        for cid in ids:
            _, d = geom.closest_point(self.model.doc.curves[cid], world)
            if d <= tol and (best is None or d < best[1]):
                best = (cid, d)
        return best[0] if best else None

    def pick_point(self, screen, among=None):
        """Construction point under the cursor, or None."""
        best = None
        ids = among if among is not None else self.model.doc.points.keys()
        for pid in ids:
            q = self.to_screen(self.model.doc.points[pid])
            d = math.hypot(q.x() - screen.x(), q.y() - screen.y())
            if d <= POINT_PX + 3 and (best is None or d < best[0]):
                best = (d, pid)
        return best[1] if best else None

    def pick_entity(self, screen, world):
        return self.pick_dimension(screen) or self.pick_point(screen) or self.pick_curve(world)

    def pick_dimension(self, screen):
        """Dimension whose label is under the cursor, or None."""
        for did, dim in self.model.doc.dimensions.items():
            layout = self.dimension_layout(dim)
            if layout is not None and layout["rect"].contains(screen):
                return did
        return None

    # --- dimensions -------------------------------------------------------------------
    def dimension_text(self, dim):
        if dim.curve2 is not None:                      # distance or angle between two lines
            from ..model import dimensions
            value = dimensions.value(self.model.doc, dim)
            if dim.kind == "distance":
                return f"{self.units.to_display(value):.6g} {self.units.name}"
            return f"{value:.6g}\N{DEGREE SIGN}"
        c = self.model.doc.curves[dim.curve]
        if dim.kind == "length":
            value = self.units.to_display(math.dist(c.p0, c.p1))
            return f"{value:.6g} {self.units.name}"
        _, _, _, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
        return f"{math.degrees(abs(sweep)):.6g}\N{DEGREE SIGN}"

    def dimension_layout(self, dim, offset=None):
        """Screen-space drawing of a dimension: polylines, arrows (tip, direction), text, label rect."""
        c = self.model.doc.curves.get(dim.curve)
        if c is None or any(i not in self.model.doc.curves for i in dim.curve_ids()):
            return None
        offset = dim.offset if offset is None else offset
        S = lambda p: self.to_screen((float(p[0]), float(p[1])))

        def unit(v):
            n = math.hypot(v.x(), v.y())
            return QPointF(v.x() / n, v.y() / n) if n > 0 else QPointF(0.0, 0.0)

        polylines, arrows = [], []
        if dim.curve2 is not None:
            drawn = self._two_line_layout(dim, offset, S, unit)
            if drawn is None:
                return None
            polylines, arrows, anchor = drawn
        elif dim.kind == "length":
            p0, p1 = np.asarray(c.p0, float), np.asarray(c.p1, float)
            d = p1 - p0
            L = float(np.hypot(*d))
            if L == 0.0:
                return None
            n = np.array([-d[1], d[0]]) / L
            side = unit(S(p0 + n * (1.0 if offset >= 0 else -1.0)) - S(p0))
            A, B = S(p0 + n * offset), S(p1 + n * offset)
            for P, Q in ((S(p0), A), (S(p1), B)):
                polylines.append([P + side * 3.0, Q + side * 6.0])
            polylines.append([A, B])
            along = unit(B - A)
            arrows += [(A, along * -1.0), (B, along)]
            anchor = (A + B) / 2.0 + side * 10.0
        else:
            centre, r, a0, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
            rd = max(r + offset, 0.05 * r)
            th = a0 + sweep * np.linspace(0.0, 1.0, 41)
            ring = [S(centre + rd * np.array([math.cos(t), math.sin(t)])) for t in th]
            polylines.append(ring)
            for t in (th[0], th[-1]):
                u = np.array([math.cos(t), math.sin(t)])
                radial = unit(S(centre + u * (rd + r)) - S(centre))
                polylines.append([S(centre + u * r), S(centre + u * rd) + radial * 6.0])
            arrows += [(ring[0], unit(ring[0] - ring[1])), (ring[-1], unit(ring[-1] - ring[-2]))]
            tm = a0 + sweep / 2.0
            um = np.array([math.cos(tm), math.sin(tm)])
            anchor = S(centre + um * rd) + unit(S(centre + um * (rd + r)) - S(centre)) * 12.0
        text = self.dimension_text(dim)
        box = QFontMetricsF(self.font()).boundingRect(text)
        rect = QRectF(0, 0, box.width() + 8, box.height() + 4)
        rect.moveCenter(anchor)
        return {"polylines": polylines, "arrows": arrows, "text": text, "rect": rect}

    def _two_line_layout(self, dim, offset, S, unit):
        """(polylines, arrows, label anchor) of a distance or angle between two lines, or None."""
        from ..model import dimensions
        a, b = self.model.doc.curves[dim.curve], self.model.doc.curves[dim.curve2]
        if a.type != "line" or b.type != "line" or a.p0 == a.p1 or b.p0 == b.p1:
            return None
        polylines = []

        def extend(line, base, ray, target):
            """Extension line from the nearer end of a line to target (base + ray * t)."""
            ts = [float((np.asarray(q, float) - base) @ ray) for q in (line.p0, line.p1)]
            t = float((target - base) @ ray)
            if min(ts) - 1e-12 <= t <= max(ts) + 1e-12:
                return                                   # the target is on the line itself
            P = S(line.p0 if abs(ts[0] - t) < abs(ts[1] - t) else line.p1)
            Q = S(target)
            out = unit(Q - P)
            polylines.append([P + out * 3.0, Q + out * 6.0])

        if dim.kind == "distance":
            u, n = dimensions.direction(a), dimensions.normal(a)
            p0 = np.asarray(a.p0, float)
            A = p0 + u * offset
            B = A + n * dimensions.signed_distance(a, b)
            extend(a, p0, u, A)
            extend(b, p0, u, B)
            SA, SB = S(A), S(B)
            polylines.append([SA, SB])
            along = unit(SB - SA)
            return polylines, [(SA, along * -1.0), (SB, along)], (SA + SB) / 2.0
        v = dimensions.vertex(a, b)
        if v is None:
            return None
        rd = offset if offset > 0 else self.px(30)
        r1, _ = dimensions.sector_rays(a, b, dim.sector)
        sweep = dimensions.signed_sector_angle(a, b, dim.sector)
        th = math.atan2(r1[1], r1[0]) + sweep * np.linspace(0.0, 1.0, 41)
        ring = [S(v + rd * np.array([math.cos(t), math.sin(t)])) for t in th]
        polylines.append(ring)
        for line, t in ((a, th[0]), (b, th[-1])):
            ray = np.array([math.cos(t), math.sin(t)])
            extend(line, v, ray, v + ray * rd)
        arrows = [(ring[0], unit(ring[0] - ring[1])), (ring[-1], unit(ring[-1] - ring[-2]))]
        tm = th[0] + sweep / 2.0
        um = np.array([math.cos(tm), math.sin(tm)])
        anchor = S(v + um * rd) + unit(S(v + um * 2.0 * rd) - S(v)) * 12.0
        return polylines, arrows, anchor

    def paint_dimension(self, p, dim, color, offset=None):
        layout = self.dimension_layout(dim, offset)
        if layout is None:
            return
        p.setPen(make_pen(color, 1.2))
        for line in layout["polylines"]:
            p.drawPolyline(QPolygonF(line))
        p.setBrush(QColor(color))
        for tip, d in layout["arrows"]:
            perp = QPointF(-d.y(), d.x())
            p.drawPolygon(QPolygonF([tip, tip - d * 9.0 + perp * 3.2, tip - d * 9.0 - perp * 3.2]))
        bg = QColor(COLORS["background"])
        bg.setAlpha(220)
        p.setPen(Qt.NoPen)
        p.setBrush(bg)
        p.drawRect(layout["rect"])
        p.setBrush(Qt.NoBrush)
        p.setPen(QColor(color))
        p.drawText(layout["rect"], Qt.AlignCenter, layout["text"])

    def pick_handle(self, screen):
        """(curve_id, handle name, point) of a selected curve's handle under the cursor."""
        best = None
        for cid in self.model.selection & set(self.model.doc.curves):
            for name, p in self.model.doc.curves[cid].handles():
                q = self.to_screen(p)
                d = math.hypot(q.x() - screen.x(), q.y() - screen.y())
                if d <= HANDLE_PX + 4 and (best is None or d < best[0]):
                    best = (d, (cid, name, p))
        return best[1] if best else None

    def curves_in_rect(self, a, b, crossing):
        """Curves inside the world rectangle a-b (window), or touching it (crossing)."""
        x0, x1 = sorted((a[0], b[0]))
        y0, y1 = sorted((a[1], b[1]))
        box = np.array([(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)])
        out = []
        for cid, c in self.model.doc.curves.items():
            P = geom.sample(c)
            inside = (P[:, 0] >= x0) & (P[:, 0] <= x1) & (P[:, 1] >= y0) & (P[:, 1] <= y1)
            if inside.all():
                out.append(cid)
            elif crossing and (inside.any() or geom.polyline_intersections(P, box)):
                out.append(cid)
        out.extend(pid for pid, (x, y) in self.model.doc.points.items()
                   if x0 <= x <= x1 and y0 <= y <= y1)
        return out

    # --- paths -------------------------------------------------------------------
    def curve_path(self, curve, cache=True):
        key = curve.geom_key()
        if cache and key in self._path_cache:
            return self._path_cache[key]
        path = QPainterPath()
        if curve.type == "line":
            path.moveTo(*curve.p0)
            path.lineTo(*curve.p1)
        elif curve.type == "spline":
            B = geom.spline_beziers(curve.points)
            path.moveTo(*B[0, 0])
            for seg in B:
                path.cubicTo(QPointF(*seg[1]), QPointF(*seg[2]), QPointF(*seg[3]))
        elif curve.type == "bezier" and len(curve.points) in (2, 3, 4):
            P = curve.points
            path.moveTo(*P[0])
            if len(P) == 2:
                path.lineTo(*P[1])
            elif len(P) == 3:
                path.quadTo(QPointF(*P[1]), QPointF(*P[2]))
            else:
                path.cubicTo(QPointF(*P[1]), QPointF(*P[2]), QPointF(*P[3]))
        else:
            pts = geom.sample(curve)
            path.moveTo(*pts[0])
            for q in pts[1:]:
                path.lineTo(*q)
        if cache:
            self._path_cache[key] = path
        return path

    def draw_curve(self, painter, curve, color, width=1.5, dashed=None, cache=False):
        if dashed is None:
            dashed = kind_style(curve.kind)
        painter.setPen(make_pen(color, width, dashed))
        painter.drawPath(self.curve_path(curve, cache))

    # --- painting ----------------------------------------------------------------
    def paintEvent(self, event):
        p = QPainter(self)
        try:
            self._paint(p)
        finally:
            p.end()          # an exception must not leave the painter active (Qt aborts)

    def _paint(self, p):
        p.setRenderHint(QPainter.Antialiasing)
        p.fillRect(self.rect(), QColor(COLORS["background"]))
        self._paint_grid(p)

        p.setTransform(self.transform())
        if self.show_regions:
            self._paint_regions(p)
        if self.show_blocks and self.model.doc.blocks:
            self._paint_block_fills(p)
            self._paint_hidden_outlines(p)
        if self.show_quality and self.quality is not None and self.mesh is not None:
            self._paint_quality(p)
        if self.show_mesh and self.mesh is not None:
            self._paint_mesh(p)
        sel = self.model.selection
        for cid, c in self.model.doc.curves.items():
            if cid in sel:
                color, width = COLORS["selected"], 2.5
            elif cid == self.hover_id:
                color, width = COLORS["hover"], 2.5
            elif c.bc_tag and c.kind == GEOMETRY and c.bc_tag in self.model.doc.bcs:
                color, width = tag_color(self.model.doc, c.bc_tag), 2.6
            else:
                color = {REFERENCE: COLORS["reference"], SPLIT: COLORS["split"]}.get(c.kind, COLORS["geometry"])
                width = 1.5
            self.draw_curve(p, c, color, width, cache=True)
        if self.tool is not None:
            self.tool.paint(p)

        p.resetTransform()
        if self.show_blocks and self.model.doc.blocks:
            self._paint_block_labels(p)
        self._paint_dimensions(p)
        self._paint_points(p)
        self._paint_handles(p)
        if self.tool is not None:
            self.tool.paint_overlay(p)
        self._paint_snap(p)
        self._paint_bc_legend(p)
        if self.show_quality and self.quality is not None:
            self._paint_quality_legend(p)

    # --- cell quality heat-map ------------------------------------------------------
    QUALITY_BUCKETS = 10

    def set_quality(self, quality):
        self.quality = quality
        self.update()

    def _quality_value(self, metrics, block):
        """0 (good) .. 1 (at or past the limit) per cell for the chosen metric, NaN if not judged."""
        if self.quality_metric == "skew":
            return metrics["skew"]
        limit = self.quality.thresholds["ar_warn"]
        if not block.check_ar:
            return np.full(metrics["ar"].shape, np.nan)        # thin by design (boundary layer)
        with np.errstate(divide="ignore", invalid="ignore"):
            return np.clip(np.log(metrics["ar"]) / np.log(limit), 0.0, 1.0)

    @staticmethod
    def _ramp(v):
        """Green -> yellow -> red."""
        stops = [(0.0, (46, 157, 87)), (0.5, (241, 196, 15)), (1.0, (212, 63, 58))]
        for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
            if v <= t1:
                f = (v - t0) / (t1 - t0)
                return QColor(*[round(a + f * (b - a)) for a, b in zip(c0, c1)], 150)
        return QColor(*stops[-1][1], 150)

    def _paint_quality(self, p):
        key = (id(self.quality), id(self.mesh), self.quality_metric, self.hidden_ids())
        if self._quality_paths[0] != key:
            n = self.QUALITY_BUCKETS
            paths = [QPainterPath() for _ in range(n + 2)]       # buckets, inverted, not judged
            for bid, xyz in self.visible_grids():
                m = self.quality.metrics.get(bid)
                block = self.quality.blocks.get(bid)
                if m is None or m["skew"].shape != (xyz.shape[0] - 1, xyz.shape[1] - 1):
                    continue
                v = self._quality_value(m, block)
                P = xyz[:, :, :2]
                for i in range(P.shape[0] - 1):
                    for j in range(P.shape[1] - 1):
                        if m["inverted"][i, j]:
                            k = n
                        elif np.isnan(v[i, j]):
                            k = n + 1
                        else:
                            k = min(int(v[i, j] * n), n - 1)
                        paths[k].addPolygon(QPolygonF([QPointF(*P[i, j]), QPointF(*P[i + 1, j]),
                                                       QPointF(*P[i + 1, j + 1]), QPointF(*P[i, j + 1])]))
            colours = [self._ramp((k + 0.5) / n) for k in range(n)] + [QColor(140, 30, 160, 200),
                                                                       QColor(160, 160, 160, 70)]
            self._quality_paths = (key, list(zip(paths, colours)))
        p.setPen(Qt.NoPen)
        for path, colour in self._quality_paths[1]:
            if not path.isEmpty():
                p.setBrush(colour)
                p.drawPath(path)
        p.setBrush(Qt.NoBrush)

    def _paint_quality_legend(self, p):
        t = self.quality.thresholds
        if self.quality_metric == "skew":
            title, lo, hi = "Skewness", "0", "1"
        else:
            title, lo, hi = "Aspect ratio", "1", f"{t['ar_warn']:g}+"
        w, h = 160, 10
        x, y = self.width() - w - 16, self.height() - 52
        bg = QColor(COLORS["background"])
        bg.setAlpha(220)
        p.setPen(Qt.NoPen)
        p.setBrush(bg)
        p.drawRect(QRectF(x - 8, y - 22, w + 16, 58))
        for k in range(w):
            p.setBrush(self._ramp(k / (w - 1)))
            p.drawRect(QRectF(x + k, y, 1.2, h))
        p.setBrush(Qt.NoBrush)
        p.setPen(QColor(COLORS["mesh"]))
        p.drawText(QPointF(x, y - 6), title + ("" if self.quality_metric == "skew"
                                               else "   (grey: boundary layer)"))
        p.drawText(QPointF(x, y + h + 14), lo)
        p.drawText(QPointF(x + w - QFontMetricsF(self.font()).horizontalAdvance(hi), y + h + 14), hi)
        p.setPen(Qt.NoPen)
        p.setBrush(QColor(140, 30, 160, 200))
        p.drawRect(QRectF(x + w / 2 - 34, y + h + 5, 9, 9))
        p.setBrush(Qt.NoBrush)
        p.setPen(QColor(COLORS["mesh"]))
        p.drawText(QPointF(x + w / 2 - 21, y + h + 14), "inverted")

    def _paint_bc_legend(self, p):
        doc = self.model.doc
        used = sorted({c.bc_tag for c in doc.curves.values()
                       if c.bc_tag in doc.bcs and c.kind == GEOMETRY})
        if not used:
            return
        rows = [(tag_color(doc, t), f"{t}   {doc.bcs[t]['class']}") for t in used]
        rows.append((COLORS["geometry"], "untagged   slip wall (default)"))
        fm = QFontMetricsF(self.font())
        h = fm.height() + 4
        w = max(fm.horizontalAdvance(text) for _, text in rows) + 34
        box = QRectF(10, self.height() - 10 - h * len(rows) - 8, w, h * len(rows) + 8)
        bg = QColor(COLORS["background"])
        bg.setAlpha(225)
        p.setPen(make_pen("#c4cad1", 1.0))
        p.setBrush(bg)
        p.drawRect(box)
        p.setBrush(Qt.NoBrush)
        for k, (color, text) in enumerate(rows):
            y = box.top() + 4 + h * k + h / 2
            p.setPen(make_pen(color, 3.0))
            p.drawLine(QPointF(box.left() + 8, y), QPointF(box.left() + 24, y))
            p.setPen(QColor(COLORS["geometry"]))
            p.drawText(QRectF(box.left() + 30, y - h / 2, w, h), Qt.AlignVCenter | Qt.AlignLeft, text)

    def _paint_grid(self, p):
        if not self.show_grid:
            return
        step = self.grid_step()
        mant = round(step / 10 ** math.floor(math.log10(step)))
        major = step * (2 if mant == 5 else 5)
        x0, y1 = self.to_world(QPointF(0, 0))
        x1, y0 = self.to_world(QPointF(self.width(), self.height()))
        for spacing, color in ((step, COLORS["grid_minor"]), (major, COLORS["grid_major"])):
            n = (x1 - x0) / spacing + (y1 - y0) / spacing
            if n > 600:
                continue
            p.setPen(QPen(QColor(color), 1))
            i = math.floor(x0 / spacing)
            while i * spacing <= x1:
                sx = self.to_screen((i * spacing, 0)).x()
                p.drawLine(QPointF(sx, 0), QPointF(sx, self.height()))
                i += 1
            j = math.floor(y0 / spacing)
            while j * spacing <= y1:
                sy = self.to_screen((0, j * spacing)).y()
                p.drawLine(QPointF(0, sy), QPointF(self.width(), sy))
                j += 1
        origin = self.to_screen((0.0, 0.0))
        p.setPen(QPen(QColor(COLORS["axes"]), 1))
        p.drawLine(QPointF(origin.x(), 0), QPointF(origin.x(), self.height()))
        p.drawLine(QPointF(0, origin.y()), QPointF(self.width(), origin.y()))

    def _paint_regions(self, p):
        p.setPen(Qt.NoPen)
        for k, r in enumerate(self.topology().regions):
            path = QPainterPath()
            path.setFillRule(Qt.OddEvenFill)
            for poly in [r.polygon] + list(r.hole_polygons):
                path.addPolygon(QPolygonF([QPointF(x, y) for x, y in poly]))
                path.closeSubpath()
            color = QColor(REGION_FILLS[k % len(REGION_FILLS)])
            color.setAlpha(40)
            p.setBrush(color)
            p.drawPath(path)
        p.setBrush(Qt.NoBrush)

    def _paint_block_fills(self, p):
        res = self.resolution()
        p.setPen(Qt.NoPen)
        for bid, poly in res.polygons.items():
            if bid in self.hidden_blocks:
                continue
            color = QColor(COLORS["block_error"] if bid in res.errors else COLORS["block"])
            color.setAlpha(55 if bid in res.errors else 28)
            p.setBrush(color)
            p.drawPolygon(QPolygonF([QPointF(x, y) for x, y in poly]))
        p.setBrush(Qt.NoBrush)

    # --- hidden blocks (view only) ----------------------------------------------------
    def hidden_ids(self):
        """Hidden block ids that still exist (stale ids are ignored), as a frozenset."""
        if not self.hidden_blocks:
            return frozenset()
        return frozenset(self.hidden_blocks & set(self.resolution().polygons))

    def visible_grids(self):
        """The (block id, xyz) mesh grids that are drawn."""
        hidden = self.hidden_ids()
        return [(b, g) for b, g in self.mesh.grids if b not in hidden] if self.mesh is not None else []

    def set_blocks_hidden(self, ids, hidden):
        ids = set(ids)
        self.hidden_blocks = (self.hidden_blocks | ids) if hidden else (self.hidden_blocks - ids)
        self.update()

    def all_block_ids(self):
        return list(self.resolution().polygons)

    def toggle_blocks_hidden(self, ids):
        """Hide the blocks, or show them if they are all hidden already."""
        ids = set(ids)
        self.set_blocks_hidden(ids, not ids <= self.hidden_ids())

    def block_visibility_menu(self, menu, bid):
        """Add the hide / show items for a click on block bid (None: outside any block)."""
        hidden = self.hidden_ids()
        if bid is not None and bid in hidden:
            menu.addAction("Show block", lambda: self.set_blocks_hidden({bid}, False))
        elif bid is not None:
            menu.addAction("Hide block", lambda: self.set_blocks_hidden({bid}, True))
            menu.addAction("Hide all blocks", lambda: self.set_blocks_hidden(self.all_block_ids(), True))
        if hidden:
            menu.addAction("Show all blocks", lambda: self.set_blocks_hidden(set(self.hidden_blocks), False))

    def _paint_hidden_outlines(self, p):
        hidden = self.hidden_ids()
        if not hidden:
            return
        res = self.resolution()
        color = QColor(COLORS["block"])
        color.setAlpha(110)
        p.setPen(make_pen(color.name(QColor.HexArgb), 1.0, dashed=True))
        for bid in hidden:
            poly = res.polygons[bid]
            if len(poly) >= 3:
                p.drawPolygon(QPolygonF([QPointF(x, y) for x, y in poly]))

    def _paint_mesh(self, p):
        key, path = self._mesh_path
        hidden = self.hidden_ids()
        if key is None or key[0] is not self.mesh or key[1] != hidden:
            path = QPainterPath()
            for _, g in self.visible_grids():
                for line in list(g[:, :, :2]) + list(g[:, :, :2].transpose(1, 0, 2)):
                    path.moveTo(*line[0])
                    for x, y in line[1:]:
                        path.lineTo(x, y)
            self._mesh_path = ((self.mesh, hidden), path)
        color = QColor(COLORS["mesh"])
        color.setAlpha(150 if self.mesh.source == "eilmer" else 90)
        pen = make_pen(color.name(QColor.HexArgb), 0.8)
        pen.setColor(color)
        p.setPen(pen)
        p.drawPath(path)

    def _paint_block_labels(self, p):
        from shapely.geometry import Polygon
        res = self.resolution()
        blocks = res.by_id()
        for bid, poly in res.polygons.items():
            if len(poly) < 3 or bid in self.hidden_blocks:
                continue
            c = Polygon(poly).representative_point()
            if bid in res.errors:
                text, color = f"{bid}: needs attention", COLORS["block_error"]
            else:
                b = blocks[bid]
                text, color = f"{bid}  {b.niv - 1} x {b.njv - 1}", COLORS["mesh"]
                if b.patch == "AOPatch":
                    text += "  AO"
                if b.smoothing:
                    text += "  smoothed"
                if bid in res.warnings:
                    text += "  (!)"
                    color = COLORS["block_error"]
            box = QFontMetricsF(self.font()).boundingRect(text)
            rect = QRectF(0, 0, box.width() + 8, box.height() + 4)
            rect.moveCenter(self.to_screen((c.x, c.y)))
            bg = QColor(COLORS["background"])
            bg.setAlpha(200)
            p.setPen(Qt.NoPen)
            p.setBrush(bg)
            p.drawRect(rect)
            p.setBrush(Qt.NoBrush)
            p.setPen(QColor(color))
            p.drawText(rect, Qt.AlignCenter, text)

    def _paint_dimensions(self, p):
        for did, dim in self.model.doc.dimensions.items():
            if did in self.model.selection:
                color = COLORS["selected"]
            elif did == self.hover_id:
                color = COLORS["hover"]
            else:
                color = COLORS["dimension"]
            self.paint_dimension(p, dim, color)

    def _paint_points(self, p):
        r = POINT_PX
        for pid, pt in self.model.doc.points.items():
            if pid in self.model.selection:
                color, width = COLORS["selected"], 2.2
            elif pid == self.hover_id:
                color, width = COLORS["hover"], 2.2
            else:
                color, width = COLORS["geometry"], 1.5
            p.setPen(make_pen(color, width))
            q = self.to_screen(pt)
            p.drawLine(q + QPointF(-r, 0), q + QPointF(r, 0))
            p.drawLine(q + QPointF(0, -r), q + QPointF(0, r))

    def _paint_handles(self, p):
        if not self.model.selection & set(self.model.doc.curves):
            return
        p.setPen(make_pen(COLORS["selected"], 1.2))
        centres = {tuple(np.round(self.model.doc.curves[cid].centre, 12))
                   for cid in self.model.selection & set(self.model.doc.curves)
                   if self.model.doc.curves[cid].type == "arc"}
        for centre in centres:                       # selected arcs and circles show their centre
            q = self.to_screen(centre)
            p.drawEllipse(q, 3.0, 3.0)
            p.drawLine(q + QPointF(-7, 0), q + QPointF(7, 0))
            p.drawLine(q + QPointF(0, -7), q + QPointF(0, 7))
        p.setBrush(QColor(COLORS["handle"]))
        h = HANDLE_PX
        for cid in self.model.selection & set(self.model.doc.curves):
            for name, pt in self.model.doc.curves[cid].handles():
                q = self.to_screen(pt)
                if name == "mid":
                    p.drawPolygon(QPolygonF([q + QPointF(0, -h-1), q + QPointF(h+1, 0),
                                             q + QPointF(0, h+1), q + QPointF(-h-1, 0)]))
                elif name.startswith("i:"):      # interior spline/Bezier point
                    p.drawEllipse(q, h - 1.5, h - 1.5)
                else:
                    p.drawRect(QRectF(q.x() - h, q.y() - h, 2*h, 2*h))
        p.setBrush(Qt.NoBrush)

    def _paint_snap(self, p):
        s = self.last_snap
        if s is None or not s.kind or (self.tool is not None and not self.tool.snaps):
            return
        q = self.to_screen(s.point)
        r = 6.0
        p.setPen(make_pen(COLORS["snap"], 1.6))
        p.setBrush(Qt.NoBrush)
        kind = s.kind
        if kind in (snapping.ENDPOINT, snapping.POINT):
            p.drawRect(QRectF(q.x() - r, q.y() - r, 2*r, 2*r))
        elif kind == snapping.INTERSECTION:
            p.drawLine(q + QPointF(-r, -r), q + QPointF(r, r))
            p.drawLine(q + QPointF(-r, r), q + QPointF(r, -r))
        elif kind == snapping.CENTRE:
            p.drawEllipse(q, r, r)
            p.drawLine(q + QPointF(-r/2, 0), q + QPointF(r/2, 0))
            p.drawLine(q + QPointF(0, -r/2), q + QPointF(0, r/2))
        elif kind == snapping.MIDPOINT:
            p.drawPolygon(QPolygonF([q + QPointF(0, -r), q + QPointF(r, r*0.8), q + QPointF(-r, r*0.8)]))
        elif kind == snapping.ON_CURVE:
            p.drawEllipse(q, r*0.6, r*0.6)
        elif kind in (snapping.HORIZONTAL, snapping.VERTICAL):
            anchor = self.tool.anchor() if self.tool else None
            if anchor is not None:
                p.setPen(make_pen(COLORS["snap"], 1.0, dashed=True))
                p.drawLine(self.to_screen(anchor), q)
                p.setPen(make_pen(COLORS["snap"], 1.6))
            off = QPointF(r, -r)
            if kind == snapping.HORIZONTAL:
                p.drawLine(q + off + QPointF(-4, 0), q + off + QPointF(4, 0))
            else:
                p.drawLine(q + off + QPointF(0, -4), q + off + QPointF(0, 4))
        elif kind == snapping.GRID:
            p.drawLine(q + QPointF(-r/2, 0), q + QPointF(r/2, 0))
            p.drawLine(q + QPointF(0, -r/2), q + QPointF(0, r/2))

    # --- events ------------------------------------------------------------------
    def _tool_event(self, ev, button=Qt.NoButton):
        pos = ev.position()
        world = self.to_world(pos)
        tool = self.tool
        if tool is not None and tool.snaps:
            anchor = tool.anchor()
            snap = self.snap(world, anchor, tool.snap_exclude())
            snap = self._apply_hold(snap, anchor)
        else:
            snap = Snap(world)
        self.last_snap = snap
        self.last_screen = pos
        return ToolEvent(world, snap, pos, button, ev.modifiers(), Qt.LeftButton in self._pressed)

    def _apply_hold(self, snap, anchor):
        """Held H/V keys force the point level with or plumb below the anchor."""
        if anchor is None:
            return snap
        x, y = snap.point
        if self.hold_keys["horizontal"] in self.held:
            return Snap((x, anchor[1]), snapping.HORIZONTAL, snap.curve_id)
        if self.hold_keys["vertical"] in self.held:
            return Snap((anchor[0], y), snapping.VERTICAL, snap.curve_id)
        return snap

    def mouseDoubleClickEvent(self, ev):
        if ev.button() != Qt.LeftButton:
            return
        te = self._tool_event(ev, ev.button())
        if self.tool is not None:
            self.tool.double_click(te)
        self.update()

    def mousePressEvent(self, ev):
        self.setFocus()
        if ev.button() == Qt.MiddleButton:
            self._pan = (ev.position(), self.cx, self.cy)
            self.setCursor(Qt.ClosedHandCursor)
            return
        self._pressed.add(ev.button())
        te = self._tool_event(ev, ev.button())
        if self.tool is not None:
            if ev.button() == Qt.LeftButton:
                self.tool.press(te)
            elif ev.button() == Qt.RightButton:
                self.tool.right_click(te)
        self.toolStateChanged.emit()
        self.update()

    def mouseMoveEvent(self, ev):
        if self._pan is not None:
            start, cx, cy = self._pan
            d = ev.position() - start
            self.cx = cx - d.x() / self.scale
            self.cy = cy + d.y() / self.scale
            self.update()
            return
        te = self._tool_event(ev)
        if self.tool is not None:
            self.tool.move(te)
            if self.tool.hover_curves and not te.dragging:
                self.hover_id = self.pick_entity(te.screen, te.world)
            else:
                self.hover_id = None
        self.cursorMoved.emit(te.snap)
        self.update()

    def mouseReleaseEvent(self, ev):
        if ev.button() == Qt.MiddleButton:
            self._pan = None
            self.setCursor(Qt.CrossCursor)
            return
        te = self._tool_event(ev, ev.button())
        self._pressed.discard(ev.button())
        if self.tool is not None and ev.button() == Qt.LeftButton:
            self.tool.release(te)
        self.toolStateChanged.emit()
        self.update()

    def wheelEvent(self, ev):
        steps = ev.angleDelta().y() / 120.0
        if steps:
            self.zoom_at(1.2 ** steps, ev.position())

    def leaveEvent(self, ev):
        self.last_snap = None
        self.hover_id = None
        self.update()

    def event(self, ev):
        # Let the active tool claim keys that are also global tool shortcuts.
        if ev.type() == QEvent.ShortcutOverride and (
                ev.key() in self.hold_keys.values()
                or (self.tool is not None and self.tool.claims_key(ev.key()))):
            ev.accept()
            return True
        return super().event(ev)

    def keyPressEvent(self, ev):
        key = ev.key()
        if (key in self.hold_keys.values() and not (ev.modifiers() & Qt.ControlModifier)
                and not (self.tool is not None and self.tool.claims_hold(key))):
            if not ev.isAutoRepeat():
                self.held.add(key)
                self._refresh_cursor()
            return
        if self.tool is not None and self.tool.key(key):
            self.toolStateChanged.emit()
            self.update()
            return
        if key == Qt.Key_Escape:
            if self.tool is None or not self.tool.cancel():
                self.escapeIdle.emit()
            self.toolStateChanged.emit()
            self.update()
            return
        if key in (Qt.Key_Return, Qt.Key_Enter):
            if self.tool is not None:
                self.tool.finish()
                self.toolStateChanged.emit()
                self.update()
            return
        if key in (Qt.Key_Delete, Qt.Key_Backspace):
            self.model.delete_selection()
            return
        text = ev.text()
        if text and text in "0123456789.-+@<," and not (ev.modifiers() & Qt.ControlModifier):
            self.typedText.emit(text)
            return
        super().keyPressEvent(ev)

    def keyReleaseEvent(self, ev):
        if ev.key() in self.held and not ev.isAutoRepeat():
            self.held.discard(ev.key())
            self._refresh_cursor()
            return
        super().keyReleaseEvent(ev)

    def focusOutEvent(self, ev):
        self.held.clear()        # never leave a constraint stuck on
        super().focusOutEvent(ev)

    def _refresh_cursor(self):
        """Re-run the move handling at the last cursor position (e.g. a hold key changed)."""
        if self.last_screen is None or self.tool is None:
            self.update()
            return
        world = self.to_world(self.last_screen)
        anchor = self.tool.anchor() if self.tool.snaps else None
        snap = self.snap(world, anchor, self.tool.snap_exclude()) if self.tool.snaps else Snap(world)
        snap = self._apply_hold(snap, anchor)
        self.last_snap = snap
        self.tool.move(ToolEvent(world, snap, self.last_screen, Qt.NoButton, Qt.NoModifier,
                                 Qt.LeftButton in self._pressed))
        self.cursorMoved.emit(snap)
        self.update()

    def preview_text(self, text):
        """Live typed value from the input box (e.g. a length lock while drawing)."""
        if self.tool is not None:
            self.tool.preview_text(text)
            self.toolStateChanged.emit()
            self.update()

    # --- typed input --------------------------------------------------------------
    def submit_text(self, text):
        """Pass typed input to the tool; returns an error message or None."""
        if self.tool is None:
            return "no active tool"
        try:
            err = self.tool.accept_text(text)
        except ValueError as e:
            err = str(e)
        self.toolStateChanged.emit()
        self.update()
        return err
