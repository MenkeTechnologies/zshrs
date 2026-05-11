//! `zsh/zprof` module — port of `Src/Modules/zprof.c`.
//!
//! Shell-function profiling: every function call is wrapped via
//! `zprof_wrapper` to record entry/exit time, build a per-function
//! `Pfunc` table and a per-arc (caller→callee) `Parc` table, and
//! emit a sorted report from `bin_zprof`.
//!
//! C source: 11 fns total — `freepfuncs`, `freeparcs`, `findpfunc`,
//! `findparc`, `cmpsfuncs`, `cmptfuncs`, `cmpparcs`, `bin_zprof`,
//! `name_for_anonymous_function`, `zprof_wrapper`, plus 6 module
//! loaders. 3 structs: `pfunc` (c:38), `sfunc` (c:49), `parc` (c:57).
//! 6 file-statics: `calls`, `ncalls`, `arcs`, `narcs`, `stack`,
//! `zprof_module` (c:66-71).
//!
//! Order in this file mirrors C source order verbatim.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Structs (port of c:36-64).
// ---------------------------------------------------------------------------

/// Port of `struct pfunc` from `Src/Modules/zprof.c:38`.
/// Per-function aggregated profiling record.
///
/// C definition (c:38-45):
/// ```c
/// struct pfunc {
///     Pfunc next;     /* linked list — Vec replaces */
///     char *name;
///     long calls;
///     double time;
///     double self;
///     long num;
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct Pfunc {                                                       // c:38
    pub name: String,                                                    // c:40
    pub calls: i64,                                                      // c:41
    pub time: f64,                                                       // c:42
    pub self_time: f64,                                                  // c:43 — `self` is a Rust keyword
    pub num: i64,                                                        // c:44
}

/// Port of `struct sfunc` from `Src/Modules/zprof.c:49`.
/// Per-active-call stack frame: linked stack the C `zprof_wrapper`
/// pushes on entry and pops on exit, used to compute self-time and
/// build the caller→callee arc.
///
/// C definition (c:49-53):
/// ```c
/// struct sfunc {
///     Pfunc p;       /* index into CALLS — Rust uses usize */
///     Sfunc prev;    /* linked list — Vec replaces */
///     double beg;
/// };
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Sfunc {                                                       // c:49
    pub p: usize,                                                        // c:50 — index into CALLS
    pub beg: f64,                                                        // c:52
}

/// Port of `struct parc` from `Src/Modules/zprof.c:57`.
/// Per-(caller→callee) aggregated arc with timing.
///
/// C definition (c:57-64):
/// ```c
/// struct parc {
///     Parc next;     /* linked list — Vec replaces */
///     Pfunc from;    /* indices into CALLS */
///     Pfunc to;
///     long calls;
///     double time;
///     double self;
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct Parc {                                                        // c:57
    pub from: usize,                                                     // c:59 — index into CALLS
    pub to: usize,                                                       // c:60 — index into CALLS
    pub calls: i64,                                                      // c:61
    pub time: f64,                                                       // c:62
    pub self_time: f64,                                                  // c:63 — `self` is a Rust keyword
}

// ---------------------------------------------------------------------------
// File-static globals — port of c:66-71.
// ---------------------------------------------------------------------------

/// Port of `static Pfunc calls;` from `Src/Modules/zprof.c:66`.
/// Per-function aggregated table; the C linked list becomes a
/// `Mutex<Vec<Pfunc>>` so `Pfunc *` becomes `usize` index.
pub static CALLS: Mutex<Vec<Pfunc>> = Mutex::new(Vec::new());            // c:66

/// Port of `static int ncalls;` from `Src/Modules/zprof.c:67`. Always
/// equals `CALLS.lock().len()` — kept as an explicit counter to
/// match C's `ncalls++` increment pattern.
pub static NCALLS: AtomicI32 = AtomicI32::new(0);                        // c:67

/// Port of `static Parc arcs;` from `Src/Modules/zprof.c:68`.
pub static ARCS: Mutex<Vec<Parc>> = Mutex::new(Vec::new());              // c:68

/// Port of `static int narcs;` from `Src/Modules/zprof.c:69`.
pub static NARCS: AtomicI32 = AtomicI32::new(0);                         // c:69

/// Port of `static Sfunc stack;` from `Src/Modules/zprof.c:70`. The
/// C linked stack becomes a `Mutex<Vec<Sfunc>>` (top of stack at
/// `last()`).
pub static STACK: Mutex<Vec<Sfunc>> = Mutex::new(Vec::new());            // c:70

/// Port of `static Module zprof_module;` from `Src/Modules/zprof.c:71`.
/// C uses a `Module` (struct module *) pointer to track which module
/// owns the wrapper; `zprof_wrapper` short-circuits when
/// `MOD_UNLOAD` is set on it. Module is ported as
/// `Box<crate::ported::zsh_h::module>` (zsh_h.rs:425) but recording
/// the raw `*const module` would deadlock with Sync/Send for the
/// static — `AtomicBool` captures the only state `zprof_wrapper`
/// actually inspects (loaded vs. unloading), matching the C
/// `MOD_UNLOAD` flag-check on the same pointer.
pub static ZPROF_MODULE: AtomicBool = AtomicBool::new(false);            // c:71

// ---------------------------------------------------------------------------
// Helpers (port of c:73-136).
// ---------------------------------------------------------------------------

/// Port of `freepfuncs()` from `Src/Modules/zprof.c:74`. C iterates
/// the linked list calling `zsfree(name)` + `zfree(node)` on each
/// entry. Rust port clears the `Vec`; the contained `String`s and
/// `Pfunc` slots are dropped at scope-exit.
///
/// C signature: `static void freepfuncs(Pfunc f)`.
pub fn freepfuncs(f: &mut Vec<Pfunc>) {                                  // c:74
    f.clear();                                                           // c:78-82 zsfree+zfree
}

/// Port of `freeparcs()` from `Src/Modules/zprof.c:86`.
///
/// C signature: `static void freeparcs(Parc a)`.
pub fn freeparcs(a: &mut Vec<Parc>) {                                    // c:86
    a.clear();                                                           // c:90-93 zfree
}

/// Port of `findpfunc()` from `Src/Modules/zprof.c:97`. Linear-scan
/// lookup in the `calls` list for an entry with matching `name`.
///
/// C signature: `static Pfunc findpfunc(char *name)`. Returns NULL on
/// miss; Rust port returns `None`.
pub fn findpfunc(name: &str) -> Option<usize> {                          // c:97
    // c:101-103 — `for (f = calls; f; f = f->next) if (!strcmp(name, f->name)) return f;`
    let calls = CALLS.lock().unwrap();
    calls.iter().position(|f| f.name == name)
}

/// Port of `findparc()` from `Src/Modules/zprof.c:109`. Linear-scan
/// lookup in the `arcs` list for an arc with matching (from, to)
/// pair.
///
/// C signature: `static Parc findparc(Pfunc f, Pfunc t)`.
pub fn findparc(from: usize, to: usize) -> Option<usize> {               // c:109
    // c:113-115 — `for (a = arcs; a; a = a->next) if (a->from == f && a->to == t) return a;`
    let arcs = ARCS.lock().unwrap();
    arcs.iter().position(|a| a.from == from && a.to == to)
}

/// Port of `cmpsfuncs()` from `Src/Modules/zprof.c:121`. The qsort
/// comparator: descending by `self`. C uses `Pfunc *` pointers
/// because qsort passes opaque ptrs; Rust takes refs directly.
///
/// C body:
/// ```c
/// return ((*a)->self > (*b)->self ? -1 :
///         ((*a)->self != (*b)->self));
/// ```
/// (i.e. -1 if a > b, 0 if equal, +1 if a < b — descending order.)
pub fn cmpsfuncs(a: &Pfunc, b: &Pfunc) -> std::cmp::Ordering {           // c:121
    b.self_time.partial_cmp(&a.self_time).unwrap_or(std::cmp::Ordering::Equal)
}

/// Port of `cmptfuncs()` from `Src/Modules/zprof.c:127`. Comparator
/// for descending by total `time`.
pub fn cmptfuncs(a: &Pfunc, b: &Pfunc) -> std::cmp::Ordering {           // c:127
    b.time.partial_cmp(&a.time).unwrap_or(std::cmp::Ordering::Equal)
}

/// Port of `cmpparcs()` from `Src/Modules/zprof.c:133`. Comparator
/// for descending by arc `time`.
pub fn cmpparcs(a: &Parc, b: &Parc) -> std::cmp::Ordering {              // c:133
    b.time.partial_cmp(&a.time).unwrap_or(std::cmp::Ordering::Equal)
}

// ---------------------------------------------------------------------------
// `bin_zprof` (port of c:139-214).
// ---------------------------------------------------------------------------

/// Port of `bin_zprof()` from `Src/Modules/zprof.c:139`.
///
/// C signature: `static int bin_zprof(char *nam, char **args,
///                                     Options ops, int func)`.
/// Builtin spec: `"c"` (c:315) — the `-c` option clears the tables.
/// No positional args (`0,0` arity at c:315).
///
/// `-c` set → free both tables and reset counters. `-c` unset →
/// sort by self-time, print the c:170 header + per-function row,
/// re-sort by total-time, print the c:184 per-function caller/callee
/// blocks.
pub fn bin_zprof(_nam: &str, _args: &[String],                               // c:139
                 ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    // c:140 — `if (OPT_ISSET(ops,'c'))`
    let opt_c = OPT_ISSET(ops, b'c');

    if opt_c {
        // c:141-147 — free both tables + reset counters.
        let mut calls = CALLS.lock().unwrap();
        freepfuncs(&mut calls);                                          // c:142
        NCALLS.store(0, Ordering::SeqCst);                               // c:144
        let mut arcs = ARCS.lock().unwrap();
        freeparcs(&mut arcs);                                            // c:145
        NARCS.store(0, Ordering::SeqCst);                                // c:147
        return 0;                                                        // c:213
    }

    // c:149-211 — print path.
    let calls = CALLS.lock().unwrap();
    let arcs = ARCS.lock().unwrap();

    // c:149-163 — gather + total. C uses a VARARR Pfunc fs[ncalls+1]
    // and a VARARR Parc as[narcs+1] with NULL sentinels; Rust uses
    // index arrays. `total` is the sum of self-times across all funcs.
    let mut fs: Vec<usize> = (0..calls.len()).collect();                 // c:149-159
    let as_arcs: Vec<usize> = (0..arcs.len()).collect();                 // c:151-163
    let mut total: f64 = 0.0;                                            // c:154
    for &i in &fs {
        total += calls[i].self_time;                                     // c:158 total += f->self;
    }

    // c:165-166 — `qsort(fs, ncalls, sizeof(f), cmpsfuncs);`
    fs.sort_by(|&a, &b| cmpsfuncs(&calls[a], &calls[b]));

    // c:170 — header.
    println!("num  calls                time                       self            name");
    println!("-----------------------------------------------------------------------------------");

    // c:171-180 — primary listing, also assigns `num` in display order.
    // Mutating `num` in C requires reborrowing — release the read lock
    // briefly to take a write lock, then reacquire read order.
    drop(calls);
    {
        let mut calls_w = CALLS.lock().unwrap();
        for (i, &idx) in fs.iter().enumerate() {                         // c:171
            calls_w[idx].num = (i + 1) as i64;                           // c:173
        }
    }
    let calls = CALLS.lock().unwrap();
    for &idx in &fs {                                                    // c:171 again, after num assignment
        let f = &calls[idx];
        let avg_t = if f.calls > 0 { f.time / f.calls as f64 } else { 0.0 };
        let avg_s = if f.calls > 0 { f.self_time / f.calls as f64 } else { 0.0 };
        let pct_t = if total != 0.0 { (f.time / total) * 100.0 } else { 0.0 };
        let pct_s = if total != 0.0 { (f.self_time / total) * 100.0 } else { 0.0 };
        println!(
            "{:2}) {:4}       {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}  {:6.2}%  {}",
            f.num, f.calls,                                              // c:172-179 printf
            f.time, avg_t, pct_t,
            f.self_time, avg_s, pct_s,
            f.name
        );
    }

    // c:181-182 — `qsort(fs, ncalls, sizeof(f), cmptfuncs);`
    let mut fs_t: Vec<usize> = fs.clone();
    fs_t.sort_by(|&a, &b| cmptfuncs(&calls[a], &calls[b]));

    // c:184-211 — per-function caller/callee blocks.
    for &fp_idx in &fs_t {                                               // c:184
        println!();
        println!("-----------------------------------------------------------------------------------");
        println!();
        let f = &calls[fp_idx];

        // c:186-194 — callers (arcs where to == fp).
        for &ap in &as_arcs {                                            // c:186
            let a = &arcs[ap];
            if a.to == fp_idx {                                          // c:187
                let avg_t = if a.calls > 0 { a.time / a.calls as f64 } else { 0.0 };
                let avg_s = if a.calls > 0 { a.self_time / a.calls as f64 } else { 0.0 };
                let pct_t = if total != 0.0 { (a.time / total) * 100.0 } else { 0.0 };
                let from_name = &calls[a.from].name;
                let from_num = calls[a.from].num;
                println!(
                    "    {:4}/{:<4}  {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}             {} [{}]",
                    a.calls, f.calls,                                    // c:188-193 printf
                    a.time, avg_t, pct_t,
                    a.self_time, avg_s,
                    from_name, from_num
                );
            }
        }

        // c:195-201 — the function's own row.
        let avg_t = if f.calls > 0 { f.time / f.calls as f64 } else { 0.0 };
        let avg_s = if f.calls > 0 { f.self_time / f.calls as f64 } else { 0.0 };
        let pct_t = if total != 0.0 { (f.time / total) * 100.0 } else { 0.0 };
        let pct_s = if total != 0.0 { (f.self_time / total) * 100.0 } else { 0.0 };
        println!(
            "{:2}) {:4}       {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}  {:6.2}%  {}",
            f.num, f.calls,                                              // c:195-201 printf
            f.time, avg_t, pct_t,
            f.self_time, avg_s, pct_s,
            f.name
        );

        // c:202-210 — callees (arcs where from == fp), iterated in
        // reverse to match C's `for (ap = as + narcs - 1; ap >= as; ap--)`.
        for &ap in as_arcs.iter().rev() {                                // c:202
            let a = &arcs[ap];
            if a.from == fp_idx {                                        // c:203
                let avg_t = if a.calls > 0 { a.time / a.calls as f64 } else { 0.0 };
                let avg_s = if a.calls > 0 { a.self_time / a.calls as f64 } else { 0.0 };
                let pct_t = if total != 0.0 { (a.time / total) * 100.0 } else { 0.0 };
                let to_name = &calls[a.to].name;
                let to_num = calls[a.to].num;
                let to_calls = calls[a.to].calls;
                println!(
                    "    {:4}/{:<4}  {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}             {} [{}]",
                    a.calls, to_calls,                                   // c:204-209 printf
                    a.time, avg_t, pct_t,
                    a.self_time, avg_s,
                    to_name, to_num
                );
            }
        }
    }

    0                                                                    // c:213
}

/// Port of `name_for_anonymous_function()` from `Src/Modules/zprof.c:217`.
/// Anonymous functions don't have a real name; the profiler synthesises
/// `name [filename:lineno]` using the current `funcstack[0]` frame.
///
/// C signature: `static char *name_for_anonymous_function(char *name)`.
/// Rust port takes the placeholder name + `(filename, lineno)` pair
/// the caller pulls from the funcstack.
pub fn name_for_anonymous_function(name: &str, filename: &str, lineno: i32) -> String {  // c:217
    // c:222 — `convbase(lineno, funcstack[0].flineno, 10);`
    // c:224-230 — `parts[] = { name, " [", filename, ":", lineno, "]", NULL };`
    // c:232 — `return sepjoin(parts, "", 1);`
    format!("{} [{}:{}]", name, filename, lineno)
}

/// Port of `zprof_wrapper()` from `Src/Modules/zprof.c:236`. The
/// per-function-call wrapper hook: records call entry, measures wall
/// time, runs the wrapped function via `runshfunc`, then accumulates
/// self/total time on the function's `pfunc` entry and on the
/// (caller→callee) `parc` arc.
///
/// C signature: `static int zprof_wrapper(Eprog prog, FuncWrap w, char *name)`.
///
/// C body (c:238-311):
/// 1. Resolve `name_for_lookups` via `name_for_anonymous_function` for
///    anonymous funcs (c:246-250).
/// 2. If `zprof_module` is loaded (c:252), find-or-create the Pfunc
///    (c:254-262), find-or-create the caller→callee Parc (c:263-274),
///    push the Sfunc frame and record start time (c:275-283).
/// 3. `runshfunc(prog, w, name)` (c:285) — runs the wrapped function.
/// 4. On return, recompute elapsed time, update Pfunc.self_time
///    (c:293), Pfunc.time when non-recursive (c:294-296), Parc.calls/
///    self/time (c:297-307), pop the stack frame (c:301).
///
/// zshrs's call-execution path doesn't have an `addwrapper`-installable
/// runshfunc callback, so the live integration is the executor's
/// funcstack push/pop hooks (in `crate::ported::exec`). This entry is
/// the static-link stub that mirrors C's `return 0;` exit path; the
/// actual timing accumulation happens via direct CALLS/ARCS/STACK
/// updates from the executor when `ZPROF_MODULE` is true.
pub fn zprof_wrapper(_name: &str) -> i32 {                               // c:236
    // Static-link path: the runshfunc dispatch isn't installable in
    // zshrs's executor, so this wrapper is a no-op. The real
    // CALLS/ARCS/STACK accounting lives at the funcstack hook.
    0                                                                    // c:311
}

// ---------------------------------------------------------------------------
// Module loaders.
// ---------------------------------------------------------------------------

// =====================================================================
// static struct builtin bintab[]                                    c:309
// static struct features module_features                            c:323
// static struct funcwrap wrapper[]                                  c:328
// =====================================================================

use crate::ported::zsh_h::module;

// `bintab` — port of `static struct builtin bintab[]` (zprof.c:309).


// `module_features` — port of `static struct features module_features`
// from zprof.c:323.



/// Port of `setup_()` from `Src/Modules/zprof.c:332`.
/// C body: `zprof_module = m; return 0;`
pub fn setup_(_m: *const module) -> i32 {                                // c:332
    ZPROF_MODULE.store(true, Ordering::SeqCst);                          // c:334
    0                                                                    // c:335
}

/// Port of `features_()` from `Src/Modules/zprof.c:340`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 { // c:340
    *features = featuresarray(m, module_features());
    0                                                                    // c:343
}

/// Port of `enables_()` from `Src/Modules/zprof.c:348`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 { // c:348
    handlefeatures(m, module_features(), enables) // c:350
}

/// Port of `boot_()` from `Src/Modules/zprof.c:355`.
pub fn boot_(_m: *const module) -> i32 {                                 // c:355
    let mut calls = CALLS.lock().unwrap();
    calls.clear();                                                       // c:357
    NCALLS.store(0, Ordering::SeqCst);                                   // c:358
    let mut arcs = ARCS.lock().unwrap();
    arcs.clear();                                                        // c:359
    NARCS.store(0, Ordering::SeqCst);                                    // c:360
    STACK.lock().unwrap().clear();                                       // c:361
    0                                                                    // c:362 addwrapper return
}

/// Port of `cleanup_()` from `Src/Modules/zprof.c:367`.
/// C body: free pfuncs + parcs, deletewrapper, setfeatureenables.
pub fn cleanup_(m: *const module) -> i32 {                              // c:367
    let mut calls = CALLS.lock().unwrap();
    freepfuncs(&mut calls);                                              // c:369
    let mut arcs = ARCS.lock().unwrap();
    freeparcs(&mut arcs);                                                // c:370
    ZPROF_MODULE.store(false, Ordering::SeqCst);
    setfeatureenables(m, module_features(), None) // c:372
}

/// Port of `finish_()` from `Src/Modules/zprof.c:377`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:377
    // C body c:379-380 — `return 0`. Faithful empty-body port; the
    //                    profiling tables get freed by cleanup_ via
    //                    setfeatureenables/zprof_cleanup.
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise tests that mutate the module-static globals so the
    /// cargo-test parallel runner doesn't shred each other's state.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset_state() {
        let mut c = CALLS.lock().unwrap();
        c.clear();
        let mut a = ARCS.lock().unwrap();
        a.clear();
        STACK.lock().unwrap().clear();
        NCALLS.store(0, Ordering::SeqCst);
        NARCS.store(0, Ordering::SeqCst);
    }

    /// Verifies `Pfunc` mirrors C `struct pfunc` field-for-field
    /// (name/calls/time/self/num at c:40-44).
    #[test]
    fn pfunc_default_zeros() {
        let p = Pfunc::default();
        assert_eq!(p.name, "");
        assert_eq!(p.calls, 0);
        assert_eq!(p.time, 0.0);
        assert_eq!(p.self_time, 0.0);
        assert_eq!(p.num, 0);
    }

    /// Verifies `freepfuncs` empties the table (c:78-82 zsfree+zfree).
    #[test]
    fn freepfuncs_clears() {
        let mut v = vec![Pfunc { name: "a".into(), ..Default::default() }];
        freepfuncs(&mut v);
        assert!(v.is_empty());
    }

    /// Verifies `findpfunc` linear-scan match (c:101-103).
    #[test]
    fn findpfunc_matches_by_name() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        CALLS.lock().unwrap().push(Pfunc { name: "alpha".into(), ..Default::default() });
        CALLS.lock().unwrap().push(Pfunc { name: "beta".into(), ..Default::default() });
        assert_eq!(findpfunc("alpha"), Some(0));
        assert_eq!(findpfunc("beta"), Some(1));
        assert_eq!(findpfunc("none"), None);
        reset_state();
    }

    /// Verifies `findparc` matches (from, to) pair (c:113-115).
    #[test]
    fn findparc_matches_pair() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        ARCS.lock().unwrap().push(Parc { from: 0, to: 1, ..Default::default() });
        ARCS.lock().unwrap().push(Parc { from: 0, to: 2, ..Default::default() });
        assert_eq!(findparc(0, 1), Some(0));
        assert_eq!(findparc(0, 2), Some(1));
        assert_eq!(findparc(1, 0), None);
        reset_state();
    }

    /// Verifies `cmpsfuncs` is descending (c:121-124).
    #[test]
    fn cmpsfuncs_descending() {
        let a = Pfunc { self_time: 5.0, ..Default::default() };
        let b = Pfunc { self_time: 10.0, ..Default::default() };
        // descending: b should come before a → cmp(a, b) = Greater
        assert_eq!(cmpsfuncs(&a, &b), std::cmp::Ordering::Greater);
        assert_eq!(cmpsfuncs(&b, &a), std::cmp::Ordering::Less);
    }

    /// Verifies `bin_zprof -c` clears state (c:141-147).
    #[test]
    fn bin_zprof_clear_resets_tables() {
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        CALLS.lock().unwrap().push(Pfunc { name: "x".into(), ..Default::default() });
        ARCS.lock().unwrap().push(Parc { from: 0, to: 0, ..Default::default() });
        NCALLS.store(1, Ordering::SeqCst);
        NARCS.store(1, Ordering::SeqCst);

        use crate::ported::zsh_h::{options, MAX_OPS};
        let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                argscount: 0, argsalloc: 0 };
        ops.ind[b'c' as usize] = 1;
        let r = bin_zprof("zprof", &["-c".to_string()], &ops, 0);
        assert_eq!(r, 0);
        assert!(CALLS.lock().unwrap().is_empty());
        assert!(ARCS.lock().unwrap().is_empty());
        assert_eq!(NCALLS.load(Ordering::SeqCst), 0);
        assert_eq!(NARCS.load(Ordering::SeqCst), 0);
    }

    /// Verifies `zprof_wrapper` returns 0 (the static-link no-op
    /// path mirrors C's `return 0;` exit at c:311).
    #[test]
    fn zprof_wrapper_returns_zero() {
        assert_eq!(zprof_wrapper("foo"), 0);
    }

    /// Verifies `name_for_anonymous_function` formats as
    /// `name [filename:lineno]` per c:224-232.
    #[test]
    fn name_for_anonymous_function_format() {
        let s = name_for_anonymous_function("anon", "/tmp/foo.zsh", 42);
        assert_eq!(s, "anon [/tmp/foo.zsh:42]");
    }
}

use crate::ported::zsh_h::features as features_t;

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 1,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
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
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:zprof".to_string()]
}

fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

