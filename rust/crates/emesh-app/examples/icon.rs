//! Write the app icon as PNG files (packaging): cargo run --example icon -- OUT_DIR
fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir).unwrap();
    for size in [16usize, 32, 64, 128, 256, 512] {
        let rgba = emesh_app::icons::app_icon_rgba(size);
        let px: Vec<egui::Color32> = rgba.chunks(4).map(|c| egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])).collect();
        emesh_app::png::write_rgba(&dir.join(format!("eilmer-mesher-{size}.png")), size, size, &px).unwrap();
    }
}
