//! Blocks made from sketch regions.
//!
//! A region block stores the cycle of curve ids around its region (counter-clockwise) and four
//! corner positions in that cycle. Each time the sketch changes it is re-resolved against the
//! current topology into concrete faces (CurveRefs, which may cover only part of a curve), so
//! dragging geometry re-meshes it.
//!
//! Faces are then linked: opposite faces of a block share a vertex count (niv for south/north,
//! njv for east/west), and a face shared with another block must have the same count, so Eilmer
//! can join them (codegen connects faces shared along split lines).

use std::collections::{BTreeSet, HashMap};

use indexmap::IndexMap;

use crate::document::{block_err, Block, BlockError, CurveRef, Document, Face, RegionSpec, FACES};
use crate::entities::Kind;
use crate::face::FaceEval;
use crate::geom;
use crate::num::{linspace, py_sum, round_key, P};
use crate::topology::{Region, Topology};

/// a loop vertex turning less than this is not a corner
pub const CORNER_MIN_DEG: f64 = 15.0;

#[derive(Clone, Debug)]
pub struct Group {
    pub curve: String,
    pub fwd: bool,
    pub edges: Vec<usize>,
}

/// A region boundary as runs of one curve: consecutive topology edges along the same curve, in
/// the same direction, are merged (a curve has no corner part-way along it).
#[derive(Clone)]
pub struct Loop<'a> {
    pub topo: &'a Topology,
    pub groups: Vec<Group>,
}

fn continues(topo: &Topology, group: &Group, e: usize, fwd: bool) -> bool {
    let edge = &topo.edges[e];
    let last = &topo.edges[*group.edges.last().unwrap()];
    if edge.curve_id != group.curve || fwd != group.fwd {
        return false;
    }
    if fwd {
        (last.tb - edge.ta).abs() < 1e-12
    } else {
        (last.ta - edge.tb).abs() < 1e-12
    }
}

impl<'a> Loop<'a> {
    pub fn new(topo: &'a Topology, edges: &[(usize, bool)]) -> Loop<'a> {
        let mut groups: Vec<Group> = vec![];
        for &(e, fwd) in edges {
            if let Some(last) = groups.last_mut() {
                if continues(topo, last, e, fwd) {
                    last.edges.push(e);
                    continue;
                }
            }
            groups.push(Group { curve: topo.edges[e].curve_id.clone(), fwd, edges: vec![e] });
        }
        if groups.len() > 1 {
            let (e0, f0) = (groups[0].edges[0], groups[0].fwd);
            if continues(topo, groups.last().unwrap(), e0, f0) {
                let last = groups.pop().unwrap();
                let mut merged = last.edges;
                merged.extend_from_slice(&groups[0].edges);
                groups[0].edges = merged;
            }
        }
        Loop { topo, groups }
    }

    pub fn len(&self) -> usize {
        self.groups.len()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn signature(&self) -> Vec<String> {
        self.groups.iter().map(|g| g.curve.clone()).collect()
    }

    pub fn rotated(&self, k: usize) -> Loop<'a> {
        let mut g = self.groups[k..].to_vec();
        g.extend_from_slice(&self.groups[..k]);
        Loop { topo: self.topo, groups: g }
    }

    pub fn ref_(&self, k: usize) -> CurveRef {
        let g = &self.groups[k];
        let first = &self.topo.edges[g.edges[0]];
        let last = &self.topo.edges[*g.edges.last().unwrap()];
        let (mut ta, mut tb) = if g.fwd { (first.ta, last.tb) } else { (last.ta, first.tb) };
        if ta <= 1e-12 {
            ta = 0.0;
        }
        if tb >= 1.0 - 1e-12 {
            tb = 1.0;
        }
        CurveRef::new(&g.curve, !g.fwd, ta, tb)
    }

    /// Refs for groups start .. stop - 1, wrapping round the loop.
    pub fn refs(&self, start: usize, stop: usize) -> Vec<CurveRef> {
        let n = self.groups.len();
        let mut count = (stop + n - start % n) % n;
        if count == 0 {
            count = n;
        }
        (0..count).map(|k| self.ref_((start + k) % n)).collect()
    }

    pub fn vertex(&self, k: usize) -> P {
        let g = &self.groups[k];
        let edge = &self.topo.edges[g.edges[0]];
        self.topo.vertices[if g.fwd { edge.v0 } else { edge.v1 }]
    }

    pub fn vertices(&self) -> Vec<P> {
        (0..self.groups.len()).map(|k| self.vertex(k)).collect()
    }

    fn point(&self, e: usize, fwd: bool, frac: f64) -> P {
        let edge = &self.topo.edges[e];
        let t = if fwd { edge.ta + frac * (edge.tb - edge.ta) } else { edge.tb - frac * (edge.tb - edge.ta) };
        geom::point_at(self.topo.curve(&edge.curve_id), t)
    }

    /// Turning angle (degrees) at the start of each run.
    pub fn turning_angles(&self) -> Vec<f64> {
        let n = self.groups.len();
        (0..n)
            .map(|k| {
                let g = &self.groups[k];
                let prev = &self.groups[(k + n - 1) % n];
                let v = self.vertex(k);
                let q = self.point(*prev.edges.last().unwrap(), prev.fwd, 1.0 - 1e-4);
                let din = [v[0] - q[0], v[1] - q[1]];
                let q = self.point(g.edges[0], g.fwd, 1e-4);
                let dout = [q[0] - v[0], q[1] - v[1]];
                (din[0] * dout[1] - din[1] * dout[0]).atan2(din[0] * dout[0] + din[1] * dout[1]).to_degrees()
            })
            .collect()
    }
}

pub fn loop_vertices(topo: &Topology, edges: &[(usize, bool)]) -> Vec<P> {
    Loop::new(topo, edges).vertices()
}

/// Run starts that turn by at least CORNER_MIN_DEG.
pub fn corner_candidates(topo: &Topology, edges: &[(usize, bool)]) -> Vec<usize> {
    Loop::new(topo, edges)
        .turning_angles()
        .iter()
        .enumerate()
        .filter(|(_, a)| a.abs() >= CORNER_MIN_DEG)
        .map(|(k, _)| k)
        .collect()
}

/// Four run positions for the block corners: the sharpest turns. None if impossible.
pub fn auto_corners(topo: &Topology, edges: &[(usize, bool)]) -> Option<Vec<usize>> {
    let lp = Loop::new(topo, edges);
    let n = lp.len();
    if n < 4 {
        return None;
    }
    if n == 4 {
        return Some(vec![0, 1, 2, 3]);
    }
    let angles = lp.turning_angles();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| (-angles[a].abs()).partial_cmp(&(-angles[b].abs())).unwrap());
    let mut c = order[..4].to_vec();
    c.sort();
    Some(c)
}

/// Rotate so corner 0 is the bottom-left-most corner (the south face runs from it).
fn rotate_to_south_west(lp: &Loop, corners: &[usize]) -> Vec<usize> {
    let verts = lp.vertices();
    let mut first = 0;
    for k in 1..4 {
        let v = verts[corners[k]];
        let b = verts[corners[first]];
        if v[0] + v[1] < b[0] + b[1] {
            first = k;
        }
    }
    let mut out = corners[first..].to_vec();
    out.extend_from_slice(&corners[..first]);
    out
}

// --- region blocks ------------------------------------------------------------------------------

/// The stored description of a block over a region (corners are run positions), or BlockError.
pub fn region_spec(topo: &Topology, region: &Region, corners: Option<&[usize]>) -> Result<RegionSpec, BlockError> {
    if !region.holes.is_empty() {
        return block_err("the region has a hole; split it first (Sub-divide: O-grid around hole)");
    }
    let lp = Loop::new(topo, &region.loop_);
    let corners: Vec<usize> = match corners {
        Some(c) => c.to_vec(),
        None => match auto_corners(topo, &region.loop_) {
            Some(c) => c,
            None => {
                return block_err(format!(
                    "the region has only {} sides; a block needs 4 corners (Sub-divide: centroid split)",
                    lp.len()
                ))
            }
        },
    };
    let distinct: BTreeSet<usize> = corners.iter().copied().collect();
    if distinct.len() != 4 || corners.len() != 4 {
        return block_err("a block needs four different corners");
    }
    let mut sorted = corners.clone();
    sorted.sort();
    let corners = rotate_to_south_west(&lp, &sorted);
    let k0 = corners[0];
    let n = lp.len();
    Ok(RegionSpec { loop_ids: lp.rotated(k0).signature(), corners: corners.iter().map(|c| (c + n - k0) % n).collect() })
}

/// (region index, its Loop rotated to match the spec) for a block spec.
pub fn find_region<'a>(topo: &'a Topology, spec: &RegionSpec) -> Option<(usize, Loop<'a>)> {
    let want = &spec.loop_ids;
    let n = want.len();
    for (ri, region) in topo.regions.iter().enumerate() {
        if !region.holes.is_empty() {
            continue;
        }
        let lp = Loop::new(topo, &region.loop_);
        if lp.len() != n {
            continue;
        }
        let sig = lp.signature();
        for k in 0..n {
            if (0..n).all(|i| sig[(k + i) % n] == want[i]) {
                return Some((ri, lp.rotated(k)));
            }
        }
    }
    None
}

pub fn reverse_refs(refs: &[CurveRef]) -> Vec<CurveRef> {
    refs.iter().rev().map(|r| CurveRef::new(&r.id, !r.reversed, r.t0, r.t1)).collect()
}

/// Copy of a region block with concrete sides (and its region index), or BlockError.
pub fn resolve_block(topo: &Topology, block: &Block) -> Result<(Block, usize), BlockError> {
    let spec = block.region.as_ref().ok_or_else(|| BlockError("not a region block".into()))?;
    let (ri, lp) = match find_region(topo, spec) {
        Some(x) => x,
        None => return block_err("its region no longer exists (the sketch around it changed)"),
    };
    let mut c = spec.corners.clone();
    c.sort();
    let faces: Vec<Vec<CurveRef>> = (0..4).map(|k| lp.refs(c[k], c[(k + 1) % 4])).collect();
    let mut out = block.clone();
    out.sides[Face::South.index()] = faces[0].clone();
    out.sides[Face::East.index()] = faces[1].clone();
    out.sides[Face::North.index()] = reverse_refs(&faces[2]);
    out.sides[Face::West.index()] = reverse_refs(&faces[3]);
    Ok((out, ri))
}

pub fn face_length(doc: &Document, refs: &[CurveRef]) -> f64 {
    let ls: Vec<f64> = refs
        .iter()
        .map(|r| {
            let c = &doc.curves[&r.id];
            if r.full() {
                geom::path_length(c)
            } else {
                geom::subcurve(c, r.t0, r.t1).map(|s| geom::path_length(&s)).unwrap_or(0.0)
            }
        })
        .collect();
    py_sum(&ls)
}

/// Vertex counts giving roughly square cells, about 10 across the block.
pub fn default_counts(doc: &Document, block: &Block, region_area: f64) -> (usize, usize) {
    let h = (if region_area > 1e-300 { region_area } else { 1e-300 }).sqrt() / 10.0;
    let i_len = 0.5 * (face_length(doc, block.side(Face::South)) + face_length(doc, block.side(Face::North)));
    let j_len = 0.5 * (face_length(doc, block.side(Face::East)) + face_length(doc, block.side(Face::West)));
    let count = |l: f64| ((l / h).round_ties_even() as i64 + 1).max(2) as usize;
    (count(i_len), count(j_len))
}

// --- resolution of all blocks -------------------------------------------------------------------

pub type GroupId = (String, char);

#[derive(Clone, Debug, Default)]
pub struct Resolution {
    /// concrete, valid blocks (for Lua and preview)
    pub blocks: Vec<Block>,
    /// block id -> reason it cannot be meshed
    pub errors: IndexMap<String, String>,
    /// block id -> [messages]
    pub warnings: IndexMap<String, Vec<String>>,
    /// block id -> outline for display
    pub polygons: IndexMap<String, Vec<P>>,
    /// (block id, axis) -> group id
    pub groups: IndexMap<GroupId, GroupId>,
}

impl Resolution {
    pub fn by_id(&self) -> HashMap<&str, &Block> {
        self.blocks.iter().map(|b| (b.id.as_str(), b)).collect()
    }

    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }
}

pub type FaceKey = BTreeSet<(String, u64, u64)>;

pub fn face_key(refs: &[CurveRef]) -> FaceKey {
    refs.iter()
        .map(|r| (r.id.clone(), round_key(r.t0.min(r.t1), 9), round_key(r.t0.max(r.t1), 9)))
        .collect()
}

pub fn outline(doc: &Document, block: &Block) -> Vec<P> {
    let t = linspace(0.0, 1.0, 24);
    let rt: Vec<f64> = t.iter().rev().copied().collect();
    let mut pts = vec![];
    for (face, rev) in [(Face::South, false), (Face::East, false), (Face::North, true), (Face::West, true)] {
        let p = FaceEval::new(doc, block.side(face)).eval(if rev { &rt } else { &t });
        pts.extend_from_slice(&p[..p.len() - 1]);
    }
    pts
}

struct UnionFind {
    parent: HashMap<GroupId, GroupId>,
}

impl UnionFind {
    fn find(&mut self, a: &GroupId) -> GroupId {
        let mut a = a.clone();
        self.parent.entry(a.clone()).or_insert_with(|| a.clone());
        loop {
            let p = self.parent[&a].clone();
            if p == a {
                return a;
            }
            let gp = self.parent[&p].clone();
            self.parent.insert(a.clone(), gp.clone());
            a = gp;
        }
    }
}

/// Resolve every block; harmonise counts across shared faces; collect problems.
pub fn resolve(doc: &Document, topo: &Topology) -> Resolution {
    let mut res = Resolution::default();
    for b in &doc.blocks {
        let r: Result<Block, BlockError> = (|| {
            let concrete = if b.region.is_some() {
                let (concrete, ri) = resolve_block(topo, b)?;
                res.polygons.insert(b.id.clone(), topo.regions[ri].polygon.clone());
                concrete
            } else {
                let concrete = b.clone();
                doc.validate_block(&concrete)?;
                res.polygons.insert(b.id.clone(), outline(doc, &concrete));
                concrete
            };
            doc.validate_block(&concrete)?;
            Ok(concrete)
        })();
        match r {
            Ok(c) => res.blocks.push(c),
            Err(e) => {
                res.errors.insert(b.id.clone(), e.0);
            }
        }
    }

    // Link faces: opposite faces share an axis; identical faces of two blocks share it too.
    let mut uf = UnionFind { parent: HashMap::new() };
    let mut faces_by_key: IndexMap<FaceKey, Vec<(usize, Face)>> = IndexMap::new();
    for (bi, b) in res.blocks.iter().enumerate() {
        for f in FACES {
            faces_by_key.entry(face_key(b.side(f))).or_default().push((bi, f));
        }
    }
    for shared in faces_by_key.values() {
        for w in shared.windows(2) {
            let ((b0, f0), (b1, f1)) = (w[0], w[1]);
            let (id0, id1) = (res.blocks[b0].id.clone(), res.blocks[b1].id.clone());
            let r1 = uf.find(&(id1.clone(), f1.axis()));
            let r0 = uf.find(&(id0.clone(), f0.axis()));
            uf.parent.insert(r0, r1);
            if res.blocks[b0].side(f0).iter().any(|r| doc.curves[&r.id].kind == Kind::Geometry) {
                res.warnings.entry(id0).or_default().push(format!(
                    "its {} face is a solid curve shared with block {}, so it is a wall between them \
                     (make it a block split line, X, to let the flow through)",
                    f0.as_str(),
                    id1
                ));
            }
        }
    }
    for b in &res.blocks {
        for axis in ['i', 'j'] {
            let key = (b.id.clone(), axis);
            let root = uf.find(&key);
            res.groups.insert(key, root);
        }
    }

    // Harmonise counts in each group (the largest wins), on the concrete copies only.
    let mut best: HashMap<GroupId, usize> = HashMap::new();
    for b in &res.blocks {
        let gi = res.groups[&(b.id.clone(), 'i')].clone();
        let gj = res.groups[&(b.id.clone(), 'j')].clone();
        let e = best.entry(gi).or_insert(0);
        *e = (*e).max(b.niv);
        let e = best.entry(gj).or_insert(0);
        *e = (*e).max(b.njv);
    }
    let groups = res.groups.clone();
    for b in res.blocks.iter_mut() {
        b.niv = best[&groups[&(b.id.clone(), 'i')]];
        b.njv = best[&groups[&(b.id.clone(), 'j')]];
    }
    check_conformity(&mut res);
    res
}

/// Flag faces that overlap another block's face without matching it exactly.
fn check_conformity(res: &mut Resolution) {
    let mut pieces: IndexMap<String, Vec<(f64, f64, FaceKey, String, Face)>> = IndexMap::new();
    for b in &res.blocks {
        for f in FACES {
            let key = face_key(b.side(f));
            for r in b.side(f) {
                pieces.entry(r.id.clone()).or_default().push((r.t0.min(r.t1), r.t0.max(r.t1), key.clone(), b.id.clone(), f));
            }
        }
    }
    for items in pieces.values() {
        for i in 0..items.len() {
            let (a0, a1, ka, ba, fa) = &items[i];
            for (b0, b1, kb, bb, fb) in &items[i + 1..] {
                let overlap = a1.min(*b1) - a0.max(*b0);
                if ba != bb && overlap > 1e-9 && ka != kb {
                    for (bid, face, other) in [(ba, fa, bb), (bb, fb, ba)] {
                        res.warnings.entry(bid.clone()).or_default().push(format!(
                            "its {} face only partly matches block {}; Eilmer needs whole faces to match \
                             (add a split line so the blocks meet corner to corner)",
                            face.as_str(),
                            other
                        ));
                    }
                }
            }
        }
    }
}

/// (block id, face) pairs of other blocks sharing this exact face.
pub fn linked_faces(res: &Resolution, block_id: &str, face: Face) -> Vec<(String, Face)> {
    let Some(b) = res.block(block_id) else { return vec![] };
    let key = face_key(b.side(face));
    let mut out = vec![];
    for other in &res.blocks {
        if other.id == block_id {
            continue;
        }
        for f in FACES {
            if face_key(other.side(f)) == key {
                out.push((other.id.clone(), f));
            }
        }
    }
    out
}
