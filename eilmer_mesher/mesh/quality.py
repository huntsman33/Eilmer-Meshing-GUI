"""
Cell quality of structured grids (Eilmer's mesh, or the preview until it arrives).

  skewness      equiangle skewness max((qmax - 90) / 90, (90 - qmin) / 90) of the
                cell's corner angles: 0 for a rectangle, 1 for a degenerate cell
  aspect ratio  longest edge / shortest edge
  inverted      a corner turning the wrong way for its block (a folded or tangled
                cell), or a cell with no area; counted with skewness 1

Blocks whose edges carry boundary-layer spacing are long and thin on purpose, so
their aspect ratio is not checked.
"""
from dataclasses import dataclass, field

import numpy as np

DEFAULT_THRESHOLDS = {"skew_warn": 0.6, "skew_bad": 0.85, "ar_warn": 20.0}


def cell_metrics(xyz):
    """{"skew", "ar", "inverted"} arrays of shape (niv - 1, njv - 1) for a grid (niv, njv, >= 2)."""
    P = np.asarray(xyz, float)[:, :, :2]
    a, b, c, d = P[:-1, :-1], P[1:, :-1], P[1:, 1:], P[:-1, 1:]          # round the cell
    corners = ((a, b, d), (b, c, a), (c, d, b), (d, a, c))             # (vertex, next, previous)
    cross, angle = [], []
    for v, n, p in corners:
        e1, e2 = n - v, p - v
        cr = e1[..., 0] * e2[..., 1] - e1[..., 1] * e2[..., 0]
        dot = (e1 * e2).sum(axis=-1)
        cross.append(cr)
        angle.append(np.degrees(np.arctan2(np.abs(cr), dot)))
    cross, angle = np.array(cross), np.array(angle)
    area = 0.5 * (cross[0] + cross[2])                                 # two triangles
    sign = 1.0 if area.sum() >= 0.0 else -1.0                          # the block's own orientation
    inverted = (sign * cross <= 0.0).any(axis=0)
    skew = np.maximum((angle.max(axis=0) - 90.0) / 90.0, (90.0 - angle.min(axis=0)) / 90.0)
    skew = np.where(inverted, 1.0, np.clip(skew, 0.0, 1.0))
    edges = np.array([np.hypot(*(q - p).transpose(2, 0, 1)) for p, q in ((a, b), (b, c), (c, d), (d, a))])
    with np.errstate(divide="ignore", invalid="ignore"):
        ar = np.where(edges.min(axis=0) > 0.0, edges.max(axis=0) / edges.min(axis=0), np.inf)
    return {"skew": skew, "ar": ar, "inverted": inverted}


@dataclass
class BlockQuality:
    id: str
    cells: int
    max_skew: float
    max_ar: float
    skew_warn: int                # cells above the warning skewness
    skew_bad: int                 # cells above the bad skewness
    ar_warn: int                  # cells above the aspect-ratio warning (0 when not checked)
    inverted: int
    check_ar: bool = True

    def level(self):
        if self.inverted:
            return "error"
        if self.skew_bad:
            return "bad"
        if self.skew_warn or self.ar_warn:
            return "warn"
        return "ok"


LEVELS = ("ok", "warn", "bad", "error")


@dataclass
class MeshQuality:
    revision: int
    source: str
    thresholds: dict
    blocks: dict = field(default_factory=dict)        # block id -> BlockQuality
    metrics: dict = field(default_factory=dict)       # block id -> cell_metrics(...)

    def level(self):
        return max((b.level() for b in self.blocks.values()), key=LEVELS.index, default="ok")

    def flagged(self, *levels):
        return [bid for bid, b in self.blocks.items() if b.level() in levels]

    def summary(self, limit=3):
        """One line naming the worst problems."""
        t = self.thresholds
        parts = []
        worst = sorted(self.blocks.values(), key=lambda b: (-LEVELS.index(b.level()), -b.max_skew))
        for b in worst:
            if b.level() == "ok":
                continue
            bits = []
            if b.inverted:
                bits.append(f"{b.inverted} inverted cell{'s' * (b.inverted > 1)}")
            if b.skew_bad:
                bits.append(f"{b.skew_bad} cells skewness > {t['skew_bad']:g} (max {b.max_skew:.2f})")
            elif b.skew_warn:
                bits.append(f"{b.skew_warn} cells skewness > {t['skew_warn']:g} (max {b.max_skew:.2f})")
            if b.ar_warn:
                bits.append(f"{b.ar_warn} cells aspect ratio > {t['ar_warn']:g} (max {b.max_ar:.3g})")
            parts.append(f"{b.id}: " + ", ".join(bits))
        more = len(parts) - limit
        return "; ".join(parts[:limit]) + (f"; and {more} more block(s)" if more > 0 else "")


def intentional_aspect(blocks):
    """Ids of blocks with boundary-layer (first cell) spacing on an edge: thin cells by design."""
    return {b.id for b in blocks
            if any((spec or {}).get("type") in ("Growth", "Grade") for spec in b.cluster.values())}


def assess(mesh, thresholds=None, skip_ar=()):
    """MeshQuality for a live Mesh."""
    t = {**DEFAULT_THRESHOLDS, **(thresholds or {})}
    q = MeshQuality(mesh.revision, mesh.source, t)
    for bid, xyz in mesh.grids:
        if xyz.shape[0] < 2 or xyz.shape[1] < 2:
            continue
        m = cell_metrics(xyz)
        check_ar = bid not in skip_ar
        finite = m["ar"][np.isfinite(m["ar"])]
        q.metrics[bid] = m
        q.blocks[bid] = BlockQuality(
            bid, int(m["skew"].size), float(m["skew"].max()),
            float(finite.max()) if finite.size else float("inf"),
            int((m["skew"] > t["skew_warn"]).sum()), int((m["skew"] > t["skew_bad"]).sum()),
            int((m["ar"] > t["ar_warn"]).sum()) if check_ar else 0,
            int(m["inverted"].sum()), check_ar)
    return q
