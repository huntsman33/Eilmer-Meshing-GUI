import matplotlib.pyplot as plt
from matplotlib.collections import LineCollection

def plot_grid(vertices, x_lim=None, y_lim=None, filename=None, revision=None):
    niv = len(vertices)
    njv = len(vertices[0])
    fig, ax = plt.subplots()
    if x_lim: ax.set_xlim(x_lim)
    if y_lim: ax.set_ylim(y_lim)
    # Build lines running in j-direction
    lines = []
    for i in range(niv):
        pts = []
        for j in range(njv):
            p = vertices[i][j]
            pts.append((p.x, p.y))
        lines.append(pts)
    lc = LineCollection(lines, linewidths=0.4)
    ax.add_collection(lc)
    # Build lines running in i-direction
    lines = []
    for j in range(njv):
        pts = []
        for i in range(niv):
            p = vertices[i][j]
            pts.append((p.x, p.y))
        lines.append(pts)
    lc = LineCollection(lines, linewidths=0.4)
    ax.add_collection(lc)
    ax.set_aspect('equal')
    if revision:
        ax.annotate(f"[rev {revision}]", xy=(0.2,0.02), xycoords='figure fraction', annotation_clip=False)
    if filename:
        plt.savefig(filename, dpi=300)
    else:
        plt.show()

def write_grid_to_vtk(baseFileName, vertices):
    niv = len(vertices)
    njv = len(vertices[0])
    with open(baseFileName + ".vtk", "wt") as f:
        f.write("# vtk DataFile Version 2.0\n")
        f.write("\n")
        f.write("ASCII\n")
        f.write("\n")
        f.write("DATASET STRUCTURED_GRID\n")
        f.write("DIMENSIONS %d %d 1\n" % (niv, njv))
        f.write("POINTS %d float\n" % (niv*njv))
        for j in range(njv):
            for i in range(niv):
                vtx = vertices[i][j]
                f.write("%.18e %.18e %.18e\n" % (vtx.x, vtx.y, vtx.z))
    return
