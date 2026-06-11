//! `zsh/zprof` module — port of `Src/Modules/zprof.c`.
//!
//! Shell-function profiling: every function call is wrapped via
//! `zprof_wrapper` to record entry/exit time, build a per-function
//! `Pfunc` table and a per-arc (caller→callee) `Parc` table, and
//! emit a sorted report from `bin_zprof`.
//!
//! C source: 11 ported total — `freepfuncs`, `freeparcs`, `findpfunc`,
//! `findparc`, `cmpsfuncs`, `cmptfuncs`, `cmpparcs`, `bin_zprof`,
//! `name_for_anonymous_function`, `zprof_wrapper`, plus 6 module
//! loaders. 3 structs: `pfunc` (c:38), `sfunc` (c:49), `parc` (c:57).
//! 6 file-statics: `calls`, `ncalls`, `arcs`, `narcs`, `stack`,
//! `zprof_module` (c:66-71).
//!
//! Order in this file mirrors C source order verbatim.

use crate::ported::compat::zgettime_monotonic_if_available;
use crate::ported::mem::ztrdup;
use crate::ported::modules::parameter::FUNCSTACK;
use crate::ported::zsh_h::{eprog, features, funcwrap, module, options, MAX_OPS, OPT_ISSET};
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
pub struct Pfunc {
    // c:38
    pub name: String,   // c:40
    pub calls: i64,     // c:41
    pub time: f64,      // c:42
    pub self_time: f64, // c:43 — `self` is a Rust keyword
    pub num: i64,       // c:44
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
pub struct Sfunc {
    // c:49
    pub p: usize, // c:50 — index into CALLS
    pub beg: f64, // c:52
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
pub struct Parc {
    // c:57
    pub from: usize,    // c:59 — index into CALLS
    pub to: usize,      // c:60 — index into CALLS
    pub calls: i64,     // c:61
    pub time: f64,      // c:62
    pub self_time: f64, // c:63 — `self` is a Rust keyword
}

// ---------------------------------------------------------------------------
// Helpers (port of c:73-136).
// ---------------------------------------------------------------------------

/// Port of `freepfuncs(Pfunc f)` from `Src/Modules/zprof.c:74`. C iterates
/// the linked list calling `zsfree(name)` + `zfree(node)` on each
/// entry. Rust port clears the `Vec`; the contained `String`s and
/// `Pfunc` slots are dropped at scope-exit.
///
/// C signature: `static void freepfuncs(Pfunc f)`.
pub fn freepfuncs(f: &mut Vec<Pfunc>) {
    // c:74
    f.clear(); // c:86-82 zsfree+zfree
}

/// Port of `freeparcs(Parc a)` from `Src/Modules/zprof.c:86`.
///
/// C signature: `static void freeparcs(Parc a)`.
pub fn freeparcs(a: &mut Vec<Parc>) {
    // c:86
    a.clear(); // c:97-93 zfree
}

/// Port of `findpfunc(char *name)` from `Src/Modules/zprof.c:97`. Linear-scan
/// lookup in the `calls` list for an entry with matching `name`.
///
/// C signature: `static Pfunc findpfunc(char *name)`. Returns NULL on
/// miss; Rust port returns `None`.
pub fn findpfunc(name: &str) -> Option<usize> {
    // c:97
    // c:109-103 — `for (f = calls; f; f = f->next) if (!strcmp(name, f->name)) return f;`
    let calls = CALLS.lock().unwrap();
    calls.iter().position(|f| f.name == name)
}

/// Port of `findparc(Pfunc f, Pfunc t)` from `Src/Modules/zprof.c:109`. Linear-scan
/// lookup in the `arcs` list for an arc with matching (from, to)
/// pair.
///
/// C signature: `static Parc findparc(Pfunc f, Pfunc t)`.
pub fn findparc(f: usize, t: usize) -> Option<usize> {
    // c:109
    // c:109-115 — `for (a = arcs; a; a = a->next) if (a->f == f && a->t == t) return a;`
    let arcs = ARCS.lock().unwrap();
    arcs.iter().position(|a| a.from == f && a.to == t)
}

/// Port of `cmpsfuncs(Pfunc *a, Pfunc *b)` from `Src/Modules/zprof.c:121`. The qsort
/// comparator: descending by `self`. C uses `Pfunc *` pointers
/// because qsort passes opaque ptrs; Rust takes refs directly.
///
/// C body:
/// ```c
/// return ((*a)->self > (*b)->self ? -1 :
///         ((*a)->self != (*b)->self));
/// ```
/// (i.e. -1 if a > b, 0 if equal, +1 if a < b — descending order.)
pub fn cmpsfuncs(a: &Pfunc, b: &Pfunc) -> std::cmp::Ordering {
    // c:121
    b.self_time
        .partial_cmp(&a.self_time)
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Port of `cmptfuncs(Pfunc *a, Pfunc *b)` from `Src/Modules/zprof.c:127`. Comparator
/// for descending by total `time`.
pub fn cmptfuncs(a: &Pfunc, b: &Pfunc) -> std::cmp::Ordering {
    // c:127
    b.time
        .partial_cmp(&a.time)
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Port of `cmpparcs(Parc *a, Parc *b)` from `Src/Modules/zprof.c:133`. Comparator
/// for descending by arc `time`.
pub fn cmpparcs(a: &Parc, b: &Parc) -> std::cmp::Ordering {
    // c:133
    b.time
        .partial_cmp(&a.time)
        .unwrap_or(std::cmp::Ordering::Equal)
}

// ---------------------------------------------------------------------------
// `bin_zprof` (port of c:139-214).
// ---------------------------------------------------------------------------

/// Port of `bin_zprof(UNUSED(char *nam), UNUSED(char **args), Options ops, UNUSED(int func))` from `Src/Modules/zprof.c:139`.
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
/// WARNING: param names don't match C — Rust=(_nam, _args, _func) vs C=(nam, args, ops, func)
pub fn bin_zprof(
    _nam: &str,
    _args: &[String], // c:139
    ops: &options,
    _func: i32,
) -> i32 {
    // c:140 — `if (OPT_ISSET(ops,'c'))`
    let opt_c = OPT_ISSET(ops, b'c');

    if opt_c {
        // c:141-147 — free both tables + reset counters.
        let mut calls = CALLS.lock().unwrap();
        freepfuncs(&mut calls); // c:142
        NCALLS.store(0, Ordering::SeqCst); // c:144
        let mut arcs = ARCS.lock().unwrap();
        freeparcs(&mut arcs); // c:145
        NARCS.store(0, Ordering::SeqCst); // c:147
        return 0; // c:213
    }

    // c:149-211 — print path.
    let calls = CALLS.lock().unwrap();
    let arcs = ARCS.lock().unwrap();

    // c:149-163 — gather + total. C uses a VARARR Pfunc fs[ncalls+1]
    // and a VARARR Parc as[narcs+1] with NULL sentinels; Rust uses
    // index arrays. `total` is the sum of self-times across all funcs.
    let mut fs: Vec<usize> = (0..calls.len()).collect(); // c:149-159
    let mut as_arcs: Vec<usize> = (0..arcs.len()).collect(); // c:151-163
    let mut total: f64 = 0.0; // c:154
    for &i in &fs {
        total += calls[i].self_time; // c:158 total += f->self;
    }

    // c:165-166 — `qsort(fs, ncalls, sizeof(f), cmpsfuncs);`
    fs.sort_by(|&a, &b| cmpsfuncs(&calls[a], &calls[b]));
    // c:167-168 — `qsort(as, narcs, sizeof(a), cmpparcs);`
    //   Prior port skipped this sort, so the per-function caller/callee
    //   blocks at c:184-211 listed arcs in chronological insertion order
    //   instead of descending-time order. With many callers, the listing
    //   buried the dominant time consumers under low-cost arcs, defeating
    //   zprof's "find the hot edges" purpose.
    as_arcs.sort_by(|&a, &b| cmpparcs(&arcs[a], &arcs[b]));

    // c:170 — header.
    println!("num  calls                time                       self            name");
    println!("-----------------------------------------------------------------------------------");

    // c:171-180 — primary listing, also assigns `num` in display order.
    // Mutating `num` in C requires reborrowing — release the read lock
    // briefly to take a write lock, then reacquire read order.
    drop(calls);
    {
        let mut calls_w = CALLS.lock().unwrap();
        for (i, &idx) in fs.iter().enumerate() {
            // c:171
            calls_w[idx].num = (i + 1) as i64; // c:173
        }
    }
    let calls = CALLS.lock().unwrap();
    for &idx in &fs {
        // c:171 again, after num assignment
        let f = &calls[idx];
        let avg_t = if f.calls > 0 {
            f.time / f.calls as f64
        } else {
            0.0
        };
        let avg_s = if f.calls > 0 {
            f.self_time / f.calls as f64
        } else {
            0.0
        };
        let pct_t = if total != 0.0 {
            (f.time / total) * 100.0
        } else {
            0.0
        };
        let pct_s = if total != 0.0 {
            (f.self_time / total) * 100.0
        } else {
            0.0
        };
        println!(
            "{:2}) {:4}       {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}  {:6.2}%  {}",
            f.num,
            f.calls, // c:172-179 printf
            f.time,
            avg_t,
            pct_t,
            f.self_time,
            avg_s,
            pct_s,
            f.name
        );
    }

    // c:181-182 — `qsort(fs, ncalls, sizeof(f), cmptfuncs);`
    let mut fs_t: Vec<usize> = fs.clone();
    fs_t.sort_by(|&a, &b| cmptfuncs(&calls[a], &calls[b]));

    // c:184-211 — per-function caller/callee blocks.
    for &fp_idx in &fs_t {
        // c:184
        println!();
        println!(
            "-----------------------------------------------------------------------------------"
        );
        println!();
        let f = &calls[fp_idx];

        // c:186-194 — callers (arcs where to == fp).
        for &ap in &as_arcs {
            // c:186
            let a = &arcs[ap];
            if a.to == fp_idx {
                // c:187
                let avg_t = if a.calls > 0 {
                    a.time / a.calls as f64
                } else {
                    0.0
                };
                let avg_s = if a.calls > 0 {
                    a.self_time / a.calls as f64
                } else {
                    0.0
                };
                let pct_t = if total != 0.0 {
                    (a.time / total) * 100.0
                } else {
                    0.0
                };
                let from_name = &calls[a.from].name;
                let from_num = calls[a.from].num;
                println!(
                    "    {:4}/{:<4}  {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}             {} [{}]",
                    a.calls,
                    f.calls, // c:188-193 printf
                    a.time,
                    avg_t,
                    pct_t,
                    a.self_time,
                    avg_s,
                    from_name,
                    from_num
                );
            }
        }

        // c:195-201 — the function's own row.
        let avg_t = if f.calls > 0 {
            f.time / f.calls as f64
        } else {
            0.0
        };
        let avg_s = if f.calls > 0 {
            f.self_time / f.calls as f64
        } else {
            0.0
        };
        let pct_t = if total != 0.0 {
            (f.time / total) * 100.0
        } else {
            0.0
        };
        let pct_s = if total != 0.0 {
            (f.self_time / total) * 100.0
        } else {
            0.0
        };
        println!(
            "{:2}) {:4}       {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}  {:6.2}%  {}",
            f.num,
            f.calls, // c:195-201 printf
            f.time,
            avg_t,
            pct_t,
            f.self_time,
            avg_s,
            pct_s,
            f.name
        );

        // c:202-210 — callees (arcs where from == fp), iterated in
        // reverse to match C's `for (ap = as + narcs - 1; ap >= as; ap--)`.
        for &ap in as_arcs.iter().rev() {
            // c:202
            let a = &arcs[ap];
            if a.from == fp_idx {
                // c:203
                let avg_t = if a.calls > 0 {
                    a.time / a.calls as f64
                } else {
                    0.0
                };
                let avg_s = if a.calls > 0 {
                    a.self_time / a.calls as f64
                } else {
                    0.0
                };
                let pct_t = if total != 0.0 {
                    (a.time / total) * 100.0
                } else {
                    0.0
                };
                let to_name = &calls[a.to].name;
                let to_num = calls[a.to].num;
                let to_calls = calls[a.to].calls;
                println!(
                    "    {:4}/{:<4}  {:8.2} {:8.2}  {:6.2}%  {:8.2} {:8.2}             {} [{}]",
                    a.calls,
                    to_calls, // c:204-209 printf
                    a.time,
                    avg_t,
                    pct_t,
                    a.self_time,
                    avg_s,
                    to_name,
                    to_num
                );
            }
        }
    }

    0 // c:217
}

/// Port of `name_for_anonymous_function(char *name)` from `Src/Modules/zprof.c:217`.
/// Anonymous functions don't have a real name; the profiler synthesises
/// `name [filename:lineno]` using the current `funcstack[0]` frame.
///
/// C signature: `static char *name_for_anonymous_function(char *name)`.
pub fn name_for_anonymous_function(name: &str) -> String {
    // c:217
    // c:219 — char lineno[DIGBUFSIZE];
    // c:220 — char *parts[7];
    // c:222 — convbase(lineno, funcstack[0].flineno, 10);
    let stack = FUNCSTACK.lock().expect("FUNCSTACK poisoned");
    let flineno = stack.first().map(|f| f.flineno).unwrap_or(0); // c:222
    let filename = stack
        .first()
        .and_then(|f| f.filename.clone())
        .unwrap_or_default(); // c:226
    drop(stack);
    let lineno_str = format!("{}", flineno); // c:222 convbase base=10
                                             // c:224-230 — parts[] = { name, " [", filename, ":", lineno, "]", NULL };
                                             // c:232 — return sepjoin(parts, "", 1);
    let parts = [name, " [", filename.as_str(), ":", lineno_str.as_str(), "]"];
    parts.concat() // c:232
}

/// Port of `zprof_wrapper(Eprog prog, FuncWrap w, char *name)` from `Src/Modules/zprof.c:236`. The
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
/// Port of `static int zprof_wrapper(Eprog prog, FuncWrap w, char *name)`
/// from `Src/Modules/zprof.c:236`.
///
/// ```c
/// static int
/// zprof_wrapper(Eprog prog, FuncWrap w, char *name)
/// {
///     int active = 0;
///     struct sfunc sf, *sp;
///     Pfunc f = NULL;
///     Parc a = NULL;
///     struct timespec ts;
///     double prev = 0, now;
///     char *name_for_lookups;
///     if (is_anonymous_function_name(name))
///         name_for_lookups = name_for_anonymous_function(name);
///     else
///         name_for_lookups = name;
///     if (zprof_module && !(zprof_module->node.flags & MOD_UNLOAD)) {
///         active = 1;
///         if (!(f = findpfunc(name_for_lookups))) { ... append calls ... }
///         if (stack) {
///             if (!(a = findparc(stack->p, f))) { ... append arcs ... }
///         }
///         sf.prev = stack; sf.p = f; stack = &sf;
///         f->calls++;
///         zgettime_monotonic_if_available(&ts);
///         sf.beg = prev = ms_now(ts);
///     }
///     runshfunc(prog, w, name);
///     if (active) {
///         if (zprof_module && !(zprof_module->node.flags & MOD_UNLOAD)) {
///             zgettime_monotonic_if_available(&ts);
///             now = ms_now(ts);
///             f->self += now - sf.beg;
///             for (sp = sf.prev; sp && sp->p != f; sp = sp->prev);
///             if (!sp) f->time += now - prev;
///             if (a) { a->calls++; a->self += now - sf.beg; }
///             stack = sf.prev;
///             if (stack) { stack->beg += now - prev;
///                          if (a) a->time += now - prev; }
///         } else stack = sf.prev;
///     }
///     return 0;
/// }
/// ```
#[allow(non_snake_case)]
pub fn zprof_wrapper(
    prog: *const eprog, // c:236
    w: *const funcwrap,
    name: &str,
    runshfunc: impl FnOnce(),
) -> i32 {
    let mut active: i32 = 0; // c:238
    let mut sf = Sfunc { p: 0, beg: 0.0 }; // c:239 struct sfunc sf
    let mut f: Option<usize> = None; // c:240 Pfunc f = NULL
    let mut a: Option<usize> = None; // c:241 Parc a = NULL
    let mut prev: f64 = 0.0; // c:243 double prev = 0

    // c:246-250 — resolve display name for anonymous functions.
    // `is_anonymous_function_name(name)` is `!strcmp(name, "(anon)")`
    // per Src/exec.c:5303-5306. ANONYMOUS_FUNCTION_NAME = "(anon)".
    let name_for_lookups: String = if name == "(anon)" {
        // c:246
        // `name_for_anonymous_function(name)` reads funcstack[0]
        // internally (S1 rule — signature matches C).
        name_for_anonymous_function(name) // c:247
    } else {
        // c:248
        name.to_string() // c:249
    };

    if ZPROF_MODULE.load(Ordering::SeqCst) {
        // c:252
        active = 1; // c:253
        f = findpfunc(&name_for_lookups); // c:254
        if f.is_none() {
            // c:254
            // c:255-261 — `f = zalloc(...); f->name = ztrdup(...); f->next = calls; calls = f; ncalls++;`
            let new_pfunc = Pfunc {
                // c:255
                name: ztrdup(&name_for_lookups), // c:256
                calls: 0,                        // c:257
                time: 0.0,                       // c:258 self/time = 0
                self_time: 0.0,                  // c:258
                num: 0,
            };
            let mut calls = CALLS.lock().unwrap();
            f = Some(calls.len()); // c:260 head-insert in C; Rust appends
            calls.push(new_pfunc); // c:260
            NCALLS.fetch_add(1, Ordering::SeqCst); // c:261
        }
        // c:263 — `if (stack)` — top-of-stack frame exists.
        let stack_top: Option<Sfunc> = {
            // c:263
            let st = STACK.lock().unwrap();
            st.last().copied()
        };
        if let Some(top) = stack_top {
            // c:263
            a = findparc(top.p, f.unwrap()); // c:264
            if a.is_none() {
                // c:264
                let new_parc = Parc {
                    // c:265
                    from: top.p,    // c:266
                    to: f.unwrap(), // c:267
                    calls: 0,       // c:268
                    self_time: 0.0, // c:269
                    time: 0.0,      // c:269
                };
                let mut arcs = ARCS.lock().unwrap();
                a = Some(arcs.len()); // c:271
                arcs.push(new_parc); // c:271
                NARCS.fetch_add(1, Ordering::SeqCst); // c:272
            }
        }
        // c:275-277 — `sf.prev = stack; sf.p = f; stack = &sf;`
        sf.p = f.unwrap(); // c:276
        STACK.lock().unwrap().push(sf); // c:277 stack = &sf

        // c:279 — `f->calls++;`
        {
            let mut calls = CALLS.lock().unwrap();
            calls[f.unwrap()].calls += 1; // c:279
        }
        // c:280-283 — read monotonic clock, compute prev (ms).
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        }; // c:280
        zgettime_monotonic_if_available(&mut ts); // c:281
        sf.beg = (ts.tv_sec as f64) * 1000.0 + (ts.tv_nsec as f64) / 1_000_000.0; // c:282-283
        prev = sf.beg; // c:282
                       // Update the stack-top copy we just pushed.
        let mut st = STACK.lock().unwrap();
        if let Some(top) = st.last_mut() {
            top.beg = sf.beg;
        }
    }

    // c:285 — `runshfunc(prog, w, name);` — the function-under-profile
    // runs HERE, between the c:282 start-timestamp and the c:289 end
    // read. Taken as the FnOnce runner (same shape as param_private's
    // wrap_private, 90dfda9df7) so the timing actually brackets the
    // call. A prior discarded placeholder meant every profiled time
    // measured the wrapper's own overhead (~0ms) instead of the
    // function.
    let _ = (prog, w);
    runshfunc(); // c:285

    if active != 0 {
        // c:286
        if ZPROF_MODULE.load(Ordering::SeqCst) {
            // c:287
            let mut ts = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }; // c:288
            zgettime_monotonic_if_available(&mut ts); // c:289
            let now = (ts.tv_sec as f64) * 1000.0 + (ts.tv_nsec as f64) / 1_000_000.0; // c:291-292

            // c:293 — `f->self += now - sf.beg;`
            {
                let mut calls = CALLS.lock().unwrap();
                if let Some(idx) = f {
                    calls[idx].self_time += now - sf.beg; // c:293
                }
            }
            // c:294 — recursion-detect: walk sf.prev looking for f.
            let recursion: bool = {
                // c:294
                let st = STACK.lock().unwrap();
                let cur_f = f.unwrap();
                // sf.prev = the frame underneath sf — walk it down.
                st.iter().rev().skip(1).any(|fr| fr.p == cur_f)
            };
            if !recursion {
                // c:295
                let mut calls = CALLS.lock().unwrap();
                if let Some(idx) = f {
                    calls[idx].time += now - prev; // c:296
                }
            }
            if let Some(arc_idx) = a {
                // c:297
                let mut arcs = ARCS.lock().unwrap();
                arcs[arc_idx].calls += 1; // c:298
                arcs[arc_idx].self_time += now - sf.beg; // c:299
            }
            // c:301 — `stack = sf.prev;`
            {
                let mut st = STACK.lock().unwrap();
                st.pop(); // c:301
            }
            // c:303-307 — propagate elapsed up to caller frame.
            let mut st = STACK.lock().unwrap();
            if let Some(top) = st.last_mut() {
                // c:303
                top.beg += now - prev; // c:304
                if let Some(arc_idx) = a {
                    // c:305
                    drop(st);
                    let mut arcs = ARCS.lock().unwrap();
                    arcs[arc_idx].time += now - prev; // c:306
                }
            }
        } else {
            // c:308
            // c:309 — `stack = sf.prev;`
            let mut st = STACK.lock().unwrap();
            st.pop();
        }
    }
    0 // c:311
}

// `bintab` — port of `static struct builtin bintab[]` (zprof.c:309).

// `module_features` — port of `static struct features module_features`
// from zprof.c:323.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/zprof.c:332`.
/// C body: `zprof_module = m; return 0;`
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:332
    ZPROF_MODULE.store(true, Ordering::SeqCst); // c:340
    0 // c:348
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/zprof.c:340`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:340
    *features = featuresarray(m, module_features());
    0 // c:355
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/zprof.c:348`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:348
    handlefeatures(m, module_features(), enables) // c:355
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/zprof.c:355`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:355
    let mut calls = CALLS.lock().unwrap();
    calls.clear(); // c:367
    NCALLS.store(0, Ordering::SeqCst); // c:367
    let mut arcs = ARCS.lock().unwrap();
    arcs.clear(); // c:367
    NARCS.store(0, Ordering::SeqCst); // c:367
    STACK.lock().unwrap().clear(); // c:367
    0 // c:367 addwrapper return
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/zprof.c:367`.
/// C body: free pfuncs + parcs, deletewrapper, setfeatureenables.
pub fn cleanup_(m: *const module) -> i32 {
    // c:367
    let mut calls = CALLS.lock().unwrap();
    freepfuncs(&mut calls); // c:377
    let mut arcs = ARCS.lock().unwrap();
    freeparcs(&mut arcs); // c:377
    ZPROF_MODULE.store(false, Ordering::SeqCst);
    setfeatureenables(m, module_features(), None) // c:377
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/zprof.c:377`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:377
    // C body c:379-380 — `return 0`. Faithful empty-body port; the
    //                    profiling tables get freed by cleanup_ via
    //                    setfeatureenables/zprof_cleanup.
    0
}

// ---------------------------------------------------------------------------
// Module loaders.
// ---------------------------------------------------------------------------

// =====================================================================
// static struct builtin bintab[]                                    c:309
// static struct features module_features                            c:323
// static struct funcwrap wrapper[]                                  c:328
// =====================================================================

// ---------------------------------------------------------------------------
// File-static globals — port of c:66-71.
// ---------------------------------------------------------------------------

/// Port of `static Pfunc calls;` from `Src/Modules/zprof.c:66`.
/// Per-function aggregated table; the C linked list becomes a
/// `Mutex<Vec<Pfunc>>` so `Pfunc *` becomes `usize` index.
pub static CALLS: Mutex<Vec<Pfunc>> = Mutex::new(Vec::new()); // c:66

/// Port of `static int ncalls;` from `Src/Modules/zprof.c:67`. Always
/// equals `CALLS.lock().len()` — kept as an explicit counter to
/// match C's `ncalls++` increment pattern.
pub static NCALLS: AtomicI32 = AtomicI32::new(0); // c:67

/// Port of `static Parc arcs;` from `Src/Modules/zprof.c:68`.
pub static ARCS: Mutex<Vec<Parc>> = Mutex::new(Vec::new()); // c:68

/// Port of `static int narcs;` from `Src/Modules/zprof.c:69`.
pub static NARCS: AtomicI32 = AtomicI32::new(0); // c:69

/// Port of `static Sfunc stack;` from `Src/Modules/zprof.c:70`. The
/// C linked stack becomes a `Mutex<Vec<Sfunc>>` (top of stack at
/// `last()`).
pub static STACK: Mutex<Vec<Sfunc>> = Mutex::new(Vec::new()); // c:70

/// Port of `static Module zprof_module;` from `Src/Modules/zprof.c:71`.
/// C uses a `Module` (struct module *) pointer to track which module
/// owns the wrapper; `zprof_wrapper` short-circuits when
/// `MOD_UNLOAD` is set on it. Module is ported as
/// `Box<crate::ported::zsh_h::module>` (zsh_h.rs:425) but recording
/// the raw `*const module` would deadlock with Sync/Send for the
/// static — `AtomicBool` captures the only state `zprof_wrapper`
/// actually inspects (loaded vs. unloading), matching the C
/// `MOD_UNLOAD` flag-check on the same pointer.
pub static ZPROF_MODULE: AtomicBool = AtomicBool::new(false); // c:74

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN ZPROF.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:zprof".to_string()]
}

// WARNING: NOT IN ZPROF.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN ZPROF.C — Rust-only module-framework shim.
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

// WARNING: NOT IN ZPROF.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::funcstack;

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

    /// Port of `bin_zprof(UNUSED(char *nam), UNUSED(char **args), Options ops, UNUSED(int func))` from `Src/Modules/zprof.c:139`.
    /// Verifies `Pfunc` mirrors C `struct pfunc` field-for-field
    /// (name/calls/time/self/num at c:40-44).
    #[test]
    fn pfunc_default_zeros() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let mut v = vec![Pfunc {
            name: "a".into(),
            ..Default::default()
        }];
        freepfuncs(&mut v);
        assert!(v.is_empty());
    }

    /// Verifies `findpfunc` linear-scan match (c:101-103).
    #[test]
    fn findpfunc_matches_by_name() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        CALLS.lock().unwrap().push(Pfunc {
            name: "alpha".into(),
            ..Default::default()
        });
        CALLS.lock().unwrap().push(Pfunc {
            name: "beta".into(),
            ..Default::default()
        });
        assert_eq!(findpfunc("alpha"), Some(0));
        assert_eq!(findpfunc("beta"), Some(1));
        assert_eq!(findpfunc("none"), None);
        reset_state();
    }

    /// Verifies `findparc` matches (from, to) pair (c:113-115).
    #[test]
    fn findparc_matches_pair() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        ARCS.lock().unwrap().push(Parc {
            from: 0,
            to: 1,
            ..Default::default()
        });
        ARCS.lock().unwrap().push(Parc {
            from: 0,
            to: 2,
            ..Default::default()
        });
        assert_eq!(findparc(0, 1), Some(0));
        assert_eq!(findparc(0, 2), Some(1));
        assert_eq!(findparc(1, 0), None);
        reset_state();
    }

    /// Verifies `cmpsfuncs` is descending (c:121-124).
    #[test]
    fn cmpsfuncs_descending() {
        let _g = crate::test_util::global_state_lock();
        let a = Pfunc {
            self_time: 5.0,
            ..Default::default()
        };
        let b = Pfunc {
            self_time: 10.0,
            ..Default::default()
        };
        // descending: b should come before a → cmp(a, b) = Greater
        assert_eq!(cmpsfuncs(&a, &b), std::cmp::Ordering::Greater);
        assert_eq!(cmpsfuncs(&b, &a), std::cmp::Ordering::Less);
    }

    /// Verifies `bin_zprof -c` clears state (c:141-147).
    #[test]
    fn bin_zprof_clear_resets_tables() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        CALLS.lock().unwrap().push(Pfunc {
            name: "x".into(),
            ..Default::default()
        });
        ARCS.lock().unwrap().push(Parc {
            from: 0,
            to: 0,
            ..Default::default()
        });
        NCALLS.store(1, Ordering::SeqCst);
        NARCS.store(1, Ordering::SeqCst);

        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zprof_wrapper(std::ptr::null(), std::ptr::null(), "foo", || {}),
            0,
        );
    }

    /// Verifies `name_for_anonymous_function` formats as
    /// `name [filename:lineno]` per c:224-232, reading filename
    /// and flineno from `funcstack[0]` per S1 rule.
    #[test]
    fn name_for_anonymous_function_format() {
        let _g = crate::test_util::global_state_lock();
        // Push a frame onto FUNCSTACK so the fn reads it.
        {
            let mut stack = FUNCSTACK.lock().unwrap();
            stack.clear();
            stack.push(funcstack {
                filename: Some("/tmp/foo.zsh".to_string()),
                flineno: 42,
                ..Default::default()
            });
        }
        let s = name_for_anonymous_function("anon");
        // Cleanup so subsequent tests aren't polluted.
        FUNCSTACK.lock().unwrap().clear();
        assert_eq!(s, "anon [/tmp/foo.zsh:42]");
    }

    /// `name_for_anonymous_function` with an EMPTY funcstack must
    /// return `"name [:0]"` — empty filename + zero lineno — not
    /// panic on the unwrap. The C body's `funcstack[0].flineno`
    /// would segfault on an empty stack; the Rust port must defend
    /// because nothing in zsh prevents the profiler from being
    /// invoked before the first function frame is pushed (e.g.
    /// during init scripts that contain anonymous functions at top
    /// level).
    #[test]
    fn name_for_anonymous_function_empty_funcstack_defaults() {
        let _g = crate::test_util::global_state_lock();
        // Ensure stack is empty.
        FUNCSTACK.lock().unwrap().clear();
        // No panic on first().unwrap() — the fn uses Option chains.
        let s = std::panic::catch_unwind(|| name_for_anonymous_function("anon"))
            .expect("must not panic on empty funcstack");
        assert_eq!(
            s, "anon [:0]",
            "empty funcstack → empty filename + 0 lineno; got {:?}",
            s
        );
    }

    /// c:97 — `findpfunc` on an empty table returns None. A
    /// regression that returns 0 (a valid index!) would silently
    /// corrupt every subsequent per-function profile accumulation.
    #[test]
    fn findpfunc_empty_table_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        assert!(findpfunc("never-called").is_none());
    }

    /// c:97 — `findpfunc` after two inserts returns the right index.
    /// Pin the index-zero-based contract because the find result
    /// feeds back into CALLS[i].
    #[test]
    fn findpfunc_returns_correct_index_after_insert() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        CALLS.lock().unwrap().push(Pfunc {
            name: "alpha".into(),
            ..Default::default()
        });
        CALLS.lock().unwrap().push(Pfunc {
            name: "beta".into(),
            ..Default::default()
        });
        assert_eq!(findpfunc("alpha"), Some(0));
        assert_eq!(findpfunc("beta"), Some(1));
        assert!(findpfunc("gamma").is_none());
    }

    /// c:109 — `findparc(f, t)` on an empty arcs table → None.
    #[test]
    fn findparc_empty_table_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        assert!(findparc(0, 1).is_none());
    }

    /// c:109 — `findparc` distinguishes (f1, t1) from (f1, t2):
    /// same `from`, different `to` is a different arc.
    #[test]
    fn findparc_distinguishes_to_field() {
        let _g = crate::test_util::global_state_lock();
        let _g = TEST_LOCK.lock().unwrap();
        reset_state();
        ARCS.lock().unwrap().push(Parc {
            from: 0,
            to: 1,
            ..Default::default()
        });
        ARCS.lock().unwrap().push(Parc {
            from: 0,
            to: 2,
            ..Default::default()
        });
        assert_eq!(findparc(0, 1), Some(0));
        assert_eq!(findparc(0, 2), Some(1));
        assert!(findparc(0, 99).is_none());
        assert!(findparc(99, 1).is_none());
    }

    /// c:121 — `cmpsfuncs` compares by `self_time` DESCENDING (C
    /// source: `(int)((*b)->self < (*a)->self) - (int)((*a)->self < (*b)->self)`
    /// — i.e. higher self_time sorts first). Pin the direction
    /// because a regen that flips to ascending would silently
    /// invert the user-facing `zprof` output ordering.
    #[test]
    fn cmpsfuncs_compares_by_self_time_descending() {
        let _g = crate::test_util::global_state_lock();
        let high = Pfunc {
            name: "_".into(),
            self_time: 100.0,
            ..Default::default()
        };
        let low = Pfunc {
            name: "_".into(),
            self_time: 1.0,
            ..Default::default()
        };
        // higher self_time sorts FIRST → Ordering::Less for (high, low)
        assert_eq!(cmpsfuncs(&high, &low), std::cmp::Ordering::Less);
        assert_eq!(cmpsfuncs(&low, &high), std::cmp::Ordering::Greater);
        assert_eq!(cmpsfuncs(&high, &high), std::cmp::Ordering::Equal);
    }

    /// c:74 — `freepfuncs` empties the input vec.
    #[test]
    fn freepfuncs_empties_input_vec() {
        let _g = crate::test_util::global_state_lock();
        let mut v = vec![
            Pfunc {
                name: "x".into(),
                ..Default::default()
            },
            Pfunc {
                name: "y".into(),
                ..Default::default()
            },
        ];
        freepfuncs(&mut v);
        assert!(v.is_empty(), "freepfuncs must clear the input vec");
    }

    /// c:86 — `freeparcs` empties the input arc vec.
    #[test]
    fn freeparcs_empties_input_vec() {
        let _g = crate::test_util::global_state_lock();
        let mut v = vec![
            Parc {
                from: 0,
                to: 1,
                ..Default::default()
            },
            Parc {
                from: 2,
                to: 3,
                ..Default::default()
            },
        ];
        freeparcs(&mut v);
        assert!(v.is_empty(), "freeparcs must clear the input vec");
    }

    /// c:121 vs c:127 — `cmpsfuncs` sorts by `self_time`, `cmptfuncs`
    /// sorts by `time` (cumulative). Pin they produce DIFFERENT
    /// orderings on an input where the two fields disagree, so a
    /// regen that aliases the field accessor in one of them gets
    /// caught.
    #[test]
    fn cmpsfuncs_and_cmptfuncs_differ_when_fields_disagree() {
        let _g = crate::test_util::global_state_lock();
        // `alpha` has high self_time but LOW cumulative time.
        // `beta`  has low self_time but HIGH cumulative time.
        let alpha = Pfunc {
            name: "_".into(),
            self_time: 100.0,
            time: 1.0,
            ..Default::default()
        };
        let beta = Pfunc {
            name: "_".into(),
            self_time: 1.0,
            time: 100.0,
            ..Default::default()
        };
        let by_self = cmpsfuncs(&alpha, &beta);
        let by_time = cmptfuncs(&alpha, &beta);
        // by_self: alpha has higher self_time → alpha sorts first → Less
        // by_time: beta has higher cumulative time → alpha sorts after → Greater
        assert_ne!(
            by_self, by_time,
            "cmpsfuncs (self_time) and cmptfuncs (time) must differ when fields disagree"
        );
    }

    // ─── zsh-corpus pins for zprof helpers ────────────────────────

    /// `cmpsfuncs` sorts higher self_time first.
    #[test]
    fn zprof_corpus_cmpsfuncs_higher_self_time_first() {
        let _g = crate::test_util::global_state_lock();
        let high = Pfunc {
            name: "high".into(),
            self_time: 100.0,
            ..Default::default()
        };
        let low = Pfunc {
            name: "low".into(),
            self_time: 1.0,
            ..Default::default()
        };
        assert_eq!(
            cmpsfuncs(&high, &low),
            std::cmp::Ordering::Less,
            "higher self_time sorts first"
        );
    }

    /// `cmptfuncs` sorts higher total time first.
    #[test]
    fn zprof_corpus_cmptfuncs_higher_time_first() {
        let _g = crate::test_util::global_state_lock();
        let high = Pfunc {
            name: "high".into(),
            time: 100.0,
            ..Default::default()
        };
        let low = Pfunc {
            name: "low".into(),
            time: 1.0,
            ..Default::default()
        };
        assert_eq!(
            cmptfuncs(&high, &low),
            std::cmp::Ordering::Less,
            "higher total time sorts first"
        );
    }

    /// `cmpsfuncs` equal self_time → Equal.
    #[test]
    fn zprof_corpus_cmpsfuncs_equal_self_time() {
        let _g = crate::test_util::global_state_lock();
        let a = Pfunc {
            name: "a".into(),
            self_time: 5.0,
            ..Default::default()
        };
        let b = Pfunc {
            name: "b".into(),
            self_time: 5.0,
            ..Default::default()
        };
        // Equal self_time may or may not tie-break by name; pin: not Greater
        let o = cmpsfuncs(&a, &b);
        assert_ne!(o, std::cmp::Ordering::Greater);
    }

    /// `findpfunc` on empty list returns None.
    #[test]
    fn zprof_corpus_findpfunc_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(findpfunc("__never_seen_function__").is_none());
    }

    /// `findparc` with arbitrary indexes returns None on empty arcs.
    #[test]
    fn zprof_corpus_findparc_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(findparc(usize::MAX, usize::MAX).is_none());
    }

    /// `name_for_anonymous_function("(anon)")` includes the input name
    /// and bracket form `[filename:lineno]`.
    #[test]
    fn zprof_corpus_name_for_anonymous_function_format() {
        let _g = crate::test_util::global_state_lock();
        let s = name_for_anonymous_function("(anon)");
        assert!(s.starts_with("(anon)"), "starts with name, got {s:?}");
        assert!(s.contains('['), "has opening bracket");
        assert!(s.contains(']'), "has closing bracket");
        assert!(s.contains(':'), "has line-number separator");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for Src/Modules/zprof.c cmp comparators + free ops.
    // ═══════════════════════════════════════════════════════════════════

    fn mk_pfunc(name: &str, self_time: f64, time: f64) -> Pfunc {
        Pfunc {
            name: name.to_string(),
            calls: 0,
            time,
            self_time,
            num: 0,
        }
    }

    fn mk_parc(from: usize, to: usize, time: f64) -> Parc {
        Parc {
            from,
            to,
            calls: 0,
            time,
            ..Default::default()
        }
    }

    /// c:121 — `cmpsfuncs` is descending by self_time: a > b → Less
    /// (sort places larger first).
    #[test]
    fn cmpsfuncs_descending_by_self_time() {
        let a = mk_pfunc("a", 10.0, 0.0);
        let b = mk_pfunc("b", 5.0, 0.0);
        assert_eq!(
            cmpsfuncs(&a, &b),
            std::cmp::Ordering::Less,
            "larger self_time sorts first"
        );
        assert_eq!(cmpsfuncs(&b, &a), std::cmp::Ordering::Greater);
    }

    /// c:121 — equal self_time → Equal.
    #[test]
    fn cmpsfuncs_equal_returns_equal() {
        let a = mk_pfunc("a", 7.5, 0.0);
        let b = mk_pfunc("b", 7.5, 99.0); // different time, equal self_time
        assert_eq!(cmpsfuncs(&a, &b), std::cmp::Ordering::Equal);
    }

    /// c:127 — `cmptfuncs` descending by total time (ignores self_time).
    #[test]
    fn cmptfuncs_descending_by_total_time() {
        let a = mk_pfunc("a", 0.0, 10.0);
        let b = mk_pfunc("b", 99.0, 5.0); // larger self but smaller time
        assert_eq!(
            cmptfuncs(&a, &b),
            std::cmp::Ordering::Less,
            "larger total time sorts first regardless of self_time"
        );
    }

    /// c:133 — `cmpparcs` descending by time.
    #[test]
    fn cmpparcs_descending_by_time() {
        let a = mk_parc(0, 1, 10.0);
        let b = mk_parc(0, 1, 5.0);
        assert_eq!(cmpparcs(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(cmpparcs(&b, &a), std::cmp::Ordering::Greater);
    }

    /// c:121 — NaN comparison → Equal (partial_cmp unwrap_or branch).
    #[test]
    fn cmpsfuncs_nan_returns_equal() {
        let a = mk_pfunc("a", f64::NAN, 0.0);
        let b = mk_pfunc("b", 5.0, 0.0);
        assert_eq!(
            cmpsfuncs(&a, &b),
            std::cmp::Ordering::Equal,
            "NaN partial_cmp returns None → Equal fallback"
        );
    }

    /// c:74 — `freepfuncs` empties the vec.
    #[test]
    fn freepfuncs_clears_vec() {
        let mut v = vec![mk_pfunc("a", 0.0, 0.0), mk_pfunc("b", 0.0, 0.0)];
        freepfuncs(&mut v);
        assert!(v.is_empty(), "freepfuncs must clear the Vec");
    }

    /// c:86 — `freeparcs` empties the vec.
    #[test]
    fn freeparcs_clears_vec() {
        let mut v = vec![mk_parc(0, 1, 0.0), mk_parc(1, 2, 0.0)];
        freeparcs(&mut v);
        assert!(v.is_empty());
    }

    /// c:74 — `freepfuncs` on empty vec is a no-op.
    #[test]
    fn freepfuncs_empty_is_noop() {
        let mut v: Vec<Pfunc> = Vec::new();
        freepfuncs(&mut v);
        assert!(v.is_empty());
    }

    /// c:97 — `findpfunc` is deterministic for the same lookup.
    #[test]
    fn findpfunc_is_deterministic_for_missing() {
        let _g = crate::test_util::global_state_lock();
        let first = findpfunc("__nope__");
        for _ in 0..10 {
            assert_eq!(findpfunc("__nope__"), first);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zprof.c
    // c:123 findpfunc / c:135 findparc / c:152-170 cmp* / c:418 name_for_anon /
    // c:509 zprof_wrapper / c:682+ lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:135 — `findparc(0,0)` on empty table returns None.
    #[test]
    fn findparc_zero_zero_empty_table_returns_none() {
        let _g = crate::test_util::global_state_lock();
        reset_state();
        assert_eq!(findparc(0, 0), None);
    }

    /// c:123 — `findpfunc("")` empty name returns None on empty table.
    #[test]
    fn findpfunc_empty_name_empty_table_returns_none() {
        let _g = crate::test_util::global_state_lock();
        reset_state();
        assert_eq!(findpfunc(""), None);
    }

    /// c:152 — `cmpsfuncs(a, a)` is Equal (reflexive).
    #[test]
    fn cmpsfuncs_reflexive() {
        let a = mk_pfunc("x", 1.0, 2.0);
        assert_eq!(cmpsfuncs(&a, &a), std::cmp::Ordering::Equal);
    }

    /// c:161 — `cmptfuncs(a, a)` is Equal (reflexive).
    #[test]
    fn cmptfuncs_reflexive() {
        let a = mk_pfunc("x", 1.0, 2.0);
        assert_eq!(cmptfuncs(&a, &a), std::cmp::Ordering::Equal);
    }

    /// c:170 — `cmpparcs(a, a)` is Equal (reflexive).
    #[test]
    fn cmpparcs_reflexive() {
        let a = mk_parc(0, 1, 5.0);
        assert_eq!(cmpparcs(&a, &a), std::cmp::Ordering::Equal);
    }

    /// c:152 — `cmpsfuncs` is antisymmetric: if a vs b is X, b vs a
    /// is X.reverse().
    #[test]
    fn cmpsfuncs_antisymmetric() {
        let a = mk_pfunc("a", 1.0, 5.0);
        let b = mk_pfunc("b", 2.0, 5.0);
        let ab = cmpsfuncs(&a, &b);
        let ba = cmpsfuncs(&b, &a);
        assert_eq!(ab.reverse(), ba, "must be antisymmetric");
    }

    /// c:509 — `zprof_wrapper(null, null, "")` no panic.
    #[test]
    fn zprof_wrapper_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = zprof_wrapper(std::ptr::null(), std::ptr::null(), "", || {});
    }

    /// c:418 — `name_for_anonymous_function` is deterministic.
    #[test]
    fn name_for_anonymous_function_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = name_for_anonymous_function("anon");
        for _ in 0..5 {
            assert_eq!(name_for_anonymous_function("anon"), first);
        }
    }

    /// c:682-... — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn zprof_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zprof.c
    // c:105 freepfuncs / c:113 freeparcs / c:123 findpfunc / c:135 findparc
    // c:152 cmpsfuncs / c:161 cmptfuncs / c:170 cmpparcs /
    // c:418 name_for_anonymous_function / c:509 zprof_wrapper
    // ═══════════════════════════════════════════════════════════════════

    /// c:105 — `freepfuncs` returns void (compile-time pin).
    #[test]
    fn freepfuncs_returns_void() {
        let mut v: Vec<Pfunc> = vec![];
        let _: () = freepfuncs(&mut v);
    }

    /// c:113 — `freeparcs` returns void.
    #[test]
    fn freeparcs_returns_void() {
        let mut v: Vec<Parc> = vec![];
        let _: () = freeparcs(&mut v);
    }

    /// c:123 — `findpfunc` returns Option<usize> (compile-time pin).
    #[test]
    fn findpfunc_returns_option_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<usize> = findpfunc("anything");
    }

    /// c:135 — `findparc(N, M)` returns Option<usize>.
    #[test]
    fn findparc_returns_option_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<usize> = findparc(0, 0);
    }

    /// c:152 — `cmpsfuncs` returns Ordering (compile-time pin).
    #[test]
    fn cmpsfuncs_returns_ordering_type() {
        let a = mk_pfunc("a", 0.0, 0.0);
        let _: std::cmp::Ordering = cmpsfuncs(&a, &a);
    }

    /// c:161 — `cmptfuncs` returns Ordering.
    #[test]
    fn cmptfuncs_returns_ordering_type() {
        let a = mk_pfunc("a", 0.0, 0.0);
        let _: std::cmp::Ordering = cmptfuncs(&a, &a);
    }

    /// c:170 — `cmpparcs` returns Ordering.
    #[test]
    fn cmpparcs_returns_ordering_type() {
        let a = mk_parc(0, 1, 0.0);
        let _: std::cmp::Ordering = cmpparcs(&a, &a);
    }

    /// c:418 — `name_for_anonymous_function` returns String type pin.
    #[test]
    fn name_for_anonymous_function_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = name_for_anonymous_function("anon");
    }

    /// c:509 — `zprof_wrapper` returns i32 type pin.
    #[test]
    fn zprof_wrapper_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = zprof_wrapper(std::ptr::null(), std::ptr::null(), "x", || {});
    }

    /// c:105 — `freepfuncs` empties the vec.
    #[test]
    fn freepfuncs_empties_vec_pin() {
        let mut v = vec![mk_pfunc("a", 0.0, 0.0)];
        freepfuncs(&mut v);
        assert!(v.is_empty(), "freepfuncs must empty");
    }

    /// c:113 — `freeparcs` empties the vec.
    #[test]
    fn freeparcs_empties_vec_pin() {
        let mut v = vec![mk_parc(0, 1, 0.0)];
        freeparcs(&mut v);
        assert!(v.is_empty(), "freeparcs must empty");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/zprof.c
    // c:123 findpfunc / c:135 findparc / c:152-170 cmp* / c:193 bin_zprof /
    // c:418 name_for_anonymous_function / c:682-719 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:123 — `findpfunc("")` empty name doesn't panic.
    #[test]
    fn findpfunc_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = findpfunc("");
    }

    /// c:123 — `findpfunc` is pure (no observable mutation across calls).
    #[test]
    fn findpfunc_repeated_calls_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = findpfunc("__never_real_pfn__");
        let b = findpfunc("__never_real_pfn__");
        assert_eq!(a, b, "findpfunc must be pure across calls");
    }

    /// c:135 — `findparc(0, 0)` doesn't panic.
    #[test]
    fn findparc_zero_indices_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = findparc(0, 0);
    }

    /// c:135 — `findparc(usize::MAX, usize::MAX)` doesn't panic.
    #[test]
    fn findparc_extreme_indices_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = findparc(usize::MAX, usize::MAX);
    }

    /// c:152 — `cmpsfuncs` reflexivity (alt).
    #[test]
    fn cmpsfuncs_reflexive_alt() {
        let p = mk_pfunc("a", 1.0, 2.0);
        let p2 = p.clone();
        assert_eq!(
            cmpsfuncs(&p, &p2),
            std::cmp::Ordering::Equal,
            "cmpsfuncs(x, x.clone()) must be Equal"
        );
    }

    /// c:161 — `cmptfuncs` reflexivity (alt).
    #[test]
    fn cmptfuncs_reflexive_alt() {
        let p = mk_pfunc("a", 1.0, 2.0);
        let p2 = p.clone();
        assert_eq!(
            cmptfuncs(&p, &p2),
            std::cmp::Ordering::Equal,
            "cmptfuncs(x, x.clone()) must be Equal"
        );
    }

    /// c:170 — `cmpparcs` reflexivity (alt).
    #[test]
    fn cmpparcs_reflexive_alt() {
        let a = mk_parc(0, 1, 5.0);
        let b = a.clone();
        assert_eq!(cmpparcs(&a, &b), std::cmp::Ordering::Equal);
    }

    /// c:193 — `bin_zprof` empty args non-negative.
    #[test]
    fn bin_zprof_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zprof("zprof", &[], &ops, 0);
        assert!(r >= 0, "bin_zprof empty must be ≥ 0, got {}", r);
    }

    /// c:418 — `name_for_anonymous_function("")` returns String type (alt).
    #[test]
    fn name_for_anonymous_function_returns_string_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: String = name_for_anonymous_function("");
    }

    /// c:418 — synthesized name is non-empty.
    #[test]
    fn name_for_anonymous_function_returns_non_empty() {
        let _g = crate::test_util::global_state_lock();
        let n = name_for_anonymous_function("");
        assert!(!n.is_empty(), "synthesized name must not be empty");
    }

    /// c:682 — `setup_` is idempotent.
    #[test]
    fn zprof_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:719 — `cleanup_` is idempotent.
    #[test]
    fn zprof_cleanup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }
}
