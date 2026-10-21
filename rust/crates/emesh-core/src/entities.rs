//! Sketch curve entities.
//!
//! Each curve maps one-to-one onto an Eilmer Lua path. Points are (x, y); z is always 0 in 2D.
//! A curve's kind is one of
//!   geometry    solid line: the domain's boundaries and walls; may carry a BC tag
//!   split       dash-dot line: an interface between blocks (Eilmer joins blocks across it)
//!   reference   dashed line: a drawing aid (mirror axes, offset sources...); the mesher ignores it

use serde_json::{json, Map, Value};

use crate::num::P;
use crate::pyfmt::repr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Geometry,
    Reference,
    Split,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Geometry => "geometry",
            Kind::Reference => "reference",
            Kind::Split => "split",
        }
    }

    pub fn parse(s: &str) -> Result<Kind, String> {
        match s {
            "geometry" => Ok(Kind::Geometry),
            "reference" => Ok(Kind::Reference),
            "split" => Ok(Kind::Split),
            _ => Err(format!("unknown curve kind '{s}'")),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    Line { p0: P, p1: P },
    /// Circular arc from p0 to p1 about centre, sweeping less than pi (as Eilmer's Arc).
    Arc { p0: P, p1: P, centre: P },
    Bezier { points: Vec<P> },
    /// Interpolating cubic spline through points (Eilmer Spline). A trimmed spline keeps its
    /// points and covers parameters [t0, t1] (Eilmer SubRangedPath).
    Spline { points: Vec<P>, t0: f64, t1: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CurveType {
    Line,
    Arc,
    Bezier,
    Spline,
}

impl CurveType {
    pub fn as_str(self) -> &'static str {
        match self {
            CurveType::Line => "line",
            CurveType::Arc => "arc",
            CurveType::Bezier => "bezier",
            CurveType::Spline => "spline",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub id: String,
    pub kind: Kind,
    /// empty -> untagged -> Eilmer default WallBC_WithSlip
    pub bc_tag: String,
    pub geom: Geom,
}

/// A handle name: "start", "end", "mid" (arc) or "i:<k>".
pub type Handle = String;

pub fn pt(p: P) -> P {
    [p[0], p[1]]
}

impl Curve {
    pub fn new(geom: Geom) -> Curve {
        Curve { id: String::new(), kind: Kind::Geometry, bc_tag: String::new(), geom }
    }

    pub fn line(p0: P, p1: P) -> Curve {
        Curve::new(Geom::Line { p0, p1 })
    }

    pub fn arc(p0: P, p1: P, centre: P) -> Curve {
        Curve::new(Geom::Arc { p0, p1, centre })
    }

    pub fn bezier(points: Vec<P>) -> Curve {
        Curve::new(Geom::Bezier { points })
    }

    pub fn spline(points: Vec<P>) -> Curve {
        Curve::new(Geom::Spline { points, t0: 0.0, t1: 1.0 })
    }

    pub fn with_kind(mut self, kind: Kind) -> Curve {
        self.kind = kind;
        self
    }

    pub fn with_id(mut self, id: &str) -> Curve {
        self.id = id.to_string();
        self
    }

    pub fn with_tag(mut self, tag: &str) -> Curve {
        self.bc_tag = tag.to_string();
        self
    }

    pub fn ctype(&self) -> CurveType {
        match self.geom {
            Geom::Line { .. } => CurveType::Line,
            Geom::Arc { .. } => CurveType::Arc,
            Geom::Bezier { .. } => CurveType::Bezier,
            Geom::Spline { .. } => CurveType::Spline,
        }
    }

    pub fn type_name(&self) -> &'static str {
        self.ctype().as_str()
    }

    pub fn is_line(&self) -> bool {
        matches!(self.geom, Geom::Line { .. })
    }

    pub fn is_arc(&self) -> bool {
        matches!(self.geom, Geom::Arc { .. })
    }

    /// (p0, p1) of a line or arc.
    pub fn ends_lp(&self) -> (P, P) {
        match &self.geom {
            Geom::Line { p0, p1 } | Geom::Arc { p0, p1, .. } => (*p0, *p1),
            _ => panic!("not a line or arc"),
        }
    }

    pub fn centre(&self) -> Option<P> {
        match &self.geom {
            Geom::Arc { centre, .. } => Some(*centre),
            _ => None,
        }
    }

    pub fn points(&self) -> Option<&Vec<P>> {
        match &self.geom {
            Geom::Bezier { points } | Geom::Spline { points, .. } => Some(points),
            _ => None,
        }
    }

    pub fn spline_range(&self) -> (f64, f64) {
        match &self.geom {
            Geom::Spline { t0, t1, .. } => (*t0, *t1),
            _ => (0.0, 1.0),
        }
    }

    pub fn trimmed(&self) -> bool {
        let (t0, t1) = self.spline_range();
        t0 != 0.0 || t1 != 1.0
    }

    pub fn start(&self) -> P {
        match &self.geom {
            Geom::Line { p0, .. } | Geom::Arc { p0, .. } => *p0,
            Geom::Bezier { points } => points[0],
            Geom::Spline { points, t0, .. } => {
                if *t0 == 0.0 {
                    points[0]
                } else {
                    crate::geom::point_at(self, 0.0)
                }
            }
        }
    }

    pub fn end(&self) -> P {
        match &self.geom {
            Geom::Line { p1, .. } | Geom::Arc { p1, .. } => *p1,
            Geom::Bezier { points } => *points.last().unwrap(),
            Geom::Spline { points, t1, .. } => {
                if *t1 == 1.0 {
                    *points.last().unwrap()
                } else {
                    crate::geom::point_at(self, 1.0)
                }
            }
        }
    }

    /// Copy with fn applied to every defining point (e.g. a translation).
    pub fn map_points(&self, f: impl Fn(P) -> P) -> Curve {
        let geom = match &self.geom {
            Geom::Line { p0, p1 } => Geom::Line { p0: f(*p0), p1: f(*p1) },
            Geom::Arc { p0, p1, centre } => Geom::Arc { p0: f(*p0), p1: f(*p1), centre: f(*centre) },
            Geom::Bezier { points } => Geom::Bezier { points: points.iter().map(|p| f(*p)).collect() },
            Geom::Spline { points, t0, t1 } => {
                Geom::Spline { points: points.iter().map(|p| f(*p)).collect(), t0: *t0, t1: *t1 }
            }
        };
        Curve { geom, ..self.clone() }
    }

    /// Editable points as (name, point).
    pub fn handles(&self) -> Vec<(Handle, P)> {
        match &self.geom {
            Geom::Line { p0, p1 } => vec![("start".into(), *p0), ("end".into(), *p1)],
            Geom::Arc { p0, p1, .. } => vec![
                ("start".into(), *p0),
                ("mid".into(), crate::geom::arc_point(self, 0.5)),
                ("end".into(), *p1),
            ],
            Geom::Bezier { points } => point_list_handles(points),
            Geom::Spline { points, .. } => {
                if self.trimmed() {
                    points.iter().enumerate().map(|(k, p)| (format!("i:{k}"), *p)).collect()
                } else {
                    point_list_handles(points)
                }
            }
        }
    }

    pub fn has_handle(&self, name: &str) -> bool {
        self.handles().iter().any(|(n, _)| n == name)
    }

    /// Copy with one handle moved, or None if that would make the curve invalid.
    pub fn with_handle(&self, name: &str, p: P) -> Option<Curve> {
        let p = pt(p);
        match &self.geom {
            Geom::Line { p0, p1 } => {
                let (a, b) = if name == "start" { (p, *p1) } else { (*p0, p) };
                if a == b {
                    None
                } else {
                    Some(Curve { geom: Geom::Line { p0: a, p1: b }, ..self.clone() })
                }
            }
            Geom::Arc { p0, p1, .. } => {
                let mut start = *p0;
                let mut mid = crate::geom::arc_point(self, 0.5);
                let mut end = *p1;
                match name {
                    "start" => start = p,
                    "mid" => mid = p,
                    _ => end = p,
                }
                let pieces = crate::geom::arcs_through_3_points(start, mid, end)?;
                if pieces.len() != 1 {
                    return None;
                }
                let (a, b, c) = pieces[0];
                Some(Curve { geom: Geom::Arc { p0: a, p1: b, centre: c }, ..self.clone() })
            }
            Geom::Bezier { points } => Some(Curve {
                geom: Geom::Bezier { points: point_list_with_handle(points, name, p) },
                ..self.clone()
            }),
            Geom::Spline { points, t0, t1 } => {
                let pts = point_list_with_handle(points, name, p);
                if pts.windows(2).any(|w| w[0] == w[1]) {
                    return None; // coincident neighbours break the spline
                }
                Some(Curve { geom: Geom::Spline { points: pts, t0: *t0, t1: *t1 }, ..self.clone() })
            }
        }
    }

    pub fn lua_class(&self) -> &'static str {
        match self.geom {
            Geom::Line { .. } => "Line",
            Geom::Arc { .. } => "Arc",
            Geom::Bezier { .. } => "Bezier",
            Geom::Spline { .. } => "Spline",
        }
    }

    /// Complete Lua constructor expression for this path, with points named by name_of.
    pub fn lua_expr(&self, name_of: &mut dyn FnMut(P) -> String) -> String {
        let args = match &self.geom {
            Geom::Line { p0, p1 } => format!("p0={}, p1={}", name_of(*p0), name_of(*p1)),
            Geom::Arc { p0, p1, centre } => {
                let a = name_of(*p0);
                let b = name_of(*p1);
                let c = name_of(*centre);
                format!("p0={a}, p1={b}, centre={c}")
            }
            Geom::Bezier { points } | Geom::Spline { points, .. } => {
                let names: Vec<String> = points.iter().map(|p| name_of(*p)).collect();
                format!("points={{{}}}", names.join(", "))
            }
        };
        let expr = format!("{}:new{{{}}}", self.lua_class(), args);
        if let Geom::Spline { t0, t1, .. } = &self.geom {
            if self.trimmed() {
                return format!("SubRangedPath:new{{underlying_path={expr}, t0={}, t1={}}}", repr(*t0), repr(*t1));
            }
        }
        expr
    }

    pub fn to_json(&self) -> Value {
        let mut d = Map::new();
        d.insert("id".into(), json!(self.id));
        d.insert("type".into(), json!(self.type_name()));
        d.insert("kind".into(), json!(self.kind.as_str()));
        if !self.bc_tag.is_empty() {
            d.insert("bc_tag".into(), json!(self.bc_tag));
        }
        match &self.geom {
            Geom::Line { p0, p1 } => {
                d.insert("p0".into(), pjson(*p0));
                d.insert("p1".into(), pjson(*p1));
            }
            Geom::Arc { p0, p1, centre } => {
                d.insert("p0".into(), pjson(*p0));
                d.insert("p1".into(), pjson(*p1));
                d.insert("centre".into(), pjson(*centre));
            }
            Geom::Bezier { points } => {
                d.insert("points".into(), Value::Array(points.iter().map(|p| pjson(*p)).collect()));
            }
            Geom::Spline { points, t0, t1 } => {
                d.insert("points".into(), Value::Array(points.iter().map(|p| pjson(*p)).collect()));
                if self.trimmed() {
                    d.insert("t0".into(), json!(t0));
                    d.insert("t1".into(), json!(t1));
                }
            }
        }
        Value::Object(d)
    }

    pub fn from_json(d: &Value) -> Result<Curve, String> {
        let s = |k: &str| d.get(k).and_then(|v| v.as_str()).map(|v| v.to_string());
        let id = s("id").ok_or("curve without an id")?;
        let typ = s("type").ok_or("curve without a type")?;
        let kind = Kind::parse(&s("kind").unwrap_or_else(|| "geometry".into()))?;
        let bc_tag = s("bc_tag").unwrap_or_default();
        let p = |k: &str| -> Result<P, String> { parse_point(d.get(k).ok_or(format!("curve {id}: no {k}"))?) };
        let points = || -> Result<Vec<P>, String> {
            d.get("points")
                .and_then(|v| v.as_array())
                .ok_or(format!("curve {id}: no points"))?
                .iter()
                .map(parse_point)
                .collect()
        };
        let geom = match typ.as_str() {
            "line" => Geom::Line { p0: p("p0")?, p1: p("p1")? },
            "arc" => Geom::Arc { p0: p("p0")?, p1: p("p1")?, centre: p("centre")? },
            "bezier" => Geom::Bezier { points: points()? },
            "spline" => Geom::Spline {
                points: points()?,
                t0: d.get("t0").and_then(|v| v.as_f64()).unwrap_or(0.0),
                t1: d.get("t1").and_then(|v| v.as_f64()).unwrap_or(1.0),
            },
            other => return Err(format!("unknown curve type '{other}'")),
        };
        Ok(Curve { id, kind, bc_tag, geom })
    }

    /// Hashable description of the shape, used to cache samples.
    pub fn geom_key(&self) -> Vec<u64> {
        let mut k = vec![self.ctype() as u64];
        let mut push = |p: &P| {
            k.push(p[0].to_bits());
            k.push(p[1].to_bits());
        };
        match &self.geom {
            Geom::Line { p0, p1 } => {
                push(p0);
                push(p1);
            }
            Geom::Arc { p0, p1, centre } => {
                push(p0);
                push(p1);
                push(centre);
            }
            Geom::Bezier { points } => points.iter().for_each(&mut push),
            Geom::Spline { points, t0, t1 } => {
                points.iter().for_each(&mut push);
                k.push(t0.to_bits());
                k.push(t1.to_bits());
            }
        }
        k
    }
}

pub fn pjson(p: P) -> Value {
    json!([p[0], p[1]])
}

pub fn parse_point(v: &Value) -> Result<P, String> {
    let a = v.as_array().ok_or("a point must be [x, y]")?;
    if a.len() < 2 {
        return Err("a point must be [x, y]".into());
    }
    let x = a[0].as_f64().ok_or("a point coordinate must be a number")?;
    let y = a[1].as_f64().ok_or("a point coordinate must be a number")?;
    Ok([x, y])
}

fn point_list_handles(points: &[P]) -> Vec<(Handle, P)> {
    let mut out = vec![("start".to_string(), points[0])];
    for (k, p) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        out.push((format!("i:{k}"), *p));
    }
    out.push(("end".to_string(), *points.last().unwrap()));
    out
}

fn point_list_with_handle(points: &[P], name: &str, p: P) -> Vec<P> {
    let mut pts = points.to_vec();
    match name {
        "start" => pts[0] = p,
        "end" => *pts.last_mut().unwrap() = p,
        _ => {
            if let Some(k) = name.strip_prefix("i:").and_then(|s| s.parse::<usize>().ok()) {
                if k < pts.len() {
                    pts[k] = p;
                }
            }
        }
    }
    pts
}
