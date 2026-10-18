"""
Sketcher GUI tests: real mouse/key events on an offscreen canvas.

The canvas is 800x600 at 100 px per unit centred on the origin, so world
(x, y) is screen (400 + 100x, 300 - 100y).
"""
import math
import os
import shutil
import tempfile
import unittest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import QEvent, QPoint, Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from eilmer_mesher.model.document import Block, CurveRef
from eilmer_mesher.model.entities import REFERENCE
from eilmer_mesher.sketch.canvas import SketchCanvas
from eilmer_mesher.sketch.importers import build_curves, parse_point_file
from eilmer_mesher.sketch.model import SketchModel
from eilmer_mesher.sketch.tools import SKETCH_TOOLS

app = QApplication.instance() or QApplication([])


def scr(p):
    return QPoint(round(400 + 100 * p[0]), round(300 - 100 * p[1]))


class SketchTest(unittest.TestCase):
    def setUp(self):
        self.model = SketchModel()
        self.canvas = SketchCanvas(self.model)
        self.canvas.resize(800, 600)
        self.canvas.show()
        self.canvas.scale, self.canvas.cx, self.canvas.cy = 100.0, 0.0, 0.0
        self.tools = {cls.name: cls(self.canvas) for cls, _, _ in SKETCH_TOOLS}

    def tearDown(self):
        # Delete Qt objects now, not during interpreter shutdown after QApplication is gone.
        self.canvas.close()
        self.canvas.deleteLater()
        self.model.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)
        del self.canvas, self.model, self.tools

    def use(self, name):
        self.canvas.set_tool(self.tools[name])

    def click(self, *points, button=Qt.LeftButton, mods=Qt.NoModifier):
        for p in points:
            QTest.mouseMove(self.canvas, scr(p))
            QTest.mouseClick(self.canvas, button, mods, scr(p))

    def drag(self, a, b, steps=5, mods=Qt.NoModifier):
        QTest.mouseMove(self.canvas, scr(a))
        QTest.mousePress(self.canvas, Qt.LeftButton, mods, scr(a))
        for k in range(1, steps + 1):
            t = k / steps
            QTest.mouseMove(self.canvas, scr((a[0] + t*(b[0]-a[0]), a[1] + t*(b[1]-a[1]))))
        QTest.mouseRelease(self.canvas, Qt.LeftButton, mods, scr(b))

    def curves(self, kind=None):
        cs = list(self.model.doc.curves.values())
        return [c for c in cs if kind is None or c.type == kind]


class TestDrawing(SketchTest):
    def test_line_and_undo_redo(self):
        self.use("line")
        self.click((0, 0), (1, 0.5))
        (line,) = self.curves()
        self.assertEqual((line.p0, line.p1), ((0.0, 0.0), (1.0, 0.5)))
        self.model.undo_stack.undo()
        self.assertEqual(self.curves(), [])
        self.model.undo_stack.redo()
        self.assertEqual(len(self.curves()), 1)

    def test_endpoint_snap_makes_exact_coincidence(self):
        self.use("line")
        self.click((0, 0), (1.234, 0.567))
        end = self.curves()[0].p1
        self.click((1.25, 0.58), (2, -1))      # within a few pixels of the endpoint
        second = self.curves()[1]
        self.assertEqual(second.p0, end)

    def test_rectangle_is_closed_loop(self):
        self.use("rectangle")
        self.click((-1, -0.5), (1, 0.5))
        lines = self.curves("line")
        self.assertEqual(len(lines), 4)
        for k in range(4):
            self.assertEqual(lines[k].p1, lines[(k + 1) % 4].p0)

    def test_triangle(self):
        self.use("triangle")
        self.click((0, 0), (2, 0), (1, 1.5))
        self.assertEqual(len(self.curves("line")), 3)

    def test_profile_lines_then_drag_arc(self):
        self.use("profile")
        self.click((-1, 0), (1, 0))
        self.drag((1, 0), (2, 1))                  # drag from the last point: tangent arc
        QTest.keyClick(self.canvas, Qt.Key_Escape)
        lines, arcs = self.curves("line"), self.curves("arc")
        self.assertEqual(len(lines), 1)
        self.assertEqual(len(arcs), 1)
        np.testing.assert_allclose(arcs[0].centre, (1, 1), atol=1e-12)
        self.assertEqual(arcs[0].p0, lines[0].p1)

    def test_profile_space_toggles_arc_and_closes_loop(self):
        self.use("profile")
        self.click((0, 0), (2, 0))
        QTest.keyClick(self.canvas, Qt.Key_Space)  # next segment is an arc
        self.click((2, 2))                          # semicircle -> two arc pieces
        self.click((0, 2), (0, 0))                  # back to start closes the profile
        self.assertFalse(self.tools["profile"].busy())
        self.assertEqual(len(self.curves("arc")), 2)
        self.assertEqual(len(self.curves("line")), 3)

    def test_arc_three_point(self):
        self.use("arc")
        self.click((1, 0), (-1, 0), (0, 1))
        arcs = self.curves("arc")
        self.assertEqual(len(arcs), 2)
        self.assertEqual((arcs[0].p0, arcs[-1].p1), ((1.0, 0.0), (-1.0, 0.0)))

    def test_circle_typed_radius(self):
        self.use("circle")
        self.click((0.5, 0.5))
        self.assertIsNone(self.canvas.submit_text("0.25"))
        arcs = self.curves("arc")
        self.assertEqual(len(arcs), 4)
        for a in arcs:
            self.assertAlmostEqual(math.dist(a.p0, (0.5, 0.5)), 0.25)

    def test_typed_relative_line(self):
        self.use("line")
        self.assertIsNone(self.canvas.submit_text("1, 1"))
        self.assertIsNone(self.canvas.submit_text("@2<0"))
        (line,) = self.curves()
        np.testing.assert_allclose(line.p1, (3, 1), atol=1e-12)
        self.assertIsNotNone(self.canvas.submit_text("nonsense"))

    def test_spline_and_bezier_finish(self):
        self.use("spline")
        self.click((0, 0), (1, 1), (2, 0))
        QTest.keyClick(self.canvas, Qt.Key_Return)
        self.use("bezier")
        self.click((0, -1), (1, -2), (2, -1))
        self.click((2, -1), button=Qt.RightButton)
        (s,) = self.curves("spline")
        (b,) = self.curves("bezier")
        self.assertEqual(len(s.points), 3)
        self.assertEqual(b.points, [(0.0, -1.0), (1.0, -2.0), (2.0, -1.0)])

    def test_escape_cancels(self):
        self.use("line")
        self.click((0, 0))
        QTest.keyClick(self.canvas, Qt.Key_Escape)
        self.click((1, 1))
        self.assertEqual(self.curves(), [])       # (1,1) started a new line instead

    def test_reference_mode(self):
        self.model.reference_mode = True
        self.use("line")
        self.click((0, 0), (1, 0))
        self.assertEqual(self.curves()[0].kind, REFERENCE)


class TestSelect(SketchTest):
    def setUp(self):
        super().setUp()
        self.use("rectangle")
        self.click((0, 0), (2, 1))
        self.use("select")
        self.south = next(c for c in self.curves() if c.p0 == (0.0, 0.0))

    def test_click_and_shift_select(self):
        self.click((1, 0))
        self.assertEqual(self.model.selection, {self.south.id})
        self.click((2, 0.5), mods=Qt.ShiftModifier)
        self.assertEqual(len(self.model.selection), 2)
        self.click((1, 3))                         # empty space clears
        self.assertEqual(self.model.selection, set())

    def test_drag_shared_corner_moves_both_curves(self):
        self.click((1, 0))                         # select south edge
        self.drag((0, 0), (-0.5, -0.5))            # drag its start handle
        corner = (-0.5, -0.5)
        starts = [c for c in self.curves() if c.p0 == corner]
        ends = [c for c in self.curves() if c.p1 == corner]
        self.assertEqual((len(starts), len(ends)), (1, 1))   # south start and west end follow
        self.model.undo_stack.undo()
        self.assertEqual(sum(c.p0 == (0.0, 0.0) or c.p1 == (0.0, 0.0) for c in self.curves()), 2)

    def test_move_selection(self):
        self.click((1, 0))
        self.drag((1, 0), (1, -1))
        moved = self.model.doc.curves[self.south.id]
        self.assertEqual((moved.p0, moved.p1), ((0.0, -1.0), (2.0, -1.0)))

    def test_box_window_vs_crossing(self):
        self.drag((-0.5, -0.5), (2.5, 0.5))        # left-to-right window: only the south edge
        self.assertEqual(self.model.selection, {self.south.id})
        self.drag((2.5, 0.5), (-0.5, -0.5))        # right-to-left crossing: all but the north edge
        self.assertEqual(len(self.model.selection), 3)
        self.drag((1.2, 1.3), (0.8, 0.7))          # crossing the middle of the north edge only
        (north,) = self.model.selection
        self.assertEqual(self.model.doc.curves[north].p0, (2.0, 1.0))

    def test_delete_and_toggle_reference(self):
        self.click((1, 0))
        self.model.toggle_reference()
        self.assertEqual(self.model.doc.curves[self.south.id].kind, REFERENCE)
        QTest.keyClick(self.canvas, Qt.Key_Delete)
        self.assertEqual(len(self.curves()), 3)
        self.model.undo_stack.undo()
        self.assertEqual(len(self.curves()), 4)


class TestSketchToMesh(SketchTest):
    """A rectangle drawn with the GUI becomes a block that Eilmer can mesh."""

    def test_drawn_rectangle_meshes(self):
        from eilmer_mesher.mesh.runner import find_lmr, preview_mesh
        try:
            find_lmr()
        except FileNotFoundError:
            self.skipTest("lmr not installed")
        self.use("rectangle")
        self.click((0, 0), (2, 1))
        by_start = {c.p0: c.id for c in self.curves()}
        sides = {"south": [CurveRef(by_start[(0.0, 0.0)])],
                 "east": [CurveRef(by_start[(2.0, 0.0)])],
                 "north": [CurveRef(by_start[(2.0, 1.0)], reversed=True)],
                 "west": [CurveRef(by_start[(0.0, 1.0)], reversed=True)]}
        self.model.doc.add_block(Block("blk0", sides, niv=9, njv=5))
        tmp = tempfile.mkdtemp(prefix="emesh-gui-")
        try:
            result = preview_mesh(self.model.doc, tmp)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)
        self.assertTrue(result.ok, result.output)
        np.testing.assert_allclose(result.grids[0][-1, -1, :2], (2, 1), atol=1e-12)

    def test_import_airfoil(self):
        from test_importers import selig_text
        ids = self.model.add_curves(build_curves(parse_point_file(selig_text())), "Import")
        self.assertEqual(len(ids), 3)
        self.canvas.zoom_to_fit()
        self.canvas.grab()                          # paints splines without error
        self.model.undo_stack.undo()
        self.assertEqual(self.curves(), [])


if __name__ == "__main__":
    unittest.main()
