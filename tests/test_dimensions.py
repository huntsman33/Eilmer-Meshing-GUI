"""
Two-line dimensions: the distance between parallel lines and the angle between two
lines, placed with the Dimension tool and driving the second line.
"""
import math
import os
import tempfile
import unittest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from test_sketch_gui import SketchTest, scr
from eilmer_mesher.model import dimensions
from eilmer_mesher.model.document import Dimension, Document
from eilmer_mesher.model.entities import LineSeg
from eilmer_mesher.sketch.edit_dialogs import dialog_for
from eilmer_mesher.units import Units

app = QApplication.instance() or QApplication([])

DEG = "\N{DEGREE SIGN}"


class TwoLineTest(SketchTest):
    def lines(self, *ends):
        """Add lines [(p0, p1), ...] in one step; returns their ids."""
        return self.model.add_curves([LineSeg("", p0=p0, p1=p1) for p0, p1 in ends], "Lines")

    def rectangle(self):
        """Bottom, right, top, left sides of the 2 x 1 rectangle at the origin, joined end to end."""
        return self.lines(((0.0, 0.0), (2.0, 0.0)), ((2.0, 0.0), (2.0, 1.0)),
                          ((2.0, 1.0), (0.0, 1.0)), ((0.0, 1.0), (0.0, 0.0)))

    def triangle(self):
        """Sides O->P, P->Q and Q->O of the triangle O(0,0), P(2,0), Q(0,2)."""
        return self.lines(((0.0, 0.0), (2.0, 0.0)), ((2.0, 0.0), (0.0, 2.0)), ((0.0, 2.0), (0.0, 0.0)))

    def curve(self, cid):
        return self.model.doc.curves[cid]

    def text(self, did):
        return self.canvas.dimension_text(self.model.doc.dimensions[did])

    def label_centre(self, did):
        return self.canvas.dimension_layout(self.model.doc.dimensions[did])["rect"].center().toPoint()


class TestDistance(TwoLineTest):
    def test_model_distance_drives_second_line_and_connected_ends(self):
        bottom, right, top, left = self.rectangle()
        did = self.model.add_dimension(bottom, 1.0, top)
        dim = self.model.doc.dimensions[did]
        self.assertEqual((dim.kind, dim.curve, dim.curve2, dim.sector), ("distance", bottom, top, None))
        self.assertAlmostEqual(self.model.dimension_value(did), 1.0)
        steps = self.model.undo_stack.count()
        self.model.set_dimension_value(did, 1.5)
        self.assertEqual(self.model.undo_stack.count(), steps + 1)          # one undo step
        np.testing.assert_allclose(self.curve(top).p0, (2.0, 1.5))
        np.testing.assert_allclose(self.curve(top).p1, (0.0, 1.5))
        np.testing.assert_allclose(self.curve(right).p1, (2.0, 1.5))         # connected ends follow
        np.testing.assert_allclose(self.curve(left).p0, (0.0, 1.5))
        self.assertEqual(self.curve(bottom).p0, (0.0, 0.0))                  # the first line stays
        self.assertAlmostEqual(self.model.dimension_value(did), 1.5)
        self.model.undo_stack.undo()
        self.assertEqual(self.curve(top).p0, (2.0, 1.0))
        self.assertEqual(self.curve(right).p1, (2.0, 1.0))
        self.assertAlmostEqual(self.model.dimension_value(did), 1.0)

    def test_second_line_stays_on_its_side(self):
        bottom, _, top, _ = self.rectangle()
        did = self.model.add_dimension(top, 0.5, bottom)                     # top is fixed now
        self.model.set_dimension_value(did, 0.25)
        np.testing.assert_allclose(self.curve(bottom).p0, (0.0, 0.75))
        np.testing.assert_allclose(self.curve(bottom).p1, (2.0, 0.75))

    def test_invalid_distance_refused(self):
        bottom, _, top, _ = self.rectangle()
        did = self.model.add_dimension(bottom, 1.0, top)
        steps = self.model.undo_stack.count()
        for bad in (0.0, -1.0):
            with self.assertRaisesRegex(ValueError, "greater than zero"):
                self.model.set_dimension_value(did, bad)
        self.assertEqual(self.model.undo_stack.count(), steps)
        self.assertEqual(self.curve(top).p0, (2.0, 1.0))

    def test_gui_clicks_place_distance(self):
        self.use("rectangle")
        self.click((0, 0), (2, 1))
        by_y = {round(c.p0[1], 6): c.id for c in self.curves() if c.p0[1] == c.p1[1]}
        bottom, top = by_y[0.0], by_y[1.0]
        self.use("dimension")
        self.click((1, 0), (1, 1))                       # a line, then a parallel line
        self.assertEqual(self.model.doc.dimensions, {})  # still placing
        QTest.mouseMove(self.canvas, scr((1.5, 0.5)))
        self.click((1.5, 0.5))
        (did,) = self.model.doc.dimensions
        dim = self.model.doc.dimensions[did]
        self.assertEqual((dim.kind, dim.curve, dim.curve2), ("distance", bottom, top))
        self.assertEqual(self.text(did), "1 m")
        self.canvas.units = Units("mm")
        self.assertEqual(self.text(did), "1000 mm")

    def test_typed_distance_while_placing(self):
        self.use("rectangle")
        self.click((0, 0), (2, 1))
        self.use("dimension")
        self.click((1, 0), (1, 1))
        QTest.mouseMove(self.canvas, scr((1.0, 0.5)))
        self.assertIsNotNone(self.canvas.submit_text("-1"))       # refused
        self.assertIsNone(self.canvas.submit_text("2"))
        (did,) = self.model.doc.dimensions
        self.assertAlmostEqual(self.model.dimension_value(did), 2.0)
        ys = sorted({round(p[1], 9) for c in self.curves() for p in (c.p0, c.p1)})
        self.assertEqual(ys, [0.0, 2.0])                          # the rectangle stays closed

    def test_line_then_empty_space_is_still_length(self):
        bottom, _, _, _ = self.rectangle()
        self.use("dimension")
        self.click((1, 0), (1, -0.5))
        (dim,) = self.model.doc.dimensions.values()
        self.assertEqual((dim.kind, dim.curve, dim.curve2), ("length", bottom, None))

    def test_dialog_edits_distance_in_display_units(self):
        bottom, right, top, _ = self.rectangle()
        did = self.model.add_dimension(bottom, 1.0, top)
        dlg, apply = dialog_for(self.model, Units("mm"), did)
        self.assertAlmostEqual(dlg.values()[0], 1000.0)
        apply([1500.0])
        np.testing.assert_allclose(self.curve(right).p1, (2.0, 1.5))


class TestAngle(TwoLineTest):
    def test_corner_angle_and_supplement(self):
        op, pq, qo = self.triangle()
        right = self.model.add_dimension(op, 0.5, qo)
        self.assertEqual(self.model.doc.dimensions[right].kind, "angle")
        self.assertAlmostEqual(self.model.dimension_value(right), 90.0)
        a, b = self.curve(op), self.curve(pq)
        np.testing.assert_allclose(dimensions.vertex(a, b), (2.0, 0.0))
        r, inside = dimensions.angle_placement(a, b, (1.7, 0.1))
        _, outside = dimensions.angle_placement(a, b, (2.3, 0.1))
        self.assertAlmostEqual(dimensions.angle_deg(a, b, inside), 45.0)
        self.assertAlmostEqual(dimensions.angle_deg(a, b, outside), 135.0)
        did = self.model.add_dimension(op, r, pq)                  # default: inside the corner
        self.assertAlmostEqual(self.model.dimension_value(did), 45.0)

    def test_edit_angle_turns_second_line_about_the_corner(self):
        op, pq, qo = self.triangle()
        did = self.model.add_dimension(op, 0.5, pq)
        self.model.set_dimension_value(did, 60.0)
        L = 2.0 * math.sqrt(2.0)
        q = (2.0 + L * math.cos(math.radians(120)), L * math.sin(math.radians(120)))
        np.testing.assert_allclose(self.curve(pq).p0, (2.0, 0.0), atol=1e-12)   # turns about P
        np.testing.assert_allclose(self.curve(pq).p1, q, atol=1e-12)
        np.testing.assert_allclose(self.curve(qo).p0, q, atol=1e-12)            # Q's other side follows
        self.assertEqual(self.curve(op).p1, (2.0, 0.0))
        self.assertAlmostEqual(self.model.dimension_value(did), 60.0)
        self.model.undo_stack.undo()
        np.testing.assert_allclose(self.curve(pq).p1, (0.0, 2.0))
        np.testing.assert_allclose(self.curve(qo).p0, (0.0, 2.0))

    def test_supplement_edit_keeps_its_sector(self):
        op, pq, _ = self.triangle()
        r, outside = dimensions.angle_placement(self.curve(op), self.curve(pq), (2.3, 0.1))
        did = self.model.add_dimension(op, r, pq, outside)
        self.assertAlmostEqual(self.model.dimension_value(did), 135.0)
        self.model.set_dimension_value(did, 120.0)                 # the inside angle becomes 60
        self.assertAlmostEqual(self.model.dimension_value(did), 120.0)
        a, b = self.curve(op), self.curve(pq)
        self.assertAlmostEqual(dimensions.angle_deg(a, b, dimensions.default_sector(a, b)), 60.0)

    def test_angle_about_intersection_of_separate_lines(self):
        a, b = self.lines(((0.0, 0.0), (1.0, 0.0)), ((2.0, 1.0), (3.0, 2.0)))   # meet at (1, 0) extended
        did = self.model.add_dimension(a, 1.0, b, [1, 1])
        self.assertAlmostEqual(self.model.dimension_value(did), 45.0)
        self.model.set_dimension_value(did, 90.0)
        np.testing.assert_allclose(self.curve(b).p0, (1.0, math.sqrt(2.0)), atol=1e-12)
        np.testing.assert_allclose(self.curve(b).p1, (1.0, 2.0 * math.sqrt(2.0)), atol=1e-12)

    def test_invalid_angle_refused(self):
        op, pq, _ = self.triangle()
        did = self.model.add_dimension(op, 0.5, pq)
        for bad in (0.0, 180.0, 200.0, -10.0):
            with self.assertRaisesRegex(ValueError, "between 0 and 180"):
                self.model.set_dimension_value(did, bad)
        self.assertEqual(self.curve(pq).p1, (0.0, 2.0))

    def test_gui_clicks_place_angle_in_chosen_sector(self):
        op, pq, qo = self.triangle()
        self.use("dimension")
        self.click((1, 0), (1, 1))                       # base, then the sloping side
        QTest.mouseMove(self.canvas, scr((1.6, 0.15)))
        self.click((1.6, 0.15))
        (did,) = self.model.doc.dimensions
        dim = self.model.doc.dimensions[did]
        self.assertEqual((dim.kind, dim.curve, dim.curve2), ("angle", op, pq))
        self.assertEqual(self.text(did), f"45{DEG}")
        # Dragging the label into the next sector shows the supplement instead.
        self.use("select")
        start = self.label_centre(did)
        QTest.mousePress(self.canvas, Qt.LeftButton, Qt.NoModifier, start)
        QTest.mouseMove(self.canvas, scr((2.2, 0.2)))
        QTest.mouseMove(self.canvas, scr((2.4, 0.15)))
        QTest.mouseRelease(self.canvas, Qt.LeftButton, Qt.NoModifier, scr((2.4, 0.15)))
        self.assertEqual(self.text(did), f"135{DEG}")
        self.model.undo_stack.undo()
        self.assertEqual(self.text(did), f"45{DEG}")

    def test_typed_angle_while_placing(self):
        op, pq, qo = self.triangle()
        self.use("dimension")
        self.click((1, 0), (1, 1))
        QTest.mouseMove(self.canvas, scr((1.6, 0.15)))
        self.assertIsNotNone(self.canvas.submit_text("190"))     # refused
        self.assertIsNone(self.canvas.submit_text("30"))
        (did,) = self.model.doc.dimensions
        self.assertAlmostEqual(self.model.dimension_value(did), 30.0)
        self.assertEqual(self.curve(pq).p1, self.curve(qo).p0)   # still closed
        self.assertAlmostEqual(math.degrees(math.atan2(self.curve(pq).p1[1], 2.0 - self.curve(pq).p1[0])),
                               30.0)

    def test_dialog_edits_angle(self):
        op, pq, qo = self.triangle()
        did = self.model.add_dimension(op, 0.5, qo)
        dlg, apply = dialog_for(self.model, Units("mm"), did)
        self.assertAlmostEqual(dlg.values()[0], 90.0)
        apply([45.0])
        self.assertAlmostEqual(self.model.dimension_value(did), 45.0)
        np.testing.assert_allclose(self.curve(qo).p1, (0.0, 0.0), atol=1e-12)   # turned about O


class TestPersistenceAndPainting(TwoLineTest):
    def test_round_trip_and_old_dimension_dicts(self):
        bottom, _, top, left = self.rectangle()
        d1 = self.model.add_dimension(bottom, 1.0, top)
        d2 = self.model.add_dimension(bottom, 0.3, left, [1, -1])
        d3 = self.model.add_dimension(bottom, 0.5)
        again = Document.from_dict(self.model.doc.to_dict())
        for did in (d1, d2, d3):
            self.assertEqual(again.dimensions[did], self.model.doc.dimensions[did])
        self.assertNotIn("curve2", self.model.doc.dimensions[d3].to_dict())
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "dims.emesh.json")
            self.model.doc.save(path)
            loaded = Document.load(path)
        self.assertEqual(loaded.dimensions[d2].sector, [1, -1])
        self.assertEqual(loaded.dimensions[d1].curve2, top)
        old = Dimension.from_dict({"id": "dim9", "kind": "length", "curve": bottom, "offset": 0.5})
        self.assertEqual((old.curve2, old.sector, old.curve_ids()), (None, None, [bottom]))
        self.assertEqual(old.to_dict(), {"id": "dim9", "kind": "length", "curve": bottom, "offset": 0.5})

    def test_deleting_either_line_removes_dimension(self):
        for which in (0, 1):
            self.model.reset(Document())
            bottom, _, top, _ = self.rectangle()
            did = self.model.add_dimension(bottom, 1.0, top)
            self.model.set_selection({(bottom, top)[which]})
            self.model.delete_selection()
            self.assertNotIn(did, self.model.doc.dimensions)
            self.model.undo_stack.undo()
            self.assertIn(did, self.model.doc.dimensions)

    def test_painting(self):
        bottom, right, top, left = self.rectangle()
        self.model.add_dimension(bottom, 2.5, top)                 # beyond the lines: extension lines
        self.model.add_dimension(bottom, 0.4, left)
        self.model.add_dimension(bottom, 0.4, right, [-1, 1])
        for dim in self.model.doc.dimensions.values():
            layout = self.canvas.dimension_layout(dim)
            self.assertIsNotNone(layout)
            self.assertEqual(len(layout["arrows"]), 2)
        self.use("dimension")
        self.click((1, 0), (1, 1))
        QTest.mouseMove(self.canvas, scr((-0.5, 0.5)))
        image = self.canvas.grab().toImage()
        self.assertFalse(image.isNull())
        self.assertEqual((image.width(), image.height()), (800, 600))
        # The preview dimension line crosses the gap between the lines at x = -0.5.
        c = image.pixelColor(scr((-0.5, 0.25)))
        self.assertGreater(c.blue() - c.red(), 60)                 # drawn in the preview blue


if __name__ == "__main__":
    unittest.main()
