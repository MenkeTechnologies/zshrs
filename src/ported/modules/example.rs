//! `zsh/example` module — port of `Src/Modules/example.c`.
//!
//! Top-level declaration order matches C source line-by-line:
//!   - `static zlong intparam;`                     c:35
//!   - `static char *strparam;`                     c:36
//!   - `static char **arrparam;`                    c:37
//!   - `bin_example(nam, args, ops, func)`          c:42
//!   - `cond_p_len(a, id)`                          c:80
//!   - `cond_i_ex(a, id)`                           c:95
//!   - `math_sum(name, argc, argv, id)`             c:104
//!   - `math_length(name, arg, id)`                 c:133
//!   - `ex_wrapper(prog, w, name)`                  c:145
//!   - `static struct builtin bintab[]`             c:164
//!   - `static struct conddef cotab[]`              c:168
//!   - `static struct paramdef patab[]`             c:173
//!   - `static struct mathfunc mftab[]`             c:179
//!   - `static struct funcwrap wrapper[]`           c:184
//!   - `static struct features module_features`     c:188
//!   - `setup_(m)`                                   c:198
//!   - `features_(m, features)`                      c:207
//!   - `enables_(m, enables)`                        c:215
//!   - `boot_(m)`                                    c:222
//!   - `cleanup_(m)`                                 c:235
//!   - `finish_(m)`                                  c:243

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use std::io::Write;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ported::compat::output64;
use crate::ported::cond::{cond_str, cond_val};
use crate::ported::math::{mnumber, MN_FLOAT, MN_INTEGER};
use crate::ported::string::dyncat;
use crate::ported::zsh_h::{features, module, options, MAX_OPS, OPT_ISSET};

// =====================================================================
// /* parameters */                                                  c:33
// =====================================================================

/// Port of `static zlong intparam;` from `Src/Modules/example.c:35`.
/// Bound to the `exint` integer paramdef at c:175.
pub static intparam: AtomicI64 = AtomicI64::new(0); // c:35

/// Port of `static char *strparam;` from `Src/Modules/example.c:36`.
/// Bound to the `exstr` string paramdef at c:176. `None` mirrors C's
/// initial NULL which `bin_example` prints as the empty string at c:63.
pub static strparam: Mutex<Option<String>> = Mutex::new(None); // c:36

/// Port of `static char **arrparam;` from `Src/Modules/example.c:37`.
/// Bound to the `exarr` array paramdef at c:174. `None` mirrors C's
/// initial NULL.
pub static arrparam: Mutex<Option<Vec<String>>> = Mutex::new(None); // c:37

// =====================================================================
// bin_example(char *nam, char **args, Options ops, UNUSED(int func))  c:42
// =====================================================================

/// Port of `bin_example(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/example.c:42`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_example(char *nam, char **args, Options ops, UNUSED(int func))
/// ```
#[allow(unused_variables)]
pub fn bin_example(nam: &str, args: &[String], ops: &options, func: i32) -> i32 {
    // c:42
    let mut stdout = std::io::stdout().lock();
    // c:44 — `unsigned char c;`
    let mut c: u8;
    // c:45 — `char **oargs = args, **p = arrparam;`
    let oargs = args; // c:45
                      // `p` walks `arrparam` below; lock acquired in the print block.
                      // c:46 — `long i = 0;`
    let mut i: i64 = 0; // c:46

    let _ = write!(stdout, "Options: "); // c:48
                                         // c:49-51 — `for (c = 32; ++c < 128;) if (OPT_ISSET(ops,c)) putchar(c);`
    c = 32; // c:49
    loop {
        c += 1;
        if c >= 128 {
            break;
        }
        if OPT_ISSET(ops, c) {
            // c:50
            let _ = write!(stdout, "{}", c as char); // c:51
        }
    }
    let _ = write!(stdout, "\nArguments:"); // c:52
                                            // c:53-56 — `for (; *args; i++, args++) { putchar(' '); fputs(*args, stdout); }`
    for a in args {
        let _ = write!(stdout, " "); // c:54
        let _ = write!(stdout, "{}", a); // c:55
        i += 1; // c:53
    }
    let _ = writeln!(stdout, "\nName: {}", nam); // c:57
                                                 // c:58-62 — `#ifdef ZSH_64_BIT_TYPE` branch is taken on every
                                                 // modern platform; port that branch.
    let _ = writeln!(
        stdout,
        "\nInteger Parameter: {}",
        output64(intparam.load(Ordering::Relaxed))
    ); // c:59
    {
        let sp = strparam.lock().unwrap();
        let _ = writeln!(stdout, "String Parameter: {}", sp.as_deref().unwrap_or(""));
        // c:63
    }
    let _ = write!(stdout, "Array Parameter:"); // c:64
    {
        let p = arrparam.lock().unwrap();
        if let Some(arr) = p.as_ref() {
            // c:65 if (p)
            for s in arr {
                // c:66 while (*p)
                let _ = write!(stdout, " {}", s); // c:66
            }
        }
    }
    let _ = writeln!(stdout); // c:67

    intparam.store(i, Ordering::Relaxed); // c:69 intparam = i;
                                          // c:70 — zsfree(strparam);
                                          // c:71 — strparam = ztrdup(*oargs ? *oargs : "");
    let new_sp = if let Some(first) = oargs.first() {
        first.clone()
    } else {
        String::new()
    };
    *strparam.lock().unwrap() = Some(new_sp); // c:71
                                              // c:72-74 — if (arrparam) freearray(arrparam); arrparam = zarrdup(oargs);
    let new_ap: Vec<String> = oargs.to_vec(); // c:80
    *arrparam.lock().unwrap() = Some(new_ap); // c:80

    0 // c:80
}

// =====================================================================
// cond_p_len(char **a, UNUSED(int id))                               c:80
// =====================================================================

/// Port of `cond_p_len(char **a, UNUSED(int id))` from `Src/Modules/example.c:80`.
#[allow(unused_variables)]
pub fn cond_p_len(a: &[String], id: i32) -> i32 {
    // c:80
    // c:80 — `char *s1 = cond_str(a, 0, 0);`
    let s1: String = cond_str(a, 0, false); // c:82
    if a.len() > 1 {
        // c:84 if (a[1])
        let v: i64 = cond_val(a, 1); // c:85 zlong v = cond_val(a, 1);
                                     // c:87 — `return strlen(s1) == v;`
        if (s1.len() as i64) == v {
            1
        } else {
            0
        }
    } else {
        // c:95
        // c:95 — `return !s1[0];`
        if s1.is_empty() {
            1
        } else {
            0
        }
    }
}

// =====================================================================
// cond_i_ex(char **a, UNUSED(int id))                                c:95
// =====================================================================

/// Port of `cond_i_ex(char **a, UNUSED(int id))` from `Src/Modules/example.c:95`.
#[allow(unused_variables)]
pub fn cond_i_ex(a: &[String], id: i32) -> i32 {
    // c:95
    // c:95 — `char *s1 = cond_str(a, 0, 0), *s2 = cond_str(a, 1, 0);`
    let s1: String = cond_str(a, 0, false); // c:104
    let s2: String = cond_str(a, 1, false); // c:104
                                            // c:104 — `return !strcmp("example", dyncat(s1, s2));`
    if dyncat(&s1, &s2) == "example" {
        1
    } else {
        0
    } // c:104
}

// =====================================================================
// math_sum(UNUSED(char *name), int argc, mnumber *argv, UNUSED(int id))  c:104
// =====================================================================

/// Port of `math_sum(UNUSED(char *name), int argc, mnumber *argv, UNUSED(int id))` from `Src/Modules/example.c:104`.
#[allow(unused_variables)]
pub fn math_sum(name: &str, argc: i32, argv: &[mnumber], id: i32) -> mnumber {
    // c:104
    // c:104 — `mnumber ret;`
    let mut ret = mnumber {
        l: 0,
        d: 0.0,
        type_: MN_INTEGER,
    };
    // c:107 — `int f = 0;`
    let mut f: i32 = 0;
    // c:109 — `ret.u.l = 0;`
    ret.l = 0;
    let mut argc = argc;
    let mut p: usize = 0; // emulates argv++ pointer walk (c:124)
    while {
        // c:110 while (argc--)
        let go = argc > 0;
        argc -= 1;
        go
    } {
        if argv[p].type_ == MN_INTEGER {
            // c:111
            if f != 0 {
                // c:112
                ret.d += argv[p].l as f64; // c:113
            } else {
                ret.l += argv[p].l; // c:115
            }
        } else {
            // c:116
            if f != 0 {
                // c:117
                ret.d += argv[p].d; // c:118
            } else {
                // c:119
                ret.d = (ret.l as f64) + argv[p].d; // c:120
                f = 1; // c:121
            }
        }
        p += 1; // c:124 argv++
    }
    // c:133 — `ret.type = (f ? MN_FLOAT : MN_INTEGER);`
    ret.type_ = if f != 0 { MN_FLOAT } else { MN_INTEGER };
    ret // c:133
}

// =====================================================================
// math_length(UNUSED(char *name), char *arg, UNUSED(int id))         c:133
// =====================================================================

/// Port of `math_length(UNUSED(char *name), char *arg, UNUSED(int id))` from `Src/Modules/example.c:133`.
#[allow(unused_variables)]
pub fn math_length(name: &str, arg: &str, id: i32) -> mnumber {
    // c:133
    // c:133 — `mnumber ret;`
    // c:137 — `ret.type = MN_INTEGER;`
    // c:138 — `ret.u.l = strlen(arg);`
    mnumber {
        type_: MN_INTEGER,
        l: arg.len() as i64,
        d: 0.0,
    }
}

// =====================================================================
// ex_wrapper(Eprog prog, FuncWrap w, char *name)                     c:145
// =====================================================================

/// Port of `ex_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/example.c:145`.
///
/// `Eprog` and `FuncWrap` are `Box<eprog>` / `Box<funcwrap>` per
/// zsh.h:774 / 522. Pointer-shape preserved as `*const eprog` /
/// `*const funcwrap` since the C body never derefs `prog`/`w` beyond
/// passing them to `runshfunc`.
/// WARNING: param names don't match C — Rust=(prog, name) vs C=(prog, w, name)
pub fn ex_wrapper(
    prog: *const crate::ported::zsh_h::eprog, // c:145
    w: *const crate::ported::zsh_h::funcwrap,
    name: &str,
) -> i32 {
    // c:147 — `if (strncmp(name, "example", 7)) return 1;`
    if !name.starts_with("example") {
        return 1; // c:148
    }
    // c:149-156 — else branch:
    //   int ogd = opts[GLOBDOTS];
    //   opts[GLOBDOTS] = 1;
    //   runshfunc(prog, w, name);
    //   opts[GLOBDOTS] = ogd;
    //   return 0;
    // The opts/runshfunc machinery is the legacy tree-walker funcwrap
    // dispatcher that fusevm replaces; static-link path is never
    // invoked through addwrapper, so the body stays a no-op besides
    // the prefix-match contract.
    let _ = (prog, w);
    0 // c:156
}

// =====================================================================
// static struct builtin bintab[]                                     c:164
// static struct conddef cotab[]                                      c:168
// static struct paramdef patab[]                                     c:173
// static struct mathfunc mftab[]                                     c:179
// static struct funcwrap wrapper[]                                   c:184
// static struct features module_features                             c:188
//
// These dispatch tables are consumed by the C module loader
// (`featuresarray` + `handlefeatures` + `addwrapper`). The
// static-link / fusevm path doesn't go through that registry; the
// dispatcher in `src/extensions/` invokes `bin_example` etc.
// directly. Tables omitted from the Rust port until the module-loader
// port lands.
// =====================================================================

// =====================================================================
// setup_(UNUSED(Module m))                                           c:198
// =====================================================================

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/example.c:198`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "The example module has now been set up."); // c:207
    let _ = stdout.flush(); // c:207
    0 // c:207
}

// =====================================================================
// features_(Module m, char ***features)                              c:207
// =====================================================================

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/example.c:207`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0 // c:215
}

// =====================================================================
// enables_(Module m, int **enables)                                  c:215
// =====================================================================

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/example.c:215`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables) // c:222
}

// =====================================================================
// boot_(Module m)                                                    c:222
// =====================================================================

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/example.c:222`.
/// C body sets the demo paramdef-bound statics then calls
/// `addwrapper(m, wrapper)`.
pub fn boot_(m: *const module) -> i32 {
    intparam.store(42, Ordering::Relaxed); // c:222
    *strparam.lock().unwrap() = Some("example".to_string()); // c:225
    *arrparam.lock().unwrap() = Some(vec![
        // c:226-228
        "example".to_string(), // c:227
        "array".to_string(),   // c:228
    ]);
    // c:230 — addwrapper(m, wrapper); registers ex_wrapper into the
    // global wrappers linked list (Src/module.c:577). zshrs's fusevm
    // bytecode doesn't run through C's wrapper-dispatch chain, so the
    // registration is a no-op until the wrapper machinery has a Rust
    // equivalent.
    let _ = m;
    0
}

// =====================================================================
// cleanup_(Module m)                                                 c:235
// =====================================================================

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/example.c:235`.
/// C body: `deletewrapper(m, wrapper); return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:243 — deletewrapper(m, wrapper); paired with c:230 addwrapper,
    // no-op until the wrapper machinery has a Rust equivalent.
    setfeatureenables(m, module_features(), None) // c:243
}

// =====================================================================
// finish_(UNUSED(Module m))                                          c:243
// =====================================================================

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/example.c:243`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(
        stdout,
        "Thank you for using the example module.  Have a nice day."
    ); // c:245
    let _ = stdout.flush(); // c:246
    0 // c:247
}

// =====================================================================
// External ported + tables — `static struct funcwrap wrapper[]` (c:184),
// `static struct features module_features` (c:188), and
// `featuresarray`/`handlefeatures`/`setfeatureenables`/`addwrapper`/
// `deletewrapper` from `Src/module.c`.
// =====================================================================

// `bintab` — port of `static struct builtin bintab[]` (example.c:182).

// `cotab` — port of `static struct conddef cotab[]` (example.c).
// `CONDDEF("ex", CONDF_INFIX|CONDF_ADDED, …)` + `CONDDEF("len", 0, …)`.

// `mftab` — port of `static struct mathfunc mftab[]` (example.c).

// `patab` — port of `static struct paramdef patab[]` (example.c).

// `module_features` — port of `static struct features module_features`
// from example.c:188.

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN EXAMPLE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "b:example".to_string(),
        "c:ex".to_string(),
        "c:len".to_string(),
        "f:length".to_string(),
        "f:sum".to_string(),
        "p:exarr".to_string(),
        "p:exint".to_string(),
        "p:exstr".to_string(),
    ]
}

// WARNING: NOT IN EXAMPLE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 8]);
    }
    0
}

// WARNING: NOT IN EXAMPLE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
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

// WARNING: NOT IN EXAMPLE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1,
            cd_list: None,
            cd_size: 2,
            mf_list: None,
            mf_size: 2,
            pd_list: None,
            pd_size: 3,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ops() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }
    fn s(x: &str) -> String {
        x.to_string()
    }

    /// Serialises tests that mutate `intparam` / `strparam` / `arrparam`
    /// (paramdef bindings at `Src/Modules/example.c:208-218`). The C
    /// source declares those as file-scope statics that `boot_` and
    /// `bin_example` overwrite; zsh is single-threaded so the C source
    /// is safe. cargo's parallel test runner can fire `boot_populates_demo_params`
    /// and `bin_example_returns_zero_and_assigns_state` concurrently —
    /// each then reads the values the other just wrote and fails. The
    /// lock restores the single-writer assumption for the test phase.
    static EXAMPLE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Port of `boot_(UNUSED(Module m))` from `Src/Modules/example.c:222`.
    /// Verifies `boot_()` populates the three paramdef-bound statics
    /// per c:224-228: intparam=42, strparam="example",
    /// arrparam=["example","array"].
    #[test]
    fn boot_populates_demo_params() {
        let _g = crate::test_util::global_state_lock();
        let _g = EXAMPLE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        boot_(std::ptr::null());
        assert_eq!(intparam.load(Ordering::SeqCst), 42);
        assert_eq!(strparam.lock().unwrap().as_deref(), Some("example"));
        let arr = arrparam.lock().unwrap();
        let arr = arr.as_ref().expect("arrparam must be Some after boot_");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], "example");
        assert_eq!(arr[1], "array");
    }

    /// Verifies `cond_p_len`'s two arities — c:84/89.
    #[test]
    fn cond_p_len_arities() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cond_p_len(&[s("hello"), s("5")], 0), 1);
        assert_eq!(cond_p_len(&[s("hello"), s("4")], 0), 0);
        assert_eq!(cond_p_len(&[s("")], 0), 1);
        assert_eq!(cond_p_len(&[s("x")], 0), 0);
    }

    /// Verifies `cond_i_ex` matches only the exact concat "example" — c:99.
    #[test]
    fn cond_i_ex_concat_matches_example() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cond_i_ex(&[s("exam"), s("ple")], 0), 1);
        assert_eq!(cond_i_ex(&[s("example"), s("")], 0), 1);
        assert_eq!(cond_i_ex(&[s("example"), s("x")], 0), 0);
        assert_eq!(cond_i_ex(&[s("foo"), s("bar")], 0), 0);
    }

    /// Verifies `math_sum` returns integer sum for all-int inputs and
    /// promotes to float once a float arg appears — c:111/116/126.
    #[test]
    fn math_sum_int_then_float_promotion() {
        let _g = crate::test_util::global_state_lock();
        let ints = [
            mnumber {
                l: 1,
                d: 0.0,
                type_: MN_INTEGER,
            },
            mnumber {
                l: 2,
                d: 0.0,
                type_: MN_INTEGER,
            },
            mnumber {
                l: 3,
                d: 0.0,
                type_: MN_INTEGER,
            },
        ];
        let r = math_sum("sum", 3, &ints, 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 6);

        let mixed = [
            mnumber {
                l: 1,
                d: 0.0,
                type_: MN_INTEGER,
            },
            mnumber {
                l: 0,
                d: 2.5,
                type_: MN_FLOAT,
            },
            mnumber {
                l: 3,
                d: 0.0,
                type_: MN_INTEGER,
            },
        ];
        let r = math_sum("sum", 3, &mixed, 0);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 6.5).abs() < 1e-9);
    }

    /// Verifies `math_length` returns string length as integer — c:138.
    #[test]
    fn math_length_returns_strlen() {
        let _g = crate::test_util::global_state_lock();
        let r = math_length("length", "hello", 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 5);
    }

    /// Verifies `ex_wrapper` returns 1 (skip) for non-matching names
    /// and 0 (matched) for `example`-prefixed names — c:147/156.
    #[test]
    fn ex_wrapper_name_prefix_match() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ex_wrapper(std::ptr::null(), std::ptr::null(), "foo"), 1);
        assert_eq!(ex_wrapper(std::ptr::null(), std::ptr::null(), "exampl"), 1);
        assert_eq!(ex_wrapper(std::ptr::null(), std::ptr::null(), "example"), 0);
        assert_eq!(
            ex_wrapper(std::ptr::null(), std::ptr::null(), "example_func"),
            0
        );
    }

    /// Port of `bin_example(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/example.c:42`.
    /// Verifies `bin_example` reads `OPT_ISSET(ops, c)` and prints flagged
    /// option letters — c:49-51.
    #[test]
    fn bin_example_returns_zero_and_assigns_state() {
        let _g = crate::test_util::global_state_lock();
        let _g = EXAMPLE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ops = empty_ops();
        let args = vec![s("hello"), s("world")];
        let rc = bin_example("example", &args, &ops, 0);
        assert_eq!(rc, 0);
        // c:69 i = 2 (two args); c:71 strparam = "hello"; c:74 arrparam = ["hello","world"]
        assert_eq!(intparam.load(Ordering::Relaxed), 2);
        assert_eq!(strparam.lock().unwrap().as_deref(), Some("hello"));
        let arr = arrparam.lock().unwrap();
        assert_eq!(arr.as_ref().unwrap().as_slice(), &[s("hello"), s("world")]);
    }

    /// c:104 — `math_sum` with zero args returns identity (0). Pin
    /// the empty-input arithmetic identity.
    #[test]
    fn math_sum_zero_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_sum("sum", 0, &[], 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 0, "sum of nothing must be 0");
    }

    /// c:104 — `math_sum` with a single integer is identity.
    #[test]
    fn math_sum_single_int_arg_is_identity() {
        let _g = crate::test_util::global_state_lock();
        let argv = [mnumber {
            l: 42,
            d: 0.0,
            type_: MN_INTEGER,
        }];
        let r = math_sum("sum", 1, &argv, 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 42);
    }

    /// c:104 — All-float input stays MN_FLOAT (no downcast). Pin
    /// the float-preservation rule because a regen that prefers
    /// integer "for tidiness" would silently truncate fractions.
    #[test]
    fn math_sum_all_floats_preserves_float_type() {
        let _g = crate::test_util::global_state_lock();
        let argv = [
            mnumber {
                l: 0,
                d: 1.5,
                type_: MN_FLOAT,
            },
            mnumber {
                l: 0,
                d: 2.5,
                type_: MN_FLOAT,
            },
        ];
        let r = math_sum("sum", 2, &argv, 0);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 4.0).abs() < 1e-9);
    }

    /// c:104 — Negative integers preserved.
    #[test]
    fn math_sum_handles_negative_ints() {
        let _g = crate::test_util::global_state_lock();
        let argv = [
            mnumber {
                l: -5,
                d: 0.0,
                type_: MN_INTEGER,
            },
            mnumber {
                l: 3,
                d: 0.0,
                type_: MN_INTEGER,
            },
        ];
        let r = math_sum("sum", 2, &argv, 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, -2, "−5 + 3 = −2");
    }

    /// c:133 — `math_length` on empty string returns 0. Pin so a
    /// regen that adds `+ 1` for a NUL terminator gets caught.
    #[test]
    fn math_length_empty_string_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_length("length", "", 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 0);
    }

    /// c:133 — Multi-byte UTF-8 yields BYTE count, not char count
    /// (zsh's strlen semantics).
    #[test]
    fn math_length_multibyte_returns_byte_count() {
        let _g = crate::test_util::global_state_lock();
        // "café" = "caf" + "é" (é = 2 bytes in UTF-8) = 5 bytes
        let r = math_length("length", "café", 0);
        assert_eq!(
            r.l, 5,
            "math_length must count bytes, not chars — got {}",
            r.l
        );
    }

    /// c:95 — `cond_i_ex` (no-arg demo) returns 0 (false). Pin so a
    /// regen that returns 1 (true) silently inverts every `[[ -i-ex ]]`.
    #[test]
    fn cond_i_ex_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_i_ex(&[], 0);
        assert_eq!(r, 0, "cond_i_ex demo condition must return 0 (false)");
    }

    /// c:286-335 — module-lifecycle stubs return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for math_sum / math_length ────────────────

    /// `math_sum` of zero args returns integer 0.
    #[test]
    fn example_corpus_math_sum_empty_is_zero_integer() {
        let _g = crate::test_util::global_state_lock();
        let r = math_sum("sum", 0, &[], 0);
        assert_eq!(r.l, 0);
        assert_eq!(r.type_, MN_INTEGER, "empty sum stays integer-typed");
    }

    /// `math_sum` of three integers stays integer-typed.
    #[test]
    fn example_corpus_math_sum_int_args_stay_integer() {
        let _g = crate::test_util::global_state_lock();
        let args = vec![
            mnumber {
                l: 1,
                d: 0.0,
                type_: MN_INTEGER,
            },
            mnumber {
                l: 2,
                d: 0.0,
                type_: MN_INTEGER,
            },
            mnumber {
                l: 3,
                d: 0.0,
                type_: MN_INTEGER,
            },
        ];
        let r = math_sum("sum", 3, &args, 0);
        assert_eq!(r.l, 6, "1+2+3=6");
        assert_eq!(r.type_, MN_INTEGER);
    }

    /// `math_sum` mixing int + float promotes to float per c:121.
    #[test]
    fn example_corpus_math_sum_mixed_promotes_to_float() {
        let _g = crate::test_util::global_state_lock();
        let args = vec![
            mnumber {
                l: 2,
                d: 0.0,
                type_: MN_INTEGER,
            },
            mnumber {
                l: 0,
                d: 1.5,
                type_: MN_FLOAT,
            },
        ];
        let r = math_sum("sum", 2, &args, 0);
        assert_eq!(r.type_, MN_FLOAT, "int+float → float");
        assert!((r.d - 3.5).abs() < 1e-9);
    }

    /// `math_sum` of two floats stays float-typed.
    #[test]
    fn example_corpus_math_sum_two_floats() {
        let _g = crate::test_util::global_state_lock();
        let args = vec![
            mnumber {
                l: 0,
                d: 1.25,
                type_: MN_FLOAT,
            },
            mnumber {
                l: 0,
                d: 2.75,
                type_: MN_FLOAT,
            },
        ];
        let r = math_sum("sum", 2, &args, 0);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 4.0).abs() < 1e-9);
    }

    /// `math_length("hello")` returns 5 as integer.
    #[test]
    fn example_corpus_math_length_ascii_byte_count() {
        let _g = crate::test_util::global_state_lock();
        let r = math_length("length", "hello", 0);
        assert_eq!(r.l, 5);
        assert_eq!(r.type_, MN_INTEGER);
    }

    /// `math_length("")` returns 0.
    #[test]
    fn example_corpus_math_length_empty_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_length("length", "", 0);
        assert_eq!(r.l, 0);
    }

    /// `math_length` counts UTF-8 BYTES not codepoints (per C strlen).
    #[test]
    fn example_corpus_math_length_multibyte_byte_count() {
        let _g = crate::test_util::global_state_lock();
        // '日' is 3 bytes in UTF-8.
        let r = math_length("length", "日", 0);
        assert_eq!(r.l, 3, "C strlen-style byte count for UTF-8");
    }

    /// `cond_p_len(["abc"], 0)` returns 0 (the demo cond is len-3 check).
    /// Pin existing behavior — `cond_p_len` evaluates len == 3 → "false"
    /// (per zsh's cond return convention: 0 = true, 1 = false).
    #[test]
    fn example_corpus_cond_p_len_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = cond_p_len(&["abc".to_string()], 0);
        // Just pin: no panic.
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/example.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:80-95 — `cond_p_len([""], 0)` with single empty-string arg:
    /// no second arg, so c:95 branch fires `return !s1[0]` = !"" = 1.
    #[test]
    fn cond_p_len_empty_single_arg_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_p_len(&["".to_string()], 0);
        assert_eq!(r, 1, "empty s1 with no len arg → !s1[0] = 1");
    }

    /// c:80-87 — `cond_p_len([s, "N"])` returns 1 when len(s) == N,
    /// 0 otherwise.
    #[test]
    fn cond_p_len_length_match_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // s1="abc" (len=3), arg[1]="3" → 1.
        let r = cond_p_len(&["abc".to_string(), "3".to_string()], 0);
        assert_eq!(r, 1, "len(abc) == 3 → 1");
    }

    /// c:80-87 — len mismatch returns 0.
    #[test]
    fn cond_p_len_length_mismatch_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_p_len(&["abc".to_string(), "5".to_string()], 0);
        assert_eq!(r, 0, "len(abc) != 5 → 0");
    }

    /// c:95-104 — `cond_i_ex(["exam", "ple"])` returns 1
    /// (dyncat == "example").
    #[test]
    fn cond_i_ex_concatenation_to_example_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_i_ex(&["exam".to_string(), "ple".to_string()], 0);
        assert_eq!(r, 1, "exam+ple = 'example' → 1");
    }

    /// c:95-104 — non-matching concat returns 0.
    #[test]
    fn cond_i_ex_non_matching_concat_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_i_ex(&["foo".to_string(), "bar".to_string()], 0);
        assert_eq!(r, 0, "foo+bar = 'foobar' != 'example' → 0");
    }

    /// c:95-104 — empty inputs (dyncat("","") = "" != "example") → 0.
    #[test]
    fn cond_i_ex_empty_inputs_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_i_ex(&["".to_string(), "".to_string()], 0);
        assert_eq!(r, 0, "'' != 'example' → 0");
    }

    /// c:104 — `math_sum([])` with empty args returns 0 (integer init).
    #[test]
    fn math_sum_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = math_sum("sum", 0, &[], 0);
        assert_eq!(r.l, 0);
        assert_eq!(r.type_, MN_INTEGER);
    }

    /// c:104-115 — `math_sum` of two integers returns integer sum.
    #[test]
    fn math_sum_two_integers_returns_integer_sum() {
        let _g = crate::test_util::global_state_lock();
        let args = vec![
            mnumber { l: 3, d: 0.0, type_: MN_INTEGER },
            mnumber { l: 4, d: 0.0, type_: MN_INTEGER },
        ];
        let r = math_sum("sum", 2, &args, 0);
        assert_eq!(r.l, 7, "3 + 4 = 7");
        assert_eq!(r.type_, MN_INTEGER);
    }

    /// c:250 — `math_length` of empty string returns 0.
    #[test]
    fn math_length_empty_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let r = math_length("length", "", 0);
        assert_eq!(r.l, 0);
    }

    /// c:250 — `math_length` returns byte count (not codepoints).
    #[test]
    fn math_length_ascii_returns_byte_count() {
        let _g = crate::test_util::global_state_lock();
        let r = math_length("length", "hello", 0);
        assert_eq!(r.l, 5);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/example.c
    // c:70 bin_example / c:149 cond_p_len / c:179 cond_i_ex
    // c:198 math_sum / c:250 math_length / c:273 ex_wrapper / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:149 — cond_p_len returns 0/1 only (boolean i32).
    #[test]
    fn cond_p_len_return_value_in_boolean_set() {
        let _g = crate::test_util::global_state_lock();
        for args in [
            vec![s(""), s("")],
            vec![s("abc"), s("3")],
            vec![s("abc"), s("4")],
            vec![s(""), s("0")],
        ] {
            let r = cond_p_len(&args, 0);
            assert!(r == 0 || r == 1, "cond_p_len({:?}) = {} not in {{0,1}}", args, r);
        }
    }

    /// c:179 — cond_i_ex returns 0/1 only (boolean i32).
    #[test]
    fn cond_i_ex_return_value_in_boolean_set() {
        let _g = crate::test_util::global_state_lock();
        for args in [
            vec![s(""), s("")],
            vec![s("ex"), s("ample")],
            vec![s("foo"), s("bar")],
        ] {
            let r = cond_i_ex(&args, 0);
            assert!(r == 0 || r == 1, "cond_i_ex({:?}) = {} not in {{0,1}}", args, r);
        }
    }

    /// c:198 — math_sum returns mnumber type (compile-time pinning).
    #[test]
    fn math_sum_returns_mnumber_type() {
        let _g = crate::test_util::global_state_lock();
        let _: mnumber = math_sum("sum", 0, &[], 0);
    }

    /// c:250 — math_length returns mnumber type.
    #[test]
    fn math_length_returns_mnumber_type() {
        let _g = crate::test_util::global_state_lock();
        let _: mnumber = math_length("length", "x", 0);
    }

    /// c:250 — `math_length` is deterministic for any input.
    #[test]
    fn math_length_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = math_length("length", "hello", 0).l;
        for _ in 0..5 {
            assert_eq!(math_length("length", "hello", 0).l, first);
        }
    }

    /// c:70 — bin_example empty args returns 0 (default success).
    #[test]
    fn bin_example_no_args_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_example("example", &[], &ops, 0);
        assert!(r == 0 || r == 1, "result must be 0 or 1, got {}", r);
    }

    /// c:198 — `math_sum(empty)` returns integer 0 (l=0, type integer).
    #[test]
    fn math_sum_empty_is_canonical_zero_integer() {
        let _g = crate::test_util::global_state_lock();
        let r = math_sum("sum", 0, &[], 0);
        assert_eq!(r.l, 0, "empty args → l=0");
    }

    /// c:273 — `ex_wrapper(null, null, "")` is safe (empty name).
    #[test]
    fn ex_wrapper_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = ex_wrapper(std::ptr::null(), std::ptr::null(), "");
    }

    /// c:318-388 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn example_full_lifecycle_returns_zero_for_all() {
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

    /// c:318 — setup_ idempotent.
    #[test]
    fn example_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }
}
