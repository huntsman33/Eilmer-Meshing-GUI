"""Base class for canvas tools."""
import math

from ...model.entities import ArcSeg, LineSeg
from ..canvas import COLORS

SAME_POINT_PX = 1.0


class Tool:
    name = ""
    label = ""
    snaps = True             # whether the canvas snaps the cursor for this tool
    hover_curves = False     # whether curves highlight under the cursor

    def __init__(self, canvas):
        self.canvas = canvas
        self.cursor = None
        self.reset()

    @property
    def model(self):
        return self.canvas.model

    # --- lifecycle -------------------------------------------------------------
    def activate(self):
        self.reset()

    def deactivate(self):
        self.reset()

    def reset(self):
        pass

    def busy(self):
        return False

    def cancel(self):
        """Esc: drop the operation in progress; False if there was nothing to cancel."""
        was_busy = self.busy()
        self.reset()
        return was_busy

    def finish(self):
        """Right-click or Enter: complete the operation in progress."""
        self.reset()

    # --- input -------------------------------------------------------------------
    def anchor(self):
        """Point used for horizontal/vertical inference and relative typed input."""
        return None

    def snap_exclude(self):
        return ()

    def press(self, ev):
        self.click(ev.point)

    def move(self, ev):
        self.cursor = ev.point

    def release(self, ev):
        pass

    def right_click(self, ev):
        self.finish()

    def click(self, point):
        pass

    def claims_key(self, key):
        return False

    def key(self, key):
        return False

    def double_click(self, ev):
        pass

    def accept_text(self, text):
        """Enter in the input box (display units). Returns an error message or None."""
        self.click(self.canvas.parse_point(text, self.anchor()))
        return None

    def preview_text(self, text):
        """Text being typed in the input box, before Enter (e.g. to lock a length)."""

    def consume_typed(self):
        """The typed value has been used: clear the input box."""
        self.canvas.clearTyped.emit()

    # --- text --------------------------------------------------------------------
    def prompt(self):
        return ""

    def hint(self):
        return ""

    # --- painting ------------------------------------------------------------------
    def paint(self, painter):
        """Rubber-band geometry, drawn in world coordinates."""

    def paint_overlay(self, painter):
        """Screen-space decorations."""

    def draw_preview(self, painter, curves):
        for c in curves:
            self.canvas.draw_curve(painter, c, COLORS["preview"], 1.5)

    # --- helpers -------------------------------------------------------------------
    def distinct(self, a, b):
        return math.hypot(a[0] - b[0], a[1] - b[1]) > self.canvas.px(SAME_POINT_PX)

    def line(self, a, b):
        return LineSeg(None, kind=self.model.new_kind(), p0=tuple(a), p1=tuple(b))

    def arcs(self, pieces):
        return [ArcSeg(None, kind=self.model.new_kind(), p0=a, p1=b, centre=c) for a, b, c in pieces]

    def add(self, curves, text):
        if curves:
            self.model.add_curves(curves, text)
