//! Model operations replayed on the Python reference's inputs (golden/expected/ops/*.json):
//! the resulting documents, errors and return values must match.

use emesh_app::model::{SketchModel, Subdivision};
use emesh_core::document::{Cluster, Document, Face};
use emesh_core::golden::{dir, read_json};
use emesh_core::importers::{build_curves, parse_point_file, ImportOptions};
use emesh_core::layers::LayerSettings;
use emesh_core::topology::curve_params;
use emesh_core::{geom, P};
use serde_json::{json, Value};

fn pt(v: &Value) -> P {
    [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn run(m: &mut SketchModel, op: &Value) -> Result<Value, String> {
    let s = |k: &str| op[k].as_str().unwrap().to_string();
    let f = |k: &str| op[k].as_f64().unwrap();
    Ok(match op["op"].as_str().unwrap() {
        "make_block" => json!(m.make_block(pt(&op["point"]), None)?),
        "make_all_blocks" => {
            let (made, skipped) = m.make_all_blocks();
            json!({"made": made, "skipped": skipped})
        }
        "subdivide" => {
            let o = &op["opts"];
            let kind = match op["kind"].as_str().unwrap() {
                "nxm" => Subdivision::Nxm(o["n"].as_u64().unwrap_or(2) as usize, o["m"].as_u64().unwrap_or(2) as usize),
                "ogrid" => Subdivision::OGrid(o["size"].as_f64().unwrap_or(0.5)),
                "ogrid_hole" => Subdivision::OGridHole,
                _ => Subdivision::Centroid,
            };
            json!(m.subdivide(pt(&op["point"]), kind)?)
        }
        "set_edge_cells" => {
            let cl = op.get("cluster").and_then(Cluster::from_value);
            let opposite = op.get("opposite").and_then(|v| v.as_bool()).unwrap_or(true);
            m.set_edge_cells(&s("bid"), Face::parse(&s("face")).unwrap(), op["cells"].as_u64().unwrap() as usize, cl, opposite)?;
            Value::Null
        }
        "add_boundary_layer" => {
            let faces: Vec<(String, Face)> = op["faces"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| (f[0].as_str().unwrap().to_string(), Face::parse(f[1].as_str().unwrap()).unwrap()))
                .collect();
            let tr = op.get("transition").and_then(|v| v.as_bool()).unwrap_or(false);
            json!(m.add_boundary_layer(&faces, f("first"), f("growth"), op["cells"].as_u64().unwrap() as usize, tr)?)
        }
        "edit_boundary_layer" => {
            let st = &op["settings"];
            let settings = LayerSettings {
                first: st.get("first").and_then(|v| v.as_f64()),
                growth: st.get("growth").and_then(|v| v.as_f64()),
                cells: st.get("cells").and_then(|v| v.as_u64()).map(|v| v as usize),
                transition: st.get("transition").and_then(|v| v.as_bool()),
            };
            m.edit_boundary_layer(&s("lid"), settings)?;
            Value::Null
        }
        "remove_boundary_layer" => {
            m.remove_boundary_layer(&s("lid"))?;
            Value::Null
        }
        "layer_note" => json!(m.layer_note(&s("lid"))),
        "refine_blocks" => {
            let (a, b, c) = m.refine_blocks(&strs(&op["bids"]));
            json!([a, b, c])
        }
        "offset" => json!(m.offset(
            &strs(&op["ids"]),
            f("distance"),
            op.get("reverse").and_then(|v| v.as_bool()).unwrap_or(false),
            op.get("source_to_reference").and_then(|v| v.as_bool()).unwrap_or(false)
        )?),
        "merge" => {
            let (ids, n) = m.merge(&strs(&op["ids"]));
            json!([ids, n])
        }
        "mirror" => json!(m.mirror(&strs(&op["ids"]), pt(&op["a"]), pt(&op["b"]))),
        "trim" => {
            let cid = s("cid");
            let ts = curve_params(&m.doc, &cid);
            let (t, _, _) = geom::closest_param(&m.doc.curves[&cid], pt(&op["point"]));
            let k = (0..ts.len() - 1).filter(|&i| ts[i] <= t).max().unwrap();
            m.trim(&cid, ts[k], ts[k + 1])?;
            json!([ts[k], ts[k + 1]])
        }
        "toggle_split" => {
            m.toggle_split(Some(&strs(&op["ids"])));
            Value::Null
        }
        "toggle_reference" => {
            m.toggle_reference(Some(&strs(&op["ids"])));
            Value::Null
        }
        "add_dimension" => {
            let c2 = op.get("cid2").and_then(|v| v.as_str()).map(|s| s.to_string());
            json!(m.add_dimension(&s("cid"), f("offset"), c2.as_deref(), None)?)
        }
        "set_dimension_value" => {
            m.set_dimension_value(&s("did"), f("value"))?;
            Value::Null
        }
        "set_line" => {
            m.set_line(&s("cid"), f("length"), f("angle"))?;
            Value::Null
        }
        "set_arc" => {
            m.set_arc(&s("cid"), f("radius"), f("sweep"))?;
            Value::Null
        }
        "translate" => {
            m.translate(&strs(&op["ids"]), f("dx"), f("dy"));
            Value::Null
        }
        "define_bc" => {
            let old = op.get("old_tag").and_then(|v| v.as_str()).map(|s| s.to_string());
            m.define_bc(&s("tag"), &s("cls"), op["args"].as_str().unwrap_or(""), old.as_deref())?;
            Value::Null
        }
        "set_bc" => json!(m.set_bc(&strs(&op["ids"]), &s("tag"))?),
        "delete_bc" => {
            m.delete_bc(&s("tag"));
            Value::Null
        }
        "delete" => {
            m.set_selection(strs(&op["ids"]));
            m.delete_selection();
            Value::Null
        }
        "import_points" => {
            let pf = parse_point_file(op["text"].as_str().unwrap(), "points")?;
            let o = &op["options"];
            let opts = ImportOptions {
                scale: o.get("scale").and_then(|v| v.as_f64()).unwrap_or(1.0),
                angle_deg: o.get("angle_deg").and_then(|v| v.as_f64()).unwrap_or(0.0),
                dx: o.get("dx").and_then(|v| v.as_f64()).unwrap_or(0.0),
                dy: o.get("dy").and_then(|v| v.as_f64()).unwrap_or(0.0),
                ..Default::default()
            };
            json!(m.add_curves(build_curves(&pf, &opts)?, "Import"))
        }
        other => panic!("unknown op {other}"),
    })
}

#[test]
fn operations_match_python() {
    if emesh_core::golden::skip() {
        return;
    }
    let mut names: Vec<_> = std::fs::read_dir(dir().join("expected/ops")).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    let mut failures = vec![];
    for path in &names {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let case = read_json(path);
        let mut m = SketchModel::new(Document::from_json(&case["input"]).unwrap());
        match run(&mut m, &case["op"]) {
            Err(e) => {
                if case["error"].as_str() != Some(e.as_str()) {
                    failures.push(format!("{name}: error {e:?}, python {}", case["error"]));
                }
            }
            Ok(ret) => {
                if !case["error"].is_null() {
                    failures.push(format!("{name}: no error, python: {}", case["error"]));
                    continue;
                }
                let mut d = vec![];
                emesh_core::golden::diff(&m.doc.to_json(), &case["output"], emesh_core::golden::tol(1e-9), "", &mut d);
                emesh_core::golden::diff(&ret, &case["returns"], emesh_core::golden::tol(1e-9), "returns", &mut d);
                if !d.is_empty() {
                    failures.push(format!("{name}:\n    {}", d.join("\n    ")));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} of {} operations differ:\n{}", failures.len(), names.len(), failures.join("\n"));
}
