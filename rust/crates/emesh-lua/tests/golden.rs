//! Generated Lua must match the Python reference byte for byte.

use emesh_core::blocking;
use emesh_core::golden::corpus;
use emesh_core::topology::Topology;
use emesh_core::Document;
use emesh_lua::{generate_bcs_lua, generate_mesh_lua, sim_template, used_bc_tags};

fn first_diff(got: &str, want: &str) -> String {
    for (k, (a, b)) in got.lines().zip(want.lines()).enumerate() {
        if a != b {
            return format!("line {}:\n  rust:   {a}\n  python: {b}", k + 1);
        }
    }
    format!("lengths {} vs {}", got.len(), want.len())
}

#[test]
fn lua_matches_python() {
    if emesh_core::golden::skip() {
        return;
    }
    let mut failures = vec![];
    let mut n = 0;
    for (name, path, exp) in corpus() {
        let doc = Document::load(&path).unwrap();
        let topo = Topology::new(&doc);
        let res = blocking::resolve(&doc, &topo);
        let md = doc.with_blocks(res.blocks.clone());
        let mesh = match generate_mesh_lua(&md) {
            Ok(s) => s,
            Err(e) => format!("-- error: {e}\n"),
        };
        for (file, got) in [
            ("mesh.lua", mesh),
            ("bcs.lua", generate_bcs_lua(&md)),
            ("sim.lua", sim_template(&md, &used_bc_tags(&md))),
        ] {
            let want = std::fs::read_to_string(exp.join(file)).unwrap();
            if let Err(e) = emesh_core::golden::same_text(&got, &want) {
                failures.push(format!("{name}/{file}: {e}; {}", first_diff(&got, &want)));
            }
        }
        n += 1;
    }
    assert!(failures.is_empty(), "{} of {} projects differ:\n{}", failures.len(), n, failures.join("\n"));
}

#[cfg(feature = "check")]
#[test]
fn generated_lua_compiles() {
    if emesh_core::golden::skip() {
        return;
    }
    for (name, path, _) in corpus() {
        let doc = Document::load(&path).unwrap();
        let res = blocking::resolve(&doc, &Topology::new(&doc));
        let md = doc.with_blocks(res.blocks.clone());
        if let Ok(lua) = generate_mesh_lua(&md) {
            emesh_lua::check::compiles(&lua, "mesh.lua").unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        emesh_lua::check::compiles(&generate_bcs_lua(&md), "bcs.lua").unwrap();
        emesh_lua::check::compiles(&sim_template(&md, &used_bc_tags(&md)), "sim.lua").unwrap();
    }
}
