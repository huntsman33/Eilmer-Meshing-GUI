"""
Blocks made from sketch regions.

A region block stores the cycle of curve ids around its region (counter-clockwise)
and four corner positions in that cycle. Each time the sketch changes it is
re-resolved against the current topology into concrete faces (CurveRefs, which
may cover only part of a curve), so dragging geometry re-meshes it.

Faces are then linked: opposite faces of a block share a vertex count (niv for
south/north, njv for east/west), and a face shared with another block must have
the same count, so Eilmer can join them (codegen connects faces shared along reference lines).
"""
import math
from dataclasses import dataclass, field

import numpy as np

from . import geom
from .document import FACES, Block, BlockError, CurveRef

CORNER_MIN_DEG = 15.0          # a loop vertex turning less than this is not a corner
OPPOSITE = {"south": "north", "north": "south", "east": "west", "west": "east"}
AXIS = {"south": "i", "north": "i", "east": "j", "west": "j"}


# --- loops ----------------------------------------------------------------------------

class Loop:
    """
    A region boundary as runs of one curve: consecutive topology edges along the
    same curve, in the same direction, are merged (a curve has no corner part-way
    along it). So a T-junction from outside the region does not change its loop.
    """

    def __init__(self, topo, edges):
        self.topo = topo
        groups = []
        for e, fwd in edges:
            if groups and self._continues(groups[-1], e, fwd):
                groups[-1]["edges"].append(e)
            else:
                groups.append({"curve": topo.edges[e].curve_id, "fwd": fwd, "edges": [e]})
        if len(groups) > 1 and self._continues(groups[-1], groups[0]["edges"][0], groups[0]["fwd"]):
            groups[0]["edges"] = groups[-1]["edges"] + groups[0]["edges"]
            groups.pop()
        self.groups = groups

    def _continues(self, group, e, fwd):
        edge, last = self.topo.edges[e], self.topo.edges[group["edges"][-1]]
        if edge.curve_id != group["curve"] or fwd != group["fwd"]:
            return False
        return abs(last.tb - edge.ta) < 1e-12 if fwd else abs(last.ta - edge.tb) < 1e-12

    def __len__(self):
        return len(self.groups)

    def signature(self):
        return [g["curve"] for g in self.groups]

    def rotated(self, k):
        out = Loop.__new__(Loop)
        out.topo, out.groups = self.topo, self.groups[k:] + self.groups[:k]
        return out

    def ref(self, k):
        g = self.groups[k]
        first, last = self.topo.edges[g["edges"][0]], self.topo.edges[g["edges"][-1]]
        ta, tb = (first.ta, last.tb) if g["fwd"] else (last.ta, first.tb)
        ta = 0.0 if ta <= 1e-12 else ta
        tb = 1.0 if tb >= 1.0 - 1e-12 else tb
        return CurveRef(g["curve"], not g["fwd"], ta, tb)

    def refs(self, start, stop):
        """Refs for groups start .. stop - 1, wrapping round the loop."""
        n = len(self.groups)
        count = (stop - start) % n or n
        return [self.ref((start + k) % n) for k in range(count)]

    def vertex(self, k):
        edge = self.topo.edges[self.groups[k]["edges"][0]]
        return np.asarray(self.topo.vertices[edge.v0 if self.groups[k]["fwd"] else edge.v1])

    def vertices(self):
        return [self.vertex(k) for k in range(len(self.groups))]

    def _point(self, e, fwd, frac):
        edge = self.topo.edges[e]
        t = edge.ta + frac * (edge.tb - edge.ta) if fwd else edge.tb - frac * (edge.tb - edge.ta)
        return np.asarray(geom.point_at(self.topo.doc.curves[edge.curve_id], t))

    def turning_angles(self):
        """Turning angle (degrees) at the start of each run."""
        out = []
        for k, g in enumerate(self.groups):
            prev = self.groups[k - 1]
            v = self.vertex(k)
            din = v - self._point(prev["edges"][-1], prev["fwd"], 1.0 - 1e-4)
            dout = self._point(g["edges"][0], g["fwd"], 1e-4) - v
            out.append(math.degrees(math.atan2(din[0]*dout[1] - din[1]*dout[0], din @ dout)))
        return out


def loop_vertices(topo, edges):
    return Loop(topo, edges).vertices()


def corner_candidates(topo, edges):
    """Run starts that turn by at least CORNER_MIN_DEG."""
    return [k for k, a in enumerate(Loop(topo, edges).turning_angles()) if abs(a) >= CORNER_MIN_DEG]


def auto_corners(topo, edges):
    """Four run positions for the block corners: the sharpest turns. None if impossible."""
    loop = Loop(topo, edges)
    if len(loop) < 4:
        return None
    if len(loop) == 4:
        return [0, 1, 2, 3]
    angles = loop.turning_angles()
    return sorted(sorted(range(len(loop)), key=lambda k: -abs(angles[k]))[:4])


def _rotate_to_south_west(loop, corners):
    """Rotate so corner 0 is the bottom-left-most corner (the south face runs from it)."""
    verts = loop.vertices()
    first = min(range(4), key=lambda k: (verts[corners[k]][0] + verts[corners[k]][1]))
    return corners[first:] + corners[:first]


# --- region blocks --------------------------------------------------------------------------

def region_spec(topo, region, corners=None):
    """The stored description of a block over a region (corners are run positions), or BlockError."""
    if region.holes:
        raise BlockError("the region has a hole; split it first (Sub-divide: O-grid around hole)")
    loop = Loop(topo, region.loop)
    if corners is None:
        corners = auto_corners(topo, region.loop)
        if corners is None:
            raise BlockError(f"the region has only {len(loop)} sides; a block needs 4 corners "
                             "(Sub-divide: centroid split)")
    if len(set(corners)) != 4:
        raise BlockError("a block needs four different corners")
    corners = _rotate_to_south_west(loop, sorted(corners))
    k0 = corners[0]
    return {"loop": loop.rotated(k0).signature(), "corners": [(c - k0) % len(loop) for c in corners]}


def find_region(topo, spec):
    """(region, its Loop rotated to match the spec) for a block spec, or (None, None)."""
    want = spec["loop"]
    n = len(want)
    for region in topo.regions:
        if region.holes:
            continue
        loop = Loop(topo, region.loop)
        if len(loop) != n:
            continue
        sig = loop.signature()
        for k in range(n):
            if sig[k:] + sig[:k] == want:
                return region, loop.rotated(k)
    return None, None


def _reverse(refs):
    return [CurveRef(r.id, not r.reversed, r.t0, r.t1) for r in reversed(refs)]


def _refs(topo, edges):
    """Refs for a list of topology edges, merging runs along one curve."""
    loop = Loop.__new__(Loop)
    loop.topo = topo
    groups = []
    for e, fwd in edges:
        if groups and Loop._continues(loop, groups[-1], e, fwd):
            groups[-1]["edges"].append(e)
        else:
            groups.append({"curve": topo.edges[e].curve_id, "fwd": fwd, "edges": [e]})
    loop.groups = groups
    return [loop.ref(k) for k in range(len(groups))]


def resolve_block(topo, block):
    """Copy of a region block with concrete sides, or BlockError if its region is gone."""
    region, loop = find_region(topo, block.region)
    if region is None:
        raise BlockError("its region no longer exists (the sketch around it changed)")
    c = sorted(block.region["corners"])
    faces = [loop.refs(c[k], c[(k + 1) % 4]) for k in range(4)]
    sides = {"south": faces[0], "east": faces[1], "north": _reverse(faces[2]), "west": _reverse(faces[3])}
    out = Block(**{**block.__dict__, "sides": sides})
    out.cluster = dict(block.cluster)
    return out, region


def face_length(doc, refs):
    return sum(geom.path_length(geom.subcurve(doc.curves[r.id], r.t0, r.t1) if not r.full()
                                else doc.curves[r.id]) for r in refs)


def default_counts(doc, block, region_area):
    """Vertex counts giving roughly square cells, about 10 across the block."""
    h = math.sqrt(max(region_area, 1e-300)) / 10.0
    i_len = 0.5 * (face_length(doc, block.sides["south"]) + face_length(doc, block.sides["north"]))
    j_len = 0.5 * (face_length(doc, block.sides["east"]) + face_length(doc, block.sides["west"]))
    return max(2, round(i_len / h) + 1), max(2, round(j_len / h) + 1)


# --- resolution of all blocks ----------------------------------------------------------------

@dataclass
class Resolution:
    blocks: list = field(default_factory=list)       # concrete, valid blocks (for Lua and preview)
    errors: dict = field(default_factory=dict)       # block id -> reason it cannot be meshed
    warnings: dict = field(default_factory=dict)     # block id -> [messages]
    polygons: dict = field(default_factory=dict)     # block id -> outline (k, 2) for display
    groups: dict = field(default_factory=dict)       # (block id, axis) -> group id

    def by_id(self):
        return {b.id: b for b in self.blocks}


def face_key(refs):
    return frozenset((r.id, round(min(r.t0, r.t1), 9), round(max(r.t0, r.t1), 9)) for r in refs)


def _outline(doc, block):
    from ..mesh.preview import FaceEval
    pts = []
    t = np.linspace(0.0, 1.0, 24)
    for face, rev in (("south", False), ("east", False), ("north", True), ("west", True)):
        p = FaceEval(doc, block.sides[face])(t[::-1] if rev else t)
        pts.append(p[:-1])
    return np.vstack(pts)


def resolve(doc, topo):
    """Resolve every block; harmonise counts across shared faces; collect problems."""
    res = Resolution()
    for b in doc.blocks:
        try:
            if b.region:
                concrete, region = resolve_block(topo, b)
                res.polygons[b.id] = region.polygon
            else:
                concrete = Block(**{**b.__dict__})
                doc.validate_block(concrete)
                res.polygons[b.id] = _outline(doc, concrete)
            doc.validate_block(concrete)
            res.blocks.append(concrete)
        except BlockError as e:
            res.errors[b.id] = str(e)

    # Link faces: opposite faces share an axis; identical faces of two blocks share it too.
    parent = {}

    def find(a):
        parent.setdefault(a, a)
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    faces_by_key = {}
    for b in res.blocks:
        for face in FACES:
            faces_by_key.setdefault(face_key(b.sides[face]), []).append((b, face))
    for shared in faces_by_key.values():
        for (b0, f0), (b1, f1) in zip(shared, shared[1:]):
            parent[find((b0.id, AXIS[f0]))] = find((b1.id, AXIS[f1]))
            if any(doc.curves[r.id].kind != "reference" for r in b0.sides[f0]):
                res.warnings.setdefault(b0.id, []).append(
                    f"its {f0} face is a solid curve shared with block {b1.id}, so it is a wall between "
                    "them (make it a dashed reference line to let the flow through)")
    for b in res.blocks:
        for axis in "ij":
            res.groups[(b.id, axis)] = find((b.id, axis))

    # Harmonise counts in each group (the largest wins), on the concrete copies only.
    best = {}
    for b in res.blocks:
        best[res.groups[(b.id, "i")]] = max(best.get(res.groups[(b.id, "i")], 0), b.niv)
        best[res.groups[(b.id, "j")]] = max(best.get(res.groups[(b.id, "j")], 0), b.njv)
    for b in res.blocks:
        b.niv, b.njv = best[res.groups[(b.id, "i")]], best[res.groups[(b.id, "j")]]

    _check_conformity(doc, res)
    return res


def _check_conformity(doc, res):
    """Flag faces that overlap another block's face without matching it exactly."""
    pieces = {}
    for b in res.blocks:
        for face in FACES:
            key = face_key(b.sides[face])
            for r in b.sides[face]:
                pieces.setdefault(r.id, []).append((min(r.t0, r.t1), max(r.t0, r.t1), key, b.id, face))
    for items in pieces.values():
        for i, (a0, a1, ka, ba, fa) in enumerate(items):
            for b0, b1, kb, bb, fb in items[i + 1:]:
                overlap = min(a1, b1) - max(a0, b0)
                if ba != bb and overlap > 1e-9 and ka != kb:
                    for bid, face, other in ((ba, fa, bb), (bb, fb, ba)):
                        res.warnings.setdefault(bid, []).append(
                            f"its {face} face only partly matches block {other}; Eilmer needs whole "
                            "faces to match (add a reference line so the blocks meet corner to corner)")


def linked_faces(res, block_id, face):
    """(block, face) pairs of other blocks sharing this exact face."""
    blocks = res.by_id()
    key = face_key(blocks[block_id].sides[face])
    return [(b, f) for b in res.blocks for f in FACES
            if b.id != block_id and face_key(b.sides[f]) == key]
