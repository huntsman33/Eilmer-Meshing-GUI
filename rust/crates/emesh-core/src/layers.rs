//! Boundary layers: thin blocks along walls whose cells grow geometrically away from the wall,
//! so the first cell has a chosen height (e.g. from a y+ target).
//!
//! Applying a layer to some walls (block faces on solid curves):
//!
//!   1. offsets each wall into its block by the layer thickness
//!      `delta = h (r^N - 1) / (r - 1)` (h first cell, r growth ratio, N cells)
//!      as a block split curve (a concentric arc for arcs, a line for lines, otherwise a
//!      spline) ending on the block's two side faces, delta along them from the wall;
//!   2. continues each cut through neighbouring blocks across split lines, along an iso-line
//!      of the neighbour's Coons patch, so every interface still matches face to face;
//!   3. re-blocks the split regions: across the layer N cells with the first one h at the
//!      wall; the outer part keeps the original cells (or, with transition, grows from the last
//!      layer cell to the original size); along the wall, the counts and clustering of the
//!      block each piece came from.
//!
//! Its walls are tagged as no-slip walls. A layer keeps the blocks it replaced, so it can be
//! applied again with new settings (later layers come off and go back on top) or removed.

use std::collections::HashSet;
use std::f64::consts::PI;

use indexmap::IndexMap;

use crate::blocking::{self, face_key, face_length, outline, resolve, resolve_block, FaceKey, Resolution};
use crate::document::{block_err, Bc, Block, BlockError, BoundaryLayer, Cluster, CurveRef, Document, Face, Wall, FACES};
use crate::entities::{Curve, Kind};
use crate::face::{coons, FaceEval, Faces};
use crate::geom;
use crate::num::{self, c_hypot, cumsum, linspace, np_sum, py_dist, py_mod, round_key, unwrap, P};
use crate::poly::Polygon;
use crate::pyfmt;
use crate::spacing::{mirror_cluster, thickness};
use crate::subdivide::curve_through;
use crate::topology::Topology;

/// a layer may take at most this much of a side face
pub const MAX_FRACTION: f64 = 0.9;

pub const NO_SLIP: [&str; 2] = ["WallBC_NoSlip_Adiabatic", "WallBC_NoSlip_FixedT"];

/// wall face -> (side face at the wall's start, at its end), each with whether it runs away
/// from the wall
fn sides(f: Face) -> [(Face, bool); 2] {
    match f {
        Face::South => [(Face::West, true), (Face::East, true)],
        Face::North => [(Face::West, false), (Face::East, false)],
        Face::West => [(Face::South, true), (Face::North, true)],
        Face::East => [(Face::South, false), (Face::North, false)],
    }
}

fn iso_end(f: Face, u: f64) -> (f64, f64) {
    match f {
        Face::West => (0.0, u),
        Face::East => (1.0, u),
        Face::South => (u, 0.0),
        Face::North => (u, 1.0),
    }
}

fn same(a: P, b: P) -> bool {
    py_dist(a, b) <= 1e-8 * 1.0f64.max(a[0].abs()).max(a[1].abs())
}

fn key(p: P) -> (u64, u64) {
    (round_key(p[0], 8), round_key(p[1], 8))
}

fn key_set(a: P, b: P) -> Vec<(u64, u64)> {
    let mut v = vec![key(a), key(b)];
    v.sort();
    v.dedup();
    v
}

fn arc_lengths(fe: &FaceEval) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let t = linspace(0.0, 1.0, 4001);
    let pts = fe.eval(&t);
    let seg: Vec<f64> = pts.windows(2).map(|w| c_hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).collect();
    let mut cum = vec![0.0];
    cum.extend(cumsum(&seg));
    (t, seg, cum)
}

/// The point at arc length dist along a face, from its start or its end.
fn along(fe: &FaceEval, dist: f64, from_start: bool) -> P {
    let (t, seg, cum) = arc_lengths(fe);
    let target = if from_start { dist } else { cum[cum.len() - 1] - dist };
    let k = num::searchsorted_left(&cum, target).clamp(1, cum.len() - 1);
    let frac = if seg[k - 1] > 0.0 { (target - cum[k - 1]) / seg[k - 1] } else { 0.0 };
    fe.at(t[k - 1] + frac * (t[k] - t[k - 1]))
}

/// Face parameter of the point on it nearest p.
fn param(fe: &FaceEval, p: P) -> f64 {
    let t = linspace(0.0, 1.0, 2001);
    let pts = fe.eval(&t);
    let mut k = 0;
    let mut best = f64::INFINITY;
    for (i, q) in pts.iter().enumerate() {
        let d = c_hypot(q[0] - p[0], q[1] - p[1]);
        if d < best {
            best = d;
            k = i;
        }
    }
    let lo = t[k.saturating_sub(1)];
    let hi = t[(k + 1).min(t.len() - 1)];
    num::fminbound(
        |u| {
            let q = fe.at(u);
            c_hypot(q[0] - p[0], q[1] - p[1])
        },
        lo,
        hi,
        1e-13,
    )
}

fn ccw(doc: &Document, block: &Block) -> bool {
    let poly = outline(doc, block);
    let n = poly.len();
    let terms: Vec<f64> = (0..n).map(|k| poly[k][0] * poly[(k + 1) % n][1] - poly[(k + 1) % n][0] * poly[k][1]).collect();
    np_sum(&terms) > 0.0
}

/// Split curves from e0 to e1, delta from the wall (refs) on its left or right side.
fn offset_curves(doc: &Document, refs: &[CurveRef], left: bool, delta: f64, e0: P, e1: P) -> Vec<Curve> {
    let fe = FaceEval::new(doc, refs);
    let walls: Vec<&Curve> = refs.iter().map(|r| &doc.curves[&r.id]).collect();
    if walls.iter().all(|c| c.is_arc()) {
        let c0 = walls[0].centre().unwrap();
        let (r0, r1) = (py_dist(e0, c0), py_dist(e1, c0));
        if walls.iter().all(|c| same(c.centre().unwrap(), c0)) && (r0 - r1).abs() <= 1e-9 * 1.0f64.max(r0) {
            let d: Vec<P> = fe.eval(&linspace(0.0, 1.0, 65)).iter().map(|q| [q[0] - c0[0], q[1] - c0[1]]).collect();
            let th = unwrap(&d.iter().map(|v| v[1].atan2(v[0])).collect::<Vec<_>>());
            let wall_sweep = th[th.len() - 1] - th[0];
            let a0 = (e0[1] - c0[1]).atan2(e0[0] - c0[0]);
            let a1 = (e1[1] - c0[1]).atan2(e1[0] - c0[0]);
            let sweep = wall_sweep + (py_mod(a1 - a0 - wall_sweep + PI, 2.0 * PI) - PI);
            return geom::arc_pieces(c0, r0, a0, sweep, Some(e0), Some(e1))
                .into_iter()
                .map(|(p, q, c)| Curve::arc(p, q, c).with_kind(Kind::Split))
                .collect();
        }
    }
    let n_pts: usize = walls.iter().filter_map(|c| if c.ctype() == crate::CurveType::Spline { c.points().map(|p| p.len()) } else { None }).sum();
    let n = n_pts.max(33).clamp(33, 201);
    let t = linspace(0.0, 1.0, n);
    let h = 1e-6;
    let fwd = fe.eval(&t.iter().map(|v| if v + h < 1.0 { v + h } else { 1.0 }).collect::<Vec<_>>());
    let back = fe.eval(&t.iter().map(|v| if v - h > 0.0 { v - h } else { 0.0 }).collect::<Vec<_>>());
    let base = fe.eval(&t);
    let sign = if left { 1.0 } else { -1.0 };
    let mut pts: Vec<P> = (0..n)
        .map(|k| {
            let tg = [fwd[k][0] - back[k][0], fwd[k][1] - back[k][1]];
            let l = c_hypot(tg[0], tg[1]);
            let tg = [tg[0] / l, tg[1] / l];
            let nrm = [-tg[1] * sign, tg[0] * sign];
            [base[k][0] + delta * nrm[0], base[k][1] + delta * nrm[1]]
        })
        .collect();
    let c0 = [e0[0] - pts[0][0], e0[1] - pts[0][1]];
    let c1 = [e1[0] - pts[n - 1][0], e1[1] - pts[n - 1][1]];
    for k in 0..n {
        for c in 0..2 {
            pts[k][c] += (1.0 - t[k]) * c0[c] + t[k] * c1[c];
        }
    }
    pts[0] = e0;
    pts[n - 1] = e1;
    vec![curve_through(&pts)]
}

/// Curve across a block from face f (parameter s0) to the opposite face g (s1).
#[allow(clippy::too_many_arguments)]
fn iso_curve(doc: &Document, block: &Block, f: Face, s0: f64, g: Face, s1: f64, p: P, q: P) -> Curve {
    let faces = Faces::new(doc, &block.sides);
    let (a, b) = (iso_end(f, s0), iso_end(g, s1));
    let t = linspace(0.0, 1.0, 33);
    let r: Vec<f64> = t.iter().map(|t| a.0 + t * (b.0 - a.0)).collect();
    let s: Vec<f64> = t.iter().map(|t| a.1 + t * (b.1 - a.1)).collect();
    let mut pts = coons(&faces, &r, &s);
    let n = pts.len();
    pts[0] = p;
    pts[n - 1] = q;
    curve_through(&pts)
}

/// Where a cut crosses a block face: the thin part runs from anchor to point, the rest to far.
#[derive(Clone, Debug)]
struct Split {
    point: P,
    anchor: P,
    far: P,
}

fn is_split(doc: &Document, refs: &[CurveRef]) -> bool {
    refs.iter().all(|r| doc.curves[&r.id].kind == Kind::Split)
}

// --- walls ------------------------------------------------------------------------------------

/// What a layer stores for a wall: the face's curves and a point inside its block.
pub fn wall_spec(res: &Resolution, bid: &str, face: Face) -> Wall {
    let block = res.block(bid).unwrap();
    let inside = Polygon::new(&res.polygons[bid]).representative_point();
    Wall { refs: block.side(face).clone(), inside: Some(inside) }
}

/// (index into res.blocks, face) for a stored wall, or BlockError.
pub fn find_wall(doc: &Document, res: &Resolution, wall: &Wall) -> Result<(usize, Face), BlockError> {
    if wall.refs.iter().any(|r| !doc.curves.contains_key(&r.id)) {
        return block_err("a wall of the boundary layer has been deleted");
    }
    if wall.refs.iter().any(|r| doc.curves[&r.id].kind != Kind::Geometry) {
        return block_err("only solid curves are walls (this face is a block split line)");
    }
    let k = face_key(&wall.refs);
    let mut cands = vec![];
    for (bi, b) in res.blocks.iter().enumerate() {
        for f in FACES {
            if face_key(b.side(f)) == k {
                cands.push((bi, f));
            }
        }
    }
    if cands.is_empty() {
        return block_err("a wall of the boundary layer is no longer a block face (the sketch or blocks changed)");
    }
    if let (Some(inside), true) = (wall.inside, cands.len() > 1) {
        for &(bi, f) in &cands {
            if Polygon::new(&res.polygons[&res.blocks[bi].id]).contains(inside) {
                return Ok((bi, f));
            }
        }
    }
    Ok(cands[0])
}

fn fe_ends(doc: &Document, refs: &[CurveRef]) -> (P, P) {
    let fe = FaceEval::new(doc, refs);
    (fe.at(0.0), fe.at(1.0))
}

// --- applying ---------------------------------------------------------------------------------

/// Cut the layer's walls into thin blocks, in place (its curves/blocks/base are filled in).
/// On error the document is part-changed: callers run it inside an undoable edit.
pub fn apply(doc: &mut Document, li: usize) -> Result<(), BlockError> {
    let topo = Topology::new(doc);
    let res = resolve(doc, &topo);
    let (first, growth, cells, transition) = {
        let l = &doc.layers[li];
        (l.first, l.growth, l.cells, l.transition)
    };
    let delta = thickness(first, growth, cells);
    if !(delta > 0.0) {
        return block_err("the layer needs a positive first cell height");
    }
    let last = first * growth.powf((cells as f64) - 1.0);

    let walls: Vec<(usize, Face)> =
        doc.layers[li].walls.clone().iter().map(|w| find_wall(doc, &res, w)).collect::<Result<_, _>>()?;
    tag_walls(doc, li, &res, &walls);
    let mut parents: IndexMap<String, Block> = IndexMap::new();
    let mut arena: Vec<Split> = vec![];
    let mut splits: IndexMap<FaceKey, Vec<usize>> = IndexMap::new();
    let mut new_curves: Vec<(Curve, Option<Cluster>)> = vec![];

    let split_on = |doc: &Document,
                        b: &Block,
                        g: Face,
                        away: bool,
                        corner: P,
                        arena: &mut Vec<Split>,
                        splits: &mut IndexMap<FaceKey, Vec<usize>>|
     -> Result<P, BlockError> {
        let k = face_key(b.side(g));
        if let Some(list) = splits.get(&k) {
            for &si in list {
                if same(arena[si].anchor, corner) {
                    return Ok(arena[si].point); // the neighbouring wall block made it already
                }
            }
        }
        let fe = FaceEval::new(doc, b.side(g));
        let (_, _, cum) = arc_lengths(&fe);
        let length = cum[cum.len() - 1];
        if delta >= MAX_FRACTION * length {
            return block_err(format!(
                "the layer ({} m thick) does not fit in block {}: its {} face is only {} m long",
                pyfmt::g(delta, 4),
                b.id,
                g.as_str(),
                pyfmt::g(length, 4)
            ));
        }
        let sp = Split { point: along(&fe, delta, away), anchor: corner, far: fe.at(if away { 1.0 } else { 0.0 }) };
        arena.push(sp);
        splits.entry(k).or_default().push(arena.len() - 1);
        Ok(arena[arena.len() - 1].point)
    };

    // 1. offset every wall into its block
    for &(bi, f) in &walls {
        let b = res.blocks[bi].clone();
        parents.insert(b.id.clone(), b.clone());
        let fe = FaceEval::new(doc, b.side(f));
        let corners = [fe.at(0.0), fe.at(1.0)];
        let mut ends = vec![];
        for ((g, away), c) in sides(f).into_iter().zip(corners) {
            ends.push(split_on(doc, &b, g, away, c, &mut arena, &mut splits)?);
        }
        let left = matches!(f, Face::South | Face::East) == ccw(doc, &b);
        let curves = offset_curves(doc, b.side(f), left, delta, ends[0], ends[1]);
        let single = curves.len() == 1;
        for c in curves {
            new_curves.push((c, if single { b.cluster.get(&f).cloned() } else { None }));
        }
    }

    // 2. carry the cuts on through neighbours joined across split lines
    let wall_blocks: HashSet<String> = parents.keys().cloned().collect();
    let mut used: HashSet<(String, usize)> = HashSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for n in &res.blocks {
            if wall_blocks.contains(&n.id) {
                continue;
            }
            for (f, g) in [(Face::West, Face::East), (Face::East, Face::West), (Face::South, Face::North), (Face::North, Face::South)] {
                if !is_split(doc, n.side(f)) {
                    continue; // a solid face: nothing flows across it
                }
                let current: Vec<usize> = splits.get(&face_key(n.side(f))).cloned().unwrap_or_default();
                for si in current {
                    if used.contains(&(n.id.clone(), si)) {
                        continue;
                    }
                    let fe_f = FaceEval::new(doc, n.side(f));
                    let fe_g = FaceEval::new(doc, n.side(g));
                    let sp = arena[si].clone();
                    let s0 = param(&fe_f, sp.point);
                    let at_start = same(fe_f.at(0.0), sp.anchor);
                    let mut found: Option<(usize, f64)> = None;
                    for &qi in splits.get(&face_key(n.side(g))).map(|v| v.as_slice()).unwrap_or(&[]) {
                        if !used.contains(&(n.id.clone(), qi)) {
                            let s1 = param(&fe_g, arena[qi].point);
                            if (s1 - s0).abs() < 0.2 && found.is_none_or(|m| (s1 - s0).abs() < (m.1 - s0).abs()) {
                                found = Some((qi, s1));
                            }
                        }
                    }
                    let (qi, s1) = match found {
                        Some(m) => m,
                        None => {
                            let q = Split {
                                point: fe_g.at(s0),
                                anchor: fe_g.at(if at_start { 0.0 } else { 1.0 }),
                                far: fe_g.at(if at_start { 1.0 } else { 0.0 }),
                            };
                            arena.push(q);
                            let qi = arena.len() - 1;
                            splits.entry(face_key(n.side(g))).or_default().push(qi);
                            (qi, s0)
                        }
                    };
                    used.insert((n.id.clone(), si));
                    used.insert((n.id.clone(), qi));
                    if !parents.contains_key(&n.id) {
                        parents.insert(n.id.clone(), n.clone());
                    }
                    let perp = if matches!(f, Face::West | Face::East) {
                        if at_start { Face::South } else { Face::North }
                    } else if at_start {
                        Face::West
                    } else {
                        Face::East
                    };
                    let mut spec = n.cluster.get(&perp).cloned();
                    if matches!(f, Face::East | Face::North) {
                        spec = spec.map(|s| mirror_cluster(&s));
                    }
                    let q = arena[qi].point;
                    new_curves.push((iso_curve(doc, n, f, s0, g, s1, sp.point, q), spec));
                    changed = true;
                }
            }
        }
    }

    // A cut may only touch existing curves at its two ends.
    for (c, _) in &new_curves {
        for other in doc.curves.values() {
            for (t, _, p) in geom::intersect(c, other) {
                if 1e-6 < t && t < 1.0 - 1e-6 {
                    return block_err(format!(
                        "the layer ({} m thick) would cross curve {} at ({}, {}); make it thinner",
                        pyfmt::g(delta, 4),
                        other.id,
                        pyfmt::g(p[0], 4),
                        pyfmt::g(p[1], 4)
                    ));
                }
            }
        }
    }

    // 3. swap the cut blocks for blocks over the new regions
    let stored: IndexMap<String, Block> = doc.blocks.iter().map(|b| (b.id.clone(), b.clone())).collect();
    doc.layers[li].base = doc.blocks.iter().filter(|b| parents.contains_key(&b.id)).cloned().collect();
    doc.blocks.retain(|b| !parents.contains_key(&b.id));
    let mut along_spec: IndexMap<String, Option<Cluster>> = IndexMap::new();
    for (mut c, spec) in new_curves {
        c.id = doc.new_id("c");
        along_spec.insert(c.id.clone(), spec);
        doc.add_curve(c).map_err(BlockError)?;
    }
    doc.layers[li].curves = along_spec.keys().cloned().collect();

    let new_topo = Topology::new(doc);
    let outlines: Vec<(String, Vec<P>)> = parents.keys().map(|bid| (bid.clone(), res.polygons[bid].clone())).collect();
    let mut made: Vec<(Block, Block, String)> = vec![]; // (block, concrete, parent id)
    let mut covered = 0.0;
    for region in &new_topo.regions {
        let probe = Polygon::new(&region.polygon).representative_point();
        let Some(owner) = outlines.iter().find(|(_, poly)| Polygon::new(poly).contains(probe)).map(|(b, _)| b.clone()) else {
            continue;
        };
        let parent = &stored[&owner];
        let spec = blocking::region_spec(&new_topo, region, None)
            .map_err(|e| BlockError(format!("part of block {owner} cannot be blocked after the cut ({e})")))?;
        let mut blk = Block::new(&doc.new_id("blk"));
        blk.patch = parent.patch.clone();
        blk.patch_opts = parent.patch_opts.clone();
        blk.smoothing = parent.smoothing.clone();
        blk.fs_tag = parent.fs_tag.clone();
        blk.region = Some(spec);
        let (concrete, _) = resolve_block(&new_topo, &blk)?;
        made.push((blk, concrete, owner));
        covered += region.area;
    }
    let total: f64 = num::py_sum(&outlines.iter().map(|(_, p)| Polygon::new(p).area()).collect::<Vec<_>>());
    if (covered - total).abs() > 1e-3 * total {
        return block_err("the layer cuts across other curves (is it thicker than the gap to the next wall?)");
    }

    // 4. counts and clustering
    let splits_all: Vec<Split> = splits.values().flat_map(|v| v.iter().map(|&i| arena[i].clone())).collect();
    let layer = doc.layers[li].clone();
    for (blk, concrete, owner) in made.iter_mut() {
        assign(doc, blk, concrete, &parents[owner.as_str()], &splits_all, &along_spec, &layer, last);
    }
    spread_clusters(doc, &mut made);
    for (blk, _, _) in &made {
        doc.add_block(blk.clone());
    }

    let fin = resolve(doc, &new_topo);
    for (blk, _, _) in &made {
        if let Some(e) = fin.errors.get(&blk.id) {
            return block_err(format!("block {} made by the layer cannot be meshed: {e}", blk.id));
        }
    }
    doc.layers[li].blocks = made.iter().map(|(b, _, _)| b.id.clone()).collect();
    let _ = transition;
    Ok(())
}

/// Counts and clustering for one new block, from the layer's splits and the block it came from.
#[allow(clippy::too_many_arguments)]
fn assign(
    doc: &Document,
    blk: &mut Block,
    concrete: &Block,
    parent: &Block,
    splits: &[Split],
    along: &IndexMap<String, Option<Cluster>>,
    layer: &BoundaryLayer,
    last: f64,
) {
    let mut role: IndexMap<Face, &str> = IndexMap::new();
    let mut parent_faces: IndexMap<FaceKey, Face> = IndexMap::new();
    for f in FACES {
        parent_faces.insert(face_key(parent.side(f)), f);
    }
    for f in FACES {
        let (a, z) = fe_ends(doc, concrete.side(f));
        let ends = key_set(a, z);
        let mut matched = false;
        for sp in splits {
            if ends == key_set(sp.anchor, sp.point) {
                role.insert(f, "thin");
                blk.cluster.insert(f, Cluster::growth(layer.first, !same(a, sp.anchor)));
                matched = true;
                break;
            }
            if ends == key_set(sp.point, sp.far) {
                role.insert(f, "outer");
                if layer.transition {
                    blk.cluster.insert(f, Cluster::grade(last, transition_growth(layer.growth), !same(a, sp.point)));
                } else {
                    // keep the spacing the parent had along this side
                    for pf in FACES {
                        let (pa, pz) = fe_ends(doc, parent.side(pf));
                        if let Some(spec) = parent.cluster.get(&pf) {
                            if !spec.0.is_empty() && key_set(pa, pz) == key_set(sp.anchor, sp.far) {
                                let s = same(pa, sp.anchor) == same(a, sp.point);
                                blk.cluster.insert(f, if s { spec.clone() } else { mirror_cluster(spec) });
                            }
                        }
                    }
                }
                matched = true;
                break;
            }
        }
        if !matched {
            let pf = parent_faces.get(&face_key(concrete.side(f))).copied();
            let refs = concrete.side(f);
            let pspec = pf.and_then(|pf| parent.cluster.get(&pf).filter(|c| !c.0.is_empty()).map(|c| (pf, c)));
            if let Some((pf, spec)) = pspec {
                let s = same(a, fe_ends(doc, parent.side(pf)).0);
                blk.cluster.insert(f, if s { spec.clone() } else { mirror_cluster(spec) });
            } else if refs.len() == 1 && refs[0].full() {
                if let Some(Some(spec)) = along.get(&refs[0].id) {
                    if !spec.0.is_empty() {
                        blk.cluster.insert(f, if refs[0].reversed { mirror_cluster(spec) } else { spec.clone() });
                    }
                }
            }
        }
    }

    for (is_i, faces) in [(true, [Face::South, Face::North]), (false, [Face::West, Face::East])] {
        let n = if faces.iter().any(|f| role.get(f) == Some(&"thin")) {
            layer.cells + 1
        } else {
            let (mut n, pf) = parent_count(doc, concrete, faces[0], parent);
            if layer.transition && faces.iter().any(|f| role.get(f) == Some(&"outer")) {
                // Grow from the last layer cell up to the parent's own cell size, then stay uniform.
                let target = face_length(doc, parent.side(pf)) / (n.max(2) - 1) as f64;
                n = 1 + faces
                    .iter()
                    .filter(|f| role.get(*f) == Some(&"outer"))
                    .map(|f| outer_cells(face_length(doc, concrete.side(*f)), last, transition_growth(layer.growth), target))
                    .max()
                    .unwrap();
            }
            n
        };
        if is_i {
            blk.niv = n;
        } else {
            blk.njv = n;
        }
    }
}

/// Growth ratio for grading the outer blocks: the layer's, but at least 1.1.
pub fn transition_growth(growth: f64) -> f64 {
    if 1.1 > growth {
        1.1
    } else {
        growth
    }
}

/// Cells across length: growing from `last` by `growth` until `target` size, then uniform.
pub fn outer_cells(length: f64, last: f64, growth: f64, target: f64) -> usize {
    let (mut k, mut filled, mut cell) = (0i64, 0.0f64, last);
    while cell < target && filled + cell < length {
        filled += cell;
        cell *= growth;
        k += 1;
    }
    (k + ((length - filled) / target - 1e-9).ceil() as i64).max(1) as usize
}

/// (Vertex count, face) of the parent along the direction this face runs.
fn parent_count(doc: &Document, concrete: &Block, face: Face, parent: &Block) -> (usize, Face) {
    let direction = |refs: &[CurveRef]| -> P {
        let (a, z) = fe_ends(doc, refs);
        let d = [z[0] - a[0], z[1] - a[1]];
        let mut n = c_hypot(d[0], d[1]);
        if n == 0.0 {
            n = 1.0;
        }
        [d[0] / n, d[1] / n]
    };
    let d = direction(concrete.side(face));
    let mut best = FACES[0];
    let mut bv = f64::NEG_INFINITY;
    for f in FACES {
        let e = direction(parent.side(f));
        let v = (e[0] * d[0] + e[1] * d[1]).abs();
        if v > bv {
            bv = v;
            best = f;
        }
    }
    (parent.count(best), best)
}

/// Faces without clustering take it from a shared face, else from the opposite face, so
/// vertices still match across interfaces and the layer runs straight through each block.
fn spread_clusters(doc: &Document, made: &mut [(Block, Block, String)]) {
    let mut faces: IndexMap<FaceKey, Vec<(usize, Face)>> = IndexMap::new();
    for (k, (_, concrete, _)) in made.iter().enumerate() {
        for f in FACES {
            faces.entry(face_key(concrete.side(f))).or_default().push((k, f));
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for group in faces.values() {
            let Some(&(k0, f0)) = group.iter().find(|(k, f)| made[*k].0.cluster.contains_key(f)) else { continue };
            let start = fe_ends(doc, made[k0].1.side(f0)).0;
            let spec0 = made[k0].0.cluster[&f0].clone();
            for &(k, f) in group {
                if !made[k].0.cluster.contains_key(&f) {
                    let s = same(fe_ends(doc, made[k].1.side(f)).0, start);
                    made[k].0.cluster.insert(f, if s { spec0.clone() } else { mirror_cluster(&spec0) });
                    changed = true;
                }
            }
        }
        for (blk, _, _) in made.iter_mut() {
            for f in FACES {
                let o = f.opposite();
                if !blk.cluster.contains_key(&f) {
                    if let Some(c) = blk.cluster.get(&o).cloned() {
                        blk.cluster.insert(f, c);
                        changed = true;
                    }
                }
            }
        }
    }
}

/// A tag for viscous walls: an existing no-slip one, else a new adiabatic "wall".
pub fn no_slip_tag(doc: &mut Document) -> String {
    let mut existing: Vec<String> = doc.bcs.iter().filter(|(_, s)| NO_SLIP.contains(&s.class.as_str())).map(|(t, _)| t.clone()).collect();
    existing.sort();
    if !existing.is_empty() {
        return if existing.iter().any(|t| t == "wall") { "wall".into() } else { existing[0].clone() };
    }
    let mut tag = if !doc.bcs.contains_key("wall") { "wall".to_string() } else { "noslip_wall".to_string() };
    let mut k = 2;
    while doc.bcs.contains_key(&tag) {
        tag = format!("noslip_wall{k}");
        k += 1;
    }
    doc.bcs.insert(tag.clone(), Bc { class: "WallBC_NoSlip_Adiabatic".into(), args: String::new() });
    tag
}

/// Tag the walls' slip (or untagged) curves as no-slip walls, remembering what they were.
fn tag_walls(doc: &mut Document, li: usize, res: &Resolution, walls: &[(usize, Face)]) {
    let mut slip: Vec<String> = vec![String::new()];
    slip.extend(doc.bcs.iter().filter(|(_, s)| s.class == "WallBC_WithSlip").map(|(t, _)| t.clone()));
    let mut ids: Vec<String> = vec![];
    for &(bi, f) in walls {
        for r in res.blocks[bi].side(f) {
            if slip.contains(&doc.curves[&r.id].bc_tag) && !ids.contains(&r.id) {
                ids.push(r.id.clone());
            }
        }
    }
    if ids.is_empty() {
        return;
    }
    let tag = no_slip_tag(doc);
    for cid in ids {
        let old = doc.curves[&cid].bc_tag.clone();
        doc.layers[li].tagged.entry(cid.clone()).or_insert((old, tag.clone()));
        doc.curves.get_mut(&cid).unwrap().bc_tag = tag.clone();
    }
}

/// Take a layer off: remove its curves and blocks, put back the blocks it replaced and the wall
/// tags it changed (unless they have been changed again since).
pub fn unapply(doc: &mut Document, li: usize) {
    let tagged = std::mem::take(&mut doc.layers[li].tagged);
    for (cid, (old, new)) in tagged {
        let restore = old.is_empty() || doc.bcs.contains_key(&old);
        if let Some(c) = doc.curves.get_mut(&cid) {
            if c.bc_tag == new {
                c.bc_tag = if restore { old } else { String::new() };
            }
        }
    }
    let dead: Vec<String> = doc.layers[li].curves.clone();
    for cid in &dead {
        doc.curves.shift_remove(cid);
    }
    let curves = doc.curves.clone();
    doc.dimensions.retain(|_, d| d.curve_ids().iter().all(|c| curves.contains_key(*c)));
    let gone: Vec<String> = doc.layers[li].blocks.clone();
    doc.blocks.retain(|b| !gone.contains(&b.id) && !b.curve_ids().iter().any(|c| dead.contains(c)));
    let base = std::mem::take(&mut doc.layers[li].base);
    doc.blocks.extend(base);
    doc.layers[li].curves.clear();
    doc.layers[li].blocks.clear();
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayerSettings {
    pub first: Option<f64>,
    pub growth: Option<f64>,
    pub cells: Option<usize>,
    pub transition: Option<bool>,
}

/// Change (or remove) layer lid: later layers come off first and go back on afterwards.
pub fn redo_layers(doc: &mut Document, lid: &str, settings: LayerSettings, remove: bool) -> Result<(), BlockError> {
    let k = doc.layers.iter().position(|l| l.id == lid).ok_or_else(|| BlockError(format!("no layer {lid}")))?;
    for i in (k..doc.layers.len()).rev() {
        unapply(doc, i);
    }
    let start = if remove {
        doc.layers.remove(k);
        k
    } else {
        let l = &mut doc.layers[k];
        if let Some(v) = settings.first {
            l.first = v;
        }
        if let Some(v) = settings.growth {
            l.growth = v;
        }
        if let Some(v) = settings.cells {
            l.cells = v;
        }
        if let Some(v) = settings.transition {
            l.transition = v;
        }
        k
    };
    for i in start..doc.layers.len() {
        apply(doc, i)?;
    }
    Ok(())
}

/// The layer whose wall this block face is (on one of the layer's own blocks).
pub fn layer_of_face(doc: &Document, res: &Resolution, bid: &str, face: Face) -> Option<usize> {
    let block = res.block(bid)?;
    let k = face_key(block.side(face));
    doc.layers
        .iter()
        .position(|l| l.blocks.iter().any(|b| b == bid) && l.walls.iter().any(|w| face_key(&w.refs) == k))
}
