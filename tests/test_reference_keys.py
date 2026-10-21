"""X converts the selection solid <-> dashed, Ctrl+X toggles draw-reference mode; both rebindable."""
import os
import unittest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QEvent, Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from test_blocking import box
from eilmer_mesher import settings
from eilmer_mesher.app import MainWindow
from eilmer_mesher.model.entities import GEOMETRY, REFERENCE

app = QApplication.instance() or QApplication([])


class TestReferenceKeys(unittest.TestCase):
    def setUp(self):
        self.win = MainWindow()
        self.win.mesher.enabled = False
        self.m = self.win.model
        self.ids = self.m.add_curves(box(0, 0, 0.2, 0.05), "box")
        self.win.show()
        self.win.canvas.setFocus()
        app.processEvents()

    def tearDown(self):
        self.win.hide()
        self.win.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)

    def kinds(self):
        return [self.m.doc.curves[i].kind for i in self.ids]

    def test_defaults(self):
        self.assertEqual(settings.DEFAULT_SHORTCUTS["edit.toggle_reference"][1], "Shift+X")
        self.assertEqual(settings.DEFAULT_SHORTCUTS["sketch.reference_mode"][1], "Ctrl+Shift+X")

    def test_x_toggles_selection_in_one_undo_step(self):
        self.m.doc.curves[self.ids[0]].bc_tag = "wall"
        self.m.set_selection(set(self.ids[:2]))
        n = self.m.undo_stack.count()
        QTest.keyClick(self.win.canvas, Qt.Key_X, Qt.ShiftModifier)
        self.assertEqual(self.kinds(), [REFERENCE, REFERENCE, GEOMETRY, GEOMETRY])
        self.assertEqual(self.m.doc.curves[self.ids[0]].bc_tag, "")
        self.assertEqual(self.m.undo_stack.count(), n + 1)
        QTest.keyClick(self.win.canvas, Qt.Key_X, Qt.ShiftModifier)
        self.assertEqual(self.kinds(), [GEOMETRY] * 4)
        self.m.undo_stack.undo()
        self.assertEqual(self.kinds(), [REFERENCE, REFERENCE, GEOMETRY, GEOMETRY])

    def test_x_with_nothing_selected_does_nothing(self):
        self.m.set_selection(set())
        n = self.m.undo_stack.count()
        QTest.keyClick(self.win.canvas, Qt.Key_X, Qt.ShiftModifier)
        self.assertEqual(self.m.undo_stack.count(), n)
        self.assertEqual(self.kinds(), [GEOMETRY] * 4)
        self.assertFalse(self.m.reference_mode)

    def test_ctrl_x_toggles_reference_mode(self):
        act = self.win.act_ref_mode
        self.assertFalse(act.isChecked())
        QTest.keyClick(self.win.canvas, Qt.Key_X, Qt.ControlModifier | Qt.ShiftModifier)
        self.assertTrue(act.isChecked())
        self.assertTrue(self.m.reference_mode)
        self.assertEqual(self.kinds(), [GEOMETRY] * 4)
        QTest.keyClick(self.win.canvas, Qt.Key_X, Qt.ControlModifier | Qt.ShiftModifier)
        self.assertFalse(self.m.reference_mode)

    def test_typing_x_in_input_box_types(self):
        self.m.set_selection(set(self.ids[:2]))
        self.win.set_tool("line")
        edit = self.win.input_bar.edit if hasattr(self.win, "input_bar") else None
        if edit is None:
            from PySide6.QtWidgets import QLineEdit
            edit = self.win.findChild(QLineEdit)
        edit.setFocus()
        app.processEvents()
        QTest.keyClick(edit, Qt.Key_X)
        self.assertEqual(edit.text(), "x")
        self.assertEqual(self.kinds(), [GEOMETRY] * 4)
        self.assertFalse(self.m.reference_mode)

    def test_no_tool_claims_x(self):
        for name, tool in self.win.tools.items():
            self.assertFalse(tool.claims_key(Qt.Key_X), name)


if __name__ == "__main__":
    unittest.main()
