"""Main window behaviour: quitting and platform selection."""
import os
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QEvent, Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication, QMessageBox

import fixtures  # noqa: F401
from eilmer_mesher import __main__ as entry
from eilmer_mesher.app import MainWindow
from eilmer_mesher.model.entities import LineSeg

app = QApplication.instance() or QApplication([])


class TestQuit(unittest.TestCase):
    def setUp(self):
        self.win = MainWindow()
        self.win.show()
        self.win.activateWindow()
        QTest.qWaitForWindowExposed(self.win)

    def tearDown(self):
        self.win.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)

    def press_ctrl_q(self):
        QTest.keyClick(self.win.canvas, Qt.Key_Q, Qt.ControlModifier)
        app.processEvents()

    def test_ctrl_q_closes_clean_document(self):
        self.assertEqual(self.win.act_quit.shortcut().toString(), "Ctrl+Q")
        with mock.patch.object(QMessageBox, "question") as ask:
            self.press_ctrl_q()
        ask.assert_not_called()
        self.assertFalse(self.win.isVisible())

    def test_unsaved_changes_prompt(self):
        self.win.model.add_curves([LineSeg(None, p0=(0, 0), p1=(1, 0))], "Line")
        with mock.patch.object(QMessageBox, "question", return_value=QMessageBox.Cancel) as ask:
            self.press_ctrl_q()
        ask.assert_called_once()
        self.assertTrue(self.win.isVisible())          # Cancel keeps the window open
        with mock.patch.object(QMessageBox, "question", return_value=QMessageBox.Discard):
            self.press_ctrl_q()
        self.assertFalse(self.win.isVisible())


class TestPlatformChoice(unittest.TestCase):
    def choose(self, env, wsl):
        with mock.patch.dict(os.environ, env, clear=True), \
                mock.patch.object(entry, "_running_on_wsl", return_value=wsl):
            entry._choose_platform()
            return os.environ.get("QT_QPA_PLATFORM")

    def test_wsl_with_display_uses_xcb(self):
        self.assertEqual(self.choose({"DISPLAY": ":0", "WAYLAND_DISPLAY": "wayland-0"}, True), "xcb")

    def test_user_setting_wins(self):
        self.assertEqual(self.choose({"DISPLAY": ":0", "QT_QPA_PLATFORM": "wayland"}, True), "wayland")

    def test_not_wsl_untouched(self):
        self.assertIsNone(self.choose({"DISPLAY": ":0"}, False))


if __name__ == "__main__":
    unittest.main()
