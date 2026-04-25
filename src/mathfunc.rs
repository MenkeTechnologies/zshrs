//! Mathematical functions for arithmetic expressions - port of Modules/mathfunc.c
//!
//! Provides standard math functions like sin, cos, sqrt, log, etc.

use std::f64::consts::PI;

/// Math number type - can be integer or float
#[derive(Debug, Clone, Copy)]
pub enum MathNumber {
    Integer(i64),
    Float(f64),
}

impl MathNumber {
    pub fn as_float(&self) -> f64 {
        match self {
            MathNumber::Integer(i) => *i as f64,
            MathNumber::Float(f) => *f,
        }
    }

    pub fn as_int(&self) -> i64 {
        match self {
            MathNumber::Integer(i) => *i,
            MathNumber::Float(f) => *f as i64,
        }
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, MathNumber::Integer(_))
    }
}

impl From<i64> for MathNumber {
    fn from(i: i64) -> Self {
        MathNumber::Integer(i)
    }
}

impl From<f64> for MathNumber {
    fn from(f: f64) -> Self {
        MathNumber::Float(f)
    }
}

/// Math function registry
pub struct MathFunctions;

impl MathFunctions {
    /// Evaluate a math function by name
    pub fn call(name: &str, args: &[MathNumber]) -> Result<MathNumber, String> {
        match name {
            "abs" => Self::abs(args),
            "acos" => Self::unary_float(args, f64::acos),
            "acosh" => Self::unary_float(args, f64::acosh),
            "asin" => Self::unary_float(args, f64::asin),
            "asinh" => Self::unary_float(args, f64::asinh),
            "atan" => Self::atan(args),
            "atanh" => Self::unary_float(args, f64::atanh),
            "cbrt" => Self::unary_float(args, f64::cbrt),
            "ceil" => Self::unary_float(args, f64::ceil),
            "copysign" => Self::binary_float(args, f64::copysign),
            "cos" => Self::unary_float(args, f64::cos),
            "cosh" => Self::unary_float(args, f64::cosh),
            "erf" => Self::erf(args),
            "erfc" => Self::erfc(args),
            "exp" => Self::unary_float(args, f64::exp),
            "expm1" => Self::unary_float(args, f64::exp_m1),
            "fabs" => Self::unary_float(args, f64::abs),
            "float" => Self::to_float(args),
            "floor" => Self::unary_float(args, f64::floor),
            "fmod" => Self::binary_float(args, |a, b| a % b),
            "gamma" | "tgamma" => Self::gamma(args),
            "hypot" => Self::binary_float(args, f64::hypot),
            "ilogb" => Self::ilogb(args),
            "int" => Self::to_int(args),
            "isinf" => Self::isinf(args),
            "isnan" => Self::isnan(args),
            "j0" => Self::j0(args),
            "j1" => Self::j1(args),
            "jn" => Self::jn(args),
            "ldexp" => Self::ldexp(args),
            "lgamma" => Self::lgamma(args),
            "log" | "ln" => Self::unary_float(args, f64::ln),
            "log10" => Self::unary_float(args, f64::log10),
            "log1p" => Self::unary_float(args, f64::ln_1p),
            "log2" => Self::unary_float(args, f64::log2),
            "logb" => Self::logb(args),
            "max" => Self::max(args),
            "min" => Self::min(args),
            "nextafter" => Self::nextafter(args),
            "pow" => Self::binary_float(args, f64::powf),
            "rint" | "round" => Self::unary_float(args, f64::round),
            "scalb" | "scalbn" => Self::scalbn(args),
            "sin" => Self::unary_float(args, f64::sin),
            "sinh" => Self::unary_float(args, f64::sinh),
            "sqrt" => Self::unary_float(args, f64::sqrt),
            "tan" => Self::unary_float(args, f64::tan),
            "tanh" => Self::unary_float(args, f64::tanh),
            "trunc" => Self::unary_float(args, f64::trunc),
            "y0" => Self::y0(args),
            "y1" => Self::y1(args),
            "yn" => Self::yn(args),
            _ => Err(format!("unknown function: {}", name)),
        }
    }

    /// List all available functions
    pub fn list() -> Vec<&'static str> {
        vec![
            "abs",
            "acos",
            "acosh",
            "asin",
            "asinh",
            "atan",
            "atanh",
            "cbrt",
            "ceil",
            "copysign",
            "cos",
            "cosh",
            "erf",
            "erfc",
            "exp",
            "expm1",
            "fabs",
            "float",
            "floor",
            "fmod",
            "gamma",
            "hypot",
            "ilogb",
            "int",
            "isinf",
            "isnan",
            "j0",
            "j1",
            "jn",
            "ldexp",
            "lgamma",
            "log",
            "log10",
            "log1p",
            "log2",
            "logb",
            "max",
            "min",
            "nextafter",
            "pow",
            "rint",
            "round",
            "scalb",
            "scalbn",
            "sin",
            "sinh",
            "sqrt",
            "tan",
            "tanh",
            "trunc",
            "y0",
            "y1",
            "yn",
        ]
    }

    fn check_args(args: &[MathNumber], min: usize, max: usize, name: &str) -> Result<(), String> {
        if args.len() < min {
            return Err(format!("{}: not enough arguments", name));
        }
        if args.len() > max {
            return Err(format!("{}: too many arguments", name));
        }
        Ok(())
    }

    fn unary_float(args: &[MathNumber], f: fn(f64) -> f64) -> Result<MathNumber, String> {
        if args.is_empty() {
            return Err("not enough arguments".to_string());
        }
        Ok(MathNumber::Float(f(args[0].as_float())))
    }

    fn binary_float(args: &[MathNumber], f: fn(f64, f64) -> f64) -> Result<MathNumber, String> {
        if args.len() < 2 {
            return Err("not enough arguments".to_string());
        }
        Ok(MathNumber::Float(f(args[0].as_float(), args[1].as_float())))
    }

    fn abs(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "abs")?;
        match args[0] {
            MathNumber::Integer(i) => Ok(MathNumber::Integer(i.abs())),
            MathNumber::Float(f) => Ok(MathNumber::Float(f.abs())),
        }
    }

    fn atan(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 2, "atan")?;
        if args.len() == 2 {
            Ok(MathNumber::Float(
                args[0].as_float().atan2(args[1].as_float()),
            ))
        } else {
            Ok(MathNumber::Float(args[0].as_float().atan()))
        }
    }

    fn to_float(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "float")?;
        Ok(MathNumber::Float(args[0].as_float()))
    }

    fn to_int(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "int")?;
        Ok(MathNumber::Integer(args[0].as_int()))
    }

    fn isinf(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "isinf")?;
        let f = args[0].as_float();
        Ok(MathNumber::Integer(if f.is_infinite() { 1 } else { 0 }))
    }

    fn isnan(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "isnan")?;
        let f = args[0].as_float();
        Ok(MathNumber::Integer(if f.is_nan() { 1 } else { 0 }))
    }

    fn ilogb(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "ilogb")?;
        let f = args[0].as_float();
        if f == 0.0 {
            return Ok(MathNumber::Integer(i64::MIN));
        }
        Ok(MathNumber::Integer(f.abs().log2().floor() as i64))
    }

    fn logb(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "logb")?;
        let f = args[0].as_float();
        if f == 0.0 {
            return Ok(MathNumber::Float(f64::NEG_INFINITY));
        }
        Ok(MathNumber::Float(f.abs().log2().floor()))
    }

    fn ldexp(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 2, 2, "ldexp")?;
        let x = args[0].as_float();
        let exp = args[1].as_int() as i32;
        Ok(MathNumber::Float(x * 2f64.powi(exp)))
    }

    fn scalbn(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::ldexp(args)
    }

    fn nextafter(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 2, 2, "nextafter")?;
        let x = args[0].as_float();
        let y = args[1].as_float();

        if x == y {
            return Ok(MathNumber::Float(y));
        }

        let bits = x.to_bits();
        let next_bits = if (y > x) == (x >= 0.0) {
            bits.wrapping_add(1)
        } else {
            bits.wrapping_sub(1)
        };
        Ok(MathNumber::Float(f64::from_bits(next_bits)))
    }

    fn max(args: &[MathNumber]) -> Result<MathNumber, String> {
        if args.is_empty() {
            return Err("max: not enough arguments".to_string());
        }
        let mut max_val = args[0].as_float();
        for arg in &args[1..] {
            let val = arg.as_float();
            if val > max_val {
                max_val = val;
            }
        }
        if args.iter().all(|a| a.is_integer()) {
            Ok(MathNumber::Integer(max_val as i64))
        } else {
            Ok(MathNumber::Float(max_val))
        }
    }

    fn min(args: &[MathNumber]) -> Result<MathNumber, String> {
        if args.is_empty() {
            return Err("min: not enough arguments".to_string());
        }
        let mut min_val = args[0].as_float();
        for arg in &args[1..] {
            let val = arg.as_float();
            if val < min_val {
                min_val = val;
            }
        }
        if args.iter().all(|a| a.is_integer()) {
            Ok(MathNumber::Integer(min_val as i64))
        } else {
            Ok(MathNumber::Float(min_val))
        }
    }

    fn gamma(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "gamma")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(gamma_fn(x)))
    }

    fn lgamma(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "lgamma")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(lgamma_fn(x)))
    }

    fn erf(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "erf")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(erf_fn(x)))
    }

    fn erfc(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "erfc")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(1.0 - erf_fn(x)))
    }

    fn j0(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "j0")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(bessel_j0(x)))
    }

    fn j1(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "j1")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(bessel_j1(x)))
    }

    fn jn(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 2, 2, "jn")?;
        let n = args[0].as_int() as i32;
        let x = args[1].as_float();
        Ok(MathNumber::Float(bessel_jn(n, x)))
    }

    fn y0(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "y0")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(bessel_y0(x)))
    }

    fn y1(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 1, 1, "y1")?;
        let x = args[0].as_float();
        Ok(MathNumber::Float(bessel_y1(x)))
    }

    fn yn(args: &[MathNumber]) -> Result<MathNumber, String> {
        Self::check_args(args, 2, 2, "yn")?;
        let n = args[0].as_int() as i32;
        let x = args[1].as_float();
        Ok(MathNumber::Float(bessel_yn(n, x)))
    }
}

fn gamma_fn(x: f64) -> f64 {
    if x <= 0.0 && x == x.floor() {
        return f64::INFINITY;
    }

    if x < 0.5 {
        PI / (PI * x).sin() / gamma_fn(1.0 - x)
    } else {
        let x = x - 1.0;
        let g = 7;
        let c = [
            0.99999999999980993,
            676.5203681218851,
            -1259.1392167224028,
            771.32342877765313,
            -176.61502916214059,
            12.507343278686905,
            -0.13857109526572012,
            9.9843695780195716e-6,
            1.5056327351493116e-7,
        ];

        let mut sum = c[0];
        for (i, &coef) in c.iter().enumerate().skip(1) {
            sum += coef / (x + i as f64);
        }

        let t = x + g as f64 + 0.5;
        (2.0 * PI).sqrt() * t.powf(x + 0.5) * (-t).exp() * sum
    }
}

fn lgamma_fn(x: f64) -> f64 {
    gamma_fn(x).abs().ln()
}

fn erf_fn(x: f64) -> f64 {
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let p = 0.3275911;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();

    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();

    sign * y
}

fn bessel_j0(x: f64) -> f64 {
    let x = x.abs();
    if x < 8.0 {
        let y = x * x;
        let ans1 = 57568490574.0
            + y * (-13362590354.0
                + y * (651619640.7 + y * (-11214424.18 + y * (77392.33017 + y * (-184.9052456)))));
        let ans2 = 57568490411.0
            + y * (1029532985.0
                + y * (9494680.718 + y * (59272.64853 + y * (267.8532712 + y * 1.0))));
        ans1 / ans2
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 0.785398164;
        let ans1 = 1.0
            + y * (-0.1098628627e-2
                + y * (0.2734510407e-4 + y * (-0.2073370639e-5 + y * 0.2093887211e-6)));
        let ans2 = -0.1562499995e-1
            + y * (0.1430488765e-3
                + y * (-0.6911147651e-5 + y * (0.7621095161e-6 - y * 0.934945152e-7)));
        (0.636619772 / x).sqrt() * (xx.cos() * ans1 - z * xx.sin() * ans2)
    }
}

fn bessel_j1(x: f64) -> f64 {
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let ans1 = x
            * (72362614232.0
                + y * (-7895059235.0
                    + y * (242396853.1
                        + y * (-2972611.439 + y * (15704.48260 + y * (-30.16036606))))));
        let ans2 = 144725228442.0
            + y * (2300535178.0
                + y * (18583304.74 + y * (99447.43394 + y * (376.9991397 + y * 1.0))));
        ans1 / ans2
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356194491;
        let ans1 = 1.0
            + y * (0.183105e-2
                + y * (-0.3516396496e-4 + y * (0.2457520174e-5 + y * (-0.240337019e-6))));
        let ans2 = 0.04687499995
            + y * (-0.2002690873e-3
                + y * (0.8449199096e-5 + y * (-0.88228987e-6 + y * 0.105787412e-6)));
        let ans = (0.636619772 / ax).sqrt() * (xx.cos() * ans1 - z * xx.sin() * ans2);
        if x < 0.0 {
            -ans
        } else {
            ans
        }
    }
}

fn bessel_jn(n: i32, x: f64) -> f64 {
    match n {
        0 => bessel_j0(x),
        1 => bessel_j1(x),
        _ => {
            if x == 0.0 {
                return 0.0;
            }
            let n = n.unsigned_abs() as usize;
            let ax = x.abs();

            if ax > n as f64 {
                let mut bjm = bessel_j0(ax);
                let mut bj = bessel_j1(ax);
                for j in 1..n {
                    let bjp = 2.0 * j as f64 / ax * bj - bjm;
                    bjm = bj;
                    bj = bjp;
                }
                bj
            } else {
                let tox = 2.0 / ax;
                let m = 2 * ((n + (((40.0 * n as f64).sqrt()) as usize)) / 2);
                let mut bjp = 0.0;
                let mut bj = 1.0;
                let mut ans = 0.0;
                let mut sum = 0.0;
                for j in (1..=m).rev() {
                    let bjm = j as f64 * tox * bj - bjp;
                    bjp = bj;
                    bj = bjm;
                    if bj.abs() > 1e10 {
                        bj *= 1e-10;
                        bjp *= 1e-10;
                        ans *= 1e-10;
                        sum *= 1e-10;
                    }
                    if j % 2 != 0 {
                        sum += bj;
                    }
                    if j == n {
                        ans = bjp;
                    }
                }
                sum = 2.0 * sum - bj;
                ans /= sum;
                if x < 0.0 && n % 2 != 0 {
                    -ans
                } else {
                    ans
                }
            }
        }
    }
}

fn bessel_y0(x: f64) -> f64 {
    if x < 8.0 {
        let y = x * x;
        let ans1 = -2957821389.0
            + y * (7062834065.0
                + y * (-512359803.6 + y * (10879881.29 + y * (-86327.92757 + y * 228.4622733))));
        let ans2 = 40076544269.0
            + y * (745249964.8
                + y * (7189466.438 + y * (47447.26470 + y * (226.1030244 + y * 1.0))));
        ans1 / ans2 + 0.636619772 * bessel_j0(x) * x.ln()
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 0.785398164;
        let ans1 = 1.0
            + y * (-0.1098628627e-2
                + y * (0.2734510407e-4 + y * (-0.2073370639e-5 + y * 0.2093887211e-6)));
        let ans2 = -0.1562499995e-1
            + y * (0.1430488765e-3
                + y * (-0.6911147651e-5 + y * (0.7621095161e-6 + y * (-0.934945152e-7))));
        (0.636619772 / x).sqrt() * (xx.sin() * ans1 + z * xx.cos() * ans2)
    }
}

fn bessel_y1(x: f64) -> f64 {
    if x < 8.0 {
        let y = x * x;
        let ans1 = x
            * (-0.4900604943e13
                + y * (0.1275274390e13
                    + y * (-0.5153438139e11
                        + y * (0.7349264551e9 + y * (-0.4237922726e7 + y * 0.8511937935e4)))));
        let ans2 = 0.2499580570e14
            + y * (0.4244419664e12
                + y * (0.3733650367e10
                    + y * (0.2245904002e8 + y * (0.1020426050e6 + y * (0.3549632885e3 + y)))));
        ans1 / ans2 + 0.636619772 * (bessel_j1(x) * x.ln() - 1.0 / x)
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 2.356194491;
        let ans1 = 1.0
            + y * (0.183105e-2
                + y * (-0.3516396496e-4 + y * (0.2457520174e-5 + y * (-0.240337019e-6))));
        let ans2 = 0.04687499995
            + y * (-0.2002690873e-3
                + y * (0.8449199096e-5 + y * (-0.88228987e-6 + y * 0.105787412e-6)));
        (0.636619772 / x).sqrt() * (xx.sin() * ans1 + z * xx.cos() * ans2)
    }
}

fn bessel_yn(n: i32, x: f64) -> f64 {
    match n {
        0 => bessel_y0(x),
        1 => bessel_y1(x),
        _ => {
            let tox = 2.0 / x;
            let mut bym = bessel_y0(x);
            let mut by = bessel_y1(x);
            for j in 1..n {
                let byp = j as f64 * tox * by - bym;
                bym = by;
                by = byp;
            }
            by
        }
    }
}

/// Mathematical constants
pub mod constants {
    pub const PI: f64 = std::f64::consts::PI;
    pub const E: f64 = std::f64::consts::E;
    pub const TAU: f64 = std::f64::consts::TAU;
    pub const PHI: f64 = 1.618033988749895; // Golden ratio
    pub const SQRT2: f64 = std::f64::consts::SQRT_2;
    pub const LN2: f64 = std::f64::consts::LN_2;
    pub const LN10: f64 = std::f64::consts::LN_10;
}

#[cfg(test)]
mod tests {
    use super::*;
    use constants::E;

    #[test]
    fn test_abs() {
        let result = MathFunctions::call("abs", &[MathNumber::Integer(-5)]).unwrap();
        assert!(matches!(result, MathNumber::Integer(5)));

        let result = MathFunctions::call("abs", &[MathNumber::Float(-3.14)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 3.14).abs() < 1e-10);
        } else {
            panic!("expected float");
        }
    }

    #[test]
    fn test_trig() {
        let result = MathFunctions::call("sin", &[MathNumber::Float(0.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!(f.abs() < 1e-10);
        }

        let result = MathFunctions::call("cos", &[MathNumber::Float(0.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 1.0).abs() < 1e-10);
        }

        let result = MathFunctions::call("tan", &[MathNumber::Float(0.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!(f.abs() < 1e-10);
        }
    }

    #[test]
    fn test_sqrt() {
        let result = MathFunctions::call("sqrt", &[MathNumber::Float(4.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 2.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_log() {
        let result = MathFunctions::call("log", &[MathNumber::Float(E)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 1.0).abs() < 1e-10);
        }

        let result = MathFunctions::call("log10", &[MathNumber::Float(100.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 2.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_exp() {
        let result = MathFunctions::call("exp", &[MathNumber::Float(1.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - E).abs() < 1e-10);
        }
    }

    #[test]
    fn test_floor_ceil() {
        let result = MathFunctions::call("floor", &[MathNumber::Float(3.7)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 3.0).abs() < 1e-10);
        }

        let result = MathFunctions::call("ceil", &[MathNumber::Float(3.2)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 4.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_pow() {
        let result =
            MathFunctions::call("pow", &[MathNumber::Float(2.0), MathNumber::Float(3.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 8.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_atan2() {
        let result =
            MathFunctions::call("atan", &[MathNumber::Float(1.0), MathNumber::Float(1.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - PI / 4.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_hypot() {
        let result =
            MathFunctions::call("hypot", &[MathNumber::Float(3.0), MathNumber::Float(4.0)])
                .unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 5.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_min_max() {
        let result = MathFunctions::call(
            "max",
            &[
                MathNumber::Integer(1),
                MathNumber::Integer(5),
                MathNumber::Integer(3),
            ],
        )
        .unwrap();
        assert!(matches!(result, MathNumber::Integer(5)));

        let result = MathFunctions::call(
            "min",
            &[
                MathNumber::Float(1.5),
                MathNumber::Float(0.5),
                MathNumber::Float(2.5),
            ],
        )
        .unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 0.5).abs() < 1e-10);
        }
    }

    #[test]
    fn test_int_float_conversion() {
        let result = MathFunctions::call("int", &[MathNumber::Float(3.7)]).unwrap();
        assert!(matches!(result, MathNumber::Integer(3)));

        let result = MathFunctions::call("float", &[MathNumber::Integer(5)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 5.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_isinf_isnan() {
        let result = MathFunctions::call("isinf", &[MathNumber::Float(f64::INFINITY)]).unwrap();
        assert!(matches!(result, MathNumber::Integer(1)));

        let result = MathFunctions::call("isinf", &[MathNumber::Float(1.0)]).unwrap();
        assert!(matches!(result, MathNumber::Integer(0)));

        let result = MathFunctions::call("isnan", &[MathNumber::Float(f64::NAN)]).unwrap();
        assert!(matches!(result, MathNumber::Integer(1)));
    }

    #[test]
    fn test_bessel_j0() {
        let result = MathFunctions::call("j0", &[MathNumber::Float(0.0)]).unwrap();
        if let MathNumber::Float(f) = result {
            assert!((f - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn test_list() {
        let funcs = MathFunctions::list();
        assert!(funcs.contains(&"sin"));
        assert!(funcs.contains(&"cos"));
        assert!(funcs.contains(&"sqrt"));
        assert!(funcs.contains(&"log"));
    }
}
