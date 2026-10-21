//! Offset curves: new curves a fixed distance to one side of existing ones (as NX's Offset
//! Curve).
//!
//! Picked curves that meet end to end are offset as chains:
//!   line            a parallel line
//!   arc             a concentric arc (radius +- d); refused if the radius would vanish
//!   Bezier, spline  a spline through normal-offset samples of the curve
//!
//! Neighbouring pieces are joined exactly: where they overlap (inside a corner) they are
//! trimmed to their crossing; where they leave a gap (outside a corner) lines are extended to
//! meet and other pieces get a round corner (an arc about the original vertex).
//!
//! A closed chain stays closed: by default it grows (offset outwards) and reverse shrinks it.
//! An open chain is offset to the left of its direction of travel by default. A result that
//! would turn inside out, vanish or cross itself is refused.

use std::collections::HashMap;

use crate::entities::{Curve, Geom};
use crate::geom;
use crate::num::{c_hypot, linspace, np_sum, py_dist, P};
use crate::operations::{geom_size, reversed_curve};
use crate::poly;

/// extend lines to meet only while the corner is within this many offsets
pub const MITRE_LIMIT: f64 = 20.0;
/// joins turning less than this (radians) are treated as tangent
pub const TANGENT_ANGLE: f64 = 1e-3;
pub const MIN_SAMPLES: usize = 65;
pub const MAX_SAMPLES: usize = 801;

/// Curves joined end to end, each turned to run along the chain.
#[derive(Clone, Debug)]
pub struct Chain {
    pub curves: Vec<Curve>,
    pub closed: bool,
}

fn polyline_of(curves: &[Curve]) -> Vec<P> {
    let mut out = geom::sample(&curves[0]);
    for c in &curves[1..] {
        out.extend_from_slice(&geom::sample(c)[1..]);
    }
    out
}

/// 0.5 * np.sum(x * roll(y, -1) - roll(x, -1) * y)
fn signed_area(p: &[P]) -> f64 {
    let n = p.len();
    let terms: Vec<f64> = (0..n).map(|k| p[k][0] * p[(k + 1) % n][1] - p[(k + 1) % n][0] * p[k][1]).collect();
    0.5 * np_sum(&terms)
}

impl Chain {
    pub fn polyline(&self) -> Vec<P> {
        polyline_of(&self.curves)
    }

    /// +1 to offset to the left of the direction of travel, -1 to the right.
    pub fn side(&self, reverse: bool) -> f64 {
        let side = if self.closed {
            if signed_area(&self.polyline()) > 0.0 {
                -1.0 // outwards
            } else {
                1.0
            }
        } else {
            1.0
        };
        if reverse {
            -side
        } else {
            side
        }
    }
}

fn tol_of(curves: &[Curve]) -> f64 {
    1e-8 * geom_size(curves).max(1e-300)
}

/// Group curves into chains that meet end to end (a chain stops where more than two curve ends
/// meet). Open chains run so most of their curves keep their own direction.
pub fn find_chains(curves: &[Curve], tol: Option<f64>) -> Vec<Chain> {
    let tol = tol.unwrap_or_else(|| tol_of(curves));
    let mut verts: Vec<P> = vec![];
    let mut vid = |p: P| -> usize {
        for (k, v) in verts.iter().enumerate() {
            if py_dist(p, *v) <= tol {
                return k;
            }
        }
        verts.push(p);
        verts.len() - 1
    };
    let ends: Vec<(usize, usize)> = curves.iter().map(|c| (vid(c.start()), vid(c.end()))).collect();
    let mut incident: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for (i, &(v0, v1)) in ends.iter().enumerate() {
        incident.entry(v0).or_default().push((i, 0));
        incident.entry(v1).or_default().push((i, 1));
    }
    let deg = |v: usize| incident[&v].len();
    let mut used = vec![false; curves.len()];
    let walk = |i: usize, forward: bool, used: &mut Vec<bool>| -> Vec<(usize, bool)> {
        let mut seq = vec![(i, forward)];
        used[i] = true;
        loop {
            let (last, fwd) = *seq.last().unwrap();
            let leave = if fwd { 1 } else { 0 };
            let v = if leave == 1 { ends[last].1 } else { ends[last].0 };
            if incident[&v].len() != 2 {
                break;
            }
            let (j, e) = *incident[&v].iter().find(|x| **x != (last, leave)).unwrap();
            if used[j] {
                break;
            }
            used[j] = true;
            seq.push((j, e == 0));
        }
        seq
    };
    let mut order: Vec<usize> = (0..curves.len()).filter(|&i| deg(ends[i].0) != 2 || deg(ends[i].1) != 2).collect();
    let rest: Vec<usize> = (0..curves.len()).filter(|i| !order.contains(i)).collect();
    order.extend(rest);
    let mut out = vec![];
    for i in order {
        if used[i] {
            continue;
        }
        let mut seq = if deg(ends[i].0) != 2 {
            walk(i, true, &mut used)
        } else if deg(ends[i].1) != 2 {
            walk(i, false, &mut used)
        } else {
            walk(i, true, &mut used)
        };
        let (first, last) = (seq[0], *seq.last().unwrap());
        let v_start = if first.1 { ends[first.0].0 } else { ends[first.0].1 };
        let v_end = if last.1 { ends[last.0].1 } else { ends[last.0].0 };
        let closed = v_start == v_end && deg(v_start) == 2;
        let forward = seq.iter().filter(|s| s.1).count();
        if !closed && forward * 2 < seq.len() {
            seq = seq.iter().rev().map(|&(k, f)| (k, !f)).collect();
        }
        out.push(Chain {
            curves: seq.iter().map(|&(k, f)| if f { curves[k].clone() } else { reversed_curve(&curves[k]) }).collect(),
            closed,
        });
    }
    out
}

fn left_normal(t: P) -> P {
    [-t[1], t[0]]
}

/// Parameters for sampling a Bezier or spline: the spline's own knots and midpoints, refined
/// to at least MIN_SAMPLES.
fn sample_params(c: &Curve) -> Vec<f64> {
    let mut ts = linspace(0.0, 1.0, MIN_SAMPLES);
    if let Geom::Spline { points, t0, t1 } = &c.geom {
        let data = geom::spline_data(points);
        let mut knots = vec![0.0];
        knots.extend_from_slice(&data.tv);
        let mut u: Vec<f64> = knots.clone();
        u.extend(knots.windows(2).map(|w| (w[0] + w[1]) / 2.0));
        let u: Vec<f64> = u.into_iter().filter(|v| v > t0 && v < t1).collect();
        let mut all = vec![0.0, 1.0];
        all.extend(u.iter().map(|v| (v - t0) / (t1 - t0)));
        ts = unique(all);
        while ts.len() < MIN_SAMPLES {
            let mut more = ts.clone();
            more.extend(ts.windows(2).map(|w| (w[0] + w[1]) / 2.0));
            ts = unique(more);
        }
        if ts.len() > MAX_SAMPLES {
            let idx = linspace(0.0, (ts.len() - 1) as f64, MAX_SAMPLES);
            ts = idx.iter().map(|&i| ts[i.round_ties_even() as usize]).collect();
        }
    }
    ts
}

fn unique(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.dedup();
    v
}

/// Unit tangents at ts: central differences, second order one-sided at the ends.
fn tangents(c: &Curve, ts: &[f64]) -> Result<Vec<P>, String> {
    let h = 1e-6;
    let a: Vec<f64> = ts.iter().map(|t| (t - h).clamp(0.0, 1.0)).collect();
    let b: Vec<f64> = ts.iter().map(|t| (t + h).clamp(0.0, 1.0)).collect();
    let pa = geom::points_at(c, &a);
    let pb = geom::points_at(c, &b);
    let mut t: Vec<P> =
        (0..ts.len()).map(|k| [(pb[k][0] - pa[k][0]) / (b[k] - a[k]), (pb[k][1] - pa[k][1]) / (b[k] - a[k])]).collect();
    let p = geom::points_at(c, &[0.0, h, 2.0 * h]);
    t[0] = [(-3.0 * p[0][0] + 4.0 * p[1][0] - p[2][0]) / (2.0 * h), (-3.0 * p[0][1] + 4.0 * p[1][1] - p[2][1]) / (2.0 * h)];
    let q = geom::points_at(c, &[1.0, 1.0 - h, 1.0 - 2.0 * h]);
    let n = t.len() - 1;
    t[n] = [(3.0 * q[0][0] - 4.0 * q[1][0] + q[2][0]) / (2.0 * h), (3.0 * q[0][1] - 4.0 * q[1][1] + q[2][1]) / (2.0 * h)];
    let mut out = vec![];
    for v in t {
        let l = c_hypot(v[0], v[1]);
        if !(l > 0.0) {
            return Err("a curve has a point where its direction is undefined".into());
        }
        out.push([v[0] / l, v[1] / l]);
    }
    Ok(out)
}

/// One curve offset by d to the left of its direction (d < 0: to the right).
pub fn offset_curve(c: &Curve, d: f64, tol: f64) -> Result<Curve, String> {
    let mk = |geom: Geom| Curve { id: String::new(), kind: c.kind, bc_tag: String::new(), geom };
    match &c.geom {
        Geom::Line { p0, p1 } => {
            let t = [p1[0] - p0[0], p1[1] - p0[1]];
            let l = c_hypot(t[0], t[1]);
            let nn = left_normal([t[0] / l, t[1] / l]);
            let n = [nn[0] * d, nn[1] * d];
            Ok(mk(Geom::Line { p0: [p0[0] + n[0], p0[1] + n[1]], p1: [p1[0] + n[0], p1[1] + n[1]] }))
        }
        Geom::Arc { p0, p1, centre } => {
            let (ctr, r, _, sweep) = geom::arc_geometry(*p0, *p1, *centre);
            let r_new = if sweep > 0.0 { r - d } else { r + d }; // the left of an anticlockwise arc is its centre
            if r_new <= tol {
                return Err(format!(
                    "an arc of radius {} m cannot be offset {} m towards its centre (its radius would vanish)",
                    crate::pyfmt::g(r, 4),
                    crate::pyfmt::g(d.abs(), 4)
                ));
            }
            let s = r_new / r;
            let q0 = [ctr[0] + (p0[0] - ctr[0]) * s, ctr[1] + (p0[1] - ctr[1]) * s];
            let q1 = [ctr[0] + (p1[0] - ctr[0]) * s, ctr[1] + (p1[1] - ctr[1]) * s];
            Ok(mk(Geom::Arc { p0: q0, p1: q1, centre: ctr }))
        }
        _ => {
            let ts = sample_params(c);
            let p = geom::points_at(c, &ts);
            let t = tangents(c, &ts)?;
            let q: Vec<P> = (0..ts.len()).map(|k| [p[k][0] + d * -t[k][1], p[k][1] + d * t[k][0]]).collect();
            for k in 0..ts.len() - 1 {
                let dp = [p[k + 1][0] - p[k][0], p[k + 1][1] - p[k][1]];
                let dq = [q[k + 1][0] - q[k][0], q[k + 1][1] - q[k][1]];
                if dp[0] * dq[0] + dp[1] * dq[1] <= 0.0 {
                    return Err(format!(
                        "a {} curve bends too tightly to offset {} m to that side",
                        c.type_name(),
                        crate::pyfmt::g(d.abs(), 4)
                    ));
                }
            }
            Ok(mk(Geom::Spline { points: q, t0: 0.0, t1: 1.0 }))
        }
    }
}

// --- joins ------------------------------------------------------------------------------------

fn with_end(c: &Curve, start: bool, p: P) -> Curve {
    let mut c = c.clone();
    match &mut c.geom {
        Geom::Line { p0, p1 } | Geom::Arc { p0, p1, .. } => {
            if start {
                *p0 = p;
            } else {
                *p1 = p;
            }
        }
        Geom::Bezier { points } | Geom::Spline { points, .. } => {
            if start {
                points[0] = p;
            } else {
                let n = points.len();
                points[n - 1] = p;
            }
        }
    }
    c
}

/// The piece of an offset curve between parameters ta < tb (a spline is resampled, untrimmed).
fn part(c: &Curve, ta: f64, tb: f64) -> Result<Curve, String> {
    if ta >= tb {
        return Err("the offset is too large: a piece of the result vanishes".into());
    }
    match &c.geom {
        Geom::Line { .. } | Geom::Arc { .. } => geom::subcurve(c, ta, tb),
        _ => {
            let n = c.points().map(|p| p.len()).unwrap_or(0).max(MIN_SAMPLES);
            let pts = geom::points_at(c, &linspace(ta, tb, n));
            let mut out = c.clone();
            out.geom = match &c.geom {
                Geom::Spline { .. } => Geom::Spline { points: pts, t0: 0.0, t1: 1.0 },
                _ => Geom::Bezier { points: pts },
            };
            Ok(out)
        }
    }
}

/// Unit directions of travel at the start and end of a curve.
fn end_tangents(c: &Curve) -> Result<(P, P), String> {
    match &c.geom {
        Geom::Line { .. } | Geom::Arc { .. } => {
            let s = geom::end_tangent(&reversed_curve(c));
            Ok(([-s[0], -s[1]], geom::end_tangent(c)))
        }
        _ => {
            let t = tangents(c, &[0.0, 1.0])?;
            Ok((t[0], t[1]))
        }
    }
}

/// Crossing of the infinite lines through two line pieces, or None if parallel.
fn line_cross(a: &Curve, b: &Curve) -> Option<P> {
    let (ap0, ap1) = a.ends_lp();
    let (bp0, bp1) = b.ends_lp();
    let r = [ap1[0] - ap0[0], ap1[1] - ap0[1]];
    let s = [bp1[0] - bp0[0], bp1[1] - bp0[1]];
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-14 * c_hypot(r[0], r[1]) * c_hypot(s[0], s[1]) {
        return None;
    }
    let w = [bp0[0] - ap0[0], bp0[1] - ap0[1]];
    let f = (w[0] * s[1] - w[1] * s[0]) / den;
    Some([ap0[0] + r[0] * f, ap0[1] + r[1] * f])
}

/// Join offset piece A (of original oa) to the following piece B (of ob):
/// (A, B, extra curves between them).
fn join(a: &Curve, b: &Curve, oa: &Curve, ob: &Curve, d: f64, tol: f64) -> Result<(Curve, Curve, Vec<Curve>), String> {
    let v = oa.end();
    let (pa, pb) = (a.end(), b.start());
    let ta = end_tangents(oa)?.1;
    let tb = end_tangents(ob)?.0;
    let turn = (ta[0] * tb[1] - ta[1] * tb[0]).atan2(ta[0] * tb[0] + ta[1] * tb[1]);
    if py_dist(pa, pb) <= tol || turn.abs() < TANGENT_ANGLE {
        return Ok((a.clone(), with_end(b, true, pa), vec![]));
    }
    let both_lines = a.is_line() && b.is_line();
    if turn * d > 0.0 {
        // the offset is inside the corner: the pieces overlap
        if both_lines {
            let x = line_cross(a, b).ok_or("the offset is too large at a corner")?;
            return Ok((with_end(a, false, x), with_end(b, true, x), vec![]));
        }
        let hits = geom::intersect(a, b);
        if hits.is_empty() {
            return Err(format!(
                "the offset is too large at the corner ({}, {})",
                crate::pyfmt::g(v[0], 4),
                crate::pyfmt::g(v[1], 4)
            ));
        }
        let (la, lb) = (geom::path_length(a), geom::path_length(b));
        let mut best = hits[0];
        for h in &hits[1..] {
            if (1.0 - h.0) * la + h.1 * lb < (1.0 - best.0) * la + best.1 * lb {
                best = *h;
            }
        }
        let (t_a, t_b, x) = best;
        let na = with_end(&part(a, 0.0, t_a)?, false, x);
        let nb = with_end(&part(b, t_b, 1.0)?, true, x);
        return Ok((na, nb, vec![]));
    }
    // the offset is outside the corner: a gap to fill
    if both_lines {
        if let Some(x) = line_cross(a, b) {
            if py_dist(x, v) <= MITRE_LIMIT * d.abs() {
                return Ok((with_end(a, false, x), with_end(b, true, x), vec![]));
            }
        }
    }
    let a0 = (pa[1] - v[1]).atan2(pa[0] - v[0]);
    let (u, w) = ([pa[0] - v[0], pa[1] - v[1]], [pb[0] - v[0], pb[1] - v[1]]);
    let sweep = (u[0] * w[1] - u[1] * w[0]).atan2(u[0] * w[0] + u[1] * w[1]);
    let arcs = geom::arc_pieces(v, d.abs(), a0, sweep, Some(pa), Some(pb))
        .into_iter()
        .map(|(p, q, c)| Curve::arc(p, q, c).with_kind(a.kind))
        .collect();
    Ok((a.clone(), b.clone(), arcs))
}

/// Refuse pieces that turned inside out or shrank to nothing.
fn check_piece(new: &Curve, old: &Curve, tol: f64) -> Result<(), String> {
    if geom::path_length(new) <= tol {
        return Err("the offset is too large: a piece of the result vanishes".into());
    }
    match (&new.geom, &old.geom) {
        (Geom::Line { p0, p1 }, Geom::Line { p0: q0, p1: q1 }) => {
            if (p1[0] - p0[0]) * (q1[0] - q0[0]) + (p1[1] - p0[1]) * (q1[1] - q0[1]) <= 0.0 {
                return Err("the offset is too large: the result would turn inside out".into());
            }
        }
        (Geom::Arc { p0, p1, centre }, Geom::Arc { p0: q0, p1: q1, centre: c2 }) => {
            let (_, _, _, s_new) = geom::arc_geometry(*p0, *p1, *centre);
            let (_, _, _, s_old) = geom::arc_geometry(*q0, *q1, *c2);
            if s_new * s_old <= 0.0 {
                return Err("the offset is too large: the result would turn inside out".into());
            }
        }
        _ => {}
    }
    Ok(())
}

fn polyline_dedup(curves: &[Curve]) -> Vec<P> {
    let p = polyline_of(curves);
    let mut out = vec![p[0]];
    for w in p.windows(2) {
        if c_hypot(w[1][0] - w[0][0], w[1][1] - w[0][1]) > 0.0 {
            out.push(w[1]);
        }
    }
    out
}

/// Offset one chain by d (> 0: to the left of its direction). The new curves in order.
pub fn offset_chain(chain: &Chain, d: f64, tol: f64) -> Result<Vec<Curve>, String> {
    let n = chain.curves.len();
    let mut pieces: Vec<Curve> = chain.curves.iter().map(|c| offset_curve(c, d, tol)).collect::<Result<_, _>>()?;
    let mut extras: Vec<Vec<Curve>> = vec![vec![]; n];
    let joins = if chain.closed { n } else { n - 1 };
    for k in 0..joins {
        let j = (k + 1) % n;
        if n == 1 && chain.closed {
            let s = pieces[0].start();
            pieces[0] = with_end(&pieces[0], false, s); // a single closed curve
            break;
        }
        let (a, b, e) = join(&pieces[k], &pieces[j], &chain.curves[k], &chain.curves[j], d, tol)?;
        pieces[k] = a;
        pieces[j] = b;
        extras[k] = e;
    }
    for (new, old) in pieces.iter().zip(&chain.curves) {
        check_piece(new, old, tol)?;
    }
    let mut out = vec![];
    for (p, e) in pieces.into_iter().zip(extras) {
        out.push(p);
        out.extend(e);
    }
    // The result must not cross itself (a closed one must keep its orientation).
    if out.len() > 1 || chain.closed {
        let p = polyline_dedup(&out);
        if chain.closed {
            let area0 = signed_area(&chain.polyline());
            let area1 = signed_area(&p);
            if p.len() < 4 || area1 * area0 <= 0.0 || area1.abs() <= tol * tol {
                return Err("the offset is too large: the closed shape would vanish".into());
            }
            if !poly::ring_is_simple(&p) {
                return Err("the offset is too large: the result would cross itself".into());
            }
        } else if poly::line_is_simple(&chain.polyline()) && !poly::line_is_simple(&p) {
            return Err("the offset is too large: the result would cross itself".into());
        }
    }
    Ok(out)
}

/// New curves offsetting the given ones by distance (> 0, metres): each chain of connected
/// curves is offset as a whole. The originals are not changed.
pub fn offset_chains(curves: &[Curve], distance: f64, reverse: bool) -> Result<Vec<Curve>, String> {
    if curves.is_empty() {
        return Err("pick the curves to offset first".into());
    }
    if !(distance > 0.0) {
        return Err("the offset distance must be positive".into());
    }
    let tol = tol_of(curves);
    let mut out = vec![];
    for chain in find_chains(curves, Some(tol)) {
        out.extend(offset_chain(&chain, chain.side(reverse) * distance, tol)?);
    }
    Ok(out)
}

/// [(point, unit normal)] per chain: where to draw an arrow showing the offset side.
pub fn side_markers(curves: &[Curve], reverse: bool) -> Vec<(P, P)> {
    if curves.is_empty() {
        return vec![];
    }
    find_chains(curves, None)
        .iter()
        .map(|chain| {
            let c = &chain.curves[chain.curves.len() / 2];
            let p = geom::point_at(c, 0.5);
            let ab = geom::points_at(c, &[0.5 - 1e-4, 0.5 + 1e-4]);
            let d = [ab[1][0] - ab[0][0], ab[1][1] - ab[0][1]];
            let l = c_hypot(d[0], d[1]).max(1e-300);
            let n = left_normal([d[0] / l, d[1] / l]);
            let s = chain.side(reverse);
            (p, [n[0] * s, n[1] * s])
        })
        .collect()
}
