"""
The project document: the single source of truth that geometry.lua is generated from.

A Block is a four-sided CoonsPatch region. Each face is an ordered list of
CurveRefs; a face with several curves becomes an Eilmer Polyline, and a curve
traversed backwards is wrapped in a ReversedPath. Orientation follows Eilmer's
CoonsPatch convention: south and north run west->east, west and east run south->north.
"""
import json
import math
from dataclasses import dataclass, field

from .entities import REFERENCE, curve_from_dict

FACES = ("north", "east", "south", "west")
CORNER_TOL = 1.0e-9

DEFAULT_SMOOTHING = {"nsweeps": 500, "tol": 1.0e-10}


class BlockError(ValueError):
    pass


@dataclass
class CurveRef:
    """A curve, or the part of it between parameters t0 < t1, traversed forwards or reversed."""
    id: str
    reversed: bool = False
    t0: float = 0.0
    t1: float = 1.0

    def full(self):
        return self.t0 == 0.0 and self.t1 == 1.0

    def to_dict(self):
        d = {"id": self.id}
        if self.reversed:
            d["reversed"] = True
        if not self.full():
            d["t0"], d["t1"] = self.t0, self.t1
        return d

    @classmethod
    def from_dict(cls, d):
        return cls(d["id"], d.get("reversed", False), float(d.get("t0", 0.0)), float(d.get("t1", 1.0)))


@dataclass
class Block:
    id: str
    sides: dict                       # face -> list[CurveRef]
    niv: int = 21
    njv: int = 21
    patch: str = "CoonsPatch"         # "CoonsPatch" or "AOPatch"
    patch_opts: dict = field(default_factory=dict)   # e.g. {"nx": 10, "ny": 10} for AOPatch
    smoothing: dict = None            # e.g. DEFAULT_SMOOTHING; None -> unsmoothed
    cluster: dict = field(default_factory=dict)      # face -> {"type": "RobertsFunction", ...}
    fs_tag: str = "initial"
    # A block made from a sketch region: {"loop": [curve ids around the region, CCW],
    # "corners": [4 loop positions]}. Its sides are derived from the topology on demand.
    region: dict = None

    def to_dict(self):
        d = {"id": self.id, "niv": self.niv, "njv": self.njv,
             "patch": self.patch, "fs_tag": self.fs_tag}
        if self.region:
            d["region"] = self.region
        else:
            d["sides"] = {f: [r.to_dict() for r in self.sides[f]] for f in FACES}
        if self.patch_opts: d["patch_opts"] = self.patch_opts
        if self.smoothing: d["smoothing"] = self.smoothing
        if self.cluster: d["cluster"] = self.cluster
        return d

    @classmethod
    def from_dict(cls, d):
        sides = {f: [CurveRef.from_dict(r) for r in d.get("sides", {}).get(f, [])] for f in FACES}
        return cls(id=d["id"], sides=sides, niv=d["niv"], njv=d["njv"],
                   patch=d.get("patch", "CoonsPatch"), patch_opts=d.get("patch_opts", {}),
                   smoothing=d.get("smoothing"), cluster=d.get("cluster", {}),
                   fs_tag=d.get("fs_tag", "initial"), region=d.get("region"))

    def curve_ids(self):
        if self.region:
            return set(self.region["loop"])
        return {r.id for f in FACES for r in self.sides[f]}


@dataclass
class Dimension:
    """
    A dimension on one curve: "length" (line) or "angle" (arc sweep). Its value is
    always read from the curve; editing the value drives the curve.
    offset places the annotation: distance from the line (left of p0 -> p1), or
    from the arc (outwards).
    """
    id: str
    kind: str
    curve: str
    offset: float = 0.0

    def to_dict(self):
        return {"id": self.id, "kind": self.kind, "curve": self.curve, "offset": self.offset}

    @classmethod
    def from_dict(cls, d):
        return cls(d["id"], d["kind"], d["curve"], float(d.get("offset", 0.0)))


def _close(a, b, tol=CORNER_TOL):
    return math.hypot(a[0]-b[0], a[1]-b[1]) <= tol * max(1.0, abs(a[0]), abs(a[1]))


class Document:
    VERSION = 1

    def __init__(self):
        self.curves = {}      # id -> Curve, insertion ordered
        self.points = {}      # id -> (x, y): construction points (snap targets, not geometry)
        self.dimensions = {}  # id -> Dimension
        self.blocks = []      # list[Block]
        self.bcs = {}         # tag -> {"class": "InFlowBC_Supersonic", "args": "flowState=inflow"}
        self._counters = {}

    # --- construction -----------------------------------------------------
    def new_id(self, prefix):
        n = self._counters.get(prefix, 0)
        existing = set(self.curves) | set(self.points) | set(self.dimensions) | {b.id for b in self.blocks}
        while True:
            n += 1
            cid = f"{prefix}{n}"
            if cid not in existing:
                self._counters[prefix] = n
                return cid

    def add_curve(self, curve):
        if curve.id in self.curves:
            raise ValueError(f"duplicate curve id {curve.id}")
        self.curves[curve.id] = curve
        return curve

    def add_block(self, block):
        self.blocks.append(block)
        return block

    def add_point(self, pid, p):
        if pid in self.points or pid in self.curves:
            raise ValueError(f"duplicate id {pid}")
        self.points[pid] = (float(p[0]), float(p[1]))
        return pid

    def remove_blocks_using(self, ids):
        """Remove blocks with a face on any of the given curves; returns their ids."""
        ids = set(ids)
        dead = [b for b in self.blocks if b.curve_ids() & ids]
        self.blocks = [b for b in self.blocks if b not in dead]
        return [b.id for b in dead]

    def with_blocks(self, blocks):
        """A shallow copy with a different block list (e.g. resolved region blocks for codegen)."""
        other = Document()
        other.curves, other.points, other.dimensions, other.bcs = \
            self.curves, self.points, self.dimensions, self.bcs
        other.blocks = list(blocks)
        return other

    def remove_curves(self, ids):
        """Remove curves/points and any blocks that reference them; returns the removed block ids."""
        ids = set(ids)
        for cid in ids:
            self.curves.pop(cid, None)
            self.points.pop(cid, None)
            self.dimensions.pop(cid, None)
        for did in [d.id for d in self.dimensions.values() if d.curve not in self.curves]:
            del self.dimensions[did]                # dimensions go with their curve
        return self.remove_blocks_using(ids)

    def add_dimension(self, dim):
        self.dimensions[dim.id] = dim
        return dim

    # --- block geometry checks --------------------------------------------
    def ref_endpoints(self, ref):
        c = self.curves[ref.id]
        if ref.full():
            a, b = c.start(), c.end()
        else:
            from .geom import point_at
            a, b = point_at(c, ref.t0), point_at(c, ref.t1)
        return (b, a) if ref.reversed else (a, b)

    def face_endpoints(self, block, face):
        refs = block.sides[face]
        if not refs:
            raise BlockError(f"block {block.id}: {face} face has no curves")
        start, prev_end = self.ref_endpoints(refs[0])
        for ref in refs[1:]:
            s, e = self.ref_endpoints(ref)
            if not _close(prev_end, s):
                raise BlockError(f"block {block.id}: {face} face is not continuous at curve {ref.id}")
            prev_end = e
        return start, prev_end

    def validate_block(self, block):
        ends = {f: self.face_endpoints(block, f) for f in FACES}
        corners = [
            ("p00 (south-west)", ends["south"][0], ends["west"][0]),
            ("p10 (south-east)", ends["south"][1], ends["east"][0]),
            ("p01 (north-west)", ends["north"][0], ends["west"][1]),
            ("p11 (north-east)", ends["north"][1], ends["east"][1]),
        ]
        for name, a, b in corners:
            if not _close(a, b):
                raise BlockError(f"block {block.id}: open corner {name}: {a} vs {b}")
        if block.niv < 2 or block.njv < 2:
            raise BlockError(f"block {block.id}: niv and njv must be >= 2")
        for face in FACES:
            self.face_bc_tag(block, face)

    def face_bc_tag(self, block, face):
        """BC tag for a block face; reference curves are internal and never tagged."""
        tags = {self.curves[r.id].bc_tag for r in block.sides[face]
                if self.curves[r.id].kind != REFERENCE}
        tags.discard("")
        if len(tags) > 1:
            raise BlockError(f"block {block.id}: {face} face mixes BC tags {sorted(tags)}")
        return tags.pop() if tags else ""

    # --- serialisation ----------------------------------------------------
    def to_dict(self):
        return {
            "version": self.VERSION,
            "curves": [c.to_dict() for c in self.curves.values()],
            "points": [{"id": k, "x": x, "y": y} for k, (x, y) in self.points.items()],
            "dimensions": [d.to_dict() for d in self.dimensions.values()],
            "blocks": [b.to_dict() for b in self.blocks],
            "bcs": self.bcs,
        }

    @classmethod
    def from_dict(cls, d):
        doc = cls()
        for cd in d.get("curves", []):
            doc.add_curve(curve_from_dict(cd))
        for pd in d.get("points", []):
            doc.add_point(pd["id"], (pd["x"], pd["y"]))
        for dd in d.get("dimensions", []):
            doc.add_dimension(Dimension.from_dict(dd))
        for bd in d.get("blocks", []):
            doc.add_block(Block.from_dict(bd))
        doc.bcs = dict(d.get("bcs", {}))
        return doc

    def save(self, path):
        with open(path, "w") as f:
            json.dump(self.to_dict(), f, indent=2)

    @classmethod
    def load(cls, path):
        with open(path) as f:
            return cls.from_dict(json.load(f))
