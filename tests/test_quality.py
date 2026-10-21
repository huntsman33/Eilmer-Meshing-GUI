"""Phase 6: cell quality, the warning banner, Refine (Laplace / AOPatch) and the heat-map."""
import os
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import QEvent
from PySide6.QtWidgets import QApplication, QMenu

import fixtures  # noqa: F401
from test_blocking import ModelTest, have_lmr
from eilmer_mesher import settings
from eilmer_mesher.app import MainWindow
from eilmer_mesher.lua.codegen import generate_geometry_lua
from eilmer_mesher.mesh import quality
from eilmer_mesher.mesh.live import Mesh
from eilmer_mesher.mesh.preview import block_grid
from eilmer_mesher.model.entities import LineSeg, SplineSeg

app = QApplication.instance() or QApplication([])


def quad(a, b, c, d):
    g = np.zeros((2, 2, 3))
    g[0, 0, :2], g[1, 0, :2], g[1, 1, :2], g[0, 1, :2] = a, b, c, d
    return g


def bulge(height=2.0):
    """A square whose top bulges up: Coons cells near the top are badly skewed."""
    return [LineSeg(None, p0=(0, 0), p1=(1, 0)), LineSeg(None, p0=(1, 0), p1=(1, 1)),
            SplineSeg(None, points=[(1, 1), (0.5, 1 + height), (0, 1)]), LineSeg(None, p0=(0, 1), p1=(0, 0))]


def dent():
    """A square whose top dips deep inside: Laplace smoothing folds cells here."""
    return [LineSeg(None, p0=(0, 0), p1=(1, 0)), LineSeg(None, p0=(1, 0), p1=(1, 1)),
            SplineSeg(None, points=[(1, 1), (0.6, 1), (0.5, 0.3), (0.4, 1), (0, 1)]),
            LineSeg(None, p0=(0, 1), p1=(0, 0))]


def preview(model):
    res = model.resolution()
    md = model.doc.with_blocks(res.blocks)
    return Mesh(model.revision, [(b.id, block_grid(md, b)) for b in res.blocks], "preview")


class TestMetrics(unittest.TestCase):
    def test_reference_cells(self):
        m = quality.cell_metrics(quad((0, 0), (1, 0), (1, 1), (0, 1)))
        self.assertEqual((m["skew"][0, 0], m["ar"][0, 0], m["inverted"][0, 0]), (0.0, 1.0, False))
        m = quality.cell_metrics(quad((0, 0), (1, 0), (1.5, 3 ** 0.5 / 2), (0.5, 3 ** 0.5 / 2)))
        self.assertAlmostEqual(m["skew"][0, 0], 1 / 3)                 # 60 / 120 degree corners
        self.assertAlmostEqual(quality.cell_metrics(quad((0, 0), (10, 0), (10, 1), (0, 1)))["ar"][0, 0], 10)

    def test_folded_cells(self):
        m = quality.cell_metrics(quad((0, 0), (1, 0), (0, 1), (1, 1)))  # bow-tie
        self.assertTrue(m["inverted"][0, 0])
        self.assertEqual(m["skew"][0, 0], 1.0)
        # A block that runs clockwise is not "inverted": only cells against their block are.
        g = np.zeros((3, 2, 3))
        g[:, 0, 0], g[:, 1, 0], g[:, 1, 1] = [0, 1, 2], [0, 1, 2], -1.0
        self.assertFalse(quality.cell_metrics(g)["inverted"].any())
        g[1, 1, :2] = (1.0, 0.5)                                        # push a vertex through
        self.assertTrue(quality.cell_metrics(g)["inverted"].any())


class TestAssess(ModelTest):
    def test_bulge_needs_refinement(self):
        self.add(bulge())
        bid = self.model.make_block((0.5, 0.5))
        self.model.set_edge_cells(bid, "south", 20)
        self.model.set_edge_cells(bid, "west", 20)
        q = quality.assess(preview(self.model))
        self.assertEqual(q.level(), "bad")
        self.assertEqual(q.flagged("bad"), [bid])
        self.assertIn("skewness > 0.85", q.summary())
        q = quality.assess(preview(self.model), {"skew_warn": 0.95, "skew_bad": 0.99})
        self.assertEqual(q.level(), "ok")

    def test_boundary_layer_aspect_is_not_flagged(self):
        from test_blocking import box
        self.add(box(0, 0, 1, 0.2))
        bid = self.model.make_block((0.5, 0.1))
        self.model.add_boundary_layer([(bid, "south")], 1e-5, 1.2, 10)
        res = self.model.resolution()
        skip = quality.intentional_aspect(res.blocks)
        self.assertEqual(len(skip), 1)
        q = quality.assess(preview(self.model), skip_ar=skip)
        thin = next(iter(skip))
        self.assertGreater(q.blocks[thin].max_ar, 1000)
        self.assertEqual(q.blocks[thin].ar_warn, 0)
        self.assertGreater(quality.assess(preview(self.model)).blocks[thin].ar_warn, 0)


class TestRefine(ModelTest):
    def block(self, curves):
        self.add(curves)
        bid = self.model.make_block((0.5, 0.1))
        self.model.set_edge_cells(bid, "south", 20)
        self.model.set_edge_cells(bid, "west", 20)
        return bid

    def test_laplace_where_it_helps(self):
        bid = self.block(bulge())
        before = quality.assess(preview(self.model)).blocks[bid].skew_bad
        self.assertEqual(self.model.refine_blocks([bid]), ([bid], [], []))
        self.assertIsNotNone(self.model.doc.blocks[0].smoothing)
        after = quality.assess(preview(self.model)).blocks[bid].skew_bad
        self.assertLess(after, before / 2)
        self.model.undo_stack.undo()
        self.assertIsNone(self.model.doc.blocks[0].smoothing)

    def test_aopatch_where_laplace_would_fold(self):
        bid = self.block(dent())
        self.assertEqual(self.model.refine_blocks([bid]), ([], [bid], []))
        b = self.model.doc.blocks[0]
        self.assertEqual((b.patch, b.smoothing), ("AOPatch", None))
        self.assertIn("AOPatch:new{", generate_geometry_lua(self.model.doc.with_blocks(self.model.resolution().blocks)))

    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_eilmer_aopatch_untangles_the_dent(self):
        bid = self.block(dent())
        self.model.refine_blocks([bid])
        result, _ = self.eilmer(check_preview=False)      # AOPatch: Eilmer's own mesh differs from the preview
        from eilmer_mesher.mesh.runner import preview_mesh
        res = self.model.resolution()
        out = preview_mesh(self.model.doc.with_blocks(res.blocks), self.tmp)
        q = quality.assess(Mesh(0, [(bid, out.grids[0])], "eilmer"))
        self.assertEqual(q.blocks[bid].inverted, 0)
        self.assertLess(q.blocks[bid].skew_bad, 8)

    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_smoothed_block_matches_eilmer(self):
        bid = self.block(bulge())
        self.model.refine_blocks([bid])
        self.eilmer()                                     # Laplace in Lua == the numpy preview

    def test_boundary_layer_blocks_are_skipped(self):
        from test_blocking import box
        self.add(box(0, 0, 1, 0.2))
        bid = self.model.make_block((0.5, 0.1))
        self.model.add_boundary_layer([(bid, "south")], 1e-5, 1.2, 10)
        thin = next(iter(quality.intentional_aspect(self.model.resolution().blocks)))
        self.assertEqual(self.model.refine_blocks([thin]), ([], [], [thin]))


class TestQualityWindow(unittest.TestCase):
    def setUp(self):
        self.win = MainWindow()
        self.win.mesher.enabled = False
        self.m = self.win.model
        self.m.add_curves(bulge(), "bulge")
        self.bid = self.m.make_block((0.5, 0.5))
        self.m.set_edge_cells(self.bid, "south", 20)
        self.m.set_edge_cells(self.bid, "west", 20)
        self.win.resize(900, 700)
        self.win.show()

    def tearDown(self):
        settings.set_quality_thresholds(quality.DEFAULT_THRESHOLDS)
        self.win.hide()                                   # (close() would ask to save)
        self.win.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)

    def test_banner_refine_and_heatmap(self):
        self.win.update_quality(preview(self.m))
        self.assertTrue(self.win.banner.isVisible())
        self.assertIn("needs refinement", self.win.banner.text.text())
        self.assertTrue(self.win.act_refine.isEnabled())
        self.assertFalse(self.win.banner.ao.isVisible())
        for metric in ("skew", "ar"):                     # the heat-map paints both metrics
            self.win.metric_actions[metric].trigger()
            self.assertTrue(self.win.canvas.show_quality)
            self.win.canvas.grab()

        self.win.act_refine.trigger()
        self.assertIsNotNone(self.m.doc.blocks[0].smoothing)
        self.win.update_quality(preview(self.m))          # still some bad cells at the corners
        self.assertFalse(self.win.act_refine.isEnabled())
        self.assertTrue(self.win.banner.ao.isVisible())
        self.win.banner.ao.click()
        self.assertEqual(self.m.doc.blocks[0].patch, "AOPatch")

    def test_dismiss_and_thresholds(self):
        self.win.update_quality(preview(self.m))
        self.win.banner.dismiss()
        self.win.update_quality(preview(self.m))
        self.assertFalse(self.win.banner.isVisible())     # same problems: stays hidden
        with mock.patch("eilmer_mesher.app.ThresholdsDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.values.return_value = {"skew_warn": 0.97, "skew_bad": 0.99, "ar_warn": 50.0}
            self.win.mesher.mesh = preview(self.m)
            self.win.edit_thresholds()
        self.assertEqual(self.win.mesh_quality.level(), "ok")
        self.assertFalse(self.win.banner.isVisible())
        self.win.update_quality(None)
        self.assertIsNone(self.win.canvas.quality)

    def test_block_patch_menu(self):
        tool = self.win.tools["block"]
        menu = QMenu()
        tool._patch_menu(menu, self.bid)
        sub = menu.actions()[0].menu()
        next(a for a in sub.actions() if "Laplace" in a.text()).trigger()
        self.assertIsNotNone(self.m.doc.blocks[0].smoothing)
        with mock.patch("eilmer_mesher.sketch.block_dialogs.AOPatchDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.values.return_value = {"nx": 12, "ny": 8}
            next(a for a in sub.actions() if a.text().startswith("AOPatch")).trigger()
        b = self.m.doc.blocks[0]
        self.assertEqual((b.patch, b.patch_opts, b.smoothing), ("AOPatch", {"nx": 12, "ny": 8}, None))
        self.win.canvas.grab()                            # label shows "AO"


if __name__ == "__main__":
    unittest.main()
