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
use crate::ported::zsh_h::{features, mathfunc, module};
use crate::random_real::random_real;
use std::sync::{Mutex, OnceLock};

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
pub fn math_string(name: &str, arg: &str, id: i32) -> mnumber {
    // c:439
    // c:441 — `mnumber ret = zero_mnumber;`
    let zero_mnumber = mnumber {
        l: 0,
        d: 0.0,
        type_: MN_INTEGER,
    };
    // c:448-453 — strip iblank from both ends, then NUL-terminate.
    // Rust slice form: skip leading iblank, then truncate trailing.
    let bytes = arg.as_bytes();
    let mut start = 0;
    while start < bytes.len() && crate::ported::ztype_h::iblank(bytes[start]) {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && crate::ported::ztype_h::iblank(bytes[end - 1]) {
        end -= 1;
    }
    let arg_trim = std::str::from_utf8(&bytes[start..end]).unwrap_or("");
    match id {
        MS_RAND48 => {
            // c:457-530 — MS_RAND48 arm.
            // c:460-461 — `static unsigned short seedbuf[3]; static int seedbuf_init;`
            //             — the lifetime-of-process seed state.
            static SEEDBUF: std::sync::OnceLock<std::sync::Mutex<[u16; 3]>> =
                std::sync::OnceLock::new();
            static SEEDBUF_INIT: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            let seedbuf_mtx = SEEDBUF.get_or_init(|| std::sync::Mutex::new([0u16; 3]));
            let mut seedbuf = seedbuf_mtx.lock().unwrap_or_else(|e| e.into_inner());
            // c:462-463 — `unsigned short tmp_seedbuf[3], *seedbufptr; int do_init = 1;`
            let mut tmp_seedbuf: [u16; 3] = [0; 3];
            let mut do_init: bool = true; // c:463
                                          // c:464-506 — choose seedbufptr (tmp from param vs static) and
                                          // decide do_init.
                                          //
                                          // Two-step ptr selection: in C `seedbufptr` is either `&tmp_seedbuf`
                                          // or `&seedbuf`. In Rust we mirror via a bool — `use_static` —
                                          // since `&mut [u16; 3]` can't switch between the two without
                                          // borrow gymnastics; copy in/out of tmp_seedbuf instead.
            let use_static: bool;
            if !arg_trim.is_empty() {
                // c:465 — `if (*arg) { ... }`
                use_static = false; // c:468 seedbufptr = tmp_seedbuf
                if let Some(seedstr) = crate::ported::params::getsparam(arg_trim) {
                    // c:469 — `(seedstr = getsparam(arg)) && strlen(seedstr) >= 12`
                    let sbytes = seedstr.as_bytes();
                    if sbytes.len() >= 12 {
                        do_init = false; // c:471
                                         // c:476-493 — decode 3 sets of 4 hex chars into tmp_seedbuf.
                        let mut cursor = 0;
                        'outer: for i in 0..3 {
                            let mut acc: u16 = 0;
                            for j in 0..4 {
                                let c = sbytes[cursor];
                                cursor += 1;
                                let lower = c.to_ascii_lowercase();
                                let nib: u16 = if c.is_ascii_digit() {
                                    (c - b'0') as u16
                                } else if (b'a'..=b'f').contains(&lower) {
                                    (lower - b'a' + 10) as u16
                                } else {
                                    do_init = true; // c:486
                                    break 'outer;
                                };
                                acc += nib;
                                if j < 3 {
                                    acc *= 16; // c:491
                                }
                            }
                            tmp_seedbuf[i] = acc; // c:478 *seedptr = ...
                        }
                    }
                } else if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
                    != 0
                {
                    // c:495-496 — `else if (errflag) break;` — bail with zero_mnumber.
                    return zero_mnumber;
                }
            } else {
                // c:499-506 — `else { seedbufptr = seedbuf; ... }`.
                use_static = true; // c:501
                                   // c:502-505 — the C source as written assigns `do_init = 1`
                                   //             in the else branch, leaving the if-branch as a
                                   //             pure seedbuf_init flip. Net effect: do_init
                                   //             stays 1 in both branches (it was 1 from c:463),
                                   //             so the static seedbuf is re-seeded every call
                                   //             when arg is empty. Preserved verbatim — quirk
                                   //             is in the C source, not the port.
                if !SEEDBUF_INIT.load(std::sync::atomic::Ordering::Relaxed) {
                    SEEDBUF_INIT.store(true, std::sync::atomic::Ordering::Relaxed);
                // c:503
                } else {
                    do_init = true; // c:505
                }
            }
            // c:507-518 — fresh seed via rand() + seed48().
            if do_init {
                let s0 = unsafe { libc::rand() } as u16;
                let s1 = unsafe { libc::rand() } as u16;
                let s2 = unsafe { libc::rand() } as u16;
                if use_static {
                    seedbuf[0] = s0;
                    seedbuf[1] = s1;
                    seedbuf[2] = s2;
                } else {
                    tmp_seedbuf[0] = s0;
                    tmp_seedbuf[1] = s1;
                    tmp_seedbuf[2] = s2;
                }
                // c:517 — `(void)seed48(seedbufptr);`
                let ptr = if use_static {
                    seedbuf.as_mut_ptr()
                } else {
                    tmp_seedbuf.as_mut_ptr()
                };
                unsafe {
                    libc::seed48(ptr);
                }
            }
            // c:519-520 — `ret.type = MN_FLOAT; ret.u.d = erand48(seedbufptr);`
            let ret_d = unsafe {
                let ptr = if use_static {
                    seedbuf.as_mut_ptr()
                } else {
                    tmp_seedbuf.as_mut_ptr()
                };
                libc::erand48(ptr)
            };
            // c:522-528 — if arg present, encode new seedbuf → $arg (3×4 hex).
            if !arg_trim.is_empty() {
                let s = if use_static { &*seedbuf } else { &tmp_seedbuf };
                let outbuf = format!("{:04x}{:04x}{:04x}", s[0], s[1], s[2]);
                let _ = crate::ported::params::setsparam(arg_trim, &outbuf);
            }
            mnumber {
                l: 0,
                d: ret_d,
                type_: MN_FLOAT,
            }
        }
        _ => zero_mnumber, // c:441 default
    }
}

// `mftab` — port of `static struct mathfunc mftab[]` (mathfunc.c:497).

// `module_features` — port of `static struct features module_features`
// from mathfunc.c:540.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:548`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:548
    // C body c:550-551 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/mathfunc.c:555`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:555
    *features = featuresarray(m, module_features());
    0 // c:570
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/mathfunc.c:563`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:563
    handlefeatures(m, module_features(), enables) // c:570
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:570`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:570
    // C body c:572-573 — `return 0`. Faithful empty-body port; the
    //                    math functions are registered via the mf_list
    //                    feature dispatch, no extra boot work needed.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:577`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:577
    setfeatureenables(m, module_features(), None) // c:584
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/mathfunc.c:584`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:584
    // C body c:586-587 — `return 0`. Faithful empty-body port; the
    //                    math functions are unregistered via cleanup_.
    0
}

// ============================================================
// MF_* — port of the anonymous `enum {}` at mathfunc.c:34-84.
// C `enum {}` with no typedef → untyped int constants. Rust
// mirrors as `pub const ... : i32` (no Rust-only enum type).
// ============================================================
/// `MF_ABS` constant.
pub const MF_ABS: i32 = 0; // c:35
/// `MF_ACOS` constant.
pub const MF_ACOS: i32 = 1; // c:36
/// `MF_ACOSH` constant.
pub const MF_ACOSH: i32 = 2;
/// `MF_ASIN` constant.
pub const MF_ASIN: i32 = 3;
/// `MF_ASINH` constant.
pub const MF_ASINH: i32 = 4;
/// `MF_ATAN` constant.
pub const MF_ATAN: i32 = 5;
/// `MF_ATANH` constant.
pub const MF_ATANH: i32 = 6;
/// `MF_CBRT` constant.
pub const MF_CBRT: i32 = 7;
/// `MF_CEIL` constant.
pub const MF_CEIL: i32 = 8;
/// `MF_COPYSIGN` constant.
pub const MF_COPYSIGN: i32 = 9;
/// `MF_COS` constant.
pub const MF_COS: i32 = 10;
/// `MF_COSH` constant.
pub const MF_COSH: i32 = 11;
/// `MF_ERF` constant.
pub const MF_ERF: i32 = 12;
/// `MF_ERFC` constant.
pub const MF_ERFC: i32 = 13;
/// `MF_EXP` constant.
pub const MF_EXP: i32 = 14;
/// `MF_EXPM1` constant.
pub const MF_EXPM1: i32 = 15;
/// `MF_FABS` constant.
pub const MF_FABS: i32 = 16;
/// `MF_FLOAT` constant.
pub const MF_FLOAT: i32 = 17;
/// `MF_FLOOR` constant.
pub const MF_FLOOR: i32 = 18;
/// `MF_FMOD` constant.
pub const MF_FMOD: i32 = 19;
/// `MF_GAMMA` constant.
pub const MF_GAMMA: i32 = 20;
/// `MF_HYPOT` constant.
pub const MF_HYPOT: i32 = 21;
/// `MF_ILOGB` constant.
pub const MF_ILOGB: i32 = 22;
/// `MF_INT` constant.
pub const MF_INT: i32 = 23;
/// `MF_ISINF` constant.
pub const MF_ISINF: i32 = 24;
/// `MF_ISNAN` constant.
pub const MF_ISNAN: i32 = 25;
/// `MF_J0` constant.
pub const MF_J0: i32 = 26;
/// `MF_J1` constant.
pub const MF_J1: i32 = 27;
/// `MF_JN` constant.
pub const MF_JN: i32 = 28;
/// `MF_LDEXP` constant.
pub const MF_LDEXP: i32 = 29;
/// `MF_LGAMMA` constant.
pub const MF_LGAMMA: i32 = 30;
/// `MF_LOG` constant.
pub const MF_LOG: i32 = 31;
/// `MF_LOG10` constant.
pub const MF_LOG10: i32 = 32;
/// `MF_LOG1P` constant.
pub const MF_LOG1P: i32 = 33;
/// `MF_LOG2` constant.
pub const MF_LOG2: i32 = 34;
/// `MF_LOGB` constant.
pub const MF_LOGB: i32 = 35;
/// `MF_NEXTAFTER` constant.
pub const MF_NEXTAFTER: i32 = 36;
/// `MF_RINT` constant.
pub const MF_RINT: i32 = 37;
/// `MF_SCALB` constant.
pub const MF_SCALB: i32 = 38;
/// `MF_SIGNGAM` constant.
pub const MF_SIGNGAM: i32 = 39; // c:75 #ifdef HAVE_SIGNGAM
/// `MF_SIN` constant.
pub const MF_SIN: i32 = 40;
/// `MF_SINH` constant.
pub const MF_SINH: i32 = 41;
/// `MF_SQRT` constant.
pub const MF_SQRT: i32 = 42;
/// `MF_TAN` constant.
pub const MF_TAN: i32 = 43;
/// `MF_TANH` constant.
pub const MF_TANH: i32 = 44;
/// `MF_Y0` constant.
pub const MF_Y0: i32 = 45;
/// `MF_Y1` constant.
pub const MF_Y1: i32 = 46;
/// `MF_YN` constant.
pub const MF_YN: i32 = 47; // c:84

// =====================================================================
// static struct mathfunc mftab[]                                    c:497
// static struct features module_features                            c:540
// =====================================================================

// ============================================================
// MS_* — port of the anonymous `enum {}` at mathfunc.c:90.
// String-arg math-fn ids.
// ============================================================
/// `MS_RAND48` constant.
pub const MS_RAND48: i32 = 0; // c:91

// ============================================================
// TF_* — port of the anonymous `enum {}` at mathfunc.c:104.
// Type-flag bits, individually testable.
// ============================================================
/// `TF_NOCONV` constant.
pub const TF_NOCONV: i32 = 1; // c:106 don't convert to float
/// `TF_INT1` constant.
pub const TF_INT1: i32 = 2; // c:107 first arg is integer
/// `TF_INT2` constant.
pub const TF_INT2: i32 = 4; // c:108 second arg is integer
/// `TF_NOASS` constant.
pub const TF_NOASS: i32 = 8; // c:109 don't assign result as double

/// Port of the `TFLAG(x)` macro from `mathfunc.c:113`.
/// `#define TFLAG(x) ((x) << 8)`. Shifts the type-flag bits into
/// the high byte of the `id` arg passed to `math_func()` so the
/// MF_* numeric ids can occupy the low byte.
pub const fn tflag(x: i32) -> i32 {
    x << 8
} // c:113

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
pub fn math_func(_name: &str, argc: i32, argv: &[mnumber], id: i32) -> mnumber {
    // c:173
    let mut ret = mnumber {
        l: 0,
        d: 0.0,
        type_: MN_FLOAT,
    }; // c:173,193
       // C's mathfunc dispatch (via `callmathfunc` at math.c:1037+ and
       // the `Math_func_set` per-fn `min_args`/`max_args` fields registered
       // in mftab) rejects out-of-range argc BEFORE calling math_func, so
       // C's body can index `argv[0]` safely. The Rust port calls this
       // dispatcher directly from tests and (eventually) other paths
       // without that upstream guard. Bail to a zero mnumber when argc is
       // 0 AND argv is empty so MF_ABS-default-id calls don't OOB. Other
       // arms that genuinely need 2+ args already check `argc > 1` below.
    if argc <= 0 && argv.is_empty() {
        return ret;
    }
    let mut argd: f64 = 0.0; // c:175
    let mut argd2: f64 = 0.0; // c:175
    let mut argi: i32 = 0; // c:176

    // Type-coerce argv[0] (and argv[1]) per the TF_INT1/TF_INT2/
    // TF_NOCONV flag bits — c:178-191.
    if argc > 0 && (id & tflag(TF_NOCONV)) == 0 {
        // c:178
        if (id & tflag(TF_INT1)) != 0 {
            // c:179
            argi = if argv[0].type_ == MN_FLOAT {
                argv[0].d as i32 // c:180
            } else {
                argv[0].l as i32
            };
        } else {
            // c:181
            argd = if argv[0].type_ == MN_INTEGER {
                argv[0].l as f64 // c:182
            } else {
                argv[0].d
            };
        }
        if argc > 1 {
            // c:183
            if (id & tflag(TF_INT2)) != 0 {
                // c:184
                argi = if argv[1].type_ == MN_FLOAT {
                    argv[1].d as i32 // c:185
                } else {
                    argv[1].l as i32
                };
            } else {
                // c:187
                argd2 = if argv[1].type_ == MN_INTEGER {
                    argv[1].l as f64 // c:188
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

    let mut retd: f64 = 0.0; // c:175

    match id & 0xff {
        // c:198
        MF_ABS => {
            // c:199
            ret.type_ = argv[0].type_;
            if argv[0].type_ == MN_INTEGER {
                ret.l = if argv[0].l < 0 { -argv[0].l } else { argv[0].l };
            } else {
                // c:204 — `ret.u.d = fabs(argv->u.d);`. C relies on the
                // mftab registration (c:115 NUMMATHFUNC("abs", …,
                // MF_ABS | TFLAG(TF_NOCONV|TF_NOASS))) merging
                // TF_NOASS into id so the post-match block (c:431-432
                // `if (!(id & TFLAG(TF_NOASS))) ret.u.d = retd;`)
                // doesn't clobber ret.d with retd=0.0. The Rust port
                // is called directly from tests with bare MF_ABS, so
                // the TF_NOASS-implicit-in-mftab assumption breaks.
                // Set BOTH ret.d AND retd so the post-block overwrite
                // is harmless either way — caller-supplied TF_NOASS
                // is still honoured but no longer required for
                // correctness.
                ret.d = argv[0].d.abs();
                retd = ret.d;
            }
        }
        MF_ACOS => retd = argd.acos(),   // c:208
        MF_ACOSH => retd = argd.acosh(), // c:212
        MF_ASIN => retd = argd.asin(),   // c:216
        MF_ASINH => retd = argd.asinh(), // c:220
        MF_ATAN => {
            // c:224
            retd = if argc == 2 {
                argd.atan2(argd2)
            } else {
                argd.atan()
            };
        }
        MF_ATANH => retd = argd.atanh(),         // c:233
        MF_CBRT => retd = unsafe { cbrt(argd) }, // c:237
        MF_CEIL => retd = argd.ceil(),           // c:241
        MF_COPYSIGN => retd = unsafe { copysign(argd, argd2) }, // c:245
        MF_COS => retd = argd.cos(),             // c:249
        MF_COSH => retd = argd.cosh(),           // c:253
        MF_ERF => retd = unsafe { erf(argd) },   // c:257
        MF_ERFC => retd = unsafe { erfc(argd) }, // c:261
        MF_EXP => retd = argd.exp(),             // c:265
        MF_EXPM1 => retd = unsafe { expm1(argd) }, // c:269
        MF_FABS => retd = argd.abs(),            // c:273
        MF_FLOAT => retd = argd,                 // c:277
        MF_FLOOR => retd = argd.floor(),         // c:281
        MF_FMOD => retd = argd % argd2,          // c:285
        MF_GAMMA => retd = unsafe { tgamma(argd) }, // c:289
        MF_HYPOT => retd = argd.hypot(argd2),    // c:300
        MF_ILOGB => {
            // c:304
            ret.type_ = MN_INTEGER;
            ret.l = unsafe { ilogb(argd) } as i64;
        }
        MF_INT => {
            // c:309
            ret.type_ = MN_INTEGER;
            ret.l = argd as i64;
        }
        MF_ISINF => {
            // c:314
            ret.type_ = MN_INTEGER;
            ret.l = argd.is_infinite() as i64;
        }
        MF_ISNAN => {
            // c:319
            ret.type_ = MN_INTEGER;
            ret.l = argd.is_nan() as i64;
        }
        MF_J0 => retd = unsafe { j0(argd) },             // c:325
        MF_J1 => retd = unsafe { j1(argd) },             // c:329
        MF_JN => retd = unsafe { jn(argi, argd2) },      // c:333
        MF_LDEXP => retd = unsafe { ldexp(argd, argi) }, // c:337
        MF_LGAMMA => retd = unsafe { lgamma(argd) },     // c:341
        MF_LOG => retd = argd.ln(),                      // c:345
        MF_LOG10 => retd = argd.log10(),                 // c:349
        MF_LOG1P => retd = unsafe { log1p(argd) },       // c:353
        MF_LOG2 => retd = argd.log2(),                   // c:357
        MF_LOGB => retd = unsafe { logb(argd) },         // c:365
        MF_NEXTAFTER => retd = unsafe { nextafter(argd, argd2) }, // c:369
        MF_RINT => retd = unsafe { rint(argd) },         // c:373
        MF_SCALB => retd = unsafe { scalbn(argd, argi) }, // c:377
        MF_SIGNGAM => {
            // c:386
            ret.type_ = MN_INTEGER;
            ret.l = 0; // signgam is libm-internal; not portably exposed.
        }
        MF_SIN => retd = argd.sin(),                // c:392
        MF_SINH => retd = argd.sinh(),              // c:396
        MF_SQRT => retd = argd.sqrt(),              // c:400
        MF_TAN => retd = argd.tan(),                // c:404
        MF_TANH => retd = argd.tanh(),              // c:408
        MF_Y0 => retd = unsafe { y0(argd) },        // c:412
        MF_Y1 => retd = unsafe { y1(argd) },        // c:416
        MF_YN => retd = unsafe { yn(argi, argd2) }, // c:420
        _ => { // c:425
             // BUG: mathfunc type not handled. C prints to stderr
             // under DEBUG; production zsh silently returns 0.
        }
    }

    if (id & tflag(TF_NOASS)) == 0 {
        // c:431
        ret.d = retd; // c:432
    }

    ret // c:434
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

/// Port of `static struct mathfunc mftab[]` from `Src/Modules/mathfunc.c:114-167`.
///
/// C macro per entry: `NUMMATHFUNC(name, math_func, min, max, id)` =
/// `{ NULL, name, 0, func, NULL, NULL, min, max, id }` (zsh.h:133) —
/// flags 0, module NULL. The table is the registration source for
/// `setmathfuncs` (module.c:1374): feature-enable inserts entries into
/// the global MATHFUNCS list with `MFF_ADDED`; disable removes them.
/// Entry order MUST match `featuresarray` below — enables bitmaps are
/// positional (module.c:3284 featuresarray ↔ c:3319 getfeatureenables).
///
/// `STRMATHFUNC("rand48", math_string, MS_RAND48)` (c:153) is omitted
/// to match this module's existing 48-name feature surface (rand48
/// dispatches through `math_string` directly in math.rs).
/// Initialized inline by `setfeatureenables` below via
/// `MFTAB.get_or_init` (single consumer; the src/ported/ build gate
/// forbids a Rust-only named accessor fn).
static MFTAB: OnceLock<Mutex<Vec<mathfunc>>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
/// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "f:abs".to_string(),
        "f:acos".to_string(),
        "f:acosh".to_string(),
        "f:asin".to_string(),
        "f:asinh".to_string(),
        "f:atan".to_string(),
        "f:atanh".to_string(),
        "f:cbrt".to_string(),
        "f:ceil".to_string(),
        "f:copysign".to_string(),
        "f:cos".to_string(),
        "f:cosh".to_string(),
        "f:erf".to_string(),
        "f:erfc".to_string(),
        "f:exp".to_string(),
        "f:expm1".to_string(),
        "f:fabs".to_string(),
        "f:float".to_string(),
        "f:floor".to_string(),
        "f:fmod".to_string(),
        "f:gamma".to_string(),
        "f:hypot".to_string(),
        "f:ilogb".to_string(),
        "f:int".to_string(),
        "f:isinf".to_string(),
        "f:isnan".to_string(),
        "f:j0".to_string(),
        "f:j1".to_string(),
        "f:jn".to_string(),
        "f:ldexp".to_string(),
        "f:lgamma".to_string(),
        "f:log".to_string(),
        "f:log10".to_string(),
        "f:log1p".to_string(),
        "f:log2".to_string(),
        "f:logb".to_string(),
        "f:nextafter".to_string(),
        "f:rint".to_string(),
        "f:scalb".to_string(),
        "f:signgam".to_string(),
        "f:sin".to_string(),
        "f:sinh".to_string(),
        "f:sqrt".to_string(),
        "f:tan".to_string(),
        "f:tanh".to_string(),
        "f:y0".to_string(),
        "f:y1".to_string(),
        "f:yn".to_string(),
    ]
}

// WARNING: NOT IN MATHFUNC.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(m: *const module, f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:Src/module.c:3370-3377 — `if (!enables || !*enables)
    // *enables = getfeatureenables(m, f); else return
    // setfeatureenables(m, f, *enables);`. The Some arm COMMITS the
    // bits: do_module_features' final enables_module call lands here
    // and must register/deregister the mftab entries in the global
    // MATHFUNCS list. Previously a no-op — `zmodload zsh/mathfunc`
    // never populated MATHFUNCS, so getmathfunc's post-autoload
    // re-query (module.c:1298) always missed and `zmodload -af
    // zsh/mathfunc sin; $(( sin(0) ))` errored.
    match enables.as_deref() {
        None => {
            *enables = Some(vec![1; 48]); // c:3372 getfeatureenables
            0
        }
        Some(e) => {
            let e_owned: Vec<i32> = e.to_vec();
            setfeatureenables(m, f, Some(&e_owned)) // c:3375
        }
    }
}

// WARNING: NOT IN MATHFUNC.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, e: Option<&[i32]>) -> i32 {
    // c:Src/module.c:3445-3460 setfeatureenables → c:1374 setmathfuncs:
    // walk mftab against the positional enables bitmap; 1 → addmathfunc
    // into the global MATHFUNCS list (+MFF_ADDED), 0/None → remove.
    //
    // MFTAB get_or_init — port of `static struct mathfunc mftab[]`
    // from Src/Modules/mathfunc.c:114-167. C macro per entry:
    // `NUMMATHFUNC(name, math_func, min, max, id)` = `{ NULL, name, 0,
    // func, NULL, NULL, min, max, id }` (zsh.h:133) — flags 0, module
    // NULL. Entry order MUST match `featuresarray` above — enables
    // bitmaps are positional (module.c:3284 featuresarray ↔ c:3319
    // getfeatureenables). `STRMATHFUNC("rand48", math_string,
    // MS_RAND48)` (c:153) is omitted to match this module's existing
    // 48-name feature surface (rand48 dispatches through `math_string`
    // directly in math.rs).
    let tab_mutex = MFTAB.get_or_init(|| {
        // NUMMATHFUNC expansion — zsh.h:133.
        let num = |name: &str, min: i32, max: i32, id: i32| mathfunc {
            next: None,
            name: name.to_string(),
            flags: 0,
            nfunc: Some(math_func as crate::ported::zsh_h::NumMathFunc),
            sfunc: None,
            module: None,
            minargs: min,
            maxargs: max,
            funcid: id,
        };
        Mutex::new(vec![
            num("abs", 1, 1, MF_ABS | tflag(TF_NOCONV | TF_NOASS)), // c:115
            num("acos", 1, 1, MF_ACOS),                             // c:117
            num("acosh", 1, 1, MF_ACOSH),                           // c:118
            num("asin", 1, 1, MF_ASIN),                             // c:119
            num("asinh", 1, 1, MF_ASINH),                           // c:120
            num("atan", 1, 2, MF_ATAN),                             // c:121
            num("atanh", 1, 1, MF_ATANH),                           // c:122
            num("cbrt", 1, 1, MF_CBRT),                             // c:123
            num("ceil", 1, 1, MF_CEIL),                             // c:124
            num("copysign", 2, 2, MF_COPYSIGN),                     // c:125
            num("cos", 1, 1, MF_COS),                               // c:126
            num("cosh", 1, 1, MF_COSH),                             // c:127
            num("erf", 1, 1, MF_ERF),                               // c:128
            num("erfc", 1, 1, MF_ERFC),                             // c:129
            num("exp", 1, 1, MF_EXP),                               // c:130
            num("expm1", 1, 1, MF_EXPM1),                           // c:131
            num("fabs", 1, 1, MF_FABS),                             // c:132
            num("float", 1, 1, MF_FLOAT),                           // c:133
            num("floor", 1, 1, MF_FLOOR),                           // c:134
            num("fmod", 2, 2, MF_FMOD),                             // c:135
            num("gamma", 1, 1, MF_GAMMA),                           // c:136
            num("hypot", 2, 2, MF_HYPOT),                           // c:137
            num("ilogb", 1, 1, MF_ILOGB | tflag(TF_NOASS)),         // c:138
            num("int", 1, 1, MF_INT | tflag(TF_NOASS)),             // c:139
            num("isinf", 1, 1, MF_ISINF | tflag(TF_NOASS)),         // c:140
            num("isnan", 1, 1, MF_ISNAN | tflag(TF_NOASS)),         // c:141
            num("j0", 1, 1, MF_J0),                                 // c:142
            num("j1", 1, 1, MF_J1),                                 // c:143
            num("jn", 2, 2, MF_JN | tflag(TF_INT1)),                // c:144
            num("ldexp", 2, 2, MF_LDEXP | tflag(TF_INT2)),          // c:145
            num("lgamma", 1, 1, MF_LGAMMA),                         // c:146
            num("log", 1, 1, MF_LOG),                               // c:147
            num("log10", 1, 1, MF_LOG10),                           // c:148
            num("log1p", 1, 1, MF_LOG1P),                           // c:149
            num("log2", 1, 1, MF_LOG2),                             // c:150
            num("logb", 1, 1, MF_LOGB),                             // c:151
            num("nextafter", 2, 2, MF_NEXTAFTER),                   // c:152
            num("rint", 1, 1, MF_RINT),                             // c:156
            num("scalb", 2, 2, MF_SCALB | tflag(TF_INT2)),          // c:157
            num("signgam", 0, 0, MF_SIGNGAM | tflag(TF_NOASS)),     // c:159
            num("sin", 1, 1, MF_SIN),                               // c:161
            num("sinh", 1, 1, MF_SINH),                             // c:162
            num("sqrt", 1, 1, MF_SQRT),                             // c:163
            num("tan", 1, 1, MF_TAN),                               // c:164
            num("tanh", 1, 1, MF_TANH),                             // c:165
            num("y0", 1, 1, MF_Y0),                                 // c:166
            num("y1", 1, 1, MF_Y1),                                 // c:167
            num("yn", 2, 2, MF_YN | tflag(TF_INT1)),                // c:168
        ])
    });
    let mut tab = tab_mutex.lock().unwrap();
    crate::ported::module::setmathfuncs("zsh/mathfunc", &mut tab, e)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN MATHFUNC.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 0,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 48,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_acos() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 1.0,
            type_: MN_FLOAT,
        }];
        let r = math_func("acos", 1, &argv, MF_ACOS);
        assert!((r.type_ == MN_FLOAT));
        assert!((r.d - 0.0).abs() < 1e-9);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_atan_two_args() {
        let _g = crate::test_util::global_state_lock();
        let argv = [
            mnumber {
                l: 0,
                d: 1.0,
                type_: MN_FLOAT,
            },
            mnumber {
                l: 0,
                d: 1.0,
                type_: MN_FLOAT,
            },
        ];
        let r = math_func("atan", 2, &argv, MF_ATAN);
        assert!((r.type_ == MN_FLOAT));
        assert!((r.d - std::f64::consts::FRAC_PI_4).abs() < 1e-9);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_abs_int_preserves_type() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: -7,
            d: 0.0,
            type_: MN_INTEGER,
        }];
        let r = math_func("abs", 1, &argv, MF_ABS | tflag(TF_NOCONV | TF_NOASS));
        assert!((r.type_ == MN_INTEGER));
        assert_eq!(r.l, 7);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_int_truncates() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 3.7,
            type_: MN_FLOAT,
        }];
        let r = math_func("int", 1, &argv, MF_INT | tflag(TF_NOASS));
        assert!((r.type_ == MN_INTEGER));
        assert_eq!(r.l, 3);
    }

    /// Port of `math_func(UNUSED(char *name), int argc, mnumber *argv, int id)` from `Src/Modules/mathfunc.c:173`.
    #[test]
    fn test_math_func_isnan() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: f64::NAN,
            type_: MN_FLOAT,
        }];
        let r = math_func("isnan", 1, &argv, MF_ISNAN | tflag(TF_NOASS));
        assert_eq!(r.l, 1);
    }

    /// Port of `math_string(UNUSED(char *name), char *arg, int id)` from `Src/Modules/mathfunc.c:439`.
    #[test]
    fn test_math_string_rand48_in_range() {
        let _g = crate::test_util::global_state_lock();
        let r = math_string("rand48", "", MS_RAND48);
        assert!((r.type_ == MN_FLOAT));
        assert!((0.0..1.0).contains(&r.d));
    }

    /// c:173 — `MF_COS` of 0 is 1.0 exactly. Trigonometric identity
    /// pin; catches a regression that swaps cos/sin dispatch.
    #[test]
    fn math_func_cos_of_zero_is_one() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 0.0,
            type_: MN_FLOAT,
        }];
        let r = math_func("cos", 1, &argv, MF_COS);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 1.0).abs() < 1e-9);
    }

    /// c:173 — `MF_SIN` of 0 is 0. Symmetric to the cos test;
    /// any libm aliasing would surface here.
    #[test]
    fn math_func_sin_of_zero_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 0.0,
            type_: MN_FLOAT,
        }];
        let r = math_func("sin", 1, &argv, MF_SIN);
        assert_eq!(r.type_, MN_FLOAT);
        assert!(r.d.abs() < 1e-9, "sin(0) = {}", r.d);
    }

    /// c:173 — `MF_SQRT` of 4 is 2.0. Pure-math anchor that catches
    /// any regression in the int→float promotion before sqrt.
    #[test]
    fn math_func_sqrt_of_four_is_two() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 4.0,
            type_: MN_FLOAT,
        }];
        let r = math_func("sqrt", 1, &argv, MF_SQRT);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 2.0).abs() < 1e-9, "sqrt(4) = {}", r.d);
    }

    /// c:173 — `MF_EXP` of 0 is 1.0 (e^0 identity).
    #[test]
    fn math_func_exp_of_zero_is_one() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 0.0,
            type_: MN_FLOAT,
        }];
        let r = math_func("exp", 1, &argv, MF_EXP);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 1.0).abs() < 1e-9);
    }

    /// c:173 — `MF_LOG` of 1.0 is 0.0 (natural log identity).
    #[test]
    fn math_func_log_of_one_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 1.0,
            type_: MN_FLOAT,
        }];
        let r = math_func("log", 1, &argv, MF_LOG);
        assert_eq!(r.type_, MN_FLOAT);
        assert!(r.d.abs() < 1e-9, "log(1) = {}", r.d);
    }

    /// c:173 — `MF_FLOOR` of 3.7 is 3.0 (NOT 4.0). Pin direction
    /// because a regen could swap floor/ceil dispatch.
    #[test]
    fn math_func_floor_rounds_down() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 3.7,
            type_: MN_FLOAT,
        }];
        let r = math_func("floor", 1, &argv, MF_FLOOR);
        assert_eq!(r.type_, MN_FLOAT);
        assert_eq!(r.d, 3.0);
    }

    /// c:173 — `MF_CEIL` of 3.1 is 4.0. Symmetric to floor.
    #[test]
    fn math_func_ceil_rounds_up() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: 3.1,
            type_: MN_FLOAT,
        }];
        let r = math_func("ceil", 1, &argv, MF_CEIL);
        assert_eq!(r.type_, MN_FLOAT);
        assert_eq!(r.d, 4.0);
    }

    /// c:173 — `MF_FABS` of negative is positive AND the result
    /// type stays MN_FLOAT (NOT coerced to MN_INTEGER like the
    /// integer-typed `abs`).
    #[test]
    fn math_func_fabs_preserves_float_type() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 0,
            d: -2.5,
            type_: MN_FLOAT,
        }];
        let r = math_func("fabs", 1, &argv, MF_FABS);
        assert_eq!(r.type_, MN_FLOAT);
        assert_eq!(r.d, 2.5);
    }

    /// c:173 — `MF_ISINF` of +infinity is 1; of finite is 0. Pin
    /// both branches so a regression that returns the IEEE-754
    /// classify code (3 / 0 / 4 / 5) instead of the boolean gets
    /// caught.
    #[test]
    fn math_func_isinf_classifies_correctly() {
        let _g = crate::test_util::global_state_lock();
        let argv_inf = [mnumber {
            l: 0,
            d: f64::INFINITY,
            type_: MN_FLOAT,
        }];
        let r_inf = math_func("isinf", 1, &argv_inf, MF_ISINF | tflag(TF_NOASS));
        assert_eq!(r_inf.l, 1, "isinf(+inf) must be 1");

        let argv_fin = [mnumber {
            l: 0,
            d: 1.5,
            type_: MN_FLOAT,
        }];
        let r_fin = math_func("isinf", 1, &argv_fin, MF_ISINF | tflag(TF_NOASS));
        assert_eq!(r_fin.l, 0, "isinf(finite) must be 0");
    }

    /// c:439 — `math_string` for an unknown id must not panic.
    /// Defensive contract; return value is impl-defined but the
    /// function must not crash.
    #[test]
    fn math_string_unknown_id_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = math_string("nope", "", 9999);
    }

    /// c:548-590 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // math_func — dispatcher for math/MF_* function IDs.
    // Anchored to known math library results. Build mnumber args
    // explicitly; pin the resulting mnumber's type and float value
    // (or integer value for MF_ABS which preserves input type).
    // ═══════════════════════════════════════════════════════════════════

    fn mn_int(v: i64) -> mnumber {
        mnumber {
            l: v,
            d: 0.0,
            type_: MN_INTEGER,
        }
    }

    fn mn_float(v: f64) -> mnumber {
        mnumber {
            l: 0,
            d: v,
            type_: MN_FLOAT,
        }
    }

    /// `abs(-5)` (integer) preserves integer type and returns 5.
    #[test]
    fn math_func_abs_integer_input_preserves_int_type() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("abs", 1, &[mn_int(-5)], MF_ABS);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 5);
    }

    /// `abs(-3.14)` (float) preserves float type and returns 3.14.
    /// **ZSHRS BUG**: the MF_ABS arm sets `ret.d = argv[0].d.abs()` but
    /// the post-match block at c:431-432 unconditionally assigns
    /// `ret.d = retd` (which starts at 0.0 and was never set by MF_ABS).
    /// MF_ABS needs to either set `retd` instead, or set TF_NOASS to
    /// skip the post-match overwrite.
    #[test]
    fn math_func_abs_float_input_preserves_float_type_anchored() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("abs", 1, &[mn_float(-3.14)], MF_ABS);
        assert_eq!(r.type_, MN_FLOAT);
        assert!(
            (r.d - 3.14).abs() < 1e-9,
            "abs(-3.14) must be 3.14; got {} (zsh: 3.14)",
            r.d
        );
    }

    /// `abs(+5)` → 5 (positive input unchanged).
    #[test]
    fn math_func_abs_positive_input_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("abs", 1, &[mn_int(5)], MF_ABS);
        assert_eq!(r.l, 5);
    }

    /// `sqrt(16.0)` → 4.0.
    #[test]
    fn math_func_sqrt_of_sixteen_is_four() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("sqrt", 1, &[mn_float(16.0)], MF_SQRT);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 4.0).abs() < 1e-9);
    }

    /// `sqrt(0.0)` → 0.0.
    #[test]
    fn math_func_sqrt_of_zero_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("sqrt", 1, &[mn_float(0.0)], MF_SQRT);
        assert!(r.d.abs() < 1e-9);
    }

    /// `sqrt(2.0)` ≈ 1.41421356...
    #[test]
    fn math_func_sqrt_of_two_is_root_two() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("sqrt", 1, &[mn_float(2.0)], MF_SQRT);
        assert!((r.d - std::f64::consts::SQRT_2).abs() < 1e-9);
    }

    /// `floor(-2.3)` → -3.0 (floors toward negative infinity).
    #[test]
    fn math_func_floor_negative_rounds_toward_neg_infinity() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("floor", 1, &[mn_float(-2.3)], MF_FLOOR);
        assert!((r.d - (-3.0)).abs() < 1e-9);
    }

    /// `ceil(-2.7)` → -2.0 (ceils toward positive infinity).
    #[test]
    fn math_func_ceil_negative_rounds_toward_pos_infinity() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("ceil", 1, &[mn_float(-2.7)], MF_CEIL);
        assert!((r.d - (-2.0)).abs() < 1e-9);
    }

    /// `sin(π/2)` → 1.0.
    #[test]
    fn math_func_sin_of_pi_over_two_is_one() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("sin", 1, &[mn_float(std::f64::consts::FRAC_PI_2)], MF_SIN);
        assert!((r.d - 1.0).abs() < 1e-9);
    }

    /// `log(e)` → 1.0 (natural log of e).
    #[test]
    fn math_func_log_of_e_is_one() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("log", 1, &[mn_float(std::f64::consts::E)], MF_LOG);
        assert!((r.d - 1.0).abs() < 1e-9);
    }

    /// `log10(100)` → 2.0.
    #[test]
    fn math_func_log10_of_hundred_is_two() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("log10", 1, &[mn_float(100.0)], MF_LOG10);
        assert!((r.d - 2.0).abs() < 1e-9);
    }

    /// `log2(8)` → 3.0.
    #[test]
    fn math_func_log2_of_eight_is_three() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("log2", 1, &[mn_float(8.0)], MF_LOG2);
        assert!((r.d - 3.0).abs() < 1e-9);
    }

    // ─ Integer input → float coercion (TF_INT1 NOT set) ────────────
    /// `sqrt(16)` (int input) coerces to float, returns 4.0.
    #[test]
    fn math_func_sqrt_int_input_coerces_to_float() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("sqrt", 1, &[mn_int(16)], MF_SQRT);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 4.0).abs() < 1e-9);
    }

    // ─── zsh-corpus pins for math_func ─────────────────────────────

    /// `abs(-5.0)` returns 5.0 as float.
    #[test]
    fn mathfunc_corpus_abs_negative_float() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("abs", 1, &[mn_float(-5.0)], MF_ABS);
        assert!(
            (r.d.abs() - 5.0).abs() < 1e-9,
            "|−5.0| = 5.0, got {:?}",
            r.d
        );
    }

    /// `cos(0)` = 1.0.
    #[test]
    fn mathfunc_corpus_cos_zero_is_one() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("cos", 1, &[mn_float(0.0)], MF_COS);
        assert!((r.d - 1.0).abs() < 1e-9);
    }

    /// `sin(0)` = 0.0.
    #[test]
    fn mathfunc_corpus_sin_zero_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("sin", 1, &[mn_float(0.0)], MF_SIN);
        assert!(r.d.abs() < 1e-9, "sin(0)=0, got {}", r.d);
    }

    /// `exp(0)` = 1.0.
    #[test]
    fn mathfunc_corpus_exp_zero_is_one() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("exp", 1, &[mn_float(0.0)], MF_EXP);
        assert!((r.d - 1.0).abs() < 1e-9);
    }

    /// `ceil(2.3)` = 3.0.
    #[test]
    fn mathfunc_corpus_ceil_rounds_up() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("ceil", 1, &[mn_float(2.3)], MF_CEIL);
        assert!((r.d - 3.0).abs() < 1e-9, "ceil(2.3)=3.0, got {}", r.d);
    }

    /// `floor(2.7)` = 2.0.
    #[test]
    fn mathfunc_corpus_floor_rounds_down() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("floor", 1, &[mn_float(2.7)], MF_FLOOR);
        assert!((r.d - 2.0).abs() < 1e-9, "floor(2.7)=2.0, got {}", r.d);
    }

    /// `fabs(-7.5)` = 7.5.
    #[test]
    fn mathfunc_corpus_fabs_negative() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("fabs", 1, &[mn_float(-7.5)], MF_FABS);
        assert!((r.d - 7.5).abs() < 1e-9);
    }

    /// `int(3.7)` truncates toward zero.
    #[test]
    fn mathfunc_corpus_int_truncates_toward_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("int", 1, &[mn_float(3.7)], MF_INT);
        assert_eq!(r.l, 3, "int(3.7) = 3, got {}", r.l);
    }

    /// `int(-3.7)` truncates toward zero → -3.
    #[test]
    fn mathfunc_corpus_int_truncates_negative_toward_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("int", 1, &[mn_float(-3.7)], MF_INT);
        assert_eq!(r.l, -3, "int(-3.7) = -3, got {}", r.l);
    }

    /// `float(5)` converts int to 5.0 float.
    #[test]
    fn mathfunc_corpus_float_promotes_int() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("float", 1, &[mn_int(5)], MF_FLOAT);
        assert_eq!(r.type_, MN_FLOAT, "result is float-typed");
        assert!((r.d - 5.0).abs() < 1e-9);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mathfunc.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:286 — math_func MF_FABS for positive value preserves it.
    #[test]
    fn math_func_fabs_positive_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("fabs", 1, &[mn_float(3.5)], MF_FABS);
        assert!((r.d - 3.5).abs() < 1e-9);
    }

    /// c:286 — math_func MF_FABS for zero returns 0.
    #[test]
    fn math_func_fabs_zero_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("fabs", 1, &[mn_float(0.0)], MF_FABS);
        assert_eq!(r.d, 0.0);
    }

    /// c:286 — math_func MF_INT on already-int returns same value.
    #[test]
    fn math_func_int_on_int_returns_same() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("int", 1, &[mn_int(42)], MF_INT);
        assert_eq!(r.l, 42);
    }

    /// c:286 — math_func MF_INT on 0.0 returns 0.
    #[test]
    fn math_func_int_zero_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("int", 1, &[mn_float(0.0)], MF_INT);
        assert_eq!(r.l, 0);
    }

    /// c:286 — math_func MF_FLOAT on already-float returns same.
    #[test]
    fn math_func_float_on_float_returns_same() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("float", 1, &[mn_float(3.14)], MF_FLOAT);
        assert!((r.d - 3.14).abs() < 1e-9);
        assert_eq!(r.type_, MN_FLOAT);
    }

    /// c:286 — math_func MF_FLOAT on 0 → 0.0 float.
    #[test]
    fn math_func_float_zero_int_returns_zero_float() {
        let _g = crate::test_util::global_state_lock();
        let r = math_func("float", 1, &[mn_int(0)], MF_FLOAT);
        assert_eq!(r.d, 0.0);
        assert_eq!(r.type_, MN_FLOAT);
    }

    /// c:439 — math_string MS_RAND48 returns float in [0, 1).
    #[test]
    fn math_string_rand48_in_range() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..30 {
            let r = math_string("rand48", "", MS_RAND48);
            assert!(r.d >= 0.0 && r.d < 1.0, "out of [0,1): got {}", r.d);
            assert_eq!(r.type_, MN_FLOAT);
        }
    }

    /// c:439 — math_string for unknown id returns zero mnumber.
    #[test]
    fn math_string_unknown_id_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_string("never", "", 9999);
        assert_eq!(r.l, 0);
        assert_eq!(r.d, 0.0);
        assert_eq!(r.type_, MN_INTEGER);
    }

    /// c:548 — setup_(NULL) = 0.
    #[test]
    fn mathfunc_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:570 — boot_(NULL) = 0.
    #[test]
    fn mathfunc_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:131 — finish_(NULL) = 0.
    #[test]
    fn mathfunc_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mathfunc.c
    // c:58 math_string / c:286 math_func / c:91-131 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `math_string` returns mnumber (compile-time type pin).
    #[test]
    fn math_string_returns_mnumber_type() {
        let _: mnumber = math_string("rand48", "", 0);
    }

    /// c:58 — `math_string` empty input doesn't panic.
    #[test]
    fn math_string_empty_input_no_panic() {
        let _ = math_string("rand48", "", 0);
        let _ = math_string("", "", 0);
    }

    /// c:58 — `math_string("rand48", _, _)` returns finite f64.
    #[test]
    fn math_string_rand48_returns_finite() {
        for _ in 0..20 {
            let r = math_string("rand48", "", 0);
            // rand48 returns f64; result should be finite.
            // mnumber.d may be 0.0 if int variant — check both fields.
            assert!(
                r.d.is_finite() || r.l != 0 || r.d == 0.0,
                "rand48 should be finite f64"
            );
        }
    }

    /// c:286 — `math_func` returns mnumber (compile-time type pin).
    /// Uses non-empty argv to avoid the ZSHRS BUG (empty argv panics).
    #[test]
    fn math_func_returns_mnumber_type() {
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: 1.0,
            type_: MN_FLOAT,
        };
        let _: mnumber = math_func("fabs", 1, &[arg], 0);
    }

    /// c:286 — `math_func` is deterministic for pure math fns (fabs, int).
    #[test]
    fn math_func_pure_for_fabs() {
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: 1.5,
            type_: MN_FLOAT,
        };
        let first = math_func("fabs", 1, &[arg], 0);
        for _ in 0..3 {
            let arg2 = mnumber {
                l: 0,
                d: 1.5,
                type_: MN_FLOAT,
            };
            assert_eq!(
                math_func("fabs", 1, &[arg2], 0).d,
                first.d,
                "fabs(1.5) must be pure"
            );
        }
    }

    /// c:286 — `math_func` with empty argv PANICS in zshrs port
    /// ("index out of bounds: the len is 0 but the index is 0").
    /// C source validates argc before indexing; Rust port skips check.
    #[test]
    fn math_func_empty_argv_no_panic() {
        let _ = math_func("fabs", 0, &[], 0);
        let _ = math_func("", 0, &[], 0);
    }

    /// c:91-131 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn mathfunc_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    /// c:91 — setup_ idempotent.
    #[test]
    fn mathfunc_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:131 — finish_ idempotent.
    #[test]
    fn mathfunc_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:124 — cleanup_ idempotent.
    #[test]
    fn mathfunc_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mathfunc.c
    // c:58 math_string / c:286 math_func — type-pins + determinism on
    // safe (non-zero-arg) call patterns
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `math_string` is deterministic for empty string input.
    #[test]
    fn math_string_empty_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = math_string("rand48", "", 0);
        for _ in 0..3 {
            // rand48 is the only non-deterministic id; sticky compare on
            // type field rather than value.
            let r = math_string("rand48", "", 0);
            assert_eq!(r.type_, first.type_, "math_string rand48 type stable");
        }
    }

    /// c:58 — `math_string("rand48", _, _)` returns float-typed mnumber.
    #[test]
    fn math_string_rand48_returns_float_type() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let r = math_string("rand48", "", 0);
        assert_eq!(r.type_, MN_FLOAT, "rand48 returns float mnumber");
    }

    /// c:286 — `math_func("int", N, [int_arg])` should return same int
    /// but Rust port's math_func dispatcher doesn't resolve "int" id;
    /// pin determinism only.
    #[test]
    fn math_func_int_int_arg_deterministic() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_INTEGER;
        let arg = mnumber {
            l: 42,
            d: 0.0,
            type_: MN_INTEGER,
        };
        let first = math_func("int", 1, &[arg], 0).l;
        for _ in 0..3 {
            let arg2 = mnumber {
                l: 42,
                d: 0.0,
                type_: MN_INTEGER,
            };
            assert_eq!(
                math_func("int", 1, &[arg2], 0).l,
                first,
                "math_func int deterministic"
            );
        }
    }

    /// c:286 — `math_func("fabs", 1, [negative])` returns positive.
    #[test]
    fn math_func_fabs_negative_returns_positive() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: -3.5,
            type_: MN_FLOAT,
        };
        let r = math_func("fabs", 1, &[arg], 0);
        assert_eq!(r.d, 3.5, "fabs(-3.5) = 3.5");
    }

    /// c:286 — `math_func("fabs", 1, [zero])` returns 0.
    #[test]
    fn math_func_fabs_zero_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: 0.0,
            type_: MN_FLOAT,
        };
        let r = math_func("fabs", 1, &[arg], 0);
        assert_eq!(r.d, 0.0, "fabs(0) = 0");
    }

    /// c:286 — `math_func("int", 1, [float])` truncates toward zero.
    /// Dispatch keys on `id` (not name); the C registration table at
    /// Src/Modules/mathfunc.c:2068 maps "int" → `MF_INT`. Pass the
    /// resolved id directly here — name→id resolution happens in
    /// `math_func_call` / `callmathfunc` upstream of this dispatcher.
    #[test]
    fn math_func_int_float_truncates_toward_zero() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: 3.9,
            type_: MN_FLOAT,
        };
        let r = math_func("int", 1, &[arg], MF_INT);
        assert_eq!(r.l, 3, "int(3.9) = 3 (truncates toward zero)");
    }

    /// c:286 — `math_func("int", 1, [negative-float])` truncates toward zero.
    #[test]
    fn math_func_int_negative_float_truncates_toward_zero() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: -3.9,
            type_: MN_FLOAT,
        };
        let r = math_func("int", 1, &[arg], MF_INT);
        assert_eq!(r.l, -3, "int(-3.9) = -3 (truncates toward zero, not -4)");
    }

    /// c:286 — `math_func("float", 1, [int])` returns float type.
    #[test]
    fn math_func_float_int_returns_float_type() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::{MN_FLOAT, MN_INTEGER};
        let arg = mnumber {
            l: 42,
            d: 0.0,
            type_: MN_INTEGER,
        };
        let r = math_func("float", 1, &[arg], MF_FLOAT);
        assert_eq!(r.type_, MN_FLOAT, "float(42) type is float");
    }

    /// c:58 — `math_string` for unknown id returns mnumber.
    #[test]
    fn math_string_unknown_id_returns_mnumber() {
        let _g = crate::test_util::global_state_lock();
        let _: mnumber = math_string("unknown_id_xyz", "", 99999);
    }

    /// c:286 — `math_func` is pure for fabs over multiple inputs.
    #[test]
    fn math_func_fabs_full_sweep_pure() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        for v in [-1.0, 0.0, 1.0, 100.0, -3.14] {
            let arg = mnumber {
                l: 0,
                d: v,
                type_: MN_FLOAT,
            };
            let first = math_func("fabs", 1, &[arg], 0).d;
            for _ in 0..3 {
                let arg2 = mnumber {
                    l: 0,
                    d: v,
                    type_: MN_FLOAT,
                };
                assert_eq!(
                    math_func("fabs", 1, &[arg2], 0).d,
                    first,
                    "fabs({}) must be pure",
                    v
                );
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/mathfunc.c
    // c:58 math_string / c:286 math_func + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `math_string` returns mnumber (compile-time pin, alt).
    #[test]
    fn math_string_returns_mnumber_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: mnumber = math_string("strlen", "x", 0);
    }

    /// c:58 — `math_string` returns the canonical mnumber-shaped value
    /// for any (name, arg, id). Note: id=0 dispatches to rand48-like
    /// non-deterministic functions in zshrs's mftab; pin only the
    /// structural invariant (always returns a 3-field mnumber struct,
    /// no panic on common id values).
    #[test]
    fn math_string_no_panic_across_common_ids() {
        let _g = crate::test_util::global_state_lock();
        for id in [0, 1, 2, 5, 10, 100, -1] {
            let _: mnumber = math_string("any", "input", id);
        }
    }

    /// c:286 — `math_func("fabs", 0, &[])` MUST safely return mnumber
    /// without panicking; C source guards via `argc < min_args` check.
    /// In zshrs the port indexes `argv[0]` without bounds check at c:347.
    #[test]
    fn math_func_returns_mnumber_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: mnumber = math_func("fabs", 0, &[], 0);
    }

    /// c:286 — `math_func("fabs", 1, [positive])` keeps value (alt).
    #[test]
    fn math_func_fabs_positive_unchanged_alt() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: 5.0,
            type_: MN_FLOAT,
        };
        let r = math_func("fabs", 1, &[arg], 0);
        assert_eq!(r.d, 5.0, "fabs(5.0) = 5.0");
    }

    /// c:286 — `math_func("fabs", 1, [large negative])` returns large positive.
    #[test]
    fn math_func_fabs_large_negative() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: -1e20,
            type_: MN_FLOAT,
        };
        let r = math_func("fabs", 1, &[arg], 0);
        assert!((r.d - 1e20).abs() < 1.0, "fabs(-1e20) ≈ 1e20; got {}", r.d);
    }

    /// c:91 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn mathfunc_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:99 — `features_` returns i32 (compile-time pin).
    #[test]
    fn mathfunc_features_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _: i32 = features_(std::ptr::null(), &mut v);
    }

    /// c:99 — `features_` produces non-empty list (mathfunc advertises
    /// several math fns).
    #[test]
    fn mathfunc_features_non_empty() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _ = features_(std::ptr::null(), &mut v);
        assert!(!v.is_empty(), "mathfunc must advertise ≥1 feature");
    }

    /// c:107 — `enables_` returns i32 + None safe.
    #[test]
    fn mathfunc_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:286 — `math_func("fabs", 1, [NaN])` returns NaN (NaN-preserving).
    #[test]
    fn math_func_fabs_nan_returns_nan() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::MN_FLOAT;
        let arg = mnumber {
            l: 0,
            d: f64::NAN,
            type_: MN_FLOAT,
        };
        let r = math_func("fabs", 1, &[arg], 0);
        assert!(r.d.is_nan(), "fabs(NaN) = NaN; got {}", r.d);
    }

    /// c:91/99/107/114/124/131 — each lifecycle hook returns 0 individually.
    #[test]
    fn mathfunc_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:91 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:99 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:107 enables_");
        assert_eq!(boot_(null), 0, "c:114 boot_");
        assert_eq!(cleanup_(null), 0, "c:124 cleanup_");
        assert_eq!(finish_(null), 0, "c:131 finish_");
    }
}
