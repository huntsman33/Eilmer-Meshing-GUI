"""
Geometry of two-line dimensions (no Qt).

Two lines are dimensioned by their perpendicular "distance" when parallel, or by the
"angle" between them otherwise. Both measure from the first line to the second.

A distance dimension's offset is where its dimension line sits along the first line,
measured from that line's start (metres). An angle dimension is drawn as an arc of
radius offset about the vertex (the lines' common end, or where they cross); its
sector [s1, s2] picks one of the four angles the lines make: the one between the
rays s1*(p1 - p0) of the first line and s2*(p1 - p0) of the second.
"""
import math

import numpy as np

PARALLEL_DEG = 0.5      # lines this close to parallel get a distance dimension


def _cross(a, b):
    """z component of the cross product of two 2-D vectors."""
    return float(a[0] * b[1] - a[1] * b[0])


def direction(line):
    """Unit vector from a line's start to its end."""
    d = np.asarray(line.p1, float) - np.asarray(line.p0, float)
    n = float(np.hypot(*d))
    if n == 0.0:
        raise ValueError("the line has no length")
    return d / n


def normal(line):
    """Unit normal to the left of the line (p0 -> p1)."""
    u = direction(line)
    return np.array([-u[1], u[0]])


def acute_angle(a, b):
    """Angle between two lines, ignoring direction, in degrees within [0, 90]."""
    cross = abs(_cross(direction(a), direction(b)))
    dot = abs(float(direction(a) @ direction(b)))
    return math.degrees(math.atan2(cross, dot))


def two_line_kind(a, b, tol_deg=PARALLEL_DEG):
    """"distance" for (nearly) parallel lines, "angle" otherwise."""
    return "distance" if acute_angle(a, b) <= tol_deg else "angle"


def signed_distance(a, b):
    """Distance from line a to the middle of line b, positive on a's left."""
    mid = (np.asarray(b.p0, float) + np.asarray(b.p1, float)) / 2.0
    return float((mid - np.asarray(a.p0, float)) @ normal(a))


def _scale(*points):
    return max([1.0] + [abs(float(c)) for p in points for c in p])


def vertex(a, b):
    """The lines' common end if they share one, else where they (extended) cross, else None."""
    tol = 1e-9 * _scale(a.p0, a.p1, b.p0, b.p1)
    for p in (a.p0, a.p1):
        for q in (b.p0, b.p1):
            if math.dist(p, q) <= tol:
                return np.asarray(p, float)
    ua, ub = direction(a), direction(b)
    det = _cross(ua, ub)
    if abs(det) < 1e-12:
        return None
    pa, pb = np.asarray(a.p0, float), np.asarray(b.p0, float)
    t = _cross(pb - pa, ub) / det
    return pa + t * ua


def angle_placement(a, b, world):
    """(radius, sector) that puts an angle dimension of lines a, b under world."""
    v = vertex(a, b)
    if v is None:
        return 0.0, [1, 1]
    w = np.asarray(world, float) - v
    ua, ub = direction(a), direction(b)
    # w = s*ua + t*ub: the signs of s and t say which of the four sectors w is in.
    s, t = np.linalg.solve(np.column_stack([ua, ub]), w)
    return float(np.hypot(*w)), [1 if s >= 0 else -1, 1 if t >= 0 else -1]


def default_sector(a, b):
    """The sector between the rays pointing away from the vertex (the inside of a corner)."""
    v = vertex(a, b)
    if v is None:
        return [1, 1]
    out = []
    for line in (a, b):
        p0, p1 = np.asarray(line.p0, float), np.asarray(line.p1, float)
        out.append(1 if np.hypot(*(p1 - v)) >= np.hypot(*(p0 - v)) else -1)
    return out


def sector_rays(a, b, sector):
    s1, s2 = sector or (1, 1)
    return s1 * direction(a), s2 * direction(b)


def signed_sector_angle(a, b, sector):
    """Signed angle (radians, in (-pi, pi)) from the first ray of the sector to the second."""
    r1, r2 = sector_rays(a, b, sector)
    return math.atan2(_cross(r1, r2), float(r1 @ r2))


def angle_deg(a, b, sector):
    """The angle the dimension shows, in degrees within [0, 180]."""
    return math.degrees(abs(signed_sector_angle(a, b, sector)))


def value(doc, dim):
    """Distance (metres) or angle (degrees) a two-line dimension currently shows."""
    a, b = doc.curves[dim.curve], doc.curves[dim.curve2]
    if dim.kind == "distance":
        return abs(signed_distance(a, b))
    return angle_deg(a, b, dim.sector)


def driven_line(doc, dim, new_value):
    """The second line moved so the dimension reads new_value. ValueError if it can't."""
    a, b = doc.curves[dim.curve], doc.curves[dim.curve2]
    p0, p1 = np.asarray(b.p0, float), np.asarray(b.p1, float)
    if dim.kind == "distance":
        if not new_value > 0:
            raise ValueError("a distance must be greater than zero")
        d = signed_distance(a, b)
        shift = normal(a) * ((1.0 if d >= 0 else -1.0) * new_value - d)
        q0, q1 = p0 + shift, p1 + shift
    else:
        if not 0 < new_value < 180:
            raise ValueError("an angle between two lines must be between 0 and 180 degrees")
        v = vertex(a, b)
        if v is None:
            raise ValueError("the lines are parallel, so they make no angle")
        now = signed_sector_angle(a, b, dim.sector)
        turn = math.copysign(math.radians(new_value), now if now != 0 else 1.0) - now
        c, s = math.cos(turn), math.sin(turn)
        rot = np.array([[c, -s], [s, c]])
        q0, q1 = v + rot @ (p0 - v), v + rot @ (p1 - v)
    return b.copy(p0=(float(q0[0]), float(q0[1])), p1=(float(q1[0]), float(q1[1])))
