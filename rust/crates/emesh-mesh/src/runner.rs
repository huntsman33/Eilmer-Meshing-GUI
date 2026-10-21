//! Run lmr on generated Lua and collect results (export, and the synchronous preview used by
//! tests); the GUI wraps the same steps in [`crate::live::LiveMesher`].

use std::path::{Path, PathBuf};
use std::process::Command;

use emesh_core::document::{BlockError, Document};
use emesh_lua::bcs::{generate_bcs_lua, sim_template, used_bc_tags, GAS_INPUT_FILE, SIM_FILE};
use emesh_lua::templates::GAS_TEMPLATE;
use emesh_lua::{generate_mesh_lua, BCS_FILE, MESH_FILE, PREVIEW_DIR};

use crate::preview::Grid;
use crate::vtk::read_structured_vtk;

/// The lmr executable: $LMR, `lmr` on PATH, or ~/gdtkinst/bin/lmr.
pub fn find_lmr() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("LMR") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let exe = if cfg!(windows) { "lmr.exe" } else { "lmr" };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let c = dir.join(exe);
            if c.is_file() {
                return Ok(c);
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let c = PathBuf::from(home).join("gdtkinst").join("bin").join(exe);
        if c.is_file() {
            return Ok(c);
        }
    }
    Err("lmr not found: put ~/gdtkinst/bin on PATH or set LMR".into())
}

#[derive(Clone, Debug, Default)]
pub struct LmrResult {
    pub command: String,
    pub returncode: i32,
    pub output: String,
    /// one per block, in block order
    pub grids: Vec<Grid>,
    /// files written (export)
    pub written: Vec<String>,
}

impl LmrResult {
    pub fn ok(&self) -> bool {
        self.returncode == 0
    }

    /// First meaningful error line, without the D stack trace (empty when ok).
    pub fn error_summary(&self) -> String {
        error_summary(&self.command, self.returncode, &self.output)
    }
}

pub fn error_summary(command: &str, code: i32, output: &str) -> String {
    if code == 0 {
        return String::new();
    }
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("----") || line.starts_with("??:?") {
            continue;
        }
        // "object.Error@(0): msg" / "lmr...Exception@file(97): msg" -> "msg"
        let head = line.split(':').next().unwrap_or("");
        if head.contains('@') {
            if let Some((_, msg)) = line.split_once(": ") {
                return msg.to_string();
            }
        }
        return line.to_string();
    }
    format!("lmr {command} failed with exit code {code}")
}

pub fn run_lmr(command: &str, script: &str, workdir: &Path) -> Result<LmrResult, String> {
    let lmr = find_lmr()?;
    let out = Command::new(&lmr)
        .args([command, "-j", script])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("could not run {}: {e}", lmr.display()))?;
    let mut output = String::from_utf8_lossy(&out.stdout).to_string();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(LmrResult { command: command.into(), returncode: out.status.code().unwrap_or(-1), output, ..Default::default() })
}

#[derive(Debug)]
pub enum RunError {
    Block(BlockError),
    Io(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Block(e) => write!(f, "{e}"),
            RunError::Io(e) => f.write_str(e),
        }
    }
}

impl From<BlockError> for RunError {
    fn from(e: BlockError) -> Self {
        RunError::Block(e)
    }
}

impl From<std::io::Error> for RunError {
    fn from(e: std::io::Error) -> Self {
        RunError::Io(e.to_string())
    }
}

impl From<String> for RunError {
    fn from(e: String) -> Self {
        RunError::Io(e)
    }
}

/// Write mesh.lua and bcs.lua for a document with concrete blocks.
pub fn write_scripts(doc: &Document, workdir: &Path) -> Result<(), RunError> {
    std::fs::create_dir_all(workdir)?;
    let lua = generate_mesh_lua(doc)?;
    std::fs::write(workdir.join(MESH_FILE), lua)?;
    std::fs::write(workdir.join(BCS_FILE), generate_bcs_lua(doc))?;
    Ok(())
}

pub fn preview_files(workdir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(workdir.join(PREVIEW_DIR))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    n.starts_with("blk-") && n.ends_with(".vtk")
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

pub fn clear_preview(workdir: &Path) {
    for f in preview_files(workdir) {
        let _ = std::fs::remove_file(f);
    }
}

/// Generate mesh.lua, run lmr custom-script, and load the block grids it writes.
pub fn preview_mesh(doc: &Document, workdir: &Path) -> Result<LmrResult, RunError> {
    write_scripts(doc, workdir)?;
    clear_preview(workdir);
    let mut result = run_lmr("custom-script", MESH_FILE, workdir)?;
    if result.ok() {
        result.grids = preview_files(workdir).iter().map(|p| read_structured_vtk(p)).collect::<Result<_, _>>()?;
    }
    Ok(result)
}

/// sim.lua and a gas-model input, only where they do not exist yet. Returns the files written.
pub fn write_starter_files(doc: &Document, simdir: &Path) -> Result<Vec<String>, RunError> {
    let mut written = vec![];
    for (name, text) in [(SIM_FILE, sim_template(doc, &used_bc_tags(doc))), (GAS_INPUT_FILE, GAS_TEMPLATE.to_string())] {
        let path = simdir.join(name);
        if !path.exists() {
            std::fs::write(&path, text)?;
            written.push(name.to_string());
        }
    }
    Ok(written)
}

/// Write mesh.lua and bcs.lua (and optionally starter sim files) into a folder; run lmr prep-grids.
pub fn export_grids(doc: &Document, simdir: &Path, starter_files: bool) -> Result<LmrResult, RunError> {
    write_scripts(doc, simdir)?;
    let written = if starter_files { write_starter_files(doc, simdir)? } else { vec![] };
    let mut result = run_lmr("prep-grids", MESH_FILE, simdir)?;
    result.written = vec![MESH_FILE.to_string(), BCS_FILE.to_string()];
    result.written.extend(written);
    Ok(result)
}

/// Number of grid connections in an exported lmrsim/grid/grid.metadata.
pub fn exported_connections(simdir: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(simdir.join("lmrsim").join("grid").join("grid.metadata")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("grid-connections").and_then(|c| c.as_array()).map(|a| a.len())
}
