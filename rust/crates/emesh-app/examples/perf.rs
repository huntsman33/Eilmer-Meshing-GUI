//! Frame time with the pointer moving over a project: cargo run --release --example perf -- PROJECT
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("project");
    let mut h = emesh_app::headless::Headless::new(egui::vec2(1600.0, 1000.0));
    let t = Instant::now();
    h.app.open_path(std::path::Path::new(&path));
    h.settle();
    println!("open + first frames: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let ok = h.wait_for_mesh();
    println!("preview mesh ready: {ok} after {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let r = h.app.canvas.view.rect;
    let n = 200;
    let t = Instant::now();
    for k in 0..n {
        let f = k as f32 / n as f32;
        let p = r.min + (r.max - r.min) * egui::vec2(f, 0.5 + 0.3 * (f * 12.0).sin());
        h.frame(vec![egui::Event::PointerMoved(p)]);
    }
    let dt = t.elapsed().as_secs_f64() * 1e3 / n as f64;
    let cells = h.app.canvas.mesh.as_ref().map_or(0, |m| m.cells());
    println!("{n} frames with hover picking: {dt:.2} ms/frame ({cells} cells shown)");
    h.action("view.quality");
    let t = Instant::now();
    for _ in 0..50 {
        h.frame(vec![]);
    }
    println!("with the quality heat-map: {:.2} ms/frame", t.elapsed().as_secs_f64() * 1e3 / 50.0);
    // an edit: the model, topology and resolution are rebuilt
    let t = Instant::now();
    for _ in 0..20 {
        h.app.model.notify();
        h.frame(vec![]);
    }
    println!("frame after an edit (topology + blocks rebuilt): {:.2} ms", t.elapsed().as_secs_f64() * 1e3 / 20.0);
}
