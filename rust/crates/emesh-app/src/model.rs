//! Owner of the Document: undo/redo, selection, change tracking and every editing operation.
//!
//! Every edit is recorded as a before/after snapshot of the whole document (projects are small,
//! so this keeps undo simple and exact). Topology and block resolution are cached per revision.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use serde_json::{json, Map, Value};

use emesh_core::blocking::{self, Resolution};
use emesh_core::dimensions;
use emesh_core::document::{default_smoothing, BoundaryLayer, Cluster, Dimension, Document, Face};
use emesh_core::entities::{Curve, Geom, Kind};
use emesh_core::face::FaceEval;
use emesh_core::geom;
use emesh_core::layers::{self, LayerSettings};
use emesh_core::num::{py_dist, P};
use emesh_core::offset::offset_chains;
use emesh_core::operations::{lies_on, merge_curves, mirror_curve, reflect};
use emesh_core::poly::Polygon;
use emesh_core::pyfmt;
use emesh_core::spacing::mirror_cluster;
use emesh_core::subdivide;
use emesh_core::topology::Topology;
use emesh_mesh::preview::block_grid;
use emesh_mesh::quality::{cell_metrics, intentional_aspect};

pub type EditResult<T> = Result<T, String>;

#[derive(Clone)]
struct Snapshot {
    before: Value,
    after: Value,
    text: String,
}

/// Which subdivision to apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Subdivision {
    Nxm(usize, usize),
    OGrid(f64),
    OGridHole,
    Centroid,
}

/// Patch smoothing change for set_block_patch.
#[derive(Clone, Debug, PartialEq)]
pub enum Smoothing {
    Keep,
    Set(Option<Map<String, Value>>),
}

pub struct SketchModel {
    pub doc: Document,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    clean: Option<usize>,
    pub selection: BTreeSet<String>,
    /// new curves are dashed drawing aids
    pub reference_mode: bool,
    /// new curves are block split lines
    pub split_mode: bool,
    pub revision: u64,
    before: Option<Value>,
    /// an edit finished, or undo / redo / load: time to re-mesh (cleared by the app)
    pub committed: bool,
    topo: RefCell<Option<(u64, Rc<Topology>)>>,
    res: RefCell<Option<(u64, Rc<Resolution>)>>,
}

impl Default for SketchModel {
    fn default() -> Self {
        SketchModel::new(Document::new())
    }
}

impl SketchModel {
    pub fn new(doc: Document) -> SketchModel {
        SketchModel {
            doc,
            undo: vec![],
            redo: vec![],
            clean: Some(0),
            selection: BTreeSet::new(),
            reference_mode: false,
            split_mode: false,
            revision: 0,
            before: None,
            committed: true,
            topo: RefCell::new(None),
            res: RefCell::new(None),
        }
    }

    // --- editing -------------------------------------------------------------------------------
    pub fn new_kind(&self) -> Kind {
        if self.split_mode {
            Kind::Split
        } else if self.reference_mode {
            Kind::Reference
        } else {
            Kind::Geometry
        }
    }

    pub fn begin_edit(&mut self) {
        if self.before.is_none() {
            self.before = Some(self.doc.to_json());
        }
    }

    pub fn commit_edit(&mut self, text: &str) {
        let Some(before) = self.before.take() else { return };
        let after = self.doc.to_json();
        if after != before {
            if self.clean.is_some_and(|c| c > self.undo.len()) {
                self.clean = None; // the clean state was in the redo history
            }
            self.undo.push(Snapshot { before, after, text: text.into() });
            self.redo.clear();
        }
        self.notify();
        self.committed = true;
    }

    pub fn cancel_edit(&mut self) {
        if let Some(before) = self.before.take() {
            self.restore(&before);
        }
    }

    /// Run f as one undoable edit; on error the document is put back.
    pub fn edit<T>(&mut self, text: &str, f: impl FnOnce(&mut Document) -> EditResult<T>) -> EditResult<T> {
        self.begin_edit();
        match f(&mut self.doc) {
            Ok(v) => {
                self.commit_edit(text);
                Ok(v)
            }
            Err(e) => {
                self.cancel_edit();
                Err(e)
            }
        }
    }

    pub fn editing(&self) -> bool {
        self.before.is_some()
    }

    pub fn notify(&mut self) {
        self.revision += 1;
    }

    fn restore(&mut self, data: &Value) {
        self.doc = Document::from_json(data).expect("snapshot");
        self.prune_selection();
        self.notify();
        self.committed = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_text(&self) -> Option<&str> {
        self.undo.last().map(|s| s.text.as_str())
    }

    pub fn redo_text(&self) -> Option<&str> {
        self.redo.last().map(|s| s.text.as_str())
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            let before = s.before.clone();
            self.redo.push(s);
            self.restore(&before);
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            let after = s.after.clone();
            self.undo.push(s);
            self.restore(&after);
        }
    }

    pub fn is_clean(&self) -> bool {
        self.clean == Some(self.undo.len())
    }

    pub fn set_clean(&mut self) {
        self.clean = Some(self.undo.len());
    }

    pub fn entity_ids(&self) -> BTreeSet<String> {
        self.doc.curves.keys().chain(self.doc.points.keys()).chain(self.doc.dimensions.keys()).cloned().collect()
    }

    fn prune_selection(&mut self) {
        let alive = self.entity_ids();
        self.selection.retain(|i| alive.contains(i));
    }

    pub fn set_selection<I: IntoIterator<Item = String>>(&mut self, ids: I) {
        let alive = self.entity_ids();
        self.selection = ids.into_iter().filter(|i| alive.contains(i)).collect();
    }

    // --- common operations ---------------------------------------------------------------------
    /// Give curves fresh ids and add them in one undo step; returns their ids.
    pub fn add_curves(&mut self, curves: Vec<Curve>, text: &str) -> Vec<String> {
        self.edit(text, |doc| {
            let mut ids = vec![];
            for mut c in curves {
                c.id = doc.new_id("c");
                ids.push(c.id.clone());
                doc.add_curve(c)?;
            }
            Ok(ids)
        })
        .unwrap_or_default()
    }

    pub fn add_point(&mut self, p: P) -> String {
        self.edit("Point", |doc| {
            let pid = doc.new_id("pt");
            doc.add_point(&pid, p)?;
            Ok(pid)
        })
        .unwrap_or_default()
    }

    /// (curve id, "start"/"end") of curve ends at point, excluding curves in exclude.
    pub fn connected_ends(&self, point: P, exclude: &[String], tol: Option<f64>) -> Vec<(String, &'static str)> {
        let tol = tol.unwrap_or(1e-9 * 1.0f64.max(point[0].abs()).max(point[1].abs()));
        let mut out = vec![];
        for c in self.doc.curves.values() {
            if exclude.contains(&c.id) {
                continue;
            }
            for (which, q) in [("start", c.start()), ("end", c.end())] {
                if c.has_handle(which) && py_dist(q, point) <= tol {
                    out.push((c.id.clone(), which));
                }
            }
        }
        out
    }

    fn move_ends(doc: &mut Document, partners: &[(String, &str)], p: P) {
        for (cid, which) in partners {
            if let Some(moved) = doc.curves[cid].with_handle(which, p) {
                doc.curves.insert(cid.clone(), moved);
            }
        }
    }

    pub fn set_point(&mut self, pid: &str, p: P) {
        let pid = pid.to_string();
        let _ = self.edit("Edit point", |doc| {
            doc.points.insert(pid, p);
            Ok(())
        });
    }

    /// Keep the start, put the end at length along angle_deg; connected curves follow.
    pub fn set_line(&mut self, cid: &str, length: f64, angle_deg: f64) -> EditResult<()> {
        if length <= 0.0 {
            return Err("length must be positive".into());
        }
        let c = self.doc.curves[cid].clone();
        let (p0, old_end) = c.ends_lp();
        let a = angle_deg.to_radians();
        let end = [p0[0] + length * a.cos(), p0[1] + length * a.sin()];
        let partners = self.connected_ends(old_end, &[cid.to_string()], None);
        self.edit("Edit line", |doc| {
            doc.curves.insert(cid.to_string(), Curve { geom: Geom::Line { p0, p1: end }, ..c });
            Self::move_ends(doc, &partners, end);
            Ok(())
        })
    }

    /// Keep the centre and start direction; set radius and sweep (sign keeps the direction).
    pub fn set_arc(&mut self, cid: &str, radius: f64, sweep_deg: f64) -> EditResult<()> {
        if radius <= 0.0 || !(0.0 < sweep_deg.abs() && sweep_deg.abs() < 360.0) {
            return Err("radius must be positive and the sweep between 0 and 360 degrees".into());
        }
        let c = self.doc.curves[cid].clone();
        let (centre, _, a0, sweep) = geom::arc_geom(&c);
        let new_sweep = sweep_deg.abs().to_radians().copysign(sweep);
        let pieces = geom::arc_pieces(centre, radius, a0, new_sweep, None, None);
        let (old_start, old_end) = c.ends_lp();
        let (new_start, new_end) = (pieces[0].0, pieces[pieces.len() - 1].1);
        let at_start = self.connected_ends(old_start, &[cid.to_string()], None);
        let at_end = self.connected_ends(old_end, &[cid.to_string()], None);
        self.edit("Edit arc", |doc| {
            if pieces.len() > 1 {
                doc.remove_blocks_using(&[cid.to_string()]); // the face would now need several curves
            }
            let (p0, p1, cen) = pieces[0];
            doc.curves.insert(cid.to_string(), Curve { geom: Geom::Arc { p0, p1, centre: cen }, ..c.clone() });
            for &(p0, p1, cen) in &pieces[1..] {
                let id = doc.new_id("c");
                doc.add_curve(Curve { id, geom: Geom::Arc { p0, p1, centre: cen }, ..c.clone() })?;
            }
            Self::move_ends(doc, &at_start, new_start);
            Self::move_ends(doc, &at_end, new_end);
            Ok(())
        })
    }

    /// Move curves and points by (dx, dy); dimensions follow their curves.
    pub fn translate(&mut self, ids: &[String], dx: f64, dy: f64) {
        let _ = self.edit("Move", |doc| {
            for i in ids {
                if let Some(c) = doc.curves.get(i) {
                    let m = c.map_points(|p| [p[0] + dx, p[1] + dy]);
                    doc.curves.insert(i.clone(), m);
                } else if let Some(p) = doc.points.get_mut(i) {
                    *p = [p[0] + dx, p[1] + dy];
                }
            }
            Ok(())
        });
    }

    // --- dimensions -------------------------------------------------------------------------------
    pub fn dimension_kind(curve: &Curve) -> Option<&'static str> {
        match curve.geom {
            Geom::Line { .. } => Some("length"),
            Geom::Arc { .. } => Some("angle"),
            _ => None,
        }
    }

    /// "distance" (parallel) or "angle" for two lines, or None if they aren't two lines.
    pub fn two_line_kind(&self, cid: &str, cid2: &str) -> Option<&'static str> {
        let (a, b) = (self.doc.curves.get(cid)?, self.doc.curves.get(cid2)?);
        if cid == cid2 || !a.is_line() || !b.is_line() {
            return None;
        }
        dimensions::two_line_kind(a, b).ok()
    }

    /// Dimension a line (length) or arc (angle), or with cid2 the distance or angle between two
    /// lines. Re-placing an existing one moves it.
    pub fn add_dimension(&mut self, cid: &str, offset: f64, cid2: Option<&str>, sector: Option<[i32; 2]>) -> EditResult<String> {
        let (kind, sector) = if let Some(c2) = cid2 {
            let kind = self.two_line_kind(cid, c2).ok_or("a two-line dimension needs two different lines")?;
            let sector = if kind == "angle" {
                Some(sector.unwrap_or_else(|| dimensions::default_sector(&self.doc.curves[cid], &self.doc.curves[c2])))
            } else {
                None
            };
            (kind, sector)
        } else {
            let kind = Self::dimension_kind(&self.doc.curves[cid]).ok_or("only lines and arcs can be dimensioned")?;
            (kind, None)
        };
        let existing = self
            .doc
            .dimensions
            .values()
            .find(|d| d.curve == cid && d.curve2.as_deref() == cid2 && d.kind == kind)
            .map(|d| d.id.clone());
        self.edit("Dimension", |doc| {
            if let Some(did) = existing {
                let d = doc.dimensions.get_mut(&did).unwrap();
                d.offset = offset;
                d.sector = sector;
                return Ok(did);
            }
            let did = doc.new_id("dim");
            doc.add_dimension(Dimension {
                id: did.clone(),
                kind: kind.into(),
                curve: cid.into(),
                offset,
                curve2: cid2.map(|s| s.to_string()),
                sector,
            });
            Ok(did)
        })
    }

    /// Length or distance (metres), or sweep or two-line angle (degrees) the dimension shows.
    pub fn dimension_value(&self, did: &str) -> EditResult<f64> {
        let d = &self.doc.dimensions[did];
        if d.curve2.is_some() {
            return dimensions::value(&self.doc, d);
        }
        let c = &self.doc.curves[&d.curve];
        if d.kind == "length" {
            let (p0, p1) = c.ends_lp();
            return Ok(py_dist(p0, p1));
        }
        let (_, _, _, sweep) = geom::arc_geom(c);
        Ok(sweep.abs().to_degrees())
    }

    /// Drive the curve to a new length (metres) or sweep angle (degrees); a two-line dimension
    /// moves its second line.
    pub fn set_dimension_value(&mut self, did: &str, value: f64) -> EditResult<()> {
        let d = self.doc.dimensions[did].clone();
        if d.curve2.is_some() {
            return self.set_two_line_value(did, value);
        }
        let c = self.doc.curves[&d.curve].clone();
        if d.kind == "length" {
            let (p0, p1) = c.ends_lp();
            let angle = (p1[1] - p0[1]).atan2(p1[0] - p0[0]).to_degrees();
            self.set_line(&d.curve, value, angle)
        } else {
            let (_, r, _, _) = geom::arc_geom(&c);
            self.set_arc(&d.curve, r, value)
        }
    }

    pub fn set_two_line_value(&mut self, did: &str, value: f64) -> EditResult<()> {
        let d = self.doc.dimensions[did].clone();
        let c2 = d.curve2.clone().unwrap();
        let old = self.doc.curves[&c2].clone();
        let new = dimensions::driven_line(&self.doc, &d, value)?;
        let (o0, o1) = old.ends_lp();
        let (n0, n1) = new.ends_lp();
        let moves: Vec<(Vec<(String, &'static str)>, P)> = [(o0, n0), (o1, n1)]
            .into_iter()
            .filter(|(p, q)| p != q)
            .map(|(p, q)| (self.connected_ends(p, std::slice::from_ref(&c2), None), q))
            .collect();
        self.edit("Edit dimension", |doc| {
            doc.curves.insert(c2.clone(), new);
            for (partners, q) in &moves {
                Self::move_ends(doc, partners, *q);
            }
            Ok(())
        })
    }

    // --- boundary conditions --------------------------------------------------------------------
    /// Create or change a BC tag (renaming old_tag updates every curve using it).
    pub fn define_bc(&mut self, tag: &str, cls: &str, args: &str, old_tag: Option<&str>) -> EditResult<()> {
        emesh_lua::bcs::check_tag(tag)?;
        if Some(tag) != old_tag && self.doc.bcs.contains_key(tag) {
            return Err(format!("the tag '{tag}' already exists"));
        }
        self.edit("Boundary condition", |doc| {
            if let Some(old) = old_tag {
                if old != tag {
                    doc.bcs.shift_remove(old);
                    for c in doc.curves.values_mut() {
                        if c.bc_tag == old {
                            c.bc_tag = tag.into();
                        }
                    }
                }
            }
            doc.bcs.insert(tag.into(), emesh_core::document::Bc { class: cls.into(), args: args.trim().into() });
            Ok(())
        })
    }

    /// Tag solid curves ('' = untagged, Eilmer's slip wall). Returns how many were tagged.
    pub fn set_bc(&mut self, curve_ids: &[String], tag: &str) -> EditResult<usize> {
        if !tag.is_empty() && !self.doc.bcs.contains_key(tag) {
            return Err(format!("no boundary condition called '{tag}'"));
        }
        let ids: Vec<String> = curve_ids
            .iter()
            .filter(|i| self.doc.curves.get(*i).is_some_and(|c| c.kind == Kind::Geometry))
            .cloned()
            .collect();
        if !ids.is_empty() {
            self.edit("Set boundary condition", |doc| {
                for i in &ids {
                    doc.curves.get_mut(i).unwrap().bc_tag = tag.into();
                }
                Ok(())
            })?;
        }
        Ok(ids.len())
    }

    /// Remove a tag; its curves go back to the default slip wall.
    pub fn delete_bc(&mut self, tag: &str) {
        let _ = self.edit("Delete boundary condition", |doc| {
            doc.bcs.shift_remove(tag);
            for c in doc.curves.values_mut() {
                if c.bc_tag == tag {
                    c.bc_tag.clear();
                }
            }
            Ok(())
        });
    }

    /// tag -> number of curves using it.
    pub fn bc_usage(&self) -> Vec<(String, usize)> {
        self.doc
            .bcs
            .keys()
            .map(|t| (t.clone(), self.doc.curves.values().filter(|c| &c.bc_tag == t).count()))
            .collect()
    }

    // --- blocks ------------------------------------------------------------------------------------
    /// The sketch's regions, cached until the next change.
    pub fn topology(&self) -> Rc<Topology> {
        let mut cache = self.topo.borrow_mut();
        if let Some((rev, t)) = cache.as_ref() {
            if *rev == self.revision {
                return t.clone();
            }
        }
        let t = Rc::new(Topology::new(&self.doc));
        *cache = Some((self.revision, t.clone()));
        t
    }

    /// Every block resolved against the topology, cached like topology().
    pub fn resolution(&self) -> Rc<Resolution> {
        {
            let cache = self.res.borrow();
            if let Some((rev, r)) = cache.as_ref() {
                if *rev == self.revision {
                    return r.clone();
                }
            }
        }
        let topo = self.topology();
        let r = Rc::new(blocking::resolve(&self.doc, &topo));
        *self.res.borrow_mut() = Some((self.revision, r.clone()));
        r
    }

    /// Id of the block whose region contains point.
    pub fn block_at(&self, point: P, res: &Resolution) -> Option<String> {
        res.polygons.iter().find(|(_, poly)| poly.len() >= 3 && Polygon::new(poly).contains(point)).map(|(b, _)| b.clone())
    }

    /// Add a block over a region; counts default to square-ish cells, or follow neighbours.
    fn new_region_block(doc: &mut Document, topo: &Topology, ri: usize, corners: Option<&[usize]>) -> EditResult<String> {
        let region = &topo.regions[ri];
        let spec = blocking::region_spec(topo, region, corners).map_err(|e| e.0)?;
        let mut block = emesh_core::Block::new(&doc.new_id("blk"));
        block.region = Some(spec);
        let (concrete, _) = blocking::resolve_block(topo, &block).map_err(|e| e.0)?;
        let (niv, njv) = blocking::default_counts(doc, &concrete, region.area);
        block.niv = niv;
        block.njv = njv;
        let bid = block.id.clone();
        doc.add_block(block);
        let res = blocking::resolve(doc, topo);
        for axis in ['i', 'j'] {
            let group = res.groups.get(&(bid.clone(), axis)).cloned();
            let neighbour = doc.blocks.iter().find(|b| {
                b.id != bid && ['i', 'j'].iter().any(|ax| res.groups.get(&(b.id.clone(), *ax)).cloned() == group)
            });
            if let Some(n) = neighbour {
                let ax = ['i', 'j'].into_iter().find(|ax| res.groups.get(&(n.id.clone(), *ax)).cloned() == group).unwrap();
                let v = if ax == 'i' { n.niv } else { n.njv };
                let b = doc.block_mut(&bid).unwrap();
                if axis == 'i' {
                    b.niv = v;
                } else {
                    b.njv = v;
                }
            }
        }
        Ok(bid)
    }

    /// Block the region containing point; returns the block id.
    pub fn make_block(&mut self, point: P, corners: Option<Vec<usize>>) -> EditResult<String> {
        let topo = self.topology();
        let ri = topo.region_index_at(point).ok_or("there is no closed region here")?;
        let res = self.resolution();
        if self.block_at(point, &res).is_some() {
            return Err("this region already has a block".into());
        }
        self.edit("Make block", |doc| Self::new_region_block(doc, &topo, ri, corners.as_deref()))
    }

    /// Replace broken blocks and block every free region that can be blocked: (made, skipped).
    pub fn make_all_blocks(&mut self) -> (Vec<String>, usize) {
        let topo = self.topology();
        let res = self.resolution();
        self.edit("Block all regions", |doc| {
            doc.blocks.retain(|b| !res.errors.contains_key(&b.id));
            let taken: Vec<&Vec<P>> = res.polygons.iter().filter(|(b, _)| !res.errors.contains_key(*b)).map(|(_, p)| p).collect();
            let (mut made, mut skipped) = (vec![], 0);
            for ri in 0..topo.regions.len() {
                let probe = Polygon::new(&topo.regions[ri].polygon).representative_point();
                if taken.iter().any(|t| Polygon::new(t).contains(probe)) {
                    continue;
                }
                match Self::new_region_block(doc, &topo, ri, None) {
                    Ok(b) => made.push(b),
                    Err(_) => skipped += 1,
                }
            }
            Ok((made, skipped))
        })
        .unwrap_or_default()
    }

    pub fn delete_block(&mut self, bid: &str) {
        let _ = self.edit("Delete block", |doc| {
            doc.blocks.retain(|b| b.id != bid);
            Ok(())
        });
    }

    /// Re-pick a region block's corners at the four loop vertices nearest the given points.
    pub fn set_block_corners(&mut self, bid: &str, points: &[P]) -> EditResult<()> {
        let topo = self.topology();
        let block = self.doc.block(bid).ok_or("no such block")?.clone();
        let spec = block.region.as_ref().ok_or("the block's region no longer exists")?;
        let (ri, _) = blocking::find_region(&topo, spec).ok_or("the block's region no longer exists")?;
        let verts = blocking::loop_vertices(&topo, &topo.regions[ri].loop_);
        let corners: Vec<usize> = points
            .iter()
            .map(|p| {
                let mut best = 0;
                for k in 1..verts.len() {
                    if py_dist(verts[k], *p) < py_dist(verts[best], *p) {
                        best = k;
                    }
                }
                best
            })
            .collect();
        let spec = blocking::region_spec(&topo, &topo.regions[ri], Some(&corners)).map_err(|e| e.0)?;
        self.edit("Set block corners", |doc| {
            let b = doc.block_mut(bid).unwrap();
            b.region = Some(spec);
            b.cluster.clear();
            Ok(())
        })
    }

    /// Cells along one block face; the count applies to every linked face, the clustering to this
    /// face, optionally its opposite, and the same face in neighbouring blocks (mirrored where
    /// they run the other way).
    pub fn set_edge_cells(&mut self, bid: &str, face: Face, cells: usize, cluster: Option<Cluster>, opposite: bool) -> EditResult<()> {
        if cells < 1 {
            return Err("an edge needs at least one cell".into());
        }
        let res = self.resolution();
        let group = res.groups[&(bid.to_string(), face.axis())].clone();
        let mut targets = vec![face];
        if opposite {
            targets.push(face.opposite());
        }
        let blk = res.block(bid).ok_or("no such block")?.clone();
        let starts: Vec<P> = targets.iter().map(|f| FaceEval::new(&self.doc, blk.side(*f)).at(0.0)).collect();
        self.edit("Edge cells", |doc| {
            for b in doc.blocks.iter_mut() {
                if res.groups.get(&(b.id.clone(), 'i')) == Some(&group) {
                    b.niv = cells + 1;
                }
                if res.groups.get(&(b.id.clone(), 'j')) == Some(&group) {
                    b.njv = cells + 1;
                }
            }
            for (f, start) in targets.iter().zip(&starts) {
                set_cluster(doc.block_mut(bid).unwrap(), *f, cluster.as_ref());
                for (other, of) in blocking::linked_faces(&res, bid, *f) {
                    let o = res.block(&other).unwrap();
                    let s0 = FaceEval::new(&*doc, o.side(of)).at(0.0);
                    let same = py_dist(s0, *start) < 1e-9 * 1.0f64.max(start[0].abs()).max(start[1].abs());
                    let spec = if same { cluster.clone() } else { cluster.as_ref().map(mirror_cluster) };
                    if let Some(b) = doc.block_mut(&other) {
                        set_cluster(b, of, spec.as_ref());
                    }
                }
            }
            Ok(())
        })
    }

    /// Split the region at point with split curves, then block the new regions.
    pub fn subdivide(&mut self, point: P, kind: Subdivision) -> EditResult<Vec<String>> {
        let topo = self.topology();
        let ri = topo.region_index_at(point).ok_or("there is no closed region here")?;
        let region = &topo.regions[ri];
        let res = self.resolution();
        let old_block = self.block_at(point, &res);
        let curves = match kind {
            Subdivision::Nxm(n, m) => subdivide::split_nxm(&topo, region, n, m, None),
            Subdivision::OGrid(size) => subdivide::ogrid(&topo, region, size, None),
            Subdivision::OGridHole => subdivide::ogrid_hole(&topo, region),
            Subdivision::Centroid => subdivide::centroid_split(&topo, region),
        }
        .map_err(|e| e.0)?;
        let shell = region.polygon.clone();
        let holes: Vec<Vec<P>> = region.hole_polygons.clone();
        self.edit("Sub-divide", |doc| {
            if let Some(ob) = &old_block {
                doc.blocks.retain(|b| &b.id != ob);
            }
            for mut c in curves {
                c.id = doc.new_id("c");
                doc.add_curve(c)?;
            }
            let outline = Polygon::with_holes(&shell, holes.iter().map(|h| h.as_slice()).collect());
            let new_topo = Topology::new(doc);
            let mut made = vec![];
            for k in 0..new_topo.regions.len() {
                let r = &new_topo.regions[k];
                if !r.holes.is_empty() || !outline.contains(Polygon::new(&r.polygon).representative_point()) {
                    continue;
                }
                if let Ok(b) = Self::new_region_block(doc, &new_topo, k, None) {
                    made.push(b);
                }
            }
            Ok(made)
        })
    }

    // --- patches and smoothing (refinement) ---------------------------------------------------------
    pub fn set_block_patch(&mut self, bids: &[String], patch: Option<(&str, Map<String, Value>)>, smoothing: Smoothing) {
        let _ = self.edit("Block patch", |doc| {
            for b in doc.blocks.iter_mut() {
                if !bids.contains(&b.id) {
                    continue;
                }
                if let Some((p, opts)) = &patch {
                    b.patch = p.to_string();
                    b.patch_opts = opts.clone();
                }
                if let Smoothing::Set(s) = &smoothing {
                    b.smoothing = s.clone().filter(|m| !m.is_empty());
                }
            }
            Ok(())
        });
    }

    /// The Refine button: Laplace-smooth the given blocks, in one undo step. Where the (exact)
    /// preview shows smoothing would fold cells the block gets AOPatch instead. Boundary-layer
    /// blocks are skipped. Returns (smoothed, AOPatch, skipped).
    pub fn refine_blocks(&mut self, bids: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
        let res = self.resolution();
        let layered = intentional_aspect(&res.blocks);
        let md = self.doc.with_blocks(res.blocks.clone());
        let (mut smooth, mut ao, mut skipped) = (vec![], vec![], vec![]);
        for bid in bids {
            let Some(b) = res.block(bid) else {
                skipped.push(bid.clone());
                continue;
            };
            if layered.contains(bid) {
                skipped.push(bid.clone());
                continue;
            }
            let mut trial = b.clone();
            trial.smoothing = Some(default_smoothing());
            let count = |blk: &emesh_core::Block| cell_metrics(&block_grid(&md, blk)).inverted.iter().filter(|v| **v).count();
            if count(&trial) > count(b) {
                ao.push(bid.clone());
            } else {
                smooth.push(bid.clone());
            }
        }
        if !smooth.is_empty() || !ao.is_empty() {
            let _ = self.edit("Refine", |doc| {
                for b in doc.blocks.iter_mut() {
                    if smooth.contains(&b.id) {
                        b.smoothing = Some(default_smoothing());
                    } else if ao.contains(&b.id) {
                        b.patch = "AOPatch".into();
                        b.patch_opts = json!({"nx": 10, "ny": 10}).as_object().unwrap().clone();
                        b.smoothing = None;
                    }
                }
                Ok(())
            });
        }
        (smooth, ao, skipped)
    }

    // --- boundary layers ----------------------------------------------------------------------------
    /// Boundary layer on block walls [(block id, face)]; returns the layer id.
    pub fn add_boundary_layer(&mut self, faces: &[(String, Face)], first: f64, growth: f64, cells: usize, transition: bool) -> EditResult<String> {
        let res = self.resolution();
        for (bid, face) in faces {
            if layers::layer_of_face(&self.doc, &res, bid, *face).is_some() {
                return Err(format!(
                    "the {} face of {bid} already has a boundary layer (double-click it to change the settings)",
                    face.as_str()
                ));
            }
        }
        let walls: Vec<_> = faces.iter().map(|(bid, f)| layers::wall_spec(&res, bid, *f)).collect();
        self.edit("Boundary layer", |doc| {
            let lid = doc.new_id("bl");
            doc.layers.push(BoundaryLayer::new(&lid, walls, first, growth, cells, transition));
            let li = doc.layers.len() - 1;
            layers::apply(doc, li).map_err(|e| e.0)?;
            Ok(lid)
        })
    }

    pub fn layer(&self, lid: &str) -> Option<&BoundaryLayer> {
        self.doc.layers.iter().find(|l| l.id == lid)
    }

    pub fn layer_at_face(&self, bid: &str, face: Face) -> Option<String> {
        let res = self.resolution();
        layers::layer_of_face(&self.doc, &res, bid, face).map(|i| self.doc.layers[i].id.clone())
    }

    /// New settings for a layer: it is cut again, re-meshing its blocks.
    pub fn edit_boundary_layer(&mut self, lid: &str, settings: LayerSettings) -> EditResult<()> {
        self.edit("Boundary layer settings", |doc| layers::redo_layers(doc, lid, settings, false).map_err(|e| e.0))
    }

    pub fn remove_boundary_layer(&mut self, lid: &str) -> EditResult<()> {
        self.edit("Remove boundary layer", |doc| layers::redo_layers(doc, lid, LayerSettings::default(), true).map_err(|e| e.0))
    }

    /// A sentence on how well the layer's last cells meet the outer cells, or ''.
    pub fn layer_note(&self, lid: &str) -> String {
        let Some(layer) = self.layer(lid) else { return String::new() };
        let (ratio, inner, outer) = size_jump(&self.doc, &self.resolution(), layer);
        if ratio <= 2.0 * layer.growth.max(1.1) {
            return String::new();
        }
        let more = (ratio.ln() / layer.growth.max(1.05).ln()).ceil() as i64;
        format!(
            "The outer cells ({} m) are {}x the last layer cell ({} m): about {more} more layer cells at this \
             growth ratio would meet them smoothly, or tick 'grade the outer blocks' in the layer settings.",
            pyfmt::g(outer, 3),
            pyfmt::f(ratio, 0),
            pyfmt::g(inner, 3)
        )
    }

    // --- merge, mirror, offset ---------------------------------------------------------------------
    /// Join connected collinear lines, concentric arcs and pieces of one spline:
    /// (surviving ids, number of joins).
    pub fn merge(&mut self, ids: &[String]) -> (Vec<String>, usize) {
        let curves: Vec<Curve> = self.doc.curves.values().filter(|c| ids.contains(&c.id)).cloned().collect();
        let (result, removed) = merge_curves(&curves);
        if removed.is_empty() {
            return (curves.iter().map(|c| c.id.clone()).collect(), 0);
        }
        let all: Vec<String> = curves.iter().map(|c| c.id.clone()).collect();
        let _ = self.edit("Merge", |doc| {
            doc.remove_blocks_using(&all);
            doc.remove_curves(&removed);
            for c in &result {
                doc.curves.insert(c.id.clone(), c.clone());
            }
            Ok(())
        });
        let survivors: Vec<String> = result.iter().map(|c| c.id.clone()).collect();
        self.set_selection(survivors.clone());
        (survivors, removed.len())
    }

    /// Mirrored copies of curves and points across the line through a and b; returns new ids.
    pub fn mirror(&mut self, ids: &[String], a: P, b: P) -> Vec<String> {
        let tol = 1e-9 * [1.0, a[0].abs(), a[1].abs(), b[0].abs(), b[1].abs()].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        self.edit("Mirror", |doc| {
            let mut new_ids = vec![];
            for i in ids {
                if let Some(c) = doc.curves.get(i).cloned() {
                    if lies_on(&c, a, b, tol) {
                        continue; // its mirror image is itself
                    }
                    let mut m = mirror_curve(&c, a, b);
                    m.id = doc.new_id("c");
                    new_ids.push(m.id.clone());
                    doc.add_curve(m)?;
                } else if let Some(p) = doc.points.get(i).copied() {
                    let q = reflect(p, a, b);
                    if py_dist(q, p) > tol {
                        let pid = doc.new_id("pt");
                        doc.add_point(&pid, q)?;
                        new_ids.push(pid);
                    }
                }
            }
            Ok(new_ids)
        })
        .unwrap_or_default()
    }

    /// New curves offset from the given ones by distance (metres), in one undo step.
    pub fn offset(&mut self, ids: &[String], distance: f64, reverse: bool, source_to_reference: bool) -> EditResult<Vec<String>> {
        let curves: Vec<Curve> = self.doc.curves.values().filter(|c| ids.contains(&c.id)).cloned().collect();
        let new = offset_chains(&curves, distance, reverse)?;
        self.edit("Offset", |doc| {
            if source_to_reference {
                for c in &curves {
                    let src = doc.curves.get_mut(&c.id).unwrap();
                    src.kind = Kind::Reference;
                    src.bc_tag.clear();
                }
            }
            let mut out = vec![];
            for mut c in new {
                c.id = doc.new_id("c");
                out.push(c.id.clone());
                doc.add_curve(c)?;
            }
            Ok(out)
        })
    }

    /// Quick trim: delete the piece of curve cid between parameters ta and tb.
    pub fn trim(&mut self, cid: &str, ta: f64, tb: f64) -> EditResult<()> {
        let r = self.edit("Trim", |doc| {
            let c = doc.curves[cid].clone();
            let mut keep = vec![];
            if ta > 0.0 {
                keep.push(geom::subcurve(&c, 0.0, ta)?);
            }
            if tb < 1.0 {
                keep.push(geom::subcurve(&c, tb, 1.0)?);
            }
            doc.remove_blocks_using(&[cid.to_string()]);
            if keep.is_empty() {
                doc.remove_curves(&[cid.to_string()]);
                return Ok(());
            }
            let mut first = keep.remove(0);
            first.id = cid.to_string();
            doc.curves.insert(cid.to_string(), first); // the first piece keeps the id
            if let Some(mut second) = keep.pop() {
                second.id = doc.new_id("c");
                doc.add_curve(second)?;
            }
            Ok(())
        });
        self.prune_selection();
        r
    }

    pub fn delete_selection(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        let ids: Vec<String> = self.selection.iter().cloned().collect();
        let _ = self.edit("Delete", |doc| {
            doc.remove_curves(&ids);
            Ok(())
        });
        self.prune_selection();
    }

    /// Selected (or given) curves: reference (dashed drawing aid) <-> solid.
    pub fn toggle_reference(&mut self, ids: Option<&[String]>) {
        self.toggle_kind(ids, Kind::Reference, "Toggle reference");
    }

    /// Selected (or given) curves: block split line (dash-dot) <-> solid.
    pub fn toggle_split(&mut self, ids: Option<&[String]>) {
        self.toggle_kind(ids, Kind::Split, "Toggle split line");
    }

    fn toggle_kind(&mut self, ids: Option<&[String]>, kind: Kind, text: &str) {
        let ids: Vec<String> = match ids {
            Some(i) => i.to_vec(),
            None => self.selection.iter().cloned().collect(),
        }
        .into_iter()
        .filter(|i| self.doc.curves.contains_key(i))
        .collect();
        if ids.is_empty() {
            return;
        }
        let _ = self.edit(text, |doc| {
            for cid in &ids {
                let c = doc.curves.get_mut(cid).unwrap();
                c.kind = if c.kind == kind { Kind::Geometry } else { kind };
                if c.kind != Kind::Geometry {
                    c.bc_tag.clear(); // only solid curves carry boundary conditions
                }
            }
            Ok(())
        });
    }

    // --- files -----------------------------------------------------------------------------------
    pub fn reset(&mut self, doc: Document) {
        self.doc = doc;
        self.undo.clear();
        self.redo.clear();
        self.clean = Some(0);
        self.selection.clear();
        self.before = None;
        self.notify();
        self.committed = true;
    }
}

fn set_cluster(block: &mut emesh_core::Block, face: Face, spec: Option<&Cluster>) {
    match spec {
        Some(s) if !s.0.is_empty() => {
            block.cluster.insert(face, s.clone());
        }
        _ => {
            block.cluster.shift_remove(&face);
        }
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// (ratio, last layer cell, outer cell) for the worst place where a layer's cut meets the block
/// outside it; (1, 0, 0) if there is nothing to compare.
pub fn size_jump(doc: &Document, res: &Resolution, layer: &BoundaryLayer) -> (f64, f64, f64) {
    let blocks: Vec<&emesh_core::Block> = res.blocks.iter().filter(|b| layer.blocks.contains(&b.id)).collect();
    let md = doc.with_blocks(blocks.iter().map(|b| (*b).clone()).collect());
    let mut sizes: indexmap::IndexMap<String, Vec<f64>> = indexmap::IndexMap::new();
    for b in &blocks {
        let hits: Vec<Face> = emesh_core::FACES
            .into_iter()
            .filter(|f| b.side(*f).len() == 1 && layer.curves.contains(&b.side(*f)[0].id))
            .collect();
        if hits.is_empty() {
            continue;
        }
        let g = block_grid(&md, b);
        for f in hits {
            let (ni, nj) = (g.niv, g.njv);
            let pairs: Vec<(P, P)> = match f {
                Face::South => (0..ni).map(|i| (g.at(i, 0), g.at(i, 1))).collect(),
                Face::North => (0..ni).map(|i| (g.at(i, nj - 1), g.at(i, nj - 2))).collect(),
                Face::West => (0..nj).map(|j| (g.at(0, j), g.at(1, j))).collect(),
                Face::East => (0..nj).map(|j| (g.at(ni - 1, j), g.at(ni - 2, j))).collect(),
            };
            let d: Vec<f64> = pairs.iter().map(|(e, i)| (i[0] - e[0]).hypot(i[1] - e[1])).collect();
            sizes.entry(b.side(f)[0].id.clone()).or_default().push(median(d));
        }
    }
    let mut worst = (1.0, 0.0, 0.0);
    for pair in sizes.values() {
        if pair.len() == 2 {
            let (lo, hi) = (pair[0].min(pair[1]), pair[0].max(pair[1]));
            if lo > 0.0 && hi / lo > worst.0 {
                worst = (hi / lo, lo, hi);
            }
        }
    }
    worst
}
