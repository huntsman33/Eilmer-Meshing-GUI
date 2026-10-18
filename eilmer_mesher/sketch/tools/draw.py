"""
Drawing tools: point, line, profile, arc, circle, rectangle, triangle, Bezier, spline.

Typing a number while drawing locks a value live (line length, circle radius,
arc radius then sweep angle); the next click or Enter places the point with it.
"""
import math

from PySide6.QtCore import Qt

from ...model import geom
from ...model.entities import BezierSeg, SplineSeg
from ..canvas import COLORS
from ..typed_input import single_number
from .base import Tool

TWO_PI = 2.0 * math.pi


def _unit(v):
    n = math.hypot(v[0], v[1])
    return (v[0] / n, v[1] / n) if n > 0 else (1.0, 0.0)


def _toward(origin, point, length):
    d = _unit((point[0] - origin[0], point[1] - origin[1]))
    return (origin[0] + length * d[0], origin[1] + length * d[1])


class PointTool(Tool):
    """Construction point: a snap target drawn as '+'. Double-click it (Select) to edit."""
    name, label = "point", "Point"

    def click(self, point):
        self.model.add_point(point)

    def prompt(self):
        return "Point (x, y)"


class LineTool(Tool):
    name, label = "line", "Line"

    def reset(self):
        self.p0 = None
        self.lock = None

    def busy(self):
        return self.p0 is not None

    def anchor(self):
        return self.p0

    def target(self, point):
        """End point for the cursor, honouring a typed length."""
        if self.p0 is None or self.lock is None:
            return point
        return _toward(self.p0, point, self.lock)

    def click(self, point):
        if self.p0 is None:
            self.p0 = point
            return
        end = self.target(point)
        if self.distinct(self.p0, end):
            self.add([self.line(self.p0, end)], "Line")
            self.p0 = None
            if self.lock is not None:
                self.lock = None
                self.consume_typed()

    def preview_text(self, text):
        self.lock = self.canvas.parse_length(text) if self.p0 is not None else None
        if self.lock is not None and self.lock <= 0:
            self.lock = None

    def accept_text(self, text):
        length = self.canvas.parse_length(text)
        if length is not None and self.p0 is not None:
            if length <= 0:
                return "length must be positive"
            self.lock = length
            cursor = self.cursor if self.cursor and self.distinct(self.cursor, self.p0) else \
                (self.p0[0] + 1.0, self.p0[1])
            self.click(cursor)
            return None
        return super().accept_text(text)

    def prompt(self):
        if self.p0 is None:
            return "Start point"
        if self.lock is not None:
            return f"Direction (length {self.canvas.units.format(self.lock)})"
        return "End point, or type a length"

    def paint(self, painter):
        if self.p0 is not None and self.cursor is not None:
            end = self.target(self.cursor)
            if self.distinct(self.p0, end):
                self.draw_preview(painter, [self.line(self.p0, end)])


class ProfileTool(Tool):
    """Chain of lines and tangent arcs. Drag from the last point (or press A) for an arc."""
    name, label = "profile", "Profile"
    DRAG_PX = 8

    def reset(self):
        self.start = None
        self.last = None
        self.tangent = None
        self.arc_mode = False
        self.lock = None
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

    def _line_end(self, point):
        return point if self.lock is None else _toward(self.last, point, self.lock)

    def _add_line(self, point):
        end = self._line_end(point)
        if not self.distinct(self.last, end):
            return
        self.add([self.line(self.last, end)], "Profile line")
        self.tangent = _unit((end[0] - self.last[0], end[1] - self.last[1]))
        if self.lock is not None:
            self.lock = None
            self.consume_typed()
        self._advance(end)

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
        return key == Qt.Key_A and self.busy()

    def key(self, key):
        if key == Qt.Key_A and self.tangent is not None:
            self.arc_mode = not self.arc_mode
            return True
        return False

    def preview_text(self, text):
        ok = self.last is not None and not self.arc_mode
        self.lock = self.canvas.parse_length(text) if ok else None
        if self.lock is not None and self.lock <= 0:
            self.lock = None

    def accept_text(self, text):
        length = self.canvas.parse_length(text)
        if length is not None and self.last is not None and not self.arc_mode:
            if length <= 0:
                return "length must be positive"
            self.lock = length
            cursor = self.cursor if self.cursor and self.distinct(self.cursor, self.last) else \
                (self.last[0] + (self.tangent or (1.0, 0.0))[0], self.last[1] + (self.tangent or (1.0, 0.0))[1])
            self._add_line(cursor)
            return None
        return super().accept_text(text)

    def _preview_curves(self):
        if self.last is None or self.cursor is None or not self.distinct(self.last, self.cursor):
            return []
        if (self.arc_mode or self._drag_arc) and self.tangent is not None:
            pieces = geom.tangent_arc(self.last, self.tangent, self.cursor)
            if pieces is not None:
                return self.arcs(pieces)
        end = self._line_end(self.cursor)
        return [self.line(self.last, end)] if self.distinct(self.last, end) else []

    def prompt(self):
        if self.last is None:
            return "Start point"
        if self.arc_mode:
            return "Arc end point"
        if self.lock is not None:
            return f"Direction (length {self.canvas.units.format(self.lock)})"
        return "Next point, or type a length"

    def hint(self):
        return ("Click to add lines; type a number to fix the length. Drag from the last point or press A "
                "for a tangent arc. Right-click or Esc to finish.")

    def paint(self, painter):
        self.draw_preview(painter, self._preview_curves())


class ArcTool(Tool):
    """
    Centre mode (default): click the centre, type a radius (optional), click the
    start, type a sweep angle (optional), click the end. The sweep follows the
    cursor round from the start, so arcs past 180 degrees just work.
    3-point mode (press M): start, end, then a point on the arc.
    """
    name, label = "arc", "Arc"

    def __init__(self, canvas):
        self.mode = "centre"
        super().__init__(canvas)

    def reset(self):
        self.pts = []
        self.radius_lock = None
        self.sweep_lock = None      # radians, magnitude (sign follows the cursor)
        self._sweep = 0.0           # accumulated signed sweep following the cursor
        self._last_angle = None

    def busy(self):
        return bool(self.pts)

    def anchor(self):
        return self.pts[-1] if self.pts else None

    # --- centre mode helpers -------------------------------------------------------
    def _radius(self, point):
        c = self.pts[0]
        return self.radius_lock if self.radius_lock is not None else math.dist(c, point)

    def _angle(self, p):
        c = self.pts[0]
        return math.atan2(p[1] - c[1], p[0] - c[0])

    def _current_sweep(self):
        sweep = self._sweep
        if self.sweep_lock is not None:
            sweep = math.copysign(self.sweep_lock, sweep if sweep != 0.0 else 1.0)
        return max(-TWO_PI + 1e-9, min(TWO_PI - 1e-9, sweep))

    def _centre_pieces(self):
        c, start = self.pts
        sweep = self._current_sweep()
        if abs(sweep) < 1e-9:
            return None
        r = math.dist(c, start)
        return geom.arc_pieces(c, r, self._angle(start), sweep, start, None)

    def move(self, ev):
        super().move(ev)
        if self.mode == "centre" and len(self.pts) == 2:
            a = self._angle(ev.point)
            if self._last_angle is not None:
                d = (a - self._last_angle + math.pi) % TWO_PI - math.pi
                self._sweep += d
            self._last_angle = a

    # --- input ------------------------------------------------------------------------
    def click(self, point):
        if self.pts and not self.distinct(self.pts[-1], point):
            return
        if self.mode == "3pt":
            if len(self.pts) < 2:
                self.pts.append(point)
                return
            pieces = geom.arcs_through_3_points(self.pts[0], point, self.pts[1])
            if pieces is not None:
                self.add(self.arcs(pieces), "Arc")
                self.reset()
            return

        if not self.pts:                                  # centre
            self.pts.append(point)
        elif len(self.pts) == 1:                          # start, at the (typed) radius
            r = self._radius(point)
            if r <= self.canvas.px(1):
                return
            self.pts.append(_toward(self.pts[0], point, r))
            self._sweep = 0.0
            self._last_angle = self._angle(self.pts[1])
            if self.radius_lock is not None:
                self.radius_lock = None
                self.consume_typed()
        else:                                             # end
            pieces = self._centre_pieces()
            if pieces is not None:
                self.add(self.arcs(pieces), "Arc")
                if self.sweep_lock is not None:
                    self.consume_typed()
                self.reset()

    def preview_text(self, text):
        self.radius_lock = self.sweep_lock = None
        if self.mode != "centre":
            return
        if len(self.pts) == 1:
            r = self.canvas.parse_length(text)
            self.radius_lock = r if r is not None and r > 0 else None
        elif len(self.pts) == 2:
            deg = single_number(text)
            if deg is not None and 0 < abs(deg) < 360:
                self.sweep_lock = math.radians(abs(deg))
                if deg < 0 and self._sweep == 0.0:
                    self._sweep = -1e-12          # a negative typed angle means clockwise

    def accept_text(self, text):
        if self.mode == "centre" and len(self.pts) in (1, 2):
            self.preview_text(text)
            locked = self.radius_lock if len(self.pts) == 1 else self.sweep_lock
            if locked is not None:
                if len(self.pts) == 1:
                    cursor = self.cursor if self.cursor and self.distinct(self.cursor, self.pts[0]) else \
                        (self.pts[0][0] + 1.0, self.pts[0][1])
                    self.click(cursor)
                else:
                    self.click(self.cursor or self.pts[1])
                return None
        return super().accept_text(text)

    def claims_key(self, key):
        return key == Qt.Key_M

    def key(self, key):
        if key == Qt.Key_M:
            self.mode = "3pt" if self.mode == "centre" else "centre"
            self.reset()
            return True
        return False

    def prompt(self):
        if self.mode == "3pt":
            return ["Arc start", "Arc end", "Point on arc"][len(self.pts)]
        if not self.pts:
            return "Arc centre"
        if len(self.pts) == 1:
            if self.radius_lock is not None:
                return f"Start direction (radius {self.canvas.units.format(self.radius_lock)})"
            return "Start point, or type a radius"
        if self.sweep_lock is not None:
            return f"End side (sweep {math.degrees(self.sweep_lock):.6g} deg)"
        return "End point, or type a sweep angle (deg)"

    def hint(self):
        if self.mode == "3pt":
            return "Arc, 3-point: start, end, then a point on the arc. M switches to centre mode."
        return ("Arc: centre, then start (type a radius first to fix it), then end (type an angle to fix "
                "the sweep). Move round the centre past 180 degrees for larger arcs. M switches to 3-point.")

    def paint(self, painter):
        if not self.pts or self.cursor is None:
            return
        if self.mode == "3pt":
            if len(self.pts) == 1:
                if self.distinct(self.pts[0], self.cursor):
                    self.canvas.draw_curve(painter, self.line(self.pts[0], self.cursor),
                                           COLORS["preview"], 1.0, dashed=True)
                return
            pieces = geom.arcs_through_3_points(self.pts[0], self.cursor, self.pts[1])
            if pieces is not None:
                self.draw_preview(painter, self.arcs(pieces))
            return
        c = self.pts[0]
        if len(self.pts) == 1:
            r = self._radius(self.cursor)
            if r > self.canvas.px(1):
                guide = self.arcs(geom.circle_arcs(c, r))
                for a in guide:
                    self.canvas.draw_curve(painter, a, COLORS["axes"], 1.0, dashed=True)
                self.canvas.draw_curve(painter, self.line(c, _toward(c, self.cursor, r)),
                                       COLORS["preview"], 1.0, dashed=True)
            return
        self.canvas.draw_curve(painter, self.line(c, self.pts[1]), COLORS["axes"], 1.0, dashed=True)
        pieces = self._centre_pieces()
        if pieces is not None:
            self.draw_preview(painter, self.arcs(pieces))


class CircleTool(Tool):
    name, label = "circle", "Circle"

    def reset(self):
        self.centre = None
        self.lock = None

    def busy(self):
        return self.centre is not None

    def anchor(self):
        return self.centre

    def _circle(self, r):
        return self.arcs(geom.circle_arcs(self.centre, r))

    def _radius(self, point):
        return self.lock if self.lock is not None else math.dist(self.centre, point)

    def click(self, point):
        if self.centre is None:
            self.centre = point
            return
        r = self._radius(point)
        if r > self.canvas.px(1):
            self.add(self._circle(r), "Circle")
            if self.lock is not None:
                self.consume_typed()
            self.reset()

    def preview_text(self, text):
        r = self.canvas.parse_length(text) if self.centre is not None else None
        self.lock = r if r is not None and r > 0 else None

    def accept_text(self, text):
        r = self.canvas.parse_length(text)
        if r is not None and self.centre is not None:
            if r <= 0:
                return "radius must be positive"
            self.add(self._circle(r), "Circle")
            self.reset()
            return None
        return super().accept_text(text)

    def prompt(self):
        if self.centre is None:
            return "Centre"
        if self.lock is not None:
            return f"Radius {self.canvas.units.format(self.lock)}: click or Enter"
        return "Radius, or point on circle"

    def paint(self, painter):
        if self.centre is not None and self.cursor is not None:
            r = self._radius(self.cursor)
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
