"""Hide / show blocks: a view-only state on the canvas."""
import os
import unittest
from types import SimpleNamespace
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication, QMenu

import fixtures  # noqa: F401
from test_blocking import box
from test_sketch_gui import SketchTest, scr
from eilmer_mesher.app import MainWindow
from eilmer_mesher.model.document import Document

app = QApplication.instance() or QApplication([])

LEFT, RIGHT = (-2, 0), (2, 0)


class HideTest(SketchTest):
    def setUp(self):
        super().setUp()
        self.model.add_curves(box(-3, -1, -1, 1) + box(1, -1, 3, 1), "boxes")
        self.use("block")
        self.click(LEFT)
        self.click(RIGHT)
        self.ids = [b.id for b in self.model.doc.blocks]
        self.assertEqual(len(self.ids), 2)
        self.left = self.model.block_at(LEFT)
        self.right = self.model.block_at(RIGHT)
        grid = np.zeros((3, 3, 3))
        self.canvas.mesh = SimpleNamespace(grids=[(self.left, grid), (self.right, grid)], source="preview")

    def menu_at(self, point):
        shown = []
        with mock.patch.object(QMenu, "popup", lambda m, *a: shown.append(m)):
            self.click(point, button=Qt.RightButton)
        return shown[-1] if shown else None

    def trigger(self, point, text):
        menu = self.menu_at(point)
        acts = {a.text(): a for a in menu.actions() if a.text()}
        acts[text].trigger()

    def names(self, point):
        menu = self.menu_at(point)
        return [a.text() for a in menu.actions() if a.text()] if menu else []

    def drawn(self):
        return {b for b, _ in self.canvas.visible_grids()}


class TestHideBlocks(HideTest):
    def test_hide_and_show_one(self):
        self.assertIn("Hide block", self.names(LEFT))
        self.trigger(LEFT, "Hide block")
        self.assertEqual(self.canvas.hidden_blocks, {self.left})
        self.assertEqual(self.drawn(), {self.right})
        self.assertEqual(len(self.model.doc.blocks), 2)
        self.assertFalse(self.model.undo_stack.canUndo() and self.model.undo_stack.undoText() == "Hide")
        names = self.names(LEFT)
        self.assertIn("Show block", names)
        self.assertNotIn("Hide block", names)
        self.trigger(LEFT, "Show block")
        self.assertEqual(self.canvas.hidden_blocks, set())
        self.assertEqual(self.drawn(), {self.left, self.right})

    def test_hide_all_and_show_all(self):
        self.trigger(LEFT, "Hide all blocks")
        self.assertEqual(self.canvas.hidden_ids(), {self.left, self.right})
        self.assertEqual(self.drawn(), set())
        self.assertIn("Show all blocks", self.names(RIGHT))
        self.trigger(RIGHT, "Show all blocks")
        self.assertEqual(self.canvas.hidden_blocks, set())

    def test_show_all_offered_on_visible_block(self):
        self.trigger(LEFT, "Hide block")
        names = self.names(RIGHT)
        self.assertIn("Hide block", names)
        self.assertIn("Show all blocks", names)

    def test_select_tool_menu(self):
        self.use("select")
        self.assertIn("Hide block", self.names(LEFT))
        self.trigger(LEFT, "Hide block")
        self.assertEqual(self.canvas.hidden_blocks, {self.left})
        self.assertIn("Show block", self.names(LEFT))
        self.trigger(LEFT, "Show all blocks")
        self.assertEqual(self.canvas.hidden_blocks, set())

    def test_labels_and_fills_skip_hidden(self):
        self.canvas.set_blocks_hidden({self.left}, True)
        self.canvas.show_quality = False
        self.canvas.grab()
        self.canvas.set_blocks_hidden({self.left, self.right}, True)
        self.canvas.grab()
        self.canvas.set_blocks_hidden({"gone"}, True)       # stale ids are ignored
        self.assertEqual(self.canvas.hidden_ids(), {self.left, self.right})

    def test_mesh_path_rebuilt_when_hidden_changes(self):
        self.canvas.grab()
        first = self.canvas._mesh_path
        self.canvas.set_blocks_hidden({self.left}, True)
        self.canvas.grab()
        self.assertIsNot(self.canvas._mesh_path[1], first[1])
        self.assertEqual(self.canvas._mesh_path[0][1], {self.left})

    def test_grab_with_quality(self):
        self.canvas.set_blocks_hidden({self.left}, True)
        self.canvas.grab()

    def test_h_key(self):
        self.click(LEFT)                                    # selects the block
        self.assertEqual(self.tools["block"].selected, self.left)
        QTest.keyClick(self.canvas, Qt.Key_H)
        self.assertEqual(self.canvas.hidden_blocks, {self.left})
        QTest.keyClick(self.canvas, Qt.Key_H)
        self.assertEqual(self.canvas.hidden_blocks, set())
        self.assertEqual(self.canvas.held, set())

    def test_h_key_cursor_block(self):
        self.tools["block"].selected = None
        QTest.mouseMove(self.canvas, scr(RIGHT))
        QTest.keyClick(self.canvas, Qt.Key_H)
        self.assertEqual(self.canvas.hidden_blocks, {self.right})

    def test_shift_h_toggles_all(self):
        QTest.keyClick(self.canvas, Qt.Key_H, Qt.ShiftModifier)
        self.assertEqual(self.canvas.hidden_ids(), {self.left, self.right})
        QTest.keyClick(self.canvas, Qt.Key_H, Qt.ShiftModifier)
        self.assertEqual(self.canvas.hidden_blocks, set())
        self.canvas.set_blocks_hidden({self.left}, True)    # some visible: hide all
        QTest.keyClick(self.canvas, Qt.Key_H, Qt.ShiftModifier)
        self.assertEqual(self.canvas.hidden_ids(), {self.left, self.right})

    def test_h_is_hold_key_in_line_tool(self):
        self.use("line")
        QTest.keyPress(self.canvas, Qt.Key_H)
        self.assertIn(Qt.Key_H, self.canvas.held)
        QTest.keyRelease(self.canvas, Qt.Key_H)
        self.assertEqual(self.canvas.hidden_blocks, set())


class TestWindow(unittest.TestCase):
    def test_new_document_clears_hidden_and_view_menu(self):
        win = MainWindow()
        try:
            win.canvas.hidden_blocks.add("b1")
            self.assertTrue(win.act_show_all_blocks.text())
            win.act_show_all_blocks.trigger()
            self.assertEqual(win.canvas.hidden_blocks, set())
            win.canvas.hidden_blocks.add("b1")
            with mock.patch.object(win, "_confirm_discard", return_value=True):
                win.new_document()
            self.assertEqual(win.canvas.hidden_blocks, set())
            win.canvas.hidden_blocks.add("b1")
            path = os.path.join(fixtures.__dict__.get("TMP", "/tmp"), "hide_test.emesh.json")
            Document().save(path)
            try:
                win.open_path(path)
            finally:
                os.remove(path)
            self.assertEqual(win.canvas.hidden_blocks, set())
        finally:
            win.hide()
            win.deleteLater()
            QApplication.sendPostedEvents(None, 0)


if __name__ == "__main__":
    unittest.main()
