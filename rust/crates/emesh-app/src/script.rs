//! Scripted input for demos and visual checks: `eilmer-mesher --script FILE`.
//!
//! One command per line (world coordinates in metres):
//!   wait N            let N frames pass
//!   tool NAME         switch tool
//!   move X Y          move the pointer
//!   click X Y         left click        rclick X Y   right click      dclick X Y   double click
//!   drag X0 Y0 X1 Y1  left drag
//!   key SEQ           a key press, e.g. Enter, Ctrl+A, Shift+X
//!   type TEXT         typed text
//!   open PATH         open a project       action ID   run a menu action (e.g. view.fit)
//!   shot PATH         screenshot           quit
//!   clickpx X Y       click at screen pixels (also rclickpx, dclickpx), e.g. inside a dialog

use std::collections::VecDeque;

use egui::{Event, Modifiers, PointerButton, Pos2};

use crate::settings::KeySeq;

#[derive(Clone, Debug)]
pub enum Cmd {
    Wait(u32),
    Tool(String),
    Move(f64, f64),
    Click(f64, f64, PointerButton, bool),
    /// a click at screen pixels (for dialogs): `clickpx X Y`, `dclickpx X Y`, `rclickpx X Y`
    ClickPx(f32, f32, PointerButton, bool),
    Drag(f64, f64, f64, f64),
    Key(KeySeq),
    Type(String),
    Open(String),
    Action(String),
    Shot(String),
    Quit,
    /// internal: release the left button
    Release(f64, f64),
    /// internal: a button press or release
    Press(f64, f64, PointerButton, bool),
}

pub fn parse(text: &str) -> Result<VecDeque<Cmd>, String> {
    let mut out = VecDeque::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
        let nums = || -> Result<Vec<f64>, String> {
            rest.split_whitespace().map(|v| v.parse::<f64>().map_err(|_| format!("line {}: bad number '{v}'", n + 1))).collect()
        };
        out.push_back(match cmd {
            "wait" => Cmd::Wait(rest.trim().parse().unwrap_or(1)),
            "tool" => Cmd::Tool(rest.trim().into()),
            "move" => {
                let v = nums()?;
                Cmd::Move(v[0], v[1])
            }
            "click" | "rclick" | "dclick" => {
                let v = nums()?;
                let b = if cmd == "rclick" { PointerButton::Secondary } else { PointerButton::Primary };
                Cmd::Click(v[0], v[1], b, cmd == "dclick")
            }
            "clickpx" | "rclickpx" | "dclickpx" => {
                let v = nums()?;
                let b = if cmd == "rclickpx" { PointerButton::Secondary } else { PointerButton::Primary };
                Cmd::ClickPx(v[0] as f32, v[1] as f32, b, cmd == "dclickpx")
            }
            "drag" => {
                let v = nums()?;
                Cmd::Drag(v[0], v[1], v[2], v[3])
            }
            "key" => Cmd::Key(KeySeq::parse(rest.trim()).ok_or(format!("line {}: bad key '{rest}'", n + 1))?),
            "type" => Cmd::Type(rest.to_string()),
            "open" => Cmd::Open(rest.trim().into()),
            "action" => Cmd::Action(rest.trim().into()),
            "shot" => Cmd::Shot(rest.trim().into()),
            "quit" => Cmd::Quit,
            other => return Err(format!("line {}: unknown command '{other}'", n + 1)),
        });
    }
    Ok(out)
}

pub fn mods(k: &KeySeq) -> Modifiers {
    Modifiers { alt: k.alt, ctrl: k.ctrl, shift: k.shift, mac_cmd: false, command: k.ctrl }
}

pub fn key_events(k: &KeySeq) -> Vec<Event> {
    let m = mods(k);
    vec![
        Event::Key { key: k.key, physical_key: None, pressed: true, repeat: false, modifiers: m },
        Event::Key { key: k.key, physical_key: None, pressed: false, repeat: false, modifiers: m },
    ]
}

pub fn press(pos: Pos2, b: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button: b, pressed, modifiers: Modifiers::NONE }
}
