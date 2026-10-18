"""Qt-free sketch geometry: arcs, splines, closest points, intersections, snapping, parsing."""
import math
import unittest

import numpy as np

import fixtures  # noqa: F401  (sets up sys.path)
from eilmer_mesher.model import geom
from eilmer_mesher.model.document import Document
from eilmer_mesher.model.entities import ArcSeg, BezierSeg, LineSeg, SplineSeg
from eilmer_mesher.sketch import snapping
from eilmer_mesher.sketch.snapping import Snapper, nice_step
from eilmer_mesher.sketch.typed_input import parse_point, single_number
from gridgen.path import Spline


def arcs_from(pieces):
    return [ArcSeg(f"a{k}", p0=a, p1=b, centre=c) for k, (a, b, c) in enumerate(pieces)]


def assert_valid_arcs(test, arcs):
    """Every piece must be acceptable to Eilmer's Arc: equal radii, sweep < pi."""
    for a in arcs:
        _, r, _, sweep = geom.arc_geometry(a.p0, a.p1, a.centre)
        r1 = math.hypot(a.p1[0] - a.centre[0], a.p1[1] - a.centre[1])
        test.assertAlmostEqual(r, r1, delta=1e-9 * max(r, 1.0))
        test.assertLess(abs(sweep), math.pi)
        test.assertGreater(abs(sweep), 0.0)


class TestArcs(unittest.TestCase):
    def test_three_point_semicircle_is_split(self):
        pieces = geom.arcs_through_3_points((1, 0), (0, 1), (-1, 0))
        arcs = arcs_from(pieces)
        self.assertEqual(len(arcs), 2)
        assert_valid_arcs(self, arcs)
        self.assertEqual(arcs[0].p0, (1.0, 0.0))
        self.assertEqual(arcs[-1].p1, (-1.0, 0.0))
        np.testing.assert_allclose(arcs[0].centre, (0, 0), atol=1e-12)
        # passes through the top, not the bottom
        self.assertGreater(max(geom.sample(arcs[0])[:, 1].max(), geom.sample(arcs[1])[:, 1].max()), 0.99)

    def test_three_point_major_arc(self):
        # start (1,0), end (0,1) via (0,-1): 270 degrees clockwise
        arcs = arcs_from(geom.arcs_through_3_points((1, 0), (0, -1), (0, 1)))
        assert_valid_arcs(self, arcs)
        total = sum(geom.arc_geometry(a.p0, a.p1, a.centre)[3] for a in arcs)
        self.assertAlmostEqual(total, -1.5 * math.pi, places=9)

    def test_collinear_three_points(self):
        self.assertIsNone(geom.arcs_through_3_points((0, 0), (1, 0), (2, 0)))

    def test_circle_quarters(self):
        arcs = arcs_from(geom.circle_arcs((2, 3), 0.5))
        self.assertEqual(len(arcs), 4)
        assert_valid_arcs(self, arcs)
        for k in range(4):
            self.assertEqual(arcs[k].p1, arcs[(k + 1) % 4].p0)

    def test_tangent_arc(self):
        arcs = arcs_from(geom.tangent_arc((1, 0), (1, 0), (2, 1)))
        self.assertEqual(len(arcs), 1)
        np.testing.assert_allclose(arcs[0].centre, (1, 1), atol=1e-12)
        np.testing.assert_allclose(geom.end_tangent(arcs[0]), (0, 1), atol=1e-12)
        # turning right
        arcs = arcs_from(geom.tangent_arc((0, 0), (1, 0), (1, -1)))
        np.testing.assert_allclose(arcs[0].centre, (0, -1), atol=1e-12)
        self.assertIsNone(geom.tangent_arc((0, 0), (1, 0), (3, 0)))

    def test_centre_start_end_directions(self):
        ccw = arcs_from(geom.arcs_centre_start_end((0, 0), (1, 0), (0, 2), ccw=True))
        cw = arcs_from(geom.arcs_centre_start_end((0, 0), (1, 0), (0, 2), ccw=False))
        assert_valid_arcs(self, ccw + cw)
        self.assertEqual(ccw[-1].p1, (math.cos(math.pi/2), 1.0))   # end projected to radius 1
        self.assertAlmostEqual(sum(geom.arc_geometry(a.p0, a.p1, a.centre)[3] for a in cw), -1.5*math.pi)

    def test_arc_handle_refit(self):
        arc = ArcSeg("a", p0=(1, 0), p1=(0, 1), centre=(0, 0))
        moved = arc.with_handle("mid", (0.6, 0.6))
        self.assertIsNotNone(moved)
        assert_valid_arcs(self, [moved])
        self.assertEqual((moved.p0, moved.p1), ((1.0, 0.0), (0.0, 1.0)))
        self.assertIsNone(arc.with_handle("mid", (-0.5, -0.5)))   # would need > 180 degrees


class TestSplineAndSampling(unittest.TestCase):
    def test_spline_matches_gridgen(self):
        pts = [(0, 0), (1, 0.5), (2, -0.2), (3.5, 0.4), (4, 1.0)]
        B = geom.spline_beziers(pts)
        ref = Spline(pts)
        for seg, gseg in zip(B, ref.segments):
            np.testing.assert_allclose(seg, [(v.x, v.y) for v in gseg.B], atol=1e-9)

    def test_two_point_spline_is_straight(self):
        P = geom.sample(SplineSeg("s", points=[(0, 0), (2, 2)]))
        np.testing.assert_allclose(P[:, 0], P[:, 1], atol=1e-12)

    def test_samples_end_exactly(self):
        for c in (LineSeg("l", p0=(0, 0), p1=(1, 2)), ArcSeg("a", p0=(1, 0), p1=(0, 1), centre=(0, 0)),
                  BezierSeg("b", points=[(0, 0), (1, 2), (3, 0)]),
                  SplineSeg("s", points=[(0, 0), (1, 1), (2, 0)])):
            P = geom.sample(c)
            np.testing.assert_allclose(P[0], c.start(), atol=1e-14)
            np.testing.assert_allclose(P[-1], c.end(), atol=1e-14)


class TestClosestAndIntersections(unittest.TestCase):
    def test_closest_point_arc(self):
        arc = ArcSeg("a", p0=(1, 0), p1=(0, 1), centre=(0, 0))
        q, d = geom.closest_point(arc, (2, 2))
        np.testing.assert_allclose(q, (math.sqrt(0.5), math.sqrt(0.5)), atol=1e-12)
        self.assertAlmostEqual(d, 2*math.sqrt(2) - 1)
        q, _ = geom.closest_point(arc, (-1, -0.1))     # outside the sweep -> nearest end
        self.assertEqual(q, (0.0, 1.0))

    def test_line_line(self):
        a = LineSeg("a", p0=(0, 0), p1=(2, 2))
        b = LineSeg("b", p0=(0, 2), p1=(2, 0))
        np.testing.assert_allclose(geom.intersections(a, b), [(1, 1)], atol=1e-14)
        c = LineSeg("c", p0=(3, 0), p1=(4, 1))
        self.assertEqual(geom.intersections(a, c), [])

    def test_line_arc_and_arc_arc(self):
        circle = arcs_from(geom.circle_arcs((0, 0), 1.0))
        line = LineSeg("l", p0=(-2, 0.5), p1=(2, 0.5))
        hits = [p for a in circle for p in geom.intersections(line, a)]
        hits = sorted(set((round(x, 12), round(y, 12)) for x, y in hits))
        np.testing.assert_allclose(hits, [(-math.sqrt(0.75), 0.5), (math.sqrt(0.75), 0.5)], atol=1e-12)
        other = ArcSeg("o", p0=(2, 0), p1=(0, 0), centre=(1, 0))   # upper half of circle at (1,0)
        hits = [p for a in circle for p in geom.intersections(a, other)]
        self.assertTrue(any(np.allclose(p, (0.5, math.sqrt(0.75)), atol=1e-12) for p in hits))

    def test_spline_line_approximate(self):
        s = SplineSeg("s", points=[(0, -1), (1, 1), (2, -1)])
        line = LineSeg("l", p0=(0, 0), p1=(2, 0))
        hits = geom.intersections(s, line)
        self.assertEqual(len(hits), 2)
        for p in hits:
            self.assertAlmostEqual(p[1], 0.0, places=12)
            _, d = geom.closest_point(s, p)
            self.assertLess(d, 1e-3)

    def test_midpoint(self):
        self.assertEqual(geom.midpoint(LineSeg("l", p0=(0, 0), p1=(2, 4))), (1.0, 2.0))
        mid = geom.midpoint(ArcSeg("a", p0=(1, 0), p1=(0, 1), centre=(0, 0)))
        np.testing.assert_allclose(mid, (math.sqrt(0.5), math.sqrt(0.5)), atol=1e-12)


class TestSnapping(unittest.TestCase):
    def setUp(self):
        self.doc = Document()
        self.doc.add_curve(LineSeg("h", p0=(0, 0), p1=(2, 0)))
        self.doc.add_curve(LineSeg("v", p0=(1, -1), p1=(1, 1)))
        self.doc.add_curve(ArcSeg("a", p0=(4, 0), p1=(3, 1), centre=(3, 0)))
        self.s = Snapper()

    def snap(self, p, **kw):
        return self.s.snap(self.doc, 0, p, 0.1, **kw)

    def test_kinds_and_priority(self):
        self.assertEqual(self.snap((0.05, 0.02)).kind, snapping.ENDPOINT)
        s = self.snap((1.04, 0.03))
        self.assertEqual((s.kind, s.point), (snapping.INTERSECTION, (1.0, 0.0)))
        self.assertEqual(self.snap((3.02, 0.05)).kind, snapping.CENTRE)
        s = self.snap((0.5, 0.05))
        self.assertEqual((s.kind, s.point), (snapping.ON_CURVE, (0.5, 0.0)))
        self.assertEqual(self.snap((1.5, 0.5)).kind, snapping.NONE)

    def test_midpoint_beats_on_curve(self):
        # midpoint of "v" coincides with the intersection, so use the arc's midpoint
        mid = geom.midpoint(self.doc.curves["a"])
        s = self.snap((mid[0] + 0.02, mid[1]))
        self.assertEqual(s.kind, snapping.MIDPOINT)

    def test_exclude(self):
        s = self.snap((0.02, 0.01), exclude={"h"})
        self.assertNotEqual(s.kind, snapping.ENDPOINT)

    def test_horizontal_vertical_and_grid(self):
        s = self.snap((6.0, 3.05), anchor=(5.0, 3.0))
        self.assertEqual((s.kind, s.point), (snapping.HORIZONTAL, (6.0, 3.0)))
        s = self.snap((5.03, 4.0), anchor=(5.0, 3.0))
        self.assertEqual((s.kind, s.point), (snapping.VERTICAL, (5.0, 4.0)))
        self.s.enabled[snapping.GRID] = True
        s = self.snap((6.26, 2.74), grid_step=0.5)
        self.assertEqual((s.kind, s.point), (snapping.GRID, (6.5, 2.5)))

    def test_nice_step(self):
        self.assertEqual(nice_step(0.13), 0.2)
        self.assertEqual(nice_step(3.0), 5.0)
        self.assertAlmostEqual(nice_step(7e-4), 1e-3)


class TestTypedInput(unittest.TestCase):
    def test_forms(self):
        self.assertEqual(parse_point("1.5, -2"), (1.5, -2.0))
        self.assertEqual(parse_point("3 4"), (3.0, 4.0))
        self.assertEqual(parse_point("@1,2", (10, 10)), (11.0, 12.0))
        p = parse_point("@2<90", (1, 1))
        np.testing.assert_allclose(p, (1, 3), atol=1e-12)
        np.testing.assert_allclose(parse_point("1<180"), (-1, 0), atol=1e-12)
        with self.assertRaises(ValueError):
            parse_point("@1,2")
        with self.assertRaises(ValueError):
            parse_point("1,2,3")
        self.assertEqual(single_number(" 0.25 "), 0.25)
        self.assertIsNone(single_number("1,2"))


if __name__ == "__main__":
    unittest.main()
