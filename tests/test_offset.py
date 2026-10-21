"""
Offset tool: chains offset as a whole with exact joins (mitred lines, concentric
arcs, round corners), closed shapes growing or shrinking, refusals, undo and the
pick-then-Enter dialog flow.
"""
import math
import os
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import Qt
from PySide6.QtGui import QKeySequence
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication, QDialog

import fixtures  # noqa: F401
from test_sketch_gui import SketchTest
from eilmer_mesher import settings
from eilmer_mesher.model import geom
from eilmer_mesher.model.document import Document
from eilmer_mesher.model.entities import REFERENCE, ArcSeg, BezierSeg, LineSeg, SplineSeg
from eilmer_mesher.model.offset import find_chains, offset_chains, side_markers
from eilmer_mesher.model.topology import Topology
from eilmer_mesher.sketch.model import SketchModel
from eilmer_mesher.sketch.offset_dialog import OffsetDialog
from eilmer_mesher.units import Units

app = QApplication.instance() or QApplication([])


def rectangle(x0, y0, x1, y1):
    P = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    return [LineSeg(f"r{k}", p0=P[k], p1=P[(k + 1) % 4]) for k in range(4)]


def circle(centre, r):
    return [ArcSeg(f"q{k}", p0=a, p1=b, centre=c) for k, (a, b, c) in enumerate(geom.circle_arcs(centre, r))]


def slot(r=0.5, length=2.0):
    """A rounded slot: two lines and four quarter arcs, all tangent."""
    return [LineSeg("b", p0=(0, -r), p1=(length, -r)),
            ArcSeg("c1", p0=(length, -r), p1=(length + r, 0), centre=(length, 0)),
            ArcSeg("c2", p0=(length + r, 0), p1=(length, r), centre=(length, 0)),
            LineSeg("t", p0=(length, r), p1=(0, r)),
            ArcSeg("c3", p0=(0, r), p1=(-r, 0), centre=(0, 0)),
            ArcSeg("c4", p0=(-r, 0), p1=(0, -r), centre=(0, 0))]


def regions(curves):
    """Closed regions the topology finds in the curves on their own."""
    doc = Document()
    for k, c in enumerate(curves):
        doc.add_curve(c.copy(id=f"n{k}"))
    return Topology(doc).regions


def assert_joined(test, curves, closed=True):
    """Consecutive curves share identical end coordinates."""
    pairs = list(zip(curves, curves[1:] + (curves[:1] if closed else [])))
    for a, b in pairs:
        test.assertEqual(tuple(a.end()), tuple(b.start()))


class TestOffsetGeometry(unittest.TestCase):
    def test_rectangle_grows_and_shrinks_with_sharp_corners(self):
        for reverse, (x0, y0, x1, y1) in ((False, (-0.1, -0.1, 2.1, 1.1)), (True, (0.1, 0.1, 1.9, 0.9))):
            out = offset_chains(rectangle(0, 0, 2, 1), 0.1, reverse)
            self.assertEqual([c.type for c in out], ["line"] * 4)
            assert_joined(self, out)
            corners = {tuple(np.round(c.p0, 12)) for c in out}
            self.assertEqual(corners, {(x0, y0), (x1, y0), (x1, y1), (x0, y1)})
            (region,) = regions(out)
            self.assertAlmostEqual(region.area, (x1 - x0) * (y1 - y0), places=9)

    def test_clockwise_rectangle_also_grows(self):
        cw = [LineSeg(c.id, p0=c.p1, p1=c.p0) for c in reversed(rectangle(0, 0, 2, 1))]
        (region,) = regions(offset_chains(cw, 0.1))
        self.assertAlmostEqual(region.area, 2.2 * 1.2, places=9)

    def test_circle_gives_concentric_circle(self):
        for reverse, r in ((False, 1.25), (True, 0.75)):
            out = offset_chains(circle((1, 2), 1.0), 0.25, reverse)
            self.assertEqual([c.type for c in out], ["arc"] * 4)
            assert_joined(self, out)
            for c in out:
                self.assertEqual(c.centre, (1.0, 2.0))
                self.assertAlmostEqual(c.radius(), r, places=12)
            (region,) = regions(out)
            self.assertAlmostEqual(region.area, math.pi * r * r, delta=0.01 * r * r)

    def test_open_polyline_is_mitred_on_both_sides(self):
        poly = [LineSeg("a", p0=(0, 0), p1=(1, 0)), LineSeg("b", p0=(1, 0), p1=(1, 1)),
                LineSeg("c", p0=(1, 1), p1=(2, 1))]
        left = offset_chains(poly, 0.1)
        assert_joined(self, left, closed=False)
        np.testing.assert_allclose([c.p0 for c in left] + [left[-1].p1],
                                   [(0, 0.1), (0.9, 0.1), (0.9, 1.1), (2, 1.1)], atol=1e-14)
        right = offset_chains(poly, 0.1, reverse=True)
        np.testing.assert_allclose([c.p0 for c in right] + [right[-1].p1],
                                   [(0, -0.1), (1.1, -0.1), (1.1, 0.9), (2, 0.9)], atol=1e-14)

    def test_open_chain_follows_most_curves_and_markers_show_the_side(self):
        poly = [LineSeg("a", p0=(0, 0), p1=(1, 0)), LineSeg("b", p0=(2, 0), p1=(1, 0)),   # drawn backwards
                LineSeg("c", p0=(2, 0), p1=(3, 0))]
        (chain,) = find_chains(poly)
        self.assertFalse(chain.closed)
        self.assertEqual(chain.curves[0].start(), (0, 0))
        out = offset_chains(poly, 0.5)
        self.assertTrue(all(abs(c.p0[1] - 0.5) < 1e-14 for c in out))
        assert_joined(self, out, closed=False)
        ((p, n),) = side_markers(poly)
        np.testing.assert_allclose(n, (0, 1), atol=1e-9)
        ((p, n),) = side_markers(poly, reverse=True)
        np.testing.assert_allclose(n, (0, -1), atol=1e-9)

    def test_rounded_slot_stays_connected(self):
        for reverse, r in ((False, 0.6), (True, 0.4)):
            out = offset_chains(slot(), 0.1, reverse)
            self.assertEqual(len(out), 6)
            assert_joined(self, out)
            (region,) = regions(out)
            self.assertAlmostEqual(region.area, 2.0 * 2 * r + math.pi * r * r, delta=2e-3)

    def test_line_and_arc_corners_round_outside_and_trim_inside(self):
        # a half disc: two quarter arcs and the diameter, with sharp corners at (+-1, 0)
        half = [ArcSeg("a", p0=(1, 0), p1=(0, 1), centre=(0, 0)),
                ArcSeg("b", p0=(0, 1), p1=(-1, 0), centre=(0, 0)),
                LineSeg("l", p0=(-1, 0), p1=(1, 0))]
        grown = offset_chains(half, 0.1)
        assert_joined(self, grown)
        corners = [c for c in grown if c.type == "arc" and c.centre in ((1.0, 0.0), (-1.0, 0.0))]
        self.assertEqual(len(corners), 2)                     # round corners about the old vertices
        self.assertTrue(all(abs(c.radius() - 0.1) < 1e-12 for c in corners))
        (region,) = regions(grown)
        self.assertAlmostEqual(region.area, math.pi / 2 * 1.21 + 0.2 + math.pi * 0.01 / 2, delta=5e-3)

        shrunk = offset_chains(half, 0.1, reverse=True)
        self.assertEqual([c.type for c in shrunk], ["arc", "arc", "line"])
        assert_joined(self, shrunk)
        self.assertAlmostEqual(shrunk[-1].p0[1], 0.1, places=12)
        self.assertAlmostEqual(math.hypot(*shrunk[-1].p0), 0.9, places=12)
        theta = 2.0 * math.acos(0.1 / 0.9)
        (region,) = regions(shrunk)
        self.assertAlmostEqual(region.area, 0.405 * (theta - math.sin(theta)), delta=5e-3)

    def test_spline_offset_is_a_spline_at_the_distance(self):
        sp = SplineSeg("s", points=[(0, 0), (1, 0.3), (2, 0.1), (3, 0.5)])
        (out,) = offset_chains([sp], 0.05)
        self.assertEqual(out.type, "spline")
        for p in geom.sample(out)[::5]:
            _, _, d = geom.closest_param(sp, p)
            self.assertAlmostEqual(d, 0.05, delta=1e-6)
        bez = BezierSeg("b", points=[(0, 0), (1, 1), (2, 0)])
        (out,) = offset_chains([bez], 0.1)
        self.assertGreater(out.points[len(out.points) // 2][1], 0.5)   # left of travel: above

    def test_closed_spline_and_line_grow_and_shrink(self):
        shape = [SplineSeg("s", points=[(0, 0), (0.3, 0.35), (0.7, 0.35), (1, 0)]),
                 LineSeg("l", p0=(1, 0), p1=(0, 0))]
        area0 = regions(shape)[0].area
        grown = offset_chains(shape, 0.02)
        shrunk = offset_chains(shape, 0.02, reverse=True)
        for out in (grown, shrunk):
            assert_joined(self, out)
            self.assertEqual(len(regions(out)), 1)
        self.assertGreater(regions(grown)[0].area, area0)
        self.assertLess(regions(shrunk)[0].area, area0)

    def test_too_large_shrinks_are_refused(self):
        with self.assertRaisesRegex(ValueError, "too large"):
            offset_chains(rectangle(0, 0, 2, 1), 0.6, reverse=True)
        with self.assertRaisesRegex(ValueError, "too large"):
            offset_chains(rectangle(0, 0, 2, 1), 0.5, reverse=True)       # collapses to a line
        with self.assertRaisesRegex(ValueError, "radius would vanish"):
            offset_chains(circle((0, 0), 1.0), 1.0, reverse=True)
        with self.assertRaisesRegex(ValueError, "positive"):
            offset_chains(rectangle(0, 0, 2, 1), 0.0)

    def test_separate_shapes_are_offset_separately(self):
        curves = rectangle(0, 0, 1, 1) + circle((5, 0), 1.0)
        out = offset_chains(curves, 0.1)
        self.assertEqual(len(out), 8)
        self.assertEqual(len(regions(out)), 2)


class TestOffsetModel(unittest.TestCase):
    def setUp(self):
        self.model = SketchModel()

    def tearDown(self):
        self.model.deleteLater()

    def test_one_undo_step_keeps_originals_and_kind(self):
        rect = rectangle(0, 0, 2, 1)
        rect[0] = rect[0].copy(kind=REFERENCE)
        rect[1] = rect[1].copy(bc_tag="inflow")
        ids = self.model.add_curves(rect, "Rectangle")
        steps = self.model.undo_stack.count()
        new = self.model.offset(ids, 0.1)
        self.assertEqual(len(new), 4)
        self.assertEqual(self.model.undo_stack.count(), steps + 1)
        self.assertEqual(self.model.undo_stack.undoText(), "Offset")
        curves = [self.model.doc.curves[i] for i in new]
        self.assertEqual([c.kind for c in curves], [REFERENCE, "geometry", "geometry", "geometry"])
        self.assertTrue(all(c.bc_tag == "" for c in curves))
        self.assertTrue(all(i in self.model.doc.curves for i in ids))
        self.model.undo_stack.undo()
        self.assertEqual(set(self.model.doc.curves), set(ids))

    def test_refusal_leaves_the_document_alone(self):
        ids = self.model.add_curves(circle((0, 0), 1.0), "Circle")
        steps = self.model.undo_stack.count()
        with self.assertRaises(ValueError):
            self.model.offset(ids, 2.0, reverse=True)
        self.assertEqual(set(self.model.doc.curves), set(ids))
        self.assertEqual(self.model.undo_stack.count(), steps)


class TestOffsetDialog(unittest.TestCase):
    def test_display_units_and_validation(self):
        dlg = OffsetDialog(Units("mm"), 0.01)
        seen = []
        dlg.valuesChanged.connect(lambda d, r: seen.append((d, r)))
        self.assertEqual(dlg.distance_edit.text(), "10")
        dlg.set_reverse(True)
        self.assertAlmostEqual(seen[-1][0], 0.01)
        self.assertTrue(seen[-1][1])
        dlg.distance_edit.setText("-3")
        dlg.refresh()
        self.assertIn("positive", dlg.error.text())
        self.assertFalse(dlg.buttons.button(dlg.buttons.StandardButton.Ok).isEnabled())
        dlg.deleteLater()


class TestOffsetTool(SketchTest):
    def setUp(self):
        super().setUp()
        self.use("rectangle")
        self.click((-1, -0.5), (1, 0.5))
        self.rect = set(self.model.doc.curves)
        self.use("offset")
        self.click((0, -0.5), (1, 0), (0, 0.5), (-1, 0))        # the four sides

    def run_dialog(self, distance, reverse, accept=True):
        seen = {}

        def fake_exec(dlg):
            dlg.set_distance(distance)
            dlg.set_reverse(reverse)
            tool = self.tools["offset"]
            seen["preview"] = list(tool.preview)
            seen["markers"] = list(tool.markers)
            seen["error"] = dlg.error.text()
            return QDialog.Accepted if accept and not seen["error"] else QDialog.Rejected

        with mock.patch.object(OffsetDialog, "exec", fake_exec):
            QTest.keyClick(self.canvas, Qt.Key_Return)
        return seen

    def test_pick_enter_and_accept(self):
        self.assertEqual(self.model.selection, self.rect)
        seen = self.run_dialog(0.25, reverse=False)
        self.assertEqual(len(seen["preview"]), 4)
        self.assertEqual(len(seen["markers"]), 1)
        new = set(self.model.doc.curves) - self.rect
        self.assertEqual(len(new), 4)
        self.assertEqual(self.model.selection, new)
        (region,) = regions([self.model.doc.curves[i] for i in new])
        self.assertAlmostEqual(region.area, 2.5 * 1.5, places=9)
        self.assertEqual(len(Topology(self.model.doc).regions), 2)      # the gap and the old rectangle
        self.assertEqual(self.tools["offset"].preview, [])
        self.assertAlmostEqual(self.tools["offset"].last_distance, 0.25)
        self.model.undo_stack.undo()
        self.assertEqual(set(self.model.doc.curves), self.rect)

    def test_source_to_reference_option(self):
        first = next(iter(self.rect))
        self.model.doc.curves[first].bc_tag = "wall"
        tool = self.tools["offset"]
        seen = {}

        def fake_exec(dlg):
            self.assertFalse(dlg.source_to_reference())
            dlg.set_source_to_reference(True)
            seen["src"] = len(tool.source_curves)
            return QDialog.Accepted

        with mock.patch.object(OffsetDialog, "exec", fake_exec):
            QTest.keyClick(self.canvas, Qt.Key_Return)
        self.assertEqual(seen["src"], 4)                       # sources previewed dashed
        self.assertTrue(tool.last_source_ref)
        docc = self.model.doc.curves
        self.assertTrue(all(docc[i].kind == REFERENCE for i in self.rect))
        self.assertEqual(docc[first].bc_tag, "")
        new = set(docc) - self.rect
        self.assertEqual(len(new), 4)
        self.assertTrue(all(docc[i].kind != REFERENCE for i in new))
        self.model.undo_stack.undo()                           # one step undoes both
        self.assertEqual(set(self.model.doc.curves), self.rect)
        self.assertTrue(all(self.model.doc.curves[i].kind != REFERENCE for i in self.rect))
        self.assertEqual(self.model.doc.curves[first].bc_tag, "wall")

    def test_source_to_reference_remembered_and_failure_changes_nothing(self):
        tool = self.tools["offset"]
        tool.last_source_ref = True
        seen = {}

        def fake_exec(dlg):
            seen["initial"] = dlg.source_to_reference()
            dlg.set_distance(0.25)
            return QDialog.Rejected

        with mock.patch.object(OffsetDialog, "exec", fake_exec):
            QTest.keyClick(self.canvas, Qt.Key_Return)
        self.assertTrue(seen["initial"])
        self.assertEqual(tool.source_curves, [])               # preview cleared
        n = self.model.undo_stack.count()
        with self.assertRaises(ValueError):
            self.model.offset(list(self.rect), 0.6, True, True)
        self.assertEqual(self.model.undo_stack.count(), n)
        self.assertTrue(all(self.model.doc.curves[i].kind != REFERENCE for i in self.rect))

    def test_click_again_drops_a_curve_and_cancel_makes_nothing(self):
        self.click((-1, 0))
        self.assertEqual(len(self.model.selection), 3)
        seen = self.run_dialog(0.1, reverse=True, accept=False)
        self.assertEqual(len(seen["preview"]), 3)              # an open chain of three sides
        self.assertEqual(set(self.model.doc.curves), self.rect)

    def test_too_large_shrink_shows_an_error(self):
        seen = self.run_dialog(0.6, reverse=True)
        self.assertIn("too large", seen["error"])
        self.assertEqual(seen["preview"], [])
        self.assertEqual(set(self.model.doc.curves), self.rect)

    def test_default_shortcut(self):
        self.assertEqual(settings.DEFAULT_SHORTCUTS["tool.offset"], ("Offset", "F"))
        self.assertEqual(QKeySequence(settings.shortcut("tool.offset")), QKeySequence("F"))


if __name__ == "__main__":
    unittest.main()
