//! A built-in X cursor theme, for Linux systems without one (e.g. WSLg, minimal installs).
//!
//! winit loads cursors by name in the app's own process: on X11 through libXcursor
//! (`XcursorLibraryLoadCursor`, which honours XCURSOR_PATH / XCURSOR_THEME and falls back to the
//! core cursor font), on Wayland through wayland-cursor / the `xcursor` crate (same variables).
//! With no theme installed the app writes this theme to its cache folder and points
//! XCURSOR_PATH / XCURSOR_THEME at it. EMESH_BUILTIN_CURSORS=1 forces it.
//!
//! That alone is not enough on X11: winit never sets the window's cursor for the default arrow
//! (its window starts with "no cursor" and `set_cursor(Default)` is skipped as unchanged), so the
//! window shows its parent's cursor, i.e. the root window's. Under WSLg the root cursor belongs to
//! Weston's X window manager, which runs in the WSLg system distro with its own XCURSOR_PATH
//! (/mnt/wslg/distro/usr/share/icons:/usr/share/icons, see /mnt/wslg/weston.log), reads the
//! "default" theme once at startup, and leaves the root cursor invisible when that has no
//! left_ptr. Hence [`set_window_cursor`], which makes winit set the arrow on the window itself.

use std::path::{Path, PathBuf};

pub const THEME: &str = "emesh";
const VERSION: &str = "1";

type Poly = Vec<(f32, f32)>;

/// Shapes in a 24 x 24 design grid: filled black with a white outline; hotspot.
fn shapes(kind: &str) -> (Vec<Poly>, (f32, f32)) {
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let rot = |p: Poly, a: f32| -> Poly {
        let (c, s) = (a.cos(), a.sin());
        p.into_iter().map(|(x, y)| (12.0 + (x - 12.0) * c - (y - 12.0) * s, 12.0 + (x - 12.0) * s + (y - 12.0) * c)).collect()
    };
    let double_arrow = || -> Vec<Poly> {
        vec![rect(6.0, 11.0, 18.0, 13.0), vec![(2.5, 12.0), (7.5, 7.5), (7.5, 16.5)], vec![(21.5, 12.0), (16.5, 7.5), (16.5, 16.5)]]
    };
    match kind {
        "crosshair" => (vec![rect(11.0, 3.0, 13.0, 21.0), rect(3.0, 11.0, 21.0, 13.0)], (12.0, 12.0)),
        "text" => (vec![rect(11.25, 4.5, 12.75, 19.5), rect(8.5, 3.5, 15.5, 5.0), rect(8.5, 19.0, 15.5, 20.5)], (12.0, 12.0)),
        "move" => {
            let mut v = double_arrow();
            v.extend(double_arrow().into_iter().map(|p| rot(p, std::f32::consts::FRAC_PI_2)));
            (v, (12.0, 12.0))
        }
        "ew" => (double_arrow(), (12.0, 12.0)),
        "ns" => (double_arrow().into_iter().map(|p| rot(p, std::f32::consts::FRAC_PI_2)).collect(), (12.0, 12.0)),
        "nwse" => (double_arrow().into_iter().map(|p| rot(p, std::f32::consts::FRAC_PI_4)).collect(), (12.0, 12.0)),
        "nesw" => (double_arrow().into_iter().map(|p| rot(p, -std::f32::consts::FRAC_PI_4)).collect(), (12.0, 12.0)),
        _ => (vec![vec![(1.5, 1.5), (1.5, 18.0), (5.5, 14.2), (8.2, 20.2), (10.6, 19.2), (8.0, 13.3), (13.2, 13.3)]], (1.5, 1.5)),
    }
}

fn inside(polys: &[Poly], x: f32, y: f32) -> bool {
    polys.iter().any(|p| {
        let mut c = false;
        let n = p.len();
        for i in 0..n {
            let (a, b) = (p[i], p[(i + 1) % n]);
            if (a.1 > y) != (b.1 > y) && x < a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1) {
                c = !c;
            }
        }
        c
    })
}

/// ARGB (premultiplied) pixels of a cursor at `size` pixels, and its hotspot.
fn render(kind: &str, size: usize) -> (Vec<u32>, (u32, u32)) {
    let (polys, hot) = shapes(kind);
    let k = 4usize; // supersampling
    let n = size * k;
    let s = 24.0 / n as f32;
    let fill: Vec<bool> = (0..n * n).map(|i| inside(&polys, ((i % n) as f32 + 0.5) * s, ((i / n) as f32 + 0.5) * s)).collect();
    // the white outline: samples outside the shape within r of it
    let r = ((1.3 / 24.0) * n as f32).ceil() as isize;
    let mut outline = vec![false; n * n];
    for y in 0..n as isize {
        for x in 0..n as isize {
            if fill[(y as usize) * n + x as usize] {
                continue;
            }
            'search: for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let (u, v) = (x + dx, y + dy);
                    if u >= 0 && v >= 0 && (u as usize) < n && (v as usize) < n && fill[v as usize * n + u as usize] {
                        outline[y as usize * n + x as usize] = true;
                        break 'search;
                    }
                }
            }
        }
    }
    let mut px = vec![0u32; size * size];
    for py in 0..size {
        for pxx in 0..size {
            let (mut black, mut white) = (0u32, 0u32);
            for sy in 0..k {
                for sx in 0..k {
                    let i = (py * k + sy) * n + pxx * k + sx;
                    if fill[i] {
                        black += 1;
                    } else if outline[i] {
                        white += 1;
                    }
                }
            }
            let total = (k * k) as u32;
            let a = (black + white) * 255 / total;
            let w = white * 255 / total; // premultiplied white
            px[py * size + pxx] = (a << 24) | (w << 16) | (w << 8) | w;
        }
    }
    let f = size as f32 / 24.0;
    (px, ((hot.0 * f) as u32, (hot.1 * f) as u32))
}

/// An Xcursor file holding the cursor at several sizes.
fn xcursor(kind: &str) -> Vec<u8> {
    let sizes = [24usize, 32, 48];
    let images: Vec<(usize, Vec<u32>, (u32, u32))> = sizes.iter().map(|&n| {
        let (p, h) = render(kind, n);
        (n, p, h)
    }).collect();
    let mut out = vec![];
    let put = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(b"Xcur");
    put(&mut out, 16);
    put(&mut out, 0x10000);
    put(&mut out, images.len() as u32);
    let mut pos = 16 + 12 * images.len() as u32;
    for (n, _, _) in &images {
        put(&mut out, 0xfffd0002);
        put(&mut out, *n as u32);
        put(&mut out, pos);
        pos += 36 + 4 * (*n * *n) as u32;
    }
    for (n, pixels, hot) in &images {
        for v in [36, 0xfffd0002, *n as u32, 1, *n as u32, *n as u32, hot.0, hot.1, 0] {
            put(&mut out, v);
        }
        for p in pixels {
            put(&mut out, *p);
        }
    }
    out
}

/// Cursor file names (CSS names and the X11 aliases winit tries) for each shape.
const NAMES: [(&str, &[&str]); 7] = [
    ("arrow", &["default", "left_ptr", "arrow", "top_left_arrow", "left_arrow", "context-menu", "help", "progress", "left_ptr_watch",
        "wait", "watch", "pointer", "hand1", "hand2", "pointing_hand", "copy", "alias", "no-drop", "not-allowed", "crossed_circle", "cell",
        "zoom-in", "zoom-out", "dnd-none"]),
    ("crosshair", &["crosshair", "cross", "tcross"]),
    ("text", &["text", "xterm", "ibeam", "vertical-text"]),
    ("move", &["move", "grab", "grabbing", "openhand", "closedhand", "fleur", "all-scroll", "size_all"]),
    ("ew", &["ew-resize", "e-resize", "w-resize", "col-resize", "sb_h_double_arrow", "h_double_arrow", "size_hor", "split_h", "right_side", "left_side"]),
    ("ns", &["ns-resize", "n-resize", "s-resize", "row-resize", "sb_v_double_arrow", "v_double_arrow", "size_ver", "split_v", "top_side", "bottom_side"]),
    ("nwse", &["nwse-resize", "nw-resize", "se-resize", "size_fdiag", "bd_double_arrow", "top_left_corner", "bottom_right_corner"]),
];

const NESW: (&str, &[&str]) = ("nesw", &["nesw-resize", "ne-resize", "sw-resize", "size_bdiag", "fd_double_arrow", "top_right_corner", "bottom_left_corner"]);

/// Write the theme under `root` (root/emesh/cursors/...); returns root.
pub fn write_theme(root: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join(THEME).join("cursors");
    std::fs::create_dir_all(&dir)?;
    for (kind, names) in NAMES.iter().copied().chain(std::iter::once(NESW)) {
        let data = xcursor(kind);
        for name in names {
            std::fs::write(dir.join(name), &data)?;
        }
    }
    std::fs::write(root.join(THEME).join("index.theme"), "[Icon Theme]\nName=emesh\nComment=Eilmer Mesher built-in cursors\n")?;
    std::fs::write(root.join(THEME).join("version"), VERSION)?;
    Ok(root.to_path_buf())
}

/// The folders libXcursor searches by default (and any XCURSOR_PATH).
fn search_path() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::var("XCURSOR_PATH")
        .map(|p| p.split(':').filter(|s| !s.is_empty()).map(|s| PathBuf::from(s.replace('~', &home))).collect())
        .unwrap_or_default();
    for d in [format!("{home}/.local/share/icons"), format!("{home}/.icons"), "/usr/share/icons".into(), "/usr/share/pixmaps".into()] {
        dirs.push(PathBuf::from(d));
    }
    dirs
}

/// True if some theme on the search path has cursor files.
pub fn system_has_cursors() -> bool {
    search_path().iter().any(|d| {
        std::fs::read_dir(d).is_ok_and(|rd| {
            rd.filter_map(|e| e.ok()).any(|e| std::fs::read_dir(e.path().join("cursors")).is_ok_and(|mut c| c.next().is_some()))
        })
    })
}

/// When the system has no cursor theme (or EMESH_BUILTIN_CURSORS=1), install the built-in one and
/// point the cursor library at it. Call before the windowing library starts.
pub fn ensure() {
    let forced = std::env::var("EMESH_BUILTIN_CURSORS").is_ok_and(|v| !v.is_empty() && v != "0");
    if !forced && system_has_cursors() {
        return;
    }
    let root = directories::ProjectDirs::from("", "", "eilmer-mesher")
        .map(|d| d.cache_dir().join("cursors"))
        .unwrap_or_else(|| std::env::temp_dir().join("eilmer-mesher-cursors"));
    let current = std::fs::read_to_string(root.join(THEME).join("version")).ok();
    if current.as_deref() != Some(VERSION) && write_theme(&root).is_err() {
        return;
    }
    let mut path = root.display().to_string();
    if let Ok(old) = std::env::var("XCURSOR_PATH") {
        path = format!("{path}:{old}");
    } else {
        path += ":~/.local/share/icons:~/.icons:/usr/share/icons:/usr/share/pixmaps";
    }
    std::env::set_var("XCURSOR_PATH", path);
    std::env::set_var("XCURSOR_THEME", THEME);
}

/// Make winit give the window an explicit arrow cursor instead of inheriting the root window's
/// (invisible under WSLg without a cursor theme in the distro; see the module notes). Hiding and
/// showing the cursor makes winit's X11 backend load "default" through libXcursor and set it on
/// the window; elsewhere it is a no-op. Call once, e.g. from the app creator.
pub fn set_window_cursor(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
}
