//! Planar topology of a sketch: where curves meet, the edges between those points, and the
//! enclosed regions (faces) they bound, including holes.
//!
//! Solid and split curves take part; reference curves are drawing aids and are ignored.
//! Curves are never modified here; an edge is a parameter sub-range [ta, tb] of one curve.

use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::IndexMap;

use crate::document::Document;
use crate::entities::{Curve, Kind};
use crate::geom;
use crate::num::{linspace, py_hypot, py_sum, P};
use crate::poly::Polygon;

pub const PARAM_TOL: f64 = 1.0e-9;
pub const EDGE_SAMPLES: usize = 24;

#[derive(Clone, Debug)]
pub struct Edge {
    pub curve_id: String,
    pub ta: f64,
    pub tb: f64,
    /// vertex at ta
    pub v0: usize,
    /// vertex at tb
    pub v1: usize,
}

pub type Loop = Vec<(usize, bool)>;

#[derive(Clone, Debug)]
pub struct Region {
    /// counter-clockwise outer boundary: (edge index, forward)
    pub loop_: Loop,
    /// clockwise loops of islands inside it
    pub holes: Vec<Loop>,
    /// sampled outer boundary
    pub polygon: Vec<P>,
    pub hole_polygons: Vec<Vec<P>>,
    pub area: f64,
}

impl Region {
    pub fn shape(&self) -> Polygon<'_> {
        Polygon::with_holes(&self.polygon, self.hole_polygons.iter().map(|h| h.as_slice()).collect())
    }

    pub fn contains(&self, p: P) -> bool {
        self.shape().contains(p)
    }
}

pub fn model_size(curves: &[&Curve]) -> f64 {
    geom::model_size(curves.iter().copied())
}

fn pclamp(t: f64) -> f64 {
    let t = if 0.0 > t { 0.0 } else { t };
    if 1.0 < t {
        1.0
    } else {
        t
    }
}

pub fn merge_params(ts: &[f64]) -> Vec<f64> {
    let mut sorted: Vec<f64> = ts.iter().map(|&t| pclamp(t)).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut out: Vec<f64> = vec![];
    for t in sorted {
        if out.is_empty() || t - out[out.len() - 1] > PARAM_TOL {
            out.push(t);
        }
    }
    let n = out.len();
    out[0] = 0.0;
    out[n - 1] = 1.0;
    out
}

/// Sorted parameters (including 0 and 1) where curve cid meets any other curve.
pub fn curve_params(doc: &Document, cid: &str) -> Vec<f64> {
    let curve = &doc.curves[cid];
    let all: Vec<&Curve> = doc.curves.values().collect();
    let size = model_size(&all);
    let tol = 1e-9 * if size > 1e-300 { size } else { 1e-300 };
    let mut ts = vec![0.0, 1.0];
    for other in doc.curves.values() {
        if other.id == cid {
            continue;
        }
        ts.extend(geom::intersect(curve, other).into_iter().map(|h| h.0));
        for p in [other.start(), other.end()] {
            let (t, _, d) = geom::closest_param(curve, p);
            if d <= tol {
                ts.push(t);
            }
        }
    }
    merge_params(&ts)
}

pub struct Topology {
    pub curves: IndexMap<String, Curve>,
    pub size: f64,
    pub tol: f64,
    pub vertices: Vec<P>,
    pub edges: Vec<Edge>,
    pub regions: Vec<Region>,
}

impl Topology {
    pub fn new(doc: &Document) -> Topology {
        let curves: Vec<&Curve> = doc.curves.values().filter(|c| c.kind != Kind::Reference).collect();
        let size = model_size(&curves);
        let mut topo = Topology {
            curves: doc.curves.clone(),
            size,
            tol: 1e-8 * if size > 1e-300 { size } else { 1e-300 },
            vertices: vec![],
            edges: vec![],
            regions: vec![],
        };
        if !curves.is_empty() {
            topo.build(&curves);
        }
        topo
    }

    pub fn curve(&self, id: &str) -> &Curve {
        &self.curves[id]
    }

    fn vertex(&mut self, p: P) -> usize {
        for (k, v) in self.vertices.iter().enumerate() {
            if py_hypot(p[0] - v[0], p[1] - v[1]) <= self.tol {
                return k;
            }
        }
        self.vertices.push(p);
        self.vertices.len() - 1
    }

    fn build(&mut self, curves: &[&Curve]) {
        let mut params: HashMap<&str, Vec<f64>> = curves.iter().map(|c| (c.id.as_str(), vec![0.0, 1.0])).collect();
        for (i, a) in curves.iter().enumerate() {
            for b in &curves[i + 1..] {
                for (ta, tb, _) in geom::intersect(a, b) {
                    params.get_mut(a.id.as_str()).unwrap().push(ta);
                    params.get_mut(b.id.as_str()).unwrap().push(tb);
                }
            }
        }
        for a in curves {
            for b in curves {
                if std::ptr::eq(*a, *b) {
                    continue;
                }
                for p in [b.start(), b.end()] {
                    let (t, _, d) = geom::closest_param(a, p);
                    if d <= self.tol {
                        params.get_mut(a.id.as_str()).unwrap().push(t);
                    }
                }
            }
        }
        for c in curves {
            let ts = merge_params(&params[c.id.as_str()]);
            let pts = geom::points_at(c, &ts);
            let vids: Vec<usize> = pts.iter().map(|p| self.vertex(*p)).collect();
            for k in 0..ts.len() - 1 {
                if vids[k] == vids[k + 1] {
                    if let Ok(sc) = geom::subcurve(c, ts[k], ts[k + 1]) {
                        if geom::path_length(&sc) <= self.tol {
                            continue;
                        }
                    }
                }
                self.edges.push(Edge { curve_id: c.id.clone(), ta: ts[k], tb: ts[k + 1], v0: vids[k], v1: vids[k + 1] });
            }
        }
        let live = self.prune_dangling();
        self.find_regions(&live);
    }

    /// Edge indices left after repeatedly removing edges with a free end.
    fn prune_dangling(&self) -> BTreeSet<usize> {
        let mut live: BTreeSet<usize> = (0..self.edges.len()).collect();
        loop {
            let mut degree = vec![0usize; self.vertices.len()];
            for &e in &live {
                degree[self.edges[e].v0] += 1;
                degree[self.edges[e].v1] += 1;
            }
            let dead: Vec<usize> = live
                .iter()
                .copied()
                .filter(|&e| degree[self.edges[e].v0] == 1 || degree[self.edges[e].v1] == 1)
                .collect();
            if dead.is_empty() {
                return live;
            }
            for e in dead {
                live.remove(&e);
            }
        }
    }

    // --- edge geometry -----------------------------------------------------------------------
    pub fn edge_points(&self, e: usize, forward: bool, n: usize) -> Vec<P> {
        let edge = &self.edges[e];
        let c = &self.curves[&edge.curve_id];
        let k = if c.is_line() { 2 } else { n };
        let mut pts = geom::points_at(c, &linspace(edge.ta, edge.tb, k));
        pts[0] = self.vertices[edge.v0];
        let last = pts.len() - 1;
        pts[last] = self.vertices[edge.v1];
        if !forward {
            pts.reverse();
        }
        pts
    }

    fn leaving_angle(&self, e: usize, forward: bool) -> f64 {
        let edge = &self.edges[e];
        let c = &self.curves[&edge.curve_id];
        let (t_from, t_to) = if forward { (edge.ta, edge.tb) } else { (edge.tb, edge.ta) };
        let origin = self.vertices[if forward { edge.v0 } else { edge.v1 }];
        let q = geom::point_at(c, t_from + 1e-3 * (t_to - t_from));
        (q[1] - origin[1]).atan2(q[0] - origin[0])
    }

    // --- faces -----------------------------------------------------------------------------------
    fn find_regions(&mut self, live: &BTreeSet<usize>) {
        // half-edge h = 2e (v0 -> v1) or 2e + 1 (v1 -> v0)
        let mut out: IndexMap<usize, Vec<usize>> = IndexMap::new();
        for &e in live {
            out.entry(self.edges[e].v0).or_default().push(2 * e);
            out.entry(self.edges[e].v1).or_default().push(2 * e + 1);
        }
        for hs in out.values_mut() {
            let mut keyed: Vec<(f64, usize)> = hs.iter().map(|&h| (self.leaving_angle(h / 2, h % 2 == 0), h)).collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            *hs = keyed.into_iter().map(|(_, h)| h).collect();
        }
        let mut position: HashMap<usize, (usize, usize)> = HashMap::new();
        for (v, hs) in &out {
            for (k, &h) in hs.iter().enumerate() {
                position.insert(h, (*v, k));
            }
        }
        let next_half = |h: usize| -> usize {
            let (v, k) = position[&(h ^ 1)];
            let hs = &out[&v];
            hs[(k + hs.len() - 1) % hs.len()] // first clockwise from the twin: face stays on the left
        };
        let mut starts: Vec<usize> = position.keys().copied().collect();
        starts.sort();
        let mut seen: HashSet<usize> = HashSet::new();
        let mut faces: Vec<Loop> = vec![];
        for start in starts {
            if seen.contains(&start) {
                continue;
            }
            let mut lp = vec![];
            let mut h = start;
            while !seen.contains(&h) {
                seen.insert(h);
                lp.push((h / 2, h % 2 == 0));
                h = next_half(h);
            }
            faces.push(lp);
        }

        let component = self.components(live);
        let mut bounded: Vec<Region> = vec![];
        let mut outers: Vec<(Loop, Vec<P>, usize)> = vec![];
        for lp in faces {
            let mut poly: Vec<P> = vec![];
            for &(e, fwd) in &lp {
                let pts = self.edge_points(e, fwd, EDGE_SAMPLES);
                poly.extend_from_slice(&pts[..pts.len() - 1]);
            }
            let area = signed_area(&poly);
            if area > 0.0 {
                bounded.push(Region { loop_: lp, holes: vec![], polygon: poly, hole_polygons: vec![], area });
            } else {
                let comp = component[self.edges[lp[0].0].v0];
                outers.push((lp, poly, comp));
            }
        }
        // An island's outer (clockwise) boundary is a hole in the smallest region around it.
        for (lp, poly, comp) in outers {
            let probe = poly[0];
            let mut host: Option<usize> = None;
            for (k, r) in bounded.iter().enumerate() {
                if component[self.edges[r.loop_[0].0].v0] != comp && Polygon::new(&r.polygon).contains(probe)
                    && host.is_none_or(|h| r.area < bounded[h].area) {
                        host = Some(k);
                    }
            }
            if let Some(h) = host {
                bounded[h].holes.push(lp);
                bounded[h].hole_polygons.push(poly);
            }
        }
        for r in bounded.iter_mut() {
            let hs: Vec<f64> = r.hole_polygons.iter().map(|h| signed_area(h).abs()).collect();
            r.area -= py_sum(&hs);
        }
        bounded.sort_by(|a, b| a.area.partial_cmp(&b.area).unwrap());
        self.regions = bounded;
    }

    fn components(&self, live: &BTreeSet<usize>) -> Vec<usize> {
        let mut parent: Vec<usize> = (0..self.vertices.len()).collect();
        fn find(parent: &mut [usize], mut a: usize) -> usize {
            while parent[a] != a {
                parent[a] = parent[parent[a]];
                a = parent[a];
            }
            a
        }
        for &e in live {
            let a = find(&mut parent, self.edges[e].v0);
            let b = find(&mut parent, self.edges[e].v1);
            parent[a] = b;
        }
        (0..self.vertices.len()).map(|v| find(&mut parent, v)).collect()
    }

    // --- queries ---------------------------------------------------------------------------------
    /// The innermost region containing point p.
    pub fn region_at(&self, p: P) -> Option<&Region> {
        self.regions.iter().find(|r| r.contains(p))
    }

    pub fn region_index_at(&self, p: P) -> Option<usize> {
        self.regions.iter().position(|r| r.contains(p))
    }

    pub fn loop_curves(&self, lp: &Loop) -> Vec<String> {
        lp.iter().map(|(e, _)| self.edges[*e].curve_id.clone()).collect()
    }
}

/// 0.5 * (np.dot(x, roll(y, -1)) - np.dot(y, roll(x, -1)))
pub fn signed_area(p: &[P]) -> f64 {
    let n = p.len();
    let x: Vec<f64> = p.iter().map(|q| q[0]).collect();
    let y: Vec<f64> = p.iter().map(|q| q[1]).collect();
    let ry: Vec<f64> = (0..n).map(|k| y[(k + 1) % n]).collect();
    let rx: Vec<f64> = (0..n).map(|k| x[(k + 1) % n]).collect();
    // x, y are column views (strided); the rolled copies are contiguous
    0.5 * (crate::num::blas_dot_strided(&x, &ry) - crate::num::blas_dot_strided(&y, &rx))
}
