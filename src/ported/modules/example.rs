//! `zsh/example` module — port of `Src/Modules/example.c`.
//!
//! `example.c` is zsh's documentation/template module — it ships with
//! the source tree as a worked example of the loadable-module contract
//! (`setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`
//! plus the `bintab` / `cotab` / `pmtab` / `mftab` / `wrapper` shapes).
//! It is `zmodload`-ed only when the user explicitly invokes it to
//! learn the API; on load it announces itself via stdout, exposes the
//! demo `example` builtin, two demo conds (`-len`, `-ex`), two demo
//! math fns (`length`, `sum`), three demo params (`exint`, `exstr`,
//! `exarr`), and a function-wrapper that flips `GLOBDOTS` for any
//! function whose name starts with `example`.
//!
//! C source: 12 fns total — `bin_example`, `cond_p_len`, `cond_i_ex`,
//! `math_sum`, `math_length`, `ex_wrapper`, `setup_`, `features_`,
//! `enables_`, `boot_`, `cleanup_`, `finish_`. Zero structs/enums in
//! the C source body (only the `static struct builtin bintab[]`,
//! `static struct conddef cotab[]`, etc. arrays of pre-defined zsh-
//! framework types — those types are not redefined by example.c).
//! Three file-statics: `intparam`, `strparam`, `arrparam`.

use crate::ported::compat::output64;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

// ---------------------------------------------------------------------------
// File-static globals — the demo "module storage" that the example
// builtin and the `exint` / `exstr` / `exarr` paramdefs share.
// ---------------------------------------------------------------------------

/// Port of `static zlong intparam;` from `Src/Modules/example.c:35`.
/// Bound to the `exint` integer paramdef at c:175.
pub static INTPARAM: AtomicI64 = AtomicI64::new(0);                      // c:35

/// Port of `static char *strparam;` from `Src/Modules/example.c:36`.
/// Bound to the `exstr` string paramdef at c:176. `None` mirrors C's
/// initial `NULL` which `bin_example` prints as the empty string at
/// c:63.
pub static STRPARAM: Mutex<Option<String>> = Mutex::new(None);           // c:36

/// Port of `static char **arrparam;` from `Src/Modules/example.c:37`.
/// Bound to the `exarr` array paramdef at c:174. `None` mirrors C's
/// initial `NULL`.
pub static ARRPARAM: Mutex<Option<Vec<String>>> = Mutex::new(None);      // c:37

// ---------------------------------------------------------------------------
// Builtin / cond / math / wrapper bodies.
// ---------------------------------------------------------------------------

/// Port of `bin_example()` from `Src/Modules/example.c:42`. The demo
/// `example` builtin: prints set option flags (chars 33..127 with the
/// matching bit in the option bitmap), the argument list, the builtin
/// name, and the module's `intparam`/`strparam`/`arrparam` storage;
/// then assigns argc → intparam, argv[0] → strparam, argv → arrparam
/// (the module's "side effect" demo).
///
/// C signature: `static int bin_example(char *nam, char **args,
///                                       Options ops, int func)`.
/// `Options ops` is a `struct options *` (zsh.h) carrying the parsed
/// option bitmap consulted by the `OPT_ISSET(ops, c)` macro. Per the
/// PORT_CHECKLIST.md rule-3 directive ("Options ops is a bitmask, not
/// a struct"), the Rust port takes a `[bool; 256]` indexed by char —
/// same observable lookup, no abstraction added.
pub fn bin_example(nam: &str, args: &[&str], ops: &[bool; 256]) -> i32 { // c:42
    let mut stdout = std::io::stdout().lock();
    // c:44 — `unsigned char c;`
    // c:45 — `char **oargs = args, **p = arrparam;`
    let oargs = args;                                                    // c:45
    // c:46 — `long i = 0;`
    let mut i: i64 = 0;                                                  // c:46

    // c:48 — `printf("Options: ");`
    let _ = write!(stdout, "Options: ");                                 // c:48
    // c:49-51 — `for (c = 32; ++c < 128;) if (OPT_ISSET(ops,c)) putchar(c);`
    let mut c: u8 = 32;                                                  // c:49
    loop {                                                               // c:49
        c += 1;
        if c >= 128 { break; }
        if ops[c as usize] {                                             // c:50
            let _ = write!(stdout, "{}", c as char);                     // c:51
        }
    }
    // c:52 — `printf("\nArguments:");`
    let _ = write!(stdout, "\nArguments:");                              // c:52
    // c:53-56 — `for (; *args; i++, args++) { putchar(' '); fputs(*args, stdout); }`
    for a in args {                                                      // c:53
        i += 1;                                                          // c:53
        let _ = write!(stdout, " ");                                     // c:54
        let _ = write!(stdout, "{}", a);                                 // c:55
    }
    // c:57 — `printf("\nName: %s\n", nam);`
    let _ = writeln!(stdout, "\nName: {}", nam);                         // c:57

    // c:58-62 — `printf("\nInteger Parameter: %s\n", output64(intparam));`
    // (the `#ifdef ZSH_64_BIT_TYPE` branch is taken on every modern
    // platform — port that branch).
    let intparam = INTPARAM.load(Ordering::Relaxed);                     // c:35 read
    let _ = writeln!(stdout, "\nInteger Parameter: {}", output64(intparam));  // c:59
    // c:63 — `printf("String Parameter: %s\n", strparam ? strparam : "");`
    let sp_guard = STRPARAM.lock().unwrap();
    let sp_str: &str = sp_guard.as_deref().unwrap_or("");                // c:63
    let _ = writeln!(stdout, "String Parameter: {}", sp_str);            // c:63
    drop(sp_guard);
    // c:64-67 — `printf("Array Parameter:"); if (p) while (*p) printf(" %s", *p++); printf("\n");`
    let _ = write!(stdout, "Array Parameter:");                          // c:64
    let ap_guard = ARRPARAM.lock().unwrap();
    if let Some(arr) = ap_guard.as_ref() {                               // c:65
        for s in arr {                                                   // c:66
            let _ = write!(stdout, " {}", s);                            // c:66
        }
    }
    drop(ap_guard);
    let _ = writeln!(stdout);                                            // c:67

    // c:69-74 — side-effect demo:
    //   intparam = i;
    //   zsfree(strparam);
    //   strparam = ztrdup(*oargs ? *oargs : "");
    //   if (arrparam) freearray(arrparam);
    //   arrparam = zarrdup(oargs);
    INTPARAM.store(i, Ordering::Relaxed);                                // c:69
    let new_sp = oargs.first().map(|s| (*s).to_string()).unwrap_or_default();  // c:70-71
    *STRPARAM.lock().unwrap() = Some(new_sp);                            // c:71
    let new_ap: Vec<String> = oargs.iter().map(|s| (*s).to_string()).collect();  // c:74
    *ARRPARAM.lock().unwrap() = Some(new_ap);                            // c:74

    0                                                                    // c:75
}

/// Port of `cond_p_len()` from `Src/Modules/example.c:80`. The demo
/// `-len` cond op: with one arg, true iff the string is empty; with
/// two args, true iff `strlen(s1) == cond_val(a, 1)`.
///
/// C signature: `static int cond_p_len(char **a, int id)`.
pub fn cond_p_len(a: &[&str], _id: i32) -> i32 {                         // c:80
    // c:82 — `char *s1 = cond_str(a, 0, 0);`
    let s1: &str = a.first().copied().unwrap_or("");                     // c:82
    if a.len() >= 2 {                                                    // c:84 a[1]
        // c:85 — `zlong v = cond_val(a, 1);`
        let v: i64 = a[1].parse::<i64>().unwrap_or(0);                   // c:85
        // c:87 — `return strlen(s1) == v;`
        if s1.len() as i64 == v { 1 } else { 0 }                         // c:87
    } else {                                                             // c:88
        // c:89 — `return !s1[0];`
        if s1.is_empty() { 1 } else { 0 }                                // c:89
    }
}

/// Port of `cond_i_ex()` from `Src/Modules/example.c:95`. The demo
/// `-ex` infix cond op: true iff `s1 ++ s2 == "example"`.
///
/// C signature: `static int cond_i_ex(char **a, int id)`.
pub fn cond_i_ex(a: &[&str], _id: i32) -> i32 {                          // c:95
    // c:97 — `char *s1 = cond_str(a, 0, 0), *s2 = cond_str(a, 1, 0);`
    let s1: &str = a.first().copied().unwrap_or("");                     // c:97
    let s2: &str = a.get(1).copied().unwrap_or("");                      // c:97
    // c:99 — `return !strcmp("example", dyncat(s1, s2));`
    let mut combined = String::with_capacity(s1.len() + s2.len());
    combined.push_str(s1);                                               // c:99 dyncat
    combined.push_str(s2);                                               // c:99 dyncat
    if combined == "example" { 1 } else { 0 }                            // c:99 !strcmp
}

/// Port of `math_sum()` from `Src/Modules/example.c:104`. The demo
/// `sum(...)` math fn: variadic numeric sum. Promotes integer running
/// total to float on the first float arg (C's `f` flag).
///
/// C signature: `static mnumber math_sum(char *name, int argc,
///                                        mnumber *argv, int id)`.
pub fn math_sum(_name: &str, argc: i32, argv: &[crate::ported::math::Mnumber],
                _id: i32) -> crate::ported::math::Mnumber                // c:104
{
    use crate::ported::math::{Mnumber, MN_INTEGER, MN_FLOAT};
    // c:106 — `mnumber ret;`
    let mut ret = Mnumber::default();
    // c:107 — `int f = 0;`
    let mut f: i32 = 0;                                                  // c:107
    // c:109 — `ret.u.l = 0;`
    ret.l = 0;                                                           // c:109
    let mut i: usize = 0;
    let mut argc = argc;                                                 // c:110
    // c:110 — `while (argc--)`
    while argc > 0 {
        argc -= 1;                                                       // c:110
        if argv[i].type_ == MN_INTEGER {                                 // c:111
            if f != 0 {                                                  // c:112
                ret.d += argv[i].l as f64;                               // c:113
            } else {                                                     // c:114
                ret.l += argv[i].l;                                      // c:115
            }
        } else {                                                         // c:116
            if f != 0 {                                                  // c:117
                ret.d += argv[i].d;                                      // c:118
            } else {                                                     // c:119
                ret.d = (ret.l as f64) + argv[i].d;                      // c:120
                f = 1;                                                   // c:121
            }
        }
        i += 1;                                                          // c:124 argv++
    }
    // c:126 — `ret.type = (f ? MN_FLOAT : MN_INTEGER);`
    ret.type_ = if f != 0 { MN_FLOAT } else { MN_INTEGER };              // c:126
    ret                                                                  // c:128
}

/// Port of `math_length()` from `Src/Modules/example.c:133`. The demo
/// `length("...")` math fn: returns `strlen(arg)` as an integer.
///
/// C signature: `static mnumber math_length(char *name, char *arg,
///                                            int id)`.
pub fn math_length(_name: &str, arg: &str, _id: i32)
    -> crate::ported::math::Mnumber                                      // c:133
{
    use crate::ported::math::{Mnumber, MN_INTEGER};
    // c:135 — `mnumber ret;`
    // c:137 — `ret.type = MN_INTEGER;`
    // c:138 — `ret.u.l = strlen(arg);`
    Mnumber {
        type_: MN_INTEGER,                                               // c:137
        l: arg.len() as i64,                                             // c:138 strlen(arg)
        d: 0.0,
    }
}

/// Port of `ex_wrapper()` from `Src/Modules/example.c:145`. The
/// per-function wrapper hook: when the function name starts with
/// "example", set `GLOBDOTS` for the duration of the call, then
/// restore.
///
/// C signature: `static int ex_wrapper(Eprog prog, FuncWrap w, char *name)`.
/// Returns 1 to skip wrapping (name doesn't match), 0 after running.
///
/// `Eprog` and `FuncWrap` aren't ported types in zshrs (the wrapper
/// registry is the legacy tree-walker hook system that fusevm
/// replaces); the Rust port keeps the prototype as `(prog, w, name)`
/// with `prog`/`w` as opaque `i32`s so the C signature is preserved
/// at the surface, and skips the inner `runshfunc` invocation that
/// the static-link path can't fire (no addwrapper registry).
pub fn ex_wrapper(_prog: i32, _w: i32, name: &str) -> i32 {              // c:145
    // c:147 — `if (strncmp(name, "example", 7)) return 1;`
    if !name.starts_with("example") {                                    // c:147
        return 1;                                                        // c:148
    }
    // c:149-156 — else branch:
    //   int ogd = opts[GLOBDOTS];
    //   opts[GLOBDOTS] = 1;
    //   runshfunc(prog, w, name);
    //   opts[GLOBDOTS] = ogd;
    //   return 0;
    // Static-link path: never installed via addwrapper, never invoked
    // through the runshfunc dispatcher. Return 0 (matched + ran).
    0                                                                    // c:156
}

// ---------------------------------------------------------------------------
// Module loaders.
// ---------------------------------------------------------------------------

/// Port of `setup_()` from `Src/Modules/example.c:198`.
/// C body:
/// ```c
/// printf("The example module has now been set up.\n");
/// fflush(stdout);
/// return 0;
/// ```
/// Module is opt-in via `zmodload zsh/example`; the announce line is
/// the demo's documented behavior, not zshrs startup chatter.
pub fn setup_() -> i32 {                                                 // c:198
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "The example module has now been set up."); // c:200
    let _ = stdout.flush();                                              // c:201
    0                                                                    // c:202
}

/// Port of `features_()` from `Src/Modules/example.c:207`.
/// C body is `*features = featuresarray(m, &module_features); return 0;`.
/// zshrs static-link path: no runtime feature table, return 0.
pub fn features_() -> i32 {                                              // c:207
    0                                                                    // c:210
}

/// Port of `enables_()` from `Src/Modules/example.c:215`.
/// C body is `return handlefeatures(m, &module_features, enables);`.
/// Static-link path: 0.
pub fn enables_() -> i32 {                                               // c:215
    0                                                                    // c:217
}

/// Port of `boot_()` from `Src/Modules/example.c:222`.
/// C body:
/// ```c
/// intparam = 42;
/// strparam = ztrdup("example");
/// arrparam = (char **) zalloc(3 * sizeof(char *));
/// arrparam[0] = ztrdup("example");
/// arrparam[1] = ztrdup("array");
/// arrparam[2] = NULL;
/// return addwrapper(m, wrapper);
/// ```
/// The `addwrapper` registry doesn't exist in zshrs (fusevm replaces
/// the tree-walker funcwrap dispatcher); the Rust port performs the
/// param initialisation faithfully and returns 0 in place of the
/// `addwrapper` return.
pub fn boot_() -> i32 {                                                  // c:222
    INTPARAM.store(42, Ordering::Relaxed);                               // c:224
    *STRPARAM.lock().unwrap() = Some("example".to_string());             // c:225
    *ARRPARAM.lock().unwrap() = Some(vec![                               // c:226-228
        "example".to_string(),                                           // c:227
        "array".to_string(),                                             // c:228
    ]);
    0                                                                    // c:230 addwrapper return
}

/// Port of `cleanup_()` from `Src/Modules/example.c:235`.
/// C body:
/// ```c
/// deletewrapper(m, wrapper);
/// return setfeatureenables(m, &module_features, NULL);
/// ```
/// `deletewrapper` is the inverse of the addwrapper that boot_
/// skipped; `setfeatureenables` on an empty feature table is a no-op
/// in zshrs's static-link path. Body returns 0.
pub fn cleanup_() -> i32 {                                               // c:235
    0                                                                    // c:238
}

/// Port of `finish_()` from `Src/Modules/example.c:243`.
/// C body:
/// ```c
/// printf("Thank you for using the example module.  Have a nice day.\n");
/// fflush(stdout);
/// return 0;
/// ```
pub fn finish_() -> i32 {                                                // c:243
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout,
        "Thank you for using the example module.  Have a nice day.");    // c:245
    let _ = stdout.flush();                                              // c:246
    0                                                                    // c:247
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::math::{Mnumber, MN_INTEGER, MN_FLOAT};

    /// Verifies `boot_()` populates the three paramdef-bound
    /// statics per c:224-228: intparam=42, strparam="example",
    /// arrparam=["example","array"].
    #[test]
    fn boot_populates_demo_params() {
        boot_();
        assert_eq!(INTPARAM.load(Ordering::SeqCst), 42);
        assert_eq!(STRPARAM.lock().unwrap().as_deref(), Some("example"));
        let arr = ARRPARAM.lock().unwrap();
        let arr = arr.as_ref().expect("arrparam must be Some after boot_");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], "example");
        assert_eq!(arr[1], "array");
    }

    /// Verifies `cond_p_len`'s two-arity forms — c:84/89.
    #[test]
    fn cond_p_len_arities() {
        assert_eq!(cond_p_len(&["hello", "5"], 0), 1);
        assert_eq!(cond_p_len(&["hello", "4"], 0), 0);
        assert_eq!(cond_p_len(&[""], 0), 1);
        assert_eq!(cond_p_len(&["x"], 0), 0);
    }

    /// Verifies `cond_i_ex` matches only the exact concat "example".
    /// — c:99 `!strcmp("example", dyncat(s1, s2))`.
    #[test]
    fn cond_i_ex_concat_matches_example() {
        assert_eq!(cond_i_ex(&["exam", "ple"], 0), 1);
        assert_eq!(cond_i_ex(&["example", ""], 0), 1);
        assert_eq!(cond_i_ex(&["example", "x"], 0), 0);
        assert_eq!(cond_i_ex(&["foo", "bar"], 0), 0);
    }

    /// Verifies `math_sum` returns integer sum for all-int inputs
    /// and promotes to float once a float arg is seen — c:111/116/126.
    #[test]
    fn math_sum_int_then_float_promotion() {
        let ints = [Mnumber::integer(1), Mnumber::integer(2), Mnumber::integer(3)];
        let r = math_sum("sum", 3, &ints, 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 6);

        let mixed = [Mnumber::integer(1), Mnumber::float(2.5),
                     Mnumber::integer(3)];
        let r = math_sum("sum", 3, &mixed, 0);
        assert_eq!(r.type_, MN_FLOAT);
        assert!((r.d - 6.5).abs() < 1e-9);
    }

    /// Verifies `math_length` returns string length as integer — c:138.
    #[test]
    fn math_length_returns_strlen() {
        let r = math_length("length", "hello", 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 5);
    }

    /// Verifies `ex_wrapper` returns 1 (skip) for non-matching names
    /// and 0 (matched) for `example`-prefixed names — c:147/156.
    #[test]
    fn ex_wrapper_name_prefix_match() {
        assert_eq!(ex_wrapper(0, 0, "foo"), 1);
        assert_eq!(ex_wrapper(0, 0, "exampl"), 1);  // 6 chars, doesn't match prefix
        assert_eq!(ex_wrapper(0, 0, "example"), 0);
        assert_eq!(ex_wrapper(0, 0, "example_func"), 0);
    }
}
