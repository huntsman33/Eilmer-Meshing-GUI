"""
Instant numpy mesh preview, reproducing Eilmer's own construction so it matches
what `lmr custom-script` writes (to round-off) for CoonsPatch blocks:

  faces      one path, or a Polyline whose breakpoints are segment lengths
             (Line and Arc exact; Bezier, Spline, ReversedPath and SubRangedPath
             estimated with 100 chords, as Eilmer's Path.length does)
  clustering RobertsFunction or boundary-layer growth per face, blended into
             (r, s) as in StructuredGrid.make_grid_from_surface
  smoothing  the same Jacobi Laplace sweep the generated Lua runs

AOPatch blocks preview as CoonsPatch; Eilmer's result replaces the preview.
"""
import numpy as np

from ..model import geom
from ..model.document import FACES

CHORDS = geom.PATH_LENGTH_CHORDS


def chord_length(curve, a, b):
    P = geom.points_at(curve, a + (b - a) * np.linspace(0.0, 1.0, CHORDS + 1))
    return float(np.hypot(*np.diff(P, axis=0).T).sum())


class FaceEval:
    """A block face (list of CurveRefs) evaluated at arrays of parameters."""

    def __init__(self, doc, refs):
        self.pieces = []
        lengths = []
        for r in refs:
            c = doc.curves[r.id]
            a, b = (r.t1, r.t0) if r.reversed else (r.t0, r.t1)
            self.pieces.append((c, a, b))
            if r.full() and not r.reversed:
                lengths.append(geom.path_length(c))
            else:
                lengths.append(chord_length(c, a, b))
        tv = np.cumsum(lengths) / sum(lengths)
        tv[-1] = 1.0
        self.tv = tv

    def __call__(self, t):
        t = np.clip(np.atleast_1d(np.asarray(t, float)), 0.0, 1.0)
        if len(self.pieces) == 1:
            c, a, b = self.pieces[0]
            return geom.points_at(c, a + (b - a) * t)
        tv = self.tv
        seg = np.minimum(np.searchsorted(tv, t, side="left"), len(tv) - 1)
        prev = np.where(seg == 0, 0.0, tv[seg - 1])
        local = (t - prev) / (tv[seg] - prev)
        out = np.empty((len(t), 2))
        for k in np.unique(seg):
            m = seg == k
            c, a, b = self.pieces[k]
            out[m] = geom.points_at(c, a + (b - a) * local[m])
        return out


def roberts(t, end0=False, end1=False, beta=1.0):
    """Eilmer RobertsFunction (geom/misc/univariatefunctions.d)."""
    t = np.asarray(t, float)
    if not (end0 or end1) or beta <= 1.0:
        return t
    alpha = 0.5 if (end0 and end1) else 0.0
    reverse = end0 and not end1
    if reverse:
        t = 1.0 - t
    lam = ((beta + 1.0) / (beta - 1.0)) ** ((t - alpha) / (1.0 - alpha))
    tbar = ((beta + 2.0*alpha) * lam - beta + 2.0*alpha) / ((2.0*alpha + 1.0) * (1.0 + lam))
    return 1.0 - tbar if reverse else tbar


def distribute(spec, n):
    """Vertex parameters for a concrete clustering spec (layers.concrete_cluster)."""
    t = np.linspace(0.0, 1.0, n)
    if spec and spec.get("type") == "RobertsFunction":
        return roberts(t, spec.get("end0", False), spec.get("end1", False), spec.get("beta", 1.0))
    if spec and spec.get("type") == "growth":
        from ..model.layers import growth_values
        return growth_values(spec["r"], spec["n"], t, spec["reverse"])
    return t


def block_grid(doc, block):
    """Vertices (niv, njv, 3) of a block, as Eilmer would generate them."""
    from ..model.layers import concrete_cluster
    f = {face: FaceEval(doc, block.sides[face]) for face in FACES}
    niv, njv = block.niv, block.njv
    rS = distribute(concrete_cluster(doc, block, "south"), niv)
    rN = distribute(concrete_cluster(doc, block, "north"), niv)
    sW = distribute(concrete_cluster(doc, block, "west"), njv)
    sE = distribute(concrete_cluster(doc, block, "east"), njv)
    r = np.linspace(0.0, 1.0, niv)[:, None]
    s = np.linspace(0.0, 1.0, njv)[None, :]
    sdash = ((1.0 - r) * sW[None, :] + r * sE[None, :]).ravel()
    rdash = ((1.0 - s) * rS[:, None] + s * rN[:, None]).ravel()
    xy = coons(f, rdash, sdash).reshape(niv, njv, 2)
    xyz = np.concatenate([xy, np.zeros((niv, njv, 1))], axis=2)
    if block.smoothing:
        xyz = laplace_smooth(xyz, block.smoothing["nsweeps"], block.smoothing.get("tol", 0.0))
    return xyz


def coons(f, r, s):
    """Coons patch points at parameter arrays r, s (Eilmer CoonsPatch)."""
    S, N, W, E = f["south"](r), f["north"](r), f["west"](s), f["east"](s)
    p00, p10 = f["south"]([0.0])[0], f["south"]([1.0])[0]
    p01, p11 = f["north"]([0.0])[0], f["north"]([1.0])[0]
    r, s = r[:, None], s[:, None]
    return ((1 - r) * W + r * E + (1 - s) * S + s * N
            - ((1 - r) * (1 - s) * p00 + r * (1 - s) * p10 + (1 - r) * s * p01 + r * s * p11))


def laplace_smooth(xyz, nsweeps, tol=0.0):
    """Vectorised Jacobi sweep matching the generated Lua; boundary vertices fixed."""
    v = xyz.copy()
    for _ in range(nsweeps):
        new = 0.25 * (v[2:, 1:-1] + v[:-2, 1:-1] + v[1:-1, 2:] + v[1:-1, :-2])
        change = np.abs(new - v[1:-1, 1:-1]).max() if new.size else 0.0
        v[1:-1, 1:-1] = new
        if change < tol:
            break
    return v
