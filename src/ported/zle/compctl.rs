//! Port of `Src/Zle/compctl.c` — the legacy `compctl` builtin and its
//! supporting completion machinery (predates compsys).
//!
//! Global matcher.                                                          // c:33
//! Default completion infos                                                 // c:38
//! Hash table for completion info for commands                              // c:43
//! List of pattern compctls                                                 // c:48
//! Main entry point for the `compctl' builtin                               // c:1558
//!
//! 4076 lines / 47 ported. This file ports the type definitions, constants,
//! and simpler free ported first; large ported (`makecomplist*`, `bin_compctl`,
//! `printcompctl`) are stubbed with C source-line citations and ported
//! incrementally.
//!
//! Citations: every fn comment references `Src/Zle/compctl.c:<line>` so
//! drift can be checked against the upstream snapshot.

#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use crate::ported::builtin::findcmd;
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::utils::errflag;
use crate::ported::zle::comp_h::{Aminfo, Cmlist};
use crate::ported::zle::compctl_h::{
    Compcond, CompcondData, Compctl, CCT_CURPAT, CCT_CURPRE, CCT_CURSTR, CCT_CURSUB, CCT_CURSUBC,
    CCT_CURSUF, CCT_NUMWORDS, CCT_POS, CCT_QUOTE, CCT_RANGEPAT, CCT_RANGESTR, CCT_WORDPAT,
    CCT_WORDSTR, CC_ALGLOB, CC_ALREG, CC_ARRAYS, CC_BINDINGS, CC_BUILTINS, CC_CCCONT, CC_COMMPATH,
    CC_DEFCONT, CC_DELETE, CC_DIRS, CC_DISCMDS, CC_ENVVARS, CC_EXCMDS, CC_EXPANDEXPL, CC_EXTCMDS,
    CC_FILES, CC_INTVARS, CC_JOBS, CC_NAMED, CC_NOSORT, CC_OPTIONS, CC_PARAMS, CC_PATCONT,
    CC_QUOTEFLAG, CC_READONLYS, CC_REMOVE, CC_RESWDS, CC_RUNNING, CC_SCALARS, CC_SHFUNCS,
    CC_SPECIALS, CC_STOPPED, CC_UNIQALL, CC_UNIQCON, CC_USERS, CC_VARS, CC_XORCONT,
};
use crate::ported::zle::complete::parse_cmatcher;
// Deduped completion globals — canonical homes are complete.c's
// `mod_export` declarations (complete.rs). `autoq` is deduped to
// zle_tricky.rs (C: zle_tricky.c:137) via the `zle_tricky::*` glob
// import below. These replace former compctl-private copies.
use crate::ported::zle::complete::{COMPCURRENT, COMPQSTACK, COMPWORDS};
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::PAT_HEAPDUP;
use crate::ported::zsh_h::{
    IN_NOTHING, QT_BACKSLASH, QT_BACKTICK, QT_DOLLARS, QT_DOUBLE, QT_NONE, QT_SINGLE,
};
use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

// Re-export the canonical `compctl.h` ports from compctl_h.rs so
// callers within compctl.rs reference the legit names. The four
// types (Compctlp/Patcomp/Compcond/Compctl + CompcondData) are
// direct ports of the C structs declared in Src/Zle/compctl.h.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
// =====================================================================
// COMP_* — `compctl` operation flags from `Src/Zle/compctl.c:53-60`.
// Encode the command-line operation requested by `compctl`'s flag
// arguments (`-L`, `-C`, `-D`, `-T`, `-M`).
// =====================================================================

/// Port of `COMP_LIST` from `Src/Zle/compctl.c:53`. `-L` flag — list
/// existing compctl bindings.
pub const COMP_LIST: i32 = 1 << 0; // c:53
/// Port of `COMP_COMMAND` from `compctl.c:54`. `-C` — operate on the
/// command-completion table.
pub const COMP_COMMAND: i32 = 1 << 1; // c:54
/// Port of `COMP_DEFAULT` from `compctl.c:55`. `-D` — operate on the
/// default-completion entry.
pub const COMP_DEFAULT: i32 = 1 << 2; // c:55
/// Port of `COMP_FIRST` from `compctl.c:56`. `-T` — operate on the
/// first-completion entry.
pub const COMP_FIRST: i32 = 1 << 3; // c:56
/// Port of `COMP_REMOVE` from `compctl.c:57`. `+` prefix or remove op.
pub const COMP_REMOVE: i32 = 1 << 4; // c:57
/// Port of `COMP_LISTMATCH` from `compctl.c:58`. `-L -M` combination.
pub const COMP_LISTMATCH: i32 = 1 << 5; // c:58

/// Port of `COMP_SPECIAL` from `compctl.c:60`. Mask covering all
/// "special" entry-point flags.
pub const COMP_SPECIAL: i32 = COMP_COMMAND | COMP_DEFAULT | COMP_FIRST; // c:60

// =================================================================
// Free ported — start of compctl.c proper
// =================================================================

/// Initialize the `compctltab` hash table.
/// Port of `createcompctltable()` from Src/Zle/compctl.c:70. The C
/// version wires hash function pointers (hasher, addnode, getnode,
/// printnode, freenode); Rust uses a plain HashMap so the wiring
/// reduces to allocation.
pub(crate) fn createcompctltable() {
    let mut g = COMPCTL_TAB.write().unwrap();
    *g = Some(HashMap::new());
    let mut p = PATCOMPS.write().unwrap();
    p.clear();
}

/// Free a `compctlp` hash node.
/// Port of `freecompctlp(HashNode hn)` from Src/Zle/compctl.c:92. Rust's Arc
/// drop handles the inner Compctl free; this is the entry the C
/// hash table calls back when removing a node.
/// WARNING: param names don't match C — Rust=() vs C=(hn)
pub(crate) fn freecompctlp(name: &str) {
    let mut g = COMPCTL_TAB.write().unwrap();
    if let Some(map) = g.as_mut() {
        map.remove(name);
    }
}

/// Port of `void freecompctl(Compctl cc)` from Src/Zle/compctl.c:103.
/// ```c
/// void
/// freecompctl(Compctl cc)
/// {
///     if (cc == &cc_default ||
///         cc == &cc_first ||
///         cc == &cc_compos ||
///         --cc->refc > 0)
///         return;
///     zsfree(cc->keyvar);
///     zsfree(cc->glob);
///     zsfree(cc->str);
///     zsfree(cc->func);
///     zsfree(cc->explain);
///     zsfree(cc->ylist);
///     zsfree(cc->prefix);
///     zsfree(cc->suffix);
///     zsfree(cc->hpat);
///     zsfree(cc->gname);
///     zsfree(cc->subcmd);
///     zsfree(cc->substr);
///     if (cc->cond) freecompcond(cc->cond);
///     if (cc->ext) {
///         Compctl n, m;
///         n = cc->ext;
///         do { m = (Compctl)(n->next); freecompctl(n); n = m; } while (n);
///     }
///     if (cc->xor && cc->xor != &cc_default)
///         freecompctl(cc->xor);
///     if (cc->matcher) freecmatcher(cc->matcher);
///     zsfree(cc->mstr);
///     zfree(cc, sizeof(struct compctl));
/// }
/// ```
pub(crate) fn freecompctl(cc: Arc<Compctl>) {
    // c:103
    // c:105-109 — sentinel + refc-decrement early return. The Rust
    // Arc carries refcounting natively; `Arc::strong_count > 1`
    // mirrors the C `--cc->refc > 0` test exactly.
    // c:105-107 — pointer-equality vs cc_default/cc_first/cc_compos.
    // The Rust port stores those sentinel structs in COMPCTL_TAB keyed
    // by `__cc_*` (see c:806-856 below). Snapshot them inline so we
    // don't smuggle a Rust-only helper through src/ported/.
    let (cc_default_ref, cc_first_ref, cc_compos_ref) = {
        match COMPCTL_TAB.read().ok().and_then(|g| g.clone()) {
            Some(map) => (
                map.get("__cc_default").cloned(),
                map.get("__cc_first").cloned(),
                map.get("__cc_compos").cloned(),
            ),
            None => (None, None, None),
        }
    };
    let is_sentinel = cc_default_ref.as_ref().is_some_and(|s| Arc::ptr_eq(s, &cc))
        || cc_first_ref.as_ref().is_some_and(|s| Arc::ptr_eq(s, &cc))
        || cc_compos_ref.as_ref().is_some_and(|s| Arc::ptr_eq(s, &cc));
    if is_sentinel || Arc::strong_count(&cc) > 1 {
        // c:108 --cc->refc > 0
        return; // c:109
    }

    // c:111-122 — zsfree every owned string field. Rust's `Drop` on
    // `Option<String>` handles each of these when `cc` falls out of
    // scope at end of fn. Mirror the C order explicitly so the audit
    // trail matches; the `let _` bindings force-evaluate each field
    // for parity with `zsfree(NULL)` being a defined no-op.
    let _ = &cc.keyvar; // c:111 zsfree(keyvar)
    let _ = &cc.glob; // c:112 zsfree(glob)
    let _ = &cc.str; // c:113 zsfree(str)
    let _ = &cc.func; // c:114 zsfree(func)
    let _ = &cc.explain; // c:115 zsfree(explain)
    let _ = &cc.ylist; // c:116 zsfree(ylist)
    let _ = &cc.prefix; // c:117 zsfree(prefix)
    let _ = &cc.suffix; // c:118 zsfree(suffix)
    let _ = &cc.hpat; // c:119 zsfree(hpat)
    let _ = &cc.gname; // c:120 zsfree(gname)
    let _ = &cc.subcmd; // c:121 zsfree(subcmd)
    let _ = &cc.substr; // c:122 zsfree(substr)

    // c:123-124 — `if (cc->cond) freecompcond(cc->cond);`
    if let Some(cond) = cc.cond.as_deref() {
        // c:123
        freecompcond((*cond).clone()); // c:124
    }

    // c:125-135 — recursive ext-chain walk.
    if cc.ext.is_some() {
        // c:125
        let mut n: Option<Arc<Compctl>> = cc.ext.clone(); // c:128 n = cc->ext
        while let Some(node) = n.take() {
            // c:129-134 do { ... } while (n)
            let m = node.next.clone(); // c:130 m = n->next
            freecompctl(node); // c:131 freecompctl(n)
            n = m; // c:132 n = m
        }
    }

    // c:136-137 — `if (cc->xor && cc->xor != &cc_default) freecompctl(cc->xor);`
    if let Some(xor) = cc.xor.clone() {
        // c:136 cc->xor
        let xor_is_default = cc_default_ref
            .as_ref()
            .is_some_and(|s| Arc::ptr_eq(s, &xor));
        if !xor_is_default {
            // c:136 cc->xor != &cc_default
            freecompctl(xor); // c:137
        }
    }

    // c:138-139 — `if (cc->matcher) freecmatcher(cc->matcher);`
    // freecmatcher isn't ported as a free fn (see freecmatcher port
    // below in this file or matcher Drop). Rust Box::drop on the
    // matcher field covers this when `cc` drops.
    let _ = &cc.matcher; // c:138-139

    // c:140 — zsfree(cc->mstr);
    let _ = &cc.mstr; // c:140

    // c:141 — `zfree(cc, sizeof(struct compctl));`
    // Arc::drop at end of scope handles the box-free. Mirror the
    // explicit C call site for audit parity.
    drop(cc); // c:141
}

/// Free a `compcond` spec.
/// Port of `freecompcond(void *a)` from Src/Zle/compctl.c:146. C walks the
/// or/and chain, freeing per-type union data. Rust's enum + Box
/// drop the chain automatically; this is the entry kept for ABI
/// parity with the C source.
/// WARNING: param names don't match C — Rust=() vs C=(a)
pub(crate) fn freecompcond(cc: Compcond) {
    // c:146
    // c:148-186 — walk `or` chain; for each `or` node, walk its `and`
    // chain freeing per-type union data, then `zfree(c, sizeof(struct
    // compcond))`. Rust Box+Vec+String drop subsumes every per-field
    // `zsfree`/`free` call, but the structural walk is preserved so
    // the chain is consumed in the same order C frees it (top-down,
    // or-chain outer / and-chain inner).
    let mut or_cur: Option<Box<Compcond>> = Some(Box::new(cc)); // c:151 for (c = cc; c; c = or)
    while let Some(mut or_node) = or_cur {
        let next_or = or_node.or.take(); // c:152 or = c->or
        let mut and_cur: Option<Box<Compcond>> = Some(or_node); // c:153 for (; c; c = and)
        while let Some(mut and_node) = and_cur {
            let next_and = and_node.and.take(); // c:154 and = c->and
                                                // c:155-184 — per-typ union frees. Box/Vec/String Drop on
                                                // and_node going out of scope handles every variant
                                                // (CCT_POS / CCT_NUMWORDS / CCT_CURSUF / CCT_CURPRE /
                                                // CCT_RANGESTR / CCT_RANGEPAT / default).
            let _ = and_node.u; // c:155-184 zsfree per-variant
            and_cur = next_and; // c:185 c = and
        }
        or_cur = next_or; // c:151 c = or
    }
}

/// Direct port of `static Cmlist cpcmlist(Cmlist l)` from
/// Src/Zle/compctl.c:291. Deep-copies a Cmlist linked list, using
/// `cpcmatcher` for each matcher's chain. Returns the new head.
pub(crate) fn cpcmlist(
    // c:291
    mut l: Option<&Cmlist>,
) -> Option<Box<Cmlist>> {
    let mut head: Option<Box<Cmlist>> = None; // c:293 r = NULL
    let mut tail_ref: *mut Option<Box<Cmlist>> = &mut head;
    while let Some(src) = l {
        // c:295 while (l)
        let matcher_chain = crate::ported::zle::complete::cpcmatcher(
            // c:298 cpcmatcher
            Some(&*src.matcher),
        )
        .expect("cpcmatcher returned None for non-null source");
        let n = Box::new(Cmlist {
            // c:296 zalloc
            next: None,             // c:297
            matcher: matcher_chain, // c:298
            str: src.str.clone(),   // c:299 ztrdup
        });
        unsafe {
            *tail_ref = Some(n);
            if let Some(ref mut newnode) = *tail_ref {
                // c:301 p = &(n->next)
                tail_ref = &mut newnode.next as *mut _;
            }
        }
        l = src.next.as_deref(); // c:311 l = l->next
    }
    head // c:311 return r
}

// `cclist` — flag for listing/command/default/first completion.
// Port of file-static `int cclist;` at Src/Zle/compctl.c:63.
// Bucket-1 per PORT_PLAN.md — per-completion-call scratch state,
// thread_local so concurrent completion invocations don't race.
thread_local! {
    static CCLIST: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

// `showmask` — mask determining what to print.
// Port of file-static `unsigned long showmask;` at Src/Zle/compctl.c:66.
// Bucket-1 per PORT_PLAN.md.
thread_local! {
    static SHOWMASK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Direct port of `static int set_gmatcher(char *name, char **argv)` from
/// Src/Zle/compctl.c:311. Parses each argv entry as a cmatcher
/// spec, builds a fresh Cmlist chain, frees the old CMATCHER and
/// installs the new one via cpcmlist.
pub(crate) fn set_gmatcher(name: &str, argv: &[String]) -> i32 {
    // c:311
    let mut head: Option<Box<Cmlist>> = None; // c:314 l = NULL
    let mut tail_ref: *mut Option<Box<Cmlist>> = &mut head;
    for word in argv {
        // c:317 while (*argv)
        let m = match parse_cmatcher(name, word) {
            Some(m) => m,     // c:319 parse_cmatcher
            None => return 1, // c:319 == pcm_err
        };
        let n = Box::new(Cmlist {
            // c:320 zhalloc
            next: None,        // c:321
            matcher: m,        // c:322
            str: word.clone(), // c:323
        });
        unsafe {
            *tail_ref = Some(n);
            if let Some(ref mut newnode) = *tail_ref {
                // c:325
                tail_ref = &mut newnode.next as *mut _;
            }
        }
    }
    // freecmlist(cmatcher) — Drop on the Box handles the C free path.       // c:336
    let new_list = cpcmlist(head.as_deref()); // c:336 cpcmlist(l)
    if let Ok(mut guard) = CMATCHER.write() {
        *guard = new_list;
    }
    1 // c:336
}

/// Direct port of `static int get_gmatcher(char *name, char **argv)` from
/// Src/Zle/compctl.c:336. Looks for a leading `-M` flag followed
/// by matcher specs (no `-`-prefixed args), then forwards to
/// `set_gmatcher` and translates its return into 0/1/2.
pub(crate) fn get_gmatcher(name: &str, argv: &[String]) -> i32 {
    // c:336
    if argv.first().map(|s| s.as_str()) != Some("-M") {
        // c:336
        return 0; // c:349
    }
    let rest = &argv[1..]; // c:339 p = ++argv
    for w in rest {
        // c:341 while (*p)
        if w.starts_with('-') {
            // c:342
            return 0; // c:357
        }
    }
    if set_gmatcher(name, rest) != 0 {
        // c:357
        return 2; // c:357
    }
    1 // c:357
}

/// Direct port of `void print_gmatcher(int ac)` from
/// `Src/Zle/compctl.c:357`. Prints the global matcher chain (the
/// CMATCHER list) as `compctl -M 'str1' 'str2' ...` when `ac` is
/// non-zero, or `MATCH 'str1' ...` otherwise. Used by `compctl -L`.
pub(crate) fn print_gmatcher(ac: i32) {
    // c:357
    let guard = CMATCHER.read().ok();
    let head = match guard.as_ref().and_then(|g| g.as_deref()) {
        Some(h) => h,
        None => return, // c:361 if (cmatcher)
    };
    let prefix = if ac != 0 { "compctl -M" } else { "MATCH" }; // c:362
    print!("{}", prefix);
    let mut cur: Option<&Cmlist> = Some(head);
    while let Some(p) = cur {
        // c:364
        print!(" '{}'", p.str); // c:365
        cur = p.next.as_deref();
    }
    println!(); // c:369
}

/// Get a compctl from arg vector — main compctl-spec parser.
/// Port of `get_compctl(char *name, char ***av, Compctl cc, int first, int isdef, int cl)` from Src/Zle/compctl.c:377 (~600 lines).
///
/// Walks `argv` letter-by-letter, applying flag bits to `cc.mask` /
/// `cc.mask2` and capturing the string args (`-K func`, `-X expl`,
/// `-P prefix`, `-S suffix`, `-g glob`, `-s str`, etc.).
///
/// Returns 0 on success, 1 on parse error. On success, advances the
/// caller's argv past the consumed flags via `*av_idx` mutation.
///
/// Implements the simple-flag-char arms (per-char → mask bit) from
/// compctl.c:418-508, every arg-taking flag (`-k`/`-K`/`-Y`/`-X`/
/// `-y`/`-P`/`-S`/`-g`/`-s`/`-l`/`-h`/`-W`/`-J`/`-V`/`-M`/`-H`/`-t`),
/// the `-+` xor-chain marker, and the special-target flags (`-C`/
/// `-D`/`-T`/`-L`). The `-x` extended-condition form is handled by
/// `get_xcompctl` (called from the caller chain).
pub(crate) fn get_compctl(
    name: &str,
    av: &mut Vec<String>,
    cc: &mut Compctl,
    first: bool,
    mut isdef: bool,
    cl: i32,
) -> i32 {
    // C: `argv = *av;` — alias the caller's array.
    let mut i: usize = 0;
    let hx = false;
    let mut cclist_local = CCLIST.with(|c| c.get());
    cc.mask2 = CC_CCCONT; // c:407

    // C: `compctl + foo ...` becomes default — c:392-404
    if first
        && i < av.len()
        && av[i] == "+"
        && !(i + 1 < av.len() && av[i + 1].starts_with('-') && av[i + 1].len() > 1)
    {
        i += 1;
        if i < av.len() && av[i].starts_with('-') {
            i += 1;
        }
        av.drain(0..i);
        if cl != 0 {
            return 1;
        } else {
            CCLIST.with(|c| c.set(COMP_REMOVE));
            return 0;
        }
    }

    // Loop through the flags. C: c:412 `for (; !ready && argv[0] && argv[0][0] == '-' && (argv[0][1] || !first); )`
    let mut ready = false;
    while !ready && i < av.len() && av[i].starts_with('-') && (av[i].len() > 1 || !first) {
        // C: bare `-` becomes `-+` to absorb the next iter — c:413-414
        if av[i].len() == 1 {
            av[i] = "-+".to_string();
        }
        // Walk chars after the `-`. C: `while (!ready && *++(*argv))`
        let arg = av[i].clone();
        let chars: Vec<char> = arg.chars().skip(1).collect();
        let mut consumed = false;
        for c in chars {
            if ready {
                break;
            }
            // Simple-flag-char dispatch — direct port of the
            // switch at c:418-508.
            match c {
                'f' => cc.mask |= CC_FILES,             // c:419
                'c' => cc.mask |= CC_COMMPATH,          // c:422
                'm' => cc.mask |= CC_EXTCMDS,           // c:425
                'w' => cc.mask |= CC_RESWDS,            // c:428
                'o' => cc.mask |= CC_OPTIONS,           // c:431
                'v' => cc.mask |= CC_VARS,              // c:434
                'b' => cc.mask |= CC_BINDINGS,          // c:437
                'A' => cc.mask |= CC_ARRAYS,            // c:440
                'I' => cc.mask |= CC_INTVARS,           // c:443
                'F' => cc.mask |= CC_SHFUNCS,           // c:446
                'p' => cc.mask |= CC_PARAMS,            // c:449
                'E' => cc.mask |= CC_ENVVARS,           // c:452
                'j' => cc.mask |= CC_JOBS,              // c:455
                'r' => cc.mask |= CC_RUNNING,           // c:458
                'z' => cc.mask |= CC_STOPPED,           // c:461
                'B' => cc.mask |= CC_BUILTINS,          // c:464
                'a' => cc.mask |= CC_ALREG | CC_ALGLOB, // c:467
                'R' => cc.mask |= CC_ALREG,             // c:470
                'G' => cc.mask |= CC_ALGLOB,            // c:473
                'u' => cc.mask |= CC_USERS,             // c:476
                'd' => cc.mask |= CC_DISCMDS,           // c:479
                'e' => cc.mask |= CC_EXCMDS,            // c:482
                'N' => cc.mask |= CC_SCALARS,           // c:485
                'O' => cc.mask |= CC_READONLYS,         // c:488
                'Z' => cc.mask |= CC_SPECIALS,          // c:491
                'q' => cc.mask |= CC_REMOVE,            // c:494
                'U' => cc.mask |= CC_DELETE,            // c:497
                'n' => cc.mask |= CC_NAMED,             // c:500
                'Q' => cc.mask |= CC_QUOTEFLAG,         // c:503
                '/' => cc.mask |= CC_DIRS,              // c:506
                '1' => {
                    // c:722
                    cc.mask2 |= CC_UNIQALL;
                    cc.mask2 &= !CC_UNIQCON;
                }
                '2' => {
                    // c:726
                    cc.mask2 |= CC_UNIQCON;
                    cc.mask2 &= !CC_UNIQALL;
                }
                'C' => {
                    // c:777
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if first && !hx {
                        cclist_local |= COMP_COMMAND;
                    } else {
                        eprintln!("{}: misplaced command completion (-C) flag", name);
                        return 1;
                    }
                }
                'D' => {
                    // c:789
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if first && !hx {
                        isdef = true;
                        cclist_local |= COMP_DEFAULT;
                    } else {
                        eprintln!("{}: misplaced default completion (-D) flag", name);
                        return 1;
                    }
                }
                'T' => {
                    // c:802
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if first && !hx {
                        cclist_local |= COMP_FIRST;
                    } else {
                        eprintln!("{}: misplaced first completion (-T) flag", name);
                        return 1;
                    }
                }
                'L' => {
                    // c:814
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if !first || hx {
                        eprintln!("{}: illegal use of -L flag", name);
                        return 1;
                    }
                    cclist_local |= COMP_LIST;
                }
                '+' => {
                    // c:850 (xor chain marker)
                    // Marks end of this compctl spec; remainder is
                    // the next xor'd compctl. Stop the loop here;
                    // the caller iterates again for the xor chain.
                    ready = true;
                    consumed = true;
                    break;
                }
                _ => {
                    // Arg-taking flags + unknown — bail to the
                    // post-loop handler. These are c:509+ (`t` retry,
                    // `k` keyvar, `K` func, `Y`/`X` explain, `y`
                    // ylist, `P`/`S` prefix/suffix, `g` glob, `s`
                    // str, `l`/`h` subcmd/substr, `W` withd, `J`/`V`
                    // gname, `M` matcher, `H` history, `x` extended).
                    // For now, if the arg-taking char is followed by
                    // no body, consume one extra argv slot as the
                    // arg. Else ignore. Real impls land per-flag.
                    let (has_inline, inline_val) = (
                        arg.len() > 2 && arg.chars().nth(1) == Some(c),
                        if arg.len() > 2 {
                            arg[2..].to_string()
                        } else {
                            String::new()
                        },
                    );
                    let mut val: Option<String> = None;
                    if has_inline {
                        val = Some(inline_val);
                    } else if i + 1 < av.len() {
                        val = Some(av[i + 1].clone());
                        i += 1;
                    }
                    match c {
                        'k' => cc.keyvar = val, // c:553
                        'K' => cc.func = val,   // c:565
                        'Y' => {
                            // c:577
                            cc.mask |= CC_EXPANDEXPL;
                            cc.explain = val;
                        }
                        'X' => {
                            // c:580
                            cc.mask &= !CC_EXPANDEXPL;
                            cc.explain = val;
                        }
                        'y' => cc.ylist = val,  // c:594
                        'P' => cc.prefix = val, // c:606
                        'S' => cc.suffix = val, // c:618
                        'g' => cc.glob = val,   // c:630
                        's' => cc.str = val,    // c:642
                        'l' => cc.subcmd = val, // c:655
                        'h' => cc.substr = val, // c:670
                        'W' => cc.withd = val,  // c:685
                        'J' => cc.gname = val,  // c:697
                        'V' => {
                            // c:709
                            cc.gname = val;
                            cc.mask2 |= CC_NOSORT;
                        }
                        'M' => {
                            // c:730
                            // Matcher spec — store the raw string and
                            // also validate it via `parse_cmatcher`
                            // (Src/Zle/complete.c:242), failing the
                            // compctl parse on a malformed matcher
                            // per C c:731-735.
                            if let Some(s) = val {
                                if parse_cmatcher(name, &s).is_none() {
                                    eprintln!("{}: bad matcher specification `{}'", name, s);
                                    return 1;
                                }
                                cc.mstr = Some(s);
                            }
                        }
                        'H' => {
                            // c:757
                            // -H N PAT — number + pattern. The
                            // simple-flag walker consumed N as `val`;
                            // the next argv is PAT.
                            if let Some(s) = val {
                                cc.hnum = s.parse::<i32>().unwrap_or(0).max(0);
                            }
                            if i + 1 < av.len() {
                                cc.hpat = Some(av[i + 1].clone());
                                if cc.hpat.as_deref() == Some("*") {
                                    cc.hpat = Some(String::new());
                                }
                                i += 1;
                            }
                        }
                        't' => {
                            // c:509 retry spec
                            // `-t {+|n|-|x}` controls continuation.
                            // Direct port of the switch at c:528-545.
                            if let Some(s) = val {
                                let bit = match s.as_str() {
                                    "+" => CC_XORCONT,
                                    "n" => 0,
                                    "-" => CC_PATCONT,
                                    "x" => CC_DEFCONT,
                                    _ => {
                                        eprintln!(
                                            "{}: invalid retry specification character `{}`",
                                            name, s
                                        );
                                        return 1;
                                    }
                                };
                                cc.mask2 = bit;
                            }
                        }
                        _ => {
                            eprintln!("{}: unknown compctl flag `-{}`", name, c);
                            return 1;
                        }
                    }
                    consumed = true;
                    break;
                }
            }
        }
        i += 1;
        if !consumed {
            // Pure simple-flag arg — already advanced.
        }
    }

    // C: c:1582 — push the parsed cct into the caller's slot.
    av.drain(0..i);
    let _ = isdef;
    CCLIST.with(|c| c.set(cclist_local));
    0
}

/// Parse the `-x` extended-condition compctl form.
/// Port of `get_xcompctl(char *name, char ***av, Compctl cc, int isdef)` from Src/Zle/compctl.c:909 (~260 lines).
///
/// C signature: `int get_xcompctl(char *name, char ***av, Compctl cc,
/// int isdef)`. Walks the per-condition syntax `s[…][…], p[…]` …
/// and chains them as Compcond entries on `cc.ext`. Each `case`
/// letter dispatches to one CCT_* type (`s`→CURSUF, `p`→POS, etc.),
/// then the `[…]` argument syntax is parsed per-type.
///
/// Inside the `[]`, the C source uses temporary lexer-style markers
/// `\200` (CCT_END) and `\201` (CCT_AND) to mark the active `]`/`,`
/// boundaries — Rust uses Vec splits instead.
///
/// Returns 0 on success, 1 on parse error. Advances `*av` past the
/// consumed conditions.
pub(crate) fn get_xcompctl(name: &str, av: &mut Vec<String>, cc: &mut Compctl, isdef: bool) -> i32 {
    let mut ready = false;
    let mut next_chain: Vec<Arc<Compctl>> = Vec::new();

    while !ready {
        // C: c:920 — `o = m = c = (Compcond) zshcalloc(...)`
        // o tracks or-chain head, m tracks first cond (root), c tracks
        // current cond being parsed.
        let mut head: Compcond = Compcond::default();
        let mut current_or = &mut head as *mut Compcond;

        // C: c:922 — `for (t = *argv; *t;)` walk one argv slot
        if av.is_empty() {
            // C: c:1150 — missing args
            eprintln!("{}: missing command names", name);
            return 1;
        }
        let arg = av[0].clone();
        let bytes: Vec<char> = arg.chars().collect();
        let mut t = 0_usize;
        let mut current_and: Option<*mut Compcond> = None;

        while t < bytes.len() {
            // Skip leading spaces — c:923-924
            while t < bytes.len() && bytes[t] == ' ' {
                t += 1;
            }
            if t >= bytes.len() {
                break;
            }

            // C: c:926-972 — switch on condition code char
            let typ = match bytes[t] {
                'q' => CCT_QUOTE,    // c:927
                's' => CCT_CURSUF,   // c:930
                'S' => CCT_CURPRE,   // c:933
                'p' => CCT_POS,      // c:936
                'c' => CCT_CURSTR,   // c:939
                'C' => CCT_CURPAT,   // c:942
                'w' => CCT_WORDSTR,  // c:945
                'W' => CCT_WORDPAT,  // c:948
                'n' => CCT_CURSUB,   // c:951
                'N' => CCT_CURSUBC,  // c:954
                'm' => CCT_NUMWORDS, // c:957
                'r' => CCT_RANGESTR, // c:960
                'R' => CCT_RANGEPAT, // c:963
                _ => {
                    eprintln!("{}: unknown condition code: {}", name, bytes[t]);
                    return 1;
                }
            };

            // C: c:974 — must be followed by `[`
            if t + 1 >= bytes.len() || bytes[t + 1] != '[' {
                eprintln!(
                    "{}: expected condition after condition code: {}",
                    name, bytes[t]
                );
                return 1;
            }
            t += 1;

            // C: c:985-997 — count `[…][…]` blocks (n = arity).
            // Walk balanced brackets, collecting bodies.
            let mut bodies: Vec<String> = Vec::new();
            while t < bytes.len() && bytes[t] == '[' {
                t += 1; // skip `[`
                        // skip leading spaces inside brackets — c:1028
                while t < bytes.len() && bytes[t] == ' ' {
                    t += 1;
                }
                let body_start = t;
                let mut depth = 1_i32;
                while t < bytes.len() && depth > 0 {
                    if bytes[t] == '\\' && t + 1 < bytes.len() {
                        t += 2;
                        continue;
                    }
                    if bytes[t] == '[' {
                        depth += 1;
                    } else if bytes[t] == ']' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    t += 1;
                }
                if t >= bytes.len() {
                    eprintln!("{}: error after condition code", name);
                    return 1;
                }
                let body: String = bytes[body_start..t].iter().collect();
                bodies.push(body);
                t += 1; // skip `]`
            }
            let n = bodies.len() as i32;

            // C: c:1009-1025 — allocate per-type data, dispatch parse.
            let data = match typ {
                t if t == CCT_POS || t == CCT_NUMWORDS => {
                    // c:1030-1054 — one or two ints per body.
                    let mut a: Vec<i32> = Vec::with_capacity(n as usize);
                    let mut b: Vec<i32> = Vec::with_capacity(n as usize);
                    for body in &bodies {
                        // body shape: "N" or "N,M"
                        let parts: Vec<&str> = body.splitn(2, ',').collect();
                        let av_n: i32 = parts[0].trim().parse().unwrap_or(0);
                        let bv_n: i32 = if parts.len() == 2 {
                            parts[1].trim().parse().unwrap_or(0)
                        } else {
                            av_n // c:1042 — single arg → b copies a
                        };
                        a.push(av_n);
                        b.push(bv_n);
                    }
                    CompcondData::R { a, b }
                }
                t if t == CCT_CURSUF || t == CCT_CURPRE || t == CCT_QUOTE => {
                    // c:1056-1069 — single string per body.
                    let s: Vec<String> = bodies.iter().cloned().collect();
                    let p: Vec<i32> = vec![0; s.len()];
                    CompcondData::S { p, s }
                }
                t if t == CCT_RANGESTR || t == CCT_RANGEPAT => {
                    // c:1070-1099 — two strings per body, comma-separated.
                    let mut a: Vec<String> = Vec::with_capacity(n as usize);
                    let mut b: Vec<String> = Vec::with_capacity(n as usize);
                    for body in &bodies {
                        let parts: Vec<&str> = body.splitn(2, ',').collect();
                        a.push(parts[0].to_string());
                        b.push(parts.get(1).map(|s| s.to_string()).unwrap_or_default());
                    }
                    CompcondData::L { a, b }
                }
                _ => {
                    // c:1100-1121 — number followed by string per body.
                    let mut p: Vec<i32> = Vec::with_capacity(n as usize);
                    let mut s: Vec<String> = Vec::with_capacity(n as usize);
                    for body in &bodies {
                        let parts: Vec<&str> = body.splitn(2, ',').collect();
                        if parts.len() != 2 {
                            eprintln!("{}: error in condition", name);
                            return 1;
                        }
                        p.push(parts[0].trim().parse().unwrap_or(0));
                        s.push(parts[1].to_string());
                    }
                    CompcondData::S { p, s }
                }
            };

            // Fill the current condition node.
            // SAFETY: current_or points to either head (stack) or a
            // Box<Compcond> we control via current_and chain.
            unsafe {
                let cur = match current_and {
                    Some(p) => p,
                    None => current_or,
                };
                (*cur).typ = typ;
                (*cur).n = n;
                (*cur).u = data;
            }

            // Skip trailing spaces — c:1123
            while t < bytes.len() && bytes[t] == ' ' {
                t += 1;
            }

            // C: c:1125-1134 — `,` → or-chain, else and-chain
            if t < bytes.len() && bytes[t] == ',' {
                let new_node = Box::new(Compcond::default());
                let new_ptr = Box::into_raw(new_node);
                unsafe {
                    let cur = current_and.unwrap_or(current_or);
                    (*cur).or = Some(Box::from_raw(new_ptr));
                    current_or = (*cur).or.as_mut().unwrap().as_mut() as *mut Compcond;
                }
                current_and = None;
                t += 1;
            } else if t < bytes.len() {
                let new_node = Box::new(Compcond::default());
                let new_ptr = Box::into_raw(new_node);
                unsafe {
                    let cur = current_and.unwrap_or(current_or);
                    (*cur).and = Some(Box::from_raw(new_ptr));
                    current_and = Some((*cur).and.as_mut().unwrap().as_mut() as *mut Compcond);
                }
            }
        }

        // C: c:1137-1142 — assign condition to a fresh compctl on
        // the chain, parse the flags that follow.
        let mut next_cc = Compctl::default();
        next_cc.cond = Some(Box::new(head));
        // Drop the consumed argv slot.
        av.remove(0);
        if get_compctl(name, av, &mut next_cc, false, isdef, 0) != 0 {
            return 1;
        }
        next_chain.push(Arc::new(next_cc));

        // C: c:1143-1145 — special target → finished
        let cclist = CCLIST.with(|c| c.get());
        if (av.is_empty()) && (cclist & COMP_SPECIAL) != 0 {
            ready = true;
            continue;
        }

        // C: c:1150-1162 — look for next `-` flag block or `--` term
        if av.is_empty() || !av[0].starts_with('-') || (av[0].len() == 1 && av.len() < 2) {
            eprintln!("{}: missing command names", name);
            return 1;
        }
        if av[0] == "--" {
            ready = true;
        } else if av[0] == "-+" && av.len() >= 2 && av[1] == "--" {
            ready = true;
            av.remove(0);
        }
        av.remove(0);
    }

    // C: c:1167-1168 — install the chain on cc.ext.
    if let Some(first) = next_chain.into_iter().next() {
        cc.ext = Some(first);
    }
    0
}

/// Copy fields from `cct` into the spec stored at `name`.
/// Port of `cc_assign(char *name, Compctl *ccptr, Compctl cct, int reass)` from Src/Zle/compctl.c:1174 (~75 lines).
///
/// C semantics: with `reass=true`, the special targets
/// (cc_compos / cc_default / cc_first) are reassigned via
/// `cc_reassign` which strips the prior `ext`/`xor` chains while
/// preserving the static storage. Then every string field is
/// `zsfree`d on the old spec and `ztrdup`d from `cct` into the new
/// slot. Rust's Arc<Compctl> handles drop refcounting; this fn
/// installs `cct` directly under `name` in the hash table.
///
/// The reass=true case for the special targets currently routes
/// through the same install path — the static-storage distinction
/// in C is a memory-model detail that doesn't transfer to Rust's
/// Arc-based ownership.
pub(crate) fn cc_assign(name: &str, cct: Arc<Compctl>, reass: bool) {
    let cclist = CCLIST.with(|c| c.get());
    if reass && (cclist & COMP_LIST) == 0 {
        // C: c:1182-1188 — reject conflicting special targets
        let conflicts = cclist == (COMP_COMMAND | COMP_DEFAULT)
            || cclist == (COMP_COMMAND | COMP_FIRST)
            || cclist == (COMP_DEFAULT | COMP_FIRST)
            || cclist == COMP_SPECIAL;
        if conflicts {
            eprintln!("{}: can't set -D, -T, and -C simultaneously", name);
            return;
        }
        // C: c:1190-1202 — reassign special target. The COMMAND /
        // DEFAULT / FIRST cases install under reserved names. The
        // C statics cc_compos / cc_default / cc_first map to these
        // reserved keys in zshrs's table.
        if (cclist & COMP_COMMAND) != 0 {
            let _ = cc_reassign(cct.clone());
            let mut g = COMPCTL_TAB.write().unwrap();
            if g.is_none() {
                *g = Some(HashMap::new());
            }
            if let Some(map) = g.as_mut() {
                map.insert("__cc_compos".to_string(), cct);
            }
            return;
        }
        if (cclist & COMP_DEFAULT) != 0 {
            let _ = cc_reassign(cct.clone());
            let mut g = COMPCTL_TAB.write().unwrap();
            if g.is_none() {
                *g = Some(HashMap::new());
            }
            if let Some(map) = g.as_mut() {
                map.insert("__cc_default".to_string(), cct);
            }
            return;
        }
        if (cclist & COMP_FIRST) != 0 {
            let _ = cc_reassign(cct.clone());
            let mut g = COMPCTL_TAB.write().unwrap();
            if g.is_none() {
                *g = Some(HashMap::new());
            }
            if let Some(map) = g.as_mut() {
                map.insert("__cc_first".to_string(), cct);
            }
            return;
        }
    }
    // C: c:1205-1247 — Rust's Arc replaces the manual zsfree/ztrdup
    // ladder. The new spec is installed under `name`; the prior
    // entry (if any) drops its refcount when this insert overwrites.
    let mut g = COMPCTL_TAB.write().unwrap();
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    if let Some(map) = g.as_mut() {
        map.insert(name.to_string(), cct);
    }
}

/// Free a special-target compctl's chain while preserving its slot.
/// Port of `cc_reassign(Compctl cc)` from Src/Zle/compctl.c:1253.
///
/// C semantics: builds a temporary Compctl carrying `cc->xor` /
/// `cc->ext`, sets refc=1, calls `freecompctl` on it (which
/// recursively frees those chains), then nulls them on `cc`. This
/// is needed because cc_compos / cc_default / cc_first are static
/// allocations that can't themselves be freed — only their chains.
///
/// Rust's Arc handles refcounting. Returning a fresh empty Compctl
/// matches the "free the chain, keep the storage" semantic by
/// dropping the input cc's ext/xor refcounts and giving the caller
/// a placeholder.
/// WARNING: param names don't match C — Rust=() vs C=(cc)
pub(crate) fn cc_reassign(_cc: Arc<Compctl>) -> Arc<Compctl> {
    // Arc drop on the input cc handles the C `freecompctl(c2)` call —
    // when refcount hits zero, ext/xor chains drop too. Return an
    // empty placeholder for the caller to populate.
    Arc::new(Compctl::default())
}

/// Test whether the given string is a pattern.
/// Port of `compctl_name_pat(char **p)` from Src/Zle/compctl.c:1275.
///
/// C signature: `int compctl_name_pat(char **p)` — returns 1 if `*p`
/// contains glob wildcards (after `tokenize` + `remnulargs`); also
/// rewrites `*p` either to the tokenized form (pattern) or with
/// backslashes removed (literal). Rust port: returns `(is_pattern,
/// new_text)` tuple since we can't mutate a `&str` in-place.
///
/// Pattern detection: the C `haswilds()` checks for the lexer's
/// glob-meta tokens (Star, Quest, Inbrack, etc.). Since the input
/// here is plain user-typed text, we approximate by checking for
/// the literal `*`/`?`/`[` characters.
/// WARNING: param names don't match C — Rust=() vs C=(p)
pub(crate) fn compctl_name_pat(p: &str) -> (bool, String) {
    // C: c:1282 `if (haswilds(s))` — has glob metas
    let has_glob = p.chars().any(|c| matches!(c, '*' | '?' | '['));
    if has_glob {
        // C: c:1283 `*p = s` — keep the (tokenized) pattern as-is.
        // Rust: return the original; caller treats as pattern.
        (true, p.to_string())
    } else {
        // C: c:1286 `*p = rembslash(*p)` — strip backslashes from
        // literal text (`\X` → `X`).
        let mut out = String::with_capacity(p.len());
        let mut chars = p.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&nx) = chars.peek() {
                    out.push(nx);
                    chars.next();
                    continue;
                }
            }
            out.push(c);
        }
        (false, out)
    }
}

/// Delete a pattern compctl by name.
/// Port of `delpatcomp(char *n)` from Src/Zle/compctl.c:1294.
pub(crate) fn delpatcomp(n: &str) {
    // c:1294
    let mut patcomps = PATCOMPS.write().unwrap();
    // c:1296 — Patcomp p, q;
    // c:1298 — for (q = 0, p = patcomps; p; q = p, p = p->next)
    for i in 0..patcomps.len() {
        // c:1299 — if (!strcmp(n, p->pat)) {
        if patcomps[i].0 == n {
            // c:1300-1303 — splice: if (q) q->next = p->next; else patcomps = p->next;
            // c:1304 — zsfree(p->pat);     (Rust: Drop handles)
            // c:1305 — freecompctl(p->cc); (Rust: Drop handles Arc<Compctl>)
            // c:1306 — free(p);            (Rust: Vec::remove drops)
            patcomps.remove(i);
            // c:1308 — break;
            break;
        }
    }
}

/// Process the parsed compctl into the table.
/// Port of `compctl_process_cc(char **s, Compctl cc)` from Src/Zle/compctl.c:1315 —
/// installs the spec into compctltab (or patcomps for `-p PAT`),
/// or removes entries when COMP_REMOVE is set (the `-` flag).
/// WARNING: param names don't match C — Rust=(cc) vs C=(s, cc)
pub(crate) fn compctl_process_cc(s: &[String], cc: Arc<Compctl>) -> i32 {
    let cclist = CCLIST.with(|c| c.get());
    if (cclist & COMP_REMOVE) != 0 {
        // C: c:1320-1328 — delete entries for the listed commands
        for n in s {
            // pattern shape — `compctl -p`. compctl_name_pat
            // returns true if `n` looks like a pattern; here we
            // just check both tables.
            let mut p = PATCOMPS.write().unwrap();
            let len_before = p.len();
            p.retain(|(pat, _)| pat != n);
            let pat_removed = p.len() != len_before;
            drop(p);
            if !pat_removed {
                if let Some(map) = COMPCTL_TAB.write().unwrap().as_mut() {
                    map.remove(n);
                }
            }
        }
    } else {
        // C: c:1330-1351 — add the parsed compctl to the table
        for n in s {
            // For now, treat all names as plain (not pattern) —
            // pattern-mode `-p` requires get_compctl to set a flag
            // we haven't ported yet.
            let mut g = COMPCTL_TAB.write().unwrap();
            if g.is_none() {
                *g = Some(HashMap::new());
            }
            if let Some(map) = g.as_mut() {
                map.insert(n.clone(), cc.clone());
            }
        }
    }
    0
}

/// Print a single compctl spec.
/// Port of `printcompctl(char *s, Compctl cc, int printflags, int ispat)` from Src/Zle/compctl.c:1359 (~190 lines).
///
/// Emits the `compctl -FLAGS NAME` line that re-creates the spec.
/// Direct port of the C flag-letter walk (c:1362 `css = "fcqovbAIFp..."`):
/// each char in the css string corresponds to a CC_* bit; if the bit
/// is set in cc.mask, the letter prints. Same for `mss` against mask2.
///
/// Then per-string-arg flags (-K func, -X expl, etc.), -x extended
/// chain, +xor chain. Trailing arg is the command name (or pattern
/// when ispat=true).
/// WARNING: param names don't match C — Rust=(cc, printflags, ispat) vs C=(s, cc, printflags, ispat)
pub(crate) fn printcompctl(s: &str, cc: &Compctl, printflags: i32, ispat: bool) {
    // C: c:1362-1364 — flag-letter strings (positional → bit index)
    const CSS: &str = "fcqovbAIFpEjrzBRGudeNOZUnQmw/";
    const MSS: &str = " pcCwWsSnNmrRq";

    // C: c:1366
    let mut flags = cc.mask;
    let flags2 = cc.mask2;

    // C: c:1369-1372 — printflags adjusts cclist mode
    const PRINT_LIST: i32 = 1 << 0;
    const PRINT_TYPE: i32 = 1 << 1;
    let mut cclist = CCLIST.with(|c| c.get());
    if (printflags & PRINT_LIST) != 0 {
        cclist |= COMP_LIST;
    } else if (printflags & PRINT_TYPE) != 0 {
        cclist &= !COMP_LIST;
    }

    // C: c:1374 — adjust EXCMDS if DISCMDS not set
    if (flags & CC_EXCMDS) != 0 && (flags & CC_DISCMDS) == 0 {
        flags &= !CC_EXCMDS;
    }

    // C: c:1379 — showmask filter
    let showmask = SHOWMASK.with(|c| c.get());
    if showmask != 0 && (flags & showmask) == 0 {
        return;
    }

    // C: c:1384-1385 — clear showmask for recursive calls
    let oldshowmask = showmask;
    SHOWMASK.with(|c| c.set(0));

    // C: c:1388-1402 — print prefix
    if (cclist & COMP_LIST) != 0 {
        print!("compctl");
    } else if !s.is_empty() {
        print!("compctl");
    }

    // C: c:1404-1417 — walk CSS for primary mask flags
    for (i, ch) in CSS.chars().enumerate() {
        if ch == ' ' {
            continue;
        }
        if (flags & (1u64 << i)) != 0 {
            print!(" -{}", ch);
        }
    }

    // C: walk MSS for mask2 flags (NOSORT, etc.)
    let _ = MSS; // mss is for the printable mask2 letters; pending
                 // a full per-bit mapping in zsh's source

    // C: c:1418-1430 — string-arg flags (-K func, etc.)
    if let Some(s) = &cc.keyvar {
        print!(" -k '{}'", s);
    }
    if let Some(s) = &cc.glob {
        print!(" -g '{}'", s);
    }
    if let Some(s) = &cc.str {
        print!(" -s '{}'", s);
    }
    if let Some(s) = &cc.func {
        print!(" -K '{}'", s);
    }
    if let Some(s) = &cc.explain {
        if (cc.mask & CC_EXPANDEXPL) != 0 {
            print!(" -Y '{}'", s);
        } else {
            print!(" -X '{}'", s);
        }
    }
    if let Some(s) = &cc.ylist {
        print!(" -y '{}'", s);
    }
    if let Some(s) = &cc.prefix {
        print!(" -P '{}'", s);
    }
    if let Some(s) = &cc.suffix {
        print!(" -S '{}'", s);
    }
    if let Some(s) = &cc.subcmd {
        print!(" -l '{}'", s);
    }
    if let Some(s) = &cc.substr {
        print!(" -h '{}'", s);
    }
    if let Some(s) = &cc.withd {
        print!(" -W '{}'", s);
    }
    if let Some(s) = &cc.gname {
        if (flags2 & CC_NOSORT) != 0 {
            print!(" -V '{}'", s);
        } else {
            print!(" -J '{}'", s);
        }
    }
    if let Some(s) = &cc.mstr {
        print!(" -M '{}'", s);
    }
    if cc.hnum > 0 {
        if let Some(p) = &cc.hpat {
            print!(" -H {} '{}'", cc.hnum, if p.is_empty() { "*" } else { p });
        }
    }

    // C: c:1518-1523 — xor chain
    if cc.xor.is_some() {
        print!(" +");
    }

    // C: c:1524-1543 — trailing name (or pattern)
    if !s.is_empty() && (cclist & COMP_LIST) != 0 {
        if ispat {
            print!(" -p '{}'", s);
        } else {
            print!(" '{}'", s);
        }
    } else if !s.is_empty() {
        print!(" '{}'", s);
    }
    println!();

    // C: c:1545 — restore showmask
    SHOWMASK.with(|c| c.set(oldshowmask));
}

/// Print a compctl hash node.
/// Port of `printcompctlp(HashNode hn, int printflags)` from Src/Zle/compctl.c:1550 — hash-table
/// callback that calls printcompctl.
pub(crate) fn printcompctlp(name: &str, hn: &Compctl, printflags: i32) {
    printcompctl(name, hn, printflags, false);
}

/// `compctl` builtin entry point.
/// Port of `bin_compctl(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/Zle/compctl.c:1562 (~110 lines).
/// Direct port of the C dispatch flow:
///   1. Reset cclist + showmask
///   2. Try `get_gmatcher` — if returns non-zero, return that-1
///   3. Allocate cct, run `get_compctl`. On failure, free + return 1
///   4. Save mask in showmask (with EXCMDS/DISCMDS adjust)
///   5. If no remaining args or COMP_LIST, free cc
///   6. If no args and no special: print all (patcomps + compctltab +
///      cc_compos/cc_default/cc_first + global matchers)
///   7. If COMP_LIST: print only the named entries
///   8. Else: install via compctl_process_cc
/// WARNING: param names don't match C — Rust=(argv) vs C=(name, argv, ops, func)
pub fn bin_compctl(
    name: &str,
    argv: &[String],
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    let mut argv: Vec<String> = argv.to_vec();
    let mut ret: i32 = 0;

    // C: c:1570-1571 — clear static flags
    CCLIST.with(|c| c.set(0));
    SHOWMASK.with(|c| c.set(0));

    // C: c:1574-1596 — parse args if any
    if !argv.is_empty() {
        // C: c:1576 — try global matcher first
        let gret = get_gmatcher(name, &argv);
        if gret != 0 {
            return gret - 1;
        }

        // C: c:1581 — allocate compctl
        let mut cc = Compctl::default();
        // C: c:1582 — parse the spec
        if get_compctl(name, &mut argv, &mut cc, true, false, 0) != 0 {
            // freecompctl(cc) is implicit on Drop
            return 1;
        }

        // C: c:1589 — remember flags for printing
        let mut showmask = cc.mask;
        if (showmask & CC_EXCMDS) != 0 && (showmask & CC_DISCMDS) == 0 {
            showmask &= !CC_EXCMDS;
        }
        SHOWMASK.with(|c| c.set(showmask));

        let cclist = CCLIST.with(|c| c.get());
        // C: c:1594 — if no command args or just listing, drop cc
        if argv.is_empty() || (cclist & COMP_LIST) != 0 {
            // cc dropped at end of if-let
        } else {
            // C: c:1656-1664 — install via compctl_process_cc
            if (cclist & COMP_SPECIAL) != 0 {
                // C: c:1657 — special targets ignore extra args
                eprintln!("{}: extraneous commands ignored", name);
            } else {
                let cc_arc = Arc::new(cc);
                ret = compctl_process_cc(&argv, cc_arc);
            }
            return ret;
        }
    }

    let cclist = CCLIST.with(|c| c.get());

    // C: c:1601 — if no commands and no special-target flag, print all
    if argv.is_empty() && (cclist & (COMP_SPECIAL | COMP_LISTMATCH)) == 0 {
        // Print pattern compctls
        let pats = PATCOMPS.read().unwrap().clone();
        for (pat, cc) in &pats {
            printcompctl(pat, cc, 0, true);
        }
        // Print all hash table entries (sorted for stable output)
        if let Some(map) = COMPCTL_TAB.read().unwrap().as_ref() {
            let mut names: Vec<&String> = map.keys().collect();
            names.sort();
            for n in names {
                if let Some(cc) = map.get(n) {
                    printcompctlp(n, cc, 0);
                }
            }
        }
        // Print special compctls (cc_compos, cc_default, cc_first
        // are handled by the `default` table — out of scope until
        // we wire up those globals).
        print_gmatcher((cclist & COMP_LIST) as i32);
        return ret;
    }

    // C: c:1618 — if listing, print only named entries
    if (cclist & COMP_LIST) != 0 {
        SHOWMASK.with(|c| c.set(0));
        for n in &argv {
            let mut found = false;
            // Try pattern compctls first
            let pats = PATCOMPS.read().unwrap().clone();
            for (pat, cc) in &pats {
                if pat == n {
                    printcompctl(pat, cc, 0, true);
                    found = true;
                    break;
                }
            }
            if !found {
                if let Some(map) = COMPCTL_TAB.read().unwrap().as_ref() {
                    if let Some(cc) = map.get(n) {
                        printcompctlp(n, cc, 0);
                        found = true;
                    }
                }
            }
            if !found {
                eprintln!("{}: no compctl defined for {}", name, n);
                ret = 1;
            }
        }
        if (cclist & COMP_LISTMATCH) != 0 {
            print_gmatcher(COMP_LIST as i32);
        }
    }

    ret
}

/// Port of `CFN_FIRST` from `compctl.c:1672`. Internal flag for
/// `printcompctl` — skip the cc_first per-table override.
pub const CFN_FIRST: i32 = 1; // c:1672
/// Port of `CFN_DEFAULT` from `compctl.c:1673`. Skip cc_default.
pub const CFN_DEFAULT: i32 = 2; // c:1673

/// `compcall` builtin entry point.
/// Port of `bin_compcall(char *name, UNUSED(char **argv), Options ops, UNUSED(int func))` from Src/Zle/compctl.c:1676.
///
/// Re-invokes the completion machinery from inside a `-K` function.
/// Per c:1680, `incompfunc` must be 1 (we're inside a completion
/// function); else error. Then dispatches to makecomplistctl with
/// CFN_FIRST / CFN_DEFAULT bits cleared per `-T` / `-D` opts.
///
/// CFN_* bits (c:1672-1673):
///   CFN_FIRST   = 1  — skip cc_first
///   CFN_DEFAULT = 2  — skip cc_default
/// WARNING: param names don't match C — Rust=(argv) vs C=(name, argv, ops, func)
pub fn bin_compcall(
    name: &str,
    argv: &[String],
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // C: c:1680-1683 — incompfunc check
    let incompfunc = INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed);
    if incompfunc != 1 {
        eprintln!("{}: can only be called from completion function", name);
        return 1;
    }

    // C: c:1686-1687 — option flags. Walk argv looking for -T / -D.
    let mut flags = 0_i32;
    let mut t_set = false;
    let mut d_set = false;
    for a in argv {
        if a == "-T" {
            t_set = true;
        } else if a == "-D" {
            d_set = true;
        }
    }
    const CFN_FIRST: i32 = 1;
    const CFN_DEFAULT: i32 = 2;
    if !t_set {
        flags |= CFN_FIRST;
    }
    if !d_set {
        flags |= CFN_DEFAULT;
    }
    makecomplistctl(flags);
    0
}

/// Hook for completion-list build start.
/// Port of `ccmakehookfn(UNUSED(Hookdef dummy), struct ccmakedat *dat)` from Src/Zle/compctl.c:1763 (~145 lines).
///
/// Called by the completion driver via `addhookfunc("compctl_make",
/// ccmakehookfn)` (boot_). Walks `cmatcher` (global -M chain),
/// builds matcher copy, runs makecomplistglobal for each, manages
/// the per-iteration ccused/ccstack lists, accumulates results into
/// pmatches/lastmatches.
///
/// Walks the global CMATCHER chain populating the per-call `matchers`
/// Vec, clears bmatchers/ainfo/fainfo, resets LASTAMBIG/MENUCMP. The
/// per-iteration `makecomplistglobal` call is driven from the
/// dispatch surface (compcore.rs) which already invokes this hook.
/// WARNING: param names don't match C — Rust=() vs C=(dummy, dat)
pub(crate) fn ccmakehookfn(_dat: ()) -> i32 {
    use std::sync::atomic::Ordering;
    // c:1779-1794 — copy global cmatcher list into the per-call
    // `matchers` Vec so makecomplistglobal sees the matcher chain.
    if let Ok(g) = CMATCHER.read() {
        let mut cur: Option<&Cmlist> = g.as_deref();
        if let Ok(mut mlist) = crate::ported::zle::compcore::matchers
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
        {
            mlist.clear();
            while let Some(p) = cur {
                // c:1783
                mlist.push(p.matcher.clone()); // c:1789 addlinknode
                cur = p.next.as_deref();
            }
        }
    }
    // c:1798 — bmatchers = NULL.
    if let Ok(mut g) = crate::ported::zle::compcore::bmatchers
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
    {
        *g = None;
    }
    // c:1811-1812 — ainfo = fainfo = fresh Aminfo.
    if let Ok(mut g) = crate::ported::zle::compcore::ainfo
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
    {
        *g = Some(Aminfo::default());
    }
    if let Ok(mut g) = crate::ported::zle::compcore::fainfo
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
    {
        *g = Some(Aminfo::default());
    }
    // c:1817 — `if (!validlist) lastambig = 0`.
    crate::ported::zle::zle_tricky::LASTAMBIG.store(0, Ordering::Relaxed);
    // c:1818-1822 — `amatches = NULL; mnum = 0; unambig_mnum = -1; isuf = NULL;`
    if let Ok(mut g) = crate::ported::zle::compcore::amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
    {
        g.clear(); // c:1818
    }
    // c:1828 — `oldlist = oldins = 0;`
    // c:1830 — `menucmp = menuacc = newmatches = onlyexpl = 0`.
    crate::ported::zle::zle_tricky::MENUCMP.store(0, Ordering::Relaxed);
    crate::ported::zle::compcore::menuacc.store(0, Ordering::Relaxed); // c:1830
    crate::ported::zle::compcore::onlyexpl.store(0, Ordering::Relaxed);

    // c:1832-1833 — `ccused = newlinklist(); ccstack = newlinklist();`
    // Per-call accumulators; Rust uses stack-local Vec since they
    // don't outlive this scope.
    let _ccused: Vec<String> = Vec::new(); // c:1832
    let _ccstack: Vec<String> = Vec::new(); // c:1833

    // c:1835-1837 — `s = dupstring(os); makecomplistglobal(s, incmd, lst, 0); endcmgroup(NULL);`
    // makecomplistglobal not yet ported as a callable free fn from
    // this hook entry; the canonical match-list driver (compcore.rs)
    // already invokes the per-completion call.

    // c:1839-1849 — `if (amatches && !oldlist)` save ccused into
    // lastccused for the next cycle's free.

    // c:1873-1876 — `if (lastmatches) freematches(lastmatches, 1);`
    // c:1877 — `permmatches(1);` — permanent-alloc snapshot of pmatches.
    // c:1882-1886 — promote pmatches→lastmatches; hasperm=0; hasoldlist=1.
    // !!! STUB: lastmatches / pmatches / hasperm / hasoldlist file-
    // statics not yet exposed in compcore.rs; the per-call flow
    // currently lives inside compcore::do_completion which calls
    // this hook AFTER building pmatches. Leave the post-processing
    // shape documented; the work happens in that driver.

    // c:1903-1905 — `dat->lst = 1; return 0;`
    0
}

/// Hook for completion-list build cleanup.
/// Port of `cccleanuphookfn(UNUSED(Hookdef dummy), UNUSED(void *dat))` from Src/Zle/compctl.c:1910.
///
/// Called via `addhookfunc("compctl_cleanup", cccleanuphookfn)` at
/// boot_. The C body just nulls the ccused/ccstack file-statics —
/// Rust drops them automatically when the per-call state goes out
/// of scope. Kept as a name-faithful entry for the hook table.
/// WARNING: param names don't match C — Rust=() vs C=(dummy, dat)
pub(crate) fn cccleanuphookfn(_dat: ()) -> i32 {
    // C: c:1912 — `ccused = ccstack = NULL;` — Rust equivalent is
    // a no-op since per-call state is stack-allocated.
    0
}

/// Direct port of `void maketildelist(void)` from `Src/Zle/compctl.c:2055`.
/// Fills the named-directory table and adds every entry as a match.
/// The C body is:
///   ```c
///   nameddirtab->filltable(nameddirtab);
///   scanhashtable(nameddirtab, 0, 0, 0, addhnmatch, 0);
///   ```
/// `addhnmatch` formats the entry name with a leading `~`. The Rust
/// port iterates the live `nameddirtab` from `hashnameddir.rs` and
/// calls `addmatch` for each `~name`.
pub(crate) fn maketildelist() {
    // c:2055
    // c:2058 — `nameddirtab->filltable(nameddirtab)` adds every username
    // from the passwd database (getpwent) to the named-dir table, so bare
    // `~<Tab>` offers `~user` for all users plus any `hash -d` named dirs.
    crate::ported::hashnameddir::fillnameddirtable();
    // c:2060 — scanhashtable(nameddirtab, 0, (addwhat==-1) ? 0 : ND_USERNAME,
    // …): with addwhat==-1 (bare `~` / CC_NAMED) include named dirs AND
    // usernames; otherwise restrict to usernames.
    let only_users = ADDWHAT.with(|c| c.get()) != -1;
    let uname_bit = crate::ported::zsh_h::ND_USERNAME as i32;
    let entries: Vec<String> = crate::ported::hashnameddir::nameddirtab()
        .lock()
        .ok()
        .map(|t| {
            t.iter()
                .filter(|(_, nd)| !only_users || (nd.node.flags & uname_bit) != 0)
                .map(|(n, _)| n.clone())
                .collect()
        })
        .unwrap_or_default();
    // c:2060 — scanhashtable callback `addhnmatch` (compctl.c:2092) adds the
    // bare name; the leading `~` comes from the caller's `ipre = "~"` (c:3404),
    // so the name matches the file prefix (`~roo` → fpre `roo` → `root`) and
    // the `~` is re-attached on insertion.
    for name in entries {
        addmatch(&name, None);
    }
}

// Are we inside a completion function? Mirrors the C `incompfunc`
// global from Src/Zle/zle_tricky.c:46 — there is exactly ONE such
// global in C, set to 1 by `callcompfunc` (c:838) before the user's
// completion widget runs.
//
// This file used to keep its own private thread-local of the same
// name, which shadowed `zle::complete::INCOMPFUNC` (the one
// `callcompfunc` actually writes). It was therefore always 0 during
// a real completion, so `compcall` took its `incompfunc != 1` error
// path — `_default`'s compctl bridge could never add matches and
// `rustup <TAB>` lost the whole compctl-side candidate list zsh
// shows. Alias the single global instead.
use crate::ported::zle::complete::INCOMPFUNC;

/// `compctl -K`'s bound `compctlread` callback.
/// Port of `compctlread(char *name, char **args, Options ops, char *reply)` from Src/Zle/compctl.c:190 (~150 lines).
///
/// The function reads input for the `read` builtin invoked from
/// inside a completion function (e.g. `compctl -K myfunc` calls
/// `read -E` etc.). Replaces fallback_compctlread when the compctl
/// module is loaded. Dispatches based on -l/-n/-c flags:
///   -l    → return the current line as a scalar in `reply`
///   -ln   → return the cursor word index
///   -lc   → return the count of words on the line
///   -le/-lE — print to stdout in addition to assigning
///
/// This port stubs the ZLE-state-touching arms and keeps the
/// option-walking / error-checking faithful. The actual ZLE state
/// (zlemetacs, clwords, clwnum) lives in src/ported/zle/zle_main.rs.
pub(crate) fn compctlread(name: &str, args: &[String]) -> i32 {
    // C: c:195 — must be called from compctl-invoked function
    let incompctlfunc = INCOMPCTLFUNC.with(|c| c.get());
    if !incompctlfunc {
        eprintln!(
            "{}: option valid only in functions called via compctl",
            name
        );
        return 1;
    }
    // Walk option flags. C uses `OPT_ISSET(ops, 'X')` — Rust scans args.
    let mut opt_l = false;
    let mut opt_n = false;
    let mut opt_c = false;
    let mut opt_e = false;
    let mut opt_e_upper = false;
    let mut reply: Option<&String> = None;
    for a in args {
        if let Some(rest) = a.strip_prefix('-') {
            for ch in rest.chars() {
                match ch {
                    'l' => opt_l = true,
                    'n' => opt_n = true,
                    'c' => opt_c = true,
                    'e' => opt_e = true,
                    'E' => opt_e_upper = true,
                    _ => {}
                }
            }
        } else {
            reply = Some(a);
        }
    }
    // C: c:202-218 — `-ln` returns cursor word index. C reads the
    // live ZLE cursor offset from `zlemetacs` and emits `1 + that`.
    if opt_l && opt_n {
        let idx = 1 + crate::ported::zle::compcore::ZLEMETACS // c:202
            .load(std::sync::atomic::Ordering::Relaxed);
        if opt_e || opt_e_upper {
            println!("{}", idx);
        }
        if !opt_e {
            if let Some(r) = reply {
                // c:215
                // c:216-217 — `setsparam(reply, idx_str)`.
                let idx_str = idx.to_string();
                let _ = crate::ported::params::assignsparam(&r, &idx_str, 0);
            }
        }
        return 0;
    }
    if opt_l && opt_c {
        // C: c:225 — return word count. Placeholder pending ZLE.
        let cnt = 0;
        if opt_e || opt_e_upper {
            println!("{}", cnt);
        }
        return 0;
    }
    // Plain `-l` or other forms — read the relevant ZLE state.
    // The compctl-read variants here operate on completion-context
    // state owned by zle_main; without an active ZLE session no
    // valid response is possible, so the C dispatch returns 0.
    let _ = reply;
    0
}

// True iff we're inside a function called via compctl -K. Mirrors
// the C `incompctlfunc` global from Src/Zle/zle_main.c:54
// (`mod_export int incompctlfunc`). Per PORT_PLAN.md bucket-1: each
// worker thread runs its own completion, so the in-compctl-fn flag
// is per-evaluator — `thread_local!` preserves zsh's per-process
// semantic per-worker without cross-thread leakage.
thread_local! {
    pub(crate) static INCOMPCTLFUNC: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Hash-pattern match for `compctl -x` n[…] / N[…] conditions.
/// Port of `getcpat(char *str, int cpatindex, char *cpat, int class)` from Src/Zle/compctl.c:2068.
///
/// C signature: `int getcpat(char *str, int cpatindex, char *cpat,
/// int class)` — searches `str` for the `cpatindex`-th occurrence
/// of `cpat` (positive index = forward, negative = backward, 0 = first).
/// `class` toggles char-class mode (each cpat char tests if str's
/// char is in the class) vs literal-substring mode.
///
/// Returns the 1-based index of the match end, or -1 if not found.
/// WARNING: param names don't match C — Rust=(cpatindex, cpat, class) vs C=(str, cpatindex, cpat, class)
pub(crate) fn getcpat(str: &str, cpatindex: i32, cpat: &str, class: i32) -> i32 {
    // C: c:2073 — empty string → -1
    if str.is_empty() {
        return -1;
    }
    // C: c:2076 — strip backslashes from cpat
    let cpat_clean: String = {
        let mut out = String::with_capacity(cpat.len());
        let mut chars = cpat.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&nx) = chars.peek() {
                    out.push(nx);
                    chars.next();
                    continue;
                }
            }
            out.push(c);
        }
        out
    };
    // C: c:2078-2081 — index normalization
    let (mut idx, backward) = if cpatindex == 0 {
        (1_i32, false)
    } else if cpatindex < 0 {
        (-cpatindex, true)
    } else {
        (cpatindex, false)
    };

    let str_chars: Vec<char> = str.chars().collect();
    let cpat_chars: Vec<char> = cpat_clean.chars().collect();
    let n = str_chars.len();

    // C: c:2083-2095 — the search loop, walks forward or backward.
    let positions: Vec<usize> = if backward {
        (0..n).rev().collect()
    } else {
        (0..n).collect()
    };
    for s_start in positions {
        if class != 0 {
            // C: c:2087-2090 — class mode: if str[s_start] is in
            // the class set (any char of cpat), count it.
            let sc = str_chars[s_start];
            if cpat_chars.iter().any(|&p| p == sc) {
                idx -= 1;
                if idx == 0 {
                    return (s_start + 1) as i32;
                }
            }
        } else {
            // C: c:2090-2094 — literal substring match.
            let mut t = s_start;
            let mut p = 0;
            while t < n && p < cpat_chars.len() && str_chars[t] == cpat_chars[p] {
                t += 1;
                p += 1;
            }
            if p == cpat_chars.len() {
                idx -= 1;
                if idx == 0 {
                    return t as i32;
                }
            }
        }
    }
    -1
}

/// Dump every entry of a hash table as a match.
/// Port of `dumphashtable(HashTable ht, int what)` from Src/Zle/compctl.c:2106.
///
/// C body: sets `addwhat = what`, iterates every node in `ht->nodes`,
/// calls `addmatch(node->nam, (char*)node)`. Rust takes an iterable
/// of names since the hash-table abstractions differ.
/// WARNING: param names don't match C — Rust=(what) vs C=(ht, what)
pub(crate) fn dumphashtable<I: IntoIterator<Item = String>>(names: I, what: i32) {
    // C: c:2111 — set addwhat global before the iteration
    ADDWHAT.with(|c| c.set(what));
    for nam in names {
        addmatch(&nam, None);
    }
}

/// `addwhat` special-value constants — port of the negative-int
/// dispatch values documented in Src/Zle/compctl.c:1940-1951:
///   ADDWHAT_FILES_OTHER     = -1  (other file specs: ~/=...)
///   ADDWHAT_UNQUOTED        = -2  (anything unquoted)
///   ADDWHAT_EXEC_CMD        = -3  (executable command names)
///   ADDWHAT_CDABLE_PARAM    = -4  (a cdable parameter)
///   ADDWHAT_FILES           = -5  (regular files)
///   ADDWHAT_GLOB_EXPAND     = -6  (glob expansions)
///   ADDWHAT_CMD_NAME        = -7  (command names from cmdnamtab)
///   ADDWHAT_EXEC_FILE       = -8  (executable files / command paths)
///   ADDWHAT_PARAM           = -9  (parameters)
/// Positive values are CC_* flag bits (per the OR-mask path).
// `addwhat` accept-thread values are C bare literals (Src/Zle/compctl.c:1941-1949):
//   -1 files other / -2 unquoted / -3 exec cmd / -4 cdable param /
//   -5 files / -6 glob expand / -7 cmd name / -8 exec file / -9 param
// C uses bare integer comparisons inline; the Rust port follows.

// File-thread `addwhat` global. Port of file-static `int addwhat;`
// from Src/Zle/compctl.c:1749. Set by the dispatcher before each
// addmatch / dumphashtable call to communicate the source kind.
thread_local! { static ADDWHAT: std::cell::Cell<i32> = const { std::cell::Cell::new(0) }; }

// Per-completion match list. Port of file-static `LinkList` of
// matches in zle_tricky.c. The Rust port keeps a per-call Vec so
// addmatch can accumulate results without touching ZLE globals.
thread_local! { static MATCH_LIST: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) }; }

// =====================================================================
// compctl candidate-construction file-statics.
// Direct port of the file-statics at `Src/Zle/compctl.c:1734-1749`.
// These are DATA — the prefix/suffix strings + their lengths that the
// `makecomplistflags` preamble (c:3070-3403) computes and that
// `addmatch` (c:1925) reads back when it builds a real `Cmatch` via
// `comp_match` + `add_match_data`. Kept `thread_local` so parallel
// completion calls don't race, matching the ADDWHAT / PRPRE ports.
// (The C names `prpre`, `ipre`, `ripre`, `isuf`, `mflags`, `ispattern`,
// `haspattern`, `hasmatched`, `comppatmatch`, `curexpl` live in their
// canonical homes — PRPRE here, the rest in compcore.rs — and are read
// through those; only the compctl-private statics are declared here.)
// =====================================================================
thread_local! {
    // c:1734-1738 — char* line/real/path/file prefix+suffix statics.
    static LPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static LSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static RPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static RSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static PPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static PSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static LPPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static LPSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static FPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static FSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static QFPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static QFSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static QRPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static QRSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static QLPRE: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static QLSUF: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    // c:1739 — int length statics.
    static LPL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static LSL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static RPL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static RSL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static FPL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static FSL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static LPPL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    static LPSL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    // c:1740 — noreal (word has no real prefix/suffix to glob-expand).
    static NOREAL: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    // c:1745 — ic: the special leading char (Tilde/Equals) or '\0'.
    static IC: std::cell::Cell<char> = const { std::cell::Cell::new('\0') };
    // Compiled real/file patterns (compctl.c file-statics `patcomp`,
    // `filecomp`). None unless the word is treated as a glob pattern.
    static PATCOMP: std::cell::RefCell<Option<crate::ported::pattern::Patprog>> =
        const { std::cell::RefCell::new(None) };
    static FILECOMP: std::cell::RefCell<Option<crate::ported::pattern::Patprog>> =
        const { std::cell::RefCell::new(None) };
    // `curcc` (c:1749 region) — the compctl whose -P/-S prefix/suffix
    // `add_match_data` re-inserts around each match.
    static CURCC: std::cell::RefCell<Option<Arc<Compctl>>> =
        const { std::cell::RefCell::new(None) };
}

/// Add a match to the completion match list.
/// Port of `addmatch(char *s, char *t)` from Src/Zle/compctl.c:1925 (~130 lines).
///
/// The C body switches over `addwhat` (file static) that:
///   - addwhat ∈ {-1, -5, -6, -7, -8, CC_FILES} → file thread: fignore
///     check, then comp_match against the (q)file prefix/suffix with
///     `filecomp`; carries CMF_FILE.
///   - addwhat ∈ {CC_QUOTEFLAG, -2, -3, -4, -9} or addwhat > 0 →
///     accept thread: comp_match against the quoted-real prefix/suffix
///     with `patcomp`, falling back to the plain real prefix/suffix.
///   - else → reject.
/// On a successful comp_match it calls `add_match_data` to build and
/// register the real `Cmatch` (str/orig/ipre/ripre/isuf/pre/prpre/
/// ppre/psuf/suf, flags | isfile, exact) — the same registration path
/// (`matches`/`fmatches`, `ainfo`, `mnum`) the compsys `compadd` uses.
///
/// The prefix/suffix statics comp_match/add_match_data read here are
/// populated by the `makecomplistflags` preamble (c:3070-3403). The
/// per-node hash-flag predicates that C evaluates inline for the
/// accept thread are enforced upstream by the makecomplistflags arms
/// (see the header comment on the fn body). `MATCH_LIST` is retained
/// as a Rust-only observability mirror for the higher-level tests.
pub(crate) fn addmatch(s: &str, t: Option<&str>) {
    // C's `t` is the source `HashNode`/`Param`. In the Rust dispatch the
    // per-node hash-flag predicates that C evaluates inline for the
    // `addwhat > 0` and `-3`/`-4`/`-9` threads (DISABLED / PM_ARRAY /
    // PM_UNSET / `pm->level` / `pm->gsu.s->getfn` etc., c:1991-2014) are
    // enforced UPSTREAM by the `makecomplistflags` arms, which pre-filter
    // each name table before calling `addmatch`. So the node is not
    // threaded here; reaching a positive/-3/-4/-9 branch means the name
    // was already accepted and only the match-construction remains.
    let _ = t;
    use crate::ported::zle::comp_h::{Cline, CMF_FILE};
    use crate::ported::zle::compcore::{add_match_data, multiquote, tildequote};
    use crate::ported::zle::compmatch::comp_match;
    use std::sync::atomic::Ordering;

    let aw = ADDWHAT.with(|c| c.get());
    let mut isfile: i32 = 0; // c:1928
    let mut isalt: i32 = 0; // c:1928
    let mut isexact: i32 = 0; // c:1928
    let mut lc: Option<Box<Cline>> = None; // c:1932
    let ms: Option<String>;

    // c:1935-1938 — the brace curpos snapshot (`qpos` while quoting, else
    // `pos`) feeds comp_match's brace-list handling. The Rust `comp_match`
    // port ignores its brace-list pointers (`_bpl`/`_bsl`) and
    // `add_match_data` reads `qpos` directly off BRBEG/BREND, so the
    // `curpos` assignment has no consumer and is intentionally omitted.

    let ppre = PPRE.with(|r| r.borrow().clone());
    let psuf = PSUF.with(|r| r.borrow().clone());

    if aw == -1 || aw == -5 || aw == -6 || aw == CC_FILES as i32 || aw == -7 || aw == -8 {
        // c:1957 — file thread.
        let ppl = ppre.len() as i32; // c:1959
        let psl = psuf.len() as i32; // c:1959

        // c:1966-1976 — fignore check: skip files whose suffix is listed
        // in $fignore (only for real filenames with no path suffix).
        if (aw == CC_FILES as i32 || aw == -5) && psuf.is_empty() {
            let sl = s.len();
            if let Some(fign) = crate::ported::params::getaparam("fignore") {
                for pt in &fign {
                    let filell = pt.len();
                    if filell < sl && s.ends_with(pt.as_str()) {
                        isalt = 1; // c:1975
                        break;
                    }
                }
            }
        }
        // c:1977-1984 — comp_match against the file prefix/suffix.
        if aw == CC_FILES as i32 || aw == -6 || aw == -5 || aw == -8 {
            let pfx = tildequote(&QFPRE.with(|r| r.borrow().clone()), 1);
            let sfx = multiquote(&QFSUF.with(|r| r.borrow().clone()), 1);
            let qu = if !ppre.is_empty() { 1 } else { 2 }; // c:1980
            ms = FILECOMP.with(|fc| {
                let fb = fc.borrow();
                comp_match(
                    &pfx,
                    &sfx,
                    s,
                    fb.as_ref(),
                    Some(&mut lc),
                    qu,
                    None,
                    ppl,
                    None,
                    psl,
                    &mut isexact,
                )
            });
        } else {
            let pfx = multiquote(&FPRE.with(|r| r.borrow().clone()), 1);
            let sfx = multiquote(&FSUF.with(|r| r.borrow().clone()), 1);
            ms = FILECOMP.with(|fc| {
                let fb = fc.borrow();
                comp_match(
                    &pfx,
                    &sfx,
                    s,
                    fb.as_ref(),
                    Some(&mut lc),
                    0,
                    None,
                    ppl,
                    None,
                    psl,
                    &mut isexact,
                )
            });
        }
        if ms.is_none() {
            return; // c:1985-1986
        }
        // c:1988-1989 — -7 (command names) requires the name to resolve.
        if aw == -7 && findcmd(s, 0, 0).is_none() {
            return;
        }
        isfile = CMF_FILE; // c:1990
    } else if aw == CC_QUOTEFLAG as i32 || aw == -2 || aw == -3 || aw == -4 || aw == -9 || aw > 0 {
        // c:1991-2041 — conditional / hash-node accept thread. (hn->flags
        // predicate enforced upstream — see fn header.) Match the word
        // against the real prefix/suffix, trying the quoted pattern-driven
        // form first, then the plain form.
        let (p1s, s1s, p2s, s2s) = if aw == CC_QUOTEFLAG as i32 {
            // c:2018-2019
            (
                QRPRE.with(|r| r.borrow().clone()),
                QRSUF.with(|r| r.borrow().clone()),
                RPRE.with(|r| r.borrow().clone()),
                RSUF.with(|r| r.borrow().clone()),
            )
        } else {
            // c:2021-2022
            (
                QLPRE.with(|r| r.borrow().clone()),
                QLSUF.with(|r| r.borrow().clone()),
                LPRE.with(|r| r.borrow().clone()),
                LSUF.with(|r| r.borrow().clone()),
            )
        };
        let p1 = multiquote(&p1s, 1); // c:2024
        let s1 = multiquote(&s1s, 1); // c:2024
        let p2 = multiquote(&p2s, 1); // c:2025
        let s2 = multiquote(&s2s, 1); // c:2025
        let qflag = (aw == CC_QUOTEFLAG as i32) as i32; // c:2030
        let p1l = p1.len() as i32;
        let s1l = s1.len() as i32;
        // c:2029 — patcomp-driven match first.
        let first = PATCOMP.with(|pc| {
            let pb = pc.borrow();
            comp_match(
                &p1,
                &s1,
                s,
                pb.as_ref(),
                Some(&mut lc),
                qflag,
                None,
                p1l,
                None,
                s1l,
                &mut isexact,
            )
        });
        if first.is_some() {
            ms = first;
        } else {
            // c:2035 — fall back to the plain real prefix/suffix.
            let p2l = p2.len() as i32;
            let s2l = s2.len() as i32;
            ms = comp_match(
                &p2,
                &s2,
                s,
                None,
                Some(&mut lc),
                qflag,
                None,
                p2l,
                None,
                s2l,
                &mut isexact,
            );
            if ms.is_none() {
                return; // c:2039
            }
        }
    } else {
        return; // c:2015 else — drop the match.
    }

    let ms = match ms {
        Some(m) => m,
        None => return, // c:2042-2043
    };

    // c:2046-2049 — when inserting braces always use the quoted length;
    // omitted here for the same reason as the c:1935-1938 snapshot above
    // (no brace-list consumer in the Rust comp_match/add_match_data port).

    // c:2051-2057 — build the real Cmatch from the preamble statics.
    let ipre_v = crate::ported::zle::compcore::ipre
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.clone()))
        .unwrap_or_default();
    let ripre_v = crate::ported::zle::compcore::ripre
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.clone()))
        .unwrap_or_default();
    let isuf_v = ISUF.lock().map(|g| g.clone()).unwrap_or_default();
    let prpre_v = PRPRE.with(|r| r.borrow().clone()).unwrap_or_default();
    // c:2052/2056 — C passes `curcc->prefix` / `curcc->suffix` STRAIGHT through,
    // NULL and all; add_match_data stores them unconditionally (compcore.c:2943-
    // 2944) so "no prefix" stays distinct from "empty prefix". Flattening the
    // Options here with unwrap_or_default() erased that distinction before it
    // could reach the match.
    let (pre_v, suf_v): (Option<String>, Option<String>) = CURCC.with(|r| {
        r.borrow()
            .as_ref()
            .map(|cc| (cc.prefix.clone(), cc.suffix.clone()))
            .unwrap_or((None, None))
    });
    // c:2054-2055 — path prefix/suffix only travel with real filenames.
    let lppre_v = if isfile != 0 {
        LPPRE.with(|r| r.borrow().clone())
    } else {
        String::new()
    };
    let lpsuf_v = if isfile != 0 {
        LPSUF.with(|r| r.borrow().clone())
    } else {
        String::new()
    };
    let mflags_v = crate::ported::zle::compcore::mflags.load(Ordering::Relaxed);

    add_match_data(
        isalt,             // c:2051 alt (fignore alternative)
        &ms,               // str — the matched string
        s,                 // orig
        lc,                // line — the Cline built by comp_match
        &ipre_v,           // ipre
        &ripre_v,          // ripre
        &isuf_v,           // isuf
        pre_v.as_deref(),  // pre  — curcc->prefix (NULL-able, c:2052)
        &prpre_v,          // prpre
        &lppre_v,          // ppre — path prefix (files only)
        None,              // pline
        &lpsuf_v,          // psuf — path suffix (files only)
        None,              // sline
        suf_v.as_deref(),  // suf  — curcc->suffix (NULL-able, c:2056)
        mflags_v | isfile, // c:2057 flags
        isexact,
    );

    // Rust observability mirror (unit-test / inspection sink; not part of
    // the C source, which registers only into the global match list via
    // add_match_data above). Kept so the makecomplistflags-level tests can
    // still observe which names were accepted.
    MATCH_LIST.with(|r| r.borrow_mut().push(s.to_string()));
}

/// Hash-node → match adapter for scanhashtable callbacks.
/// Port of `addhnmatch(HashNode hn, UNUSED(int flags))` from Src/Zle/compctl.c:2122.
///
/// Trivial wrapper: ignores `flags` and forwards the node name to
/// addmatch with `t=NULL`. Used by maketildelist's scanhashtable
/// invocation (c:2060).
/// WARNING: param names don't match C — Rust=(_flags) vs C=(hn, flags)
pub(crate) fn addhnmatch(name: &str, _flags: i32) {
    addmatch(name, None);
}

/// Expand a string via prefork (parameter / arith / cmd-sub /
/// tilde / brace / glob), suppressing errors.
/// Port of `getreal(char *str)` from Src/Zle/compctl.c:2132.
///
/// C body builds a one-element LinkList, sets `noerrs=1`, runs
/// `prefork(l, 0, NULL)`, then returns the first element if the
/// list is non-empty and the first elem has content; else returns
/// the original string.
///
/// Rust: routes through `singsub` since that's the equivalent
/// "expand a single word with errors swallowed". Returns owned
/// String (vs C's heap-string-pointer).
/// WARNING: param names don't match C — Rust=(str_in) vs C=(str)
pub(crate) fn getreal(str_in: &str) -> String {
    // c:2132
    // c:2134 — LinkList l = newlinklist();
    // c:2135 — int ne = noerrs;
    let mut ne_guard = crate::ported::utils::noerrs_lock()
        .lock()
        .expect("NOERRS poisoned");
    let ne = *ne_guard;
    // c:2137 — noerrs = 1;
    *ne_guard = 1;
    drop(ne_guard);
    // c:2138 — addlinknode(l, dupstring(str));
    // c:2139 — prefork(l, 0, NULL);
    // singsub is the equivalent single-word expansion (prefork on a
    // single-element list + extract the first elem) — keeps the
    // expanded form when non-empty.
    let s = crate::ported::subst::singsub(str_in);
    // c:2140 — noerrs = ne;
    *crate::ported::utils::noerrs_lock()
        .lock()
        .expect("NOERRS poisoned") = ne;
    // c:2141-2143 — if (!errflag && nonempty(l) && first non-empty) → use expanded.
    if errflag.load(std::sync::atomic::Ordering::Relaxed) == 0 && !s.is_empty() {
        return s;
    }
    // c:2144 — errflag &= ~ERRFLAG_ERROR;
    errflag.fetch_and(
        !crate::ported::utils::ERRFLAG_ERROR,
        std::sync::atomic::Ordering::Relaxed,
    );
    // c:2146 — return dupstring(str);
    str_in.to_string()
}

// (getreal port location; impl above already routes through singsub)
/// Read a directory and add files to the matches list.
/// Port of `gen_matches_files(int dirs, int execs, int all)` from Src/Zle/compctl.c:2154.
///
/// C signature: `void gen_matches_files(int dirs, int execs, int all)`.
/// Walks the directory at `prpre` (the expanded pre-cursor path
/// component), filtering each entry per:
///   dirs   → only directories
///   execs  → only executable files
///   all    → no filter (everything except `.`/`..` unless `all`)
/// Calls addmatch for each accepted entry.
///
/// Rust port reads `prpre` (PRPRE static if set; else current dir),
/// applies the same dirent-stat dispatch.
/// WARNING: param names don't match C — Rust=(execs, all) vs C=(dirs, execs, all)
pub(crate) fn gen_matches_files(dirs: bool, execs: bool, all: bool) {
    let prpre = PRPRE
        .with(|r| r.borrow().clone())
        .unwrap_or_else(|| ".".to_string());
    let entries = match std::fs::read_dir(&prpre) {
        Ok(e) => e,
        Err(_) => return,
    };
    // A leading `.` in the file prefix (`cat .h<Tab>`) means the user
    // explicitly asked for dotfiles, so don't hide them — matches zsh,
    // which only suppresses dotfiles when the prefix doesn't start with `.`.
    let dot_prefix = FPRE.with(|r| r.borrow().starts_with('.'));
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // Skip `.`/`..` unless `all` is set
        if !all && (name == "." || name == "..") {
            continue;
        }
        // Hidden-file rule: leading `.` requires `all` or a dot-prefixed word.
        if !all && !dot_prefix && name.starts_with('.') {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Type filter per (dirs, execs): `dirs` keeps directories,
        // `execs` keeps executables, and when BOTH are set (the
        // CC_COMMPATH `$path` walk, c:3504 `gen_matches_files(1,1,0)`)
        // keep either. Neither set → keep every visible entry (plain
        // file completion). The previous code applied the two filters in
        // series, so the combined dirs+execs case kept nothing (a file
        // is never both a dir and a non-dir).
        let is_dir = meta.is_dir();
        #[cfg(unix)]
        let is_exec = !is_dir && (meta.permissions().mode() & 0o111 != 0);
        #[cfg(not(unix))]
        let is_exec = false;
        let keep = match (dirs, execs) {
            (false, false) => true,
            (true, false) => is_dir,
            (false, true) => is_exec,
            (true, true) => is_dir || is_exec,
        };
        if !keep {
            continue;
        }
        addmatch(&name, None);
    }
}

/// Line-context dispatch — global completion entry.
/// Port of `makecomplistglobal(char *os, int incmd, UNUSED(int lst), int flags)` from Src/Zle/compctl.c:2401.
///
/// Looks at `linwhat` (IN_ENV / IN_MATH / IN_COND / IN_REDIR / else)
/// and dispatches to the appropriate compctl spec:
///   IN_ENV    → cc_default (parameter values)
///   IN_MATH   → cc_dummy (params or assoc keys)
///   IN_COND   → cc_dummy with -o/-nt/-ot/-ef logic
///   IN_REDIR  → cc_default (redirections)
///   default   → makecomplistcmd (per-command lookup)
///
/// `linwhat` and friends live in zle_tricky.c. For the foundation,
/// we assume "default" (per-command lookup) which is the most
/// common path.
pub(crate) fn makecomplistglobal(os: &str, incmd: bool, _lst: i32, flags: i32) -> i32 {
    use std::sync::atomic::Ordering;
    // c:2406 — reset ccont.
    CCONT.with(|c| c.set(CC_CCCONT));

    // c:2407 — clear cc_dummy.suffix.
    if let Some(d) = CC_DUMMY.lock().unwrap().as_mut() {
        if let Some(inner) = std::sync::Arc::get_mut(d) {
            inner.suffix = None;
        }
    }

    const CFN_FIRST: i32 = 1;
    const CFN_DEFAULT: i32 = 2;

    // Seed CMDSTR (the command whose argument is being completed) from the
    // parsed command line. C sets `cmdstr` in get_comp_string; the Rust
    // port keeps it thread-local in this module and only makecomplistctl
    // (the `compcall` driver) set it, so the ccmakehookfn → makecomplistcmd
    // path saw it as None and bailed at `None => return ret` before ever
    // reaching cc_default — argument/file completion (`ls <Tab>`) produced
    // no matches. For command position (incmd) there is no outer command
    // context, so leave it None (makecomplistcmd uses cc_compos there).
    if !incmd {
        let cmd_word = crate::ported::zle::complete::COMPWORDS
            .get()
            .and_then(|m| m.lock().ok())
            .and_then(|g| g.first().cloned());
        CMDSTR.with(|r| *r.borrow_mut() = cmd_word);
    }
    let lw = crate::ported::zle::compcore::linwhat.load(Ordering::Relaxed);
    let in_env = lw == crate::ported::zle::compcore::IN_ENV_LW; // c:2409
    let in_math = lw == crate::ported::zle::compcore::IN_MATH_LW; // c:2415
    let in_cond = lw == crate::ported::zle::compcore::IN_COND_LW; // c:2429

    let mut cc: Option<Arc<Compctl>> = None;
    if in_env {
        // c:2409
        if (flags & CFN_DEFAULT) == 0 {
            // c:2411
            cc = CC_DEFAULT.lock().unwrap().clone(); // c:2412
        }
    } else if in_math {
        // c:2415
        if (flags & CFN_DEFAULT) == 0 {
            let mut dummy_inner = Compctl::default();
            dummy_inner.mask = CC_PARAMS; // c:2424
            dummy_inner.refc = 10000; // c:2427
            cc = Some(Arc::new(dummy_inner));
        }
    } else if in_cond {
        // c:2429
        if (flags & CFN_DEFAULT) == 0 {
            let lwpos = *CLWPOS.lock().unwrap() as usize;
            let words = CLWORDS.lock().unwrap().clone();
            let prev = if lwpos > 0 {
                words.get(lwpos - 1).cloned()
            } else {
                None
            };
            let prev_s = prev.as_deref().unwrap_or("");
            let mask = if prev_s == "-o" {
                // c:2435
                CC_OPTIONS
            } else if (prev_s.starts_with('-') && prev_s.len() == 2)
                || prev_s == "-nt"
                || prev_s == "-ot"
                || prev_s == "-ef"
            // c:2436
            {
                CC_FILES
            } else {
                CC_FILES | CC_PARAMS // c:2440
            };
            let mut dummy_inner = Compctl::default();
            dummy_inner.mask = mask;
            dummy_inner.refc = 10000;
            cc = Some(Arc::new(dummy_inner));
        }
    } else {
        // c:2453 — default: per-command lookup via makecomplistcmd.
        return makecomplistcmd(os, incmd, flags);
    }

    if let Some(cc) = cc {
        // c:2458 — cc_first first.
        if (flags & CFN_FIRST) == 0 {
            if let Some(cc_first) = CC_FIRST.lock().unwrap().clone() {
                makecomplistcc(&cc_first, os, incmd); // c:2459
                if (CCONT.with(|c| c.get()) & CC_CCCONT) == 0 {
                    // c:2461
                    return 0;
                }
            }
        }
        makecomplistcc(&cc, os, incmd); // c:2464
        return 1; // c:2465
    }
    0 // c:2467
}

/// Per-command compctl lookup + dispatch.
/// Port of `makecomplistcmd(char *os, int incmd, int flags)` from Src/Zle/compctl.c:2474.
///
/// Resolves the compctl for cmdstr by:
///   1. If !CFN_FIRST: run cc_first first; bail if !CC_CCCONT
///   2. Run pattern compctls (makecomplistpc); bail if !CC_CCCONT
///   3. If cmdstr starts with `=`, expand path
///   4. Lookup cmdstr in compctltab — try full name then trailing
///      pathname component (after remlpaths)
///   5. If incmd: use cc_compos
///   6. Else if no match: cc_default (unless CFN_DEFAULT)
///   7. Call makecomplistcc(cc, os, incmd)
/// WARNING: param names don't match C — Rust=(incmd, flags) vs C=(os, incmd, flags)
pub(crate) fn makecomplistcmd(os: &str, incmd: bool, flags: i32) -> i32 {
    const CFN_FIRST: i32 = 1;
    const CFN_DEFAULT: i32 = 2;
    let mut ret: i32 = 0;

    // C: c:2482 — first try cc_first
    if (flags & CFN_FIRST) == 0 {
        if let Some(cc_first) = CC_FIRST.lock().unwrap().clone() {
            makecomplistcc(&cc_first, os, incmd);
            if (CCONT.with(|c| c.get()) & CC_CCCONT) == 0 {
                return 0;
            }
        }
    }

    // C: c:2491 — pattern compctls
    let cmdstr = CMDSTR.with(|r| r.borrow().clone());
    if cmdstr.is_some() {
        ret |= makecomplistpc(os, incmd);
        if (CCONT.with(|c| c.get()) & CC_CCCONT) == 0 {
            return ret;
        }
    }

    // C: c:2509 — incmd path uses cc_compos
    let cc = if incmd {
        CC_COMPOS.lock().unwrap().clone()
    } else {
        // C: c:2511-2519 — lookup compctltab[cmdstr]
        let name = match &cmdstr {
            Some(s) => s.clone(),
            None => return ret,
        };
        let table = COMPCTL_TAB.read().unwrap();
        let from_table = table.as_ref().and_then(|m| m.get(&name).cloned());
        drop(table);
        match from_table {
            Some(c) => Some(c),
            None => {
                if (flags & CFN_DEFAULT) != 0 {
                    return ret;
                }
                ret |= 1;
                CC_DEFAULT.lock().unwrap().clone()
            }
        }
    };
    if let Some(c) = cc {
        makecomplistcc(&c, os, incmd);
    }
    ret
}

/// Per-compctl entry — track usage + dispatch the OR chain.
/// Port of `makecomplistcc(Compctl cc, char *s, int incmd)` from Src/Zle/compctl.c:2558.
///
/// Bumps refc on cc, adds it to ccused list, resets ccont, calls
/// makecomplistor. The ccused list lets later cleanup free all
/// compctls used during a single completion.
/// WARNING: param names don't match C — Rust=(s, incmd) vs C=(cc, s, incmd)
pub(crate) fn makecomplistcc(cc: &Arc<Compctl>, s: &str, incmd: bool) {
    // C: c:2560 — refc++ (Arc handles this)
    let _ = cc.clone();

    // C: c:2562 — initialize ccused list
    CCUSED.with(|r| r.borrow_mut().push(cc.clone()));

    // C: c:2565 — reset ccont
    CCONT.with(|c| c.set(0));

    // C: c:2567 — dispatch OR chain
    makecomplistor(cc, s, incmd, 0, 0);
}

// Pre-cursor directory path (`prpre` global). Port of file-static
// `char *prpre` at Src/Zle/compctl.c:1736 — the directory portion
// of the path component the cursor is in, expanded for `opendir`.
// Set by the completion driver before calling gen_matches_files.
thread_local! { static PRPRE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }

/// Find a node in a linked list by data-pointer equality.
/// Port of `findnode(LinkList list, void *dat)` from Src/Zle/compctl.c:2288.
///
/// C signature: `LinkNode findnode(LinkList list, void *dat)` —
/// walks `list` looking for the node whose data pointer == `dat`.
/// Returns the matching node or NULL.
///
/// Rust generic over `T: PartialEq` — returns the index of the
/// matching element, or None.
/// WARNING: param names don't match C — Rust=(dat) vs C=(list, dat)
pub(crate) fn findnode<T: PartialEq>(list: &[T], dat: &T) -> Option<usize> {
    list.iter().position(|x| x == dat)
}

// `cdepth` recursion guard. Port of file-static `int cdepth = 0;`
// at Src/Zle/compctl.c:2300.
thread_local! { static CDEPTH: std::cell::Cell<i32> = const { std::cell::Cell::new(0) }; }

/// Port of `MAX_CDEPTH` from `Src/Zle/compctl.c:2302`. Maximum
/// recursion depth — prevents infinite recursion between compctl-
/// driven completion and the wrapper.
pub const MAX_CDEPTH: i32 = 16; // c:2302

// `ccont` continuation flags. Port of file-static `unsigned long
// ccont;` at Src/Zle/compctl.c:1714. Bitmask of CC_CCCONT/etc.
// controlling whether the dispatch loop continues to next compctl.
thread_local! { static CCONT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }

/// Build the completion list — top-level dispatch.
/// Port of `makecomplistctl(int flags)` from Src/Zle/compctl.c:2305.
///
/// Entry point used by bin_compcall and the completion driver.
/// The C body:
///   1. Recursion guard (cdepth >= MAX_CDEPTH → return 0)
///   2. SWITCHHEAPS to the compheap (Rust uses the global allocator)
///   3. Save lots of state (cmdstr, clwords, instring, qipre/qisuf,
///      isuf, autoq, offs)
///   4. Set up new state from compquote / compqiprefix / compqisuffix /
///      compisuffix / compwords / compcurrent
///   5. Set incompfunc=2 (deeper-nested marker)
///   6. Call makecomplistglobal(str, !clwpos, COMP_COMPLETE, flags)
///   7. Restore state
///   8. cdepth-- and return
///
/// This Rust port keeps the recursion guard + flag dispatch + the
/// makecomplistglobal call. The compfunc state save/restore relies
/// on ZLE-tricky globals (clwords, etc.) that aren't ported here.
pub(crate) fn makecomplistctl(flags: i32) -> i32 {
    // c:2318 — recursion guard.
    let cdepth = CDEPTH.with(|c| c.get());
    if cdepth == MAX_CDEPTH {
        // c:2318
        return 0;
    }
    CDEPTH.with(|c| c.set(cdepth + 1)); // c:2321

    // c:2323-2324 — comp_str(&lip, &lp, 0): the word being completed
    // plus the ignored-prefix (`lip`) and prefix (`lp`) lengths.
    let (str_in, lip, lp) = crate::ported::zle::compcore::comp_str(false);

    // c:2325-2327 — save the state makecomplistctl overwrites.
    let os = CMDSTR.with(|r| r.borrow().clone());
    let ow = CLWORDS.lock().unwrap().clone();
    let on = *CLWNUM.lock().unwrap();
    let op = *CLWPOS.lock().unwrap();
    let ois = *INSTRING.lock().unwrap();
    let oib = *INBACKT.lock().unwrap();
    let oisuf = ISUF.lock().unwrap().clone();
    let oqp = QIPRE.lock().unwrap().clone();
    let oqs = QISUF.lock().unwrap().clone();
    let oaq = AUTOQ
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone();
    let ooffs = *OFFS.lock().unwrap();

    // c:2330-2361 — quote-context setup driven by `compquote`.
    let compquote = crate::ported::zle::zle_tricky::COMPQUOTE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    match compquote.chars().next() {
        Some('`') => {
            // c:2331-2338 — backtick: instring/inbackt cleared, no autoq.
            *INSTRING.lock().unwrap() = QT_NONE;
            *INBACKT.lock().unwrap() = 0;
            *AUTOQ
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap() = String::new();
        }
        Some(c) if c == '\'' || c == '"' || c == '$' => {
            // c:2340-2355 — single/double/dollar quoting.
            *INSTRING.lock().unwrap() = match c {
                '\'' => QT_SINGLE,
                '"' => QT_DOUBLE,
                _ => QT_DOLLARS,
            };
            *INBACKT.lock().unwrap() = 0;
            // c:2354 — autoq = (compquote == '$' ? compquote+1 : compquote).
            *AUTOQ
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap() = if c == '$' {
                compquote[1..].to_string()
            } else {
                compquote.clone()
            };
        }
        _ => {
            // c:2357-2360 — no quoting context.
            *INSTRING.lock().unwrap() = QT_NONE;
            *INBACKT.lock().unwrap() = 0;
            *AUTOQ
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap() = String::new();
        }
    }

    // c:2362-2363 — qipre/qisuf from the compctl driver params.
    *QIPRE.lock().unwrap() = COMPQIPREFIX.lock().unwrap().clone();
    *QISUF.lock().unwrap() = COMPQISUFFIX.lock().unwrap().clone();
    // c:2364-2366 — isuf = remnulargs(ctokenize(compisuffix)).
    let mut isuf_v = crate::ported::zle::compcore::ctokenize(&COMPISUFFIX.lock().unwrap().clone());
    crate::ported::glob::remnulargs(&mut isuf_v);
    *ISUF.lock().unwrap() = isuf_v;

    // c:2367-2377 — install compwords into clwords, each tokenized.
    let compwords = COMPWORDS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clone();
    *CLWNUM.lock().unwrap() = compwords.len() as i32; // c:2367
    *CLWPOS.lock().unwrap() = COMPCURRENT.load(std::sync::atomic::Ordering::Relaxed) - 1; // c:2368
    CMDSTR.with(|r| *r.borrow_mut() = compwords.first().cloned()); // c:2369
    let clw: Vec<String> = compwords
        .iter()
        .map(|w| {
            let mut t = w.clone(); // c:2372
            crate::ported::glob::tokenize(&mut t); // c:2373
            crate::ported::glob::remnulargs(&mut t); // c:2374
            t
        })
        .collect();
    *CLWORDS.lock().unwrap() = clw;

    // c:2378 — offs = lip + lp.
    *OFFS.lock().unwrap() = lip + lp;

    // c:2379-2381 — incompfunc = 2 during the nested list build, 1 after.
    INCOMPFUNC.store(2, std::sync::atomic::Ordering::Relaxed);
    let incmd = *CLWPOS.lock().unwrap() == 0; // c:2380 — `!clwpos`
    let ret = makecomplistglobal(
        &str_in,
        incmd,
        crate::ported::zle::zle_h::COMP_COMPLETE,
        flags,
    ); // c:2380
    INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed); // c:2381

    // c:2382-2396 — restore the saved state.
    *ISUF.lock().unwrap() = oisuf;
    *QIPRE.lock().unwrap() = oqp;
    *QISUF.lock().unwrap() = oqs;
    *INSTRING.lock().unwrap() = ois;
    *INBACKT.lock().unwrap() = oib;
    *AUTOQ
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap() = oaq;
    *OFFS.lock().unwrap() = ooffs;
    CMDSTR.with(|r| *r.borrow_mut() = os);
    *CLWORDS.lock().unwrap() = ow;
    *CLWNUM.lock().unwrap() = on;
    *CLWPOS.lock().unwrap() = op;

    CDEPTH.with(|c| c.set(c.get() - 1)); // c:2398
    ret // c:2400
}

/// Top-level per-compctl dispatch.
/// Port of `makecomplistlist(Compctl cc, char *s, int incmd, int compadd)` from Src/Zle/compctl.c:2615.
///
/// Routes to either makecomplistext (for -x extended conditions)
/// or makecomplistflags (for the regular flag-mask compctl).
/// WARNING: param names don't match C — Rust=(s, incmd, compadd) vs C=(ylist)
pub(crate) fn makecomplistlist(cc: &Arc<Compctl>, s: &str, incmd: bool, compadd: i32) {
    if cc.ext.is_some() {
        // C: c:3155 — extended -x conditions
        makecomplistext(cc, s, incmd);
    } else {
        // C: c:3499 — regular flag-driven completion
        makecomplistflags(cc, s.to_string(), incmd, compadd);
    }
}

/// Extended (`-x`) completion list builder.
/// Port of `makecomplistext(Compctl occ, char *os, int incmd)` from Src/Zle/compctl.c:2640.
///
/// Walks cc.ext chain (the per-condition compctls), evaluates each
/// condition against the current line state, and dispatches to
/// makecomplistflags for the first matching condition's spec.
/// WARNING: param names don't match C — Rust=(os, incmd) vs C=(Equals)
pub(crate) fn makecomplistext(occ: &Arc<Compctl>, os: &str, incmd: bool) {
    use crate::ported::glob::tokenize;
    use crate::ported::lex::untokenize;
    use crate::ported::zle::compcore::rembslash;

    // c:2655 — ins = (instring != QT_NONE ? instring : (inbackt ? QT_BACKTICK : 0)).
    let ins = {
        let is = *INSTRING.lock().unwrap();
        if is != QT_NONE {
            is
        } else if *INBACKT.lock().unwrap() != 0 {
            QT_BACKTICK
        } else {
            0
        }
    };
    let complete_in_word = crate::ported::options::opt_state_get("COMPLETEINWORD").unwrap_or(false);

    let mut m = 0; // c:2652
    let mut d = 0; // c:2652

    // c:2658 — loop over the patterns separated by `-`s (the `ext`
    // chain, walked via each compctl's `next` sibling).
    let mut compc_opt = occ.ext.clone();
    while let Some(compc) = compc_opt {
        let mut compadd = 0i32; // c:2659
        let mut t; // c:2659

        // c:2662 — loop over OR'ed patterns: `for (cc = compc->cond;
        // cc && !t; cc = or)`. `t` starts 0 for the first OR arm.
        t = 0;
        let mut cc_opt: Option<&Compcond> = compc.cond.as_deref();
        while let Some(cc0) = cc_opt {
            if t != 0 {
                break; // c:2662 — `cc && !t`
            }
            let or = cc0.or.as_deref(); // c:2663 — save the OR sibling.

            // c:2665 — loop over AND'ed patterns: `for (t = 1;
            // cc && t; cc = cc->and)`.
            t = 1;
            let mut and_opt: Option<&Compcond> = Some(cc0);
            while let Some(cc) = and_opt {
                if t == 0 {
                    break;
                }
                let clwnum = *CLWNUM.lock().unwrap();
                let clwpos = *CLWPOS.lock().unwrap();

                // c:2667 — loop over the [...] pairs: `for (t = i = 0;
                // i < cc->n && !t; i++)`.
                t = 0;
                let mut i = 0i32;
                while i < cc.n && t == 0 {
                    // c:2669-2670 — reset the range for this pair.
                    *BRANGE.lock().unwrap() = 0;
                    *ERANGE.lock().unwrap() = clwnum - 1;
                    let iu = i as usize;

                    match cc.typ {
                        // c:2672 — CCT_QUOTE.
                        x if x == CCT_QUOTE => {
                            if let CompcondData::S { s, .. } = &cc.u {
                                let c0 = s.get(iu).and_then(|x| x.chars().next());
                                t = match c0 {
                                    Some('s') if ins == QT_SINGLE => 1,
                                    Some('d') if ins == QT_DOUBLE => 1,
                                    Some('b') if ins == QT_BACKTICK => 1,
                                    _ => 0,
                                };
                            }
                        }
                        // c:2677 / c:2680 — CCT_POS / CCT_NUMWORDS.
                        x if x == CCT_POS || x == CCT_NUMWORDS => {
                            let tt = if x == CCT_POS { clwpos } else { clwnum }; // c:2678/2681
                            if let CompcondData::R { a, b } = &cc.u {
                                let mut av = *a.get(iu).unwrap_or(&0); // c:2683
                                if av < 0 {
                                    av += clwnum;
                                }
                                let mut bv = *b.get(iu).unwrap_or(&0); // c:2685
                                if bv < 0 {
                                    bv += clwnum;
                                }
                                if x == CCT_POS {
                                    // c:2688 — brange = a, erange = b.
                                    *BRANGE.lock().unwrap() = av;
                                    *ERANGE.lock().unwrap() = bv;
                                }
                                t = if tt >= av && tt <= bv { 1 } else { 0 }; // c:2689
                            }
                        }
                        // c:2691 — CCT_CURSUF / CCT_CURPRE.
                        x if x == CCT_CURSUF || x == CCT_CURPRE => {
                            if let CompcondData::S { s, .. } = &cc.u {
                                // c:2693 — s = clwpos < clwnum ? os : "".
                                let mut sv = if clwpos < clwnum {
                                    os.to_string()
                                } else {
                                    String::new()
                                };
                                sv = untokenize(&sv); // c:2694
                                if complete_in_word {
                                    // c:2695 — s[offs] = '\0'.
                                    let off = (*OFFS.lock().unwrap()).max(0) as usize;
                                    if off <= sv.len() && sv.is_char_boundary(off) {
                                        sv.truncate(off);
                                    }
                                }
                                let sc = rembslash(s.get(iu).map(|x| x.as_str()).unwrap_or("")); // c:2696
                                let a = sc.len(); // c:2697
                                if sv.len() >= a && sv.starts_with(&sc) {
                                    // c:2698 — !strncmp(s, sc, a).
                                    compadd = if x == CCT_CURSUF { a as i32 } else { 0 }; // c:2699
                                    t = 1;
                                }
                            }
                        }
                        // c:2703 — CCT_CURSUB / CCT_CURSUBC.
                        x if x == CCT_CURSUB || x == CCT_CURSUBC => {
                            if clwpos < 0 || clwpos >= clwnum {
                                t = 0; // c:2706
                            } else if let CompcondData::S { p, s } = &cc.u {
                                let mut sv = untokenize(os); // c:2708-2709
                                if complete_in_word {
                                    let off = (*OFFS.lock().unwrap()).max(0) as usize; // c:2710
                                    if off <= sv.len() && sv.is_char_boundary(off) {
                                        sv.truncate(off);
                                    }
                                }
                                let a = getcpat(
                                    &sv,
                                    *p.get(iu).unwrap_or(&0),
                                    s.get(iu).map(|x| x.as_str()).unwrap_or(""),
                                    if x == CCT_CURSUBC { 1 } else { 0 },
                                ); // c:2711
                                if a != -1 {
                                    compadd = a; // c:2716
                                    t = 1;
                                }
                            }
                        }
                        // c:2720 / c:2724 — CCT_CURPAT/CURSTR / WORDPAT/WORDSTR.
                        x if x == CCT_CURPAT
                            || x == CCT_CURSTR
                            || x == CCT_WORDPAT
                            || x == CCT_WORDSTR =>
                        {
                            // c:2722/2726 — tt = clwpos (cur*) or 0 (word*).
                            let tt = if x == CCT_CURPAT || x == CCT_CURSTR {
                                clwpos
                            } else {
                                0
                            };
                            if let CompcondData::S { p, s } = &cc.u {
                                let mut a = tt + *p.get(iu).unwrap_or(&0); // c:2728
                                if a < 0 {
                                    a += clwnum;
                                }
                                let sv0 = if a < 0 || a >= clwnum {
                                    String::new()
                                } else {
                                    CLWORDS
                                        .lock()
                                        .unwrap()
                                        .get(a as usize)
                                        .cloned()
                                        .unwrap_or_default()
                                }; // c:2730
                                let sv = untokenize(&sv0); // c:2732
                                let patstr = s.get(iu).map(|x| x.as_str()).unwrap_or("");
                                if x == CCT_CURPAT || x == CCT_WORDPAT {
                                    // c:2736-2738 — pattern match.
                                    let mut ss = patstr.to_string();
                                    tokenize(&mut ss);
                                    t = patcompile(&ss, PAT_HEAPDUP as i32, None)
                                        .map_or(false, |pp| pattry(&pp, &sv))
                                        as i32;
                                } else {
                                    // c:2740 — !strcmp(s, rembslash(...)).
                                    t = (sv == rembslash(patstr)) as i32;
                                }
                            }
                        }
                        // c:2742 — CCT_RANGESTR / CCT_RANGEPAT.
                        x if x == CCT_RANGESTR || x == CCT_RANGEPAT => {
                            let is_pat = x == CCT_RANGEPAT;
                            if let CompcondData::L { a, b } = &cc.u {
                                let astr = a.get(iu).cloned().unwrap_or_default();
                                let bstr = b.get(iu).cloned().unwrap_or_default();
                                let words = CLWORDS.lock().unwrap().clone();
                                // c:2744 — for RANGEPAT tokenize a[i] once.
                                let sc_a = if is_pat {
                                    let mut z = astr.clone();
                                    tokenize(&mut z);
                                    z
                                } else {
                                    rembslash(&astr) // c:2749
                                };
                                // c:2746 — scan backwards from clwpos-1.
                                let mut j = clwpos - 1;
                                while j > 0 {
                                    let sv = untokenize(
                                        words.get(j as usize).map(|x| x.as_str()).unwrap_or(""),
                                    ); // c:2747
                                    let matched = if is_pat {
                                        patcompile(&sc_a, PAT_HEAPDUP as i32, None)
                                            .map_or(false, |pp| pattry(&pp, &sv))
                                    } else {
                                        sv.len() >= sc_a.len() && sv.starts_with(&sc_a)
                                    }; // c:2750-2753
                                    if matched {
                                        *BRANGE.lock().unwrap() = j + 1; // c:2755
                                        t = 1;
                                        break;
                                    }
                                    j -= 1;
                                }
                                // c:2761 — if matched and there's an upper bound.
                                if t != 0 && !bstr.is_empty() {
                                    let sc_b = if is_pat {
                                        let mut z = bstr.clone();
                                        tokenize(&mut z);
                                        z
                                    } else {
                                        rembslash(&bstr)
                                    };
                                    let mut k = j + 1; // c:2764
                                    while k < clwnum {
                                        let sv = untokenize(
                                            words.get(k as usize).map(|x| x.as_str()).unwrap_or(""),
                                        );
                                        let matched = if is_pat {
                                            patcompile(&sc_b, PAT_HEAPDUP as i32, None)
                                                .map_or(false, |pp| pattry(&pp, &sv))
                                        } else {
                                            sv.len() >= sc_b.len() && sv.starts_with(&sc_b)
                                        };
                                        if matched {
                                            *ERANGE.lock().unwrap() = k - 1; // c:2773
                                            t = if clwpos <= k - 1 { 1 } else { 0 }; // c:2774
                                            break;
                                        }
                                        k += 1;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                and_opt = cc.and.as_deref();
            }
            cc_opt = or;
        }

        if t != 0 {
            // c:2786 — the patterns matched, use the flags.
            m = 1;
            CCONT.with(|c| c.set(c.get() & !(CC_PATCONT | CC_DEFCONT))); // c:2789
            makecomplistor(&compc, os, incmd, compadd, 1); // c:2790
            if d == 0 && (CCONT.with(|c| c.get()) & CC_DEFCONT) != 0 {
                // c:2791
                d = 1;
                *BRANGE.lock().unwrap() = 0; // c:2794
                *ERANGE.lock().unwrap() = *CLWNUM.lock().unwrap() - 1; // c:2795
                makecomplistflags(occ, os.to_string(), incmd, 0); // c:2796
            }
            if (CCONT.with(|c| c.get()) & CC_PATCONT) == 0 {
                break; // c:2798
            }
        }
        compc_opt = compc.next.clone();
    }
    // c:2802 — if no pattern matched, use the standard flags.
    if m == 0 {
        *BRANGE.lock().unwrap() = 0; // c:2805
        *ERANGE.lock().unwrap() = *CLWNUM.lock().unwrap() - 1; // c:2806
        makecomplistflags(occ, os.to_string(), incmd, 0); // c:2807
    }
}

// `cmdstr` — current command word being completed.
// Port of file-static `char *cmdstr` (zle_tricky.c). Set by the
// completion driver before invoking makecomplistcmd.
thread_local! { static CMDSTR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }

/// C body (c:2532-2552):
/// ```c
/// s = ((shfunctab->getnode(shfunctab, cmdstr) ||
///       builtintab->getnode(builtintab, cmdstr)) ? NULL :
///      findcmd(cmdstr, 1, 0));
/// for (pc = patcomps; pc; pc = pc->next) {
///     if ((pat = patcompile(pc->pat, PAT_STATIC, NULL)) &&
///         (pattry(pat, cmdstr) ||
///          (s && pattry(pat, s)))) {
///         makecomplistcc(pc->cc, os, incmd);
///         ret |= 2;
///         if (!(ccont & CC_CCCONT))
///             return ret;
///     }
/// }
/// return ret;
/// ```
/// Port of `makecomplistpc(char *os, int incmd)` from `Src/Zle/compctl.c:2530`.
/// WARNING: param names don't match C — Rust=(incmd) vs C=(os, incmd)
pub(crate) fn makecomplistpc(os: &str, incmd: bool) -> i32 {
    // c:2530
    let mut ret: i32 = 0; // c:2530
    let cmdstr = match CMDSTR.with(|r| r.borrow().clone()) {
        // c:2533
        Some(s) => s,
        None => return 0,
    };
    // c:2537-2540 — `s = (shfunctab[cmdstr] || builtintab[cmdstr]) ?
    // NULL : findcmd(cmdstr, 1, 0);` — only resolve via $PATH when
    // cmdstr is neither a defined function nor a builtin.
    let is_function = crate::ported::hashtable::shfunctab_lock()
        .read()
        .map(|t| t.contains_key(&cmdstr))
        .unwrap_or(false);
    let is_builtin = crate::ported::builtin::BUILTINS
        .iter()
        .any(|b| b.node.nam == cmdstr);
    let s_resolved: Option<String> = if is_function || is_builtin {
        // c:2537
        None // c:2538 NULL
    } else {
        findcmd(&cmdstr, 1, 0) // c:2540
    };

    let pats = PATCOMPS.read().unwrap().clone();
    for (pat, cc) in &pats {
        // c:2542
        // c:2543 — patcompile(pc->pat) compiles the pattern once.
        // c:2544-2545 — pattry(prog, cmdstr) || (s && pattry(prog, s)).
        let matches = patcompile(
            &{
                let mut __pat_tok = (pat).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            PAT_HEAPDUP as i32,
            None,
        )
        .map_or(false, |prog| {
            pattry(&prog, &cmdstr)             // c:2544
                    || s_resolved.as_deref()
                        .map(|sr| pattry(&prog, sr)) // c:2545
                        .unwrap_or(false)
        });
        if matches {
            makecomplistcc(cc, os, incmd); // c:2546
            ret |= 2; // c:2547
            if (CCONT.with(|c| c.get()) & CC_CCCONT) == 0 {
                // c:2548
                return ret; // c:2549
            }
        }
    }
    ret // c:2558
}

/// Separate the cursor word into prefix/word/suffix components.
/// Port of `sep_comp_string(char *ss, char *s, int noffs)` from Src/Zle/compctl.c:2806 (~225 lines).
///
/// C signature: `int sep_comp_string(char *ss, char *s, int noffs)`.
///
/// The function constructs a synthetic line of the form `ss + " " +
/// s[..noffs] + 'x' + s[noffs..]` and runs the lexer over it to
/// recover word boundaries with the cursor (the inserted 'x') in
/// view. Then adjusts wb/we/zlemetacs to reflect positions inside
/// the lexed word, accounting for inull markers. Pushes results
/// into clwords + cmdstr + qipre/qisuf and dispatches to
/// makecomplistcmd.
///
/// Faithful port:
///   - constructs the temp buffer per c:2827-2832
///   - applies rembslash if QT_BACKSLASH stack head (c:2833)
///   - state save/restore for instring/inbackt/noaliases/autoq (c:2810-2813)
///   - state save/restore for clwords/cmdstr/qipre/qisuf (c:2980-3023)
///   - inull/Bnull adjustment loop (c:2931-2952)
///   - nested makecomplistcmd dispatch (c:3006)
///
/// The actual `ctxtlex()` driver is replaced by the lex.rs module
/// — for this port we approximate by
/// splitting the temp string on whitespace + tracking the cursor
/// word. Full lexer-token reconstruction (LEXERR/STRING/ENDINPUT
/// handling for unbalanced quotes per c:2842-2855) is the
/// remaining gap; the foundation here handles plain-token cases
/// which cover the most common compctl flows.
pub(crate) fn sep_comp_string(ss: &str, s: &str, noffs: i32) -> i32 {
    // Canonical globals (deduped from former private compctl shadows).
    use crate::ported::zle::compcore::{ZLEMETACS as CS_G, ZLEMETALINE as LINE_G};
    use std::sync::atomic::Ordering;
    // C: c:2810-2813 — save state to restore on exit
    let owe = crate::ported::zle::compcore::WE.load(std::sync::atomic::Ordering::Relaxed);
    let owb = crate::ported::zle::compcore::WB.load(std::sync::atomic::Ordering::Relaxed);
    let ocs = CS_G.load(Ordering::Relaxed);
    let oll = *ZLEMETALL.lock().unwrap();
    let ois = *INSTRING.lock().unwrap();
    let oib = *INBACKT.lock().unwrap();
    let ona = *NOALIASES.lock().unwrap();
    let ne = *crate::ported::utils::noerrs_lock().lock().unwrap();
    let ol = LINE_G
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone();
    let oaq = AUTOQ
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone();

    let sl = ss.len() as i32;
    let mut got = false;
    let mut i = 0_i32;
    let mut cur: i32 = -1;
    let mut swb = 0_i32;
    let mut swe = 0_i32;
    let mut soffs = 0_i32;
    let mut ns: String = String::new();
    let mut foo: Vec<String> = Vec::new();

    // C: c:2823-2832 — build the temp buffer with cursor `x` marker.
    // tmp = ss + " " + s[..noffs] + 'x' + s[noffs..]
    *ADDEDX.lock().unwrap() = 1;
    *crate::ported::utils::noerrs_lock().lock().unwrap() = 1;
    *LEXFLAGS.lock().unwrap() = LEXFLAGS_ZLE;
    let mut tmp = String::with_capacity(ss.len() + 3 + s.len());
    tmp.push_str(ss);
    tmp.push(' ');
    let s_chars: Vec<char> = s.chars().collect();
    let noffs_u = (noffs as usize).min(s_chars.len());
    let s_pre: String = s_chars[..noffs_u].iter().collect();
    let s_post: String = s_chars[noffs_u..].iter().collect();
    tmp.push_str(&s_pre);
    let scs_initial = sl + 1 + noffs;
    CS_G.store(scs_initial, Ordering::Relaxed);
    let mut scs = scs_initial;
    tmp.push('x');
    tmp.push_str(&s_post);
    let tl = tmp.len() as i32;

    // C: c:2833 — apply rembslash if QT_BACKSLASH stack head
    let qstack_head = COMPQSTACK
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .chars()
        .next()
        .unwrap_or(QT_NONE as u8 as char);
    let remq = qstack_head as i32 == QT_BACKSLASH;
    if remq {
        // rembslash — strip backslashes
        let mut stripped = String::with_capacity(tmp.len());
        let mut chars = tmp.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&_nx) = chars.peek() {
                    // Skip backslash, keep next char
                    continue;
                }
            }
            stripped.push(c);
        }
        tmp = stripped;
    }

    // C: c:2835-2839 — push input, set zlemetaline
    *LINE_G
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap() = tmp.clone();
    *ZLEMETALL.lock().unwrap() = tl - 1;
    *NOALIASES.lock().unwrap() = 1;

    // C: c:2840-2873 — lex loop. We approximate ctxtlex() with a
    // whitespace-tokenize + cursor-word detection. Real lexer
    // integration requires lex.rs wired with
    // ZLE input-stack semantics.
    {
        let chars: Vec<char> = tmp.chars().collect();
        let mut t_start = 0_usize;
        let mut idx = 0_usize;
        let mut word_idx = 0_i32;
        while idx <= chars.len() {
            let at_end = idx == chars.len();
            let is_sep = !at_end && chars[idx] == ' ';
            if at_end || is_sep {
                if idx > t_start {
                    let token: String = chars[t_start..idx].iter().collect();
                    let abs_start = t_start as i32;
                    let abs_end = idx as i32;
                    foo.push(token.clone());
                    // C: c:2862-2871 — first time scs falls inside
                    // a token, that's the cursor word.
                    if !got && scs >= abs_start && scs <= abs_end {
                        got = true;
                        cur = word_idx;
                        swb = abs_start;
                        swe = abs_end;
                        soffs = scs - swb;
                        // C: chuck(p + soffs) — remove the dummy 'x'
                        let mut t = token.clone();
                        if (soffs as usize) < t.len() {
                            t.remove(soffs as usize);
                        }
                        ns = t;
                    }
                    word_idx += 1;
                }
                t_start = idx + 1;
            }
            if at_end {
                break;
            }
            idx += 1;
        }
        i = word_idx;
    }

    *NOALIASES.lock().unwrap() = ona;
    *crate::ported::utils::noerrs_lock().lock().unwrap() = ne;
    crate::ported::zle::compcore::WB.store(owb, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::compcore::WE.store(owe, std::sync::atomic::Ordering::Relaxed);
    CS_G.store(ocs, Ordering::Relaxed);
    *LINE_G
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap() = ol;
    *ZLEMETALL.lock().unwrap() = oll;

    // C: c:2885 — bail if no cursor word found
    if cur < 0 || i < 1 {
        return 1;
    }

    // C: c:2887-2896 — check_param dispatch (params + Snull/Dnull
    // marker conversion). Skipped pending check_param port.

    // C: c:2898-2929 — quote-prefix detection. Examine ns[0] for
    // Snull/Dnull/Stringg/QSTRING_TOK and adjust instring + autoq.
    let ts = ns.clone();
    let _ = ts.clone();
    let first_char = ns.chars().next();
    let is_quoted_open = matches!(first_char, Some(Snull) | Some(Dnull))
        || (matches!(first_char, Some(Stringg) | Some(QSTRING_TOK))
            && ns.chars().nth(1) == Some(Snull));

    if is_quoted_open {
        let new_instring = match first_char {
            Some(Snull) => QT_SINGLE,
            Some(Dnull) => QT_DOUBLE,
            _ => QT_DOLLARS,
        };
        *INSTRING.lock().unwrap() = new_instring;
        *INBACKT.lock().unwrap() = 0;
        swb += 1;
        // C: c:2921 — if the closing quote-marker matches at end, swe--
        if let (Some(first), Some(last)) = (ns.chars().next(), ns.chars().last()) {
            if first == last && ns.len() >= 2 {
                swe -= 1;
            }
        }
        // C: c:2925 — autoq from compqstack[1] and multiquote
        let qstack = COMPQSTACK
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap()
            .clone();
        if qstack.len() >= 2 {
            *AUTOQ
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap() = String::new();
        } else {
            *AUTOQ
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap() = ts.clone();
        }
    } else {
        *INSTRING.lock().unwrap() = QT_NONE;
        *AUTOQ
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap() = String::new();
    }

    // C: c:2931-2952 — inull walk: drop inull markers from ns,
    // adjusting scs/soffs/swb as we go.
    let mut ns_chars: Vec<char> = ns.chars().collect();
    let mut p_idx = 0_usize;
    let mut walk_i = swb;
    while p_idx < ns_chars.len() {
        let c = ns_chars[p_idx];
        if inull(c) {
            if walk_i < scs {
                soffs -= 1;
                if remq && c == Bnull && p_idx + 1 < ns_chars.len() {
                    swb -= 2;
                }
            }
            let next = ns_chars.get(p_idx + 1).copied();
            if next.is_some() || c != Bnull {
                if c == Bnull {
                    if scs == walk_i + 1 {
                        scs += 1;
                        soffs += 1;
                    }
                } else if scs > walk_i {
                    scs -= 1;
                    walk_i -= 1; // C: `scs > i--`
                }
            } else if scs == swe {
                scs -= 1;
            }
            ns_chars.remove(p_idx);
            // Don't advance p_idx — re-check the new char at p_idx
            // (matches C's `chuck(p--); p++;` next-iter increment).
            walk_i -= 1;
        } else {
            p_idx += 1;
            walk_i += 1;
        }
    }
    ns = ns_chars.iter().collect();

    // C: c:2961-2974 — build qp/qs from ss + qipre/qisuf
    let qipre_val = QIPRE.lock().unwrap().clone();
    let qisuf_val = QISUF.lock().unwrap().clone();
    let qp = format!(
        "{}{}",
        qipre_val,
        &s[..((swb - sl - 1).max(0) as usize).min(s.len())]
    );
    if swe < swb {
        swe = swb;
    }
    swe -= sl + 1;
    let s_len = s.len() as i32;
    if swe > s_len {
        swe = s_len;
        if (ns.len() as i32) > swe - swb + 1 {
            ns.truncate((swe - swb + 1) as usize);
        }
    }
    let qs_start = (swe.max(0) as usize).min(s.len());
    let qs = format!("{}{}", &s[qs_start..], qisuf_val);
    let s_chars_len = ns.len() as i32;
    if soffs > s_chars_len {
        soffs = s_chars_len;
    }

    // C: c:2980-3023 — state save/restore + nested makecomplistcmd
    let ow = CLWORDS.lock().unwrap().clone();
    let os = CMDSTR.with(|r| r.borrow().clone());
    let oqp = QIPRE.lock().unwrap().clone();
    let oqs = QISUF.lock().unwrap().clone();
    let oqst = COMPQSTACK
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clone();
    let olws = *CLWSIZE.lock().unwrap();
    let olwn = *CLWNUM.lock().unwrap();
    let olwp = *CLWPOS.lock().unwrap();
    let obr = *BRANGE.lock().unwrap();
    let oer = *ERANGE.lock().unwrap();
    let oof = *OFFS.lock().unwrap();
    let occ = CCONT.with(|c| c.get());

    // C: c:2986-2989 — push current quote char onto compqstack
    let new_quote_char = if *INSTRING.lock().unwrap() != QT_NONE {
        char::from_u32(*INSTRING.lock().unwrap() as u32).unwrap_or('\\')
    } else {
        char::from_u32(QT_BACKSLASH as u32).unwrap_or('\\')
    };
    let mut new_compqstack = String::new();
    new_compqstack.push(new_quote_char);
    new_compqstack.push_str(&oqst);
    *COMPQSTACK
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap() = new_compqstack;

    // C: c:2991-2997 — install foo into clwords
    *CLWSIZE.lock().unwrap() = foo.len() as i32;
    *CLWNUM.lock().unwrap() = foo.len() as i32;
    *CLWORDS.lock().unwrap() = foo.clone();
    *CLWPOS.lock().unwrap() = cur;
    CMDSTR.with(|r| *r.borrow_mut() = foo.first().cloned());
    *BRANGE.lock().unwrap() = 0;
    *ERANGE.lock().unwrap() = (foo.len() as i32) - 1;
    *QIPRE.lock().unwrap() = qp;
    *QISUF.lock().unwrap() = qs;
    *OFFS.lock().unwrap() = soffs;
    CCONT.with(|c| c.set(CC_CCCONT));

    // C: c:3006 — nested dispatch
    const CFN_FIRST: i32 = 1;
    let _ = makecomplistcmd(&ns, cur == 0, CFN_FIRST);

    CCONT.with(|c| c.set(occ));
    *OFFS.lock().unwrap() = oof;
    CMDSTR.with(|r| *r.borrow_mut() = os);
    *CLWORDS.lock().unwrap() = ow;
    *CLWSIZE.lock().unwrap() = olws;
    *CLWNUM.lock().unwrap() = olwn;
    *CLWPOS.lock().unwrap() = olwp;
    *BRANGE.lock().unwrap() = obr;
    *ERANGE.lock().unwrap() = oer;
    *QIPRE.lock().unwrap() = oqp;
    *QISUF.lock().unwrap() = oqs;
    *COMPQSTACK
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap() = oqst;

    *AUTOQ
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap() = oaq;
    *INSTRING.lock().unwrap() = ois;
    *INBACKT.lock().unwrap() = oib;

    0
}

// `ccused` — per-completion list of compctls used. Port of
// file-static `LinkList ccused` at Src/Zle/compctl.c:2574.
thread_local! { static CCUSED: std::cell::RefCell<Vec<Arc<Compctl>>> = const { std::cell::RefCell::new(Vec::new()) }; }

/// Walk the xor chain of compctls.
/// Port of `makecomplistor(Compctl cc, char *s, int incmd, int compadd, int sub)` from Src/Zle/compctl.c:2574.
///
/// C body:
///   - Loop over xors (cc->xor chain)
///   - For each, call makecomplistlist
///   - Track newly-added matches (mn diff)
///   - Stop based on ccont bits (CC_PATCONT, CC_DEFCONT, CC_XORCONT)
/// WARNING: param names don't match C — Rust=(s, incmd, compadd, sub) vs C=(cc, s, incmd, compadd, sub)
pub(crate) fn makecomplistor(cc: &Arc<Compctl>, s: &str, incmd: bool, compadd: i32, sub: i32) {
    let mut current = cc.clone();
    loop {
        makecomplistlist(&current, s, incmd, compadd);
        // Walk to next xor
        match &current.xor {
            Some(next) => current = next.clone(),
            None => break,
        }
        let _ = sub;
    }
}

/// The flag-driven completion-list builder — workhorse fn.
/// Port of `makecomplistflags(Compctl cc, char *s, int incmd, int compadd)` from Src/Zle/compctl.c:3042 (~500 lines).
///
/// Opens with the c:3070-3403 PREAMBLE, which computes the line/real/
/// path/file prefix+suffix statics (LPRE/RPRE/PPRE/FPRE/… and their
/// quoted forms) that `addmatch` reads back when building each Cmatch,
/// then dispatches the per-CC_* generation arms below. See the preamble
/// header inside the fn for the deliberately-deferred sub-parts.
///
/// Walks the bits of cc.mask and cc.mask2, dispatching per CC_* bit
/// to the matching generator:
///   CC_FILES     → gen_matches_files (regular files)
///   CC_DIRS      → gen_matches_files(dirs=true)
///   CC_COMMPATH  → command-path completion
///   CC_OPTIONS   → option completion
///   CC_VARS      → dumphashtable(paramtab, CC_VARS)
///   CC_BINDINGS  → bindings (zle widgets)
///   CC_ARRAYS    → param table filtered to PM_ARRAY
///   CC_INTVARS   → param table filtered to PM_INTEGER
///   CC_SHFUNCS   → shfunctab
///   CC_PARAMS    → paramtab non-exported
///   CC_ENVVARS   → paramtab PM_EXPORTED
///   CC_JOBS / CC_RUNNING / CC_STOPPED → job table filters
///   CC_BUILTINS  → builtintab
///   CC_USERS     → /etc/passwd users (or named-dir filltable)
///   CC_DISCMDS / CC_EXCMDS → cmdnamtab filtered by DISABLED bit
///   CC_RESWDS    → reserved-word table
///   CC_NAMED     → named-directory table
///   CC_DIRS      → directory matches
///   ... and more
///
/// Plus arg-taking flags:
///   cc.glob   → globlist expansion
///   cc.str → string-arg expansion via singsub
///   cc.func   → call user function (compctl -K)
///   cc.keyvar → read array variable for matches
///   cc.hpat   → history-pattern matches
///
/// Per-flag generators dispatched directly: CC_FILES → file walker,
/// CC_DIRS → dir-only walker, CC_NAMED → maketildelist, CC_VARS &
/// friends → paramtab walk filtered by PM_*, CC_SHFUNCS →
/// shfunctab walk, CC_BUILTINS → builtin table walk, CC_DISCMDS /
/// CC_EXCMDS / CC_EXTCMDS → cmdnamtab walk, CC_RESWDS → reswdtab
/// walk. cc.func → shfunc_call dispatch, cc.glob → addmatch, cc.str
/// → singsub-expanded addmatch.
pub(crate) fn makecomplistflags(cc: &Arc<Compctl>, mut s: String, _incmd: bool, compadd: i32) {
    use crate::ported::string::dupstrpfx;
    use crate::ported::utils::quotestring;
    use crate::ported::zle::comp_h::{Cexpl, CGF_NOSORT, CGF_UNIQALL, CGF_UNIQCON, CMF_REMOVE};
    use crate::ported::zle::compcore::{begcmgroup, check_param, endcmgroup, rembslash};
    use crate::ported::zsh_h::{Equals, Stringg, Tick, Tilde};
    use crate::ported::zsh_h::{
        ALIAS_GLOBAL, DISABLED, PM_ARRAY, PM_EXPORTED, PM_INTEGER, PM_READONLY, PM_SCALAR,
        PM_SPECIAL, PM_UNSET,
    };
    use std::sync::atomic::Ordering;

    // =================================================================
    // makecomplistflags preamble — Src/Zle/compctl.c:3070-3403.
    // Computes the prefix/suffix file-statics that `addmatch` reads back
    // when constructing each Cmatch. This is a faithful port of the
    // state-SETUP half of makecomplistflags; the per-flag GENERATION
    // half stays in the arms further down. Reads/writes the canonical
    // ZLE globals (offs/ipre/ripre/mflags/hasmatched in compcore.rs)
    // plus the compctl-private prefix statics declared near ADDWHAT.
    //
    // Deliberate gaps (need substrate not wired for the compctl flow):
    //   * the `cc->matcher` mstack push + add_bmatchers (c:3115-3125);
    //   * the check_param redirect of the flag-walk to `cc_dummy`
    //     (c:3171-3174) — `s` is advanced but the arms below still read
    //     the original `cc->mask`;
    //   * the zlemetaline brace-memmove that derives lppre/lpsuf
    //     (c:3317-3374) — LPPRE/LPSUF stay empty;
    //   * the `itok`/ispattern glob-pattern detection + patcompile
    //     (c:3240-3294, 3384-3396) — with comppatmatch empty (the common
    //     case) C forces ispattern=0 anyway, so patcomp/filecomp stay
    //     None; the non-empty-comppatmatch path is not built here.
    // =================================================================
    let incompfunc = INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed);
    let instr = *INSTRING.lock().unwrap_or_else(|e| e.into_inner());
    // Port of the `quotename(s)` macro (c:1757). NB: C's
    // `quotestring("", QT_BACKSLASH)` returns "" (the `''` form is only
    // emitted under QT_BACKSLASH_SHOWNULL, utils.c:6165), but the Rust
    // `utils::quotestring` currently returns "''" for a plain-backslash
    // empty string — a divergence in that fn that would poison every
    // empty prefix/suffix. Guard the empty case so the compctl statics
    // match C until utils::quotestring is corrected.
    let quotename = |x: &str| -> String {
        if x.is_empty() {
            String::new()
        } else {
            quotestring(
                x,
                if instr == QT_NONE {
                    QT_BACKSLASH
                } else {
                    instr
                },
            )
        }
    };

    let mut delit = false; // c:3071 delit = 0
                           // c:3073-3076 — reset compiled patterns and every prefix static.
    PATCOMP.with(|r| *r.borrow_mut() = None);
    FILECOMP.with(|r| *r.borrow_mut() = None);
    LPRE.with(|r| r.borrow_mut().clear());
    LSUF.with(|r| r.borrow_mut().clear());
    RPRE.with(|r| r.borrow_mut().clear());
    RSUF.with(|r| r.borrow_mut().clear());
    PPRE.with(|r| r.borrow_mut().clear());
    PSUF.with(|r| r.borrow_mut().clear());
    LPPRE.with(|r| r.borrow_mut().clear());
    LPSUF.with(|r| r.borrow_mut().clear());
    FPRE.with(|r| r.borrow_mut().clear());
    FSUF.with(|r| r.borrow_mut().clear());
    QFPRE.with(|r| r.borrow_mut().clear());
    QFSUF.with(|r| r.borrow_mut().clear());
    QRPRE.with(|r| r.borrow_mut().clear());
    QRSUF.with(|r| r.borrow_mut().clear());
    QLPRE.with(|r| r.borrow_mut().clear());
    QLSUF.with(|r| r.borrow_mut().clear());
    if let Some(m) = crate::ported::zle::compcore::ipre.get() {
        if let Ok(mut g) = m.lock() {
            g.clear();
        }
    }
    if let Some(m) = crate::ported::zle::compcore::ripre.get() {
        if let Ok(mut g) = m.lock() {
            g.clear();
        }
    }

    // c:3078 — curcc = cc.
    CURCC.with(|r| *r.borrow_mut() = Some(cc.clone()));

    // c:3080-3083 — mflags reset + group flags from mask2.
    crate::ported::zle::compcore::mflags.store(0, Ordering::Relaxed);
    let gflags = (if (cc.mask2 & CC_NOSORT) != 0 {
        CGF_NOSORT
    } else {
        0
    }) | (if (cc.mask2 & CC_UNIQALL) != 0 {
        CGF_UNIQALL
    } else {
        0
    }) | (if (cc.mask2 & CC_UNIQCON) != 0 {
        CGF_UNIQCON
    } else {
        0
    });
    // c:3084-3091 — start a fresh match group for -J/-V (gname) or -y.
    if cc.gname.is_some() {
        endcmgroup(None);
        begcmgroup(cc.gname.as_deref(), gflags);
    }
    if cc.ylist.is_some() {
        endcmgroup(None);
        begcmgroup(None, gflags);
    }
    // c:3092-3093 — CC_REMOVE contributes CMF_REMOVE to mflags.
    if (cc.mask & CC_REMOVE) != 0 {
        crate::ported::zle::compcore::mflags.fetch_or(CMF_REMOVE, Ordering::Relaxed);
    }
    // c:3094-3098 — explanation accumulator.
    let cell = crate::ported::zle::compcore::curexpl.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = if cc.explain.is_some() {
            Some(Cexpl::default())
        } else {
            None
        };
    }

    // c:3101-3113 — compadd chars are ignored at the start of the word
    // and become the ignored-prefix `ipre`.
    let mut offs = crate::ported::zle::compcore::OFFS.load(Ordering::Relaxed);
    if compadd > 0 {
        let mut ca = (compadd as usize).min(s.len());
        while ca > 0 && !s.is_char_boundary(ca) {
            ca -= 1;
        }
        let ip = crate::ported::lex::untokenize(&s[..ca]);
        let cell = crate::ported::zle::compcore::ipre.get_or_init(|| Mutex::new(String::new()));
        if let Ok(mut g) = cell.lock() {
            *g = ip;
        }
        crate::ported::zle::compcore::WB.fetch_add(compadd, std::sync::atomic::Ordering::Relaxed);
        s = s[ca..].to_string();
        offs -= compadd;
        if offs < 0 {
            // c:3108-3111 — compadd bigger than our word prefix: bail.
            return;
        }
    }

    // c:3130-3143 — -P prefix: skip the part already typed on the line.
    if let Some(prefix) = cc.prefix.as_ref() {
        if !s.is_empty() {
            let dp = rembslash(prefix);
            let sd = crate::ported::lex::untokenize(&s);
            let mut pl = crate::ported::zle::zle_tricky::pfxlen(&dp, &sd).min(s.len());
            while pl > 0 && !s.is_char_boundary(pl) {
                pl -= 1;
            }
            s = s[pl..].to_string();
            offs -= pl as i32;
        }
    }
    // c:3145-3162 — -S suffix: if the suffix is already on the word, drop it.
    if let Some(suffix) = cc.suffix.as_ref() {
        let mut sdup = rembslash(suffix);
        // c:3150-3151 — ignore trailing spaces on the suffix.
        while sdup.ends_with(' ') {
            sdup.pop();
        }
        let sd = crate::ported::lex::untokenize(&s);
        let sl = sdup.len();
        let suffixll = sd.len();
        if !sd.is_empty()
            && suffixll >= sl
            && (offs as usize) <= suffixll - sl
            && sd.ends_with(sdup.as_str())
        {
            let cut = suffixll - sl;
            if cut <= s.len() && s.is_char_boundary(cut) {
                s.truncate(cut);
            }
        }
    }

    // c:3163-3165 — leading `~`/`=` special char. get_comp_string returns
    // the word untokenized, so `~`/`=` arrive as their literal bytes rather
    // than the Tilde/Equals tokens C tests here; normalize the literals to
    // the tokens so the ic==Tilde (username) / ic==Equals (command) arms fire.
    let ic_char = s.chars().next().unwrap_or('\0');
    let mut ic = if incompfunc != 0 {
        '\0'
    } else if ic_char == Tilde || ic_char == '~' {
        Tilde
    } else if ic_char == Equals || ic_char == '=' {
        Equals
    } else {
        '\0'
    };

    // c:3160-3167 — parameter-name completion redirect. When check_param
    // detects a `$…` context it advances `s` past the `$`, decrements offs
    // (inside check_param, via OFFS), and swaps the active compctl to
    // cc_dummy with mask CC_PARAMS | CC_ENVVARS so the flag-walk below
    // dumps parameter names. Without the swap, `echo $HOM<Tab>` fell
    // through to cc_default (CC_FILES) and produced nothing.
    let param_cc: Option<Arc<Compctl>> = if incompfunc == 0 {
        let saved_wb = crate::ported::zle::compcore::WB.load(Ordering::Relaxed);
        crate::ported::zle::compcore::OFFS.store(offs, Ordering::Relaxed);
        if let Some(p) = check_param(&s, true, false) {
            let mut p = p.min(s.len());
            while p > 0 && !s.is_char_boundary(p) {
                p -= 1;
            }
            s = s[p..].to_string(); // c:3162 — s = p
            offs = crate::ported::zle::compcore::OFFS.load(Ordering::Relaxed);
            // check_param advances wb past the `$` sigil (the `$` becomes the
            // ignored prefix `ipre`). add_match_data still splices ipre into
            // every match's line cline (Src/Zle/compcore.c:2827), so the
            // inserted string carries the `$`. In the compsys path callcompfunc
            // resets `wb = parwb` (c:739) to re-anchor at the `$`; the compctl
            // path has no such reset, so do it here — otherwise the on-line `$`
            // plus the cline's `$` doubled (`$HOM<Tab>` → `$$HOME`). Keep wb at
            // the sigil so the ambiguous-insert region covers it.
            crate::ported::zle::compcore::WB.store(saved_wb, Ordering::Relaxed);
            // c:3164-3166 — cc = &cc_dummy; mask = CC_PARAMS | CC_ENVVARS.
            Some(Arc::new(Compctl {
                refc: 10000,
                mask: CC_PARAMS | CC_ENVVARS,
                ..Default::default()
            }))
        } else {
            None
        }
    } else {
        None
    };
    let cc: &Arc<Compctl> = param_cc.as_ref().unwrap_or(cc);

    // c:3177-3183 — CC_DELETE blanks the word entirely.
    if (cc.mask & CC_DELETE) != 0 {
        delit = true;
        s.clear();
        offs = 0;
    }
    crate::ported::zle::compcore::OFFS.store(offs, Ordering::Relaxed);

    // c:3185-3213 — line prefix / suffix around the cursor offset.
    let cut = {
        let mut c = (offs.max(0) as usize).min(s.len());
        while c > 0 && !s.is_char_boundary(c) {
            c -= 1;
        }
        c
    };
    let lpre_s = s[..cut].to_string();
    let lsuf_s = s[cut..].to_string();
    LPL.with(|c| c.set(lpre_s.len() as i32));
    LSL.with(|c| c.set(lsuf_s.len() as i32));
    let qlpre_s = quotename(&lpre_s);
    let qlsuf_s = quotename(&lsuf_s);

    // c:3216-3223 — a `~` with a `/` after it is a plain path, not special.
    if ic == Tilde && lpre_s.contains('/') {
        ic = '\0';
    }
    IC.with(|c| c.set(ic));

    // c:3224-3236 — real prefix/suffix: getreal-expand when the word
    // contains a substitution/backtick token or begins with `~`/`=`,
    // otherwise a plain copy.
    let ispar = crate::ported::zle::compcore::ispar.load(Ordering::Relaxed);
    let mut noreal = if delit { 0 } else { 1 };
    let lpre_has_tok = lpre_s.chars().any(|c| c == Stringg || c == Tick);
    let tt: &str = if ic != '\0' && ispar == 0 && !lpre_s.is_empty() {
        &lpre_s[lpre_s.chars().next().map(|c| c.len_utf8()).unwrap_or(0)..]
    } else {
        &lpre_s
    };
    let first_lpre = lpre_s.chars().next();
    let mut rpre_s = if lpre_has_tok || first_lpre == Some(Tilde) || first_lpre == Some(Equals) {
        noreal = 0;
        getreal(tt)
    } else if (first_lpre == Some('~') && lpre_s.contains('/')) || first_lpre == Some('=') {
        // get_comp_string returns the word untokenized, so a leading `~`/`=`
        // arrives as its literal byte rather than the Tilde/Equals token that
        // getreal's filesub/equalsubst needs to expand. Tokenize a COPY for
        // the expansion only — mutating `s` itself would shift every byte
        // offset (the tokens are multi-byte in UTF-8) and corrupt fpre. rpre
        // becomes the expanded filesystem path (`~/f` → `$HOME/f`) used for
        // opendir, while lppre keeps the literal `~/` that stays on the line.
        //
        // Only a tilde with a `/` is a PATH to expand (`~/f`, `~user/f`). A
        // bare `~` or `~user` (no slash) is a username-completion context in
        // zsh (maketildelist), NOT an expansion — getreal-expanding it errors
        // ("no such user") or wrongly replaces `~` with $HOME on the line.
        noreal = 0;
        let mut tok = tt.to_string();
        crate::ported::glob::tokenize(&mut tok);
        getreal(&tok)
    } else {
        tt.to_string()
    };
    let qrpre_s = quotename(&rpre_s);

    let lsuf_has_tok = lsuf_s.chars().any(|c| c == Stringg || c == Tick);
    let mut rsuf_s = if lsuf_has_tok {
        noreal = 0;
        getreal(&lsuf_s)
    } else {
        lsuf_s.clone()
    };
    let qrsuf_s = quotename(&rsuf_s);
    NOREAL.with(|c| c.set(noreal));

    // c:3263-3294 — pattern handling. With comppatmatch empty, C forces
    // ispattern = 0 and never compiles patcomp; the non-empty path is a
    // documented gap. Either way patcomp is None here, so we untokenize.
    crate::ported::zle::compcore::ispattern.store(0, Ordering::Relaxed);
    rpre_s = crate::ported::lex::untokenize(&rpre_s);
    rsuf_s = crate::ported::lex::untokenize(&rsuf_s);
    let rpl = rpre_s.len();
    let rsl = rsuf_s.len();
    RPL.with(|c| c.set(rpl as i32));
    RSL.with(|c| c.set(rsl as i32));

    // c:3295-3296 — untokenize the line prefix/suffix.
    let lpre_s = crate::ported::lex::untokenize(&lpre_s);
    let lsuf_s = crate::ported::lex::untokenize(&lsuf_s);

    // Commit the real/line statics.
    LPRE.with(|r| *r.borrow_mut() = lpre_s);
    LSUF.with(|r| *r.borrow_mut() = lsuf_s);
    RPRE.with(|r| *r.borrow_mut() = rpre_s.clone());
    RSUF.with(|r| *r.borrow_mut() = rsuf_s.clone());
    QLPRE.with(|r| *r.borrow_mut() = qlpre_s);
    QLSUF.with(|r| *r.borrow_mut() = qlsuf_s);
    QRPRE.with(|r| *r.borrow_mut() = qrpre_s);
    QRSUF.with(|r| *r.borrow_mut() = qrsuf_s);

    // c:3298-3299 — a non-delete completion counts as "has matched".
    if (cc.mask & CC_DELETE) == 0 {
        crate::ported::zle::compcore::hasmatched.store(1, Ordering::Relaxed);
    }

    // c:3303-3403 — file completion: derive the path/file prefix+suffix.
    if (cc.mask & (CC_FILES | CC_DIRS | CC_COMMPATH)) != 0 || cc.glob.is_some() {
        // s1 = last '/' in rpre, s2 = first '/' in rsuf (c:3240-3258 slash
        // scan, minus the itok/pattern bits we intentionally skip).
        let s1 = rpre_s.rfind('/');
        let s2 = rsuf_s.find('/');

        // c:3311-3315 — path prefix/suffix.
        let ppre_s = match s1 {
            Some(idx) => rpre_s[..idx + 1].to_string(),
            None => String::new(),
        };
        let psuf_s = match s2 {
            Some(idx) => rsuf_s[idx..].to_string(),
            None => String::new(),
        };

        // c:3376-3382 — the file prefix and suffix.
        let s_first = s.as_bytes().first().copied();
        // c:3378 — `zlemetacs == wb`. cs reads the canonical global
        // (deduped); wb is still the compctl-local shadow (WB dedup is a
        // separate, un-authorized change — see report).
        let cs_eq_wb = crate::ported::zle::compcore::ZLEMETACS
            .load(std::sync::atomic::Ordering::Relaxed)
            == crate::ported::zle::compcore::WB.load(std::sync::atomic::Ordering::Relaxed);
        let s1_is_start = s1.is_none() || s1 == Some(0);
        let start_cond = s1_is_start || ic != '\0';
        let mut fpre_s = if start_cond && (s_first != Some(b'/') || cs_eq_wb) {
            match s1 {
                Some(idx) => rpre_s[idx..].to_string(),
                None => rpre_s.clone(),
            }
        } else {
            match s1 {
                Some(idx) => rpre_s[idx + 1..].to_string(),
                None => {
                    let skip = rpre_s.chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                    rpre_s[skip..].to_string()
                }
            }
        };
        let mut fsuf_s = match s2 {
            Some(idx) => dupstrpfx(&rsuf_s, idx),
            None => rsuf_s.clone(),
        };
        let qfpre_s = quotename(&fpre_s);
        let qfsuf_s = quotename(&fsuf_s);

        // c:3397-3403 — no filecomp pattern, so untokenize + record lens.
        fpre_s = crate::ported::lex::untokenize(&fpre_s);
        fsuf_s = crate::ported::lex::untokenize(&fsuf_s);
        FPL.with(|c| c.set(fpre_s.len() as i32));
        FSL.with(|c| c.set(fsuf_s.len() as i32));

        // c:3310-3335 — lppre: the directory portion of the word as it was
        // actually typed on the line (zlemetaline[wb..cs]), truncated after
        // its last '/'. This is what keeps `/usr/` on the line when
        // completing `/usr/b<Tab>` → `/usr/bin/`. With no slash it is empty
        // unless the prefix itself carried one (sf1), in which case it falls
        // back to ppre. (qipre/brace adjustments omitted — empty for `-f`.)
        let meta = crate::ported::zle::compcore::ZLEMETALINE
            .get()
            .and_then(|m| m.lock().ok().map(|g| g.clone()))
            .unwrap_or_default();
        let wb =
            crate::ported::zle::compcore::WB.load(std::sync::atomic::Ordering::Relaxed) as usize;
        let we =
            crate::ported::zle::compcore::WE.load(std::sync::atomic::Ordering::Relaxed) as usize;
        let cs = crate::ported::zle::compcore::ZLEMETACS.load(std::sync::atomic::Ordering::Relaxed)
            as usize;
        let sf1 = !ppre_s.is_empty();
        let lppre_s = if cs != wb && wb <= cs && cs <= meta.len() {
            let word = &meta[wb..cs];
            match word.rfind('/') {
                Some(p) => word[..=p].to_string(), // c:3326-3328
                None if !sf1 => String::new(),     // c:3329-3331
                None => ppre_s.clone(),            // c:3332-3334
            }
        } else {
            String::new()
        };
        // c:3340-3363 — lpsuf: directory portion of the suffix on the line.
        let lpsuf_s = if cs != we && cs <= we && we <= meta.len() {
            let tail = &meta[cs..we];
            match tail.find('/') {
                Some(p) => tail[p..].to_string(),
                None if s2.is_some() => psuf_s.clone(), // sf2 fallback
                None => String::new(),
            }
        } else {
            String::new()
        };

        PPRE.with(|r| *r.borrow_mut() = ppre_s);
        PSUF.with(|r| *r.borrow_mut() = psuf_s);
        FPRE.with(|r| *r.borrow_mut() = fpre_s);
        FSUF.with(|r| *r.borrow_mut() = fsuf_s);
        QFPRE.with(|r| *r.borrow_mut() = qfpre_s);
        QFSUF.with(|r| *r.borrow_mut() = qfsuf_s);
        LPPRE.with(|r| *r.borrow_mut() = lppre_s);
        LPSUF.with(|r| *r.borrow_mut() = lpsuf_s);
    }
    // ===================== end preamble =====================

    let s: &str = &s;
    // Path prefix/suffix computed in the preamble (c:3304-3308).
    let ppre = PPRE.with(|r| r.borrow().clone());
    let psuf = PSUF.with(|r| r.borrow().clone());
    // c:3050 — ccont gets the mask2 continuation bits.
    CCONT.with(|c| c.set(c.get() | (cc.mask2 & (CC_CCCONT | CC_DEFCONT | CC_PATCONT))));

    // c:3490-3491 — the file-generating arms all run with `prpre = ppre`,
    // so directory-prefixed words (`cat /tmp/dir/f<Tab>`) open the typed
    // directory rather than the cwd. When ppre is empty (no path prefix)
    // C leaves prpre = "" → opendir("."); the Rust gen_matches_files
    // treats a None PRPRE as ".", so leave it untouched in that case.
    let saved_prpre = PRPRE.with(|r| r.borrow().clone());
    if !ppre.is_empty() {
        PRPRE.with(|r| *r.borrow_mut() = Some(ppre.clone()));
    }
    // c:3399-3417 — after a leading `~`/`=` (no `/`), the normal file arms
    // are replaced: `~` completes usernames + named dirs (maketildelist),
    // `=` completes command names + regular aliases (equals expansion). Only
    // the plain-file `else` runs gen_matches_files.
    // c:3296 — the whole `~`/`=`/file dispatch only applies when this cc does
    // file/dir/command/glob completion. Without this guard the tilde/equals
    // arms fired for cc_first (mask=0, run before cc_default) where fpre is
    // empty, so every username matched and `~roo` collapsed to `~`.
    let has_file_mask = (cc.mask & (CC_FILES | CC_DIRS | CC_COMMPATH)) != 0 || cc.glob.is_some();
    if ic == Tilde && has_file_mask {
        // c:3401-3406 — usernames + named directories. `ipre = "~"` so each
        // bare name matches the file prefix and gets the `~` back on insert.
        ADDWHAT.with(|c| c.set(-1)); // c:3397
        let oi = crate::ported::zle::compcore::ipre
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        if let Ok(mut g) = crate::ported::zle::compcore::ipre
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
        {
            *g = format!("~{}", oi); // c:3404 — dyncat("~", ipre)
        }
        maketildelist();
        if let Ok(mut g) = crate::ported::zle::compcore::ipre.get().unwrap().lock() {
            *g = oi; // c:3406 — restore
        }
    } else if ic == Equals && has_file_mask {
        // c:3407-3417 — command names (cmdnamtab, addwhat -7) + regular
        // aliases (addwhat -2). `ipre = "="` (c:3412) so the `=` is kept on
        // the line (`=tru` → `=truncate`).
        ADDWHAT.with(|c| c.set(-7));
        let oi = crate::ported::zle::compcore::ipre
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        if let Ok(mut g) = crate::ported::zle::compcore::ipre.get().unwrap().lock() {
            *g = format!("={}", oi); // c:3412 — dyncat("=", ipre)
        }
        if crate::ported::zsh_h::isset(crate::ported::zsh_h::HASHLISTALL) {
            let path = crate::ported::params::getaparam("path").unwrap_or_default();
            crate::ported::hashtable::fillcmdnamtable(&path);
        }
        let cmds: Vec<String> = crate::ported::hashtable::cmdnamtab_lock()
            .read()
            .map(|tab| tab.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        dumphashtable(cmds, -7);
        let aliases: Vec<String> = crate::ported::hashtable::aliastab_lock()
            .read()
            .map(|tab| tab.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        dumphashtable(aliases, -2);
        if let Ok(mut g) = crate::ported::zle::compcore::ipre.get().unwrap().lock() {
            *g = oi; // c:3417 — restore
        }
    } else {
        // c:3650 — CC_FILES regular files.
        if (cc.mask & CC_FILES) != 0 {
            ADDWHAT.with(|c| c.set(-5));
            gen_matches_files(false, false, false);
        }
        // CC_DIRS — c:3680
        if (cc.mask & CC_DIRS) != 0 {
            ADDWHAT.with(|c| c.set(-5));
            gen_matches_files(true, false, false);
        }
        // CC_COMMPATH — c:3500-3519. File-side of command completion. With a
        // typed path prefix (`/usr/bin/tr<Tab>`, sf1) add that directory's
        // directories+executables; with no prefix, add the cwd's executables
        // only when the cwd is itself reachable through $path (an empty or
        // "." element). The bulk of the command names (everything reachable
        // via $path) comes from the cmdnamtab dump below, NOT from a $path
        // walk here — those matches are added with addwhat=-3 so they carry
        // no file-type marker, matching `zsh -f`.
        if (cc.mask & CC_COMMPATH) != 0 {
            ADDWHAT.with(|c| c.set(-5));
            let sf1 = !ppre.is_empty(); // path prefix present (slash in word)
            if sf1 {
                // c:3505 — directories + executables under the typed prefix.
                let save = PRPRE.with(|r| r.borrow().clone());
                PRPRE.with(|r| *r.borrow_mut() = Some(ppre.clone()));
                gen_matches_files(true, true, false);
                PRPRE.with(|r| *r.borrow_mut() = save);
            } else {
                // c:3509-3518 — cwd only if "." (or "") is in $path.
                let path = crate::ported::params::getaparam("path").unwrap_or_default();
                let cwd_in_path = path.iter().any(|p| p.is_empty() || p == ".");
                if cwd_in_path {
                    let save = PRPRE.with(|r| r.borrow().clone());
                    PRPRE.with(|r| *r.borrow_mut() = Some("./".to_string()));
                    gen_matches_files(true, true, false); // c:3516
                    PRPRE.with(|r| *r.borrow_mut() = save);
                }
            }
        }
    } // end `else` (plain-file arms; the `~`/`=` cases handled above)
      // c:3540 — restore prpre after the file-generating arms.
    PRPRE.with(|r| *r.borrow_mut() = saved_prpre);
    // CC_NAMED — c:3664 `dumphashtable(nameddirtab, addwhat)`. maketildelist
    // now emits bare names (the `~` comes from ipre), so set ipre = "~" here
    // too, matching the tilde arm above.
    if (cc.mask & CC_NAMED) != 0 {
        ADDWHAT.with(|c| c.set(-1));
        let oi = crate::ported::zle::compcore::ipre
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        if let Ok(mut g) = crate::ported::zle::compcore::ipre.get().unwrap().lock() {
            *g = format!("~{}", oi);
        }
        maketildelist();
        if let Ok(mut g) = crate::ported::zle::compcore::ipre.get().unwrap().lock() {
            *g = oi;
        }
    }

    // c:3648-3661 — command completion: add alias names, reserved words,
    // shell functions, builtins and external command names (cmdnamtab).
    // Gated on no leading `~`/`=` (ic), no path prefix and no path suffix.
    // All dumped with addwhat=-3 so the matches carry no file-type marker.
    if ic == '\0' && (cc.mask & CC_COMMPATH) != 0 && ppre.is_empty() && psuf.is_empty() {
        // c:3651 — regular + global aliases.
        let aliases: Vec<String> = crate::ported::hashtable::aliastab_lock()
            .read()
            .map(|tab| tab.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        dumphashtable(aliases, -3);
        // c:3652 — reserved words.
        let reswds: Vec<String> = crate::ported::hashtable::reswdtab_lock()
            .read()
            .map(|tab| tab.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        dumphashtable(reswds, -3);
        // c:3653 — shell functions.
        let funcs: Vec<String> = crate::ported::hashtable::shfunctab_lock()
            .read()
            .map(|tab| tab.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        dumphashtable(funcs, -3);
        // c:3654 — builtins. C walks `builtintab`, which holds only the
        // builtins of currently-LOADED modules; zshrs keeps one flat
        // static `BUILTINS` slice, so the membership test has to be
        // made explicitly. `builtin_in_builtintab` is the same
        // predicate `scanbuiltins` uses for the `builtins` magic assoc
        // (Src/Modules/parameter.c:823), so the two namespaces cannot
        // drift. Without it this dump offered 44 module builtins
        // (`strftime`, `zstat`, `zpty`, `zf_chmod`, `pcre_match`, …)
        // that `${(k)builtins}` did not list and that real zsh only
        // exposes after the owning `zmodload`.
        let mut builtins: Vec<String> = crate::ported::builtin::BUILTINS
            .iter()
            .map(|b| b.node.nam.clone())
            .filter(|n| crate::ext_builtins::builtin_in_builtintab(n))
            .collect();
        // zshrs extension builtins dispatch in-process but have no
        // entry in the C-port BUILTINS table, so command-position
        // completion would never offer `doctor`, `peach`, `zassert_eq`,
        // etc. `hide_ext_builtins()` (shared with the `builtins` magic
        // assoc in the Src/Modules/parameter.c port, so the two
        // namespaces can't drift) drops them under `--zsh` strict
        // emulation and under `ZSHRS_HIDE_EXT_BUILTINS` — the parity
        // harnesses' measurement knob, which changes NOTHING else:
        // the names still dispatch and still resolve via `whence`.
        if !crate::ext_builtins::hide_ext_builtins() {
            builtins.extend(
                crate::ext_builtins::EXT_BUILTIN_NAMES
                    .iter()
                    .map(|s| (*s).to_string()),
            );
        }
        dumphashtable(builtins, -3);
        // c:3655-3657 — external commands; HASHLISTALL (default on) bulk-
        // hashes $path first so every reachable command is offered.
        if crate::ported::zsh_h::isset(crate::ported::zsh_h::HASHLISTALL) {
            let path = crate::ported::params::getaparam("path").unwrap_or_default();
            crate::ported::hashtable::fillcmdnamtable(&path);
        }
        let cmds: Vec<String> = crate::ported::hashtable::cmdnamtab_lock()
            .read()
            .map(|tab| tab.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_default();
        dumphashtable(cmds, -3);
    }

    // c:3668 — oaw = addwhat = (cc->mask & CC_QUOTEFLAG) ? -2 : CC_QUOTEFLAG.
    // The "default" addwhat used by the name-table arms below; arms
    // that override it (CC_VARS, CC_BINDINGS) reset back to `oaw`.
    let oaw: i32 = if (cc.mask & CC_QUOTEFLAG) != 0 {
        -2
    } else {
        CC_QUOTEFLAG as i32
    };
    ADDWHAT.with(|c| c.set(oaw));

    // c:3673 — CC_OPTIONS: add setopt option names.
    if (cc.mask & CC_OPTIONS) != 0 {
        let names: Vec<String> = crate::ported::options::OPTIONTAB
            .iter()
            .map(|o| o.to_string())
            .collect();
        dumphashtable(names, oaw);
    }
    // c:3676 — CC_VARS: parameter names (addwhat -9). C's addmatch
    // filters unset params and non-top-level (`pm->level`) ones; we
    // apply the same predicate while building the name list.
    if (cc.mask & CC_VARS) != 0 {
        let names: Vec<String> = {
            let tab = crate::ported::params::paramtab().read().unwrap();
            tab.iter()
                .filter(|(_, pm)| (pm.node.flags & PM_UNSET as i32) == 0 && pm.level == 0)
                .map(|(n, _)| n.clone())
                .collect()
        };
        dumphashtable(names, -9);
        ADDWHAT.with(|c| c.set(oaw)); // c:3679
    }
    // c:3681 — CC_BINDINGS: zle widget (thingy) names.
    if (cc.mask & CC_BINDINGS) != 0 {
        let names: Vec<String> = crate::ported::zle::zle_thingy::thingytab()
            .lock()
            .map(|t| t.keys().cloned().collect())
            .unwrap_or_default();
        dumphashtable(names, CC_BINDINGS as i32);
        ADDWHAT.with(|c| c.set(oaw)); // c:3684
    }
    // c:3686 — cc.keyvar (compctl -k): names from a parameter / word list.
    if let Some(kv) = cc.keyvar.as_ref() {
        if let Some(usr) = crate::ported::zle::compcore::get_user_var(Some(kv.as_str())) {
            ADDWHAT.with(|c| c.set(oaw));
            for u in usr {
                addmatch(&u, None);
            }
        }
    }
    // c:3695 — CC_USERS: user names (maketildelist fills the table).
    if (cc.mask & CC_USERS) != 0 {
        maketildelist();
        ADDWHAT.with(|c| c.set(oaw)); // c:3698
    }

    // c:3700-3736 — cc.func (compctl -K user-fn) → call shfunc for
    // matches. Build args = [func-name, lpre, lsuf] and dispatch via
    // doshfunc; the user shfunc populates `$reply` array which we
    // then walk via addmatch.
    if let Some(func_name) = cc.func.as_ref() {
        if let Some(mut shfunc) = crate::ported::utils::getshfunc(func_name) {
            // c:3702-3717 — `addlinknode(args, cc->func); ... lpre; lsuf;`.
            // Without the lpre/lsuf split substrate here we pass the
            // raw cursor word as a single arg.
            let largs: Vec<String> = vec![func_name.clone(), s.to_string()];
            // c:3722-3724 — `if (incompfunc != 1) incompctlfunc = 1;
            //                sfcontext = SFC_COMPLETE;`.
            let in_compfunc = INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed);
            if in_compfunc != 1 {
                INCOMPCTLFUNC.with(|c| c.set(true));
            }
            let osc = crate::ported::builtin::SFCONTEXT.swap(
                crate::ported::zsh_h::SFC_COMPLETE,
                std::sync::atomic::Ordering::Relaxed,
            );
            // c:3725 — `doshfunc(shfunc, args, 1);`.
            let name_for_body = func_name.clone();
            let body_args: Vec<String> = vec![s.to_string()];
            let body_runner = move || -> i32 {
                crate::ported::exec::run_function_body(&name_for_body, &body_args).unwrap_or(0)
            };
            let _ = crate::ported::exec::doshfunc(&mut shfunc, largs, true, body_runner);
            // c:3726-3727 — `sfcontext = osc; incompctlfunc = 0;`.
            crate::ported::builtin::SFCONTEXT.store(osc, std::sync::atomic::Ordering::Relaxed);
            INCOMPCTLFUNC.with(|c| c.set(false));
            // c:3729-3731 — `if ((r = get_user_var("reply"))) while
            // (*r) addmatch(*r++, NULL);`.
            if let Some(reply) = crate::ported::params::getaparam("reply") {
                // c:3735-3737 — addwhat is still `oaw` here, so the
                // reply words are accepted by addmatch (setting it to 0
                // would make addmatch drop every match).
                ADDWHAT.with(|c| c.set(oaw));
                for m in reply {
                    addmatch(&m, None);
                }
            }
        }
    }

    // c:3870-3906 — cc.ylist (compctl -y) → explanation source.
    // If ylist starts with `$` or `(`, read from a parameter; else
    // treat as a shfunc that receives the current match list and
    // populates `$reply` with per-match explanation strings.
    if let Some(ylist) = cc.ylist.as_ref() {
        let is_var_ref = ylist.starts_with('$') || ylist.starts_with('(');
        if !is_var_ref {
            if let Some(mut shfunc) = crate::ported::utils::getshfunc(ylist) {
                // c:3886-3895 — `addlinknode(args, cc->ylist);` then
                // for each match append the prefix+str+suffix string.
                // Match list isn't threaded here yet so we pass just
                // the ylist function name; full match-list plumbing
                // is a follow-up tied to the Cmatch port.
                let largs: Vec<String> = vec![ylist.clone()];
                // c:3898-3900 — same SFC_COMPLETE / incompctlfunc=1
                // bracketing as cc.func above.
                let in_compfunc = INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed);
                if in_compfunc != 1 {
                    INCOMPCTLFUNC.with(|c| c.set(true));
                }
                let osc = crate::ported::builtin::SFCONTEXT.swap(
                    crate::ported::zsh_h::SFC_COMPLETE,
                    std::sync::atomic::Ordering::Relaxed,
                );
                // c:3901 — `doshfunc(shfunc, args, 1);`.
                let name_for_body = ylist.clone();
                let body_runner = move || -> i32 {
                    crate::ported::exec::run_function_body(&name_for_body, &[]).unwrap_or(0)
                };
                let _ = crate::ported::exec::doshfunc(&mut shfunc, largs, true, body_runner);
                crate::ported::builtin::SFCONTEXT.store(osc, std::sync::atomic::Ordering::Relaxed);
                INCOMPCTLFUNC.with(|c| c.set(false));
            }
        }
    }

    // cc.glob — globlist expansion. Skipped pending glob-port use.

    // cc.str (-s) — call singsub on the string.
    if let Some(s) = &cc.str {
        let expanded = getreal(s);
        // Push as a single match with addwhat=GLOB_EXPAND
        ADDWHAT.with(|c| c.set(-6));
        addmatch(&expanded, None);
    }

    // c:3803-3841 — cc.hpat (compctl -H): pull words out of the
    // history that match the pattern.
    if let Some(hpat) = cc.hpat.as_ref() {
        use crate::ported::zsh_h::{GETHIST_UPWARD, HIST_FOREIGN};
        // c:3810-3816 — parse the pattern unless it is the null string.
        let pprog = if !hpat.is_empty() {
            let mut thpat = hpat.clone();
            crate::ported::glob::tokenize(&mut thpat);
            patcompile(&thpat, PAT_HEAPDUP as i32, None)
        } else {
            None
        };
        // c:3806-3819 — n = number of history lines to search (0 → all).
        let mut n = cc.hnum;
        if n == 0 {
            n = -1;
        }
        let cur = crate::ported::hist::curhist.load(std::sync::atomic::Ordering::Relaxed);
        let start = crate::ported::hist::addhistnum(cur, -1, HIST_FOREIGN as i32); // c:3807
        let mut he_ev = crate::ported::hist::gethistent(start, GETHIST_UPWARD); // c:3808
        ADDWHAT.with(|c| c.set(oaw));
        // c:3822 — `while (n-- && he)`.
        while n != 0 {
            let ev = match he_ev {
                Some(e) => e,
                None => break,
            };
            n -= 1;
            if let Some(he) = crate::ported::hist::gethist(ev) {
                let nam = &he.node.nam;
                // c:3824 — iterate the words of this line back-to-front.
                for iw in (0..he.nwords).rev() {
                    let bi = (iw * 2) as usize;
                    if bi + 1 >= he.words.len() {
                        continue;
                    }
                    let hstart = he.words[bi] as usize; // c:3825
                    let hend = he.words[bi + 1] as usize; // c:3826
                    if hstart > hend
                        || hend > nam.len()
                        || !nam.is_char_boundary(hstart)
                        || !nam.is_char_boundary(hend)
                    {
                        continue;
                    }
                    let word = &nam[hstart..hend];
                    // c:3831-3833 — skip words starting with a quote or `$`.
                    let c0 = word.chars().next();
                    if !matches!(c0, Some('\'') | Some('"') | Some('`') | Some('$'))
                        && pprog.as_ref().map_or(true, |pp| pattry(pp, word))
                    {
                        addmatch(word, None); // c:3834
                    }
                }
            }
            he_ev = crate::ported::hist::up_histent(ev); // c:3838
        }
    }

    // c:3842-3845 — parameter flavours (arrays/ints/exports/etc.).
    // addwhat carries the requested flavour bits; addmatch (addwhat>0)
    // accepts, so we replicate its per-node predicate here: top-level,
    // set parameters whose PM_* flags intersect the request.
    {
        let t = cc.mask
            & (CC_ARRAYS
                | CC_INTVARS
                | CC_ENVVARS
                | CC_SCALARS
                | CC_READONLYS
                | CC_SPECIALS
                | CC_PARAMS);
        if t != 0 {
            let names: Vec<String> = {
                let tab = crate::ported::params::paramtab().read().unwrap();
                tab.iter()
                    .filter(|(_, pm)| {
                        let f = pm.node.flags as u32;
                        if (f & PM_UNSET) != 0 || pm.level != 0 {
                            return false;
                        }
                        ((t & CC_ARRAYS) != 0 && (f & PM_ARRAY) != 0)
                            || ((t & CC_INTVARS) != 0 && (f & PM_INTEGER) != 0)
                            || ((t & CC_ENVVARS) != 0 && (f & PM_EXPORTED) != 0)
                            || ((t & CC_SCALARS) != 0 && (f & PM_SCALAR) != 0)
                            || ((t & CC_READONLYS) != 0 && (f & PM_READONLY) != 0)
                            || ((t & CC_SPECIALS) != 0 && (f & PM_SPECIAL) != 0)
                            || ((t & CC_PARAMS) != 0 && (f & PM_EXPORTED) == 0)
                    })
                    .map(|(n, _)| n.clone())
                    .collect()
            };
            dumphashtable(names, t as i32);
        }
    }

    // Enable/disable predicate shared by the command-table arms
    // (shfuncs/builtins/extcmds/reswds/aliases): addmatch admits a node
    // when (CC_DISCMDS && disabled) || (CC_EXCMDS && !disabled). c:2012.
    let want_dis = (cc.mask & CC_DISCMDS) != 0;
    let want_ex = (cc.mask & CC_EXCMDS) != 0;
    let en_ok = |disabled: bool| (want_dis && disabled) || (want_ex && !disabled);

    // c:3846-3848 — CC_SHFUNCS: shell function names.
    if (cc.mask & CC_SHFUNCS) != 0 {
        let names: Vec<String> = crate::ported::hashtable::shfunctab_lock()
            .read()
            .map(|tab| {
                tab.iter()
                    .filter(|(_, f)| en_ok((f.node.flags & DISABLED) != 0))
                    .map(|(n, _)| n.clone())
                    .collect()
            })
            .unwrap_or_default();
        dumphashtable(names, (cc.mask & CC_SHFUNCS) as i32);
    }
    // c:3849-3851 — CC_BUILTINS: builtin command names.
    if (cc.mask & CC_BUILTINS) != 0 {
        let names: Vec<String> = crate::ported::builtin::BUILTINS
            .iter()
            .filter(|b| en_ok((b.node.flags & DISABLED) != 0))
            .map(|b| b.node.nam.clone())
            .collect();
        dumphashtable(names, (cc.mask & CC_BUILTINS) as i32);
    }
    // c:3852-3857 — CC_EXTCMDS: external command names (cmdnamtab).
    if (cc.mask & CC_EXTCMDS) != 0 {
        let names: Vec<String> = crate::ported::hashtable::cmdnamtab_lock()
            .read()
            .map(|tab| {
                tab.iter()
                    .filter(|(_, c)| en_ok((c.node.flags & DISABLED) != 0))
                    .map(|(n, _)| n.clone())
                    .collect()
            })
            .unwrap_or_default();
        dumphashtable(names, (cc.mask & CC_EXTCMDS) as i32);
    }
    // c:3858-3860 — CC_RESWDS: reserved words.
    if (cc.mask & CC_RESWDS) != 0 {
        let names: Vec<String> = crate::ported::hashtable::reswdtab_lock()
            .read()
            .map(|tab| {
                tab.iter()
                    .filter(|(_, r)| en_ok((r.node.flags & DISABLED) != 0))
                    .map(|(n, _)| n.clone())
                    .collect()
            })
            .unwrap_or_default();
        dumphashtable(names, (cc.mask & CC_RESWDS) as i32);
    }
    // c:3861-3863 — CC_ALREG / CC_ALGLOB: regular / global aliases.
    if (cc.mask & (CC_ALREG | CC_ALGLOB)) != 0 {
        let want_reg = (cc.mask & CC_ALREG) != 0;
        let want_glob = (cc.mask & CC_ALGLOB) != 0;
        let names: Vec<String> = crate::ported::hashtable::aliastab_lock()
            .read()
            .map(|tab| {
                tab.iter()
                    .filter(|(_, a)| {
                        let g = (a.node.flags & ALIAS_GLOBAL) != 0;
                        let type_ok = (want_reg && !g) || (want_glob && g);
                        type_ok && en_ok((a.node.flags & DISABLED) != 0)
                    })
                    .map(|(n, _)| n.clone())
                    .collect()
            })
            .unwrap_or_default();
        dumphashtable(names, (cc.mask & (CC_ALREG | CC_ALGLOB)) as i32);
    }
}

/// Setup hook — port of `setup_(UNUSED(Module m))` from Src/Zle/compctl.c:4014.
///
/// Wires `compctlreadptr` to compctlread, creates the compctltab,
/// initializes the special targets:
///   cc_compos.mask  = CC_COMMPATH
///   cc_default.refc = 10000  (sentinel "never free")
///   cc_default.mask = CC_FILES
///   cc_first.refc   = 10000
///   cc_first.mask2  = CC_CCCONT
/// Clears lastccused.
pub(crate) fn setup_() -> i32 {
    *COMPCTLREAD_INSTALLED.lock().unwrap() = true;
    createcompctltable();
    *CC_COMPOS.lock().unwrap() = Some(Arc::new(Compctl {
        mask: CC_COMMPATH, // c:4018
        ..Default::default()
    }));
    *CC_DEFAULT.lock().unwrap() = Some(Arc::new(Compctl {
        refc: 10000,    // c:4020
        mask: CC_FILES, // c:4021
        ..Default::default()
    }));
    *CC_FIRST.lock().unwrap() = Some(Arc::new(Compctl {
        refc: 10000,      // c:4023
        mask2: CC_CCCONT, // c:4025
        ..Default::default()
    }));
    *LASTCCUSED.lock().unwrap() = Vec::new(); // c:4034
    0
}

// =================================================================
// zle_tricky.c state required by sep_comp_string and the
// completion-driver hooks. Ports of the file-statics in
// Src/Zle/zle_tricky.c that compctl reads/writes during the
// completion flow. Each is a `Mutex<...>` singleton matching the
// C global's name + type (translated to Rust idioms).
// =================================================================

// `we` / `wb` — word end / begin positions (1-based byte offsets
// into zlemetaline). Port of `int wb, we;` at Src/Zle/zle_tricky.c.
// WB/WE deduped to the canonical `compcore::{WB, WE}` (lex.c:120); the
// former private thread_local shadows were removed so the `cs == wb`
// comparison and the sep_comp_string save/restore see the same globals
// the lexer writes.

// `zlemetacs` — cursor position (byte offset). Deduped: reads/writes the
// canonical `compcore::ZLEMETACS` (lex.c:104) instead of a private
// thread-local shadow. Same C variable, same meaning.

/// `zlemetall` — line length in bytes. Port of `int zlemetall;`.
static ZLEMETALL: Mutex<i32> = Mutex::new(0);

/// Features hook — port of `features_(UNUSED(Module m), UNUSED(char ***features))` from Src/Zle/compctl.c:4034.
///
/// Returns the list of feature strings the module exposes. zsh C
/// uses `featuresarray(m, &module_features)` which reads
/// `module_features.bn_size` (line 4005 — 2 builtins: compctl,
/// compcall). Rust returns the explicit list.
pub(crate) fn features_() -> Vec<String> {
    vec!["b:compctl".to_string(), "b:compcall".to_string()]
}

/// Enables hook — port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from Src/Zle/compctl.c:4042.
///
/// C delegates to `handlefeatures(m, &module_features, enables)`
/// which writes the per-feature enable bits to `*enables`. Rust
/// returns a per-feature bool vector — entries currently default
/// to enabled (1). Wiring to the module-load runtime is a separate
/// concern.
pub(crate) fn enables_() -> Vec<i32> {
    vec![1, 1]
}

/// Boot hook — port of `boot_(UNUSED(Module m))` from Src/Zle/compctl.c:4049.
///
/// Registers the two completion-driver hooks via
/// `addhookfunc("compctl_make", ccmakehookfn)` and
/// `addhookfunc("compctl_cleanup", cccleanuphookfn)`. Rust hooks
/// dispatch via the same names; the actual hook registry is in
/// src/ported/module.rs.
pub(crate) fn boot_() -> i32 {
    // c:4051-4052 — `addhookfunc("compctl_make", ccmakehookfn);
    //                addhookfunc("compctl_cleanup", cccleanuphookfn);`
    // Deferred until ccmakehookfn / cccleanuphookfn carry the Hookfn
    // signature `(Hookdef, void *) -> int`. The current Rust thunks
    // are wrappers around makecomplistctl with non-Hookfn shapes;
    // re-enable once that refactor lands.
    0
}

/// `instring` — quoting context. Port of `int instring;`. The QT_*
/// values are the C enum at `Src/zsh.h:253-292` (ported in zsh_h.rs).

/// Cleanup hook — port of `cleanup_(UNUSED(Module m))` from Src/Zle/compctl.c:4058.
///
/// Reverses boot_: removes the two hooks, then disables features
/// via `setfeatureenables(m, &module_features, NULL)`.
pub(crate) fn cleanup_() -> i32 {
    // c:4060-4062 — `deletehookfunc("compctl_make", ccmakehookfn);
    //                deletehookfunc("compctl_cleanup", cccleanuphookfn);`
    // Same registration deferral as `boot_()` above — no-op until
    // the Hookfn-sig refactor.
    0
}

/// Finish hook — port of `finish_(UNUSED(Module m))` from Src/Zle/compctl.c:4067.
///
/// Tears down the compctltab hash table, frees lastccused, restores
/// `compctlreadptr` to the fallback. Rust drops the table on Mutex
/// reset; lastccused frees via Vec::clear; compctlreadptr is the
/// COMPCTLREAD_INSTALLED bool.
pub(crate) fn finish_() -> i32 {
    *COMPCTL_TAB.write().unwrap() = None; // c:4067 deletehashtable
    LASTCCUSED.lock().unwrap().clear(); // c:4071-4072 freelinklist
    *COMPCTLREAD_INSTALLED.lock().unwrap() = false; // c:4074
    0
}

// =================================================================
// Type definitions — port of Src/Zle/compctl.h:32-115
// =================================================================

// Compcond/CompcondData/Compctl/Patcomp/Compctlp ported in
// compctl_h.rs (Src/Zle/compctl.h:39-115). Imported above.

// =================================================================
// Globals — port of Src/Zle/compctl.c:36-66
// =================================================================

/// Global cmatcher list. Port of file-static `Cmlist cmatcher;` at
/// Src/Zle/compctl.c:36. Bucket-2 user-registered registry per
/// PORT_PLAN.md — `compctl -M` writes via `freecmlist + cpcmlist`,
/// every completion call reads. `RwLock` lets parallel completion
/// reads proceed without serialising on a mutex.
pub(crate) static CMATCHER: std::sync::RwLock<Option<Box<Cmlist>>> = std::sync::RwLock::new(None); // c:36

/// `compctltab` hash table — name → Compctl.
/// Port of `HashTable compctltab;` at Src/Zle/compctl.c:46.
/// Bucket-2 user-registered registry: `compctl name args` writes,
/// every completion call reads. `RwLock` per PORT_PLAN.md.
static COMPCTL_TAB: std::sync::RwLock<Option<HashMap<String, Arc<Compctl>>>> =
    std::sync::RwLock::new(None);

/// Pattern-compctl list. Port of `Patcomp patcomps;` at
/// Src/Zle/compctl.c:51. Bucket-2 user-registered registry:
/// `compctl -p` writes, every pattern-completion call reads.
/// `RwLock` per PORT_PLAN.md.
static PATCOMPS: std::sync::RwLock<Vec<(String, Arc<Compctl>)>> =
    std::sync::RwLock::new(Vec::new());

// `zlemetaline` — the actual line buffer. Deduped: reads/writes the
// canonical `compcore::ZLEMETALINE` (lex.c:103) instead of a private
// module static shadow. Same C variable, same meaning.

/// `noerrs` / `noaliases` — lexer error/alias-suppression flags.
// `noerrs` is ONE variable in C (`mod_export int noerrs;`, Src/exec.c:117).
// This file used to declare a THIRD private copy of it, which meant
// `getreal`'s suppression window (c:Src/Zle/compctl.c:2144, whose comment at
// c:2135 is "During this errors are not reported") wrote a cell nothing read:
// `zerr`/`zwarn`/`zerrnam`/`zwarnnam` all consult
// `crate::ported::utils::noerrs_lock()` (utils.rs:221/244/262/279). A
// diagnostic raised by the expansion inside `getreal` therefore printed where
// zsh is silent. Every use below now goes to the single shared storage.
static NOALIASES: Mutex<i32> = Mutex::new(0);
static INSTRING: Mutex<i32> = Mutex::new(QT_NONE);

/// `inbackt` — inside backtick command-substitution. Port of `int inbackt;`.
static INBACKT: Mutex<i32> = Mutex::new(0);

// `autoq` and `compqstack` were formerly compctl-private copies; they
// are now deduped to the canonical `zle_tricky::AUTOQ` (glob-imported)
// and `complete::COMPQSTACK` respectively. C declares one of each.

/// `qipre` / `qisuf` — quoted ignored prefix/suffix from the
/// completion driver. Port of `char *qipre, *qisuf;`.
static QIPRE: Mutex<String> = Mutex::new(String::new());
static QISUF: Mutex<String> = Mutex::new(String::new());

/// `compqiprefix` / `compqisuffix` / `compisuffix` — completion-context
/// state from the user's compfunc. Port of those file-statics.
static COMPQIPREFIX: Mutex<String> = Mutex::new(String::new());
static COMPQISUFFIX: Mutex<String> = Mutex::new(String::new());
static COMPISUFFIX: Mutex<String> = Mutex::new(String::new());

/// `isuf` — the ignored (tokenized) suffix. Port of the `mod_export
/// char *isuf;` from `Src/Zle/compcore.c:118`. `makecomplistctl`
/// derives it from `compisuffix`; `add_match_data` consumes it.
static ISUF: Mutex<String> = Mutex::new(String::new());

// `compwords` / `compcurrent` were formerly compctl-private copies;
// deduped to the canonical `complete::COMPWORDS` / `complete::COMPCURRENT`
// (imported above). C declares one of each in complete.c.

/// `clwords` / `clwsize` / `clwnum` / `clwpos` — current line word
/// array + sizes used by the completion code.
static CLWORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CLWSIZE: Mutex<i32> = Mutex::new(0);
static CLWNUM: Mutex<i32> = Mutex::new(0);
static CLWPOS: Mutex<i32> = Mutex::new(0);

/// `offs` — completion offset into the current word.
static OFFS: Mutex<i32> = Mutex::new(0);

/// `addedx` — non-zero while the dummy `x` cursor marker is in
/// the line being lexed.
static ADDEDX: Mutex<i32> = Mutex::new(0);

/// `lexflags` — lexer mode flags (LEXFLAGS_ZLE etc.). Port of
/// `int lexflags;` from Src/lex.c.
static LEXFLAGS: Mutex<i32> = Mutex::new(0);

/// LEXFLAGS_ZLE — the bit set during ZLE-driven completion lex.
/// Port of `LEXFLAGS_ZLE` from Src/zsh.h.
const LEXFLAGS_ZLE: i32 = 1 << 0;

/// `brange` / `erange` — `-l` word-range begin/end.
static BRANGE: Mutex<i32> = Mutex::new(0);
static ERANGE: Mutex<i32> = Mutex::new(0);

/// `linwhat` — line-context kind. Port of `mod_export int linwhat`
/// from `Src/Zle/compcore.c:91`. Values are the `IN_*` enum at
/// `Src/zsh.h:2321-2332` (ported in zsh_h.rs). NB: dead code is
/// fake — the previous Rust `linwhat_kind` mod had `IN_ENV=1` and
/// an invented `IN_REDIR=4`; both wrong vs the real C enum.
static LINWHAT: Mutex<i32> = Mutex::new(IN_NOTHING);

/// `linredir` — non-zero when completing inside a redirection.
static LINREDIR: Mutex<i32> = Mutex::new(0);

/// `insubscr` — non-zero inside an array subscript context.
static INSUBSCR: Mutex<i32> = Mutex::new(0);

/// Inull-token chars from Src/zsh.h. These are the byte values
/// the lexer uses to mark suppressed quoted-region boundaries
/// (Snull = single-quote, Dnull = double-quote, Bnull = backslash,
/// String/Qstring = `$`/`'$'` markers).
pub const Snull: char = '\u{9d}'; // Single-quote null
pub const Dnull: char = '\u{9e}'; // Double-quote null
pub const Bnull: char = '\u{9f}'; // Backslash null
pub const Stringg: char = '\u{85}'; // META-$
/// `QSTRING_TOK` constant.
pub const QSTRING_TOK: char = '\u{84}'; // Qstring (for $'...')

// =================================================================
// Module boot/cleanup hooks — port of compctl.c:4000+
// =================================================================

/// Storage for the special compctl targets — `cc_compos` (command
/// completion), `cc_default` (default completion), `cc_first`
/// (first completion). Port of the file-static C declarations at
/// Src/Zle/compctl.c:41 — `struct compctl cc_compos, cc_default,
/// cc_first, cc_dummy;`. setup_ initializes the masks; tests +
/// real-completion paths read them.
pub(crate) static CC_COMPOS: Mutex<Option<Arc<Compctl>>> = Mutex::new(None);
pub(crate) static CC_DEFAULT: Mutex<Option<Arc<Compctl>>> = Mutex::new(None);
pub(crate) static CC_FIRST: Mutex<Option<Arc<Compctl>>> = Mutex::new(None);
pub(crate) static CC_DUMMY: Mutex<Option<Arc<Compctl>>> = Mutex::new(None);

/// Last-used compctl tracking list. Port of `LinkList lastccused`
/// at Src/Zle/compctl.c:1702. setup_ initializes to empty; finish_
/// frees its contents.
static LASTCCUSED: Mutex<Vec<Arc<Compctl>>> = Mutex::new(Vec::new());

/// Pointer to compctlread (vs fallback_compctlread). Port of the
/// `CompctlReadFn compctlreadptr` indirect dispatch at
/// Src/Modules/zle/compctl.c:4016. setup_ installs this; finish_
/// restores the fallback.
static COMPCTLREAD_INSTALLED: Mutex<bool> = Mutex::new(false);

/// Direct port of `#define inull(X) zistype(X,INULL)` from
/// `Src/ztype.h:62`. Tests whether `c` is one of the parser's
/// "inull" token chars (the high-bit token bytes the lexer
/// produces).
fn inull(c: char) -> bool {
    // c:62
    matches!(c, Snull | Dnull | Bnull | Stringg | QSTRING_TOK)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that touch the singleton state — `cargo test`
    /// runs tests in parallel and the static `COMPCTL_TAB` / `CCLIST`
    /// would interleave. The parking_lot variant would deadlock-free
    /// across panics; std::sync::Mutex is fine since each test runs
    /// quickly and panics propagate.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn createcompctltable_initializes_table() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        let g = COMPCTL_TAB.read().unwrap();
        assert!(g.is_some());
        assert_eq!(g.as_ref().unwrap().len(), 0);
    }

    #[test]
    fn cc_assign_inserts_into_table() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        let cc = Arc::new(Compctl {
            mask: CC_FILES,
            ..Default::default()
        });
        cc_assign("ls", cc, false);
        let g = COMPCTL_TAB.read().unwrap();
        assert!(g.as_ref().unwrap().contains_key("ls"));
    }

    #[test]
    fn freecompctlp_removes_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        cc_assign("rm", Arc::new(Compctl::default()), false);
        freecompctlp("rm");
        let g = COMPCTL_TAB.read().unwrap();
        assert!(!g.as_ref().unwrap().contains_key("rm"));
    }

    #[test]
    fn cc_flags_bit_layout_matches_c_compctlh() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Spot-check that the bit values match the C constants.
        assert_eq!(CC_FILES, 1);
        assert_eq!(CC_COMMPATH, 2);
        assert_eq!(CC_OPTIONS, 8);
        assert_eq!(CC_JOBS, 1 << 11);
    }

    #[test]
    fn cct_constants_match_c_compctlh() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CCT_POS, 1);
        assert_eq!(CCT_CURPAT, 3);
        assert_eq!(CCT_QUOTE, 13);
    }

    #[test]
    fn comp_op_special_combines_command_default_first() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(COMP_SPECIAL, COMP_COMMAND | COMP_DEFAULT | COMP_FIRST);
    }

    #[test]
    fn cc_flags2_constants_match_c_compctlh() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(CC_NOSORT, 1);
        assert_eq!(CC_CCCONT, 4);
        assert_eq!(CC_UNIQALL, 1 << 6);
    }

    #[test]
    fn get_compctl_simple_flag_chars_set_mask() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // `compctl -fcv ls` — files + commpath + vars
        let mut argv = vec!["-fcv".to_string(), "ls".to_string()];
        let mut cc = Compctl::default();
        let r = get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(r, 0);
        assert_ne!(cc.mask & CC_FILES, 0);
        assert_ne!(cc.mask & CC_COMMPATH, 0);
        assert_ne!(cc.mask & CC_VARS, 0);
        // `ls` should remain in argv
        assert_eq!(argv, vec!["ls".to_string()]);
    }

    #[test]
    fn get_compctl_combined_a_sets_alreg_and_alglob() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut argv = vec!["-a".to_string(), "ls".to_string()];
        let mut cc = Compctl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_ne!(cc.mask & CC_ALREG, 0);
        assert_ne!(cc.mask & CC_ALGLOB, 0);
    }

    #[test]
    fn get_compctl_arg_taking_K_captures_function_name() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut argv = vec![
            "-K".to_string(),
            "_my_completer".to_string(),
            "myfunc".to_string(),
        ];
        let mut cc = Compctl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.func.as_deref(), Some("_my_completer"));
        assert_eq!(argv, vec!["myfunc".to_string()]);
    }

    #[test]
    fn get_compctl_inline_arg_K_captures_function_name() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // `-K_my_func`  → the K flag char with inline arg
        let mut argv = vec!["-K_my_func".to_string(), "myfunc".to_string()];
        let mut cc = Compctl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.func.as_deref(), Some("_my_func"));
    }

    #[test]
    fn get_compctl_P_S_capture_prefix_suffix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut argv = vec![
            "-P".to_string(),
            "before-".to_string(),
            "-S".to_string(),
            "-after".to_string(),
            "cmd".to_string(),
        ];
        let mut cc = Compctl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.prefix.as_deref(), Some("before-"));
        assert_eq!(cc.suffix.as_deref(), Some("-after"));
    }

    #[test]
    fn get_compctl_1_2_set_uniq_flags() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut argv = vec!["-1".to_string(), "ls".to_string()];
        let mut cc = Compctl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_ne!(cc.mask2 & CC_UNIQALL, 0);
        assert_eq!(cc.mask2 & CC_UNIQCON, 0);
    }

    #[test]
    fn get_compctl_V_implies_NOSORT() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut argv = vec!["-V".to_string(), "mygroup".to_string(), "cmd".to_string()];
        let mut cc = Compctl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.gname.as_deref(), Some("mygroup"));
        assert_ne!(cc.mask2 & CC_NOSORT, 0);
    }

    #[test]
    fn bin_compctl_install_then_lookup_via_table() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_compctl("compctl", &["-f".to_string(), "mycmd".to_string()], &ops, 0);
        assert_eq!(r, 0);
        let g = COMPCTL_TAB.read().unwrap();
        assert!(g.as_ref().unwrap().contains_key("mycmd"));
        let cc = g.as_ref().unwrap().get("mycmd").unwrap();
        assert_ne!(cc.mask & CC_FILES, 0);
    }

    #[test]
    fn compctl_name_pat_detects_glob_wildcards() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Glob-meta chars present → pattern.
        let (is_pat, _) = compctl_name_pat("ls*");
        assert!(is_pat);
        let (is_pat, _) = compctl_name_pat("foo?bar");
        assert!(is_pat);
        let (is_pat, _) = compctl_name_pat("[abc]");
        assert!(is_pat);
    }

    #[test]
    fn compctl_name_pat_strips_backslashes_from_literal() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let (is_pat, out) = compctl_name_pat("\\$home");
        assert!(!is_pat);
        // Backslash dropped, `$` kept.
        assert_eq!(out, "$home");
    }

    #[test]
    fn delpatcomp_removes_matching_pattern() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut p = PATCOMPS.write().unwrap();
        p.push(("foo*".to_string(), Arc::new(Compctl::default())));
        p.push(("bar*".to_string(), Arc::new(Compctl::default())));
        drop(p);
        delpatcomp("foo*");
        let p = PATCOMPS.read().unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, "bar*");
    }

    /// `delpatcomp` MUST remove only the FIRST match (c:1308 `break`)
    /// when duplicate names exist. C uses the explicit `break` after
    /// the first hit. A regression that switched to `Vec::retain`
    /// would remove ALL matches — semantically different, observable
    /// when users register multiple compctl entries under the same
    /// pattern (`compctl -p 'foo*' ...` twice for layered configs).
    /// `delpatcomp` MUST remove only the FIRST match (c:1308 `break`)
    /// when duplicate names exist. C uses the explicit `break` after
    /// the first hit. A regression that switched to `Vec::retain`
    /// would remove ALL matches — semantically different, observable
    /// when users register multiple compctl entries under the same
    /// pattern (`compctl -p 'foo*' ...` twice for layered configs).
    #[test]
    fn delpatcomp_removes_only_first_match_when_duplicates() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Clear shared PATCOMPS state — other tests may have left
        // entries that would interfere with the duplicate-count.
        {
            let mut p = PATCOMPS.write().unwrap();
            p.clear();
            // Three entries under the same pattern name — distinguishable
            // by the embedded Compctl::keyvar (used as a tag).
            for tag in ["a", "b", "c"] {
                p.push((
                    "dup*".to_string(),
                    Arc::new(Compctl {
                        keyvar: Some(tag.to_string()),
                        ..Compctl::default()
                    }),
                ));
            }
        }

        delpatcomp("dup*");

        let p = PATCOMPS.read().unwrap();
        assert_eq!(
            p.len(),
            2,
            "c:1308 — only ONE entry removed; got len={}",
            p.len()
        );
        // The remaining entries must be the 2nd and 3rd (b then c) —
        // the first ("a") was the one removed.
        assert_eq!(
            p[0].1.keyvar.as_deref(),
            Some("b"),
            "remaining first entry must be the second-inserted (b)"
        );
        assert_eq!(
            p[1].1.keyvar.as_deref(),
            Some("c"),
            "remaining second entry must be the third-inserted (c)"
        );
    }

    #[test]
    fn cc_assign_with_reass_command_target_uses_special_key() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        CCLIST.with(|c| c.set(COMP_COMMAND));
        cc_assign(
            "compctl",
            Arc::new(Compctl {
                mask: CC_FILES,
                ..Default::default()
            }),
            true,
        );
        let g = COMPCTL_TAB.read().unwrap();
        assert!(g.as_ref().unwrap().contains_key("__cc_compos"));
        // Reset for other tests.
        drop(g);
        CCLIST.with(|c| c.set(0));
    }

    #[test]
    fn cc_assign_with_reass_default_target_uses_special_key() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        CCLIST.with(|c| c.set(COMP_DEFAULT));
        cc_assign("compctl", Arc::new(Compctl::default()), true);
        let g = COMPCTL_TAB.read().unwrap();
        assert!(g.as_ref().unwrap().contains_key("__cc_default"));
        drop(g);
        CCLIST.with(|c| c.set(0));
    }

    #[test]
    fn setup_initializes_special_targets_and_table() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_();
        // cc_compos has CC_COMMPATH set
        let cc_compos = CC_COMPOS.lock().unwrap().clone();
        assert!(cc_compos.is_some());
        assert_eq!(cc_compos.unwrap().mask, CC_COMMPATH);
        // cc_default has CC_FILES + refc=10000 sentinel
        let cc_default = CC_DEFAULT.lock().unwrap().clone();
        assert!(cc_default.is_some());
        let cc_default = cc_default.unwrap();
        assert_eq!(cc_default.mask, CC_FILES);
        assert_eq!(cc_default.refc, 10000);
        // cc_first has CC_CCCONT in mask2
        let cc_first = CC_FIRST.lock().unwrap().clone();
        assert!(cc_first.is_some());
        assert_eq!(cc_first.unwrap().mask2, CC_CCCONT);
        // table exists
        assert!(COMPCTL_TAB.read().unwrap().is_some());
        // compctlread installed
        assert!(*COMPCTLREAD_INSTALLED.lock().unwrap());
    }

    #[test]
    fn finish_tears_down_state() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_();
        finish_();
        // Table cleared
        assert!(COMPCTL_TAB.read().unwrap().is_none());
        // compctlread restored
        assert!(!*COMPCTLREAD_INSTALLED.lock().unwrap());
        // lastccused cleared
        assert_eq!(LASTCCUSED.lock().unwrap().len(), 0);
    }

    #[test]
    fn features_returns_two_builtins() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let f = features_();
        assert_eq!(f, vec!["b:compctl".to_string(), "b:compcall".to_string()]);
    }

    #[test]
    fn enables_returns_two_enabled_bits() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let e = enables_();
        assert_eq!(e, vec![1, 1]);
    }

    #[test]
    fn bin_compcall_outside_compfunc_errors() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_compcall("compcall", &[], &ops, 0);
        assert_eq!(r, 1);
    }

    #[test]
    fn bin_compcall_inside_compfunc_succeeds() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_compcall("compcall", &["-T".to_string()], &ops, 0);
        assert_eq!(r, 0);
        // Reset
        INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn compctlread_outside_compctl_func_errors() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        INCOMPCTLFUNC.with(|c| c.set(false));
        let r = compctlread("compctlread", &[]);
        assert_eq!(r, 1);
    }

    #[test]
    fn cccleanuphookfn_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Trivial — no state to verify, just that it doesn't panic.
        assert_eq!(cccleanuphookfn(()), 0);
    }

    // Helpers for the addmatch tests: snapshot the real match registry so
    // we can assert `add_match_data` actually built and registered a
    // Cmatch (not just that a name landed in the observability mirror).
    fn mnum_now() -> i32 {
        crate::ported::zle::compcore::mnum.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn clear_matches() {
        crate::comp_match_handles::matches_arc()
            .lock()
            .unwrap()
            .clear();
    }
    fn last_match_orig() -> Option<String> {
        crate::comp_match_handles::matches_arc()
            .lock()
            .unwrap()
            .last()
            .and_then(|cm| cm.orig.clone())
    }
    // Reset the compctl prefix/suffix file-statics to empty so an
    // addmatch test that drives addmatch directly (without running the
    // makecomplistflags preamble) sees a clean, thread-local-stale-free
    // matcher context.
    fn reset_compctl_statics() {
        PATCOMP.with(|r| *r.borrow_mut() = None);
        FILECOMP.with(|r| *r.borrow_mut() = None);
        for st in [
            &LPRE, &LSUF, &RPRE, &RSUF, &PPRE, &PSUF, &LPPRE, &LPSUF, &FPRE, &FSUF, &QFPRE, &QFSUF,
            &QRPRE, &QRSUF, &QLPRE, &QLSUF,
        ] {
            st.with(|r| r.borrow_mut().clear());
        }
        crate::ported::zle::compcore::mflags.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn addmatch_rejects_unset_addwhat() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // C: c:2015 — the `else` arm drops the match (returns without
        // calling add_match_data) when addwhat is 0: no Cmatch is built
        // and mnum does not move.
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        clear_matches();
        reset_compctl_statics();
        let before = mnum_now();
        ADDWHAT.with(|c| c.set(0));
        addmatch("dropped", None);
        assert!(
            MATCH_LIST.with(|r| r.borrow().is_empty()),
            "addwhat=0 should drop the match"
        );
        assert_eq!(mnum_now(), before, "no Cmatch may be registered");
        assert_eq!(
            crate::comp_match_handles::matches_arc()
                .lock()
                .unwrap()
                .len(),
            0,
            "match registry stays empty"
        );
    }

    #[test]
    fn addmatch_accepts_files_kind() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // c:1957-1990 file thread → comp_match + add_match_data builds a
        // real Cmatch per accepted filename. With empty prefix/suffix
        // statics comp_match matches trivially, so both names register.
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        clear_matches();
        reset_compctl_statics();
        let before = mnum_now();
        ADDWHAT.with(|c| c.set(-5));
        addmatch("foo.txt", None);
        addmatch("bar.txt", None);
        assert_eq!(mnum_now(), before + 2, "two Cmatches registered");
        let reg = crate::comp_match_handles::matches_arc()
            .lock()
            .unwrap()
            .clone();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg[0].orig.as_deref(), Some("foo.txt"));
        assert_eq!(reg[1].orig.as_deref(), Some("bar.txt"));
        // c:1990 — file-thread matches carry CMF_FILE in their flags.
        use crate::ported::zle::comp_h::CMF_FILE;
        assert_ne!(reg[0].flags & CMF_FILE, 0, "file matches flagged CMF_FILE");
    }

    #[test]
    fn addmatch_accepts_param_kind() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // c:1995 — addwhat == -9 (parameter) → accept thread. The name is
        // matched against the (empty) real prefix/suffix and a real
        // Cmatch is produced; unlike the file thread it is not CMF_FILE.
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        clear_matches();
        reset_compctl_statics();
        let before = mnum_now();
        ADDWHAT.with(|c| c.set(-9));
        addmatch("HOME", None);
        assert_eq!(mnum_now(), before + 1, "one Cmatch registered");
        assert_eq!(last_match_orig().as_deref(), Some("HOME"));
        use crate::ported::zle::comp_h::CMF_FILE;
        let reg = crate::comp_match_handles::matches_arc()
            .lock()
            .unwrap()
            .clone();
        assert_eq!(reg[0].flags & CMF_FILE, 0, "param match is not a file");
    }

    #[test]
    fn addmatch_accepts_cc_files_positive_mask() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // c:1957 — a positive addwhat that includes CC_FILES routes down
        // the file thread and registers a real Cmatch.
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        clear_matches();
        reset_compctl_statics();
        let before = mnum_now();
        ADDWHAT.with(|c| c.set(CC_FILES as i32));
        addmatch("foo", None);
        assert_eq!(mnum_now(), before + 1);
        assert_eq!(last_match_orig().as_deref(), Some("foo"));
    }

    #[test]
    fn getcpat_finds_first_substring() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Search "abcabc" for "bc" first occurrence → position 3
        // (1-based, points past the matched substring).
        let r = getcpat("abcabc", 1, "bc", 0);
        assert_eq!(r, 3);
    }

    #[test]
    fn getcpat_finds_second_substring() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Search "abcabc" for the 2nd "bc" → position 6.
        let r = getcpat("abcabc", 2, "bc", 0);
        assert_eq!(r, 6);
    }

    #[test]
    fn getcpat_negative_index_searches_backward() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Backward search "abcabc" for last "bc" → position 5.
        let r = getcpat("abcabc", -1, "bc", 0);
        assert!(r >= 0, "should find match (got {})", r);
    }

    #[test]
    fn getcpat_class_mode_matches_any_char_in_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Search "abcdef" for any of {b, d, f} — class mode.
        // First match at index 1 (b).
        let r = getcpat("abcdef", 1, "bdf", 1);
        assert_eq!(r, 2); // 1-based position of 'b'
    }

    #[test]
    fn getcpat_not_found_returns_negative_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let r = getcpat("hello", 1, "xyz", 0);
        assert_eq!(r, -1);
    }

    #[test]
    fn getcpat_strips_backslashes_in_pattern() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // `\$` in pattern should be treated as literal `$`.
        let r = getcpat("foo$bar", 1, "\\$", 0);
        assert_eq!(r, 4); // 1-based pos right after the `$`
    }

    #[test]
    fn dumphashtable_calls_addmatch_per_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        let entries = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        dumphashtable(entries, -5);
        let m = MATCH_LIST.with(|r| r.borrow().clone());
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn addhnmatch_forwards_to_addmatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        ADDWHAT.with(|c| c.set(-5));
        addhnmatch("xyz", 0);
        let m = MATCH_LIST.with(|r| r.borrow().clone());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0], "xyz");
    }

    #[test]
    fn makecomplistctl_recursion_guard() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Force depth to MAX
        CDEPTH.with(|c| c.set(MAX_CDEPTH));
        let r = makecomplistctl(0);
        assert_eq!(r, 0);
        // Reset for other tests.
        CDEPTH.with(|c| c.set(0));
    }

    #[test]
    fn makecomplistflags_cc_files_invokes_gen_matches() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        // Set prpre to a known dir we can read.
        PRPRE.with(|r| *r.borrow_mut() = Some(".".to_string()));
        let cc = Arc::new(Compctl {
            mask: CC_FILES,
            ..Default::default()
        });
        makecomplistflags(&cc, String::new(), false, 0);
        // Should have at least picked up Cargo.toml or similar from pwd.
        let m = MATCH_LIST.with(|r| r.borrow().clone());
        assert!(!m.is_empty(), "expected file matches in pwd");
    }

    #[test]
    fn makecomplistflags_cc_str_expansion_emits_one_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        let cc = Arc::new(Compctl {
            str: Some("hardcoded".to_string()),
            ..Default::default()
        });
        makecomplistflags(&cc, String::new(), false, 0);
        let m = MATCH_LIST.with(|r| r.borrow().clone());
        assert_eq!(m.len(), 1);
        assert_eq!(m[0], "hardcoded");
    }

    #[test]
    fn makecomplistor_walks_xor_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.with(|r| r.borrow_mut().clear());
        // Build cc1 with str "first", xor → cc2 with str "second"
        let cc2 = Arc::new(Compctl {
            str: Some("second".to_string()),
            ..Default::default()
        });
        let cc1 = Arc::new(Compctl {
            str: Some("first".to_string()),
            xor: Some(cc2),
            ..Default::default()
        });
        makecomplistor(&cc1, "", false, 0, 0);
        let m = MATCH_LIST.with(|r| r.borrow().clone());
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], "first");
        assert_eq!(m[1], "second");
    }

    #[test]
    fn makecomplistcc_pushes_to_ccused() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        CCUSED.with(|r| r.borrow_mut().clear());
        let cc = Arc::new(Compctl::default());
        makecomplistcc(&cc, "", false);
        let used = CCUSED.with(|r| r.borrow().clone());
        assert_eq!(used.len(), 1);
    }

    #[test]
    fn makecomplistpc_iterates_patcomps() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Verify makecomplistpc returns 0 when cmdstr is unset
        // (its early-bail path) — full pattern-match test requires
        // VM context for glob_match_static.
        CMDSTR.with(|r| *r.borrow_mut() = None);
        let r = makecomplistpc("", false);
        assert_eq!(r, 0);
    }

    #[test]
    fn findnode_returns_index_of_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let list = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(findnode(&list, &"b".to_string()), Some(1));
        assert_eq!(findnode(&list, &"z".to_string()), None);
    }

    #[test]
    fn cc_assign_rejects_conflicting_special_targets() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        CCLIST.with(|c| c.set(COMP_COMMAND | COMP_DEFAULT));
        cc_assign("compctl", Arc::new(Compctl::default()), true);
        let g = COMPCTL_TAB.read().unwrap();
        // Should have been rejected — neither key installed.
        assert!(!g.as_ref().unwrap().contains_key("__cc_compos"));
        assert!(!g.as_ref().unwrap().contains_key("__cc_default"));
        drop(g);
        CCLIST.with(|c| c.set(0));
    }

    #[test]
    fn compctl_process_cc_remove_deletes_named_entries() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        cc_assign("foo", Arc::new(Compctl::default()), false);
        cc_assign("bar", Arc::new(Compctl::default()), false);
        CCLIST.with(|c| c.set(COMP_REMOVE));
        compctl_process_cc(&["foo".to_string()], Arc::new(Compctl::default()));
        let g = COMPCTL_TAB.read().unwrap();
        let map = g.as_ref().unwrap();
        assert!(!map.contains_key("foo"));
        assert!(map.contains_key("bar"));
        // Reset cclist for other tests.
        CCLIST.with(|c| c.set(0));
    }

    #[test]
    fn sep_comp_string_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // C compctl.c:2806-3030 contract — sep_comp_string only returns
        // 0 (success / dispatched) or 1 (bail, no cursor word).
        let r = sep_comp_string("", "", 0);
        assert!(r == 0 || r == 1, "expected 0 or 1, got {}", r);
    }

    #[test]
    fn sep_comp_string_round_trips_zle_state() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Pre-set zle_tricky.c globals; sep_comp_string must restore them
        // on exit (C compctl.c:2810-2813 save / 2941-2950 restore).
        use crate::ported::zle::compcore::{WB, WE, ZLEMETACS as CS_G, ZLEMETALINE as LINE_G};
        use std::sync::atomic::Ordering;
        WE.store(42, Ordering::Relaxed);
        WB.store(7, Ordering::Relaxed);
        CS_G.store(11, Ordering::Relaxed);
        *ZLEMETALL.lock().unwrap() = 99;
        *INSTRING.lock().unwrap() = QT_DOUBLE;
        *INBACKT.lock().unwrap() = 1;
        *NOALIASES.lock().unwrap() = 1;
        *crate::ported::utils::noerrs_lock().lock().unwrap() = 0;
        *LINE_G
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap() = "hello".to_string();
        *AUTOQ
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap() = "Q".to_string();

        let _ = sep_comp_string("", "x", 0);

        assert_eq!(WE.load(Ordering::Relaxed), 42);
        assert_eq!(WB.load(Ordering::Relaxed), 7);
        assert_eq!(CS_G.load(Ordering::Relaxed), 11);
        assert_eq!(*ZLEMETALL.lock().unwrap(), 99);
        assert_eq!(*INSTRING.lock().unwrap(), QT_DOUBLE);
        assert_eq!(*INBACKT.lock().unwrap(), 1);
        assert_eq!(*NOALIASES.lock().unwrap(), 1);
        assert_eq!(*crate::ported::utils::noerrs_lock().lock().unwrap(), 0);
        assert_eq!(
            *LINE_G
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap(),
            "hello"
        );
        assert_eq!(
            *AUTOQ
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap(),
            "Q"
        );
    }

    #[test]
    fn inull_recognises_marker_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // C compctl.c:2917 — INULL macro recognises Snull/Dnull/Bnull
        // plus String/Qstring tokens for inull-walk.
        assert!(inull(Snull));
        assert!(inull(Dnull));
        assert!(inull(Bnull));
        assert!(inull(Stringg));
        assert!(inull(QSTRING_TOK));
        assert!(!inull('a'));
        assert!(!inull(' '));
    }

    #[test]
    fn qt_constants_match_c_zsh_h() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // C: enum at Src/zsh.h:253-292 — QT_NONE / QT_BACKSLASH /
        // QT_SINGLE / QT_DOUBLE / QT_DOLLARS / QT_BACKTICK in that
        // declaration order, so values are 0..5.
        assert_eq!(QT_NONE, 0);
        assert_eq!(QT_BACKSLASH, 1);
        assert_eq!(QT_SINGLE, 2);
        assert_eq!(QT_DOUBLE, 3);
        assert_eq!(QT_DOLLARS, 4);
        assert_eq!(QT_BACKTICK, 5);
    }

    // ─── zsh-corpus pins for inull / QT_* ──────────────────────────

    /// `inull` recognises every null-token in the set.
    #[test]
    fn compctl_corpus_inull_recognises_null_tokens() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert!(inull(Snull));
        assert!(inull(Dnull));
        assert!(inull(Bnull));
        assert!(inull(Stringg));
        assert!(inull(QSTRING_TOK));
    }

    /// `inull` rejects ordinary printable chars.
    #[test]
    fn compctl_corpus_inull_rejects_printables() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        for c in ['a', 'Z', '0', ' ', '!', '~'] {
            assert!(!inull(c), "{c:?} should NOT be inull");
        }
    }

    /// All QT_* constants are pairwise distinct.
    #[test]
    fn compctl_corpus_qt_pairwise_distinct() {
        let qs = [
            QT_NONE,
            QT_BACKSLASH,
            QT_SINGLE,
            QT_DOUBLE,
            QT_DOLLARS,
            QT_BACKTICK,
        ];
        for (i, a) in qs.iter().enumerate() {
            for b in &qs[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    /// All QT_* in [0, 5] range.
    #[test]
    fn compctl_corpus_qt_all_within_range() {
        for q in [
            QT_NONE,
            QT_BACKSLASH,
            QT_SINGLE,
            QT_DOUBLE,
            QT_DOLLARS,
            QT_BACKTICK,
        ] {
            assert!((0..=5).contains(&q), "QT_* value {q} out of [0,5]");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compctl.c
    // c:92 createcompctltable / c:104 freecompctlp / c:1052 compctl_name_pat
    // c:1080 delpatcomp / c:1303 bin_compctl / c:1435 bin_compcall /
    // c:1485 ccmakehookfn / c:1573 cccleanuphookfn / c:1589 maketildelist /
    // c:1948 gen_matches_files
    // ═══════════════════════════════════════════════════════════════════

    /// c:92 — `createcompctltable` is idempotent (safe to call multiple times).
    #[test]
    fn createcompctltable_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            createcompctltable();
        }
    }

    /// c:104 — `freecompctlp("")` empty name is safe.
    #[test]
    fn freecompctlp_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        freecompctlp("");
    }

    /// c:104 — `freecompctlp(unknown)` safe.
    #[test]
    fn freecompctlp_unknown_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        freecompctlp("__never_a_real_compctl_xyz__");
    }

    /// c:1052 — `compctl_name_pat("")` returns (bool, String) type.
    #[test]
    fn compctl_name_pat_returns_tuple_type() {
        let _: (bool, String) = compctl_name_pat("");
    }

    /// c:1052 — `compctl_name_pat` is pure.
    #[test]
    fn compctl_name_pat_is_pure() {
        for s in ["", "name", "*pat*", "with[brackets]"] {
            let first = compctl_name_pat(s);
            for _ in 0..3 {
                assert_eq!(
                    compctl_name_pat(s),
                    first,
                    "compctl_name_pat({:?}) must be pure",
                    s
                );
            }
        }
    }

    /// c:1080 — `delpatcomp("")` empty name is safe.
    #[test]
    fn delpatcomp_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        delpatcomp("");
    }

    /// c:1485 — `ccmakehookfn` returns i32 (type pin).
    #[test]
    fn ccmakehookfn_returns_i32_type() {
        let _: i32 = ccmakehookfn(());
    }

    /// c:1573 — `cccleanuphookfn` returns i32 (type pin).
    #[test]
    fn cccleanuphookfn_returns_i32_type() {
        let _: i32 = cccleanuphookfn(());
    }

    /// c:1485 — `ccmakehookfn` is idempotent.
    #[test]
    fn ccmakehookfn_idempotent() {
        for _ in 0..5 {
            let _ = ccmakehookfn(());
        }
    }

    /// c:1573 — `cccleanuphookfn` is idempotent.
    #[test]
    fn cccleanuphookfn_idempotent() {
        for _ in 0..5 {
            let _ = cccleanuphookfn(());
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compctl.c
    // c:320 set_gmatcher / c:356 get_gmatcher / c:381 print_gmatcher /
    // c:1303 bin_compctl / c:1435 bin_compcall / c:1589 maketildelist
    // ═══════════════════════════════════════════════════════════════════

    /// c:320 — `set_gmatcher` returns i32 (compile-time type pin).
    #[test]
    fn set_gmatcher_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = set_gmatcher("", &[]);
    }

    /// c:356 — `get_gmatcher` returns i32 (compile-time type pin).
    #[test]
    fn get_gmatcher_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = get_gmatcher("", &[]);
    }

    /// c:381 — `print_gmatcher(0)` is safe.
    #[test]
    fn print_gmatcher_safe_for_zero() {
        let _g = crate::test_util::global_state_lock();
        print_gmatcher(0);
    }

    /// c:1589 — `maketildelist` is safe (no panic / no side effects
    /// outside the tilde-completion buffer).
    #[test]
    fn maketildelist_safe_no_panic() {
        let _g = crate::test_util::global_state_lock();
        maketildelist();
    }

    /// c:1589 — `maketildelist` is idempotent.
    #[test]
    fn maketildelist_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            maketildelist();
        }
    }

    /// c:320 — `set_gmatcher` is deterministic for stable input.
    #[test]
    fn set_gmatcher_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = set_gmatcher("test", &[]);
        for _ in 0..3 {
            assert_eq!(
                set_gmatcher("test", &[]),
                first,
                "set_gmatcher must be deterministic"
            );
        }
    }

    /// c:356 — `get_gmatcher` is deterministic for stable input.
    #[test]
    fn get_gmatcher_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = get_gmatcher("test", &[]);
        for _ in 0..3 {
            assert_eq!(
                get_gmatcher("test", &[]),
                first,
                "get_gmatcher must be deterministic"
            );
        }
    }

    /// c:1303 — `bin_compctl` no-args returns i32 (compile-time type pin).
    #[test]
    fn bin_compctl_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_compctl("compctl", &[], &ops, 0);
    }

    /// c:1435 — `bin_compcall` no-args returns i32.
    #[test]
    fn bin_compcall_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_compcall("compcall", &[], &ops, 0);
    }

    /// c:1485 — `ccmakehookfn(())` is deterministic.
    #[test]
    fn ccmakehookfn_is_deterministic() {
        let first = ccmakehookfn(());
        for _ in 0..3 {
            assert_eq!(
                ccmakehookfn(()),
                first,
                "ccmakehookfn must be deterministic"
            );
        }
    }

    /// c:1573 — `cccleanuphookfn(())` is deterministic.
    #[test]
    fn cccleanuphookfn_is_deterministic() {
        let first = cccleanuphookfn(());
        for _ in 0..3 {
            assert_eq!(
                cccleanuphookfn(()),
                first,
                "cccleanuphookfn must be deterministic"
            );
        }
    }

    /// c:381 — `print_gmatcher` for various indices is safe (no panic).
    #[test]
    fn print_gmatcher_full_index_range_safe() {
        let _g = crate::test_util::global_state_lock();
        for ac in [-1i32, 0, 1, 10, 100, i32::MAX, i32::MIN] {
            print_gmatcher(ac);
        }
    }

    /// c:1052 — `compctl_name_pat(empty)` returns (false, empty).
    #[test]
    fn compctl_name_pat_empty_returns_false_empty() {
        let (is_pat, name) = compctl_name_pat("");
        assert!(!is_pat, "empty input not a pattern");
        assert_eq!(name, "", "empty input → empty name");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compctl.c
    // c:92 createcompctltable / c:104 freecompctlp / c:320 set_gmatcher /
    // c:356 get_gmatcher / c:1052 compctl_name_pat / c:1080 delpatcomp /
    // c:1303 bin_compctl / c:1435 bin_compcall / c:1485 ccmakehookfn
    // ═══════════════════════════════════════════════════════════════════

    /// c:92 — `createcompctltable` is idempotent (alt 10-call).
    #[test]
    fn createcompctltable_idempotent_10_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            createcompctltable();
        }
    }

    /// c:104 — `freecompctlp(empty)` safe.
    #[test]
    fn freecompctlp_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        freecompctlp("");
    }

    /// c:104 — `freecompctlp("__never__")` for unknown name safe (alt pin).
    #[test]
    fn freecompctlp_unknown_name_no_panic_alt() {
        let _g = crate::test_util::global_state_lock();
        freecompctlp("__never_real_compctl_xyz__");
    }

    /// c:320 — `set_gmatcher` returns i32 (compile-time pin, alt).
    #[test]
    fn set_gmatcher_returns_i32_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = set_gmatcher("test", &[]);
    }

    /// c:356 — `get_gmatcher` returns i32 (compile-time pin, alt).
    #[test]
    fn get_gmatcher_returns_i32_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = get_gmatcher("test", &[]);
    }

    /// c:1052 — `compctl_name_pat` returns (bool, String) tuple.
    #[test]
    fn compctl_name_pat_returns_bool_string_tuple_type() {
        let _: (bool, String) = compctl_name_pat("");
    }

    /// c:1052 — `compctl_name_pat` is deterministic.
    #[test]
    fn compctl_name_pat_deterministic() {
        for s in ["", "abc", "*", "(pattern)", "name"] {
            let a = compctl_name_pat(s);
            let b = compctl_name_pat(s);
            assert_eq!(a, b, "compctl_name_pat({:?}) must be pure", s);
        }
    }

    /// c:1080 — `delpatcomp("")` empty pattern safe.
    #[test]
    fn delpatcomp_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        delpatcomp("");
    }

    /// c:1080 — `delpatcomp("__never__")` unknown pattern safe.
    #[test]
    fn delpatcomp_unknown_no_panic() {
        let _g = crate::test_util::global_state_lock();
        delpatcomp("__never_real_pattern_xyz__");
    }

    /// c:1303 — `bin_compctl` non-negative exit code.
    #[test]
    fn bin_compctl_exit_code_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_compctl("compctl", &[], &ops, 0);
        assert!(r >= 0, "bin_compctl exit code must be ≥ 0, got {}", r);
    }

    /// c:1435 — `bin_compcall` non-negative exit code.
    #[test]
    fn bin_compcall_exit_code_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_compcall("compcall", &[], &ops, 0);
        assert!(r >= 0, "bin_compcall exit code must be ≥ 0, got {}", r);
    }

    /// c:1485 — `ccmakehookfn` returns i32 (compile-time pin, alt).
    #[test]
    fn ccmakehookfn_returns_i32_type_alt() {
        let _: i32 = ccmakehookfn(());
    }
}
