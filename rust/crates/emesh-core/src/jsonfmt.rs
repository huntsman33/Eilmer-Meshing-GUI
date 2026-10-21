//! JSON text exactly as Python's `json.dump(value, f, indent=n)` writes it, so project files
//! saved by either version are identical.

use serde_json::Value;

use crate::pyfmt::repr;

pub fn to_python_json(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    write(v, indent, 0, &mut out);
    out
}

fn write(v: &Value, indent: usize, level: usize, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_f64() {
                let x = n.as_f64().unwrap();
                out.push_str(&if x.is_nan() {
                    "NaN".to_string()
                } else if x.is_infinite() {
                    if x > 0.0 { "Infinity".into() } else { "-Infinity".into() }
                } else {
                    repr(x)
                });
            } else {
                out.push_str(&n.to_string());
            }
        }
        Value::String(s) => write_str(s, out),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (k, item) in a.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                newline(indent, level + 1, out);
                write(item, indent, level + 1, out);
            }
            newline(indent, level, out);
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (k, (key, item)) in m.iter().enumerate() {
                if k > 0 {
                    out.push(',');
                }
                newline(indent, level + 1, out);
                write_str(key, out);
                out.push_str(": ");
                write(item, indent, level + 1, out);
            }
            newline(indent, level, out);
            out.push('}');
        }
    }
}

fn newline(indent: usize, level: usize, out: &mut String) {
    out.push('\n');
    for _ in 0..indent * level {
        out.push(' ');
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
