//! emesh [project.emesh.json] [--renderer wgpu|glow] [--screenshot out.png]: the app;
//! emesh export|lua|check|preview ...: the command-line tools (see emesh-cli).

// no console window behind the app on Windows
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;

fn usage() -> ! {
    eprintln!("{}\n\napp options: [--renderer auto|wgpu|glow] [--script FILE] [--screenshot out.png] [--version]", emesh_cli::USAGE);
    std::process::exit(2);
}

fn run(renderer: eframe::Renderer, project: Option<PathBuf>, shot: Option<PathBuf>, script: Option<String>) -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer,
        viewport: egui::ViewportBuilder::default()
            .with_title("Eilmer Mesher")
            .with_app_id("eilmer-mesher")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 420.0])
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: emesh_app::icons::app_icon_rgba(128), width: 128, height: 128 })),
        ..Default::default()
    };
    eframe::run_native(
        "Eilmer Mesher",
        options,
        Box::new(move |cc| {
            #[cfg(target_os = "linux")]
            emesh_app::cursors::set_window_cursor(&cc.egui_ctx);
            let mut app = emesh_app::app::App::new(&cc.egui_ctx, project);
            if let Some(p) = shot {
                app.screenshot = Some((p, 20));
            }
            if let Some(text) = &script {
                match emesh_app::script::parse(text) {
                    Ok(cmds) => app.script = cmds,
                    Err(e) => eprintln!("emesh: script: {e}"),
                }
            }
            Ok(Box::new(app))
        }),
    )
}

/// "0.7.0-pre.1 (abc1234)"
fn version() -> String {
    let commit = env!("EMESH_COMMIT");
    if commit.is_empty() {
        env!("CARGO_PKG_VERSION").to_string()
    } else {
        format!("{} ({commit})", env!("CARGO_PKG_VERSION"))
    }
}

/// On a panic, write a crash log next to the settings and say where it is.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = emesh_app::settings::settings_path().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(std::env::temp_dir);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = dir.join(format!("crash-{stamp}.log"));
        let bt = std::backtrace::Backtrace::force_capture();
        let text = format!("Eilmer Mesher {}\n{}\n\n{info}\n\n{bt}\n", version(), std::env::consts::OS);
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::write(&path, text).is_ok() {
            eprintln!("emesh crashed; details are in {}", path.display());
        }
        default(info);
    }));
}

/// Wayland draws the pointer from an installed cursor theme; with none (as under WSLg) the
/// pointer is invisible. Use X11 there instead, as the Python version does under WSL.
/// EMESH_WAYLAND=1 keeps Wayland.
#[cfg(target_os = "linux")]
fn choose_display_backend() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() || std::env::var_os("DISPLAY").is_none() || std::env::var_os("EMESH_WAYLAND").is_some() {
        return;
    }
    let wsl = std::fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"));
    if wsl || !emesh_app::cursors::system_has_cursors() {
        // SAFETY: single-threaded, before the windowing library starts
        std::env::remove_var("WAYLAND_DISPLAY");
    }
}

#[cfg(not(target_os = "linux"))]
fn choose_display_backend() {}

/// A Windows GUI program has no console: print to the terminal it was started from, if any.
#[cfg(windows)]
fn attach_console() {
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

fn main() {
    install_crash_log();
    let first = std::env::args().nth(1).unwrap_or_default();
    if emesh_cli::is_command(&first) || first == "--version" || first == "-V" || first == "--help" || first == "-h" {
        attach_console();
    }
    if emesh_cli::is_command(&first) {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Err(e) = emesh_cli::run(&args) {
            eprintln!("emesh: {e}");
            std::process::exit(1);
        }
        return;
    }
    choose_display_backend();
    #[cfg(target_os = "linux")]
    emesh_app::cursors::ensure();
    let mut project = None;
    let mut shot = None;
    let mut script: Option<String> = None;
    let mut renderer = std::env::var("EMESH_RENDERER").unwrap_or_else(|_| emesh_app::settings::Settings::load().renderer);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--version" | "-V" => {
                println!("emesh {}", version());
                return;
            }
            "--renderer" => renderer = args.next().unwrap_or_else(|| usage()),
            "--screenshot" => shot = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--script" => {
                let p = args.next().unwrap_or_else(|| usage());
                script = Some(std::fs::read_to_string(&p).unwrap_or_else(|e| {
                    eprintln!("emesh: {p}: {e}");
                    std::process::exit(2)
                }));
            }
            "-h" | "--help" => usage(),
            s if s.starts_with('-') => usage(),
            s => project = Some(PathBuf::from(s)),
        }
    }
    let result = match renderer.as_str() {
        "glow" => run(eframe::Renderer::Glow, project, shot, script),
        "wgpu" => run(eframe::Renderer::Wgpu, project, shot, script),
        _ => {
            // wgpu first; fall back to OpenGL where no GPU device is available (e.g. some VMs, WSLg)
            match run(eframe::Renderer::Wgpu, project.clone(), shot.clone(), script.clone()) {
                Err(e) => {
                    eprintln!("emesh: wgpu is not available ({e}); using OpenGL");
                    // remember, so later launches go straight to OpenGL
                    let mut s = emesh_app::settings::Settings::load();
                    if s.renderer == "auto" {
                        s.renderer = "glow".into();
                        s.save();
                    }
                    run(eframe::Renderer::Glow, project, shot, script)
                }
                ok => ok,
            }
        }
    };
    if let Err(e) = result {
        eprintln!("emesh: {e}");
        std::process::exit(1);
    }
}
