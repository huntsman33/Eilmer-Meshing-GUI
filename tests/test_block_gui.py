"""Block tool and main-window wiring for blocking and the live mesh."""
import os
import tempfile
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QEvent, Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from test_blocking import box
from test_sketch_gui import SketchTest, scr
from eilmer_mesher.app import MainWindow
from eilmer_mesher.model.entities import SPLIT, LineSeg

app = QApplication.instance() or QApplication([])


class TestBlockTool(SketchTest):
    def setUp(self):
        super().setUp()
        self.model.add_curves(box(-2, -1, 2, 1), "box")
        self.messages = []
        self.canvas.message.connect(self.messages.append)
        self.use("block")

    def test_click_region_makes_block(self):
        self.click((0, 0))
        self.assertEqual(len(self.model.doc.blocks), 1)
        self.assertEqual(self.messages[-1], "Block made.")
        self.click((0, 0))                                  # now selects it instead
        self.assertEqual(len(self.model.doc.blocks), 1)
        self.assertEqual(self.tools["block"].selected, self.model.doc.blocks[0].id)

    def test_double_click_edge_requests_cells(self):
        self.click((0, 0))
        asked = []
        self.canvas.edgeRequested.connect(lambda b, f: asked.append((b, f)))
        QTest.mouseMove(self.canvas, scr((0, -1)))
        QTest.mouseDClick(self.canvas, Qt.LeftButton, Qt.NoModifier, scr((0, -1)))
        self.assertEqual(asked, [(self.model.doc.blocks[0].id, "south")])

    def test_reference_line_needs_new_blocks(self):
        self.click((0, 0))
        self.model.add_curves([LineSeg(None, kind=SPLIT, p0=(0, -1), p1=(0, 1))], "split")
        self.assertTrue(self.canvas.resolution().errors)
        self.canvas.grab()                                  # broken blocks paint (in red)
        self.click((-1, 0))
        self.assertEqual(self.messages[-1], "Block made.")

    def test_set_corners(self):
        self.click((0, 0))
        bid = self.model.doc.blocks[0].id
        tool = self.tools["block"]
        tool._start_corners(bid)
        self.click((2, -1), (2, 1), (-2, 1), (-2, -1))     # same corners, chosen by hand
        self.assertIsNone(tool.corner_pick)
        self.assertEqual(self.messages[-1], "Corners set.")
        self.assertEqual(self.canvas.resolution().errors, {})

    def test_regions_with_three_sides_explain(self):
        self.model.add_curves([LineSeg(None, p0=(3, 0), p1=(5, 0)), LineSeg(None, p0=(5, 0), p1=(4, 1.5)),
                               LineSeg(None, p0=(4, 1.5), p1=(3, 0))], "tri")
        self.click((4, 0.5))
        self.assertIn("only 3 sides", self.messages[-1])


class TestWindowBlocking(unittest.TestCase):
    def setUp(self):
        self.win = MainWindow()
        self.win.mesher.enabled = False                       # no background Eilmer runs here

    def tearDown(self):
        self.win.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)

    def test_edit_edge_dialog_applies(self):
        m = self.win.model
        m.add_curves(box(0, 0, 2, 1), "box")
        bid = m.make_block((1, 0.5))
        spec = {"type": "RobertsFunction", "end0": True, "end1": False, "beta": 1.1}
        with mock.patch("eilmer_mesher.app.EdgeCellsDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.values.return_value = (24, spec, True)
            self.win.edit_edge(bid, "east")
        block = m.resolution().by_id()[bid]
        self.assertEqual(block.njv, 25)
        self.assertEqual(m.doc.blocks[0].cluster["west"], spec)

    def test_split_dialog_and_block_all(self):
        m = self.win.model
        m.add_curves(box(0, 0, 2, 1), "box")
        with mock.patch("eilmer_mesher.app.SplitDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.values.return_value = (2, 3)
            self.win.split_region((1, 0.5))
        self.assertEqual(len(m.doc.blocks), 6)
        m.add_curves(box(5, 0, 6, 1), "another")
        self.win.block_all()
        self.assertEqual(len(m.doc.blocks), 7)

    def test_mesh_overlay_paints_preview(self):
        m = self.win.model
        m.add_curves(box(0, 0, 2, 1), "box")
        m.make_block((1, 0.5))
        self.win.mesher.enabled = True
        self.win.mesher._preview()
        self.assertEqual(self.win.canvas.mesh.source, "preview")
        self.win.canvas.grab()
        self.win.act_show_mesh.trigger()
        self.assertFalse(self.win.canvas.show_mesh)

    def test_workdir_follows_project(self):
        path = os.path.join(tempfile.mkdtemp(), "wing.emesh.json")
        self.win._set_path(path)
        self.assertEqual(self.win.mesher.workdir, os.path.join(os.path.dirname(path), ".emesh", "wing"))
        self.win._set_path(None)
        self.assertIsNone(self.win.mesher.workdir)


if __name__ == "__main__":
    unittest.main()
