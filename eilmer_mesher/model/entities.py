"""
Sketch curve entities.

Each curve maps one-to-one onto an Eilmer Lua path (and a gridgen Path for the
Python preview). Points are stored as (x, y) tuples; z is always 0 in 2D.
A curve's kind is one of
  "geometry"   solid line: the domain's boundaries and walls; may carry a BC tag
  "split"      dash-dot line: an interface between blocks (Eilmer joins blocks across it)
  "reference"  dashed line: a drawing aid (mirror axes, offset sources...); the mesher ignores it
"""
import math
from dataclasses import dataclass, field, replace

from gridgen.path import Line, Arc

GEOMETRY = "geometry"
REFERENCE = "reference"
SPLIT = "split"


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

    def lua_expr(self, name_of):
        """Complete Lua constructor expression for this path."""
        return f"{self.lua_class()}:new{{{self.lua_args(name_of)}}}"

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

    def copy(self, **changes):
        return replace(self, **changes)

    def map_points(self, fn):
        """Copy with fn applied to every defining point (e.g. a translation)."""
        raise NotImplementedError

    def handles(self):
        """Editable points as (name, point): "start", "end", "mid" (arc) or "i:<k>"."""
        raise NotImplementedError

    def with_handle(self, name, p):
        """Copy with one handle moved, or None if that would make the curve invalid."""
        raise NotImplementedError

    def geom_key(self):
        """Hashable description of the shape, used to cache samples and paths."""
        def freeze(v):
            return tuple(freeze(x) for x in v) if isinstance(v, (list, tuple)) else v
        return (self.type, freeze(list(self.geometry_dict().values())))


def _point_list_handles(points):
    out = [("start", points[0])]
    out += [(f"i:{k}", p) for k, p in enumerate(points[1:-1], start=1)]
    out.append(("end", points[-1]))
    return out


def _point_list_with_handle(points, name, p):
    pts = list(points)
    if name == "start":
        pts[0] = _pt(p)
    elif name == "end":
        pts[-1] = _pt(p)
    else:
        pts[int(name.split(":")[1])] = _pt(p)
    return pts


@dataclass
class LineSeg(Curve):
    p0: tuple = (0.0, 0.0)
    p1: tuple = (0.0, 0.0)

    type = "line"

    def start(self): return self.p0
    def end(self): return self.p1

    def map_points(self, fn):
        return self.copy(p0=_pt(fn(self.p0)), p1=_pt(fn(self.p1)))

    def handles(self):
        return [("start", self.p0), ("end", self.p1)]

    def with_handle(self, name, p):
        new = self.copy(**{"p0" if name == "start" else "p1": _pt(p)})
        return None if new.p0 == new.p1 else new

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

    def map_points(self, fn):
        return self.copy(p0=_pt(fn(self.p0)), p1=_pt(fn(self.p1)), centre=_pt(fn(self.centre)))

    def handles(self):
        from .geom import arc_point
        return [("start", self.p0), ("mid", arc_point(self, 0.5)), ("end", self.p1)]

    def with_handle(self, name, p):
        """Moving any handle refits the arc through start, mid and end (three-point arc)."""
        from .geom import arc_point, arcs_through_3_points
        pts = {"start": self.p0, "mid": arc_point(self, 0.5), "end": self.p1}
        pts[name] = _pt(p)
        pieces = arcs_through_3_points(pts["start"], pts["mid"], pts["end"])
        if pieces is None or len(pieces) != 1:
            return None
        p0, p1, c = pieces[0]
        return self.copy(p0=p0, p1=p1, centre=c)

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

    def map_points(self, fn):
        return self.copy(points=[_pt(fn(p)) for p in self.points])

    def handles(self):
        return _point_list_handles(self.points)

    def with_handle(self, name, p):
        return self.copy(points=_point_list_with_handle(self.points, name, p))

    def to_gridgen(self):
        from .geom import CurvePath
        return CurvePath(self)

    def lua_class(self): return "Bezier"

    def lua_args(self, name_of):
        return "points={" + ", ".join(name_of(p) for p in self.points) + "}"

    def geometry_dict(self):
        return {"points": [list(p) for p in self.points]}


@dataclass
class SplineSeg(Curve):
    """
    Interpolating cubic spline through points (Eilmer Spline). A trimmed spline
    keeps its points and covers parameters [t0, t1] (Eilmer SubRangedPath).
    """
    points: list = field(default_factory=list)
    t0: float = 0.0
    t1: float = 1.0

    type = "spline"

    def trimmed(self):
        return self.t0 != 0.0 or self.t1 != 1.0

    def start(self):
        if self.t0 == 0.0:
            return self.points[0]
        from .geom import point_at
        return point_at(self, 0.0)

    def end(self):
        if self.t1 == 1.0:
            return self.points[-1]
        from .geom import point_at
        return point_at(self, 1.0)

    def map_points(self, fn):
        return self.copy(points=[_pt(fn(p)) for p in self.points])

    def handles(self):
        if self.trimmed():   # the trimmed ends are not defining points
            return [(f"i:{k}", p) for k, p in enumerate(self.points)]
        return _point_list_handles(self.points)

    def with_handle(self, name, p):
        pts = _point_list_with_handle(self.points, name, p)
        if any(pts[k] == pts[k+1] for k in range(len(pts)-1)):
            return None     # coincident neighbours break the spline
        return self.copy(points=pts)

    def to_gridgen(self):
        from .geom import CurvePath
        return CurvePath(self)

    def lua_class(self): return "Spline"

    def lua_args(self, name_of):
        return "points={" + ", ".join(name_of(p) for p in self.points) + "}"

    def lua_expr(self, name_of):
        spline = super().lua_expr(name_of)
        if not self.trimmed():
            return spline
        return (f"SubRangedPath:new{{underlying_path={spline}, "
                f"t0={float(self.t0)!r}, t1={float(self.t1)!r}}}")

    def geometry_dict(self):
        d = {"points": [list(p) for p in self.points]}
        if self.trimmed():
            d["t0"], d["t1"] = self.t0, self.t1
        return d


CURVE_TYPES = {cls.type: cls for cls in (LineSeg, ArcSeg, BezierSeg, SplineSeg)}


def curve_from_dict(d):
    cls = CURVE_TYPES[d["type"]]
    common = {"id": d["id"], "kind": d.get("kind", GEOMETRY), "bc_tag": d.get("bc_tag", "")}
    if cls in (LineSeg, ArcSeg):
        geom = {k: _pt(d[k]) for k in ("p0", "p1", "centre") if k in d}
    else:
        geom = {"points": [_pt(p) for p in d["points"]]}
        if cls is SplineSeg:
            geom["t0"] = float(d.get("t0", 0.0))
            geom["t1"] = float(d.get("t1", 1.0))
    return cls(**common, **geom)
