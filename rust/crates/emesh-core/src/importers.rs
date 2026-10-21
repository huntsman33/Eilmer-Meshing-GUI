//! Import point files (.dat/.txt) as spline geometry, e.g. airfoils from the UIUC database.
//!
//! Formats (auto-detected):
//!   Selig     name line, then x y from the trailing edge over the upper surface to the
//!             leading edge and back along the lower surface
//!   Lednicer  name line, "NU. NL." counts, then upper (LE->TE) and lower (LE->TE) blocks
//!   plain     x y (or x,y / x y z) per line, used in the order given

use crate::entities::{Curve, Kind};
use crate::num::{py_hypot, P};

const DUP_TOL: f64 = 1.0e-12;

#[derive(Clone, Debug, PartialEq)]
pub struct PointFile {
    pub name: String,
    pub fmt: String,
    pub points: Vec<P>,
}

fn numeric_row(line: &str) -> Option<Vec<f64>> {
    let body = line.split('#').next().unwrap_or("").trim();
    let parts: Vec<&str> = body.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    parts.iter().map(|p| p.parse::<f64>().ok()).collect()
}

fn dedupe(points: Vec<P>) -> Vec<P> {
    let mut out: Vec<P> = vec![];
    for p in points {
        if out.last().is_none_or(|q| py_hypot(p[0] - q[0], p[1] - q[1]) > DUP_TOL) {
            out.push(p);
        }
    }
    out
}

pub fn parse_point_file(text: &str, default_name: &str) -> Result<PointFile, String> {
    let mut name: Option<String> = None;
    let mut rows: Vec<Vec<f64>> = vec![];
    for line in text.lines() {
        match numeric_row(line) {
            Some(r) => rows.push(r),
            None => {
                let t = line.trim();
                if rows.is_empty() && name.is_none() && !t.is_empty() && !t.starts_with('#') {
                    name = Some(t.to_string());
                }
            }
        }
    }
    if rows.is_empty() {
        return Err("no coordinate rows found".into());
    }
    let first = &rows[0];
    let is_count = |v: f64| v >= 2.0 && v.fract() == 0.0;
    if first.len() == 2 && is_count(first[0]) && is_count(first[1]) && rows.len() > (first[0] + first[1]) as usize {
        let (nu, nl) = (first[0] as usize, first[1] as usize);
        let upper: Vec<P> = rows[1..1 + nu].iter().map(|r| [r[0], r[1]]).collect();
        let mut lower: Vec<P> = rows[1 + nu..1 + nu + nl].iter().map(|r| [r[0], r[1]]).collect();
        if py_hypot(upper[0][0] - lower[0][0], upper[0][1] - lower[0][1]) <= DUP_TOL {
            lower.remove(0);
        }
        let mut pts: Vec<P> = upper.into_iter().rev().collect();
        pts.extend(lower);
        return Ok(PointFile { name: name.unwrap_or_else(|| default_name.into()), fmt: "lednicer".into(), points: dedupe(pts) });
    }
    let points = dedupe(rows.iter().map(|r| [r[0], r[1]]).collect());
    let fmt = if name.is_some() { "selig" } else { "xy" };
    Ok(PointFile { name: name.unwrap_or_else(|| default_name.into()), fmt: fmt.into(), points })
}

/// Index of the minimum-x point (the leading edge for airfoil-style data).
pub fn leading_edge_index(points: &[P]) -> usize {
    let xmin = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let ymid = points.iter().map(|p| p[1]).sum::<f64>() / points.len() as f64;
    let tied: Vec<usize> = (0..points.len()).filter(|&k| points[k][0] - xmin <= 1e-12 * 1.0f64.max(xmin.abs())).collect();
    let mut best = tied[0];
    for &k in &tied[1..] {
        if (points[k][1] - ymid).abs() < (points[best][1] - ymid).abs() {
            best = k;
        }
    }
    best
}

/// Scale, rotate nose-up by angle_deg (clockwise, as an angle of attack), then translate.
pub fn transform_points(points: &[P], scale: f64, angle_deg: f64, dx: f64, dy: f64) -> Vec<P> {
    let a = angle_deg.to_radians();
    let (ca, sa) = (a.cos(), a.sin());
    points
        .iter()
        .map(|p| {
            let (x, y) = (p[0] * scale, p[1] * scale);
            [x * ca + y * sa + dx, -x * sa + y * ca + dy]
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct ImportOptions {
    pub scale: f64,
    pub angle_deg: f64,
    pub dx: f64,
    pub dy: f64,
    pub split_le: bool,
    pub close_te: bool,
    pub kind: Kind,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions { scale: 1.0, angle_deg: 0.0, dx: 0.0, dy: 0.0, split_le: true, close_te: true, kind: Kind::Geometry }
    }
}

/// Spline curves (ids unset) for a parsed point file.
pub fn build_curves(pf: &PointFile, o: &ImportOptions) -> Result<Vec<Curve>, String> {
    let raw = &pf.points;
    if raw.len() < 2 {
        return Err("need at least two points".into());
    }
    let k_le = if o.split_le { leading_edge_index(raw) } else { 0 };
    let pts = transform_points(raw, o.scale, o.angle_deg, o.dx, o.dy);
    let mut curves = vec![];
    if o.split_le && 0 < k_le && k_le < pts.len() - 1 {
        curves.push(Curve::spline(pts[..=k_le].to_vec()).with_kind(o.kind));
        curves.push(Curve::spline(pts[k_le..].to_vec()).with_kind(o.kind));
    } else {
        curves.push(Curve::spline(pts.clone()).with_kind(o.kind));
    }
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    if o.close_te && py_hypot(first[0] - last[0], first[1] - last[1]) > DUP_TOL * 1.0f64.max(o.scale) {
        curves.push(Curve::line(last, first).with_kind(o.kind));
    }
    Ok(curves)
}
