//! Persistent user settings (display units, keyboard shortcuts, view and layer defaults).
//!
//! Stored as JSON in the platform's configuration folder (~/.config/eilmer-mesher/ on Linux).
//! Set EMESH_SETTINGS to a file path to use a different file (the tests do).

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};

use emesh_mesh::quality::{Thresholds, DEFAULT_THRESHOLDS};

/// id -> (menu text, default shortcut), in the Python version's portable key-sequence format
pub const DEFAULT_SHORTCUTS: &[(&str, &str, &str)] = &[
    ("tool.select", "Select", "S"),
    ("tool.line", "Line", "L"),
    ("tool.profile", "Profile", "P"),
    ("tool.arc", "Arc", "A"),
    ("tool.circle", "Circle", "C"),
    ("tool.rectangle", "Rectangle", "R"),
    ("tool.triangle", "Triangle", "T"),
    ("tool.bezier", "Bezier", "B"),
    ("tool.spline", "Spline", "N"),
    ("tool.point", "Point", "O"),
    ("tool.dimension", "Dimension", "D"),
    ("tool.move", "Move", "M"),
    ("tool.trim", "Quick trim", "Q"),
    ("tool.block", "Block", "K"),
    ("tool.layer", "Boundary layer", "Y"),
    ("block.all", "Block all regions", "Shift+K"),
    ("mesh.run", "Run Eilmer now", "F5"),
    ("tool.merge", "Merge", "J"),
    ("tool.mirror", "Mirror", "Shift+M"),
    ("tool.offset", "Offset", "F"),
    ("file.new", "New", "Ctrl+N"),
    ("file.open", "Open", "Ctrl+O"),
    ("file.save", "Save", "Ctrl+S"),
    ("file.save_as", "Save as", "Ctrl+Shift+S"),
    ("file.import", "Import points", "Ctrl+I"),
    ("file.export", "Export to Eilmer", "Ctrl+E"),
    ("block.bcs", "Boundary conditions", "Ctrl+B"),
    ("file.quit", "Quit", "Ctrl+Q"),
    ("edit.undo", "Undo", "Ctrl+Z"),
    ("edit.redo", "Redo", "Ctrl+Shift+Z"),
    ("edit.delete", "Delete", "Del"),
    ("edit.select_all", "Select all", "Ctrl+A"),
    ("edit.toggle_split", "Convert to/from block split line", "X"),
    ("sketch.split_mode", "Draw block split lines", "Ctrl+X"),
    ("edit.toggle_reference", "Convert to/from reference", "Shift+X"),
    ("sketch.reference_mode", "Draw reference lines", "Ctrl+Shift+X"),
    ("snap.grid", "Grid snap", "G"),
    ("view.fit", "Zoom to fit", "Space"),
    ("view.regions", "Show regions", "Shift+R"),
    ("view.grid", "Show grid", "Shift+G"),
    ("view.blocks", "Show blocks", "Shift+B"),
    ("view.mesh", "Show mesh", "Shift+N"),
    ("view.quality", "Show cell quality", "Shift+Q"),
    ("block.refine", "Refine flagged blocks", "Ctrl+R"),
];

/// Keys held down while drawing (single keys, not menu shortcuts).
pub const DEFAULT_HOLD_KEYS: &[(&str, &str, &str)] =
    &[("hold.horizontal", "Hold for horizontal", "H"), ("hold.vertical", "Hold for vertical", "V")];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerDefaults {
    pub first: f64,
    pub growth: f64,
    pub cells: usize,
    pub transition: bool,
}

impl Default for LayerDefaults {
    fn default() -> Self {
        LayerDefaults { first: 1.0e-5, growth: 1.2, cells: 10, transition: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowDefaults {
    pub yplus: f64,
    pub velocity: f64,
    pub density: f64,
    pub viscosity: f64,
    /// metres
    pub length: f64,
}

impl Default for FlowDefaults {
    fn default() -> Self {
        FlowDefaults { yplus: 1.0, velocity: 100.0, density: 1.225, viscosity: 1.789e-5, length: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualityLimits {
    pub skew_warn: f64,
    pub skew_bad: f64,
    pub ar_warn: f64,
}

impl Default for QualityLimits {
    fn default() -> Self {
        let t = DEFAULT_THRESHOLDS;
        QualityLimits { skew_warn: t.skew_warn, skew_bad: t.skew_bad, ar_warn: t.ar_warn }
    }
}

impl QualityLimits {
    pub fn thresholds(&self) -> Thresholds {
        Thresholds { skew_warn: self.skew_warn, skew_bad: self.skew_bad, ar_warn: self.ar_warn }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub units: String,
    /// shortcut overrides: id -> key sequence ("" = none)
    pub shortcuts: BTreeMap<String, String>,
    pub show_grid: bool,
    pub quality: QualityLimits,
    pub layer: LayerDefaults,
    pub layer_flow: FlowDefaults,
    pub last_export_dir: String,
    pub last_open_dir: String,
    pub recent: Vec<String>,
    /// "auto", "wgpu" or "glow"
    pub renderer: String,
    /// the user has been told lmr was not found
    pub lmr_warned: bool,
    /// "light", "dark" or "system" (follow the OS)
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            units: "m".into(),
            shortcuts: BTreeMap::new(),
            show_grid: true,
            quality: QualityLimits::default(),
            layer: LayerDefaults::default(),
            layer_flow: FlowDefaults::default(),
            last_export_dir: String::new(),
            last_open_dir: String::new(),
            recent: vec![],
            renderer: "auto".into(),
            lmr_warned: false,
            theme: "light".into(),
        }
    }
}

pub fn settings_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("EMESH_SETTINGS") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    directories::ProjectDirs::from("", "", "eilmer-mesher").map(|d| d.config_dir().join("settings.json"))
}

impl Settings {
    pub fn load() -> Settings {
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = settings_path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, serde_json::to_string_pretty(self).unwrap_or_default());
        }
    }

    pub fn shortcut(&self, id: &str) -> String {
        if let Some(s) = self.shortcuts.get(id) {
            return s.clone();
        }
        DEFAULT_SHORTCUTS.iter().chain(DEFAULT_HOLD_KEYS).find(|(i, _, _)| *i == id).map(|t| t.2.to_string()).unwrap_or_default()
    }

    pub fn set_shortcuts(&mut self, values: &BTreeMap<String, String>) {
        for (id, seq) in values {
            let default = DEFAULT_SHORTCUTS.iter().chain(DEFAULT_HOLD_KEYS).find(|(i, _, _)| i == id).map(|t| t.2).unwrap_or("");
            if seq == default {
                self.shortcuts.remove(id);
            } else {
                self.shortcuts.insert(id.clone(), seq.clone());
            }
        }
    }

    pub fn remember_recent(&mut self, path: &str) {
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_string());
        self.recent.truncate(8);
    }
}

// --- key sequences ------------------------------------------------------------------------------

/// A key with exact modifiers ("Ctrl" means Cmd on macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySeq {
    pub key: Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeySeq {
    pub fn parse(text: &str) -> Option<KeySeq> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let (mut ctrl, mut shift, mut alt) = (false, false, false);
        let parts: Vec<&str> = if text == "+" { vec!["+"] } else { text.split('+').collect() };
        let (last, mods) = parts.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "cmd" | "meta" | "command" => ctrl = true,
                "shift" => shift = true,
                "alt" | "option" => alt = true,
                _ => return None,
            }
        }
        let key = key_from_name(last)?;
        Some(KeySeq { key, ctrl, shift, alt })
    }

    pub fn matches(&self, key: Key, m: &Modifiers) -> bool {
        self.key == key && self.ctrl == m.command && self.shift == m.shift && self.alt == m.alt
    }

    /// Portable text, e.g. "Ctrl+Shift+X".
    pub fn text(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s += "Ctrl+";
        }
        if self.alt {
            s += "Alt+";
        }
        if self.shift {
            s += "Shift+";
        }
        s + key_name(self.key)
    }

    /// For menus: Cmd on macOS.
    pub fn native(&self) -> String {
        if cfg!(target_os = "macos") {
            self.text().replace("Ctrl+", "Cmd+")
        } else {
            self.text()
        }
    }
}

pub fn key_from_name(name: &str) -> Option<Key> {
    let n = match name {
        "Del" => "Delete",
        "Esc" => "Escape",
        "Return" => "Enter",
        "Ins" => "Insert",
        "PgUp" => "PageUp",
        "PgDown" => "PageDown",
        "+" => "Plus",
        "-" => "Minus",
        "=" => "Equals",
        other => other,
    };
    Key::from_name(n).or_else(|| Key::from_name(&n.to_ascii_uppercase()))
}

pub fn key_name(key: Key) -> &'static str {
    match key {
        Key::Delete => "Del",
        Key::Escape => "Esc",
        Key::Plus => "+",
        Key::Minus => "-",
        Key::Equals => "=",
        k => k.name(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sequences() {
        let k = KeySeq::parse("Ctrl+Shift+X").unwrap();
        assert!(k.ctrl && k.shift && !k.alt && k.key == Key::X);
        assert_eq!(k.text(), "Ctrl+Shift+X");
        assert_eq!(KeySeq::parse("Del").unwrap().key, Key::Delete);
        assert_eq!(KeySeq::parse("F5").unwrap().key, Key::F5);
        assert_eq!(KeySeq::parse("Space").unwrap().key, Key::Space);
        for (id, _, seq) in DEFAULT_SHORTCUTS.iter().chain(DEFAULT_HOLD_KEYS) {
            assert!(KeySeq::parse(seq).is_some(), "{id}: {seq}");
        }
    }
}
