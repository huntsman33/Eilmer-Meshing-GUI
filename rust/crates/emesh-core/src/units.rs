//! Display units. The document, the generated Lua and Eilmer always work in metres; units only
//! change what the user sees and types.

use crate::pyfmt;

/// name -> metres per unit
pub const UNITS: [(&str, f64); 4] = [("m", 1.0), ("mm", 1.0e-3), ("in", 0.0254), ("ft", 0.3048)];
pub const DEFAULT_UNITS: &str = "m";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Units {
    pub name: &'static str,
    pub factor: f64,
}

impl Default for Units {
    fn default() -> Self {
        Units::new(DEFAULT_UNITS)
    }
}

impl Units {
    pub fn new(name: &str) -> Units {
        let (name, factor) = UNITS.iter().copied().find(|(n, _)| *n == name).unwrap_or(UNITS[0]);
        Units { name, factor }
    }

    pub fn to_display(&self, metres: f64) -> f64 {
        metres / self.factor
    }

    pub fn from_display(&self, value: f64) -> f64 {
        value * self.factor
    }

    pub fn point_to_display(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] / self.factor, p[1] / self.factor]
    }

    pub fn point_from_display(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] * self.factor, p[1] * self.factor]
    }

    /// Decimals worth showing for a resolution of step_metres.
    pub fn digits(&self, step_metres: f64) -> usize {
        let step = (step_metres / self.factor).max(1e-300);
        (-(step.log10().floor()) + 1.0).max(0.0) as usize
    }

    /// "1.25 mm" (6 significant digits), or with fixed decimals.
    pub fn format(&self, metres: f64, digits: Option<usize>) -> String {
        let v = self.to_display(metres);
        match digits {
            None => format!("{} {}", pyfmt::g(v, 6), self.name),
            Some(d) => format!("{} {}", pyfmt::f(v, d), self.name),
        }
    }
}
