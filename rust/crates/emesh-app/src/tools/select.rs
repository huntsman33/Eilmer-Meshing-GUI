//! Select and move.
//!
//!   click curve or point   select it (Ctrl toggles)
//!   double-click           edit its dimensions (point, line, arc)
//!   drag a handle          move that point; curves sharing the endpoint follow
//!   drag a selection       move it
//!   drag empty space       box select: left-to-right = window (fully inside),
//!                          right-to-left = crossing (touching)
//!   right-click            context menu

use std::collections::{BTreeMap, HashSet};

use egui::{Color32, Pos2, Rect, Shape, Stroke};

use emesh_core::entities::{Curve, Kind};
use emesh_core::geom;
use emesh_core::num::P;

use super::dimension::place_at;
use super::{AppEvent, Ctx, Tool, ToolEvent};
use crate::canvas::Draw;
use crate::theme::{alpha, polyline, Dash};

const DRAG_START_PX: f32 = 3.0;
const CONNECT_TOL_PX: f64 = 0.5;

enum DragKind {
    Dim(String),
    Handle { cid: String, name: String, partners: Vec<(String, &'static str)> },
    Body { points: BTreeMap<String, P> },
    Box { start: P, end: P, additive: bool },
}

struct Drag {
    kind: DragKind,
    origin: Option<P>,
    moving: HashSet<String>,
    moved: bool,
    start_screen: Pos2,
    originals: BTreeMap<String, Curve>,
}

#[derive(Default)]
pub struct SelectTool {
    drag: Option<Drag>,
    menu_block: Option<String>,
}

pub fn editable(c: &Ctx, eid: &str) -> bool {
    let doc = &c.model.doc;
    doc.points.contains_key(eid) || doc.dimensions.contains_key(eid) || doc.curves.get(eid).is_some_and(|c| c.is_line() || c.is_arc())
}

/// "Boundary condition" submenu for the solid curves among ids. True when an item was chosen.
pub fn bc_menu(ui: &mut egui::Ui, c: &mut Ctx, ids: &[String]) -> bool {
    let solid: Vec<String> = ids.iter().filter(|i| c.model.doc.curves.get(*i).is_some_and(|c| c.kind == Kind::Geometry)).cloned().collect();
    let mut chosen = false;
    ui.add_enabled_ui(!solid.is_empty(), |ui| {
        ui.menu_button("Boundary condition", |ui| {
            let current: HashSet<String> = solid.iter().map(|i| c.model.doc.curves[i].bc_tag.clone()).collect();
            let untagged = current.len() == 1 && current.contains("");
            if ui.add(egui::Button::new("Slip wall (default, untagged)").selected(untagged)).clicked() {
                let _ = c.model.set_bc(&solid, "");
                chosen = true;
            }
            if !c.model.doc.bcs.is_empty() {
                ui.separator();
            }
            let mut tags: Vec<String> = c.model.doc.bcs.keys().cloned().collect();
            tags.sort();
            for t in tags {
                let on = current.len() == 1 && current.contains(&t);
                let color = crate::canvas::tag_color(&c.model.doc, &t);
                let text = egui::RichText::new(format!("\u{25A0} {t}  ({})", c.model.doc.bcs[&t].class));
                let resp = ui.add(egui::Button::new(text).selected(on));
                ui.painter().rect_filled(Rect::from_center_size(resp.rect.left_center() + egui::vec2(10.0, 0.0), egui::vec2(9.0, 9.0)), 1.0, color);
                if resp.clicked() {
                    let _ = c.model.set_bc(&solid, &t);
                    chosen = true;
                }
            }
            ui.separator();
            if ui.button("New boundary condition...").clicked() {
                c.events.push(AppEvent::NewBc(solid.clone()));
                chosen = true;
            }
        });
    });
    if chosen {
        ui.close_menu();
    }
    chosen
}

/// Hide / show items for a click on block bid (None: outside any block).
pub fn visibility_menu(ui: &mut egui::Ui, c: &mut Ctx, bid: Option<&str>) -> bool {
    let res = c.model.resolution();
    let hidden = c.canvas.hidden_ids(&res);
    let mut chosen = false;
    match bid {
        Some(b) if hidden.contains(b) => {
            if ui.button("Show block").clicked() {
                c.canvas.set_blocks_hidden(&[b.to_string()], false);
                chosen = true;
            }
        }
        Some(b) => {
            if ui.button("Hide block").clicked() {
                c.canvas.set_blocks_hidden(&[b.to_string()], true);
                chosen = true;
            }
            if ui.button("Hide all blocks").clicked() {
                let all: Vec<String> = res.polygons.keys().cloned().collect();
                c.canvas.set_blocks_hidden(&all, true);
                chosen = true;
            }
        }
        None => {}
    }
    if !hidden.is_empty() && ui.button("Show all blocks").clicked() {
        let all: Vec<String> = c.canvas.hidden_blocks.iter().cloned().collect();
        c.canvas.set_blocks_hidden(&all, false);
        chosen = true;
    }
    chosen
}

impl SelectTool {
    fn merge(c: &mut Ctx, ids: &[String]) {
        let (_, joins) = c.model.merge(ids);
        if joins > 0 {
            c.message(format!("Merged {} curves into {}.", ids.len(), ids.len() - joins));
        } else {
            c.message(
                "Nothing to merge: pieces must share an end and be collinear lines, arcs of one circle (under 180 \
                 degrees in total) or parts of one spline.",
            );
        }
    }
}

impl Tool for SelectTool {
    fn name(&self) -> &'static str {
        "select"
    }
    fn label(&self) -> &'static str {
        "Select"
    }
    fn hover_curves(&self) -> bool {
        true
    }
    fn takes_input(&self) -> bool {
        false
    }
    fn snaps(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.moved && matches!(d.kind, DragKind::Handle { .. } | DragKind::Body { .. }))
    }
    fn reset(&mut self) {
        self.drag = None;
    }
    fn busy(&self) -> bool {
        self.drag.is_some()
    }
    fn anchor(&self) -> Option<P> {
        self.drag.as_ref().and_then(|d| d.origin)
    }
    fn snap_exclude(&self) -> HashSet<String> {
        self.drag.as_ref().map(|d| d.moving.clone()).unwrap_or_default()
    }

    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        let add = ev.modifiers.command; // Ctrl+click adds / removes
        if let Some(did) = c.pick_dimension(ev.screen) {
            if add {
                if !c.model.selection.remove(&did) {
                    c.model.selection.insert(did);
                }
                return;
            }
            c.model.set_selection([did.clone()]);
            self.drag = Some(Drag {
                kind: DragKind::Dim(did),
                origin: None,
                moving: HashSet::new(),
                moved: false,
                start_screen: ev.screen,
                originals: BTreeMap::new(),
            });
            return;
        }
        if let (Some((cid, name, point)), false) = (c.canvas.pick_handle(c.model, ev.screen), add) {
            let partners = if name == "start" || name == "end" {
                c.model.connected_ends(point, std::slice::from_ref(&cid), Some(c.px(CONNECT_TOL_PX)))
            } else {
                vec![]
            };
            let mut moving: HashSet<String> = partners.iter().map(|(p, _)| p.clone()).collect();
            moving.insert(cid.clone());
            let originals = moving.iter().map(|i| (i.clone(), c.model.doc.curves[i].clone())).collect();
            self.drag = Some(Drag {
                kind: DragKind::Handle { cid, name, partners },
                origin: Some(point),
                moving,
                moved: false,
                start_screen: ev.screen,
                originals,
            });
            return;
        }
        if let Some(eid) = c.pick_entity(ev.screen, ev.world) {
            if add {
                if !c.model.selection.remove(&eid) {
                    c.model.selection.insert(eid);
                }
                return;
            }
            if !c.model.selection.contains(&eid) {
                c.model.set_selection([eid.clone()]);
            }
            let doc = &c.model.doc;
            let grab = match doc.points.get(&eid) {
                Some(p) => *p,
                None => geom::closest_point(&doc.curves[&eid], ev.world).0,
            };
            let moving: HashSet<String> = c.model.selection.iter().cloned().collect();
            let originals = moving.iter().filter_map(|i| doc.curves.get(i).map(|cv| (i.clone(), cv.clone()))).collect();
            let points = moving.iter().filter_map(|i| doc.points.get(i).map(|p| (i.clone(), *p))).collect();
            self.drag = Some(Drag {
                kind: DragKind::Body { points },
                origin: Some(grab),
                moving,
                moved: false,
                start_screen: ev.screen,
                originals,
            });
            return;
        }
        if !add {
            c.model.selection.clear();
        }
        self.drag = Some(Drag {
            kind: DragKind::Box { start: ev.world, end: ev.world, additive: add },
            origin: None,
            moving: HashSet::new(),
            moved: false,
            start_screen: ev.screen,
            originals: BTreeMap::new(),
        });
    }

    fn double_click(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        self.drag = None;
        if let Some(eid) = c.pick_entity(ev.screen, ev.world) {
            if editable(c, &eid) {
                c.model.set_selection([eid.clone()]);
                c.events.push(AppEvent::EditEntity(eid));
            }
        }
    }

    fn moved(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        let Some(d) = self.drag.as_mut() else { return };
        if !ev.dragging {
            return;
        }
        let mut point = ev.point();
        if !d.moved {
            if (ev.screen - d.start_screen).length() < DRAG_START_PX {
                return;
            }
            d.moved = true;
            match d.kind {
                DragKind::Dim(_) => c.model.begin_edit(),
                DragKind::Handle { .. } | DragKind::Body { .. } => {
                    c.model.begin_edit();
                    // re-snap now that snapping is active for this drag
                    let snap = c.canvas.snap(c.model, ev.world, d.origin, &d.moving);
                    point = snap.point;
                    c.canvas.last_snap = Some(snap);
                }
                DragKind::Box { .. } => {}
            }
        }
        match &mut d.kind {
            DragKind::Dim(did) => {
                let mut dim = c.model.doc.dimensions[did.as_str()].clone();
                place_at(&c.model.doc, &mut dim, ev.world);
                c.model.doc.dimensions.insert(did.clone(), dim);
                c.model.notify();
            }
            DragKind::Handle { cid, name, partners } => {
                let Some(new) = d.originals[cid.as_str()].with_handle(name, point) else { return };
                c.model.doc.curves.insert(cid.clone(), new);
                for (pid, which) in partners.iter() {
                    if let Some(m) = d.originals[pid.as_str()].with_handle(which, point) {
                        c.model.doc.curves.insert(pid.clone(), m);
                    }
                }
                c.model.notify();
            }
            DragKind::Body { points } => {
                let o = d.origin.unwrap();
                let (dx, dy) = (point[0] - o[0], point[1] - o[1]);
                for (i, orig) in &d.originals {
                    c.model.doc.curves.insert(i.clone(), orig.map_points(|p| [p[0] + dx, p[1] + dy]));
                }
                for (i, p) in points.iter() {
                    c.model.doc.points.insert(i.clone(), [p[0] + dx, p[1] + dy]);
                }
                c.model.notify();
            }
            DragKind::Box { end, .. } => *end = ev.world,
        }
    }

    fn release(&mut self, c: &mut Ctx, _ev: &ToolEvent) {
        let Some(d) = self.drag.take() else { return };
        match d.kind {
            DragKind::Dim(_) => {
                if d.moved {
                    c.model.commit_edit("Move dimension");
                }
            }
            DragKind::Handle { .. } => {
                if d.moved {
                    c.model.commit_edit("Edit point");
                }
            }
            DragKind::Body { .. } => {
                if d.moved {
                    c.model.commit_edit("Move");
                }
            }
            DragKind::Box { start, end, additive } => {
                if d.moved {
                    let crossing = end[0] < start[0];
                    let mut ids: Vec<String> = c.canvas.curves_in_rect(&c.model.doc, start, end, crossing);
                    if additive {
                        ids.extend(c.model.selection.iter().cloned());
                    }
                    c.model.set_selection(ids);
                }
            }
        }
    }

    fn cancel(&mut self, c: &mut Ctx) -> bool {
        if let Some(d) = self.drag.take() {
            if d.moved && !matches!(d.kind, DragKind::Box { .. }) {
                c.model.cancel_edit();
            }
            return true;
        }
        if !c.model.selection.is_empty() {
            c.model.selection.clear();
            return true;
        }
        false
    }

    fn finish(&mut self, _c: &mut Ctx) {}

    fn right_click(&mut self, c: &mut Ctx, ev: &ToolEvent) -> bool {
        if let Some(eid) = c.pick_entity(ev.screen, ev.world) {
            if !c.model.selection.contains(&eid) {
                c.model.set_selection([eid]);
            }
        }
        let res = c.model.resolution();
        self.menu_block = c.model.block_at(ev.world, &res);
        if c.model.selection.is_empty() {
            return self.menu_block.is_some() || !c.canvas.hidden_ids(&res).is_empty();
        }
        true
    }

    fn context_menu(&mut self, ui: &mut egui::Ui, c: &mut Ctx) -> bool {
        let bid = self.menu_block.clone();
        let mut chosen = false;
        let sel: Vec<String> = c.model.selection.iter().cloned().collect();
        if !sel.is_empty() {
            if sel.len() == 1 && editable(c, &sel[0]) && ui.button("Edit dimensions...").clicked() {
                c.events.push(AppEvent::EditEntity(sel[0].clone()));
                chosen = true;
            }
            let curves: Vec<String> = sel.iter().filter(|i| c.model.doc.curves.contains_key(*i)).cloned().collect();
            if curves.len() >= 2 && ui.button("Merge").clicked() {
                Self::merge(c, &curves);
                chosen = true;
            }
            if !curves.is_empty() {
                chosen |= bc_menu(ui, c, &curves);
                if ui.button("Toggle reference line (dashed, ignored by the mesher)").clicked() {
                    c.model.toggle_reference(None);
                    chosen = true;
                }
                if ui.button("Toggle block split line (dash-dot)").clicked() {
                    c.model.toggle_split(None);
                    chosen = true;
                }
            }
            if ui.button("Delete").clicked() {
                c.model.delete_selection();
                chosen = true;
            }
            let res = c.model.resolution();
            if bid.is_some() || !c.canvas.hidden_ids(&res).is_empty() {
                ui.separator();
            }
        }
        chosen |= visibility_menu(ui, c, bid.as_deref());
        chosen
    }

    fn paint_overlay(&self, d: &mut Draw, c: &Ctx) {
        let Some(drag) = &self.drag else { return };
        let DragKind::Box { start, end, .. } = &drag.kind else { return };
        if !drag.moved {
            return;
        }
        let a = c.canvas.view.to_screen(*start);
        let b = c.canvas.view.to_screen(*end);
        let crossing = end[0] < start[0];
        let p = crate::theme::palette();
        let color: Color32 = if crossing { p.select_crossing } else { p.select_window };
        let r = Rect::from_two_pos(a, b);
        d.shapes.push(Shape::rect_filled(r, 0.0, alpha(color, 28)));
        let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        polyline(&pts, Stroke::new(1.0f32, color), if crossing { Dash::Dashed } else { Dash::Solid }, &mut d.shapes);
    }

    fn hint(&self, _c: &Ctx) -> String {
        "Click to select (Ctrl adds), double-click to edit dimensions, drag points or curves to edit, drag on empty \
         space to box select (left-to-right: inside, right-to-left: crossing)."
            .into()
    }
}
