"""Drawing tools: line, profile, arc, circle, rectangle, triangle, Bezier, spline."""
import math

from PySide6.QtCore import Qt

from ...model import geom
from ...model.entities import BezierSeg, SplineSeg
from ..canvas import COLORS
from ..typed_input import single_number
from .base import Tool


def _unit(v):
    n = math.hypot(v[0], v[1])
    return (v[0] / n, v[1] / n) if n > 0 else (1.0, 0.0)


class LineTool(Tool):
    name, label = "line", "Line"

    def reset(self):
        self.p0 = None

    def busy(self):
        return self.p0 is not None

    def anchor(self):
        return self.p0

    def click(self, point):
        if self.p0 is None:
            self.p0 = point
        elif self.distinct(self.p0, point):
            self.add([self.line(self.p0, point)], "Line")
            self.p0 = None

    def accept_text(self, text):
        length = single_number(text)
        if length is not None and self.p0 is not None:
            d = _unit((self.cursor[0] - self.p0[0], self.cursor[1] - self.p0[1])) if self.cursor else (1.0, 0.0)
            self.click((self.p0[0] + length*d[0], self.p0[1] + length*d[1]))
            return None
        return super().accept_text(text)

    def prompt(self):
        return "Start point" if self.p0 is None else "End point or length"

    def paint(self, painter):
        if self.p0 is not None and self.cursor is not None and self.distinct(self.p0, self.cursor):
            self.draw_preview(painter, [self.line(self.p0, self.cursor)])


class ProfileTool(Tool):
    """Chain of lines and tangent arcs. Drag from the last point (or press Space) for an arc."""
    name, label = "profile", "Profile"
    DRAG_PX = 8

    def reset(self):
        self.start = None
        self.last = None
        self.tangent = None
        self.arc_mode = False
        self._press_screen = None
        self._drag_arc = False

    def busy(self):
        return self.last is not None

    def anchor(self):
        return self.last

    def press(self, ev):
        self._press_screen = ev.screen
        self._drag_arc = False

    def move(self, ev):
        super().move(ev)
        if ev.dragging and self.last is not None and self.tangent is not None \
                and self._press_screen is not None:
            at_last = self.canvas.to_screen(self.last)
            near_last = math.hypot(self._press_screen.x() - at_last.x(),
                                   self._press_screen.y() - at_last.y()) <= 10
            moved = math.hypot(ev.screen.x() - self._press_screen.x(),
                               ev.screen.y() - self._press_screen.y()) > self.DRAG_PX
            self._drag_arc = near_last and moved

    def release(self, ev):
        self._press_screen = None
        if self._drag_arc:
            self._drag_arc = False
            self._add_arc(ev.point)
        else:
            self.click(ev.point)

    def click(self, point):
        if self.last is None:
            self.start = self.last = point
        elif self.arc_mode and self.tangent is not None:
            self._add_arc(point)
        else:
            self._add_line(point)

    def _add_line(self, point):
        if not self.distinct(self.last, point):
            return
        self.add([self.line(self.last, point)], "Profile line")
        self.tangent = _unit((point[0] - self.last[0], point[1] - self.last[1]))
        self._advance(point)

    def _add_arc(self, point):
        if not self.distinct(self.last, point):
            return
        pieces = geom.tangent_arc(self.last, self.tangent, point)
        if pieces is None:
            self._add_line(point)
            return
        curves = self.arcs(pieces)
        self.add(curves, "Profile arc")
        self.tangent = tuple(geom.end_tangent(curves[-1]))
        self.arc_mode = False
        self._advance(point)

    def _advance(self, point):
        self.last = point
        if not self.distinct(point, self.start):
            self.reset()          # closed the loop

    def claims_key(self, key):
        return key == Qt.Key_Space

    def key(self, key):
        if key == Qt.Key_Space and self.tangent is not None:
            self.arc_mode = not self.arc_mode
            return True
        return False

    def _preview_curves(self):
        if self.last is None or self.cursor is None or not self.distinct(self.last, self.cursor):
            return []
        if (self.arc_mode or self._drag_arc) and self.tangent is not None:
            pieces = geom.tangent_arc(self.last, self.tangent, self.cursor)
            if pieces is not None:
                return self.arcs(pieces)
        return [self.line(self.last, self.cursor)]

    def prompt(self):
        if self.last is None:
            return "Start point"
        return "Next point" + (" (arc)" if self.arc_mode else "")

    def hint(self):
        return "Click to add lines. Drag from the last point or press Space for a tangent arc. Right-click or Esc to finish."

    def paint(self, painter):
        self.draw_preview(painter, self._preview_curves())


class ArcTool(Tool):
    """Three-point arc (start, end, point on arc) or centre-start-end (press M to switch)."""
    name, label = "arc", "Arc"

    def __init__(self, canvas):
        self.mode = "3pt"
        self.ccw = True
        super().__init__(canvas)

    def reset(self):
        self.pts = []

    def busy(self):
        return bool(self.pts)

    def anchor(self):
        return self.pts[-1] if self.pts else None

    def _pieces(self, point):
        if self.mode == "3pt" and len(self.pts) == 2:
            return geom.arcs_through_3_points(self.pts[0], point, self.pts[1])
        if self.mode == "centre" and len(self.pts) == 2:
            return geom.arcs_centre_start_end(self.pts[0], self.pts[1], point, self.ccw)
        return None

    def click(self, point):
        if self.pts and not self.distinct(self.pts[-1], point):
            return
        if len(self.pts) < 2:
            self.pts.append(point)
            return
        pieces = self._pieces(point)
        if pieces is not None:
            self.add(self.arcs(pieces), "Arc")
            self.reset()

    def claims_key(self, key):
        return key in (Qt.Key_M, Qt.Key_D)

    def key(self, key):
        if key == Qt.Key_M:
            self.mode = "centre" if self.mode == "3pt" else "3pt"
            self.reset()
            return True
        if key == Qt.Key_D:
            self.ccw = not self.ccw
            return True
        return False

    def prompt(self):
        if self.mode == "3pt":
            return ["Arc start", "Arc end", "Point on arc"][len(self.pts)]
        return ["Arc centre", "Arc start", "Arc end"][len(self.pts)]

    def hint(self):
        mode = "3-point" if self.mode == "3pt" else "centre-start-end (D flips direction)"
        return f"Arc: {mode}. Press M to switch mode."

    def paint(self, painter):
        if not self.pts or self.cursor is None:
            return
        if len(self.pts) == 1:
            if self.distinct(self.pts[0], self.cursor):
                self.canvas.draw_curve(painter, self.line(self.pts[0], self.cursor),
                                       COLORS["preview"], 1.0, dashed=True)
            return
        pieces = self._pieces(self.cursor)
        if pieces is not None:
            self.draw_preview(painter, self.arcs(pieces))


class CircleTool(Tool):
    name, label = "circle", "Circle"

    def reset(self):
        self.centre = None

    def busy(self):
        return self.centre is not None

    def anchor(self):
        return self.centre

    def _circle(self, r):
        return self.arcs(geom.circle_arcs(self.centre, r))

    def click(self, point):
        if self.centre is None:
            self.centre = point
            return
        r = math.hypot(point[0] - self.centre[0], point[1] - self.centre[1])
        if r > self.canvas.px(1):
            self.add(self._circle(r), "Circle")
            self.reset()

    def accept_text(self, text):
        r = single_number(text)
        if r is not None and self.centre is not None:
            if r <= 0:
                return "radius must be positive"
            self.add(self._circle(r), "Circle")
            self.reset()
            return None
        return super().accept_text(text)

    def prompt(self):
        return "Centre" if self.centre is None else "Radius or point on circle"

    def paint(self, painter):
        if self.centre is not None and self.cursor is not None:
            r = math.hypot(self.cursor[0] - self.centre[0], self.cursor[1] - self.centre[1])
            if r > self.canvas.px(1):
                self.draw_preview(painter, self._circle(r))


def _rect_lines(tool, a, b):
    x0, x1 = sorted((a[0], b[0]))
    y0, y1 = sorted((a[1], b[1]))
    c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    return [tool.line(c[k], c[(k + 1) % 4]) for k in range(4)]


class RectangleTool(Tool):
    name, label = "rectangle", "Rectangle"

    def reset(self):
        self.p0 = None

    def busy(self):
        return self.p0 is not None

    def anchor(self):
        return self.p0

    def click(self, point):
        if self.p0 is None:
            self.p0 = point
        elif abs(point[0] - self.p0[0]) > self.canvas.px(1) and abs(point[1] - self.p0[1]) > self.canvas.px(1):
            self.add(_rect_lines(self, self.p0, point), "Rectangle")
            self.reset()

    def prompt(self):
        return "First corner" if self.p0 is None else "Opposite corner (@width, height)"

    def paint(self, painter):
        if self.p0 is not None and self.cursor is not None \
                and abs(self.cursor[0] - self.p0[0]) > 0 and abs(self.cursor[1] - self.p0[1]) > 0:
            self.draw_preview(painter, _rect_lines(self, self.p0, self.cursor))


class TriangleTool(Tool):
    name, label = "triangle", "Triangle"

    def reset(self):
        self.pts = []

    def busy(self):
        return bool(self.pts)

    def anchor(self):
        return self.pts[-1] if self.pts else None

    def click(self, point):
        if self.pts and not self.distinct(self.pts[-1], point):
            return
        self.pts.append(point)
        if len(self.pts) == 3:
            (ax, ay), (bx, by), (cx, cy) = self.pts
            if abs((bx - ax)*(cy - ay) - (by - ay)*(cx - ax)) > self.canvas.px(1) ** 2:
                self.add([self.line(self.pts[k], self.pts[(k + 1) % 3]) for k in range(3)], "Triangle")
            self.reset()

    def prompt(self):
        return ["First vertex", "Second vertex", "Third vertex"][len(self.pts)]

    def paint(self, painter):
        if not self.pts or self.cursor is None:
            return
        pts = self.pts + [self.cursor]
        lines = [self.line(pts[k], pts[k + 1]) for k in range(len(pts) - 1) if self.distinct(pts[k], pts[k + 1])]
        if len(pts) == 3 and self.distinct(pts[-1], pts[0]):
            lines.append(self.line(pts[-1], pts[0]))
        self.draw_preview(painter, lines)


class _PointListTool(Tool):
    """Collects points by clicking; right-click or Enter creates the curve."""
    curve_cls = None
    min_points = 2

    def reset(self):
        self.pts = []

    def busy(self):
        return bool(self.pts)

    def anchor(self):
        return self.pts[-1] if self.pts else None

    def click(self, point):
        if not self.pts or self.distinct(self.pts[-1], point):
            self.pts.append(point)

    def make(self, pts):
        return self.curve_cls(None, kind=self.model.new_kind(), points=list(pts))

    def finish(self):
        if len(self.pts) >= self.min_points:
            self.add([self.make(self.pts)], self.label)
        self.reset()

    def prompt(self):
        return f"Point {len(self.pts) + 1} (right-click or Enter to finish)"

    def paint(self, painter):
        if not self.pts:
            return
        pts = list(self.pts)
        if self.cursor is not None and self.distinct(pts[-1], self.cursor):
            pts.append(self.cursor)
        if len(pts) >= 2:
            self.draw_preview(painter, [self.make(pts)])
        self.paint_points(painter, pts)

    def paint_points(self, painter, pts):
        pass


class BezierTool(_PointListTool):
    name, label = "bezier", "Bezier"
    curve_cls = BezierSeg

    def hint(self):
        return "Click control points; the curve passes through the first and last. Right-click to finish."

    def paint_points(self, painter, pts):
        for k in range(len(pts) - 1):
            self.canvas.draw_curve(painter, self.line(pts[k], pts[k + 1]), COLORS["axes"], 1.0, dashed=True)


class SplineTool(_PointListTool):
    name, label = "spline", "Spline"
    curve_cls = SplineSeg

    def hint(self):
        return "Click points for the spline to pass through. Right-click to finish."
