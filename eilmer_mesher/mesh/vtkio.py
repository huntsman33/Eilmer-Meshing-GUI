"""
Reader for legacy ASCII VTK STRUCTURED_GRID files, as written by Eilmer's
StructuredGrid:write_to_vtk_file and gridgen.gridutils.write_grid_to_vtk.
"""
import numpy as np


def read_structured_vtk(path):
    """Return vertices as an array of shape (niv, njv, 3) (k = 0 slice for 2D grids)."""
    with open(path) as f:
        lines = f.read().splitlines()
    dims = npts = start = None
    for n, line in enumerate(lines):
        tok = line.split()
        if not tok:
            continue
        if tok[0] == "DATASET" and tok[1] != "STRUCTURED_GRID":
            raise ValueError(f"{path}: expected STRUCTURED_GRID, got {tok[1]}")
        if tok[0] == "DIMENSIONS":
            dims = tuple(int(t) for t in tok[1:4])
        elif tok[0] == "POINTS":
            npts = int(tok[1])
            start = n + 1
            break
    if dims is None or start is None:
        raise ValueError(f"{path}: missing DIMENSIONS or POINTS")
    niv, njv, nkv = dims
    if npts != niv * njv * nkv:
        raise ValueError(f"{path}: POINTS {npts} does not match DIMENSIONS {dims}")
    xyz = np.array(" ".join(lines[start:]).split()[:3 * npts], dtype=float)
    # VTK order: i fastest, then j, then k.
    xyz = xyz.reshape(nkv, njv, niv, 3)
    return xyz[0].transpose(1, 0, 2).copy()
