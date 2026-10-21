//! Geometry of two-line dimensions.
//!
//! Two lines are dimensioned by their perpendicular "distance" when parallel, or by the
//! "angle" between them otherwise. Both measure from the first line to the second.
//!
//! A distance dimension's offset is where its dimension line sits along the first line,
//! measured from that line's start (metres). An angle dimension is drawn as an arc of radius
//! offset about the vertex (the lines' common end, or where they cross); its sector [s1, s2]
//! picks one of the four angles: the one between rays s1*(p1 - p0) of the first line and
//! s2*(p1 - p0) of the second.

use crate::document::{Dimension, Document};
use crate::entities::{Curve, Geom};
use crate::num::{c_hypot, py_dist, P};

/// lines this close to parallel get a distance dimension
pub const PARALLEL_DEG: f64 = 0.5;

fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// Unit vector from a line's start to its end.
pub fn direction(line: &Curve) -> Result<P, String> {
    let (p0, p1) = line.ends_lp();
    let d = [p1[0] - p0[0], p1[1] - p0[1]];
    let n = c_hypot(d[0], d[1]);
    if n == 0.0 {
        return Err("the line has no length".into());
    }
    Ok([d[0] / n, d[1] / n])
}

/// Unit normal to the left of the line (p0 -> p1).
pub fn normal(line: &Curve) -> Result<P, String> {
    let u = direction(line)?;
    Ok([-u[1], u[0]])
}

/// Angle between two lines, ignoring direction, in degrees within [0, 90].
pub fn acute_angle(a: &Curve, b: &Curve) -> Result<f64, String> {
    let (da, db) = (direction(a)?, direction(b)?);
    Ok(cross(da, db).abs().atan2(dot(da, db).abs()).to_degrees())
}

/// "distance" for (nearly) parallel lines, "angle" otherwise.
pub fn two_line_kind(a: &Curve, b: &Curve) -> Result<&'static str, String> {
    Ok(if acute_angle(a, b)? <= PARALLEL_DEG { "distance" } else { "angle" })
}

/// Distance from line a to the middle of line b, positive on a's left.
pub fn signed_distance(a: &Curve, b: &Curve) -> Result<f64, String> {
    let (b0, b1) = b.ends_lp();
    let mid = [(b0[0] + b1[0]) / 2.0, (b0[1] + b1[1]) / 2.0];
    let a0 = a.ends_lp().0;
    Ok(dot([mid[0] - a0[0], mid[1] - a0[1]], normal(a)?))
}

fn scale(points: &[P]) -> f64 {
    points.iter().flat_map(|p| [p[0].abs(), p[1].abs()]).fold(1.0, f64::max)
}

/// The lines' common end if they share one, else where they (extended) cross, else None.
pub fn vertex(a: &Curve, b: &Curve) -> Option<P> {
    let (a0, a1) = a.ends_lp();
    let (b0, b1) = b.ends_lp();
    let tol = 1e-9 * scale(&[a0, a1, b0, b1]);
    for p in [a0, a1] {
        for q in [b0, b1] {
            if py_dist(p, q) <= tol {
                return Some(p);
            }
        }
    }
    let (ua, ub) = (direction(a).ok()?, direction(b).ok()?);
    let det = cross(ua, ub);
    if det.abs() < 1e-12 {
        return None;
    }
    let t = cross([b0[0] - a0[0], b0[1] - a0[1]], ub) / det;
    Some([a0[0] + t * ua[0], a0[1] + t * ua[1]])
}

/// (radius, sector) that puts an angle dimension of lines a, b under world.
pub fn angle_placement(a: &Curve, b: &Curve, world: P) -> (f64, [i32; 2]) {
    let Some(v) = vertex(a, b) else { return (0.0, [1, 1]) };
    let w = [world[0] - v[0], world[1] - v[1]];
    let (ua, ub) = (direction(a).unwrap(), direction(b).unwrap());
    // w = s*ua + t*ub: the signs of s and t say which of the four sectors w is in.
    let st = crate::num::solve2([[ua[0], ub[0]], [ua[1], ub[1]]], w).unwrap_or([1.0, 1.0]);
    (c_hypot(w[0], w[1]), [if st[0] >= 0.0 { 1 } else { -1 }, if st[1] >= 0.0 { 1 } else { -1 }])
}

/// The sector between the rays pointing away from the vertex (the inside of a corner).
pub fn default_sector(a: &Curve, b: &Curve) -> [i32; 2] {
    let Some(v) = vertex(a, b) else { return [1, 1] };
    let mut out = [1, 1];
    for (k, line) in [a, b].into_iter().enumerate() {
        let (p0, p1) = line.ends_lp();
        out[k] = if c_hypot(p1[0] - v[0], p1[1] - v[1]) >= c_hypot(p0[0] - v[0], p0[1] - v[1]) { 1 } else { -1 };
    }
    out
}

pub fn sector_rays(a: &Curve, b: &Curve, sector: Option<[i32; 2]>) -> Result<(P, P), String> {
    let [s1, s2] = sector.unwrap_or([1, 1]);
    let (da, db) = (direction(a)?, direction(b)?);
    Ok(([s1 as f64 * da[0], s1 as f64 * da[1]], [s2 as f64 * db[0], s2 as f64 * db[1]]))
}

/// Signed angle (radians, in (-pi, pi)) from the first ray of the sector to the second.
pub fn signed_sector_angle(a: &Curve, b: &Curve, sector: Option<[i32; 2]>) -> Result<f64, String> {
    let (r1, r2) = sector_rays(a, b, sector)?;
    Ok(cross(r1, r2).atan2(dot(r1, r2)))
}

/// The angle the dimension shows, in degrees within [0, 180].
pub fn angle_deg(a: &Curve, b: &Curve, sector: Option<[i32; 2]>) -> Result<f64, String> {
    Ok(signed_sector_angle(a, b, sector)?.abs().to_degrees())
}

/// Distance (metres) or angle (degrees) a two-line dimension currently shows.
pub fn value(doc: &Document, dim: &Dimension) -> Result<f64, String> {
    let a = &doc.curves[&dim.curve];
    let b = &doc.curves[dim.curve2.as_ref().unwrap()];
    if dim.kind == "distance" {
        return Ok(signed_distance(a, b)?.abs());
    }
    angle_deg(a, b, dim.sector)
}

/// The second line moved so the dimension reads new_value.
pub fn driven_line(doc: &Document, dim: &Dimension, new_value: f64) -> Result<Curve, String> {
    let a = &doc.curves[&dim.curve];
    let b = &doc.curves[dim.curve2.as_ref().unwrap()];
    let (p0, p1) = b.ends_lp();
    let (q0, q1);
    if dim.kind == "distance" {
        if !(new_value > 0.0) {
            return Err("a distance must be greater than zero".into());
        }
        let d = signed_distance(a, b)?;
        let n = normal(a)?;
        let k = (if d >= 0.0 { 1.0 } else { -1.0 }) * new_value - d;
        let shift = [n[0] * k, n[1] * k];
        q0 = [p0[0] + shift[0], p0[1] + shift[1]];
        q1 = [p1[0] + shift[0], p1[1] + shift[1]];
    } else {
        if !(0.0 < new_value && new_value < 180.0) {
            return Err("an angle between two lines must be between 0 and 180 degrees".into());
        }
        let v = vertex(a, b).ok_or("the lines are parallel, so they make no angle")?;
        let now = signed_sector_angle(a, b, dim.sector)?;
        let turn = new_value.to_radians().copysign(if now != 0.0 { now } else { 1.0 }) - now;
        let (c, s) = (turn.cos(), turn.sin());
        // numpy's 2x2 @ vector: fma(r0, v0, r1 * v1) per row
        let rot = |p: P| -> P {
            let w = [p[0] - v[0], p[1] - v[1]];
            [v[0] + c.mul_add(w[0], -s * w[1]), v[1] + s.mul_add(w[0], c * w[1])]
        };
        q0 = rot(p0);
        q1 = rot(p1);
    }
    let mut out = b.clone();
    out.geom = Geom::Line { p0: q0, p1: q1 };
    Ok(out)
}
