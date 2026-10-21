"""Background grid display toggle (View > Show grid, Shift+G)."""
import os
import unittest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QEvent
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from eilmer_mesher import settings
from eilmer_mesher.app import MainWindow

app = QApplication.instance() or QApplication([])


class TestGridToggle(unittest.TestCase):
    def setUp(self):
        self.win = MainWindow()
        self.win.mesher.enabled = False

    def tearDown(self):
        self.win.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)
        settings.set_show_grid(True)          # leave the setting as other tests expect it

    def test_toggle_changes_canvas(self):
        self.win.act_grid.trigger()           # checked -> unchecked
        self.assertFalse(self.win.act_grid.isChecked())
        self.assertFalse(self.win.canvas.show_grid)
        self.win.act_grid.trigger()
        self.assertTrue(self.win.canvas.show_grid)

    def test_setting_persists(self):
        self.win.act_grid.trigger()           # switch off
        self.assertFalse(settings.show_grid())
        self.win.deleteLater()
        self.win = MainWindow()
        self.win.mesher.enabled = False
        self.assertFalse(self.win.act_grid.isChecked())
        self.assertFalse(self.win.canvas.show_grid)

    def test_canvas_paints_in_both_states(self):
        self.win.canvas.show_grid = True
        self.win.canvas.grab()
        self.win.canvas.show_grid = False
        self.win.canvas.grab()


if __name__ == "__main__":
    unittest.main()
