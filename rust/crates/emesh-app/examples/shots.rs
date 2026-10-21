//! Headless screenshots from a script: cargo run --example shots -- SCRIPT [WIDTH HEIGHT]
//! (see src/script.rs for the commands; no window is opened).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: shots SCRIPT [WIDTH HEIGHT]");
        std::process::exit(2);
    };
    let w: f32 = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(1280.0);
    let h: f32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(820.0);
    let text = std::fs::read_to_string(path).expect("script");
    let cmds = emesh_app::script::parse(&text).unwrap_or_else(|e| panic!("{e}"));
    let mut hl = emesh_app::headless::Headless::new(egui::vec2(w, h));
    hl.run(cmds).expect("run");
}
