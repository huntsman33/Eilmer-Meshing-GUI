//! Sub-divide a region into quadrilateral regions by adding block split curves. The blocks are
//! then made from the new regions in the usual way.
//!
//!   split_nxm       iso-parameter lines of the region's Coons patch
//!   ogrid           an inner quadrilateral joined to the four corners (e.g. a disc)
//!   ogrid_hole      spokes from the region's corners towards an island's centre (e.g. a
//!                   cylinder in a channel), leaving four blocks around the island
//!   centroid_split  spokes from each side's midpoint to the centroid (triangles, pentagons...)

use std::f64::consts::PI;

use crate::blocking::{self, auto_corners, corner_candidates, loop_vertices, Loop};
use crate::document::{block_err, Block, BlockError};
use crate::entities::{Curve, Kind};
use crate::face::{coons, FaceEval, Faces};
use crate::geom;
use crate::num::{c_hypot, linspace, py_dist, py_mod, P};
use crate::poly::Polygon;
use crate::topology::{Region, Topology};

/// The Coons patch of a region with four corners: (r, s) -> point.
pub struct Patch {
    faces: Faces,
}

impl Patch {
    pub fn new(topo: &Topology, region: &Region, corners: Option<&[usize]>) -> Result<Patch, BlockError> {
        let spec = blocking::region_spec(topo, region, corners)?;
        let mut b = Block::new("_tmp");
        b.region = Some(spec);
        let (block, _) = blocking::resolve_block(topo, &b)?;
        Ok(Patch { faces: Faces::new(topo, &block.sides) })
    }

    pub fn eval(&self, r: &[f64], s: &[f64]) -> Vec<P> {
        coons(&self.faces, r, s)
    }
}

/// A split line if the points are collinear, otherwise a spline through them.
pub fn curve_through(pts: &[P]) -> Curve {
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = c_hypot(d[0], d[1]);
    let lm = if l > 1e-300 { l } else { 1e-300 };
    let off = pts.iter().map(|p| ((p[0] - a[0]) * d[1] - (p[1] - a[1]) * d[0]).abs() / lm).fold(f64::NEG_INFINITY, f64::max);
    if off <= 1e-9 * lm {
        return Curve::line(a, b).with_kind(Kind::Split);
    }
    Curve::spline(pts.to_vec()).with_kind(Kind::Split)
}

fn iso(patch: &Patch, a: (f64, f64), b: (f64, f64), n: usize) -> Curve {
    let t = linspace(0.0, 1.0, n);
    let r: Vec<f64> = t.iter().map(|t| a.0 + t * (b.0 - a.0)).collect();
    let s: Vec<f64> = t.iter().map(|t| a.1 + t * (b.1 - a.1)).collect();
    curve_through(&patch.eval(&r, &s))
}

/// n x m blocks: n - 1 lines across the i direction and m - 1 across j.
pub fn split_nxm(topo: &Topology, region: &Region, n: usize, m: usize, corners: Option<&[usize]>) -> Result<Vec<Curve>, BlockError> {
    if n < 1 || m < 1 || n * m < 2 {
        return block_err("split into at least two blocks");
    }
    let patch = Patch::new(topo, region, corners)?;
    let mut curves = vec![];
    for k in 1..n {
        let r = k as f64 / n as f64;
        curves.push(iso(&patch, (r, 0.0), (r, 1.0), 17));
    }
    for k in 1..m {
        let s = k as f64 / m as f64;
        curves.push(iso(&patch, (0.0, s), (1.0, s), 17));
    }
    Ok(curves)
}

/// An inner quadrilateral (size = its fraction of the patch) plus four spokes: five blocks.
pub fn ogrid(topo: &Topology, region: &Region, size: f64, corners: Option<&[usize]>) -> Result<Vec<Curve>, BlockError> {
    if !(0.05..=0.95).contains(&size) {
        return block_err("the inner block size must be between 0.05 and 0.95");
    }
    let patch = Patch::new(topo, region, corners)?;
    let (a, b) = ((1.0 - size) / 2.0, (1.0 + size) / 2.0);
    let inner = [(a, a), (b, a), (b, b), (a, b)];
    let outer = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let mut curves: Vec<Curve> = (0..4).map(|k| iso(&patch, inner[k], inner[(k + 1) % 4], 17)).collect();
    curves.extend((0..4).map(|k| iso(&patch, outer[k], inner[k], 17)));
    Ok(curves)
}

/// Four spokes from the region's corners towards the island's centre, ending where they meet
/// the island: four blocks around it, meeting the island near square-on.
pub fn ogrid_hole(topo: &Topology, region: &Region) -> Result<Vec<Curve>, BlockError> {
    if region.holes.len() != 1 {
        return block_err("O-grid around a hole needs a region with exactly one island in it");
    }
    let Some(outer) = auto_corners(topo, &region.loop_) else {
        return block_err("the region needs four corners");
    };
    let centre = Polygon::new(&region.hole_polygons[0]).centroid();
    let mut island: Vec<String> = region.holes[0].iter().map(|(e, _)| topo.edges[*e].curve_id.clone()).collect();
    island.sort();
    island.dedup();
    let verts = loop_vertices(topo, &region.loop_);
    let mut spokes = vec![];
    for &k in &outer {
        let corner = verts[k];
        let ray = Curve::line(corner, centre).with_id("_ray");
        let hits: Vec<P> =
            island.iter().flat_map(|cid| geom::intersect(&ray, topo.curve(cid)).into_iter().map(|h| h.2)).collect();
        if hits.is_empty() {
            return ogrid_hole_by_corners(topo, region, &outer);
        }
        let mut hit = hits[0];
        for h in &hits[1..] {
            if py_dist(*h, corner) < py_dist(hit, corner) {
                hit = *h;
            }
        }
        spokes.push(Curve::line(corner, hit).with_kind(Kind::Split));
    }
    Ok(spokes)
}

/// Fallback: spokes from the region's corners to the island's own four corners.
fn ogrid_hole_by_corners(topo: &Topology, region: &Region, outer: &[usize]) -> Result<Vec<Curve>, BlockError> {
    let Some(hole) = auto_corners(topo, &region.holes[0]) else {
        return block_err("the island needs four corners, or its centre must be visible from the region's corners");
    };
    let ov = loop_vertices(topo, &region.loop_);
    let hv = loop_vertices(topo, &region.holes[0]);
    let mut po: Vec<P> = outer.iter().map(|&k| ov[k]).collect();
    let mut ph: Vec<P> = hole.iter().map(|&k| hv[k]).collect();
    let centre = [ph.iter().map(|p| p[0]).sum::<f64>() / 4.0, ph.iter().map(|p| p[1]).sum::<f64>() / 4.0];
    let ang = |p: &P| (p[1] - centre[1]).atan2(p[0] - centre[0]);
    po.sort_by(|a, b| ang(a).partial_cmp(&ang(b)).unwrap());
    ph.sort_by(|a, b| ang(a).partial_cmp(&ang(b)).unwrap());
    let cost = |shift: usize| -> f64 {
        (0..4).map(|k| (py_mod(ang(&po[k]) - ang(&ph[(k + shift) % 4]) + PI, 2.0 * PI) - PI).abs()).sum()
    };
    let mut shift = 0;
    for s in 1..4 {
        if cost(s) < cost(shift) {
            shift = s;
        }
    }
    Ok((0..4).map(|k| Curve::line(po[k], ph[(k + shift) % 4]).with_kind(Kind::Split)).collect())
}

/// Spokes from each side's midpoint to the region's centroid: one block per corner.
pub fn centroid_split(topo: &Topology, region: &Region) -> Result<Vec<Curve>, BlockError> {
    if !region.holes.is_empty() {
        return block_err("centroid split does not work around a hole (use O-grid around hole)");
    }
    let lp = Loop::new(topo, &region.loop_);
    let corners = corner_candidates(topo, &region.loop_);
    if corners.len() < 3 {
        return block_err("the region needs at least three corners");
    }
    let g = Polygon::new(&region.polygon).centroid();
    let mut curves = vec![];
    for (k, &c) in corners.iter().enumerate() {
        let side = lp.refs(c, corners[(k + 1) % corners.len()]);
        let mid = FaceEval::new(topo, &side).at(0.5);
        curves.push(Curve::line(mid, g).with_kind(Kind::Split));
    }
    Ok(curves)
}
