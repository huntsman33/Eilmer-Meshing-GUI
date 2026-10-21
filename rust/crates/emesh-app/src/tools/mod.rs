//! Canvas tools: each is a small state machine driven by pointer and key events.

pub mod block;
pub mod dimension;
pub mod draw;
pub mod layer;
pub mod modify;
pub mod offset;
pub mod select;
pub mod trim;

use std::collections::HashSet;

use egui::{Key, Modifiers, Pos2};

use emesh_core::document::Face;
use emesh_core::entities::Curve;
use emesh_core::num::P;
use emesh_core::snapping::Snap;

use crate::canvas::{Canvas, Draw};
use crate::model::SketchModel;
use crate::settings::Settings;

/// Pixels within which two clicks are the same point.
pub const SAME_POINT_PX: f64 = 1.0;

#[derive(Clone, Debug)]
pub struct ToolEvent {
    /// raw cursor position
    pub world: P,
    pub snap: Snap,
    pub screen: Pos2,
    pub modifiers: Modifiers,
    /// the primary button is held
    pub dragging: bool,
}

impl ToolEvent {
    pub fn point(&self) -> P {
        self.snap.point
    }
}

/// Requests from tools to the main window.
#[derive(Clone, Debug)]
pub enum AppEvent {
    Message(String),
    /// open the dimension editor for a point, line, arc or dimension
    EditEntity(String),
    EdgeCells(String, Face),
    SplitRegion(P),
    NewBc(Vec<String>),
    /// boundary-layer settings for a wall (None: the defaults for new layers)
    LayerSettings(Option<(String, Face)>),
    AoPatch(String),
    ClearTyped,
}

pub struct Ctx<'a> {
    pub model: &'a mut SketchModel,
    pub canvas: &'a mut Canvas,
    pub painter: &'a egui::Painter,
    pub events: &'a mut Vec<AppEvent>,
    pub settings: &'a mut Settings,
}

impl Ctx<'_> {
    pub fn message(&mut self, text: impl Into<String>) {
        self.events.push(AppEvent::Message(text.into()));
    }

    pub fn px(&self, n: f64) -> f64 {
        self.canvas.px(n)
    }

    pub fn distinct(&self, a: P, b: P) -> bool {
        (a[0] - b[0]).hypot(a[1] - b[1]) > self.px(SAME_POINT_PX)
    }

    pub fn line(&self, a: P, b: P) -> Curve {
        Curve::line(a, b).with_kind(self.model.new_kind())
    }

    pub fn arcs(&self, pieces: &[(P, P, P)]) -> Vec<Curve> {
        pieces.iter().map(|&(a, b, c)| Curve::arc(a, b, c).with_kind(self.model.new_kind())).collect()
    }

    pub fn add(&mut self, curves: Vec<Curve>, text: &str) {
        if !curves.is_empty() {
            self.model.add_curves(curves, text);
        }
    }

    pub fn pick_entity(&self, screen: Pos2, world: P) -> Option<String> {
        self.canvas.pick_entity(self.painter, &self.model.doc, screen, world)
    }

    pub fn pick_curve(&self, world: P, among: Option<&[String]>) -> Option<String> {
        self.canvas.pick_curve(&self.model.doc, world, crate::canvas::PICK_PX, among)
    }

    pub fn pick_dimension(&self, screen: Pos2) -> Option<String> {
        self.canvas.pick_dimension(self.painter, &self.model.doc, screen)
    }

    pub fn parse_point(&self, text: &str, anchor: Option<P>) -> Result<P, String> {
        let u = self.canvas.units;
        let a = anchor.map(|p| u.point_to_display(p));
        Ok(u.point_from_display(emesh_core::typed_input::parse_point(text, a)?))
    }

    pub fn parse_length(&self, text: &str) -> Option<f64> {
        emesh_core::typed_input::single_number(text).map(|v| self.canvas.units.from_display(v))
    }

    pub fn fmt(&self, metres: f64) -> String {
        self.canvas.units.format(metres, None)
    }
}

pub trait Tool {
    fn name(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// whether the canvas snaps the cursor for this tool
    fn snaps(&self) -> bool {
        true
    }
    /// whether entities highlight under the cursor
    fn hover_curves(&self) -> bool {
        false
    }
    /// whether typed input applies
    fn takes_input(&self) -> bool {
        true
    }
    fn reset(&mut self) {}
    fn busy(&self) -> bool {
        false
    }
    /// Esc: drop the operation in progress; false if there was nothing to cancel.
    fn cancel(&mut self, _c: &mut Ctx) -> bool {
        let b = self.busy();
        self.reset();
        b
    }
    /// Right-click or Enter: complete the operation in progress.
    fn finish(&mut self, _c: &mut Ctx) {
        self.reset();
    }
    /// Point used for horizontal/vertical inference and relative typed input.
    fn anchor(&self) -> Option<P> {
        None
    }
    fn snap_exclude(&self) -> HashSet<String> {
        HashSet::new()
    }
    fn press(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        self.click(c, ev.point());
    }
    fn moved(&mut self, _c: &mut Ctx, _ev: &ToolEvent) {}
    fn release(&mut self, _c: &mut Ctx, _ev: &ToolEvent) {}
    /// Returns true to open the tool's context menu.
    fn right_click(&mut self, c: &mut Ctx, _ev: &ToolEvent) -> bool {
        self.finish(c);
        false
    }
    /// The context menu; returns true when an item was chosen (the menu closes).
    fn context_menu(&mut self, _ui: &mut egui::Ui, _c: &mut Ctx) -> bool {
        false
    }
    fn click(&mut self, _c: &mut Ctx, _p: P) {}
    fn claims_key(&self, _key: Key) -> bool {
        false
    }
    /// True if this tool uses a canvas hold key (H, V) as its own shortcut.
    fn claims_hold(&self, _key: Key) -> bool {
        false
    }
    fn key(&mut self, _c: &mut Ctx, _key: Key, _m: Modifiers) -> bool {
        false
    }
    fn double_click(&mut self, _c: &mut Ctx, _ev: &ToolEvent) {}
    /// Enter in the input box (display units).
    fn accept_text(&mut self, c: &mut Ctx, text: &str) -> Result<(), String> {
        let p = c.parse_point(text, self.anchor())?;
        self.click(c, p);
        Ok(())
    }
    /// Text being typed in the input box, before Enter (e.g. to lock a length).
    fn preview_text(&mut self, _c: &mut Ctx, _text: &str) {}
    fn prompt(&self, _c: &Ctx) -> String {
        String::new()
    }
    fn hint(&self, _c: &Ctx) -> String {
        String::new()
    }
    /// Rubber-band geometry, in world coordinates.
    fn paint(&self, _d: &mut Draw, _c: &Ctx) {}
    /// Screen-space decorations.
    fn paint_overlay(&self, _d: &mut Draw, _c: &Ctx) {}
    /// Windows the tool owns (e.g. the offset dialog), shown every frame.
    fn windows(&mut self, _ctx: &egui::Context, _c: &mut Ctx) {}
    /// A window of the tool's is open: the canvas ignores input meanwhile.
    fn modal(&self) -> bool {
        false
    }
}

pub fn draw_preview(d: &mut Draw, curves: &[Curve]) {
    for c in curves {
        d.curve(c, crate::theme::col::preview(), 1.5, None);
    }
}

/// All the tools, in toolbar order, with their tooltips.
pub fn all_tools() -> Vec<(Box<dyn Tool>, &'static str)> {
    vec![
        (Box::new(select::SelectTool::default()), "Select and move; double-click to edit dimensions"),
        (Box::new(draw::PointTool), "Construction point"),
        (Box::new(draw::LineTool::default()), "Line (type a number to set the length)"),
        (Box::new(draw::ProfileTool::default()), "Profile: chained lines and tangent arcs"),
        (Box::new(draw::ArcTool::default()), "Arc: centre, start, end (M for 3-point)"),
        (Box::new(draw::CircleTool::default()), "Circle"),
        (Box::new(draw::RectangleTool::default()), "Rectangle"),
        (Box::new(draw::TriangleTool::default()), "Triangle"),
        (Box::new(draw::PointListTool::bezier()), "Bezier curve"),
        (Box::new(draw::PointListTool::spline()), "Spline through points"),
        (
            Box::new(dimension::DimensionTool::default()),
            "Dimension: length of a line, angle of an arc, or distance / angle between two lines; edit to drive the geometry",
        ),
        (Box::new(modify::MoveTool::default()), "Move: click objects, then drag (or type dx, dy)"),
        (Box::new(trim::TrimTool::default()), "Quick trim: delete a piece up to where it meets other curves"),
        (Box::new(modify::MergeTool), "Merge connected collinear lines, arcs of one circle or spline pieces"),
        (Box::new(modify::MirrorTool::default()), "Mirror: pick objects, Enter, pick an axis line or the x / y axis, Enter"),
        (Box::new(offset::OffsetTool::default()), "Offset: pick curves, Enter for the distance and side; closed shapes grow or shrink"),
        (Box::new(block::BlockTool::default()), "Block: click a region to block it; double-click an edge for its cells"),
        (Box::new(layer::LayerTool::default()), "Boundary layer: click block walls, Enter; double-click a wall for its settings (y+)"),
    ]
}
