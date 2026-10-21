//! The command-line half of `emesh` (export, Lua, checks, preview), shared by the `emesh` app
//! binary and the console-only `emesh-cli`.
//!
//!   emesh export PROJECT.emesh.json FOLDER [--no-starter]   write mesh.lua, bcs.lua (and sim.lua,
//!                                                           ideal-air.lua) and run lmr prep-grids
//!   emesh lua PROJECT [mesh|bcs|sim]                        print generated Lua
//!   emesh check PROJECT                                     blocks needing attention, cell quality
//!   emesh preview PROJECT [WORKDIR]                         run lmr custom-script, compare with the preview

use std::path::{Path, PathBuf};

use emesh_core::blocking::{self, Resolution};
use emesh_core::topology::Topology;
use emesh_core::Document;
use emesh_lua::{generate_bcs_lua, generate_mesh_lua, sim_template, used_bc_tags};
use emesh_mesh::preview::block_grid;
use emesh_mesh::quality::{assess, intentional_aspect, Level, DEFAULT_THRESHOLDS};
use emesh_mesh::runner::{export_grids, exported_connections, preview_mesh};

pub const USAGE: &str = "usage:
  emesh [PROJECT.emesh.json]                      open the app
  emesh export PROJECT.emesh.json FOLDER [--no-starter]
  emesh lua PROJECT.emesh.json [mesh|bcs|sim]
  emesh check PROJECT.emesh.json
  emesh preview PROJECT.emesh.json [WORKDIR]
  emesh --version";

fn load(path: &str) -> Result<(Document, Resolution), String> {
    let doc = Document::load(Path::new(path))?;
    let res = blocking::resolve(&doc, &Topology::new(&doc));
    Ok((doc, res))
}

fn concrete(doc: &Document, res: &Resolution) -> Document {
    doc.with_blocks(res.blocks.clone())
}

fn report_errors(res: &Resolution) -> bool {
    for (bid, msg) in &res.errors {
        eprintln!("{bid}: {msg}");
    }
    !res.errors.is_empty()
}

/// The subcommands (a first argument that is one of these runs the command line, not the app).
pub const COMMANDS: [&str; 4] = ["export", "lua", "check", "preview"];

pub fn is_command(arg: &str) -> bool {
    COMMANDS.contains(&arg)
}

pub fn run(args: &[String]) -> Result<(), String> {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match cmd {
        "export" if args.len() >= 3 => {
            let (doc, res) = load(&args[1])?;
            if report_errors(&res) {
                return Err("some blocks need attention before exporting".into());
            }
            if res.blocks.is_empty() {
                return Err("there are no blocks to export".into());
            }
            let folder = PathBuf::from(&args[2]);
            std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            let starter = !args.iter().any(|a| a == "--no-starter");
            let result = export_grids(&concrete(&doc, &res), &folder, starter).map_err(|e| e.to_string())?;
            if !result.ok() {
                eprintln!("{}", result.output);
                return Err(format!("lmr prep-grids failed: {}", result.error_summary()));
            }
            let warnings: usize = res.warnings.values().map(|w| w.len()).sum();
            println!(
                "Exported {} block(s) and {} connection(s) to {}\nFiles: {}",
                res.blocks.len(),
                exported_connections(&folder).unwrap_or(0),
                folder.display(),
                result.written.join(", ")
            );
            for (bid, w) in &res.warnings {
                for m in w {
                    println!("warning: {bid}: {m}");
                }
            }
            if warnings > 0 {
                println!("{warnings} block warning(s)");
            }
            Ok(())
        }
        "lua" if args.len() >= 2 => {
            let (doc, res) = load(&args[1])?;
            let md = concrete(&doc, &res);
            match args.get(2).map(|s| s.as_str()).unwrap_or("mesh") {
                "mesh" => print!("{}", generate_mesh_lua(&md).map_err(|e| e.to_string())?),
                "bcs" => print!("{}", generate_bcs_lua(&md)),
                "sim" => print!("{}", sim_template(&md, &used_bc_tags(&md))),
                other => return Err(format!("unknown script '{other}' (mesh, bcs or sim)")),
            }
            report_errors(&res);
            Ok(())
        }
        "check" if args.len() >= 2 => {
            let (doc, res) = load(&args[1])?;
            let md = concrete(&doc, &res);
            println!("{} block(s), {} needing attention", doc.blocks.len(), res.errors.len());
            report_errors(&res);
            for (bid, w) in &res.warnings {
                for m in w {
                    println!("warning: {bid}: {m}");
                }
            }
            let grids: Vec<_> = res.blocks.iter().map(|b| (b.id.clone(), block_grid(&md, b))).collect();
            let q = assess(0, &grids, DEFAULT_THRESHOLDS, &intentional_aspect(&res.blocks));
            let cells: usize = grids.iter().map(|(_, g)| g.cells()).sum();
            println!("{cells} cells; quality: {}", q.level().as_str());
            if q.level() != Level::Ok {
                println!("{}", q.summary(10));
            }
            Ok(())
        }
        "preview" if args.len() >= 2 => {
            let (doc, res) = load(&args[1])?;
            report_errors(&res);
            let md = concrete(&doc, &res);
            let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("emesh-preview"));
            let result = preview_mesh(&md, &dir).map_err(|e| e.to_string())?;
            if !result.ok() {
                return Err(format!("lmr custom-script failed: {}", result.error_summary()));
            }
            let mut worst = 0.0f64;
            for (b, g) in res.blocks.iter().zip(&result.grids) {
                if b.patch != "CoonsPatch" {
                    continue; // AOPatch: Eilmer's own construction
                }
                let p = block_grid(&md, b);
                for (a, c) in p.pts.iter().zip(&g.pts) {
                    worst = worst.max((a[0] - c[0]).abs()).max((a[1] - c[1]).abs());
                }
            }
            println!("Eilmer wrote {} grid(s); largest preview difference {worst:.3e} m", result.grids.len());
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    }
}

