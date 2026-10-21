//! Colours, strokes and dash patterns for the canvas, and the light and dark themes.
//!
//! The canvas colours (`col::*`) read the current [`Palette`], switched with the egui theme by
//! [`apply`] (settings) and [`sync`] (each frame, so "follow the system" tracks the OS).

use egui::{Color32, Pos2, Shape, Stroke};

use emesh_core::entities::Kind;

pub fn hex(s: &str) -> Color32 {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// An sRGBA colour premultiplied in gamma space.
pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    alpha(Color32::from_rgb(r, g, b), a)
}

/// A translucent colour, premultiplied in gamma space (as Qt and the Python version blend).
pub fn alpha(c: Color32, a: u8) -> Color32 {
    let m = |v: u8| ((v as u32 * a as u32 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(m(c.r()), m(c.g()), m(c.b()), a)
}

/// `0xRRGGBB` as an opaque colour (const).
pub const fn h(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// `0xRRGGBB` at alpha `a`, premultiplied in gamma space (const [`alpha`]).
pub const fn ha(v: u32, a: u8) -> Color32 {
    let a32 = a as u32;
    let (r, g, b) = ((v >> 16) & 255, (v >> 8) & 255, v & 255);
    Color32::from_rgba_premultiplied(((r * a32 + 127) / 255) as u8, ((g * a32 + 127) / 255) as u8, ((b * a32 + 127) / 255) as u8, a)
}

/// The canvas colours of one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub background: Color32,
    pub grid_minor: Color32,
    pub grid_major: Color32,
    pub axes: Color32,
    pub geometry: Color32,
    pub reference: Color32,
    pub split: Color32,
    pub selected: Color32,
    pub hover: Color32,
    pub preview: Color32,
    pub snap: Color32,
    pub handle: Color32,
    pub trim: Color32,
    pub dimension: Color32,
    pub block: Color32,
    pub block_error: Color32,
    pub mesh: Color32,
    pub layer: Color32,
    /// icon strokes
    pub ink: Color32,
    pub accent: Color32,
    pub error_text: Color32,
    /// small notes in dialogs
    pub weak_text: Color32,
    /// outlines of the canvas legends
    pub frame: Color32,
    /// inverted cells in the quality heat-map
    pub inverted: Color32,
    /// cells without a metric (boundary-layer aspect ratio)
    pub no_metric: Color32,
    /// box selection: window (left to right) and crossing
    pub select_window: Color32,
    pub select_crossing: Color32,
    /// toolbar buttons: (fill, outline) when on, and when hovered
    pub button_on: (Color32, Color32),
    pub button_hover: (Color32, Color32),
    /// quality banner (fill, outline): warning, bad, error
    pub banner: [(Color32, Color32); 3],
    /// the shadow under the toolbar (darkest row)
    pub shadow: Color32,
    /// translucency of the region / block fills and the heat-map
    pub region_alpha: u8,
    pub block_alpha: u8,
    pub heat_alpha: u8,
    /// how far the boundary-condition colours are lifted toward white (dark canvas)
    pub tag_lift: f32,
    /// icon fills: block, two regions
    pub icon_fills: [Color32; 3],
}

pub const LIGHT: Palette = Palette {
    dark: false,
    background: h(0xffffff),
    grid_minor: h(0xeef1f5),
    grid_major: h(0xdbe1e8),
    axes: h(0x98a4b2),
    geometry: h(0x101820),
    reference: h(0x7d8895),
    split: h(0xa35a1a),
    selected: h(0xf07800),
    hover: h(0x0a90d8),
    preview: h(0x0a90d8),
    snap: h(0xe0382d),
    handle: h(0xffffff),
    trim: h(0xe0382d),
    dimension: h(0x1c5fb5),
    block: h(0x0a90d8),
    block_error: h(0xd8352d),
    mesh: h(0x32526e),
    layer: h(0x4c8a0e),
    ink: h(0x1b2531),
    accent: h(0x0f6cbd),
    error_text: h(0xc42b1c),
    weak_text: h(0x5a6472),
    frame: h(0xd0d6dd),
    inverted: ha(0x8c1ea0, 200),
    no_metric: ha(0xa0a0a0, 70),
    select_window: h(0x0f6cbd),
    select_crossing: h(0x2e9d57),
    button_on: (h(0xd9e8f8), h(0x0f6cbd)),
    button_hover: (h(0xe8edf3), h(0xc4d0dd)),
    banner: [(h(0xfff6d5), h(0xd4a514)), (h(0xffecd6), h(0xe67e22)), (h(0xfde3e1), h(0xd8352d))],
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 34),
    region_alpha: 40,
    block_alpha: 28,
    heat_alpha: 150,
    tag_lift: 0.0,
    icon_fills: [h(0xc4e3f6), h(0xa9d8f2), h(0xa6e0bd)],
};

pub const DARK: Palette = Palette {
    dark: true,
    background: h(0x1b1e23),
    grid_minor: h(0x23272d),
    grid_major: h(0x30353d),
    axes: h(0x5b6675),
    geometry: h(0xe6e9ee),
    reference: h(0x8b95a2),
    split: h(0xe0a262),
    selected: h(0xff9b30),
    hover: h(0x41bbf5),
    preview: h(0x41bbf5),
    snap: h(0xff5d52),
    handle: h(0x1b1e23),
    trim: h(0xff5d52),
    dimension: h(0x74b2ff),
    block: h(0x41bbf5),
    block_error: h(0xff5d52),
    mesh: h(0xa3bbd1),
    layer: h(0xa8d65c),
    ink: h(0xdde2e8),
    accent: h(0x4ca2f2),
    error_text: h(0xff7266),
    weak_text: h(0x9ba5b1),
    frame: h(0x3d444d),
    inverted: ha(0xc354e0, 210),
    no_metric: ha(0x9a9a9a, 60),
    select_window: h(0x4ca2f2),
    select_crossing: h(0x4fc57d),
    button_on: (h(0x24456a), h(0x4ca2f2)),
    button_hover: (h(0x363d46), h(0x4c5765)),
    banner: [(h(0x3a3216), h(0xc9a227)), (h(0x3d2918), h(0xe0883a)), (h(0x401f1f), h(0xe0554f))],
    shadow: Color32::from_rgba_premultiplied(0, 0, 0, 90),
    region_alpha: 50,
    block_alpha: 34,
    heat_alpha: 180,
    tag_lift: 0.28,
    icon_fills: [h(0x1f4a68), h(0x235a7a), h(0x22603c)],
};

thread_local! {
    // per thread: the GUI paints on one thread, and each headless test runs on its own
    static IS_DARK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The current canvas palette.
pub fn palette() -> &'static Palette {
    if is_dark() {
        &DARK
    } else {
        &LIGHT
    }
}

pub fn is_dark() -> bool {
    IS_DARK.with(|d| d.get())
}

pub fn set_dark(dark: bool) {
    IS_DARK.with(|d| d.set(dark));
}

/// A boundary-condition tag colour as drawn on the canvas (lifted toward white on dark).
pub fn tag(c: Color32) -> Color32 {
    let t = palette().tag_lift;
    if t <= 0.0 {
        return c;
    }
    let m = |v: u8| (v as f32 + (255.0 - v as f32) * t).round() as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// The canvas colours, from the current palette (light or dark).
pub mod col {
    use super::palette;
    use egui::Color32;
    pub fn background() -> Color32 { palette().background }
    pub fn grid_minor() -> Color32 { palette().grid_minor }
    pub fn grid_major() -> Color32 { palette().grid_major }
    pub fn axes() -> Color32 { palette().axes }
    pub fn geometry() -> Color32 { palette().geometry }
    /// drawing aids (dashed): ignored by the mesher
    pub fn reference() -> Color32 { palette().reference }
    /// block split lines (dash-dot)
    pub fn split() -> Color32 { palette().split }
    pub fn selected() -> Color32 { palette().selected }
    pub fn hover() -> Color32 { palette().hover }
    pub fn preview() -> Color32 { palette().preview }
    pub fn snap() -> Color32 { palette().snap }
    pub fn handle() -> Color32 { palette().handle }
    pub fn trim() -> Color32 { palette().trim }
    pub fn dimension() -> Color32 { palette().dimension }
    pub fn block() -> Color32 { palette().block }
    pub fn block_error() -> Color32 { palette().block_error }
    pub fn mesh() -> Color32 { palette().mesh }
    pub fn layer() -> Color32 { palette().layer }
    pub fn ink() -> Color32 { palette().ink }
    pub fn accent() -> Color32 { palette().accent }
    pub fn error_text() -> Color32 { palette().error_text }
    pub fn weak_text() -> Color32 { palette().weak_text }
    pub fn frame() -> Color32 { palette().frame }
}

pub const REGION_FILLS: [&str; 6] = ["#1a9fd9", "#2e9d57", "#f28c28", "#8e5bd0", "#d43f3a", "#c9a227"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dash {
    Solid,
    Dashed,
    DashDot,
}

impl Dash {
    pub fn pattern(self) -> &'static [f32] {
        match self {
            Dash::Solid => &[],
            Dash::Dashed => &[6.0, 4.0],
            Dash::DashDot => &[9.0, 3.0, 1.5, 3.0],
        }
    }
}

/// Dash style for a curve kind: split lines dash-dot, reference (drawing aid) lines dashed.
pub fn kind_dash(kind: Kind) -> Dash {
    match kind {
        Kind::Reference => Dash::Dashed,
        Kind::Split => Dash::DashDot,
        Kind::Geometry => Dash::Solid,
    }
}

pub fn kind_color(kind: Kind) -> Color32 {
    match kind {
        Kind::Reference => col::reference(),
        Kind::Split => col::split(),
        Kind::Geometry => col::geometry(),
    }
}

/// A polyline in screen space, solid or dashed (pattern lengths scale with the stroke width).
pub fn polyline(points: &[Pos2], stroke: Stroke, dash: Dash, out: &mut Vec<Shape>) {
    if points.len() < 2 {
        return;
    }
    let pat = dash.pattern();
    if pat.is_empty() {
        out.push(Shape::line(points.to_vec(), stroke));
        return;
    }
    let scale = (stroke.width / 1.5).max(1.0);
    let pat: Vec<f32> = pat.iter().map(|v| v * scale).collect();
    let (mut k, mut left, mut on) = (0usize, pat[0], true);
    let mut current: Vec<Pos2> = vec![points[0]];
    for w in points.windows(2) {
        let (mut a, b) = (w[0], w[1]);
        let mut seg = (b - a).length();
        while seg > 0.0 {
            if seg < left {
                left -= seg;
                if on {
                    current.push(b);
                }
                seg = 0.0;
            } else {
                let p = a + (b - a) * (left / seg);
                if on {
                    current.push(p);
                    if current.len() >= 2 {
                        out.push(Shape::line(std::mem::take(&mut current), stroke));
                    }
                } else {
                    current = vec![p];
                }
                seg -= left;
                a = p;
                on = !on;
                k = (k + 1) % pat.len();
                left = pat[k];
            }
        }
    }
    if on && current.len() >= 2 {
        out.push(Shape::line(current, stroke));
    }
}

/// Green -> yellow -> red for 0..1 (cell-quality heat-map), translucent.
pub fn ramp(v: f64) -> Color32 {
    let stops = [(0.0, (46.0, 157.0, 87.0)), (0.5, (241.0, 196.0, 15.0)), (1.0, (212.0, 63.0, 58.0))];
    for w in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if v <= t1 {
            let f = (v - t0) / (t1 - t0);
            let m = |a: f64, b: f64| (a + f * (b - a)).round() as u8;
            return rgba(m(c0.0, c1.0), m(c0.1, c1.1), m(c0.2, c1.2), palette().heat_alpha);
        }
    }
    rgba(212, 63, 58, palette().heat_alpha)
}

// --- egui themes ---------------------------------------------------------------------------------

fn widget(v: &mut egui::style::WidgetVisuals, bg: Color32, stroke: Option<Color32>, fg: Color32) {
    v.bg_fill = bg;
    v.weak_bg_fill = bg;
    v.bg_stroke = stroke.map_or(Stroke::NONE, |c| Stroke::new(1.0f32, c));
    v.fg_stroke = Stroke::new(1.0f32, fg);
    v.corner_radius = egui::CornerRadius::same(4);
}

fn shadow(offset: i8, blur: u8, alpha: u8) -> egui::epaint::Shadow {
    egui::epaint::Shadow { offset: [0, offset], blur, spread: 0, color: Color32::from_black_alpha(alpha) }
}

/// Light panels: near-white chrome, crisp hairlines, dark text, a deep blue accent.
pub fn light_visuals() -> egui::Visuals {
    let p = &LIGHT;
    let mut v = egui::Visuals::light();
    let text = h(0x1a2129);
    v.panel_fill = h(0xf6f7f9);
    v.window_fill = h(0xffffff);
    v.extreme_bg_color = h(0xffffff);
    v.faint_bg_color = h(0xf3f5f8);
    v.code_bg_color = h(0xeef1f4);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = h(0xa15c00);
    v.error_fg_color = p.error_text;
    v.selection.bg_fill = h(0xd3e5f8);
    v.selection.stroke = Stroke::new(1.0f32, h(0x0b4f8f));
    let w = &mut v.widgets;
    widget(&mut w.noninteractive, h(0xf6f7f9), Some(h(0xd6dbe1)), text);
    widget(&mut w.inactive, h(0xe9ecf0), Some(h(0xcdd4dc)), text);
    widget(&mut w.hovered, h(0xe1eaf5), Some(h(0x98bde4)), h(0x05090e));
    widget(&mut w.active, h(0xcfe1f5), Some(p.accent), h(0x05090e));
    widget(&mut w.open, h(0xe1e7ee), None, h(0x05090e));
    w.hovered.expansion = 0.0;
    w.active.expansion = 0.0;
    v.window_stroke = Stroke::new(1.0f32, h(0xc9d0d8));
    v.window_shadow = shadow(6, 20, 40);
    v.popup_shadow = shadow(3, 10, 36);
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.menu_corner_radius = egui::CornerRadius::same(6);
    v
}

/// Dark panels: graphite chrome a step lighter than the canvas, soft white text.
pub fn dark_visuals() -> egui::Visuals {
    let p = &DARK;
    let mut v = egui::Visuals::dark();
    let text = h(0xdfe3e8);
    v.panel_fill = h(0x262a30);
    v.window_fill = h(0x2c3137);
    v.extreme_bg_color = h(0x1b1e22);
    v.faint_bg_color = h(0x30353c);
    v.code_bg_color = h(0x1f2226);
    v.hyperlink_color = p.accent;
    v.warn_fg_color = h(0xe8b44a);
    v.error_fg_color = p.error_text;
    v.selection.bg_fill = h(0x26527f);
    v.selection.stroke = Stroke::new(1.0f32, h(0xeef5ff));
    let w = &mut v.widgets;
    widget(&mut w.noninteractive, h(0x262a30), Some(h(0x3a4048)), text);
    widget(&mut w.inactive, h(0x363b43), Some(h(0x464d57)), text);
    widget(&mut w.hovered, h(0x3d4652), Some(h(0x5f81a8)), h(0xffffff));
    widget(&mut w.active, h(0x2d4d70), Some(p.accent), h(0xffffff));
    widget(&mut w.open, h(0x3a414a), None, h(0xffffff));
    w.hovered.expansion = 0.0;
    w.active.expansion = 0.0;
    v.window_stroke = Stroke::new(1.0f32, h(0x434a54));
    v.window_shadow = shadow(6, 22, 110);
    v.popup_shadow = shadow(3, 12, 90);
    v.window_corner_radius = egui::CornerRadius::same(8);
    v.menu_corner_radius = egui::CornerRadius::same(6);
    v
}

fn styled(base: &egui::Style, visuals: egui::Visuals) -> egui::Style {
    let mut s = base.clone();
    s.visuals = visuals;
    s.spacing.item_spacing = egui::vec2(8.0, 5.0);
    s.spacing.button_padding = egui::vec2(6.0, 3.0);
    s.spacing.menu_margin = egui::Margin::same(5);
    s
}

/// Install both egui styles and choose the theme: "light", "dark" or "system" (follow the OS,
/// light when it does not say).
pub fn apply(ctx: &egui::Context, preference: &str) {
    use egui::Theme;
    ctx.set_style_of(Theme::Light, styled(&ctx.style_of(Theme::Light), light_visuals()));
    ctx.set_style_of(Theme::Dark, styled(&ctx.style_of(Theme::Dark), dark_visuals()));
    ctx.options_mut(|o| o.fallback_theme = Theme::Light);
    ctx.set_theme(match preference {
        "dark" => egui::ThemePreference::Dark,
        "system" => egui::ThemePreference::System,
        _ => egui::ThemePreference::Light,
    });
    sync(ctx);
}

/// Make the canvas palette follow egui's current theme (call once a frame).
pub fn sync(ctx: &egui::Context) {
    set_dark(ctx.theme() == egui::Theme::Dark);
}
