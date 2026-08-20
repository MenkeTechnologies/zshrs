//! Shared-handle accessors for the completion match accumulators.
//!
//! NOT a port of any C function — this is Rust-original glue that makes
//! explicit the *pointer aliasing* zsh's C relies on. In C the file-scope
//! `matches`/`fmatches`/`expls`/`allccs` LinkLists ARE the current group's
//! `l*` lists (`begcmgroup` does `matches = mgroup->lmatches`), so a
//! `compadd` append flows into the group with zero copy. The port replaced
//! that alias with owned copies + a manual `endcmgroup` flush, which is the
//! whole "copy-vs-alias" divergence class (nmatches reading 0, the message
//! flush gap, mgroup/amatches divergence).
//!
//! This module restores the alias: each `compcore::{matches,…}` global is a
//! rebindable HANDLE (`Mutex<Arc<Mutex<Vec<…>>>>`) whose inner `Arc` points
//! at the current group's `l*` field. `begcmgroup` rebinds via
//! [`rebind_current`]; every reader takes the `Arc` through the accessors
//! below. Lives OUTSIDE `src/ported/` deliberately: (a) it is not a C-fn
//! port, so it does not belong under the port tree (per the port-gate rule),
//! and (b) centralising the "lock handle → clone Arc → DROP handle guard →
//! return Arc" sequence guarantees the handle mutex is never held while the
//! inner `Vec` mutex is locked — the only way a completer body could deadlock
//! against `begcmgroup` (std `Mutex` is not reentrant).

use crate::ported::zle::comp_h::{Aminfo, Cexpl, Cmatch, Cmatcher, Cmgroup};
use crate::ported::zle::compcore::{allccs, expls, fmatches, matches};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

/// The current group's `lmatches` accumulator (the file-scope `matches`
/// alias). The handle guard is dropped before returning, so callers only
/// ever hold the inner `Vec` lock.
pub fn matches_arc() -> Arc<Mutex<Vec<Cmatch>>> {
    Arc::clone(&matches.get_or_init(default_match_handle).lock().unwrap())
}

/// The current group's `lfmatches` accumulator (file-scope `fmatches` alias).
pub fn fmatches_arc() -> Arc<Mutex<Vec<Cmatch>>> {
    Arc::clone(&fmatches.get_or_init(default_match_handle).lock().unwrap())
}

/// The current group's `lexpls` accumulator (file-scope `expls` alias).
pub fn expls_arc() -> Arc<Mutex<Vec<Cexpl>>> {
    Arc::clone(&expls.get_or_init(default_expl_handle).lock().unwrap())
}

/// The current group's `lallccs` accumulator (file-scope `allccs` alias).
pub fn allccs_arc() -> Arc<Mutex<Vec<String>>> {
    Arc::clone(&allccs.get_or_init(default_ccs_handle).lock().unwrap())
}

/// Point all four file-scope handles at the group `l*` fields — the Rust
/// equivalent of C's `begcmgroup` alias assignment
/// (`matches = mgroup->lmatches; …`). Called by `begcmgroup` for both the
/// reuse and the fresh-group paths. Each store briefly locks a handle mutex
/// and drops it; no inner `Vec` lock is taken here.
#[allow(clippy::type_complexity)]
pub fn rebind_current(
    lmatches: &Arc<Mutex<Vec<Cmatch>>>,
    lfmatches: &Arc<Mutex<Vec<Cmatch>>>,
    lexpls: &Arc<Mutex<Vec<Cexpl>>>,
    lallccs: &Arc<Mutex<Vec<String>>>,
) {
    *matches.get_or_init(default_match_handle).lock().unwrap() = Arc::clone(lmatches);
    *fmatches.get_or_init(default_match_handle).lock().unwrap() = Arc::clone(lfmatches);
    *expls.get_or_init(default_expl_handle).lock().unwrap() = Arc::clone(lexpls);
    *allccs.get_or_init(default_ccs_handle).lock().unwrap() = Arc::clone(lallccs);
}

fn default_match_handle() -> Mutex<Arc<Mutex<Vec<Cmatch>>>> {
    Mutex::new(Arc::new(Mutex::new(Vec::new())))
}
fn default_expl_handle() -> Mutex<Arc<Mutex<Vec<Cexpl>>>> {
    Mutex::new(Arc::new(Mutex::new(Vec::new())))
}
fn default_ccs_handle() -> Mutex<Arc<Mutex<Vec<String>>>> {
    Mutex::new(Arc::new(Mutex::new(Vec::new())))
}

// =====================================================================
// !!! WARNING: RUST-ONLY HELPER !!!
//
// C zsh has no counterpart because C never needs one: `$(…)` goes
// through `getoutput()` (c:Src/exec.c:4782), whose child runs
// `entersubsh(ESUB_PGRP|ESUB_NOMONITOR)` after a `fork()`. Every
// completion global — `matches`, `amatches`, `mgroup`, `ainfo`,
// `mnum`, … (c:Src/Zle/compcore.c:124-259) — is therefore
// copy-on-write private to that child, so a `compadd` executed inside
// a command substitution can never reach the completing shell.
//
// zshrs runs `$(…)` IN-PROCESS (`vm_helper::run_command_substitution`,
// for the ~20x speedup a fork costs with a large shell state), so the
// completion arena is shared with the parent and those matches DO
// reach it. Real divergence this caught: `_tmux`'s description-gen
// loop (`desc="$($f)"` over `${(M)${(k)functions}:#_tmux-*}`) calls
// the *completion* functions `_tmux-backup`, `_tmux-cssh`,
// `_tmux-fingers`, … expecting only their `print`ed description. In
// zsh their `compadd`s die with the forked child; in zshrs they landed
// in the live arena, so `tmux <TAB>` listed 551 matches (five spurious
// groups: `tmux-backup commands`, `tmux-fingers subcommand`, `host`,
// `filename`, `arguments`) against zsh's 450.
//
// So this is the fork's isolation, done by hand — the same technique
// `vm_helper::SubshellSnapshot` already applies to paramtab / opts /
// traps / jobs / aliases for the very same reason.
// =====================================================================

/// The completion-match arena as it stood at command-substitution entry.
///
/// Covers exactly the globals the `compadd` path mutates:
/// `addmatches` / `add_match_data` (c:compcore.c:2277 / c:2470),
/// `begcmgroup` / `endcmgroup` (c:3153 / c:3210) and `addexpl`
/// (c:3244). Nothing else is touched, so a `$(…)` that does not
/// complete restores a bit-identical arena.
pub struct CompArenaSnapshot {
    /// The four rebindable handles (`begcmgroup` repoints them at the new
    /// group's `l*` fields — see [`rebind_current`]) plus the contents of
    /// the `Vec`s they pointed at.
    matches_h: Arc<Mutex<Vec<Cmatch>>>,
    matches_v: Vec<Cmatch>,
    fmatches_h: Arc<Mutex<Vec<Cmatch>>>,
    fmatches_v: Vec<Cmatch>,
    expls_h: Arc<Mutex<Vec<Cexpl>>>,
    expls_v: Vec<Cexpl>,
    allccs_h: Arc<Mutex<Vec<String>>>,
    allccs_v: Vec<String>,
    /// Group lists — c:compcore.c:135.
    amatches: Vec<Cmgroup>,
    pmatches: Vec<Cmgroup>,
    lastmatches: Vec<Cmgroup>,
    lmatches: Option<Cmgroup>,
    lastlmatches: Option<Cmgroup>,
    mgroup: Option<Cmgroup>,
    /// c:compcore.c:246 / :221 / :227.
    ainfo: Option<Aminfo>,
    fainfo: Option<Aminfo>,
    curexpl: Option<Cexpl>,
    matchers: Vec<Box<Cmatcher>>,
    /// The `i32` accumulators listed by [`arena_counters`], in order.
    counters: Vec<i32>,
    /// `complete.c:41 compignored` — an `AtomicI64`, hence separate.
    compignored: i64,
}

/// The scalar completion counters `compadd` maintains, in a fixed order
/// shared by [`comp_arena_save`] and [`comp_arena_restore`].
fn arena_counters() -> [&'static std::sync::atomic::AtomicI32; 24] {
    use crate::ported::zle::compcore as cc;
    [
        &cc::mnum,           // c:202
        &cc::nmatches,       // c:160
        &cc::smatches,       // c:162
        &cc::diffmatches,    // c:167
        &cc::nmessages,      // c:172
        &cc::onlyexpl,       // c:177
        &cc::newmatches,     // c:150
        &cc::hasmatched,     // c:192
        &cc::hasunmatched,   // c:192
        &cc::hasallmatch,    // c:145
        &cc::haspattern,     // c:187
        &cc::ispattern,      // c:187
        &cc::maxmlen,        // c:212
        &cc::minmlen,        // c:212
        &cc::unambig_mnum,   // c:207
        &cc::useexact,       // c:36
        &cc::lenchanged,     // c:54
        &cc::dolastprompt,   // c:44
        &cc::hasoldlist,     // c:140
        &cc::hasperm,        // c:140
        &cc::permmnum,       // c:155
        &cc::permgnum,       // c:155
        &cc::lastpermmnum,   // c:155
        &cc::lastpermgnum,   // c:155
    ]
}

/// Capture the arena. Cheap when no completion is running (the `Vec`s are
/// empty), which is the overwhelmingly common `$(…)`.
pub fn comp_arena_save() -> CompArenaSnapshot {
    use crate::ported::zle::compcore as cc;
    let matches_h = matches_arc();
    let fmatches_h = fmatches_arc();
    let expls_h = expls_arc();
    let allccs_h = allccs_arc();
    let matches_v = matches_h.lock().map(|v| v.clone()).unwrap_or_default();
    let fmatches_v = fmatches_h.lock().map(|v| v.clone()).unwrap_or_default();
    let expls_v = expls_h.lock().map(|v| v.clone()).unwrap_or_default();
    let allccs_v = allccs_h.lock().map(|v| v.clone()).unwrap_or_default();
    CompArenaSnapshot {
        matches_h,
        matches_v,
        fmatches_h,
        fmatches_v,
        expls_h,
        expls_v,
        allccs_h,
        allccs_v,
        amatches: cc::amatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        pmatches: cc::pmatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        lastmatches: cc::lastmatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        lmatches: cc::lmatches
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        lastlmatches: cc::lastlmatches
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        mgroup: cc::mgroup
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        ainfo: cc::ainfo
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        fainfo: cc::fainfo
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        curexpl: cc::curexpl
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        matchers: cc::matchers
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
        counters: arena_counters().iter().map(|a| a.load(Relaxed)).collect(),
        compignored: crate::ported::zle::complete::COMPIGNORED.load(Relaxed),
    }
}

/// Put the arena back exactly as [`comp_arena_save`] found it — the
/// in-process stand-in for the forked child's address space going away.
pub fn comp_arena_restore(snap: CompArenaSnapshot) {
    use crate::ported::zle::compcore as cc;
    // Rebind the handles FIRST (a `begcmgroup` inside the substitution may
    // have repointed them at a group that is about to be dropped), then
    // rewrite the `Vec`s they alias.
    rebind_current(
        &snap.matches_h,
        &snap.fmatches_h,
        &snap.expls_h,
        &snap.allccs_h,
    );
    if let Ok(mut v) = snap.matches_h.lock() {
        *v = snap.matches_v;
    }
    if let Ok(mut v) = snap.fmatches_h.lock() {
        *v = snap.fmatches_v;
    }
    if let Ok(mut v) = snap.expls_h.lock() {
        *v = snap.expls_v;
    }
    if let Ok(mut v) = snap.allccs_h.lock() {
        *v = snap.allccs_v;
    }
    if let Ok(mut g) = cc::amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = snap.amatches;
    }
    if let Ok(mut g) = cc::pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = snap.pmatches;
    }
    if let Ok(mut g) = cc::lastmatches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
    {
        *g = snap.lastmatches;
    }
    if let Ok(mut g) = cc::lmatches.get_or_init(|| Mutex::new(None)).lock() {
        *g = snap.lmatches;
    }
    if let Ok(mut g) = cc::lastlmatches.get_or_init(|| Mutex::new(None)).lock() {
        *g = snap.lastlmatches;
    }
    if let Ok(mut g) = cc::mgroup.get_or_init(|| Mutex::new(None)).lock() {
        *g = snap.mgroup;
    }
    if let Ok(mut g) = cc::ainfo.get_or_init(|| Mutex::new(None)).lock() {
        *g = snap.ainfo;
    }
    if let Ok(mut g) = cc::fainfo.get_or_init(|| Mutex::new(None)).lock() {
        *g = snap.fainfo;
    }
    if let Ok(mut g) = cc::curexpl.get_or_init(|| Mutex::new(None)).lock() {
        *g = snap.curexpl;
    }
    if let Ok(mut g) = cc::matchers.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = snap.matchers;
    }
    for (a, v) in arena_counters().iter().zip(snap.counters.iter()) {
        a.store(*v, Relaxed);
    }
    crate::ported::zle::complete::COMPIGNORED.store(snap.compignored, Relaxed);
}
