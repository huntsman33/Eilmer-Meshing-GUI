//! Tools that act on a set of existing objects: move, merge and mirror. They use the selection
//! as their target set; a click toggles an object in or out.

use std::collections::{BTreeMap, HashSet};

use egui::{Key, Modifiers, Pos2, Vec2};

use emesh_core::entities::Curve;
use emesh_core::num::P;
use emesh_core::operations::{lies_on, mirror_curve, reflect};

use super::{draw_preview, Ctx, Tool, ToolEvent};
use crate::canvas::{Draw, PICK_PX};
use crate::theme::{col, Dash};

fn toggle_target(c: &mut Ctx, ev: &ToolEvent) {
    if let Some(eid) = c.pick_entity(ev.screen, ev.world) {
        if !c.model.selection.remove(&eid) {
            c.model.selection.insert(eid);
        }
    }
}

struct MoveDrag {
    origin: P,
    eid: Option<String>,
    added: bool,
    moved: bool,
    start_screen: Pos2,
    moving: HashSet<String>,
    curves: BTreeMap<String, Curve>,
    points: BTreeMap<String, P>,
}

/// Click objects to add them (click a selected one again to drop it), then drag: the drag runs
/// from the snapped press point to the snapped release point. Typing dx, dy moves exactly.
#[derive(Default)]
pub struct MoveTool {
    drag: Option<MoveDrag>,
}

impl MoveTool {
    fn movable(c: &Ctx) -> Vec<String> {
        c.model.selection.iter().filter(|i| c.model.doc.curves.contains_key(*i) || c.model.doc.points.contains_key(*i)).cloned().collect()
    }
}

impl Tool for MoveTool {
    fn name(&self) -> &'static str {
        "move"
    }
    fn label(&self) -> &'static str {
        "Move"
    }
    fn hover_curves(&self) -> bool {
        true
    }
    fn reset(&mut self) {
        self.drag = None;
    }
    fn busy(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| d.moved)
    }
    fn anchor(&self) -> Option<P> {
        self.drag.as_ref().map(|d| d.origin)
    }
    fn snap_exclude(&self) -> HashSet<String> {
        self.drag.as_ref().filter(|d| d.moved).map(|d| d.moving.clone()).unwrap_or_default()
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        let eid = c.pick_entity(ev.screen, ev.world).filter(|e| !c.model.doc.dimensions.contains_key(e));
        let added = eid.as_ref().is_some_and(|e| !c.model.selection.contains(e));
        if added {
            c.model.selection.insert(eid.clone().unwrap());
        }
        self.drag = Some(MoveDrag {
            origin: ev.point(),
            eid,
            added,
            moved: false,
            start_screen: ev.screen,
            moving: HashSet::new(),
            curves: BTreeMap::new(),
            points: BTreeMap::new(),
        });
    }
    fn moved(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        let Some(d) = self.drag.as_mut() else { return };
        if !ev.dragging {
            return;
        }
        let mut point = ev.point();
        if !d.moved {
            if (ev.screen - d.start_screen).length() < 3.0 {
                return;
            }
            let ids = Self::movable(c);
            if ids.is_empty() {
                return;
            }
            d.moved = true;
            d.moving = ids.iter().cloned().collect();
            d.curves = ids.iter().filter_map(|i| c.model.doc.curves.get(i).map(|cv| (i.clone(), cv.clone()))).collect();
            d.points = ids.iter().filter_map(|i| c.model.doc.points.get(i).map(|p| (i.clone(), *p))).collect();
            c.model.begin_edit();
            let snap = c.canvas.snap(c.model, ev.world, Some(d.origin), &d.moving);
            point = snap.point;
            c.canvas.last_snap = Some(snap);
        }
        let (dx, dy) = (point[0] - d.origin[0], point[1] - d.origin[1]);
        for (i, cv) in &d.curves {
            c.model.doc.curves.insert(i.clone(), cv.map_points(|p| [p[0] + dx, p[1] + dy]));
        }
        for (i, p) in &d.points {
            c.model.doc.points.insert(i.clone(), [p[0] + dx, p[1] + dy]);
        }
        c.model.notify();
    }
    fn release(&mut self, c: &mut Ctx, _ev: &ToolEvent) {
        let Some(d) = self.drag.take() else { return };
        if d.moved {
            c.model.commit_edit("Move");
        } else if d.eid.is_none() {
            c.model.selection.clear(); // click on empty space clears
        } else if !d.added {
            c.model.selection.remove(d.eid.as_ref().unwrap());
        }
    }
    fn cancel(&mut self, c: &mut Ctx) -> bool {
        if let Some(d) = self.drag.take() {
            if d.moved {
                c.model.cancel_edit();
                return true;
            }
        }
        if !c.model.selection.is_empty() {
            c.model.selection.clear();
            return true;
        }
        false
    }
    fn finish(&mut self, _c: &mut Ctx) {}
    fn accept_text(&mut self, c: &mut Ctx, text: &str) -> Result<(), String> {
        let ids = Self::movable(c);
        if ids.is_empty() {
            return Err("select something to move first".into());
        }
        let d = c.parse_point(text.trim_start_matches('@'), None)?;
        c.model.translate(&ids, d[0], d[1]);
        Ok(())
    }
    fn prompt(&self, c: &Ctx) -> String {
        format!("Objects to move ({} selected): drag, or type dx, dy", Self::movable(c).len())
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Move: click objects to add them, then drag from any point to where it should go (snaps apply to both). Type \
         dx, dy to move by an exact amount."
            .into()
    }
}

/// Click the pieces to join, then Enter (or right-click).
pub struct MergeTool;

impl Tool for MergeTool {
    fn name(&self) -> &'static str {
        "merge"
    }
    fn label(&self) -> &'static str {
        "Merge"
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
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        toggle_target(c, ev);
    }
    fn finish(&mut self, c: &mut Ctx) {
        let ids: Vec<String> = c.model.selection.iter().filter(|i| c.model.doc.curves.contains_key(*i)).cloned().collect();
        if ids.len() < 2 {
            c.message("Merge: select at least two connected curves.");
            return;
        }
        let (_, joins) = c.model.merge(&ids);
        if joins > 0 {
            c.message(format!("Merged {} curves into {}.", ids.len(), ids.len() - joins));
        } else {
            c.message(
                "Nothing to merge: pieces must share an end and be collinear lines, arcs of one circle (under 180 \
                 degrees in total) or parts of one spline.",
            );
        }
    }
    fn cancel(&mut self, c: &mut Ctx) -> bool {
        if c.model.selection.is_empty() {
            return false;
        }
        c.model.selection.clear();
        true
    }
    fn prompt(&self, c: &Ctx) -> String {
        let n = c.model.selection.iter().filter(|i| c.model.doc.curves.contains_key(*i)).count();
        format!("Pieces to merge ({n} selected): Enter or right-click to merge")
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Merge: click connected pieces (collinear lines, arcs of one circle, parts of one spline), then press Enter.".into()
    }
}

#[derive(Clone)]
enum Axis {
    X,
    Y,
    Line(String),
}

/// Step 1: click objects to mirror, then Enter. Step 2: click the symmetry axis (a line, or the
/// x / y axis); Enter creates mirrored copies.
#[derive(Default)]
pub struct MirrorTool {
    step2: bool,
    axis: Option<(P, P, Axis)>,
}

impl MirrorTool {
    fn pick_axis(&self, c: &Ctx, ev: &ToolEvent) -> Option<(P, P, Axis)> {
        let lines: Vec<String> =
            c.model.doc.curves.values().filter(|cv| cv.is_line() && !c.model.selection.contains(&cv.id)).map(|cv| cv.id.clone()).collect();
        if let Some(cid) = c.pick_curve(ev.world, Some(&lines)) {
            let (a, b) = c.model.doc.curves[&cid].ends_lp();
            return Some((a, b, Axis::Line(cid)));
        }
        let o = c.canvas.view.to_screen([0.0, 0.0]);
        if ((ev.screen.y - o.y) as f64).abs() <= PICK_PX {
            return Some(([0.0, 0.0], [1.0, 0.0], Axis::X));
        }
        if ((ev.screen.x - o.x) as f64).abs() <= PICK_PX {
            return Some(([0.0, 0.0], [0.0, 1.0], Axis::Y));
        }
        None
    }
}

impl Tool for MirrorTool {
    fn name(&self) -> &'static str {
        "mirror"
    }
    fn label(&self) -> &'static str {
        "Mirror"
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
        self.step2 = false;
        self.axis = None;
    }
    fn busy(&self) -> bool {
        self.step2
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        if !self.step2 {
            toggle_target(c, ev);
        } else if let Some(a) = self.pick_axis(c, ev) {
            self.axis = Some(a);
        }
    }
    fn finish(&mut self, c: &mut Ctx) {
        if !self.step2 {
            if c.model.selection.is_empty() {
                c.message("Mirror: select the objects to mirror first.");
                return;
            }
            self.step2 = true;
            return;
        }
        let Some((a, b, _)) = self.axis.clone() else {
            c.message("Mirror: click a line, or the x or y axis, as the symmetry axis.");
            return;
        };
        let ids: Vec<String> = c.model.selection.iter().cloned().collect();
        let new_ids = c.model.mirror(&ids, a, b);
        c.message(format!("Mirrored {} object(s).", new_ids.len()));
        c.model.set_selection(new_ids);
        self.reset();
    }
    fn right_click(&mut self, c: &mut Ctx, _ev: &ToolEvent) -> bool {
        self.finish(c);
        false
    }
    fn cancel(&mut self, c: &mut Ctx) -> bool {
        if self.step2 {
            self.reset(); // back to choosing objects
            return true;
        }
        if !c.model.selection.is_empty() {
            c.model.selection.clear();
            return true;
        }
        false
    }
    fn prompt(&self, c: &Ctx) -> String {
        if !self.step2 {
            return format!("Objects to mirror ({} selected): then Enter", c.model.selection.len());
        }
        match &self.axis {
            None => "Symmetry axis: click a line or the x / y axis".into(),
            Some((_, _, Axis::X)) => "Axis: x axis. Enter to mirror".into(),
            Some((_, _, Axis::Y)) => "Axis: y axis. Enter to mirror".into(),
            Some(_) => "Axis: line. Enter to mirror".into(),
        }
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Mirror: click objects, press Enter, click the symmetry axis (a solid or dashed line, or the x / y axis), then \
         press Enter."
            .into()
    }
    fn key(&mut self, _c: &mut Ctx, _key: Key, _m: Modifiers) -> bool {
        false
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        let Some((a, b, axis)) = &self.axis else { return };
        if !self.step2 {
            return;
        }
        match axis {
            Axis::X | Axis::Y => {
                let r = c.canvas.view.rect;
                let lo = c.canvas.view.to_world(r.left_bottom());
                let hi = c.canvas.view.to_world(r.right_top());
                let ends = if matches!(axis, Axis::X) { [[lo[0], 0.0], [hi[0], 0.0]] } else { [[0.0, lo[1]], [0.0, hi[1]]] };
                d.world_line(&ends, col::snap(), 2.0, Dash::Dashed);
            }
            Axis::Line(cid) => {
                if let Some(cv) = c.model.doc.curves.get(cid) {
                    d.curve(cv, col::snap(), 3.0, None);
                }
            }
        }
        let tol = 1e-9 * [1.0, a[0].abs(), a[1].abs(), b[0].abs(), b[1].abs()].iter().cloned().fold(0.0, f64::max);
        let previews: Vec<Curve> = c
            .model
            .selection
            .iter()
            .filter_map(|i| c.model.doc.curves.get(i))
            .filter(|cv| !lies_on(cv, *a, *b, tol))
            .map(|cv| mirror_curve(cv, *a, *b))
            .collect();
        draw_preview(d, &previews);
        for i in &c.model.selection {
            if let Some(p) = c.model.doc.points.get(i) {
                let q = c.canvas.view.to_screen(reflect(*p, *a, *b));
                d.screen_line(vec![q - Vec2::new(5.0, 0.0), q + Vec2::new(5.0, 0.0)], col::preview(), 1.5);
                d.screen_line(vec![q - Vec2::new(0.0, 5.0), q + Vec2::new(0.0, 5.0)], col::preview(), 1.5);
            }
        }
    }
}
