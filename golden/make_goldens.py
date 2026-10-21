"""
Golden files for the Rust port (Phase 7): the Python implementation is the reference.

    python3 golden/make_goldens.py                 corpus/ + expected/ (committed)
    python3 golden/make_goldens.py --local DIR...  also every *.emesh.json in DIR into local/ (not committed)

corpus/NAME.emesh.json      input projects, built here with the Python model
expected/NAME/              for each project:
    mesh.lua, bcs.lua, sim.lua    generated scripts (Rust must match byte for byte)
    roundtrip.json                Document.from_dict(file).to_dict()
    topology.json                 vertices, edges, regions (loops, areas, holes)
    resolution.json               concrete blocks, errors, warnings, count groups
    grids.json                    preview grids per block (tolerance 1e-12)
    quality.json                  cell-quality summary of the preview grids
expected/ops/NAME.json      {"input": doc, "op": {...}, "output": doc | null, "error": str | null,
                             "returns": ...}: a model operation replayed on a freshly loaded document
"""
import argparse
import glob
import json
import math
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)
sys.path.insert(0, os.path.join(ROOT, "tests"))
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np                                                    # noqa: E402
from PySide6.QtCore import QCoreApplication                           # noqa: E402

import fixtures                                                       # noqa: E402
from eilmer_mesher.lua.codegen import generate_bcs_lua, generate_mesh_lua, used_bc_tags  # noqa: E402
from eilmer_mesher.mesh import quality                                # noqa: E402
from eilmer_mesher.mesh.preview import block_grid                     # noqa: E402
from eilmer_mesher.model import blocking, geom                        # noqa: E402
from eilmer_mesher.model.bcs import sim_template                      # noqa: E402
from eilmer_mesher.model.document import Block, BlockError, Document  # noqa: E402
from eilmer_mesher.model.entities import (REFERENCE, SPLIT, ArcSeg, BezierSeg, LineSeg,  # noqa: E402
                                          SplineSeg)
from eilmer_mesher.model.topology import Topology                     # noqa: E402
from eilmer_mesher.sketch.importers import build_curves, parse_point_file  # noqa: E402
from eilmer_mesher.sketch.model import SketchModel                    # noqa: E402

app = QCoreApplication.instance() or QCoreApplication([])


# --- corpus ------------------------------------------------------------------------------

def box(x0, y0, x1, y1, kind="geometry"):
    c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    return [LineSeg(None, kind=kind, p0=c[k], p1=c[(k + 1) % 4]) for k in range(4)]


def circle(c, r, kind="geometry"):
    return [ArcSeg(None, kind=kind, p0=a, p1=b, centre=cc) for a, b, cc in geom.circle_arcs(c, r)]


def naca(t=0.12, n=41):
    """NACA 00xx in Selig order (TE, upper, LE, lower, TE) as point-file text."""
    xs = [0.5 * (1 - math.cos(math.pi * k / (n - 1))) for k in range(n)]
    yt = [5 * t * (0.2969 * math.sqrt(x) - 0.1260 * x - 0.3516 * x**2 + 0.2843 * x**3 - 0.1036 * x**4)
          for x in xs]
    upper = [(x, y) for x, y in zip(xs, yt)][::-1]
    lower = [(x, -y) for x, y in zip(xs, yt)][1:]
    return "NACA test\n" + "\n".join(f"{x:.6f} {y:.6f}" for x, y in upper + lower)


def model_with(curves=()):
    m = SketchModel()
    if curves:
        m.add_curves(list(curves), "add")
    return m


def corpus():
    """name -> Document."""
    out = {}
    out["sphere"] = fixtures.sphere_doc()
    out["wk7"] = fixtures.wk7_doc()
    out["wk7_smoothed"] = fixtures.wk7_doc(smoothing={"nsweeps": 50, "tol": 1e-12})
    out["two_block"] = fixtures.two_block_doc()

    m = model_with(box(0, 0, 2, 1))
    bid = m.make_block((1, 0.5))
    m.set_edge_cells(bid, "south", 30, {"type": "RobertsFunction", "end0": True, "end1": False, "beta": 1.05})
    m.set_edge_cells(bid, "west", 12, {"type": "RobertsFunction", "end0": True, "end1": True, "beta": 1.2},
                     opposite=False)
    out["box_roberts"] = m.doc

    m = model_with(box(0, 0, 3, 1) + [LineSeg(None, kind=SPLIT, p0=(1, 0), p1=(1, 1)),
                                      LineSeg(None, kind=SPLIT, p0=(2, 0), p1=(2, 1))])
    m.make_all_blocks()
    m.define_bc("inflow", "InFlowBC_Supersonic", "flowState=inflow")
    m.define_bc("outflow", "OutFlowBC_FixedP", "p_outside=101.325e3")
    m.set_bc([c.id for c in m.doc.curves.values() if c.type == "line" and c.p0[0] == 0 == c.p1[0]], "inflow")
    m.set_bc([c.id for c in m.doc.curves.values() if c.type == "line" and c.p0[0] == 3 == c.p1[0]], "outflow")
    out["channel_three"] = m.doc

    m = model_with(circle((0, 0), 1.0))
    m.subdivide((0.1, 0.1), "ogrid", size=0.5)
    out["disc_ogrid"] = m.doc

    m = model_with(box(-2, -2, 2, 2) + circle((0, 0), 0.5))
    m.subdivide((1.5, 0), "ogrid_hole")
    out["cylinder_ogrid_hole"] = m.doc

    m = model_with([LineSeg(None, p0=(0, 0), p1=(2, 0)), LineSeg(None, p0=(2, 0), p1=(0.6, 1.5)),
                    LineSeg(None, p0=(0.6, 1.5), p1=(0, 0))])
    m.subdivide((0.8, 0.5), "centroid")
    out["triangle_centroid"] = m.doc

    m = model_with([LineSeg(None, p0=(0, 0), p1=(2, 0)),
                    ArcSeg(None, p0=(2, 0), p1=(2, 1.2), centre=(1.4, 0.6)),
                    LineSeg(None, p0=(2, 1.2), p1=(0, 1)),
                    LineSeg(None, p0=(0, 1), p1=(0, 0))])
    m.subdivide((1, 0.5), "nxm", n=2, m=3)
    out["arc_quad_2x3"] = m.doc

    m = model_with(box(0, 0, 1, 0.2))
    bid = m.make_block((0.5, 0.1))
    m.add_boundary_layer([(bid, "south")], 1e-4, 1.2, 10, False)
    out["layer_flat_plate"] = m.doc

    m = model_with(box(0, 0, 1, 0.2))
    bid = m.make_block((0.5, 0.1))
    m.add_boundary_layer([(bid, "south")], 1e-4, 1.2, 10, True)
    out["layer_flat_plate_graded"] = m.doc

    m = model_with(box(0, 0, 2, 0.5) + [LineSeg(None, kind=SPLIT, p0=(1, 0), p1=(1, 0.5))])
    m.make_all_blocks()
    right = m.block_at((1.5, 0.25))
    m.add_boundary_layer([(right, "south")], 1e-3, 1.2, 8)
    out["layer_propagates"] = m.doc

    m = model_with(box(0, 0, 1, 1))
    bid = m.make_block((0.5, 0.5))
    m.add_boundary_layer([(bid, "south"), (bid, "west")], 1e-3, 1.2, 8)
    out["layer_corner"] = m.doc

    m = model_with(box(-2, -2, 2, 2) + circle((0, 0), 0.5))
    m.subdivide((1.5, 0), "ogrid_hole")
    res = m.resolution()
    walls = []
    for b in res.blocks:
        for f in ("north", "east", "south", "west"):
            if all(m.doc.curves[r.id].type == "arc" for r in b.sides[f]):
                walls.append((b.id, f))
    m.add_boundary_layer(walls, 1e-3, 1.15, 12)
    out["layer_cylinder_ring"] = m.doc

    m = model_with([LineSeg(None, p0=(0, 0), p1=(1, 0)), LineSeg(None, p0=(1, 0), p1=(1, 1)),
                    SplineSeg(None, points=[(1, 1), (0.5, 3.0), (0, 1)]), LineSeg(None, p0=(0, 1), p1=(0, 0))])
    bid = m.make_block((0.5, 0.1))
    m.set_edge_cells(bid, "south", 20)
    m.set_edge_cells(bid, "west", 20)
    m.refine_blocks([bid])
    out["bulge_smoothed"] = m.doc

    m = model_with([LineSeg(None, p0=(0, 0), p1=(1, 0)), LineSeg(None, p0=(1, 0), p1=(1, 1)),
                    SplineSeg(None, points=[(1, 1), (0.6, 1), (0.5, 0.3), (0.4, 1), (0, 1)]),
                    LineSeg(None, p0=(0, 1), p1=(0, 0))])
    bid = m.make_block((0.5, 0.1))
    m.set_edge_cells(bid, "south", 20)
    m.set_edge_cells(bid, "west", 20)
    m.refine_blocks([bid])
    out["dent_aopatch"] = m.doc

    pf = parse_point_file(naca())
    m = model_with(build_curves(pf, scale=1.0, angle_deg=4.0, dx=0.0, dy=0.0))
    m.add_curves(box(-1, -1, 2, 1), "box")
    m.add_curves([LineSeg(None, kind=REFERENCE, p0=(-1, 0), p1=(2, 0))], "axis")
    out["airfoil_in_box"] = m.doc

    m = model_with([BezierSeg(None, points=[(0, 0), (0.5, 0.4), (1, 0)]), LineSeg(None, p0=(1, 0), p1=(1, 1)),
                    LineSeg(None, p0=(1, 1), p1=(0, 1)), LineSeg(None, p0=(0, 1), p1=(0, 0))])
    ids = list(m.doc.curves)
    m.add_dimension(ids[1], 0.2)
    m.add_dimension(ids[1], 0.3, ids[3])
    m.add_dimension(ids[1], 0.25, ids[2])
    m.add_point((0.5, 0.5))
    bid = m.make_block((0.5, 0.6))
    m.set_block_patch([bid], "AOPatch", {"nx": 12, "ny": 8}, smoothing=None)
    m.define_bc("wall", "WallBC_NoSlip_FixedT", "Twall=300.0")
    m.define_bc("ambient", "InOutFlowBC_Ambient", "flowState=ambient")
    m.set_bc([ids[0]], "wall")
    m.set_bc([ids[2]], "ambient")
    out["bezier_dims_aopatch"] = m.doc

    m = model_with(box(0, 0, 1, 1) + [SplineSeg(None, points=[(0, 0.3), (0.4, 0.5), (1, 0.6)], kind=SPLIT)])
    m.make_all_blocks()
    out["spline_split"] = m.doc

    m = model_with(box(0, 0, 1, 1))
    m.make_block((0.5, 0.5))
    m.add_curves([LineSeg(None, p0=(1, 0), p1=(2, 0)), LineSeg(None, p0=(2, 0), p1=(2, 1)),
                  LineSeg(None, p0=(2, 1), p1=(1, 1))], "more")
    m.make_all_blocks()
    out["solid_shared_wall"] = m.doc            # warning: a solid curve between two blocks

    m = model_with(box(0, 0, 1, 1))
    m.make_block((0.5, 0.5))
    m.add_curves([LineSeg(None, kind=SPLIT, p0=(0.5, 0), p1=(0.5, 1))], "split it")
    out["stale_block"] = m.doc                  # error: the region no longer exists
    return out


# --- expected outputs --------------------------------------------------------------------

def topology_json(topo):
    return {
        "size": topo.size,
        "vertices": [list(v) for v in topo.vertices],
        "edges": [{"curve": e.curve_id, "ta": e.ta, "tb": e.tb, "v0": e.v0, "v1": e.v1} for e in topo.edges],
        "regions": [{"loop": [[e, fwd] for e, fwd in r.loop], "holes": [[[e, fwd] for e, fwd in h] for h in r.holes],
                     "area": r.area, "polygon": np.asarray(r.polygon).tolist(),
                     "hole_polygons": [np.asarray(h).tolist() for h in r.hole_polygons]}
                    for r in topo.regions],
    }


def resolution_json(res):
    return {
        "blocks": [b.to_dict() | {"sides": {f: [r.to_dict() for r in b.sides[f]] for f in b.sides}}
                   for b in res.blocks],
        "errors": res.errors,
        "warnings": res.warnings,
        "polygons": {bid: np.asarray(p).tolist() for bid, p in res.polygons.items()},
        "groups": [[bid, axis, list(g)] for (bid, axis), g in res.groups.items()],
    }


def expected(name, doc, outdir):
    d = os.path.join(outdir, name)
    os.makedirs(d, exist_ok=True)
    data = doc.to_dict()
    doc = Document.from_dict(json.loads(json.dumps(data)))
    with open(os.path.join(d, "roundtrip.json"), "w") as f:
        json.dump(doc.to_dict(), f, indent=2)
    topo = Topology(doc)
    res = blocking.resolve(doc, topo)
    with open(os.path.join(d, "topology.json"), "w") as f:
        json.dump(topology_json(topo), f, indent=1)
    with open(os.path.join(d, "resolution.json"), "w") as f:
        json.dump(resolution_json(res), f, indent=1)
    md = doc.with_blocks(res.blocks)
    try:
        lua = generate_mesh_lua(md)
    except BlockError as e:
        lua = f"-- error: {e}\n"
    with open(os.path.join(d, "mesh.lua"), "w") as f:
        f.write(lua)
    with open(os.path.join(d, "bcs.lua"), "w") as f:
        f.write(generate_bcs_lua(md))
    with open(os.path.join(d, "sim.lua"), "w") as f:
        f.write(sim_template(md, used_bc_tags(md)))
    grids = {b.id: block_grid(md, b)[:, :, :2].tolist() for b in res.blocks}
    with open(os.path.join(d, "grids.json"), "w") as f:
        json.dump(grids, f)
    from eilmer_mesher.mesh.live import Mesh
    q = quality.assess(Mesh(0, [(b.id, np.asarray(block_grid(md, b))) for b in res.blocks], "preview"),
                       None, quality.intentional_aspect(res.blocks))
    with open(os.path.join(d, "quality.json"), "w") as f:
        json.dump({"level": q.level(), "summary": q.summary(),
                   "blocks": {bid: b.__dict__ for bid, b in q.blocks.items()}}, f, indent=1)


# --- operations ----------------------------------------------------------------------------

def run_op(doc_dict, op):
    """Apply op to a freshly loaded document. Returns (output dict or None, error or None, returns)."""
    m = SketchModel(Document.from_dict(json.loads(json.dumps(doc_dict))))
    kind = op["op"]
    try:
        if kind == "make_block":
            ret = m.make_block(tuple(op["point"]), op.get("corners"))
        elif kind == "make_all_blocks":
            made, skipped = m.make_all_blocks()
            ret = {"made": made, "skipped": len(skipped)}
        elif kind == "subdivide":
            ret = m.subdivide(tuple(op["point"]), op["kind"], **op.get("opts", {}))
        elif kind == "set_edge_cells":
            m.set_edge_cells(op["bid"], op["face"], op["cells"], op.get("cluster"), op.get("opposite", True))
            ret = None
        elif kind == "set_block_corners":
            m.set_block_corners(op["bid"], [tuple(p) for p in op["points"]])
            ret = None
        elif kind == "add_boundary_layer":
            ret = m.add_boundary_layer([tuple(f) for f in op["faces"]], op["first"], op["growth"], op["cells"],
                                       op.get("transition", False))
        elif kind == "edit_boundary_layer":
            m.edit_boundary_layer(op["lid"], **op["settings"])
            ret = None
        elif kind == "remove_boundary_layer":
            m.remove_boundary_layer(op["lid"])
            ret = None
        elif kind == "layer_note":
            ret = m.layer_note(op["lid"])
        elif kind == "refine_blocks":
            ret = list(m.refine_blocks(op["bids"]))
        elif kind == "offset":
            ret = m.offset(op["ids"], op["distance"], op.get("reverse", False), op.get("source_to_reference", False))
        elif kind == "merge":
            ret = list(m.merge(op["ids"]))
        elif kind == "mirror":
            ret = m.mirror(op["ids"], tuple(op["a"]), tuple(op["b"]))
        elif kind == "trim":
            from eilmer_mesher.model.topology import curve_params
            cid = op["cid"]
            ts = curve_params(m.doc, cid)
            t, _, _ = geom.closest_param(m.doc.curves[cid], tuple(op["point"]))
            k = max(i for i in range(len(ts) - 1) if ts[i] <= t)
            ta, tb = ts[k], ts[k + 1]
            with m.edit("Trim") as doc:
                c = doc.curves[cid]
                keep = []
                if ta > 0.0:
                    keep.append(geom.subcurve(c, 0.0, ta))
                if tb < 1.0:
                    keep.append(geom.subcurve(c, tb, 1.0))
                doc.remove_blocks_using([cid])
                if not keep:
                    doc.remove_curves([cid])
                else:
                    doc.curves[cid] = keep[0]
                    if len(keep) > 1:
                        doc.add_curve(keep[1].copy(id=doc.new_id("c")))
            ret = [ta, tb]
        elif kind == "toggle_split":
            m.toggle_split(op["ids"])
            ret = None
        elif kind == "toggle_reference":
            m.toggle_reference(op["ids"])
            ret = None
        elif kind == "add_dimension":
            ret = m.add_dimension(op["cid"], op["offset"], op.get("cid2"), op.get("sector"))
        elif kind == "set_dimension_value":
            m.set_dimension_value(op["did"], op["value"])
            ret = None
        elif kind == "set_line":
            m.set_line(op["cid"], op["length"], op["angle"])
            ret = None
        elif kind == "set_arc":
            m.set_arc(op["cid"], op["radius"], op["sweep"])
            ret = None
        elif kind == "translate":
            m.translate(op["ids"], op["dx"], op["dy"])
            ret = None
        elif kind == "define_bc":
            m.define_bc(op["tag"], op["cls"], op.get("args", ""), op.get("old_tag"))
            ret = None
        elif kind == "set_bc":
            ret = m.set_bc(op["ids"], op["tag"])
        elif kind == "delete_bc":
            m.delete_bc(op["tag"])
            ret = None
        elif kind == "delete":
            m.set_selection(op["ids"])
            m.delete_selection()
            ret = None
        elif kind == "import_points":
            pf = parse_point_file(op["text"], op.get("name", "points"))
            o = op.get("options", {})
            ret = m.add_curves(build_curves(pf, **o), "Import")
        else:
            raise ValueError(kind)
    except (BlockError, ValueError) as e:
        return None, str(e), None
    return m.doc.to_dict(), None, ret


def operations(docs):
    """[(name, input doc dict, op)]."""
    d = {name: doc.to_dict() for name, doc in docs.items()}
    m = model_with(box(0, 0, 2, 1) + [LineSeg(None, kind=SPLIT, p0=(1, 0), p1=(1, 1))])
    plain_split = m.doc.to_dict()
    m = model_with(box(0, 0, 1, 1) + circle((3, 0.5), 0.4))
    shapes = m.doc.to_dict()
    sids = list(m.doc.curves)
    m = model_with(box(0, 0, 1, 0.2))
    m.make_block((0.5, 0.1))
    plate = m.doc.to_dict()
    pbid = m.doc.blocks[0].id
    m = model_with([LineSeg(None, p0=(0, 0), p1=(1, 0)), LineSeg(None, p0=(1, 0), p1=(2, 0)),
                    ArcSeg(None, p0=(3, 0), p1=(3, 1), centre=(3, 0.5)),
                    ArcSeg(None, p0=(3, 1), p1=(3.5, 0.5), centre=(3, 0.5))])
    pieces = m.doc.to_dict()
    pids = list(m.doc.curves)
    m = model_with([LineSeg(None, p0=(0, 0), p1=(2, 0)), LineSeg(None, p0=(1, -1), p1=(1, 1)),
                    LineSeg(None, p0=(0, 0.5), p1=(2, 0.5))])
    crossing = m.doc.to_dict()
    cids = list(m.doc.curves)
    ring = docs["layer_cylinder_ring"]
    ring_lid = ring.layers[0].id
    out = [
        ("make_block_box", plain_split, {"op": "make_block", "point": [0.5, 0.5]}),
        ("make_block_corners", d["two_block"] | {"blocks": []}, {"op": "make_all_blocks"}),
        ("make_all_split", plain_split, {"op": "make_all_blocks"}),
        ("make_all_shapes", shapes, {"op": "make_all_blocks"}),
        ("subdivide_nxm", shapes, {"op": "subdivide", "point": [0.5, 0.5], "kind": "nxm", "opts": {"n": 3, "m": 2}}),
        ("subdivide_ogrid", shapes, {"op": "subdivide", "point": [3, 0.5], "kind": "ogrid", "opts": {"size": 0.4}}),
        ("subdivide_hole", model_with(box(-2, -2, 2, 2) + circle((0, 0), 0.5)).doc.to_dict(),
         {"op": "subdivide", "point": [1.5, 0], "kind": "ogrid_hole"}),
        ("edge_cells_roberts", d["channel_three"],
         {"op": "set_edge_cells", "bid": "blk1", "face": "south", "cells": 15,
          "cluster": {"type": "RobertsFunction", "end0": False, "end1": True, "beta": 1.1}}),
        ("edge_cells_growth", plate,
         {"op": "set_edge_cells", "bid": pbid, "face": "west", "cells": 9,
          "cluster": {"type": "Growth", "first": 0.002, "reverse": False}, "opposite": True}),
        ("set_corners", d["arc_quad_2x3"] | {"blocks": [], "curves": d["arc_quad_2x3"]["curves"][:4]},
         {"op": "make_block", "point": [1.0, 0.5]}),
        ("layer_plate", plate, {"op": "add_boundary_layer", "faces": [[pbid, "south"]], "first": 2e-4,
                                "growth": 1.25, "cells": 6}),
        ("layer_plate_graded", plate, {"op": "add_boundary_layer", "faces": [[pbid, "south"], [pbid, "north"]],
                                       "first": 2e-4, "growth": 1.25, "cells": 6, "transition": True}),
        ("layer_too_thick", plate, {"op": "add_boundary_layer", "faces": [[pbid, "south"]], "first": 0.05,
                                    "growth": 1.2, "cells": 10}),
        ("layer_edit", ring.to_dict(), {"op": "edit_boundary_layer", "lid": ring_lid,
                                        "settings": {"first": 5e-4, "cells": 8}}),
        ("layer_remove", ring.to_dict(), {"op": "remove_boundary_layer", "lid": ring_lid}),
        ("layer_note", d["layer_flat_plate"], {"op": "layer_note", "lid": docs["layer_flat_plate"].layers[0].id}),
        ("refine_bulge", d["bulge_smoothed"] | {"blocks": [b | {"smoothing": None} for b in
                                                             d["bulge_smoothed"]["blocks"]]},
         {"op": "refine_blocks", "bids": [d["bulge_smoothed"]["blocks"][0]["id"]]}),
        ("offset_box_grow", shapes, {"op": "offset", "ids": sids[:4], "distance": 0.1}),
        ("offset_box_shrink", shapes, {"op": "offset", "ids": sids[:4], "distance": 0.1, "reverse": True,
                                       "source_to_reference": True}),
        ("offset_circle", shapes, {"op": "offset", "ids": sids[4:], "distance": 0.15, "reverse": True}),
        ("offset_open", shapes, {"op": "offset", "ids": sids[:2], "distance": 0.05}),
        ("offset_spline", d["airfoil_in_box"],
         {"op": "offset", "ids": [c["id"] for c in d["airfoil_in_box"]["curves"][:3]], "distance": 0.01}),
        ("offset_too_far", shapes, {"op": "offset", "ids": sids[4:], "distance": 0.5, "reverse": True}),
        ("merge_pieces", pieces, {"op": "merge", "ids": pids}),
        ("mirror_shapes", shapes, {"op": "mirror", "ids": sids, "a": [0, 2], "b": [1, 3]}),
        ("trim_middle", crossing, {"op": "trim", "cid": cids[0], "point": [0.7, 0.0]}),
        ("trim_end", crossing, {"op": "trim", "cid": cids[1], "point": [1.0, -0.6]}),
        ("trim_arc", d["cylinder_ogrid_hole"],
         {"op": "trim", "cid": d["cylinder_ogrid_hole"]["curves"][4]["id"], "point": [0.5, 0.05]}),
        ("toggle_split", plain_split, {"op": "toggle_split", "ids": [list(json.loads(json.dumps(plain_split))
                                                                          ["curves"])[0]["id"]]}),
        ("toggle_reference", d["channel_three"], {"op": "toggle_reference", "ids": ["c1", "c5"]}),
        ("dimension_length", shapes, {"op": "add_dimension", "cid": sids[0], "offset": -0.2}),
        ("dimension_angle", shapes, {"op": "add_dimension", "cid": sids[0], "offset": 0.3, "cid2": sids[1]}),
        ("dimension_distance", shapes, {"op": "add_dimension", "cid": sids[0], "offset": 0.4, "cid2": sids[2]}),
        ("drive_distance", d["bezier_dims_aopatch"],
         {"op": "set_dimension_value", "did": list(docs["bezier_dims_aopatch"].dimensions)[1], "value": 1.5}),
        ("drive_angle", d["bezier_dims_aopatch"],
         {"op": "set_dimension_value", "did": list(docs["bezier_dims_aopatch"].dimensions)[2], "value": 75.0}),
        ("drive_length", d["bezier_dims_aopatch"],
         {"op": "set_dimension_value", "did": list(docs["bezier_dims_aopatch"].dimensions)[0], "value": 0.6}),
        ("set_line", shapes, {"op": "set_line", "cid": sids[1], "length": 1.5, "angle": 80.0}),
        ("set_arc", shapes, {"op": "set_arc", "cid": sids[4], "radius": 0.6, "sweep": 200.0}),
        ("translate", shapes, {"op": "translate", "ids": sids[4:], "dx": 0.5, "dy": -0.25}),
        ("define_rename_bc", d["channel_three"], {"op": "define_bc", "tag": "inlet", "cls": "InFlowBC_Supersonic",
                                                  "args": "flowState=inflow", "old_tag": "inflow"}),
        ("set_bc_bad_tag", d["channel_three"], {"op": "set_bc", "ids": ["c1"], "tag": "nope"}),
        ("delete_bc", d["channel_three"], {"op": "delete_bc", "tag": "outflow"}),
        ("delete_curves", d["channel_three"], {"op": "delete", "ids": ["c5"]}),
        ("import_naca", Document().to_dict(), {"op": "import_points", "text": naca(0.15, 31),
                                               "options": {"scale": 2.0, "angle_deg": 5.0, "dx": 0.1}}),
    ]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--local", nargs="*", default=[], help="folders of extra .emesh.json projects")
    args = ap.parse_args()
    corpus_dir = os.path.join(HERE, "corpus")
    exp_dir = os.path.join(HERE, "expected")
    for d in (corpus_dir, exp_dir):
        shutil.rmtree(d, ignore_errors=True)
        os.makedirs(d)
    docs = corpus()
    for name, doc in docs.items():
        doc.save(os.path.join(corpus_dir, f"{name}.emesh.json"))
        expected(name, Document.load(os.path.join(corpus_dir, f"{name}.emesh.json")), exp_dir)
    ops_dir = os.path.join(exp_dir, "ops")
    os.makedirs(ops_dir)
    for name, doc_dict, op in operations(docs):
        output, error, ret = run_op(doc_dict, op)
        with open(os.path.join(ops_dir, f"{name}.json"), "w") as f:
            json.dump({"input": doc_dict, "op": op, "output": output, "error": error, "returns": ret}, f, indent=1)
    if args.local:
        local = os.path.join(HERE, "local")
        shutil.rmtree(local, ignore_errors=True)
        os.makedirs(os.path.join(local, "corpus"))
        for folder in args.local:
            for path in sorted(glob.glob(os.path.join(folder, "*.emesh.json"))):
                name = os.path.basename(path)[:-len(".emesh.json")]
                shutil.copy(path, os.path.join(local, "corpus", os.path.basename(path)))
                expected(name, Document.load(path), os.path.join(local, "expected"))
    print(f"{len(docs)} projects, {len(operations(docs))} operations")


if __name__ == "__main__":
    main()
