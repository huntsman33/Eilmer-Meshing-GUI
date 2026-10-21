//! Python number formatting: `repr(float)` (used for every number in the generated Lua),
//! the `g` and `f` format specs used in messages, and tuple/list reprs.

/// Shortest round-trip digits and decimal exponent: x = 0.d1d2...dn * 10^decpt.
fn shortest(x: f64) -> (String, i32) {
    let s = format!("{:e}", x.abs()); // e.g. "1.2345e-7", "1e16"
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0').to_string();
    let digits = if digits.is_empty() { "0".to_string() } else { digits };
    (digits, exp + 1)
}

/// Python `repr(x)` for a float.
pub fn repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let sign = if x.is_sign_negative() { "-" } else { "" };
    if x == 0.0 {
        return format!("{sign}0.0");
    }
    let (digits, decpt) = shortest(x);
    let n = digits.len() as i32;
    if decpt > -4 && decpt <= 16 {
        // fixed notation
        if decpt <= 0 {
            format!("{sign}0.{}{}", "0".repeat((-decpt) as usize), digits)
        } else if decpt >= n {
            format!("{sign}{}{}.0", digits, "0".repeat((decpt - n) as usize))
        } else {
            let (a, b) = digits.split_at(decpt as usize);
            format!("{sign}{a}.{b}")
        }
    } else {
        let e = decpt - 1;
        let mant = if n == 1 { digits.clone() } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        format!("{sign}{mant}e{}{:02}", if e < 0 { "-" } else { "+" }, e.abs())
    }
}

/// Python `format(x, f".{prec}g")` (and `"g"` with prec 6).
pub fn g(x: f64, prec: usize) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let p = prec.max(1);
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    // exponent after rounding to p significant digits
    let s = format!("{:.*e}", p - 1, x);
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    if exp >= -4 && exp < p as i32 {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        let f = format!("{:.*}", decimals, x);
        strip_zeros(&f)
    } else {
        let m = strip_zeros(mant);
        format!("{m}e{}{:02}", if exp < 0 { "-" } else { "+" }, exp.abs())
    }
}

fn strip_zeros(s: &str) -> String {
    if s.contains('.') {
        let t = s.trim_end_matches('0');
        t.trim_end_matches('.').to_string()
    } else {
        s.to_string()
    }
}

/// Python `format(x, f".{decimals}f")`.
pub fn f(x: f64, decimals: usize) -> String {
    format!("{:.*}", decimals, x)
}

/// Python `repr((x, y))` of a point.
pub fn point(p: [f64; 2]) -> String {
    format!("({}, {})", repr(p[0]), repr(p[1]))
}

/// Python `repr(sorted(strings))` e.g. `['a', 'b']`.
pub fn str_list(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_cases() {
        let cases = [
            (1.0, "1.0"),
            (0.1, "0.1"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (123.456, "123.456"),
            (-0.0, "-0.0"),
            (2.5e-7, "2.5e-07"),
            (1.2345678901234567e20, "1.2345678901234567e+20"),
            (0.30000000000000004, "0.30000000000000004"),
            (-12.5, "-12.5"),
        ];
        for (x, want) in cases {
            assert_eq!(repr(x), want, "{x}");
        }
    }

    #[test]
    fn g_cases() {
        let cases = [
            (0.0494, 3, "0.0494"),
            (1.298, 4, "1.298"),
            (100.0, 3, "100"),
            (1234.5, 3, "1.23e+03"),
            (1e-5, 3, "1e-05"),
            (0.5, 6, "0.5"),
            (20.0, 6, "20"),
            (0.85, 6, "0.85"),
            (123456789.0, 6, "1.23457e+08"),
            (0.0001, 4, "0.0001"),
            (0.00001234, 4, "1.234e-05"),
        ];
        for (x, p, want) in cases {
            assert_eq!(g(x, p), want, "{x} .{p}g");
        }
    }
}
