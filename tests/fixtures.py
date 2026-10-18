"""Example documents shared by the tests."""
import os
import sys

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from eilmer_mesher.model.document import Block, CurveRef, Document
from eilmer_mesher.model.entities import REFERENCE, ArcSeg, BezierSeg, LineSeg


def sides(**faces):
    """sides(north="c1", south="-c2", west=["c3", "c4"]); a leading '-' means reversed."""
    out = {}
    for face, ids in faces.items():
        ids = [ids] if isinstance(ids, str) else ids
        out[face] = [CurveRef(i.lstrip("-"), i.startswith("-")) for i in ids]
    return out


def sphere_doc():
    """tutes/wk9/sphere.lua expressed as a Document."""
    ctr, a, b = (0.01, 0.0), (0.0, 0.0), (0.01, 0.01)
    c, d = (0.01, 0.02), (-0.003, 0.0)
    dc, cd = (-0.003, 0.01), (0.003, 0.015)
    doc = Document()
    doc.add_curve(ArcSeg("east", p0=a, p1=b, centre=ctr))
    doc.add_curve(LineSeg("north", p0=c, p1=b, bc_tag="outflow"))
    doc.add_curve(BezierSeg("west", points=[d, dc, cd, c], bc_tag="inflow"))
    doc.add_curve(LineSeg("south", p0=d, p1=a))
    doc.add_block(Block("blk0", sides(north="north", east="east", south="south", west="west"),
                        niv=61, njv=81))
    doc.bcs = {"inflow": {"class": "InFlowBC_Supersonic", "args": "flowState=inflow"},
               "outflow": {"class": "OutFlowBC_Simple", "args": ""}}
    return doc


WK7_SIZE = 21


def wk7_doc(smoothing=None):
    """The tutes/wk7 Coons patch domain."""
    p00, p10, p11, p01 = (0.0, 0.0), (1.0, 0.0), (2.0, 1.0), (0.0, 2.0)
    pb1, pb2, pa1 = (0.7, 2.5), (0.7, 1.0), (2.0, 0.0)
    doc = Document()
    doc.add_curve(LineSeg("s", p0=p00, p1=p10))
    doc.add_curve(BezierSeg("n", points=[p01, pb1, p11]))
    doc.add_curve(BezierSeg("w", points=[p00, pb2, p01]))
    doc.add_curve(ArcSeg("e", p0=p10, p1=p11, centre=pa1))
    doc.add_block(Block("blk0", sides(north="n", east="e", south="s", west="w"),
                        niv=WK7_SIZE, njv=WK7_SIZE, smoothing=smoothing))
    return doc


def two_block_doc():
    """
    [0,2]x[0,1] split at x=1 by a dashed reference line.
    Exercises a reversed curve (b's south is drawn east->west) and a
    two-curve Polyline face (a's north).
    """
    doc = Document()
    doc.add_curve(LineSeg("sa", p0=(0, 0), p1=(1, 0)))
    doc.add_curve(LineSeg("sb", p0=(2, 0), p1=(1, 0)))            # drawn backwards
    doc.add_curve(LineSeg("na1", p0=(0, 1), p1=(0.5, 1)))
    doc.add_curve(LineSeg("na2", p0=(0.5, 1), p1=(1, 1)))
    doc.add_curve(LineSeg("nb", p0=(1, 1), p1=(2, 1)))
    doc.add_curve(LineSeg("w", p0=(0, 0), p1=(0, 1), bc_tag="inflow"))
    doc.add_curve(LineSeg("e", p0=(2, 0), p1=(2, 1), bc_tag="outflow"))
    doc.add_curve(LineSeg("ref", p0=(1, 0), p1=(1, 1), kind=REFERENCE))
    doc.add_block(Block("a", sides(north=["na1", "na2"], east="ref", south="sa", west="w"),
                        niv=11, njv=11))
    doc.add_block(Block("b", sides(north="nb", east="e", south="-sb", west="ref"),
                        niv=11, njv=11))
    return doc
