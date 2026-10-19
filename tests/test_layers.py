"""Boundary layers: spacing maths, cutting and re-blocking, Eilmer output, and the tool."""
import math
import os
import unittest
from unittest import mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import numpy as np
from PySide6.QtCore import QEvent, Qt
from PySide6.QtTest import QTest
from PySide6.QtWidgets import QApplication

import fixtures  # noqa: F401
from test_blocking import ModelTest, box, circle, have_lmr
from test_sketch_gui import SketchTest, scr
from eilmer_mesher.app import MainWindow
from eilmer_mesher.lua.codegen import generate_geometry_lua
from eilmer_mesher.mesh.preview import block_grid
from eilmer_mesher.model import layers
from eilmer_mesher.model.document import BlockError, Document
from eilmer_mesher.model.entities import REFERENCE, LineSeg
from eilmer_mesher.sketch.block_dialogs import EdgeCellsDialog
from eilmer_mesher.sketch.layer_dialog import LayerDialog
from eilmer_mesher.units import Units

app = QApplication.instance() or QApplication([])

ROWS = {"south": ((slice(None), 0), (slice(None), 1)), "north": ((slice(None), -1), (slice(None), -2)),
        "west": ((0, slice(None)), (1, slice(None))), "east": ((-1, slice(None)), (-2, slice(None)))}


class TestSpacing(unittest.TestCase):
    def test_growth_ratio_inverts_thickness(self):
        for first, r, n in ((1e-6, 1.2, 30), (1e-3, 1.05, 8), (0.01, 1.5, 3), (2e-5, 1.001, 50)):
            length = layers.thickness(first, r, n)
            self.assertAlmostEqual(layers.growth_ratio(first / length, n), r, delta=1e-10)
        self.assertEqual(layers.growth_ratio(0.5, 4), 1.0)            # uniform is already finer
        self.assertAlmostEqual(layers.thickness(1e-3, 1.0, 10), 1e-2)

    def test_growth_values(self):
        x = layers.growth_values(1.2, 10, np.linspace(0, 1, 11))
        self.assertAlmostEqual(x[0], 0.0)
        self.assertAlmostEqual(x[-1], 1.0)
        np.testing.assert_allclose(np.diff(x)[1:] / np.diff(x)[:-1], 1.2)
        xr = layers.growth_values(1.2, 10, np.linspace(0, 1, 11), reverse=True)
        np.testing.assert_allclose(xr, 1.0 - x[::-1], atol=1e-15)

    def test_first_cell_from_yplus(self):
        h = layers.first_cell_height(1.0, 100.0, 1.225, 1.789e-5, 1.0)
        self.assertAlmostEqual(h, 3.94e-6, delta=0.02e-6)
        self.assertAlmostEqual(layers.first_cell_height(30.0, 100.0, 1.225, 1.789e-5, 1.0), 30 * h)
        with self.assertRaises(ValueError):
            layers.first_cell_height(1.0, 0.0, 1.225, 1.789e-5, 1.0)


class LayerTest(ModelTest):
    def wall_heights(self, wall_ids):
        """First cell heights on every block face lying on the given curves."""
        res = self.res()
        md = self.model.doc.with_blocks(res.blocks)
        out = []
        for b in res.blocks:
            g = block_grid(md, b)[:, :, :2]
            for f, (row, nxt) in ROWS.items():
                if {r.id for r in b.sides[f]} <= set(wall_ids):
                    out.append(np.hypot(*(g[nxt] - g[row]).T))
        return np.concatenate(out)

    def plate(self):
        self.ids = self.add(box(0, 0, 1, 0.2))
        self.bid = self.model.make_block((0.5, 0.1))
        return self.model.add_boundary_layer([(self.bid, "south")], 1e-4, 1.2, 10)


class TestCutting(LayerTest):
    def test_flat_plate(self):
        lid = self.plate()
        res = self.res()
        self.assertEqual((res.errors, res.warnings), ({}, {}))
        self.assertEqual(len(res.blocks), 2)
        np.testing.assert_allclose(self.wall_heights([self.ids[0]]), 1e-4, rtol=1e-9)
        thin = next(b for b in res.blocks if b.njv == 11)
        self.assertEqual(thin.niv, res.blocks[0].niv)
        layer = self.model.layer(lid)
        self.assertEqual(self.model.doc.curves[layer.curves[0]].type, "line")
        self.assertEqual(self.model.doc.curves[layer.curves[0]].kind, REFERENCE)
        # The outer block grades on from the last layer cell, at no more than the layer's ratio.
        outer = next(b for b in res.blocks if b is not thin)
        g = block_grid(self.model.doc.with_blocks(res.blocks), outer)[0, :, 1]
        h = np.diff(g)
        self.assertAlmostEqual(h[0], 1e-4 * 1.2 ** 9, delta=1e-12)
        self.assertLessEqual((h[1:] / h[:-1]).max(), 1.2 + 1e-9)

    def test_too_thick_is_refused_and_nothing_changes(self):
        self.ids = self.add(box(0, 0, 1, 0.2))
        bid = self.model.make_block((0.5, 0.1))
        before = self.model.doc.to_dict()
        with self.assertRaises(BlockError):
            self.model.add_boundary_layer([(bid, "south")], 0.05, 1.2, 10)
        self.assertEqual(self.model.doc.to_dict(), before)

    def test_crossing_the_opposite_wall_is_refused(self):
        from eilmer_mesher.model.entities import SplineSeg
        self.add([LineSeg(None, p0=(0, 0), p1=(1, 0)), LineSeg(None, p0=(1, 0), p1=(1, 1)),
                  SplineSeg(None, points=[(1, 1), (0.5, 0.02), (0, 1)]), LineSeg(None, p0=(0, 1), p1=(0, 0))])
        bid = self.model.make_block((0.5, 0.01))
        with self.assertRaisesRegex(BlockError, "would cross curve"):
            self.model.add_boundary_layer([(bid, "south")], 5e-3, 1.2, 10)

    def test_dashed_face_and_repeat_are_refused(self):
        lid = self.plate()
        res = self.res()
        thin = next(b for b in res.blocks if b.njv == 11)
        with self.assertRaises(BlockError):
            self.model.add_boundary_layer([(thin.id, "south")], 1e-4, 1.2, 10)
        self.assertEqual(self.model.layer_at_face(thin.id, "south"), lid)
        with self.assertRaises(BlockError):
            self.model.add_boundary_layer([(thin.id, "north")], 1e-4, 1.2, 10)    # the dashed cut

    def test_cut_continues_into_neighbours(self):
        self.add(box(0, 0, 2, 0.5) + [LineSeg(None, kind=REFERENCE, p0=(1, 0), p1=(1, 0.5))])
        self.model.make_all_blocks()
        right = self.model.block_at((1.5, 0.2))
        self.model.add_boundary_layer([(right, "south")], 1e-3, 1.2, 8)
        res = self.res()
        self.assertEqual(len(res.blocks), 4)
        self.assertEqual((res.errors, res.warnings), ({}, {}))
        if have_lmr():
            _, connections = self.eilmer()
            self.assertEqual(connections, 4)

    def test_corner_walls(self):
        ids = self.add(box(0, 0, 1, 1))
        bid = self.model.make_block((0.5, 0.5))
        self.model.add_boundary_layer([(bid, "south"), (bid, "west")], 1e-3, 1.2, 8)
        res = self.res()
        self.assertEqual(len(res.blocks), 4)
        self.assertEqual((res.errors, res.warnings), ({}, {}))
        np.testing.assert_allclose(self.wall_heights([ids[0], ids[3]]), 1e-3, rtol=1e-9)


class TestCylinder(LayerTest):
    def setUp(self):
        super().setUp()
        self.add(box(-2, -2, 2, 2) + circle((0, 0), 0.5))
        self.model.subdivide((1.5, 0), "ogrid_hole")
        self.cyl = {c.id for c in self.model.doc.curves.values() if c.type == "arc"}
        res = self.res()
        self.walls = [(b.id, f) for b in res.blocks for f in ("south", "north", "east", "west")
                      if {r.id for r in b.sides[f]} <= self.cyl]
        self.assertEqual(len(self.walls), 4)

    def test_ring_of_arcs_edit_remove_undo(self):
        base = sorted(b.id for b in self.model.doc.blocks)
        lid = self.model.add_boundary_layer(self.walls, 1e-3, 1.15, 12)
        layer = self.model.layer(lid)
        arcs = [self.model.doc.curves[c] for c in layer.curves]
        self.assertTrue(all(c.type == "arc" for c in arcs))
        delta = layers.thickness(1e-3, 1.15, 12)
        for c in arcs:
            self.assertAlmostEqual(math.dist(c.p0, c.centre), 0.5 + delta, delta=1e-12)
        self.assertEqual(len(self.res().blocks), 8)
        np.testing.assert_allclose(self.wall_heights(self.cyl), 1e-3, rtol=1e-9)

        self.model.edit_boundary_layer(lid, first=2e-3, cells=6)
        res = self.res()
        self.assertEqual((len(res.blocks), res.errors, res.warnings), (8, {}, {}))
        np.testing.assert_allclose(self.wall_heights(self.cyl), 2e-3, rtol=1e-9)
        self.assertEqual(sum(1 for b in res.blocks if 7 in (b.niv, b.njv)), 4)

        self.model.remove_boundary_layer(lid)
        self.assertEqual(sorted(b.id for b in self.model.doc.blocks), base)
        self.assertEqual(self.model.doc.layers, [])
        self.model.undo_stack.undo()
        self.assertEqual(len(self.model.doc.layers), 1)
        self.assertEqual(len(self.res().blocks), 8)

    @unittest.skipUnless(have_lmr(), "lmr not installed")
    def test_one_wall_closes_round_the_ring_in_eilmer(self):
        self.model.add_boundary_layer(self.walls[:1], 1e-3, 1.15, 12)
        res = self.res()
        self.assertEqual((len(res.blocks), res.errors, res.warnings), (8, {}, {}))
        _, connections = self.eilmer()                      # also checks preview == Eilmer
        self.assertEqual(connections, 12)
        lua = generate_geometry_lua(self.model.doc.with_blocks(res.blocks))
        self.assertIn("LuaFnClustering", lua)
        self.assertIn("growthCluster(1.15", lua)

    def test_save_and_load(self):
        self.model.add_boundary_layer(self.walls, 1e-3, 1.15, 12)
        doc = Document.from_dict(self.model.doc.to_dict())
        self.assertEqual(doc.to_dict(), self.model.doc.to_dict())
        self.assertEqual(doc.layers[0].cells, 12)


class TestWallTags(LayerTest):
    def test_walls_become_no_slip_and_revert(self):
        lid = self.plate()
        wall = self.model.doc.curves[self.ids[0]]
        self.assertEqual(wall.bc_tag, "wall")
        self.assertEqual(self.model.doc.bcs["wall"]["class"], "WallBC_NoSlip_Adiabatic")
        self.assertEqual([c.bc_tag for c in self.model.doc.curves.values() if c.id != wall.id
                          and c.kind != REFERENCE], ["", "", ""])          # only the wall
        md = self.model.doc.with_blocks(self.res().blocks)
        self.assertIn('south="wall"', generate_geometry_lua(md))
        self.model.edit_boundary_layer(lid, cells=6)                       # re-cut keeps the tag
        self.assertEqual(self.model.doc.curves[self.ids[0]].bc_tag, "wall")
        self.model.remove_boundary_layer(lid)
        self.assertEqual(self.model.doc.curves[self.ids[0]].bc_tag, "")

    def test_existing_tags(self):
        self.ids = self.add(box(0, 0, 1, 0.2))
        bid = self.model.make_block((0.5, 0.1))
        self.model.define_bc("hot", "WallBC_NoSlip_FixedT", "Twall=500.0")
        self.model.define_bc("inflow", "InFlowBC_Supersonic", "flowState=inflow")
        self.model.set_bc([self.ids[3]], "inflow")
        lid = self.model.add_boundary_layer([(bid, "south"), (bid, "west")], 1e-4, 1.2, 10)
        curves = self.model.doc.curves
        self.assertEqual((curves[self.ids[0]].bc_tag, curves[self.ids[3]].bc_tag), ("hot", "inflow"))
        self.assertNotIn("wall", self.model.doc.bcs)                       # reused the no-slip tag
        self.model.define_bc("cold", "WallBC_NoSlip_FixedT", "Twall=200.0")
        self.model.set_bc([self.ids[0]], "cold")                           # the user's later choice stays
        self.model.remove_boundary_layer(lid)
        self.assertEqual(curves[self.ids[0]].bc_tag, "cold")


class TestCascade(LayerTest):
    def test_editing_an_earlier_layer_keeps_later_ones(self):
        self.add(box(0, 0, 1, 0.3))
        bid = self.model.make_block((0.5, 0.1))
        first = self.model.add_boundary_layer([(bid, "south")], 1e-3, 1.2, 8)
        outer = self.model.block_at((0.5, 0.25))
        second = self.model.add_boundary_layer([(outer, "north")], 2e-3, 1.2, 6)
        self.assertEqual(len(self.res().blocks), 3)
        self.model.edit_boundary_layer(first, cells=5)
        res = self.res()
        self.assertEqual((len(res.blocks), res.errors), (3, {}))
        self.assertEqual([layer.id for layer in self.model.doc.layers], [first, second])
        self.assertEqual(sorted(b.njv for b in res.blocks)[:2], [6, 7])
        self.model.remove_boundary_layer(first)
        self.assertEqual(len(self.res().blocks), 2)
        self.assertEqual([layer.id for layer in self.model.doc.layers], [second])


class TestEdgeDialog(unittest.TestCase):
    def test_first_cell_spacing_round_trip(self):
        spec = {"type": "Growth", "first": 2.54e-4, "reverse": True}
        dlg = EdgeCellsDialog(10, spec, units=Units("mm"))
        self.assertEqual(dlg.first.text(), "0.254")
        cells, out, _ = dlg.values()
        self.assertEqual(cells, 10)
        self.assertAlmostEqual(out["first"], 2.54e-4)
        self.assertTrue(out["reverse"])


class TestLayerTool(SketchTest):
    def setUp(self):
        super().setUp()
        self.ids = self.model.add_curves(box(-2, -1, 2, 1), "box")
        self.bid = self.model.make_block((0, 0))
        self.messages = []
        self.canvas.message.connect(self.messages.append)
        self.canvas.set_tool(self.tools["layer"])

    def test_pick_walls_and_enter(self):
        QTest.mouseMove(self.canvas, scr((0, -1)))
        QTest.mouseClick(self.canvas, Qt.LeftButton, Qt.NoModifier, scr((0, -1)))
        QTest.mouseMove(self.canvas, scr((0, 1)))
        QTest.mouseClick(self.canvas, Qt.LeftButton, Qt.NoModifier, scr((0, 1)))
        self.assertEqual(self.tools["layer"].walls, [(self.bid, "south"), (self.bid, "north")])
        self.canvas.grab()                                  # picked walls paint
        with mock.patch("eilmer_mesher.settings.layer_defaults",
                        return_value={"first": 1e-3, "growth": 1.2, "cells": 5, "transition": True}):
            QTest.keyClick(self.canvas, Qt.Key_Return)
        res = self.canvas.resolution()
        self.assertEqual((len(res.blocks), res.errors), (3, {}))
        self.assertEqual(len(self.model.doc.layers), 1)
        self.canvas.grab()                                  # layer walls paint with ticks
        thin = res.by_id()[self.model.block_at((0, -0.9999))]
        self.assertEqual(thin.njv, 6)
        QTest.mouseMove(self.canvas, scr((0, -1)))
        QTest.mouseClick(self.canvas, Qt.LeftButton, Qt.NoModifier, scr((0, -1)))
        self.assertIn("already has a boundary layer", self.messages[-1])
        asked = []
        self.canvas.layerRequested.connect(lambda b, f: asked.append((b, f)))
        QTest.mouseDClick(self.canvas, Qt.LeftButton, Qt.NoModifier, scr((0, -1)))
        self.assertEqual(asked, [(thin.id, "south")])
        self.assertEqual(self.model.layer_at_face(thin.id, "south"), self.model.doc.layers[0].id)

    def test_escape_drops_picks(self):
        QTest.mouseMove(self.canvas, scr((0, -1)))
        QTest.mouseClick(self.canvas, Qt.LeftButton, Qt.NoModifier, scr((0, -1)))
        QTest.keyClick(self.canvas, Qt.Key_Escape)
        self.assertEqual(self.tools["layer"].walls, [])


class TestLayerWindow(unittest.TestCase):
    def setUp(self):
        self.win = MainWindow()
        self.win.mesher.enabled = False
        self.m = self.win.model
        self.m.add_curves(box(0, 0, 1, 0.2), "box")
        self.bid = self.m.make_block((0.5, 0.1))

    def tearDown(self):
        self.win.deleteLater()
        QApplication.sendPostedEvents(None, QEvent.DeferredDelete)

    def test_dialog_values_and_estimate(self):
        dlg = LayerDialog(Units("mm"), {"first": 1e-5, "growth": 1.2, "cells": 10, "transition": True},
                          {"yplus": 1.0, "velocity": 100.0, "density": 1.225, "viscosity": 1.789e-5, "length": 1.0})
        self.assertEqual(dlg.first.text(), "0.01")
        self.assertIn("Layer thickness 0.2595", dlg.summary.text())
        dlg._use_estimate()
        self.assertAlmostEqual(dlg.values()["first"], 3.94e-6, delta=0.01e-6)
        self.assertEqual(dlg.flow_values()["length"], 1.0)

    def test_settings_for_new_layers_then_edit_and_remove(self):
        with mock.patch("eilmer_mesher.app.LayerDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.removed = False
            dlg.return_value.values.return_value = {"first": 2e-4, "growth": 1.1, "cells": 7, "transition": False}
            dlg.return_value.flow_values.return_value = {"yplus": 2.0}
            self.win.edit_layer(self.bid, "south")
        from eilmer_mesher import settings
        self.assertEqual(settings.layer_defaults()["cells"], 7)
        self.assertFalse(settings.layer_defaults()["transition"])
        self.assertEqual(settings.layer_flow()["yplus"], 2.0)

        lid = self.m.add_boundary_layer([(self.bid, "south")], 2e-4, 1.1, 7, False)
        thin = next(b for b in self.m.resolution().blocks if b.njv == 8)
        with mock.patch("eilmer_mesher.app.LayerDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.removed = False
            dlg.return_value.values.return_value = {"first": 1e-4, "growth": 1.2, "cells": 9, "transition": True}
            dlg.return_value.flow_values.return_value = {}
            self.win.edit_layer(thin.id, "south")
        self.assertEqual(self.m.layer(lid).cells, 9)
        thin = next(b for b in self.m.resolution().blocks if b.njv == 10)
        with mock.patch("eilmer_mesher.app.LayerDialog") as dlg:
            dlg.return_value.exec.return_value = True
            dlg.return_value.removed = True
            dlg.return_value.flow_values.return_value = {}
            self.win.edit_layer(thin.id, "south")
        self.assertEqual(self.m.doc.layers, [])
        self.assertEqual([b.id for b in self.m.doc.blocks], [self.bid])


if __name__ == "__main__":
    unittest.main()
