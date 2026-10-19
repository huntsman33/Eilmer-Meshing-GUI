from .block import BlockTool
from .dimension import DimensionTool
from .layer import LayerTool
from .modify import MergeTool, MirrorTool, MoveTool
from .draw import (ArcTool, BezierTool, CircleTool, LineTool, PointTool, ProfileTool, RectangleTool,
                   SplineTool, TriangleTool)
from .select import SelectTool
from .trim import TrimTool

# (tool class, tooltip); shortcuts come from settings ("tool.<name>")
SKETCH_TOOLS = [
    (SelectTool, "Select and move; double-click to edit dimensions"),
    (PointTool, "Construction point"),
    (LineTool, "Line (type a number to set the length)"),
    (ProfileTool, "Profile: chained lines and tangent arcs"),
    (ArcTool, "Arc: centre, start, end (M for 3-point)"),
    (CircleTool, "Circle"),
    (RectangleTool, "Rectangle"),
    (TriangleTool, "Triangle"),
    (BezierTool, "Bezier curve"),
    (SplineTool, "Spline through points"),
    (DimensionTool, "Dimension: length of a line, angle of an arc; edit to drive the geometry"),
    (MoveTool, "Move: click objects, then drag (or type dx, dy)"),
    (TrimTool, "Quick trim: delete a piece up to where it meets other curves"),
    (MergeTool, "Merge connected collinear lines, arcs of one circle or spline pieces"),
    (MirrorTool, "Mirror: pick objects, Enter, pick an axis line or the x / y axis, Enter"),
    (BlockTool, "Block: click a region to block it; double-click an edge for its cells"),
    (LayerTool, "Boundary layer: click block walls, Enter; double-click a wall for its settings (y+)"),
]
