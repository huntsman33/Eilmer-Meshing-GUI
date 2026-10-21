//! Boundary conditions: the Eilmer BC classes offered in the GUI, tag rules, bcs.lua, and the
//! starter simulation script written on export.
//!
//! A solid curve's bc_tag names an entry in Document.bcs. Untagged faces get Eilmer's default,
//! WallBC_WithSlip. The tags go into each grid's bcTags in mesh.lua; bcs.lua maps them to BC
//! objects (bcDict) for `lmr prep-sim`.

use indexmap::IndexMap;

use emesh_core::document::{Document, FACES};

use crate::templates::SIM_TEMPLATE;

pub const SIM_FILE: &str = "sim.lua";
pub const GAS_INPUT_FILE: &str = "ideal-air.lua";

/// (class, group, default constructor arguments, default tag, description)
pub const BC_CLASSES: &[(&str, &str, &str, &str, &str)] = &[
    ("WallBC_WithSlip", "Wall", "", "slipwall", "Inviscid wall (slip); the default for untagged faces"),
    ("WallBC_NoSlip_FixedT", "Wall", "Twall=300.0", "wall", "Viscous wall at a fixed temperature"),
    ("WallBC_NoSlip_Adiabatic", "Wall", "", "wall", "Viscous adiabatic wall"),
    ("InFlowBC_Supersonic", "Inflow", "flowState=inflow", "inflow", "Supersonic inflow: every property fixed"),
    ("InFlowBC_FromStagnation", "Inflow", "stagnationState=stagnation", "inflow", "Subsonic inflow from a stagnation (reservoir) state"),
    ("InFlowBC_StaticProfile", "Inflow", "fileName=\"inflow-profile.dat\"", "inflow", "Inflow from a profile file"),
    ("InOutFlowBC_Ambient", "In/out", "flowState=ambient", "ambient", "Ambient conditions; the flow may go in or out"),
    ("OutFlowBC_Simple", "Outflow", "", "outflow", "Supersonic outflow (extrapolates the flux)"),
    ("OutFlowBC_SimpleExtrapolate", "Outflow", "", "outflow", "Outflow extrapolating the flow state"),
    ("OutFlowBC_FixedP", "Outflow", "p_outside=101.325e3", "outflow", "Subsonic outflow at a fixed pressure"),
    ("OutFlowBC_FixedPT", "Outflow", "p_outside=101.325e3, T_outside=300.0", "outflow", "Subsonic outflow at a fixed pressure and temperature"),
    ("UserDefinedBC", "Other", "fileName=\"udf-bc.lua\"", "udf", "User-defined ghost-cell BC (Lua file)"),
];

pub fn bc_class(name: &str) -> Option<&'static (&'static str, &'static str, &'static str, &'static str, &'static str)> {
    BC_CLASSES.iter().find(|c| c.0 == name)
}

pub const TAG_COLORS: &[&str] = &["#d43f3a", "#2e9d57", "#8e5bd0", "#c9a227", "#1a73d9", "#e0679b", "#17a2b8", "#7f5a2e"];

pub const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not",
    "or", "repeat", "return", "then", "true", "until", "while",
];

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Tags are bcDict keys and bcTags strings, so they must be Lua identifiers.
pub fn check_tag(tag: &str) -> Result<String, String> {
    let mut chars = tag.chars();
    let ok = match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => chars.all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    };
    if !ok || LUA_KEYWORDS.contains(&tag) {
        return Err(format!("'{tag}' is not a valid tag: use letters, digits and _, starting with a letter"));
    }
    Ok(tag.to_string())
}

pub fn tag_color(doc: &Document, tag: &str) -> &'static str {
    let mut tags: Vec<&String> = doc.bcs.keys().collect();
    tags.sort();
    match tags.iter().position(|t| *t == tag) {
        Some(k) => TAG_COLORS[k % TAG_COLORS.len()],
        None => TAG_COLORS[0],
    }
}

/// Lua names a BC's arguments refer to (e.g. flowState=inflow -> inflow).
pub fn flow_state_names(args: &str) -> Vec<String> {
    let c: Vec<char> = args.chars().collect();
    let n = c.len();
    let mut names: Vec<String> = vec![];
    let mut pos = 0;
    while pos < n {
        // (\w+)\s*=\s*([A-Za-z_]\w*)
        let mut k = pos;
        while k < n && is_word(c[k]) {
            k += 1;
        }
        let mut matched = None;
        if k > pos {
            let mut j = k;
            while j < n && c[j].is_whitespace() {
                j += 1;
            }
            if j < n && c[j] == '=' {
                j += 1;
                while j < n && c[j].is_whitespace() {
                    j += 1;
                }
                if j < n && (c[j].is_ascii_alphabetic() || c[j] == '_') {
                    let start = j;
                    j += 1;
                    while j < n && is_word(c[j]) {
                        j += 1;
                    }
                    matched = Some((c[start..j].iter().collect::<String>(), j));
                }
            }
        }
        match matched {
            Some((value, end)) => {
                if !LUA_KEYWORDS.contains(&value.as_str()) && !names.contains(&value) {
                    names.push(value);
                }
                pos = end;
            }
            None => pos += 1,
        }
    }
    names
}

pub fn used_bc_tags(doc: &Document) -> Vec<String> {
    let mut tags: Vec<String> = vec![];
    for b in &doc.blocks {
        for f in FACES {
            if let Ok(t) = doc.face_bc_tag(b, f) {
                if !t.is_empty() && !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
    }
    tags.sort();
    tags
}

/// bcDict stub mapping each tag used in the document to its chosen BC class.
pub fn generate_bcs_lua(doc: &Document) -> String {
    let mut lines = vec![
        "-- bcs.lua".to_string(),
        "-- Generated by eilmer-mesher. Load it in your simulation script with dofile(\"bcs.lua\")".into(),
        "-- after defining the flow states it uses, then call makeFluidBlocks(bcDict, flowDict).".into(),
        "-- Untagged faces default to WallBC_WithSlip.".into(),
        "bcDict = {".into(),
    ];
    for t in used_bc_tags(doc) {
        let (class, args) = match doc.bcs.get(&t) {
            Some(b) => (b.class.clone(), b.args.clone()),
            None => ("WallBC_WithSlip".into(), String::new()),
        };
        lines.push(format!("   {t} = {class}:new{{{args}}},"));
    }
    lines.push("}".into());
    lines.join("\n") + "\n"
}

/// Python `str.format` with named slots ({{ and }} are literal braces).
fn format_named(template: &str, slots: &[(&str, &str)]) -> String {
    let c: Vec<char> = template.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '{' && i + 1 < c.len() && c[i + 1] == '{' {
            out.push('{');
            i += 2;
        } else if c[i] == '}' && i + 1 < c.len() && c[i + 1] == '}' {
            out.push('}');
            i += 2;
        } else if c[i] == '{' {
            let end = c[i..].iter().position(|&ch| ch == '}').map(|p| p + i).unwrap();
            let name: String = c[i + 1..end].iter().collect();
            out.push_str(slots.iter().find(|s| s.0 == name).map(|s| s.1).unwrap_or(""));
            i = end + 1;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

fn ljust(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

/// Starter sim.lua: 'initial', the free-stream 'inflow' (always, ready for an inflow BC), and a
/// FlowState for every other name the BCs refer to.
pub fn sim_template(doc: &Document, tags: &[String]) -> String {
    let mut users: IndexMap<String, String> = IndexMap::new();
    users.insert("inflow".into(), "-- free stream, for an inflow boundary condition".into());
    for tag in tags {
        let (class, args) = match doc.bcs.get(tag) {
            Some(b) => (b.class.clone(), b.args.clone()),
            None => ("?".into(), String::new()),
        };
        for name in flow_state_names(&args) {
            let replace = match users.get(&name) {
                None => true,
                Some(u) => name == "inflow" && !u.contains("used by"),
            };
            if name != "initial" && replace {
                users.insert(name, format!("-- used by {class} ({tag})"));
            }
        }
    }
    let states: Vec<(String, String)> = users
        .keys()
        .map(|name| {
            let vel = if name == "inflow" { ", velx=u_inf" } else { "" };
            (name.clone(), format!("{name} = FlowState:new{{p=p_inf, T=T_inf{vel}}}"))
        })
        .collect();
    let width = (states.iter().map(|(_, d)| d.chars().count()).max().unwrap_or(0) + 3).max(55);
    let lines: Vec<String> = states.iter().map(|(n, d)| ljust(d, width) + &users[n]).collect();
    let mut flow = vec!["initial=initial".to_string()];
    flow.extend(users.keys().map(|n| format!("{n}={n}")));
    format_named(SIM_TEMPLATE, &[("states", &lines.join("\n")), ("flowdict", &flow.join(", "))])
}
