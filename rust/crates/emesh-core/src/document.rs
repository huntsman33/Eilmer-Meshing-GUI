//! The project document: the single source of truth that mesh.lua is generated from.
//!
//! A Block is a four-sided CoonsPatch region. Each face is an ordered list of CurveRefs; a face
//! with several curves becomes an Eilmer Polyline, and a curve traversed backwards is wrapped in
//! a ReversedPath. Orientation follows Eilmer's CoonsPatch convention: south and north run
//! west->east, west and east run south->north.
//!
//! The JSON format is the Python version's `.emesh.json` (version 2), so either version opens
//! the other's projects.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde_json::{json, Map, Value};

use crate::entities::{pjson, Curve, Kind};
use crate::num::P;
use crate::pyfmt;

pub const CORNER_TOL: f64 = 1.0e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Face {
    North,
    East,
    South,
    West,
}

pub const FACES: [Face; 4] = [Face::North, Face::East, Face::South, Face::West];

impl Face {
    pub fn as_str(self) -> &'static str {
        match self {
            Face::North => "north",
            Face::East => "east",
            Face::South => "south",
            Face::West => "west",
        }
    }

    pub fn parse(s: &str) -> Option<Face> {
        match s {
            "north" => Some(Face::North),
            "east" => Some(Face::East),
            "south" => Some(Face::South),
            "west" => Some(Face::West),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn opposite(self) -> Face {
        match self {
            Face::South => Face::North,
            Face::North => Face::South,
            Face::East => Face::West,
            Face::West => Face::East,
        }
    }

    /// 'i' for south/north (niv), 'j' for east/west (njv)
    pub fn axis(self) -> char {
        match self {
            Face::South | Face::North => 'i',
            Face::East | Face::West => 'j',
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockError(pub String);

impl std::fmt::Display for BlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BlockError {}

pub fn block_err<T>(msg: impl Into<String>) -> Result<T, BlockError> {
    Err(BlockError(msg.into()))
}

/// A curve, or the part of it between parameters t0 < t1, traversed forwards or reversed.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveRef {
    pub id: String,
    pub reversed: bool,
    pub t0: f64,
    pub t1: f64,
}

impl CurveRef {
    pub fn new(id: &str, reversed: bool, t0: f64, t1: f64) -> CurveRef {
        CurveRef { id: id.to_string(), reversed, t0, t1 }
    }

    pub fn full(&self) -> bool {
        self.t0 == 0.0 && self.t1 == 1.0
    }

    pub fn to_json(&self) -> Value {
        let mut d = Map::new();
        d.insert("id".into(), json!(self.id));
        if self.reversed {
            d.insert("reversed".into(), json!(true));
        }
        if !self.full() {
            d.insert("t0".into(), json!(self.t0));
            d.insert("t1".into(), json!(self.t1));
        }
        Value::Object(d)
    }

    pub fn from_json(d: &Value) -> Result<CurveRef, String> {
        Ok(CurveRef {
            id: d.get("id").and_then(|v| v.as_str()).ok_or("curve reference without an id")?.to_string(),
            reversed: d.get("reversed").and_then(|v| v.as_bool()).unwrap_or(false),
            t0: d.get("t0").and_then(|v| v.as_f64()).unwrap_or(0.0),
            t1: d.get("t1").and_then(|v| v.as_f64()).unwrap_or(1.0),
        })
    }
}

/// A clustering spec, kept as the JSON object it is stored as (key order matters for Lua).
/// Types: RobertsFunction {end0, end1, beta}; Growth {first, reverse}; Grade {first, growth,
/// reverse}; and the concrete forms growth {r, n, reverse} and table {x}.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Cluster(pub Map<String, Value>);

impl Cluster {
    pub fn kind(&self) -> &str {
        self.0.get("type").and_then(|v| v.as_str()).unwrap_or("")
    }

    pub fn f(&self, key: &str) -> Option<f64> {
        self.0.get(key).and_then(|v| v.as_f64())
    }

    pub fn b(&self, key: &str) -> bool {
        self.0.get(key).map(truthy).unwrap_or(false)
    }

    pub fn set(&mut self, key: &str, v: Value) {
        self.0.insert(key.to_string(), v);
    }

    pub fn from_value(v: &Value) -> Option<Cluster> {
        v.as_object().map(|m| Cluster(m.clone()))
    }

    pub fn roberts(end0: bool, end1: bool, beta: f64) -> Cluster {
        let mut m = Map::new();
        m.insert("type".into(), json!("RobertsFunction"));
        m.insert("end0".into(), json!(end0));
        m.insert("end1".into(), json!(end1));
        m.insert("beta".into(), json!(beta));
        Cluster(m)
    }

    pub fn growth(first: f64, reverse: bool) -> Cluster {
        let mut m = Map::new();
        m.insert("type".into(), json!("Growth"));
        m.insert("first".into(), json!(first));
        m.insert("reverse".into(), json!(reverse));
        Cluster(m)
    }

    pub fn grade(first: f64, growth: f64, reverse: bool) -> Cluster {
        let mut m = Map::new();
        m.insert("type".into(), json!("Grade"));
        m.insert("first".into(), json!(first));
        m.insert("growth".into(), json!(growth));
        m.insert("reverse".into(), json!(reverse));
        Cluster(m)
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A block made from a sketch region: the curve ids around it (CCW) and 4 corner positions.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionSpec {
    pub loop_ids: Vec<String>,
    pub corners: Vec<usize>,
}

impl RegionSpec {
    pub fn to_json(&self) -> Value {
        json!({"loop": self.loop_ids, "corners": self.corners})
    }

    pub fn from_json(v: &Value) -> Result<RegionSpec, String> {
        let loop_ids = v
            .get("loop")
            .and_then(|l| l.as_array())
            .ok_or("block region without a loop")?
            .iter()
            .map(|s| s.as_str().map(|s| s.to_string()).ok_or("bad loop id"))
            .collect::<Result<Vec<_>, _>>()?;
        let corners = v
            .get("corners")
            .and_then(|l| l.as_array())
            .ok_or("block region without corners")?
            .iter()
            .map(|s| s.as_u64().map(|k| k as usize).ok_or("bad corner"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(RegionSpec { loop_ids, corners })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: String,
    pub sides: [Vec<CurveRef>; 4], // indexed by Face
    pub niv: usize,
    pub njv: usize,
    /// "CoonsPatch" or "AOPatch"
    pub patch: String,
    /// e.g. {"nx": 10, "ny": 10} for AOPatch
    pub patch_opts: Map<String, Value>,
    /// e.g. {"nsweeps": 500, "tol": 1e-10}; None -> unsmoothed
    pub smoothing: Option<Map<String, Value>>,
    pub cluster: IndexMap<Face, Cluster>,
    pub fs_tag: String,
    pub region: Option<RegionSpec>,
}

pub fn default_smoothing() -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("nsweeps".into(), json!(500));
    m.insert("tol".into(), json!(1.0e-10));
    m
}

impl Block {
    pub fn new(id: &str) -> Block {
        Block {
            id: id.to_string(),
            sides: Default::default(),
            niv: 21,
            njv: 21,
            patch: "CoonsPatch".into(),
            patch_opts: Map::new(),
            smoothing: None,
            cluster: IndexMap::new(),
            fs_tag: "initial".into(),
            region: None,
        }
    }

    pub fn side(&self, f: Face) -> &Vec<CurveRef> {
        &self.sides[f.index()]
    }

    pub fn smoothed(&self) -> bool {
        self.smoothing.as_ref().is_some_and(|m| !m.is_empty())
    }

    pub fn count(&self, f: Face) -> usize {
        if f.axis() == 'i' {
            self.niv
        } else {
            self.njv
        }
    }

    pub fn to_json(&self) -> Value {
        let mut d = Map::new();
        d.insert("id".into(), json!(self.id));
        d.insert("niv".into(), json!(self.niv));
        d.insert("njv".into(), json!(self.njv));
        d.insert("patch".into(), json!(self.patch));
        d.insert("fs_tag".into(), json!(self.fs_tag));
        if let Some(r) = &self.region {
            d.insert("region".into(), r.to_json());
        } else {
            d.insert("sides".into(), self.sides_json());
        }
        if !self.patch_opts.is_empty() {
            d.insert("patch_opts".into(), Value::Object(self.patch_opts.clone()));
        }
        if let Some(s) = &self.smoothing {
            if !s.is_empty() {
                d.insert("smoothing".into(), Value::Object(s.clone()));
            }
        }
        if !self.cluster.is_empty() {
            let m: Map<String, Value> =
                self.cluster.iter().map(|(f, c)| (f.as_str().to_string(), Value::Object(c.0.clone()))).collect();
            d.insert("cluster".into(), Value::Object(m));
        }
        Value::Object(d)
    }

    pub fn sides_json(&self) -> Value {
        let m: Map<String, Value> = FACES
            .iter()
            .map(|f| (f.as_str().to_string(), Value::Array(self.side(*f).iter().map(|r| r.to_json()).collect())))
            .collect();
        Value::Object(m)
    }

    pub fn from_json(d: &Value) -> Result<Block, String> {
        let mut b = Block::new(d.get("id").and_then(|v| v.as_str()).ok_or("block without an id")?);
        for f in FACES {
            if let Some(refs) = d.get("sides").and_then(|s| s.get(f.as_str())).and_then(|v| v.as_array()) {
                b.sides[f.index()] = refs.iter().map(CurveRef::from_json).collect::<Result<_, _>>()?;
            }
        }
        b.niv = as_count(d.get("niv")).ok_or("block without niv")?;
        b.njv = as_count(d.get("njv")).ok_or("block without njv")?;
        if let Some(p) = d.get("patch").and_then(|v| v.as_str()) {
            b.patch = p.to_string();
        }
        if let Some(o) = d.get("patch_opts").and_then(|v| v.as_object()) {
            b.patch_opts = o.clone();
        }
        b.smoothing = d.get("smoothing").and_then(|v| v.as_object()).cloned();
        if let Some(c) = d.get("cluster").and_then(|v| v.as_object()) {
            for (k, v) in c {
                if let (Some(f), Some(cl)) = (Face::parse(k), Cluster::from_value(v)) {
                    b.cluster.insert(f, cl);
                }
            }
        }
        if let Some(t) = d.get("fs_tag").and_then(|v| v.as_str()) {
            b.fs_tag = t.to_string();
        }
        if let Some(r) = d.get("region") {
            if !r.is_null() {
                b.region = Some(RegionSpec::from_json(r)?);
            }
        }
        Ok(b)
    }

    pub fn curve_ids(&self) -> Vec<String> {
        if let Some(r) = &self.region {
            let mut v = r.loop_ids.clone();
            v.sort();
            v.dedup();
            return v;
        }
        let mut v: Vec<String> = FACES.iter().flat_map(|f| self.side(*f).iter().map(|r| r.id.clone())).collect();
        v.sort();
        v.dedup();
        v
    }
}

fn as_count(v: Option<&Value>) -> Option<usize> {
    let v = v?;
    v.as_u64().map(|n| n as usize).or_else(|| v.as_f64().map(|f| f as usize))
}

/// A dimension on one curve ("length" of a line, "angle" of an arc), or with curve2 the
/// "distance" between parallel lines or "angle" between two lines (see dimensions.rs).
#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    pub id: String,
    pub kind: String,
    pub curve: String,
    pub offset: f64,
    pub curve2: Option<String>,
    pub sector: Option<[i32; 2]>,
}

impl Dimension {
    pub fn curve_ids(&self) -> Vec<&str> {
        match &self.curve2 {
            None => vec![self.curve.as_str()],
            Some(c2) => vec![self.curve.as_str(), c2.as_str()],
        }
    }

    pub fn to_json(&self) -> Value {
        let mut d = Map::new();
        d.insert("id".into(), json!(self.id));
        d.insert("kind".into(), json!(self.kind));
        d.insert("curve".into(), json!(self.curve));
        d.insert("offset".into(), json!(self.offset));
        if let Some(c2) = &self.curve2 {
            d.insert("curve2".into(), json!(c2));
        }
        if let Some(s) = &self.sector {
            d.insert("sector".into(), json!([s[0], s[1]]));
        }
        Value::Object(d)
    }

    pub fn from_json(d: &Value) -> Result<Dimension, String> {
        let s = |k: &str| d.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        let sector = d.get("sector").and_then(|v| v.as_array()).map(|a| {
            [a.first().and_then(|v| v.as_f64()).unwrap_or(1.0) as i32, a.get(1).and_then(|v| v.as_f64()).unwrap_or(1.0) as i32]
        });
        Ok(Dimension {
            id: s("id").ok_or("dimension without an id")?,
            kind: s("kind").ok_or("dimension without a kind")?,
            curve: s("curve").ok_or("dimension without a curve")?,
            offset: d.get("offset").and_then(|v| v.as_f64()).unwrap_or(0.0),
            curve2: s("curve2"),
            sector,
        })
    }
}

/// A boundary-layer wall: the face's curves and a point inside its block.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    pub refs: Vec<CurveRef>,
    pub inside: Option<P>,
}

/// Thin wall blocks with geometric growth away from the wall (see layers.rs).
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryLayer {
    pub id: String,
    pub walls: Vec<Wall>,
    pub first: f64,
    pub growth: f64,
    pub cells: usize,
    pub transition: bool,
    pub curves: Vec<String>,
    pub blocks: Vec<String>,
    pub base: Vec<Block>,
    /// wall curve -> [previous tag, the layer's tag]
    pub tagged: IndexMap<String, (String, String)>,
}

impl BoundaryLayer {
    pub fn new(id: &str, walls: Vec<Wall>, first: f64, growth: f64, cells: usize, transition: bool) -> Self {
        BoundaryLayer {
            id: id.into(),
            walls,
            first,
            growth,
            cells,
            transition,
            curves: vec![],
            blocks: vec![],
            base: vec![],
            tagged: IndexMap::new(),
        }
    }

    pub fn to_json(&self) -> Value {
        let walls: Vec<Value> = self
            .walls
            .iter()
            .map(|w| {
                let mut m = Map::new();
                m.insert("refs".into(), Value::Array(w.refs.iter().map(|r| r.to_json()).collect()));
                m.insert("inside".into(), w.inside.map(pjson).unwrap_or(Value::Null));
                Value::Object(m)
            })
            .collect();
        let tagged: Map<String, Value> =
            self.tagged.iter().map(|(k, (a, b))| (k.clone(), json!([a, b]))).collect();
        let mut d = Map::new();
        d.insert("id".into(), json!(self.id));
        d.insert("walls".into(), Value::Array(walls));
        d.insert("first".into(), json!(self.first));
        d.insert("growth".into(), json!(self.growth));
        d.insert("cells".into(), json!(self.cells));
        d.insert("transition".into(), json!(self.transition));
        d.insert("curves".into(), json!(self.curves));
        d.insert("blocks".into(), json!(self.blocks));
        d.insert("base".into(), Value::Array(self.base.iter().map(|b| b.to_json()).collect()));
        d.insert("tagged".into(), Value::Object(tagged));
        Value::Object(d)
    }

    pub fn from_json(d: &Value) -> Result<BoundaryLayer, String> {
        let walls = d
            .get("walls")
            .and_then(|v| v.as_array())
            .ok_or("layer without walls")?
            .iter()
            .map(|w| -> Result<Wall, String> {
                let refs = w
                    .get("refs")
                    .and_then(|v| v.as_array())
                    .ok_or("wall without refs")?
                    .iter()
                    .map(CurveRef::from_json)
                    .collect::<Result<_, _>>()?;
                let inside = match w.get("inside") {
                    Some(v) if !v.is_null() => Some(crate::entities::parse_point(v)?),
                    _ => None,
                };
                Ok(Wall { refs, inside })
            })
            .collect::<Result<_, _>>()?;
        let strs = |k: &str| -> Vec<String> {
            d.get(k)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default()
        };
        let mut tagged = IndexMap::new();
        if let Some(t) = d.get("tagged").and_then(|v| v.as_object()) {
            for (k, v) in t {
                let a = v.as_array().ok_or("bad tagged entry")?;
                let s = |i: usize| a.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
                tagged.insert(k.clone(), (s(0), s(1)));
            }
        }
        Ok(BoundaryLayer {
            id: d.get("id").and_then(|v| v.as_str()).ok_or("layer without an id")?.into(),
            walls,
            first: d.get("first").and_then(|v| v.as_f64()).ok_or("layer without first")?,
            growth: d.get("growth").and_then(|v| v.as_f64()).ok_or("layer without growth")?,
            cells: as_count(d.get("cells")).ok_or("layer without cells")?,
            transition: d.get("transition").map(truthy).unwrap_or(false),
            curves: strs("curves"),
            blocks: strs("blocks"),
            base: d
                .get("base")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().map(Block::from_json).collect::<Result<Vec<_>, _>>())
                .transpose()?
                .unwrap_or_default(),
            tagged,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bc {
    pub class: String,
    pub args: String,
}

fn close(a: P, b: P, tol: f64) -> bool {
    crate::num::py_hypot(a[0] - b[0], a[1] - b[1]) <= tol * 1.0f64.max(a[0].abs()).max(a[1].abs())
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    pub curves: IndexMap<String, Curve>,
    /// construction points (snap targets, not geometry)
    pub points: IndexMap<String, P>,
    pub dimensions: IndexMap<String, Dimension>,
    pub blocks: Vec<Block>,
    /// tag -> BC class and constructor arguments
    pub bcs: IndexMap<String, Bc>,
    /// boundary layers, in the order they were applied
    pub layers: Vec<BoundaryLayer>,
    counters: HashMap<String, u64>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.to_json() == other.to_json()
    }
}

impl Document {
    /// 2: dashed 'reference' curves became drawing aids; block interfaces are 'split' curves
    pub const VERSION: u64 = 2;

    pub fn new() -> Document {
        Document::default()
    }

    pub fn new_id(&mut self, prefix: &str) -> String {
        let mut n = *self.counters.get(prefix).unwrap_or(&0);
        loop {
            n += 1;
            let cid = format!("{prefix}{n}");
            let taken = self.curves.contains_key(&cid)
                || self.points.contains_key(&cid)
                || self.dimensions.contains_key(&cid)
                || self.blocks.iter().any(|b| b.id == cid)
                || self.layers.iter().any(|l| l.id == cid || l.base.iter().any(|b| b.id == cid));
            if !taken {
                self.counters.insert(prefix.to_string(), n);
                return cid;
            }
        }
    }

    pub fn add_curve(&mut self, curve: Curve) -> Result<(), String> {
        if self.curves.contains_key(&curve.id) {
            return Err(format!("duplicate curve id {}", curve.id));
        }
        self.curves.insert(curve.id.clone(), curve);
        Ok(())
    }

    pub fn add_block(&mut self, block: Block) {
        self.blocks.push(block);
    }

    pub fn add_point(&mut self, pid: &str, p: P) -> Result<(), String> {
        if self.points.contains_key(pid) || self.curves.contains_key(pid) {
            return Err(format!("duplicate id {pid}"));
        }
        self.points.insert(pid.to_string(), p);
        Ok(())
    }

    pub fn curve(&self, id: &str) -> &Curve {
        &self.curves[id]
    }

    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.id == id)
    }

    pub fn block_mut(&mut self, id: &str) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    /// Remove blocks with a face on any of the given curves; returns their ids.
    pub fn remove_blocks_using(&mut self, ids: &[String]) -> Vec<String> {
        let dead: Vec<String> =
            self.blocks.iter().filter(|b| b.curve_ids().iter().any(|c| ids.contains(c))).map(|b| b.id.clone()).collect();
        self.blocks.retain(|b| !dead.contains(&b.id));
        dead
    }

    /// A copy with a different block list (e.g. resolved region blocks for codegen).
    pub fn with_blocks(&self, blocks: Vec<Block>) -> Document {
        Document {
            curves: self.curves.clone(),
            points: self.points.clone(),
            dimensions: self.dimensions.clone(),
            blocks,
            bcs: self.bcs.clone(),
            layers: self.layers.clone(),
            counters: self.counters.clone(),
        }
    }

    /// Remove curves/points and any blocks that reference them; returns the removed block ids.
    pub fn remove_curves(&mut self, ids: &[String]) -> Vec<String> {
        for cid in ids {
            self.curves.shift_remove(cid);
            self.points.shift_remove(cid);
            self.dimensions.shift_remove(cid);
        }
        let dead: Vec<String> = self
            .dimensions
            .values()
            .filter(|d| d.curve_ids().iter().any(|c| !self.curves.contains_key(*c)))
            .map(|d| d.id.clone())
            .collect();
        for did in dead {
            self.dimensions.shift_remove(&did);
        }
        self.remove_blocks_using(ids)
    }

    pub fn add_dimension(&mut self, dim: Dimension) {
        self.dimensions.insert(dim.id.clone(), dim);
    }

    // --- block geometry checks ------------------------------------------------------------
    pub fn ref_endpoints(&self, r: &CurveRef) -> Result<(P, P), BlockError> {
        let c = self.curves.get(&r.id).ok_or_else(|| BlockError(format!("curve {} does not exist", r.id)))?;
        let (a, b) = if r.full() {
            (c.start(), c.end())
        } else {
            (crate::geom::point_at(c, r.t0), crate::geom::point_at(c, r.t1))
        };
        Ok(if r.reversed { (b, a) } else { (a, b) })
    }

    pub fn face_endpoints(&self, block: &Block, face: Face) -> Result<(P, P), BlockError> {
        let refs = block.side(face);
        if refs.is_empty() {
            return block_err(format!("block {}: {} face has no curves", block.id, face.as_str()));
        }
        let (start, mut prev_end) = self.ref_endpoints(&refs[0])?;
        for r in &refs[1..] {
            let (s, e) = self.ref_endpoints(r)?;
            if !close(prev_end, s, CORNER_TOL) {
                return block_err(format!("block {}: {} face is not continuous at curve {}", block.id, face.as_str(), r.id));
            }
            prev_end = e;
        }
        Ok((start, prev_end))
    }

    pub fn validate_block(&self, block: &Block) -> Result<(), BlockError> {
        let mut ends = HashMap::new();
        for f in FACES {
            ends.insert(f, self.face_endpoints(block, f)?);
        }
        let corners = [
            ("p00 (south-west)", ends[&Face::South].0, ends[&Face::West].0),
            ("p10 (south-east)", ends[&Face::South].1, ends[&Face::East].0),
            ("p01 (north-west)", ends[&Face::North].0, ends[&Face::West].1),
            ("p11 (north-east)", ends[&Face::North].1, ends[&Face::East].1),
        ];
        for (name, a, b) in corners {
            if !close(a, b, CORNER_TOL) {
                return block_err(format!(
                    "block {}: open corner {}: {} vs {}",
                    block.id,
                    name,
                    pyfmt::point(a),
                    pyfmt::point(b)
                ));
            }
        }
        if block.niv < 2 || block.njv < 2 {
            return block_err(format!("block {}: niv and njv must be >= 2", block.id));
        }
        for f in FACES {
            self.face_bc_tag(block, f)?;
        }
        Ok(())
    }

    /// BC tag for a block face; only solid curves carry tags (split lines are internal).
    pub fn face_bc_tag(&self, block: &Block, face: Face) -> Result<String, BlockError> {
        let mut tags: Vec<String> = vec![];
        for r in block.side(face) {
            if let Some(c) = self.curves.get(&r.id) {
                if c.kind == Kind::Geometry && !c.bc_tag.is_empty() && !tags.contains(&c.bc_tag) {
                    tags.push(c.bc_tag.clone());
                }
            }
        }
        if tags.len() > 1 {
            tags.sort();
            return block_err(format!("block {}: {} face mixes BC tags {}", block.id, face.as_str(), pyfmt::str_list(&tags)));
        }
        Ok(tags.pop().unwrap_or_default())
    }

    // --- serialisation ----------------------------------------------------------------------
    pub fn to_json(&self) -> Value {
        let bcs: Map<String, Value> =
            self.bcs.iter().map(|(t, b)| (t.clone(), json!({"class": b.class, "args": b.args}))).collect();
        json!({
            "version": Self::VERSION,
            "curves": self.curves.values().map(|c| c.to_json()).collect::<Vec<_>>(),
            "points": self.points.iter().map(|(k, p)| json!({"id": k, "x": p[0], "y": p[1]})).collect::<Vec<_>>(),
            "dimensions": self.dimensions.values().map(|d| d.to_json()).collect::<Vec<_>>(),
            "blocks": self.blocks.iter().map(|b| b.to_json()).collect::<Vec<_>>(),
            "bcs": Value::Object(bcs),
            "layers": self.layers.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(d: &Value) -> Result<Document, String> {
        let mut doc = Document::new();
        let old = d.get("version").and_then(|v| v.as_u64()).unwrap_or(1) < 2;
        let arr = |k: &str| d.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for cd in arr("curves") {
            let mut c = Curve::from_json(&cd)?;
            if old && c.kind == Kind::Reference {
                c.kind = Kind::Split; // version 1 dashed lines split blocks
            }
            doc.add_curve(c)?;
        }
        for pd in arr("points") {
            let id = pd.get("id").and_then(|v| v.as_str()).ok_or("point without an id")?;
            let x = pd.get("x").and_then(|v| v.as_f64()).ok_or("point without x")?;
            let y = pd.get("y").and_then(|v| v.as_f64()).ok_or("point without y")?;
            doc.add_point(id, [x, y])?;
        }
        for dd in arr("dimensions") {
            doc.add_dimension(Dimension::from_json(&dd)?);
        }
        for bd in arr("blocks") {
            doc.add_block(Block::from_json(&bd)?);
        }
        if let Some(b) = d.get("bcs").and_then(|v| v.as_object()) {
            for (tag, spec) in b {
                doc.bcs.insert(
                    tag.clone(),
                    Bc {
                        class: spec.get("class").and_then(|v| v.as_str()).unwrap_or("WallBC_WithSlip").into(),
                        args: spec.get("args").and_then(|v| v.as_str()).unwrap_or("").into(),
                    },
                );
            }
        }
        for ld in arr("layers") {
            doc.layers.push(BoundaryLayer::from_json(&ld)?);
        }
        Ok(doc)
    }

    pub fn from_text(text: &str) -> Result<Document, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        Document::from_json(&v)
    }

    /// The project file text (JSON, indented as the Python version writes it).
    pub fn to_string_pretty(&self) -> String {
        crate::jsonfmt::to_python_json(&self.to_json(), 2)
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_string_pretty())
    }

    pub fn load(path: &std::path::Path) -> Result<Document, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Document::from_text(&text)
    }
}
