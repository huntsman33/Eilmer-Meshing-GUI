//! Dialogs. Each is a small state struct whose `show` draws it as a modal window and returns
//! Some(outcome) once the user is done.

use std::collections::BTreeMap;

use egui::{Key, RichText};

use emesh_core::document::{Cluster, Document};
use emesh_core::importers::{build_curves, leading_edge_index, ImportOptions, PointFile};
use emesh_core::pyfmt;
use emesh_core::spacing::{first_cell_height, thickness};
use emesh_core::units::Units;
use emesh_core::{Curve, Kind};
use emesh_lua::bcs::{bc_class, check_tag, BC_CLASSES};

use crate::settings::{FlowDefaults, KeySeq, LayerDefaults, QualityLimits, DEFAULT_HOLD_KEYS, DEFAULT_SHORTCUTS};
use crate::theme::col;

/// A modal window; the background is dimmed and blocked while it is open.
pub fn modal<R>(ctx: &egui::Context, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> Option<R> {
    let mut out = None;
    egui::Modal::new(egui::Id::new(("modal", title))).show(ctx, |ui| {
        ui.set_min_width(360.0);
        ui.heading(title);
        ui.add_space(4.0);
        out = Some(add(ui));
    });
    out
}

/// OK / Cancel buttons (Enter = OK when enabled, Esc = Cancel): Some(true) for OK.
pub fn ok_cancel(ui: &mut egui::Ui, ok_enabled: bool) -> Option<bool> {
    let mut out = None;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Cancel").clicked() {
                out = Some(false);
            }
            if ui.add_enabled(ok_enabled, egui::Button::new(RichText::new("  OK  ").strong())).clicked() {
                out = Some(true);
            }
        });
    });
    let (enter, esc) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    if esc {
        out = Some(false);
    } else if enter && ok_enabled && out.is_none() {
        out = Some(true);
    }
    out
}

fn note(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(col::weak_text()));
}

fn number(text: &str, name: &str) -> Result<f64, String> {
    let v: f64 = text.trim().parse().map_err(|_| format!("{name} must be a number"))?;
    if !(v > 0.0) {
        return Err(format!("{name} must be positive"));
    }
    Ok(v)
}

// --- edge cells --------------------------------------------------------------------------------

/// (label, kind, end0, end1): Roberts stretching toward the marked ends, or a fixed first cell
pub const SPACING: [(&str, &str, bool, bool); 6] = [
    ("Uniform", "uniform", false, false),
    ("Finer toward the start (o)", "roberts", true, false),
    ("Finer toward the end (>)", "roberts", false, true),
    ("Finer toward both ends", "roberts", true, true),
    ("First cell height at the start (o)", "growth", true, false),
    ("First cell height at the end (>)", "growth", false, true),
];

pub struct EdgeCells {
    pub bid: String,
    pub face: emesh_core::Face,
    pub cells: usize,
    pub spacing: usize,
    pub beta: f64,
    pub first: String,
    pub opposite: bool,
    grade: Option<f64>,
    units: Units,
    error: String,
}

impl EdgeCells {
    pub fn new(bid: &str, face: emesh_core::Face, cells: usize, spec: Option<&Cluster>, units: Units) -> Self {
        let mut d = EdgeCells {
            bid: bid.into(),
            face,
            cells,
            spacing: 0,
            beta: 1.05,
            first: String::new(),
            opposite: true,
            grade: None,
            units,
            error: String::new(),
        };
        if let Some(s) = spec {
            match s.kind() {
                "RobertsFunction" => {
                    let ends = (s.b("end0"), s.b("end1"));
                    if let Some(k) = SPACING.iter().position(|(_, kind, e0, e1)| *kind == "roberts" && (*e0, *e1) == ends) {
                        d.spacing = k;
                    }
                    d.beta = s.f("beta").unwrap_or(1.05);
                }
                "Growth" | "Grade" => {
                    d.spacing = if s.b("reverse") { 5 } else { 4 };
                    d.first = pyfmt::g(units.to_display(s.f("first").unwrap_or(0.0)), 6);
                    // a boundary layer's outer edge grows only up to a size, then stays uniform: keep that
                    if s.kind() == "Grade" {
                        d.grade = s.f("growth");
                    }
                }
                _ => {}
            }
        }
        d
    }

    /// (cells, cluster spec or None, apply to the opposite edge)
    pub fn values(&self) -> Result<(usize, Option<Cluster>, bool), String> {
        let (_, kind, end0, end1) = SPACING[self.spacing];
        let spec = match kind {
            "roberts" => Some(Cluster::roberts(end0, end1, self.beta)),
            "growth" => {
                let v: f64 = self.first.trim().parse().map_err(|_| "the first cell height must be a number".to_string())?;
                let first = self.units.from_display(v);
                if !(first > 0.0) {
                    return Err("the first cell height must be positive".into());
                }
                Some(match self.grade {
                    Some(g) => Cluster::grade(first, g, end1),
                    None => Cluster::growth(first, end1),
                })
            }
            _ => None,
        };
        Ok((self.cells, spec, self.opposite))
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        modal(ctx, "Edge cells", |ui| {
            egui::Grid::new("edge").num_columns(2).show(ui, |ui| {
                ui.label("Cells");
                ui.add(egui::DragValue::new(&mut self.cells).range(1..=100000));
                ui.end_row();
                ui.label("Spacing");
                egui::ComboBox::from_id_salt("spacing").width(260.0).selected_text(SPACING[self.spacing].0).show_ui(ui, |ui| {
                    for (k, s) in SPACING.iter().enumerate() {
                        ui.selectable_value(&mut self.spacing, k, s.0);
                    }
                });
                ui.end_row();
                let kind = SPACING[self.spacing].1;
                ui.label("Clustering strength (beta)");
                ui.add_enabled(kind == "roberts", egui::DragValue::new(&mut self.beta).range(1.00001..=10.0).speed(0.001).max_decimals(5));
                ui.end_row();
                ui.label(format!("First cell height ({})", self.units.name));
                ui.add_enabled(kind == "growth", egui::TextEdit::singleline(&mut self.first).desired_width(120.0));
                ui.end_row();
            });
            ui.checkbox(&mut self.opposite, "Also the opposite edge of this block");
            note(
                ui,
                "The edge's start (o) and end (>) are marked on the canvas while you hover it. Beta closer to 1 \
                 clusters more strongly (Roberts stretching). A first cell height grows the cells geometrically from \
                 that end (as in a boundary layer). The cell count also applies to every edge linked to this one, so \
                 shared faces match.",
            );
            if !self.error.is_empty() {
                ui.colored_label(col::error_text(), &self.error);
            }
            match ok_cancel(ui, true) {
                Some(true) => match self.values() {
                    Ok(_) => Some(true),
                    Err(e) => {
                        self.error = e;
                        None
                    }
                },
                other => other,
            }
        })
        .flatten()
    }
}

// --- split, AOPatch, thresholds ------------------------------------------------------------------

pub struct Split {
    pub point: emesh_core::P,
    pub n: usize,
    pub m: usize,
}

impl Split {
    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        modal(ctx, "Split into N x M blocks", |ui| {
            egui::Grid::new("split").num_columns(2).show(ui, |ui| {
                ui.label("Blocks along i (south edge)");
                ui.add(egui::DragValue::new(&mut self.n).range(1..=50));
                ui.end_row();
                ui.label("Blocks along j (west edge)");
                ui.add(egui::DragValue::new(&mut self.m).range(1..=50));
                ui.end_row();
            });
            ok_cancel(ui, true)
        })
        .flatten()
    }
}

pub struct AoPatch {
    pub bid: String,
    pub nx: i64,
    pub ny: i64,
}

impl AoPatch {
    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        modal(ctx, "AOPatch", |ui| {
            egui::Grid::new("ao").num_columns(2).show(ui, |ui| {
                ui.label("Background mesh nx");
                ui.add(egui::DragValue::new(&mut self.nx).range(2..=200));
                ui.end_row();
                ui.label("Background mesh ny");
                ui.add(egui::DragValue::new(&mut self.ny).range(2..=200));
                ui.end_row();
            });
            note(
                ui,
                "AOPatch smooths the interior towards area-orthogonal cells; it can untangle blocks with strongly curved \
                 sides. The preview shows a CoonsPatch until Eilmer's mesh arrives.",
            );
            ok_cancel(ui, true)
        })
        .flatten()
    }
}

pub struct ThresholdsDlg {
    pub v: QualityLimits,
}

impl ThresholdsDlg {
    pub fn values(&self) -> QualityLimits {
        let (w, b) = (self.v.skew_warn, self.v.skew_bad);
        QualityLimits { skew_warn: w.min(b), skew_bad: w.max(b), ar_warn: self.v.ar_warn }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        modal(ctx, "Mesh quality thresholds", |ui| {
            egui::Grid::new("thr").num_columns(2).show(ui, |ui| {
                ui.label("Warn above skewness");
                ui.add(egui::DragValue::new(&mut self.v.skew_warn).range(0.05..=0.99).speed(0.01).fixed_decimals(2));
                ui.end_row();
                ui.label("Needs refinement above skewness");
                ui.add(egui::DragValue::new(&mut self.v.skew_bad).range(0.05..=1.0).speed(0.01).fixed_decimals(2));
                ui.end_row();
                ui.label("Warn above aspect ratio");
                ui.add(egui::DragValue::new(&mut self.v.ar_warn).range(1.5..=1e6).speed(0.5).fixed_decimals(2));
                ui.end_row();
            });
            note(
                ui,
                "Equiangle skewness: 0 for rectangular cells, 1 for degenerate ones. Blocks with boundary-layer spacing \
                 are long and thin by design, so their aspect ratio is not checked.",
            );
            ok_cancel(ui, true)
        })
        .flatten()
    }
}

// --- boundary conditions -------------------------------------------------------------------------

pub struct BcDialog {
    pub old_tag: Option<String>,
    pub class: String,
    pub tag: String,
    pub args: String,
    args_edited: bool,
    tag_edited: bool,
    existing: Vec<String>,
    error: String,
    /// curves to tag once defined (from "New boundary condition...")
    pub for_curves: Vec<String>,
}

impl BcDialog {
    pub fn new(doc: &Document, tag: Option<&str>) -> Self {
        let existing: Vec<String> = doc.bcs.keys().cloned().collect();
        let mut d = BcDialog {
            old_tag: tag.map(|t| t.to_string()),
            class: "InFlowBC_Supersonic".into(),
            tag: tag.unwrap_or("").to_string(),
            args: String::new(),
            args_edited: tag.is_some(),
            tag_edited: false,
            existing,
            error: String::new(),
            for_curves: vec![],
        };
        if let Some(spec) = tag.and_then(|t| doc.bcs.get(t)) {
            d.class = spec.class.clone();
            d.args = spec.args.clone();
        } else {
            d.class_changed();
        }
        d
    }

    fn free_tag(&self, base: &str) -> String {
        let (mut tag, mut k) = (base.to_string(), 2);
        while self.existing.contains(&tag) {
            tag = format!("{base}{k}");
            k += 1;
        }
        tag
    }

    fn class_changed(&mut self) {
        let Some((_, _, args, default_tag, _)) = bc_class(&self.class) else { return };
        if !self.args_edited {
            self.args = args.to_string();
        }
        let is_default = self.tag.is_empty() || BC_CLASSES.iter().any(|c| c.3 == self.tag);
        if !self.tag_edited && is_default {
            self.tag = self.free_tag(default_tag);
        }
    }

    pub fn values(&self) -> (String, String, String) {
        (self.tag.trim().to_string(), self.class.clone(), self.args.trim().to_string())
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        modal(ctx, "Boundary condition", |ui| {
            ui.set_min_width(460.0);
            egui::Grid::new("bc").num_columns(2).show(ui, |ui| {
                ui.label("Eilmer class");
                let before = self.class.clone();
                let sel = bc_class(&self.class).map(|c| format!("{}: {}", c.1, c.0)).unwrap_or_default();
                egui::ComboBox::from_id_salt("bccls").width(320.0).selected_text(sel).show_ui(ui, |ui| {
                    for (cls, group, _, _, desc) in BC_CLASSES {
                        ui.selectable_value(&mut self.class, cls.to_string(), format!("{group}: {cls}")).on_hover_text(*desc);
                    }
                });
                if self.class != before {
                    self.class_changed();
                }
                ui.end_row();
                ui.label("Tag");
                if ui.text_edit_singleline(&mut self.tag).changed() {
                    self.tag_edited = true;
                }
                ui.end_row();
                ui.label("Arguments");
                if ui.add(egui::TextEdit::singleline(&mut self.args).desired_width(320.0)).changed() {
                    self.args_edited = true;
                }
                ui.end_row();
            });
            if let Some(c) = bc_class(&self.class) {
                ui.label(c.4);
            }
            note(
                ui,
                "Arguments go inside Class:new{...} in bcs.lua, e.g. flowState=inflow. Names such as 'inflow' are flow \
                 states; the starter sim.lua written on export defines them.",
            );
            if !self.error.is_empty() {
                ui.colored_label(col::error_text(), &self.error);
            }
            match ok_cancel(ui, true) {
                Some(true) => match check_tag(self.tag.trim()) {
                    Ok(_) => Some(true),
                    Err(e) => {
                        self.error = e;
                        None
                    }
                },
                o => o,
            }
        })
        .flatten()
    }
}

// --- dimension editor -----------------------------------------------------------------------------

#[derive(Clone)]
pub struct Field {
    pub label: String,
    pub value: f64,
    pub suffix: String,
    pub lo: f64,
    pub hi: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditKind {
    Point,
    Line,
    Arc,
    Length,
    ArcAngle,
    Distance,
    TwoLineAngle,
}

pub struct EditDims {
    pub eid: String,
    pub kind: EditKind,
    pub title: String,
    pub fields: Vec<Field>,
    pub note: String,
    texts: Vec<String>,
    error: String,
}

impl EditDims {
    pub fn new(eid: &str, kind: EditKind, title: &str, fields: Vec<Field>, note: &str) -> Self {
        let texts = fields.iter().map(|f| pyfmt::g(f.value, 12)).collect();
        EditDims { eid: eid.into(), kind, title: title.into(), fields, note: note.into(), texts, error: String::new() }
    }

    pub fn values(&self) -> Result<Vec<f64>, String> {
        self.fields
            .iter()
            .zip(&self.texts)
            .map(|(f, t)| {
                let v: f64 = t.trim().parse().map_err(|_| format!("{} must be a number", f.label))?;
                if v < f.lo || v > f.hi {
                    return Err(format!("{} must be between {} and {}", f.label, pyfmt::g(f.lo, 6), pyfmt::g(f.hi, 6)));
                }
                Ok(v)
            })
            .collect()
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        let title = self.title.clone();
        modal(ctx, &title, |ui| {
            egui::Grid::new("dims").num_columns(3).show(ui, |ui| {
                for (k, f) in self.fields.iter().enumerate() {
                    ui.label(&f.label);
                    let r = ui.add(egui::TextEdit::singleline(&mut self.texts[k]).desired_width(180.0));
                    if k == 0 && ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    ui.label(&f.suffix);
                    ui.end_row();
                }
            });
            if !self.note.is_empty() {
                note(ui, &self.note);
            }
            if !self.error.is_empty() {
                ui.colored_label(col::error_text(), &self.error);
            }
            match ok_cancel(ui, true) {
                Some(true) => match self.values() {
                    Ok(_) => Some(true),
                    Err(e) => {
                        self.error = e;
                        None
                    }
                },
                o => o,
            }
        })
        .flatten()
    }
}

// --- boundary layer ---------------------------------------------------------------------------------

pub struct LayerDlg {
    pub target: Option<(String, emesh_core::Face)>,
    pub lid: Option<String>,
    pub first: String,
    pub growth: f64,
    pub cells: usize,
    pub transition: bool,
    pub yplus: String,
    pub velocity: String,
    pub density: String,
    pub viscosity: String,
    pub length: String,
    pub removed: bool,
    units: Units,
    error: String,
}

impl LayerDlg {
    pub fn new(target: Option<(String, emesh_core::Face)>, lid: Option<String>, v: LayerDefaults, f: FlowDefaults, units: Units) -> Self {
        LayerDlg {
            target,
            lid,
            first: pyfmt::g(units.to_display(v.first), 6),
            growth: v.growth,
            cells: v.cells,
            transition: v.transition,
            yplus: pyfmt::g(f.yplus, 6),
            velocity: pyfmt::g(f.velocity, 6),
            density: pyfmt::g(f.density, 6),
            viscosity: pyfmt::g(f.viscosity, 6),
            length: pyfmt::g(units.to_display(f.length), 6),
            removed: false,
            units,
            error: String::new(),
        }
    }

    pub fn values(&self) -> Result<LayerDefaults, String> {
        Ok(LayerDefaults {
            first: self.units.from_display(number(&self.first, "The first cell height")?),
            growth: self.growth,
            cells: self.cells,
            transition: self.transition,
        })
    }

    fn estimate(&self) -> Result<f64, String> {
        first_cell_height(
            number(&self.yplus, "y+")?,
            number(&self.velocity, "The velocity")?,
            number(&self.density, "The density")?,
            number(&self.viscosity, "The viscosity")?,
            self.units.from_display(number(&self.length, "The distance")?),
        )
    }

    /// The y+ inputs as typed (skipping any that are not valid numbers).
    pub fn flow_values(&self, old: FlowDefaults) -> FlowDefaults {
        let p = |t: &str, d: f64| t.trim().parse::<f64>().unwrap_or(d);
        FlowDefaults {
            yplus: p(&self.yplus, old.yplus),
            velocity: p(&self.velocity, old.velocity),
            density: p(&self.density, old.density),
            viscosity: p(&self.viscosity, old.viscosity),
            length: self.length.trim().parse::<f64>().map(|v| self.units.from_display(v)).unwrap_or(old.length),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        let title = if self.lid.is_some() { "Boundary layer" } else { "Boundary layer: settings for new layers" };
        let u = self.units;
        modal(ctx, title, |ui| {
            ui.set_min_width(480.0);
            egui::Grid::new("layer").num_columns(2).show(ui, |ui| {
                ui.label(format!("First cell height ({})", u.name));
                ui.text_edit_singleline(&mut self.first);
                ui.end_row();
                ui.label("Growth ratio (1 = uniform)");
                ui.add(egui::DragValue::new(&mut self.growth).range(1.0..=3.0).speed(0.01).fixed_decimals(3));
                ui.end_row();
                ui.label("Cells across the layer");
                ui.add(egui::DragValue::new(&mut self.cells).range(1..=500));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut self.transition, "Grade the outer blocks from the last layer cell").on_hover_text(
                    "Off: the blocks outside the layer keep their original cells.\nOn: they grow from the last layer \
                     cell (at the layer's ratio, at least 1.1) up to their original cell size, then stay uniform, \
                     adding cells as needed. When the last layer cell is much smaller than the outer cells, this \
                     growth fills most of the outer block.",
                );
                ui.end_row();
                ui.label("");
                match number(&self.first, "The first cell height") {
                    Ok(v) => {
                        let first = u.from_display(v);
                        ui.label(format!(
                            "Layer thickness {}, last cell {}",
                            u.format(thickness(first, self.growth, self.cells), None),
                            u.format(first * self.growth.powf(self.cells as f64 - 1.0), None)
                        ));
                    }
                    Err(e) => {
                        ui.colored_label(col::error_text(), e);
                    }
                }
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.group(|ui| {
                ui.label(RichText::new("First cell from y+  (turbulent flat plate, Cf = 0.026 Re_x^-1/7)").strong());
                egui::Grid::new("yplus").num_columns(2).show(ui, |ui| {
                    for (label, text) in [
                        ("Target y+".to_string(), &mut self.yplus),
                        ("Free-stream velocity (m/s)".to_string(), &mut self.velocity),
                        ("Density (kg/m^3)".to_string(), &mut self.density),
                        ("Dynamic viscosity (Pa s)".to_string(), &mut self.viscosity),
                        (format!("Distance from the leading edge ({})", u.name), &mut self.length),
                    ] {
                        ui.label(label);
                        ui.text_edit_singleline(text);
                        ui.end_row();
                    }
                });
                match self.estimate() {
                    Ok(h) => {
                        let f = self.flow_values(FlowDefaults::default());
                        let re = f.density * f.velocity * f.length / f.viscosity;
                        ui.label(format!("Re_x = {}: first cell height {}", pyfmt::g(re, 3), u.format(h, None)));
                        if ui.button("Use this first cell height").clicked() {
                            self.first = pyfmt::g(u.to_display(h), 4);
                        }
                    }
                    Err(e) => {
                        ui.colored_label(col::error_text(), e);
                    }
                }
            });
            note(
                ui,
                "The first cell height is the wall-normal size of the cell on the wall; Eilmer's cell centre sits at \
                 half of it. Changing an applied layer cuts its blocks again (their counts and clustering are set afresh).",
            );
            if !self.error.is_empty() {
                ui.colored_label(col::error_text(), &self.error);
            }
            let mut out = None;
            ui.horizontal(|ui| {
                if self.lid.is_some() && ui.button("Remove layer").clicked() {
                    self.removed = true;
                    out = Some(true);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        out = Some(false);
                    }
                    if ui.button(RichText::new("  OK  ").strong()).clicked() {
                        match self.values() {
                            Ok(_) => out = Some(true),
                            Err(e) => self.error = e,
                        }
                    }
                });
            });
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                out = Some(false);
            }
            out
        })
        .flatten()
    }
}

// --- import points ---------------------------------------------------------------------------------

pub struct ImportDlg {
    pub pf: PointFile,
    pub path: String,
    pub scale: f64,
    pub angle: f64,
    pub dx: f64,
    pub dy: f64,
    pub split_le: bool,
    pub close_te: bool,
    pub reference: bool,
    units: Units,
}

impl ImportDlg {
    pub fn new(pf: PointFile, path: &str, units: Units) -> Self {
        let k = leading_edge_index(&pf.points);
        let n = pf.points.len();
        ImportDlg {
            pf,
            path: path.into(),
            scale: units.to_display(1.0),
            angle: 0.0,
            dx: 0.0,
            dy: 0.0,
            split_le: 0 < k && k < n - 1,
            close_te: true,
            reference: false,
            units,
        }
    }

    pub fn curves(&self) -> Result<Vec<Curve>, String> {
        let f = |v: f64| self.units.from_display(v);
        build_curves(
            &self.pf,
            &ImportOptions {
                scale: f(self.scale),
                angle_deg: self.angle,
                dx: f(self.dx),
                dy: f(self.dy),
                split_le: self.split_le,
                close_te: self.close_te,
                kind: if self.reference { Kind::Reference } else { Kind::Geometry },
            },
        )
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        modal(ctx, "Import points", |ui| {
            let pts = &self.pf.points;
            let (xs, ys): (Vec<f64>, Vec<f64>) = pts.iter().map(|p| (p[0], p[1])).unzip();
            let mn = |v: &[f64]| v.iter().cloned().fold(f64::INFINITY, f64::min);
            let mx = |v: &[f64]| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            ui.label(RichText::new(&self.pf.name).strong());
            ui.label(&self.path);
            ui.label(format!(
                "{} points, {} format\nx: {} to {}, y: {} to {}",
                pts.len(),
                self.pf.fmt,
                pyfmt::g(mn(&xs), 6),
                pyfmt::g(mx(&xs), 6),
                pyfmt::g(mn(&ys), 6),
                pyfmt::g(mx(&ys), 6)
            ));
            let u = self.units.name;
            egui::Grid::new("imp").num_columns(2).show(ui, |ui| {
                ui.label("Scale (length of 1 file unit)");
                ui.add(egui::DragValue::new(&mut self.scale).range(1e-9..=1e9).speed(0.01).suffix(format!(" {u}")));
                ui.end_row();
                ui.label("Angle of attack (deg, nose up)");
                ui.add(egui::DragValue::new(&mut self.angle).range(-180.0..=180.0).speed(0.1));
                ui.end_row();
                ui.label("Translate x");
                ui.add(egui::DragValue::new(&mut self.dx).speed(0.01).suffix(format!(" {u}")));
                ui.end_row();
                ui.label("Translate y");
                ui.add(egui::DragValue::new(&mut self.dy).speed(0.01).suffix(format!(" {u}")));
                ui.end_row();
            });
            ui.checkbox(&mut self.split_le, "Split at leading edge (min x) so it can be a block corner");
            ui.checkbox(&mut self.close_te, "Close the trailing edge with a line");
            ui.checkbox(&mut self.reference, "Import as reference (dashed) curves");
            ok_cancel(ui, true)
        })
        .flatten()
    }
}

// --- keyboard shortcuts --------------------------------------------------------------------------------

pub struct ShortcutsDlg {
    pub values: BTreeMap<String, String>,
    capturing: Option<String>,
    error: String,
}

impl ShortcutsDlg {
    pub fn new(current: BTreeMap<String, String>) -> Self {
        ShortcutsDlg { values: current, capturing: None, error: String::new() }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        if let Some(id) = self.capturing.clone() {
            let ev = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, m)) = ev {
                if key == Key::Escape {
                    self.capturing = None;
                } else if key == Key::Backspace && !m.any() {
                    self.values.insert(id, String::new());
                    self.capturing = None;
                } else {
                    let hold = DEFAULT_HOLD_KEYS.iter().any(|h| h.0 == id);
                    let seq = KeySeq { key, ctrl: m.command && !hold, shift: m.shift && !hold, alt: m.alt && !hold };
                    self.values.insert(id, seq.text());
                    self.capturing = None;
                }
                ctx.input_mut(|i| i.events.clear());
            }
        }
        modal(ctx, "Keyboard shortcuts", |ui| {
            ui.set_min_width(460.0);
            note(ui, "Click a shortcut, then press the new key (Backspace clears it, Esc keeps it).");
            egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                egui::Grid::new("keys").num_columns(2).striped(true).show(ui, |ui| {
                    for (id, label, _) in DEFAULT_SHORTCUTS.iter().chain(DEFAULT_HOLD_KEYS) {
                        ui.label(*label);
                        let text = if self.capturing.as_deref() == Some(id) {
                            "press a key...".to_string()
                        } else {
                            self.values.get(*id).cloned().unwrap_or_default()
                        };
                        if ui.add(egui::Button::new(text).min_size(egui::vec2(160.0, 0.0))).clicked() {
                            self.capturing = Some(id.to_string());
                        }
                        ui.end_row();
                    }
                });
            });
            if ui.button("Restore defaults").clicked() {
                for (id, _, d) in DEFAULT_SHORTCUTS.iter().chain(DEFAULT_HOLD_KEYS) {
                    self.values.insert(id.to_string(), d.to_string());
                }
            }
            if !self.error.is_empty() {
                ui.colored_label(col::error_text(), &self.error);
            }
            if self.capturing.is_some() {
                return None;
            }
            match ok_cancel(ui, true) {
                Some(true) => {
                    let mut seen: BTreeMap<String, String> = BTreeMap::new();
                    for (id, seq) in &self.values {
                        if seq.is_empty() {
                            continue;
                        }
                        if let Some(other) = seen.get(seq) {
                            let name = |i: &str| {
                                DEFAULT_SHORTCUTS.iter().chain(DEFAULT_HOLD_KEYS).find(|t| t.0 == i).map(|t| t.1).unwrap_or(i).to_string()
                            };
                            self.error = format!("'{seq}' is used by both '{}' and '{}'.", name(other), name(id));
                            return None;
                        }
                        seen.insert(seq.clone(), id.clone());
                    }
                    Some(true)
                }
                o => o,
            }
        })
        .flatten()
    }
}

// --- messages ---------------------------------------------------------------------------------------------

pub struct Message {
    pub title: String,
    pub text: String,
    pub details: String,
    pub error: bool,
}

impl Message {
    pub fn show(&mut self, ctx: &egui::Context) -> Option<bool> {
        let title = self.title.clone();
        modal(ctx, &title, |ui| {
            ui.set_max_width(560.0);
            if self.error {
                ui.colored_label(col::error_text(), &self.text);
            } else {
                ui.label(&self.text);
            }
            if !self.details.is_empty() {
                egui::CollapsingHeader::new("Details").show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(&self.details).monospace().small()).wrap());
                    });
                });
            }
            let mut out = None;
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("  OK  ").clicked() {
                        out = Some(true);
                    }
                });
            });
            if ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Escape)) {
                out = Some(true);
            }
            out
        })
        .flatten()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Save,
    Discard,
    Cancel,
}

/// "Save changes to the current project?"
pub fn confirm_discard(ctx: &egui::Context) -> Option<Choice> {
    modal(ctx, "Unsaved changes", |ui| {
        ui.label("Save changes to the current project?");
        let mut out = None;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    out = Some(Choice::Cancel);
                }
                if ui.button("Discard").clicked() {
                    out = Some(Choice::Discard);
                }
                if ui.button(RichText::new("  Save  ").strong()).clicked() {
                    out = Some(Choice::Save);
                }
            });
        });
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            out = Some(Choice::Cancel);
        }
        out
    })
    .flatten()
}

// --- file browser -------------------------------------------------------------------------------------------

// The file dialog lives in file_browser.rs (Qt-style Look in / places / detail list).
pub use crate::file_browser::{FileBrowser, FileMode};
