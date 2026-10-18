"""
Parsing for the typed-coordinate box.

  x, y        absolute point
  @dx, dy     relative to the last point
  @L<deg      polar, relative to the last point
  L<deg       polar, from the origin
A single number is left to the active tool (length, radius, ...).
"""
import math
import re

_SPLIT = re.compile(r"[,\s]+")


def parse_numbers(text):
    parts = [p for p in _SPLIT.split(text.strip()) if p]
    try:
        return [float(p) for p in parts]
    except ValueError:
        raise ValueError(f"not a number list: {text!r}")


def single_number(text):
    """The value if text is one number, else None."""
    try:
        nums = parse_numbers(text)
    except ValueError:
        return None
    return nums[0] if len(nums) == 1 and "<" not in text and "@" not in text else None


def parse_point(text, anchor=None):
    t = text.strip()
    relative = t.startswith("@")
    if relative:
        if anchor is None:
            raise ValueError("no previous point for a relative coordinate")
        t = t[1:]
    base = anchor if relative else (0.0, 0.0)
    if "<" in t:
        length_text, angle_text = t.split("<", 1)
        length = float(length_text)
        angle = math.radians(float(angle_text))
        return (base[0] + length*math.cos(angle), base[1] + length*math.sin(angle))
    nums = parse_numbers(t)
    if len(nums) != 2:
        raise ValueError("expected two numbers: x, y")
    return (base[0] + nums[0], base[1] + nums[1])
