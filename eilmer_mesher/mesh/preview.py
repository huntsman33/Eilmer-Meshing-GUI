"""
Instant Python preview of a block, used while dragging before Eilmer re-runs.

Uses gridgen's Coons patch evaluation; clustering functions are not applied
here, so the authoritative mesh is always the one Eilmer writes.
"""
import numpy as np

from gridgen.path import Path, Polyline
from gridgen.sgrid_gen import gridpts

from ..model.document import FACES


class ReversedPath(Path):
    def __init__(self, underlying):
        self.underlying = underlying

    def __repr__(self):
        return f"ReversedPath({self.underlying!r})"

    def __call__(self, t):
        return self.underlying(1.0 - t)

    def length(self):
        return self.underlying.length()


def face_path(doc, refs):
    paths = []
    for r in refs:
        p = doc.curves[r.id].to_gridgen()
        paths.append(ReversedPath(p) if r.reversed else p)
    return paths[0] if len(paths) == 1 else Polyline(paths)


def block_grid(doc, block):
    """Coons grid for a block (with Laplace smoothing if set) as an (niv, njv, 3) array."""
    f = {face: face_path(doc, block.sides[face]) for face in FACES}
    pts = gridpts(f["north"], f["east"], f["south"], f["west"], block.niv, block.njv)
    xyz = np.array([[(p.x, p.y, p.z) for p in row] for row in pts])
    if block.smoothing:
        xyz = laplace_smooth(xyz, block.smoothing["nsweeps"], block.smoothing.get("tol", 0.0))
    return xyz


def laplace_smooth(xyz, nsweeps, tol=0.0):
    """Vectorised Jacobi sweep matching tutes/wk7 smooth_grid; boundary vertices fixed."""
    v = xyz.copy()
    for _ in range(nsweeps):
        new = 0.25 * (v[2:, 1:-1] + v[:-2, 1:-1] + v[1:-1, 2:] + v[1:-1, :-2])
        change = np.abs(new - v[1:-1, 1:-1]).max() if new.size else 0.0
        v[1:-1, 1:-1] = new
        if change < tol:
            break
    return v
