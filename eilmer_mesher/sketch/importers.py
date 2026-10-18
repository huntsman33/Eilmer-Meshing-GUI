"""
Import point files (.dat/.txt) as spline geometry, e.g. airfoils from the
UIUC database.

Formats (auto-detected):
  Selig     name line, then x y from the trailing edge over the upper surface
            to the leading edge and back along the lower surface
  Lednicer  name line, "NU. NL." counts, then upper (LE->TE) and lower (LE->TE) blocks
  plain     x y (or x,y / x y z) per line, used in the order given
"""
import math
import re
from dataclasses import dataclass

from ..model.entities import GEOMETRY, LineSeg, SplineSeg

_SPLIT = re.compile(r"[,\s;]+")
DUP_TOL = 1.0e-12


@dataclass
class PointFile:
    name: str
    fmt: str
    points: list


def _numeric_row(line):
    parts = [p for p in _SPLIT.split(line.split("#", 1)[0].strip()) if p]
    if len(parts) < 2:
        return None
    try:
        return [float(p) for p in parts]
    except ValueError:
        return None


def _dedupe(points):
    out = []
    for p in points:
        if not out or math.hypot(p[0]-out[-1][0], p[1]-out[-1][1]) > DUP_TOL:
            out.append(p)
    return out


def parse_point_file(text, default_name="points"):
    name = None
    rows = []
    for line in text.splitlines():
        row = _numeric_row(line)
        if row is None:
            if not rows and name is None and line.strip() and not line.strip().startswith("#"):
                name = line.strip()
            continue
        rows.append(row)
    if not rows:
        raise ValueError("no coordinate rows found")

    first = rows[0]
    is_count = lambda v: v >= 2 and float(v).is_integer()
    if len(first) == 2 and is_count(first[0]) and is_count(first[1]) \
            and len(rows) - 1 >= int(first[0]) + int(first[1]):
        nu, nl = int(first[0]), int(first[1])
        upper = [(r[0], r[1]) for r in rows[1:1+nu]]
        lower = [(r[0], r[1]) for r in rows[1+nu:1+nu+nl]]
        if math.hypot(upper[0][0]-lower[0][0], upper[0][1]-lower[0][1]) <= DUP_TOL:
            lower = lower[1:]
        return PointFile(name or default_name, "lednicer", _dedupe(upper[::-1] + lower))

    points = _dedupe([(r[0], r[1]) for r in rows])
    fmt = "selig" if name is not None else "xy"
    return PointFile(name or default_name, fmt, points)


def leading_edge_index(points):
    """Index of the minimum-x point (the leading edge for airfoil-style data)."""
    xmin = min(p[0] for p in points)
    tied = [k for k, p in enumerate(points) if p[0] - xmin <= 1e-12 * max(1.0, abs(xmin))]
    ymid = sum(p[1] for p in points) / len(points)
    return min(tied, key=lambda k: abs(points[k][1] - ymid))


def transform_points(points, scale=1.0, angle_deg=0.0, dx=0.0, dy=0.0):
    """Scale, rotate nose-up by angle_deg (clockwise, as an angle of attack), then translate."""
    a = math.radians(angle_deg)
    ca, sa = math.cos(a), math.sin(a)
    out = []
    for x, y in points:
        x, y = x * scale, y * scale
        out.append((x*ca + y*sa + dx, -x*sa + y*ca + dy))
    return out


def build_curves(pf, scale=1.0, angle_deg=0.0, dx=0.0, dy=0.0,
                 split_le=True, close_te=True, kind=GEOMETRY):
    """Spline curves (ids unset) for a parsed point file."""
    raw = pf.points
    if len(raw) < 2:
        raise ValueError("need at least two points")
    k_le = leading_edge_index(raw) if split_le else 0
    pts = transform_points(raw, scale, angle_deg, dx, dy)

    curves = []
    if split_le and 0 < k_le < len(pts) - 1:
        curves.append(SplineSeg(None, kind=kind, points=pts[:k_le+1]))
        curves.append(SplineSeg(None, kind=kind, points=pts[k_le:]))
    else:
        curves.append(SplineSeg(None, kind=kind, points=pts))
    first, last = pts[0], pts[-1]
    if close_te and math.hypot(first[0]-last[0], first[1]-last[1]) > DUP_TOL * max(1.0, scale):
        curves.append(LineSeg(None, kind=kind, p0=last, p1=first))
    return curves
