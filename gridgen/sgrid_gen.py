from .vector3 import Vector3
from .path import Line, Arc, Bezier
from .gridutils import plot_grid

def coons_point(north, east, south, west, r, s):

    p00 = south(0.0)
    p10 = south(1.0)
    p01 = north(0.0)
    p11 = north(1.0)

    trm0 = west(s)*(1 - r) + east(s)*r
    trm1 = south(r)*(1 - s) + north(r)*s
    trm2 = p00*(1 - r)*(1 - s) + p10*r*(1 - s) + p01*(1 - r)*s + p11*r*s

    return trm0 + trm1 - trm2

def gridpts(north, east, south, west, niv, njv):

    pts = []
    dr = 1.0/(niv-1)
    ds = 1.0/(njv-1)

    for i in range(niv):
        pts.append([])
        for j in range(njv):
            r = i * dr
            s = j * ds
            p = coons_point(north, east, south, west, r, s)
            pts[i].append(p)

    return pts

if __name__ == "__main__":

    p00 = Vector3(0.0, 0.0)
    p10 = Vector3(1.0, 0.0)
    p11 = Vector3(2.0, 1.0)
    p01 = Vector3(0.0, 2.0)

    pb1 = Vector3(0.7, 2.5)
    pb2 = Vector3(0.7, 1.0)
    pa1 = Vector3(2.0, 0.0)

    south = Line(p00, p10)
    north = Bezier([p01, pb1, p11])
    west = Bezier([p00, pb2, p01])
    east = Arc(p10, p11, pa1)

    # r = 0.5; s = 0.5
    # p = coons_point(north, east, south, west, r, s)

    # print(p)
    # # plot_grid([south, north, west, east])
    pts = gridpts(north, east, south, west, 10, 25)
    plot_grid(pts, (-1,3), (-1,3))