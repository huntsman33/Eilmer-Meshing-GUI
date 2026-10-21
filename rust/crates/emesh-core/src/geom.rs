//! Geometry for sketch curves: sampling, closest points, intersections and arc construction.
//!
//! Arcs follow Eilmer's Arc: p0 -> p1 about centre, the short way round, so every arc sweeps
//! less than pi. Longer arcs are built from several pieces.
//!
//! Curve parameter t in [0, 1] matches Eilmer exactly, so a split at t here is the same point
//! as SubRangedPath{t0=t} there:
//!   Line, Bezier   their native parameter
//!   Arc            linear in angle
//!   Spline         an Eilmer Polyline of cubic Beziers; segment breakpoints are cumulative
//!                  segment lengths, each estimated (as Path.length does) by 100 chords; a
//!                  trimmed spline is a SubRangedPath [t0, t1]
//!
//! Every formula follows the Python reference operation by operation (see num.rs).

use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::rc::Rc;

use crate::entities::{Curve, Geom};
use crate::num::{self, c_hypot, linspace, np_sum, py_dist, py_hypot, py_mod, searchsorted_left, P};

pub const TWO_PI: f64 = 2.0 * PI;
pub const MAX_ARC_SWEEP: f64 = 0.95 * PI;
pub const ARC_STEP: f64 = 3.0 * PI / 180.0;
pub const BEZIER_SAMPLES: usize = 64;
pub const SPLINE_SEG_SAMPLES: usize = 16;
pub const PATH_LENGTH_CHORDS: usize = 100;

fn arc_step() -> f64 {
    // math.radians(3.0) = 3.0 * (pi / 180.0)
    3.0 * (PI / 180.0)
}

pub fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

pub fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

pub fn scale(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}

pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

pub fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

pub fn norm(a: P) -> f64 {
    c_hypot(a[0], a[1])
}

// --- arcs ---------------------------------------------------------------------------------

/// (centre, radius, start angle, signed sweep) with |sweep| < pi, as Eilmer evaluates it.
pub fn arc_geometry(p0: P, p1: P, centre: P) -> (P, f64, f64, f64) {
    let c = centre;
    let a = sub(p0, c);
    let b = sub(p1, c);
    let r = py_hypot(a[0], a[1]);
    let a0 = a[1].atan2(a[0]);
    let sweep = (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]);
    (c, r, a0, sweep)
}

pub fn arc_geom(c: &Curve) -> (P, f64, f64, f64) {
    match &c.geom {
        Geom::Arc { p0, p1, centre } => arc_geometry(*p0, *p1, *centre),
        _ => panic!("not an arc"),
    }
}

pub fn arc_point(arc: &Curve, t: f64) -> P {
    let (c, r, a0, sweep) = arc_geom(arc);
    let th = a0 + t * sweep;
    [c[0] + r * th.cos(), c[1] + r * th.sin()]
}

pub type ArcPiece = (P, P, P);

/// Split an arc into [(p0, p1, centre)] pieces each sweeping at most MAX_ARC_SWEEP.
pub fn arc_pieces(centre: P, r: f64, a0: f64, sweep: f64, p_start: Option<P>, p_end: Option<P>) -> Vec<ArcPiece> {
    let c = centre;
    let n = ((sweep.abs() / MAX_ARC_SWEEP - 1e-12).ceil() as i64).max(1) as usize;
    let mut pts: Vec<P> = (0..=n)
        .map(|k| {
            let a = a0 + sweep * k as f64 / n as f64;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect();
    if let Some(p) = p_start {
        pts[0] = p;
    }
    if let Some(p) = p_end {
        pts[n] = p;
    }
    (0..n).map(|k| (pts[k], pts[k + 1], c)).collect()
}

/// Centre and radius of the circle through three points, or None if collinear.
pub fn circle_through(p0: P, pm: P, p1: P) -> Option<(P, f64)> {
    let [ax, ay] = p0;
    let [bx, by] = pm;
    let [cx, cy] = p1;
    let d = 2.0 * (ax * (by - cy) + bx * (cy - ay) + cx * (ay - by));
    let scale = [ax.abs(), ay.abs(), bx.abs(), by.abs(), cx.abs(), cy.abs(), 1e-300]
        .iter()
        .fold(f64::NEG_INFINITY, |m, &v| if v > m { v } else { m });
    if d.abs() < 1e-12 * scale * scale {
        return None;
    }
    let (a2, b2, c2) = (ax * ax + ay * ay, bx * bx + by * by, cx * cx + cy * cy);
    let ux = (a2 * (by - cy) + b2 * (cy - ay) + c2 * (ay - by)) / d;
    let uy = (a2 * (cx - bx) + b2 * (ax - cx) + c2 * (bx - ax)) / d;
    Some(([ux, uy], py_hypot(ax - ux, ay - uy)))
}

/// Arc pieces from p0 through pm to p1, or None if the points are collinear.
pub fn arcs_through_3_points(p0: P, pm: P, p1: P) -> Option<Vec<ArcPiece>> {
    let ([ux, uy], r) = circle_through(p0, pm, p1)?;
    let ang = |p: P| (p[1] - uy).atan2(p[0] - ux);
    let (a0, am, a1) = (ang(p0), ang(pm), ang(p1));
    let ccw = py_mod(a1 - a0, TWO_PI);
    let mid = py_mod(am - a0, TWO_PI);
    let sweep = if mid < ccw { ccw } else { ccw - TWO_PI };
    if sweep.abs() < 1e-12 {
        return None;
    }
    Some(arc_pieces([ux, uy], r, a0, sweep, Some(p0), Some(p1)))
}

fn allclose(a: P, b: P) -> bool {
    (0..2).all(|k| (a[k] - b[k]).abs() <= 1e-8 + 1e-5 * b[k].abs())
}

/// Arc about centre from ps, ending on the ray towards pe (radius set by ps).
pub fn arcs_centre_start_end(centre: P, ps: P, pe: P, ccw: bool) -> Option<Vec<ArcPiece>> {
    let c = centre;
    let d = sub(ps, c);
    let r = c_hypot(d[0], d[1]);
    if r == 0.0 || allclose(pe, c) {
        return None;
    }
    let a0 = (ps[1] - c[1]).atan2(ps[0] - c[0]);
    let a1 = (pe[1] - c[1]).atan2(pe[0] - c[0]);
    let mut sweep = py_mod(a1 - a0, TWO_PI);
    if sweep < 1e-12 {
        return None;
    }
    if !ccw {
        sweep -= TWO_PI;
    }
    let end = [c[0] + r * a1.cos(), c[1] + r * a1.sin()];
    Some(arc_pieces(c, r, a0, sweep, Some(ps), Some(end)))
}

/// Full circle as four quarter arcs (natural O-grid corners), counter-clockwise from +x.
pub fn circle_arcs(centre: P, r: f64) -> Vec<ArcPiece> {
    let [cx, cy] = centre;
    let q = [[cx + r, cy], [cx, cy + r], [cx - r, cy], [cx, cy - r]];
    (0..4).map(|k| (q[k], q[(k + 1) % 4], centre)).collect()
}

/// Arc pieces leaving p along tangent and ending at e, or None if e lies on the tangent line.
pub fn tangent_arc(p: P, tangent: P, e: P) -> Option<Vec<ArcPiece>> {
    let n = c_hypot(tangent[0], tangent[1]);
    let t = [tangent[0] / n, tangent[1] / n];
    let nrm = [-t[1], t[0]];
    let d = sub(e, p);
    let h = dot(d, nrm);
    let dn = c_hypot(d[0], d[1]);
    if h.abs() < 1e-9 * (if 1e-300 > dn { 1e-300 } else { dn }) {
        return None;
    }
    let r_signed = dot(d, d) / (2.0 * h);
    let c = [p[0] + nrm[0] * r_signed, p[1] + nrm[1] * r_signed];
    let a0 = (p[1] - c[1]).atan2(p[0] - c[0]);
    let a1 = (e[1] - c[1]).atan2(e[0] - c[0]);
    let sweep = if r_signed > 0.0 { py_mod(a1 - a0, TWO_PI) } else { -py_mod(a0 - a1, TWO_PI) };
    Some(arc_pieces(c, r_signed.abs(), a0, sweep, Some(p), Some(e)))
}

/// Unit direction of travel at the end of a curve.
pub fn end_tangent(curve: &Curve) -> P {
    let v = match &curve.geom {
        Geom::Line { p0, p1 } => sub(*p1, *p0),
        Geom::Arc { p0, p1, centre } => {
            let (c, _, _, sweep) = arc_geometry(*p0, *p1, *centre);
            let rv = sub(*p1, c);
            if sweep > 0.0 {
                [-rv[1], rv[0]]
            } else {
                [rv[1], -rv[0]]
            }
        }
        _ => {
            let pts = sample(curve);
            let n = pts.len();
            sub(pts[n - 1], pts[n - 2])
        }
    };
    let n = c_hypot(v[0], v[1]);
    if n > 0.0 {
        [v[0] / n, v[1] / n]
    } else {
        [1.0, 0.0]
    }
}

// --- Bezier and spline -----------------------------------------------------------------------

/// de Casteljau evaluation of control points B at parameter t.
pub fn bezier_point(b: &[P], t: f64) -> P {
    let mut q: Vec<P> = b.to_vec();
    let n = b.len() - 1;
    let s = 1.0 - t;
    for k in 0..n {
        for j in 0..(n - k) {
            q[j] = [s * q[j][0] + t * q[j + 1][0], s * q[j][1] + t * q[j + 1][1]];
        }
    }
    q[0]
}

pub fn bezier_eval(b: &[P], ts: &[f64]) -> Vec<P> {
    ts.iter().map(|&t| bezier_point(b, t)).collect()
}

pub struct SplineData {
    /// cubic Bezier control points of each segment
    pub beziers: Vec<[P; 4]>,
    /// segment end parameters (cumulative length fractions), last = 1
    pub tv: Vec<f64>,
}

thread_local! {
    static SPLINES: RefCell<HashMap<Vec<u64>, Rc<SplineData>>> = RefCell::new(HashMap::new());
    static SAMPLES: RefCell<HashMap<Vec<u64>, Rc<(Vec<f64>, Vec<P>)>>> = RefCell::new(HashMap::new());
}

fn spline_key(points: &[P]) -> Vec<u64> {
    points.iter().flat_map(|p| [p[0].to_bits(), p[1].to_bits()]).collect()
}

/// Cubic Bezier control points of the interpolating spline through points.
pub fn spline_beziers(points: &[P]) -> Vec<[P; 4]> {
    let m = points.len() - 1;
    let mut d: Vec<P> = points.to_vec();
    if m >= 2 {
        // Natural cubic spline weight points: d[i-1] + 4 d[i] + d[i+1] = 6 p[i],
        // the fixed point of the Gauss-Seidel iteration in gridgen/Eilmer Spline.
        let n = m - 1;
        let dl = vec![1.0; n.saturating_sub(1)];
        let du = vec![1.0; n.saturating_sub(1)];
        let dd = vec![4.0; n];
        let mut rhs = vec![vec![0.0; n], vec![0.0; n]];
        for i in 0..n {
            for c in 0..2 {
                rhs[c][i] = 6.0 * points[i + 1][c];
            }
        }
        for c in 0..2 {
            rhs[c][0] -= points[0][c];
            rhs[c][n - 1] -= points[m][c];
        }
        num::solve_tridiagonal(&dl, &dd, &du, &mut rhs);
        for i in 0..n {
            d[i + 1] = [rhs[0][i], rhs[1][i]];
        }
    }
    (0..m)
        .map(|k| {
            let b1 = [(2.0 * d[k][0] + d[k + 1][0]) / 3.0, (2.0 * d[k][1] + d[k + 1][1]) / 3.0];
            let b2 = [(d[k][0] + 2.0 * d[k + 1][0]) / 3.0, (d[k][1] + 2.0 * d[k + 1][1]) / 3.0];
            [points[k], b1, b2, points[k + 1]]
        })
        .collect()
}

pub fn spline_data(points: &[P]) -> Rc<SplineData> {
    let key = spline_key(points);
    if let Some(d) = SPLINES.with(|s| s.borrow().get(&key).cloned()) {
        return d;
    }
    let beziers = spline_beziers(points);
    let t = linspace(0.0, 1.0, PATH_LENGTH_CHORDS + 1);
    let lengths: Vec<f64> = beziers
        .iter()
        .map(|seg| {
            let pts = bezier_eval(seg, &t);
            let segs: Vec<f64> = pts.windows(2).map(|w| c_hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).collect();
            np_sum(&segs)
        })
        .collect();
    let total = np_sum(&lengths);
    let mut tv: Vec<f64> = num::cumsum(&lengths).iter().map(|v| v / total).collect();
    let last = tv.len() - 1;
    tv[last] = 1.0;
    let data = Rc::new(SplineData { beziers, tv });
    SPLINES.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() > 2048 {
            s.clear();
        }
        s.insert(key, data.clone());
    });
    data
}

/// Evaluate the full (untrimmed) spline at Polyline parameter u.
pub fn spline_full_point(data: &SplineData, u: f64) -> P {
    let u = num::clip(u, 0.0, 1.0);
    let nb = data.beziers.len();
    let seg = searchsorted_left(&data.tv, u).min(nb - 1);
    let prev = if seg == 0 { 0.0 } else { data.tv[seg - 1] };
    let local = (u - prev) / (data.tv[seg] - prev);
    bezier_point(&data.beziers[seg], local)
}

/// Points at curve parameters ts.
pub fn points_at(curve: &Curve, ts: &[f64]) -> Vec<P> {
    match &curve.geom {
        Geom::Line { p0, p1 } => {
            let d = sub(*p1, *p0);
            ts.iter().map(|&t| [p0[0] + t * d[0], p0[1] + t * d[1]]).collect()
        }
        Geom::Arc { p0, p1, centre } => {
            let (c, r, a0, sweep) = arc_geometry(*p0, *p1, *centre);
            ts.iter()
                .map(|&t| {
                    let th = a0 + t * sweep;
                    [c[0] + r * th.cos(), c[1] + r * th.sin()]
                })
                .collect()
        }
        Geom::Bezier { points } => bezier_eval(points, ts),
        Geom::Spline { points, t0, t1 } => {
            let data = spline_data(points);
            ts.iter().map(|&t| spline_full_point(&data, t0 + (t1 - t0) * t)).collect()
        }
    }
}

pub fn point_at(curve: &Curve, t: f64) -> P {
    points_at(curve, &[t])[0]
}

/// Length as Eilmer computes it (exact for Line and Arc, 100 chords otherwise).
pub fn path_length(curve: &Curve) -> f64 {
    match &curve.geom {
        Geom::Line { p0, p1 } => py_dist(*p0, *p1),
        Geom::Arc { p0, p1, centre } => {
            let (_, r, _, sweep) = arc_geometry(*p0, *p1, *centre);
            (r * sweep).abs()
        }
        _ => polyline_length(&points_at(curve, &linspace(0.0, 1.0, PATH_LENGTH_CHORDS + 1))),
    }
}

/// np.hypot(*np.diff(P, axis=0).T).sum()
pub fn polyline_length(pts: &[P]) -> f64 {
    let segs: Vec<f64> = pts.windows(2).map(|w| c_hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).collect();
    np_sum(&segs)
}

// --- sampling ---------------------------------------------------------------------------------

fn sampled(curve: &Curve) -> Rc<(Vec<f64>, Vec<P>)> {
    let key = curve.geom_key();
    if let Some(s) = SAMPLES.with(|s| s.borrow().get(&key).cloned()) {
        return s;
    }
    let out = match &curve.geom {
        Geom::Line { p0, p1 } => (vec![0.0, 1.0], vec![*p0, *p1]),
        Geom::Arc { p0, p1, centre } => {
            let (c, r, a0, sweep) = arc_geometry(*p0, *p1, *centre);
            let n = ((sweep.abs() / arc_step()).ceil() as usize).max(4);
            let ts = linspace(0.0, 1.0, n + 1);
            let mut pts: Vec<P> = ts
                .iter()
                .map(|&t| {
                    let th = a0 + sweep * t;
                    [c[0] + r * th.cos(), c[1] + r * th.sin()]
                })
                .collect();
            pts[0] = *p0;
            pts[n] = *p1;
            (ts, pts)
        }
        Geom::Bezier { points } => {
            let ts = linspace(0.0, 1.0, BEZIER_SAMPLES);
            let pts = bezier_eval(points, &ts);
            (ts, pts)
        }
        Geom::Spline { points, t0, t1 } => {
            let data = spline_data(points);
            let mut starts = vec![0.0];
            starts.extend_from_slice(&data.tv[..data.tv.len() - 1]);
            let local = linspace(0.0, 1.0, SPLINE_SEG_SAMPLES + 1);
            let local = &local[..SPLINE_SEG_SAMPLES];
            let mut u = vec![*t0];
            for (k, &s) in starts.iter().enumerate() {
                let w = data.tv[k] - s;
                for &l in local {
                    let v = s + l * w;
                    if v > *t0 && v < *t1 {
                        u.push(v);
                    }
                }
            }
            u.push(*t1);
            let ts: Vec<f64> = u.iter().map(|v| (v - t0) / (t1 - t0)).collect();
            let pts: Vec<P> = u.iter().map(|&v| spline_full_point(&data, v)).collect();
            (ts, pts)
        }
    };
    let rc = Rc::new(out);
    SAMPLES.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() > 8192 {
            s.clear();
        }
        s.insert(key, rc.clone());
    });
    rc
}

/// (ts, points) polyline approximation, cached by geometry.
pub fn sample_with_params(curve: &Curve) -> Rc<(Vec<f64>, Vec<P>)> {
    sampled(curve)
}

/// Polyline approximation of a curve, cached by its geometry.
pub fn sample(curve: &Curve) -> Vec<P> {
    sampled(curve).1.clone()
}

pub fn bbox(curve: &Curve) -> (P, P) {
    let s = sampled(curve);
    let mut lo = [f64::INFINITY, f64::INFINITY];
    let mut hi = [f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut take = |p: &P| {
        for k in 0..2 {
            if p[k] < lo[k] {
                lo[k] = p[k];
            }
            if p[k] > hi[k] {
                hi[k] = p[k];
            }
        }
    };
    s.1.iter().for_each(&mut take);
    if let Geom::Bezier { points } = &curve.geom {
        points.iter().for_each(&mut take);
    }
    (lo, hi)
}

// --- closest point ----------------------------------------------------------------------------

/// (segment index, fraction, closest point, distance) of a polyline to p.
pub fn project_segments(pts: &[P], p: P) -> (usize, f64, P, f64) {
    let mut best: Option<(usize, f64, P, f64)> = None;
    for k in 0..pts.len() - 1 {
        let a = pts[k];
        let d = sub(pts[k + 1], a);
        let mut l2 = d[0] * d[0] + d[1] * d[1];
        if !(l2 > 0.0) {
            l2 = 1e-300;
        }
        let f = num::clip(((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / l2, 0.0, 1.0);
        let q = [a[0] + d[0] * f, a[1] + d[1] * f];
        let e = sub(q, p);
        let d2 = e[0] * e[0] + e[1] * e[1];
        if best.is_none_or(|b| d2 < b.3) {
            best = Some((k, f, q, d2));
        }
    }
    let (k, f, q, d2) = best.unwrap();
    (k, f, q, d2.sqrt())
}

/// Closest point on polyline P to p: (point, distance).
pub fn project_polyline(pts: &[P], p: P) -> (P, f64) {
    if pts.len() == 1 {
        let d = sub(pts[0], p);
        return (pts[0], c_hypot(d[0], d[1]));
    }
    let (_, _, q, d) = project_segments(pts, p);
    (q, d)
}

pub fn angle_in_sweep(a0: f64, sweep: f64, ang: f64, tol: f64) -> bool {
    if sweep >= 0.0 {
        let rel = py_mod(ang - a0, TWO_PI);
        return rel <= sweep + tol || rel >= TWO_PI - tol;
    }
    let rel = py_mod(a0 - ang, TWO_PI);
    rel <= -sweep + tol || rel >= TWO_PI - tol
}

/// Parameter of the point on the arc's circle nearest p, unclamped (may be < 0 or > 1).
pub fn arc_param(arc: &Curve, p: P) -> f64 {
    let (c, _, a0, sweep) = arc_geom(arc);
    let ang = (p[1] - c[1]).atan2(p[0] - c[0]);
    let mut rel = if sweep >= 0.0 { py_mod(ang - a0, TWO_PI) } else { py_mod(a0 - ang, TWO_PI) };
    if rel > PI + sweep.abs() / 2.0 {
        rel -= TWO_PI;
    }
    rel / sweep.abs()
}

/// Closest point on a curve to p: (point, distance). Exact for lines and arcs.
pub fn closest_point(curve: &Curve, p: P) -> (P, f64) {
    match &curve.geom {
        Geom::Line { p0, p1 } => project_polyline(&[*p0, *p1], p),
        Geom::Arc { p0, p1, centre } => {
            let (c, r, a0, sweep) = arc_geometry(*p0, *p1, *centre);
            let v = sub(p, c);
            let dv = py_hypot(v[0], v[1]);
            if dv > 0.0 && angle_in_sweep(a0, sweep, v[1].atan2(v[0]), 0.0) {
                let q = [c[0] + v[0] * (r / dv), c[1] + v[1] * (r / dv)];
                return (q, (dv - r).abs());
            }
            let d0 = { let e = sub(p, *p0); py_hypot(e[0], e[1]) };
            let d1 = { let e = sub(p, *p1); py_hypot(e[0], e[1]) };
            if d1 < d0 {
                (*p1, d1)
            } else {
                (*p0, d0)
            }
        }
        _ => project_polyline(&sampled(curve).1, p),
    }
}

/// (t, point, distance) of the closest point on the curve, refined to full precision.
pub fn closest_param(curve: &Curve, p: P) -> (f64, P, f64) {
    let t = match &curve.geom {
        Geom::Line { p0, p1 } => {
            let d = sub(*p1, *p0);
            let dd = dot(d, d);
            if dd > 0.0 {
                num::clip(dot(sub(p, *p0), d) / dd, 0.0, 1.0)
            } else {
                0.0
            }
        }
        Geom::Arc { .. } => num::clip(arc_param(curve, p), 0.0, 1.0),
        _ => {
            let s = sampled(curve);
            let (ts, pts) = (&s.0, &s.1);
            let (k, f, _, _) = project_segments(pts, p);
            let lo = ts[k.saturating_sub(1)];
            let hi = ts[(k + 2).min(ts.len() - 1)];
            let dist2 = |t: f64| {
                let q = point_at(curve, t);
                let e = sub(q, p);
                e[0] * e[0] + e[1] * e[1]
            };
            let w = hi - lo;
            let mut t = num::fminbound(dist2, lo, hi, 1e-13 * if w > 1e-300 { w } else { 1e-300 });
            let t_guess = ts[k] + f * (ts[k + 1] - ts[k]);
            if dist2(t_guess) < dist2(t) {
                t = t_guess;
            }
            polish_closest(curve, p, t)
        }
    };
    let q = point_at(curve, t);
    let e = sub(q, p);
    (t, q, c_hypot(e[0], e[1]))
}

fn pmax(a: f64, b: f64) -> f64 {
    if b > a {
        b
    } else {
        a
    }
}

fn pmin(a: f64, b: f64) -> f64 {
    if b < a {
        b
    } else {
        a
    }
}

/// Newton steps on (P(t) - p) . P'(t) = 0, to round-off precision.
fn polish_closest(curve: &Curve, p: P, mut t: f64) -> f64 {
    let h = 1e-6;
    for _ in 0..8 {
        let a = pmax(t - h, 0.0);
        let b = pmin(t + h, 1.0);
        let pts = points_at(curve, &[t, a, b]);
        let d1 = [(pts[2][0] - pts[1][0]) / (b - a), (pts[2][1] - pts[1][1]) / (b - a)];
        let g = dot(sub(pts[0], p), d1);
        let gp = dot(d1, d1);
        if gp <= 0.0 {
            break;
        }
        let t_new = pmin(pmax(t - g / gp, 0.0), 1.0);
        if (t_new - t).abs() < 1e-16 {
            break;
        }
        t = t_new;
    }
    t
}

pub fn midpoint(curve: &Curve) -> P {
    match &curve.geom {
        Geom::Line { p0, p1 } => [(p0[0] + p1[0]) / 2.0, (p0[1] + p1[1]) / 2.0],
        Geom::Arc { .. } => arc_point(curve, 0.5),
        _ => {
            let pts = &sampled(curve).1;
            let seg: Vec<f64> = pts.windows(2).map(|w| c_hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).collect();
            let mut cum = vec![0.0];
            cum.extend(num::cumsum(&seg));
            let half = cum[cum.len() - 1] / 2.0;
            let k = searchsorted_left(&cum, half) as i64 - 1;
            let k = k.clamp(0, seg.len() as i64 - 1) as usize;
            let f = if seg[k] > 0.0 { (half - cum[k]) / seg[k] } else { 0.0 };
            [pts[k][0] + f * (pts[k + 1][0] - pts[k][0]), pts[k][1] + f * (pts[k + 1][1] - pts[k][1])]
        }
    }
}

// --- splitting --------------------------------------------------------------------------------

/// Split Bezier control points at t into (left, right) control points (exact).
pub fn bezier_split(b: &[P], t: f64) -> (Vec<P>, Vec<P>) {
    let mut q: Vec<P> = b.to_vec();
    let mut left = vec![q[0]];
    let mut right = vec![q[q.len() - 1]];
    while q.len() > 1 {
        q = (0..q.len() - 1)
            .map(|k| [(1.0 - t) * q[k][0] + t * q[k + 1][0], (1.0 - t) * q[k][1] + t * q[k + 1][1]])
            .collect();
        left.push(q[0]);
        right.push(q[q.len() - 1]);
    }
    right.reverse();
    (left, right)
}

/// Copy of the curve restricted to parameters [ta, tb] (ta < tb), exact geometry.
pub fn subcurve(curve: &Curve, ta: f64, tb: f64) -> Result<Curve, String> {
    if !(0.0 <= ta && ta < tb && tb <= 1.0) {
        return Err(format!("bad sub-range {}, {}", crate::pyfmt::repr(ta), crate::pyfmt::repr(tb)));
    }
    let start = if ta == 0.0 { curve.start() } else { point_at(curve, ta) };
    let end = if tb == 1.0 { curve.end() } else { point_at(curve, tb) };
    let geom = match &curve.geom {
        Geom::Line { .. } => Geom::Line { p0: start, p1: end },
        Geom::Arc { centre, .. } => Geom::Arc { p0: start, p1: end, centre: *centre },
        Geom::Bezier { points } => {
            let mut b = points.clone();
            if tb < 1.0 {
                b = bezier_split(&b, tb).0;
            }
            if ta > 0.0 {
                b = bezier_split(&b, ta / tb).1;
            }
            let n = b.len();
            b[0] = start;
            b[n - 1] = end;
            Geom::Bezier { points: b }
        }
        Geom::Spline { points, t0, t1 } => {
            Geom::Spline { points: points.clone(), t0: t0 + (t1 - t0) * ta, t1: t0 + (t1 - t0) * tb }
        }
    };
    Ok(Curve { geom, ..curve.clone() })
}

// --- intersections ----------------------------------------------------------------------------

fn line_circle(p0: P, p1: P, c: P, r: f64) -> Vec<(P, f64)> {
    let d = sub(p1, p0);
    let f = sub(p0, c);
    let a = dot(d, d);
    let b = 2.0 * dot(f, d);
    let cc = dot(f, f) - r * r;
    let disc = b * b - 4.0 * a * cc;
    if a == 0.0 || disc < 0.0 {
        return vec![];
    }
    let sq = disc.sqrt();
    let mut roots = vec![(-b - sq) / (2.0 * a)];
    let r2 = (-b + sq) / (2.0 * a);
    if r2 != roots[0] {
        roots.push(r2);
    }
    roots.into_iter().map(|s| ([p0[0] + s * d[0], p0[1] + s * d[1]], s)).collect()
}

fn circle_circle(c1: P, r1: f64, c2: P, r2: f64) -> Vec<P> {
    let v = sub(c2, c1);
    let d = py_hypot(v[0], v[1]);
    if d == 0.0 || d > r1 + r2 || d < (r1 - r2).abs() {
        return vec![];
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = pmax(r1 * r1 - a * a, 0.0).sqrt();
    let m = [c1[0] + a * v[0] / d, c1[1] + a * v[1] / d];
    let perp = [-v[1] / d, v[0] / d];
    vec![[m[0] + h * perp[0], m[1] + h * perp[1]], [m[0] - h * perp[0], m[1] - h * perp[1]]]
}

/// Approximate crossings of polylines A, B: (segment i, frac s, segment j, frac u).
pub fn polyline_crossings(a: &[P], b: &[P]) -> Vec<(usize, f64, usize, f64)> {
    let mut out = vec![];
    if a.len() < 2 || b.len() < 2 {
        return out;
    }
    let nb = b.len() - 1;
    let mut bmin = Vec::with_capacity(nb);
    let mut bmax = Vec::with_capacity(nb);
    for j in 0..nb {
        bmin.push([b[j][0].min(b[j + 1][0]), b[j][1].min(b[j + 1][1])]);
        bmax.push([b[j][0].max(b[j + 1][0]), b[j][1].max(b[j + 1][1])]);
    }
    for i in 0..a.len() - 1 {
        let p = a[i];
        let r = sub(a[i + 1], p);
        let pr = add(p, r);
        let amin = [p[0].min(pr[0]), p[1].min(pr[1])];
        let amax = [p[0].max(pr[0]), p[1].max(pr[1])];
        for j in 0..nb {
            if !(amin[0] <= bmax[j][0] && amin[1] <= bmax[j][1] && amax[0] >= bmin[j][0] && amax[1] >= bmin[j][1]) {
                continue;
            }
            let q = b[j];
            let s = sub(b[j + 1], q);
            let denom = cross(r, s);
            if denom == 0.0 {
                continue;
            }
            let qp = sub(q, p);
            let t = cross(qp, s) / denom;
            let u = cross(qp, r) / denom;
            if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) {
                out.push((i, t, j, u));
            }
        }
    }
    out
}

/// Approximate crossing points of two polylines.
pub fn polyline_intersections(a: &[P], b: &[P]) -> Vec<P> {
    polyline_crossings(a, b)
        .into_iter()
        .map(|(i, s, _, _)| [a[i][0] + s * (a[i + 1][0] - a[i][0]), a[i][1] + s * (a[i + 1][1] - a[i][1])])
        .collect()
}

/// Newton iteration on c1(t1) = c2(t2).
fn refine(c1: &Curve, c2: &Curve, mut t1: f64, mut t2: f64) -> (f64, f64) {
    let h = 1e-7;
    for _ in 0..40 {
        let p1 = point_at(c1, t1);
        let p2 = point_at(c2, t2);
        let f = sub(p1, p2);
        let (a1, b1) = (pmax(t1 - h, 0.0), pmin(t1 + h, 1.0));
        let (a2, b2) = (pmax(t2 - h, 0.0), pmin(t2 + h, 1.0));
        let q1 = sub(point_at(c1, b1), point_at(c1, a1));
        let d1 = [q1[0] / (b1 - a1), q1[1] / (b1 - a1)];
        let q2 = sub(point_at(c2, b2), point_at(c2, a2));
        let d2 = [q2[0] / (b2 - a2), q2[1] / (b2 - a2)];
        let j = [[d1[0], -d2[0]], [d1[1], -d2[1]]];
        let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
        if det.abs() < 1e-300 {
            return (t1, t2);
        }
        let dt = match num::solve2(j, [-f[0], -f[1]]) {
            Some(v) => v,
            None => return (t1, t2),
        };
        let t1n = pmin(pmax(t1 + dt[0], 0.0), 1.0);
        let t2n = pmin(pmax(t2 + dt[1], 0.0), 1.0);
        let done = (t1n - t1).abs() < 1e-15 && (t2n - t2).abs() < 1e-15;
        t1 = t1n;
        t2 = t2n;
        if done {
            break;
        }
    }
    (t1, t2)
}

fn on_arc(arc: &Curve, p: P, tol: f64) -> bool {
    let (c, r, a0, sweep) = arc_geom(arc);
    let ang_tol = if r > 0.0 { tol / r } else { 0.0 };
    angle_in_sweep(a0, sweep, (p[1] - c[1]).atan2(p[0] - c[0]), ang_tol)
}

fn clamp01(x: f64) -> f64 {
    pmin(pmax(x, 0.0), 1.0)
}

/// Crossings of two curves as [(t1, t2, point)], exact to round-off.
pub fn intersect(c1: &Curve, c2: &Curve) -> Vec<(f64, f64, P)> {
    intersect_tol(c1, c2, 1e-9)
}

pub fn intersect_tol(c1: &Curve, c2: &Curve, tol: f64) -> Vec<(f64, f64, P)> {
    let ((lo1, hi1), (lo2, hi2)) = (bbox(c1), bbox(c2));
    let size = [hi1[0] - lo1[0], hi1[1] - lo1[1], hi2[0] - lo2[0], hi2[1] - lo2[1], 1e-300]
        .iter()
        .fold(f64::NEG_INFINITY, |m, &v| pmax(m, v));
    // max(float(np.max(hi1 - lo1)), float(np.max(hi2 - lo2)), 1e-300)
    let size = {
        let m1 = pmax(hi1[0] - lo1[0], hi1[1] - lo1[1]);
        let m2 = pmax(hi2[0] - lo2[0], hi2[1] - lo2[1]);
        let _ = size;
        pmax(pmax(m1, m2), 1e-300)
    };
    let eps = tol * pmax(size, 1.0);
    if lo1[0] > hi2[0] + eps || lo1[1] > hi2[1] + eps || lo2[0] > hi1[0] + eps || lo2[1] > hi1[1] + eps {
        return vec![];
    }
    let mut hits: Vec<(f64, f64)> = vec![];
    match (&c1.geom, &c2.geom) {
        (Geom::Line { p0: a0, p1: a1 }, Geom::Line { p0: b0, p1: b1 }) => {
            let (p, r) = (*a0, sub(*a1, *a0));
            let (q, s) = (*b0, sub(*b1, *b0));
            let denom = cross(r, s);
            if denom.abs() < 1e-14 * py_hypot(r[0], r[1]) * py_hypot(s[0], s[1]) {
                return vec![]; // parallel (overlaps are not reported)
            }
            let t = cross(sub(q, p), s) / denom;
            let u = cross(sub(q, p), r) / denom;
            let e = 1e-12;
            if -e <= t && t <= 1.0 + e && -e <= u && u <= 1.0 + e {
                hits.push((clamp01(t), clamp01(u)));
            }
        }
        (Geom::Line { .. }, Geom::Arc { .. }) | (Geom::Arc { .. }, Geom::Line { .. }) => {
            let (line, arc) = if c1.is_line() { (c1, c2) } else { (c2, c1) };
            let (lp0, lp1) = line.ends_lp();
            let (_, r, _, _) = arc_geom(arc);
            let e = 1e-12;
            for (p, s) in line_circle(lp0, lp1, arc.centre().unwrap(), r) {
                if -e <= s && s <= 1.0 + e && on_arc(arc, p, eps) {
                    let ta = clamp01(arc_param(arc, p));
                    let s = clamp01(s);
                    hits.push(if c1.is_line() { (s, ta) } else { (ta, s) });
                }
            }
        }
        (Geom::Arc { .. }, Geom::Arc { .. }) => {
            let (_, r1, _, _) = arc_geom(c1);
            let (_, r2, _, _) = arc_geom(c2);
            for p in circle_circle(c1.centre().unwrap(), r1, c2.centre().unwrap(), r2) {
                if on_arc(c1, p, eps) && on_arc(c2, p, eps) {
                    hits.push((clamp01(arc_param(c1, p)), clamp01(arc_param(c2, p))));
                }
            }
        }
        _ => {
            let s1 = sample_with_params(c1);
            let s2 = sample_with_params(c2);
            for (i, s, j, u) in polyline_crossings(&s1.1, &s2.1) {
                let t1 = s1.0[i] + s * (s1.0[i + 1] - s1.0[i]);
                let t2 = s2.0[j] + u * (s2.0[j + 1] - s2.0[j]);
                hits.push(refine(c1, c2, t1, t2));
            }
        }
    }
    let mut out: Vec<(f64, f64, P)> = vec![];
    for (t1, t2) in hits {
        let p = point_at(c1, t1);
        let e = sub(p, point_at(c2, t2));
        if c_hypot(e[0], e[1]) > pmax(eps, 1e-9 * size) * 10.0 {
            continue; // refinement failed (near-miss in the polyline)
        }
        if out.iter().all(|o| (t1 - o.0).abs() > 1e-9 || (t2 - o.1).abs() > 1e-9) {
            out.push((t1, t2, p));
        }
    }
    out
}

/// Points where two curves cross or touch.
pub fn intersections(c1: &Curve, c2: &Curve) -> Vec<P> {
    intersect(c1, c2).into_iter().map(|h| h.2).collect()
}

/// Size of the bounding box of curves (diagonal), as topology.model_size.
pub fn model_size<'a>(curves: impl IntoIterator<Item = &'a Curve>) -> f64 {
    let mut lo = [f64::INFINITY, f64::INFINITY];
    let mut hi = [f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut any = false;
    for c in curves {
        any = true;
        let (a, b) = bbox(c);
        lo = [pmin(lo[0], a[0]).min(a[0]), pmin(lo[1], a[1]).min(a[1])];
        hi = [pmax(hi[0], b[0]).max(b[0]), pmax(hi[1], b[1]).max(b[1])];
    }
    if !any {
        return 1.0;
    }
    c_hypot(hi[0] - lo[0], hi[1] - lo[1])
}

pub fn dist(a: P, b: P) -> f64 {
    py_dist(a, b)
}
