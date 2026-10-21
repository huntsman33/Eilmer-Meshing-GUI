//! Block faces evaluated as Eilmer does: one path, or a Polyline whose breakpoints are
//! segment lengths (Line and Arc exact; others, ReversedPath and SubRangedPath estimated
//! with 100 chords, as Eilmer's Path.length does); and the Coons patch over four faces.

use crate::document::{CurveRef, Document, Face};
use crate::topology::Topology;

/// Anything that holds the sketch's curves by id.
pub trait Curves {
    fn curve_by_id(&self, id: &str) -> &Curve;
}

impl Curves for Document {
    fn curve_by_id(&self, id: &str) -> &Curve {
        &self.curves[id]
    }
}

impl Curves for Topology {
    fn curve_by_id(&self, id: &str) -> &Curve {
        &self.curves[id]
    }
}
use crate::entities::Curve;
use crate::geom::{self, PATH_LENGTH_CHORDS};
use crate::num::{self, c_hypot, linspace, np_sum, py_sum, searchsorted_left, P};

pub fn chord_length(curve: &Curve, a: f64, b: f64) -> f64 {
    let ts: Vec<f64> = linspace(0.0, 1.0, PATH_LENGTH_CHORDS + 1).iter().map(|t| a + (b - a) * t).collect();
    let pts = geom::points_at(curve, &ts);
    let segs: Vec<f64> = pts.windows(2).map(|w| c_hypot(w[1][0] - w[0][0], w[1][1] - w[0][1])).collect();
    np_sum(&segs)
}

/// A block face (list of CurveRefs) evaluated at parameters.
#[derive(Clone)]
pub struct FaceEval {
    pieces: Vec<(Curve, f64, f64)>,
    tv: Vec<f64>,
}

impl FaceEval {
    pub fn new(doc: &(impl Curves + ?Sized), refs: &[CurveRef]) -> FaceEval {
        let mut pieces = vec![];
        let mut lengths = vec![];
        for r in refs {
            let c = doc.curve_by_id(&r.id).clone();
            let (a, b) = if r.reversed { (r.t1, r.t0) } else { (r.t0, r.t1) };
            lengths.push(if r.full() && !r.reversed { geom::path_length(&c) } else { chord_length(&c, a, b) });
            pieces.push((c, a, b));
        }
        let total = py_sum(&lengths);
        let mut tv: Vec<f64> = num::cumsum(&lengths).iter().map(|v| v / total).collect();
        if let Some(last) = tv.last_mut() {
            *last = 1.0;
        }
        FaceEval { pieces, tv }
    }

    pub fn at(&self, t: f64) -> P {
        let t = num::clip(t, 0.0, 1.0);
        if self.pieces.len() == 1 {
            let (c, a, b) = &self.pieces[0];
            return geom::point_at(c, a + (b - a) * t);
        }
        let seg = searchsorted_left(&self.tv, t).min(self.tv.len() - 1);
        let prev = if seg == 0 { 0.0 } else { self.tv[seg - 1] };
        let local = (t - prev) / (self.tv[seg] - prev);
        let (c, a, b) = &self.pieces[seg];
        geom::point_at(c, a + (b - a) * local)
    }

    pub fn eval(&self, ts: &[f64]) -> Vec<P> {
        if self.pieces.len() == 1 {
            let (c, a, b) = &self.pieces[0];
            let us: Vec<f64> = ts.iter().map(|&t| a + (b - a) * num::clip(t, 0.0, 1.0)).collect();
            return geom::points_at(c, &us);
        }
        ts.iter().map(|&t| self.at(t)).collect()
    }
}

/// The four faces of a block, by Face index.
pub struct Faces(pub [FaceEval; 4]);

impl Faces {
    pub fn new(doc: &(impl Curves + ?Sized), sides: &[Vec<CurveRef>; 4]) -> Faces {
        Faces([
            FaceEval::new(doc, &sides[0]),
            FaceEval::new(doc, &sides[1]),
            FaceEval::new(doc, &sides[2]),
            FaceEval::new(doc, &sides[3]),
        ])
    }

    pub fn get(&self, f: Face) -> &FaceEval {
        &self.0[f.index()]
    }
}

/// Coons patch points at parameters (r[k], s[k]) (Eilmer CoonsPatch).
pub fn coons(f: &Faces, r: &[f64], s: &[f64]) -> Vec<P> {
    let south = f.get(Face::South);
    let north = f.get(Face::North);
    let sv = south.eval(r);
    let nv = north.eval(r);
    let wv = f.get(Face::West).eval(s);
    let ev = f.get(Face::East).eval(s);
    let p00 = south.at(0.0);
    let p10 = south.at(1.0);
    let p01 = north.at(0.0);
    let p11 = north.at(1.0);
    (0..r.len())
        .map(|k| {
            let (r, s) = (r[k], s[k]);
            let mut out = [0.0; 2];
            for c in 0..2 {
                let a = (1.0 - r) * wv[k][c] + r * ev[k][c] + (1.0 - s) * sv[k][c] + s * nv[k][c];
                let b = (1.0 - r) * (1.0 - s) * p00[c] + r * (1.0 - s) * p10[c] + (1.0 - r) * s * p01[c] + r * s * p11[c];
                out[c] = a - b;
            }
            out
        })
        .collect()
}
