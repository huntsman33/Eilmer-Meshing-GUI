"""
Sub-divide a region into quadrilateral regions by adding dashed reference curves.
The blocks are then made from the new regions in the usual way.

  split_nxm       iso-parameter lines of the region's Coons patch
  ogrid           an inner quadrilateral joined to the four corners (e.g. a disc)
  ogrid_hole      spokes from the region's corners towards an island's centre (e.g. a
                  cylinder in a channel), leaving four blocks around the island
  centroid_split  spokes from each side's midpoint to the centroid (triangles,
                  pentagons, ...): N quadrilaterals
"""
import math

import numpy as np
from shapely.geometry import Polygon

from . import blocking, geom
from .document import FACES, Block, BlockError
from .entities import REFERENCE, LineSeg, SplineSeg


def _patch(topo, region, corners=None):
    """Coons patch function (r, s arrays) -> points, for a region with four corners."""
    from ..mesh.preview import FaceEval, coons
    spec = blocking.region_spec(topo, region, corners)
    block, _ = blocking.resolve_block(topo, Block("_tmp", {f: [] for f in FACES}, region=spec))
    faces = {f: FaceEval(topo.doc, block.sides[f]) for f in FACES}
    return lambda r, s: coons(faces, np.atleast_1d(np.asarray(r, float)), np.atleast_1d(np.asarray(s, float)))


def _curve_through(pts):
    """A reference line if the points are collinear, otherwise a spline through them."""
    pts = np.asarray(pts, float)
    a, b = pts[0], pts[-1]
    d = b - a
    L = float(np.hypot(*d))
    off = np.abs((pts[:, 0] - a[0]) * d[1] - (pts[:, 1] - a[1]) * d[0]) / max(L, 1e-300)
    if off.max() <= 1e-9 * max(L, 1e-300):
        return LineSeg(None, kind=REFERENCE, p0=tuple(a), p1=tuple(b))
    return SplineSeg(None, kind=REFERENCE, points=[tuple(p) for p in pts])


def _iso(patch, rs, n=17):
    """Curve through the patch along the straight parameter path rs[0] -> rs[1]."""
    t = np.linspace(0.0, 1.0, n)
    r = rs[0][0] + t * (rs[1][0] - rs[0][0])
    s = rs[0][1] + t * (rs[1][1] - rs[0][1])
    return _curve_through(patch(r, s))


def split_nxm(topo, region, n, m, corners=None):
    """n x m blocks: n - 1 lines across the i direction and m - 1 across j."""
    if n < 1 or m < 1 or n * m < 2:
        raise BlockError("split into at least two blocks")
    patch = _patch(topo, region, corners)
    curves = [_iso(patch, ((k / n, 0.0), (k / n, 1.0))) for k in range(1, n)]
    curves += [_iso(patch, ((0.0, k / m), (1.0, k / m))) for k in range(1, m)]
    return curves


def ogrid(topo, region, size=0.5, corners=None):
    """An inner quadrilateral (size = its fraction of the patch) plus four spokes: five blocks."""
    if not 0.05 <= size <= 0.95:
        raise BlockError("the inner block size must be between 0.05 and 0.95")
    patch = _patch(topo, region, corners)
    a, b = (1.0 - size) / 2.0, (1.0 + size) / 2.0
    inner = [(a, a), (b, a), (b, b), (a, b)]
    outer = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
    curves = [_iso(patch, (inner[k], inner[(k + 1) % 4])) for k in range(4)]
    curves += [_iso(patch, (outer[k], inner[k])) for k in range(4)]
    return curves


def ogrid_hole(topo, region):
    """
    Four spokes from the region's corners towards the island's centre, ending where
    they meet the island: four blocks around it, meeting the island near square-on.
    """
    if len(region.holes) != 1:
        raise BlockError("O-grid around a hole needs a region with exactly one island in it")
    outer = blocking.auto_corners(topo, region.loop)
    if outer is None:
        raise BlockError("the region needs four corners")
    hole_poly = Polygon(region.hole_polygons[0])
    centre = (hole_poly.centroid.x, hole_poly.centroid.y)
    island = {topo.edges[e].curve_id for e, _ in region.holes[0]}
    verts = blocking.loop_vertices(topo, region.loop)
    spokes = []
    for k in outer:
        corner = tuple(verts[k])
        ray = LineSeg("_ray", p0=corner, p1=centre)
        hits = [p for cid in island for _, _, p in geom.intersect(ray, topo.doc.curves[cid])]
        if not hits:
            return _ogrid_hole_by_corners(topo, region, outer)
        hit = min(hits, key=lambda p: math.dist(p, corner))
        spokes.append(LineSeg(None, kind=REFERENCE, p0=corner, p1=hit))
    return spokes


def _ogrid_hole_by_corners(topo, region, outer):
    """Fallback: spokes from the region's corners to the island's own four corners."""
    hole = blocking.auto_corners(topo, region.holes[0])
    if hole is None:
        raise BlockError("the island needs four corners, or its centre must be visible from the region's corners")
    po = [blocking.loop_vertices(topo, region.loop)[k] for k in outer]
    ph = [blocking.loop_vertices(topo, region.holes[0])[k] for k in hole]
    centre = np.mean(ph, axis=0)
    ang = lambda p: math.atan2(p[1] - centre[1], p[0] - centre[0])
    po.sort(key=ang)
    ph.sort(key=ang)

    def cost(shift):
        return sum(abs((ang(po[k]) - ang(ph[(k + shift) % 4]) + math.pi) % (2 * math.pi) - math.pi)
                   for k in range(4))
    shift = min(range(4), key=cost)
    return [LineSeg(None, kind=REFERENCE, p0=tuple(po[k]), p1=tuple(ph[(k + shift) % 4])) for k in range(4)]


def centroid_split(topo, region):
    """Spokes from each side's midpoint to the region's centroid: one block per corner."""
    from ..mesh.preview import FaceEval
    if region.holes:
        raise BlockError("centroid split does not work around a hole (use O-grid around hole)")
    loop = blocking.Loop(topo, region.loop)
    corners = blocking.corner_candidates(topo, region.loop)
    if len(corners) < 3:
        raise BlockError("the region needs at least three corners")
    centroid = Polygon(region.polygon).centroid
    g = (centroid.x, centroid.y)
    curves = []
    for k, c in enumerate(corners):
        side = loop.refs(c, corners[(k + 1) % len(corners)])
        mid = FaceEval(topo.doc, side)([0.5])[0]
        curves.append(LineSeg(None, kind=REFERENCE, p0=tuple(mid), p1=g))
    return curves
