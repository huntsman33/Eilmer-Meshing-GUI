"""
Sketch curve entities.

Each curve maps one-to-one onto an Eilmer Lua path (and a gridgen Path for the
Python preview). Points are stored as (x, y) tuples; z is always 0 in 2D.
A curve's kind is either "geometry" (solid line, may carry a BC tag) or
"reference" (dashed line, an internal block interface).
"""
import math
from dataclasses import dataclass, field

from gridgen.path import Line, Arc, Bezier, Spline

GEOMETRY = "geometry"
REFERENCE = "reference"


def _pt(p):
    return (float(p[0]), float(p[1]))


@dataclass
class Curve:
    id: str
    kind: str = GEOMETRY
    bc_tag: str = ""        # empty -> untagged -> Eilmer default WallBC_WithSlip

    type = None             # set by subclasses

    def start(self):
        raise NotImplementedError

    def end(self):
        raise NotImplementedError

    def to_gridgen(self):
        raise NotImplementedError

    def lua_args(self, name_of):
        """Lua constructor arguments, with points named by name_of(point)."""
        raise NotImplementedError

    def lua_class(self):
        raise NotImplementedError

    def geometry_dict(self):
        raise NotImplementedError

    def to_dict(self):
        d = {"id": self.id, "type": self.type, "kind": self.kind}
        if self.bc_tag:
            d["bc_tag"] = self.bc_tag
        d.update(self.geometry_dict())
        return d

    def sample(self, n=64):
        path = self.to_gridgen()
        return [(p.x, p.y) for p in (path(i/(n-1)) for i in range(n))]


@dataclass
class LineSeg(Curve):
    p0: tuple = (0.0, 0.0)
    p1: tuple = (0.0, 0.0)

    type = "line"

    def start(self): return self.p0
    def end(self): return self.p1

    def to_gridgen(self):
        return Line(self.p0, self.p1)

    def lua_class(self): return "Line"

    def lua_args(self, name_of):
        return f"p0={name_of(self.p0)}, p1={name_of(self.p1)}"

    def geometry_dict(self):
        return {"p0": list(self.p0), "p1": list(self.p1)}


@dataclass
class ArcSeg(Curve):
    """Circular arc from p0 to p1 about centre, sweeping less than pi (as Eilmer's Arc)."""
    p0: tuple = (0.0, 0.0)
    p1: tuple = (0.0, 0.0)
    centre: tuple = (0.0, 0.0)

    type = "arc"

    def start(self): return self.p0
    def end(self): return self.p1

    def radius(self):
        return math.hypot(self.p0[0]-self.centre[0], self.p0[1]-self.centre[1])

    def to_gridgen(self):
        return Arc(self.p0, self.p1, self.centre)

    def lua_class(self): return "Arc"

    def lua_args(self, name_of):
        return f"p0={name_of(self.p0)}, p1={name_of(self.p1)}, centre={name_of(self.centre)}"

    def geometry_dict(self):
        return {"p0": list(self.p0), "p1": list(self.p1), "centre": list(self.centre)}


@dataclass
class BezierSeg(Curve):
    points: list = field(default_factory=list)

    type = "bezier"

    def start(self): return self.points[0]
    def end(self): return self.points[-1]

    def to_gridgen(self):
        return Bezier(self.points)

    def lua_class(self): return "Bezier"

    def lua_args(self, name_of):
        return "points={" + ", ".join(name_of(p) for p in self.points) + "}"

    def geometry_dict(self):
        return {"points": [list(p) for p in self.points]}


@dataclass
class SplineSeg(Curve):
    """Interpolating cubic spline through points (Eilmer Spline)."""
    points: list = field(default_factory=list)

    type = "spline"

    def start(self): return self.points[0]
    def end(self): return self.points[-1]

    def to_gridgen(self):
        return Spline(self.points)

    def lua_class(self): return "Spline"

    def lua_args(self, name_of):
        return "points={" + ", ".join(name_of(p) for p in self.points) + "}"

    def geometry_dict(self):
        return {"points": [list(p) for p in self.points]}


CURVE_TYPES = {cls.type: cls for cls in (LineSeg, ArcSeg, BezierSeg, SplineSeg)}


def curve_from_dict(d):
    cls = CURVE_TYPES[d["type"]]
    common = {"id": d["id"], "kind": d.get("kind", GEOMETRY), "bc_tag": d.get("bc_tag", "")}
    if cls in (LineSeg, ArcSeg):
        geom = {k: _pt(d[k]) for k in ("p0", "p1", "centre") if k in d}
    else:
        geom = {"points": [_pt(p) for p in d["points"]]}
    return cls(**common, **geom)
