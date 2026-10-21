//! Quick trim (as in NX): hover a curve to highlight the piece between the nearest points where
//! it meets other curves (or its ends); click to delete it.

use std::cell::RefCell;
use std::collections::HashMap;

use emesh_core::geom;
use emesh_core::num::P;
use emesh_core::topology::curve_params;

use super::{Ctx, Tool, ToolEvent};
use crate::canvas::Draw;
use crate::theme::{col, Dash};

#[derive(Default)]
pub struct TrimTool {
    target: Option<(String, f64, f64)>,
    cache: RefCell<(u64, HashMap<String, Vec<f64>>)>,
}

impl TrimTool {
    fn params(&self, c: &Ctx, cid: &str) -> Vec<f64> {
        let mut cache = self.cache.borrow_mut();
        if cache.0 != c.model.revision {
            *cache = (c.model.revision, HashMap::new());
        }
        cache.1.entry(cid.to_string()).or_insert_with(|| curve_params(&c.model.doc, cid)).clone()
    }

    /// (curve id, ta, tb) of the piece under the cursor.
    fn piece_at(&self, c: &Ctx, world: P) -> Option<(String, f64, f64)> {
        let cid = c.pick_curve(world, None)?;
        let ts = self.params(c, &cid);
        let (t, _, _) = geom::closest_param(&c.model.doc.curves[&cid], world);
        let k = (0..ts.len() - 1).filter(|&i| ts[i] <= t).max()?;
        Some((cid, ts[k], ts[k + 1]))
    }
}

impl Tool for TrimTool {
    fn name(&self) -> &'static str {
        "trim"
    }
    fn label(&self) -> &'static str {
        "Quick trim"
    }
    fn snaps(&self) -> bool {
        false
    }
    fn takes_input(&self) -> bool {
        false
    }
    fn reset(&mut self) {
        self.target = None;
    }
    fn moved(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        self.target = self.piece_at(c, ev.world);
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if let Some((cid, ta, tb)) = self.piece_at(c, ev.world) {
            if let Err(e) = c.model.trim(&cid, ta, tb) {
                c.message(format!("Cannot trim: {e}."));
            }
        }
        self.target = self.piece_at(c, ev.world);
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Quick trim: click the piece of a curve to delete it, up to where it meets other curves.".into()
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        let Some((cid, ta, tb)) = &self.target else { return };
        let Some(cv) = c.model.doc.curves.get(cid) else { return };
        if let Ok(piece) = geom::subcurve(cv, *ta, *tb) {
            d.curve(&piece, col::trim(), 3.0, Some(Dash::Solid));
        }
    }
}
