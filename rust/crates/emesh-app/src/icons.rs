//! Toolbar icons drawn as vector shapes (32 x 32 design units), so the app ships without image
//! files and stays sharp at any display scale.

use egui::{Color32, Pos2, Rect, Shape, Stroke};

use crate::theme::{col, hex, palette, Dash};

struct Icon<'a> {
    rect: Rect,
    out: &'a mut Vec<Shape>,
}

impl Icon<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.rect.min + egui::vec2(x, y) * (self.rect.width() / 32.0)
    }

    fn s(&self, w: f32) -> f32 {
        w * self.rect.width() / 32.0
    }

    fn line(&mut self, pts: &[(f32, f32)], color: Color32, w: f32) {
        self.dash(pts, color, w, Dash::Solid);
    }

    fn dash(&mut self, pts: &[(f32, f32)], color: Color32, w: f32, dash: Dash) {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| self.p(x, y)).collect();
        let stroke = Stroke::new(self.s(w), color);
        if dash == Dash::Solid {
            self.out.push(Shape::line(pts, stroke));
        } else {
            // icon dashes are short: pattern in design units
            let scale = self.rect.width() / 32.0;
            let pat = match dash {
                Dash::DashDot => [3.0, 1.2, 0.6, 1.2],
                _ => [2.0 * w, 1.6 * w, 2.0 * w, 1.6 * w],
            };
            let mut shapes = vec![];
            dashed(&pts, &pat.map(|v| v * scale), stroke, &mut shapes);
            self.out.extend(shapes);
        }
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: Option<Color32>, stroke: Option<(Color32, f32)>) {
        let r = Rect::from_min_max(self.p(x, y), self.p(x + w, y + h));
        if let Some(f) = fill {
            self.out.push(Shape::rect_filled(r, 0.0, f));
        }
        if let Some((c, w)) = stroke {
            self.out.push(Shape::rect_stroke(r, 0.0, Stroke::new(self.s(w), c), egui::StrokeKind::Middle));
        }
    }

    fn poly(&mut self, pts: &[(f32, f32)], fill: Color32) {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.out.push(Shape::convex_polygon(pts, fill, Stroke::NONE));
    }

    fn closed(&mut self, pts: &[(f32, f32)], color: Color32, w: f32) {
        let mut v = pts.to_vec();
        v.push(pts[0]);
        self.line(&v, color, w);
    }

    fn circle(&mut self, x: f32, y: f32, r: f32, color: Color32, w: f32) {
        self.out.push(Shape::circle_stroke(self.p(x, y), self.s(r), Stroke::new(self.s(w), color)));
    }

    fn dots(&mut self, pts: &[(f32, f32)]) {
        for &(x, y) in pts {
            self.out.push(Shape::circle_filled(self.p(x, y), self.s(2.4), col::accent()));
        }
    }

    fn quad(&mut self, a: (f32, f32), c: (f32, f32), b: (f32, f32), color: Color32, w: f32) {
        let pts: Vec<(f32, f32)> = (0..=24)
            .map(|k| {
                let t = k as f32 / 24.0;
                let u = 1.0 - t;
                (u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1)
            })
            .collect();
        self.line(&pts, color, w);
    }

    fn cubic(&mut self, a: (f32, f32), c1: (f32, f32), c2: (f32, f32), b: (f32, f32), color: Color32, w: f32) {
        let pts: Vec<(f32, f32)> = (0..=24)
            .map(|k| {
                let t = k as f32 / 24.0;
                let u = 1.0 - t;
                let f = |a: f32, c1: f32, c2: f32, b: f32| u * u * u * a + 3.0 * u * u * t * c1 + 3.0 * u * t * t * c2 + t * t * t * b;
                (f(a.0, c1.0, c2.0, b.0), f(a.1, c1.1, c2.1, b.1))
            })
            .collect();
        self.line(&pts, color, w);
    }
}

fn dashed(points: &[Pos2], pat: &[f32], stroke: Stroke, out: &mut Vec<Shape>) {
    let (mut k, mut left, mut on) = (0usize, pat[0], true);
    let mut cur = vec![points[0]];
    for w in points.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = (b - a).length();
        while seg > 0.0 {
            if seg < left {
                left -= seg;
                if on {
                    cur.push(b);
                }
                seg = 0.0;
            } else {
                let p = a + (b - a) * (left / seg);
                if on {
                    cur.push(p);
                    out.push(Shape::line(std::mem::take(&mut cur), stroke));
                } else {
                    cur = vec![p];
                }
                seg -= left;
                a = p;
                on = !on;
                k = (k + 1) % pat.len();
                left = pat[k];
            }
        }
    }
    if on && cur.len() >= 2 {
        out.push(Shape::line(cur, stroke));
    }
}

/// Shapes for icon `name` filling `rect`.
pub fn shapes(name: &str, rect: Rect) -> Vec<Shape> {
    let mut out = vec![];
    let mut i = Icon { rect, out: &mut out };
    let ink = col::ink();
    let acc = col::accent();
    let refc = col::split();
    let red = col::trim();
    let fills = palette().icon_fills;
    match name {
        "select" => {
            let pts = [(9.0, 5.0), (9.0, 25.0), (14.0, 20.0), (18.0, 28.0), (21.0, 26.0), (17.0, 19.0), (24.0, 19.0)];
            // concave: draw as outline plus a filled core
            i.poly(&[(9.0, 5.0), (9.0, 25.0), (14.0, 20.0), (24.0, 19.0)], ink);
            i.poly(&[(14.0, 20.0), (18.0, 28.0), (21.0, 26.0), (17.0, 19.0)], ink);
            i.closed(&pts, ink, 1.2);
        }
        "line" => {
            i.line(&[(6.0, 26.0), (26.0, 6.0)], ink, 2.0);
            i.dots(&[(6.0, 26.0), (26.0, 6.0)]);
        }
        "profile" => {
            i.line(&[(5.0, 26.0), (12.0, 10.0), (19.0, 20.0)], ink, 2.0);
            i.quad((19.0, 20.0), (24.0, 6.0), (28.0, 8.0), ink, 2.0);
            i.dots(&[(5.0, 26.0), (12.0, 10.0), (19.0, 20.0)]);
        }
        "arc" => {
            let pts: Vec<(f32, f32)> = (0..=24)
                .map(|k| {
                    let a = std::f32::consts::PI * k as f32 / 24.0;
                    (17.0 + 11.0 * a.cos(), 19.0 - 11.0 * a.sin())
                })
                .collect();
            i.line(&pts, ink, 2.0);
            i.dots(&[(6.0, 19.0), (28.0, 19.0), (17.0, 8.0)]);
        }
        "circle" => {
            i.circle(16.0, 16.0, 10.0, ink, 2.0);
            i.dots(&[(16.0, 16.0)]);
        }
        "rectangle" => i.rect(5.0, 9.0, 22.0, 14.0, None, Some((ink, 2.0))),
        "triangle" => i.closed(&[(16.0, 5.0), (28.0, 26.0), (4.0, 26.0)], ink, 2.0),
        "bezier" => {
            i.dash(&[(5.0, 26.0), (10.0, 5.0), (27.0, 8.0)], acc, 1.0, Dash::Dashed);
            i.quad((5.0, 26.0), (10.0, 5.0), (27.0, 8.0), ink, 2.0);
            i.dots(&[(5.0, 26.0), (10.0, 5.0), (27.0, 8.0)]);
        }
        "spline" => {
            i.cubic((4.0, 22.0), (9.0, 6.0), (14.0, 6.0), (17.0, 16.0), ink, 2.0);
            i.cubic((17.0, 16.0), (20.0, 26.0), (25.0, 26.0), (28.0, 10.0), ink, 2.0);
            i.dots(&[(4.0, 22.0), (17.0, 16.0), (28.0, 10.0)]);
        }
        "point" => {
            i.line(&[(16.0, 6.0), (16.0, 26.0)], ink, 2.4);
            i.line(&[(6.0, 16.0), (26.0, 16.0)], ink, 2.4);
        }
        "trim" => {
            i.line(&[(4.0, 22.0), (28.0, 22.0)], ink, 2.0);
            i.line(&[(10.0, 6.0), (10.0, 28.0)], ink, 2.0);
            i.line(&[(22.0, 6.0), (22.0, 28.0)], ink, 2.0);
            i.line(&[(11.5, 22.0), (20.5, 22.0)], red, 3.0);
        }
        "dimension" => {
            i.line(&[(5.0, 8.0), (5.0, 24.0)], ink, 2.0);
            i.line(&[(27.0, 8.0), (27.0, 24.0)], ink, 2.0);
            i.line(&[(6.0, 16.0), (26.0, 16.0)], acc, 1.6);
            i.poly(&[(6.0, 16.0), (11.0, 13.0), (11.0, 19.0)], acc);
            i.poly(&[(26.0, 16.0), (21.0, 19.0), (21.0, 13.0)], acc);
        }
        "move" => {
            i.line(&[(16.0, 4.0), (16.0, 28.0)], ink, 2.0);
            i.line(&[(4.0, 16.0), (28.0, 16.0)], ink, 2.0);
            for (tip, a, b) in [
                ((16.0, 4.0), (12.0, 9.0), (20.0, 9.0)),
                ((16.0, 28.0), (12.0, 23.0), (20.0, 23.0)),
                ((4.0, 16.0), (9.0, 12.0), (9.0, 20.0)),
                ((28.0, 16.0), (23.0, 12.0), (23.0, 20.0)),
            ] {
                i.line(&[a, tip, b], ink, 2.0);
            }
        }
        "block" => {
            i.rect(5.0, 5.0, 22.0, 22.0, Some(fills[0]), Some((ink, 1.6)));
            for k in [10.5, 16.0, 21.5] {
                i.line(&[(k, 5.0), (k, 27.0)], col::mesh(), 0.9);
                i.line(&[(5.0, k), (27.0, k)], col::mesh(), 0.9);
            }
        }
        "layer" => {
            i.line(&[(4.0, 26.0), (28.0, 26.0)], ink, 2.4);
            let mut x = 6.0;
            while x < 28.0 {
                i.line(&[(x, 26.0), (x - 3.0, 30.0)], ink, 1.2);
                x += 4.0;
            }
            let mut y = 26.0;
            for step in [1.6, 2.2, 3.0, 4.2, 5.8, 8.0] {
                y -= step;
                i.line(&[(4.0, y), (28.0, y)], acc, 1.4);
            }
        }
        "quality" => {
            for ((x, y), c) in [((5.0, 5.0), "#2e9d57"), ((16.0, 5.0), "#f1c40f"), ((5.0, 16.0), "#f1c40f"), ((16.0, 16.0), "#d43f3a")] {
                i.rect(x, y, 11.0, 11.0, Some(hex(c)), None);
            }
            i.rect(5.0, 5.0, 22.0, 22.0, None, Some((ink, 1.2)));
            i.line(&[(16.0, 5.0), (16.0, 27.0)], ink, 1.2);
            i.line(&[(5.0, 16.0), (27.0, 16.0)], ink, 1.2);
        }
        "refine" => {
            i.line(&[(5.0, 8.0), (12.0, 14.0), (16.0, 6.0), (27.0, 12.0)], red, 1.2);
            i.cubic((5.0, 22.0), (11.0, 16.0), (20.0, 28.0), (27.0, 21.0), acc, 2.0);
            i.line(&[(16.0, 13.0), (16.0, 19.0)], ink, 1.4);
            i.line(&[(13.0, 16.5), (16.0, 19.5), (19.0, 16.5)], ink, 1.4);
        }
        "merge" => {
            i.line(&[(4.0, 16.0), (13.0, 16.0)], ink, 2.0);
            i.line(&[(19.0, 16.0), (28.0, 16.0)], ink, 2.0);
            i.line(&[(11.0, 10.0), (16.0, 16.0), (11.0, 22.0)], acc, 2.0);
            i.line(&[(21.0, 10.0), (16.0, 16.0), (21.0, 22.0)], acc, 2.0);
        }
        "mirror" => {
            i.dash(&[(16.0, 3.0), (16.0, 29.0)], refc, 1.6, Dash::Dashed);
            i.line(&[(12.0, 8.0), (5.0, 24.0), (12.0, 24.0)], ink, 2.0);
            i.line(&[(20.0, 8.0), (27.0, 24.0), (20.0, 24.0)], acc, 2.0);
        }
        "offset" => {
            i.rect(11.0, 13.0, 10.0, 8.0, None, Some((ink, 2.0)));
            i.rect(4.0, 6.0, 24.0, 22.0, None, Some((acc, 2.0)));
            i.line(&[(16.0, 12.0), (16.0, 7.5)], acc, 1.4);
            i.line(&[(13.5, 10.0), (16.0, 7.5), (18.5, 10.0)], acc, 1.4);
        }
        "regions" => {
            i.rect(5.0, 7.0, 11.0, 18.0, Some(fills[1]), None);
            i.rect(16.0, 7.0, 11.0, 18.0, Some(fills[2]), None);
            i.rect(5.0, 7.0, 22.0, 18.0, None, Some((ink, 2.0)));
        }
        "grid" => {
            i.rect(5.0, 5.0, 22.0, 22.0, None, Some((ink, 1.0)));
            for k in [1.0, 2.0] {
                let x = 5.0 + 22.0 * k / 3.0;
                i.line(&[(x, 5.0), (x, 27.0)], ink, 1.0);
                i.line(&[(5.0, x), (27.0, x)], ink, 1.0);
            }
        }
        "split" => {
            i.rect(5.0, 7.0, 22.0, 18.0, None, Some((ink, 1.4)));
            i.dash(&[(16.0, 4.0), (16.0, 28.0)], refc, 2.0, Dash::DashDot);
        }
        "reference" => i.dash(&[(5.0, 27.0), (27.0, 5.0)], col::reference(), 2.0, Dash::Dashed),
        "import" => {
            i.line(&[(6.0, 18.0), (6.0, 27.0), (26.0, 27.0), (26.0, 18.0)], ink, 2.0);
            i.line(&[(16.0, 4.0), (16.0, 20.0)], ink, 2.0);
            i.line(&[(10.0, 14.0), (16.0, 20.0), (22.0, 14.0)], ink, 2.0);
        }
        "export" => {
            i.line(&[(6.0, 18.0), (6.0, 27.0), (26.0, 27.0), (26.0, 18.0)], ink, 2.0);
            i.line(&[(16.0, 21.0), (16.0, 5.0)], ink, 2.0);
            i.line(&[(10.0, 11.0), (16.0, 5.0), (22.0, 11.0)], ink, 2.0);
        }
        "fit" => {
            for (x, y, dx, dy) in [(5.0, 5.0, 1.0, 1.0), (27.0, 5.0, -1.0, 1.0), (5.0, 27.0, 1.0, -1.0), (27.0, 27.0, -1.0, -1.0)] {
                i.line(&[(x + 7.0 * dx, y), (x, y), (x, y + 7.0 * dy)], ink, 2.0);
            }
        }
        "snap" => {
            i.rect(10.0, 10.0, 12.0, 12.0, None, Some((ink, 2.0)));
            i.line(&[(4.0, 16.0), (28.0, 16.0)], acc, 1.5);
        }
        "mesh" => {
            for k in [6.0, 12.0, 18.0, 24.0] {
                i.line(&[(k, 5.0), (k + 2.0, 27.0)], col::mesh(), 1.2);
                i.line(&[(5.0, k), (27.0, k + 1.0)], col::mesh(), 1.2);
            }
        }
        "eilmer" => {
            i.line(&[(5.0, 24.0), (13.0, 8.0), (19.0, 20.0), (27.0, 6.0)], acc, 2.2);
            i.dots(&[(27.0, 6.0)]);
        }
        "sun" => {
            i.circle(16.0, 16.0, 5.0, ink, 2.0);
            for k in 0..8 {
                let a = std::f32::consts::FRAC_PI_4 * k as f32;
                let (c, s) = (a.cos(), a.sin());
                i.line(&[(16.0 + 8.5 * c, 16.0 + 8.5 * s), (16.0 + 12.0 * c, 16.0 + 12.0 * s)], ink, 2.0);
            }
        }
        "moon" => {
            // a crescent: the outer circle's arc away from the bite, then the bite's arc inside it
            let (c1, r1, c2, r2) = ((15.0f32, 17.0f32), 10.5f32, (21.5f32, 10.5f32), 8.5f32);
            let (dx, dy) = (c2.0 - c1.0, c2.1 - c1.1);
            let d = dx.hypot(dy);
            let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let hh = (r1 * r1 - a * a).max(0.0).sqrt();
            let (mx, my) = (c1.0 + a * dx / d, c1.1 + a * dy / d);
            let p = [(mx - hh * dy / d, my + hh * dx / d), (mx + hh * dy / d, my - hh * dx / d)];
            let ang = |c: (f32, f32), q: (f32, f32)| (q.1 - c.1).atan2(q.0 - c.0);
            let tau = std::f32::consts::TAU;
            // outer arc from p[0] to p[1] the long way round (away from the bite)
            let (o0, mut o1) = (ang(c1, p[0]), ang(c1, p[1]));
            let toward = ang(c1, c2);
            let between = |a0: f32, a1: f32, t: f32| ((t - a0).rem_euclid(tau)) < ((a1 - a0).rem_euclid(tau));
            if between(o0, o1, toward) {
                o1 -= tau;
                while o1 > o0 {
                    o1 -= tau;
                }
            } else {
                while o1 < o0 {
                    o1 += tau;
                }
            }
            let mut pts: Vec<(f32, f32)> = (0..=32).map(|k| {
                let t = o0 + (o1 - o0) * k as f32 / 32.0;
                (c1.0 + r1 * t.cos(), c1.1 + r1 * t.sin())
            }).collect();
            // inner arc from p[1] back to p[0], through the side facing the outer centre
            let (i0, mut i1) = (ang(c2, p[1]), ang(c2, p[0]));
            let facing = ang(c2, c1);
            if between(i0, i1, facing) {
                while i1 < i0 {
                    i1 += tau;
                }
            } else {
                while i1 > i0 {
                    i1 -= tau;
                }
            }
            pts.extend((1..=32).map(|k| {
                let t = i0 + (i1 - i0) * k as f32 / 32.0;
                (c2.0 + r2 * t.cos(), c2.1 + r2 * t.sin())
            }));
            i.line(&pts, ink, 2.0);
        }
        _ => {}
    }
    out
}

/// An icon-sized button-like image for menus and toolbars.
pub fn paint(ui: &egui::Ui, name: &str, rect: Rect) {
    ui.painter().extend(shapes(name, rect));
}

/// The application icon (RGBA, size x size): a meshed channel with a curved wall.
pub fn app_icon_rgba(size: usize) -> Vec<u8> {
    let s = size as f32 / 256.0;
    let p = |x: f32, y: f32| egui::pos2(x * s, y * s);
    let mut shapes = vec![];
    let rect = Rect::from_min_max(p(16.0, 16.0), p(240.0, 240.0));
    shapes.push(Shape::rect_filled(rect, 44.0 * s, hex("#1d4f73")));
    // a block mesh: lines bunching towards a curved wall at the bottom
    let wall = |x: f32| 196.0 - 34.0 * (std::f32::consts::PI * (x - 40.0) / 176.0).sin();
    let mesh = egui::Color32::from_rgb(140, 200, 235);
    for k in 0..=8 {
        let t = k as f32 / 8.0;
        let y_of = |x: f32| 58.0 + (wall(x) - 58.0) * (1.0 - (1.0 - t).powf(1.8));
        let pts: Vec<Pos2> = (0..=32).map(|i| {
            let x = 40.0 + 176.0 * i as f32 / 32.0;
            p(x, y_of(x))
        }).collect();
        shapes.push(Shape::line(pts, Stroke::new(3.0 * s, mesh)));
    }
    for k in 0..=8 {
        let x = 40.0 + 176.0 * k as f32 / 8.0;
        shapes.push(Shape::line_segment([p(x, 58.0), p(x, wall(x))], Stroke::new(3.0 * s, mesh)));
    }
    let wall_pts: Vec<Pos2> = (0..=48).map(|i| {
        let x = 40.0 + 176.0 * i as f32 / 48.0;
        p(x, wall(x))
    }).collect();
    shapes.push(Shape::line(wall_pts, Stroke::new(9.0 * s, hex("#f28c28"))));
    let ctx = egui::Context::default();
    let screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(size as f32, size as f32));
    let full = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| {
        ctx.layer_painter(egui::LayerId::background()).extend(shapes.clone());
    });
    let mut r = crate::softrender::SoftRenderer::default();
    r.update_textures(&full);
    let prims = ctx.tessellate(full.shapes, 1.0);
    let img = r.render(&prims, size, size, egui::Color32::TRANSPARENT);
    img.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect()
}
