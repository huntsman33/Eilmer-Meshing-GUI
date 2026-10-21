//! Helpers for the golden-file tests (Python reference outputs in `golden/`).
#![doc(hidden)]

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The repository's `golden/` folder.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../golden")
}

/// Corpus project names (sorted), plus the local ones if `golden/local/` exists.
pub fn corpus() -> Vec<(String, PathBuf, PathBuf)> {
    let mut out = vec![];
    for (corpus, expected) in [("corpus", "expected"), ("local/corpus", "local/expected")] {
        let Ok(rd) = std::fs::read_dir(dir().join(corpus)) else { continue };
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.ends_with(".emesh.json"))
            .collect();
        names.sort();
        for n in names {
            let name = n.trim_end_matches(".emesh.json").to_string();
            out.push((name.clone(), dir().join(corpus).join(&n), dir().join(expected).join(&name)));
        }
    }
    out
}

pub fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())))
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Differences between two JSON values, numbers compared with a relative tolerance.
pub fn diff(a: &Value, b: &Value, tol: f64, path: &str, out: &mut Vec<String>) {
    if out.len() > 20 {
        return;
    }
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() > tol * 1.0f64.max(x.abs()).max(y.abs()) {
                out.push(format!("{path}: {x:?} vs {y:?}"));
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{path}: length {} vs {}", x.len(), y.len()));
                return;
            }
            for (k, (u, v)) in x.iter().zip(y).enumerate() {
                diff(u, v, tol, &format!("{path}[{k}]"), out);
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for (k, u) in x {
                match y.get(k) {
                    Some(v) => diff(u, v, tol, &format!("{path}.{k}"), out),
                    None => out.push(format!("{path}.{k}: missing on the right")),
                }
            }
            for k in y.keys() {
                if !x.contains_key(k) {
                    out.push(format!("{path}.{k}: missing on the left"));
                }
            }
        }
        _ => {
            if a != b {
                out.push(format!("{path}: {a} vs {b}"));
            }
        }
    }
}

pub fn assert_close(name: &str, got: &Value, want: &Value, tol: f64) {
    let mut out = vec![];
    diff(got, want, tol, "", &mut out);
    assert!(out.is_empty(), "{name}: {} difference(s):\n  {}", out.len(), out.join("\n  "));
}

/// Bit-for-bit parity (EMESH_GOLDEN_STRICT=1). The reference numbers come from CPython, numpy
/// and glibc's libm; another platform's libm can differ in the last bit, so by default numbers
/// only need to agree to 1e-12 (relative).
pub fn strict() -> bool {
    std::env::var("EMESH_GOLDEN_STRICT").is_ok_and(|v| v == "1")
}

/// Skip the parity tests (EMESH_GOLDEN_SKIP=1), e.g. on platforms whose libm differs too much.
pub fn skip() -> bool {
    std::env::var("EMESH_GOLDEN_SKIP").is_ok_and(|v| v == "1")
}

fn tokens(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let is_num = |c: char| c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '-' || c == '+';
    for ch in s.chars() {
        if is_num(ch) {
            cur.push(ch);
        } else {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(ch.to_string());
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Ok if the texts are identical, or (not strict) identical apart from numbers within 1e-12.
pub fn same_text(got: &str, want: &str) -> Result<(), String> {
    if got == want {
        return Ok(());
    }
    if strict() {
        return Err("texts differ (strict)".into());
    }
    let (a, b) = (tokens(got), tokens(want));
    if a.len() != b.len() {
        return Err(format!("texts differ in structure ({} vs {} tokens)", a.len(), b.len()));
    }
    for (x, y) in a.iter().zip(&b) {
        if x == y {
            continue;
        }
        match (x.parse::<f64>(), y.parse::<f64>()) {
            (Ok(u), Ok(v)) if (u - v).abs() <= 1e-12 * 1.0f64.max(u.abs()).max(v.abs()) => {}
            _ => return Err(format!("'{x}' vs '{y}'")),
        }
    }
    Ok(())
}

/// The numeric tolerance for documents and grids: 0 when strict.
pub fn tol(loose: f64) -> f64 {
    if strict() {
        0.0
    } else {
        loose
    }
}
