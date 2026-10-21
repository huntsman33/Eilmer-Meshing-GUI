//! Numerics that reproduce the Python reference (CPython `math`, numpy, scipy) bit for bit,
//! so generated Lua and stored geometry match the Python version exactly.
//!
//! - [`py_hypot`] / [`py_dist`] is CPython's `math.hypot` / `math.dist` (compensated vector norm),
//!   which differ from libm's `hypot` in the last bit for about 1 in 160 inputs.
//! - [`c_hypot`] is libm `hypot`, which numpy's `np.hypot` calls.
//! - [`np_sum`] is numpy's pairwise summation (`np.sum`, `ndarray.sum`).
//! - [`linspace`] is `np.linspace`.
//! - [`fminbound`] is scipy's bounded Brent minimiser (`minimize_scalar(method="bounded")`).
//! - [`brentq`] is scipy's Brent root finder.
//! - [`solve_tridiagonal`] is LAPACK `dgtsv` without pivoting (scipy `solve_banded((1, 1), ...)`
//!   on a diagonally dominant system).

pub type P = [f64; 2];

const T27: f64 = 134217729.0; // 2^27 + 1

/// CPython `math.hypot(*vec)` / `math.dist` core (Python 3.10 `vector_norm`).
pub fn vector_norm(vec: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for &v in vec {
        let a = v.abs();
        if a.is_nan() {
            nan = true;
        }
        if a > max {
            max = a;
        }
    }
    if max.is_infinite() {
        return max;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 || vec.len() <= 1 {
        return max;
    }
    let (_, e) = frexp(max);
    if e < -1023 {
        // subnormal range: CPython rescales; it never happens for sketch geometry
        let scale = 2f64.powi(1022 - e);
        let scaled: Vec<f64> = vec.iter().map(|v| v * scale).collect();
        return vector_norm(&scaled) / scale;
    }
    let scale = ldexp(1.0, -e);
    let mut csum = 1.0f64;
    let mut frac = 0.0f64;
    let add = |y: f64, csum: &mut f64, frac: &mut f64| {
        let old = *csum;
        *csum += y;
        *frac += (old - *csum) + y;
    };
    for &v in vec {
        let x = v.abs() * scale;
        let t = x * T27;
        let hi = t - (t - x);
        let lo = x - hi;
        add(hi * hi, &mut csum, &mut frac);
        add(2.0 * hi * lo, &mut csum, &mut frac);
        add(lo * lo, &mut csum, &mut frac);
    }
    let h = (csum - 1.0 + frac).sqrt();
    let x = h;
    let t = x * T27;
    let hi = t - (t - x);
    let lo = x - hi;
    add(-hi * hi, &mut csum, &mut frac);
    add(-2.0 * hi * lo, &mut csum, &mut frac);
    add(-lo * lo, &mut csum, &mut frac);
    let x = csum - 1.0 + frac;
    (h + x / (2.0 * h)) / scale
}

/// C `frexp`: x = m 2^e with 0.5 <= |m| < 1.
pub fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // subnormal
        let (m, e) = frexp(x * 2f64.powi(54));
        return (m, e - 54);
    }
    let e = exp - 1022;
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, e)
}

/// C `ldexp`.
pub fn ldexp(x: f64, e: i32) -> f64 {
    if e > 1023 {
        return x * 2f64.powi(1023) * 2f64.powi(e - 1023);
    }
    if e < -1022 {
        return x * 2f64.powi(-1022) * 2f64.powi(e + 1022);
    }
    x * f64::from_bits(((e + 1023) as u64) << 52)
}

/// CPython `math.hypot(x, y)`.
pub fn py_hypot(x: f64, y: f64) -> f64 {
    vector_norm(&[x, y])
}

/// CPython `math.dist(a, b)` for 2-D points.
pub fn py_dist(a: P, b: P) -> f64 {
    vector_norm(&[a[0] - b[0], a[1] - b[1]])
}

/// libm `hypot` (numpy `np.hypot`).
pub fn c_hypot(x: f64, y: f64) -> f64 {
    x.hypot(y)
}

fn pairwise(a: &[f64]) -> f64 {
    let n = a.len();
    if n < 8 {
        let mut res = 0.0;
        for &v in a {
            res += v;
        }
        res
    } else if n <= 128 {
        let mut r = [a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]];
        let mut i = 8;
        while i < n - (n % 8) {
            for j in 0..8 {
                r[j] += a[i + j];
            }
            i += 8;
        }
        let mut res = ((r[0] + r[1]) + (r[2] + r[3])) + ((r[4] + r[5]) + (r[6] + r[7]));
        while i < n {
            res += a[i];
            i += 1;
        }
        res
    } else {
        let mut n2 = n / 2;
        n2 -= n2 % 8;
        pairwise(&a[..n2]) + pairwise(&a[n2..])
    }
}

/// numpy `np.sum` of a 1-D float64 array.
pub fn np_sum(a: &[f64]) -> f64 {
    0.0 + pairwise(a)
}

/// numpy `np.dot(x, y)` of 1-D float64 arrays: OpenBLAS ddot (Haswell kernel: four 4-wide
/// FMA accumulators over blocks of 16, then a plain loop over the rest).
pub fn blas_dot(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    let n1 = n & !15;
    let mut dot = 0.0;
    if n1 > 0 {
        let mut acc = [[0.0f64; 4]; 4];
        let mut i = 0;
        while i < n1 {
            for k in 0..4 {
                for l in 0..4 {
                    let j = i + 4 * k + l;
                    acc[k][l] = x[j].mul_add(y[j], acc[k][l]);
                }
            }
            i += 16;
        }
        let h: Vec<[f64; 2]> = acc.iter().map(|a| [a[0] + a[2], a[1] + a[3]]).collect();
        let s45 = [h[0][0] + h[1][0], h[0][1] + h[1][1]];
        let s67 = [h[2][0] + h[3][0], h[2][1] + h[3][1]];
        let s = [s45[0] + s67[0], s45[1] + s67[1]];
        dot = s[0] + s[1];
    }
    for i in n1..n {
        dot += y[i] * x[i];
    }
    dot
}

/// numpy `np.dot(x, y)` when either array is a strided view (e.g. a column `P[:, 0]`):
/// OpenBLAS ddot's non-unit-stride loop (two accumulators, pairs of products).
pub fn blas_dot_strided(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    let n1 = n & !3;
    let (mut t1, mut t2) = (0.0f64, 0.0f64);
    let mut i = 0;
    while i < n1 {
        let m1 = y[i] * x[i];
        let m2 = y[i + 1] * x[i + 1];
        let m3 = y[i + 2] * x[i + 2];
        let m4 = y[i + 3] * x[i + 3];
        t1 += m1 + m3;
        t2 += m2 + m4;
        i += 4;
    }
    while i < n {
        t1 += y[i] * x[i];
        i += 1;
    }
    t1 + t2
}

/// Python builtin `sum` (left to right, starting from the integer 0).
pub fn py_sum(a: &[f64]) -> f64 {
    let mut it = a.iter();
    let mut s = match it.next() {
        Some(&v) => v,
        None => return 0.0,
    };
    for &v in it {
        s += v;
    }
    s
}

/// `np.cumsum`.
pub fn cumsum(a: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(a.len());
    let mut s = 0.0;
    for (k, &v) in a.iter().enumerate() {
        s = if k == 0 { v } else { s + v };
        out.push(s);
    }
    out
}

/// `np.linspace(start, stop, num)` (endpoint included).
pub fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    if num == 0 {
        return vec![];
    }
    if num == 1 {
        return vec![start];
    }
    let div = (num - 1) as f64;
    let delta = stop - start;
    let step = delta / div;
    let mut out: Vec<f64> = if step == 0.0 {
        (0..num).map(|i| (i as f64) / div * delta + start).collect()
    } else {
        (0..num).map(|i| (i as f64) * step + start).collect()
    };
    out[num - 1] = stop;
    out
}

/// `np.searchsorted(a, v, side="left")`: the first index with a[i] >= v.
pub fn searchsorted_left(a: &[f64], v: f64) -> usize {
    let (mut lo, mut hi) = (0usize, a.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if a[mid] < v {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// `np.clip` for one value.
pub fn clip(x: f64, lo: f64, hi: f64) -> f64 {
    // numpy: minimum(maximum(x, lo), hi)
    let y = if x > lo || x.is_nan() { x } else { lo };
    if y < hi || y.is_nan() {
        y
    } else {
        hi
    }
}

/// Python `min(max(x, lo), hi)` (as written in the reference code).
pub fn py_clamp(x: f64, lo: f64, hi: f64) -> f64 {
    let y = if x > lo { x } else { lo }; // max(x, lo) returns x unless lo > x
    let y = if lo > x { lo } else { y };
    if hi < y {
        hi
    } else {
        y
    }
}

/// Python `round(x, ndigits)` as a value usable as a key (-0.0 becomes 0.0).
pub fn py_round(x: f64, ndigits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let s = format!("{:.*}", ndigits, x);
    s.parse::<f64>().unwrap_or(x) + 0.0
}

/// A hashable key of `round(x, ndigits)`.
pub fn round_key(x: f64, ndigits: usize) -> u64 {
    py_round(x, ndigits).to_bits()
}

/// Python's `x % m` for floats (result has the sign of m).
pub fn py_mod(x: f64, m: f64) -> f64 {
    // CPython float_rem
    let mut r = x % m;
    if r != 0.0 {
        if (m < 0.0) != (r < 0.0) {
            r += m;
        }
    } else {
        r = 0.0f64.copysign(m);
    }
    r
}

/// scipy.optimize.minimize_scalar(fun, bounds=(x1, x2), method="bounded", options={"xatol": xatol}).x
/// (scipy `_minimize_scalar_bounded`, maxiter 500).
pub fn fminbound<F: FnMut(f64) -> f64>(mut func: F, x1: f64, x2: f64, xatol: f64) -> f64 {
    let maxfun = 500;
    let sqrt_eps = (2.2e-16f64).sqrt();
    let golden_mean = 0.5 * (3.0 - 5.0f64.sqrt());
    let (mut a, mut b) = (x1, x2);
    let mut fulc = a + golden_mean * (b - a);
    let (mut nfc, mut xf) = (fulc, fulc);
    let mut rat = 0.0f64;
    let mut e = 0.0f64;
    let mut x = xf;
    let mut fx = func(x);
    let mut num = 1;
    let mut fu;
    let (mut ffulc, mut fnfc) = (fx, fx);
    let mut xm = 0.5 * (a + b);
    let mut tol1 = sqrt_eps * xf.abs() + xatol / 3.0;
    let mut tol2 = 2.0 * tol1;

    while (xf - xm).abs() > (tol2 - 0.5 * (b - a)) {
        let mut golden = true;
        // Check for parabolic fit
        if e.abs() > tol1 {
            golden = false;
            let mut r = (xf - nfc) * (fx - ffulc);
            let q0 = (xf - fulc) * (fx - fnfc);
            let mut p = (xf - fulc) * q0 - (xf - nfc) * r;
            let mut q = 2.0 * (q0 - r);
            if q > 0.0 {
                p = -p;
            }
            q = q.abs();
            r = e;
            e = rat;
            // Check for acceptability of parabola
            if (p.abs() < (0.5 * q * r).abs()) && (p > q * (a - xf)) && (p < q * (b - xf)) {
                rat = (p + 0.0) / q;
                x = xf + rat;
                if ((x - a) < tol2) || ((b - x) < tol2) {
                    let si = sign(xm - xf) + (if xm - xf == 0.0 { 1.0 } else { 0.0 });
                    rat = tol1 * si;
                }
            } else {
                golden = true;
            }
        }
        if golden {
            // do a golden-section step
            e = if xf >= xm { a - xf } else { b - xf };
            rat = golden_mean * e;
        }
        let si = sign(rat) + (if rat == 0.0 { 1.0 } else { 0.0 });
        x = xf + si * rat.abs().max(tol1);
        fu = func(x);
        num += 1;

        if fu <= fx {
            if x >= xf {
                a = xf;
            } else {
                b = xf;
            }
            fulc = nfc;
            ffulc = fnfc;
            nfc = xf;
            fnfc = fx;
            xf = x;
            fx = fu;
        } else {
            if x < xf {
                a = x;
            } else {
                b = x;
            }
            if (fu <= fnfc) || (nfc == xf) {
                fulc = nfc;
                ffulc = fnfc;
                nfc = x;
                fnfc = fu;
            } else if (fu <= ffulc) || (fulc == xf) || (fulc == nfc) {
                fulc = x;
                ffulc = fu;
            }
        }
        xm = 0.5 * (a + b);
        tol1 = sqrt_eps * xf.abs() + xatol / 3.0;
        tol2 = 2.0 * tol1;
        if num >= maxfun {
            break;
        }
    }
    xf
}

fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// scipy.optimize.brentq(f, xa, xb, xtol, rtol, maxiter) (scipy's C `brentq`).
pub fn brentq<F: FnMut(f64) -> f64>(mut f: F, xa: f64, xb: f64, xtol: f64, rtol: f64, iter: usize) -> Result<f64, String> {
    let mut xpre = xa;
    let mut xcur = xb;
    let (mut xblk, mut fblk, mut spre, mut scur) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut fpre = f(xpre);
    let mut fcur = f(xcur);
    if fpre == 0.0 {
        return Ok(xpre);
    }
    if fcur == 0.0 {
        return Ok(xcur);
    }
    if fpre.signum() == fcur.signum() {
        return Err("f(a) and f(b) must have different signs".into());
    }
    for _ in 0..iter {
        if fpre != 0.0 && fcur != 0.0 && (fpre.signum() != fcur.signum()) {
            xblk = xpre;
            fblk = fpre;
            spre = xcur - xpre;
            scur = spre;
        }
        if fblk.abs() < fcur.abs() {
            xpre = xcur;
            xcur = xblk;
            xblk = xpre;
            fpre = fcur;
            fcur = fblk;
            fblk = fpre;
        }
        let delta = (xtol + rtol * xcur.abs()) / 2.0;
        let sbis = (xblk - xcur) / 2.0;
        if fcur == 0.0 || sbis.abs() < delta {
            return Ok(xcur);
        }
        if spre.abs() > delta && fcur.abs() < fpre.abs() {
            
            let stry = if xpre == xblk {
                // interpolate
                -fcur * (xcur - xpre) / (fcur - fpre)
            } else {
                // extrapolate
                let dpre = (fpre - fcur) / (xpre - xcur);
                let dblk = (fblk - fcur) / (xblk - xcur);
                -fcur * (fblk * dblk - fpre * dpre) / (dblk * dpre * (fblk - fpre))
            };
            if 2.0 * stry.abs() < spre.abs().min(3.0 * sbis.abs() - delta) {
                // good short step
                spre = scur;
                scur = stry;
            } else {
                // bisect
                spre = sbis;
                scur = sbis;
            }
        } else {
            // bisect
            spre = sbis;
            scur = sbis;
        }
        xpre = xcur;
        fpre = fcur;
        if scur.abs() > delta {
            xcur += scur;
        } else {
            xcur += if sbis > 0.0 { delta } else { -delta };
        }
        fcur = f(xcur);
    }
    Err("brentq did not converge".into())
}

/// Solve the tridiagonal system dl (sub), d (diagonal), du (super) for one or more right-hand
/// sides, as LAPACK dgtsv does when no row interchanges are needed.
pub fn solve_tridiagonal(dl: &[f64], d: &[f64], du: &[f64], b: &mut [Vec<f64>]) {
    let n = d.len();
    for rhs in b.iter_mut() {
        let mut dd = d.to_vec();
        for i in 0..n.saturating_sub(1) {
            if dd[i] != 0.0 {
                let fact = dl[i] / dd[i];
                dd[i + 1] -= fact * du[i];
                rhs[i + 1] -= fact * rhs[i];
            }
        }
        rhs[n - 1] /= dd[n - 1];
        if n > 1 {
            rhs[n - 2] = (rhs[n - 2] - du[n - 2] * rhs[n - 1]) / dd[n - 2];
        }
        if n > 2 {
            for i in (0..n - 2).rev() {
                rhs[i] = (rhs[i] - du[i] * rhs[i + 1] - 0.0 * rhs[i + 2]) / dd[i];
            }
        }
    }
}

/// `np.unwrap` (period 2 pi) of a sequence of angles.
pub fn unwrap(p: &[f64]) -> Vec<f64> {
    use std::f64::consts::PI;
    let mut out = Vec::with_capacity(p.len());
    if p.is_empty() {
        return out;
    }
    out.push(p[0]);
    let mut correction = 0.0;
    for k in 1..p.len() {
        let dd = p[k] - p[k - 1];
        // ddmod = mod(dd + pi, 2 pi) - pi; with ddmod = pi where ddmod == -pi and dd > 0
        let mut ddmod = py_mod(dd + PI, 2.0 * PI) - PI;
        if ddmod == -PI && dd > 0.0 {
            ddmod = PI;
        }
        let mut ph = ddmod - dd;
        if dd.abs() < PI {
            ph = 0.0;
        }
        correction += ph;
        out.push(p[k] + correction);
    }
    out
}

/// Polynomial-free 2x2 solve as numpy.linalg.solve does (LU with partial pivoting).
pub fn solve2(a: [[f64; 2]; 2], b: [f64; 2]) -> Option<[f64; 2]> {
    let (mut a, mut b) = (a, b);
    if a[1][0].abs() > a[0][0].abs() {
        a.swap(0, 1);
        b.swap(0, 1);
    }
    if a[0][0] == 0.0 {
        return None;
    }
    let l = a[1][0] * (1.0 / a[0][0]);
    let u11 = a[1][1] - l * a[0][1];
    let b1 = b[1] - l * b[0];
    if u11 == 0.0 {
        return None;
    }
    let x1 = b1 / u11;
    let x0 = (b[0] - a[0][1] * x1) / a[0][0];
    Some([x0, x1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linspace_matches_numpy() {
        assert_eq!(linspace(0.0, 1.0, 5), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!(linspace(0.0, 1.0, 3)[1], 0.5);
    }

    #[test]
    fn frexp_ldexp() {
        assert_eq!(frexp(8.0), (0.5, 4));
        assert_eq!(ldexp(0.5, 4), 8.0);
        assert_eq!(frexp(0.75), (0.75, 0));
    }

    #[test]
    fn hypot_simple() {
        assert_eq!(py_hypot(3.0, 4.0), 5.0);
        assert_eq!(py_dist([1.0, 1.0], [4.0, 5.0]), 5.0);
    }

    #[test]
    fn brentq_root() {
        let r = brentq(|x| x * x - 2.0, 0.0, 2.0, 1e-15, 1e-15, 100).unwrap();
        assert!((r - 2f64.sqrt()).abs() < 1e-14);
    }

    #[test]
    fn fminbound_parabola() {
        let x = fminbound(|x| (x - 0.3) * (x - 0.3), 0.0, 1.0, 1e-10);
        assert!((x - 0.3).abs() < 1e-8);
    }
}
