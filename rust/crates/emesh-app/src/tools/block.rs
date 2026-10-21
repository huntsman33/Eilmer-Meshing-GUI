//! Block tool.
//!
//!   click a region          make a block over it (corners: its four sharpest turns)
//!   click a block           select it
//!   double-click an edge    cells and clustering along that edge
//!   right-click             make block / patch (Laplace, AOPatch) / sub-divide / set corners / delete
//!   H / Shift+H             hide or show the selected block / all blocks

use std::cell::RefCell;
use std::collections::HashMap;

use egui::{Align2, Key, Modifiers, Pos2, Shape, Stroke, Vec2};

use emesh_core::blocking;
use emesh_core::document::{default_smoothing, Face, FACES};
use emesh_core::face::FaceEval;
use emesh_core::num::{linspace, P};
use serde_json::Map;

use super::select::{bc_menu, visibility_menu};
use super::{AppEvent, Ctx, Tool, ToolEvent};
use crate::canvas::{Draw, PICK_PX};
use crate::model::{Smoothing, Subdivision};
use crate::theme::col;

#[derive(Clone, Debug, PartialEq)]
pub enum Thing {
    Face(String, Face),
    Block(String),
    Region(usize),
}

/// Block faces under the cursor (cached per revision).
#[derive(Default)]
pub struct FacePicker {
    faces: RefCell<(Option<u64>, HashMap<(String, Face), Vec<P>>)>,
}

impl FacePicker {
    pub fn face_at(&self, c: &Ctx, screen: Pos2) -> Option<(String, Face)> {
        let mut cache = self.faces.borrow_mut();
        if cache.0 != Some(c.model.revision) {
            let res = c.model.resolution();
            let md = c.model.doc.with_blocks(res.blocks.clone());
            let t = linspace(0.0, 1.0, 48);
            let mut map = HashMap::new();
            for b in &res.blocks {
                for f in FACES {
                    map.insert((b.id.clone(), f), FaceEval::new(&md, b.side(f)).eval(&t));
                }
            }
            *cache = (Some(c.model.revision), map);
        }
        let cursor = c.canvas.view.to_world(screen);
        let mut best: Option<(f64, (String, Face))> = None;
        for (key, pts) in &cache.1 {
            // distance to the face itself, not just its sample points (long faces at high zoom)
            let d = emesh_core::geom::project_polyline(pts, cursor).1 * c.canvas.view.scale;
            if d <= PICK_PX && best.as_ref().is_none_or(|b| d < b.0 || (d == b.0 && key < &b.1)) {
                best = Some((d, key.clone()));
            }
        }
        best.map(|b| b.1)
    }
}

pub fn face_screen_points(c: &Ctx, bid: &str, face: Face, n: usize) -> Option<Vec<Pos2>> {
    let res = c.model.resolution();
    let b = res.block(bid)?;
    let md = c.model.doc.with_blocks(res.blocks.clone());
    Some(FaceEval::new(&md, b.side(face)).eval(&linspace(0.0, 1.0, n)).iter().map(|p| c.canvas.view.to_screen(*p)).collect())
}

#[derive(Default)]
pub struct BlockTool {
    pub picker: FacePicker,
    hover: Option<Thing>,
    selected: Option<String>,
    corner_pick: Option<(String, Vec<P>)>,
    menu: Option<(Thing, P)>,
}

impl BlockTool {
    pub fn thing_at(&self, c: &Ctx, ev: &ToolEvent) -> Option<Thing> {
        if let Some((b, f)) = self.picker.face_at(c, ev.screen) {
            return Some(Thing::Face(b, f));
        }
        let res = c.model.resolution();
        if let Some(b) = c.model.block_at(ev.world, &res) {
            return Some(Thing::Block(b));
        }
        c.model.topology().region_index_at(ev.world).map(Thing::Region)
    }

    fn try_do(c: &mut Ctx, r: Result<impl Sized, String>, done: &str) {
        match r {
            Ok(_) => c.message(done),
            Err(e) => c.message(format!("Cannot do that: {e}.")),
        }
    }

    fn report(c: &mut Ctx, bid: &str) {
        let res = c.model.resolution();
        if let Some(e) = res.errors.get(bid) {
            c.message(format!("{bid}: {e}."));
        } else if let Some(w) = res.warnings.get(bid) {
            c.message(format!("{bid}: {}.", w.join("; ")));
        }
    }

    fn nearest_vertex(c: &Ctx, bid: &str, world: P) -> P {
        let topo = c.model.topology();
        let verts = c
            .model
            .doc
            .block(bid)
            .and_then(|b| b.region.as_ref())
            .and_then(|spec| blocking::find_region(&topo, spec))
            .map(|(ri, _)| blocking::loop_vertices(&topo, &topo.regions[ri].loop_))
            .unwrap_or_else(|| vec![world]);
        *verts.iter().min_by(|a, b| ((a[0] - world[0]).hypot(a[1] - world[1])).partial_cmp(&(b[0] - world[0]).hypot(b[1] - world[1])).unwrap()).unwrap()
    }

    fn cursor_block(c: &Ctx) -> Option<String> {
        let pos = c.canvas.last_screen?;
        let res = c.model.resolution();
        c.model.block_at(c.canvas.view.to_world(pos), &res)
    }

    fn patch_menu(ui: &mut egui::Ui, c: &mut Ctx, bid: &str) -> bool {
        let Some(block) = c.model.doc.block(bid).cloned() else { return false };
        let mut chosen = false;
        ui.menu_button("Patch", |ui| {
            let plain = block.patch == "CoonsPatch" && !block.smoothed();
            if ui.add(egui::Button::new("CoonsPatch").selected(plain)).clicked() {
                c.model.set_block_patch(&[bid.to_string()], Some(("CoonsPatch", Map::new())), Smoothing::Set(None));
                chosen = true;
            }
            let lap = block.patch == "CoonsPatch" && block.smoothed();
            if ui.add(egui::Button::new("CoonsPatch + Laplace smoothing").selected(lap)).clicked() {
                c.model.set_block_patch(&[bid.to_string()], Some(("CoonsPatch", Map::new())), Smoothing::Set(Some(default_smoothing())));
                chosen = true;
            }
            if ui.add(egui::Button::new("AOPatch...").selected(block.patch == "AOPatch")).clicked() {
                c.events.push(AppEvent::AoPatch(bid.to_string()));
                chosen = true;
            }
            if chosen {
                ui.close_menu();
            }
        });
        chosen
    }

    fn outline(d: &mut Draw, poly: &[P], color: egui::Color32, width: f32) {
        let mut pts: Vec<Pos2> = poly.iter().map(|p| d.s(*p)).collect();
        if let Some(f) = pts.first().copied() {
            pts.push(f);
        }
        d.screen_line(pts, color, width);
    }
}

impl Tool for BlockTool {
    fn name(&self) -> &'static str {
        "block"
    }
    fn label(&self) -> &'static str {
        "Block"
    }
    fn snaps(&self) -> bool {
        false
    }
    fn takes_input(&self) -> bool {
        false
    }
    fn reset(&mut self) {
        self.hover = None;
        self.selected = None;
        self.corner_pick = None;
    }
    fn busy(&self) -> bool {
        self.corner_pick.is_some()
    }
    fn moved(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        self.hover = self.thing_at(c, ev);
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if let Some((bid, pts)) = self.corner_pick.as_mut() {
            let v = Self::nearest_vertex(c, bid, ev.world);
            pts.push(v);
            if pts.len() == 4 {
                let (bid, pts) = self.corner_pick.take().unwrap();
                let r = c.model.set_block_corners(&bid, &pts);
                Self::try_do(c, r, "Corners set.");
            }
            return;
        }
        match self.thing_at(c, ev) {
            None => self.selected = None,
            Some(Thing::Face(b, _)) | Some(Thing::Block(b)) => {
                Self::report(c, &b);
                self.selected = Some(b);
            }
            Some(Thing::Region(_)) => {
                let r = c.model.make_block(ev.world, None);
                Self::try_do(c, r, "Block made.");
            }
        }
    }
    fn double_click(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if let Some(Thing::Face(b, f)) = self.thing_at(c, ev) {
            c.events.push(AppEvent::EdgeCells(b, f));
        }
    }
    fn right_click(&mut self, c: &mut Ctx, ev: &ToolEvent) -> bool {
        match self.thing_at(c, ev) {
            None => {
                let res = c.model.resolution();
                if c.canvas.hidden_ids(&res).is_empty() {
                    return false;
                }
                self.menu = None;
                true
            }
            Some(t) => {
                self.menu = Some((t, ev.world));
                true
            }
        }
    }
    fn context_menu(&mut self, ui: &mut egui::Ui, c: &mut Ctx) -> bool {
        let res = c.model.resolution();
        let Some((thing, point)) = self.menu.clone() else {
            let bid = c.canvas.last_screen.and_then(|s| c.model.block_at(c.canvas.view.to_world(s), &res));
            return visibility_menu(ui, c, bid.as_deref());
        };
        let at = c.model.block_at(point, &res);
        let has_visibility = at.is_some() || !c.canvas.hidden_ids(&res).is_empty();
        let mut chosen = visibility_menu(ui, c, at.as_deref());
        if has_visibility {
            ui.separator();
        }
        if let Thing::Face(b, f) = &thing {
            if ui.button("Edge cells...").clicked() {
                c.events.push(AppEvent::EdgeCells(b.clone(), *f));
                chosen = true;
            }
            if let Some(block) = res.block(b) {
                let ids: Vec<String> = block.side(*f).iter().map(|r| r.id.clone()).collect();
                chosen |= bc_menu(ui, c, &ids);
            }
        }
        if let Thing::Region(_) = thing {
            if ui.button("Make block").clicked() {
                let r = c.model.make_block(point, None);
                Self::try_do(c, r, "Block made.");
                chosen = true;
            }
        }
        if let Thing::Face(b, _) | Thing::Block(b) = &thing {
            chosen |= Self::patch_menu(ui, c, b);
            if ui.button("Set corners...").clicked() {
                self.corner_pick = Some((b.clone(), vec![]));
                c.message("Click the block's four corners, going round it.");
                chosen = true;
            }
            if ui.button("Delete block").clicked() {
                c.model.delete_block(b);
                chosen = true;
            }
        }
        let topo = c.model.topology();
        if let Some(ri) = topo.region_index_at(point) {
            let holes = topo.regions[ri].holes.len();
            ui.menu_button("Sub-divide", |ui| {
                let mut sub: Option<Subdivision> = None;
                if ui.button("Split N x M...").clicked() {
                    c.events.push(AppEvent::SplitRegion(point));
                    chosen = true;
                }
                if ui.button("O-grid").clicked() {
                    sub = Some(Subdivision::OGrid(0.5));
                }
                if ui.add_enabled(holes == 1, egui::Button::new("O-grid around hole")).clicked() {
                    sub = Some(Subdivision::OGridHole);
                }
                if ui.button("Centroid split").clicked() {
                    sub = Some(Subdivision::Centroid);
                }
                if let Some(k) = sub {
                    let r = c.model.subdivide(point, k);
                    Self::try_do(c, r, "Sub-divided.");
                    chosen = true;
                }
                if chosen {
                    ui.close_menu();
                }
            });
        }
        chosen
    }
    fn cancel(&mut self, _c: &mut Ctx) -> bool {
        if self.corner_pick.take().is_some() {
            return true;
        }
        self.selected.take().is_some()
    }
    fn finish(&mut self, _c: &mut Ctx) {}
    fn claims_key(&self, key: Key) -> bool {
        key == Key::H
    }
    fn claims_hold(&self, key: Key) -> bool {
        key == Key::H
    }
    fn key(&mut self, c: &mut Ctx, key: Key, m: Modifiers) -> bool {
        if key != Key::H {
            return false;
        }
        let res = c.model.resolution();
        if m.shift {
            let all: Vec<String> = res.polygons.keys().cloned().collect();
            c.canvas.toggle_blocks_hidden(&all, &res);
            return true;
        }
        if let Some(b) = self.selected.clone().or_else(|| Self::cursor_block(c)) {
            c.canvas.toggle_blocks_hidden(&[b], &res);
        }
        true
    }
    fn prompt(&self, _c: &Ctx) -> String {
        match &self.corner_pick {
            Some((_, pts)) => format!("Corner {} of 4", pts.len() + 1),
            None => "Click a region to block it; double-click an edge for its cells".into(),
        }
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Block: click a closed region to make a block, double-click a block edge to set its cells, right-click to \
         sub-divide, set corners or delete. H hides / shows the selected block, Shift+H all blocks."
            .into()
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        let res = c.model.resolution();
        if let Some(poly) = self.selected.as_ref().and_then(|s| res.polygons.get(s)) {
            Self::outline(d, poly, col::selected(), 2.5);
        }
        match &self.hover {
            Some(Thing::Region(ri)) => {
                let topo = c.model.topology();
                if let Some(r) = topo.regions.get(*ri) {
                    Self::outline(d, &r.polygon, col::hover(), 2.0);
                }
            }
            Some(Thing::Block(b)) => {
                if let Some(poly) = res.polygons.get(b) {
                    Self::outline(d, poly, col::hover(), 2.0);
                }
            }
            _ => {}
        }
    }
    fn paint_overlay(&self, d: &mut Draw, c: &Ctx) {
        if let Some((_, pts)) = &self.corner_pick {
            for p in pts {
                d.shapes.push(Shape::circle_stroke(d.s(*p), 6.0, Stroke::new(2.0f32, col::selected())));
            }
        }
        let Some(Thing::Face(bid, face)) = &self.hover else { return };
        let res = c.model.resolution();
        let Some(block) = res.block(bid) else { return };
        let Some(scr) = face_screen_points(c, bid, *face, 64) else { return };
        paint_face_arrow(d, &scr, col::selected(), 3.5);
        let n = block.count(*face) - 1;
        let tip = *scr.last().unwrap();
        let fonts = d.painter.fonts(|f| f.clone());
        let text = format!("{} ({n} cells)", face.as_str());
        let width = d.painter.layout_no_wrap(text.clone(), c.canvas.font.clone(), col::geometry()).size().x;
        // keep the label on screen: to the left of the end when it would run off the right
        let (pos, align) = if tip.x + 8.0 + width > c.canvas.view.rect.right() {
            (tip + Vec2::new(-8.0, -8.0), Align2::RIGHT_BOTTOM)
        } else {
            (tip + Vec2::new(8.0, -8.0), Align2::LEFT_BOTTOM)
        };
        d.shapes.push(Shape::text(&fonts, pos, align, text, c.canvas.font.clone(), col::geometry()));
    }
}

/// A face highlighted with its start (o) and end (>) marked.
pub fn paint_face_arrow(d: &mut Draw, scr: &[Pos2], color: egui::Color32, width: f32) {
    d.screen_line(scr.to_vec(), color, width);
    let stroke = Stroke::new(width, color);
    d.shapes.push(Shape::circle_stroke(scr[0], 5.0, stroke));
    let n = scr.len();
    let dd = scr[n - 1] - scr[n.saturating_sub(4)];
    let l = dd.length().max(1.0);
    let dir = dd / l;
    let perp = Vec2::new(-dir.y, dir.x);
    let tip = scr[n - 1];
    d.shapes.push(Shape::line_segment([tip, tip - dir * 10.0 + perp * 5.0], stroke));
    d.shapes.push(Shape::line_segment([tip, tip - dir * 10.0 - perp * 5.0], stroke));
}
