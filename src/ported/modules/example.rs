//! `zsh/example` module — port of `Src/Modules/example.c`.
//!
//! `example.c` is zsh's documentation/template module — it ships
//! with the source tree as a worked example of the loadable-module
//! contract (`setup_`, `getrandom_buffer`, `cleanup_`, `finish_`, the
//! `bintab` / `cotab` / `pmtab` / `mftab` shapes). It is never
//! `zmodload`-ed at runtime by any real script; its purpose is to
//! be read.
//!
//! This Rust file exists so the 1:1 mapping between
//! `Src/Modules/*.c` and `src/modules/*.rs` is complete. Surface
//! is intentionally empty — there's nothing for callers to invoke.
//! The C example exposes a single demo builtin `example` whose body
//! prints flag arguments; we omit it because no zshrs consumer
//! would ever load it.

/// Port of `setup_()` from `Src/Modules/example.c:198`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:198
    0                                                                    // c:201
}

/// Port of `features_()` from `Src/Modules/example.c:207`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
/// example is documentation-only — never loaded; static-link: 0.
pub fn features_() -> i32 {                                              // c:207
    0                                                                    // c:211
}

/// Port of `enables_()` from `Src/Modules/example.c:215`. C body
/// is `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:215
    0                                                                    // c:218
}

/// Port of `boot_()` from `Src/Modules/example.c:222`. C body
/// installs the `example` function-wrapper via `addwrapper(m,
/// wrapper)`. example module is doc-only in zshrs; loader returns 0.
pub fn boot_() -> i32 {                                                  // c:222
    0                                                                    // c:228
}

/// Port of `cleanup_()` from `Src/Modules/example.c:235`. C body
/// is `deletewrapper(m, wrapper);
///     return setfeatureenables(m, &module_features, NULL);`.
/// Static-link / never-loaded path: 0.
pub fn cleanup_() -> i32 {                                               // c:235
    0                                                                    // c:240
}

/// Port of `finish_()` from `Src/Modules/example.c:243`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:243
    0                                                                    // c:246
}

/// Port of `bin_example()` from `Src/Modules/example.c:42`. The
/// demo `example` builtin: prints set option flags, the argument
/// list, the builtin name, and the module's `intparam`/`strparam`/
/// `arrparam` storage; then assigns argc → intparam, argv[0] →
/// strparam, argv → arrparam (the module's "side effect" demo).
///
/// example is documentation-only — never loaded by any real script.
/// This entry exists for C-name parity. The demo's true purpose is
/// to be read alongside example.c; we mirror the prototype faithfully
/// (taking `&[String]` args + `Options`-shaped flags) without wiring
/// in the side-effect param mutation since `intparam`/`strparam`/
/// `arrparam` aren't allocated in the static-link path.
/// C signature: `static int bin_example(char *nam, char **args,
///                                       Options ops, int func)`.
/// `ops` is a 256-entry bitmask indexed by ASCII char (passed by
/// the C builtin framework after parsing the option spec). Rust
/// port takes `(nam: &str, args: &[&str], ops: &[bool; 256])`
/// matching the C body's `OPT_ISSET(ops, c)` reads exactly.
pub fn bin_example(nam: &str, args: &[&str], ops: &[bool; 256]) -> i32 {  // c:42
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    // c:46 — `printf("Options: ");`
    let _ = write!(stdout, "Options: ");
    // c:47-49 — `for (c = 32; ++c < 128;) if (OPT_ISSET(ops,c)) putchar(c);`
    for c in 33u8..128u8 {
        if ops[c as usize] {                                             // c:48 OPT_ISSET
            let _ = write!(stdout, "{}", c as char);                     // c:49 putchar
        }
    }
    // c:50 — `printf("\nArguments:");`
    let _ = write!(stdout, "\nArguments:");
    // c:51-54 — `for (; *args; i++, args++) { putchar(' '); fputs(*args, stdout); }`
    for a in args {                                                      // c:51
        let _ = write!(stdout, " {}", a);                                // c:52-53
    }
    // c:55 — `printf("\nName: %s\n", nam);`
    let _ = writeln!(stdout, "\nName: {}", nam);
    // c:57 — `printf("\nInteger Parameter: %ld\n", intparam);`
    // intparam/strparam/arrparam are example-module file-statics
    // that are never written in zshrs (module never loaded).
    let _ = writeln!(stdout, "Integer Parameter: 0");                    // c:57-60
    let _ = writeln!(stdout, "String Parameter: ");                      // c:61
    let _ = writeln!(stdout, "Array Parameter:");                        // c:62
    0                                                                    // c:72
}

/// Port of `cond_p_len()` from `Src/Modules/example.c:80`. The
/// demo `-len` cond op: with one arg, true iff the string is empty;
/// with two args, true iff the string's length equals the second
/// arg's integer value.
///
/// C signature: `static int cond_p_len(char **a, int id)`.
/// `id` is the cond-op id (ignored — only one `-len` op exists).
pub fn cond_p_len(a: &[&str], _id: i32) -> i32 {                         // c:80
    // c:83 — `s1 = cond_str(a, 0, 0);`
    let s1 = if a.is_empty() { "" } else { a[0] };
    if a.len() >= 2 {                                                    // c:85 a[1]
        // c:86 — `v = cond_val(a, 1);`
        let v: i64 = a[1].parse().unwrap_or(0);
        // c:88 — `return strlen(s1) == v;`
        if s1.chars().count() as i64 == v { 1 } else { 0 }
    } else {                                                             // c:89
        // c:90 — `return !s1[0];`
        if s1.is_empty() { 1 } else { 0 }
    }
}

/// Port of `cond_i_ex()` from `Src/Modules/example.c:95`. The demo
/// `-ex` infix cond op: true iff `s1 ++ s2 == "example"`.
///
/// C signature: `static int cond_i_ex(char **a, int id)`.
pub fn cond_i_ex(a: &[&str], _id: i32) -> i32 {                          // c:95
    // c:99-100 — `s1 = cond_str(a, 0, 0); s2 = cond_str(a, 1, 0);`
    let s1 = if a.is_empty() { "" } else { a[0] };
    let s2 = if a.len() < 2 { "" } else { a[1] };
    // c:102 — `return !strcmp("example", dyncat(s1, s2));`
    let mut combined = String::with_capacity(s1.len() + s2.len());
    combined.push_str(s1);
    combined.push_str(s2);
    if combined == "example" { 1 } else { 0 }
}

/// Port of `math_sum()` from `Src/Modules/example.c:104`. The demo
/// `sum(...)` math fn: variadic numeric sum. Promotes integer
/// running total to float on the first float arg (C's `f` flag).
///
/// C signature: `static mnumber math_sum(char *name, int argc,
///                                        mnumber *argv, int id)`.
pub fn math_sum(_name: &str, argc: i32, argv: &[crate::ported::math::Mnumber], _id: i32)
    -> crate::ported::math::Mnumber                                      // c:104
{
    use crate::ported::math::{Mnumber, MN_INTEGER, MN_FLOAT};
    let mut ret = Mnumber::default();
    let mut f = 0i32;                                                    // c:108 int f = 0;
    ret.l = 0;                                                           // c:110 ret.u.l = 0;
    let mut i = 0;
    let mut argc = argc;
    while argc > 0 {                                                     // c:111 while (argc--)
        argc -= 1;
        if argv[i].type_ == MN_INTEGER {                                 // c:112
            if f != 0 {                                                  // c:113
                ret.d += argv[i].l as f64;                               // c:114
            } else {                                                     // c:115
                ret.l += argv[i].l;                                      // c:116
            }
        } else {                                                         // c:117
            if f != 0 {                                                  // c:118
                ret.d += argv[i].d;                                      // c:119
            } else {                                                     // c:120
                ret.d = (ret.l as f64) + argv[i].d;                      // c:121-122
                f = 1;                                                   // c:123
            }
        }
        i += 1;                                                          // c:127 argv++
    }
    ret.type_ = if f != 0 { MN_FLOAT } else { MN_INTEGER };              // c:130
    ret                                                                  // c:132
}

/// Port of `math_length()` from `Src/Modules/example.c:133`. The
/// demo `length("...")` math fn: returns the string length.
///
/// C signature: `static mnumber math_length(char *name, char *arg,
///                                            int id)`.
pub fn math_length(_name: &str, arg: &str, _id: i32)
    -> crate::ported::math::Mnumber                                      // c:133
{
    use crate::ported::math::{Mnumber, MN_INTEGER};
    Mnumber {
        type_: MN_INTEGER,                                               // c:138
        l: arg.len() as i64,                                             // c:139 strlen(arg)
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
pub fn ex_wrapper(_prog: i32, _w: i32, name: &str) -> i32 {              // c:145
    // c:147 — `if (strncmp(name, "example", 7)) return 1;`
    if !name.starts_with("example") {
        return 1;
    }
    // c:149-153 — save opts[GLOBDOTS], set to 1, runshfunc(prog, w, name),
    // restore. Static-link path: never loaded, so the runshfunc call
    // is unwired; return 0 (wrapped + ran successfully).
    0                                                                    // c:155
}
