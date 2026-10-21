//! Polygon predicates used by the Python reference through shapely (GEOS), reimplemented with
//! GEOS's own algorithms so the results (e.g. representative points stored in projects) match.
//!
//! A polygon is its outer ring plus hole rings; rings are given open (first point not repeated)
//! as the Python code passes sampled boundaries to shapely, which closes them.

use crate::num::P;

pub struct Polygon<'a> {
    pub shell: &'a [P],
    pub holes: Vec<&'a [P]>,
}

impl<'a> Polygon<'a> {
    pub fn new(shell: &'a [P]) -> Self {
        Polygon { shell, holes: vec![] }
    }

    pub fn with_holes(shell: &'a [P], holes: Vec<&'a [P]>) -> Self {
        Polygon { shell, holes: holes.into_iter().filter(|h| h.len() >= 3).collect() }
    }

    /// shapely `Polygon.contains(Point)`: strictly inside (not on the boundary).
    pub fn contains(&self, p: P) -> bool {
        if self.shell.len() < 3 {
            return false;
        }
        match ring_locate(self.shell, p) {
            Loc::Interior => {}
            _ => return false,
        }
        for h in &self.holes {
            match ring_locate(h, p) {
                Loc::Exterior => {}
                _ => return false,
            }
        }
        true
    }

    /// shapely `representative_point()` (GEOS InteriorPointArea).
    pub fn representative_point(&self) -> P {
        if self.shell.is_empty() {
            return [f64::NAN, f64::NAN];
        }
        let rings: Vec<Vec<P>> = std::iter::once(self.shell).chain(self.holes.iter().copied()).map(closed).collect();
        // ScanLineYOrdinateFinder
        let (mut lo_y, mut hi_y) = (f64::INFINITY, f64::NEG_INFINITY);
        for p in &rings[0] {
            lo_y = lo_y.min(p[1]);
            hi_y = hi_y.max(p[1]);
        }
        let centre_y = (lo_y + hi_y) / 2.0;
        for ring in &rings {
            for p in ring {
                let y = p[1];
                if y <= centre_y {
                    if y > lo_y {
                        lo_y = y;
                    }
                } else if y > centre_y && y < hi_y {
                    hi_y = y;
                }
            }
        }
        let scan_y = (hi_y + lo_y) / 2.0;
        // InteriorPointPolygon
        let mut interior = rings[0][0];
        let mut crossings: Vec<f64> = vec![];
        for ring in &rings {
            let (mut rlo, mut rhi) = (f64::INFINITY, f64::NEG_INFINITY);
            for p in ring {
                rlo = rlo.min(p[1]);
                rhi = rhi.max(p[1]);
            }
            if !(scan_y >= rlo && scan_y <= rhi) {
                continue;
            }
            for w in ring.windows(2) {
                let (p0, p1) = (w[0], w[1]);
                if p0[1] > scan_y && p1[1] > scan_y {
                    continue;
                }
                if p0[1] < scan_y && p1[1] < scan_y {
                    continue;
                }
                if p0[1] == p1[1] {
                    continue; // horizontal
                }
                if p0[1] == scan_y && p1[1] < scan_y {
                    continue;
                }
                if p1[1] == scan_y && p0[1] < scan_y {
                    continue;
                }
                let x = if p0[0] == p1[0] {
                    p0[0]
                } else {
                    let m = (p1[1] - p0[1]) / (p1[0] - p0[0]);
                    p0[0] + ((scan_y - p0[1]) / m)
                };
                crossings.push(x);
            }
        }
        crossings.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut width = 0.0;
        let mut i = 0;
        while i + 1 < crossings.len() {
            let (x1, x2) = (crossings[i], crossings[i + 1]);
            let w = x2 - x1;
            if w > width {
                width = w;
                interior = [(x1 + x2) / 2.0, scan_y];
            }
            i += 2;
        }
        interior
    }

    /// shapely `centroid` of the polygon (GEOS Centroid, area weighted).
    pub fn centroid(&self) -> P {
        let shell = closed(self.shell);
        let base = shell[0];
        let (mut cx, mut cy, mut area2sum) = (0.0f64, 0.0f64, 0.0f64);
        let mut add_ring = |pts: &[P], positive: bool| {
            let sign = if positive { 1.0 } else { -1.0 };
            for k in 0..pts.len() - 1 {
                let (p1, p2) = (pts[k], pts[k + 1]);
                let c = [base[0] + p1[0] + p2[0], base[1] + p1[1] + p2[1]];
                let a2 = (p1[0] - base[0]) * (p2[1] - base[1]) - (p2[0] - base[0]) * (p1[1] - base[1]);
                cx += sign * a2 * c[0];
                cy += sign * a2 * c[1];
                area2sum += sign * a2;
            }
        };
        let shell_ccw = signed_area(self.shell) > 0.0;
        add_ring(&shell, !shell_ccw);
        for h in &self.holes {
            let hc = closed(h);
            add_ring(&hc, signed_area(h) > 0.0);
        }
        if area2sum != 0.0 {
            [cx / 3.0 / area2sum, cy / 3.0 / area2sum]
        } else {
            // degenerate: the mean of the vertices
            let n = self.shell.len() as f64;
            let s = self.shell.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
            [s[0] / n, s[1] / n]
        }
    }

    pub fn area(&self) -> f64 {
        let mut a = signed_area(self.shell).abs();
        for h in &self.holes {
            a -= signed_area(h).abs();
        }
        a
    }
}

fn closed(r: &[P]) -> Vec<P> {
    let mut v = r.to_vec();
    if !v.is_empty() && v[0] != v[v.len() - 1] {
        v.push(v[0]);
    }
    v
}

/// Shoelace signed area of an open ring (as numpy np.dot(x, roll(y)) - ..., written plainly).
pub fn signed_area(r: &[P]) -> f64 {
    let n = r.len();
    let mut s1 = 0.0;
    let mut s2 = 0.0;
    for k in 0..n {
        let j = (k + 1) % n;
        s1 += r[k][0] * r[j][1];
        s2 += r[k][1] * r[j][0];
    }
    0.5 * (s1 - s2)
}

#[derive(Debug, PartialEq)]
enum Loc {
    Interior,
    Boundary,
    Exterior,
}

fn orient(p1: P, p2: P, q: P) -> i32 {
    let d = (p2[0] - p1[0]) * (q[1] - p2[1]) - (p2[1] - p1[1]) * (q[0] - p2[0]);
    if d > 0.0 {
        1
    } else if d < 0.0 {
        -1
    } else {
        0
    }
}

/// GEOS RayCrossingCounter.
fn ring_locate(ring: &[P], p: P) -> Loc {
    let r = closed(ring);
    let mut count = 0;
    for w in r.windows(2) {
        let (p1, p2) = (w[0], w[1]);
        if p1[0] < p[0] && p2[0] < p[0] {
            continue;
        }
        if p[0] == p2[0] && p[1] == p2[1] {
            return Loc::Boundary;
        }
        if p1[1] == p[1] && p2[1] == p[1] {
            let (mn, mx) = if p1[0] > p2[0] { (p2[0], p1[0]) } else { (p1[0], p2[0]) };
            if p[0] >= mn && p[0] <= mx {
                return Loc::Boundary;
            }
            continue;
        }
        if (p1[1] > p[1] && p2[1] <= p[1]) || (p2[1] > p[1] && p1[1] <= p[1]) {
            let mut o = orient(p1, p2, p);
            if o == 0 {
                return Loc::Boundary;
            }
            if p2[1] < p1[1] {
                o = -o;
            }
            if o == 1 {
                count += 1;
            }
        }
    }
    if count % 2 == 1 {
        Loc::Interior
    } else {
        Loc::Exterior
    }
}

fn seg_bbox_overlap(a: P, b: P, c: P, d: P) -> bool {
    a[0].min(b[0]) <= c[0].max(d[0])
        && c[0].min(d[0]) <= a[0].max(b[0])
        && a[1].min(b[1]) <= c[1].max(d[1])
        && c[1].min(d[1]) <= a[1].max(b[1])
}

fn on_segment(a: P, b: P, p: P) -> bool {
    p[0] >= a[0].min(b[0]) && p[0] <= a[0].max(b[0]) && p[1] >= a[1].min(b[1]) && p[1] <= a[1].max(b[1])
}

/// How two segments meet: None, or Some(true) if only at the given shared vertex.
fn segments_intersect(a: P, b: P, c: P, d: P) -> bool {
    if !seg_bbox_overlap(a, b, c, d) {
        return false;
    }
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);
    if o1 != o2 && o3 != o4 && o1 != 0 && o2 != 0 && o3 != 0 && o4 != 0 {
        return true;
    }
    (o1 == 0 && on_segment(a, b, c))
        || (o2 == 0 && on_segment(a, b, d))
        || (o3 == 0 && on_segment(c, d, a))
        || (o4 == 0 && on_segment(c, d, b))
}

/// Adjacent segments a-b, b-c meet only at b.
fn adjacent_ok(a: P, b: P, c: P) -> bool {
    // collinear and folding back onto itself overlaps
    if orient(a, b, c) == 0 {
        let u = [b[0] - a[0], b[1] - a[1]];
        let v = [c[0] - b[0], c[1] - b[1]];
        return u[0] * v[0] + u[1] * v[1] > 0.0;
    }
    true
}

fn dedupe(pts: &[P]) -> Vec<P> {
    let mut out: Vec<P> = vec![];
    for p in pts {
        if out.last() != Some(p) {
            out.push(*p);
        }
    }
    out
}

/// shapely `LineString(pts).is_simple` (a closed one may touch itself only at its ends).
pub fn line_is_simple(pts: &[P]) -> bool {
    let pts = dedupe(pts);
    let n = pts.len();
    if n < 3 {
        return true;
    }
    let closed = pts[0] == pts[n - 1];
    simple_check(&pts, closed)
}

/// shapely `LinearRing(pts).is_simple` (pts open or closed).
pub fn ring_is_simple(pts: &[P]) -> bool {
    let mut pts = dedupe(pts);
    if pts.len() >= 2 && pts[0] == pts[pts.len() - 1] {
        pts.pop();
    }
    if pts.len() < 3 {
        return false;
    }
    let c = closed(&pts);
    simple_check(&c, true)
}

fn simple_check(pts: &[P], closed: bool) -> bool {
    let m = pts.len() - 1; // segments
    for i in 0..m {
        let (a, b) = (pts[i], pts[i + 1]);
        for j in (i + 1)..m {
            let (c, d) = (pts[j], pts[j + 1]);
            let adjacent = j == i + 1;
            let wrap = closed && i == 0 && j == m - 1;
            if adjacent {
                if !adjacent_ok(a, b, d) {
                    return false;
                }
                // a non-shared touch (e.g. a on c-d) is still an intersection
                if orient(c, d, a) == 0 && on_segment(c, d, a) && a != c {
                    return false;
                }
                continue;
            }
            if wrap {
                if !adjacent_ok(c, d, b) {
                    return false;
                }
                if orient(c, d, b) == 0 && on_segment(c, d, b) && b != d {
                    return false;
                }
                continue;
            }
            if segments_intersect(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square() {
        let sq = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let p = Polygon::new(&sq);
        assert!(p.contains([0.5, 0.5]));
        assert!(!p.contains([1.0, 0.5]));
        assert!(!p.contains([1.5, 0.5]));
        assert_eq!(p.representative_point(), [0.5, 0.5]);
        assert_eq!(p.centroid(), [0.5, 0.5]);
        assert!(ring_is_simple(&sq));
        assert!(!ring_is_simple(&[[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]]));
        assert!(line_is_simple(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]]));
        assert!(!line_is_simple(&[[0.0, 0.0], [2.0, 0.0], [1.0, 1.0], [1.0, -1.0]]));
    }
}
