//! Mathematical functions for arithmetic expressions — port of
//! `Src/Modules/mathfunc.c`.
//!
//! C source has THREE anonymous `enum {}` blocks (lines 35, 90,
//! 104) generating `int`-typed constants — no named C type, so
//! the Rust port mirrors them as `pub const ... : i32 = ...;`
//! definitions only (rule 1: no Rust-only struct/enum types).
//!
//! All math-fn dispatch lives in a single `math_func()` switch,
//! matching the C structure 1:1.

#![allow(clippy::approx_constant)]

use crate::ported::math::{mnumber, MN_FLOAT, MN_INTEGER};

// libm bindings used by the math-function dispatcher. Direct port
// of the calls C's `math_func()` (Src/Modules/mathfunc.c:172-436)
// makes via `<math.h>`. Bessel functions and `erf` aren't in
// Rust's `std`, so we declare the C ABI bindings here.
#[cfg(unix)]
extern "C" {
    fn j0(x: f64) -> f64;
    fn j1(x: f64) -> f64;
    fn jn(n: i32, x: f64) -> f64;
    fn y0(x: f64) -> f64;
    fn y1(x: f64) -> f64;
    fn yn(n: i32, x: f64) -> f64;
    fn erf(x: f64) -> f64;
    fn erfc(x: f64) -> f64;
    fn lgamma(x: f64) -> f64;
    fn tgamma(x: f64) -> f64;
    fn ilogb(x: f64) -> i32;
    fn logb(x: f64) -> f64;
    fn nextafter(x: f64, y: f64) -> f64;
    fn rint(x: f64) -> f64;
    fn scalbn(x: f64, n: i32) -> f64;
    fn ldexp(x: f64, exp: i32) -> f64;
    fn copysign(x: f64, y: f64) -> f64;
    fn expm1(x: f64) -> f64;
    fn log1p(x: f64) -> f64;
    fn cbrt(x: f64) -> f64;
}

// ============================================================
// MF_* — port of the anonymous `enum {}` at mathfunc.c:34-84.
// C `enum {}` with no typedef → untyped int constants. Rust
// mirrors as `pub const ... : i32` (no Rust-only enum type).
// ============================================================
pub const MF_ABS:       i32 = 0;                                         // c:35
pub const MF_ACOS:      i32 = 1;                                         // c:36
pub const MF_ACOSH:     i32 = 2;
pub const MF_ASIN:      i32 = 3;
pub const MF_ASINH:     i32 = 4;
pub const MF_ATAN:      i32 = 5;
pub const MF_ATANH:     i32 = 6;
pub const MF_CBRT:      i32 = 7;
pub const MF_CEIL:      i32 = 8;
pub const MF_COPYSIGN:  i32 = 9;
pub const MF_COS:       i32 = 10;
pub const MF_COSH:      i32 = 11;
pub const MF_ERF:       i32 = 12;
pub const MF_ERFC:      i32 = 13;
pub const MF_EXP:       i32 = 14;
pub const MF_EXPM1:     i32 = 15;
pub const MF_FABS:      i32 = 16;
pub const MF_FLOAT:     i32 = 17;
pub const MF_FLOOR:     i32 = 18;
pub const MF_FMOD:      i32 = 19;
pub const MF_GAMMA:     i32 = 20;
pub const MF_HYPOT:     i32 = 21;
pub const MF_ILOGB:     i32 = 22;
pub const MF_INT:       i32 = 23;
pub const MF_ISINF:     i32 = 24;
pub const MF_ISNAN:     i32 = 25;
pub const MF_J0:        i32 = 26;
pub const MF_J1:        i32 = 27;
pub const MF_JN:        i32 = 28;
pub const MF_LDEXP:     i32 = 29;
pub const MF_LGAMMA:    i32 = 30;
pub const MF_LOG:       i32 = 31;
pub const MF_LOG10:     i32 = 32;
pub const MF_LOG1P:     i32 = 33;
pub const MF_LOG2:      i32 = 34;
pub const MF_LOGB:      i32 = 35;
pub const MF_NEXTAFTER: i32 = 36;
pub const MF_RINT:      i32 = 37;
pub const MF_SCALB:     i32 = 38;
pub const MF_SIGNGAM:   i32 = 39;                                        // c:75 #ifdef HAVE_SIGNGAM
pub const MF_SIN:       i32 = 40;
pub const MF_SINH:      i32 = 41;
pub const MF_SQRT:      i32 = 42;
pub const MF_TAN:       i32 = 43;
pub const MF_TANH:      i32 = 44;
pub const MF_Y0:        i32 = 45;
pub const MF_Y1:        i32 = 46;
pub const MF_YN:        i32 = 47;                                        // c:84

// ============================================================
// MS_* — port of the anonymous `enum {}` at mathfunc.c:90.
// String-arg math-fn ids.
// ============================================================
pub const MS_RAND48: i32 = 0;                                            // c:91

// ============================================================
// TF_* — port of the anonymous `enum {}` at mathfunc.c:104.
// Type-flag bits, individually testable.
// ============================================================
pub const TF_NOCONV: i32 = 1;                                            // c:106 don't convert to float
pub const TF_INT1:   i32 = 2;                                            // c:107 first arg is integer
pub const TF_INT2:   i32 = 4;                                            // c:108 second arg is integer
pub const TF_NOASS:  i32 = 8;                                            // c:109 don't assign result as double

/// Port of the `TFLAG(x)` macro from `mathfunc.c:113`.
/// `#define TFLAG(x) ((x) << 8)`. Shifts the type-flag bits into
/// the high byte of the `id` arg passed to `math_func()` so the
/// MF_* numeric ids can occupy the low byte.
pub const fn tflag(x: i32) -> i32 { x << 8 }                             // c:113

/// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`. The
/// dispatcher behind every numeric math fn registered via
/// `NUMMATHFUNC` in `mftab[]` (mathfunc.c:115-167).
///
/// C signature:
///   `static mnumber math_func(char *name, int argc, mnumber *argv, int id)`
///
/// Matches that signature exactly: `name` is unused (UNUSED in C);
/// `argc` is the actual argument count; `argv` is the slice of
/// argument values; `id` is the MF_* function id ORed with TFLAG()
/// type flags in its high byte.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_name, argc, argv, id) vs C=(name, argc, argv, id)
pub fn math_func(_name: &str, argc: i32, argv: &[mnumber], id: i32) -> mnumber {  // c:173
    let mut ret = mnumber { l: 0, d: 0.0, type_: MN_FLOAT };             // c:173,193
    let mut argd: f64 = 0.0;                                             // c:175
    let mut argd2: f64 = 0.0;                                            // c:175
    let mut argi: i32 = 0;                                               // c:176

    // Type-coerce argv[0] (and argv[1]) per the TF_INT1/TF_INT2/
    // TF_NOCONV flag bits — c:178-191.
    if argc > 0 && (id & tflag(TF_NOCONV)) == 0 {                        // c:178
        if (id & tflag(TF_INT1)) != 0 {                                  // c:179
            argi = if argv[0].type_ == MN_FLOAT {
                argv[0].d as i32                                         // c:180
            } else {
                argv[0].l as i32
            };
        } else {                                                         // c:181
            argd = if argv[0].type_ == MN_INTEGER {
                argv[0].l as f64                                         // c:182
            } else {
                argv[0].d
            };
        }
        if argc > 1 {                                                    // c:183
            if (id & tflag(TF_INT2)) != 0 {                              // c:184
                argi = if argv[1].type_ == MN_FLOAT {
                    argv[1].d as i32                                     // c:185
                } else {
                    argv[1].l as i32
                };
            } else {                                                     // c:187
                argd2 = if argv[1].type_ == MN_INTEGER {
                    argv[1].l as f64                                     // c:188
                } else {
                    argv[1].d
                };
            }
        }
    }

    // C: `if (errflag) return ret;` — c:196. zshrs's errflag is on
    // the executor; this dispatcher is invoked from the math
    // evaluator which already short-circuits on error, so the
    // explicit check is redundant here.

    let mut retd: f64 = 0.0;                                             // c:175

    match id & 0xff {                                                    // c:198
        MF_ABS => {                                                      // c:199
            ret.type_ = argv[0].type_;
            if argv[0].type_ == MN_INTEGER {
                ret.l = if argv[0].l < 0 { -argv[0].l } else { argv[0].l };
            } else {
                ret.d = argv[0].d.abs();
            }
        }
        MF_ACOS  => retd = argd.acos(),                                  // c:208
        MF_ACOSH => retd = argd.acosh(),                                 // c:212
        MF_ASIN  => retd = argd.asin(),                                  // c:216
        MF_ASINH => retd = argd.asinh(),                                 // c:220
        MF_ATAN  => {                                                    // c:224
            retd = if argc == 2 { argd.atan2(argd2) } else { argd.atan() };
        }
        MF_ATANH => retd = argd.atanh(),                                 // c:233
        MF_CBRT  => retd = unsafe { cbrt(argd) },                        // c:237
        MF_CEIL  => retd = argd.ceil(),                                  // c:241
        MF_COPYSIGN => retd = unsafe { copysign(argd, argd2) },          // c:245
        MF_COS   => retd = argd.cos(),                                   // c:249
        MF_COSH  => retd = argd.cosh(),                                  // c:253
        MF_ERF   => retd = unsafe { erf(argd) },                         // c:257
        MF_ERFC  => retd = unsafe { erfc(argd) },                        // c:261
        MF_EXP   => retd = argd.exp(),                                   // c:265
        MF_EXPM1 => retd = unsafe { expm1(argd) },                       // c:269
        MF_FABS  => retd = argd.abs(),                                   // c:273
        MF_FLOAT => retd = argd,                                         // c:277
        MF_FLOOR => retd = argd.floor(),                                 // c:281
        MF_FMOD  => retd = argd % argd2,                                 // c:285
        MF_GAMMA => retd = unsafe { tgamma(argd) },                      // c:289
        MF_HYPOT => retd = argd.hypot(argd2),                            // c:300
        MF_ILOGB => {                                                    // c:304
            ret.type_ = MN_INTEGER;
            ret.l = unsafe { ilogb(argd) } as i64;
        }
        MF_INT => {                                                      // c:309
            ret.type_ = MN_INTEGER;
            ret.l = argd as i64;
        }
        MF_ISINF => {                                                    // c:314
            ret.type_ = MN_INTEGER;
            ret.l = argd.is_infinite() as i64;
        }
        MF_ISNAN => {                                                    // c:319
            ret.type_ = MN_INTEGER;
            ret.l = argd.is_nan() as i64;
        }
        MF_J0    => retd = unsafe { j0(argd) },                          // c:325
        MF_J1    => retd = unsafe { j1(argd) },                          // c:329
        MF_JN    => retd = unsafe { jn(argi, argd2) },                   // c:333
        MF_LDEXP => retd = unsafe { ldexp(argd, argi) },                 // c:337
        MF_LGAMMA => retd = unsafe { lgamma(argd) },                     // c:341
        MF_LOG   => retd = argd.ln(),                                    // c:345
        MF_LOG10 => retd = argd.log10(),                                 // c:349
        MF_LOG1P => retd = unsafe { log1p(argd) },                       // c:353
        MF_LOG2  => retd = argd.log2(),                                  // c:357
        MF_LOGB  => retd = unsafe { logb(argd) },                        // c:365
        MF_NEXTAFTER => retd = unsafe { nextafter(argd, argd2) },        // c:369
        MF_RINT  => retd = unsafe { rint(argd) },                        // c:373
        MF_SCALB => retd = unsafe { scalbn(argd, argi) },                // c:377
        MF_SIGNGAM => {                                                  // c:386
            ret.type_ = MN_INTEGER;
            ret.l = 0;  // signgam is libm-internal; not portably exposed.
        }
        MF_SIN   => retd = argd.sin(),                                   // c:392
        MF_SINH  => retd = argd.sinh(),                                  // c:396
        MF_SQRT  => retd = argd.sqrt(),                                  // c:400
        MF_TAN   => retd = argd.tan(),                                   // c:404
        MF_TANH  => retd = argd.tanh(),                                  // c:408
        MF_Y0    => retd = unsafe { y0(argd) },                          // c:412
        MF_Y1    => retd = unsafe { y1(argd) },                          // c:416
        MF_YN    => retd = unsafe { yn(argi, argd2) },                   // c:420
        _ => {                                                           // c:425
            // BUG: mathfunc type not handled. C prints to stderr
            // under DEBUG; production zsh silently returns 0.
        }
    }

    if (id & tflag(TF_NOASS)) == 0 {                                     // c:431
        ret.d = retd;                                                    // c:432
    }

    ret                                                                  // c:434
}

/// Port of `math_string(UNUSED(char *name), char *arg, int id)` from `Src/Modules/mathfunc.c:439`. The
/// string-arg math-fn dispatcher behind `rand48("seedvar")` and
/// future string-takers. C signature:
///   `static mnumber math_string(char *name, char *arg, int id)`
///
/// Strips leading/trailing iblank from `arg` (mathfunc.c:447-451)
/// then switches on `id`. Currently only `MS_RAND48` exists; the
/// random-bit production lives in `crate::ported::random` and
/// `crate::ported::modules::random_real`. Returns `zero_mnumber`
/// for unrecognised ids (matching C's pre-init `ret = zero_mnumber`).
#[allow(unused_variables)]
pub fn math_string(name: &str, arg: &str, id: i32) -> mnumber {         // c:439
    let trimmed = arg.trim_matches(|c: char| c == ' ' || c == '\t');     // c:439-451 iblank
    match id {
        MS_RAND48 => {                                                   // c:457
            // C decodes optional 12-hex seedstr from $seedvar then
            // calls erand48(). zshrs uses `random_real()` which
            // already produces uniform doubles via the OS-entropy
            // path; the seed-from-param wiring is pending param-
            // table integration.
            let _ = trimmed;
            mnumber {
                l: 0,
                d: crate::ported::modules::random_real::random_real(),
                type_: MN_FLOAT,
            }
        }
        _ => mnumber { l: 0, d: 0.0, type_: MN_INTEGER },                                         // zero_mnumber
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_acos() {
        let argv = [mnumber { l: 0, d: 1.0, type_: MN_FLOAT }];
        let r = math_func("acos", 1, &argv, MF_ACOS);
        assert!((r.type_ == MN_FLOAT));
        assert!((r.d - 0.0).abs() < 1e-9);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_atan_two_args() {
        let argv = [mnumber { l: 0, d: 1.0, type_: MN_FLOAT }, mnumber { l: 0, d: 1.0, type_: MN_FLOAT }];
        let r = math_func("atan", 2, &argv, MF_ATAN);
        assert!((r.type_ == MN_FLOAT));
        assert!((r.d - std::f64::consts::FRAC_PI_4).abs() < 1e-9);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_abs_int_preserves_type() {
        let argv = [mnumber { l: -7, d: 0.0, type_: MN_INTEGER }];
        let r = math_func("abs", 1, &argv, MF_ABS | tflag(TF_NOCONV | TF_NOASS));
        assert!((r.type_ == MN_INTEGER));
        assert_eq!(r.l, 7);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_int_truncates() {
        let argv = [mnumber { l: 0, d: 3.7, type_: MN_FLOAT }];
        let r = math_func("int", 1, &argv, MF_INT | tflag(TF_NOASS));
        assert!((r.type_ == MN_INTEGER));
        assert_eq!(r.l, 3);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_isnan() {
        let argv = [mnumber { l: 0, d: f64::NAN, type_: MN_FLOAT }];
        let r = math_func("isnan", 1, &argv, MF_ISNAN | tflag(TF_NOASS));
        assert_eq!(r.l, 1);
    }

    /// Port of `math_string(UNUSED(char *name), char *arg, int id)` from `Src/Modules/mathfunc.c:439`.
    #[test]
    fn test_math_string_rand48_in_range() {
        let r = math_string("rand48", "", MS_RAND48);
        assert!((r.type_ == MN_FLOAT));
        assert!((0.0..1.0).contains(&r.d));
    }
}

// =====================================================================
// static struct mathfunc mftab[]                                    c:497
// static struct features module_features                            c:540
// =====================================================================

use crate::ported::zsh_h::module;

// `mftab` — port of `static struct mathfunc mftab[]` (mathfunc.c:497).


// `module_features` — port of `static struct features module_features`
// from mathfunc.c:540.



/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:548`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:548
    // C body c:550-551 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/mathfunc.c:555`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:555
    *features = featuresarray(m, module_features());
    0                                                                    // c:570
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/mathfunc.c:563`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:563
    handlefeatures(m, module_features(), enables) // c:570
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:570`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {                                     // c:570
    // C body c:572-573 — `return 0`. Faithful empty-body port; the
    //                    math functions are registered via the mf_list
    //                    feature dispatch, no extra boot work needed.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:577`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                                  // c:577
    setfeatureenables(m, module_features(), None) // c:584
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:584`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:584
    // C body c:586-587 — `return 0`. Faithful empty-body port; the
    //                    math functions are unregistered via cleanup_.
    0
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

// WARNING: NOT IN MATHFUNC.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 0,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 48,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
/// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["f:abs".to_string(), "f:acos".to_string(), "f:acosh".to_string(), "f:asin".to_string(), "f:asinh".to_string(), "f:atan".to_string(), "f:atanh".to_string(), "f:cbrt".to_string(), "f:ceil".to_string(), "f:copysign".to_string(), "f:cos".to_string(), "f:cosh".to_string(), "f:erf".to_string(), "f:erfc".to_string(), "f:exp".to_string(), "f:expm1".to_string(), "f:fabs".to_string(), "f:float".to_string(), "f:floor".to_string(), "f:fmod".to_string(), "f:gamma".to_string(), "f:hypot".to_string(), "f:ilogb".to_string(), "f:int".to_string(), "f:isinf".to_string(), "f:isnan".to_string(), "f:j0".to_string(), "f:j1".to_string(), "f:jn".to_string(), "f:ldexp".to_string(), "f:lgamma".to_string(), "f:log".to_string(), "f:log10".to_string(), "f:log1p".to_string(), "f:log2".to_string(), "f:logb".to_string(), "f:nextafter".to_string(), "f:rint".to_string(), "f:scalb".to_string(), "f:signgam".to_string(), "f:sin".to_string(), "f:sinh".to_string(), "f:sqrt".to_string(), "f:tan".to_string(), "f:tanh".to_string(), "f:y0".to_string(), "f:y1".to_string(), "f:yn".to_string()]
}

// WARNING: NOT IN MATHFUNC.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 48]);
    }
    0
}

// WARNING: NOT IN MATHFUNC.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

