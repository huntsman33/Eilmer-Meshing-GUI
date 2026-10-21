"""
Phase 4: blocks from regions, count propagation, sub-division, and the live
mesh loop. Eilmer runs check the generated Lua really meshes and connects.
"""
import json
import math
import os
import shutil
import tempfile
import time
import unittest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import QEvent
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from eilmer_mesher.mesh import preview
from eilmer_mesher.mesh.live import LiveMesher
from eilmer_mesher.mesh.runner import export_grids, find_lmr, preview_mesh
from eilmer_mesher.model import blocking, geom
from eilmer_mesher.model.document import Document
from eilmer_mesher.model.entities import SPLIT, ArcSeg, LineSeg
from eilmer_mesher.sketch.model import SketchModel, _mirror_cluster

app = QApplication.instance() or QApplication([])


def have_lmr():
    try:
        find_lmr()
        return True
    except FileNotFoundError:
        return False


def box(x0, y0, x1, y1, kind="geometry"):
    c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    return [LineSeg(None, kind=kind, p0=c[k], p1=c[(k + 1) % 4]) for k in range(4)]


def circle(c, r):
    return [ArcSeg(None, p0=a, p1=b, centre=cc) for a, b, cc in geom.circle_arcs(c, r)]


class ModelTest(unittest.TestCase):
    def setUp(self):
        self.model = SketchModel()
        self.tmp = tempfile.mkdtemp(prefix="emesh-blk-")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)
        # Delete the Qt object now, not in garbage collection at exit (which can crash).
        self.model.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)

    def add(self, curves):
        return self.model.add_curves(curves, "add")

    def res(self):
        return self.model.resolution()

    def eilmer(self, check_preview=True):
        """Run custom-script and prep-grids on the resolved blocks; returns (grids, connections)."""
        res = self.res()
        self.assertEqual(res.errors, {})
        md = self.model.doc.with_blocks(res.blocks)
        result = preview_mesh(md, self.tmp)
        self.assertTrue(result.ok, result.output)
        if check_preview:
            for g, b in zip(result.grids, res.blocks):
                np.testing.assert_allclose(g, preview.block_grid(md, b), atol=1e-9, err_msg=b.id)
        result = export_grids(md, self.tmp)
        self.assertTrue(result.ok, result.output)
        with open(os.path.join(self.tmp, "lmrsim", "grid", "grid.metadata")) as f:
            return result, len(json.load(f)["grid-connections"])


class TestRegionBlocks(ModelTest):
    def setUp(self):
        super().setUp()
        self.add(box(0, 0, 3, 2))
        self.ref = self.add([LineSeg(None, kind=SPLIT, p0=(1, 0), p1=(1, 2))])[0]
        self.left = self.model.make_block((0.5, 1))
        self.right = self.model.make_block((2, 1))

    def test_shared_face_counts_match(self):
        res = self.res()
        b = res.by_id()
        self.assertEqual(b[self.left].njv, b[self.right].njv)
        self.model.set_edge_cells(self.left, "east", 30)
        b = self.res().by_id()
        self.assertEqual((b[self.left].njv, b[self.right].njv), (31, 31))

    def test_count_propagates_through_a_chain(self):
        self.add([LineSeg(None, kind=SPLIT, p0=(2, 0), p1=(2, 2))])
        self.model.make_all_blocks()
        self.assertEqual(len(self.res().blocks), 3)
        self.model.set_edge_cells(self.res().blocks[0].id, "west", 7)
        self.assertEqual({b.njv for b in self.res().blocks}, {8})

    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_eilmer_meshes_and_connects(self):
        _, connections = self.eilmer()
        self.assertEqual(connections, 1)

    def test_block_follows_dragged_geometry(self):
        doc = self.model.doc
        with self.model.edit("drag") as d:
            d.curves[self.ref] = d.curves[self.ref].map_points(lambda p: (p[0] + 0.5, p[1]))
        res = self.res()
        self.assertEqual(res.errors, {})
        left = res.by_id()[self.left]
        np.testing.assert_allclose(doc.ref_endpoints(left.sides["east"][0]), [(1.5, 0), (1.5, 2)])

    def test_split_region_breaks_block_and_block_all_fixes_it(self):
        self.add([LineSeg(None, kind=SPLIT, p0=(0, 1), p1=(1, 1))])
        self.assertIn(self.left, self.res().errors)
        made, skipped = self.model.make_all_blocks()
        self.assertEqual((len(made), skipped), (2, []))
        self.assertEqual(self.res().errors, {})
        self.assertEqual(len(self.model.doc.blocks), 3)

    def test_json_round_trip(self):
        again = Document.from_dict(self.model.doc.to_dict())
        self.assertEqual([b.region for b in again.blocks], [b.region for b in self.model.doc.blocks])
        self.assertNotIn("sides", self.model.doc.to_dict()["blocks"][0])

    def test_clustering_shared_and_mirrored(self):
        spec = {"type": "RobertsFunction", "end0": True, "end1": False, "beta": 1.1}
        self.model.set_edge_cells(self.left, "east", 20, spec, opposite=True)
        stored = {b.id: b for b in self.model.doc.blocks}
        self.assertEqual(stored[self.left].cluster["east"], spec)
        self.assertEqual(stored[self.left].cluster["west"], spec)
        self.assertEqual(stored[self.right].cluster["west"], spec)      # same direction: unchanged
        self.assertEqual(_mirror_cluster(spec)["end1"], True)
        self.assertEqual(_mirror_cluster(spec)["end0"], False)

    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_clustered_preview_matches_eilmer(self):
        spec = {"type": "RobertsFunction", "end0": True, "end1": True, "beta": 1.02}
        self.model.set_edge_cells(self.left, "south", 12, spec)
        self.eilmer()

    def test_warnings(self):
        # Right half split horizontally: the left block's east face only partly matches each.
        self.add([LineSeg(None, kind=SPLIT, p0=(1, 1), p1=(3, 1))])
        self.model.make_all_blocks()
        self.assertTrue(any("partly matches" in w for w in self.res().warnings.get(self.left, [])))


class TestSolidSharedFace(ModelTest):
    def test_solid_line_between_blocks_warns(self):
        self.model.add_curves(box(0, 0, 2, 1) + [LineSeg(None, p0=(1, 0), p1=(1, 1))], "boxes")
        self.model.make_all_blocks()
        warnings = [w for ws in self.res().warnings.values() for w in ws]
        self.assertTrue(any("solid curve" in w for w in warnings))


@unittest.skipUnless(have_lmr(), "lmr not installed")
class TestSubdivide(ModelTest):
    def test_nxm(self):
        self.model.add_curves(box(0, 0, 3, 2), "box")
        made = self.model.subdivide((1, 1), "nxm", n=3, m=2)
        self.assertEqual(len(made), 6)
        _, connections = self.eilmer()
        self.assertEqual(connections, 7)

    def test_disc_ogrid(self):
        self.model.add_curves(circle((0, 0), 1.0), "disc")
        made = self.model.subdivide((0, 0), "ogrid", size=0.5)
        self.assertEqual(len(made), 5)
        _, connections = self.eilmer()
        self.assertEqual(connections, 8)

    def test_ogrid_around_hole(self):
        self.model.add_curves(box(-3, -2, 3, 2) + circle((0, 0), 1.0), "channel")
        made = self.model.subdivide((2.5, 1.5), "ogrid_hole")
        self.assertEqual(len(made), 4)
        self.assertIsNone(self.model.block_at((0, 0)))         # the cylinder itself is not meshed
        _, connections = self.eilmer()
        self.assertEqual(connections, 4)

    def test_centroid_split_triangle(self):
        p = [(0, 0), (2, 0), (1, 1.7)]
        self.model.add_curves([LineSeg(None, p0=p[k], p1=p[(k + 1) % 3]) for k in range(3)], "tri")
        made = self.model.subdivide((1, 0.5), "centroid")
        self.assertEqual(len(made), 3)
        _, connections = self.eilmer()
        self.assertEqual(connections, 3)

    def test_curved_nxm_uses_splines(self):
        top = [ArcSeg(None, p0=a, p1=b, centre=c) for a, b, c in
               geom.arcs_through_3_points((3, 1), (1.5, 1.6), (0, 1))]
        self.model.add_curves([LineSeg(None, p0=(0, 0), p1=(3, 0)), LineSeg(None, p0=(3, 0), p1=(3, 1)),
                               LineSeg(None, p0=(0, 1), p1=(0, 0))] + top, "bump")
        self.model.subdivide((1.5, 0.5), "nxm", n=2, m=2)
        self.assertTrue(any(c.type == "spline" for c in self.model.doc.curves.values()))
        _, connections = self.eilmer()
        self.assertEqual(connections, 4)


class TestErrors(ModelTest):
    def test_messages(self):
        from eilmer_mesher.model.document import BlockError
        p = [(0, 0), (2, 0), (1, 1.7)]
        self.model.add_curves([LineSeg(None, p0=p[k], p1=p[(k + 1) % 3]) for k in range(3)], "tri")
        with self.assertRaisesRegex(BlockError, "only 3 sides"):
            self.model.make_block((1, 0.5))
        with self.assertRaisesRegex(BlockError, "no closed region"):
            self.model.make_block((5, 5))
        self.model.add_curves(box(-3, -2, 3, 2), "box")
        with self.assertRaisesRegex(BlockError, "hole"):
            self.model.make_block((-2.5, 1.5))


@unittest.skipUnless(have_lmr(), "lmr not installed")
class TestLiveMesher(ModelTest):
    def wait_for(self, mesher, source="eilmer", timeout=20.0):
        end = time.time() + timeout
        while time.time() < end:
            QTest.qWait(50)
            m = mesher.mesh
            if not mesher.busy() and m is not None and m.source == source and \
                    m.revision == self.model.revision:
                return m
        self.fail("no mesh from Eilmer in time")

    def test_preview_then_eilmer(self):
        mesher = LiveMesher(self.model)
        mesher.workdir = self.tmp
        self.model.add_curves(box(0, 0, 2, 1), "box")
        self.model.make_block((1, 0.5))
        QTest.qWait(80)
        self.assertIsNotNone(mesher.mesh)                     # the instant preview came first
        mesh = self.wait_for(mesher)
        (bid, grid), = mesh.grids
        res = self.res()
        np.testing.assert_allclose(grid, preview.block_grid(self.model.doc.with_blocks(res.blocks),
                                                            res.blocks[0]), atol=1e-12)

    def test_rapid_edits_end_on_latest_revision(self):
        mesher = LiveMesher(self.model)
        mesher.workdir = self.tmp
        self.model.add_curves(box(0, 0, 2, 1), "box")
        bid = self.model.make_block((1, 0.5))
        for n in (5, 9, 13):
            self.model.set_edge_cells(bid, "south", n)
            QTest.qWait(20)
        mesh = self.wait_for(mesher)
        self.assertEqual(mesh.grids[0][1].shape[0], 14)

    def test_eilmer_error_is_reported(self):
        mesher = LiveMesher(self.model)
        mesher.workdir = self.tmp
        messages = []
        mesher.status.connect(messages.append)
        self.model.add_curves(box(0, 0, 2, 1), "box")
        bid = self.model.make_block((1, 0.5))
        with self.model.edit("bad patch") as doc:
            doc.blocks[0].patch = "AOPatch"
            doc.blocks[0].patch_opts = {"nx": 0, "ny": 0}
        end = time.time() + 20
        while time.time() < end and not any("error" in m.lower() or "Eilmer," in m for m in messages):
            QTest.qWait(50)
        self.assertTrue(messages)


if __name__ == "__main__":
    unittest.main()
