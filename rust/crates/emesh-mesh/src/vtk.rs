//! Reader for legacy ASCII VTK STRUCTURED_GRID files, as written by Eilmer's
//! StructuredGrid:write_to_vtk_file.

use std::path::Path;

use crate::preview::Grid;

/// The k = 0 slice of a structured grid file.
pub fn read_structured_vtk(path: &Path) -> Result<Grid, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_structured_vtk(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn parse_structured_vtk(text: &str) -> Result<Grid, String> {
    let mut dims: Option<(usize, usize, usize)> = None;
    let mut npts = 0usize;
    let mut rest: Option<&str> = None;
    let mut offset = 0usize;
    for line in text.split_inclusive('\n') {
        let tok: Vec<&str> = line.split_whitespace().collect();
        offset += line.len();
        if tok.is_empty() {
            continue;
        }
        if tok[0] == "DATASET" && tok.get(1) != Some(&"STRUCTURED_GRID") {
            return Err(format!("expected STRUCTURED_GRID, got {}", tok.get(1).unwrap_or(&"")));
        }
        if tok[0] == "DIMENSIONS" && tok.len() >= 4 {
            let p = |s: &str| s.parse::<usize>().map_err(|e| e.to_string());
            dims = Some((p(tok[1])?, p(tok[2])?, p(tok[3])?));
        } else if tok[0] == "POINTS" && tok.len() >= 2 {
            npts = tok[1].parse::<usize>().map_err(|e| e.to_string())?;
            rest = Some(&text[offset..]);
            break;
        }
    }
    let (niv, njv, nkv) = dims.ok_or("missing DIMENSIONS")?;
    let rest = rest.ok_or("missing POINTS")?;
    if npts != niv * njv * nkv {
        return Err(format!("POINTS {npts} does not match DIMENSIONS ({niv}, {njv}, {nkv})"));
    }
    let mut vals = rest.split_whitespace().map(|s| s.parse::<f64>());
    let mut pts = vec![[0.0; 2]; niv * njv];
    // VTK order: i fastest, then j, then k.
    for j in 0..njv {
        for i in 0..niv {
            let x = vals.next().ok_or("too few coordinates")?.map_err(|e| e.to_string())?;
            let y = vals.next().ok_or("too few coordinates")?.map_err(|e| e.to_string())?;
            let _z = vals.next().ok_or("too few coordinates")?;
            pts[i * njv + j] = [x, y];
        }
    }
    Ok(Grid { niv, njv, pts })
}
