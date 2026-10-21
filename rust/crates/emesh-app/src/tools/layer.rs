//! Boundary-layer tool.
//!
//!   click block walls     pick them (click again to drop one)
//!   Enter                 cut a boundary layer along the picked walls and re-block
//!   double-click a wall   the layer's settings (first cell / y+, growth, cells); on a wall
//!                         without a layer, the settings for the next one
//!   right-click           apply / settings / remove layer

use egui::{Align2, Shape, Stroke, Vec2};

use emesh_core::document::Face;
use emesh_core::entities::Kind;
use emesh_core::face::FaceEval;
use emesh_core::num::linspace;

use super::block::{face_screen_points, FacePicker};
use super::{AppEvent, Ctx, Tool, ToolEvent};
use crate::canvas::Draw;
use crate::theme::col;

#[derive(Default)]
pub struct LayerTool {
    picker: FacePicker,
    walls: Vec<(String, Face)>,
    hover: Option<(String, Face)>,
    menu_face: Option<(String, Face)>,
}

impl LayerTool {
    fn is_wall(c: &Ctx, bid: &str, face: Face) -> bool {
        let res = c.model.resolution();
        res.block(bid).is_some_and(|b| b.side(face).iter().all(|r| c.model.doc.curves[&r.id].kind == Kind::Geometry))
    }

    fn remove(c: &mut Ctx, lid: &str) {
        match c.model.remove_boundary_layer(lid) {
            Ok(()) => c.message("Boundary layer removed."),
            Err(e) => c.message(format!("Cannot remove it: {e}.")),
        }
    }
}

impl Tool for LayerTool {
    fn name(&self) -> &'static str {
        "layer"
    }
    fn label(&self) -> &'static str {
        "Boundary layer"
    }
    fn snaps(&self) -> bool {
        false
    }
    fn takes_input(&self) -> bool {
        false
    }
    fn reset(&mut self) {
        self.walls.clear();
        self.hover = None;
    }
    fn busy(&self) -> bool {
        !self.walls.is_empty()
    }
    fn moved(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        self.hover = self.picker.face_at(c, ev.screen);
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        let Some(face) = self.picker.face_at(c, ev.screen) else { return };
        if let Some(k) = self.walls.iter().position(|w| *w == face) {
            self.walls.remove(k);
        } else if !Self::is_wall(c, &face.0, face.1) {
            c.message("That is a block split line, not a wall.");
            return;
        } else if c.model.layer_at_face(&face.0, face.1).is_some() {
            c.message("This wall already has a boundary layer: double-click it to change it.");
            return;
        } else {
            self.walls.push(face);
        }
        c.message(format!("{} wall(s) picked: Enter cuts the boundary layer.", self.walls.len()));
    }
    fn double_click(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if let Some(face) = self.picker.face_at(c, ev.screen) {
            c.events.push(AppEvent::LayerSettings(Some(face)));
        }
    }
    fn finish(&mut self, c: &mut Ctx) {
        let res = c.model.resolution();
        let walls: Vec<(String, Face)> = self.walls.drain(..).filter(|(b, _)| res.block(b).is_some()).collect();
        if walls.is_empty() {
            return;
        }
        let d = c.settings.layer;
        match c.model.add_boundary_layer(&walls, d.first, d.growth, d.cells, d.transition) {
            Ok(lid) => {
                let note = c.model.layer_note(&lid);
                c.message(format!(
                    "Boundary layer cut along {} wall(s), tagged as no-slip walls. {}",
                    walls.len(),
                    if note.is_empty() { "Its blocks were re-meshed.".to_string() } else { note }
                ));
            }
            Err(e) => c.message(format!("Cannot cut the boundary layer: {e}.")),
        }
    }
    fn cancel(&mut self, _c: &mut Ctx) -> bool {
        if self.walls.is_empty() {
            return false;
        }
        self.walls.clear();
        true
    }
    fn right_click(&mut self, c: &mut Ctx, ev: &ToolEvent) -> bool {
        self.menu_face = self.picker.face_at(c, ev.screen);
        true
    }
    fn context_menu(&mut self, ui: &mut egui::Ui, c: &mut Ctx) -> bool {
        let mut chosen = false;
        if ui.add_enabled(!self.walls.is_empty(), egui::Button::new(format!("Cut boundary layer on {} wall(s)", self.walls.len()))).clicked() {
            self.finish(c);
            chosen = true;
        }
        match self.menu_face.clone() {
            Some(face) => {
                let lid = c.model.layer_at_face(&face.0, face.1);
                if ui.button(if lid.is_some() { "Layer settings..." } else { "Settings for new layers..." }).clicked() {
                    c.events.push(AppEvent::LayerSettings(Some(face)));
                    chosen = true;
                }
                if let Some(lid) = lid {
                    if ui.button("Remove boundary layer").clicked() {
                        Self::remove(c, &lid);
                        chosen = true;
                    }
                }
            }
            None => {
                if ui.button("Settings for new layers...").clicked() {
                    c.events.push(AppEvent::LayerSettings(None));
                    chosen = true;
                }
            }
        }
        if !self.walls.is_empty() && ui.button("Clear picked walls").clicked() {
            self.walls.clear();
            chosen = true;
        }
        chosen
    }
    fn prompt(&self, _c: &Ctx) -> String {
        if self.walls.is_empty() {
            "Click block walls, then Enter; double-click a wall for settings".into()
        } else {
            format!("{} wall(s) picked; Enter cuts the layer", self.walls.len())
        }
    }
    fn hint(&self, c: &Ctx) -> String {
        let d = c.settings.layer;
        format!(
            "Boundary layer: pick walls and press Enter (next layer: first cell {} m, growth {}, {} cells). Double-click a wall for its settings.",
            emesh_core::pyfmt::g(d.first, 3),
            emesh_core::pyfmt::g(d.growth, 3),
            d.cells
        )
    }
    fn paint_overlay(&self, d: &mut Draw, c: &Ctx) {
        let doc = &c.model.doc;
        // existing layers: their walls with ticks on the side the layer grows into
        for layer in &doc.layers {
            for w in &layer.walls {
                if !w.refs.iter().all(|r| doc.curves.contains_key(&r.id)) {
                    continue;
                }
                let fe = FaceEval::new(doc, &w.refs);
                let pts = fe.eval(&linspace(0.0, 1.0, 25));
                let scr: Vec<egui::Pos2> = pts.iter().map(|p| d.s(*p)).collect();
                d.screen_line(scr.clone(), col::layer(), 2.5);
                let mut side = 1.0f32;
                if let Some(inside) = w.inside {
                    let (mid, dd) = (pts[12], [pts[13][0] - pts[11][0], pts[13][1] - pts[11][1]]);
                    side = if -dd[1] * (inside[0] - mid[0]) + dd[0] * (inside[1] - mid[1]) > 0.0 { 1.0 } else { -1.0 };
                }
                let stroke = Stroke::new(1.5f32, col::layer());
                let mut k = 1;
                while k + 1 < scr.len() {
                    let dv = scr[k + 1] - scr[k - 1];
                    let n = dv.length().max(1.0);
                    // screen y points down, so the world-left normal (-dy, dx) is (dy, -dx) on screen
                    let normal = Vec2::new(dv.y / n, -dv.x / n) * side;
                    d.shapes.push(Shape::line_segment([scr[k], scr[k] + normal * 7.0], stroke));
                    k += 2;
                }
            }
        }
        for (b, f) in &self.walls {
            if let Some(scr) = face_screen_points(c, b, *f, 64) {
                d.screen_line(scr, col::selected(), 4.0);
            }
        }
        if let Some((b, f)) = &self.hover {
            if let Some(scr) = face_screen_points(c, b, *f, 64) {
                d.screen_line(scr.clone(), col::hover(), 3.0);
                let text = if c.model.layer_at_face(b, *f).is_some() {
                    "boundary layer (double-click to edit)"
                } else if !Self::is_wall(c, b, *f) {
                    "split line: not a wall"
                } else {
                    "wall: click to pick"
                };
                let fonts = d.painter.fonts(|f| f.clone());
                d.shapes.push(Shape::text(&fonts, scr[32] + Vec2::new(10.0, -10.0), Align2::LEFT_BOTTOM, text, c.canvas.font.clone(), col::geometry()));
            }
        }
    }
}
