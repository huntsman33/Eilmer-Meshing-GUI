"""
Offset curves: new curves a fixed distance to one side of existing ones (as NX's
Offset Curve). Qt-free; the sketch model wraps it in one undoable edit.

Picked curves that meet end to end are offset as chains:

  line            a parallel line
  arc             a concentric arc (radius +- d); refused if the radius would vanish
  Bezier, spline  a spline through normal-offset samples of the curve

Neighbouring pieces are joined exactly (identical end coordinates): where they
overlap (inside a corner) they are trimmed to their crossing; where they leave
a gap (outside a corner) lines are extended to meet and other pieces get a round
corner (an arc about the original vertex). Tangent joins just have their ends
snapped together.

A closed chain stays closed: by default it grows (offset outwards, found from the
loop's signed area) and reverse shrinks it. An open chain is offset to the left of
its direction of travel by default. A result that would turn inside out, vanish or
cross itself is refused with a ValueError.
"""
import math
from dataclasses import dataclass

import numpy as np
from shapely.geometry import LinearRing, LineString

from . import geom
from .entities import ArcSeg, LineSeg, SplineSeg
from .operations import geom_size, reversed_curve

MITRE_LIMIT = 20.0          # extend lines to meet only while the corner is within this many offsets
TANGENT_ANGLE = 1e-3        # joins turning less than this (radians) are treated as tangent
MIN_SAMPLES = 65            # samples of an offset Bezier or spline
MAX_SAMPLES = 801


@dataclass
class Chain:
    """Curves joined end to end, each turned to run along the chain."""
    curves: list
    closed: bool

    def polyline(self):
        pts = [geom.sample(c) for c in self.curves]
        return np.vstack([pts[0]] + [p[1:] for p in pts[1:]])

    def side(self, reverse=False):
        """+1 to offset to the left of the direction of travel, -1 to the right."""
        if self.closed:
            side = -1.0 if _signed_area(self.polyline()) > 0 else 1.0     # outwards
        else:
            side = 1.0
        return -side if reverse else side


def _signed_area(P):
    x, y = P[:, 0], P[:, 1]
    return 0.5 * float(np.sum(x * np.roll(y, -1) - np.roll(x, -1) * y))


def _tol(curves):
    return 1e-8 * max(geom_size(curves), 1e-300)


# --- chains ------------------------------------------------------------------------------

def find_chains(curves, tol=None):
    """
    Group curves into chains that meet end to end. A chain stops where more than two
    curve ends meet. Open chains run so most of their curves keep their own direction.
    """
    curves = list(curves)
    if tol is None:
        tol = _tol(curves)
    verts = []

    def vid(p):
        for k, v in enumerate(verts):
            if math.dist(p, v) <= tol:
                return k
        verts.append(p)
        return len(verts) - 1

    ends = [(vid(c.start()), vid(c.end())) for c in curves]
    incident = {}
    for i, (v0, v1) in enumerate(ends):
        incident.setdefault(v0, []).append((i, 0))
        incident.setdefault(v1, []).append((i, 1))
    used = set()

    def walk(i, forward):
        seq = [(i, forward)]
        used.add(i)
        while True:
            last, fwd = seq[-1]
            leave = 1 if fwd else 0
            v = ends[last][leave]
            if len(incident[v]) != 2:
                break
            j, e = next(x for x in incident[v] if x != (last, leave))
            if j in used:
                break
            used.add(j)
            seq.append((j, e == 0))
        return seq

    out = []
    order = [i for i in range(len(curves)) if any(len(incident[v]) != 2 for v in ends[i])]
    order += [i for i in range(len(curves)) if i not in order]
    for i in order:
        if i in used:
            continue
        if len(incident[ends[i][0]]) != 2:
            seq = walk(i, True)
        elif len(incident[ends[i][1]]) != 2:
            seq = walk(i, False)
        else:
            seq = walk(i, True)
        first, last = seq[0], seq[-1]
        v_start = ends[first[0]][0 if first[1] else 1]
        v_end = ends[last[0]][1 if last[1] else 0]
        closed = v_start == v_end and len(incident[v_start]) == 2
        if not closed and sum(f for _, f in seq) * 2 < len(seq):
            seq = [(k, not f) for k, f in reversed(seq)]
        out.append(Chain([curves[k] if f else reversed_curve(curves[k]) for k, f in seq], closed))
    return out


# --- offsetting one curve ----------------------------------------------------------------

def _left_normal(t):
    return np.array([-t[1], t[0]])


def _sample_params(c):
    """Parameters for sampling a Bezier or spline: the spline's own knots (dense where its
    points are, e.g. an airfoil's leading edge) and midpoints, refined to at least MIN_SAMPLES."""
    ts = np.linspace(0.0, 1.0, MIN_SAMPLES)
    if c.type == "spline":
        _, tv = geom._spline_data(tuple(geom._tup(p) for p in c.points))
        t0, t1 = geom.spline_range(c)
        knots = np.concatenate([[0.0], tv])
        u = np.concatenate([knots, (knots[:-1] + knots[1:]) / 2.0])
        u = u[(u > t0) & (u < t1)]
        ts = np.unique(np.concatenate([[0.0, 1.0], (u - t0) / (t1 - t0)]))
        while len(ts) < MIN_SAMPLES:
            ts = np.unique(np.concatenate([ts, (ts[:-1] + ts[1:]) / 2.0]))
        if len(ts) > MAX_SAMPLES:
            ts = ts[np.round(np.linspace(0, len(ts) - 1, MAX_SAMPLES)).astype(int)]
    return ts


def _tangents(c, ts, h=1e-6):
    """Unit tangents at ts: central differences, second order one-sided at the ends."""
    a = np.clip(ts - h, 0.0, 1.0)
    b = np.clip(ts + h, 0.0, 1.0)
    T = (geom.points_at(c, b) - geom.points_at(c, a)) / (b - a)[:, None]
    P0, P1, P2 = geom.points_at(c, [0.0, h, 2*h])
    T[0] = (-3*P0 + 4*P1 - P2) / (2*h)
    Q0, Q1, Q2 = geom.points_at(c, [1.0, 1.0 - h, 1.0 - 2*h])
    T[-1] = (3*Q0 - 4*Q1 + Q2) / (2*h)
    n = np.hypot(*T.T)
    if np.any(n <= 0.0):
        raise ValueError("a curve has a point where its direction is undefined")
    return T / n[:, None]


def offset_curve(c, d, tol=0.0):
    """
    One curve offset by d to the left of its direction (d < 0: to the right). Bezier
    and spline curves become splines. ValueError if an arc's radius would vanish, or
    a curve would fold back on itself (d beyond its radius of curvature).
    """
    common = {"id": None, "kind": c.kind}
    if c.type == "line":
        t = np.subtract(c.p1, c.p0)
        n = _left_normal(t / np.hypot(*t)) * d
        return LineSeg(**common, p0=geom._tup(c.p0 + n), p1=geom._tup(c.p1 + n))
    if c.type == "arc":
        ctr, r, _, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
        r_new = r - d if sweep > 0 else r + d          # the left of an anticlockwise arc is its centre
        if r_new <= tol:
            raise ValueError(f"an arc of radius {r:.4g} m cannot be offset {abs(d):.4g} m towards its "
                             "centre (its radius would vanish)")
        scale = r_new / r
        p0 = ctr + (np.asarray(c.p0) - ctr) * scale
        p1 = ctr + (np.asarray(c.p1) - ctr) * scale
        return ArcSeg(**common, p0=geom._tup(p0), p1=geom._tup(p1), centre=geom._tup(ctr))
    ts = _sample_params(c)
    P = geom.points_at(c, ts)
    T = _tangents(c, ts)
    Q = P + d * np.stack([-T[:, 1], T[:, 0]], axis=1)
    dP, dQ = np.diff(P, axis=0), np.diff(Q, axis=0)
    if np.any((dP * dQ).sum(axis=1) <= 0.0):
        raise ValueError(f"a {c.type} curve bends too tightly to offset {abs(d):.4g} m to that side")
    return SplineSeg(**common, points=[geom._tup(q) for q in Q])


# --- joins ---------------------------------------------------------------------------------

def _with_end(c, which, p):
    """Copy of an offset piece with its start or end moved exactly to p."""
    p = geom._tup(p)
    if c.type in ("line", "arc"):
        return c.copy(**{"p0" if which == "start" else "p1": p})
    pts = list(c.points)
    pts[0 if which == "start" else -1] = p
    return c.copy(points=pts)


def _part(c, ta, tb, n=None):
    """The piece of an offset curve between parameters ta < tb (a spline is resampled, untrimmed)."""
    if ta >= tb:
        raise ValueError("the offset is too large: a piece of the result vanishes")
    if c.type in ("line", "arc"):
        return geom.subcurve(c, ta, tb)
    n = n or max(len(c.points), MIN_SAMPLES)
    P = geom.points_at(c, np.linspace(ta, tb, n))
    return c.copy(points=[geom._tup(q) for q in P])


def _end_tangents(c):
    """Unit directions of travel at the start and end of a curve (accurate for splines too)."""
    if c.type in ("line", "arc"):
        return -geom.end_tangent(reversed_curve(c)), geom.end_tangent(c)
    T = _tangents(c, np.array([0.0, 1.0]))
    return T[0], T[1]


def _line_cross(a, b):
    """Crossing of the infinite lines through two line pieces, or None if parallel."""
    p, r = np.asarray(a.p0, float), np.subtract(a.p1, a.p0)
    q, s = np.asarray(b.p0, float), np.subtract(b.p1, b.p0)
    den = r[0]*s[1] - r[1]*s[0]
    if abs(den) < 1e-14 * np.hypot(*r) * np.hypot(*s):
        return None
    w = q - p
    return p + r * ((w[0]*s[1] - w[1]*s[0]) / den)


def _join(A, B, oA, oB, d, tol):
    """
    Join offset piece A (of original oA) to the following piece B (of oB).
    Returns (A, B, extra curves between them).
    """
    V = np.asarray(oA.end(), float)
    pA, pB = np.asarray(A.end(), float), np.asarray(B.start(), float)
    tA, tB = _end_tangents(oA)[1], _end_tangents(oB)[0]
    turn = math.atan2(tA[0]*tB[1] - tA[1]*tB[0], tA @ tB)
    if math.dist(pA, pB) <= tol or abs(turn) < TANGENT_ANGLE:
        return A, _with_end(B, "start", pA), []
    both_lines = A.type == "line" and B.type == "line"
    if turn * d > 0:                       # the offset is inside the corner: the pieces overlap
        if both_lines:
            X = _line_cross(A, B)
            if X is None:
                raise ValueError("the offset is too large at a corner")
            return _with_end(A, "end", X), _with_end(B, "start", X), []
        hits = geom.intersect(A, B)
        if not hits:
            raise ValueError(f"the offset is too large at the corner ({V[0]:.4g}, {V[1]:.4g})")
        la, lb = geom.path_length(A), geom.path_length(B)
        ta, tb, X = min(hits, key=lambda h: (1.0 - h[0]) * la + h[1] * lb)
        A = _with_end(_part(A, 0.0, ta), "end", X)
        B = _with_end(_part(B, tb, 1.0), "start", X)
        return A, B, []
    # the offset is outside the corner: a gap to fill
    if both_lines:
        X = _line_cross(A, B)
        if X is not None and math.dist(X, V) <= MITRE_LIMIT * abs(d):
            return _with_end(A, "end", X), _with_end(B, "start", X), []
    a0 = math.atan2(pA[1] - V[1], pA[0] - V[0])
    u, v = pA - V, pB - V
    sweep = math.atan2(u[0]*v[1] - u[1]*v[0], u @ v)
    arcs = [ArcSeg(None, kind=A.kind, p0=p, p1=q, centre=c)
            for p, q, c in geom.arc_pieces(V, abs(d), a0, sweep, tuple(pA), tuple(pB))]
    return A, B, arcs


# --- chains ------------------------------------------------------------------------------

def _check_piece(new, old, tol):
    """Refuse pieces that turned inside out or shrank to nothing."""
    if geom.path_length(new) <= tol:
        raise ValueError("the offset is too large: a piece of the result vanishes")
    if new.type == "line":
        if np.dot(np.subtract(new.p1, new.p0), np.subtract(old.p1, old.p0)) <= 0.0:
            raise ValueError("the offset is too large: the result would turn inside out")
    elif new.type == "arc":
        _, _, _, s_new = geom.arc_geometry(new.p0, new.p1, new.centre)
        _, _, _, s_old = geom.arc_geometry(old.p0, old.p1, old.centre)
        if s_new * s_old <= 0.0:
            raise ValueError("the offset is too large: the result would turn inside out")


def _polyline(curves):
    pts = [geom.sample(c) for c in curves]
    P = np.vstack([pts[0]] + [p[1:] for p in pts[1:]])
    keep = np.concatenate([[True], np.hypot(*np.diff(P, axis=0).T) > 0.0])
    return P[keep]


def offset_chain(chain, d, tol):
    """Offset one chain by d (> 0: to the left of its direction). Returns the new curves in order."""
    n = len(chain.curves)
    pieces = [offset_curve(c, d, tol) for c in chain.curves]
    extras = [[] for _ in range(n)]
    for k in range(n - 1 if not chain.closed else n):
        j = (k + 1) % n
        if n == 1 and chain.closed:
            pieces[0] = _with_end(pieces[0], "end", pieces[0].start())    # a single closed curve
            break
        pieces[k], pieces[j], extras[k] = _join(pieces[k], pieces[j], chain.curves[k], chain.curves[j], d, tol)
    for new, old in zip(pieces, chain.curves):
        _check_piece(new, old, tol)
    out = []
    for p, e in zip(pieces, extras):
        out.append(p)
        out.extend(e)

    # The result must not cross itself (a closed one must keep its orientation).
    if len(out) > 1 or chain.closed:
        P = _polyline(out)
        if chain.closed:
            area0 = _signed_area(chain.polyline())
            area1 = _signed_area(P)
            if len(P) < 4 or area1 * area0 <= 0.0 or abs(area1) <= tol * tol:
                raise ValueError("the offset is too large: the closed shape would vanish")
            if not LinearRing(P).is_simple:
                raise ValueError("the offset is too large: the result would cross itself")
        elif LineString(chain.polyline()).is_simple and not LineString(P).is_simple:
            raise ValueError("the offset is too large: the result would cross itself")
    return out


def offset_chains(curves, distance, reverse=False):
    """
    New curves offsetting the given ones by distance (> 0, metres): each chain of
    connected curves is offset as a whole (closed chains grow, or shrink if reverse;
    open chains go to the left of their direction, or the right if reverse).
    The originals are not changed. ValueError explains a failure.
    """
    curves = list(curves)
    if not curves:
        raise ValueError("pick the curves to offset first")
    if not distance > 0.0:
        raise ValueError("the offset distance must be positive")
    tol = _tol(curves)
    out = []
    for chain in find_chains(curves, tol):
        out.extend(offset_chain(chain, chain.side(reverse) * distance, tol))
    return out


def side_markers(curves, reverse=False):
    """[(point, unit normal)] per chain: where to draw an arrow showing the offset side."""
    out = []
    curves = list(curves)
    if not curves:
        return out
    for chain in find_chains(curves):
        c = chain.curves[len(chain.curves) // 2]
        p = geom.point_at(c, 0.5)
        a, b = geom.points_at(c, [0.5 - 1e-4, 0.5 + 1e-4])
        tan = (b - a) / max(np.hypot(*(b - a)), 1e-300)
        out.append((p, geom._tup(_left_normal(tan) * chain.side(reverse))))
    return out
