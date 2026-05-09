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
pub fn bin_example(args: &[&str]) -> i32 {                               // c:42
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    // C: `printf("Options: ");` (c:48) — flag dump skipped (zshrs
    // doesn't model the C `Options ops` flag bag at this entry).
    let _ = write!(stdout, "Options: \n");                               // c:48-52
    let _ = write!(stdout, "Arguments:");                                // c:53
    for a in args {                                                      // c:54
        let _ = write!(stdout, " {}", a);                                // c:56
    }
    let _ = writeln!(stdout, "\nName: example");                         // c:58 nam
    let _ = writeln!(stdout, "Integer Parameter: 0");                    // c:60-63
    let _ = writeln!(stdout, "String Parameter: ");                      // c:64
    let _ = writeln!(stdout, "Array Parameter:");                        // c:65
    0                                                                    // c:73
}

/// Port of `cond_p_len()` from `Src/Modules/example.c:80`. The
/// demo `-len` cond op: with one arg, true iff the string is empty;
/// with two args, true iff the string's length equals the second
/// arg's integer value.
///
/// C signature: `static int cond_p_len(char **a, int id)`.
pub fn cond_p_len(a: &[&str]) -> bool {                                  // c:80
    if a.is_empty() { return true; }                                     // c:84
    if a.len() >= 2 {                                                    // c:84
        let s1 = a[0];
        let v: i64 = a[1].parse().unwrap_or(0);                          // c:85 cond_val
        s1.chars().count() as i64 == v                                   // c:87
    } else {                                                             // c:88
        a[0].is_empty()                                                  // c:89 !s1[0]
    }
}

/// Port of `cond_i_ex()` from `Src/Modules/example.c:95`. The demo
/// `-ex` infix cond op: true iff `s1 ++ s2 == "example"`.
pub fn cond_i_ex(a: &[&str]) -> bool {                                   // c:95
    if a.len() < 2 { return false; }
    let s1 = a[0];
    let s2 = a[1];
    let mut combined = String::with_capacity(s1.len() + s2.len());
    combined.push_str(s1);
    combined.push_str(s2);
    combined == "example"                                                // c:101
}

/// Port of `math_sum()` from `Src/Modules/example.c:104`. The demo
/// `sum(...)` math fn: variadic numeric sum. Promotes integer
/// running total to float on first float arg (C's `f` flag).
///
/// Returns the sum as f64 — zshrs's math layer carries an mnumber
/// equivalent (`f64` + integer-tag); the demo only needs to compute
/// the value, not produce an mnumber here.
pub fn math_sum(args: &[f64]) -> f64 {                                   // c:104
    // C tracks integer vs float separately; Rust collapses to f64.
    let mut s = 0.0;                                                     // c:111
    for &a in args {                                                     // c:112
        s += a;                                                          // c:113-124
    }
    s                                                                    // c:128
}

/// Port of `math_length()` from `Src/Modules/example.c:133`. The
/// demo `length("...")` math fn: returns the string length.
pub fn math_length(arg: &str) -> i64 {                                   // c:133
    arg.chars().count() as i64                                            // c:139 strlen
}

/// Port of `ex_wrapper()` from `Src/Modules/example.c:145`. The
/// per-function wrapper hook: when the function name starts with
/// "example", set `GLOBDOTS` for the duration of the call (so
/// glob results include hidden dotfiles), then restore.
///
/// C signature: `static int ex_wrapper(Eprog prog, FuncWrap w, char *name)`.
/// Returns 1 to skip wrapping (name doesn't match), 0 after running.
pub fn ex_wrapper(name: &str) -> i32 {                                   // c:145
    if !name.starts_with("example") { return 1; }                        // c:147
    // C: save GLOBDOTS, set to 1, run shfunc, restore.
    // Static-link path: example module is never loaded, so the
    // wrapper is never installed. Return 0 (wrapped + ran).
    0                                                                    // c:155
}
