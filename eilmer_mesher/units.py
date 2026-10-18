"""
Display units. The document, the generated Lua and Eilmer always work in metres;
units only change what the user sees and types.
"""
import math

UNITS = {           # name -> metres per unit
    "m": 1.0,
    "mm": 1.0e-3,
    "in": 0.0254,
    "ft": 0.3048,
}
DEFAULT_UNITS = "m"


class Units:
    def __init__(self, name=DEFAULT_UNITS):
        if name not in UNITS:
            name = DEFAULT_UNITS
        self.name = name
        self.factor = UNITS[name]

    def to_display(self, metres):
        return metres / self.factor

    def from_display(self, value):
        return value * self.factor

    def point_to_display(self, p):
        return (p[0] / self.factor, p[1] / self.factor)

    def point_from_display(self, p):
        return (p[0] * self.factor, p[1] * self.factor)

    def digits(self, step_metres):
        """Decimals worth showing for a resolution of step_metres."""
        step = max(step_metres / self.factor, 1e-300)
        return max(0, -math.floor(math.log10(step)) + 1)

    def format(self, metres, digits=None):
        value = self.to_display(metres)
        if digits is None:
            return f"{value:.6g} {self.name}"
        return f"{value:.{digits}f} {self.name}"
