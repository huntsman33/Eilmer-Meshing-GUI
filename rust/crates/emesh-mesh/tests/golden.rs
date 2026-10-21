//! Preview grids within 1e-12 of the Python reference, and the same cell-quality verdicts.

use emesh_core::blocking;
use emesh_core::golden::{corpus, read_json};
use emesh_core::topology::Topology;
use emesh_core::Document;
use emesh_mesh::preview::block_grid;
use emesh_mesh::quality::{assess, intentional_aspect, DEFAULT_THRESHOLDS};

#[test]
fn grids_and_quality_match_python() {
    if emesh_core::golden::skip() {
        return;
    }
    let mut worst = 0.0f64;
    for (name, path, exp) in corpus() {
        let doc = Document::load(&path).unwrap();
        let res = blocking::resolve(&doc, &Topology::new(&doc));
        let md = doc.with_blocks(res.blocks.clone());
        let want = read_json(&exp.join("grids.json"));
        let mut grids = vec![];
        for b in &res.blocks {
            let g = block_grid(&md, b);
            let w = want[&b.id].as_array().unwrap();
            assert_eq!(w.len(), g.niv, "{name} {}", b.id);
            for i in 0..g.niv {
                let row = w[i].as_array().unwrap();
                assert_eq!(row.len(), g.njv);
                for j in 0..g.njv {
                    let p = g.at(i, j);
                    for c in 0..2 {
                        let v = row[j][c].as_f64().unwrap();
                        let err = (p[c] - v).abs() / 1.0f64.max(v.abs());
                        worst = worst.max(err);
                        assert!(err <= emesh_core::golden::tol(1e-12), "{name} {} ({i}, {j}): {:?} vs {v}", b.id, p[c]);
                    }
                }
            }
            grids.push((b.id.clone(), g));
        }
        let q = assess(0, &grids, DEFAULT_THRESHOLDS, &intentional_aspect(&res.blocks));
        let wq = read_json(&exp.join("quality.json"));
        assert_eq!(q.level().as_str(), wq["level"].as_str().unwrap(), "{name}");
        assert_eq!(q.summary(3), wq["summary"].as_str().unwrap(), "{name}");
        for (bid, b) in &q.blocks {
            let w = &wq["blocks"][bid];
            assert_eq!(b.skew_warn as u64, w["skew_warn"].as_u64().unwrap(), "{name} {bid}");
            assert_eq!(b.skew_bad as u64, w["skew_bad"].as_u64().unwrap(), "{name} {bid}");
            assert_eq!(b.ar_warn as u64, w["ar_warn"].as_u64().unwrap(), "{name} {bid}");
            assert_eq!(b.inverted as u64, w["inverted"].as_u64().unwrap(), "{name} {bid}");
        }
    }
    eprintln!("worst relative grid difference {worst:e}");
}
