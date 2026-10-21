"""Exact intersections and splits, Eilmer-consistent parameterisation, planar topology."""
import math
import shutil
import tempfile
import unittest

import numpy as np

import fixtures  # noqa: F401
from fixtures import sides
from eilmer_mesher.lua.codegen import generate_geometry_lua
from eilmer_mesher.mesh import preview
from eilmer_mesher.mesh.runner import find_lmr, preview_mesh
from eilmer_mesher.model import geom
from eilmer_mesher.model.document import Block, Document
from eilmer_mesher.model.entities import REFERENCE, SPLIT, ArcSeg, BezierSeg, LineSeg, SplineSeg
from eilmer_mesher.model.topology import Topology, curve_params

SPLINE = SplineSeg("s", points=[(0, 0), (0.5, 0.2), (1, 0), (1.5, -0.1), (2, 0)])


def have_lmr():
    try:
        find_lmr()
        return True
    except FileNotFoundError:
        return False


class TestExactGeometry(unittest.TestCase):
    def test_spline_line_intersection_is_exact(self):
        line = LineSeg("l", p0=(0.4, -1), p1=(0.4, 1))
        (t1, t2, p), = geom.intersect(SPLINE, line)
        self.assertAlmostEqual(p[0], 0.4, places=13)
        np.testing.assert_allclose(geom.point_at(SPLINE, t1), geom.point_at(line, t2), atol=1e-13)

    def test_bezier_arc_intersection_is_exact(self):
        b = BezierSeg("b", points=[(0.2, 0.0), (0.5, 1.5), (1.5, 1.2)])   # starts inside the circle
        arc = ArcSeg("a", p0=(1, 0), p1=(0, 1), centre=(0, 0))
        hits = geom.intersect(b, arc)
        self.assertEqual(len(hits), 1)
        t1, t2, p = hits[0]
        self.assertAlmostEqual(math.hypot(*p), 1.0, places=13)
        np.testing.assert_allclose(geom.point_at(b, t1), geom.point_at(arc, t2), atol=1e-13)

    def test_subcurve_reparameterises_linearly(self):
        curves = [LineSeg("l", p0=(0, 0), p1=(2, 1)),
                  ArcSeg("a", p0=(1, 0), p1=(0, 1), centre=(0, 0)),
                  BezierSeg("b", points=[(0, 0), (1, 2), (3, -1), (4, 0)]),
                  SPLINE]
        for c in curves:
            sub = geom.subcurve(c, 0.2, 0.7)
            for s in (0.0, 0.3, 1.0):
                np.testing.assert_allclose(geom.point_at(sub, s), geom.point_at(c, 0.2 + 0.5 * s),
                                           atol=1e-13, err_msg=c.type)

    def test_closest_param_round_trip(self):
        for t in (0.0, 0.123, 0.5, 0.987, 1.0):
            p = geom.point_at(SPLINE, t)
            t2, q, d = geom.closest_param(SPLINE, p)
            self.assertAlmostEqual(t2, t, places=9)
            self.assertLess(d, 1e-12)

    def test_trimmed_spline_lua(self):
        sub = geom.subcurve(SPLINE, 0.25, 0.75)
        doc = Document()
        doc.add_curve(sub)
        expr = sub.lua_expr(lambda p: "P")
        self.assertTrue(expr.startswith("SubRangedPath:new{underlying_path=Spline:new{points={"))
        self.assertIn("t0=0.25, t1=0.75}", expr)
        again = Document.from_dict(doc.to_dict())
        self.assertEqual((again.curves["s"].t0, again.curves["s"].t1), (0.25, 0.75))


@unittest.skipUnless(have_lmr(), "lmr not installed")
class TestTrimmedSplineInEilmer(unittest.TestCase):
    """A block with a trimmed-spline face and a line+Bezier polyline face: Eilmer and the preview agree."""

    def test_block(self):
        left = LineSeg("left", p0=(0.4, -1), p1=(0.4, 2))
        right = LineSeg("right", p0=(1.6, -1), p1=(1.6, 2))
        (ta, _, pa), = geom.intersect(SPLINE, left)
        (tb, _, pb), = geom.intersect(SPLINE, right)
        doc = Document()
        doc.add_curve(geom.subcurve(SPLINE, ta, tb))
        doc.add_curve(LineSeg("w", p0=pa, p1=(0.4, 1)))
        doc.add_curve(LineSeg("e", p0=pb, p1=(1.6, 1)))
        doc.add_curve(LineSeg("n1", p0=(0.4, 1), p1=(1.0, 1)))
        doc.add_curve(BezierSeg("n2", points=[(1.0, 1), (1.3, 1.3), (1.6, 1)]))
        doc.add_block(Block("blk", sides(south="s", west="w", east="e", north=["n1", "n2"]), niv=17, njv=9))
        self.assertIn("SubRangedPath", generate_geometry_lua(doc))
        tmp = tempfile.mkdtemp(prefix="emesh-trim-")
        try:
            result = preview_mesh(doc, tmp)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
        self.assertTrue(result.ok, result.output)
        eilmer = result.grids[0]
        np.testing.assert_allclose(eilmer[0, 0, :2], pa, atol=1e-9)      # trimmed ends land on the cut
        np.testing.assert_allclose(eilmer[-1, 0, :2], pb, atol=1e-9)
        np.testing.assert_allclose(eilmer, preview.block_grid(doc, doc.blocks[0]), atol=1e-9)


def box(doc, x0, y0, x1, y1, prefix="b"):
    c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    for k in range(4):
        doc.add_curve(LineSeg(f"{prefix}{k}", p0=c[k], p1=c[(k + 1) % 4]))


class TestTopology(unittest.TestCase):
    def test_split_line_splits_box(self):
        doc = Document()
        box(doc, 0, 0, 3, 2)
        doc.add_curve(LineSeg("ref", kind=SPLIT, p0=(1, 0), p1=(1, 2)))
        topo = Topology(doc)
        self.assertEqual(sorted(round(r.area, 12) for r in topo.regions), [2.0, 4.0])
        self.assertAlmostEqual(topo.region_at((0.5, 1)).area, 2.0)
        self.assertAlmostEqual(topo.region_at((2.0, 1)).area, 4.0)
        self.assertIsNone(topo.region_at((5, 5)))

    def test_reference_lines_are_ignored(self):
        doc = Document()
        box(doc, 0, 0, 3, 2)
        doc.add_curve(LineSeg("axis", kind=REFERENCE, p0=(1, -1), p1=(1, 3)))   # a drawing aid
        self.assertEqual([round(r.area, 12) for r in Topology(doc).regions], [6.0])

    def test_version_1_dashed_lines_become_split_lines(self):
        doc = Document()
        box(doc, 0, 0, 3, 2)
        doc.add_curve(LineSeg("ref", kind=SPLIT, p0=(1, 0), p1=(1, 2)))
        old = doc.to_dict()
        old["version"] = 1
        old["curves"][-1]["kind"] = REFERENCE                  # how version 1 saved block splitters
        loaded = Document.from_dict(old)
        self.assertEqual(loaded.curves["ref"].kind, SPLIT)
        self.assertEqual(len(Topology(loaded).regions), 2)
        again = Document.from_dict(loaded.to_dict())           # saved as version 2: no conversion
        again.curves["ref"].kind = REFERENCE
        self.assertEqual(Document.from_dict(again.to_dict()).curves["ref"].kind, REFERENCE)

    def test_t_junction_and_dangling(self):
        doc = Document()
        box(doc, 0, 0, 2, 2)
        doc.add_curve(LineSeg("t", p0=(1, 0), p1=(1, 1)))          # ends mid-box: dangling, pruned
        doc.add_curve(LineSeg("u", p0=(0, 1.5), p1=(2, 1.5)))      # crosses wall to wall
        topo = Topology(doc)
        self.assertEqual(sorted(round(r.area, 12) for r in topo.regions), [1.0, 3.0])
        self.assertEqual(curve_params(doc, "b0"), [0.0, 0.5, 1.0])  # T-junction splits the bottom

    def test_island_is_a_hole(self):
        doc = Document()
        box(doc, -2, -2, 2, 2)
        for k, (a, b, c) in enumerate(geom.circle_arcs((0, 0), 1.0)):
            doc.add_curve(ArcSeg(f"q{k}", p0=a, p1=b, centre=c))
        topo = Topology(doc)
        self.assertEqual(len(topo.regions), 2)
        disc, outer = topo.regions
        self.assertAlmostEqual(disc.area, math.pi, places=2)
        self.assertEqual(len(outer.holes), 1)
        self.assertAlmostEqual(outer.area, 16 - disc.area, places=9)
        self.assertIs(topo.region_at((1.5, 1.5)), outer)
        self.assertIs(topo.region_at((0.1, 0.1)), disc)

    def test_crossing_lines_make_four_regions(self):
        doc = Document()
        box(doc, 0, 0, 2, 2)
        doc.add_curve(LineSeg("d1", p0=(0, 0), p1=(2, 2)))
        doc.add_curve(LineSeg("d2", p0=(0, 2), p1=(2, 0)))
        topo = Topology(doc)
        self.assertEqual(len(topo.regions), 4)
        for r in topo.regions:
            self.assertAlmostEqual(r.area, 1.0)


if __name__ == "__main__":
    unittest.main()
