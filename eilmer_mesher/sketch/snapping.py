"""
NX-style point snapping: endpoint, intersection, centre, midpoint, on-curve,
horizontal/vertical inference from an anchor point, and an optional grid.
"""
import itertools
import math
from dataclasses import dataclass

import numpy as np

from ..model import geom

ENDPOINT = "endpoint"
INTERSECTION = "intersection"
CENTRE = "centre"
MIDPOINT = "midpoint"
ON_CURVE = "on curve"
HORIZONTAL = "horizontal"
VERTICAL = "vertical"
GRID = "grid"
NONE = ""

POINT_KINDS = (ENDPOINT, INTERSECTION, CENTRE, MIDPOINT)
PRIORITY = {ENDPOINT: 0, INTERSECTION: 1, CENTRE: 2, MIDPOINT: 3}
HV = "horizontal/vertical"
ALL_KINDS = POINT_KINDS + (ON_CURVE, HV, GRID)


@dataclass
class Snap:
    point: tuple
    kind: str = NONE
    curve_id: str = None


class Snapper:
    def __init__(self):
        self.enabled = {k: True for k in ALL_KINDS}
        self.enabled[GRID] = False
        self._revision = None
        self._points = np.empty((0, 2))
        self._kinds = []
        self._owners = []

    def _build(self, doc):
        pts, kinds, owners = [], [], []

        def add(p, kind, ids):
            pts.append(p)
            kinds.append(kind)
            owners.append(frozenset(ids))

        curves = list(doc.curves.values())
        for c in curves:
            add(c.start(), ENDPOINT, [c.id])
            add(c.end(), ENDPOINT, [c.id])
            add(geom.midpoint(c), MIDPOINT, [c.id])
            if c.type == "arc":
                add(c.centre, CENTRE, [c.id])
        for a, b in itertools.combinations(curves, 2):
            for p in geom.intersections(a, b):
                add(p, INTERSECTION, [a.id, b.id])
        self._points = np.array(pts, dtype=float).reshape(-1, 2)
        self._kinds = kinds
        self._owners = owners

    def snap(self, doc, revision, p, tol, grid_step=None, anchor=None, exclude=frozenset()):
        """Snap world point p; tol is the capture radius in world units."""
        if revision != self._revision:
            self._build(doc)
            self._revision = revision
        exclude = frozenset(exclude)

        best = None
        if len(self._points):
            d = np.hypot(*(self._points - np.asarray(p, float)).T)
            for k in np.nonzero(d <= tol)[0]:
                kind = self._kinds[k]
                if not self.enabled[kind] or (self._owners[k] & exclude):
                    continue
                rank = (PRIORITY[kind], d[k])
                if best is None or rank < best[0]:
                    owner = sorted(self._owners[k])[0]
                    best = (rank, Snap((float(self._points[k, 0]), float(self._points[k, 1])), kind, owner))
        if best is not None:
            return best[1]

        if self.enabled[ON_CURVE]:
            near = None
            for c in doc.curves.values():
                if c.id in exclude:
                    continue
                q, dist = geom.closest_point(c, p)
                if dist <= tol and (near is None or dist < near[1]):
                    near = (q, dist, c.id)
            if near is not None:
                return Snap(near[0], ON_CURVE, near[2])

        x, y = float(p[0]), float(p[1])
        kind = NONE
        lock_x = lock_y = False
        if anchor is not None and self.enabled[HV]:
            dx, dy = x - anchor[0], y - anchor[1]
            if abs(dy) <= tol and abs(dx) > tol:
                y, lock_y, kind = anchor[1], True, HORIZONTAL
            elif abs(dx) <= tol and abs(dy) > tol:
                x, lock_x, kind = anchor[0], True, VERTICAL
        if grid_step and self.enabled[GRID]:
            if not lock_x:
                x = round(x / grid_step) * grid_step
            if not lock_y:
                y = round(y / grid_step) * grid_step
            kind = kind or GRID
        return Snap((x, y), kind)


def nice_step(min_step):
    """Smallest 1-2-5 x 10^k step that is at least min_step."""
    if min_step <= 0:
        return 1.0
    k = math.floor(math.log10(min_step))
    for m in (1.0, 2.0, 5.0, 10.0):
        if m * 10**k >= min_step * (1 - 1e-12):
            return m * 10**k
    return 10.0 ** (k + 1)
