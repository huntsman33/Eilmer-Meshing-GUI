"""
Phase 1 pipeline tests: Document -> geometry.lua -> lmr -> VTK -> numpy.
Tests that call lmr are skipped when it is not installed.
"""
import json
import os
import shutil
import tempfile
import unittest

import numpy as np

from fixtures import WK7_SIZE, sphere_doc, two_block_doc, wk7_doc

from eilmer_mesher.lua.codegen import generate_bcs_lua, generate_geometry_lua
from eilmer_mesher.mesh import preview
from eilmer_mesher.mesh.runner import export_grids, find_lmr, preview_mesh
from eilmer_mesher.mesh.vtkio import read_structured_vtk
from eilmer_mesher.model.document import BlockError, Document
from gridgen.gridutils import write_grid_to_vtk
from gridgen.vector3 import Vector3


def have_lmr():
    try:
        find_lmr()
        return True
    except FileNotFoundError:
        return False


def reference_smooth_grid(vertices, nsweeps):
    """Reference Jacobi Laplace smoother on gridgen Vector3 vertices (boundary fixed)."""
    niv, njv = len(vertices), len(vertices[0])
    for _ in range(nsweeps):
        old = [[Vector3(p) for p in row] for row in vertices]
        for i in range(1, niv - 1):
            for j in range(1, njv - 1):
                vertices[i][j] = 0.25 * (old[i + 1][j] + old[i - 1][j] + old[i][j + 1] + old[i][j - 1])
    return vertices


class TempDirTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="emesh-test-")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)


class TestVtkio(TempDirTest):
    def test_round_trip_with_gridgen_writer(self):
        niv, njv = 4, 3
        verts = [[Vector3(i * 0.5, j * 0.25 + 0.1 * i) for j in range(njv)] for i in range(niv)]
        base = os.path.join(self.tmp, "g")
        write_grid_to_vtk(base, verts)
        xyz = read_structured_vtk(base + ".vtk")
        self.assertEqual(xyz.shape, (niv, njv, 3))
        for i in range(niv):
            for j in range(njv):
                self.assertAlmostEqual(xyz[i, j, 0], verts[i][j].x, places=12)
                self.assertAlmostEqual(xyz[i, j, 1], verts[i][j].y, places=12)


class TestDocument(TempDirTest):
    def test_json_round_trip_gives_identical_lua(self):
        for doc in (sphere_doc(), wk7_doc({"nsweeps": 10, "tol": 0.0}), two_block_doc()):
            path = os.path.join(self.tmp, "p.emesh.json")
            doc.save(path)
            again = Document.load(path)
            self.assertEqual(generate_geometry_lua(doc), generate_geometry_lua(again))
            self.assertEqual(json.load(open(path)), again.to_dict())

    def test_open_corner_rejected(self):
        doc = two_block_doc()
        doc.curves["sa"].p1 = (0.9, 0.0)
        with self.assertRaisesRegex(BlockError, "south"):
            generate_geometry_lua(doc)

    def test_mixed_bc_tags_on_one_face_rejected(self):
        doc = two_block_doc()
        doc.curves["na1"].bc_tag = "inflow"
        doc.curves["na2"].bc_tag = "outflow"
        with self.assertRaisesRegex(BlockError, "mixes BC tags"):
            generate_geometry_lua(doc)

    def test_reference_curves_never_tagged(self):
        doc = two_block_doc()
        doc.curves["ref"].bc_tag = "inflow"
        lua = generate_geometry_lua(doc)
        self.assertIn('tag="a", fsTag="initial", bcTags={west="inflow"}', lua)
        self.assertIn('tag="b", fsTag="initial", bcTags={east="outflow"}', lua)

    def test_bcs_stub(self):
        bcs = generate_bcs_lua(sphere_doc())
        self.assertIn("inflow = InFlowBC_Supersonic:new{flowState=inflow},", bcs)
        self.assertIn("outflow = OutFlowBC_Simple:new{},", bcs)


@unittest.skipUnless(have_lmr(), "lmr not installed")
class TestEilmer(TempDirTest):
    def test_sphere_matches_gridgen_preview(self):
        doc = sphere_doc()
        result = preview_mesh(doc, self.tmp)
        self.assertTrue(result.ok, result.output)
        self.assertEqual(len(result.grids), 1)
        eilmer = result.grids[0]
        self.assertEqual(eilmer.shape, (61, 81, 3))
        python = preview.block_grid(doc, doc.blocks[0])
        np.testing.assert_allclose(eilmer, python, rtol=0, atol=1e-12)

    def test_sphere_prep_grids_metadata(self):
        result = export_grids(sphere_doc(), self.tmp)
        self.assertTrue(result.ok, result.output)
        with open(os.path.join(self.tmp, "lmrsim", "grid", "grid-0000.metadata")) as f:
            md = json.load(f)
        self.assertEqual((md["niv"], md["njv"]), (61, 81))
        self.assertEqual(md["bcTags"]["west"], "inflow")
        self.assertEqual(md["bcTags"]["north"], "outflow")
        self.assertEqual(md["bcTags"]["south"], "")
        self.assertEqual(md["fsTag"], "initial")

    def test_laplace_lua_matches_reference_smoother(self):
        nsweeps = 500
        doc = wk7_doc({"nsweeps": nsweeps, "tol": 0.0})
        result = preview_mesh(doc, self.tmp)
        self.assertTrue(result.ok, result.output)

        from gridgen.path import Arc, Bezier, Line
        from gridgen.sgrid_gen import gridpts
        p00, p10, p11, p01 = Vector3(0.0, 0.0), Vector3(1.0, 0.0), Vector3(2.0, 1.0), Vector3(0.0, 2.0)
        south, north = Line(p00, p10), Bezier([p01, Vector3(0.7, 2.5), p11])
        west, east = Bezier([p00, Vector3(0.7, 1.0), p01]), Arc(p10, p11, Vector3(2.0, 0.0))
        pts = reference_smooth_grid(gridpts(north, east, south, west, WK7_SIZE, WK7_SIZE), nsweeps)
        reference = np.array([[(p.x, p.y, p.z) for p in row] for row in pts])

        np.testing.assert_allclose(result.grids[0], reference, rtol=0, atol=1e-12)
        np.testing.assert_allclose(preview.block_grid(doc, doc.blocks[0]), reference, rtol=0, atol=1e-12)

    def test_two_blocks_connect_across_reference_line(self):
        doc = two_block_doc()
        result = preview_mesh(doc, self.tmp)
        self.assertTrue(result.ok, result.output)
        a, b = result.grids
        np.testing.assert_allclose(a[-1, :, :], b[0, :, :], atol=1e-14)   # shared interface
        np.testing.assert_allclose(b[:, 0, 0], np.linspace(1, 2, 11), atol=1e-14)  # reversed south

        result = export_grids(doc, self.tmp)
        self.assertTrue(result.ok, result.output)
        with open(os.path.join(self.tmp, "lmrsim", "grid", "grid.metadata")) as f:
            connections = json.load(f)["grid-connections"]
        self.assertEqual(len(connections), 1)

    def test_lua_error_is_reported(self):
        from eilmer_mesher.model.entities import ArcSeg
        doc = two_block_doc()
        # Endpoints still close the block, but the centre is not equidistant from them.
        doc.curves["sa"] = ArcSeg("sa", p0=(0, 0), p1=(1, 0), centre=(0.3, 0.5))
        result = preview_mesh(doc, self.tmp)
        self.assertFalse(result.ok)
        self.assertEqual(result.grids, [])
        self.assertTrue(result.error_summary.startswith("Arc.evaluate(): radii do not match"),
                        result.error_summary)

    def test_lua_syntax_error_summary(self):
        with open(os.path.join(self.tmp, "bad.lua"), "w") as f:
            f.write("x = Line:new{p0=nil\n")
        from eilmer_mesher.mesh.runner import run_lmr
        result = run_lmr("custom-script", "bad.lua", self.tmp)
        self.assertFalse(result.ok)
        self.assertIn("bad.lua", result.error_summary)

    def test_aopatch_block(self):
        doc = wk7_doc()
        doc.blocks[0].patch = "AOPatch"
        doc.blocks[0].patch_opts = {"nx": 10, "ny": 10}
        result = preview_mesh(doc, self.tmp)
        self.assertTrue(result.ok, result.output)
        g = result.grids[0]
        coons = preview.block_grid(wk7_doc(), wk7_doc().blocks[0])
        np.testing.assert_allclose(g[0, :], coons[0, :], atol=1e-9)   # boundaries unchanged
        self.assertGreater(np.abs(g - coons).max(), 1e-6)             # interior redistributed


if __name__ == "__main__":
    unittest.main()
