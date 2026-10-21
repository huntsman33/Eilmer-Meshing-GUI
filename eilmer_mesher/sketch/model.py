"""
Qt-side owner of the Document: undo/redo, selection and change notification.

Every edit is recorded as a before/after snapshot of the whole document. Sketch
documents are small, so this keeps undo simple and exact.
"""
import math
from contextlib import contextmanager

from PySide6.QtCore import QObject, Signal
from PySide6.QtGui import QUndoCommand, QUndoStack

from ..model.document import Dimension, Document
from ..model.entities import GEOMETRY, REFERENCE, SPLIT


class SnapshotCommand(QUndoCommand):
    def __init__(self, model, before, after, text):
        super().__init__(text)
        self.model = model
        self.before = before
        self.after = after
        self._applied = True      # the edit has already been made when the command is pushed

    def redo(self):
        if self._applied:
            self._applied = False
            return
        self.model._restore(self.after)

    def undo(self):
        self.model._restore(self.before)


class SketchModel(QObject):
    changed = Signal()             # geometry changed (also every frame of a drag)
    committed = Signal()           # an edit finished, or undo / redo / load: time to re-mesh
    selectionChanged = Signal()

    def __init__(self, doc=None, parent=None):
        super().__init__(parent)
        self.doc = doc or Document()
        self.undo_stack = QUndoStack(self)
        self.selection = set()
        self.reference_mode = False    # new curves are dashed drawing aids
        self.split_mode = False        # new curves are block split lines
        self.revision = 0
        self._before = None

    # --- editing -----------------------------------------------------------
    def new_kind(self):
        if self.split_mode:
            return SPLIT
        return REFERENCE if self.reference_mode else GEOMETRY

    def begin_edit(self):
        if self._before is None:
            self._before = self.doc.to_dict()

    def commit_edit(self, text):
        before, self._before = self._before, None
        if before is None:
            return
        after = self.doc.to_dict()
        if after != before:
            self.undo_stack.push(SnapshotCommand(self, before, after, text))
        self.notify()
        self.committed.emit()

    def cancel_edit(self):
        before, self._before = self._before, None
        if before is not None:
            self._restore(before)

    @contextmanager
    def edit(self, text):
        self.begin_edit()
        try:
            yield self.doc
        except Exception:
            self.cancel_edit()
            raise
        self.commit_edit(text)

    def notify(self):
        self.revision += 1
        self.changed.emit()

    def _restore(self, data):
        self.doc = Document.from_dict(data)
        self._prune_selection()
        self.notify()
        self.committed.emit()

    def entity_ids(self):
        return set(self.doc.curves) | set(self.doc.points) | set(self.doc.dimensions)

    def _prune_selection(self):
        alive = self.selection & self.entity_ids()
        if alive != self.selection:
            self.selection = alive
            self.selectionChanged.emit()

    # --- common operations ------------------------------------------------
    def add_curves(self, curves, text):
        """Give curves fresh ids and add them in one undo step; returns their ids."""
        ids = []
        with self.edit(text) as doc:
            for c in curves:
                c.id = doc.new_id("c")
                doc.add_curve(c)
                ids.append(c.id)
        return ids

    def add_point(self, p, text="Point"):
        with self.edit(text) as doc:
            pid = doc.new_id("pt")
            doc.add_point(pid, p)
        return pid

    def connected_ends(self, point, exclude=(), tol=None):
        """(curve id, "start"/"end") of curve ends at point, excluding curves in exclude."""
        if tol is None:
            tol = 1e-9 * max(1.0, abs(point[0]), abs(point[1]))
        out = []
        for c in self.doc.curves.values():
            if c.id in exclude:
                continue
            for which, q in (("start", c.start()), ("end", c.end())):
                if which in dict(c.handles()) and math.dist(q, point) <= tol:
                    out.append((c.id, which))
        return out

    # --- dimension edits (double-click dialogs) -------------------------------------
    @staticmethod
    def _move_ends(doc, partners, new_point):
        """Move the listed (curve id, "start"/"end") curve ends to new_point."""
        for cid, which in partners:
            moved = doc.curves[cid].with_handle(which, new_point)
            if moved is not None:
                doc.curves[cid] = moved

    def set_point(self, pid, p):
        with self.edit("Edit point") as doc:
            doc.points[pid] = (float(p[0]), float(p[1]))

    def set_line(self, cid, length, angle_deg):
        """Keep the start, put the end at length along angle_deg; connected curves follow."""
        if length <= 0:
            raise ValueError("length must be positive")
        c = self.doc.curves[cid]
        a = math.radians(angle_deg)
        end = (c.p0[0] + length*math.cos(a), c.p0[1] + length*math.sin(a))
        old_end = c.p1
        partners = self.connected_ends(old_end, exclude={cid})
        with self.edit("Edit line") as doc:
            doc.curves[cid] = c.copy(p1=end)
            self._move_ends(doc, partners, end)

    def set_arc(self, cid, radius, sweep_deg):
        """Keep the centre and start direction; set radius and sweep (sign keeps the direction)."""
        from ..model import geom
        if radius <= 0 or not 0 < abs(sweep_deg) < 360:
            raise ValueError("radius must be positive and the sweep between 0 and 360 degrees")
        c = self.doc.curves[cid]
        centre, _, a0, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
        new_sweep = math.copysign(math.radians(abs(sweep_deg)), sweep)
        pieces = geom.arc_pieces(centre, radius, a0, new_sweep)
        old_start, old_end = c.p0, c.p1
        new_start, new_end = pieces[0][0], pieces[-1][1]
        at_start = self.connected_ends(old_start, {cid})
        at_end = self.connected_ends(old_end, {cid})
        with self.edit("Edit arc") as doc:
            if len(pieces) > 1:            # the face would now need several curves
                doc.remove_blocks_using([cid])
            first = pieces[0]
            doc.curves[cid] = c.copy(p0=first[0], p1=first[1], centre=first[2])
            for p0, p1, cen in pieces[1:]:
                doc.add_curve(c.copy(id=doc.new_id("c"), p0=p0, p1=p1, centre=cen))
            self._move_ends(doc, at_start, new_start)
            self._move_ends(doc, at_end, new_end)

    def translate(self, ids, dx, dy, text="Move"):
        """Move curves and points by (dx, dy); dimensions follow their curves."""
        with self.edit(text) as doc:
            for i in ids:
                if i in doc.curves:
                    doc.curves[i] = doc.curves[i].map_points(lambda p: (p[0] + dx, p[1] + dy))
                elif i in doc.points:
                    x, y = doc.points[i]
                    doc.points[i] = (x + dx, y + dy)

    # --- dimensions ------------------------------------------------------------------------
    @staticmethod
    def dimension_kind(curve):
        return {"line": "length", "arc": "angle"}.get(curve.type)

    def two_line_kind(self, cid, cid2):
        """"distance" (parallel) or "angle" for two lines, or None if they aren't two lines."""
        from ..model import dimensions
        a, b = self.doc.curves.get(cid), self.doc.curves.get(cid2)
        if cid == cid2 or a is None or b is None or a.type != "line" or b.type != "line":
            return None
        return dimensions.two_line_kind(a, b)

    def add_dimension(self, cid, offset, cid2=None, sector=None):
        """
        Dimension a line (length) or arc (angle), or with cid2 the distance or angle
        between two lines (sector picks the angle; see model/dimensions.py).
        Re-placing an existing one moves it.
        """
        if cid2 is not None:
            kind = self.two_line_kind(cid, cid2)
            if kind is None:
                raise ValueError("a two-line dimension needs two different lines")
            if kind == "angle" and sector is None:
                from ..model import dimensions
                sector = dimensions.default_sector(self.doc.curves[cid], self.doc.curves[cid2])
            sector = [int(s) for s in sector] if kind == "angle" else None
        else:
            kind = self.dimension_kind(self.doc.curves[cid])
            if kind is None:
                raise ValueError("only lines and arcs can be dimensioned")
        existing = next((d for d in self.doc.dimensions.values()
                         if d.curve == cid and d.curve2 == cid2 and d.kind == kind), None)
        with self.edit("Dimension") as doc:
            if existing is not None:
                existing.offset = float(offset)
                existing.sector = sector
                return existing.id
            did = doc.new_id("dim")
            doc.add_dimension(Dimension(did, kind, cid, float(offset), cid2, sector))
        return did

    def dimension_value(self, did):
        """Length or distance (metres), or sweep or two-line angle (degrees) the dimension shows."""
        from ..model import dimensions, geom
        d = self.doc.dimensions[did]
        if d.curve2 is not None:
            return dimensions.value(self.doc, d)
        c = self.doc.curves[d.curve]
        if d.kind == "length":
            return math.dist(c.p0, c.p1)
        _, _, _, sweep = geom.arc_geometry(c.p0, c.p1, c.centre)
        return math.degrees(abs(sweep))

    def set_dimension_value(self, did, value):
        """Drive the curve to a new length (metres) or sweep angle (degrees). A two-line
        dimension moves its second line (distance in metres, angle in degrees)."""
        from ..model import geom
        d = self.doc.dimensions[did]
        if d.curve2 is not None:
            self.set_two_line_value(did, value)
            return
        c = self.doc.curves[d.curve]
        if d.kind == "length":
            angle = math.degrees(math.atan2(c.p1[1] - c.p0[1], c.p1[0] - c.p0[0]))
            self.set_line(d.curve, value, angle)
        else:
            _, r, _, _ = geom.arc_geometry(c.p0, c.p1, c.centre)
            self.set_arc(d.curve, r, value)

    def set_two_line_value(self, did, value):
        """Move a two-line dimension's second line: parallel to itself to a new distance
        (same side), or turned about the vertex to a new angle. Connected ends follow."""
        from ..model import dimensions
        d = self.doc.dimensions[did]
        old = self.doc.curves[d.curve2]
        new = dimensions.driven_line(self.doc, d, value)
        moves = [(self.connected_ends(p, exclude={d.curve2}), q)
                 for p, q in ((old.p0, new.p0), (old.p1, new.p1)) if p != q]
        with self.edit("Edit dimension") as doc:
            doc.curves[d.curve2] = new
            for partners, q in moves:
                self._move_ends(doc, partners, q)

    # --- boundary conditions -------------------------------------------------------------
    def define_bc(self, tag, cls, args="", old_tag=None):
        """Create or change a BC tag (renaming old_tag updates every curve using it)."""
        from ..model.bcs import check_tag
        check_tag(tag)
        if tag != old_tag and tag in self.doc.bcs:
            raise ValueError(f"the tag '{tag}' already exists")
        with self.edit("Boundary condition") as doc:
            if old_tag is not None and old_tag != tag:
                doc.bcs.pop(old_tag, None)
                for c in doc.curves.values():
                    if c.bc_tag == old_tag:
                        c.bc_tag = tag
            doc.bcs[tag] = {"class": cls, "args": args.strip()}

    def set_bc(self, curve_ids, tag):
        """Tag solid curves ('' = untagged, Eilmer's slip wall). Returns how many were tagged."""
        if tag and tag not in self.doc.bcs:
            raise ValueError(f"no boundary condition called '{tag}'")
        ids = [i for i in curve_ids if i in self.doc.curves and self.doc.curves[i].kind == GEOMETRY]
        if ids:
            with self.edit("Set boundary condition") as doc:
                for i in ids:
                    doc.curves[i].bc_tag = tag
        return len(ids)

    def delete_bc(self, tag):
        """Remove a tag; its curves go back to the default slip wall."""
        with self.edit("Delete boundary condition") as doc:
            doc.bcs.pop(tag, None)
            for c in doc.curves.values():
                if c.bc_tag == tag:
                    c.bc_tag = ""

    def bc_usage(self):
        """tag -> number of curves using it."""
        out = {t: 0 for t in self.doc.bcs}
        for c in self.doc.curves.values():
            if c.bc_tag in out:
                out[c.bc_tag] += 1
        return out

    # --- blocks ---------------------------------------------------------------------------
    def topology(self):
        """The sketch's regions, cached until the next change (it takes a while on big sketches)."""
        from ..model.topology import Topology
        cached = getattr(self, "_topology", None)
        if cached is None or cached[0] != self.revision or cached[1] is not self.doc:
            cached = (self.revision, self.doc, Topology(self.doc))
            self._topology = cached
        return cached[2]

    def resolution(self, topo=None):
        """Every block resolved against the topology, cached like topology()."""
        from ..model import blocking
        if topo is not None and topo is not self.topology():
            return blocking.resolve(self.doc, topo)
        cached = getattr(self, "_resolution", None)
        if cached is None or cached[0] != self.revision or cached[1] is not self.doc:
            cached = (self.revision, self.doc, blocking.resolve(self.doc, self.topology()))
            self._resolution = cached
        return cached[2]

    def block_at(self, point, topo=None, res=None):
        """Id of the block whose region contains point, or None."""
        from shapely.geometry import Point, Polygon
        res = res or self.resolution(topo)
        for bid, poly in res.polygons.items():
            if len(poly) >= 3 and Polygon(poly).contains(Point(point)):
                return bid
        return None

    def _new_region_block(self, doc, topo, region, corners=None):
        """Add a block over a region; counts default to square-ish cells, or follow neighbours."""
        from ..model import blocking
        from ..model.document import Block, FACES
        spec = blocking.region_spec(topo, region, corners)
        block = Block(doc.new_id("blk"), {f: [] for f in FACES}, region=spec)
        concrete, _ = blocking.resolve_block(topo, block)
        block.niv, block.njv = blocking.default_counts(doc, concrete, region.area)
        doc.add_block(block)
        res = blocking.resolve(doc, topo)
        for axis, attr in (("i", "niv"), ("j", "njv")):
            group = res.groups.get((block.id, axis))
            others = [b for b in doc.blocks if b.id != block.id
                      for ax in "ij" if res.groups.get((b.id, ax)) == group]
            if others:                                   # a neighbour already fixes this count
                neighbour = others[0]
                ax = next(a for a in "ij" if res.groups.get((neighbour.id, a)) == group)
                setattr(block, attr, neighbour.niv if ax == "i" else neighbour.njv)
        return block.id

    def make_block(self, point, corners=None):
        """Block the region containing point; returns the block id (BlockError explains failures)."""
        from ..model.document import BlockError
        topo = self.topology()
        region = topo.region_at(point)
        if region is None:
            raise BlockError("there is no closed region here")
        if self.block_at(point, topo) is not None:
            raise BlockError("this region already has a block")
        with self.edit("Make block") as doc:
            return self._new_region_block(doc, topo, region, corners)

    def make_all_blocks(self):
        """Replace broken blocks and block every free region that can be blocked. Returns (made, skipped)."""
        from ..model.document import BlockError
        from shapely.geometry import Polygon
        topo = self.topology()
        res = self.resolution(topo)
        made, skipped = [], []
        with self.edit("Block all regions") as doc:
            doc.blocks = [b for b in doc.blocks if b.id not in res.errors]
            taken = [Polygon(p) for bid, p in res.polygons.items() if bid not in res.errors]
            for region in topo.regions:
                probe = Polygon(region.polygon).representative_point()
                if any(t.contains(probe) for t in taken):
                    continue
                try:
                    made.append(self._new_region_block(doc, topo, region))
                except BlockError:
                    skipped.append(region)
        return made, skipped

    def delete_block(self, bid):
        with self.edit("Delete block") as doc:
            doc.blocks = [b for b in doc.blocks if b.id != bid]

    def set_block_corners(self, bid, points):
        """Re-pick a region block's corners at the four loop vertices nearest the given points."""
        from ..model import blocking
        from ..model.document import BlockError
        topo = self.topology()
        block = next(b for b in self.doc.blocks if b.id == bid)
        region, _ = blocking.find_region(topo, block.region)
        if region is None:
            raise BlockError("the block's region no longer exists")
        verts = blocking.loop_vertices(topo, region.loop)
        corners = [min(range(len(verts)), key=lambda k: math.dist(verts[k], p)) for p in points]
        spec = blocking.region_spec(topo, region, corners)
        with self.edit("Set block corners") as doc:
            b = next(b for b in doc.blocks if b.id == bid)
            b.region = spec
            b.cluster = {}

    def set_edge_cells(self, bid, face, cells, cluster=None, opposite=True):
        """
        Cells along one block face. The count applies to every face linked to it (the
        opposite face, and faces shared with other blocks, transitively). The clustering
        applies to this face, optionally its opposite face, and the same face in
        neighbouring blocks (mirrored where they run the other way).
        """
        from ..mesh.preview import FaceEval
        from ..model import blocking
        if cells < 1:
            raise ValueError("an edge needs at least one cell")
        res = self.resolution()
        blocks = res.by_id()
        group = res.groups[(bid, blocking.AXIS[face])]
        targets = [face] + ([blocking.OPPOSITE[face]] if opposite else [])
        start = {f: FaceEval(self.doc, blocks[bid].sides[f])([0.0])[0] for f in targets}
        with self.edit("Edge cells") as doc:
            for b in doc.blocks:
                if res.groups.get((b.id, "i")) == group:
                    b.niv = cells + 1
                if res.groups.get((b.id, "j")) == group:
                    b.njv = cells + 1
            by_id = {b.id: b for b in doc.blocks}
            for f in targets:
                _set_cluster(by_id[bid], f, cluster)
                for other, of in blocking.linked_faces(res, bid, f):
                    same = math.dist(FaceEval(doc, other.sides[of])([0.0])[0], start[f]) < 1e-9 * \
                        max(1.0, abs(start[f][0]), abs(start[f][1]))
                    _set_cluster(by_id[other.id], of, cluster if same else _mirror_cluster(cluster))

    def subdivide(self, point, kind, **opts):
        """Split the region at point with reference curves, then block the new regions. Returns block ids."""
        from shapely.geometry import Polygon
        from ..model import subdivide
        from ..model.document import BlockError
        topo = self.topology()
        region = topo.region_at(point)
        if region is None:
            raise BlockError("there is no closed region here")
        old_block = self.block_at(point, topo)
        if kind == "nxm":
            curves = subdivide.split_nxm(topo, region, opts.get("n", 2), opts.get("m", 2))
        elif kind == "ogrid":
            curves = subdivide.ogrid(topo, region, opts.get("size", 0.5))
        elif kind == "ogrid_hole":
            curves = subdivide.ogrid_hole(topo, region)
        elif kind == "centroid":
            curves = subdivide.centroid_split(topo, region)
        else:
            raise ValueError(kind)
        outline = Polygon(region.polygon, [h for h in region.hole_polygons if len(h) >= 3])
        made = []
        with self.edit("Sub-divide") as doc:
            if old_block is not None:
                doc.blocks = [b for b in doc.blocks if b.id != old_block]
            for c in curves:
                c.id = doc.new_id("c")
                doc.add_curve(c)
            from ..model.topology import Topology
            new_topo = Topology(doc)
            for r in new_topo.regions:
                if r.holes or not outline.contains(Polygon(r.polygon).representative_point()):
                    continue
                try:
                    made.append(self._new_region_block(doc, new_topo, r))
                except BlockError:
                    pass
        return made

    # --- patches and smoothing (refinement) -------------------------------------------------
    def set_block_patch(self, bids, patch=None, patch_opts=None, smoothing="keep"):
        """Patch type ("CoonsPatch" / "AOPatch", with options) and/or Laplace smoothing
        (a dict, or None for none) for blocks, in one undo step."""
        bids = set(bids)
        with self.edit("Block patch") as doc:
            for b in doc.blocks:
                if b.id not in bids:
                    continue
                if patch is not None:
                    b.patch = patch
                    b.patch_opts = dict(patch_opts or {})
                if smoothing != "keep":
                    b.smoothing = dict(smoothing) if smoothing else None

    def refine_blocks(self, bids):
        """
        The Refine button: Laplace-smooth the given blocks, in one undo step. Where the
        (exact) preview shows smoothing would fold cells, as it can next to a concave
        side, the block gets Eilmer's AOPatch instead. Blocks with boundary-layer
        spacing are left alone: smoothing would pull their cells off the wall.
        Returns (smoothed ids, AOPatch ids, skipped ids).
        """
        from ..mesh.preview import block_grid
        from ..mesh.quality import cell_metrics, intentional_aspect
        from ..model.document import Block, DEFAULT_SMOOTHING
        res = self.resolution()
        concrete = res.by_id()
        layered = intentional_aspect(res.blocks)
        md = self.doc.with_blocks(res.blocks)
        smooth, ao, skipped = [], [], []
        for bid in bids:
            if bid in layered or bid not in concrete:
                skipped.append(bid)
                continue
            trial = Block(**{**concrete[bid].__dict__, "smoothing": dict(DEFAULT_SMOOTHING)})
            before = cell_metrics(block_grid(md, concrete[bid]))["inverted"].sum()
            after = cell_metrics(block_grid(md, trial))["inverted"].sum()
            (ao if after > before else smooth).append(bid)
        if smooth or ao:
            with self.edit("Refine") as doc:
                for b in doc.blocks:
                    if b.id in smooth:
                        b.smoothing = dict(DEFAULT_SMOOTHING)
                    elif b.id in ao:
                        b.patch, b.patch_opts, b.smoothing = "AOPatch", {"nx": 10, "ny": 10}, None
        return smooth, ao, skipped

    # --- boundary layers ------------------------------------------------------------------
    def add_boundary_layer(self, faces, first, growth, cells, transition=False):
        """
        Boundary layer on block walls [(block id, face)]: the blocks are cut and re-blocked
        with a thin, wall-clustered block along each wall. Returns the layer id.
        """
        from ..model import layers
        from ..model.document import BlockError, BoundaryLayer
        res = self.resolution()
        for bid, face in faces:
            if layers.layer_of_face(self.doc, res, bid, face) is not None:
                raise BlockError(f"the {face} face of {bid} already has a boundary layer "
                                 "(double-click it to change the settings)")
        walls = [layers.wall_spec(res, bid, face) for bid, face in faces]
        with self.edit("Boundary layer") as doc:
            layer = BoundaryLayer(doc.new_id("bl"), walls, float(first), float(growth), int(cells), bool(transition))
            doc.layers.append(layer)
            layers.apply(doc, layer)
        return layer.id

    def layer(self, lid):
        return next(layer for layer in self.doc.layers if layer.id == lid)

    def layer_at_face(self, bid, face):
        from ..model import layers
        layer = layers.layer_of_face(self.doc, self.resolution(), bid, face)
        return layer.id if layer is not None else None

    def edit_boundary_layer(self, lid, **settings):
        """New first cell / growth / cells / transition for a layer: it is cut again, re-meshing its blocks."""
        from ..model import layers
        with self.edit("Boundary layer settings") as doc:
            layers.redo_layers(doc, lid, settings)

    def layer_note(self, lid):
        """A sentence on how well the layer's last cells meet the outer cells, or ''."""
        from ..model import layers
        layer = self.layer(lid)
        ratio, inner, outer = layers.size_jump(self.doc, self.resolution(), layer)
        if ratio <= 2.0 * max(layer.growth, 1.1):
            return ""
        more = math.ceil(math.log(ratio) / math.log(max(layer.growth, 1.05)))
        return (f"The outer cells ({outer:.3g} m) are {ratio:.0f}x the last layer cell ({inner:.3g} m): "
                f"about {more} more layer cells at this growth ratio would meet them smoothly, or tick "
                "'grade the outer blocks' in the layer settings.")

    def remove_boundary_layer(self, lid):
        from ..model import layers
        with self.edit("Remove boundary layer") as doc:
            layers.redo_layers(doc, lid, remove=True)

    # --- merge and mirror --------------------------------------------------------------
    def merge(self, ids):
        """Join connected collinear lines, concentric arcs and pieces of one spline.
        Returns (surviving ids, number of joins)."""
        from ..model.operations import merge_curves
        ids = set(ids)
        curves = [c for cid, c in self.doc.curves.items() if cid in ids]   # document order
        result, removed = merge_curves(curves)
        if not removed:
            return [c.id for c in curves], 0
        with self.edit("Merge") as doc:
            doc.remove_blocks_using([c.id for c in curves])
            doc.remove_curves(removed)
            for c in result:
                doc.curves[c.id] = c
        survivors = [c.id for c in result]
        self.set_selection(survivors)
        return survivors, len(removed)

    def mirror(self, ids, a, b):
        """Mirrored copies of curves and points across the line through a and b; returns new ids."""
        from ..model.operations import lies_on, mirror_curve, reflect
        tol = 1e-9 * max(1.0, abs(a[0]), abs(a[1]), abs(b[0]), abs(b[1]))
        new_ids = []
        with self.edit("Mirror") as doc:
            for i in ids:
                if i in doc.curves:
                    c = doc.curves[i]
                    if lies_on(c, a, b, tol):
                        continue                  # its mirror image is itself
                    m = mirror_curve(c, a, b).copy(id=doc.new_id("c"))
                    doc.add_curve(m)
                    new_ids.append(m.id)
                elif i in doc.points:
                    q = reflect(doc.points[i], a, b)
                    if math.dist(q, doc.points[i]) > tol:
                        new_ids.append(doc.add_point(doc.new_id("pt"), q))
        return new_ids

    def offset(self, ids, distance, reverse=False, source_to_reference=False):
        """
        New curves offset from the given ones by distance (metres), in one undo step;
        the originals stay. Connected curves are offset as chains (closed ones grow,
        or shrink if reverse). With source_to_reference the picked curves also become
        reference (dashed) curves with their BC tag cleared, in the same undo step.
        Returns the new ids; ValueError explains a failure (nothing changes).
        """
        from ..model.offset import offset_chains
        ids = set(ids)
        curves = [c for cid, c in self.doc.curves.items() if cid in ids]   # document order
        new = offset_chains(curves, float(distance), reverse)
        new_ids = []
        with self.edit("Offset") as doc:
            if source_to_reference:
                for c in curves:
                    src = doc.curves[c.id]
                    src.kind = REFERENCE
                    src.bc_tag = ""
            for c in new:
                c.id = doc.new_id("c")
                doc.add_curve(c)
                new_ids.append(c.id)
        return new_ids

    def delete_selection(self):
        if not self.selection:
            return
        with self.edit("Delete") as doc:
            doc.remove_curves(self.selection)
        self._prune_selection()

    def toggle_reference(self, ids=None):
        """Selected curves: reference (dashed drawing aid) <-> solid."""
        self._toggle_kind(ids, REFERENCE, "Toggle reference")

    def toggle_split(self, ids=None):
        """Selected curves: block split line (dash-dot) <-> solid."""
        self._toggle_kind(ids, SPLIT, "Toggle split line")

    def _toggle_kind(self, ids, kind, text):
        ids = set(ids if ids is not None else self.selection) & set(self.doc.curves)
        if not ids:
            return
        with self.edit(text) as doc:
            for cid in ids:
                c = doc.curves[cid]
                c.kind = GEOMETRY if c.kind == kind else kind
                if c.kind != GEOMETRY:
                    c.bc_tag = ""                 # only solid curves carry boundary conditions

    def set_selection(self, ids):
        ids = set(ids) & self.entity_ids()
        if ids != self.selection:
            self.selection = ids
            self.selectionChanged.emit()

    # --- files ---------------------------------------------------------------
    def reset(self, doc):
        self.doc = doc
        self.undo_stack.clear()
        self.selection = set()
        self._before = None
        self.selectionChanged.emit()
        self.notify()
        self.committed.emit()


def _set_cluster(block, face, spec):
    if spec:
        block.cluster[face] = dict(spec)
    else:
        block.cluster.pop(face, None)


def _mirror_cluster(spec):
    """The same clustering seen from the other end of the edge."""
    from ..model.layers import mirror_cluster
    return mirror_cluster(spec)
