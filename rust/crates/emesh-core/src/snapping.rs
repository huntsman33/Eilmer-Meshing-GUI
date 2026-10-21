//! NX-style point snapping: endpoint, intersection, centre, midpoint, on-curve,
//! horizontal/vertical inference from an anchor point, and an optional grid.

use std::collections::HashSet;

use crate::document::Document;
use crate::entities::Geom;
use crate::geom;
use crate::num::{c_hypot, P};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapKind {
    None,
    Point,
    Endpoint,
    Intersection,
    Centre,
    Midpoint,
    OnCurve,
    Horizontal,
    Vertical,
    Grid,
}

impl SnapKind {
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::None => "",
            SnapKind::Point => "point",
            SnapKind::Endpoint => "endpoint",
            SnapKind::Intersection => "intersection",
            SnapKind::Centre => "centre",
            SnapKind::Midpoint => "midpoint",
            SnapKind::OnCurve => "on curve",
            SnapKind::Horizontal => "horizontal",
            SnapKind::Vertical => "vertical",
            SnapKind::Grid => "grid",
        }
    }

    fn priority(self) -> u8 {
        match self {
            SnapKind::Point | SnapKind::Endpoint => 0,
            SnapKind::Intersection => 1,
            SnapKind::Centre => 2,
            _ => 3,
        }
    }
}

/// The snap options a user can switch on and off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapOption {
    Point,
    Endpoint,
    Intersection,
    Centre,
    Midpoint,
    OnCurve,
    HorizontalVertical,
    Grid,
}

pub const SNAP_OPTIONS: [(SnapOption, &str); 8] = [
    (SnapOption::Point, "Point"),
    (SnapOption::Endpoint, "Endpoint"),
    (SnapOption::Intersection, "Intersection"),
    (SnapOption::Centre, "Centre"),
    (SnapOption::Midpoint, "Midpoint"),
    (SnapOption::OnCurve, "On curve"),
    (SnapOption::HorizontalVertical, "Horizontal/vertical"),
    (SnapOption::Grid, "Grid"),
];

#[derive(Clone, Debug)]
pub struct Snap {
    pub point: P,
    pub kind: SnapKind,
    pub curve_id: Option<String>,
}

impl Snap {
    pub fn free(p: P) -> Snap {
        Snap { point: p, kind: SnapKind::None, curve_id: None }
    }
}

pub struct Snapper {
    pub enabled: HashSet<SnapOption>,
    revision: Option<u64>,
    points: Vec<(P, SnapKind, Vec<String>)>,
}

impl Default for Snapper {
    fn default() -> Self {
        let mut enabled: HashSet<SnapOption> = SNAP_OPTIONS.iter().map(|(o, _)| *o).collect();
        enabled.remove(&SnapOption::Grid);
        Snapper { enabled, revision: None, points: vec![] }
    }
}

fn option_of(kind: SnapKind) -> SnapOption {
    match kind {
        SnapKind::Point => SnapOption::Point,
        SnapKind::Endpoint => SnapOption::Endpoint,
        SnapKind::Intersection => SnapOption::Intersection,
        SnapKind::Centre => SnapOption::Centre,
        _ => SnapOption::Midpoint,
    }
}

impl Snapper {
    pub fn is_on(&self, o: SnapOption) -> bool {
        self.enabled.contains(&o)
    }

    pub fn set(&mut self, o: SnapOption, on: bool) {
        if on {
            self.enabled.insert(o);
        } else {
            self.enabled.remove(&o);
        }
    }

    fn build(&mut self, doc: &Document) {
        let mut pts = vec![];
        for (pid, p) in &doc.points {
            pts.push((*p, SnapKind::Point, vec![pid.clone()]));
        }
        let curves: Vec<_> = doc.curves.values().collect();
        for c in &curves {
            pts.push((c.start(), SnapKind::Endpoint, vec![c.id.clone()]));
            pts.push((c.end(), SnapKind::Endpoint, vec![c.id.clone()]));
            pts.push((geom::midpoint(c), SnapKind::Midpoint, vec![c.id.clone()]));
            if let Geom::Arc { centre, .. } = &c.geom {
                pts.push((*centre, SnapKind::Centre, vec![c.id.clone()]));
            }
        }
        for i in 0..curves.len() {
            for j in i + 1..curves.len() {
                for p in geom::intersections(curves[i], curves[j]) {
                    pts.push((p, SnapKind::Intersection, vec![curves[i].id.clone(), curves[j].id.clone()]));
                }
            }
        }
        self.points = pts;
    }

    /// Snap world point p; tol is the capture radius in world units.
    pub fn snap(
        &mut self,
        doc: &Document,
        revision: u64,
        p: P,
        tol: f64,
        grid_step: Option<f64>,
        anchor: Option<P>,
        exclude: &HashSet<String>,
    ) -> Snap {
        if self.revision != Some(revision) {
            self.build(doc);
            self.revision = Some(revision);
        }
        let mut best: Option<((u8, f64), Snap)> = None;
        for (q, kind, owners) in &self.points {
            let d = c_hypot(q[0] - p[0], q[1] - p[1]);
            if d > tol || !self.is_on(option_of(*kind)) || owners.iter().any(|o| exclude.contains(o)) {
                continue;
            }
            let rank = (kind.priority(), d);
            if best.as_ref().is_none_or(|b| rank < b.0) {
                let mut o = owners.clone();
                o.sort();
                best = Some((rank, Snap { point: *q, kind: *kind, curve_id: o.into_iter().next() }));
            }
        }
        if let Some((_, s)) = best {
            return s;
        }
        if self.is_on(SnapOption::OnCurve) {
            let mut near: Option<(P, f64, &str)> = None;
            for c in doc.curves.values() {
                if exclude.contains(&c.id) {
                    continue;
                }
                let (q, d) = geom::closest_point(c, p);
                if d <= tol && near.is_none_or(|n| d < n.1) {
                    near = Some((q, d, &c.id));
                }
            }
            if let Some((q, _, cid)) = near {
                let c = &doc.curves[cid];
                if matches!(c.geom, Geom::Bezier { .. } | Geom::Spline { .. }) {
                    let (_, q, _) = geom::closest_param(c, p); // refine the polyline estimate
                    return Snap { point: q, kind: SnapKind::OnCurve, curve_id: Some(cid.to_string()) };
                }
                return Snap { point: q, kind: SnapKind::OnCurve, curve_id: Some(cid.to_string()) };
            }
        }
        let (mut x, mut y) = (p[0], p[1]);
        let mut kind = SnapKind::None;
        let (mut lock_x, mut lock_y) = (false, false);
        if let (Some(a), true) = (anchor, self.is_on(SnapOption::HorizontalVertical)) {
            let (dx, dy) = (x - a[0], y - a[1]);
            if dy.abs() <= tol && dx.abs() > tol {
                y = a[1];
                lock_y = true;
                kind = SnapKind::Horizontal;
            } else if dx.abs() <= tol && dy.abs() > tol {
                x = a[0];
                lock_x = true;
                kind = SnapKind::Vertical;
            }
        }
        if let (Some(step), true) = (grid_step, self.is_on(SnapOption::Grid)) {
            if !lock_x {
                x = (x / step).round_ties_even() * step;
            }
            if !lock_y {
                y = (y / step).round_ties_even() * step;
            }
            if kind == SnapKind::None {
                kind = SnapKind::Grid;
            }
        }
        Snap { point: [x, y], kind, curve_id: None }
    }
}

/// Smallest 1-2-5 x 10^k step that is at least min_step.
pub fn nice_step(min_step: f64) -> f64 {
    if min_step <= 0.0 {
        return 1.0;
    }
    let k = min_step.log10().floor();
    for m in [1.0, 2.0, 5.0, 10.0] {
        if m * 10f64.powf(k) >= min_step * (1.0 - 1e-12) {
            return m * 10f64.powf(k);
        }
    }
    10f64.powf(k + 1.0)
}
