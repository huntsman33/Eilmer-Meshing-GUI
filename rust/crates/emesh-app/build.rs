//! Version stamp: the git commit the app was built from (when built in a git checkout).
fn main() {
    let commit = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty", "--tags"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=EMESH_COMMIT={commit}");
    println!("cargo:rerun-if-changed=../../../.git/HEAD");
}
