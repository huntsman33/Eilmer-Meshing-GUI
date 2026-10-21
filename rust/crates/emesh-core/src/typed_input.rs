//! Parsing for the typed-coordinate box.
//!
//!   x, y        absolute point
//!   @dx, dy     relative to the last point
//!   @L<deg      polar, relative to the last point
//!   L<deg       polar, from the origin
//! A single number is left to the active tool (length, radius, ...).

pub fn parse_numbers(text: &str) -> Result<Vec<f64>, String> {
    text.trim()
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<f64>().map_err(|_| format!("not a number list: '{text}'")))
        .collect()
}

/// The value if text is one number, else None.
pub fn single_number(text: &str) -> Option<f64> {
    let nums = parse_numbers(text).ok()?;
    if nums.len() == 1 && !text.contains('<') && !text.contains('@') {
        Some(nums[0])
    } else {
        None
    }
}

pub fn parse_point(text: &str, anchor: Option<[f64; 2]>) -> Result<[f64; 2], String> {
    let mut t = text.trim();
    let relative = t.starts_with('@');
    if relative {
        if anchor.is_none() {
            return Err("no previous point for a relative coordinate".into());
        }
        t = &t[1..];
    }
    let base = if relative { anchor.unwrap() } else { [0.0, 0.0] };
    if let Some((l, a)) = t.split_once('<') {
        let length: f64 = l.trim().parse().map_err(|_| format!("not a length: '{l}'"))?;
        let angle: f64 = a.trim().parse::<f64>().map_err(|_| format!("not an angle: '{a}'"))?.to_radians();
        return Ok([base[0] + length * angle.cos(), base[1] + length * angle.sin()]);
    }
    let nums = parse_numbers(t)?;
    if nums.len() != 2 {
        return Err("expected two numbers: x, y".into());
    }
    Ok([base[0] + nums[0], base[1] + nums[1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        assert_eq!(parse_point("1, 2", None).unwrap(), [1.0, 2.0]);
        assert_eq!(parse_point("@1 2", Some([1.0, 1.0])).unwrap(), [2.0, 3.0]);
        let p = parse_point("@2<90", Some([0.0, 0.0])).unwrap();
        assert!((p[0]).abs() < 1e-12 && (p[1] - 2.0).abs() < 1e-12);
        assert!(parse_point("@1,1", None).is_err());
        assert_eq!(single_number("2.5"), Some(2.5));
        assert_eq!(single_number("2,5"), None);
    }
}
