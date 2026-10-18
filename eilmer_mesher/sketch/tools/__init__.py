from .draw import (ArcTool, BezierTool, CircleTool, LineTool, ProfileTool, RectangleTool,
                   SplineTool, TriangleTool)
from .select import SelectTool

# (tool class, shortcut, tooltip)
SKETCH_TOOLS = [
    (SelectTool, "S", "Select and move"),
    (LineTool, "L", "Line"),
    (ProfileTool, "P", "Profile: chained lines and tangent arcs"),
    (ArcTool, "A", "Arc: 3-point or centre-start-end (M switches)"),
    (CircleTool, "C", "Circle"),
    (RectangleTool, "R", "Rectangle"),
    (TriangleTool, "T", "Triangle"),
    (BezierTool, "B", "Bezier curve"),
    (SplineTool, "N", "Spline through points"),
]
