"""
Numpy geometry for sketch curves: sampling, closest points, intersections and
arc construction. Qt-free so it can be unit tested and reused by topology/trim.

Arcs follow Eilmer's Arc: p0 -> p1 about centre, the short way round, so every
ArcSeg sweeps less than pi. Longer arcs are built from several pieces.
"""
import math
from functools import lru_cache

import numpy as np
from scipy.linalg import solve_banded

TWO_PI = 2.0 * math.pi
MAX_ARC_SWEEP = 0.95 * math.pi
ARC_STEP = math.radians(3.0)
BEZIER_SAMPLES = 64
SPLINE_SEG_SAMPLES = 16


def _arr(p):
    return np.asarray(p, dtype=float)


def _tup(p):
    return (float(p[0]), float(p[1]))


# --- arcs ------------------------------------------------------------------

def arc_geometry(p0, p1, centre):
    """(centre, radius, start angle, signed sweep) with |sweep| < pi, as Eilmer evaluates it."""
    c = _arr(centre)
    a = _arr(p0) - c
    b = _arr(p1) - c
    r = math.hypot(a[0], a[1])
    a0 = math.atan2(a[1], a[0])
    sweep = math.atan2(a[0]*b[1] - a[1]*b[0], a[0]*b[0] + a[1]*b[1])
    return c, r, a0, sweep


def arc_point(arc, t):
    c, r, a0, sweep = arc_geometry(arc.p0, arc.p1, arc.centre)
    th = a0 + t * sweep
    return (c[0] + r*math.cos(th), c[1] + r*math.sin(th))


def arc_pieces(centre, r, a0, sweep, p_start=None, p_end=None):
    """Split an arc into [(p0, p1, centre)] pieces each sweeping at most MAX_ARC_SWEEP."""
    c = _arr(centre)
    n = max(1, math.ceil(abs(sweep) / MAX_ARC_SWEEP - 1e-12))
    pts = [(c[0] + r*math.cos(a0 + sweep*k/n), c[1] + r*math.sin(a0 + sweep*k/n))
           for k in range(n + 1)]
    if p_start is not None:
        pts[0] = _tup(p_start)
    if p_end is not None:
        pts[-1] = _tup(p_end)
    return [(pts[k], pts[k+1], _tup(c)) for k in range(n)]


def circle_through(p0, pm, p1):
    """Centre and radius of the circle through three points, or None if collinear."""
    ax, ay = p0
    bx, by = pm
    cx, cy = p1
    d = 2.0 * (ax*(by - cy) + bx*(cy - ay) + cx*(ay - by))
    scale = max(abs(ax), abs(ay), abs(bx), abs(by), abs(cx), abs(cy), 1e-300)
    if abs(d) < 1e-12 * scale * scale:
        return None
    a2, b2, c2 = ax*ax + ay*ay, bx*bx + by*by, cx*cx + cy*cy
    ux = (a2*(by - cy) + b2*(cy - ay) + c2*(ay - by)) / d
    uy = (a2*(cx - bx) + b2*(ax - cx) + c2*(bx - ax)) / d
    return (ux, uy), math.hypot(ax - ux, ay - uy)


def arcs_through_3_points(p0, pm, p1):
    """Arc pieces from p0 through pm to p1, or None if the points are collinear."""
    circ = circle_through(p0, pm, p1)
    if circ is None:
        return None
    (ux, uy), r = circ
    ang = lambda p: math.atan2(p[1] - uy, p[0] - ux)
    a0, am, a1 = ang(p0), ang(pm), ang(p1)
    ccw = (a1 - a0) % TWO_PI
    mid = (am - a0) % TWO_PI
    sweep = ccw if mid < ccw else ccw - TWO_PI
    if abs(sweep) < 1e-12:
        return None
    return arc_pieces((ux, uy), r, a0, sweep, p0, p1)


def arcs_centre_start_end(centre, ps, pe, ccw=True):
    """Arc about centre from ps, ending on the ray towards pe (radius set by ps)."""
    c = _arr(centre)
    r = float(np.hypot(*(_arr(ps) - c)))
    if r == 0.0 or np.allclose(_arr(pe), c):
        return None
    a0 = math.atan2(ps[1] - c[1], ps[0] - c[0])
    a1 = math.atan2(pe[1] - c[1], pe[0] - c[0])
    sweep = (a1 - a0) % TWO_PI
    if sweep < 1e-12:
        return None
    if not ccw:
        sweep -= TWO_PI
    end = (c[0] + r*math.cos(a1), c[1] + r*math.sin(a1))
    return arc_pieces(c, r, a0, sweep, ps, end)


def circle_arcs(centre, r):
    """Full circle as four quarter arcs (natural O-grid corners), counter-clockwise from +x."""
    cx, cy = centre
    q = [(cx + r, cy), (cx, cy + r), (cx - r, cy), (cx, cy - r)]
    return [(q[k], q[(k + 1) % 4], _tup(centre)) for k in range(4)]


def tangent_arc(p, tangent, e):
    """Arc pieces leaving p along tangent and ending at e, or None if e lies on the tangent line."""
    p, e, t = _arr(p), _arr(e), _arr(tangent)
    t = t / np.hypot(*t)
    nrm = np.array([-t[1], t[0]])          # left normal
    d = e - p
    h = float(d @ nrm)
    if abs(h) < 1e-9 * max(1e-300, float(np.hypot(*d))):
        return None
    r_signed = float(d @ d) / (2.0 * h)
    c = p + nrm * r_signed
    a0 = math.atan2(p[1] - c[1], p[0] - c[0])
    a1 = math.atan2(e[1] - c[1], e[0] - c[0])
    sweep = (a1 - a0) % TWO_PI if r_signed > 0 else -((a0 - a1) % TWO_PI)
    return arc_pieces(c, abs(r_signed), a0, sweep, p, e)


def end_tangent(curve):
    """Unit direction of travel at the end of a curve."""
    if curve.type == "line":
        v = _arr(curve.p1) - _arr(curve.p0)
    elif curve.type == "arc":
        c, _, _, sweep = arc_geometry(curve.p0, curve.p1, curve.centre)
        rv = _arr(curve.p1) - c
        v = np.array([-rv[1], rv[0]]) if sweep > 0 else np.array([rv[1], -rv[0]])
    else:
        pts = sample(curve)
        v = pts[-1] - pts[-2]
    n = np.hypot(*v)
    return (v / n) if n > 0 else np.array([1.0, 0.0])


# --- Bezier and spline -----------------------------------------------------

def bezier_eval(B, t):
    """de Casteljau evaluation of control points B (n+1, 2) at parameters t (k,)."""
    B = _arr(B)
    t = _arr(t)[:, None, None]
    Q = np.broadcast_to(B, (t.shape[0],) + B.shape).copy()
    n = B.shape[0] - 1
    for k in range(n):
        Q[:, :n-k] = (1.0 - t) * Q[:, :n-k] + t * Q[:, 1:n-k+1]
    return Q[:, 0]


@lru_cache(maxsize=1024)
def _spline_beziers(points):
    P = _arr(points)
    m = len(P) - 1
    if m < 1:
        raise ValueError("a spline needs at least two points")
    d = P.copy()
    if m >= 2:
        # Natural cubic spline weight points: d[i-1] + 4 d[i] + d[i+1] = 6 p[i],
        # the fixed point of the Gauss-Seidel iteration in gridgen/Eilmer Spline.
        n = m - 1
        ab = np.zeros((3, n))
        ab[0, 1:] = 1.0
        ab[1, :] = 4.0
        ab[2, :-1] = 1.0
        rhs = 6.0 * P[1:m].copy()
        rhs[0] -= P[0]
        rhs[-1] -= P[m]
        d[1:m] = solve_banded((1, 1), ab, rhs)
    B = np.empty((m, 4, 2))
    B[:, 0] = P[:-1]
    B[:, 1] = (2.0*d[:-1] + d[1:]) / 3.0
    B[:, 2] = (d[:-1] + 2.0*d[1:]) / 3.0
    B[:, 3] = P[1:]
    B.setflags(write=False)
    return B


def spline_beziers(points):
    """Cubic Bezier control points (m, 4, 2) of the interpolating spline through points."""
    return _spline_beziers(tuple(_tup(p) for p in points))


# --- sampling ----------------------------------------------------------------

@lru_cache(maxsize=4096)
def _samples(key):
    kind, geo = key
    if kind == "line":
        out = _arr(geo)
    elif kind == "arc":
        p0, p1, centre = geo
        c, r, a0, sweep = arc_geometry(p0, p1, centre)
        n = max(4, math.ceil(abs(sweep) / ARC_STEP))
        th = a0 + sweep * np.linspace(0.0, 1.0, n + 1)
        out = np.column_stack([c[0] + r*np.cos(th), c[1] + r*np.sin(th)])
        out[0], out[-1] = p0, p1
    elif kind == "bezier":
        out = bezier_eval(geo[0], np.linspace(0.0, 1.0, BEZIER_SAMPLES))
    elif kind == "spline":
        B = _spline_beziers(geo[0])
        t = np.linspace(0.0, 1.0, SPLINE_SEG_SAMPLES + 1)[:-1]
        out = np.vstack([bezier_eval(seg, t) for seg in B] + [B[-1, 3][None]])
    else:
        raise ValueError(kind)
    out.setflags(write=False)
    return out


def sample(curve):
    """Polyline approximation (k, 2) of a curve, cached by its geometry."""
    return _samples(curve.geom_key())


def bbox(curve):
    pts = sample(curve)
    if curve.type == "bezier":   # include control points, which bound the curve
        pts = np.vstack([pts, _arr(curve.points)])
    return pts.min(axis=0), pts.max(axis=0)


# --- closest point -----------------------------------------------------------

def project_polyline(P, p):
    """Closest point on polyline P to p: (point, distance)."""
    p = _arr(p)
    if len(P) == 1:
        return _tup(P[0]), float(np.hypot(*(P[0] - p)))
    A = P[:-1]
    D = P[1:] - A
    L2 = (D * D).sum(axis=1)
    L2 = np.where(L2 > 0.0, L2, 1e-300)
    t = np.clip(((p - A) * D).sum(axis=1) / L2, 0.0, 1.0)
    Q = A + D * t[:, None]
    d2 = ((Q - p) ** 2).sum(axis=1)
    k = int(np.argmin(d2))
    return _tup(Q[k]), float(math.sqrt(d2[k]))


def _angle_in_sweep(a0, sweep, ang, tol=0.0):
    if sweep >= 0.0:
        rel = (ang - a0) % TWO_PI
        return rel <= sweep + tol or rel >= TWO_PI - tol
    rel = (a0 - ang) % TWO_PI
    return rel <= -sweep + tol or rel >= TWO_PI - tol


def closest_point(curve, p):
    """Closest point on a curve to p: (point, distance). Exact for lines and arcs."""
    p = _arr(p)
    if curve.type == "line":
        return project_polyline(_arr([curve.p0, curve.p1]), p)
    if curve.type == "arc":
        c, r, a0, sweep = arc_geometry(curve.p0, curve.p1, curve.centre)
        v = p - c
        dv = math.hypot(v[0], v[1])
        if dv > 0.0 and _angle_in_sweep(a0, sweep, math.atan2(v[1], v[0])):
            q = c + v * (r / dv)
            return _tup(q), abs(dv - r)
        ends = [curve.p0, curve.p1]
        dists = [math.hypot(*(p - _arr(e))) for e in ends]
        k = int(np.argmin(dists))
        return _tup(ends[k]), dists[k]
    return project_polyline(sample(curve), p)


def midpoint(curve):
    if curve.type == "line":
        return ((curve.p0[0] + curve.p1[0]) / 2.0, (curve.p0[1] + curve.p1[1]) / 2.0)
    if curve.type == "arc":
        return arc_point(curve, 0.5)
    P = sample(curve)
    seg = np.hypot(*(P[1:] - P[:-1]).T)
    cum = np.concatenate([[0.0], np.cumsum(seg)])
    half = cum[-1] / 2.0
    k = int(np.searchsorted(cum, half)) - 1
    k = min(max(k, 0), len(seg) - 1)
    f = (half - cum[k]) / seg[k] if seg[k] > 0 else 0.0
    return _tup(P[k] + f * (P[k+1] - P[k]))


# --- intersections -------------------------------------------------------------

def _cross(a, b):
    return a[..., 0]*b[..., 1] - a[..., 1]*b[..., 0]


def _dedupe(points, tol):
    out = []
    for p in points:
        if all(math.hypot(p[0]-q[0], p[1]-q[1]) > tol for q in out):
            out.append(_tup(p))
    return out


def _line_circle(p0, p1, c, r):
    """Intersections of segment p0-p1 with a circle: [(point, s)] with s along the segment."""
    p0, p1, c = _arr(p0), _arr(p1), _arr(c)
    d = p1 - p0
    f = p0 - c
    a = d @ d
    b = 2.0 * (f @ d)
    cc = f @ f - r*r
    disc = b*b - 4.0*a*cc
    if a == 0.0 or disc < 0.0:
        return []
    sq = math.sqrt(disc)
    roots = {(-b - sq) / (2*a), (-b + sq) / (2*a)}
    return [(p0 + s*d, s) for s in roots]


def _circle_circle(c1, r1, c2, r2):
    c1, c2 = _arr(c1), _arr(c2)
    v = c2 - c1
    d = math.hypot(v[0], v[1])
    if d == 0.0 or d > r1 + r2 or d < abs(r1 - r2):
        return []
    a = (r1*r1 - r2*r2 + d*d) / (2*d)
    h = math.sqrt(max(r1*r1 - a*a, 0.0))
    m = c1 + a * v / d
    perp = np.array([-v[1], v[0]]) / d
    return [m + h*perp, m - h*perp]


def _on_arc(arc, p, tol):
    c, r, a0, sweep = arc_geometry(arc.p0, arc.p1, arc.centre)
    ang_tol = tol / r if r > 0 else 0.0
    return _angle_in_sweep(a0, sweep, math.atan2(p[1] - c[1], p[0] - c[0]), ang_tol)


def polyline_intersections(A, B, chunk=512):
    """Approximate crossings of two polylines (k, 2), (m, 2)."""
    pts = []
    q = B[:-1]
    s = B[1:] - q
    bmin = np.minimum(B[:-1], B[1:])
    bmax = np.maximum(B[:-1], B[1:])
    for k0 in range(0, len(A) - 1, chunk):
        p = A[k0:k0+chunk+1]
        r = p[1:] - p[:-1]
        p = p[:-1]
        amin = np.minimum(p, p + r)
        amax = np.maximum(p, p + r)
        overlap = ((amin[:, None, :] <= bmax[None]) & (amax[:, None, :] >= bmin[None])).all(axis=2)
        ia, ib = np.nonzero(overlap)
        if ia.size == 0:
            continue
        pr, rr, qq, ss = p[ia], r[ia], q[ib], s[ib]
        denom = _cross(rr, ss)
        ok = denom != 0.0
        qp = qq - pr
        with np.errstate(divide="ignore", invalid="ignore"):
            t = _cross(qp, ss) / denom
            u = _cross(qp, rr) / denom
        ok &= (t >= 0.0) & (t <= 1.0) & (u >= 0.0) & (u <= 1.0)
        pts.extend(pr[ok] + t[ok, None] * rr[ok])
    return pts


def intersections(c1, c2, tol=1e-9):
    """Points where two curves cross or touch (exact for line/arc pairs)."""
    (lo1, hi1), (lo2, hi2) = bbox(c1), bbox(c2)
    size = max(float(np.max(hi1 - lo1)), float(np.max(hi2 - lo2)), 1e-300)
    eps = tol * max(size, 1.0)
    if np.any(lo1 > hi2 + eps) or np.any(lo2 > hi1 + eps):
        return []
    kinds = (c1.type, c2.type)
    if kinds == ("line", "line"):
        p, r = _arr(c1.p0), _arr(c1.p1) - _arr(c1.p0)
        q, s = _arr(c2.p0), _arr(c2.p1) - _arr(c2.p0)
        denom = _cross(r, s)
        if abs(denom) < 1e-14 * math.hypot(*r) * math.hypot(*s):
            return []           # parallel (overlaps are not reported)
        t = _cross(q - p, s) / denom
        u = _cross(q - p, r) / denom
        e = 1e-12
        if -e <= t <= 1 + e and -e <= u <= 1 + e:
            return [_tup(p + t * r)]
        return []
    if kinds in (("line", "arc"), ("arc", "line")):
        line, arc = (c1, c2) if c1.type == "line" else (c2, c1)
        _, r, _, _ = arc_geometry(arc.p0, arc.p1, arc.centre)
        e = 1e-12
        hits = [p for p, s in _line_circle(line.p0, line.p1, arc.centre, r)
                if -e <= s <= 1 + e and _on_arc(arc, p, eps)]
        return _dedupe(hits, eps)
    if kinds == ("arc", "arc"):
        _, r1, _, _ = arc_geometry(c1.p0, c1.p1, c1.centre)
        _, r2, _, _ = arc_geometry(c2.p0, c2.p1, c2.centre)
        hits = [p for p in _circle_circle(c1.centre, r1, c2.centre, r2)
                if _on_arc(c1, p, eps) and _on_arc(c2, p, eps)]
        return _dedupe(hits, eps)
    return _dedupe(polyline_intersections(sample(c1), sample(c2)), eps * 1e3)
