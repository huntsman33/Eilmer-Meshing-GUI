//! Drawing tools: point, line, profile, arc, circle, rectangle, triangle, Bezier, spline.
//!
//! Typing a number while drawing locks a value live (line length, circle radius, arc radius then
//! sweep angle); the next click or Enter places the point with it.

use std::f64::consts::PI;

use egui::{Key, Modifiers, Pos2};

use emesh_core::entities::Curve;
use emesh_core::geom;
use emesh_core::num::P;
use emesh_core::typed_input::single_number;

use super::{draw_preview, AppEvent, Ctx, Tool, ToolEvent};
use crate::canvas::Draw;
use crate::theme::{col, Dash};

const TWO_PI: f64 = 2.0 * PI;

fn unit(v: P) -> P {
    let n = v[0].hypot(v[1]);
    if n > 0.0 {
        [v[0] / n, v[1] / n]
    } else {
        [1.0, 0.0]
    }
}

fn toward(origin: P, point: P, length: f64) -> P {
    let d = unit([point[0] - origin[0], point[1] - origin[1]]);
    [origin[0] + length * d[0], origin[1] + length * d[1]]
}

fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Construction point: a snap target drawn as '+'.
pub struct PointTool;

impl Tool for PointTool {
    fn name(&self) -> &'static str {
        "point"
    }
    fn label(&self) -> &'static str {
        "Point"
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        c.model.add_point(p);
    }
    fn prompt(&self, _c: &Ctx) -> String {
        "Point (x, y)".into()
    }
}

#[derive(Default)]
pub struct LineTool {
    p0: Option<P>,
    lock: Option<f64>,
    cursor: Option<P>,
}

impl LineTool {
    fn target(&self, p: P) -> P {
        match (self.p0, self.lock) {
            (Some(a), Some(l)) => toward(a, p, l),
            _ => p,
        }
    }
}

impl Tool for LineTool {
    fn name(&self) -> &'static str {
        "line"
    }
    fn label(&self) -> &'static str {
        "Line"
    }
    fn reset(&mut self) {
        self.p0 = None;
        self.lock = None;
    }
    fn busy(&self) -> bool {
        self.p0.is_some()
    }
    fn anchor(&self) -> Option<P> {
        self.p0
    }
    fn moved(&mut self, _c: &mut Ctx, ev: &ToolEvent) {
        self.cursor = Some(ev.point());
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        let Some(a) = self.p0 else {
            self.p0 = Some(p);
            return;
        };
        let end = self.target(p);
        if c.distinct(a, end) {
            let l = c.line(a, end);
            c.add(vec![l], "Line");
            self.p0 = None;
            if self.lock.take().is_some() {
                c.events.push(AppEvent::ClearTyped);
            }
        }
    }
    fn preview_text(&mut self, c: &mut Ctx, text: &str) {
        self.lock = if self.p0.is_some() { c.parse_length(text).filter(|l| *l > 0.0) } else { None };
    }
    fn accept_text(&mut self, c: &mut Ctx, text: &str) -> Result<(), String> {
        if let (Some(len), Some(a)) = (c.parse_length(text), self.p0) {
            if len <= 0.0 {
                return Err("length must be positive".into());
            }
            self.lock = Some(len);
            let cursor = match self.cursor {
                Some(q) if c.distinct(q, a) => q,
                _ => [a[0] + 1.0, a[1]],
            };
            self.click(c, cursor);
            return Ok(());
        }
        let p = c.parse_point(text, self.anchor())?;
        self.click(c, p);
        Ok(())
    }
    fn prompt(&self, c: &Ctx) -> String {
        match (self.p0, self.lock) {
            (None, _) => "Start point".into(),
            (_, Some(l)) => format!("Direction (length {})", c.fmt(l)),
            _ => "End point, or type a length".into(),
        }
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        if let (Some(a), Some(q)) = (self.p0, self.cursor) {
            let end = self.target(q);
            if c.distinct(a, end) {
                draw_preview(d, &[c.line(a, end)]);
            }
        }
    }
}

/// Chain of lines and tangent arcs. Drag from the last point (or press A) for an arc.
#[derive(Default)]
pub struct ProfileTool {
    start: Option<P>,
    last: Option<P>,
    tangent: Option<P>,
    arc_mode: bool,
    lock: Option<f64>,
    press_screen: Option<Pos2>,
    drag_arc: bool,
    cursor: Option<P>,
}

impl ProfileTool {
    const DRAG_PX: f32 = 8.0;

    fn line_end(&self, p: P) -> P {
        match (self.last, self.lock) {
            (Some(a), Some(l)) => toward(a, p, l),
            _ => p,
        }
    }

    fn add_line(&mut self, c: &mut Ctx, p: P) {
        let last = self.last.unwrap();
        let end = self.line_end(p);
        if !c.distinct(last, end) {
            return;
        }
        let l = c.line(last, end);
        c.add(vec![l], "Profile line");
        self.tangent = Some(unit([end[0] - last[0], end[1] - last[1]]));
        if self.lock.take().is_some() {
            c.events.push(AppEvent::ClearTyped);
        }
        self.advance(c, end);
    }

    fn add_arc(&mut self, c: &mut Ctx, p: P) {
        let last = self.last.unwrap();
        if !c.distinct(last, p) {
            return;
        }
        let Some(pieces) = geom::tangent_arc(last, self.tangent.unwrap(), p) else {
            self.add_line(c, p);
            return;
        };
        let curves = c.arcs(&pieces);
        self.tangent = Some(geom::end_tangent(curves.last().unwrap()));
        c.add(curves, "Profile arc");
        self.arc_mode = false;
        self.advance(c, p);
    }

    fn advance(&mut self, c: &Ctx, p: P) {
        self.last = Some(p);
        if !c.distinct(p, self.start.unwrap()) {
            self.reset(); // closed the loop
        }
    }

    fn preview_curves(&self, c: &Ctx) -> Vec<Curve> {
        let (Some(last), Some(cur)) = (self.last, self.cursor) else { return vec![] };
        if !c.distinct(last, cur) {
            return vec![];
        }
        if let (true, Some(t)) = (self.arc_mode || self.drag_arc, self.tangent) {
            if let Some(pieces) = geom::tangent_arc(last, t, cur) {
                return c.arcs(&pieces);
            }
        }
        let end = self.line_end(cur);
        if c.distinct(last, end) {
            vec![c.line(last, end)]
        } else {
            vec![]
        }
    }
}

impl Tool for ProfileTool {
    fn name(&self) -> &'static str {
        "profile"
    }
    fn label(&self) -> &'static str {
        "Profile"
    }
    fn reset(&mut self) {
        *self = ProfileTool { cursor: self.cursor, ..Default::default() };
    }
    fn busy(&self) -> bool {
        self.last.is_some()
    }
    fn anchor(&self) -> Option<P> {
        self.last
    }
    fn press(&mut self, _c: &mut Ctx, ev: &ToolEvent) {
        self.press_screen = Some(ev.screen);
        self.drag_arc = false;
    }
    fn moved(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        self.cursor = Some(ev.point());
        if let (true, Some(last), Some(_), Some(ps)) = (ev.dragging, self.last, self.tangent, self.press_screen) {
            let at_last = c.canvas.view.to_screen(last);
            let near_last = (ps - at_last).length() <= 10.0;
            let moved = (ev.screen - ps).length() > Self::DRAG_PX;
            self.drag_arc = near_last && moved;
        }
    }
    fn release(&mut self, c: &mut Ctx, ev: &ToolEvent) {
        self.press_screen = None;
        if self.drag_arc {
            self.drag_arc = false;
            self.add_arc(c, ev.point());
        } else {
            self.click(c, ev.point());
        }
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        if self.last.is_none() {
            self.start = Some(p);
            self.last = Some(p);
        } else if self.arc_mode && self.tangent.is_some() {
            self.add_arc(c, p);
        } else {
            self.add_line(c, p);
        }
    }
    fn claims_key(&self, key: Key) -> bool {
        key == Key::A && self.busy()
    }
    fn key(&mut self, _c: &mut Ctx, key: Key, _m: Modifiers) -> bool {
        if key == Key::A && self.tangent.is_some() {
            self.arc_mode = !self.arc_mode;
            return true;
        }
        false
    }
    fn preview_text(&mut self, c: &mut Ctx, text: &str) {
        let ok = self.last.is_some() && !self.arc_mode;
        self.lock = if ok { c.parse_length(text).filter(|l| *l > 0.0) } else { None };
    }
    fn accept_text(&mut self, c: &mut Ctx, text: &str) -> Result<(), String> {
        if let (Some(len), Some(last), false) = (c.parse_length(text), self.last, self.arc_mode) {
            if len <= 0.0 {
                return Err("length must be positive".into());
            }
            self.lock = Some(len);
            let t = self.tangent.unwrap_or([1.0, 0.0]);
            let cursor = match self.cursor {
                Some(q) if c.distinct(q, last) => q,
                _ => [last[0] + t[0], last[1] + t[1]],
            };
            self.add_line(c, cursor);
            return Ok(());
        }
        let p = c.parse_point(text, self.anchor())?;
        self.click(c, p);
        Ok(())
    }
    fn prompt(&self, c: &Ctx) -> String {
        if self.last.is_none() {
            "Start point".into()
        } else if self.arc_mode {
            "Arc end point".into()
        } else if let Some(l) = self.lock {
            format!("Direction (length {})", c.fmt(l))
        } else {
            "Next point, or type a length".into()
        }
    }
    fn hint(&self, _c: &Ctx) -> String {
        "Click to add lines; type a number to fix the length. Drag from the last point or press A for a tangent arc. \
         Right-click or Esc to finish."
            .into()
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        draw_preview(d, &self.preview_curves(c));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ArcMode {
    Centre,
    ThreePoint,
}

/// Centre mode (default): click the centre, type a radius (optional), click the start, type a
/// sweep angle (optional), click the end. 3-point mode (M): start, end, then a point on the arc.
pub struct ArcTool {
    mode: ArcMode,
    pts: Vec<P>,
    radius_lock: Option<f64>,
    /// radians, magnitude (sign follows the cursor)
    sweep_lock: Option<f64>,
    /// accumulated signed sweep following the cursor
    sweep: f64,
    last_angle: Option<f64>,
    cursor: Option<P>,
}

impl Default for ArcTool {
    fn default() -> Self {
        ArcTool { mode: ArcMode::Centre, pts: vec![], radius_lock: None, sweep_lock: None, sweep: 0.0, last_angle: None, cursor: None }
    }
}

impl ArcTool {
    fn radius(&self, p: P) -> f64 {
        self.radius_lock.unwrap_or_else(|| dist(self.pts[0], p))
    }

    fn angle(&self, p: P) -> f64 {
        let c = self.pts[0];
        (p[1] - c[1]).atan2(p[0] - c[0])
    }

    fn current_sweep(&self) -> f64 {
        let mut s = self.sweep;
        if let Some(l) = self.sweep_lock {
            s = l.copysign(if s != 0.0 { s } else { 1.0 });
        }
        s.clamp(-TWO_PI + 1e-9, TWO_PI - 1e-9)
    }

    fn centre_pieces(&self) -> Option<Vec<(P, P, P)>> {
        let (c, start) = (self.pts[0], self.pts[1]);
        let s = self.current_sweep();
        if s.abs() < 1e-9 {
            return None;
        }
        Some(geom::arc_pieces(c, dist(c, start), self.angle(start), s, Some(start), None))
    }
}

impl Tool for ArcTool {
    fn name(&self) -> &'static str {
        "arc"
    }
    fn label(&self) -> &'static str {
        "Arc"
    }
    fn reset(&mut self) {
        self.pts.clear();
        self.radius_lock = None;
        self.sweep_lock = None;
        self.sweep = 0.0;
        self.last_angle = None;
    }
    fn busy(&self) -> bool {
        !self.pts.is_empty()
    }
    fn anchor(&self) -> Option<P> {
        self.pts.last().copied()
    }
    fn moved(&mut self, _c: &mut Ctx, ev: &ToolEvent) {
        self.cursor = Some(ev.point());
        if self.mode == ArcMode::Centre && self.pts.len() == 2 {
            let a = self.angle(ev.point());
            if let Some(la) = self.last_angle {
                self.sweep += emesh_core::num::py_mod(a - la + PI, TWO_PI) - PI;
            }
            self.last_angle = Some(a);
        }
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        if let Some(l) = self.pts.last() {
            if !c.distinct(*l, p) {
                return;
            }
        }
        if self.mode == ArcMode::ThreePoint {
            if self.pts.len() < 2 {
                self.pts.push(p);
                return;
            }
            if let Some(pieces) = geom::arcs_through_3_points(self.pts[0], p, self.pts[1]) {
                let a = c.arcs(&pieces);
                c.add(a, "Arc");
                self.reset();
            }
            return;
        }
        match self.pts.len() {
            0 => self.pts.push(p),
            1 => {
                let r = self.radius(p);
                if r <= c.px(1.0) {
                    return;
                }
                let start = toward(self.pts[0], p, r);
                self.pts.push(start);
                self.sweep = 0.0;
                self.last_angle = Some(self.angle(start));
                if self.radius_lock.take().is_some() {
                    c.events.push(AppEvent::ClearTyped);
                }
            }
            _ => {
                if let Some(pieces) = self.centre_pieces() {
                    let a = c.arcs(&pieces);
                    c.add(a, "Arc");
                    if self.sweep_lock.is_some() {
                        c.events.push(AppEvent::ClearTyped);
                    }
                    self.reset();
                }
            }
        }
    }
    fn preview_text(&mut self, c: &mut Ctx, text: &str) {
        self.radius_lock = None;
        self.sweep_lock = None;
        if self.mode != ArcMode::Centre {
            return;
        }
        if self.pts.len() == 1 {
            self.radius_lock = c.parse_length(text).filter(|r| *r > 0.0);
        } else if self.pts.len() == 2 {
            if let Some(deg) = single_number(text) {
                if 0.0 < deg.abs() && deg.abs() < 360.0 {
                    self.sweep_lock = Some(deg.abs().to_radians());
                    if deg < 0.0 && self.sweep == 0.0 {
                        self.sweep = -1e-12; // a negative typed angle means clockwise
                    }
                }
            }
        }
    }
    fn accept_text(&mut self, c: &mut Ctx, text: &str) -> Result<(), String> {
        if self.mode == ArcMode::Centre && (self.pts.len() == 1 || self.pts.len() == 2) {
            self.preview_text(c, text);
            let locked = if self.pts.len() == 1 { self.radius_lock.is_some() } else { self.sweep_lock.is_some() };
            if locked {
                if self.pts.len() == 1 {
                    let a = self.pts[0];
                    let cursor = match self.cursor {
                        Some(q) if c.distinct(q, a) => q,
                        _ => [a[0] + 1.0, a[1]],
                    };
                    self.click(c, cursor);
                } else {
                    let q = self.cursor.unwrap_or(self.pts[1]);
                    self.click(c, q);
                }
                return Ok(());
            }
        }
        let p = c.parse_point(text, self.anchor())?;
        self.click(c, p);
        Ok(())
    }
    fn claims_key(&self, key: Key) -> bool {
        key == Key::M
    }
    fn key(&mut self, _c: &mut Ctx, key: Key, _m: Modifiers) -> bool {
        if key == Key::M {
            self.mode = if self.mode == ArcMode::Centre { ArcMode::ThreePoint } else { ArcMode::Centre };
            self.reset();
            return true;
        }
        false
    }
    fn prompt(&self, c: &Ctx) -> String {
        if self.mode == ArcMode::ThreePoint {
            return ["Arc start", "Arc end", "Point on arc"][self.pts.len().min(2)].into();
        }
        match self.pts.len() {
            0 => "Arc centre".into(),
            1 => match self.radius_lock {
                Some(r) => format!("Start direction (radius {})", c.fmt(r)),
                None => "Start point, or type a radius".into(),
            },
            _ => match self.sweep_lock {
                Some(s) => format!("End side (sweep {} deg)", emesh_core::pyfmt::g(s.to_degrees(), 6)),
                None => "End point, or type a sweep angle (deg)".into(),
            },
        }
    }
    fn hint(&self, _c: &Ctx) -> String {
        if self.mode == ArcMode::ThreePoint {
            return "Arc, 3-point: start, end, then a point on the arc. M switches to centre mode.".into();
        }
        "Arc: centre, then start (type a radius first to fix it), then end (type an angle to fix the sweep). \
         Move round the centre past 180 degrees for larger arcs. M switches to 3-point."
            .into()
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        let Some(cur) = self.cursor else { return };
        if self.pts.is_empty() {
            return;
        }
        if self.mode == ArcMode::ThreePoint {
            if self.pts.len() == 1 {
                if c.distinct(self.pts[0], cur) {
                    d.curve(&c.line(self.pts[0], cur), col::preview(), 1.0, Some(Dash::Dashed));
                }
                return;
            }
            if let Some(pieces) = geom::arcs_through_3_points(self.pts[0], cur, self.pts[1]) {
                draw_preview(d, &c.arcs(&pieces));
            }
            return;
        }
        let ctr = self.pts[0];
        if self.pts.len() == 1 {
            let r = self.radius(cur);
            if r > c.px(1.0) {
                for a in c.arcs(&geom::circle_arcs(ctr, r)) {
                    d.curve(&a, col::axes(), 1.0, Some(Dash::Dashed));
                }
                d.curve(&c.line(ctr, toward(ctr, cur, r)), col::preview(), 1.0, Some(Dash::Dashed));
            }
            return;
        }
        d.curve(&c.line(ctr, self.pts[1]), col::axes(), 1.0, Some(Dash::Dashed));
        if let Some(pieces) = self.centre_pieces() {
            draw_preview(d, &c.arcs(&pieces));
        }
    }
}

#[derive(Default)]
pub struct CircleTool {
    centre: Option<P>,
    lock: Option<f64>,
    cursor: Option<P>,
}

impl CircleTool {
    fn circle(&self, c: &Ctx, r: f64) -> Vec<Curve> {
        c.arcs(&geom::circle_arcs(self.centre.unwrap(), r))
    }

    fn radius(&self, p: P) -> f64 {
        self.lock.unwrap_or_else(|| dist(self.centre.unwrap(), p))
    }
}

impl Tool for CircleTool {
    fn name(&self) -> &'static str {
        "circle"
    }
    fn label(&self) -> &'static str {
        "Circle"
    }
    fn reset(&mut self) {
        self.centre = None;
        self.lock = None;
    }
    fn busy(&self) -> bool {
        self.centre.is_some()
    }
    fn anchor(&self) -> Option<P> {
        self.centre
    }
    fn moved(&mut self, _c: &mut Ctx, ev: &ToolEvent) {
        self.cursor = Some(ev.point());
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        if self.centre.is_none() {
            self.centre = Some(p);
            return;
        }
        let r = self.radius(p);
        if r > c.px(1.0) {
            let cs = self.circle(c, r);
            c.add(cs, "Circle");
            if self.lock.is_some() {
                c.events.push(AppEvent::ClearTyped);
            }
            self.reset();
        }
    }
    fn preview_text(&mut self, c: &mut Ctx, text: &str) {
        self.lock = if self.centre.is_some() { c.parse_length(text).filter(|r| *r > 0.0) } else { None };
    }
    fn accept_text(&mut self, c: &mut Ctx, text: &str) -> Result<(), String> {
        if let (Some(r), Some(_)) = (c.parse_length(text), self.centre) {
            if r <= 0.0 {
                return Err("radius must be positive".into());
            }
            let cs = self.circle(c, r);
            c.add(cs, "Circle");
            self.reset();
            return Ok(());
        }
        let p = c.parse_point(text, self.anchor())?;
        self.click(c, p);
        Ok(())
    }
    fn prompt(&self, c: &Ctx) -> String {
        match (self.centre, self.lock) {
            (None, _) => "Centre".into(),
            (_, Some(r)) => format!("Radius {}: click or Enter", c.fmt(r)),
            _ => "Radius, or point on circle".into(),
        }
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        if let (Some(_), Some(cur)) = (self.centre, self.cursor) {
            let r = self.radius(cur);
            if r > c.px(1.0) {
                draw_preview(d, &self.circle(c, r));
            }
        }
    }
}

fn rect_lines(c: &Ctx, a: P, b: P) -> Vec<Curve> {
    let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
    let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
    let k = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    (0..4).map(|i| c.line(k[i], k[(i + 1) % 4])).collect()
}

#[derive(Default)]
pub struct RectangleTool {
    p0: Option<P>,
    cursor: Option<P>,
}

impl Tool for RectangleTool {
    fn name(&self) -> &'static str {
        "rectangle"
    }
    fn label(&self) -> &'static str {
        "Rectangle"
    }
    fn reset(&mut self) {
        self.p0 = None;
    }
    fn busy(&self) -> bool {
        self.p0.is_some()
    }
    fn anchor(&self) -> Option<P> {
        self.p0
    }
    fn moved(&mut self, _c: &mut Ctx, ev: &ToolEvent) {
        self.cursor = Some(ev.point());
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        match self.p0 {
            None => self.p0 = Some(p),
            Some(a) => {
                if (p[0] - a[0]).abs() > c.px(1.0) && (p[1] - a[1]).abs() > c.px(1.0) {
                    let l = rect_lines(c, a, p);
                    c.add(l, "Rectangle");
                    self.reset();
                }
            }
        }
    }
    fn prompt(&self, _c: &Ctx) -> String {
        if self.p0.is_none() { "First corner" } else { "Opposite corner (@width, height)" }.into()
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        if let (Some(a), Some(q)) = (self.p0, self.cursor) {
            if q[0] != a[0] && q[1] != a[1] {
                draw_preview(d, &rect_lines(c, a, q));
            }
        }
    }
}

#[derive(Default)]
pub struct TriangleTool {
    pts: Vec<P>,
    cursor: Option<P>,
}

impl Tool for TriangleTool {
    fn name(&self) -> &'static str {
        "triangle"
    }
    fn label(&self) -> &'static str {
        "Triangle"
    }
    fn reset(&mut self) {
        self.pts.clear();
    }
    fn busy(&self) -> bool {
        !self.pts.is_empty()
    }
    fn anchor(&self) -> Option<P> {
        self.pts.last().copied()
    }
    fn moved(&mut self, _c: &mut Ctx, ev: &ToolEvent) {
        self.cursor = Some(ev.point());
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        if let Some(l) = self.pts.last() {
            if !c.distinct(*l, p) {
                return;
            }
        }
        self.pts.push(p);
        if self.pts.len() == 3 {
            let [a, b, cc] = [self.pts[0], self.pts[1], self.pts[2]];
            if ((b[0] - a[0]) * (cc[1] - a[1]) - (b[1] - a[1]) * (cc[0] - a[0])).abs() > c.px(1.0).powi(2) {
                let l = (0..3).map(|k| c.line(self.pts[k], self.pts[(k + 1) % 3])).collect();
                c.add(l, "Triangle");
            }
            self.reset();
        }
    }
    fn prompt(&self, _c: &Ctx) -> String {
        ["First vertex", "Second vertex", "Third vertex"][self.pts.len().min(2)].into()
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        let Some(cur) = self.cursor else { return };
        if self.pts.is_empty() {
            return;
        }
        let mut pts = self.pts.clone();
        pts.push(cur);
        let mut lines: Vec<Curve> = pts.windows(2).filter(|w| c.distinct(w[0], w[1])).map(|w| c.line(w[0], w[1])).collect();
        if pts.len() == 3 && c.distinct(pts[2], pts[0]) {
            lines.push(c.line(pts[2], pts[0]));
        }
        draw_preview(d, &lines);
    }
}

/// Collects points by clicking; right-click or Enter creates the curve (Bezier or spline).
pub struct PointListTool {
    bezier: bool,
    pts: Vec<P>,
    cursor: Option<P>,
}

impl PointListTool {
    pub fn bezier() -> Self {
        PointListTool { bezier: true, pts: vec![], cursor: None }
    }

    pub fn spline() -> Self {
        PointListTool { bezier: false, pts: vec![], cursor: None }
    }

    fn make(&self, c: &Ctx, pts: Vec<P>) -> Curve {
        let k = c.model.new_kind();
        if self.bezier {
            Curve::bezier(pts).with_kind(k)
        } else {
            Curve::spline(pts).with_kind(k)
        }
    }
}

impl Tool for PointListTool {
    fn name(&self) -> &'static str {
        if self.bezier {
            "bezier"
        } else {
            "spline"
        }
    }
    fn label(&self) -> &'static str {
        if self.bezier {
            "Bezier"
        } else {
            "Spline"
        }
    }
    fn reset(&mut self) {
        self.pts.clear();
    }
    fn busy(&self) -> bool {
        !self.pts.is_empty()
    }
    fn anchor(&self) -> Option<P> {
        self.pts.last().copied()
    }
    fn moved(&mut self, _c: &mut Ctx, ev: &ToolEvent) {
        self.cursor = Some(ev.point());
    }
    fn click(&mut self, c: &mut Ctx, p: P) {
        if self.pts.last().is_none_or(|l| c.distinct(*l, p)) {
            self.pts.push(p);
        }
    }
    fn finish(&mut self, c: &mut Ctx) {
        if self.pts.len() >= 2 {
            let curve = self.make(c, self.pts.clone());
            c.add(vec![curve], self.label());
        }
        self.reset();
    }
    fn prompt(&self, _c: &Ctx) -> String {
        format!("Point {} (right-click or Enter to finish)", self.pts.len() + 1)
    }
    fn hint(&self, _c: &Ctx) -> String {
        if self.bezier {
            "Click control points; the curve passes through the first and last. Right-click to finish.".into()
        } else {
            "Click points for the spline to pass through. Right-click to finish.".into()
        }
    }
    fn paint(&self, d: &mut Draw, c: &Ctx) {
        if self.pts.is_empty() {
            return;
        }
        let mut pts = self.pts.clone();
        if let Some(cur) = self.cursor {
            if c.distinct(*pts.last().unwrap(), cur) {
                pts.push(cur);
            }
        }
        if pts.len() >= 2 {
            draw_preview(d, &[self.make(c, pts.clone())]);
        }
        if self.bezier {
            for w in pts.windows(2) {
                d.curve(&c.line(w[0], w[1]), col::axes(), 1.0, Some(Dash::Dashed));
            }
        }
    }
}
