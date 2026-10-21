use emesh_core::{topology::{Topology, signed_area}, Document, golden::read_json};
fn main() {
    let doc = Document::load(std::path::Path::new("../golden/corpus/cylinder_ogrid_hole.emesh.json")).unwrap();
    let t = Topology::new(&doc);
    let g = read_json(std::path::Path::new("../golden/expected/cylinder_ogrid_hole/topology.json"));
    for (k, r) in g["regions"].as_array().unwrap().iter().enumerate() {
        let poly: Vec<[f64;2]> = r["polygon"].as_array().unwrap().iter().map(|p| [p[0].as_f64().unwrap(), p[1].as_f64().unwrap()]).collect();
        let lp: Vec<usize> = r["loop"].as_array().unwrap().iter().map(|e| e[0].as_u64().unwrap() as usize).collect();
        let mine = t.regions.iter().find(|m| m.loop_.iter().map(|x| x.0).collect::<Vec<_>>() == lp).unwrap();
        let same = mine.polygon.iter().zip(&poly).filter(|(a, b)| a != b).count();
        println!("{k} python {:?} rust-on-python-poly {:?} differing pts {}", r["area"].as_f64().unwrap(), signed_area(&poly), same);
    }
}
