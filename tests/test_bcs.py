"""Phase 5: boundary conditions, explicit block connections, and export to Eilmer."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QEvent
from PySide6.QtWidgets import QApplication, QMenu, QMessageBox

import fixtures  # noqa: F401
from test_blocking import ModelTest, box, have_lmr
from eilmer_mesher.app import MainWindow
from eilmer_mesher.lua.codegen import generate_bcs_lua, generate_geometry_lua
from eilmer_mesher.model.bcs import BC_CLASSES, check_tag, flow_state_names, sim_template
from eilmer_mesher.model.document import BlockError
from eilmer_mesher.model.entities import REFERENCE, LineSeg
from eilmer_mesher.sketch.bc_dialogs import BCDialog, add_bc_menu
from eilmer_mesher.sketch.canvas import SketchCanvas

app = QApplication.instance() or QApplication([])


class TestCatalogue(unittest.TestCase):
    def test_tags_must_be_lua_names(self):
        self.assertEqual(check_tag("inflow_2"), "inflow_2")
        for bad in ("2in", "in flow", "", "end", "out-flow"):
            with self.assertRaises(ValueError):
                check_tag(bad)

    def test_flow_state_names(self):
        self.assertEqual(flow_state_names("flowState=inflow"), ["inflow"])
        self.assertEqual(flow_state_names("p_outside=101.325e3, T_outside=300.0"), [])
        self.assertEqual(flow_state_names('fileName="x.dat"'), [])

    def test_catalogue_classes_exist_in_eilmer(self):
        lib = os.path.expanduser("~/gdtkinst/lib/bc.lua")
        if not os.path.exists(lib):
            self.skipTest("Eilmer lib not installed")
        text = open(lib).read()
        for cls, *_ in BC_CLASSES:
            self.assertRegex(text, rf"\b{cls}\s*=", cls)


class TestTagging(ModelTest):
    def setUp(self):
        super().setUp()
        self.south, self.east, self.north, self.west = self.add(box(0, 0, 2, 1))
        self.model.define_bc("inflow", "InFlowBC_Supersonic", "flowState=inflow")
        self.model.define_bc("outflow", "OutFlowBC_Simple")

    def test_set_rename_delete_and_undo(self):
        self.assertEqual(self.model.set_bc([self.west], "inflow"), 1)
        self.model.define_bc("inlet", "InFlowBC_Supersonic", "flowState=inflow", old_tag="inflow")
        self.assertEqual(self.model.doc.curves[self.west].bc_tag, "inlet")
        self.assertNotIn("inflow", self.model.doc.bcs)
        self.model.delete_bc("inlet")
        self.assertEqual(self.model.doc.curves[self.west].bc_tag, "")
        self.model.undo_stack.undo()
        self.assertEqual(self.model.doc.curves[self.west].bc_tag, "inlet")
        with self.assertRaises(ValueError):
            self.model.define_bc("outflow", "OutFlowBC_FixedP")       # tag already exists

    def test_reference_curves_are_never_tagged(self):
        ref = self.add([LineSeg(None, kind=REFERENCE, p0=(1, 0), p1=(1, 1))])[0]
        self.assertEqual(self.model.set_bc([ref], "inflow"), 0)

    def test_tags_reach_lua(self):
        self.model.set_bc([self.west], "inflow")
        self.model.set_bc([self.east], "outflow")
        self.model.make_block((1, 0.5))
        md = self.model.doc.with_blocks(self.res().blocks)
        self.assertIn('bcTags={east="outflow", west="inflow"}', generate_geometry_lua(md))
        bcs = generate_bcs_lua(md)
        self.assertIn("inflow = InFlowBC_Supersonic:new{flowState=inflow},", bcs)
        self.assertIn("outflow = OutFlowBC_Simple:new{},", bcs)
        self.assertIn("inflow = FlowState:new{p=100.0e3, T=300.0, velx=0.0}", sim_template(md, ["inflow"]))

    def test_mixed_tags_on_a_face_explained(self):
        # The south side made of two collinear curves with different tags, inside one block face.
        with self.model.edit("split south") as doc:
            s = doc.curves[self.south]
            doc.curves[self.south] = s.copy(p1=(1.0, 0.0))
            doc.add_curve(s.copy(id="south2", p0=(1.0, 0.0)))
        self.model.set_bc([self.south], "inflow")
        self.model.set_bc(["south2"], "outflow")
        self.model.make_block((1, 0.5))
        (bid, msg), = self.res().errors.items()
        self.assertIn("mixes BC tags", msg)


class TestConnections(ModelTest):
    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_solid_line_is_an_interior_wall(self):
        self.add(box(0, 0, 2, 1) + [LineSeg(None, p0=(1, 0), p1=(1, 1))])
        self.model.make_all_blocks()
        _, connections = self.eilmer()
        self.assertEqual(connections, 0)                   # the plate separates the blocks
        lua = generate_geometry_lua(self.model.doc.with_blocks(self.res().blocks))
        self.assertIn("(no connections)", lua)
        self.assertNotIn("identifyGridConnections()", lua)

    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_dashed_line_connects(self):
        self.add(box(0, 0, 2, 1) + [LineSeg(None, kind=REFERENCE, p0=(1, 0), p1=(1, 1))])
        self.model.make_all_blocks()
        _, connections = self.eilmer()
        self.assertEqual(connections, 1)


class TestBCGui(unittest.TestCase):
    def setUp(self):
        self.win = MainWindow()
        self.win.mesher.enabled = False
        self.m = self.win.model
        self.ids = self.m.add_curves(box(0, 0, 0.2, 0.05), "box")
        self.m.make_block((0.1, 0.02))

    def tearDown(self):
        self.win.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)

    def test_menu_offers_tags_and_new(self):
        self.m.define_bc("inflow", "InFlowBC_Supersonic", "flowState=inflow")
        menu = QMenu()
        sub = add_bc_menu(menu, self.win.canvas, [self.ids[3]])
        texts = [a.text() for a in sub.actions()]
        self.assertIn("inflow  (InFlowBC_Supersonic)", texts)
        next(a for a in sub.actions() if a.text().startswith("inflow")).trigger()
        self.assertEqual(self.m.doc.curves[self.ids[3]].bc_tag, "inflow")
        asked = []
        self.win.canvas.bcRequested.connect(asked.append)
        with mock.patch("eilmer_mesher.app.BCDialog") as dlg:      # the window opens its dialog too
            dlg.return_value.exec.return_value = False
            next(a for a in sub.actions() if a.text().startswith("New")).trigger()
        self.assertEqual(asked, [[self.ids[3]]])
        self.win.canvas.grab()                             # coloured curve and legend paint

    def test_dialog_fills_arguments_and_tag(self):
        dlg = BCDialog(self.m.doc)
        dlg.cls.setCurrentIndex(dlg.cls.findData("OutFlowBC_FixedP"))
        self.assertEqual(dlg.values(), ("outflow", "OutFlowBC_FixedP", "p_outside=101.325e3"))
        dlg.cls.setCurrentIndex(dlg.cls.findData("WallBC_NoSlip_FixedT"))
        self.assertEqual(dlg.values(), ("wall", "WallBC_NoSlip_FixedT", "Twall=300.0"))

    def test_new_bc_for_curves(self):
        with mock.patch("eilmer_mesher.app.BCDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.values.return_value = ("outflow", "OutFlowBC_Simple", "")
            self.win.new_bc_for([self.ids[1]])
        self.assertEqual(self.m.doc.curves[self.ids[1]].bc_tag, "outflow")

    def test_export_refuses_broken_blocks(self):
        self.m.add_curves([LineSeg(None, kind=REFERENCE, p0=(0.1, 0), p1=(0.1, 0.05))], "split")
        with mock.patch.object(QMessageBox, "warning") as warn, \
                mock.patch("eilmer_mesher.app.QFileDialog.getExistingDirectory") as pick:
            self.win.export_to_eilmer()
        warn.assert_called_once()
        pick.assert_not_called()

    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_export_end_to_end(self):
        self.m.define_bc("inflow", "InFlowBC_Supersonic", "flowState=inflow")
        self.m.define_bc("outflow", "OutFlowBC_Simple")
        self.m.set_bc([self.ids[3]], "inflow")
        self.m.set_bc([self.ids[1]], "outflow")
        folder = tempfile.mkdtemp(prefix="emesh-export-")
        self.addCleanup(shutil.rmtree, folder, True)
        with open(os.path.join(folder, "sim.lua"), "w") as f:
            f.write("-- mine\n")                             # existing files are kept
        with mock.patch("eilmer_mesher.app.QMessageBox.exec"):
            result = self.win.export_to(folder)
        self.assertTrue(result.ok, result.output)
        self.assertEqual(open(os.path.join(folder, "sim.lua")).read(), "-- mine\n")
        self.assertIn("ideal-air.lua", result.written)
        os.remove(os.path.join(folder, "sim.lua"))
        with mock.patch("eilmer_mesher.app.QMessageBox.exec"):
            self.win.export_to(folder)
        lmr = os.path.expanduser("~/gdtkinst/bin/lmr") if not shutil.which("lmr") else "lmr"
        for cmd in (["prep-gas", "-i", "ideal-air.lua", "-o", "ideal-air.gas"], ["prep-sim", "-j", "sim.lua"]):
            out = subprocess.run([lmr] + cmd, cwd=folder, capture_output=True, text=True)
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
        with open(os.path.join(folder, "lmrsim", "grid", "grid-0000.metadata")) as f:
            tags = json.load(f)["bcTags"]
        self.assertEqual((tags["west"], tags["east"]), ("inflow", "outflow"))
        self.assertTrue(os.path.exists(os.path.join(folder, "lmrsim", "snapshots", "0000", "fluid-0000.gz")))


if __name__ == "__main__":
    unittest.main()
