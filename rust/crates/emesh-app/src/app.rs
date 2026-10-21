//! The main window: sketch canvas, tool palette, typed input, menus, dialogs, and the live mesh.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Color32, Key, Modifiers, PointerButton, Pos2, RichText, Sense, Stroke, Vec2};

use emesh_core::document::{Document, Face};
use emesh_core::geom;
use emesh_core::layers::LayerSettings;
use emesh_core::snapping::{Snap, SnapKind, SnapOption, SNAP_OPTIONS};
use emesh_core::units::{Units, UNITS};
use emesh_mesh::live::{Event as MeshEvent, LiveMesher, Mesh, Source};
use emesh_mesh::quality::{assess, intentional_aspect, Level, MeshQuality};
use emesh_mesh::runner::{export_grids, exported_connections, find_lmr, LmrResult, RunError};
use serde_json::json;

use crate::canvas::{Canvas, Draw, Metric, PaintInput};
use crate::dialogs::{self, AoPatch, BcDialog, Choice, EdgeCells, EditDims, EditKind, Field, FileBrowser, FileMode, ImportDlg, LayerDlg, Message, ShortcutsDlg, Split, ThresholdsDlg};
use crate::icons;
use crate::model::{SketchModel, Smoothing, Subdivision};
use crate::settings::{KeySeq, Settings, DEFAULT_HOLD_KEYS, DEFAULT_SHORTCUTS};
use crate::theme::{self, alpha, col};
use crate::tools::{all_tools, AppEvent, Ctx, Tool, ToolEvent};

pub const PROJECT_SUFFIX: &str = ".emesh.json";
const BIG: f64 = 1.0e12;

enum Dialog {
    EdgeCells(EdgeCells),
    Split(Split),
    AoPatch(AoPatch),
    Bc(BcDialog),
    BcManager { selected: Option<String>, sub: Option<BcDialog> },
    EditDims(EditDims),
    Layer(LayerDlg),
    Import(ImportDlg),
    Shortcuts(ShortcutsDlg),
    Thresholds(ThresholdsDlg),
    Message(Message),
    Confirm(Pending),
    File(FileBrowser, FilePurpose),
    Exporting(PathBuf, Receiver<Result<LmrResult, RunError>>),
    About,
}

#[derive(Clone)]
enum Pending {
    New,
    Open,
    OpenPath(PathBuf),
    Quit,
}

#[derive(Clone, Copy, PartialEq)]
enum FilePurpose {
    Open,
    SaveAs,
    Import,
    Export,
}

struct Banner {
    level: Level,
    text: String,
    dismissed: Option<String>,
}

pub struct App {
    pub model: SketchModel,
    pub canvas: Canvas,
    tools: Vec<(Box<dyn Tool>, &'static str)>,
    tool: usize,
    pub settings: Settings,
    mesher: LiveMesher,
    pub path: Option<PathBuf>,
    events: Vec<AppEvent>,
    input_text: String,
    input_error: String,
    focus_input: bool,
    focus_canvas: bool,
    status: Option<(String, Instant)>,
    mesh_status: String,
    dialog: Option<Dialog>,
    menu_at: Option<Pos2>,
    menu_opened: bool,
    quality: Option<MeshQuality>,
    refine_targets: Vec<String>,
    ao_targets: Vec<String>,
    banner: Option<Banner>,
    banner_dismissed: Option<String>,
    sent_preview: Option<u64>,
    held: HashSet<Key>,
    pan: Option<(Pos2, f64, f64)>,
    primary_down: bool,
    last_click: Option<(Instant, Pos2)>,
    pending_zoom_fit: bool,
    quit_confirmed: bool,
    pub screenshot: Option<(PathBuf, u32)>,
    /// scripted input (demos, visual checks)
    pub script: std::collections::VecDeque<crate::script::Cmd>,
    script_events: Vec<egui::Event>,
    script_wait: u32,
    script_shot: Option<PathBuf>,
    shortcuts: Vec<(KeySeq, String)>,
    hold_keys: (Key, Key),
    tool_painter_layer: egui::LayerId,
}

fn concrete(model: &SketchModel) -> Document {
    let res = model.resolution();
    model.doc.with_blocks(res.blocks.clone())
}

impl App {
    pub fn new(egui_ctx: &egui::Context, path: Option<PathBuf>) -> App {
        let settings = Settings::load();
        theme::apply(egui_ctx, &settings.theme);
        let ctx = egui_ctx.clone();
        let mesher = LiveMesher::new(Arc::new(move || ctx.request_repaint()));
        let mut canvas = Canvas::default();
        canvas.units = Units::new(&settings.units);
        canvas.show_grid = settings.show_grid;
        let mut app = App {
            model: SketchModel::default(),
            canvas,
            tools: all_tools(),
            tool: 0,
            settings,
            mesher,
            path: None,
            events: vec![],
            input_text: String::new(),
            input_error: String::new(),
            focus_input: false,
            focus_canvas: true,
            status: None,
            mesh_status: "No blocks".into(),
            dialog: None,
            menu_at: None,
            menu_opened: false,
            quality: None,
            refine_targets: vec![],
            ao_targets: vec![],
            banner: None,
            banner_dismissed: None,
            sent_preview: None,
            held: HashSet::new(),
            pan: None,
            primary_down: false,
            last_click: None,
            pending_zoom_fit: false,
            quit_confirmed: false,
            screenshot: None,
            script: Default::default(),
            script_events: vec![],
            script_wait: 0,
            script_shot: None,
            shortcuts: vec![],
            hold_keys: (Key::H, Key::V),
            tool_painter_layer: egui::LayerId::new(egui::Order::Middle, egui::Id::new("canvas")),
        };
        app.apply_shortcuts();
        if let Err(e) = find_lmr() {
            app.mesh_status = "Eilmer (lmr) not found: preview only".into();
            if !app.settings.lmr_warned {
                app.settings.lmr_warned = true;
                app.settings.save();
                app.dialog = Some(Dialog::Message(Message {
                    title: "Eilmer not found".into(),
                    text: format!(
                        "{e}.\n\nEilmer Mesher writes Eilmer scripts and runs Eilmer's lmr program to build the \
                         meshes. Until lmr is found, the meshes shown are the built-in preview (identical for \
                         CoonsPatch blocks), and Export to Eilmer is not available.\n\nInstall Eilmer (GDTk), \
                         then put its bin folder on PATH or set the LMR environment variable to the lmr program."
                    ),
                    details: String::new(),
                    error: false,
                }));
            }
        }
        if let Some(p) = path {
            app.open_path(&p);
        }
        app
    }

    // --- helpers -------------------------------------------------------------------------------
    fn message(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    fn show_message(&mut self, title: &str, text: impl Into<String>, error: bool) {
        self.dialog = Some(Dialog::Message(Message { title: title.into(), text: text.into(), details: String::new(), error }));
    }

    fn apply_shortcuts(&mut self) {
        self.shortcuts = DEFAULT_SHORTCUTS
            .iter()
            .filter_map(|(id, _, _)| KeySeq::parse(&self.settings.shortcut(id)).map(|k| (k, id.to_string())))
            .collect();
        let h = KeySeq::parse(&self.settings.shortcut("hold.horizontal")).map(|k| k.key).unwrap_or(Key::H);
        let v = KeySeq::parse(&self.settings.shortcut("hold.vertical")).map(|k| k.key).unwrap_or(Key::V);
        self.hold_keys = (h, v);
        self.held.clear();
    }

    fn shortcut_text(&self, id: &str) -> String {
        KeySeq::parse(&self.settings.shortcut(id)).map(|k| k.native()).unwrap_or_default()
    }

    fn tool_index(&self, name: &str) -> Option<usize> {
        self.tools.iter().position(|(t, _)| t.name() == name)
    }

    fn with_ctx<R>(&mut self, painter: &egui::Painter, f: impl FnOnce(&mut dyn Tool, &mut Ctx) -> R) -> R {
        let (tool, _) = &mut self.tools[self.tool];
        let mut c = Ctx {
            model: &mut self.model,
            canvas: &mut self.canvas,
            painter,
            events: &mut self.events,
            settings: &mut self.settings,
        };
        f(tool.as_mut(), &mut c)
    }

    pub fn set_tool(&mut self, ctx: &egui::Context, name: &str) {
        let Some(k) = self.tool_index(name) else { return };
        let painter = ctx.layer_painter(self.tool_painter_layer);
        self.with_ctx(&painter, |t, _| t.reset());
        self.tool = k;
        self.with_ctx(&painter, |t, _| t.reset());
        self.input_text.clear();
        self.input_error.clear();
        self.focus_canvas = true;
        self.menu_at = None;
    }

    fn title(&self) -> String {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or("untitled".into());
        format!("{name}{} - Eilmer Mesher", if self.model.is_clean() { "" } else { " *" })
    }

    // --- files -----------------------------------------------------------------------------------
    fn set_path(&mut self, path: Option<PathBuf>) {
        let workdir = path.as_ref().map(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let base = name.strip_suffix(PROJECT_SUFFIX).map(|s| s.to_string()).unwrap_or_else(|| {
                Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
            });
            let dir = std::fs::canonicalize(p).ok().and_then(|a| a.parent().map(|d| d.to_path_buf())).unwrap_or_default();
            dir.join(".emesh").join(base)
        });
        self.mesher.set_workdir(workdir);
        self.path = path;
    }

    pub fn open_path(&mut self, path: &Path) {
        match Document::load(path) {
            Ok(doc) => {
                self.set_path(Some(path.to_path_buf()));
                self.canvas.hidden_blocks.clear();
                self.model.reset(doc);
                self.model.set_clean();
                self.pending_zoom_fit = true;
                self.settings.remember_recent(&path.display().to_string());
                if let Some(d) = path.parent() {
                    self.settings.last_open_dir = d.display().to_string();
                }
                self.settings.save();
            }
            Err(e) => self.show_message("Open failed", format!("Could not open {}:\n{e}", path.display()), true),
        }
    }

    fn save(&mut self) -> bool {
        let Some(p) = self.path.clone() else {
            self.save_as();
            return false;
        };
        match self.model.doc.save(&p) {
            Ok(()) => {
                self.model.set_clean();
                self.message(format!("Saved {}", p.display()));
                self.settings.remember_recent(&p.display().to_string());
                self.settings.save();
                true
            }
            Err(e) => {
                self.show_message("Save failed", format!("Could not save {}:\n{e}", p.display()), true);
                false
            }
        }
    }

    fn start_dir(&self) -> PathBuf {
        if let Some(p) = &self.path {
            if let Some(d) = p.parent() {
                return d.to_path_buf();
            }
        }
        if !self.settings.last_open_dir.is_empty() {
            return PathBuf::from(&self.settings.last_open_dir);
        }
        std::env::current_dir().unwrap_or_default()
    }

    fn save_as(&mut self) {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or(format!("project{PROJECT_SUFFIX}"));
        let fb = FileBrowser::new("Save project", FileMode::Save, &self.start_dir(), &name, &["emesh.json"]);
        self.file_dialog(fb, FilePurpose::SaveAs);
    }

    /// Show a file browser with the project / recent folders as places; it starts in the folder
    /// last used for the same purpose this session (Save As of a saved project starts beside it).
    fn file_dialog(&mut self, fb: FileBrowser, purpose: FilePurpose) {
        let mut fb = fb.places(self.path.as_deref().and_then(Path::parent), &self.settings);
        if !(purpose == FilePurpose::SaveAs && self.path.is_some()) {
            fb = fb.remember(match purpose {
                FilePurpose::Open | FilePurpose::SaveAs => "project",
                FilePurpose::Import => "import",
                FilePurpose::Export => "export",
            });
        }
        self.dialog = Some(Dialog::File(fb, purpose));
    }

    fn confirm_then(&mut self, then: Pending) {
        if self.model.is_clean() {
            self.do_pending(then);
        } else {
            self.dialog = Some(Dialog::Confirm(then));
        }
    }

    fn do_pending(&mut self, p: Pending) {
        match p {
            Pending::New => {
                self.set_path(None);
                self.canvas.hidden_blocks.clear();
                self.model.reset(Document::new());
                self.pending_zoom_fit = true;
            }
            Pending::Open => {
                let fb = FileBrowser::new("Open project", FileMode::Open, &self.start_dir(), "", &["emesh.json"]);
                self.file_dialog(fb, FilePurpose::Open);
            }
            Pending::OpenPath(p) => self.open_path(&p),
            Pending::Quit => self.quit_confirmed = true,
        }
    }

    fn import_points(&mut self) {
        let fb = FileBrowser::new("Import point file", FileMode::Open, &self.start_dir(), "", &["dat", "txt", "csv", "xy"]);
        self.file_dialog(fb, FilePurpose::Import);
    }

    fn export_to_eilmer(&mut self) {
        let res = self.model.resolution();
        if !res.errors.is_empty() {
            let text: Vec<String> = res.errors.iter().map(|(b, m)| format!("{b}: {m}")).collect();
            self.show_message("Export", format!("Some blocks need attention before exporting:\n\n{}", text.join("\n")), true);
            return;
        }
        if res.blocks.is_empty() {
            self.show_message("Export", "There are no blocks to export yet (Block tool, K).", false);
            return;
        }
        let start = if !self.settings.last_export_dir.is_empty() { PathBuf::from(&self.settings.last_export_dir) } else { self.start_dir() };
        let fb = FileBrowser::new("Export to Eilmer: choose the simulation folder", FileMode::Folder, &start, "", &[]);
        self.file_dialog(fb, FilePurpose::Export);
    }

    pub fn start_export(&mut self, folder: PathBuf) {
        let doc = concrete(&self.model);
        let (tx, rx) = std::sync::mpsc::channel();
        let f = folder.clone();
        std::thread::spawn(move || {
            let _ = tx.send(export_grids(&doc, &f, true));
        });
        self.settings.last_export_dir = folder.display().to_string();
        self.settings.save();
        self.dialog = Some(Dialog::Exporting(folder, rx));
    }

    fn export_done(&mut self, folder: &Path, r: Result<LmrResult, RunError>) {
        match r {
            Err(e) => self.show_message("Export", format!("Export failed: {e}"), true),
            Ok(result) if result.ok() => {
                let res = self.model.resolution();
                let n_conn = exported_connections(folder).unwrap_or(0);
                let warn: usize = res.warnings.values().map(|w| w.len()).sum();
                let mut text = format!(
                    "Exported {} block(s) and {n_conn} connection(s) to\n{}\n\nFiles: {}",
                    res.blocks.len(),
                    folder.display(),
                    result.written.join(", ")
                );
                if warn > 0 {
                    text += &format!("\n\n{warn} block warning(s): check the blocks marked (!).");
                }
                text += "\n\nNext: edit sim.lua, then\n  lmr prep-gas -i ideal-air.lua -o ideal-air.gas\n  lmr prep-sim -j sim.lua\n  lmr run";
                self.dialog = Some(Dialog::Message(Message { title: "Export to Eilmer".into(), text, details: result.output, error: false }));
            }
            Ok(result) => {
                self.dialog = Some(Dialog::Message(Message {
                    title: "Export to Eilmer".into(),
                    text: format!("lmr prep-grids failed:\n{}", result.error_summary()),
                    details: result.output.clone(),
                    error: true,
                }))
            }
        }
    }

    // --- mesh and quality ---------------------------------------------------------------------------
    fn pump_mesher(&mut self) {
        for ev in self.mesher.poll() {
            match ev {
                MeshEvent::Status(s) => self.mesh_status = s,
                MeshEvent::Mesh(m) => {
                    if m.revision != self.model.revision {
                        continue; // stale: a newer edit is on its way
                    }
                    let keep_eilmer = m.source == Source::Preview
                        && self.canvas.mesh.as_ref().is_some_and(|o| o.revision == m.revision && o.source == Source::Eilmer);
                    if keep_eilmer {
                        continue;
                    }
                    if m.grids.is_empty() {
                        self.canvas.mesh = None;
                        self.update_quality();
                    } else {
                        self.canvas.mesh = Some(m);
                        self.update_quality();
                    }
                }
            }
        }
        if !self.canvas.show_mesh {
            return;
        }
        if self.sent_preview != Some(self.model.revision) {
            self.sent_preview = Some(self.model.revision);
            let res = self.model.resolution();
            if res.blocks.is_empty() {
                self.canvas.mesh = None;
                self.update_quality();
                self.mesh_status = if self.model.doc.blocks.is_empty() { "No blocks".into() } else { "Blocks need attention".into() };
            } else {
                self.mesher.preview(self.model.revision, concrete(&self.model));
            }
        }
        if self.model.committed && !self.model.editing() {
            self.model.committed = false;
            let res = self.model.resolution();
            if !res.blocks.is_empty() {
                self.mesher.eilmer(self.model.revision, concrete(&self.model));
            }
        }
    }

    fn update_quality(&mut self) {
        let Some(mesh) = &self.canvas.mesh else {
            self.quality = None;
            self.canvas.quality = None;
            self.banner = None;
            self.refine_targets.clear();
            self.ao_targets.clear();
            return;
        };
        let res = self.model.resolution();
        let layered = intentional_aspect(&res.blocks);
        let q = assess(mesh.revision, &mesh.grids, self.settings.quality.thresholds(), &layered);
        let (refine, ao, hint) = self.refine_plan(&q, &layered);
        self.refine_targets = refine;
        self.ao_targets = ao;
        let level = q.level();
        self.banner = if level == Level::Ok {
            None
        } else {
            let title = match level {
                Level::Warn => "Mesh quality",
                Level::Bad => "Mesh needs refinement",
                _ => "Mesh has inverted cells",
            };
            let text = format!("{title}: {}.{}", q.summary(3), if hint.is_empty() { String::new() } else { format!(" {hint}") });
            Some(Banner { level, text, dismissed: self.banner_dismissed.clone() })
        };
        self.canvas.quality = Some(q.clone());
        self.quality = Some(q);
    }

    fn refine_plan(&self, q: &MeshQuality, layered: &HashSet<String>) -> (Vec<String>, Vec<String>, String) {
        let stored: BTreeMap<&str, &emesh_core::Block> = self.model.doc.blocks.iter().map(|b| (b.id.as_str(), b)).collect();
        let bad: Vec<String> = q.flagged(&[Level::Bad, Level::Error]).into_iter().filter(|b| stored.contains_key(b.as_str())).collect();
        let targets: Vec<String> = if !bad.is_empty() {
            bad.clone()
        } else {
            q.blocks.iter().filter(|(b, bq)| bq.skew_warn > 0 && stored.contains_key(b.as_str())).map(|(b, _)| b.clone()).collect()
        };
        let refine: Vec<String> = targets
            .iter()
            .filter(|b| {
                let s = stored[b.as_str()];
                !s.smoothed() && s.patch == "CoonsPatch" && !layered.contains(*b)
            })
            .cloned()
            .collect();
        let ao: Vec<String> = bad
            .iter()
            .filter(|b| {
                let s = stored[b.as_str()];
                s.smoothed() && s.patch == "CoonsPatch" && !layered.contains(*b)
            })
            .cloned()
            .collect();
        let mut hints = vec![];
        if !ao.is_empty() {
            hints.push(format!("Smoothing has not fixed {}: try AOPatch, or sub-divide.", ao.join(", ")));
        }
        let stuck: Vec<String> = bad.iter().filter(|b| stored[b.as_str()].patch == "AOPatch").cloned().collect();
        if !stuck.is_empty() {
            hints.push(format!("Sub-divide {} or move its corners.", stuck.join(", ")));
        }
        if targets.iter().any(|b| layered.contains(b)) {
            hints.push("Boundary-layer blocks are not smoothed (it would undo the wall spacing).".into());
        }
        if q.level() == Level::Warn && targets.is_empty() {
            hints.push("Change the cell counts along the long edges to bring the aspect ratio down.".into());
        }
        (refine, ao, hints.join(" "))
    }

    fn refine_mesh(&mut self) {
        if self.refine_targets.is_empty() {
            self.message("No blocks need refining.");
            return;
        }
        let targets = self.refine_targets.clone();
        let (smoothed, ao, skipped) = self.model.refine_blocks(&targets);
        let mut parts = vec![];
        if !smoothed.is_empty() {
            parts.push(format!("Laplace smoothing on {}", smoothed.join(", ")));
        }
        if !ao.is_empty() {
            parts.push(format!("AOPatch on {} (smoothing would fold cells there)", ao.join(", ")));
        }
        let mut msg = if parts.is_empty() { String::new() } else { format!("Refine: {}; the mesh is being rebuilt.", parts.join("; ")) };
        if !skipped.is_empty() {
            msg += &format!(" Skipped boundary-layer block(s) {}.", skipped.join(", "));
        }
        self.message(msg.trim().to_string());
    }

    fn try_aopatch(&mut self) {
        if self.ao_targets.is_empty() {
            return;
        }
        let t = self.ao_targets.clone();
        self.model.set_block_patch(&t, Some(("AOPatch", json!({"nx": 10, "ny": 10}).as_object().unwrap().clone())), Smoothing::Set(None));
        self.message(format!("{} now use AOPatch.", t.join(", ")));
    }

    // --- actions -------------------------------------------------------------------------------------
    /// "light", "dark" or "system": switch now and remember it.
    fn set_theme(&mut self, ctx: &egui::Context, name: &str) {
        self.settings.theme = name.into();
        self.settings.save();
        theme::apply(ctx, name);
    }

    fn run_action(&mut self, ctx: &egui::Context, id: &str) {
        if std::env::var("EMESH_DEBUG").is_ok() {
            eprintln!("action {id}");
        }
        if let Some(name) = id.strip_prefix("tool.") {
            self.set_tool(ctx, name);
            return;
        }
        match id {
            "file.new" => self.confirm_then(Pending::New),
            "file.open" => self.confirm_then(Pending::Open),
            "file.save" => {
                self.save();
            }
            "file.save_as" => self.save_as(),
            "file.import" => self.import_points(),
            "file.export" => self.export_to_eilmer(),
            "file.quit" => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            "block.bcs" => self.dialog = Some(Dialog::BcManager { selected: None, sub: None }),
            "block.all" => {
                let (made, skipped) = self.model.make_all_blocks();
                let mut msg = format!("Made {} block(s).", made.len());
                if skipped > 0 {
                    msg += &format!(" {skipped} region(s) need sub-dividing first (holes or fewer than four corners).");
                }
                self.message(msg);
            }
            "block.refine" => self.refine_mesh(),
            "mesh.run" => {
                let res = self.model.resolution();
                if !res.blocks.is_empty() {
                    self.mesher.eilmer(self.model.revision, concrete(&self.model));
                }
            }
            "edit.undo" => self.model.undo(),
            "edit.redo" => self.model.redo(),
            "edit.delete" => self.model.delete_selection(),
            "edit.select_all" => {
                let ids = self.model.entity_ids();
                self.model.set_selection(ids);
            }
            "edit.toggle_reference" => self.model.toggle_reference(None),
            "edit.toggle_split" => self.model.toggle_split(None),
            "sketch.reference_mode" => {
                self.model.reference_mode = !self.model.reference_mode;
                if self.model.reference_mode {
                    self.model.split_mode = false;
                }
            }
            "sketch.split_mode" => {
                self.model.split_mode = !self.model.split_mode;
                if self.model.split_mode {
                    self.model.reference_mode = false;
                }
            }
            "snap.grid" => {
                let on = self.canvas.snapper.is_on(SnapOption::Grid);
                self.canvas.snapper.set(SnapOption::Grid, !on);
            }
            "view.fit" => self.canvas.zoom_to_fit(&self.model.doc),
            "view.regions" => self.canvas.show_regions = !self.canvas.show_regions,
            "view.grid" => {
                self.canvas.show_grid = !self.canvas.show_grid;
                self.settings.show_grid = self.canvas.show_grid;
                self.settings.save();
            }
            "view.blocks" => self.canvas.show_blocks = !self.canvas.show_blocks,
            "view.mesh" => {
                self.canvas.show_mesh = !self.canvas.show_mesh;
                if self.canvas.show_mesh {
                    self.sent_preview = None;
                    self.model.committed = true;
                }
            }
            "view.quality" => self.canvas.show_quality = !self.canvas.show_quality,
            "view.dark" => self.set_theme(ctx, if theme::is_dark() { "light" } else { "dark" }),
            "view.theme.light" => self.set_theme(ctx, "light"),
            "view.theme.dark" => self.set_theme(ctx, "dark"),
            "view.theme.system" => self.set_theme(ctx, "system"),
            _ => {}
        }
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        let events = std::mem::take(&mut self.events);
        for ev in events {
            match ev {
                AppEvent::Message(m) => self.message(m),
                AppEvent::ClearTyped => {
                    self.input_text.clear();
                    self.input_error.clear();
                }
                AppEvent::EditEntity(eid) => self.edit_entity(&eid),
                AppEvent::EdgeCells(bid, face) => {
                    if std::env::var("EMESH_DEBUG").is_ok() {
                        eprintln!("edge cells {bid} {face:?}");
                    }
                    let res = self.model.resolution();
                    let Some(b) = res.block(&bid) else { continue };
                    let cells = b.count(face) - 1;
                    let spec = self.model.doc.block(&bid).and_then(|s| s.cluster.get(&face).cloned());
                    self.dialog = Some(Dialog::EdgeCells(EdgeCells::new(&bid, face, cells, spec.as_ref(), self.canvas.units)));
                }
                AppEvent::SplitRegion(p) => self.dialog = Some(Dialog::Split(Split { point: p, n: 2, m: 2 })),
                AppEvent::NewBc(ids) => {
                    let mut d = BcDialog::new(&self.model.doc, None);
                    d.for_curves = ids;
                    self.dialog = Some(Dialog::Bc(d));
                }
                AppEvent::LayerSettings(target) => self.edit_layer(target),
                AppEvent::AoPatch(bid) => {
                    let b = self.model.doc.block(&bid);
                    let get = |k: &str| b.and_then(|b| b.patch_opts.get(k)).and_then(|v| v.as_i64()).unwrap_or(10);
                    self.dialog = Some(Dialog::AoPatch(AoPatch { bid: bid.clone(), nx: get("nx"), ny: get("ny") }));
                }
            }
        }
        let _ = ctx;
    }

    fn edit_layer(&mut self, target: Option<(String, Face)>) {
        let lid = target.as_ref().and_then(|(b, f)| self.model.layer_at_face(b, *f));
        let values = match lid.as_ref().and_then(|l| self.model.layer(l)) {
            Some(l) => crate::settings::LayerDefaults { first: l.first, growth: l.growth, cells: l.cells, transition: l.transition },
            None => self.settings.layer,
        };
        self.dialog = Some(Dialog::Layer(LayerDlg::new(target, lid, values, self.settings.layer_flow, self.canvas.units)));
    }

    fn edit_entity(&mut self, eid: &str) {
        let doc = &self.model.doc;
        let u = self.canvas.units;
        let un = u.name.to_string();
        let f = |label: &str, value: f64, suffix: &str, lo: f64, hi: f64| Field { label: label.into(), value, suffix: suffix.into(), lo, hi };
        let d = if let Some(p) = doc.points.get(eid) {
            let q = u.point_to_display(*p);
            EditDims::new(eid, EditKind::Point, "Point", vec![f("X", q[0], &un, -BIG, BIG), f("Y", q[1], &un, -BIG, BIG)], "")
        } else if let Some(dim) = doc.dimensions.get(eid) {
            let value = self.model.dimension_value(eid).unwrap_or(0.0);
            if dim.kind == "distance" {
                EditDims::new(eid, EditKind::Distance, "Distance dimension", vec![f("Distance", u.to_display(value), &un, 1e-12, BIG)], "The second line moves parallel to itself; the first stays.")
            } else if dim.curve2.is_some() {
                EditDims::new(eid, EditKind::TwoLineAngle, "Angle dimension", vec![f("Angle", value, "deg", 1e-9, 179.999999)], "The second line turns about the corner; the first stays.")
            } else if dim.kind == "length" {
                EditDims::new(eid, EditKind::Length, "Length dimension", vec![f("Length", u.to_display(value), &un, 1e-12, BIG)], "The line's start stays; its end moves.")
            } else {
                EditDims::new(eid, EditKind::ArcAngle, "Angle dimension", vec![f("Sweep angle", value, "deg", 1e-9, 359.999999)], "The arc's centre and start stay; its end moves.")
            }
        } else if let Some(c) = doc.curves.get(eid) {
            if c.is_line() {
                let (p0, p1) = c.ends_lp();
                let length = emesh_core::num::py_dist(p0, p1);
                let angle = (p1[1] - p0[1]).atan2(p1[0] - p0[0]).to_degrees();
                EditDims::new(eid, EditKind::Line, "Line", vec![f("Length", u.to_display(length), &un, 1e-12, BIG), f("Angle", angle, "deg", -360.0, 360.0)], "The start point stays; the end moves.")
            } else if c.is_arc() {
                let (_, r, _, sweep) = geom::arc_geom(c);
                EditDims::new(
                    eid,
                    EditKind::Arc,
                    "Arc",
                    vec![f("Radius", u.to_display(r), &un, 1e-12, BIG), f("Sweep angle", sweep.abs().to_degrees(), "deg", 1e-9, 359.999999)],
                    &format!("Arc length {}.\nThe centre and start direction stay; the end moves.", u.format((r * sweep).abs(), None)),
                )
            } else {
                return;
            }
        } else {
            return;
        };
        self.dialog = Some(Dialog::EditDims(d));
    }

    fn apply_edit(&mut self, d: &EditDims) -> Result<(), String> {
        let v = d.values()?;
        let u = self.canvas.units;
        match d.kind {
            EditKind::Point => {
                self.model.set_point(&d.eid, u.point_from_display([v[0], v[1]]));
                Ok(())
            }
            EditKind::Line => self.model.set_line(&d.eid, u.from_display(v[0]), v[1]),
            EditKind::Arc => self.model.set_arc(&d.eid, u.from_display(v[0]), v[1]),
            EditKind::Length | EditKind::Distance => self.model.set_dimension_value(&d.eid, u.from_display(v[0])),
            EditKind::ArcAngle | EditKind::TwoLineAngle => self.model.set_dimension_value(&d.eid, v[0]),
        }
    }

    // --- dialogs ---------------------------------------------------------------------------------------
    fn show_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let keep = match &mut dialog {
            Dialog::EdgeCells(d) => match d.show(ctx) {
                Some(true) => {
                    if let Ok((n, spec, opp)) = d.values() {
                        if let Err(e) = self.model.set_edge_cells(&d.bid, d.face, n, spec, opp) {
                            self.message(e);
                        }
                    }
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::Split(d) => match d.show(ctx) {
                Some(true) => {
                    if let Err(e) = self.model.subdivide(d.point, Subdivision::Nxm(d.n, d.m)) {
                        self.message(format!("Cannot split: {e}."));
                    }
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::AoPatch(d) => match d.show(ctx) {
                Some(true) => {
                    let opts = json!({"nx": d.nx, "ny": d.ny}).as_object().unwrap().clone();
                    self.model.set_block_patch(std::slice::from_ref(&d.bid), Some(("AOPatch", opts)), Smoothing::Set(None));
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::Bc(d) => match d.show(ctx) {
                Some(true) => {
                    let (tag, cls, args) = d.values();
                    match self.model.define_bc(&tag, &cls, &args, d.old_tag.as_deref()) {
                        Ok(()) => {
                            if !d.for_curves.is_empty() {
                                if let Ok(n) = self.model.set_bc(&d.for_curves, &tag) {
                                    self.message(format!("'{tag}' ({cls}) set on {n} curve(s)."));
                                }
                            }
                        }
                        Err(e) => self.show_message("Boundary condition", e, true),
                    }
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::BcManager { selected, sub } => self.bc_manager(ctx, selected, sub),
            Dialog::EditDims(d) => match d.show(ctx) {
                Some(true) => {
                    if let Err(e) = self.apply_edit(d) {
                        self.show_message("Edit", e, true);
                    }
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::Layer(d) => match d.show(ctx) {
                Some(true) => {
                    self.settings.layer_flow = d.flow_values(self.settings.layer_flow);
                    self.settings.save();
                    let r = if d.removed {
                        self.model.remove_boundary_layer(d.lid.as_deref().unwrap()).map(|_| "Boundary layer removed.".to_string())
                    } else if let Some(lid) = &d.lid {
                        let v = d.values().unwrap();
                        let s = LayerSettings { first: Some(v.first), growth: Some(v.growth), cells: Some(v.cells), transition: Some(v.transition) };
                        self.model.edit_boundary_layer(lid, s).map(|_| {
                            let note = self.model.layer_note(lid);
                            format!("Boundary layer cut again with the new settings. {note}")
                        })
                    } else {
                        self.settings.layer = d.values().unwrap();
                        self.settings.save();
                        Ok("Settings saved for the next boundary layer.".into())
                    };
                    match r {
                        Ok(m) => self.message(m),
                        Err(e) => self.show_message("Boundary layer", format!("Cannot change the boundary layer: {e}."), true),
                    }
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::Import(d) => match d.show(ctx) {
                Some(true) => {
                    match d.curves() {
                        Ok(cs) => {
                            let ids = self.model.add_curves(cs, &format!("Import {}", d.pf.name));
                            self.model.set_selection(ids);
                            self.pending_zoom_fit = true;
                        }
                        Err(e) => self.show_message("Import failed", e, true),
                    }
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::Shortcuts(d) => match d.show(ctx) {
                Some(true) => {
                    self.settings.set_shortcuts(&d.values);
                    self.settings.save();
                    self.apply_shortcuts();
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::Thresholds(d) => match d.show(ctx) {
                Some(true) => {
                    self.settings.quality = d.values();
                    self.settings.save();
                    self.update_quality();
                    false
                }
                Some(false) => false,
                None => true,
            },
            Dialog::Message(m) => m.show(ctx).is_none(),
            Dialog::Confirm(then) => match dialogs::confirm_discard(ctx) {
                Some(Choice::Save) => {
                    let then = then.clone();
                    if self.save() {
                        self.do_pending(then);
                        return;
                    }
                    false
                }
                Some(Choice::Discard) => {
                    let then = then.clone();
                    self.do_pending(then);
                    return;
                }
                Some(Choice::Cancel) => false,
                None => true,
            },
            Dialog::File(fb, purpose) => match fb.show(ctx) {
                Some(Some(p)) => {
                    let purpose = *purpose;
                    self.file_chosen(purpose, p);
                    return;
                }
                Some(None) => false,
                None => true,
            },
            Dialog::Exporting(folder, rx) => {
                let mut done = None;
                dialogs::modal(ctx, "Export to Eilmer", |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Running lmr prep-grids...");
                    });
                });
                if let Ok(r) = rx.try_recv() {
                    done = Some(r);
                }
                match done {
                    Some(r) => {
                        let f = folder.clone();
                        self.export_done(&f, r);
                        return;
                    }
                    None => {
                        ctx.request_repaint_after(Duration::from_millis(50));
                        true
                    }
                }
            }
            Dialog::About => dialogs::modal(ctx, "About Eilmer Mesher", |ui| {
                ui.label(RichText::new(format!("Eilmer Mesher {}", env!("CARGO_PKG_VERSION"))).strong());
                if !env!("EMESH_COMMIT").is_empty() {
                    ui.label(format!("Build {}", env!("EMESH_COMMIT")));
                }
                ui.label("Sketch 2D geometry and build block-structured grids for the Eilmer flow solver.");
                ui.label(match find_lmr() {
                    Ok(p) => format!("Eilmer: {}", p.display()),
                    Err(e) => format!("Eilmer: {e}"),
                });
                ui.label(format!("Settings: {}", crate::settings::settings_path().map(|p| p.display().to_string()).unwrap_or_default()));
                dialogs::ok_cancel(ui, true)
            })
            .flatten()
            .is_none(),
        };
        if keep && self.dialog.is_none() {
            self.dialog = Some(dialog);
        }
    }

    fn file_chosen(&mut self, purpose: FilePurpose, p: PathBuf) {
        match purpose {
            FilePurpose::Open => self.open_path(&p),
            FilePurpose::SaveAs => {
                let mut s = p.display().to_string();
                if !s.ends_with(PROJECT_SUFFIX) {
                    s += PROJECT_SUFFIX;
                }
                self.set_path(Some(PathBuf::from(s)));
                self.save();
            }
            FilePurpose::Import => {
                let text = match std::fs::read_to_string(&p) {
                    Ok(t) => t,
                    Err(e) => return self.show_message("Import failed", format!("Could not read {}:\n{e}", p.display()), true),
                };
                let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                match emesh_core::importers::parse_point_file(&text, &stem) {
                    Ok(pf) => self.dialog = Some(Dialog::Import(ImportDlg::new(pf, &p.display().to_string(), self.canvas.units))),
                    Err(e) => self.show_message("Import failed", format!("Could not read {}:\n{e}", p.display()), true),
                }
            }
            FilePurpose::Export => self.start_export(p),
        }
    }

    fn bc_manager(&mut self, ctx: &egui::Context, selected: &mut Option<String>, sub: &mut Option<BcDialog>) -> bool {
        if let Some(d) = sub.as_mut() {
            match d.show(ctx) {
                Some(true) => {
                    let (tag, cls, args) = d.values();
                    if let Err(e) = self.model.define_bc(&tag, &cls, &args, d.old_tag.as_deref()) {
                        self.message(e);
                    }
                    *sub = None;
                }
                Some(false) => *sub = None,
                None => {}
            }
            return true;
        }
        let usage = self.model.bc_usage();
        let mut tags: Vec<String> = self.model.doc.bcs.keys().cloned().collect();
        tags.sort();
        let mut close = false;
        dialogs::modal(ctx, "Boundary conditions", |ui| {
            ui.set_min_width(620.0);
            ui.label("Untagged solid curves are inviscid slip walls (Eilmer's default).");
            egui::Grid::new("bcs").num_columns(4).striped(true).min_col_width(80.0).show(ui, |ui| {
                for h in ["Tag", "Eilmer class", "Arguments", "Curves"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for t in &tags {
                    let spec = &self.model.doc.bcs[t];
                    let color = crate::canvas::tag_color(&self.model.doc, t);
                    let r = ui.selectable_label(selected.as_deref() == Some(t), RichText::new(format!("\u{25A0} {t}")).color(color));
                    if r.clicked() {
                        *selected = Some(t.clone());
                    }
                    if r.double_clicked() {
                        *sub = Some(BcDialog::new(&self.model.doc, Some(t)));
                    }
                    ui.label(&spec.class);
                    ui.label(&spec.args);
                    ui.label(usage.iter().find(|u| &u.0 == t).map(|u| u.1).unwrap_or(0).to_string());
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                if ui.button("New...").clicked() {
                    *sub = Some(BcDialog::new(&self.model.doc, None));
                }
                if ui.add_enabled(selected.is_some(), egui::Button::new("Edit...")).clicked() {
                    *sub = Some(BcDialog::new(&self.model.doc, selected.as_deref()));
                }
                if ui.add_enabled(selected.is_some(), egui::Button::new("Delete")).clicked() {
                    if let Some(t) = selected.take() {
                        self.model.delete_bc(&t);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                close = true;
            }
        });
        !close
    }

    // --- panels ------------------------------------------------------------------------------------------
    fn menu_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let item = |ui: &mut egui::Ui, text: &str, sc: String| -> bool {
            let b = egui::Button::new(text).shortcut_text(sc);
            let r = ui.add(b).clicked();
            if r {
                ui.close_menu();
            }
            r
        };
        let toggle = |ui: &mut egui::Ui, text: &str, sc: String, on: bool| -> bool {
            let r = ui.add(checked(egui::Button::new(text).shortcut_text(sc), on)).clicked();
            if r {
                ui.close_menu();
            }
            r
        };
        let mut run: Option<String> = None;
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                for (id, text) in [("file.new", "New"), ("file.open", "Open..."), ("file.save", "Save"), ("file.save_as", "Save As...")] {
                    if item(ui, text, self.shortcut_text(id)) {
                        run = Some(id.into());
                    }
                }
                ui.menu_button("Open recent", |ui| {
                    if self.settings.recent.is_empty() {
                        ui.label("(none)");
                    }
                    for p in self.settings.recent.clone() {
                        if ui.button(&p).clicked() {
                            self.confirm_then(Pending::OpenPath(PathBuf::from(&p)));
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                if item(ui, "Import points (.dat/.txt)...", self.shortcut_text("file.import")) {
                    run = Some("file.import".into());
                }
                if item(ui, "Export to Eilmer...", self.shortcut_text("file.export")) {
                    run = Some("file.export".into());
                }
                ui.separator();
                if item(ui, "Quit", self.shortcut_text("file.quit")) {
                    run = Some("file.quit".into());
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self.model.undo_text().map(|t| format!("Undo {t}")).unwrap_or("Undo".into());
                let redo = self.model.redo_text().map(|t| format!("Redo {t}")).unwrap_or("Redo".into());
                if ui.add_enabled(self.model.can_undo(), egui::Button::new(undo).shortcut_text(self.shortcut_text("edit.undo"))).clicked() {
                    run = Some("edit.undo".into());
                    ui.close_menu();
                }
                if ui.add_enabled(self.model.can_redo(), egui::Button::new(redo).shortcut_text(self.shortcut_text("edit.redo"))).clicked() {
                    run = Some("edit.redo".into());
                    ui.close_menu();
                }
                ui.separator();
                for (id, text) in [
                    ("edit.delete", "Delete"),
                    ("edit.select_all", "Select all"),
                    ("edit.toggle_reference", "Convert to/from reference"),
                    ("edit.toggle_split", "Convert to/from block split line"),
                ] {
                    if item(ui, text, self.shortcut_text(id)) {
                        run = Some(id.into());
                    }
                }
                ui.separator();
                if ui.button("Keyboard shortcuts...").clicked() {
                    let cur: BTreeMap<String, String> =
                        DEFAULT_SHORTCUTS.iter().chain(DEFAULT_HOLD_KEYS).map(|(id, _, _)| (id.to_string(), self.settings.shortcut(id))).collect();
                    self.dialog = Some(Dialog::Shortcuts(ShortcutsDlg::new(cur)));
                    ui.close_menu();
                }
            });
            ui.menu_button("Sketch", |ui| {
                for k in 0..self.tools.len() {
                    let (name, label) = (self.tools[k].0.name(), self.tools[k].0.label());
                    if toggle(ui, label, self.shortcut_text(&format!("tool.{name}")), k == self.tool) {
                        run = Some(format!("tool.{name}"));
                    }
                }
                ui.separator();
                if toggle(ui, "Draw reference (dashed) lines", self.shortcut_text("sketch.reference_mode"), self.model.reference_mode) {
                    run = Some("sketch.reference_mode".into());
                }
                if toggle(ui, "Draw block split (dash-dot) lines", self.shortcut_text("sketch.split_mode"), self.model.split_mode) {
                    run = Some("sketch.split_mode".into());
                }
                ui.menu_button("Snap to", |ui| self.snap_menu(ui));
            });
            ui.menu_button("Block", |ui| {
                for name in ["block", "layer"] {
                    let k = self.tool_index(name).unwrap();
                    if toggle(ui, self.tools[k].0.label(), self.shortcut_text(&format!("tool.{name}")), k == self.tool) {
                        run = Some(format!("tool.{name}"));
                    }
                }
                for (id, text) in [("block.all", "Block all regions"), ("block.bcs", "Boundary conditions..."), ("mesh.run", "Run Eilmer now")] {
                    if item(ui, text, self.shortcut_text(id)) {
                        run = Some(id.into());
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(!self.refine_targets.is_empty(), egui::Button::new("Refine flagged blocks").shortcut_text(self.shortcut_text("block.refine")))
                    .on_hover_text("Laplace-smooth the blocks with poor cells")
                    .clicked()
                {
                    run = Some("block.refine".into());
                    ui.close_menu();
                }
            });
            ui.menu_button("View", |ui| {
                if item(ui, "Zoom to fit", self.shortcut_text("view.fit")) {
                    run = Some("view.fit".into());
                }
                for (id, text, on) in [
                    ("view.regions", "Show regions", self.canvas.show_regions),
                    ("view.grid", "Show grid", self.canvas.show_grid),
                    ("view.blocks", "Show blocks", self.canvas.show_blocks),
                ] {
                    if toggle(ui, text, self.shortcut_text(id), on) {
                        run = Some(id.into());
                    }
                }
                if ui.add_enabled(!self.canvas.hidden_blocks.is_empty(), egui::Button::new("Show all blocks")).clicked() {
                    self.canvas.hidden_blocks.clear();
                    ui.close_menu();
                }
                for (id, text, on) in [("view.mesh", "Show mesh", self.canvas.show_mesh), ("view.quality", "Show cell quality", self.canvas.show_quality)] {
                    if toggle(ui, text, self.shortcut_text(id), on) {
                        run = Some(id.into());
                    }
                }
                ui.menu_button("Quality metric", |ui| {
                    for (m, text) in [(Metric::Skew, "Skewness"), (Metric::AspectRatio, "Aspect ratio")] {
                        if ui.add(checked(egui::Button::new(text), self.canvas.quality_metric == m)).clicked() {
                            self.canvas.quality_metric = m;
                            self.canvas.show_quality = true;
                            ui.close_menu();
                        }
                    }
                });
                if ui.button("Quality thresholds...").clicked() {
                    self.dialog = Some(Dialog::Thresholds(ThresholdsDlg { v: self.settings.quality }));
                    ui.close_menu();
                }
                ui.menu_button("Units", |ui| {
                    for (name, _) in UNITS {
                        if ui.add(checked(egui::Button::new(name), self.canvas.units.name == name)).clicked() {
                            self.canvas.units = Units::new(name);
                            self.settings.units = name.into();
                            self.settings.save();
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                if toggle(ui, "Dark mode", self.shortcut_text("view.dark"), theme::is_dark()) {
                    run = Some("view.dark".into());
                }
                ui.menu_button("Theme", |ui| {
                    for (name, text) in [("light", "Light"), ("dark", "Dark"), ("system", "Follow system")] {
                        if ui.add(checked(egui::Button::new(text), self.settings.theme == name)).clicked() {
                            run = Some(format!("view.theme.{name}"));
                            ui.close_menu();
                        }
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About Eilmer Mesher").clicked() {
                    self.dialog = Some(Dialog::About);
                    ui.close_menu();
                }
            });
        });
        if let Some(id) = run {
            self.run_action(ctx, &id);
        }
    }

    fn snap_menu(&mut self, ui: &mut egui::Ui) {
        for (o, label) in SNAP_OPTIONS {
            let mut on = self.canvas.snapper.is_on(o);
            let text = if o == SnapOption::Grid { format!("{label}   [{}]", self.shortcut_text("snap.grid")) } else { label.to_string() };
            if ui.checkbox(&mut on, text).changed() {
                self.canvas.snapper.set(o, on);
            }
        }
    }

    fn tip(&self, id: &str, text: &str) -> String {
        let sc = self.shortcut_text(id);
        if sc.is_empty() {
            text.to_string()
        } else {
            format!("{text}  [{sc}]")
        }
    }

    fn icon_button(&self, ui: &mut egui::Ui, icon: &str, selected: bool, tip: &str) -> egui::Response {
        let size = Vec2::splat(30.0);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let p = theme::palette();
        if selected || resp.hovered() {
            let (fill, edge) = if selected { p.button_on } else { p.button_hover };
            let fill = if resp.is_pointer_button_down_on() { ui.visuals().widgets.active.bg_fill } else { fill };
            ui.painter().rect(rect, 5.0, fill, Stroke::new(1.0f32, edge), egui::StrokeKind::Inside);
        }
        icons::paint(ui, icon, rect.shrink(3.0));
        resp.on_hover_text(tip)
    }

    fn toolbar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let mut run: Option<String> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for k in 0..self.tools.len() {
                let (name, label, tip) = (self.tools[k].0.name(), self.tools[k].0.label(), self.tools[k].1);
                if name == "block" {
                    ui.separator();
                }
                let sc = self.shortcut_text(&format!("tool.{name}"));
                let tip = if sc.is_empty() { format!("{label}: {tip}") } else { format!("{label}: {tip}  [{sc}]") };
                if self.icon_button(ui, name, k == self.tool, &tip).clicked() {
                    run = Some(format!("tool.{name}"));
                }
            }
            ui.separator();
            if self.icon_button(ui, "reference", self.model.reference_mode, &self.tip("sketch.reference_mode", "Draw reference (dashed) lines: drawing aids the mesher ignores")).clicked() {
                run = Some("sketch.reference_mode".into());
            }
            if self.icon_button(ui, "split", self.model.split_mode, &self.tip("sketch.split_mode", "Draw block split (dash-dot) lines")).clicked() {
                run = Some("sketch.split_mode".into());
            }
            let snap = self.icon_button(ui, "snap", false, "Snap options");
            let popup = ui.make_persistent_id("snap-popup");
            if snap.clicked() {
                ui.memory_mut(|m| m.toggle_popup(popup));
            }
            egui::popup_below_widget(ui, popup, &snap, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                ui.set_min_width(200.0);
                self.snap_menu(ui);
            });
            for (id, icon, on, text) in [
                ("view.regions", "regions", self.canvas.show_regions, "Show regions"),
                ("view.grid", "grid", self.canvas.show_grid, "Show grid"),
                ("view.mesh", "mesh", self.canvas.show_mesh, "Show mesh"),
                ("view.quality", "quality", self.canvas.show_quality, "Show cell quality"),
            ] {
                if self.icon_button(ui, icon, on, &self.tip(id, text)).clicked() {
                    run = Some(id.into());
                }
            }
            ui.separator();
            for (id, icon, text) in [("file.import", "import", "Import points"), ("file.export", "export", "Export to Eilmer"), ("view.fit", "fit", "Zoom to fit")] {
                if self.icon_button(ui, icon, false, &self.tip(id, text)).clicked() {
                    run = Some(id.into());
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (icon, text) = if theme::is_dark() { ("sun", "Light mode") } else { ("moon", "Dark mode") };
                if self.icon_button(ui, icon, false, &self.tip("view.dark", text)).clicked() {
                    run = Some("view.dark".into());
                }
            });
        });
        if let Some(id) = run {
            self.run_action(ctx, &id);
        }
    }

    fn banner_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let Some(b) = &self.banner else { return };
        if b.dismissed.as_deref() == Some(b.text.as_str()) {
            return;
        }
        let (bg, border) = theme::palette().banner[match b.level {
            Level::Warn => 0,
            Level::Bad => 1,
            _ => 2,
        }];
        let text = b.text.clone();
        let mut act: Option<&str> = None;
        egui::Frame::new().fill(bg).stroke(Stroke::new(1.0f32, border)).inner_margin(egui::Margin::symmetric(10, 5)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&text).color(col::ink())).wrap());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("x").on_hover_text("Hide until the mesh quality changes").clicked() {
                        act = Some("dismiss");
                    }
                    if ui.selectable_label(self.canvas.show_quality, "Show cells").clicked() {
                        act = Some("cells");
                    }
                    if !self.ao_targets.is_empty() && ui.button("Try AOPatch").on_hover_text("Switch the blocks that smoothing has not fixed to Eilmer's AOPatch").clicked() {
                        act = Some("ao");
                    }
                    if !self.refine_targets.is_empty() && ui.button("Refine").on_hover_text("Laplace-smooth the blocks with poor cells").clicked() {
                        act = Some("refine");
                    }
                });
            });
        });
        match act {
            Some("dismiss") => {
                self.banner_dismissed = Some(text);
                if let Some(b) = self.banner.as_mut() {
                    b.dismissed = self.banner_dismissed.clone();
                }
            }
            Some("cells") => self.run_action(ctx, "view.quality"),
            Some("ao") => self.try_aopatch(),
            Some("refine") => self.refine_mesh(),
            _ => {}
        }
    }

    fn input_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let painter = ctx.layer_painter(self.tool_painter_layer);
        let (label, prompt, takes, name) =
            self.with_ctx(&painter, |t, c| (t.label(), t.prompt(c), t.takes_input(), t.name()));
        ui.horizontal(|ui| {
            ui.add_sized([230.0, 20.0], egui::Label::new(RichText::new(format!("{label}  ")).strong()).truncate());
            ui.label(prompt);
            let u = self.canvas.units.name;
            let hint = if name == "move" {
                format!("type a move in {u}:  dx, dy    or  L<deg")
            } else {
                format!("type in {u}:  x, y    @dx, dy    @L<deg    or a length / radius / angle")
            };
            let id = egui::Id::new("input-bar");
            if self.focus_input && takes {
                // focus before the field runs, so it takes this frame's keys
                ctx.memory_mut(|m| m.request_focus(id));
                let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
                let n = self.input_text.chars().count();
                st.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(n))));
                st.store(ctx, id);
            }
            let focused_now = self.focus_input && takes;
            self.focus_input = false;
            let edit = egui::TextEdit::singleline(&mut self.input_text).id(id).hint_text(hint).desired_width(ui.available_width() - 260.0);
            let r = ui.add_enabled(takes, edit);
            let submitted = (r.lost_focus() || focused_now) && ui.input(|i| i.key_pressed(Key::Enter));
            if submitted {
                ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)); // not for the canvas too
            }
            if r.changed() {
                let t = self.input_text.clone();
                self.with_ctx(&painter, |tool, c| tool.preview_text(c, &t));
            }
            if submitted {
                let text = self.input_text.trim().to_string();
                if !text.is_empty() {
                    let res = self.with_ctx(&painter, |tool, c| tool.accept_text(c, &text));
                    match res {
                        Ok(()) => {
                            self.input_text.clear();
                            self.input_error.clear();
                            self.with_ctx(&painter, |tool, c| tool.preview_text(c, ""));
                        }
                        Err(e) => {
                            self.input_error = e;
                            self.focus_input = true;
                        }
                    }
                }
                self.focus_canvas = self.input_error.is_empty();
            }
            if r.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                self.input_text.clear();
                self.input_error.clear();
                self.with_ctx(&painter, |tool, c| tool.preview_text(c, ""));
                self.focus_canvas = true;
            }
            ui.colored_label(col::error_text(), &self.input_error);
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let painter = ui.painter().clone();
        let hint = self.with_ctx(&painter, |t, c| t.hint(c));
        let msg = match &self.status {
            Some((m, t)) if t.elapsed() < Duration::from_secs(if m.len() < 90 { 6 } else { 15 }) => m.clone(),
            _ => hint,
        };
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(s) = &self.canvas.last_snap {
                    let u = self.canvas.units;
                    let digits = u.digits(self.canvas.grid_step());
                    let p = u.point_to_display(s.point);
                    let kind = if s.kind == SnapKind::None { String::new() } else { format!("   [{}]", s.kind.label()) };
                    ui.add_sized([340.0, 18.0], egui::Label::new(RichText::new(format!("x = {:.*}   y = {:.*}  {}{kind}", digits, p[0], digits, p[1], u.name)).monospace()));
                }
                ui.separator();
                ui.label(&self.mesh_status);
                ui.separator();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(msg).truncate());
                });
            });
        });
    }

    // --- canvas -------------------------------------------------------------------------------------------
    fn apply_hold(&self, snap: Snap, anchor: Option<emesh_core::P>) -> Snap {
        let Some(a) = anchor else { return snap };
        let [x, y] = snap.point;
        if self.held.contains(&self.hold_keys.0) {
            return Snap { point: [x, a[1]], kind: SnapKind::Horizontal, curve_id: snap.curve_id };
        }
        if self.held.contains(&self.hold_keys.1) {
            return Snap { point: [a[0], y], kind: SnapKind::Vertical, curve_id: snap.curve_id };
        }
        snap
    }

    fn tool_event(&mut self, pos: Pos2, modifiers: Modifiers) -> ToolEvent {
        let world = self.canvas.view.to_world(pos);
        let tool = &self.tools[self.tool].0;
        let snap = if tool.snaps() {
            let anchor = tool.anchor();
            let exclude = tool.snap_exclude();
            let s = self.canvas.snap(&self.model, world, anchor, &exclude);
            self.apply_hold(s, anchor)
        } else {
            Snap::free(world)
        };
        self.canvas.last_snap = Some(snap.clone());
        self.canvas.last_screen = Some(pos);
        ToolEvent { world, snap, screen: pos, modifiers, dragging: self.primary_down }
    }

    fn canvas_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        let first = self.canvas.view.rect.width() < 10.0 || self.canvas.view.rect == egui::Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        self.canvas.view.rect = rect;
        if first || self.pending_zoom_fit {
            self.pending_zoom_fit = false;
            self.canvas.zoom_to_fit(&self.model.doc);
        }
        if resp.hovered() && self.pan.is_none() {
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
        } else if self.pan.is_some() {
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        let painter = ui.painter_at(rect);
        self.tool_painter_layer = painter.layer_id();
        let modal = self.dialog.is_some() || self.tools[self.tool].0.modal();
        if self.focus_canvas && !modal {
            resp.request_focus();
            self.focus_canvas = false;
        }
        if resp.clicked() || resp.secondary_clicked() || resp.drag_started() {
            resp.request_focus();
        }
        if !modal {
            self.canvas_input(ctx, &resp, &painter);
        }

        // painting
        let topo = self.model.topology();
        let res = self.model.resolution();
        let mut draw = Draw::new(&painter, self.canvas.view);
        let inp = PaintInput { model: &self.model, topo: &topo, res: &res };
        self.canvas.paint_background(&mut draw);
        self.canvas.paint_world(&mut draw, &inp);
        {
            let (tool, _) = &mut self.tools[self.tool];
            let c = Ctx { model: &mut self.model, canvas: &mut self.canvas, painter: &painter, events: &mut self.events, settings: &mut self.settings };
            tool.paint(&mut draw, &c);
        }
        let inp = PaintInput { model: &self.model, topo: &topo, res: &res };
        self.canvas.paint_overlay(&mut draw, &inp);
        let anchor = {
            let (tool, _) = &mut self.tools[self.tool];
            let c = Ctx { model: &mut self.model, canvas: &mut self.canvas, painter: &painter, events: &mut self.events, settings: &mut self.settings };
            tool.paint_overlay(&mut draw, &c);
            if tool.snaps() {
                tool.anchor()
            } else {
                None
            }
        };
        if self.tools[self.tool].0.snaps() {
            self.canvas.paint_snap(&mut draw, anchor);
        }
        self.canvas.paint_legends(&mut draw, &self.model.doc);
        draw.flush();

        // context menu
        if let Some(pos) = self.menu_at {
            let mut chosen = false;
            let area = egui::Area::new(egui::Id::new("canvas-menu")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(180.0);
                    // look like egui's own menus: flat items, full width
                    let st = ui.style_mut();
                    st.spacing.button_padding = Vec2::new(4.0, 1.0);
                    st.visuals.widgets.active.bg_stroke = Stroke::NONE;
                    st.visuals.widgets.hovered.bg_stroke = Stroke::NONE;
                    st.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    st.visuals.widgets.inactive.bg_stroke = Stroke::NONE;
                    st.wrap_mode = Some(egui::TextWrapMode::Extend);
                    let (tool, _) = &mut self.tools[self.tool];
                    let mut c = Ctx { model: &mut self.model, canvas: &mut self.canvas, painter: &painter, events: &mut self.events, settings: &mut self.settings };
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        chosen = tool.context_menu(ui, &mut c);
                    });
                });
            });
            let pos = ctx.input(|i| i.pointer.interact_pos());
            let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && !area.response.contains_pointer() && !Self::over_popup(ctx, pos);
            if chosen || ctx.input(|i| i.key_pressed(Key::Escape)) || (clicked_outside && !self.menu_opened && !ctx.memory(|m| m.any_popup_open())) {
                self.menu_at = None;
            }
            self.menu_opened = false;
        }
    }

    /// A menu, popup or window is under the pointer (not the panels behind the canvas).
    fn over_popup(ctx: &egui::Context, pos: Option<Pos2>) -> bool {
        pos.and_then(|p| ctx.layer_id_at(p)).is_some_and(|l| l.order != egui::Order::Background)
    }

    fn canvas_input(&mut self, ctx: &egui::Context, resp: &egui::Response, painter: &egui::Painter) {
        let rect = resp.rect;
        let events = ctx.input(|i| i.events.clone());
        let modifiers = ctx.input(|i| i.modifiers);
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let over = hover.is_some_and(|p| rect.contains(p)) && !Self::over_popup(ctx, hover) || self.primary_down;
        let mut moved_to: Option<Pos2> = None;
        for ev in &events {
            match ev {
                egui::Event::PointerMoved(p) => moved_to = Some(*p),
                egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                    let inside = rect.contains(*pos) && (self.menu_at.is_none() || !*pressed);
                    match (button, pressed) {
                        (PointerButton::Middle, true) if inside => self.pan = Some((*pos, self.canvas.view.cx, self.canvas.view.cy)),
                        (PointerButton::Middle, false) => self.pan = None,
                        (PointerButton::Primary, true) if inside && !Self::over_popup(ctx, Some(*pos)) => {
                            self.menu_at = None;
                            self.primary_down = true;
                            let te = self.tool_event(*pos, *modifiers);
                            let double = self.last_click.is_some_and(|(t, p)| t.elapsed() < Duration::from_millis(400) && (p - *pos).length() < 6.0);
                            self.last_click = Some((Instant::now(), *pos));
                            if std::env::var("EMESH_DEBUG").is_ok() {
                                eprintln!("press {:?} double {double} world {:?}", pos, te.world);
                            }
                            if double {
                                self.last_click = None;
                                self.with_ctx(painter, |t, c| t.double_click(c, &te));
                            } else {
                                self.with_ctx(painter, |t, c| t.press(c, &te));
                            }
                        }
                        (PointerButton::Primary, false) if self.primary_down => {
                            let te = self.tool_event(*pos, *modifiers);
                            self.primary_down = false;
                            self.with_ctx(painter, |t, c| t.release(c, &te));
                        }
                        (PointerButton::Secondary, true) if inside && !Self::over_popup(ctx, Some(*pos)) => {
                            let te = self.tool_event(*pos, *modifiers);
                            let wants = self.with_ctx(painter, |t, c| t.right_click(c, &te));
                            self.menu_at = if wants { Some(*pos) } else { None };
                            self.menu_opened = true;
                        }
                        _ => {}
                    }
                }
                egui::Event::MouseWheel { delta, unit, .. } if over => {
                    let dy = match unit {
                        egui::MouseWheelUnit::Line => delta.y * 1.0,
                        egui::MouseWheelUnit::Page => delta.y * 10.0,
                        egui::MouseWheelUnit::Point => delta.y / 50.0,
                    };
                    if let Some(p) = hover {
                        self.canvas.view.zoom_at(1.2f64.powf(dy as f64), p);
                    }
                }
                egui::Event::Zoom(z) if over => {
                    if let Some(p) = hover {
                        self.canvas.view.zoom_at(*z as f64, p);
                    }
                }
                _ => {}
            }
        }
        if let (Some((start, cx, cy)), Some(p)) = (self.pan, hover) {
            let d = p - start;
            self.canvas.view.cx = cx - d.x as f64 / self.canvas.view.scale;
            self.canvas.view.cy = cy + d.y as f64 / self.canvas.view.scale;
        }
        if let Some(p) = moved_to.filter(|p| rect.contains(*p) || self.primary_down) {
            let te = self.tool_event(p, modifiers);
            self.with_ctx(painter, |t, c| t.moved(c, &te));
            let hover_curves = self.tools[self.tool].0.hover_curves();
            self.canvas.hover_id =
                if hover_curves && !self.primary_down { self.canvas.pick_entity(painter, &self.model.doc, te.screen, te.world) } else { None };
        } else if hover.is_none_or(|p| !rect.contains(p)) && !self.primary_down {
            self.canvas.hover_id = None;
        }
        // keys go to the canvas unless another widget (a text field) has the keyboard
        let other_focus = ctx.memory(|m| m.focused()).is_some_and(|id| id != resp.id);
        if other_focus || self.menu_at.is_some() {
            return;
        }
        let mut refresh = false;
        // egui-winit turns Ctrl+X/C/V into Cut/Copy/Paste events and sends no Key event, so
        // rebuild the key press for shortcuts (unless a Key event for the same press arrived)
        let mut events = events;
        let clipboard: Vec<Key> = events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Cut => Some(Key::X),
                egui::Event::Copy => Some(Key::C),
                egui::Event::Paste(_) => Some(Key::V),
                _ => None,
            })
            .collect();
        for key in clipboard {
            let has_key = events.iter().any(|e| matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key));
            if !has_key {
                let m = egui::Modifiers { command: true, ctrl: modifiers.ctrl, mac_cmd: modifiers.mac_cmd, ..modifiers };
                events.push(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: m });
            }
        }
        for ev in &events {
            match ev {
                egui::Event::Key { key, pressed, repeat, modifiers, .. } => {
                    let hold = *key == self.hold_keys.0 || *key == self.hold_keys.1;
                    if hold && !modifiers.command {
                        let claimed = self.tools[self.tool].0.claims_hold(*key);
                        if !claimed {
                            if *pressed && !*repeat {
                                self.held.insert(*key);
                                refresh = true;
                            } else if !*pressed {
                                self.held.remove(key);
                                refresh = true;
                            }
                            continue;
                        }
                    }
                    if !*pressed {
                        continue;
                    }
                    // the active tool may claim keys that are also global shortcuts
                    if !modifiers.command && self.tools[self.tool].0.claims_key(*key) {
                        let m = *modifiers;
                        let k = *key;
                        if self.with_ctx(painter, |t, c| t.key(c, k, m)) {
                            continue;
                        }
                    }
                    match key {
                        Key::Escape => {
                            let cancelled = self.with_ctx(painter, |t, c| t.cancel(c));
                            if !cancelled {
                                self.set_tool(ctx, "select");
                            }
                            continue;
                        }
                        Key::Enter => {
                            self.with_ctx(painter, |t, c| t.finish(c));
                            continue;
                        }
                        Key::Backspace => {
                            self.model.delete_selection();
                            continue;
                        }
                        _ => {}
                    }
                    let id = self.shortcuts.iter().find(|(s, _)| s.matches(*key, modifiers)).map(|(_, id)| id.clone());
                    if let Some(id) = id {
                        self.run_action(ctx, &id);
                    }
                }
                egui::Event::Text(t) => {
                    if t.chars().all(|c| "0123456789.-+@<,".contains(c)) && !modifiers.command && self.tools[self.tool].0.takes_input() {
                        self.input_text.push_str(t);
                        self.focus_input = true;
                        let text = self.input_text.clone();
                        self.with_ctx(painter, |tool, c| tool.preview_text(c, &text));
                    }
                }
                egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_) => {}
                _ => {}
            }
        }
        if refresh {
            if let Some(p) = self.canvas.last_screen {
                let te = self.tool_event(p, modifiers);
                self.with_ctx(painter, |t, c| t.moved(c, &te));
            }
        }
        if !ctx.input(|i| i.focused) {
            self.held.clear(); // never leave a constraint stuck on
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame(ctx);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.run_script(ctx, raw);
    }
}

impl App {
    fn run_script(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        use crate::script::{key_events, press, Cmd};
        if std::env::var("EMESH_DEBUG").is_ok() {
            for e in &raw.events {
                if !matches!(e, egui::Event::PointerMoved(_)) {
                    eprintln!("raw {e:?}");
                }
            }
        }
        raw.events.append(&mut self.script_events);
        if self.script.is_empty() && self.script_wait == 0 {
            return;
        }
        ctx.request_repaint();
        if self.script_wait > 0 {
            self.script_wait -= 1;
            return;
        }
        let s = |app: &App, x: f64, y: f64| app.canvas.view.to_screen([x, y]);
        while let Some(cmd) = self.script.pop_front() {
            match cmd {
                Cmd::Wait(n) => {
                    self.script_wait = n;
                    return;
                }
                Cmd::Tool(t) => self.set_tool(ctx, &t),
                Cmd::Move(x, y) => {
                    raw.events.push(egui::Event::PointerMoved(s(self, x, y)));
                    self.script_wait = 1;
                    return;
                }
                Cmd::Click(x, y, b, double) => {
                    let p = s(self, x, y);
                    raw.events.push(egui::Event::PointerMoved(p));
                    let mut steps = vec![Cmd::Wait(1)];
                    let reps = if double { 2 } else { 1 };
                    for _ in 0..reps {
                        steps.push(Cmd::Press(x, y, b, true));
                        steps.push(Cmd::Wait(1));
                        steps.push(Cmd::Press(x, y, b, false));
                        steps.push(Cmd::Wait(1));
                    }
                    for c in steps.into_iter().rev() {
                        self.script.push_front(c);
                    }
                    if !double {
                        self.last_click = None;
                    }
                    return;
                }
                Cmd::Press(x, y, b, down) => {
                    raw.events.push(press(s(self, x, y), b, down));
                }
                Cmd::ClickPx(x, y, b, double) => {
                    let p = Pos2::new(x, y);
                    raw.events.push(egui::Event::PointerMoved(p));
                    for _ in 0..if double { 2 } else { 1 } {
                        raw.events.push(press(p, b, true));
                        raw.events.push(press(p, b, false));
                    }
                    self.script_wait = 1;
                    return;
                }
                Cmd::Drag(x0, y0, x1, y1) => {
                    let a = s(self, x0, y0);
                    raw.events.push(egui::Event::PointerMoved(a));
                    raw.events.push(press(a, egui::PointerButton::Primary, true));
                    let mut steps = vec![];
                    for k in 1..=6 {
                        let t = k as f64 / 6.0;
                        steps.push(Cmd::Move(x0 + t * (x1 - x0), y0 + t * (y1 - y0)));
                        steps.push(Cmd::Wait(1));
                    }
                    steps.push(Cmd::Release(x1, y1));
                    steps.push(Cmd::Wait(2));
                    for c in steps.into_iter().rev() {
                        self.script.push_front(c);
                    }
                    self.script_wait = 1;
                    return;
                }
                Cmd::Release(x, y) => {
                    raw.events.push(press(s(self, x, y), egui::PointerButton::Primary, false));
                    self.last_click = None;
                }
                Cmd::Key(k) => {
                    raw.events.extend(key_events(&k));
                    raw.modifiers = crate::script::mods(&k);
                    self.script_wait = 1;
                    return;
                }
                Cmd::Type(t) => {
                    raw.events.push(egui::Event::Text(t));
                    self.script_wait = 1;
                    return;
                }
                Cmd::Open(p) => {
                    self.open_path(Path::new(&p));
                    self.script_wait = 3;
                    return;
                }
                Cmd::Action(a) => {
                    self.run_action(ctx, &a);
                    self.script_wait = 1;
                    return;
                }
                Cmd::Shot(p) => {
                    self.script_shot = Some(PathBuf::from(p));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                    self.script_wait = 3;
                    return;
                }
                Cmd::Quit => {
                    self.quit_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
        }
    }
}

impl App {
    /// One frame of the whole window.
    pub fn frame(&mut self, ctx: &egui::Context) {
        theme::sync(ctx); // the OS may have switched (theme "system")
        self.pump_mesher();
        if ctx.input(|i| i.viewport().close_requested()) && !self.quit_confirmed && !self.model.is_clean() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.dialog.is_none() {
                self.dialog = Some(Dialog::Confirm(Pending::Quit));
            }
        }
        if self.quit_confirmed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // modal-safe global shortcuts when the canvas is not focused (e.g. a toolbar button was clicked)
        // menu bar and toolbar share one panel: one header, no seam between them
        egui::TopBottomPanel::top("menu").frame(egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin { left: 6, right: 6, top: 2, bottom: 3 })).show(ctx, |ui| {
            self.menu_bar(ctx, ui);
            ui.add_space(1.0);
            self.toolbar(ctx, ui);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        egui::TopBottomPanel::bottom("input").show(ctx, |ui| self.input_bar(ctx, ui));
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(col::background())).show(ctx, |ui| {
            self.canvas_ui(ctx, ui);
            toolbar_shadow(ui);
        });
        // the quality banner floats over the top of the canvas, so the drawing never jumps under the cursor
        if self.banner.as_ref().is_some_and(|b| b.dismissed.as_deref() != Some(b.text.as_str())) {
            let r = self.canvas.view.rect;
            egui::Area::new(egui::Id::new("banner"))
                .order(egui::Order::Middle)
                .fixed_pos(r.left_top())
                .show(ctx, |ui| {
                    ui.set_width(r.width());
                    self.banner_ui(ctx, ui)
                });
        }
        // windows owned by the tool (offset dialog)
        {
            let painter = ctx.layer_painter(self.tool_painter_layer);
            let (tool, _) = &mut self.tools[self.tool];
            let mut c = Ctx { model: &mut self.model, canvas: &mut self.canvas, painter: &painter, events: &mut self.events, settings: &mut self.settings };
            tool.windows(ctx, &mut c);
        }
        self.show_dialog(ctx);
        self.handle_events(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        self.screenshots(ctx);
    }
}

impl App {
    fn screenshots(&mut self, ctx: &egui::Context) {
        if let Some(p) = self.script_shot.clone() {
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                let _ = crate::png::write_rgba(&p, img.width(), img.height(), &img.pixels);
                self.script_shot = None;
            }
        }
        let Some((path, frames)) = self.screenshot.clone() else { return };
        if frames > 0 {
            self.screenshot = Some((path, frames - 1));
            if frames == 1 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            ctx.request_repaint();
            return;
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = shot {
            let _ = crate::png::write_rgba(&path, img.width(), img.height(), &img.pixels);
            self.screenshot = None;
            if std::env::var("EMESH_SCREENSHOT_KEEP").is_err() {
                self.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else {
            ctx.request_repaint();
        }
    }

    /// Forget the last click (tests: the next click is never a double-click).
    pub fn forget_click(&mut self) {
        self.last_click = None;
    }

    pub fn run_action_pub(&mut self, ctx: &egui::Context, id: &str) {
        self.run_action(ctx, id);
    }

    pub fn tool_name(&self) -> &'static str {
        self.tools[self.tool].0.name()
    }

    pub fn has_dialog(&self) -> bool {
        self.dialog.is_some() || self.tools[self.tool].0.modal()
    }

    pub fn menu_open(&self) -> bool {
        self.menu_at.is_some()
    }

    pub fn input_text(&self) -> &str {
        &self.input_text
    }

    pub fn status_text(&self) -> String {
        self.status.as_ref().map(|s| s.0.clone()).unwrap_or_default()
    }

    pub fn quality(&self) -> Option<&MeshQuality> {
        self.quality.as_ref()
    }

    /// For scripted screenshots and tests: run a closure on the app.
    pub fn script(&mut self, ctx: &egui::Context, f: impl FnOnce(&mut App, &egui::Context)) {
        f(self, ctx);
    }
}

/// A menu item that is on: a rounded highlight, no outline.
fn checked(b: egui::Button<'_>, on: bool) -> egui::Button<'_> {
    b.selected(on).stroke(Stroke::NONE).corner_radius(4)
}

/// A soft shadow along the top of the canvas, under the toolbar.
fn toolbar_shadow(ui: &egui::Ui) {
    let r = ui.max_rect();
    let a = theme::palette().shadow.a() as f32;
    for k in 0..6 {
        let f = 1.0 - k as f32 / 6.0;
        let c = Color32::from_black_alpha((a * f * f).round() as u8);
        let y = r.top() + k as f32 + 0.5;
        ui.painter().line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], Stroke::new(1.0f32, c));
    }
}

#[allow(dead_code)]
fn unused(_: Color32, _: Mesh) {
    let _ = alpha;
}
