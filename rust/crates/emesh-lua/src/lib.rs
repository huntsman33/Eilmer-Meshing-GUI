//! Generate Eilmer Lua from a Document.
//!
//! mesh.lua runs under both
//!   lmr prep-grids -j mesh.lua     (registerFluidGrid exists: register blocks, connect them)
//!   lmr custom-script -j mesh.lua  (it does not: write preview/blk-NNNN.vtk for the GUI)
//! Generation is one-way and deterministic; the Document is the source of truth. The output is
//! byte-identical to the Python reference's (checked against golden files).
//!
//! Also here: bcs.lua (bcDict), the starter sim.lua, the BC class list and tag rules.

pub mod bcs;
#[cfg(feature = "check")]
pub mod check;
pub mod templates;

use std::collections::HashMap;

use emesh_core::blocking::face_key;
use emesh_core::document::{Block, BlockError, Cluster, CurveRef, Document, FACES};
use emesh_core::entities::Kind;
use emesh_core::pyfmt::repr;
use emesh_core::spacing::concrete_cluster;
use emesh_core::P;
use serde_json::Value;

pub use bcs::{generate_bcs_lua, sim_template, used_bc_tags, GAS_INPUT_FILE, SIM_FILE};

pub const PREVIEW_DIR: &str = "preview";
pub const MESH_FILE: &str = "mesh.lua";
pub const BCS_FILE: &str = "bcs.lua";

fn num(v: f64) -> String {
    repr(v)
}

/// Python `repr` of a JSON number (an int stays an int).
pub fn number_repr(v: &serde_json::Number) -> String {
    if v.is_f64() {
        repr(v.as_f64().unwrap())
    } else {
        v.to_string()
    }
}

fn lua_value(v: &Value) -> String {
    match v {
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => number_repr(n),
        Value::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
        Value::Null => "\"None\"".into(),
        other => format!("\"{}\"", other.to_string().replace('"', "\\\"")),
    }
}

/// Python `re.sub(r"\W", "_", s)`.
pub fn ident(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// Deduplicates points so shared corners become one named Vector3.
struct PointTable {
    names: HashMap<(u64, u64), String>,
    lines: Vec<String>,
}

impl PointTable {
    fn name_of(&mut self, p: P) -> String {
        let key = (emesh_core::num::round_key(p[0], 12), emesh_core::num::round_key(p[1], 12));
        if let Some(n) = self.names.get(&key) {
            return n.clone();
        }
        let name = format!("p{:03}", self.names.len());
        self.lines.push(format!("{name} = Vector3:new{{x={}, y={}}}", num(p[0]), num(p[1])));
        self.names.insert(key, name.clone());
        name
    }
}

fn face_expr(refs: &[CurveRef]) -> String {
    let parts: Vec<String> = refs
        .iter()
        .map(|r| {
            let name = format!("path_{}", ident(&r.id));
            if r.full() {
                if r.reversed {
                    format!("ReversedPath:new{{underlying_path={name}}}")
                } else {
                    name
                }
            } else {
                // Part of a curve; a reversed range (t0 > t1) runs it backwards.
                let (t0, t1) = if r.reversed { (r.t1, r.t0) } else { (r.t0, r.t1) };
                format!("SubRangedPath:new{{underlying_path={name}, t0={}, t1={}}}", repr(t0), repr(t1))
            }
        })
        .collect();
    if parts.len() == 1 {
        return parts.into_iter().next().unwrap();
    }
    format!("Polyline:new{{segments={{{}}}}}", parts.join(", "))
}

fn cluster_expr(spec: &Cluster) -> String {
    match spec.kind() {
        "table" => {
            let xs: Vec<String> = spec.0["x"].as_array().unwrap().iter().map(|v| repr(v.as_f64().unwrap())).collect();
            format!("tableCluster({{{}}})", xs.join(", "))
        }
        "growth" => format!(
            "growthCluster({}, {}, {})",
            repr(spec.f("r").unwrap()),
            spec.0["n"].as_f64().unwrap() as i64,
            if spec.b("reverse") { "true" } else { "false" }
        ),
        kind => {
            let args: Vec<String> =
                spec.0.iter().filter(|(k, _)| k.as_str() != "type").map(|(k, v)| format!("{k}={}", lua_value(v))).collect();
            format!("{kind}:new{{{}}}", args.join(", "))
        }
    }
}

/// The mesh.lua script for a document whose blocks are concrete (resolved).
pub fn generate_mesh_lua(doc: &Document) -> Result<String, BlockError> {
    for b in &doc.blocks {
        doc.validate_block(b)?;
    }
    let mut used: Vec<String> = vec![];
    for b in &doc.blocks {
        for f in FACES {
            for r in b.side(f) {
                if !used.contains(&r.id) {
                    used.push(r.id.clone());
                }
            }
        }
    }
    let mut points = PointTable { names: HashMap::new(), lines: vec![] };
    let path_lines: Vec<String> = used
        .iter()
        .map(|cid| {
            let c = &doc.curves[cid];
            format!("path_{} = {}", ident(cid), c.lua_expr(&mut |p| points.name_of(p)))
        })
        .collect();

    let mut out = vec![templates::HEADER.replace("{preview}", PREVIEW_DIR)];
    if doc.blocks.iter().any(|b| b.smoothed()) {
        out.push(templates::LAPLACE_HELPER.to_string());
    }
    let any_growth = doc.blocks.iter().any(|b| {
        FACES.iter().any(|f| concrete_cluster(doc, b, *f).is_some_and(|c| c.kind() == "growth" || c.kind() == "table"))
    });
    if any_growth {
        out.push(templates::GROWTH_HELPER.to_string());
    }
    out.push("-- points".into());
    out.extend(points.lines.iter().cloned());
    out.push("\n-- paths".into());
    out.extend(path_lines);
    out.push("\n-- blocks".into());
    out.push("blocks = {}".into());
    for b in &doc.blocks {
        out.push(block_lua(doc, b)?);
    }
    let conns: Vec<String> = block_connections(doc, &doc.blocks)
        .iter()
        .map(|(a, fa, b, fb)| format!("   connectGrids(ids[{}], \"{}\", ids[{}], \"{}\")", a + 1, fa.as_str(), b + 1, fb.as_str()))
        .collect();
    let conns = if conns.is_empty() { "   -- (no connections)".to_string() } else { conns.join("\n") };
    out.push(templates::FOOTER.replace("{connections}", &conns).replace("{preview}", PREVIEW_DIR));
    Ok(out.join("\n"))
}

/// (index A, face A, index B, face B) for faces two blocks share exactly along split curves.
pub fn block_connections(
    doc: &Document,
    blocks: &[Block],
) -> Vec<(usize, emesh_core::Face, usize, emesh_core::Face)> {
    let mut first: HashMap<emesh_core::blocking::FaceKey, (usize, emesh_core::Face)> = HashMap::new();
    let mut out = vec![];
    for (i, b) in blocks.iter().enumerate() {
        for f in FACES {
            let key = face_key(b.side(f));
            match first.get(&key) {
                None => {
                    first.insert(key, (i, f));
                }
                Some(&(j, g)) => {
                    if b.side(f).iter().all(|r| doc.curves[&r.id].kind == Kind::Split) {
                        out.push((j, g, i, f));
                    }
                }
            }
        }
    }
    out
}

fn opt_number(v: Option<&Value>, default: i64) -> String {
    match v {
        Some(Value::Number(n)) => number_repr(n),
        Some(other) => other.to_string(),
        None => default.to_string(),
    }
}

fn block_lua(doc: &Document, b: &Block) -> Result<String, BlockError> {
    let bid = ident(&b.id);
    let faces: Vec<String> = FACES.iter().map(|f| format!("{}={}", f.as_str(), face_expr(b.side(*f)))).collect();
    let faces = faces.join(", ");
    let surf = match b.patch.as_str() {
        "CoonsPatch" => format!("CoonsPatch:new{{{faces}}}"),
        "AOPatch" => format!(
            "AOPatch:new{{{faces}, nx={}, ny={}}}",
            opt_number(b.patch_opts.get("nx"), 10),
            opt_number(b.patch_opts.get("ny"), 10)
        ),
        other => return Err(BlockError(format!("block {}: unsupported patch type {other}", b.id))),
    };
    let mut grid_args = format!("psurface=surf_{bid}, niv={}, njv={}", b.niv, b.njv);
    let cluster: Vec<Option<Cluster>> = FACES.iter().map(|f| concrete_cluster(doc, b, *f)).collect();
    if cluster.iter().any(|c| c.is_some()) {
        let cf: Vec<String> = FACES
            .iter()
            .zip(&cluster)
            .filter_map(|(f, c)| c.as_ref().map(|c| format!("{}={}", f.as_str(), cluster_expr(c))))
            .collect();
        grid_args += &format!(", cfList={{{}}}", cf.join(", "));
    }
    grid_args += &format!(", label=\"{}\"", b.id);
    let mut tags = vec![];
    for f in FACES {
        let t = doc.face_bc_tag(b, f)?;
        if !t.is_empty() {
            tags.push(format!("{}=\"{t}\"", f.as_str()));
        }
    }
    let mut lines = vec![
        format!("-- block {}", b.id),
        format!("surf_{bid} = {surf}"),
        format!("grid_{bid} = StructuredGrid:new{{{grid_args}}}"),
    ];
    if b.smoothed() {
        let s = b.smoothing.as_ref().unwrap();
        let nsweeps = s.get("nsweeps").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
        let tol = s.get("tol").and_then(|v| v.as_f64()).unwrap_or(0.0);
        lines.push(format!("laplaceSmooth(grid_{bid}, {nsweeps}, {})", num(tol)));
    }
    lines.push(format!(
        "blocks[#blocks+1] = {{grid=grid_{bid}, tag=\"{}\", fsTag=\"{}\", bcTags={{{}}}}}",
        b.id,
        b.fs_tag,
        tags.join(", ")
    ));
    Ok(lines.join("\n") + "\n")
}
