//! Boundary-layer spacing: geometric growth from a first cell height, grading, the y+ estimate,
//! and the concrete clustering Lua and the preview use (Python `model/layers.py`, spacing part).

use serde_json::{json, Map, Value};

use crate::blocking::face_length;
use crate::document::{Block, Cluster, Document, Face};
use crate::num::{brentq, linspace};

/// Total height of `cells` cells starting at `first` and growing by `growth`.
pub fn thickness(first: f64, growth: f64, cells: usize) -> f64 {
    if (growth - 1.0).abs() < 1e-12 {
        return first * cells as f64;
    }
    first * (growth.powf(cells as f64) - 1.0) / (growth - 1.0)
}

fn log_expm1(x: f64) -> f64 {
    if x > 30.0 {
        x + (-(-x).exp_m1()).ln()
    } else {
        x.exp_m1().ln()
    }
}

/// Ratio r >= 1 for n cells growing from a first cell a (a fraction of the edge) to fill it;
/// 1 when even uniform cells are smaller than a.
pub fn growth_ratio(a: f64, n: usize) -> Result<f64, String> {
    if a <= 0.0 || n < 1 {
        return Err("the first cell and the cell count must be positive".into());
    }
    if n == 1 || a * n as f64 >= 1.0 - 1e-12 {
        return Ok(1.0);
    }
    let nf = n as f64;
    let f = |q: f64| a.ln() + log_expm1(nf * q.ln_1p()) - q.ln();
    let mut hi = 1.0;
    while f(hi) < 0.0 {
        hi *= 2.0;
    }
    Ok(1.0 + brentq(f, 1e-12, hi, 1e-15, 1e-15, 500)?)
}

/// The growth distribution at t: vertex k of n cells at (r^k - 1) / (r^n - 1).
pub fn growth_value(r: f64, n: usize, t: f64, reverse: bool) -> f64 {
    let t = if reverse { 1.0 - t } else { t };
    let big = r.powf(n as f64);
    let x = (big.powf(t) - 1.0) / (big - 1.0);
    if reverse {
        1.0 - x
    } else {
        x
    }
}

/// Vertex positions (0..1) of n cells that start at `first` (a fraction of the edge) and grow by
/// `growth` until they reach the uniform size that fills the rest.
pub fn graded_values(first: f64, growth: f64, n: usize) -> Result<Vec<f64>, String> {
    if n as f64 * first >= 1.0 {
        return Ok(linspace(0.0, 1.0, n + 1));
    }
    let (mut k, mut filled, mut cell) = (0usize, 0.0f64, first);
    while k < n && (1.0 - filled) / (n - k) as f64 > cell {
        filled += cell;
        cell *= growth;
        k += 1;
    }
    if k == n {
        let r = growth_ratio(first, n)?;
        return Ok(linspace(0.0, 1.0, n + 1).iter().map(|&t| growth_value(r, n, t, false)).collect());
    }
    let mut sizes: Vec<f64> = (0..k).map(|i| first * growth.powf(i as f64)).collect();
    sizes.extend(std::iter::repeat_n((1.0 - filled) / (n - k) as f64, n - k));
    let mut x = vec![0.0];
    x.extend(crate::num::cumsum(&sizes));
    let last = *x.last().unwrap();
    Ok(x.iter().map(|v| v / last).collect())
}

/// Wall spacing for a y+ target, using the turbulent flat-plate skin friction
/// Cf = 0.026 Re_x^(-1/7) at x = length.
pub fn first_cell_height(yplus: f64, velocity: f64, density: f64, viscosity: f64, length: f64) -> Result<f64, String> {
    if [yplus, velocity, density, viscosity, length].iter().any(|v| *v <= 0.0) {
        return Err("all the flow values must be positive".into());
    }
    let re = density * velocity * length / viscosity;
    let cf = 0.026 / re.powf(1.0 / 7.0);
    let u_tau = velocity * (0.5 * cf).sqrt();
    Ok(yplus * viscosity / (density * u_tau))
}

/// A face's clustering as Lua and the preview use it: "Growth" becomes a ratio for the face's
/// actual length and count; "Grade" a table of vertex positions.
pub fn concrete_cluster(doc: &Document, block: &Block, face: Face) -> Option<Cluster> {
    let spec = block.cluster.get(&face)?;
    if spec.0.is_empty() {
        return None;
    }
    let kind = spec.kind();
    if kind != "Growth" && kind != "Grade" {
        return Some(spec.clone());
    }
    let n = block.count(face) - 1;
    let length = face_length(doc, block.side(face));
    let first = spec.f("first").unwrap_or(0.0);
    if kind == "Grade" {
        let mut x = if length > 0.0 {
            graded_values(first / length, spec.f("growth").unwrap_or(1.2), n).unwrap_or_else(|_| linspace(0.0, 1.0, n + 1))
        } else {
            linspace(0.0, 1.0, n + 1)
        };
        if spec.b("reverse") {
            x = x.iter().rev().map(|v| 1.0 - v).collect();
        }
        let mut m = Map::new();
        m.insert("type".into(), json!("table"));
        m.insert("x".into(), Value::Array(x.into_iter().map(|v| json!(v)).collect()));
        return Some(Cluster(m));
    }
    let r = if length > 0.0 { growth_ratio(first / length, n).unwrap_or(1.0) } else { 1.0 };
    if r < 1.0 + 1e-9 {
        return None;
    }
    let mut m = Map::new();
    m.insert("type".into(), json!("growth"));
    m.insert("r".into(), json!(r));
    m.insert("n".into(), json!(n));
    m.insert("reverse".into(), json!(spec.b("reverse")));
    Some(Cluster(m))
}

/// The same clustering seen from the other end of the edge.
pub fn mirror_cluster(spec: &Cluster) -> Cluster {
    let mut out = spec.clone();
    match spec.kind() {
        "RobertsFunction" => {
            let e0 = spec.0.get("end0").cloned().unwrap_or(json!(false));
            let e1 = spec.0.get("end1").cloned().unwrap_or(json!(false));
            out.0.insert("end0".into(), e1);
            out.0.insert("end1".into(), e0);
        }
        "Growth" | "Grade" => {
            out.0.insert("reverse".into(), json!(!spec.b("reverse")));
        }
        _ => {}
    }
    out
}

pub fn mirror_opt(spec: Option<&Cluster>) -> Option<Cluster> {
    spec.map(mirror_cluster)
}
