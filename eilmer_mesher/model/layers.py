"""
Boundary layers: thin blocks along walls whose cells grow geometrically away
from the wall, so the first cell has a chosen height (e.g. from a y+ target).

Applying a layer to some walls (block faces on solid curves):

  1. offsets each wall into its block by the layer thickness
         delta = h (r^N - 1) / (r - 1)       h first cell, r growth ratio, N cells
     as a dashed reference curve (a concentric arc for arcs, a line for lines,
     otherwise a spline) ending on the block's two side faces, delta along them
     from the wall;
  2. continues each cut through neighbouring blocks across dashed lines, along an
     iso-line of the neighbour's Coons patch, so every interface still matches face
     to face; it stops at solid curves or where it closes on itself;
  3. re-blocks the split regions: across the layer N cells with the first one h at
     the wall; the outer part grades out from the last layer cell; along the wall,
     the counts and clustering of the block each piece came from.

Its walls are tagged as no-slip walls (a viscous layer on a slip wall makes no sense):
untagged and slip-wall curves get an existing no-slip tag, or a new "wall" tag
(WallBC_NoSlip_Adiabatic). Other tags, such as inflows, are left alone.

A layer keeps the blocks it replaced, so it can be applied again with new settings
(layers applied after it come off and go back on top) or removed.
"""
import math
from dataclasses import dataclass

import numpy as np
from scipy.optimize import brentq, minimize_scalar
from shapely.geometry import Point, Polygon

from . import blocking
from . import geom
from .document import FACES, Block, BlockError, CurveRef
from .entities import REFERENCE, ArcSeg

# wall face -> (side face at the wall's start, at its end), each with whether it runs away from the wall
SIDES = {
    "south": (("west", True), ("east", True)),
    "north": (("west", False), ("east", False)),
    "west": (("south", True), ("north", True)),
    "east": (("south", False), ("north", False)),
}
ISO_END = {"west": lambda u: (0.0, u), "east": lambda u: (1.0, u),
           "south": lambda u: (u, 0.0), "north": lambda u: (u, 1.0)}
MAX_FRACTION = 0.9          # a layer may take at most this much of a side face


# --- spacing ----------------------------------------------------------------------------

def thickness(first, growth, cells):
    """Total height of `cells` cells starting at `first` and growing by `growth`."""
    if abs(growth - 1.0) < 1e-12:
        return first * cells
    return first * (growth ** cells - 1.0) / (growth - 1.0)


def _log_expm1(x):
    return x + math.log(-math.expm1(-x)) if x > 30.0 else math.log(math.expm1(x))


def growth_ratio(a, n):
    """Ratio r >= 1 for n cells growing from a first cell a (a fraction of the edge) to fill it.
    1 when even uniform cells are smaller than a."""
    if a <= 0.0 or n < 1:
        raise ValueError("the first cell and the cell count must be positive")
    if n == 1 or a * n >= 1.0 - 1e-12:
        return 1.0

    def f(q):                                   # log(a * (r^n - 1) / (r - 1)), r = 1 + q
        return math.log(a) + _log_expm1(n * math.log1p(q)) - math.log(q)
    hi = 1.0
    while f(hi) < 0.0:
        hi *= 2.0
    return 1.0 + brentq(f, 1e-12, hi, xtol=1e-15, rtol=1e-15, maxiter=500)


def growth_values(r, n, t, reverse=False):
    """The growth distribution at parameters t: vertex k of n cells at (r^k - 1) / (r^n - 1)."""
    t = np.asarray(t, float)
    if reverse:
        t = 1.0 - t
    R = r ** n
    x = (np.power(R, t) - 1.0) / (R - 1.0)
    return 1.0 - x if reverse else x


def first_cell_height(yplus, velocity, density, viscosity, length):
    """Wall spacing for a y+ target, using the turbulent flat-plate skin friction
    Cf = 0.026 Re_x^(-1/7) at x = length."""
    if min(yplus, velocity, density, viscosity, length) <= 0:
        raise ValueError("all the flow values must be positive")
    re = density * velocity * length / viscosity
    cf = 0.026 / re ** (1.0 / 7.0)
    u_tau = velocity * math.sqrt(0.5 * cf)
    return yplus * viscosity / (density * u_tau)


def concrete_cluster(doc, block, face):
    """
    A face's clustering as Lua and the preview use it. A "Growth" spec (first cell
    height in metres, from the start, or from the end if reverse) becomes a ratio
    for the face's actual length and cell count, so it survives count changes.
    """
    spec = block.cluster.get(face)
    if not spec or spec.get("type") != "Growth":
        return spec
    n = (block.niv if face in ("south", "north") else block.njv) - 1
    length = blocking.face_length(doc, block.sides[face])
    r = growth_ratio(spec["first"] / length, n) if length > 0 else 1.0
    if r < 1.0 + 1e-9:
        return None
    return {"type": "growth", "r": r, "n": n, "reverse": bool(spec.get("reverse", False))}


def mirror_cluster(spec):
    """The same clustering seen from the other end of the edge."""
    if not spec:
        return spec
    if spec.get("type") == "RobertsFunction":
        return {**spec, "end0": spec.get("end1", False), "end1": spec.get("end0", False)}
    if spec.get("type") == "Growth":
        return {**spec, "reverse": not spec.get("reverse", False)}
    return spec


# --- geometry helpers ------------------------------------------------------------------------

def _fe(doc, refs):
    from ..mesh.preview import FaceEval
    return FaceEval(doc, refs)


def _same(a, b):
    return math.dist(a, b) <= 1e-8 * max(1.0, abs(a[0]), abs(a[1]))


def _arc_lengths(fe, n=4001):
    t = np.linspace(0.0, 1.0, n)
    seg = np.hypot(*np.diff(fe(t), axis=0).T)
    return t, seg, np.concatenate([[0.0], np.cumsum(seg)])


def _along(fe, dist, from_start):
    """The point at arc length dist along a face, from its start or its end."""
    t, seg, cum = _arc_lengths(fe)
    target = dist if from_start else cum[-1] - dist
    k = int(np.clip(np.searchsorted(cum, target), 1, len(cum) - 1))
    frac = (target - cum[k - 1]) / seg[k - 1] if seg[k - 1] > 0 else 0.0
    return tuple(float(v) for v in fe([t[k - 1] + frac * (t[k] - t[k - 1])])[0])


def _param(fe, p):
    """Face parameter of the point on it nearest p."""
    t = np.linspace(0.0, 1.0, 2001)
    k = int(np.argmin(np.hypot(*(fe(t) - p).T)))
    lo, hi = t[max(k - 1, 0)], t[min(k + 1, len(t) - 1)]
    res = minimize_scalar(lambda u: float(np.hypot(*(fe([u])[0] - p))), bounds=(lo, hi),
                          method="bounded", options={"xatol": 1e-13})
    return float(res.x)


def _ccw(doc, block):
    poly = blocking._outline(doc, block)
    x, y = poly[:, 0], poly[:, 1]
    return float(np.sum(x * np.roll(y, -1) - np.roll(x, -1) * y)) > 0.0


def _offset_curves(doc, refs, left, delta, e0, e1):
    """Reference curves from e0 to e1, delta from the wall (refs) on its left or right side."""
    from .subdivide import _curve_through
    fe = _fe(doc, refs)
    walls = [doc.curves[r.id] for r in refs]
    if all(c.type == "arc" for c in walls):
        c0 = np.asarray(walls[0].centre, float)
        r0, r1 = math.dist(e0, c0), math.dist(e1, c0)
        if all(_same(c.centre, c0) for c in walls) and abs(r0 - r1) <= 1e-9 * max(1.0, r0):
            d = fe(np.linspace(0.0, 1.0, 65)) - c0
            th = np.unwrap(np.arctan2(d[:, 1], d[:, 0]))
            wall_sweep = th[-1] - th[0]
            a0 = math.atan2(e0[1] - c0[1], e0[0] - c0[0])
            a1 = math.atan2(e1[1] - c0[1], e1[0] - c0[0])
            sweep = wall_sweep + ((a1 - a0 - wall_sweep + math.pi) % (2.0 * math.pi) - math.pi)
            return [ArcSeg(None, kind=REFERENCE, p0=p, p1=q, centre=c)
                    for p, q, c in geom.arc_pieces(c0, r0, a0, sweep, e0, e1)]
    n_pts = sum(len(c.points) for c in walls if c.type == "spline")
    n = int(np.clip(max(33, n_pts), 33, 201))
    t = np.linspace(0.0, 1.0, n)
    h = 1e-6
    tangent = fe(np.minimum(t + h, 1.0)) - fe(np.maximum(t - h, 0.0))
    tangent /= np.hypot(*tangent.T)[:, None]
    normal = np.stack([-tangent[:, 1], tangent[:, 0]], axis=1) * (1.0 if left else -1.0)
    P = fe(t) + delta * normal
    c0, c1 = np.asarray(e0) - P[0], np.asarray(e1) - P[-1]      # land exactly on the side faces
    P += (1.0 - t)[:, None] * c0 + t[:, None] * c1
    P[0], P[-1] = e0, e1
    return [_curve_through(P)]


def _iso_curve(doc, block, f, s0, g, s1, p, q, n=33):
    """Curve across a block from face f (parameter s0) to the opposite face g (s1), through its Coons patch."""
    from ..mesh.preview import coons
    from .subdivide import _curve_through
    faces = {face: _fe(doc, block.sides[face]) for face in FACES}
    a, b = ISO_END[f](s0), ISO_END[g](s1)
    t = np.linspace(0.0, 1.0, n)
    P = coons(faces, a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]))
    P[0], P[-1] = p, q
    return _curve_through(P)


@dataclass(eq=False)
class _Split:
    """Where a cut crosses a block face: the thin part runs from anchor to point, the rest to far."""
    point: tuple
    anchor: tuple
    far: tuple


def _is_reference(doc, refs):
    return all(doc.curves[r.id].kind == REFERENCE for r in refs)


# --- walls ---------------------------------------------------------------------------------

def wall_spec(res, bid, face):
    """What a layer stores for a wall: the face's curves and a point inside its block."""
    from shapely.geometry import Polygon as P
    block = res.by_id()[bid]
    inside = P(res.polygons[bid]).representative_point()
    return {"refs": [r.to_dict() for r in block.sides[face]], "inside": [inside.x, inside.y]}


def find_wall(doc, res, wall):
    """(concrete block, face) for a stored wall, or BlockError."""
    refs = [CurveRef.from_dict(r) for r in wall["refs"]]
    if any(r.id not in doc.curves for r in refs):
        raise BlockError("a wall of the boundary layer has been deleted")
    if _is_reference(doc, refs):
        raise BlockError("a dashed reference line is not a wall")
    key = blocking.face_key(refs)
    cands = [(b, f) for b in res.blocks for f in FACES if blocking.face_key(b.sides[f]) == key]
    if not cands:
        raise BlockError("a wall of the boundary layer is no longer a block face (the sketch or blocks changed)")
    inside = wall.get("inside")
    if inside is not None and len(cands) > 1:            # a plate with blocks on both sides
        for b, f in cands:
            if Polygon(res.polygons[b.id]).contains(Point(inside)):
                return b, f
    return cands[0]


# --- applying --------------------------------------------------------------------------------

def apply(doc, layer):
    """Cut the layer's walls into thin blocks, in place (layer.curves/blocks/base are filled in).
    Raises BlockError, leaving doc part-changed: callers run it inside an undoable edit."""
    from .topology import Topology
    topo = Topology(doc)
    res = blocking.resolve(doc, topo)
    delta = thickness(layer.first, layer.growth, layer.cells)
    if not delta > 0.0:
        raise BlockError("the layer needs a positive first cell height")
    last = layer.first * layer.growth ** (layer.cells - 1)
    key = blocking.face_key

    walls = [find_wall(doc, res, w) for w in layer.walls]
    _tag_walls(doc, layer, walls)
    parents = {}                        # block id -> concrete block that gets cut
    splits = {}                         # face key -> [_Split]
    new_curves = []                     # (curve, wall-parallel clustering along the curve)

    def split_on(b, g, away, corner):
        k = key(b.sides[g])
        for sp in splits.get(k, []):
            if _same(sp.anchor, corner):
                return sp.point                         # the neighbouring wall block made it already
        fe = _fe(doc, b.sides[g])
        length = _arc_lengths(fe)[2][-1]
        if delta >= MAX_FRACTION * length:
            raise BlockError(f"the layer ({delta:.4g} m thick) does not fit in block {b.id}: "
                             f"its {g} face is only {length:.4g} m long")
        sp = _Split(_along(fe, delta, away), tuple(corner), tuple(fe([1.0 if away else 0.0])[0]))
        splits.setdefault(k, []).append(sp)
        return sp.point

    # 1. offset every wall into its block
    for b, f in walls:
        parents[b.id] = b
        fe = _fe(doc, b.sides[f])
        corners = (tuple(fe([0.0])[0]), tuple(fe([1.0])[0]))
        ends = [split_on(b, g, away, c) for (g, away), c in zip(SIDES[f], corners)]
        left = (f in ("south", "east")) == _ccw(doc, b)
        curves = _offset_curves(doc, b.sides[f], left, delta, ends[0], ends[1])
        for c in curves:
            new_curves.append((c, b.cluster.get(f) if len(curves) == 1 else None))

    # 2. carry the cuts on through neighbours joined across dashed lines
    wall_blocks = set(parents)
    used = set()
    changed = True
    while changed:
        changed = False
        for n in res.blocks:
            if n.id in wall_blocks:
                continue
            for f, g in (("west", "east"), ("east", "west"), ("south", "north"), ("north", "south")):
                if not _is_reference(doc, n.sides[f]):
                    continue                            # a solid face: nothing flows across it
                for sp in list(splits.get(key(n.sides[f]), [])):
                    if (n.id, id(sp)) in used:
                        continue
                    fe_f, fe_g = _fe(doc, n.sides[f]), _fe(doc, n.sides[g])
                    s0 = _param(fe_f, sp.point)
                    at_start = _same(fe_f([0.0])[0], sp.anchor)
                    match = None
                    for q in splits.get(key(n.sides[g]), []):
                        if (n.id, id(q)) not in used:
                            s1 = _param(fe_g, q.point)
                            if abs(s1 - s0) < 0.2 and (match is None or abs(s1 - s0) < abs(match[1] - s0)):
                                match = (q, s1)
                    if match is None:
                        q, s1 = _Split(tuple(fe_g([s0])[0]), tuple(fe_g([0.0 if at_start else 1.0])[0]),
                                       tuple(fe_g([1.0 if at_start else 0.0])[0])), s0
                        splits.setdefault(key(n.sides[g]), []).append(q)
                    else:
                        q, s1 = match
                    used.update({(n.id, id(sp)), (n.id, id(q))})
                    parents[n.id] = n
                    if f in ("west", "east"):
                        perp = "south" if at_start else "north"
                    else:
                        perp = "west" if at_start else "east"
                    spec = n.cluster.get(perp)
                    if f in ("east", "north"):
                        spec = mirror_cluster(spec)
                    new_curves.append((_iso_curve(doc, n, f, s0, g, s1, sp.point, q.point), spec))
                    changed = True

    # A cut may only touch existing curves at its two ends.
    for c, _ in new_curves:
        for other in doc.curves.values():
            for t, _, p in geom.intersect(c, other):
                if 1e-6 < t < 1.0 - 1e-6:
                    raise BlockError(f"the layer ({delta:.4g} m thick) would cross curve {other.id} at "
                                     f"({p[0]:.4g}, {p[1]:.4g}); make it thinner")

    # 3. swap the cut blocks for blocks over the new regions
    stored = {b.id: b for b in doc.blocks}
    layer.base = [stored[b.id].to_dict() for b in doc.blocks if b.id in parents]
    doc.blocks = [b for b in doc.blocks if b.id not in parents]
    along = {}
    for c, spec in new_curves:
        c.id = doc.new_id("c")
        doc.add_curve(c)
        along[c.id] = spec
    layer.curves = list(along)

    new_topo = Topology(doc)
    outlines = {bid: Polygon(res.polygons[bid]) for bid in parents}
    made = []
    covered = 0.0
    for region in new_topo.regions:
        probe = Polygon(region.polygon).representative_point()
        owner = next((bid for bid, poly in outlines.items() if poly.contains(probe)), None)
        if owner is None:
            continue
        parent = stored[owner]
        try:
            spec = blocking.region_spec(new_topo, region)
        except BlockError as e:
            raise BlockError(f"part of block {owner} cannot be blocked after the cut ({e})")
        blk = Block(doc.new_id("blk"), {f: [] for f in FACES}, patch=parent.patch,
                    patch_opts=dict(parent.patch_opts), smoothing=parent.smoothing,
                    fs_tag=parent.fs_tag, region=spec)
        concrete, _ = blocking.resolve_block(new_topo, blk)
        made.append((blk, concrete, parents[owner]))
        covered += region.area
    total = sum(p.area for p in outlines.values())
    if abs(covered - total) > 1e-3 * total:       # (region polygons are sampled, so not exact)
        raise BlockError("the layer cuts across other curves (is it thicker than the gap to the next wall?)")

    # 4. counts and clustering
    splits_all = [sp for group in splits.values() for sp in group]
    for blk, concrete, parent in made:
        _assign(doc, blk, concrete, parent, splits_all, along, layer, last)
        doc.add_block(blk)
    _spread_clusters(doc, made)

    final = blocking.resolve(doc, new_topo)
    for blk, _, _ in made:
        if blk.id in final.errors:
            raise BlockError(f"block {blk.id} made by the layer cannot be meshed: {final.errors[blk.id]}")
    layer.blocks = [blk.id for blk, _, _ in made]


def _ends(doc, refs):
    fe = _fe(doc, refs)
    return tuple(fe([0.0])[0]), tuple(fe([1.0])[0])


def _assign(doc, blk, concrete, parent, splits, along, layer, last):
    """Counts and clustering for one new block, from the layer's splits and the block it came from."""
    role = {}
    parent_faces = {blocking.face_key(parent.sides[f]): f for f in FACES}
    for f in FACES:
        a, z = _ends(doc, concrete.sides[f])
        for sp in splits:
            if {_key(a), _key(z)} == {_key(sp.anchor), _key(sp.point)}:
                role[f] = "thin"
                blk.cluster[f] = {"type": "Growth", "first": layer.first, "reverse": not _same(a, sp.anchor)}
                break
            if {_key(a), _key(z)} == {_key(sp.point), _key(sp.far)}:
                role[f] = "outer"
                if layer.transition:
                    blk.cluster[f] = {"type": "Growth", "first": last, "reverse": not _same(a, sp.point)}
                break
        else:
            pf = parent_faces.get(blocking.face_key(concrete.sides[f]))
            refs = concrete.sides[f]
            if pf is not None and parent.cluster.get(pf):
                same = _same(a, _ends(doc, parent.sides[pf])[0])
                blk.cluster[f] = dict(parent.cluster[pf] if same else mirror_cluster(parent.cluster[pf]))
            elif len(refs) == 1 and refs[0].full() and along.get(refs[0].id):
                spec = along[refs[0].id]
                blk.cluster[f] = dict(mirror_cluster(spec) if refs[0].reversed else spec)

    for attr, faces in (("niv", ("south", "north")), ("njv", ("west", "east"))):
        if any(role.get(f) == "thin" for f in faces):
            setattr(blk, attr, layer.cells + 1)
            continue
        n = _parent_count(doc, concrete, faces[0], parent)
        if layer.transition:
            for f in faces:
                if role.get(f) == "outer":                   # enough cells to grade out gently
                    n = max(n, outer_cells(blocking.face_length(doc, concrete.sides[f]), last, layer.growth) + 1)
        setattr(blk, attr, n)


def outer_cells(length, last, growth):
    """Cells to grade from the last layer cell across length, growing by at most max(growth, 1.1)."""
    g = max(growth, 1.1)
    if length <= last:
        return 1
    return max(1, math.ceil(math.log1p(length * (g - 1.0) / last) / math.log(g) - 1e-9))


def _key(p):
    return (round(p[0], 8), round(p[1], 8))


def _parent_count(doc, concrete, face, parent):
    """Vertex count of the parent along the direction this face runs."""
    def direction(refs):
        a, z = _ends(doc, refs)
        d = np.subtract(z, a)
        return d / (np.hypot(*d) or 1.0)
    d = direction(concrete.sides[face])
    best = max(FACES, key=lambda f: abs(float(direction(parent.sides[f]) @ d)))
    return parent.niv if best in ("south", "north") else parent.njv


def _spread_clusters(doc, made):
    """Faces without clustering take it from a shared face, else from the opposite face, so
    vertices still match across interfaces and the layer runs straight through each block."""
    faces = {}
    for blk, concrete, _ in made:
        for f in FACES:
            faces.setdefault(blocking.face_key(concrete.sides[f]), []).append((blk, concrete, f))
    changed = True
    while changed:
        changed = False
        for group in faces.values():
            have = [(b, c, f) for b, c, f in group if f in b.cluster]
            if not have:
                continue
            b0, c0, f0 = have[0]
            start = _ends(doc, c0.sides[f0])[0]
            for b, c, f in group:
                if f not in b.cluster:
                    same = _same(_ends(doc, c.sides[f])[0], start)
                    b.cluster[f] = dict(b0.cluster[f0] if same else mirror_cluster(b0.cluster[f0]))
                    changed = True
        for blk, _, _ in made:
            for f in FACES:
                o = blocking.OPPOSITE[f]
                if f not in blk.cluster and o in blk.cluster:
                    blk.cluster[f] = dict(blk.cluster[o])
                    changed = True


NO_SLIP = ("WallBC_NoSlip_Adiabatic", "WallBC_NoSlip_FixedT")


def no_slip_tag(doc):
    """A tag for viscous walls: an existing no-slip one, else a new adiabatic "wall"."""
    existing = sorted(t for t, spec in doc.bcs.items() if spec["class"] in NO_SLIP)
    if existing:
        return "wall" if "wall" in existing else existing[0]
    tag = "wall" if "wall" not in doc.bcs else "noslip_wall"
    k = 2
    while tag in doc.bcs:
        tag, k = f"noslip_wall{k}", k + 1
    doc.bcs[tag] = {"class": "WallBC_NoSlip_Adiabatic", "args": ""}
    return tag


def _tag_walls(doc, layer, walls):
    """Tag the walls' slip (or untagged) curves as no-slip walls, remembering what they were."""
    slip = {""} | {t for t, spec in doc.bcs.items() if spec["class"] == "WallBC_WithSlip"}
    ids = [r.id for b, f in walls for r in b.sides[f] if doc.curves[r.id].bc_tag in slip]
    if not ids:
        return
    tag = no_slip_tag(doc)
    for cid in dict.fromkeys(ids):
        layer.tagged.setdefault(cid, [doc.curves[cid].bc_tag, tag])
        doc.curves[cid].bc_tag = tag


def unapply(doc, layer):
    """Take a layer off: remove its curves and blocks, put back the blocks it replaced and the
    wall tags it changed (unless they have been changed again since)."""
    for cid, (old, new) in layer.tagged.items():
        if cid in doc.curves and doc.curves[cid].bc_tag == new:
            doc.curves[cid].bc_tag = old if (old == "" or old in doc.bcs) else ""
    layer.tagged = {}
    dead = set(layer.curves)
    for cid in dead:
        doc.curves.pop(cid, None)
    doc.dimensions = {k: d for k, d in doc.dimensions.items() if d.curve in doc.curves}
    gone = set(layer.blocks)
    doc.blocks = [b for b in doc.blocks if b.id not in gone and not (b.curve_ids() & dead)]
    doc.blocks.extend(Block.from_dict(d) for d in layer.base)
    layer.curves, layer.blocks, layer.base = [], [], []


def redo_layers(doc, lid, settings=None, remove=False):
    """Change (or remove) layer lid: later layers come off first and go back on afterwards."""
    k = next(i for i, layer in enumerate(doc.layers) if layer.id == lid)
    later = doc.layers[k:]
    for layer in reversed(later):
        unapply(doc, layer)
    if remove:
        doc.layers.pop(k)
        later = later[1:]
    else:
        for name, value in (settings or {}).items():
            setattr(doc.layers[k], name, value)
    for layer in later:
        apply(doc, layer)


def layer_of_face(doc, res, bid, face):
    """The layer whose wall this block face is (on one of the layer's own blocks), or None."""
    block = res.by_id().get(bid)
    if block is None:
        return None
    k = blocking.face_key(block.sides[face])
    for layer in doc.layers:
        if bid in layer.blocks and any(
                blocking.face_key([CurveRef.from_dict(r) for r in w["refs"]]) == k for w in layer.walls):
            return layer
    return None
