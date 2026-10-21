//! Install the built-in cursor theme if needed and print the environment it sets.
fn main() {
    emesh_app::cursors::ensure();
    println!("{}", std::env::var("XCURSOR_PATH").unwrap_or_default());
    println!("{}", std::env::var("XCURSOR_THEME").unwrap_or_default());
}
