"""
Planar topology of a sketch: where curves meet, the edges between those points,
and the enclosed regions (faces) they bound, including holes.

All curves take part, solid and dashed: reference lines split regions just as
geometry does (they become internal block interfaces later). Curves are never
modified here; an edge is a parameter sub-range [ta, tb] of one curve.
"""
import math
from dataclasses import dataclass, field

import numpy as np
from shapely.geometry import Point, Polygon

from . import geom
from .entities import REFERENCE

PARAM_TOL = 1.0e-9
EDGE_SAMPLES = 24


@dataclass
class Edge:
    curve_id: str
    ta: float
    tb: float
    v0: int          # vertex at ta
    v1: int          # vertex at tb


@dataclass
class Region:
    loop: list                       # [(edge index, forward)], counter-clockwise outer boundary
    holes: list = field(default_factory=list)   # clockwise loops of islands inside it
    polygon: np.ndarray = None       # sampled outer boundary
    hole_polygons: list = field(default_factory=list)
    area: float = 0.0

    def contains(self, p):
        return _shape(self).contains(Point(p))


def _shape(region):
    return Polygon(region.polygon, [h for h in region.hole_polygons if len(h) >= 3])


def model_size(curves):
    lo = np.array([math.inf, math.inf])
    hi = -lo
    for c in curves:
        a, b = geom.bbox(c)
        lo, hi = np.minimum(lo, a), np.maximum(hi, b)
    return float(np.hypot(*(hi - lo))) if curves else 1.0


def curve_params(doc, cid, tol=None):
    """Sorted parameters (including 0 and 1) where curve cid meets any other curve."""
    curve = doc.curves[cid]
    tol = tol if tol is not None else 1e-9 * max(model_size(list(doc.curves.values())), 1e-300)
    ts = [0.0, 1.0]
    for other in doc.curves.values():
        if other.id == cid:
            continue
        ts.extend(t for t, _, _ in geom.intersect(curve, other))
        for p in (other.start(), other.end()):          # T-junctions
            t, _, d = geom.closest_param(curve, p)
            if d <= tol:
                ts.append(t)
    return _merge(ts)


def _merge(ts):
    out = []
    for t in sorted(min(max(t, 0.0), 1.0) for t in ts):
        if not out or t - out[-1] > PARAM_TOL:
            out.append(t)
    out[0], out[-1] = 0.0, 1.0
    return out


class Topology:
    def __init__(self, doc):
        curves = [c for c in doc.curves.values() if c.kind != REFERENCE]    # helpers: not meshed
        self.doc = doc
        self.size = model_size(curves)
        self.tol = 1e-8 * max(self.size, 1e-300)
        self.vertices = []
        self.edges = []
        self.regions = []
        if curves:
            self._build(curves)

    # --- construction -----------------------------------------------------------
    def _vertex(self, p):
        for k, v in enumerate(self.vertices):
            if math.hypot(p[0] - v[0], p[1] - v[1]) <= self.tol:
                return k
        self.vertices.append((float(p[0]), float(p[1])))
        return len(self.vertices) - 1

    def _build(self, curves):
        params = {c.id: [0.0, 1.0] for c in curves}
        for i, a in enumerate(curves):
            for b in curves[i+1:]:
                for ta, tb, _ in geom.intersect(a, b):
                    params[a.id].append(ta)
                    params[b.id].append(tb)
        for a in curves:                                   # T-junctions
            for b in curves:
                if a is b:
                    continue
                for p in (b.start(), b.end()):
                    t, _, d = geom.closest_param(a, p)
                    if d <= self.tol:
                        params[a.id].append(t)

        for c in curves:
            ts = _merge(params[c.id])
            pts = geom.points_at(c, ts)
            vids = [self._vertex(p) for p in pts]
            for k in range(len(ts) - 1):
                if vids[k] == vids[k+1] and geom.path_length(geom.subcurve(c, ts[k], ts[k+1])) <= self.tol:
                    continue
                self.edges.append(Edge(c.id, ts[k], ts[k+1], vids[k], vids[k+1]))

        live = self._prune_dangling()
        self._find_regions(live)

    def _prune_dangling(self):
        """Edge indices left after repeatedly removing edges with a free end."""
        live = set(range(len(self.edges)))
        while True:
            degree = [0] * len(self.vertices)
            for e in live:
                degree[self.edges[e].v0] += 1
                degree[self.edges[e].v1] += 1
            dead = {e for e in live if degree[self.edges[e].v0] == 1 or degree[self.edges[e].v1] == 1}
            if not dead:
                return live
            live -= dead

    # --- edge geometry ------------------------------------------------------------
    def edge_points(self, e, forward=True, n=EDGE_SAMPLES):
        edge = self.edges[e]
        c = self.doc.curves[edge.curve_id]
        k = 2 if c.type == "line" else n
        pts = geom.points_at(c, np.linspace(edge.ta, edge.tb, k))
        pts[0] = self.vertices[edge.v0]
        pts[-1] = self.vertices[edge.v1]
        return pts if forward else pts[::-1]

    def _leaving_angle(self, e, forward):
        edge = self.edges[e]
        c = self.doc.curves[edge.curve_id]
        t_from, t_to = (edge.ta, edge.tb) if forward else (edge.tb, edge.ta)
        origin = np.array(self.vertices[edge.v0 if forward else edge.v1])
        q = geom.points_at(c, [t_from + 1e-3 * (t_to - t_from)])[0]
        d = q - origin
        return math.atan2(d[1], d[0])

    # --- faces ----------------------------------------------------------------------
    def _find_regions(self, live):
        # half-edge h = 2*e (v0 -> v1) or 2*e + 1 (v1 -> v0)
        out = {}
        for e in live:
            out.setdefault(self.edges[e].v0, []).append(2*e)
            out.setdefault(self.edges[e].v1, []).append(2*e + 1)
        for v, hs in out.items():
            hs.sort(key=lambda h: self._leaving_angle(h // 2, h % 2 == 0))
        position = {h: (v, k) for v, hs in out.items() for k, h in enumerate(hs)}

        def head(h):
            edge = self.edges[h // 2]
            return edge.v1 if h % 2 == 0 else edge.v0

        def next_half(h):
            twin = h ^ 1
            v, k = position[twin]
            hs = out[v]
            return hs[(k - 1) % len(hs)]   # first clockwise from the twin: face stays on the left

        seen = set()
        faces = []
        for start in sorted(position):
            if start in seen:
                continue
            loop = []
            h = start
            while h not in seen:
                seen.add(h)
                loop.append((h // 2, h % 2 == 0))
                h = next_half(h)
            faces.append(loop)

        component = self._components(live)
        bounded, outers = [], []
        for loop in faces:
            poly = np.vstack([self.edge_points(e, fwd)[:-1] for e, fwd in loop])
            area = _signed_area(poly)
            if area > 0:
                bounded.append(Region(loop, polygon=poly, area=area))
            else:
                outers.append((loop, poly, component[self.edges[loop[0][0]].v0]))

        # An island's outer (clockwise) boundary is a hole in the smallest region around it.
        for loop, poly, comp in outers:
            probe = Point(poly[0])
            hosts = [r for r in bounded
                     if component[self.edges[r.loop[0][0]].v0] != comp
                     and Polygon(r.polygon).contains(probe)]
            if hosts:
                host = min(hosts, key=lambda r: r.area)
                host.holes.append(loop)
                host.hole_polygons.append(poly)
        for r in bounded:
            r.area -= sum(abs(_signed_area(h)) for h in r.hole_polygons)
        self.regions = sorted(bounded, key=lambda r: r.area)

    def _components(self, live):
        parent = list(range(len(self.vertices)))

        def find(a):
            while parent[a] != a:
                parent[a] = parent[parent[a]]
                a = parent[a]
            return a

        for e in live:
            a, b = find(self.edges[e].v0), find(self.edges[e].v1)
            parent[a] = b
        return [find(v) for v in range(len(self.vertices))]

    # --- queries ----------------------------------------------------------------------
    def region_at(self, p):
        """The innermost region containing point p, or None."""
        for r in self.regions:          # sorted smallest first
            if r.contains(p):
                return r
        return None

    def loop_curves(self, loop):
        """Curve ids around a loop, in order."""
        return [self.edges[e].curve_id for e, _ in loop]


def _signed_area(P):
    x, y = P[:, 0], P[:, 1]
    return 0.5 * float(np.dot(x, np.roll(y, -1)) - np.dot(y, np.roll(x, -1)))
