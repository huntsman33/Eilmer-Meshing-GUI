//! Instant mesh preview, reproducing Eilmer's own construction so it matches what
//! `lmr custom-script` writes (to round-off) for CoonsPatch blocks:
//!
//!   faces      one path, or a Polyline whose breakpoints are segment lengths (face.rs)
//!   clustering RobertsFunction or boundary-layer growth per face, blended into (r, s) as in
//!              StructuredGrid.make_grid_from_surface
//!   smoothing  the same Jacobi Laplace sweep the generated Lua runs
//!
//! AOPatch blocks preview as CoonsPatch; Eilmer's result replaces the preview.

use emesh_core::document::{Block, Cluster, Document, Face};
use emesh_core::face::{coons, Faces};
use emesh_core::num::{linspace, P};
use emesh_core::spacing::{concrete_cluster, growth_value};

/// Structured grid vertices, (i, j) at `pts[i * njv + j]` (z = 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub niv: usize,
    pub njv: usize,
    pub pts: Vec<P>,
}

impl Grid {
    pub fn at(&self, i: usize, j: usize) -> P {
        self.pts[i * self.njv + j]
    }

    pub fn cells(&self) -> usize {
        (self.niv.saturating_sub(1)) * (self.njv.saturating_sub(1))
    }
}

/// Eilmer RobertsFunction (geom/misc/univariatefunctions.d).
pub fn roberts(t: f64, end0: bool, end1: bool, beta: f64) -> f64 {
    if !(end0 || end1) || beta <= 1.0 {
        return t;
    }
    let alpha = if end0 && end1 { 0.5 } else { 0.0 };
    let reverse = end0 && !end1;
    let t = if reverse { 1.0 - t } else { t };
    let lam = ((beta + 1.0) / (beta - 1.0)).powf((t - alpha) / (1.0 - alpha));
    let tbar = ((beta + 2.0 * alpha) * lam - beta + 2.0 * alpha) / ((2.0 * alpha + 1.0) * (1.0 + lam));
    if reverse {
        1.0 - tbar
    } else {
        tbar
    }
}

fn interp(x: f64, xp: &[f64], fp: &[f64]) -> f64 {
    if x <= xp[0] {
        return fp[0];
    }
    if x >= xp[xp.len() - 1] {
        return fp[fp.len() - 1];
    }
    let k = emesh_core::num::searchsorted_left(xp, x).max(1);
    let (x0, x1) = (xp[k - 1], xp[k]);
    let s = (fp[k] - fp[k - 1]) / (x1 - x0);
    s * (x - x0) + fp[k - 1]
}

/// Vertex parameters for a concrete clustering spec.
pub fn distribute(spec: Option<&Cluster>, n: usize) -> Vec<f64> {
    let t = linspace(0.0, 1.0, n);
    let Some(spec) = spec else { return t };
    match spec.kind() {
        "RobertsFunction" => {
            let (e0, e1, beta) = (spec.b("end0"), spec.b("end1"), spec.f("beta").unwrap_or(1.0));
            t.iter().map(|&v| roberts(v, e0, e1, beta)).collect()
        }
        "growth" => {
            let r = spec.f("r").unwrap();
            let m = spec.f("n").unwrap() as usize;
            let rev = spec.b("reverse");
            t.iter().map(|&v| growth_value(r, m, v, rev)).collect()
        }
        "table" => {
            let x: Vec<f64> = spec.0["x"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            if x.len() == n {
                x
            } else {
                let xp = linspace(0.0, 1.0, x.len());
                t.iter().map(|&v| interp(v, &xp, &x)).collect()
            }
        }
        _ => t,
    }
}

/// Vertices of a block, as Eilmer would generate them.
pub fn block_grid(doc: &Document, block: &Block) -> Grid {
    let f = Faces::new(doc, &block.sides);
    let (niv, njv) = (block.niv, block.njv);
    let rs = distribute(concrete_cluster(doc, block, Face::South).as_ref(), niv);
    let rn = distribute(concrete_cluster(doc, block, Face::North).as_ref(), niv);
    let sw = distribute(concrete_cluster(doc, block, Face::West).as_ref(), njv);
    let se = distribute(concrete_cluster(doc, block, Face::East).as_ref(), njv);
    let r = linspace(0.0, 1.0, niv);
    let s = linspace(0.0, 1.0, njv);
    let mut rdash = Vec::with_capacity(niv * njv);
    let mut sdash = Vec::with_capacity(niv * njv);
    for i in 0..niv {
        for j in 0..njv {
            sdash.push((1.0 - r[i]) * sw[j] + r[i] * se[j]);
            rdash.push((1.0 - s[j]) * rs[i] + s[j] * rn[i]);
        }
    }
    let pts = coons(&f, &rdash, &sdash);
    let mut g = Grid { niv, njv, pts };
    if block.smoothed() {
        let sm = block.smoothing.as_ref().unwrap();
        let nsweeps = sm.get("nsweeps").and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
        let tol = sm.get("tol").and_then(|v| v.as_f64()).unwrap_or(0.0);
        laplace_smooth(&mut g, nsweeps, tol);
    }
    g
}

/// Jacobi sweep matching the generated Lua; boundary vertices fixed.
pub fn laplace_smooth(g: &mut Grid, nsweeps: usize, tol: f64) {
    let (ni, nj) = (g.niv, g.njv);
    if ni < 3 || nj < 3 {
        return;
    }
    let mut new = vec![[0.0; 2]; (ni - 2) * (nj - 2)];
    for _ in 0..nsweeps {
        let mut change = 0.0f64;
        let v = &g.pts;
        for i in 1..ni - 1 {
            for j in 1..nj - 1 {
                let k = (i - 1) * (nj - 2) + (j - 1);
                for c in 0..2 {
                    let x = 0.25 * (v[(i + 1) * nj + j][c] + v[(i - 1) * nj + j][c] + v[i * nj + j + 1][c] + v[i * nj + j - 1][c]);
                    let d = (x - v[i * nj + j][c]).abs();
                    if d > change {
                        change = d;
                    }
                    new[k][c] = x;
                }
            }
        }
        for i in 1..ni - 1 {
            for j in 1..nj - 1 {
                g.pts[i * nj + j] = new[(i - 1) * (nj - 2) + (j - 1)];
            }
        }
        if change < tol {
            break;
        }
    }
}
