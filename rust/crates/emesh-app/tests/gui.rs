//! The whole window driven headlessly with scripted pointer and key events.

use egui::{Event, Key, Modifiers, Vec2};

use emesh_core::entities::Kind;
use emesh_core::P;

use emesh_app::headless::Headless as H;

fn settings_file() {
    let dir = std::env::temp_dir().join(format!("emesh-gui-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("EMESH_SETTINGS", dir.join("settings.json"));
    std::fs::write(dir.join("settings.json"), r#"{"lmr_warned": true}"#).unwrap();
    std::env::set_var("LMR", "/nonexistent/lmr"); // the preview is enough here
}

fn new() -> H {
    settings_file();
    H::new(Vec2::new(1280.0, 820.0))
}

trait Extra {
    fn rectangle(&mut self, a: P, b: P);
}

impl Extra for H {
    fn rectangle(&mut self, a: P, b: P) {
        self.tool("rectangle");
        self.click(a);
        self.click(b);
    }
}

#[test]
fn draw_rectangle_undo_redo() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [1.0, 0.5]);
    assert_eq!(h.app.model.doc.curves.len(), 4);
    h.key(Key::Z, Modifiers::COMMAND);
    assert_eq!(h.app.model.doc.curves.len(), 0);
    h.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(h.app.model.doc.curves.len(), 4);
}

#[test]
fn line_with_typed_length() {
    let mut h = new();
    h.key(Key::L, Modifiers::NONE);
    assert_eq!(h.app.tool_name(), "line");
    h.click([0.0, 0.0]);
    let q = h.s([1.0, 0.0]);
    h.frame(vec![Event::PointerMoved(q)]);
    h.type_text("0.75");
    assert_eq!(h.app.input_text(), "0.75");
    h.key(Key::Enter, Modifiers::NONE);
    h.settle();
    let c = h.app.model.doc.curves.values().next().expect("a line");
    let (p0, p1) = c.ends_lp();
    assert!(((p1[0] - p0[0]).hypot(p1[1] - p0[1]) - 0.75).abs() < 1e-9, "{p0:?} {p1:?}");
}

#[test]
fn block_select_toggle_split_and_hide() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [2.0, 1.0]);
    h.tool("block");
    h.click([1.0, 0.5]);
    h.settle();
    assert_eq!(h.app.model.doc.blocks.len(), 1);
    let res = h.app.model.resolution();
    assert!(res.errors.is_empty());
    // the preview mesh arrives from the worker thread
    assert!(h.wait_for_mesh(), "no preview mesh");
    // H hides the block under the cursor, Shift+H toggles all
    h.frame(vec![Event::PointerMoved(h.s([1.0, 0.5]))]);
    h.key(Key::H, Modifiers::NONE);
    assert_eq!(h.app.canvas.hidden_blocks.len(), 1);
    h.key(Key::H, Modifiers::SHIFT);
    assert!(h.app.canvas.hidden_ids(&h.app.model.resolution()).is_empty());
    // select the bottom edge, X makes it a split line (the block is then not closed by solid curves)
    h.tool("select");
    h.click([1.0, 0.0]);
    assert_eq!(h.app.model.selection.len(), 1);
    h.key(Key::X, Modifiers::NONE);
    let cid = h.app.model.selection.iter().next().unwrap().clone();
    assert_eq!(h.app.model.doc.curves[&cid].kind, Kind::Split);
    h.key(Key::X, Modifiers::SHIFT);
    assert_eq!(h.app.model.doc.curves[&cid].kind, Kind::Reference);
    h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(h.app.model.doc.curves.len(), 3);
}

#[test]
fn box_select_and_context_menu() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [1.0, 1.0]);
    h.tool("select");
    h.drag([-0.2, -0.2], [1.2, 1.2]);
    assert_eq!(h.app.model.selection.len(), 4);
    h.right_click([0.5, 1.0]);
    assert!(h.app.menu_open());
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.app.menu_open());
}

#[test]
fn drag_handle_moves_shared_corner() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [1.0, 1.0]);
    h.tool("select");
    h.click([0.5, 0.0]); // the bottom edge
    h.drag([1.0, 0.0], [1.5, -0.25]);
    let ends: Vec<P> = h.app.model.doc.curves.values().flat_map(|c| [c.start(), c.end()]).collect();
    assert_eq!(ends.iter().filter(|p| (p[0] - 1.5).abs() < 1e-6 && (p[1] + 0.25).abs() < 1e-6).count(), 2, "{ends:?}");
    h.key(Key::Z, Modifiers::COMMAND);
    assert_eq!(h.app.model.doc.curves.values().filter(|c| c.start() == [1.5, -0.25] || c.end() == [1.5, -0.25]).count(), 0);
}

#[test]
fn dimension_tool_places_and_drives() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [2.0, 1.0]);
    h.key(Key::D, Modifiers::NONE);
    h.click([1.0, 0.0]);
    h.frame(vec![Event::PointerMoved(h.s([1.0, -0.3]))]);
    h.type_text("3");
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(h.app.model.doc.dimensions.len(), 1);
    let did = h.app.model.doc.dimensions.keys().next().unwrap().clone();
    assert!((h.app.model.dimension_value(&did).unwrap() - 3.0).abs() < 1e-9);
}

#[test]
fn offset_dialog_grows_a_square() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [1.0, 1.0]);
    h.tool("select");
    h.key(Key::A, Modifiers::COMMAND);
    h.key(Key::F, Modifiers::NONE);
    assert_eq!(h.app.tool_name(), "offset");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.app.has_dialog());
    h.key(Key::Enter, Modifiers::NONE); // OK
    h.settle();
    assert!(!h.app.has_dialog());
    assert_eq!(h.app.model.doc.curves.len(), 8);
}

#[test]
fn boundary_layer_tool() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [1.0, 0.2]);
    h.tool("block");
    h.click([0.5, 0.1]);
    h.key(Key::Y, Modifiers::NONE);
    assert_eq!(h.app.tool_name(), "layer");
    h.app.settings.layer.first = 1e-4;
    h.click([0.5, 0.0]);
    h.key(Key::Enter, Modifiers::NONE);
    h.settle();
    assert_eq!(h.app.model.doc.layers.len(), 1, "{}", h.app.status_text());
    assert_eq!(h.app.model.doc.blocks.len(), 2);
    assert!(h.app.model.doc.bcs.contains_key("wall"));
}

#[test]
fn escape_returns_to_select() {
    let mut h = new();
    h.key(Key::C, Modifiers::NONE);
    assert_eq!(h.app.tool_name(), "circle");
    h.key(Key::Escape, Modifiers::NONE);
    assert_eq!(h.app.tool_name(), "select");
}

#[test]
fn double_click_edge_opens_edge_cells() {
    let mut h = new();
    h.rectangle([0.0, 0.0], [2.0, 1.0]);
    h.tool("block");
    h.click([1.0, 0.5]);
    h.settle();
    h.double_click([1.0, 0.0]);
    h.settle();
    assert!(h.app.has_dialog(), "status: {}", h.app.status_text());
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.app.has_dialog());
}

#[test]
fn double_click_wall_of_opened_project() {
    let mut h = new();
    let p = emesh_core::golden::dir().join("corpus/layer_flat_plate.emesh.json");
    h.app.open_path(&p);
    h.settle();
    h.tool("block");
    let before = h.app.status_text();
    h.double_click([0.5, 0.0]);
    h.settle();
    assert!(h.app.has_dialog(), "before: {before} / after: {} view {:?}", h.app.status_text(), h.app.canvas.view);
}

#[test]
fn export_through_the_window() {
    let lmr = std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join("gdtkinst/bin/lmr")).ok().filter(|p| p.exists());
    let Some(lmr) = lmr else {
        eprintln!("lmr not installed: skipped");
        return;
    };
    let mut h = new();
    std::env::set_var("LMR", &lmr);
    let p = emesh_core::golden::dir().join("corpus/channel_three.emesh.json");
    h.app.open_path(&p);
    h.settle();
    let out = std::env::temp_dir().join(format!("emesh-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    h.app.start_export(out.clone());
    for _ in 0..600 {
        h.frame(vec![]);
        std::thread::sleep(std::time::Duration::from_millis(10));
        if out.join("lmrsim/grid/grid.metadata").exists() && h.app.has_dialog() {
            break;
        }
    }
    h.settle();
    for f in ["mesh.lua", "bcs.lua", "sim.lua", "ideal-air.lua"] {
        assert!(out.join(f).exists(), "{f} missing");
    }
    let meta = std::fs::read_to_string(out.join("lmrsim/grid/grid.metadata")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(v["grid-connections"].as_array().unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(&out);
}

fn count_types(h: &H) -> (usize, usize, usize) {
    let doc = &h.app.model.doc;
    let lines = doc.curves.values().filter(|c| c.is_line()).count();
    let arcs = doc.curves.values().filter(|c| c.is_arc()).count();
    (lines, arcs, doc.curves.len() - lines - arcs)
}

#[test]
fn arc_tool_centre_mode_and_three_point() {
    let mut h = new();
    h.key(Key::A, Modifiers::NONE);
    assert_eq!(h.app.tool_name(), "arc");
    h.click([0.0, 0.0]); // centre
    h.click([1.0, 0.0]); // start
    // sweep round past 180 degrees: move through the top to the bottom-left
    for p in [[0.7, 0.7], [0.0, 1.0], [-0.7, 0.7], [-1.0, 0.0], [-0.7, -0.7]] {
        h.move_to(p);
    }
    h.click([-0.7, -0.7]);
    let (_, arcs, _) = count_types(&h);
    assert_eq!(arcs, 2, "a 225 degree arc is two pieces");
    h.key(Key::M, Modifiers::NONE); // 3-point mode (the tool claims M)
    assert_eq!(h.app.tool_name(), "arc");
    h.click([1.2, 0.0]);
    h.click([2.2, 0.0]);
    h.click([1.7, 0.4]);
    assert_eq!(count_types(&h).1, 3);
}

#[test]
fn profile_with_tangent_arc_closes() {
    let mut h = new();
    h.key(Key::P, Modifiers::NONE);
    h.click([0.0, 0.0]);
    h.click([1.0, 0.0]);
    h.key(Key::A, Modifiers::NONE); // tangent arc
    h.click([1.0, 1.0]);
    h.click([0.0, 1.0]);
    h.click([0.0, 0.0]); // closes the loop: the tool resets
    let (lines, arcs, _) = count_types(&h);
    assert_eq!((lines, arcs), (3, 2), "a tangent semicircle is two Eilmer arcs");
    h.tool("block");
    h.click([0.5, 0.5]);
    assert_eq!(h.app.model.doc.blocks.len(), 1, "{}", h.app.status_text());
}

#[test]
fn spline_and_merge_and_trim() {
    let mut h = new();
    h.key(Key::N, Modifiers::NONE);
    for p in [[0.0, 0.0], [1.0, 0.5], [2.0, 0.0]] {
        h.click(p);
    }
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(count_types(&h).2, 1);
    // two collinear lines merge into one
    h.app.canvas.view.scale = 300.0;
    h.app.canvas.view.cx = 1.0;
    h.app.canvas.view.cy = 1.5;
    h.tool("line");
    h.click([0.0, 2.0]);
    h.click([1.0, 2.0]);
    h.click([1.0, 2.0]);
    h.click([2.0, 2.0]);
    h.key(Key::J, Modifiers::NONE);
    h.click([0.5, 2.0]);
    h.click([1.5, 2.0]);
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(count_types(&h).0, 1, "{}", h.app.status_text());
    // a crossing line, then quick-trim the merged line's left part
    h.tool("line");
    h.click([1.0, 1.6]);
    h.click([1.0, 2.2]);
    h.key(Key::Q, Modifiers::NONE);
    h.click([0.4, 2.0]);
    let doc = &h.app.model.doc;
    let horizontal: Vec<_> = doc.curves.values().filter(|c| c.is_line() && c.start()[1] == 2.0 && c.end()[1] == 2.0).collect();
    assert_eq!(horizontal.len(), 1);
    let (a, b) = horizontal[0].ends_lp();
    assert!((a[0] - 1.0).abs() < 1e-9 && (b[0] - 2.0).abs() < 1e-9, "{a:?} {b:?}");
}

#[test]
fn mirror_about_the_y_axis_and_move() {
    let mut h = new();
    h.rectangle([0.5, 0.0], [1.5, 1.0]);
    h.tool("select");
    h.key(Key::A, Modifiers::COMMAND);
    h.key(Key::M, Modifiers::SHIFT);
    assert_eq!(h.app.tool_name(), "mirror");
    h.key(Key::Enter, Modifiers::NONE);
    // click the y axis (x = 0) away from any curve
    h.click([0.0, 0.9]);
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(h.app.model.doc.curves.len(), 8, "{}", h.app.status_text());
    assert!(h.app.model.doc.curves.values().any(|c| c.start()[0] < -1.0 || c.end()[0] < -1.0));
    // move the new (selected) copies by typing dx, dy
    h.key(Key::M, Modifiers::NONE);
    assert_eq!(h.app.tool_name(), "move");
    h.type_text("0,1");
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.app.model.doc.curves.values().any(|c| (c.start()[1] - 2.0).abs() < 1e-9 && c.start()[0] < 0.0));
}

#[test]
fn subdivide_ogrid_from_menu_api() {
    let mut h = new();
    h.tool("circle");
    h.click([0.0, 0.0]);
    h.type_text("1");
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(count_types(&h).1, 4);
    let made = h.app.model.subdivide([0.1, 0.1], emesh_app::model::Subdivision::OGrid(0.5)).unwrap();
    assert_eq!(made.len(), 5);
    h.settle();
    assert!(h.wait_for_mesh());
    let q = h.app.quality().expect("quality");
    assert_eq!(q.blocks.len(), 5);
}


#[test]
fn clipboard_shortcuts_arrive_as_cut_events() {
    // egui-winit sends Ctrl+X as Event::Cut with no Key event
    let mut h = new();
    assert!(!h.app.model.split_mode);
    h.frame_mods(vec![egui::Event::Cut], Modifiers::COMMAND);
    assert!(h.app.model.split_mode);
    h.frame_mods(vec![egui::Event::Cut], Modifiers::COMMAND);
    assert!(!h.app.model.split_mode);
    assert!(!h.app.model.reference_mode);
    h.frame_mods(vec![egui::Event::Cut], Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(h.app.model.reference_mode);
    h.frame_mods(vec![egui::Event::Cut], Modifiers::COMMAND | Modifiers::SHIFT);
    assert!(!h.app.model.reference_mode);
}

#[test]
fn cut_with_key_event_does_not_double_fire() {
    let mut h = new();
    let key = egui::Event::Key { key: Key::X, physical_key: Some(Key::X), pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    h.frame_mods(vec![key, egui::Event::Cut], Modifiers::COMMAND);
    assert!(h.app.model.split_mode);
}

#[test]
fn dark_mode_toggle_switches_theme_palette_and_setting() {
    use emesh_app::theme::{col, is_dark, DARK, LIGHT};
    let mut h = new();
    assert_eq!(h.app.settings.theme, "light");
    assert!(!is_dark());
    assert_eq!(col::background(), LIGHT.background);
    h.action("view.dark");
    assert_eq!(h.app.settings.theme, "dark");
    assert!(is_dark());
    assert_eq!(h.ctx.theme(), egui::Theme::Dark);
    assert_eq!(col::background(), DARK.background);
    assert_eq!(col::geometry(), DARK.geometry);
    assert_eq!(h.ctx.style().visuals.panel_fill, emesh_app::theme::dark_visuals().panel_fill);
    h.action("view.theme.system"); // no OS preference headless: light
    assert_eq!(h.app.settings.theme, "system");
    assert!(!is_dark());
    h.action("view.dark");
    assert!(is_dark());
    h.action("view.dark");
    assert_eq!(h.app.settings.theme, "light");
    assert_eq!(col::background(), LIGHT.background);
}
