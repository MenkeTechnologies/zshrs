//! The ksh93 arithmetic functions zsh/mathfunc does not have.
//!
//! !!! RUST-ONLY MODULE — no zsh C counterpart !!!
//!
//! ksh93 builds its math functions in (ksh93(1) "Arithmetic Evaluation":
//! the C99 `<math.h>` set). Under a bare `--ksh`, `callmathfunc` loads
//! zsh/mathfunc for the names the two shells share (`sqrt`, `sin`, `int`,
//! …) and that module's boot adds this table for the rest: `atan2`, `pow`,
//! `fma`, `fdim`/`fmax`/`fmin`, `round`/`trunc`/`nearbyint`, `exp2`,
//! `tgamma`, `nexttoward`, `signbit` and the classification and comparison
//! macros. Each is the host libm's, as ksh93's are: the double functions
//! where long double is double, else the long double ones ksh93 links
//! (`powl`, `atan2l`, …), computed in `ksh_long_double`. Each answers a
//! float: `print $(( isless(1,2) / 2 ))` is `0.5` in ksh93.

use crate::dash_mode::LDBL_MANT_DIG;
use crate::ksh_long_double::{
    atan2l, exp2l, fdiml, fmal, fmaxl, fminl, is_negative_zero, load, nexttowardl, powl, store_float, tgammal,
    to_integral, LongDouble, Rounding,
};
use crate::ported::math::{mnumber, MN_FLOAT, MN_INTEGER};
use crate::ported::zsh_h::mathfunc;
use std::cmp::Ordering;

#[cfg(unix)]
extern "C" {
    fn atan2(y: f64, x: f64) -> f64;
    fn pow(x: f64, y: f64) -> f64;
    fn fma(x: f64, y: f64, z: f64) -> f64;
    fn fdim(x: f64, y: f64) -> f64;
    fn fmax(x: f64, y: f64) -> f64;
    fn fmin(x: f64, y: f64) -> f64;
    fn round(x: f64) -> f64;
    fn trunc(x: f64) -> f64;
    fn nearbyint(x: f64) -> f64;
    fn exp2(x: f64) -> f64;
    fn tgamma(x: f64) -> f64;
    fn nextafter(x: f64, y: f64) -> f64;
}

const KF_ATAN2: i32 = 0;
const KF_POW: i32 = 1;
const KF_FMA: i32 = 2;
const KF_FDIM: i32 = 3;
const KF_FMAX: i32 = 4;
const KF_FMIN: i32 = 5;
const KF_ROUND: i32 = 6;
const KF_TRUNC: i32 = 7;
const KF_NEARBYINT: i32 = 8;
const KF_EXP2: i32 = 9;
const KF_TGAMMA: i32 = 10;
const KF_NEXTTOWARD: i32 = 11;
const KF_SIGNBIT: i32 = 12;
const KF_ISFINITE: i32 = 13;
const KF_ISNORMAL: i32 = 14;
const KF_ISSUBNORMAL: i32 = 15;
const KF_ISZERO: i32 = 16;
const KF_ISUNORDERED: i32 = 17;
const KF_ISGREATER: i32 = 18;
const KF_ISGREATEREQUAL: i32 = 19;
const KF_ISLESS: i32 = 20;
const KF_ISLESSEQUAL: i32 = 21;
const KF_ISLESSGREATER: i32 = 22;

/// `(name, min args, max args, id)` — ksh93's arity for each.
const TABLE: &[(&str, i32, i32, i32)] = &[
    ("atan2", 2, 2, KF_ATAN2),
    ("pow", 2, 2, KF_POW),
    ("fma", 3, 3, KF_FMA),
    ("fdim", 2, 2, KF_FDIM),
    ("fmax", 2, 2, KF_FMAX),
    ("fmin", 2, 2, KF_FMIN),
    ("round", 1, 1, KF_ROUND),
    ("trunc", 1, 1, KF_TRUNC),
    ("nearbyint", 1, 1, KF_NEARBYINT),
    ("exp2", 1, 1, KF_EXP2),
    ("tgamma", 1, 1, KF_TGAMMA),
    ("nexttoward", 2, 2, KF_NEXTTOWARD),
    ("signbit", 1, 1, KF_SIGNBIT),
    ("isfinite", 1, 1, KF_ISFINITE),
    ("isnormal", 1, 1, KF_ISNORMAL),
    ("issubnormal", 1, 1, KF_ISSUBNORMAL),
    ("iszero", 1, 1, KF_ISZERO),
    ("isunordered", 2, 2, KF_ISUNORDERED),
    ("isgreater", 2, 2, KF_ISGREATER),
    ("isgreaterequal", 2, 2, KF_ISGREATEREQUAL),
    ("isless", 2, 2, KF_ISLESS),
    ("islessequal", 2, 2, KF_ISLESSEQUAL),
    ("islessgreater", 2, 2, KF_ISLESSGREATER),
];

/// An operand as a double. An integer-typed negative zero (see
/// `ksh_long_double::store`) keeps its sign, so `signbit(-1*0)` sees it.
fn arg(n: &mnumber) -> f64 {
    if n.type_ == MN_INTEGER {
        if crate::ksh_long_double::is_negative_zero(n) {
            -0.0
        } else {
            n.l as f64
        }
    } else {
        n.d
    }
}

/// The `NumMathFunc` behind every entry of [`TABLE`]. `callmathfunc` has
/// already checked the argument count against the entry's bounds.
fn ksh_math_func(_name: &str, _argc: i32, argv: &[mnumber], id: i32) -> mnumber {
    if LDBL_MANT_DIG == 53 {
        return double_math_func(argv, id);
    }
    let args: Vec<LongDouble> = argv.iter().map(long_arg).collect();
    store_float(long_double_math_func(&args, id, LDBL_MANT_DIG))
}

/// An operand as a long double, keeping an integer-typed `-0`'s sign.
fn long_arg(n: &mnumber) -> LongDouble {
    if is_negative_zero(n) {
        LongDouble::from_f64(-0.0)
    } else {
        load(n)
    }
}

/// The table on a host whose long double is wider than double: ksh93 links
/// the `l` variants there (`powl`, `atan2l`, …; features/math.sh).
fn long_double_math_func(args: &[LongDouble], id: i32, prec: u32) -> LongDouble {
    let zero = LongDouble::ZERO;
    let x = args.first().copied().unwrap_or(zero);
    let y = args.get(1).copied().unwrap_or(zero);
    let z = args.get(2).copied().unwrap_or(zero);
    let truth = |b: bool| LongDouble::from_i64(i64::from(b));
    let order = x.partial_cmp(&y);
    match id {
        KF_ATAN2 => atan2l(x, y, prec),
        KF_POW => powl(x, y, prec),
        KF_FMA => fmal(x, y, z, prec),
        KF_FDIM => fdiml(x, y, prec),
        KF_FMAX => fmaxl(x, y, prec),
        KF_FMIN => fminl(x, y),
        KF_ROUND => to_integral(x, Rounding::Round, prec),
        KF_TRUNC => to_integral(x, Rounding::Trunc, prec),
        KF_NEARBYINT => to_integral(x, Rounding::NearestEven, prec),
        KF_EXP2 => exp2l(x, prec),
        KF_TGAMMA => tgammal(x, prec),
        KF_NEXTTOWARD => nexttowardl(x, y, prec),
        // x87's `signbit` answers 0x200, the status-word bit `fxam` sets
        // from the sign: ksh93u+m 1.0.10 prints 512 for `signbit(-1.0)` on
        // x86_64 Linux.
        KF_SIGNBIT if x.is_sign_negative() && prec == 64 => LongDouble::from_i64(512),
        KF_SIGNBIT => truth(x.is_sign_negative()),
        KF_ISFINITE => truth(x.is_finite()),
        KF_ISNORMAL => truth(x.is_normal_p(prec)),
        KF_ISSUBNORMAL => truth(x.is_subnormal_p(prec)),
        KF_ISZERO => truth(x.is_zero()),
        KF_ISUNORDERED => truth(order.is_none()),
        KF_ISGREATER => truth(order == Some(Ordering::Greater)),
        KF_ISGREATEREQUAL => truth(matches!(order, Some(Ordering::Greater | Ordering::Equal))),
        KF_ISLESS => truth(order == Some(Ordering::Less)),
        KF_ISLESSEQUAL => truth(matches!(order, Some(Ordering::Less | Ordering::Equal))),
        KF_ISLESSGREATER => truth(matches!(order, Some(Ordering::Less | Ordering::Greater))),
        _ => zero,
    }
}

/// The table where long double is double (Apple arm64): the host libm's
/// double functions.
fn double_math_func(argv: &[mnumber], id: i32) -> mnumber {
    let x = argv.first().map(arg).unwrap_or(0.0);
    let y = argv.get(1).map(arg).unwrap_or(0.0);
    let z = argv.get(2).map(arg).unwrap_or(0.0);
    let truth = |b: bool| if b { 1.0 } else { 0.0 };
    let d = unsafe {
        match id {
            KF_ATAN2 => atan2(x, y),
            KF_POW => pow(x, y),
            KF_FMA => fma(x, y, z),
            KF_FDIM => fdim(x, y),
            KF_FMAX => fmax(x, y),
            KF_FMIN => fmin(x, y),
            KF_ROUND => round(x),
            KF_TRUNC => trunc(x),
            KF_NEARBYINT => nearbyint(x),
            KF_EXP2 => exp2(x),
            KF_TGAMMA => tgamma(x),
            KF_NEXTTOWARD => nextafter(x, y),
            KF_SIGNBIT => truth(x.is_sign_negative()),
            KF_ISFINITE => truth(x.is_finite()),
            KF_ISNORMAL => truth(x.is_normal()),
            KF_ISSUBNORMAL => truth(x.is_subnormal()),
            KF_ISZERO => truth(x == 0.0),
            KF_ISUNORDERED => truth(x.is_nan() || y.is_nan()),
            KF_ISGREATER => truth(x > y),
            KF_ISGREATEREQUAL => truth(x >= y),
            KF_ISLESS => truth(x < y),
            KF_ISLESSEQUAL => truth(x <= y),
            KF_ISLESSGREATER => truth(x < y || x > y),
            _ => 0.0,
        }
    };
    mnumber { l: 0, d, type_: MN_FLOAT }
}

/// Adds every [`TABLE`] function to the global math-function list. A name
/// already defined (a `functions -M` of the user's) is left as it is,
/// the way `addmathfunc` refuses a clash.
pub fn add_ksh93_mathfuncs() {
    for &(name, minargs, maxargs, funcid) in TABLE {
        let _ = crate::ported::module::addmathfunc(mathfunc {
            next: None,
            name: name.to_string(),
            flags: 0,
            nfunc: Some(ksh_math_func as crate::ported::zsh_h::NumMathFunc),
            sfunc: None,
            module: None,
            minargs,
            maxargs,
            funcid,
        });
    }
}

/// Removes the [`TABLE`] functions again, when zsh/mathfunc is unloaded.
pub fn remove_ksh93_mathfuncs() {
    if let Ok(mut tab) = crate::ported::module::MATHFUNCS.lock() {
        tab.retain(|f| {
            // A user `functions -M` of the same name carries MFF_USERFUNC.
            !((f.flags & crate::ported::zsh_h::MFF_USERFUNC) == 0
                && f.module.is_none()
                && TABLE.iter().any(|&(n, ..)| n == f.name))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: f64) -> mnumber {
        mnumber { l: 0, d: v, type_: MN_FLOAT }
    }
    fn i(v: i64) -> mnumber {
        mnumber { l: v, d: 0.0, type_: MN_INTEGER }
    }
    fn call(id: i32, args: &[mnumber]) -> f64 {
        ksh_math_func("", args.len() as i32, args, id).d
    }

    #[test]
    fn libm_results_match_ksh93() {
        // Values printed by ksh93u+m 1.0.10 for the same calls.
        assert_eq!(call(KF_POW, &[i(2), i(10)]), 1024.0);
        assert_eq!(call(KF_FMA, &[i(2), i(3), i(4)]), 10.0);
        assert_eq!(call(KF_FDIM, &[i(3), i(5)]), 0.0);
        assert_eq!(call(KF_FMAX, &[f(f64::NAN), i(1)]), 1.0);
        assert_eq!(call(KF_ROUND, &[f(-2.5)]), -3.0);
        assert_eq!(call(KF_NEARBYINT, &[f(2.5)]), 2.0);
        assert_eq!(call(KF_TRUNC, &[f(-2.7)]), -2.0);
        assert_eq!(call(KF_TGAMMA, &[i(5)]), 24.0);
        assert!(call(KF_ATAN2, &[f(-0.0), i(-1)]) < -3.14);
    }

    #[test]
    fn predicates_answer_one_or_zero_and_see_the_sign_of_zero() {
        // x87's signbit is 0x200 (ksh93u+m prints 512 on x86_64 Linux).
        let negative = if LDBL_MANT_DIG == 64 { 512.0 } else { 1.0 };
        assert_eq!(call(KF_SIGNBIT, &[f(-0.0)]), negative);
        assert_eq!(call(KF_SIGNBIT, &[i(0)]), 0.0);
        assert_eq!(call(KF_ISZERO, &[f(-0.0)]), 1.0);
        assert_eq!(call(KF_ISUNORDERED, &[f(f64::NAN), i(1)]), 1.0);
        assert_eq!(call(KF_ISGREATER, &[f(f64::NAN), i(1)]), 0.0);
        assert_eq!(call(KF_ISLESSGREATER, &[i(1), i(1)]), 0.0);
        assert_eq!(call(KF_ISFINITE, &[f(f64::INFINITY)]), 0.0);
    }
}
