"""
Phase 3 GUI behaviour: trim, point tool, typed locks, hold keys, arc flow,
dimension dialogs, display units, shortcut settings and the launcher.
"""
import math
import os
import runpy
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import QEvent, Qt
from PySide6.QtGui import QKeySequence
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from test_sketch_gui import SketchTest, scr
from eilmer_mesher import settings
from eilmer_mesher.app import MainWindow
from eilmer_mesher.model import geom
from eilmer_mesher.model.document import Block, CurveRef
from eilmer_mesher.model.entities import LineSeg, SplineSeg
from eilmer_mesher.sketch.edit_dialogs import dialog_for
from eilmer_mesher.units import Units

app = QApplication.instance() or QApplication([])
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


class TestTrim(SketchTest):
    def setUp(self):
        super().setUp()
        self.use("line")
        self.click((-2, 0), (2, 0))          # horizontal
        self.click((0, -1), (0, 1))          # vertical, crossing at the origin
        self.h, self.v = list(self.model.doc.curves)
        self.use("trim")

    def test_trim_arm_ends_exactly_at_intersection(self):
        self.click((1.5, 0))
        h = self.model.doc.curves[self.h]
        self.assertEqual(h.p0, (-2.0, 0.0))
        np.testing.assert_allclose(h.p1, (0, 0), atol=1e-15)

    def test_trim_middle_piece_leaves_two(self):
        self.use("line")
        self.click((1, -1), (1, 1))          # second crossing at x = 1
        self.use("trim")
        self.click((0.5, 0))                 # piece between x = 0 and x = 1
        xs = sorted((c.p0[0], c.p1[0]) for c in self.curves("line") if abs(c.p0[1]) < 1e-12)
        np.testing.assert_allclose(xs, [(-2, 0), (1, 2)], atol=1e-14)
        self.model.undo_stack.undo()
        self.assertEqual(len(self.curves("line")), 3)

    def test_trim_free_curve_deletes_it(self):
        self.use("line")
        self.click((-2, 2), (2, 2))          # meets nothing
        self.use("trim")
        self.click((0, 2))
        self.assertEqual(len(self.curves()), 2)

    def test_trim_spline_gives_subranged_spline(self):
        self.model.add_curves([SplineSeg(None, points=[(-1, -0.5), (0.5, 0.5), (1.5, -0.5)])], "s")
        sid = list(self.model.doc.curves)[-1]
        self.click((1.3, -0.3))             # the piece past where it crosses the horizontal line
        s = self.model.doc.curves[sid]
        self.assertTrue(s.trimmed())
        self.assertIn("SubRangedPath", self.model.doc.curves[sid].lua_expr(lambda p: "P"))
        # the new end lies on the horizontal line
        self.assertAlmostEqual(s.end()[1], 0.0, places=12)

    def test_trim_drops_blocks_using_the_curve(self):
        doc = self.model.doc
        doc.add_block(Block("b", {f: [CurveRef(self.h)] for f in ("north", "east", "south", "west")}))
        self.click((1.5, 0))
        self.assertEqual(self.model.doc.blocks, [])


class TestPointsAndLocks(SketchTest):
    def test_point_tool_and_snap(self):
        self.use("point")
        self.click((0.5, 0.25))
        (pid,) = self.model.doc.points
        self.use("line")
        self.click((0.53, 0.27), (1, 1))     # snaps to the point
        self.assertEqual(self.curves()[0].p0, (0.5, 0.25))
        self.model.undo_stack.undo()
        self.model.undo_stack.undo()
        self.assertEqual(self.model.doc.points, {})

    def test_select_and_move_point(self):
        pid = self.model.add_point((0, 0))
        self.use("select")
        self.drag((0, 0), (1, 0.5))
        np.testing.assert_allclose(self.model.doc.points[pid], (1, 0.5))

    def test_point_and_curve_selected_together_paint(self):
        pid = self.model.add_point((0, 0))
        self.model.add_curves([LineSeg(None, p0=(1, 0), p1=(2, 0))], "line")
        self.model.set_selection(self.model.entity_ids())
        self.use("select")
        self.canvas.grab()                    # handles are drawn for curves only
        self.assertIn(pid, self.model.selection)

    def test_typed_length_lock_follows_cursor_direction(self):
        self.use("line")
        self.click((0, 0))
        self.canvas.preview_text("2")
        self.click((1, 1))
        (line,) = self.curves()
        np.testing.assert_allclose(line.p1, (math.sqrt(2), math.sqrt(2)), atol=1e-12)

    def test_typed_values_use_display_units(self):
        self.canvas.units = Units("mm")
        self.use("line")
        self.assertIsNone(self.canvas.submit_text("0, 0"))
        self.assertIsNone(self.canvas.submit_text("@500, 250"))
        (line,) = self.curves()
        np.testing.assert_allclose(line.p1, (0.5, 0.25), atol=1e-15)
        self.use("circle")
        self.canvas.submit_text("0, 0")
        self.canvas.submit_text("100")
        self.assertAlmostEqual(self.curves("arc")[0].radius(), 0.1)

    def test_hold_h_and_v(self):
        self.use("line")
        self.click((0, 0))
        QTest.keyPress(self.canvas, Qt.Key_H)
        self.click((1, 0.7))
        QTest.keyRelease(self.canvas, Qt.Key_H)
        self.assertEqual(self.curves()[0].p1, (1.0, 0.0))
        self.click((0, -1))
        QTest.keyPress(self.canvas, Qt.Key_V)
        self.click((0.6, -2))
        QTest.keyRelease(self.canvas, Qt.Key_V)
        self.assertEqual(self.curves()[1].p1, (0.0, -2.0))
        self.assertEqual(self.canvas.held, set())


class TestArcFlow(SketchTest):
    def sweep_to(self, *points):
        for p in points:
            QTest.mouseMove(self.canvas, scr(p))

    def test_typed_radius_and_angle(self):
        self.use("arc")
        self.click((0, 0))                  # centre
        self.canvas.preview_text("1")       # radius
        self.click((2, 0))                  # start direction: +x
        self.canvas.preview_text("90")      # sweep
        self.sweep_to((1, 0.5), (0.5, 1))   # moving anticlockwise
        self.click((0.4, 1.2))
        (arc,) = self.curves("arc")
        np.testing.assert_allclose(arc.p0, (1, 0), atol=1e-15)
        np.testing.assert_allclose(arc.p1, (0, 1), atol=1e-12)

    def test_clockwise_by_motion(self):
        self.use("arc")
        self.click((0, 0), (1, 0))
        self.canvas.preview_text("90")
        self.sweep_to((1, -0.5), (0.5, -1))
        self.click((0.5, -1))
        (arc,) = self.curves("arc")
        np.testing.assert_allclose(arc.p1, (0, -1), atol=1e-12)

    def test_past_180_degrees(self):
        self.use("arc")
        self.click((0, 0), (1, 0))
        self.sweep_to((0.7, 0.7), (0, 1), (-0.7, 0.7), (-1, 0.05), (-1, -0.5))
        self.click((-1, -0.5))
        arcs = self.curves("arc")
        self.assertEqual(len(arcs), 2)
        total = sum(geom.arc_geometry(a.p0, a.p1, a.centre)[3] for a in arcs)
        self.assertAlmostEqual(math.degrees(total), 180 + math.degrees(math.atan2(0.5, 1)), places=6)


class TestDimensionDialogs(SketchTest):
    def setUp(self):
        super().setUp()
        self.use("rectangle")
        self.click((0, 0), (2, 1))
        self.south = next(c.id for c in self.curves() if c.p0 == (0.0, 0.0))

    def test_line_length_moves_connected_end(self):
        dlg, apply = dialog_for(self.model, Units("m"), self.south)
        self.assertEqual([round(v, 9) for v in dlg.values()], [2.0, 0.0])
        apply([3.0, 0.0])
        self.assertEqual(self.model.doc.curves[self.south].p1, (3.0, 0.0))
        east = next(c for c in self.curves() if c.p1 == (2.0, 1.0))
        self.assertEqual(east.p0, (3.0, 0.0))            # the east side followed
        self.model.undo_stack.undo()
        self.assertEqual(self.model.doc.curves[self.south].p1, (2.0, 0.0))

    def test_units_in_dialog(self):
        dlg, apply = dialog_for(self.model, Units("mm"), self.south)
        self.assertAlmostEqual(dlg.values()[0], 2000.0)
        apply([1500.0, 0.0])
        self.assertAlmostEqual(self.model.doc.curves[self.south].p1[0], 1.5)

    def test_arc_radius_and_sweep(self):
        self.use("circle")
        self.click((5, 0))
        self.canvas.submit_text("1")
        arc = self.curves("arc")[0]                      # quarter from +x to +y
        dlg, apply = dialog_for(self.model, Units("m"), arc.id)
        np.testing.assert_allclose(dlg.values(), [1.0, 90.0])
        apply([2.0, 45.0])
        a = self.model.doc.curves[arc.id]
        np.testing.assert_allclose(a.p0, (7, 0), atol=1e-12)
        np.testing.assert_allclose(a.p1, (5 + 2*math.cos(math.pi/4), 2*math.sin(math.pi/4)), atol=1e-12)
        # neighbours on both ends followed
        ends = {tuple(np.round(c.p1, 9)) for c in self.curves("arc")} | \
               {tuple(np.round(c.p0, 9)) for c in self.curves("arc")}
        self.assertIn((7.0, 0.0), ends)

    def test_point_dialog(self):
        pid = self.model.add_point((1, 1))
        dlg, apply = dialog_for(self.model, Units("in"), pid)
        np.testing.assert_allclose(dlg.values(), [1 / 0.0254, 1 / 0.0254])
        apply([10.0, 20.0])
        np.testing.assert_allclose(self.model.doc.points[pid], (0.254, 0.508))


class TestMerge(SketchTest):
    def setUp(self):
        super().setUp()
        self.messages = []
        self.canvas.message.connect(self.messages.append)

    def merge_by_clicking(self, *points):
        self.use("merge")
        self.click(*points)
        QTest.keyClick(self.canvas, Qt.Key_Return)

    def test_collinear_lines_including_reversed(self):
        self.use("line")
        self.click((0, 0), (1, 0))
        self.click((1, 0), (2, 0))
        self.click((3, 0), (2, 0))          # drawn backwards
        self.merge_by_clicking((0.5, 0), (1.5, 0), (2.5, 0))
        (line,) = self.curves()
        self.assertEqual({line.p0, line.p1}, {(0.0, 0.0), (3.0, 0.0)})
        self.assertIn("into 1", self.messages[-1])
        self.model.undo_stack.undo()
        self.assertEqual(len(self.curves()), 3)

    def test_corner_does_not_merge(self):
        self.use("profile")
        self.click((0, 0), (1, 0), (1, 1))
        QTest.keyClick(self.canvas, Qt.Key_Escape)
        self.merge_by_clicking((0.5, 0), (1, 0.5))
        self.assertEqual(len(self.curves()), 2)
        self.assertTrue(self.messages[-1].startswith("Nothing to merge"))

    def test_arcs_of_one_circle(self):
        from eilmer_mesher.model.entities import ArcSeg
        (p0, p1, c), = geom.arc_pieces((0, 0), 1.0, 0.0, math.radians(80))   # one 80 degree arc
        mid = geom.arc_point(ArcSeg("tmp", p0=p0, p1=p1, centre=c), 0.5)
        self.model.add_curves([ArcSeg(None, p0=p0, p1=mid, centre=c),
                               ArcSeg(None, p0=mid, p1=p1, centre=c)], "arcs")
        self.model.set_selection(self.model.doc.curves)
        self.use("merge")
        QTest.keyClick(self.canvas, Qt.Key_Return)
        (arc,) = self.curves("arc")
        self.assertEqual((arc.p0, arc.p1), (p0, p1))

    def test_quarter_circles_stay_separate(self):
        self.use("circle")
        self.click((0, 0))
        self.canvas.submit_text("1")
        self.model.set_selection(self.model.doc.curves)
        self.use("merge")
        QTest.keyClick(self.canvas, Qt.Key_Return)
        self.assertEqual(len(self.curves("arc")), 4)     # 180 degrees is too much for one Eilmer Arc

    def test_spline_pieces_rejoin(self):
        s = SplineSeg(None, points=[(0, 0), (1, 1), (2, 0), (3, 1)])
        self.model.add_curves([geom.subcurve(s, 0.0, 0.4), geom.subcurve(s, 0.4, 1.0)], "pieces")
        self.model.set_selection(self.model.doc.curves)
        survivors, joins = self.model.merge(list(self.model.doc.curves))
        self.assertEqual(joins, 1)
        (whole,) = self.curves("spline")
        self.assertFalse(whole.trimmed())


class TestMirror(SketchTest):
    def setUp(self):
        super().setUp()
        self.use("triangle")
        self.click((1, 0.2), (2, 0.2), (1.5, 1))
        self.triangle = set(self.model.doc.curves)

    def endpoints(self, ids):
        return {tuple(np.round(p, 9)) for i in ids for p in
                (self.model.doc.curves[i].p0, self.model.doc.curves[i].p1)}

    def test_mirror_about_a_reference_line(self):
        self.model.reference_mode = True
        self.use("line")
        self.click((0, -1), (0, 2))
        self.model.reference_mode = False
        self.use("mirror")
        self.click((1.5, 0.2), (1.25, 0.6), (1.75, 0.6))      # the three edges
        QTest.keyClick(self.canvas, Qt.Key_Return)
        self.click((0, 1.5))                                   # axis
        QTest.keyClick(self.canvas, Qt.Key_Return)
        new = set(self.model.doc.curves) - self.triangle - {c.id for c in self.curves() if c.kind == "reference"}
        self.assertEqual(len(new), 3)
        self.assertEqual(self.endpoints(new), {(-1.0, 0.2), (-2.0, 0.2), (-1.5, 1.0)})
        self.assertEqual(self.model.selection, new)
        self.model.undo_stack.undo()
        self.assertEqual(len(self.curves()), 4)

    def test_mirror_about_x_axis_with_points_and_arcs(self):
        pid = self.model.add_point((1, 1))
        from eilmer_mesher.model.entities import ArcSeg
        self.model.add_curves([ArcSeg(None, p0=(3, 1), p1=(2, 2), centre=(2, 1))], "arc")
        self.model.set_selection(self.model.entity_ids())
        self.use("mirror")
        QTest.keyClick(self.canvas, Qt.Key_Return)
        self.click((-2.5, 0))                                  # on the x axis, away from geometry
        self.assertIn("x axis", self.tools["mirror"].prompt())
        QTest.keyClick(self.canvas, Qt.Key_Return)
        self.assertIn((1.0, -1.0), self.model.doc.points.values())
        arcs = self.curves("arc")
        mirrored = [a for a in arcs if a.p0 == (3.0, -1.0)]
        self.assertEqual(len(mirrored), 1)
        self.assertEqual((mirrored[0].p1, mirrored[0].centre), ((2.0, -2.0), (2.0, -1.0)))
        np.testing.assert_allclose(geom.sample(mirrored[0])[:, 1], -geom.sample(arcs[0])[:, 1], atol=1e-12)

    def test_curve_on_the_axis_is_skipped_and_esc_goes_back(self):
        self.use("line")
        self.click((-3, 0), (-2, 0))                           # lies on the x axis
        on_axis = list(self.model.doc.curves)[-1]
        self.model.set_selection({on_axis})
        self.use("mirror")
        QTest.keyClick(self.canvas, Qt.Key_Return)
        QTest.keyClick(self.canvas, Qt.Key_Escape)             # back to choosing objects
        self.assertEqual(self.tools["mirror"].step, 1)
        QTest.keyClick(self.canvas, Qt.Key_Return)
        self.click((3.5, 0))
        QTest.keyClick(self.canvas, Qt.Key_Return)
        self.assertEqual(len(self.curves()), 4)                # nothing new


class TestWindowSettings(unittest.TestCase):
    def tearDown(self):
        settings.app_settings().clear()

    def make(self):
        win = MainWindow()
        self.addCleanup(lambda: (win.deleteLater(), QApplication.sendPostedEvents(None, QEvent.DeferredDelete)))
        return win

    def test_space_fits_view_by_default(self):
        win = self.make()
        self.assertEqual(win.act_fit.shortcut(), QKeySequence("Space"))

    def test_shortcuts_persist(self):
        settings.set_shortcuts({"view.fit": "F", "tool.line": "Ctrl+L", "hold.horizontal": "J"})
        win = self.make()
        self.assertEqual(win.act_fit.shortcut(), QKeySequence("F"))
        self.assertEqual(win.tool_actions["line"].shortcut(), QKeySequence("Ctrl+L"))
        self.assertEqual(win.canvas.hold_keys["horizontal"], Qt.Key_J)

    def test_units_persist_and_display(self):
        win = self.make()
        win.set_units("ft")
        again = self.make()
        self.assertEqual(again.canvas.units.name, "ft")
        self.assertIn("in ft", again.input_bar.edit.placeholderText())

    def test_regions_view_paints(self):
        win = self.make()
        win.model.add_curves([LineSeg(None, p0=a, p1=b) for a, b in
                              [((0, 0), (1, 0)), ((1, 0), (1, 1)), ((1, 1), (0, 1)), ((0, 1), (0, 0))]], "box")
        win.act_regions.trigger()
        self.assertTrue(win.canvas.show_regions)
        win.canvas.grab()
        self.assertEqual(len(win.canvas.topology().regions), 1)


class TestLauncher(unittest.TestCase):
    def test_python3_mesher_runs_main(self):
        with mock.patch("eilmer_mesher.__main__.main", return_value=0) as main:
            with self.assertRaises(SystemExit) as stop:
                runpy.run_path(os.path.join(ROOT, "mesher"), run_name="__main__")
        main.assert_called_once()
        self.assertEqual(stop.exception.code, 0)


if __name__ == "__main__":
    unittest.main()
