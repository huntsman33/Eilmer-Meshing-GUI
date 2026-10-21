//! Cell quality of structured grids (Eilmer's mesh, or the preview until it arrives).
//!
//!   skewness      equiangle skewness max((qmax - 90) / 90, (90 - qmin) / 90) of the cell's
//!                 corner angles: 0 for a rectangle, 1 for a degenerate cell
//!   aspect ratio  longest edge / shortest edge
//!   inverted      a corner turning the wrong way for its block (a folded or tangled cell), or
//!                 a cell with no area; counted with skewness 1
//!
//! Blocks whose edges carry boundary-layer spacing are long and thin on purpose, so their
//! aspect ratio is not checked.

use std::collections::HashSet;

use indexmap::IndexMap;

use emesh_core::document::Block;
use emesh_core::num::{c_hypot, np_sum};
use emesh_core::pyfmt;

use crate::preview::Grid;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub skew_warn: f64,
    pub skew_bad: f64,
    pub ar_warn: f64,
}

pub const DEFAULT_THRESHOLDS: Thresholds = Thresholds { skew_warn: 0.6, skew_bad: 0.85, ar_warn: 20.0 };

/// Per-cell metrics, (i, j) at [i * (njv - 1) + j].
#[derive(Clone, Debug)]
pub struct CellMetrics {
    pub ni: usize,
    pub nj: usize,
    pub skew: Vec<f64>,
    pub ar: Vec<f64>,
    pub inverted: Vec<bool>,
}

pub fn cell_metrics(g: &Grid) -> CellMetrics {
    let (ni, nj) = (g.niv - 1, g.njv - 1);
    let n = ni * nj;
    let mut cross = vec![[0.0f64; 4]; n];
    let mut angle = vec![[0.0f64; 4]; n];
    let mut edges = vec![[0.0f64; 4]; n];
    for i in 0..ni {
        for j in 0..nj {
            let k = i * nj + j;
            let (a, b, c, d) = (g.at(i, j), g.at(i + 1, j), g.at(i + 1, j + 1), g.at(i, j + 1));
            for (q, (v, nx, pv)) in [(a, b, d), (b, c, a), (c, d, b), (d, a, c)].into_iter().enumerate() {
                let e1 = [nx[0] - v[0], nx[1] - v[1]];
                let e2 = [pv[0] - v[0], pv[1] - v[1]];
                let cr = e1[0] * e2[1] - e1[1] * e2[0];
                let dot = e1[0] * e2[0] + e1[1] * e2[1];
                cross[k][q] = cr;
                angle[k][q] = cr.abs().atan2(dot).to_degrees();
            }
            for (q, (p, r)) in [(a, b), (b, c), (c, d), (d, a)].into_iter().enumerate() {
                edges[k][q] = c_hypot(r[0] - p[0], r[1] - p[1]);
            }
        }
    }
    let area: Vec<f64> = cross.iter().map(|c| 0.5 * (c[0] + c[2])).collect();
    let sign = if np_sum(&area) >= 0.0 { 1.0 } else { -1.0 };
    let mut out = CellMetrics { ni, nj, skew: vec![0.0; n], ar: vec![0.0; n], inverted: vec![false; n] };
    for k in 0..n {
        let inv = cross[k].iter().any(|c| sign * c <= 0.0);
        let amax = angle[k].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let amin = angle[k].iter().cloned().fold(f64::INFINITY, f64::min);
        let s = ((amax - 90.0) / 90.0).max((90.0 - amin) / 90.0);
        out.inverted[k] = inv;
        out.skew[k] = if inv { 1.0 } else { s.clamp(0.0, 1.0) };
        let emax = edges[k].iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let emin = edges[k].iter().cloned().fold(f64::INFINITY, f64::min);
        out.ar[k] = if emin > 0.0 { emax / emin } else { f64::INFINITY };
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockQuality {
    pub id: String,
    pub cells: usize,
    pub max_skew: f64,
    pub max_ar: f64,
    /// cells above the warning skewness
    pub skew_warn: usize,
    /// cells above the bad skewness
    pub skew_bad: usize,
    /// cells above the aspect-ratio warning (0 when not checked)
    pub ar_warn: usize,
    pub inverted: usize,
    pub check_ar: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Warn,
    Bad,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Bad => "bad",
            Level::Error => "error",
        }
    }
}

impl BlockQuality {
    pub fn level(&self) -> Level {
        if self.inverted > 0 {
            Level::Error
        } else if self.skew_bad > 0 {
            Level::Bad
        } else if self.skew_warn > 0 || self.ar_warn > 0 {
            Level::Warn
        } else {
            Level::Ok
        }
    }
}

#[derive(Clone, Debug)]
pub struct MeshQuality {
    pub revision: u64,
    pub thresholds: Thresholds,
    pub blocks: IndexMap<String, BlockQuality>,
    pub metrics: IndexMap<String, CellMetrics>,
}

impl MeshQuality {
    pub fn level(&self) -> Level {
        self.blocks.values().map(|b| b.level()).max().unwrap_or(Level::Ok)
    }

    pub fn flagged(&self, levels: &[Level]) -> Vec<String> {
        self.blocks.iter().filter(|(_, b)| levels.contains(&b.level())).map(|(id, _)| id.clone()).collect()
    }

    /// One line naming the worst problems.
    pub fn summary(&self, limit: usize) -> String {
        let t = &self.thresholds;
        let mut worst: Vec<&BlockQuality> = self.blocks.values().collect();
        worst.sort_by(|a, b| {
            (std::cmp::Reverse(a.level()), -a.max_skew)
                .partial_cmp(&(std::cmp::Reverse(b.level()), -b.max_skew))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut parts = vec![];
        for b in worst {
            if b.level() == Level::Ok {
                continue;
            }
            let mut bits = vec![];
            if b.inverted > 0 {
                bits.push(format!("{} inverted cell{}", b.inverted, if b.inverted > 1 { "s" } else { "" }));
            }
            if b.skew_bad > 0 {
                bits.push(format!("{} cells skewness > {} (max {})", b.skew_bad, pyfmt::g(t.skew_bad, 6), pyfmt::f(b.max_skew, 2)));
            } else if b.skew_warn > 0 {
                bits.push(format!("{} cells skewness > {} (max {})", b.skew_warn, pyfmt::g(t.skew_warn, 6), pyfmt::f(b.max_skew, 2)));
            }
            if b.ar_warn > 0 {
                bits.push(format!("{} cells aspect ratio > {} (max {})", b.ar_warn, pyfmt::g(t.ar_warn, 6), pyfmt::g(b.max_ar, 3)));
            }
            parts.push(format!("{}: {}", b.id, bits.join(", ")));
        }
        let more = parts.len() as i64 - limit as i64;
        let mut s = parts.iter().take(limit).cloned().collect::<Vec<_>>().join("; ");
        if more > 0 {
            s += &format!("; and {more} more block(s)");
        }
        s
    }
}

/// Ids of blocks with boundary-layer (first cell) spacing on an edge: thin cells by design.
pub fn intentional_aspect(blocks: &[Block]) -> HashSet<String> {
    blocks
        .iter()
        .filter(|b| b.cluster.values().any(|c| c.kind() == "Growth" || c.kind() == "Grade"))
        .map(|b| b.id.clone())
        .collect()
}

pub fn assess(revision: u64, grids: &[(String, Grid)], t: Thresholds, skip_ar: &HashSet<String>) -> MeshQuality {
    let mut q = MeshQuality { revision, thresholds: t, blocks: IndexMap::new(), metrics: IndexMap::new() };
    for (bid, g) in grids {
        if g.niv < 2 || g.njv < 2 {
            continue;
        }
        let m = cell_metrics(g);
        let check_ar = !skip_ar.contains(bid);
        let finite: Vec<f64> = m.ar.iter().copied().filter(|v| v.is_finite()).collect();
        let bq = BlockQuality {
            id: bid.clone(),
            cells: m.skew.len(),
            max_skew: m.skew.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            max_ar: if finite.is_empty() { f64::INFINITY } else { finite.iter().cloned().fold(f64::NEG_INFINITY, f64::max) },
            skew_warn: m.skew.iter().filter(|s| **s > t.skew_warn).count(),
            skew_bad: m.skew.iter().filter(|s| **s > t.skew_bad).count(),
            ar_warn: if check_ar { m.ar.iter().filter(|a| **a > t.ar_warn).count() } else { 0 },
            inverted: m.inverted.iter().filter(|v| **v).count(),
            check_ar,
        };
        q.blocks.insert(bid.clone(), bq);
        q.metrics.insert(bid.clone(), m);
    }
    q
}
