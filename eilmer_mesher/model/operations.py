"""
Sketch operations that build new curves from existing ones: merge and mirror.
Qt-free; the sketch model wraps them in undoable edits.
"""
import math

import numpy as np

from . import geom


# --- reversal ----------------------------------------------------------------------

def reversed_curve(c):
    """The same curve traversed the other way."""
    if c.type in ("line", "arc"):
        return c.copy(p0=c.p1, p1=c.p0)
    if c.type == "bezier":
        return c.copy(points=list(reversed(c.points)))
    if c.type == "spline":
        # The natural spline through the reversed points is the same curve, with t -> 1 - t.
        return c.copy(points=list(reversed(c.points)), t0=1.0 - c.t1, t1=1.0 - c.t0)
    raise ValueError(c.type)


# --- merge -------------------------------------------------------------------------------

def _close(a, b, tol):
    return math.dist(a, b) <= tol


def _join(first, second):
    """One curve equal to first followed by second (first.end == second.start), or None."""
    if first.kind != second.kind or first.type != second.type:
        return None
    if first.type == "line":
        d1 = np.subtract(first.p1, first.p0)
        d2 = np.subtract(second.p1, second.p0)
        n1, n2 = np.hypot(*d1), np.hypot(*d2)
        if abs(d1[0]*d2[1] - d1[1]*d2[0]) > 1e-9 * n1 * n2 or d1 @ d2 <= 0:
            return None
        return first.copy(p1=second.p1)
    if first.type == "arc":
        c1, r1, a0, s1 = geom.arc_geometry(first.p0, first.p1, first.centre)
        c2, r2, _, s2 = geom.arc_geometry(second.p0, second.p1, second.centre)
        tol = 1e-9 * max(r1, r2)
        if math.dist(c1, c2) > tol or abs(r1 - r2) > tol or s1 * s2 <= 0:
            return None
        if abs(s1 + s2) > geom.MAX_ARC_SWEEP:
            return None                      # an Eilmer Arc must stay well under 180 degrees
        return first.copy(p1=second.p1)
    if first.type == "spline":
        same = [tuple(p) for p in first.points] == [tuple(p) for p in second.points]
        if not same or abs(first.t1 - second.t0) > 1e-9:
            return None
        t0, t1 = first.t0, second.t1
        if t0 <= 1e-12 and t1 >= 1.0 - 1e-12:
            t0, t1 = 0.0, 1.0
        return first.copy(t0=t0, t1=t1)
    return None


def try_merge(a, b, tol):
    """a and b as one curve if they share an end and can be joined exactly, else None."""
    if a.type == "spline" and b.type == "spline" and \
            [tuple(p) for p in a.points] == [tuple(p) for p in reversed(b.points)]:
        b = reversed_curve(b)          # same spline, entered the other way round
    ra, rb = reversed_curve(a), reversed_curve(b)
    # Prefer orders that keep a's direction, so a merge never flips the result.
    for f, s in ((a, b), (b, a), (a, rb), (rb, a), (ra, b), (b, ra), (ra, rb), (rb, ra)):
        if _close(f.end(), s.start(), tol):
            joined = _join(f, s)
            if joined is not None:
                return joined.copy(id=a.id, bc_tag=a.bc_tag or b.bc_tag)
    return None


def merge_curves(curves, tol=None):
    """
    Merge chains of compatible curves. Returns (result, removed ids): result holds the
    surviving curves (merged ones keep the id of one of their pieces).
    """
    pool = {c.id: c for c in curves}
    if tol is None:
        size = geom_size(curves)
        tol = 1e-9 * max(size, 1e-300)
    removed = []
    merged = True
    while merged:
        merged = False
        ids = list(pool)
        for i, ia in enumerate(ids):
            for ib in ids[i + 1:]:
                joined = try_merge(pool[ia], pool[ib], tol)
                if joined is not None:
                    pool[ia] = joined
                    del pool[ib]
                    removed.append(ib)
                    merged = True
                    break
            if merged:
                break
    return list(pool.values()), removed


def geom_size(curves):
    pts = np.vstack([geom.sample(c) for c in curves]) if curves else np.zeros((1, 2))
    return float(np.hypot(*(pts.max(axis=0) - pts.min(axis=0))))


# --- mirror ----------------------------------------------------------------------------

def reflect(p, a, b):
    """Reflection of point p across the line through a and b."""
    a = np.asarray(a, float)
    d = np.asarray(b, float) - a
    d = d / np.hypot(*d)
    v = np.asarray(p, float) - a
    q = a + 2.0 * (v @ d) * d - v
    return (float(q[0]), float(q[1]))


def lies_on(curve, a, b, tol):
    """True if the whole curve lies on the line through a and b (its mirror is itself)."""
    a = np.asarray(a, float)
    d = np.asarray(b, float) - a
    d = d / np.hypot(*d)
    rel = geom.sample(curve) - a
    return bool(np.all(np.abs(rel[:, 0]*d[1] - rel[:, 1]*d[0]) <= tol))


def mirror_curve(curve, a, b):
    return curve.map_points(lambda p: reflect(p, a, b))
