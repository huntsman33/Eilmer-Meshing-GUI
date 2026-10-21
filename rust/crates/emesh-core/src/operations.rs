//! Sketch operations that build new curves from existing ones: merge and mirror.

use crate::entities::{Curve, Geom};
use crate::geom::{self, MAX_ARC_SWEEP};
use crate::num::{c_hypot, py_dist, P};

/// The same curve traversed the other way.
pub fn reversed_curve(c: &Curve) -> Curve {
    let geom = match &c.geom {
        Geom::Line { p0, p1 } => Geom::Line { p0: *p1, p1: *p0 },
        Geom::Arc { p0, p1, centre } => Geom::Arc { p0: *p1, p1: *p0, centre: *centre },
        Geom::Bezier { points } => Geom::Bezier { points: points.iter().rev().copied().collect() },
        // The natural spline through the reversed points is the same curve, with t -> 1 - t.
        Geom::Spline { points, t0, t1 } => {
            Geom::Spline { points: points.iter().rev().copied().collect(), t0: 1.0 - t1, t1: 1.0 - t0 }
        }
    };
    Curve { geom, ..c.clone() }
}

/// One curve equal to first followed by second (first.end == second.start), or None.
fn join(first: &Curve, second: &Curve) -> Option<Curve> {
    if first.kind != second.kind || first.ctype() != second.ctype() {
        return None;
    }
    match (&first.geom, &second.geom) {
        (Geom::Line { p0, p1 }, Geom::Line { p0: q0, p1: q1 }) => {
            let d1 = [p1[0] - p0[0], p1[1] - p0[1]];
            let d2 = [q1[0] - q0[0], q1[1] - q0[1]];
            let (n1, n2) = (c_hypot(d1[0], d1[1]), c_hypot(d2[0], d2[1]));
            if (d1[0] * d2[1] - d1[1] * d2[0]).abs() > 1e-9 * n1 * n2 || d1[0] * d2[0] + d1[1] * d2[1] <= 0.0 {
                return None;
            }
            Some(Curve { geom: Geom::Line { p0: *p0, p1: *q1 }, ..first.clone() })
        }
        (Geom::Arc { p0, p1, centre }, Geom::Arc { p0: q0, p1: q1, centre: c2 }) => {
            let (c1, r1, _, s1) = geom::arc_geometry(*p0, *p1, *centre);
            let (c2, r2, _, s2) = geom::arc_geometry(*q0, *q1, *c2);
            let tol = 1e-9 * r1.max(r2);
            if py_dist(c1, c2) > tol || (r1 - r2).abs() > tol || s1 * s2 <= 0.0 {
                return None;
            }
            if (s1 + s2).abs() > MAX_ARC_SWEEP {
                return None; // an Eilmer Arc must stay well under 180 degrees
            }
            Some(Curve { geom: Geom::Arc { p0: *p0, p1: *q1, centre: *centre }, ..first.clone() })
        }
        (Geom::Spline { points: a, t0, t1 }, Geom::Spline { points: b, t0: u0, t1: u1 }) => {
            if a != b || (t1 - u0).abs() > 1e-9 {
                return None;
            }
            let (mut s, mut e) = (*t0, *u1);
            if s <= 1e-12 && e >= 1.0 - 1e-12 {
                s = 0.0;
                e = 1.0;
            }
            Some(Curve { geom: Geom::Spline { points: a.clone(), t0: s, t1: e }, ..first.clone() })
        }
        _ => None,
    }
}

/// a and b as one curve if they share an end and can be joined exactly.
pub fn try_merge(a: &Curve, b: &Curve, tol: f64) -> Option<Curve> {
    let mut b = b.clone();
    if let (Geom::Spline { points: pa, .. }, Geom::Spline { points: pb, .. }) = (&a.geom, &b.geom) {
        let rev: Vec<P> = pb.iter().rev().copied().collect();
        if *pa == rev {
            b = reversed_curve(&b); // same spline, entered the other way round
        }
    }
    let (ra, rb) = (reversed_curve(a), reversed_curve(&b));
    // Prefer orders that keep a's direction, so a merge never flips the result.
    let orders = [(a, &b), (&b, a), (a, &rb), (&rb, a), (&ra, &b), (&b, &ra), (&ra, &rb), (&rb, &ra)];
    for (f, s) in orders {
        if py_dist(f.end(), s.start()) <= tol {
            if let Some(mut joined) = join(f, s) {
                joined.id = a.id.clone();
                joined.bc_tag = if a.bc_tag.is_empty() { b.bc_tag.clone() } else { a.bc_tag.clone() };
                return Some(joined);
            }
        }
    }
    None
}

pub fn geom_size(curves: &[Curve]) -> f64 {
    let mut lo = [f64::INFINITY, f64::INFINITY];
    let mut hi = [f64::NEG_INFINITY, f64::NEG_INFINITY];
    for c in curves {
        for p in geom::sample(c) {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    if curves.is_empty() {
        return 0.0;
    }
    c_hypot(hi[0] - lo[0], hi[1] - lo[1])
}

/// Merge chains of compatible curves: (surviving curves, removed ids).
pub fn merge_curves(curves: &[Curve]) -> (Vec<Curve>, Vec<String>) {
    let tol = 1e-9 * geom_size(curves).max(1e-300);
    let mut pool: indexmap::IndexMap<String, Curve> = curves.iter().map(|c| (c.id.clone(), c.clone())).collect();
    let mut removed = vec![];
    loop {
        let ids: Vec<String> = pool.keys().cloned().collect();
        let mut merged = false;
        'outer: for (i, ia) in ids.iter().enumerate() {
            for ib in &ids[i + 1..] {
                if let Some(joined) = try_merge(&pool[ia], &pool[ib], tol) {
                    pool.insert(ia.clone(), joined);
                    pool.shift_remove(ib);
                    removed.push(ib.clone());
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    (pool.into_values().collect(), removed)
}

/// Reflection of point p across the line through a and b.
pub fn reflect(p: P, a: P, b: P) -> P {
    let d = [b[0] - a[0], b[1] - a[1]];
    let n = c_hypot(d[0], d[1]);
    let d = [d[0] / n, d[1] / n];
    let v = [p[0] - a[0], p[1] - a[1]];
    let s = v[0] * d[0] + v[1] * d[1];
    [a[0] + 2.0 * s * d[0] - v[0], a[1] + 2.0 * s * d[1] - v[1]]
}

/// True if the whole curve lies on the line through a and b (its mirror is itself).
pub fn lies_on(curve: &Curve, a: P, b: P, tol: f64) -> bool {
    let d = [b[0] - a[0], b[1] - a[1]];
    let n = c_hypot(d[0], d[1]);
    let d = [d[0] / n, d[1] / n];
    geom::sample(curve).iter().all(|p| {
        let rel = [p[0] - a[0], p[1] - a[1]];
        (rel[0] * d[1] - rel[1] * d[0]).abs() <= tol
    })
}

pub fn mirror_curve(curve: &Curve, a: P, b: P) -> Curve {
    curve.map_points(|p| reflect(p, a, b))
}
