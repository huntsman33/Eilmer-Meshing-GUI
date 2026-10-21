//! The whole window run without a display: scripted pointer and key events in, model state and
//! (software-rendered) screenshots out. Used by the GUI tests and `examples/shots.rs`.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

use emesh_core::P;

use crate::app::App;
use crate::script::{key_events, mods, Cmd};
use crate::softrender::SoftRenderer;

pub struct Headless {
    pub ctx: egui::Context,
    pub app: App,
    pub size: Vec2,
    renderer: SoftRenderer,
    last: Option<egui::FullOutput>,
}

impl Headless {
    pub fn new(size: Vec2) -> Headless {
        let ctx = egui::Context::default();
        let app = App::new(&ctx, None);
        let mut h = Headless { ctx, app, size, renderer: SoftRenderer::default(), last: None };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    pub fn frame_mods(&mut self, events: Vec<Event>, modifiers: Modifiers) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            events,
            modifiers,
            focused: true,
            ..Default::default()
        };
        let app = &mut self.app;
        let out = self.ctx.run(raw, |ctx| app.frame(ctx));
        self.renderer.update_textures(&out);
        self.last = Some(out);
    }

    pub fn frame(&mut self, events: Vec<Event>) {
        self.frame_mods(events, Modifiers::NONE);
    }

    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(vec![]);
        }
    }

    /// Frames until the live mesher has delivered a mesh (or a timeout).
    pub fn wait_for_mesh(&mut self) -> bool {
        for _ in 0..400 {
            self.frame(vec![]);
            if self.app.canvas.mesh.as_ref().is_some_and(|m| m.revision == self.app.model.revision) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        false
    }

    pub fn s(&self, p: P) -> Pos2 {
        self.app.canvas.view.to_screen(p)
    }

    fn button(&mut self, pos: Pos2, b: PointerButton, pressed: bool, m: Modifiers) {
        self.frame_mods(vec![Event::PointerButton { pos, button: b, pressed, modifiers: m }], m);
    }

    pub fn press_at(&mut self, pos: Pos2, b: PointerButton, m: Modifiers) {
        self.frame_mods(vec![Event::PointerMoved(pos)], m);
        self.button(pos, b, true, m);
        self.button(pos, b, false, m);
        self.app.forget_click();
    }

    pub fn click(&mut self, p: P) {
        let q = self.s(p);
        self.press_at(q, PointerButton::Primary, Modifiers::NONE);
    }

    pub fn ctrl_click(&mut self, p: P) {
        let q = self.s(p);
        self.press_at(q, PointerButton::Primary, Modifiers::COMMAND);
    }

    pub fn right_click(&mut self, p: P) {
        let q = self.s(p);
        self.press_at(q, PointerButton::Secondary, Modifiers::NONE);
    }

    pub fn double_click(&mut self, p: P) {
        let q = self.s(p);
        self.frame(vec![Event::PointerMoved(q)]);
        for _ in 0..2 {
            self.button(q, PointerButton::Primary, true, Modifiers::NONE);
            self.button(q, PointerButton::Primary, false, Modifiers::NONE);
        }
        self.app.forget_click();
    }

    pub fn move_to(&mut self, p: P) {
        let q = self.s(p);
        self.frame(vec![Event::PointerMoved(q)]);
    }

    pub fn drag(&mut self, a: P, b: P) {
        let (p, q) = (self.s(a), self.s(b));
        self.frame(vec![Event::PointerMoved(p)]);
        self.button(p, PointerButton::Primary, true, Modifiers::NONE);
        for k in 1..=6 {
            self.frame(vec![Event::PointerMoved(p + (q - p) * (k as f32 / 6.0))]);
        }
        self.button(q, PointerButton::Primary, false, Modifiers::NONE);
        self.app.forget_click();
    }

    pub fn key(&mut self, key: Key, m: Modifiers) {
        self.frame_mods(vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: m }], m);
        self.frame_mods(vec![Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: m }], m);
    }

    pub fn type_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.frame(vec![Event::Text(ch.to_string())]);
        }
    }

    pub fn tool(&mut self, name: &str) {
        let ctx = self.ctx.clone();
        self.app.set_tool(&ctx, name);
        self.frame(vec![]);
    }

    pub fn action(&mut self, id: &str) {
        let ctx = self.ctx.clone();
        self.app.run_action_pub(&ctx, id);
        self.frame(vec![]);
    }

    /// Software-rendered screenshot of the last frame.
    pub fn shot(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        self.frame(vec![]);
        let out = self.last.take().unwrap();
        let prims = self.ctx.tessellate(out.shapes.clone(), out.pixels_per_point);
        let (w, h) = (self.size.x as usize, self.size.y as usize);
        let img = self.renderer.render(&prims, w, h, self.ctx.style().visuals.panel_fill);
        self.last = Some(out);
        crate::png::write_rgba(path, w, h, &img)
    }

    /// Run script commands (see script.rs).
    pub fn run(&mut self, cmds: impl IntoIterator<Item = Cmd>) -> std::io::Result<()> {
        for c in cmds {
            match c {
                Cmd::Wait(n) => {
                    for _ in 0..n {
                        self.frame(vec![]);
                        std::thread::sleep(std::time::Duration::from_millis(15)); // let the mesher work
                    }
                }
                Cmd::Tool(t) => self.tool(&t),
                Cmd::Move(x, y) => self.move_to([x, y]),
                Cmd::Click(x, y, b, double) => {
                    if double {
                        self.double_click([x, y]);
                    } else {
                        let q = self.s([x, y]);
                        self.press_at(q, b, Modifiers::NONE);
                    }
                }
                Cmd::ClickPx(x, y, b, double) => {
                    let q = Pos2::new(x, y);
                    self.frame(vec![Event::PointerMoved(q)]);
                    for _ in 0..if double { 2 } else { 1 } {
                        self.button(q, b, true, Modifiers::NONE);
                        self.button(q, b, false, Modifiers::NONE);
                    }
                    self.app.forget_click();
                }
                Cmd::Drag(a, b, c, d) => self.drag([a, b], [c, d]),
                Cmd::Key(k) => {
                    let m = mods(&k);
                    let evs = key_events(&k);
                    self.frame_mods(vec![evs[0].clone()], m);
                    self.frame_mods(vec![evs[1].clone()], m);
                }
                Cmd::Type(t) => self.type_text(&t),
                Cmd::Open(p) => {
                    self.app.open_path(std::path::Path::new(&p));
                    self.settle();
                }
                Cmd::Action(a) => self.action(&a),
                Cmd::Shot(p) => self.shot(std::path::Path::new(&p))?,
                Cmd::Quit | Cmd::Release(..) | Cmd::Press(..) => {}
            }
        }
        Ok(())
    }
}
