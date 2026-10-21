//! Dimension tool (as in NX): click a line for its length or an arc for its sweep angle, move
//! to place the annotation, click. Click a line and then a second line for the distance between
//! them (parallel lines) or the angle between them; the angle shown is the one in the sector the
//! annotation is placed in. Typing a value while placing sets it and drives the curve. Click an
//! existing dimension to edit its value.

use emesh_core::dimensions;
use emesh_core::document::{Dimension, Document};
use emesh_core::entities::Curve;
use emesh_core::geom;
use emesh_core::num::P;
use emesh_core::typed_input::single_number;

use super::{AppEvent, Ctx, Tool, ToolEvent};
use crate::canvas::Draw;
use crate::model::SketchModel;
use crate::theme::col;

/// Annotation offset that puts a dimension of this curve under the cursor.
pub fn offset_at(kind: &str, curve: &Curve, world: P) -> f64 {
    if kind == "length" {
        let (p0, p1) = curve.ends_lp();
        let d = [p1[0] - p0[0], p1[1] - p0[1]];
        let l = d[0].hypot(d[1]);
        let n = [-d[1] / l, d[0] / l];
        return (world[0] - p0[0]) * n[0] + (world[1] - p0[1]) * n[1];
    }
    let (centre, r, _, _) = geom::arc_geom(curve);
    (world[0] - centre[0]).hypot(world[1] - centre[1]) - r
}

/// Set dim's placement so its annotation is under world (a two-line angle may change sector).
pub fn place_at(doc: &Document, dim: &mut Dimension, world: P) {
    let a = &doc.curves[&dim.curve];
    match &dim.curve2 {
        None => dim.offset = offset_at(&dim.kind, a, world),
        Some(_) if dim.kind == "distance" => {
            let (p0, _) = a.ends_lp();
            let u = dimensions::direction(a).unwrap_or([1.0, 0.0]);
            dim.offset = (world[0] - p0[0]) * u[0] + (world[1] - p0[1]) * u[1];
        }
        Some(c2) => {
            let (o, s) = dimensions::angle_placement(a, &doc.curves[c2], world);
            dim.offset = o;
            dim.sector = Some(s);
        }
    }
}

#[derive(Default)]
pub struct DimensionTool {
    /// the dimension being placed (not yet in the document)
    dim: Option<Dimension>,
}

impl DimensionTool {
    fn alive(&self, c: &Ctx) -> bool {
        self.dim.as_ref().is_some_and(|d| d.curve_ids().iter().all(|i| c.model.doc.curves.contains_key(*i)))
    }

    fn place(&mut self, c: &mut Ctx) -> Option<String> {
        let d = self.dim.take()?;
        match c.model.add_dimension(&d.curve, d.offset, d.curve2.as_deref(), d.sector) {
            Ok(did) => Some(did),
            Err(e) => {
                c.message(e);
                None
            }
        }
    }
}

fn preview(kind: &str, curve: &str, offset: f64, curve2: Option<String>) -> Dimension {
    Dimension { id: "_preview".into(), kind: kind.into(), curve: curve.into(), offset, curve2, sector: None }
}

impl Tool for DimensionTool {
    fn name(&self) -> &'static str {
        "dimension"
    }
    fn label(&self) -> &'static str {
        "Dimension"
    }
    fn snaps(&self) -> bool {
        false
    }
    fn hover_curves(&self) -> bool {
        true
    }
    fn reset(&mut self) {
        self.dim = None;
    }
    fn busy(&self) -> bool {
        self.dim.is_some()
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if self.dim.is_none() {
            if let Some(did) = c.pick_dimension(ev.screen) {
                c.events.push(AppEvent::EditEntity(did));
                return;
            }
            let Some(cid) = c.pick_curve(ev.world, None) else { return };
            let Some(kind) = SketchModel::dimension_kind(&c.model.doc.curves[&cid]) else {
                c.message("Dimensions apply to lines (length) and arcs (angle).");
                return;
            };
            let mut d = preview(kind, &cid, 0.0, None);
            place_at(&c.model.doc, &mut d, ev.world);
            self.dim = Some(d);
            return;
        }
        if !self.alive(c) {
            self.reset();
            return;
        }
        let d = self.dim.clone().unwrap();
        if d.curve2.is_none() && c.model.doc.curves[&d.curve].is_line() {
            let others: Vec<String> = c.model.doc.curves.keys().filter(|k| **k != d.curve).cloned().collect();
            if let Some(other) = c.pick_curve(ev.world, Some(&others)) {
                if let Some(kind) = c.model.two_line_kind(&d.curve, &other) {
                    let mut nd = preview(kind, &d.curve, 0.0, Some(other));
                    place_at(&c.model.doc, &mut nd, ev.world);
                    self.dim = Some(nd);
                    return;
                }
            }
        }
        self.place(c);
    }
    fn moved(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if self.alive(c) {
            let mut d = self.dim.take().unwrap();
            place_at(&c.model.doc, &mut d, ev.world);
            self.dim = Some(d);
        }
    }
    fn accept_text(&mut self, c: &mut Ctx, text: &str) -> Result<(), String> {
        let Some(d) = &self.dim else { return Err("click a line or arc first".into()) };
        let (kind, two) = (d.kind.clone(), d.curve2.is_some());
        let value = if kind == "length" || kind == "distance" {
            match c.parse_length(text) {
                Some(v) if v > 0.0 => v,
                _ => return Err(format!("type a positive {kind}")),
            }
        } else if two {
            match single_number(text) {
                Some(v) if 0.0 < v && v < 180.0 => v,
                _ => return Err("type an angle between 0 and 180 degrees".into()),
            }
        } else {
            match single_number(text) {
                Some(v) if 0.0 < v.abs() && v.abs() < 360.0 => v.abs(),
                _ => return Err("type an angle between 0 and 360 degrees".into()),
            }
        };
        let Some(did) = self.place(c) else { return Ok(()) };
        c.model.set_dimension_value(&did, value)
    }
    fn prompt(&self, _c: &Ctx) -> String {
        match &self.dim {
            None => "Line or arc to dimension (or click a dimension to edit it)".into(),
            Some(d) if d.curve2.is_some() => {
                format!("Place the dimension, or type the {}", if d.kind == "distance" { "distance" } else { "angle (deg)" })
            }
            Some(d) if d.kind == "length" => "Place the dimension, click a second line (distance or angle), or type the length".into(),
            Some(_) => "Place the dimension, or type the angle (deg)".into(),
        }
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Dimension: click a line (length) or arc (angle), move to place it and click. Click a second line instead for \
         the distance between parallel lines or the angle between two lines (place it in the sector you want). Type a \
         value while placing to set it; the second line moves. Double-click a dimension later to change it."
            .into()
    }
    fn paint_overlay(&self, d: &mut Draw, c: &Ctx) {
        if self.alive(c) {
            c.canvas.paint_dimension(d, &c.model.doc, self.dim.as_ref().unwrap(), col::preview(), None);
        }
    }
}
