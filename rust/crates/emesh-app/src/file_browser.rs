//! The in-app file dialog (native and portal dialogs hang under WSLg and are missing on minimal
//! Linux desktops). Modelled on Qt's own non-native QFileDialog, which the Python version used:
//! a "Look in" row with Back / Forward / Up / New Folder, a sidebar of places, a sortable detail
//! list (Name | Size | Type | Date Modified) and "File name" / "Files of type" rows.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextWrapping};
use egui::{Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};

use crate::dialogs::modal;
use crate::settings::Settings;
use crate::theme::col;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileMode {
    Open,
    Save,
    Folder,
}

// --- non-UI logic ----------------------------------------------------------------------------------

/// One row of the listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// seconds since the Unix epoch
    pub modified: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Type,
    Date,
}

/// A "Files of type" choice; no extensions means all files.
#[derive(Clone, Debug, PartialEq)]
pub struct FileType {
    pub label: String,
    pub exts: Vec<String>,
}

impl FileType {
    pub fn new(name: &str, exts: &[&str]) -> FileType {
        let pats: Vec<String> = exts.iter().map(|e| format!("*.{e}")).collect();
        FileType { label: format!("{name} ({})", pats.join(" ")), exts: exts.iter().map(|e| e.to_lowercase()).collect() }
    }

    pub fn all() -> FileType {
        FileType { label: "All files (*)".into(), exts: vec![] }
    }
}

/// The "Files of type" label for the extensions the app asks for (as the Python version named them).
fn type_label(exts: &[&str]) -> &'static str {
    match exts {
        ["emesh.json"] => "Eilmer Mesher project",
        e if e.iter().all(|x| ["dat", "txt", "csv", "xy"].contains(x)) => "Point files",
        _ => "Supported files",
    }
}

/// Does a file name pass the extension filter? (Case-insensitive; multi-part suffixes such as
/// "emesh.json" allowed; an empty filter passes everything.)
pub fn matches(name: &str, exts: &[String]) -> bool {
    let n = name.to_lowercase();
    exts.is_empty() || exts.iter().any(|e| n.len() > e.len() + 1 && n.ends_with(&format!(".{e}")))
}

pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Read a folder: sub-folders always, files when `!dirs_only` and they pass the filter.
pub fn list_dir(dir: &Path, exts: &[String], dirs_only: bool, show_hidden: bool) -> std::io::Result<Vec<Entry>> {
    let mut out = vec![];
    for e in std::fs::read_dir(dir)? {
        let Ok(e) = e else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        if !show_hidden && is_hidden(&name) {
            continue;
        }
        let mut meta = e.metadata().ok();
        if meta.as_ref().is_some_and(|m| m.file_type().is_symlink()) {
            if let Ok(m) = std::fs::metadata(e.path()) {
                meta = Some(m); // follow the link (a broken link stays a file)
            }
        }
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        if !is_dir && (dirs_only || !matches(&name, exts)) {
            continue;
        }
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = meta.and_then(|m| m.modified().ok()).map(|t| match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        });
        out.push(Entry { name, is_dir, size, modified });
    }
    Ok(out)
}

/// Case-insensitive "natural" order: "run2" before "run10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(|c| c.is_ascii_digit()) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (m, n) = (take(&mut x), take(&mut y));
                let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let o = m.len().cmp(&n.len()).then_with(|| m.cmp(n));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(c), Some(d)) => {
                let o = c.to_lowercase().cmp(d.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

/// The Type column: "Folder", "Eilmer Mesher project", "DAT file", "File".
pub fn type_name(e: &Entry) -> String {
    if e.is_dir {
        return "Folder".into();
    }
    let n = e.name.to_lowercase();
    if n.ends_with(".emesh.json") {
        return "Mesher project".into();
    }
    match n.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => format!("{} file", ext.to_uppercase()),
        _ => "File".into(),
    }
}

/// Folders first, then by the key (ties broken by name).
pub fn sort_entries(v: &mut [Entry], key: SortKey, ascending: bool) {
    v.sort_by(|a, b| {
        let by_name = || natural_cmp(&a.name, &b.name);
        let o = match key {
            SortKey::Name => by_name(),
            SortKey::Size => a.size.cmp(&b.size).then_with(by_name),
            SortKey::Type => natural_cmp(&type_name(a), &type_name(b)).then_with(by_name),
            SortKey::Date => a.modified.cmp(&b.modified).then_with(by_name),
        };
        b.is_dir.cmp(&a.is_dir).then(if ascending { o } else { o.reverse() })
    });
}

/// "New Folder", "New Folder 2", ... : the first name not taken in `dir`.
pub fn unique_name(dir: &Path, base: &str) -> String {
    let taken = |n: &str| std::fs::symlink_metadata(dir.join(n)).is_ok();
    if !taken(base) {
        return base.into();
    }
    (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).unwrap()
}

/// Create a uniquely named "New Folder" in `dir`; returns its name.
pub fn create_new_folder(dir: &Path) -> Result<String, String> {
    for _ in 0..100 {
        let name = unique_name(dir, "New Folder");
        match std::fs::create_dir(dir.join(&name)) {
            Ok(()) => return Ok(name),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not create a folder in {}: {e}", dir.display())),
        }
    }
    Err("Could not find a free folder name".into())
}

/// Rename `old` to `new` inside `dir`, refusing to clobber anything.
pub fn rename_entry(dir: &Path, old: &str, new: &str) -> Result<(), String> {
    let new = new.trim();
    if new == old {
        return Ok(());
    }
    if new.is_empty() || new == "." || new == ".." || new.contains('/') || (cfg!(windows) && new.contains('\\')) {
        return Err(format!("“{new}” is not a valid name"));
    }
    let to = dir.join(new);
    if std::fs::symlink_metadata(&to).is_ok() {
        return Err(format!("“{new}” already exists"));
    }
    std::fs::rename(dir.join(old), &to).map_err(|e| format!("Could not rename “{old}”: {e}"))
}

/// Back / forward navigation history.
#[derive(Clone, Debug, Default)]
pub struct History {
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl History {
    /// Leaving `from` for somewhere new.
    pub fn visit(&mut self, from: &Path) {
        if self.back.last().map(|p| p.as_path()) != Some(from) {
            self.back.push(from.to_path_buf());
        }
        self.forward.clear();
    }

    pub fn back(&mut self, current: &Path) -> Option<PathBuf> {
        let p = self.back.pop()?;
        self.forward.push(current.to_path_buf());
        Some(p)
    }

    pub fn forward(&mut self, current: &Path) -> Option<PathBuf> {
        let p = self.forward.pop()?;
        self.back.push(current.to_path_buf());
        Some(p)
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).filter(|p| p.is_dir())
}

/// `.` and `..` removed lexically (without following links), so Up and the path field read cleanly.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// A typed path: absolute, `~`-relative or relative to `dir`.
pub fn resolve(dir: &Path, text: &str) -> PathBuf {
    let t = text.trim();
    let p = if t == "~" || t.starts_with("~/") {
        home().map(|h| h.join(t.trim_start_matches('~').trim_start_matches('/'))).unwrap_or_else(|| PathBuf::from(t))
    } else {
        let p = PathBuf::from(t);
        if p.is_absolute() {
            p
        } else {
            dir.join(p)
        }
    };
    normalize(&p)
}

/// A Save name with the filter's suffix added when it has none ("wing" -> "wing.emesh.json").
pub fn with_default_ext(p: &Path, exts: &[String]) -> PathBuf {
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) else { return p.to_path_buf() };
    match exts.first() {
        Some(e) if !matches(&name, exts) => p.with_file_name(format!("{}.{e}", name.trim_end_matches('.'))),
        _ => p.to_path_buf(),
    }
}

/// Tab completion of a typed path: the longest common prefix of the matching entries (a `/` is
/// added when it is a single folder). None when nothing would change.
pub fn complete(dir: &Path, text: &str, dirs_only: bool) -> Option<String> {
    let (head, partial) = match text.rfind('/') {
        Some(i) => (&text[..=i], &text[i + 1..]),
        None => ("", text),
    };
    let base = if head.is_empty() { dir.to_path_buf() } else { resolve(dir, head) };
    let mut names: Vec<(String, bool)> = std::fs::read_dir(&base)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| (e.file_name().to_string_lossy().to_string(), e.path().is_dir()))
        .filter(|(n, d)| (!dirs_only || *d) && (partial.starts_with('.') || !is_hidden(n)))
        .collect();
    let exact: Vec<(String, bool)> = names.iter().filter(|(n, _)| n.starts_with(partial)).cloned().collect();
    names = if exact.is_empty() { names.into_iter().filter(|(n, _)| n.to_lowercase().starts_with(&partial.to_lowercase())).collect() } else { exact };
    let first = names.first()?.0.clone();
    let mut prefix: Vec<char> = first.chars().collect();
    for (n, _) in &names[1..] {
        let k = prefix.iter().zip(n.chars()).take_while(|(a, b)| **a == *b).count();
        prefix.truncate(k);
    }
    let mut out = format!("{head}{}", prefix.iter().collect::<String>());
    if names.len() == 1 && names[0].1 {
        out.push('/');
    }
    (out != text && out.len() >= text.len()).then_some(out)
}

pub fn fmt_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} bytes");
    }
    let mut v = n as f64 / 1024.0;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}

/// Days since 1970-01-01 to (year, month, day) (H. Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "2026-10-21 16:13" in local time.
pub fn fmt_time(secs: i64) -> String {
    let t = secs + tz::offset(secs);
    let (y, m, d) = civil(t.div_euclid(86_400));
    let s = t.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", s / 3600, s / 60 % 60)
}

/// The local UTC offset from the system's TZif zone file (no time crate in the tree; UTC when
/// there is none, e.g. on Windows).
mod tz {
    use std::sync::OnceLock;

    pub struct Zone {
        times: Vec<i64>,
        index: Vec<u8>,
        offsets: Vec<i64>,
    }

    pub fn parse(b: &[u8]) -> Option<Zone> {
        if b.get(..4)? != b"TZif" {
            return None;
        }
        let be = |o: usize, n: usize| -> Option<i64> {
            let s = b.get(o..o + n)?;
            Some(if n == 8 { i64::from_be_bytes(s.try_into().ok()?) } else { i32::from_be_bytes(s.try_into().ok()?) as i64 })
        };
        let counts = |o: usize| -> Option<[usize; 6]> {
            let mut c = [0usize; 6];
            for (k, v) in c.iter_mut().enumerate() {
                *v = be(o + 20 + 4 * k, 4)? as usize;
            }
            Some(c)
        };
        let [isut, isstd, leap, time, typ, chr] = counts(0)?;
        let (mut o, mut w) = (0usize, 4usize);
        if *b.get(4)? >= b'2' {
            o = 44 + time * 5 + typ * 6 + chr + leap * 8 + isstd + isut;
            w = 8;
        }
        let [_, _, _, time, typ, _] = counts(o)?;
        let mut p = o + 44;
        let mut times = Vec::with_capacity(time);
        for _ in 0..time {
            times.push(be(p, w)?);
            p += w;
        }
        let index = b.get(p..p + time)?.to_vec();
        p += time;
        let mut offsets = vec![];
        for _ in 0..typ {
            offsets.push(be(p, 4)?);
            p += 6;
        }
        (!offsets.is_empty()).then_some(Zone { times, index, offsets })
    }

    impl Zone {
        pub fn offset(&self, t: i64) -> i64 {
            let k = self.times.partition_point(|&x| x <= t);
            if k == 0 {
                return self.offsets[0];
            }
            self.index.get(k - 1).and_then(|&i| self.offsets.get(i as usize)).copied().unwrap_or(self.offsets[0])
        }
    }

    pub fn offset(t: i64) -> i64 {
        static ZONE: OnceLock<Option<Zone>> = OnceLock::new();
        ZONE.get_or_init(|| {
            let path = match std::env::var("TZ") {
                Ok(z) if !z.is_empty() && !z.contains(',') => {
                    let z = z.trim_start_matches(':').to_string();
                    if z.starts_with('/') {
                        z
                    } else {
                        format!("/usr/share/zoneinfo/{z}")
                    }
                }
                _ => "/etc/localtime".into(),
            };
            std::fs::read(path).ok().and_then(|b| parse(&b))
        })
        .as_ref()
        .map(|z| z.offset(t))
        .unwrap_or(0)
    }
}

/// What pressing Open / Save / Choose does with the current state.
#[derive(Clone, Debug, PartialEq)]
pub enum Accept {
    Navigate(PathBuf),
    Choose(PathBuf),
    ConfirmOverwrite(PathBuf),
    /// Folder mode: a new folder name typed in the name field
    CreateAndChoose(PathBuf),
    Error(String),
}

/// Last folder per dialog purpose, for this session.
static LAST_DIRS: Mutex<BTreeMap<String, PathBuf>> = Mutex::new(BTreeMap::new());

#[derive(Clone, Copy, Debug, PartialEq)]
enum Glyph {
    Folder,
    File,
    Project,
    Home,
    Drive,
    Recent,
    Back,
    Forward,
    Up,
    NewFolder,
}

#[derive(Clone, Debug)]
struct Place {
    label: String,
    path: PathBuf,
    glyph: Glyph,
}

#[derive(Clone, Debug)]
struct Rename {
    orig: String,
    text: String,
    started: bool,
}

enum Nav {
    To(PathBuf),
    Back,
    Forward,
    Up,
}

/// The in-app file browser.
pub struct FileBrowser {
    pub title: String,
    pub mode: FileMode,
    /// the folder shown
    pub dir: PathBuf,
    /// the File name / Folder field
    pub name: String,
    pub types: Vec<FileType>,
    pub type_index: usize,
    pub show_hidden: bool,
    pub sort: (SortKey, bool),
    pub entries: Vec<Entry>,
    pub selected: Option<String>,
    pub history: History,
    list_error: Option<String>,
    dirty: bool,
    path_text: String,
    error: String,
    places: Vec<Place>,
    recent: Vec<Place>,
    remember: Option<String>,
    rename: Option<Rename>,
    confirm: Option<(PathBuf, bool)>,
    focus_name: bool,
    scroll_to_selected: bool,
    /// height of the rows under the list, measured on the previous frame
    bottom_h: f32,
}

impl FileBrowser {
    /// `filters`: extensions without the dot (multi-part suffixes such as "emesh.json" allowed);
    /// empty means all files.
    pub fn new(title: &str, mode: FileMode, dir: &Path, name: &str, filters: &[&str]) -> Self {
        let dir = if dir.is_dir() { normalize(dir) } else { std::env::current_dir().ok().or_else(home).unwrap_or_else(|| PathBuf::from("/")) };
        let mut types = vec![];
        if !filters.is_empty() {
            types.push(FileType::new(type_label(filters), filters));
        }
        types.push(FileType::all());
        FileBrowser {
            title: title.into(),
            mode,
            path_text: dir.display().to_string(),
            dir,
            name: name.into(),
            types,
            type_index: 0,
            show_hidden: false,
            sort: (SortKey::Name, true),
            entries: vec![],
            selected: None,
            history: History::default(),
            list_error: None,
            dirty: true,
            error: String::new(),
            places: standard_places(),
            recent: vec![],
            remember: None,
            rename: None,
            confirm: None,
            focus_name: mode == FileMode::Save,
            scroll_to_selected: false,
            bottom_h: 110.0,
        }
    }

    /// Sidebar entries from the app: the open project's folder and recent folders.
    pub fn places(mut self, project_dir: Option<&Path>, settings: &Settings) -> Self {
        if let Some(p) = project_dir.filter(|p| p.is_dir()) {
            self.places.push(Place { label: "Project folder".into(), path: normalize(p), glyph: Glyph::Project });
        }
        let mut seen: Vec<PathBuf> = self.places.iter().map(|p| p.path.clone()).collect();
        let candidates = [settings.last_open_dir.clone(), settings.last_export_dir.clone()]
            .into_iter()
            .map(PathBuf::from)
            .chain(settings.recent.iter().filter_map(|r| Path::new(r).parent().map(|p| p.to_path_buf())));
        for p in candidates {
            if p.as_os_str().is_empty() || !p.is_dir() || seen.contains(&normalize(&p)) || self.recent.len() >= 6 {
                continue;
            }
            let p = normalize(&p);
            seen.push(p.clone());
            let label = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
            self.recent.push(Place { label, path: p, glyph: Glyph::Recent });
        }
        self
    }

    /// Start in (and afterwards remember) the folder last used for `key` in this session.
    pub fn remember(mut self, key: &str) -> Self {
        let last = LAST_DIRS.lock().ok().and_then(|m| m.get(key).cloned());
        if let Some(d) = last.filter(|d| d.is_dir()) {
            self.dir = d;
            self.path_text = self.dir.display().to_string();
            self.dirty = true;
        }
        self.remember = Some(key.into());
        self
    }

    fn exts(&self) -> &[String] {
        &self.types[self.type_index.min(self.types.len() - 1)].exts
    }

    /// Re-read the folder (keeps the selection when it is still there).
    pub fn refresh(&mut self) {
        let exts = self.exts().to_vec();
        match list_dir(&self.dir, &exts, self.mode == FileMode::Folder, self.show_hidden) {
            Ok(mut v) => {
                sort_entries(&mut v, self.sort.0, self.sort.1);
                self.entries = v;
                self.list_error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.list_error = Some(format!("Cannot read this folder: {e}"));
            }
        }
        if self.selected.as_ref().is_some_and(|s| !self.entries.iter().any(|e| &e.name == s)) {
            self.selected = None;
        }
        self.dirty = false;
    }

    fn set_sort(&mut self, key: SortKey) {
        self.sort = if self.sort.0 == key { (key, !self.sort.1) } else { (key, true) };
        sort_entries(&mut self.entries, self.sort.0, self.sort.1);
        self.scroll_to_selected = true;
    }

    /// Show `dir` (recording history unless it came from Back / Forward).
    pub fn navigate(&mut self, dir: PathBuf, record: bool) -> bool {
        let dir = normalize(&dir);
        if !dir.is_dir() {
            self.error = format!("“{}” is not a folder", dir.display());
            return false;
        }
        if dir != self.dir {
            if record {
                self.history.visit(&self.dir);
            }
            let from = std::mem::replace(&mut self.dir, dir);
            // going up selects the folder we came out of, as Qt does
            self.selected = from.strip_prefix(&self.dir).ok().and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().to_string());
            self.scroll_to_selected = true;
        }
        self.rename = None;
        self.error.clear();
        self.path_text = self.dir.display().to_string();
        if self.mode != FileMode::Save {
            self.name.clear();
        }
        self.refresh();
        true
    }

    pub fn go_back(&mut self) {
        if let Some(p) = self.history.back(&self.dir) {
            self.navigate(p, false);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(p) = self.history.forward(&self.dir) {
            self.navigate(p, false);
        }
    }

    pub fn go_up(&mut self) {
        if let Some(p) = self.dir.parent().map(|p| p.to_path_buf()) {
            self.navigate(p, true);
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let s = self.selected.as_ref()?;
        self.entries.iter().find(|e| &e.name == s)
    }

    /// A row was clicked or reached with the arrow keys.
    pub fn select(&mut self, name: &str) {
        self.selected = Some(name.into());
        // Save keeps the typed name when a folder is picked; otherwise the field follows the selection
        let is_dir = self.selected_entry().is_some_and(|e| e.is_dir);
        if !(self.mode == FileMode::Save && is_dir) {
            self.name = name.into();
        }
        self.error.clear();
    }

    /// Make a "New Folder" here and start renaming it.
    pub fn new_folder(&mut self) {
        self.finish_rename(true);
        match create_new_folder(&self.dir) {
            Ok(n) => {
                self.refresh();
                self.selected = Some(n.clone());
                self.scroll_to_selected = true;
                self.rename = Some(Rename { orig: n.clone(), text: n, started: false });
                self.error.clear();
            }
            Err(e) => self.error = e,
        }
    }

    fn start_rename(&mut self) {
        if let Some(e) = self.selected_entry() {
            let n = e.name.clone();
            self.rename = Some(Rename { orig: n.clone(), text: n, started: false });
        }
    }

    fn finish_rename(&mut self, keep: bool) {
        let Some(r) = self.rename.take() else { return };
        if !keep {
            return;
        }
        match rename_entry(&self.dir, &r.orig, &r.text) {
            Ok(()) => {
                let new = r.text.trim().to_string();
                self.refresh();
                if self.mode == FileMode::Folder || self.name == r.orig {
                    self.name = new.clone();
                }
                self.selected = Some(new);
                self.scroll_to_selected = true;
            }
            Err(e) => self.error = e,
        }
    }

    /// What the accept button would do now.
    pub fn accept_action(&self) -> Accept {
        let name = self.name.trim();
        let sel_dir = self.selected_entry().filter(|e| e.is_dir).map(|e| self.dir.join(&e.name));
        match self.mode {
            FileMode::Open | FileMode::Save if name.is_empty() => match sel_dir {
                Some(d) => Accept::Navigate(d),
                None if self.mode == FileMode::Open => Accept::Error("Select a file to open.".into()),
                None => Accept::Error("Enter a file name.".into()),
            },
            FileMode::Open => {
                let p = resolve(&self.dir, name);
                if p.is_dir() {
                    Accept::Navigate(p)
                } else if p.is_file() {
                    Accept::Choose(p)
                } else {
                    Accept::Error(format!("“{name}” was not found. Check the file name and try again."))
                }
            }
            FileMode::Save => {
                let p = resolve(&self.dir, name);
                if p.is_dir() {
                    return Accept::Navigate(p);
                }
                let p = with_default_ext(&p, self.exts());
                match p.parent() {
                    Some(d) if !d.is_dir() => Accept::Error(format!("The folder “{}” does not exist.", d.display())),
                    _ if p.is_file() => Accept::ConfirmOverwrite(p),
                    _ if p.exists() => Accept::Error(format!("“{}” is not a file.", p.display())),
                    _ => Accept::Choose(p),
                }
            }
            FileMode::Folder => {
                if name.is_empty() {
                    return Accept::Choose(self.dir.clone());
                }
                let p = resolve(&self.dir, name);
                if p.is_dir() {
                    Accept::Choose(p)
                } else if p.exists() {
                    Accept::Error(format!("“{name}” is not a folder."))
                } else {
                    Accept::CreateAndChoose(p)
                }
            }
        }
    }

    /// Run an accept action; Some(path) when the dialog is done.
    fn accept(&mut self, a: Accept) -> Option<PathBuf> {
        match a {
            Accept::Navigate(d) => {
                self.navigate(d, true);
                None
            }
            Accept::Choose(p) => Some(p),
            Accept::ConfirmOverwrite(p) => {
                self.confirm = Some((p, true));
                None
            }
            Accept::CreateAndChoose(p) => match std::fs::create_dir_all(&p) {
                Ok(()) => Some(p),
                Err(e) => {
                    self.error = format!("Could not create {}: {e}", p.display());
                    None
                }
            },
            Accept::Error(e) => {
                self.error = e;
                None
            }
        }
    }

    /// Enter on a row, or a double click: open a folder, accept a file.
    fn activate(&mut self, name: &str) -> Option<PathBuf> {
        let e = self.entries.iter().find(|e| e.name == name)?.clone();
        if e.is_dir {
            self.navigate(self.dir.join(&e.name), true);
            None
        } else {
            self.select(&e.name);
            let a = self.accept_action();
            self.accept(a)
        }
    }

    fn finish(&mut self, out: Option<PathBuf>) -> Option<Option<PathBuf>> {
        if let (Some(key), Ok(mut m)) = (&self.remember, LAST_DIRS.lock()) {
            let d = match (&out, self.mode) {
                (Some(p), FileMode::Folder) => p.clone(),
                (Some(p), _) => p.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| self.dir.clone()),
                (None, _) => self.dir.clone(),
            };
            m.insert(key.clone(), d);
        }
        Some(out)
    }

    // --- UI -------------------------------------------------------------------------------------

    /// Some(Some(path)) when chosen, Some(None) when cancelled.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Option<PathBuf>> {
        if self.dirty {
            self.refresh();
        }
        let title = self.title.clone();
        let mut out = None;
        modal(ctx, &title, |ui| {
            let screen = ctx.screen_rect().size();
            let max = (screen - Vec2::new(48.0, 96.0)).max(Vec2::new(420.0, 300.0));
            egui::Resize::default()
                .id_salt("file-browser")
                .default_size([760.0, 480.0])
                .min_size([560.0, 340.0])
                .max_size(max)
                .with_stroke(false)
                .show(ui, |ui| out = self.body(ui));
        });
        if let Some((p, fresh)) = self.confirm.clone() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let mut answer = None;
            modal(ctx, "Confirm Save As", |ui| {
                ui.label(format!("“{name}” already exists."));
                ui.label("Do you want to replace it?");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("  No  ").clicked() {
                            answer = Some(false);
                        }
                        if ui.button(RichText::new("  Yes  ").strong()).clicked() {
                            answer = Some(true);
                        }
                    });
                });
                if !fresh {
                    let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                    if esc {
                        answer = Some(false);
                    } else if enter && answer.is_none() {
                        answer = Some(true);
                    }
                }
            });
            match answer {
                Some(true) => {
                    self.confirm = None;
                    out = Some(Some(p));
                }
                Some(false) => self.confirm = None,
                None => self.confirm = Some((p, false)),
            }
        }
        out.and_then(|o| self.finish(o))
    }

    fn body(&mut self, ui: &mut Ui) -> Option<Option<PathBuf>> {
        let ctx = ui.ctx().clone();
        let (path_id, name_id, rename_id) = (Id::new("fb-path"), Id::new("fb-name"), Id::new("fb-rename"));
        let focused = ctx.memory(|m| m.focused());
        let mut nav: Option<Nav> = None;
        let mut out: Option<Option<PathBuf>> = None;
        let mut activate: Option<String> = None;
        let mut accept = false;
        let mut then_name: Option<String> = None;

        // --- keyboard (focus as it was when the frame began, so a field's own Enter is not reused)
        if self.confirm.is_none() {
            // Tab completes a typed path in either field
            if (focused == Some(path_id) || focused == Some(name_id)) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                let id = if focused == Some(path_id) { path_id } else { name_id };
                let text = if id == path_id { &mut self.path_text } else { &mut self.name };
                let dirs_only = id == path_id || self.mode == FileMode::Folder;
                if let Some(t) = complete(&self.dir, text, dirs_only) {
                    *text = t;
                    let n = text.chars().count();
                    if let Some(mut st) = egui::TextEdit::load_state(&ctx, id) {
                        st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(n))));
                        st.store(&ctx, id);
                    }
                }
            }
            if focused != Some(rename_id) {
                let (down, up) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::ArrowUp)));
                if (down || up) && !self.entries.is_empty() {
                    let cur = self.selected.as_ref().and_then(|s| self.entries.iter().position(|e| &e.name == s));
                    let k = match (cur, down) {
                        (None, _) => 0,
                        (Some(k), true) => (k + 1).min(self.entries.len() - 1),
                        (Some(k), false) => k.saturating_sub(1),
                    };
                    let n = self.entries[k].name.clone();
                    self.select(&n);
                    self.scroll_to_selected = true;
                }
                ui.input_mut(|i| {
                    if i.consume_key(Modifiers::ALT, Key::ArrowLeft) {
                        nav = Some(Nav::Back);
                    } else if i.consume_key(Modifiers::ALT, Key::ArrowRight) {
                        nav = Some(Nav::Forward);
                    } else if i.consume_key(Modifiers::ALT, Key::ArrowUp) {
                        nav = Some(Nav::Up);
                    }
                });
                if focused.is_none() {
                    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace)) {
                        nav = Some(Nav::Up);
                    }
                    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
                        self.start_rename();
                    }
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        match self.selected.clone() {
                            Some(s) if self.mode != FileMode::Save || self.selected_entry().is_some_and(|e| e.is_dir) || self.name.trim().is_empty() => activate = Some(s),
                            _ => accept = true,
                        }
                    }
                }
                // Esc closes the dialog (unless it is closing a drop-down first)
                if ui.input(|i| i.key_pressed(Key::Escape)) && !ctx.memory(|m| m.any_popup_open()) {
                    out = Some(None);
                }
            }
        }

        let spacing = ui.spacing().item_spacing;
        let row_h = ui.spacing().interact_size.y;

        // --- Look in: [path] < > ^ [New Folder]
        ui.horizontal(|ui| {
            row_label(ui, "Look in:", 86.0, row_h);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if tool_button(ui, Glyph::NewFolder, Some("New Folder"), true).on_hover_text("Create a new folder here").clicked() {
                    self.new_folder();
                }
                ui.add_space(4.0);
                if tool_button(ui, Glyph::Up, None, self.dir.parent().is_some()).on_hover_text("Parent folder (Backspace)").clicked() {
                    nav = Some(Nav::Up);
                }
                if tool_button(ui, Glyph::Forward, None, self.history.can_forward()).on_hover_text("Forward (Alt+Right)").clicked() {
                    nav = Some(Nav::Forward);
                }
                if tool_button(ui, Glyph::Back, None, self.history.can_back()).on_hover_text("Back (Alt+Left)").clicked() {
                    nav = Some(Nav::Back);
                }
                let w = ui.available_width();
                let r = if focused == Some(path_id) {
                    ui.add(egui::TextEdit::singleline(&mut self.path_text).id(path_id).lock_focus(true).desired_width(w))
                } else {
                    // unfocused: the tail of a long path ("…/study/runs"); a click edits the full path
                    self.path_text = self.dir.display().to_string();
                    let mut shown = elide_path(ui, &self.path_text, w - 10.0);
                    let r = ui.add(egui::TextEdit::singleline(&mut shown).id(path_id).lock_focus(true).desired_width(w));
                    if r.has_focus() {
                        let n = self.path_text.chars().count();
                        let mut st = egui::TextEdit::load_state(&ctx, path_id).unwrap_or_default();
                        st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(n))));
                        st.store(&ctx, path_id);
                    }
                    r
                };
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    let p = resolve(&self.dir, &self.path_text);
                    if p.is_dir() {
                        nav = Some(Nav::To(p));
                    } else if p.is_file() && self.mode != FileMode::Folder {
                        nav = Some(Nav::To(p.parent().map(|d| d.to_path_buf()).unwrap_or_default()));
                        then_name = p.file_name().map(|n| n.to_string_lossy().to_string());
                    } else {
                        self.error = format!("“{}” does not exist.", p.display());
                    }
                }
            });
        });
        ui.add_space(2.0);

        // --- sidebar | detail list
        let folder = self.mode == FileMode::Folder;
        let status_h = row_h;
        let mid_h = (ui.available_height() - self.bottom_h).floor().max(120.0);
        let full_w = ui.available_width();
        let side_w = (full_w * 0.22).clamp(130.0, 180.0).round();
        let (mid, _) = ui.allocate_exact_size(Vec2::new(full_w, mid_h), Sense::hover());
        let mid = Rect::from_min_max(mid.min.round(), mid.max.round());
        let side = Rect::from_min_size(mid.min, Vec2::new(side_w, mid_h));
        let list = Rect::from_min_max(Pos2::new(side.max.x + 6.0, mid.min.y), mid.max);
        self.sidebar(ui, side, &mut nav);
        self.list(ui, list, rename_id, &mut activate);

        // --- File name / Files of type (labels left, buttons right, as in Qt)
        ui.add_space(6.0);
        let label_w = 86.0;
        let button = Vec2::new(84.0, row_h);
        let accept_label = match self.mode {
            FileMode::Open => "Open",
            FileMode::Save => "Save",
            FileMode::Folder => "Choose",
        };
        ui.horizontal(|ui| {
            row_label(ui, if folder { "Folder:" } else { "File name:" }, label_w, row_h);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new(accept_label).strong()).min_size(button)).clicked() {
                    accept = true;
                }
                let r = ui.add(egui::TextEdit::singleline(&mut self.name).id(name_id).lock_focus(true).desired_width(ui.available_width()));
                if self.focus_name {
                    self.focus_name = false;
                    r.request_focus();
                    // select the name without its suffix, ready to type over
                    let stem = self.name.strip_suffix(".emesh.json").unwrap_or(&self.name).chars().count();
                    let mut st = egui::TextEdit::load_state(&ctx, name_id).unwrap_or_default();
                    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(stem))));
                    st.store(&ctx, name_id);
                }
                if r.changed() {
                    self.error.clear();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && self.confirm.is_none() {
                    accept = true;
                }
            });
        });
        ui.horizontal(|ui| {
            if folder {
                ui.add_space(label_w + spacing.x);
                ui.label(RichText::new("Choose takes the selected folder, or the one shown when none is selected.").small().weak());
            } else {
                row_label(ui, "Files of type:", label_w, row_h);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("Cancel").min_size(button)).clicked() {
                    out = Some(None);
                }
                if !folder {
                    let before = self.type_index;
                    egui::ComboBox::from_id_salt("fb-types").width(ui.available_width()).selected_text(self.types[self.type_index].label.clone()).show_ui(ui, |ui| {
                        for (k, t) in self.types.iter().enumerate() {
                            ui.selectable_value(&mut self.type_index, k, t.label.clone());
                        }
                    });
                    if before != self.type_index {
                        self.refresh();
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            ui.set_min_height(status_h);
            ui.add_space(label_w + spacing.x);
            if !self.error.is_empty() {
                ui.colored_label(col::error_text(), &self.error);
            }
        });
        // the list takes whatever the rows below leave (measured, so the window never creeps)
        let used = ui.min_rect().max.y - mid.max.y;
        if (used - self.bottom_h).abs() > 0.5 {
            self.bottom_h = used;
            ctx.request_repaint();
        }

        // --- act on what happened this frame
        if let Some(n) = nav {
            match n {
                Nav::To(p) => {
                    self.navigate(p, true);
                }
                Nav::Back => self.go_back(),
                Nav::Forward => self.go_forward(),
                Nav::Up => self.go_up(),
            }
        }
        if let Some(n) = then_name {
            self.select(&n);
            self.scroll_to_selected = true;
        }
        if out.is_some() {
            return out;
        }
        if let Some(n) = activate {
            if let Some(p) = self.activate(&n) {
                return Some(Some(p));
            }
        } else if accept {
            let a = self.accept_action();
            if let Some(p) = self.accept(a) {
                return Some(Some(p));
            }
        }
        None
    }

    fn sidebar(&mut self, ui: &mut Ui, rect: Rect, nav: &mut Option<Nav>) {
        let v = ui.visuals().clone();
        ui.painter().rect(rect, CornerRadius::same(3), v.extreme_bg_color, v.widgets.noninteractive.bg_stroke, StrokeKind::Inside);
        let inner = rect.shrink(1.0);
        ui.scope_builder(UiBuilder::new().max_rect(inner).id_salt("fb-side"), |ui| {
            egui::ScrollArea::vertical().id_salt("fb-side-scroll").auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(4.0);
                let sections = [("Places", self.places.clone()), ("Recent", self.recent.clone())];
                for (head, items) in sections {
                    if items.is_empty() {
                        continue;
                    }
                    if head == "Recent" {
                        ui.add_space(6.0);
                    }
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(head).size(11.5).weak());
                    });
                    ui.add_space(2.0);
                    for p in items {
                        let here = p.path == self.dir;
                        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
                        if here {
                            ui.painter().rect_filled(r.shrink2(Vec2::new(3.0, 0.0)), 3, v.selection.bg_fill);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r.shrink2(Vec2::new(3.0, 0.0)), 3, v.widgets.hovered.weak_bg_fill);
                        }
                        let icon = Rect::from_center_size(Pos2::new(r.min.x + 18.0, r.center().y), Vec2::splat(15.0));
                        paint_glyph(ui, p.glyph, icon, &v);
                        let text_color = if here { v.strong_text_color() } else { v.text_color() };
                        draw_text(ui, &p.label, Rect::from_min_max(Pos2::new(r.min.x + 32.0, r.min.y), Pos2::new(r.max.x - 4.0, r.max.y)), text_color, Align2::LEFT_CENTER);
                        let resp = resp.on_hover_text(p.path.display().to_string());
                        if resp.clicked() {
                            *nav = Some(Nav::To(p.path.clone()));
                        }
                    }
                }
            });
        });
    }

    fn list(&mut self, ui: &mut Ui, rect: Rect, rename_id: Id, activate: &mut Option<String>) {
        let v = ui.visuals().clone();
        let painter = ui.painter().clone();
        painter.rect(rect, CornerRadius::same(3), v.extreme_bg_color, Stroke::NONE, StrokeKind::Inside);
        let inner = rect.shrink(1.0);
        let head_h = 22.0;
        let row_h = 22.0;
        // columns: Name | Size | Type | Date Modified
        let (date_w, type_w, size_w) = (128.0, 112.0, 80.0);
        let gutter = 14.0; // the scroll bar
        let name_w = (inner.width() - date_w - type_w - size_w - gutter).max(120.0);
        let xs = [inner.min.x, inner.min.x + name_w, inner.min.x + name_w + size_w, inner.min.x + name_w + size_w + type_w, inner.max.x];
        let cols = [(SortKey::Name, "Name"), (SortKey::Size, "Size"), (SortKey::Type, "Type"), (SortKey::Date, "Date Modified")];

        // header
        let head = Rect::from_min_size(inner.min, Vec2::new(inner.width(), head_h));
        painter.rect_filled(head, CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }, v.faint_bg_color);
        painter.hline(head.x_range(), head.max.y, v.widgets.noninteractive.bg_stroke);
        for (k, (key, label)) in cols.iter().enumerate() {
            let cell = Rect::from_min_max(Pos2::new(xs[k], head.min.y), Pos2::new(xs[k + 1], head.max.y));
            let resp = ui.interact(cell, Id::new(("fb-col", k)), Sense::click());
            if resp.hovered() {
                painter.rect_filled(cell.shrink(1.0), 0.0, v.widgets.hovered.weak_bg_fill);
            }
            if k > 0 {
                painter.vline(cell.min.x, cell.y_range().shrink(4.0), v.widgets.noninteractive.bg_stroke);
            }
            let tr = Rect::from_min_max(cell.min + Vec2::new(8.0, 0.0), cell.max - Vec2::new(18.0, 0.0));
            draw_text(ui, label, tr, v.strong_text_color(), if k == 1 { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER });
            if self.sort.0 == *key {
                let c = Pos2::new(cell.max.x - 10.0, cell.center().y);
                let (a, b) = if self.sort.1 { (-1.0, 1.0) } else { (1.0, -1.0) };
                let pts = vec![c + Vec2::new(-4.0, 2.5 * b), c + Vec2::new(4.0, 2.5 * b), c + Vec2::new(0.0, 2.5 * a)];
                painter.add(egui::Shape::convex_polygon(pts, v.weak_text_color(), Stroke::NONE));
            }
            if resp.clicked() {
                self.set_sort(*key);
            }
        }

        // body
        let body = Rect::from_min_max(Pos2::new(inner.min.x, head.max.y + 1.0), inner.max);
        let bg = ui.interact(body, Id::new("fb-list-bg"), Sense::click());
        if bg.clicked() && self.rename.is_none() {
            self.selected = None;
        }
        let mut menu_on: Option<Option<String>> = None;
        if bg.secondary_clicked() {
            self.selected = None;
        }
        bg.context_menu(|ui| self.context_menu(ui, &mut menu_on, None));
        let mut rename_done: Option<bool> = None;
        ui.scope_builder(UiBuilder::new().max_rect(body).id_salt("fb-list"), |ui| {
            if let Some(err) = &self.list_error {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.colored_label(col::error_text(), err);
                });
                return;
            }
            if self.entries.is_empty() {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new(if self.mode == FileMode::Folder { "No sub-folders" } else { "No matching files" }).weak().italics());
                });
            }
            egui::ScrollArea::vertical().id_salt(("fb-list-scroll", &self.dir)).auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let width = ui.available_width();
                let entries = self.entries.clone();
                for e in &entries {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click());
                    let sel = self.selected.as_deref() == Some(e.name.as_str());
                    if sel && self.scroll_to_selected {
                        resp.scroll_to_me(None);
                    }
                    let renaming = self.rename.as_ref().is_some_and(|rn| rn.orig == e.name);
                    if ui.is_rect_visible(r) {
                        if sel {
                            ui.painter().rect_filled(r, 0.0, v.selection.bg_fill);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(r, 0.0, v.widgets.hovered.weak_bg_fill);
                        }
                        let fg = if sel { v.strong_text_color() } else { v.text_color() };
                        let weak = if sel { v.text_color() } else { v.weak_text_color() };
                        let glyph = if e.is_dir {
                            Glyph::Folder
                        } else if e.name.to_lowercase().ends_with(".emesh.json") {
                            Glyph::Project
                        } else {
                            Glyph::File
                        };
                        let icon = Rect::from_center_size(Pos2::new(r.min.x + 16.0, r.center().y), Vec2::splat(15.0));
                        paint_glyph(ui, glyph, icon, &v);
                        let cell = |k: usize, pad_l: f32, pad_r: f32| Rect::from_min_max(Pos2::new(xs[k] + pad_l, r.min.y), Pos2::new(xs[k + 1].min(r.max.x) - pad_r, r.max.y));
                        if !renaming {
                            draw_text(ui, &e.name, cell(0, 30.0, 6.0), fg, Align2::LEFT_CENTER);
                        }
                        if !e.is_dir {
                            draw_text(ui, &fmt_size(e.size), cell(1, 6.0, 18.0), weak, Align2::RIGHT_CENTER);
                        }
                        draw_text(ui, &type_name(e), cell(2, 8.0, 6.0), weak, Align2::LEFT_CENTER);
                        if let Some(t) = e.modified {
                            draw_text(ui, &fmt_time(t), cell(3, 8.0, 4.0), weak, Align2::LEFT_CENTER);
                        }
                    }
                    if renaming {
                        let rn = self.rename.as_mut().unwrap();
                        let field = Rect::from_min_max(Pos2::new(xs[0] + 26.0, r.min.y + 1.0), Pos2::new(xs[1] - 4.0, r.max.y - 1.0));
                        let te = ui.put(field, egui::TextEdit::singleline(&mut rn.text).id(rename_id).margin(Vec2::new(4.0, 1.0)));
                        if !rn.started {
                            rn.started = true;
                            te.request_focus();
                            let n = rn.text.chars().count();
                            let mut st = egui::TextEdit::load_state(ui.ctx(), rename_id).unwrap_or_default();
                            st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(n))));
                            st.store(ui.ctx(), rename_id);
                        } else if te.lost_focus() {
                            rename_done = Some(!ui.input(|i| i.key_pressed(Key::Escape)));
                        }
                    }
                    if resp.clicked() && !renaming {
                        if self.rename.is_some() {
                            rename_done = Some(true);
                        }
                        self.select(&e.name);
                    }
                    if resp.double_clicked() && !renaming {
                        *activate = Some(e.name.clone());
                    }
                    if resp.secondary_clicked() {
                        self.select(&e.name);
                    }
                    resp.context_menu(|ui| self.context_menu(ui, &mut menu_on, Some(e)));
                }
            });
        });
        self.scroll_to_selected = false;
        painter.rect_stroke(rect, CornerRadius::same(3), v.widgets.noninteractive.bg_stroke, StrokeKind::Inside);
        if let Some(keep) = rename_done {
            self.finish_rename(keep);
        }
        if let Some(target) = menu_on {
            match target {
                Some(name) if name == "\0rename" => self.start_rename(),
                Some(name) => *activate = Some(name),
                None => {}
            }
        }
    }

    /// Right-click menu (`act` gets Some(name) to open an entry, Some("\0rename") to rename).
    fn context_menu(&mut self, ui: &mut Ui, act: &mut Option<Option<String>>, e: Option<&Entry>) {
        if let Some(e) = e {
            if ui.button(if e.is_dir { "Open" } else { "Select" }).clicked() {
                *act = Some(Some(e.name.clone()));
                ui.close_menu();
            }
            if ui.button("Rename").clicked() {
                self.selected = Some(e.name.clone());
                *act = Some(Some("\0rename".into()));
                ui.close_menu();
            }
            ui.separator();
        }
        if ui.button("New Folder").clicked() {
            self.new_folder();
            ui.close_menu();
        }
        if ui.checkbox(&mut self.show_hidden, "Show hidden files").changed() {
            self.refresh();
            ui.close_menu();
        }
        if ui.button("Refresh").clicked() {
            self.refresh();
            ui.close_menu();
        }
    }
}

/// Home, Desktop, Documents, the filesystem root and (on WSL / Windows) the Windows drives.
fn standard_places() -> Vec<Place> {
    let mut v = vec![];
    if let Some(h) = home() {
        v.push(Place { label: "Home".into(), path: h.clone(), glyph: Glyph::Home });
        for d in ["Desktop", "Documents", "Downloads"] {
            if h.join(d).is_dir() {
                v.push(Place { label: d.into(), path: h.join(d), glyph: Glyph::Folder });
            }
        }
    }
    if cfg!(windows) {
        for c in b'A'..=b'Z' {
            let p = PathBuf::from(format!("{}:\\", c as char));
            if p.is_dir() {
                v.push(Place { label: format!("{}: drive", c as char), path: p, glyph: Glyph::Drive });
            }
        }
    } else {
        v.push(Place { label: "File system".into(), path: PathBuf::from("/"), glyph: Glyph::Drive });
        let wsl = std::env::var_os("WSL_DISTRO_NAME").is_some() || Path::new("/proc/sys/fs/binfmt_misc/WSLInterop").exists();
        if wsl {
            let mut drives: Vec<String> = std::fs::read_dir("/mnt")
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_lowercase())).collect())
                .unwrap_or_default();
            drives.sort();
            for d in drives {
                let p = PathBuf::from(format!("/mnt/{d}"));
                if p.is_dir() {
                    v.push(Place { label: format!("Windows ({}:)", d.to_uppercase()), path: p, glyph: Glyph::Drive });
                }
            }
        }
    }
    v
}

/// A left-aligned field label in a fixed-width cell.
fn row_label(ui: &mut Ui, text: &str, w: f32, h: f32) {
    ui.allocate_ui_with_layout(Vec2::new(w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(Vec2::new(w, h));
        ui.add(egui::Label::new(text).selectable(false));
    });
}

/// A path cut from the left at folder boundaries to fit `width` ("…/study/runs").
fn elide_path(ui: &Ui, path: &str, width: f32) -> String {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let fits = |t: &str| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::PLACEHOLDER).size().x) <= width;
    if fits(path) {
        return path.into();
    }
    let mut rest = path;
    while let Some((i, _)) = rest.char_indices().skip(1).find(|&(_, c)| c == '/' || c == '\\') {
        rest = &rest[i..];
        let t = format!("…{rest}");
        if fits(&t) {
            return t;
        }
    }
    format!("…{rest}")
}

/// One line of text, cut with an ellipsis to fit `rect`.
fn draw_text(ui: &Ui, text: &str, rect: Rect, color: Color32, align: Align2) {
    if rect.width() <= 4.0 {
        return;
    }
    let font = FontId::proportional(13.0);
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = TextWrapping::truncate_at_width(rect.width());
    let galley = ui.fonts(|f| f.layout_job(job));
    let pos = align.align_size_within_rect(galley.size(), rect).min;
    ui.painter().with_clip_rect(rect.intersect(ui.clip_rect())).galley(pos, galley, color);
}

/// A small button with a vector icon (and optional text), drawn like egui's own buttons.
fn tool_button(ui: &mut Ui, glyph: Glyph, label: Option<&str>, enabled: bool) -> egui::Response {
    let h = ui.spacing().interact_size.y;
    let font = FontId::proportional(13.0);
    let galley = label.map(|t| ui.fonts(|f| f.layout_no_wrap(t.to_string(), font.clone(), Color32::PLACEHOLDER)));
    let w = if let Some(g) = &galley { 8.0 + 16.0 + 6.0 + g.size().x + 10.0 } else { h + 6.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let v = ui.visuals().clone();
        let style = if enabled { ui.style().interact(&resp) } else { &v.widgets.inactive };
        ui.painter().rect(rect, style.corner_radius, style.weak_bg_fill, style.bg_stroke, StrokeKind::Inside);
        let fg = if enabled { v.text_color() } else { v.weak_text_color().gamma_multiply(0.6) };
        let icon_c = if galley.is_some() { Pos2::new(rect.min.x + 8.0 + 8.0, rect.center().y) } else { rect.center() };
        let icon = Rect::from_center_size(icon_c, Vec2::splat(15.0));
        match glyph {
            Glyph::NewFolder => paint_glyph(ui, glyph, icon, &v),
            _ => paint_arrow(ui, glyph, icon, fg),
        }
        if let Some(g) = galley {
            let pos = Pos2::new(rect.min.x + 30.0, rect.center().y - g.size().y / 2.0);
            ui.painter().galley(pos, g, fg);
        }
    }
    resp
}

fn paint_arrow(ui: &Ui, glyph: Glyph, r: Rect, color: Color32) {
    let p = |x: f32, y: f32| r.min + Vec2::new(x, y) * (r.width() / 16.0);
    let s = Stroke::new(1.6_f32, color);
    let painter = ui.painter();
    let (shaft, head) = match glyph {
        Glyph::Back => ([p(13.0, 8.0), p(3.5, 8.0)], [p(7.5, 4.0), p(3.5, 8.0), p(7.5, 12.0)]),
        Glyph::Forward => ([p(3.0, 8.0), p(12.5, 8.0)], [p(8.5, 4.0), p(12.5, 8.0), p(8.5, 12.0)]),
        _ => ([p(8.0, 13.5), p(8.0, 3.0)], [p(4.0, 7.0), p(8.0, 3.0), p(12.0, 7.0)]),
    };
    painter.line_segment(shaft, s);
    painter.add(egui::Shape::line(head.to_vec(), s));
}

/// Folder / file / place icons (16 x 16 design units).
fn paint_glyph(ui: &Ui, glyph: Glyph, r: Rect, v: &egui::Visuals) {
    let k = r.width() / 16.0;
    let p = |x: f32, y: f32| r.min + Vec2::new(x, y) * k;
    let painter = ui.painter();
    let folder_fill = Color32::from_rgb(0xe8, 0xb8, 0x4f);
    let folder_line = Color32::from_rgb(0xb5, 0x86, 0x22);
    let ink = v.text_color();
    let folder = |painter: &egui::Painter| {
        painter.add(egui::Shape::convex_polygon(vec![p(1.0, 3.0), p(6.0, 3.0), p(7.5, 4.5), p(7.5, 6.0), p(1.0, 6.0)], folder_line, Stroke::NONE));
        painter.rect(Rect::from_min_max(p(1.0, 4.5), p(15.0, 13.5)), CornerRadius::same(1), folder_fill, Stroke::new(1.0_f32, folder_line), StrokeKind::Inside);
    };
    match glyph {
        Glyph::Folder | Glyph::Recent => {
            folder(painter);
            if glyph == Glyph::Recent {
                let c = p(11.0, 10.0);
                painter.circle(c, 3.6 * k, v.extreme_bg_color, Stroke::new(1.0_f32, ink));
                painter.line_segment([c, c + Vec2::new(0.0, -2.2) * k], Stroke::new(1.0_f32, ink));
                painter.line_segment([c, c + Vec2::new(1.6, 0.6) * k], Stroke::new(1.0_f32, ink));
            }
        }
        Glyph::NewFolder => {
            folder(painter);
            let c = p(8.0, 9.0);
            let s = Stroke::new(1.6_f32, Color32::WHITE);
            painter.line_segment([c - Vec2::new(2.6, 0.0) * k, c + Vec2::new(2.6, 0.0) * k], s);
            painter.line_segment([c - Vec2::new(0.0, 2.6) * k, c + Vec2::new(0.0, 2.6) * k], s);
        }
        Glyph::File | Glyph::Project => {
            let line = v.weak_text_color();
            let pts = vec![p(3.0, 1.0), p(10.0, 1.0), p(13.0, 4.0), p(13.0, 15.0), p(3.0, 15.0)];
            painter.add(egui::Shape::convex_polygon(pts.clone(), v.extreme_bg_color, Stroke::NONE));
            let mut closed = pts;
            closed.push(p(3.0, 1.0));
            painter.add(egui::Shape::line(closed, Stroke::new(1.0_f32, line)));
            painter.add(egui::Shape::line(vec![p(10.0, 1.0), p(10.0, 4.0), p(13.0, 4.0)], Stroke::new(1.0_f32, line)));
            if glyph == Glyph::Project {
                // a little block grid in the accent colour
                let a = col::accent();
                painter.rect(Rect::from_min_max(p(5.0, 7.0), p(11.0, 13.0)), 0.0, crate::theme::alpha(a, 60), Stroke::new(1.0_f32, a), StrokeKind::Inside);
                painter.line_segment([p(8.0, 7.0), p(8.0, 13.0)], Stroke::new(1.0_f32, a));
                painter.line_segment([p(5.0, 10.0), p(11.0, 10.0)], Stroke::new(1.0_f32, a));
            } else {
                for y in [7.0, 9.5, 12.0] {
                    painter.line_segment([p(5.5, y), p(10.5, y)], Stroke::new(1.0_f32, line.gamma_multiply(0.7)));
                }
            }
        }
        Glyph::Home => {
            let a = col::accent();
            painter.add(egui::Shape::convex_polygon(vec![p(3.5, 7.5), p(12.5, 7.5), p(12.5, 14.0), p(3.5, 14.0)], crate::theme::alpha(a, 70), Stroke::NONE));
            painter.add(egui::Shape::line(vec![p(1.5, 8.5), p(8.0, 2.0), p(14.5, 8.5)], Stroke::new(1.5_f32, a)));
            painter.add(egui::Shape::line(vec![p(3.5, 7.0), p(3.5, 14.0), p(12.5, 14.0), p(12.5, 7.0)], Stroke::new(1.3_f32, a)));
            painter.rect_filled(Rect::from_min_max(p(6.8, 10.0), p(9.2, 14.0)), 0.0, a);
        }
        Glyph::Drive => {
            let line = v.text_color();
            painter.rect(Rect::from_min_max(p(1.5, 5.0), p(14.5, 12.0)), CornerRadius::same((2.0 * k) as u8), v.faint_bg_color, Stroke::new(1.1_f32, line), StrokeKind::Inside);
            painter.circle_filled(p(12.0, 8.5), 1.1 * k, Color32::from_rgb(0x2e, 0x9d, 0x57));
            painter.line_segment([p(3.5, 8.5), p(8.5, 8.5)], Stroke::new(1.0_f32, line.gamma_multiply(0.6)));
        }
        Glyph::Back | Glyph::Forward | Glyph::Up => paint_arrow(ui, glyph, r, ink),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Tmp {
            let d = std::env::temp_dir().join(format!("emesh-fb-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Tmp(d)
        }

        fn file(&self, name: &str, bytes: usize) {
            std::fs::write(self.0.join(name), vec![b'x'; bytes]).unwrap();
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn names(v: &[Entry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn listing_filters_hidden_and_extensions() {
        let t = Tmp::new("list");
        t.file("wing.emesh.json", 10);
        t.file("WING2.EMESH.JSON", 10);
        t.file("notes.txt", 5);
        t.file(".hidden.emesh.json", 1);
        t.file("emesh.json", 1); // no stem: not a project
        std::fs::create_dir(t.0.join("runs")).unwrap();
        std::fs::create_dir(t.0.join(".git")).unwrap();
        let exts = vec!["emesh.json".to_string()];
        let mut v = list_dir(&t.0, &exts, false, false).unwrap();
        sort_entries(&mut v, SortKey::Name, true);
        assert_eq!(names(&v), ["runs", "wing.emesh.json", "WING2.EMESH.JSON"]);
        let mut v = list_dir(&t.0, &[], false, true).unwrap();
        sort_entries(&mut v, SortKey::Name, true);
        assert_eq!(names(&v), [".git", "runs", ".hidden.emesh.json", "emesh.json", "notes.txt", "wing.emesh.json", "WING2.EMESH.JSON"]);
        let v = list_dir(&t.0, &[], true, false).unwrap();
        assert_eq!(names(&v), ["runs"]);
        assert!(list_dir(&t.0.join("missing"), &[], false, false).is_err());
    }

    #[test]
    fn sorting_keeps_folders_first() {
        let t = Tmp::new("sort");
        t.file("run10.dat", 300);
        t.file("run2.dat", 100);
        t.file("b.csv", 200);
        std::fs::create_dir(t.0.join("zeta")).unwrap();
        std::fs::create_dir(t.0.join("Alpha")).unwrap();
        let mut v = list_dir(&t.0, &[], false, false).unwrap();
        sort_entries(&mut v, SortKey::Name, true);
        assert_eq!(names(&v), ["Alpha", "zeta", "b.csv", "run2.dat", "run10.dat"]);
        sort_entries(&mut v, SortKey::Name, false);
        assert_eq!(names(&v), ["zeta", "Alpha", "run10.dat", "run2.dat", "b.csv"]);
        sort_entries(&mut v, SortKey::Size, true);
        assert_eq!(&names(&v)[2..], ["run2.dat", "b.csv", "run10.dat"]);
        sort_entries(&mut v, SortKey::Type, true);
        assert_eq!(&names(&v)[2..], ["b.csv", "run2.dat", "run10.dat"]);
        assert_eq!(natural_cmp("a2", "a10"), Ordering::Less);
        assert_eq!(natural_cmp("A", "b"), Ordering::Less);
        assert_eq!(natural_cmp("file007", "file7"), natural_cmp("file007", "file7"));
    }

    #[test]
    fn new_folder_names_are_unique() {
        let t = Tmp::new("newfolder");
        assert_eq!(create_new_folder(&t.0).unwrap(), "New Folder");
        assert_eq!(create_new_folder(&t.0).unwrap(), "New Folder 2");
        t.file("New Folder 3", 1);
        assert_eq!(unique_name(&t.0, "New Folder"), "New Folder 4");
        assert!(rename_entry(&t.0, "New Folder", "New Folder 2").is_err());
        assert!(rename_entry(&t.0, "New Folder", "a/b").is_err());
        rename_entry(&t.0, "New Folder", "case1").unwrap();
        assert!(t.0.join("case1").is_dir());
        assert!(create_new_folder(&t.0.join("missing")).is_err());

        // the browser puts the new folder into rename mode and selects it
        let mut fb = FileBrowser::new("t", FileMode::Save, &t.0, "x", &["emesh.json"]);
        fb.refresh();
        fb.new_folder();
        assert_eq!(fb.selected.as_deref(), Some("New Folder"));
        assert!(fb.rename.as_ref().is_some_and(|r| r.orig == "New Folder"));
        fb.rename.as_mut().unwrap().text = "grids".into();
        fb.finish_rename(true);
        assert!(t.0.join("grids").is_dir());
        assert_eq!(fb.selected.as_deref(), Some("grids"));
    }

    #[test]
    fn history_back_and_forward() {
        let t = Tmp::new("hist");
        for d in ["a", "a/b", "c"] {
            std::fs::create_dir_all(t.0.join(d)).unwrap();
        }
        let mut fb = FileBrowser::new("t", FileMode::Open, &t.0, "", &[]);
        assert!(!fb.history.can_back());
        fb.navigate(t.0.join("a"), true);
        fb.navigate(t.0.join("a/b"), true);
        fb.go_up();
        assert_eq!(fb.dir, t.0.join("a"));
        assert_eq!(fb.selected.as_deref(), Some("b")); // came out of b
        fb.go_back();
        assert_eq!(fb.dir, t.0.join("a/b"));
        fb.go_back();
        assert_eq!(fb.dir, t.0.join("a"));
        fb.go_back();
        assert_eq!(fb.dir, t.0);
        assert!(!fb.history.can_back());
        fb.go_forward();
        fb.go_forward();
        assert_eq!(fb.dir, t.0.join("a/b"));
        fb.navigate(t.0.join("c"), true); // a new visit clears forward
        assert!(!fb.history.can_forward());
        assert!(!fb.navigate(t.0.join("nope"), true));
        assert_eq!(fb.dir, t.0.join("c"));
        assert_eq!(resolve(&t.0.join("c"), "../a/./b"), t.0.join("a/b"));
    }

    #[test]
    fn save_adds_suffix_and_detects_overwrite() {
        let t = Tmp::new("save");
        t.file("wing.emesh.json", 3);
        std::fs::create_dir(t.0.join("sub")).unwrap();
        let mut fb = FileBrowser::new("t", FileMode::Save, &t.0, "wing", &["emesh.json"]);
        fb.refresh();
        assert_eq!(fb.accept_action(), Accept::ConfirmOverwrite(t.0.join("wing.emesh.json")));
        fb.name = "wing.emesh.json".into();
        assert_eq!(fb.accept_action(), Accept::ConfirmOverwrite(t.0.join("wing.emesh.json")));
        fb.name = "body".into();
        assert_eq!(fb.accept_action(), Accept::Choose(t.0.join("body.emesh.json")));
        fb.name = "sub".into();
        assert_eq!(fb.accept_action(), Accept::Navigate(t.0.join("sub")));
        fb.name = "nofolder/x".into();
        assert!(matches!(fb.accept_action(), Accept::Error(_)));
        fb.type_index = 1; // All files: no suffix added
        fb.name = "body.txt".into();
        assert_eq!(fb.accept_action(), Accept::Choose(t.0.join("body.txt")));
        // the overwrite question comes up before the dialog returns
        fb.type_index = 0;
        fb.name = "wing".into();
        let a = fb.accept_action();
        assert_eq!(fb.accept(a), None);
        assert!(fb.confirm.is_some());
    }

    #[test]
    fn open_and_folder_modes() {
        let t = Tmp::new("open");
        t.file("pts.dat", 3);
        std::fs::create_dir(t.0.join("sim")).unwrap();
        let mut fb = FileBrowser::new("t", FileMode::Open, &t.0, "", &["dat", "txt", "csv", "xy"]);
        fb.refresh();
        assert_eq!(fb.types[0].label, "Point files (*.dat *.txt *.csv *.xy)");
        assert!(matches!(fb.accept_action(), Accept::Error(_)));
        fb.select("pts.dat");
        assert_eq!(fb.name, "pts.dat");
        assert_eq!(fb.accept_action(), Accept::Choose(t.0.join("pts.dat")));
        fb.select("sim");
        assert_eq!(fb.accept_action(), Accept::Navigate(t.0.join("sim")));
        let mut fb = FileBrowser::new("t", FileMode::Folder, &t.0, "", &[]);
        fb.refresh();
        assert_eq!(names(&fb.entries), ["sim"]);
        assert_eq!(fb.accept_action(), Accept::Choose(t.0.clone()));
        fb.select("sim");
        assert_eq!(fb.accept_action(), Accept::Choose(t.0.join("sim")));
        fb.name = "new-sim".into();
        assert_eq!(fb.accept_action(), Accept::CreateAndChoose(t.0.join("new-sim")));
        fb.name = "pts.dat".into();
        assert!(matches!(fb.accept_action(), Accept::Error(_)));
    }

    #[test]
    fn completion_and_formatting() {
        let t = Tmp::new("complete");
        std::fs::create_dir(t.0.join("simulation")).unwrap();
        std::fs::create_dir(t.0.join("simple")).unwrap();
        t.file("single.dat", 1);
        assert_eq!(complete(&t.0, "s", false).as_deref(), Some("si"));
        assert_eq!(complete(&t.0, "si", false), None); // ambiguous
        assert_eq!(complete(&t.0, "s", true).as_deref(), Some("sim"));
        assert_eq!(complete(&t.0, "simu", true).as_deref(), Some("simulation/"));
        assert_eq!(complete(&t.0, "sin", false).as_deref(), Some("single.dat"));
        assert_eq!(complete(&t.0, "sin", true), None);
        let abs = format!("{}/simp", t.0.display());
        assert_eq!(complete(&t.0, &abs, true), Some(format!("{}/simple/", t.0.display())));
        assert_eq!(fmt_size(512), "512 bytes");
        assert_eq!(fmt_size(1536), "1.5 KB");
        assert_eq!(fmt_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_747), (2026, 10, 21));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(with_default_ext(Path::new("/x/a."), &["emesh.json".into()]), PathBuf::from("/x/a.emesh.json"));
        assert_eq!(type_name(&Entry { name: "a.dat".into(), is_dir: false, size: 0, modified: None }), "DAT file");
    }

    #[test]
    fn remembers_last_folder_per_purpose() {
        let t = Tmp::new("remember");
        std::fs::create_dir(t.0.join("pts")).unwrap();
        let key = format!("test-{}", std::process::id());
        let mut fb = FileBrowser::new("t", FileMode::Open, &t.0, "", &[]).remember(&key);
        fb.navigate(t.0.join("pts"), true);
        fb.finish(None);
        let fb = FileBrowser::new("t", FileMode::Open, &t.0, "", &[]).remember(&key);
        assert_eq!(fb.dir, t.0.join("pts"));
        let fb = FileBrowser::new("t", FileMode::Open, &t.0, "", &[]).remember("another");
        assert_eq!(fb.dir, t.0);
    }
}
