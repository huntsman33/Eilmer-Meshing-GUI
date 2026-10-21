//! The Python reference's outputs for every corpus project (golden/expected/NAME).

use emesh_core::blocking::{self, Resolution};
use emesh_core::golden::{assert_close, corpus, read_json};
use emesh_core::topology::Topology;
use emesh_core::Document;
use serde_json::{json, Value};

fn topology_json(t: &Topology) -> Value {
    json!({
        "size": t.size,
        "vertices": t.vertices,
        "edges": t.edges.iter().map(|e| json!({"curve": e.curve_id, "ta": e.ta, "tb": e.tb, "v0": e.v0, "v1": e.v1})).collect::<Vec<_>>(),
        "regions": t.regions.iter().map(|r| json!({
            "loop": r.loop_.iter().map(|(e, f)| json!([e, f])).collect::<Vec<_>>(),
            "holes": r.holes.iter().map(|h| h.iter().map(|(e, f)| json!([e, f])).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "area": r.area, "polygon": r.polygon, "hole_polygons": r.hole_polygons,
        })).collect::<Vec<_>>(),
    })
}

pub fn resolution_json(res: &Resolution) -> Value {
    json!({
        "blocks": res.blocks.iter().map(|b| {
            let mut d = b.to_json();
            d.as_object_mut().unwrap().insert("sides".into(), b.sides_json());
            d
        }).collect::<Vec<_>>(),
        "errors": res.errors,
        "warnings": res.warnings,
        "polygons": res.polygons,
        "groups": res.groups.iter().map(|((b, a), (gb, ga))| json!([b, a.to_string(), [gb, ga.to_string()]])).collect::<Vec<_>>(),
    })
}

#[test]
fn roundtrip_topology_resolution() {
    if emesh_core::golden::skip() {
        return;
    }
    let mut n = 0;
    for (name, path, exp) in corpus() {
        let text = std::fs::read_to_string(&path).unwrap();
        let doc = Document::from_text(&text).unwrap();
        assert_close(&format!("{name} roundtrip"), &doc.to_json(), &read_json(&exp.join("roundtrip.json")), 0.0);
        let want_text = std::fs::read_to_string(exp.join("roundtrip.json")).unwrap();
        let got_text = doc.to_string_pretty();
        if got_text != want_text {
            let line = got_text.lines().zip(want_text.lines()).position(|(a, b)| a != b);
            panic!("{name}: saved text differs from Python's at line {line:?}");
        }
        let _ = text;
        let topo = Topology::new(&doc);
        let want = read_json(&exp.join("topology.json"));
        let got = topology_json(&topo);
        assert_close(&format!("{name} topology"), &got, &want, emesh_core::golden::tol(1e-12));
        let res = blocking::resolve(&doc, &topo);
        assert_close(&format!("{name} resolution"), &resolution_json(&res), &read_json(&exp.join("resolution.json")), emesh_core::golden::tol(1e-12));
        n += 1;
    }
    assert!(n >= 20, "only {n} projects");
}
