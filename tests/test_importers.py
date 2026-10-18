"""Point-file import: Selig, Lednicer and plain xy formats."""
import math
import unittest

import numpy as np

import fixtures  # noqa: F401
from eilmer_mesher.model.entities import LineSeg, SplineSeg
from eilmer_mesher.sketch.importers import build_curves, parse_point_file, transform_points


def naca00xx(t=0.12, n=33):
    """Upper and lower surfaces, LE -> TE, with cosine spacing (open trailing edge)."""
    xs = [0.5 * (1 - math.cos(math.pi * k / (n - 1))) for k in range(n)]
    yt = [5*t*(0.2969*math.sqrt(x) - 0.1260*x - 0.3516*x**2 + 0.2843*x**3 - 0.1015*x**4) for x in xs]
    return list(zip(xs, yt)), list(zip(xs, [-y for y in yt]))


def selig_text():
    upper, lower = naca00xx()
    rows = upper[::-1] + lower[1:]
    return "NACA 0012 AIRFOILS\n" + "\n".join(f"  {x:.7f}  {y:.7f}" for x, y in rows) + "\n"


def lednicer_text():
    upper, lower = naca00xx()
    out = ["NACA 0012 AIRFOILS", f"{len(upper)}.  {len(lower)}.", ""]
    out += [f" {x:.7f} {y:.7f}" for x, y in upper] + [""]
    out += [f" {x:.7f} {y:.7f}" for x, y in lower]
    return "\n".join(out) + "\n"


class TestImporters(unittest.TestCase):
    def test_selig_and_lednicer_agree(self):
        s = parse_point_file(selig_text())
        l = parse_point_file(lednicer_text())
        self.assertEqual((s.fmt, l.fmt), ("selig", "lednicer"))
        self.assertEqual(s.name, "NACA 0012 AIRFOILS")
        self.assertEqual(len(s.points), 2*33 - 1)
        np.testing.assert_allclose(s.points, l.points)

    def test_split_and_close(self):
        pf = parse_point_file(selig_text())
        curves = build_curves(pf)
        self.assertEqual([type(c) for c in curves], [SplineSeg, SplineSeg, LineSeg])
        upper, lower, te = curves
        self.assertEqual(upper.end(), (0.0, 0.0))          # leading edge is a shared corner
        self.assertEqual(lower.start(), (0.0, 0.0))
        self.assertEqual((te.p0, te.p1), (lower.end(), upper.start()))
        self.assertGreater(upper.points[5][1], 0)            # upper surface first in Selig order

    def test_no_split_closed_te(self):
        upper, lower = naca00xx()
        rows = upper[::-1] + lower[1:] + [upper[-1]]       # explicitly closed loop
        pf = parse_point_file("\n".join(f"{x},{y}" for x, y in rows), "loop")
        self.assertEqual(pf.fmt, "xy")
        curves = build_curves(pf, split_le=False)
        self.assertEqual(len(curves), 1)                     # already closed: no TE line

    def test_transform(self):
        (p,) = transform_points([(1.0, 0.0)], scale=2.0, angle_deg=90.0, dx=1.0, dy=0.0)
        np.testing.assert_allclose(p, (1.0, -2.0), atol=1e-12)   # nose up: trailing edge goes down

    def test_comments_and_blank_lines(self):
        pf = parse_point_file("# header comment\n\n0 0\n1 1 0.0\n2, 0 # trailing\n")
        self.assertEqual(pf.points, [(0, 0), (1, 1), (2, 0)])

    def test_empty(self):
        with self.assertRaises(ValueError):
            parse_point_file("just a name\n")


if __name__ == "__main__":
    unittest.main()
