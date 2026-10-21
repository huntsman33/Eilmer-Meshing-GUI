//! Offset tool (as NX's Offset Curve).
//!
//!   click curves   pick them (click a picked one again to drop it); anything selected
//!                  beforehand is included
//!   Enter          the offset dialog: distance and side, previewed live; OK makes the offset
//!                  curves (the originals stay), in one undo step
//!
//! Connected curves are offset as chains: closed shapes grow (Reverse shrinks them), open
//! chains go to the left of their direction (Reverse: the right). An arrow on each chain shows
//! the side.

use egui::{Pos2, Vec2};

use emesh_core::entities::Curve;
use emesh_core::num::P;
use emesh_core::offset::{find_chains, offset_chains, side_markers};

use super::{draw_preview, Ctx, Tool, ToolEvent};
use crate::canvas::Draw;
use crate::theme::{col, Dash};

const ARROW_PX: f32 = 26.0;

struct Dialog {
    ids: Vec<String>,
    curves: Vec<Curve>,
    summary: String,
    distance: String,
    reverse: bool,
    source_ref: bool,
    error: String,
    preview: Vec<Curve>,
    markers: Vec<(P, P)>,
    dirty: bool,
    /// opened this frame: the key that opened it must not also press OK
    fresh: bool,
}

#[derive(Default)]
pub struct OffsetTool {
    /// metres; remembered for the session
    last_distance: Option<f64>,
    last_source_ref: bool,
    dialog: Option<Dialog>,
}

impl OffsetTool {
    fn picked(c: &Ctx) -> Vec<String> {
        c.model.doc.curves.keys().filter(|k| c.model.selection.contains(*k)).cloned().collect()
    }

    fn distance(c: &Ctx, d: &Dialog) -> Result<f64, String> {
        let v: f64 = d.distance.trim().parse().map_err(|_| "the distance must be a number".to_string())?;
        if !(v > 0.0) {
            return Err("the distance must be positive".into());
        }
        Ok(c.canvas.units.from_display(v))
    }

    fn refresh(c: &Ctx, d: &mut Dialog) {
        d.markers = side_markers(&d.curves, d.reverse);
        match Self::distance(c, d) {
            Err(e) => {
                d.error = capital(&e);
                d.preview.clear();
            }
            Ok(dist) => match offset_chains(&d.curves, dist, d.reverse) {
                Ok(p) => {
                    d.preview = p;
                    d.error.clear();
                }
                Err(e) => {
                    d.preview.clear();
                    d.error = capital(&e);
                }
            },
        }
        d.dirty = false;
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

impl Tool for OffsetTool {
    fn name(&self) -> &'static str {
        "offset"
    }
    fn label(&self) -> &'static str {
        "Offset"
    }
    fn snaps(&self) -> bool {
        false
    }
    fn hover_curves(&self) -> bool {
        true
    }
    fn takes_input(&self) -> bool {
        false
    }
    fn reset(&mut self) {
        self.dialog = None;
    }
    fn busy(&self) -> bool {
        self.dialog.is_some()
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if self.dialog.is_some() {
            return;
        }
        if let Some(cid) = c.pick_curve(ev.world, None) {
            if !c.model.selection.remove(&cid) {
                c.model.selection.insert(cid);
            }
        }
    }
    fn finish(&mut self, c: &mut Ctx) {
        if self.dialog.is_some() {
            return;
        }
        let ids = Self::picked(c);
        if ids.is_empty() {
            c.message("Offset: click the curves to offset first, then press Enter.");
            return;
        }
        let curves: Vec<Curve> = ids.iter().map(|i| c.model.doc.curves[i].clone()).collect();
        let dist = *self.last_distance.get_or_insert_with(|| c.canvas.grid_step());
        let chains = find_chains(&curves, None);
        let closed = chains.iter().filter(|ch| ch.closed).count();
        let summary = format!(
            "{} curve(s) in {} chain(s): {closed} closed (grow by default), {} open (to the left by default).",
            ids.len(),
            chains.len(),
            chains.len() - closed
        );
        let mut d = Dialog {
            ids,
            curves,
            summary,
            distance: emesh_core::pyfmt::g(c.canvas.units.to_display(dist), 6),
            reverse: false,
            source_ref: self.last_source_ref,
            error: String::new(),
            preview: vec![],
            markers: vec![],
            dirty: true,
            fresh: true,
        };
        Self::refresh(c, &mut d);
        self.dialog = Some(d);
    }
    fn cancel(&mut self, c: &mut Ctx) -> bool {
        if self.dialog.take().is_some() {
            return true;
        }
        if !c.model.selection.is_empty() {
            c.model.selection.clear();
            return true;
        }
        false
    }
    fn prompt(&self, c: &Ctx) -> String {
        format!("Curves to offset ({} picked): then Enter", Self::picked(c).len())
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Offset: click curves (connected ones are offset together), then press Enter to set the distance and side. \
         Closed shapes grow, or shrink with Reverse."
            .into()
    }
    fn paint(&self, d: &mut Draw, _c: &Ctx) {
        let Some(dlg) = &self.dialog else { return };
        if dlg.source_ref {
            for cv in &dlg.curves {
                d.curve(cv, col::preview(), 1.0, Some(Dash::Dashed));
            }
        }
        draw_preview(d, &dlg.preview);
    }
    fn paint_overlay(&self, d: &mut Draw, c: &Ctx) {
        let Some(dlg) = &self.dialog else { return };
        for (p, n) in &dlg.markers {
            let q: Pos2 = c.canvas.view.to_screen(*p);
            let dir = Vec2::new(n[0] as f32, -n[1] as f32); // screen y points down
            let tip = q + dir * ARROW_PX;
            d.screen_line(vec![q, tip], col::preview(), 2.0);
            for ang in [150f32.to_radians(), -150f32.to_radians()] {
                let (cs, sn) = (ang.cos(), ang.sin());
                let w = Vec2::new(dir.x * cs - dir.y * sn, dir.x * sn + dir.y * cs);
                d.screen_line(vec![tip, tip + w * 8.0], col::preview(), 2.0);
            }
        }
    }
    fn windows(&mut self, ctx: &egui::Context, c: &mut Ctx) {
        let Some(d) = self.dialog.as_mut() else { return };
        let mut close: Option<bool> = None;
        crate::dialogs::modal(ctx, "Offset", |ui| {
            ui.label(&d.summary);
            egui::Grid::new("offset-grid").num_columns(2).show(ui, |ui| {
                ui.label(format!("Distance ({})", c.canvas.units.name));
                if ui.text_edit_singleline(&mut d.distance).changed() {
                    d.dirty = true;
                }
                ui.end_row();
                ui.label("");
                if ui
                    .checkbox(&mut d.reverse, "Reverse direction")
                    .on_hover_text("Closed shapes grow by default (reverse shrinks them); open curves go to the left of their direction (the arrow shows the side).")
                    .changed()
                {
                    d.dirty = true;
                }
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut d.source_ref, "Make the source curves reference lines").on_hover_text(
                    "Turns the picked curves dashed (and clears their boundary conditions) when the offset is made, in the same undo step.",
                );
                ui.end_row();
            });
            if !d.error.is_empty() {
                ui.colored_label(col::error_text(), &d.error);
            }
            close = crate::dialogs::ok_cancel(ui, d.error.is_empty());
        });
        if d.dirty {
            Self::refresh(c, d);
        }
        if d.fresh {
            d.fresh = false;
            close = None;
        }
        match close {
            Some(true) => {
                let d = self.dialog.take().unwrap();
                let Ok(dist) = Self::distance(c, &d) else { return };
                self.last_distance = Some(dist);
                self.last_source_ref = d.source_ref;
                match c.model.offset(&d.ids, dist, d.reverse, d.source_ref) {
                    Ok(new_ids) => {
                        c.message(format!("Offset {} curve(s) by {}: {} new curve(s).", d.ids.len(), c.fmt(dist), new_ids.len()));
                        c.model.set_selection(new_ids);
                    }
                    Err(e) => c.message(format!("Cannot offset: {e}.")),
                }
            }
            Some(false) => self.dialog = None,
            None => {}
        }
    }
    fn modal(&self) -> bool {
        self.dialog.is_some()
    }
}
