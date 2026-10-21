//! The sketch canvas: a world <-> screen transform kept in double precision (Eilmer geometries
//! are often millimetre-sized), picking, and the painting of everything on it.

use std::collections::{BTreeSet, HashSet};

use egui::{Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};

use emesh_core::blocking::Resolution;
use emesh_core::document::{Dimension, Document};
use emesh_core::entities::{Curve, Geom, Kind};
use emesh_core::geom;
use emesh_core::num::P;
use emesh_core::poly::Polygon;
use emesh_core::snapping::{nice_step, Snap, SnapKind, Snapper};
use emesh_core::topology::Topology;
use emesh_core::units::Units;
use emesh_mesh::live::{Mesh, Source};
use emesh_mesh::quality::MeshQuality;

use crate::model::SketchModel;
use crate::theme::{alpha, col, hex, kind_color, kind_dash, palette, polyline, ramp, Dash, REGION_FILLS};

pub const PICK_PX: f64 = 8.0;
pub const SNAP_PX: f64 = 10.0;
pub const HANDLE_PX: f32 = 4.0;
pub const POINT_PX: f32 = 5.0;

#[derive(Clone, Copy, Debug)]
pub struct View {
    /// pixels per metre
    pub scale: f64,
    pub cx: f64,
    pub cy: f64,
    pub rect: Rect,
}

impl Default for View {
    fn default() -> Self {
        View { scale: 200.0, cx: 0.0, cy: 0.0, rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)) }
    }
}

impl View {
    pub fn to_screen(&self, p: P) -> Pos2 {
        let c = self.rect.center();
        Pos2::new((c.x as f64 + (p[0] - self.cx) * self.scale) as f32, (c.y as f64 - (p[1] - self.cy) * self.scale) as f32)
    }

    pub fn to_world(&self, q: Pos2) -> P {
        let c = self.rect.center();
        [self.cx + (q.x as f64 - c.x as f64) / self.scale, self.cy - (q.y as f64 - c.y as f64) / self.scale]
    }

    /// World length of n pixels.
    pub fn px(&self, n: f64) -> f64 {
        n / self.scale
    }

    pub fn zoom_at(&mut self, factor: f64, screen: Pos2) {
        let before = self.to_world(screen);
        self.scale = (self.scale * factor).clamp(1e-6, 1e12);
        let after = self.to_world(screen);
        self.cx += before[0] - after[0];
        self.cy += before[1] - after[1];
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Skew,
    AspectRatio,
}

/// Canvas state the tools read and change.
pub struct Canvas {
    pub view: View,
    pub snapper: Snapper,
    pub units: Units,
    pub show_regions: bool,
    pub show_grid: bool,
    pub show_blocks: bool,
    pub show_mesh: bool,
    pub show_quality: bool,
    pub quality_metric: Metric,
    /// view only: block ids not drawn (still meshed and exported)
    pub hidden_blocks: BTreeSet<String>,
    pub mesh: Option<Mesh>,
    pub quality: Option<MeshQuality>,
    pub hover_id: Option<String>,
    pub last_snap: Option<Snap>,
    pub last_screen: Option<Pos2>,
    pub font: FontId,
    pub small_font: FontId,
    /// triangulated fills (world space), by key: (revision, ring, triangles)
    fills: std::cell::RefCell<std::collections::HashMap<String, (u64, Vec<P>, Vec<[usize; 3]>)>>,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            view: View::default(),
            snapper: Snapper::default(),
            units: Units::default(),
            show_regions: false,
            show_grid: true,
            show_blocks: true,
            show_mesh: true,
            show_quality: false,
            quality_metric: Metric::Skew,
            hidden_blocks: BTreeSet::new(),
            mesh: None,
            quality: None,
            hover_id: None,
            last_snap: None,
            last_screen: None,
            font: FontId::proportional(13.0),
            small_font: FontId::proportional(12.0),
            fills: Default::default(),
        }
    }
}

/// A painter in world coordinates.
pub struct Draw<'a> {
    pub painter: &'a egui::Painter,
    pub view: View,
    pub shapes: Vec<Shape>,
}

impl<'a> Draw<'a> {
    pub fn new(painter: &'a egui::Painter, view: View) -> Self {
        Draw { painter, view, shapes: vec![] }
    }

    pub fn s(&self, p: P) -> Pos2 {
        self.view.to_screen(p)
    }

    pub fn world_line(&mut self, pts: &[P], color: Color32, width: f32, dash: Dash) {
        let s: Vec<Pos2> = pts.iter().map(|p| self.s(*p)).collect();
        polyline(&s, Stroke::new(width, color), dash, &mut self.shapes);
    }

    pub fn curve(&mut self, c: &Curve, color: Color32, width: f32, dash: Option<Dash>) {
        let dash = dash.unwrap_or_else(|| kind_dash(c.kind));
        let pts = curve_points(c);
        self.world_line(&pts, color, width, dash);
    }

    pub fn screen_line(&mut self, pts: Vec<Pos2>, color: Color32, width: f32) {
        self.shapes.push(Shape::line(pts, Stroke::new(width, color)));
    }

    pub fn screen_dashed(&mut self, pts: &[Pos2], color: Color32, width: f32, dash: Dash) {
        polyline(pts, Stroke::new(width, color), dash, &mut self.shapes);
    }

    pub fn flush(&mut self) {
        let shapes = std::mem::take(&mut self.shapes);
        self.painter.extend(shapes);
    }

    pub fn text_box(&mut self, center: Pos2, text: &str, font: &FontId, color: Color32, bg: Color32) -> Rect {
        let galley = self.painter.layout_no_wrap(text.to_string(), font.clone(), color);
        let size = galley.size() + Vec2::new(8.0, 4.0);
        let rect = Rect::from_center_size(center, size);
        self.shapes.push(Shape::rect_filled(rect, 2.0, bg));
        self.shapes.push(Shape::galley(rect.min + Vec2::new(4.0, 2.0), galley, color));
        rect
    }
}

/// Points along a curve for drawing (exact for lines, sampled otherwise).
pub fn curve_points(c: &Curve) -> Vec<P> {
    match &c.geom {
        Geom::Line { p0, p1 } => vec![*p0, *p1],
        _ => geom::sample(c),
    }
}

pub fn label_bg() -> Color32 {
    alpha(col::background(), 210)
}

pub fn tag_color(doc: &Document, tag: &str) -> Color32 {
    crate::theme::tag(hex(emesh_lua::bcs::tag_color(doc, tag)))
}

impl Canvas {
    pub fn px(&self, n: f64) -> f64 {
        self.view.px(n)
    }

    /// Minor grid spacing in metres: a round number in the display units.
    pub fn grid_step(&self) -> f64 {
        let f = self.units.factor;
        nice_step(self.px(14.0) / f) * f
    }

    pub fn zoom_to_fit(&mut self, doc: &Document) {
        if doc.curves.is_empty() {
            self.view.cx = 0.0;
            self.view.cy = 0.0;
            self.view.scale = 200.0;
            return;
        }
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for c in doc.curves.values() {
            let (a, b) = geom::bbox(c);
            lo = [lo[0].min(a[0]), lo[1].min(a[1])];
            hi = [hi[0].max(b[0]), hi[1].max(b[1])];
        }
        let w = (hi[0] - lo[0]).max(1e-12);
        let h = (hi[1] - lo[1]).max(1e-12);
        self.view.cx = (lo[0] + hi[0]) / 2.0;
        self.view.cy = (lo[1] + hi[1]) / 2.0;
        self.view.scale = 0.85 * (self.view.rect.width() as f64 / w).min(self.view.rect.height() as f64 / h);
    }

    // --- snapping and picking ---------------------------------------------------------------------
    pub fn snap(&mut self, model: &SketchModel, world: P, anchor: Option<P>, exclude: &HashSet<String>) -> Snap {
        let tol = self.px(SNAP_PX);
        let step = self.grid_step();
        self.snapper.snap(&model.doc, model.revision, world, tol, Some(step), anchor, exclude)
    }

    pub fn pick_curve(&self, doc: &Document, world: P, tol_px: f64, among: Option<&[String]>) -> Option<String> {
        let tol = self.px(tol_px);
        let mut best: Option<(String, f64)> = None;
        let check = |c: &Curve, best: &mut Option<(String, f64)>| {
            let (_, d) = geom::closest_point(c, world);
            if d <= tol && best.as_ref().is_none_or(|b| d < b.1) {
                *best = Some((c.id.clone(), d));
            }
        };
        match among {
            Some(ids) => ids.iter().filter_map(|i| doc.curves.get(i)).for_each(|c| check(c, &mut best)),
            None => doc.curves.values().for_each(|c| check(c, &mut best)),
        }
        best.map(|b| b.0)
    }

    pub fn pick_point(&self, doc: &Document, screen: Pos2) -> Option<String> {
        let mut best: Option<(f32, String)> = None;
        for (pid, p) in &doc.points {
            let d = (self.view.to_screen(*p) - screen).length();
            if d <= POINT_PX + 3.0 && best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, pid.clone()));
            }
        }
        best.map(|b| b.1)
    }

    pub fn pick_dimension(&self, painter: &egui::Painter, doc: &Document, screen: Pos2) -> Option<String> {
        for (did, dim) in &doc.dimensions {
            if let Some(l) = self.dimension_layout(painter, doc, dim, None) {
                if l.rect.contains(screen) {
                    return Some(did.clone());
                }
            }
        }
        None
    }

    pub fn pick_entity(&self, painter: &egui::Painter, doc: &Document, screen: Pos2, world: P) -> Option<String> {
        self.pick_dimension(painter, doc, screen)
            .or_else(|| self.pick_point(doc, screen))
            .or_else(|| self.pick_curve(doc, world, PICK_PX, None))
    }

    /// (curve id, handle name, point) of a selected curve's handle under the cursor.
    pub fn pick_handle(&self, model: &SketchModel, screen: Pos2) -> Option<(String, String, P)> {
        let mut best: Option<(f32, (String, String, P))> = None;
        for cid in &model.selection {
            let Some(c) = model.doc.curves.get(cid) else { continue };
            for (name, p) in c.handles() {
                let d = (self.view.to_screen(p) - screen).length();
                if d <= HANDLE_PX + 4.0 && best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, (cid.clone(), name, p)));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Curves inside the world rectangle a-b (window), or touching it (crossing); and points.
    pub fn curves_in_rect(&self, doc: &Document, a: P, b: P, crossing: bool) -> Vec<String> {
        let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
        let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
        let bx = [[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]];
        let mut out = vec![];
        for (cid, c) in &doc.curves {
            let pts = geom::sample(c);
            let inside: Vec<bool> = pts.iter().map(|p| p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1).collect();
            let whole = inside.iter().all(|v| *v);
            if whole || (crossing && (inside.iter().any(|v| *v) || !geom::polyline_intersections(&pts, &bx).is_empty())) {
                out.push(cid.clone());
            }
        }
        out.extend(doc.points.iter().filter(|(_, p)| x0 <= p[0] && p[0] <= x1 && y0 <= p[1] && p[1] <= y1).map(|(k, _)| k.clone()));
        out
    }

    // --- hidden blocks --------------------------------------------------------------------------------
    pub fn hidden_ids(&self, res: &Resolution) -> BTreeSet<String> {
        self.hidden_blocks.iter().filter(|b| res.polygons.contains_key(*b)).cloned().collect()
    }

    pub fn set_blocks_hidden(&mut self, ids: &[String], hidden: bool) {
        for i in ids {
            if hidden {
                self.hidden_blocks.insert(i.clone());
            } else {
                self.hidden_blocks.remove(i);
            }
        }
    }

    /// Hide the blocks, or show them if they are all hidden already.
    pub fn toggle_blocks_hidden(&mut self, ids: &[String], res: &Resolution) {
        let hidden = self.hidden_ids(res);
        let all_hidden = ids.iter().all(|i| hidden.contains(i));
        self.set_blocks_hidden(ids, !all_hidden);
    }

    // --- dimensions -------------------------------------------------------------------------------------
    pub fn dimension_text(&self, doc: &Document, dim: &Dimension) -> String {
        let fmt = |v: f64| emesh_core::pyfmt::g(v, 6);
        if dim.curve2.is_some() {
            let v = emesh_core::dimensions::value(doc, dim).unwrap_or(0.0);
            if dim.kind == "distance" {
                return format!("{} {}", fmt(self.units.to_display(v)), self.units.name);
            }
            return format!("{}\u{b0}", fmt(v));
        }
        let c = &doc.curves[&dim.curve];
        if dim.kind == "length" {
            let (p0, p1) = c.ends_lp();
            return format!("{} {}", fmt(self.units.to_display(emesh_core::num::py_dist(p0, p1))), self.units.name);
        }
        let (_, _, _, sweep) = geom::arc_geom(c);
        format!("{}\u{b0}", fmt(sweep.abs().to_degrees()))
    }

    /// Screen-space drawing of a dimension: polylines, arrows (tip, direction), text, label rect.
    pub fn dimension_layout(&self, painter: &egui::Painter, doc: &Document, dim: &Dimension, offset: Option<f64>) -> Option<DimLayout> {
        let c = doc.curves.get(&dim.curve)?;
        if dim.curve_ids().iter().any(|i| !doc.curves.contains_key(*i)) {
            return None;
        }
        let offset = offset.unwrap_or(dim.offset);
        let v = &self.view;
        let s = |p: P| v.to_screen(p);
        let unit = |d: Vec2| if d.length() > 0.0 { d / d.length() } else { Vec2::ZERO };
        let mut polylines: Vec<Vec<Pos2>> = vec![];
        let mut arrows: Vec<(Pos2, Vec2)> = vec![];
        let anchor;
        if dim.curve2.is_some() {
            let (pl, ar, an) = self.two_line_layout(doc, dim, offset)?;
            polylines = pl;
            arrows = ar;
            anchor = an;
        } else if dim.kind == "length" {
            let (p0, p1) = c.ends_lp();
            let d = [p1[0] - p0[0], p1[1] - p0[1]];
            let l = d[0].hypot(d[1]);
            if l == 0.0 {
                return None;
            }
            let n = [-d[1] / l, d[0] / l];
            let sg = if offset >= 0.0 { 1.0 } else { -1.0 };
            let side = unit(s([p0[0] + n[0] * sg, p0[1] + n[1] * sg]) - s(p0));
            let a = s([p0[0] + n[0] * offset, p0[1] + n[1] * offset]);
            let b = s([p1[0] + n[0] * offset, p1[1] + n[1] * offset]);
            for (p, q) in [(s(p0), a), (s(p1), b)] {
                polylines.push(vec![p + side * 3.0, q + side * 6.0]);
            }
            polylines.push(vec![a, b]);
            let along = unit(b - a);
            arrows.push((a, -along));
            arrows.push((b, along));
            anchor = Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0) + side * 10.0;
        } else {
            let (centre, r, a0, sweep) = geom::arc_geom(c);
            let rd = (r + offset).max(0.05 * r);
            let th: Vec<f64> = (0..=40).map(|k| a0 + sweep * k as f64 / 40.0).collect();
            let ring: Vec<Pos2> = th.iter().map(|t| s([centre[0] + rd * t.cos(), centre[1] + rd * t.sin()])).collect();
            for t in [th[0], th[40]] {
                let u = [t.cos(), t.sin()];
                let radial = unit(s([centre[0] + u[0] * (rd + r), centre[1] + u[1] * (rd + r)]) - s(centre));
                polylines.push(vec![s([centre[0] + u[0] * r, centre[1] + u[1] * r]), s([centre[0] + u[0] * rd, centre[1] + u[1] * rd]) + radial * 6.0]);
            }
            arrows.push((ring[0], unit(ring[0] - ring[1])));
            arrows.push((ring[40], unit(ring[40] - ring[39])));
            let tm = a0 + sweep / 2.0;
            let um = [tm.cos(), tm.sin()];
            anchor = s([centre[0] + um[0] * rd, centre[1] + um[1] * rd])
                + unit(s([centre[0] + um[0] * (rd + r), centre[1] + um[1] * (rd + r)]) - s(centre)) * 12.0;
            polylines.push(ring);
        }
        let text = self.dimension_text(doc, dim);
        let galley = painter.layout_no_wrap(text.clone(), self.font.clone(), Color32::BLACK);
        let rect = Rect::from_center_size(anchor, galley.size() + Vec2::new(8.0, 4.0));
        Some(DimLayout { polylines, arrows, text, rect })
    }

    #[allow(clippy::type_complexity)]
    fn two_line_layout(&self, doc: &Document, dim: &Dimension, offset: f64) -> Option<(Vec<Vec<Pos2>>, Vec<(Pos2, Vec2)>, Pos2)> {
        use emesh_core::dimensions as d;
        let a = &doc.curves[&dim.curve];
        let b = &doc.curves[dim.curve2.as_ref()?];
        if !a.is_line() || !b.is_line() {
            return None;
        }
        let (a0, a1) = a.ends_lp();
        let (b0, b1) = b.ends_lp();
        if a0 == a1 || b0 == b1 {
            return None;
        }
        let v = &self.view;
        let s = |p: P| v.to_screen(p);
        let unit = |d: Vec2| if d.length() > 0.0 { d / d.length() } else { Vec2::ZERO };
        let mut polylines: Vec<Vec<Pos2>> = vec![];
        let extend = |line: &Curve, base: P, ray: P, target: P, polylines: &mut Vec<Vec<Pos2>>| {
            let (l0, l1) = line.ends_lp();
            let ts: Vec<f64> = [l0, l1].iter().map(|q| (q[0] - base[0]) * ray[0] + (q[1] - base[1]) * ray[1]).collect();
            let t = (target[0] - base[0]) * ray[0] + (target[1] - base[1]) * ray[1];
            if ts[0].min(ts[1]) - 1e-12 <= t && t <= ts[0].max(ts[1]) + 1e-12 {
                return; // the target is on the line itself
            }
            let p = s(if (ts[0] - t).abs() < (ts[1] - t).abs() { l0 } else { l1 });
            let q = s(target);
            let out = unit(q - p);
            polylines.push(vec![p + out * 3.0, q + out * 6.0]);
        };
        if dim.kind == "distance" {
            let (u, n) = (d::direction(a).ok()?, d::normal(a).ok()?);
            let pa = [a0[0] + u[0] * offset, a0[1] + u[1] * offset];
            let sd = d::signed_distance(a, b).ok()?;
            let pb = [pa[0] + n[0] * sd, pa[1] + n[1] * sd];
            extend(a, a0, u, pa, &mut polylines);
            extend(b, a0, u, pb, &mut polylines);
            let (sa, sb) = (s(pa), s(pb));
            polylines.push(vec![sa, sb]);
            let along = unit(sb - sa);
            return Some((polylines, vec![(sa, -along), (sb, along)], Pos2::new((sa.x + sb.x) / 2.0, (sa.y + sb.y) / 2.0)));
        }
        let vx = d::vertex(a, b)?;
        let rd = if offset > 0.0 { offset } else { self.px(30.0) };
        let (r1, _) = d::sector_rays(a, b, dim.sector).ok()?;
        let sweep = d::signed_sector_angle(a, b, dim.sector).ok()?;
        let t0 = r1[1].atan2(r1[0]);
        let th: Vec<f64> = (0..=40).map(|k| t0 + sweep * k as f64 / 40.0).collect();
        let ring: Vec<Pos2> = th.iter().map(|t| s([vx[0] + rd * t.cos(), vx[1] + rd * t.sin()])).collect();
        for (line, t) in [(a, th[0]), (b, th[40])] {
            let ray = [t.cos(), t.sin()];
            extend(line, vx, ray, [vx[0] + ray[0] * rd, vx[1] + ray[1] * rd], &mut polylines);
        }
        let arrows = vec![(ring[0], unit(ring[0] - ring[1])), (ring[40], unit(ring[40] - ring[39]))];
        let tm = th[0] + sweep / 2.0;
        let um = [tm.cos(), tm.sin()];
        let anchor = s([vx[0] + um[0] * rd, vx[1] + um[1] * rd]) + unit(s([vx[0] + um[0] * 2.0 * rd, vx[1] + um[1] * 2.0 * rd]) - s(vx)) * 12.0;
        polylines.push(ring);
        Some((polylines, arrows, anchor))
    }

    pub fn paint_dimension(&self, draw: &mut Draw, doc: &Document, dim: &Dimension, color: Color32, offset: Option<f64>) {
        let Some(l) = self.dimension_layout(draw.painter, doc, dim, offset) else { return };
        for line in &l.polylines {
            draw.screen_line(line.clone(), color, 1.2);
        }
        for (tip, d) in &l.arrows {
            let perp = Vec2::new(-d.y, d.x);
            draw.shapes.push(Shape::convex_polygon(
                vec![*tip, *tip - *d * 9.0 + perp * 3.2, *tip - *d * 9.0 - perp * 3.2],
                color,
                Stroke::NONE,
            ));
        }
        draw.shapes.push(Shape::rect_filled(l.rect, 2.0, alpha(col::background(), 220)));
        draw.shapes.push(Shape::text(&draw.painter.fonts(|f| f.clone()), l.rect.center(), Align2::CENTER_CENTER, &l.text, self.font.clone(), color));
    }
}

pub struct DimLayout {
    pub polylines: Vec<Vec<Pos2>>,
    pub arrows: Vec<(Pos2, Vec2)>,
    pub text: String,
    pub rect: Rect,
}

// --- painting ------------------------------------------------------------------------------------------

pub struct PaintInput<'a> {
    pub model: &'a SketchModel,
    pub topo: &'a Topology,
    pub res: &'a Resolution,
}

impl Canvas {
    pub fn paint_background(&self, draw: &mut Draw) {
        let r = self.view.rect;
        draw.shapes.push(Shape::rect_filled(r, 0.0, col::background()));
        if !self.show_grid {
            return;
        }
        let step = self.grid_step();
        let mant = (step / 10f64.powf(step.log10().floor())).round();
        let major = step * if mant == 5.0 { 2.0 } else { 5.0 };
        let [x0, y1] = self.view.to_world(r.min);
        let [x1, y0] = self.view.to_world(r.max);
        for (spacing, color) in [(step, col::grid_minor()), (major, col::grid_major())] {
            let n = (x1 - x0) / spacing + (y1 - y0) / spacing;
            if n > 600.0 {
                continue;
            }
            let stroke = Stroke::new(1.0f32, color);
            let mut i = (x0 / spacing).floor();
            while i * spacing <= x1 {
                let sx = self.view.to_screen([i * spacing, 0.0]).x.round() + 0.5;
                draw.shapes.push(Shape::line_segment([Pos2::new(sx, r.top()), Pos2::new(sx, r.bottom())], stroke));
                i += 1.0;
            }
            let mut j = (y0 / spacing).floor();
            while j * spacing <= y1 {
                let sy = self.view.to_screen([0.0, j * spacing]).y.round() + 0.5;
                draw.shapes.push(Shape::line_segment([Pos2::new(r.left(), sy), Pos2::new(r.right(), sy)], stroke));
                j += 1.0;
            }
        }
        let o = self.view.to_screen([0.0, 0.0]);
        let stroke = Stroke::new(1.0f32, col::axes());
        if r.x_range().contains(o.x) {
            draw.shapes.push(Shape::line_segment([Pos2::new(o.x, r.top()), Pos2::new(o.x, r.bottom())], stroke));
        }
        if r.y_range().contains(o.y) {
            draw.shapes.push(Shape::line_segment([Pos2::new(r.left(), o.y), Pos2::new(r.right(), o.y)], stroke));
        }
    }

    /// egui fills convex polygons only: triangulate by ear clipping (holes bridged), once per
    /// revision, in world space.
    fn fill_polygon(&self, draw: &mut Draw, key: &str, revision: u64, shell: &[P], holes: &[Vec<P>], color: Color32) {
        let mut fills = self.fills.borrow_mut();
        let fresh = fills.get(key).is_none_or(|f| f.0 != revision);
        if fresh {
            let ring = bridge_holes(shell, holes);
            let tris = ear_clip(&ring);
            fills.insert(key.to_string(), (revision, ring, tris));
        }
        let (_, ring, tris) = &fills[key];
        let mut mesh = egui::Mesh::default();
        for p in ring {
            mesh.colored_vertex(self.view.to_screen(*p), color);
        }
        for t in tris {
            mesh.add_triangle(t[0] as u32, t[1] as u32, t[2] as u32);
        }
        draw.shapes.push(Shape::mesh(mesh));
    }

    pub fn paint_world(&self, draw: &mut Draw, inp: &PaintInput) {
        let doc = &inp.model.doc;
        if self.show_regions {
            for (k, r) in inp.topo.regions.iter().enumerate() {
                let c = alpha(hex(REGION_FILLS[k % REGION_FILLS.len()]), palette().region_alpha);
                self.fill_polygon(draw, &format!("r{k}"), inp.model.revision, &r.polygon, &r.hole_polygons, c);
            }
        }
        let hidden = self.hidden_ids(inp.res);
        if self.show_blocks && !doc.blocks.is_empty() {
            for (bid, poly) in &inp.res.polygons {
                if hidden.contains(bid) || poly.len() < 3 {
                    continue;
                }
                let err = inp.res.errors.contains_key(bid);
                let c = if err { alpha(col::block_error(), 55) } else { alpha(col::block(), palette().block_alpha) };
                self.fill_polygon(draw, &format!("b{bid}"), inp.model.revision, poly, &[], c);
            }
            for bid in &hidden {
                let poly = &inp.res.polygons[bid];
                if poly.len() >= 3 {
                    let mut pts: Vec<Pos2> = poly.iter().map(|p| self.view.to_screen(*p)).collect();
                    pts.push(pts[0]);
                    draw.screen_dashed(&pts, alpha(col::block(), 110), 1.0, Dash::Dashed);
                }
            }
        }
        if let (true, Some(q), Some(m)) = (self.show_quality, &self.quality, &self.mesh) {
            self.paint_quality(draw, q, m, &hidden);
        }
        if let (true, Some(m)) = (self.show_mesh, &self.mesh) {
            let c = alpha(col::mesh(), if m.source == Source::Eilmer { 150 } else { 90 });
            let stroke = Stroke::new(0.8f32, c);
            for (bid, g) in &m.grids {
                if hidden.contains(bid) {
                    continue;
                }
                for i in 0..g.niv {
                    draw.shapes.push(Shape::line((0..g.njv).map(|j| self.view.to_screen(g.at(i, j))).collect(), stroke));
                }
                for j in 0..g.njv {
                    draw.shapes.push(Shape::line((0..g.niv).map(|i| self.view.to_screen(g.at(i, j))).collect(), stroke));
                }
            }
        }
        for (cid, c) in &doc.curves {
            let (color, width) = if inp.model.selection.contains(cid) {
                (col::selected(), 2.5)
            } else if self.hover_id.as_deref() == Some(cid) {
                (col::hover(), 2.5)
            } else if !c.bc_tag.is_empty() && c.kind == Kind::Geometry && doc.bcs.contains_key(&c.bc_tag) {
                (tag_color(doc, &c.bc_tag), 2.6)
            } else {
                (kind_color(c.kind), 1.5)
            };
            draw.curve(c, color, width, None);
        }
    }

    fn paint_quality(&self, draw: &mut Draw, q: &MeshQuality, m: &Mesh, hidden: &BTreeSet<String>) {
        let n = 10usize;
        let mut mesh = egui::Mesh::default();
        let limit = q.thresholds.ar_warn;
        for (bid, g) in &m.grids {
            if hidden.contains(bid) {
                continue;
            }
            let (Some(cm), Some(bq)) = (q.metrics.get(bid), q.blocks.get(bid)) else { continue };
            if cm.ni != g.niv - 1 || cm.nj != g.njv - 1 {
                continue;
            }
            for i in 0..cm.ni {
                for j in 0..cm.nj {
                    let k = i * cm.nj + j;
                    let color = if cm.inverted[k] {
                        palette().inverted
                    } else {
                        let v = match self.quality_metric {
                            Metric::Skew => Some(cm.skew[k]),
                            Metric::AspectRatio => {
                                if bq.check_ar {
                                    Some((cm.ar[k].ln() / limit.ln()).clamp(0.0, 1.0))
                                } else {
                                    None
                                }
                            }
                        };
                        match v {
                            Some(v) if v.is_finite() => ramp((((v * n as f64) as usize).min(n - 1) as f64 + 0.5) / n as f64),
                            Some(_) => ramp(1.0),
                            None => palette().no_metric,
                        }
                    };
                    let base = mesh.vertices.len() as u32;
                    for p in [g.at(i, j), g.at(i + 1, j), g.at(i + 1, j + 1), g.at(i, j + 1)] {
                        mesh.colored_vertex(self.view.to_screen(p), color);
                    }
                    mesh.add_triangle(base, base + 1, base + 2);
                    mesh.add_triangle(base, base + 2, base + 3);
                }
            }
        }
        draw.shapes.push(Shape::mesh(mesh));
    }

    pub fn paint_overlay(&self, draw: &mut Draw, inp: &PaintInput) {
        let doc = &inp.model.doc;
        let hidden = self.hidden_ids(inp.res);
        if self.show_blocks && !doc.blocks.is_empty() {
            let blocks = inp.res.by_id();
            for (bid, poly) in &inp.res.polygons {
                if poly.len() < 3 || hidden.contains(bid) {
                    continue;
                }
                let c = Polygon::new(poly).representative_point();
                let (text, color) = if inp.res.errors.contains_key(bid) {
                    (format!("{bid}: needs attention"), col::block_error())
                } else {
                    let b = blocks[bid.as_str()];
                    let mut t = format!("{bid}  {} x {}", b.niv - 1, b.njv - 1);
                    let mut color = col::mesh();
                    if b.patch == "AOPatch" {
                        t += "  AO";
                    }
                    if b.smoothed() {
                        t += "  smoothed";
                    }
                    if inp.res.warnings.contains_key(bid) {
                        t += "  (!)";
                        color = col::block_error();
                    }
                    (t, color)
                };
                let center = self.view.to_screen(c);
                draw.text_box(center, &text, &self.small_font, color, alpha(col::background(), 200));
            }
        }
        for (did, dim) in &doc.dimensions {
            let color = if inp.model.selection.contains(did) {
                col::selected()
            } else if self.hover_id.as_deref() == Some(did) {
                col::hover()
            } else {
                col::dimension()
            };
            self.paint_dimension(draw, doc, dim, color, None);
        }
        for (pid, p) in &doc.points {
            let (color, width) = if inp.model.selection.contains(pid) {
                (col::selected(), 2.2)
            } else if self.hover_id.as_deref() == Some(pid) {
                (col::hover(), 2.2)
            } else {
                (col::geometry(), 1.5)
            };
            let q = self.view.to_screen(*p);
            let r = POINT_PX;
            draw.screen_line(vec![q - Vec2::new(r, 0.0), q + Vec2::new(r, 0.0)], color, width);
            draw.screen_line(vec![q - Vec2::new(0.0, r), q + Vec2::new(0.0, r)], color, width);
        }
        self.paint_handles(draw, inp.model);
    }

    fn paint_handles(&self, draw: &mut Draw, model: &SketchModel) {
        let sel: Vec<&Curve> = model.selection.iter().filter_map(|i| model.doc.curves.get(i)).collect();
        if sel.is_empty() {
            return;
        }
        let stroke = Stroke::new(1.2f32, col::selected());
        let mut centres: Vec<P> = vec![];
        for c in &sel {
            if let Some(ctr) = c.centre() {
                if !centres.iter().any(|q| (q[0] - ctr[0]).abs() < 1e-12 && (q[1] - ctr[1]).abs() < 1e-12) {
                    centres.push(ctr);
                }
            }
        }
        for ctr in centres {
            let q = self.view.to_screen(ctr);
            draw.shapes.push(Shape::circle_stroke(q, 3.0, stroke));
            draw.shapes.push(Shape::line_segment([q - Vec2::new(7.0, 0.0), q + Vec2::new(7.0, 0.0)], stroke));
            draw.shapes.push(Shape::line_segment([q - Vec2::new(0.0, 7.0), q + Vec2::new(0.0, 7.0)], stroke));
        }
        let h = HANDLE_PX;
        for c in sel {
            for (name, p) in c.handles() {
                let q = self.view.to_screen(p);
                if name == "mid" {
                    let pts = vec![q + Vec2::new(0.0, -h - 1.0), q + Vec2::new(h + 1.0, 0.0), q + Vec2::new(0.0, h + 1.0), q + Vec2::new(-h - 1.0, 0.0)];
                    draw.shapes.push(Shape::convex_polygon(pts, col::handle(), stroke));
                } else if name.starts_with("i:") {
                    draw.shapes.push(Shape::Circle(egui::epaint::CircleShape { center: q, radius: h - 1.5, fill: col::handle(), stroke }));
                } else {
                    draw.shapes.push(Shape::Rect(egui::epaint::RectShape::new(Rect::from_center_size(q, Vec2::splat(2.0 * h)), 0.0, col::handle(), stroke, egui::StrokeKind::Middle)));
                }
            }
        }
    }

    pub fn paint_snap(&self, draw: &mut Draw, anchor: Option<P>) {
        let Some(s) = &self.last_snap else { return };
        if s.kind == SnapKind::None {
            return;
        }
        let q = self.view.to_screen(s.point);
        let r = 6.0;
        let stroke = Stroke::new(1.6f32, col::snap());
        let seg = |a: Vec2, b: Vec2| Shape::line_segment([q + a, q + b], stroke);
        match s.kind {
            SnapKind::Endpoint | SnapKind::Point => {
                draw.shapes.push(Shape::rect_stroke(Rect::from_center_size(q, Vec2::splat(2.0 * r)), 0.0, stroke, egui::StrokeKind::Middle))
            }
            SnapKind::Intersection => {
                draw.shapes.push(seg(Vec2::new(-r, -r), Vec2::new(r, r)));
                draw.shapes.push(seg(Vec2::new(-r, r), Vec2::new(r, -r)));
            }
            SnapKind::Centre => {
                draw.shapes.push(Shape::circle_stroke(q, r, stroke));
                draw.shapes.push(seg(Vec2::new(-r / 2.0, 0.0), Vec2::new(r / 2.0, 0.0)));
                draw.shapes.push(seg(Vec2::new(0.0, -r / 2.0), Vec2::new(0.0, r / 2.0)));
            }
            SnapKind::Midpoint => draw.shapes.push(Shape::closed_line(
                vec![q + Vec2::new(0.0, -r), q + Vec2::new(r, r * 0.8), q + Vec2::new(-r, r * 0.8)],
                stroke,
            )),
            SnapKind::OnCurve => draw.shapes.push(Shape::circle_stroke(q, r * 0.6, stroke)),
            SnapKind::Horizontal | SnapKind::Vertical => {
                if let Some(a) = anchor {
                    draw.screen_dashed(&[self.view.to_screen(a), q], col::snap(), 1.0, Dash::Dashed);
                }
                let off = Vec2::new(r, -r);
                if s.kind == SnapKind::Horizontal {
                    draw.shapes.push(seg(off + Vec2::new(-4.0, 0.0), off + Vec2::new(4.0, 0.0)));
                } else {
                    draw.shapes.push(seg(off + Vec2::new(0.0, -4.0), off + Vec2::new(0.0, 4.0)));
                }
            }
            SnapKind::Grid => {
                draw.shapes.push(seg(Vec2::new(-r / 2.0, 0.0), Vec2::new(r / 2.0, 0.0)));
                draw.shapes.push(seg(Vec2::new(0.0, -r / 2.0), Vec2::new(0.0, r / 2.0)));
            }
            SnapKind::None => {}
        }
    }

    pub fn paint_legends(&self, draw: &mut Draw, doc: &Document) {
        let r = self.view.rect;
        let mut used: Vec<String> = doc
            .curves
            .values()
            .filter(|c| c.kind == Kind::Geometry && doc.bcs.contains_key(&c.bc_tag))
            .map(|c| c.bc_tag.clone())
            .collect();
        used.sort();
        used.dedup();
        if !used.is_empty() {
            let mut rows: Vec<(Color32, String)> = used.iter().map(|t| (tag_color(doc, t), format!("{t}   {}", doc.bcs[t].class))).collect();
            rows.push((col::geometry(), "untagged   slip wall (default)".into()));
            let h = 18.0;
            let w = rows.iter().map(|(_, t)| draw.painter.layout_no_wrap(t.clone(), self.small_font.clone(), Color32::BLACK).size().x).fold(0.0, f32::max) + 40.0;
            let bx = Rect::from_min_size(Pos2::new(r.left() + 10.0, r.bottom() - 10.0 - h * rows.len() as f32 - 8.0), Vec2::new(w, h * rows.len() as f32 + 8.0));
            draw.shapes.push(Shape::Rect(egui::epaint::RectShape::new(bx, 3.0, alpha(col::background(), 230), Stroke::new(1.0f32, col::frame()), egui::StrokeKind::Inside)));
            for (k, (color, text)) in rows.iter().enumerate() {
                let y = bx.top() + 4.0 + h * k as f32 + h / 2.0;
                draw.shapes.push(Shape::line_segment([Pos2::new(bx.left() + 8.0, y), Pos2::new(bx.left() + 24.0, y)], Stroke::new(3.0f32, *color)));
                draw.shapes.push(Shape::text(&draw.painter.fonts(|f| f.clone()), Pos2::new(bx.left() + 30.0, y), Align2::LEFT_CENTER, text, self.small_font.clone(), col::geometry()));
            }
        }
        if let (true, Some(q)) = (self.show_quality, &self.quality) {
            let (title, lo, hi) = match self.quality_metric {
                Metric::Skew => ("Skewness".to_string(), "0".to_string(), "1".to_string()),
                Metric::AspectRatio => (
                    "Aspect ratio   (grey: boundary layer)".to_string(),
                    "1".to_string(),
                    format!("{}+", emesh_core::pyfmt::g(q.thresholds.ar_warn, 6)),
                ),
            };
            let (w, h) = (170.0f32, 10.0f32);
            let (x, y) = (r.right() - w - 16.0, r.bottom() - 52.0);
            draw.shapes.push(Shape::rect_filled(Rect::from_min_size(Pos2::new(x - 8.0, y - 24.0), Vec2::new(w + 16.0, 60.0)), 3.0, alpha(col::background(), 225)));
            for k in 0..w as usize {
                draw.shapes.push(Shape::rect_filled(
                    Rect::from_min_size(Pos2::new(x + k as f32, y), Vec2::new(1.2, h)),
                    0.0,
                    ramp(k as f64 / (w as f64 - 1.0)),
                ));
            }
            let fonts = draw.painter.fonts(|f| f.clone());
            let f = self.small_font.clone();
            draw.shapes.push(Shape::text(&fonts, Pos2::new(x, y - 4.0), Align2::LEFT_BOTTOM, &title, f.clone(), col::mesh()));
            draw.shapes.push(Shape::text(&fonts, Pos2::new(x, y + h + 3.0), Align2::LEFT_TOP, &lo, f.clone(), col::mesh()));
            draw.shapes.push(Shape::text(&fonts, Pos2::new(x + w, y + h + 3.0), Align2::RIGHT_TOP, &hi, f.clone(), col::mesh()));
            draw.shapes.push(Shape::rect_filled(Rect::from_min_size(Pos2::new(x + w / 2.0 - 34.0, y + h + 5.0), Vec2::splat(9.0)), 0.0, palette().inverted));
            draw.shapes.push(Shape::text(&fonts, Pos2::new(x + w / 2.0 - 21.0, y + h + 3.0), Align2::LEFT_TOP, "inverted", f, col::mesh()));
        }
    }
}

// --- polygon filling ---------------------------------------------------------------------------------

/// One ring from a shell and holes, each hole joined to the shell by a zero-width bridge.
fn bridge_holes(shell: &[P], holes: &[Vec<P>]) -> Vec<P> {
    let mut ring: Vec<P> = shell.to_vec();
    if emesh_core::poly::signed_area(&ring) < 0.0 {
        ring.reverse();
    }
    let mut hs: Vec<Vec<P>> = holes.iter().filter(|h| h.len() >= 3).cloned().collect();
    // join the holes with the largest x first, each at its rightmost vertex
    hs.sort_by(|a, b| {
        let ma = a.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let mb = b.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        mb.partial_cmp(&ma).unwrap()
    });
    for mut h in hs {
        if emesh_core::poly::signed_area(&h) > 0.0 {
            h.reverse();
        }
        let (hi, hp) = h.iter().enumerate().max_by(|a, b| a.1[0].partial_cmp(&b.1[0]).unwrap()).map(|(i, p)| (i, *p)).unwrap();
        // the nearest ring vertex to the right that the bridge can see (approximate: nearest)
        let mut best = 0;
        let mut bd = f64::INFINITY;
        for (k, p) in ring.iter().enumerate() {
            let d = (p[0] - hp[0]).hypot(p[1] - hp[1]);
            if p[0] >= hp[0] && d < bd {
                bd = d;
                best = k;
            }
        }
        if bd == f64::INFINITY {
            for (k, p) in ring.iter().enumerate() {
                let d = (p[0] - hp[0]).hypot(p[1] - hp[1]);
                if d < bd {
                    bd = d;
                    best = k;
                }
            }
        }
        let mut merged: Vec<P> = ring[..=best].to_vec();
        for k in 0..=h.len() {
            merged.push(h[(hi + k) % h.len()]);
        }
        merged.extend_from_slice(&ring[best..]);
        ring = merged;
    }
    ring
}

/// Triangles (vertex indices) of a simple polygon, by ear clipping.
fn ear_clip(pts: &[P]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return vec![];
    }
    let area: f64 = (0..n).map(|k| pts[k][0] * pts[(k + 1) % n][1] - pts[(k + 1) % n][0] * pts[k][1]).sum();
    let sign = if area > 0.0 { 1.0 } else { -1.0 };
    let cross = |a: P, b: P, c: P| ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * sign;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = vec![];
    let mut guard = 0;
    while idx.len() > 3 && guard < 4 * n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let (a, b, c) = (idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]);
            if cross(pts[a], pts[b], pts[c]) <= 0.0 {
                continue;
            }
            let inside = idx.iter().any(|&o| {
                if o == a || o == b || o == c {
                    return false;
                }
                let p = pts[o];
                cross(pts[a], pts[b], p) > 0.0 && cross(pts[b], pts[c], p) > 0.0 && cross(pts[c], pts[a], p) > 0.0
            });
            if inside {
                continue;
            }
            out.push([a, b, c]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            // degenerate (e.g. a bridge): drop the flattest vertex
            let m = idx.len();
            let k = (0..m)
                .min_by(|&i, &j| {
                    let f = |k: usize| cross(pts[idx[(k + m - 1) % m]], pts[idx[k]], pts[idx[(k + 1) % m]]).abs();
                    f(i).partial_cmp(&f(j)).unwrap()
                })
                .unwrap();
            out.push([idx[(k + m - 1) % m], idx[k], idx[(k + 1) % m]]);
            idx.remove(k);
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}
