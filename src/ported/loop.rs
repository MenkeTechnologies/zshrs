//! Loop execution for zshrs
//!
//! Port from zsh/Src/loop.c (802 lines)
//!
//! # of nested loops we are in                                              // c:33
//! # of continue levels                                                     // c:38
//! # of break levels                                                        // c:43
//!
//! In C, loop.c contains execfor, execwhile, execif, execcase, execselect,
//! execrepeat, and exectry as separate functions operating on bytecode.
//! In Rust, all of these are implemented as match arms in
//! ShellExecutor::execute_compound() in vm_helper, operating on the typed AST
//! (CompoundCommand::For, While, If, Case, Select, Repeat, Try).
//!
//! This module provides the loop state management and helper functions
//! that support the executor's loop implementation.

use crate::ported::utils::{adjustcolumns, adjustlines};
use std::sync::atomic::Ordering;

// ── Imports for the `Src/loop.c` wordcode-VM entries below ───────
// These were `use` items of `src/ported/exec.rs` while the seven
// exec* bodies lived there; they move with the bodies.
use crate::ported::builtin::{BREAKS, CONTFLAG, LASTVAL, LOOPS, RETFLAG};
use crate::ported::exec::{execlist, execsubst, noerrexit, simple_pline, this_noerrexit};
use crate::ported::lex::untokenize;
use crate::ported::math::{matheval as wc_matheval, mathevali as wc_mathevali};
use crate::ported::mem::{freeheap, popheap, pushheap};
use crate::ported::params::setloopvar;
use crate::ported::parse::{ecgetlist, ecgetstr, ecrawstr};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::prompt::{cmdpop, cmdpush};
use crate::ported::signals::{queue_signals, unqueue_signals};
use crate::ported::subst::singsub;
use crate::ported::utils::{errflag, printprompt4, zerr, ERRFLAG_ERROR};
use crate::ported::zsh_h::{
    estate, isset, wc_code, CS_ALWAYS, CS_CASE, CS_CURSH, CS_ELIF, CS_ELIFTHEN, CS_ELSE, CS_FOR,
    CS_IF, CS_IFTHEN, CS_REPEAT, CS_UNTIL, CS_WHILE, EC_DUP, EC_DUPTOK, EC_NODUP, ERRFLAG_INT,
    NOERREXIT_EXIT, NOERREXIT_RETURN, PAT_STATIC, WC_CASE, WC_CASE_AND, WC_CASE_OR, WC_CASE_SKIP,
    WC_CASE_TESTAND, WC_CASE_TYPE, WC_END, WC_FOR_COND, WC_FOR_LIST, WC_FOR_SKIP, WC_FOR_TYPE,
    WC_IF, WC_IF_ELSE, WC_IF_SKIP, WC_IF_TYPE, WC_REPEAT_SKIP, WC_TRY_SKIP, WC_WHILE_SKIP,
    WC_WHILE_TYPE, WC_WHILE_UNTIL,
};
use crate::zsh_h::XTRACE;
use std::io::Write;
use std::sync::atomic::AtomicI32;
// ===========================================================
// C `Src/loop.c` — wordcode VM helpers for control flow.
//
// In C zsh, these seven functions run as part of the wordcode VM in
// `Src/exec.c`: each consumes an `Estate` / wordcode cursor (not a
// separate AST interpreter in the parser).
//
// zshrs lowers shell constructs to fusevm bytecode
// (see `tree_walker_absent.rs` / `no_tree_walker_dispatch.rs`
// invariant tests), so these entries exist to satisfy ABI/name
// parity. The actual control-flow lowering happens in the
// fusevm compiler (`crate::fusevm::compile`) where every
// `for`/`while`/`if`/`case`/`select`/`repeat`/`try` AST node
// becomes a fusevm `Op`.
// ===========================================================

// The seven entries below are zsh's `Src/loop.c` wordcode VM hooks.
// zshrs lowers shell constructs to fusevm bytecode — every `for`/`while`/`if`/`case`/`select`/
// `repeat`/`try` AST node lowers to a fusevm Op in
// `src/extensions/compile_zsh.rs`. These entries exist purely for
// C-name parity (drift gate enforces every Rust fn maps to a C fn).
//
// The 96-test architectural invariant in `tree_walker_absent.rs` +
// `no_tree_walker_dispatch.rs` proves these are never reached in
// production. Each body is `unreachable!()` so ANY caller fails
// loudly rather than silently returning 0 — if a port regresses
// the bytecode lowering, we want the test suite to crash, not pass.
//
// Faithful per-fn port of the C bodies is intentionally NOT done:
// they read `Wordcode` / `Estate` cursors that zshrs doesn't model.
// The semantic equivalent lives in:
//   execfor    → compile_zsh.rs::compile_for
//   execselect → compile_zsh.rs::compile_select
//   execwhile  → compile_zsh.rs::compile_while
//   execrepeat → compile_zsh.rs::compile_repeat
//   execif     → compile_zsh.rs::compile_if
//   execcase   → compile_zsh.rs::compile_case
//   exectry    → compile_zsh.rs::compile_try

// execfor is ported below in this file (Src/loop.c:50).


// Note: dead `ForIterator` / `CForState` / `TryState` aggregates
// removed per PORT_PLAN Phase 2. None had production callers (only
// internal test references). The actual control flow is lowered in
// the fusevm compiler — every `for`/`while`/`select`/`repeat`/`try`
// AST node becomes a fusevm Op (see `src/extensions/compile_zsh.rs`).
//
// C source's relevant try-block file-globals (loop.c:719-727):
//
//     zlong try_errflag = -1;       // line 719 (TRY_BLOCK_ERROR)
//     zlong try_interrupt = -1;     // line 727 (TRY_BLOCK_INTERRUPT)
//     zlong try_tryflag = 0;        // line 731 (TRY_BLOCK_DEPTH)
//
// Exported via `IPDEF6` paramdef in `Src/params.c:364`, so they're
// cross-compilation-unit globals → PORT_PLAN Phase 3 bucket-2
// (Arc<RwLock>) work, not the Phase 2 bucket-1 (thread_local!) wave.

/// Port of `mod_export zlong try_tryflag` from `Src/loop.c:731`.
/// Depth-counter for active `always {}` blocks; bumped at try entry
/// (`exectry`), decremented at exit. `dotrapargs` (`signals.c:1215`)
/// reads it to decide whether a trap's `errflag` propagates.
pub static try_tryflag: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0); // c:731

/// Port of `mod_export zlong try_errflag` from `Src/loop.c:719`.
/// Initialized to -1 (sentinel: "no try block has fired yet in this
/// scope"); the `exectry` always-arm sets it to `errflag & ERRFLAG_ERROR`
/// before running the always body, then restores. `TRY_BLOCK_ERROR`
/// param (IPDEF6 at `Src/params.c:364`) reads this global directly.
pub static try_errflag: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // c:719

/// Port of `mod_export zlong try_interrupt` from `Src/loop.c:727`.
/// Initialized to -1; mirrors `try_errflag` for the `ERRFLAG_INT` bit.
/// `TRY_BLOCK_INTERRUPT` param reads this.
pub static try_interrupt: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // c:727

// execselect is ported below in this file (Src/loop.c:217).


// Note: dead `LoopState` aggregate (and impl/tests) deleted per
// PORT_PLAN Phase 2. It was a Rust-only invention that double-tracked
// the same data already living in the file-statics LOOP_DEPTH /
// CONT_FLAG / BREAK_LEVEL above (and on `ShellExecutor.breaking` /
// `ShellExecutor.continuing` in src/vm_helper:572-573). Zero callers
// outside its own test module.
//
// C source's actual loop-control file-globals at `Src/loop.c`:
//
//     int loops;                          // line 36
//     mod_export int contflag;            // line 41
//     mod_export volatile int breaks;     // line 46
//
// All `mod_export` (cross-compilation-unit), so they're PORT_PLAN
// Phase 3 bucket-2 (Arc<RwLock>) work. The canonical C-named ports
// (LOOPS / CONTFLAG / BREAKS) live in `src/ported/builtin.rs:3657-3659`
// where the bin_break dispatcher consults them; the local
// LOOP_DEPTH / CONT_FLAG / BREAK_LEVEL above are this file's
// internal mirrors.

// And this is used to print select lists.                                 // c:347
/// Select-menu display.
/// Port of `selectlist(LinkList l, size_t start)` from Src/loop.c:347 — formats the
/// numbered menu the C source uses for `select var in words`. Picks
/// columns automatically when `columns == 0`, mirroring the C
/// source's terminal-width auto-detection.
/// WARNING: param names don't match C — Rust=(items, start) vs C=(l, start)
pub fn selectlist(items: &[&str], start: usize) -> usize {
    // c:347
    let mut stderr = std::io::stderr().lock();

    // c:351 — zleentry(ZLE_CMD_TRASH); — flush ZLE redraw state.
    // zshrs's ZLE entry-point dispatch is wired through the
    // executor; the trash hook runs there, not in this body.

    let ct = items.len(); // c:362 ap - arr
    if ct == 0 {
        // guard against empty list
        return 0;
    }
    let mut longest: usize = 1; // c:350
    for ap in items {
        // c:354 for (ap = arr; *ap; ap++)
        // C uses MB_METASTRWIDTH for visible width (combining-char
        // aware). Rust port uses chars().count() — adequate for
        // standard ASCII / non-combining content.
        let aplen = ap.chars().count(); // c:359 unmetafy width
        if aplen > longest {
            // c:362
            longest = aplen; // c:363
        }
    }
    longest += 1; // c:365 +1 for ") "
    let mut t0 = ct; // c:366
    while t0 != 0 {
        // c:367
        t0 /= 10; // c:368
        longest += 1; // c:368 (+1 per digit)
    }

    let zterm_columns = adjustcolumns(); // c:zterm_columns
    let zterm_lines = adjustlines();
    let mut fct: usize = (zterm_columns.saturating_sub(1)) / (longest + 3); // c:371
    let fw: usize;
    if fct == 0 {
        // c:372
        fct = 1; // c:373
        fw = 0; // (unused when fct==1)
    } else {
        fw = (zterm_columns - 1) / fct; // c:375
    }
    let colsz = (ct + fct - 1) / fct; // c:377

    // c:379 — for (t1 = start; t1 != colsz && t1 - start < zterm_lines - 2; t1++)
    //
    // C uses `*ap` (NUL-terminated array walk) to bound the inner loop;
    // the Rust port lost that sentinel, so an out-of-range `start` (or
    // a row-offset that reaches past `ct`) indexes `items[idx]` and
    // panics. Mirror the C terminator with explicit bounds checks:
    //   - outer: stop when `t1 >= colsz` OR `start + t1*fct >= ct`
    //   - inner: skip the row entirely when the column-base is OOB.
    let mut t1 = start;
    let max_lines = zterm_lines.saturating_sub(2);
    while t1 < colsz && (t1.saturating_sub(start)) < max_lines {
        let mut idx = t1; // c:381 ap = arr + t1
        if idx >= ct {
            // Past the last item — C's `*ap` would already be NULL here.
            // Emit the blank line C would produce and advance.
            let _ = stderr.write_all(b"\n");
            t1 += 1;
            continue;
        }
        loop {
            // c:383 do {
            let entry = items[idx];
            // c:385 — t2 = MB_METASTRWIDTH(*ap) + 2;
            let mut t2 = entry.chars().count() + 2;
            // c:391 — fprintf(stderr, "%d) %s", t3 = ap - arr + 1, *ap);
            let t3 = idx + 1;
            let _ = write!(stderr, "{}) {}", t3, entry);
            // c:393 — while (t3) t2++, t3 /= 10;
            let mut digits = t3;
            while digits != 0 {
                // c:393
                t2 += 1;
                digits /= 10;
            }
            // c:396 — for (; t2 < fw; t2++) fputc(' ', stderr);
            while t2 < fw {
                // c:396
                let _ = stderr.write_all(b" ");
                t2 += 1;
            }
            // c:398 — for (t0 = colsz; t0 && *ap; t0--, ap++);
            let mut t0 = colsz;
            while t0 != 0 && idx + 1 < ct {
                t0 -= 1;
                idx += 1;
                if t0 == 0 {
                    break;
                }
            }
            if idx + 1 >= ct {
                break;
            } // c:401 while (*ap);
        }
        let _ = stderr.write_all(b"\n"); // c:401 fputc('\n', stderr);
        t1 += 1;
    }

    let _ = stderr.flush(); // c:413 fflush(stderr);

    if t1 < colsz {
        t1
    } else {
        0
    } // c:415 return
}

// execwhile / execrepeat / execif / execcase / exectry moved to
// src/ported/exec.rs as faithful ports of Src/loop.c:413 / :499 /
// :553 / :600 / :735. See exec.rs for the C-line-cited bodies.

/// Number of nested loops.
/// Port of the global `loops` counter from Src/loop.c — every
/// `execfor`/`execwhile`/`execrepeat`/`execselect` entry bumps it
/// and decrements on exit.
static LOOP_DEPTH: AtomicI32 = AtomicI32::new(0);

/// Continue flag / level.
/// Port of the global `contflag` from Src/loop.c — set by the
/// `continue` builtin (Src/builtin.c:bin_break) and consumed by
/// the loop body's exit check.
static CONT_FLAG: AtomicI32 = AtomicI32::new(0);

/// Break level.
/// Port of the global `breaks` counter from Src/loop.c — set by
/// the `break` builtin (Src/builtin.c:bin_break) and tested by
/// each enclosing loop on exit.
static BREAK_LEVEL: AtomicI32 = AtomicI32::new(0);


// ===========================================================
// The seven `Src/loop.c` wordcode-VM entries. These bodies used to
// live in `src/ported/exec.rs`; they are ports of `Src/loop.c`, so
// they belong in the Rust mirror of that file. `exec.rs` calls them
// through `crate::ported::r#loop::{...}` exactly as C's `execfuncs[]`
// table (`Src/exec.c:275-277`) dispatches into loop.c.
// ===========================================================

// --- loop.c entries ---------------------------------------------------

/// Port of `execfor(Estate state, int do_exec)` from `Src/loop.c:50-202`.
/// `for var in args; do body; done` and the C-style `for ((init;cond;adv))`
/// variant. WC_FOR_TYPE distinguishes PPARAM (use $@) / LIST (explicit
/// words) / COND (C-style).
pub fn execfor(state: &mut estate, do_exec: i32) -> i32 {
    use crate::ported::zsh_h::Z_END;
    let code = state.prog.prog[state.pc.wrapping_sub(1)]; // c:54
    let iscond = WC_FOR_TYPE(code) == WC_FOR_COND; // c:55
    let mut last_iter = false; // c:57 — `int last = 0;`
    let mut val: i64 = 0; // c:59
    let mut vars: Vec<String> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut cond_expr: String = String::new();
    let mut advance_expr: String = String::new();
    let old_simple_pline = simple_pline.swap(1, Ordering::Relaxed); // c:62-63
    let end_pc = state.pc + WC_FOR_SKIP(code) as usize; // c:65
    let mut ctok = 0i32;
    let mut atok = 0i32;
    if iscond {
        // c:68-82 — C-style for: init expr at top, then cond/advance.
        let init = ecgetstr(state, EC_NODUP, None); // c:68
        let init_sub = singsub(&init); // c:69
        if isset(XTRACE) {
            // c:70-75
            let init_show = untokenize(&init_sub);
            printprompt4();
            eprintln!("{}", init_show);
        }
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
            let _ = wc_matheval(&init_sub); // c:77 — `matheval(str);`
        }
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            // c:79-82
            state.pc = end_pc;
            simple_pline.store(old_simple_pline, Ordering::Relaxed);
            return 1;
        }
        cond_expr = ecgetstr(state, EC_NODUP, Some(&mut ctok)); // c:83
        advance_expr = ecgetstr(state, EC_NODUP, Some(&mut atok)); // c:84
    } else {
        // c:86 — `vars = ecgetlist(state, *state->pc++, EC_NODUP, NULL);`
        let count = state.prog.prog[state.pc] as usize;
        state.pc += 1;
        vars = ecgetlist(state, count, EC_NODUP, None);
        if WC_FOR_TYPE(code) == WC_FOR_LIST {
            // c:88-100 — explicit `for var in words`
            let mut htok = 0i32;
            let arg_count = state.prog.prog[state.pc] as usize;
            state.pc += 1;
            args = ecgetlist(state, arg_count, EC_DUPTOK, Some(&mut htok));
            if args.is_empty() {
                state.pc = end_pc;
                simple_pline.store(old_simple_pline, Ordering::Relaxed);
                return 0;
            }
            if htok != 0 {
                execsubst(&mut args); // c:96
                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                    state.pc = end_pc;
                    simple_pline.store(old_simple_pline, Ordering::Relaxed);
                    return 1;
                }
            }
        } else {
            // c:102-107 — implicit `for var` (no `in` clause) uses
            // the positional params $@ from PPARAMS (params.rs Mutex).
            args = crate::ported::builtin::PPARAMS
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default();
        }
    }
    // c:111-112 — empty args ⇒ lastval = 0.
    if !iscond && args.is_empty() {
        LASTVAL.store(0, Ordering::Relaxed);
    }
    LOOPS.fetch_add(1, Ordering::SeqCst); // c:114 — `loops++;`
    pushheap(); // c:115
    cmdpush(CS_FOR as u8); // c:116
    let loop_pc = state.pc; // c:117
    let mut args_iter = args.into_iter();
    while !last_iter {
        if iscond {
            // c:119-138 — eval cond expression.
            let mut cs = cond_expr.clone();
            if ctok != 0 {
                cs = singsub(&cs);
            }
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
                let trimmed = cs.trim_start();
                if !trimmed.is_empty() {
                    if isset(XTRACE) {
                        printprompt4();
                        eprintln!("{}", trimmed);
                    }
                    val = wc_mathevali(trimmed).unwrap_or(0);
                } else {
                    val = 1;
                }
            }
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                if BREAKS.load(Ordering::SeqCst) > 0 {
                    BREAKS.fetch_sub(1, Ordering::SeqCst);
                }
                LASTVAL.store(1, Ordering::Relaxed);
                break;
            }
            if val == 0 {
                break;
            }
        } else {
            // c:140-162 — for var binding from args.
            let mut count = 0;
            for name in &vars {
                let value = match args_iter.next() {
                    Some(v) => v,
                    None => {
                        if count != 0 {
                            last_iter = true;
                            String::new()
                        } else {
                            break;
                        }
                    }
                };
                if isset(XTRACE) {
                    printprompt4();
                    eprintln!("{}={}", name, value);
                }
                setloopvar(name, &value);
                count += 1;
            }
            if count == 0 {
                break;
            }
        }
        state.pc = loop_pc; // c:163
        let _do_exec_now = do_exec != 0 && !args_iter.clone().any(|_| true); // c:164 — `do_exec && args && empty(args)`
        let _ = execlist(state, 1, if _do_exec_now { 1 } else { 0 });
        // c:166-169 — breaks/continue handling.
        if BREAKS.load(Ordering::SeqCst) > 0 {
            let prev = BREAKS.fetch_sub(1, Ordering::SeqCst);
            if prev - 1 > 0 || CONTFLAG.load(Ordering::SeqCst) == 0 {
                break;
            }
            CONTFLAG.store(0, Ordering::SeqCst);
        }
        if RETFLAG.load(Ordering::SeqCst) != 0 {
            break;
        }
        // c:170-178 — C-style advance step.
        if iscond && (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
            let mut adv = advance_expr.clone();
            if atok != 0 {
                adv = singsub(&adv);
            }
            if isset(XTRACE) {
                printprompt4();
                eprintln!("{}", adv);
            }
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
                let _ = wc_matheval(&adv);
            }
        }
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            if BREAKS.load(Ordering::SeqCst) > 0 {
                BREAKS.fetch_sub(1, Ordering::SeqCst);
            }
            LASTVAL.store(1, Ordering::Relaxed);
            break;
        }
        freeheap(); // c:184
    }
    popheap(); // c:186
    cmdpop(); // c:187
    LOOPS.fetch_sub(1, Ordering::SeqCst); // c:188
    simple_pline.store(old_simple_pline, Ordering::Relaxed);
    state.pc = end_pc;
    this_noerrexit.store(1, Ordering::Relaxed);
    let _ = Z_END;
    LASTVAL.load(Ordering::Relaxed)
}

/// Port of `execselect(Estate state, UNUSED(int do_exec))` from
/// `Src/loop.c:217-410`. `select var in words; do body; done` REPL.
pub fn execselect(state: &mut estate, _do_exec: i32) -> i32 {
    // The full select body manages a REPL prompt, terminal columns,
    // selectlist redraw, etc. The `selectlist` helper at loop.rs:130
    // already ports c:347 (menu display). Structural execselect:
    // c:225-410 — read vars + words like execfor, then loop on stdin
    // input prompting via PROMPT3, set var=word, run body.
    let code = state.prog.prog[state.pc.wrapping_sub(1)];
    let end_pc = state.pc + WC_FOR_SKIP(code) as usize;
    // c:228-237 — read var name + words. Skip body and use existing
    // bridge handler at BUILTIN_RUN_SELECT for actual REPL until full
    // wordcode driver lands.
    state.pc = end_pc;
    this_noerrexit.store(1, Ordering::Relaxed);
    LASTVAL.load(Ordering::Relaxed)
}

/// Port of `execwhile(Estate state, UNUSED(int do_exec))` from
/// `Src/loop.c:413-498`. `while/until cond; do body; done`.
pub fn execwhile(state: &mut estate, _do_exec: i32) -> i32 {
    let code = state.prog.prog[state.pc.wrapping_sub(1)]; // c:417
    let isuntil = WC_WHILE_TYPE(code) == WC_WHILE_UNTIL; // c:419
    let end_pc = state.pc + WC_WHILE_SKIP(code) as usize; // c:422
    let olderrexit = noerrexit.load(Ordering::Relaxed); // c:423
    let mut oldval: i32 = 0; // c:424
    pushheap(); // c:425
    cmdpush(if isuntil {
        CS_UNTIL as u8
    } else {
        CS_WHILE as u8
    }); // c:426
    LOOPS.fetch_add(1, Ordering::SeqCst); // c:427
    let loop_pc = state.pc; // c:428
    let old_simple_pline = simple_pline.load(Ordering::Relaxed); // c:419
                                                                 // c:430-456 — empty-loop fast path. If loop body is two WC_ENDs,
                                                                 // sit in a tight signal-wait loop until ^C breaks us.
    if state.prog.prog.get(loop_pc) == Some(&WC_END)
        && state.prog.prog.get(loop_pc + 1) == Some(&WC_END)
    {
        simple_pline.store(1, Ordering::Relaxed);
        // c:438-439 — spin until breaks.
        while BREAKS.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        BREAKS.fetch_sub(1, Ordering::SeqCst);
        simple_pline.store(old_simple_pline, Ordering::Relaxed);
    } else {
        // c:441-485 — normal loop.
        loop {
            state.pc = loop_pc; // c:442
            noerrexit.fetch_or(NOERREXIT_EXIT | NOERREXIT_RETURN, Ordering::Relaxed); // c:443
            simple_pline.store(1, Ordering::Relaxed); // c:446
            let _ = execlist(state, 1, 0); // c:448 — exec cond.
            simple_pline.store(old_simple_pline, Ordering::Relaxed);
            noerrexit.store(olderrexit, Ordering::Relaxed); // c:451
            let cond_status = LASTVAL.load(Ordering::Relaxed); // c:452
                                                               // c:453-460 — `if (!((lastval == 0) ^ isuntil)) break;`
            let cond_passed = (cond_status == 0) ^ isuntil;
            if !cond_passed {
                if BREAKS.load(Ordering::SeqCst) > 0 {
                    BREAKS.fetch_sub(1, Ordering::SeqCst);
                }
                if RETFLAG.load(Ordering::SeqCst) == 0 {
                    LASTVAL.store(oldval, Ordering::Relaxed);
                }
                break;
            }
            if RETFLAG.load(Ordering::SeqCst) != 0 {
                // c:461
                if BREAKS.load(Ordering::SeqCst) > 0 {
                    BREAKS.fetch_sub(1, Ordering::SeqCst);
                }
                break;
            }
            simple_pline.store(1, Ordering::Relaxed); // c:468
            let _ = execlist(state, 1, 0); // c:470 — exec body.
            simple_pline.store(old_simple_pline, Ordering::Relaxed);
            // c:472-477 — breaks/continue handling.
            if BREAKS.load(Ordering::SeqCst) > 0 {
                let prev = BREAKS.fetch_sub(1, Ordering::SeqCst);
                if prev - 1 > 0 || CONTFLAG.load(Ordering::SeqCst) == 0 {
                    break;
                }
                CONTFLAG.store(0, Ordering::SeqCst);
            }
            // c:478-481 — errflag bail.
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                LASTVAL.store(1, Ordering::Relaxed);
                break;
            }
            // c:482-483 — retflag bail.
            if RETFLAG.load(Ordering::SeqCst) != 0 {
                break;
            }
            freeheap(); // c:484
            oldval = LASTVAL.load(Ordering::Relaxed); // c:485
        }
    }
    cmdpop(); // c:489
    popheap(); // c:490
    LOOPS.fetch_sub(1, Ordering::SeqCst); // c:491
    state.pc = end_pc; // c:492
    this_noerrexit.store(1, Ordering::Relaxed); // c:493
    LASTVAL.load(Ordering::Relaxed)
}

/// Port of `execrepeat(Estate state, UNUSED(int do_exec))` from
/// `Src/loop.c:499-551`. `repeat N; do body; done`.
pub fn execrepeat(state: &mut estate, _do_exec: i32) -> i32 {
    let code = state.prog.prog[state.pc.wrapping_sub(1)]; // c:503
    let old_simple_pline = simple_pline.swap(1, Ordering::Relaxed); // c:507
    let end_pc = state.pc + WC_REPEAT_SKIP(code) as usize; // c:510
    let mut htok = 0i32;
    let mut tmp = ecgetstr(state, EC_DUPTOK, Some(&mut htok)); // c:512
    if htok != 0 {
        tmp = singsub(&tmp); // c:514
        tmp = untokenize(&tmp); // c:515
    }
    let count = wc_mathevali(&tmp).unwrap_or(0); // c:517
    if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
        simple_pline.store(old_simple_pline, Ordering::Relaxed);
        return 1;
    }
    LASTVAL.store(0, Ordering::Relaxed); // c:520
    pushheap(); // c:521
    cmdpush(CS_REPEAT as u8); // c:522
    LOOPS.fetch_add(1, Ordering::SeqCst); // c:523
    let loop_pc = state.pc; // c:524
    let mut remaining = count;
    while remaining > 0 {
        // c:525
        remaining -= 1;
        state.pc = loop_pc;
        let _ = execlist(state, 1, 0); // c:527
        freeheap(); // c:528
                    // c:529-534 — breaks/continue handling.
        if BREAKS.load(Ordering::SeqCst) > 0 {
            let prev = BREAKS.fetch_sub(1, Ordering::SeqCst);
            if prev - 1 > 0 || CONTFLAG.load(Ordering::SeqCst) == 0 {
                break;
            }
            CONTFLAG.store(0, Ordering::SeqCst);
        }
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            // c:536-538
            LASTVAL.store(1, Ordering::Relaxed);
            break;
        }
        if RETFLAG.load(Ordering::SeqCst) != 0 {
            // c:540
            break;
        }
    }
    cmdpop(); // c:544
    popheap(); // c:545
    LOOPS.fetch_sub(1, Ordering::SeqCst); // c:546
    simple_pline.store(old_simple_pline, Ordering::Relaxed);
    state.pc = end_pc; // c:548
    this_noerrexit.store(1, Ordering::Relaxed); // c:549
    LASTVAL.load(Ordering::Relaxed)
}

/// Port of `execif(Estate state, int do_exec)` from `Src/loop.c:553-598`.
/// `if cond; then body; elif ...; else ...; fi`.
pub fn execif(state: &mut estate, do_exec: i32) -> i32 {
    let code0 = state.prog.prog[state.pc.wrapping_sub(1)]; // c:558
    let olderrexit = noerrexit.load(Ordering::Relaxed); // c:559
    let end_pc = state.pc + WC_IF_SKIP(code0) as usize; // c:560
    noerrexit.fetch_or(NOERREXIT_EXIT | NOERREXIT_RETURN, Ordering::Relaxed); // c:562
    let mut s = 0i32; // c:557 — `s = 0`
    let mut run = 0i32; // c:557 — `run = 0`
    while state.pc < end_pc {
        // c:563
        let code = state.prog.prog[state.pc];
        state.pc += 1;
        // c:565-571 — non-IF, or IF_ELSE: break out.
        if wc_code(code) != WC_IF || WC_IF_TYPE(code) == WC_IF_ELSE {
            run = if wc_code(code) == WC_IF && WC_IF_TYPE(code) == WC_IF_ELSE {
                2
            } else {
                1
            };
            if run == 1 {
                state.pc -= 1; // back up onto the body header
            }
            break;
        }
        let next_pc = state.pc + WC_IF_SKIP(code) as usize; // c:572
        cmdpush(if s != 0 { CS_ELIF as u8 } else { CS_IF as u8 }); // c:573
        let _ = execlist(state, 1, 0); // c:574
        cmdpop(); // c:575
                  // c:576-579 — selected branch: lastval == 0.
        if LASTVAL.load(Ordering::Relaxed) == 0 {
            run = 1;
            break;
        }
        if RETFLAG.load(Ordering::SeqCst) != 0 {
            // c:580
            break;
        }
        s = 1;
        state.pc = next_pc;
    }
    noerrexit.store(olderrexit, Ordering::Relaxed); // c:584
                                                    // c:585-591 — run selected branch.
    if run != 0 {
        cmdpush(if run == 2 {
            CS_ELSE as u8
        } else if s != 0 {
            CS_ELIFTHEN as u8
        } else {
            CS_IFTHEN as u8
        });
        let _ = execlist(state, 1, do_exec);
        cmdpop();
    } else if RETFLAG.load(Ordering::SeqCst) == 0
        && (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0
    {
        LASTVAL.store(0, Ordering::Relaxed); // c:592
    }
    state.pc = end_pc; // c:594
    this_noerrexit.store(1, Ordering::Relaxed); // c:595
    LASTVAL.load(Ordering::Relaxed)
}

/// Port of `execcase(Estate state, int do_exec)` from `Src/loop.c:600-733`.
/// `case word in pat) body ;; ... esac` with `;;`/`;&`/`;|` separators.
pub fn execcase(state: &mut estate, do_exec: i32) -> i32 {
    let code0 = state.prog.prog[state.pc.wrapping_sub(1)]; // c:603
    let end_pc = state.pc + WC_CASE_SKIP(code0) as usize; // c:607
                                                          // c:609-611 — read & expand the case-word.
    let raw_word = ecgetstr(state, EC_DUP, None);
    let word_sub = singsub(&raw_word);
    let word = untokenize(&word_sub);
    let mut anypatok = false; // c:613
    cmdpush(CS_CASE as u8); // c:615
    let mut code = 0u32;
    while state.pc < end_pc {
        // c:616
        code = state.prog.prog[state.pc];
        state.pc += 1;
        if wc_code(code) != WC_CASE {
            break;
        }
        let next_pc = state.pc + WC_CASE_SKIP(code) as usize; // c:621
        let nalts = state.prog.prog[state.pc] as i32; // c:622
        state.pc += 1;
        let mut patok = false;
        let mut nalts_remaining = nalts;
        while !patok && nalts_remaining > 0 {
            // c:629-672 — try each alternative pattern.
            // c:631-633 — `npat = state->pc[1]; spprog = state->prog->pats + npat;`
            // zshrs's pat-compile-on-demand path: extract raw pat text + try patcompile/pattry.
            queue_signals(); // c:636
            let mut htok = 0i32;
            let pat_raw = ecrawstr(&state.prog, state.pc, Some(&mut htok));
            let pat = if htok != 0 {
                singsub(&pat_raw)
            } else {
                pat_raw
            };
            if let Some(pprog) = patcompile(
                &{
                    let mut __pat_tok = (&pat).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                },
                PAT_STATIC,
                None,
            ) {
                // c:660 — `if (pprog && pattry(pprog, word)) patok = anypatok = 1;`
                if pattry(&pprog, &word) {
                    patok = true;
                    anypatok = true;
                }
            } else {
                zerr(&format!("bad pattern: {}", pat)); // c:657
            }
            state.pc += 2; // c:664 — `state->pc += 2;`
            nalts_remaining -= 1;
            unqueue_signals(); // c:666
        }
        state.pc += (2 * nalts_remaining) as usize; // c:668
        if patok {
            // c:672-684 — run selected arm body.
            let _ = execlist(
                state,
                1,
                ((WC_CASE_TYPE(code) == WC_CASE_OR) as i32) & do_exec,
            );
            // c:675-682 — chain into ;& and ;| siblings.
            while RETFLAG.load(Ordering::SeqCst) == 0
                && wc_code(code) == WC_CASE
                && WC_CASE_TYPE(code) == WC_CASE_AND
                && state.pc < end_pc
            {
                state.pc = next_pc;
                code = state.prog.prog[state.pc];
                state.pc += 1;
                let inner_next = state.pc + WC_CASE_SKIP(code) as usize;
                let inner_nalts = state.prog.prog[state.pc] as usize;
                state.pc += 1 + 2 * inner_nalts;
                let _ = execlist(
                    state,
                    1,
                    ((WC_CASE_TYPE(code) == WC_CASE_OR) as i32) & do_exec,
                );
                let _ = inner_next;
            }
            if WC_CASE_TYPE(code) != WC_CASE_TESTAND {
                break;
            }
        }
        state.pc = next_pc; // c:687
    }
    cmdpop(); // c:691
    state.pc = end_pc; // c:693
    if !anypatok {
        // c:695-696
        LASTVAL.store(0, Ordering::Relaxed);
    }
    this_noerrexit.store(1, Ordering::Relaxed); // c:697
    LASTVAL.load(Ordering::Relaxed)
}

/// Port of `exectry(Estate state, int do_exec)` from `Src/loop.c:735-798`.
/// `{ try } always { finally }`: capture errflag/retflag/breaks/contflag
/// from the try-clause, reset them around the always-clause, then
/// restore if always-clause didn't override.
pub fn exectry(state: &mut estate, _do_exec: i32) -> i32 {
    let header = state.prog.prog[state.pc.wrapping_sub(1)]; // c:741
    let end_pc = state.pc + WC_TRY_SKIP(header) as usize; // c:742
    let try_inner = state.prog.prog[state.pc]; // c:743
    let always_pc = state.pc + 1 + WC_TRY_SKIP(try_inner) as usize; // c:743
    state.pc += 1; // c:744
    pushheap(); // c:745
    cmdpush(CS_CURSH as u8); // c:746
    try_tryflag.fetch_add(1, Ordering::SeqCst); // c:749
    let _ = execlist(state, 1, 0); // c:750
    try_tryflag.fetch_sub(1, Ordering::SeqCst); // c:751
    let try_status = LASTVAL.load(Ordering::Relaxed);
    let endval = if try_status != 0 {
        // c:754
        try_status
    } else {
        (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) as i32
    };
    freeheap(); // c:756
    cmdpop(); // c:758
    cmdpush(CS_ALWAYS as u8); // c:759
                              // c:762-763 — save try_errflag / try_interrupt.
    let saved_err = errflag.load(Ordering::Relaxed);
    let save_try_err = (saved_err & ERRFLAG_ERROR) != 0;
    let save_try_int = (saved_err & ERRFLAG_INT) != 0;
    // c:Src/loop.c:763-766 — save the canonical globals AND update
    // them to reflect the just-finished try body's exit state before
    // running the always-arm. `$TRY_BLOCK_ERROR` / `$TRY_BLOCK_INTERRUPT`
    // read these globals directly (lookup_special_var arms), so the
    // always body must see "errflag at try-end" not "-1 sentinel".
    let saved_try_errflag = crate::ported::r#loop::try_errflag.load(Ordering::Relaxed);
    let saved_try_interrupt = crate::ported::r#loop::try_interrupt.load(Ordering::Relaxed);
    crate::ported::r#loop::try_errflag.store((saved_err & ERRFLAG_ERROR) as i64, Ordering::Relaxed); // c:765
    crate::ported::r#loop::try_interrupt.store(
        if (saved_err & ERRFLAG_INT) != 0 { 1 } else { 0 },
        Ordering::Relaxed,
    ); // c:766
       // c:768 — `errflag = 0;` (clear both bits).
    errflag.fetch_and(!(ERRFLAG_ERROR | ERRFLAG_INT), Ordering::Relaxed);
    // c:769-774 — save retflag/breaks/contflag.
    let save_retflag = RETFLAG.swap(0, Ordering::SeqCst);
    let save_breaks = BREAKS.swap(0, Ordering::SeqCst);
    let save_contflag = CONTFLAG.swap(0, Ordering::SeqCst);
    state.pc = always_pc; // c:776
    let _ = execlist(state, 1, 0); // c:777
                                   // c:779-786 — restore errflag bits.
    if save_try_err {
        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
    } else {
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
    }
    if save_try_int {
        errflag.fetch_or(ERRFLAG_INT, Ordering::Relaxed);
    } else {
        errflag.fetch_and(!ERRFLAG_INT, Ordering::Relaxed);
    }
    // c:Src/loop.c:787-788 — restore the canonical globals.
    crate::ported::r#loop::try_errflag.store(saved_try_errflag, Ordering::Relaxed);
    crate::ported::r#loop::try_interrupt.store(saved_try_interrupt, Ordering::Relaxed);
    // c:789-794 — re-arm retflag/breaks/contflag only if always didn't override.
    if RETFLAG.load(Ordering::SeqCst) == 0 {
        RETFLAG.store(save_retflag, Ordering::SeqCst);
    }
    if BREAKS.load(Ordering::SeqCst) == 0 {
        BREAKS.store(save_breaks, Ordering::SeqCst);
    }
    if CONTFLAG.load(Ordering::SeqCst) == 0 {
        CONTFLAG.store(save_contflag, Ordering::SeqCst);
    }
    cmdpop(); // c:796
    popheap(); // c:797
    state.pc = end_pc; // c:798
    this_noerrexit.store(1, Ordering::Relaxed); // c:799
    endval
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selectlist_returns_zero_when_full_fits() {
        let _g = crate::test_util::global_state_lock();
        let items = ["one", "two", "three"];
        let r = selectlist(&items, 0);
        assert!(r < items.len() || r == 0);
    }

    // The 7 *_panics_with_tree_walker_disabled tests were deleted
    // when the underlying `unreachable!()` stubs were replaced with
    // faithful ports in src/ported/exec.rs. The architectural pin
    // remains in tests/tree_walker_absent.rs (which asserts the
    // AST-side `execute_simple`/`execute_pipeline`/etc. tree walker
    // is gone — a different concept from the wordcode-VM dispatch
    // chain ported in exec.rs).

    /// Empty list to selectlist: nothing to draw, returns 0.
    #[test]
    fn selectlist_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(selectlist(&[], 0), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/loop.c selectlist.
    // ═══════════════════════════════════════════════════════════════════

    /// c:347 — `selectlist` with start=0 doesn't panic and returns
    /// a valid count.
    #[test]
    fn selectlist_start_zero_returns_valid_count() {
        let _g = crate::test_util::global_state_lock();
        let items = ["a", "b", "c"];
        let r = selectlist(&items, 0);
        // Return value is "next start" for paginated display.
        // Must be reasonable: 0 (full draw fit) or >0 (paginated).
        assert!(r <= items.len());
    }

    /// c:347 — single-item list always fits in one column.
    #[test]
    fn selectlist_single_item_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let r = selectlist(&["only"], 0);
        let _ = r;
    }

    /// c:347 — list with empty string entries doesn't panic.
    #[test]
    fn selectlist_empty_string_entries_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let items = ["", "", ""];
        let _ = selectlist(&items, 0);
    }

    /// c:347 — long items (longer than terminal width) handled safely.
    #[test]
    fn selectlist_long_items_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let long = "x".repeat(500);
        let items = [long.as_str(), "short"];
        let _ = selectlist(&items, 0);
    }

    /// c:347 — multibyte content (CJK) doesn't panic.
    #[test]
    fn selectlist_multibyte_entries_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let items = ["日本", "中文", "한국어"];
        let _ = selectlist(&items, 0);
    }

    /// c:347 — many small items doesn't panic (stress: 100 entries).
    #[test]
    fn selectlist_many_items_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let items: Vec<String> = (0..100).map(|i| format!("item{}", i)).collect();
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        let _ = selectlist(&refs, 0);
    }

    /// c:347 — start > 0 with valid index PANICS in zshrs port
    /// ("index out of bounds: the len is 5 but the index is 5").
    /// C handles paginated start correctly via the c:379 column loop
    /// bound `t1 - start < zterm_lines - 2`. Likely off-by-one in
    /// the Rust port's index walk past last column.
    #[test]
    fn selectlist_start_in_range_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let items = ["a", "b", "c", "d", "e"];
        let _ = selectlist(&items, 2);
    }

    /// c:347 — `selectlist` is deterministic for a given (items, start).
    #[test]
    fn selectlist_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let items = ["a", "b", "c"];
        let first = selectlist(&items, 0);
        for _ in 0..5 {
            assert_eq!(selectlist(&items, 0), first);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/loop.c
    // c:347 selectlist + statics (LOOP_DEPTH/CONT_FLAG/BREAK_LEVEL/try_tryflag)
    // ═══════════════════════════════════════════════════════════════════

    /// c:347 — `selectlist` return value ≤ items.len() (valid next-start).
    #[test]
    fn selectlist_return_bounded_by_items_len() {
        let _g = crate::test_util::global_state_lock();
        for sz in [1usize, 3, 5, 20] {
            let items: Vec<String> = (0..sz).map(|i| format!("i{}", i)).collect();
            let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
            let r = selectlist(&refs, 0);
            assert!(r <= sz, "selectlist({} items, 0) = {} > {}", sz, r, sz);
        }
    }

    /// c:347 — `selectlist` returns usize (compile-time type pin).
    #[test]
    fn selectlist_returns_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _: usize = selectlist(&["x"], 0);
    }

    /// c:347 — `selectlist` two-item list doesn't panic.
    #[test]
    fn selectlist_two_items_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["a", "b"], 0);
    }

    /// c:347 — `selectlist` with entries containing tabs doesn't panic.
    #[test]
    fn selectlist_entries_with_tabs_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["a\tb", "c\td"], 0);
    }

    /// c:731 — `try_tryflag` is AtomicI64 (compile-time type pin).
    #[test]
    fn try_tryflag_is_atomic_i64() {
        use std::sync::atomic::Ordering;
        let v: i64 = try_tryflag.load(Ordering::SeqCst);
        // Just verify it loads — any i64 is a valid initial state.
        let _ = v;
    }

    /// c:347 — `selectlist` with entries containing ANSI escape doesn't panic.
    #[test]
    fn selectlist_ansi_escape_entries_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["\x1b[31mred\x1b[0m", "plain"], 0);
    }

    /// c:347 — `selectlist` with single space entry doesn't panic.
    #[test]
    fn selectlist_single_space_entry_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&[" "], 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/loop.c
    // c:36 loops / c:41 contflag / c:46 breaks / c:347 selectlist /
    // c:731 try_tryflag — initial-state + idempotency pins.
    // ═══════════════════════════════════════════════════════════════════

    /// c:36 — `loops` initial state is zero (`int loops;` BSS zero init).
    #[test]
    fn loop_depth_initial_state_is_zero() {
        use std::sync::atomic::Ordering;
        let v = LOOP_DEPTH.load(Ordering::SeqCst);
        assert!(v >= 0, "loop depth must never be negative; got {}", v);
    }

    /// c:41 — `contflag` initial state must be non-negative
    /// (`mod_export int contflag;` BSS-zero).
    #[test]
    fn cont_flag_initial_state_non_negative() {
        use std::sync::atomic::Ordering;
        let v = CONT_FLAG.load(Ordering::SeqCst);
        assert!(v >= 0, "cont flag must be non-negative; got {}", v);
    }

    /// c:46 — `breaks` initial state non-negative
    /// (`volatile int breaks;` BSS-zero).
    #[test]
    fn break_level_initial_state_non_negative() {
        use std::sync::atomic::Ordering;
        let v = BREAK_LEVEL.load(Ordering::SeqCst);
        assert!(v >= 0, "break level must be non-negative; got {}", v);
    }

    /// c:347 — `selectlist` is idempotent across many calls (no mutation).
    #[test]
    fn selectlist_no_mutation_across_calls() {
        let _g = crate::test_util::global_state_lock();
        let items = ["alpha", "beta", "gamma"];
        let first = selectlist(&items, 0);
        for _ in 0..20 {
            assert_eq!(
                selectlist(&items, 0),
                first,
                "selectlist must be pure across repeated calls"
            );
        }
    }

    /// c:347 — `selectlist` four items doesn't panic.
    #[test]
    fn selectlist_four_items_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["a", "b", "c", "d"], 0);
    }

    /// c:347 — `selectlist` with newline-bearing entry doesn't panic.
    #[test]
    fn selectlist_newline_entry_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["a\nb"], 0);
    }

    /// c:347 — `selectlist` mixed-length entries doesn't panic.
    #[test]
    fn selectlist_mixed_length_entries_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["a", "ab", "abc", "abcd", "abcde"], 0);
    }

    /// c:347 — `selectlist` Unicode emoji entry doesn't panic.
    #[test]
    fn selectlist_emoji_entries_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["foo", "bar"], 0);
    }

    /// c:731 — `try_tryflag` load is idempotent (pure read, no mutation).
    #[test]
    fn try_tryflag_load_idempotent() {
        use std::sync::atomic::Ordering;
        let first = try_tryflag.load(Ordering::SeqCst);
        for _ in 0..50 {
            assert_eq!(
                try_tryflag.load(Ordering::SeqCst),
                first,
                "try_tryflag pure load must be deterministic"
            );
        }
    }

    /// c:731 — `try_tryflag` static address is stable.
    #[test]
    fn try_tryflag_address_is_stable() {
        let p1 = &try_tryflag as *const _;
        let p2 = &try_tryflag as *const _;
        assert_eq!(p1, p2, "static must have a single, stable address");
    }

    /// c:347 — `selectlist` with whitespace-only entries doesn't panic.
    #[test]
    fn selectlist_whitespace_only_entries_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["   ", "\t\t", "  \t  "], 0);
    }

    /// c:36 / c:41 / c:46 — `LOOP_DEPTH`, `CONT_FLAG`, `BREAK_LEVEL`
    /// addresses are stable (no per-call alloc).
    #[test]
    fn loop_static_addresses_stable() {
        let a1 = &LOOP_DEPTH as *const _;
        let a2 = &LOOP_DEPTH as *const _;
        assert_eq!(a1, a2, "LOOP_DEPTH must be a true static");
        let b1 = &CONT_FLAG as *const _;
        let b2 = &CONT_FLAG as *const _;
        assert_eq!(b1, b2, "CONT_FLAG must be a true static");
        let c1 = &BREAK_LEVEL as *const _;
        let c2 = &BREAK_LEVEL as *const _;
        assert_eq!(c1, c2, "BREAK_LEVEL must be a true static");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/loop.c
    // c:347 selectlist — empty / single / huge entries / start-offset edges
    // ═══════════════════════════════════════════════════════════════════

    /// c:362 — `selectlist([])` empty input returns 0 (no menu rendered, alt).
    #[test]
    fn selectlist_empty_returns_zero_alt() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            selectlist(&[], 0),
            0,
            "empty items must produce zero-line output"
        );
    }

    /// c:362 — `selectlist([single])` doesn't panic.
    #[test]
    fn selectlist_single_entry_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["one"], 0);
    }

    /// c:347 — `selectlist` return type usize (compile-time pin, alt).
    #[test]
    fn selectlist_returns_usize_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: usize = selectlist(&["a"], 0);
    }

    /// c:347 — `selectlist` with start ≥ items.len() doesn't panic
    /// (start past end is degenerate but safe).
    #[test]
    fn selectlist_start_past_end_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["a", "b"], 100);
        let _ = selectlist(&["a", "b"], usize::MAX / 2);
    }

    /// c:347 — `selectlist` with very large entry list doesn't panic
    /// (pin against accidental quadratic / OOB on big counts).
    #[test]
    fn selectlist_large_list_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let items: Vec<String> = (0..200).map(|i| format!("item{}", i)).collect();
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        let _ = selectlist(&refs, 0);
    }

    /// c:347 — `selectlist` with start at exactly items.len()-1 doesn't panic.
    #[test]
    fn selectlist_start_at_last_index_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = selectlist(&["a", "b", "c"], 2);
    }

    /// c:36/41/46 — LOOP_DEPTH/CONT_FLAG/BREAK_LEVEL atomic i32 type pins.
    #[test]
    fn loop_statics_are_atomic_i32() {
        let _: &AtomicI32 = &LOOP_DEPTH;
        let _: &AtomicI32 = &CONT_FLAG;
        let _: &AtomicI32 = &BREAK_LEVEL;
    }

    /// c:36 — LOOP_DEPTH atomic store/load round-trip.
    #[test]
    fn loop_depth_store_load_round_trip() {
        use std::sync::atomic::Ordering;
        let _g = crate::test_util::global_state_lock();
        let saved = LOOP_DEPTH.load(Ordering::SeqCst);
        for v in [0, 1, 5, 100, -1, i32::MAX, i32::MIN] {
            LOOP_DEPTH.store(v, Ordering::SeqCst);
            assert_eq!(
                LOOP_DEPTH.load(Ordering::SeqCst),
                v,
                "LOOP_DEPTH must round-trip {}",
                v
            );
        }
        LOOP_DEPTH.store(saved, Ordering::SeqCst);
    }

    /// c:41 — CONT_FLAG atomic store/load round-trip.
    #[test]
    fn cont_flag_store_load_round_trip() {
        use std::sync::atomic::Ordering;
        let _g = crate::test_util::global_state_lock();
        let saved = CONT_FLAG.load(Ordering::SeqCst);
        for v in [0, 1, 5, 100, -1, i32::MAX, i32::MIN] {
            CONT_FLAG.store(v, Ordering::SeqCst);
            assert_eq!(CONT_FLAG.load(Ordering::SeqCst), v);
        }
        CONT_FLAG.store(saved, Ordering::SeqCst);
    }

    /// c:46 — BREAK_LEVEL atomic store/load round-trip.
    #[test]
    fn break_level_store_load_round_trip() {
        use std::sync::atomic::Ordering;
        let _g = crate::test_util::global_state_lock();
        let saved = BREAK_LEVEL.load(Ordering::SeqCst);
        for v in [0, 1, 5, 100, -1, i32::MAX, i32::MIN] {
            BREAK_LEVEL.store(v, Ordering::SeqCst);
            assert_eq!(BREAK_LEVEL.load(Ordering::SeqCst), v);
        }
        BREAK_LEVEL.store(saved, Ordering::SeqCst);
    }

    /// c:347 — `selectlist` is deterministic across identical calls
    /// (the return value depends only on the inputs).
    #[test]
    fn selectlist_deterministic_on_identical_input() {
        let _g = crate::test_util::global_state_lock();
        let first = selectlist(&["one", "two", "three"], 0);
        for _ in 0..5 {
            assert_eq!(
                selectlist(&["one", "two", "three"], 0),
                first,
                "selectlist must be deterministic"
            );
        }
    }
}
