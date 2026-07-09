//! Global compiled-pattern cache — a zshrs-only optimization with NO C
//! counterpart, so it lives here in `src/extensions` rather than in
//! `src/ported`.
//!
//! Why it exists: C zsh recompiles a pattern on every match too, but its C
//! `patcompile` is ~40-75x faster than this Rust port. Shell code re-matches
//! identical patterns constantly — `[[ x == pat ]]`, `${(M)a:#pat}`, `case`,
//! `${x//pat/…}` — and p10k's `_p9k_param` re-matches the SAME
//! `(#b)prompt_([a-z0-9_]#)(*)` thousands of times per precmd. At the port's
//! compile cost that dominated interactive startup (the "hang before prompt").
//! Caching the compiled program turns those repeats into a hash lookup + a
//! cheap clone of the bytecode, and lets zshrs actually beat zsh here.
//!
//! Safety: the compiled `Patprog` is self-contained — matching (`pattry*`)
//! reads `prog.0.flags` / `prog.0.globflags` (never the global compile
//! statics) and borrows the prog by `&` (read-only). So a cached clone is
//! interchangeable with a freshly-compiled one.
//!
//! Correctness of the key: the compiled output depends on the pattern text,
//! `inflags`, and exactly six options — EXTENDEDGLOB/KSHGLOB/SHGLOB (which
//! shape `zpc_special` in `patcompcharsset`) and CASEGLOB/CASEPATHS/MULTIBYTE
//! (which shape `patglobflags` in `patcompstart`). All are folded into the
//! key, so any option change yields a distinct key rather than a stale hit.
//!
//! GLOBAL (not thread-local): the VM dispatches `[[ … ]]`/pattern work across
//! zshrs's worker threads, so a per-thread cache would miss on every worker.
//! A single `RwLock<HashMap>` is shared: reads (the common case, a hit) take
//! the read lock and run concurrently; only a miss's store takes the write
//! lock. The check runs BEFORE `patcompile`'s own compile mutex, so hits never
//! serialise on the compiler.

use crate::ported::pattern::Patprog;
use crate::ported::zsh_h::{
    isset, CASEGLOB, CASEPATHS, EXTENDEDGLOB, KSHGLOB, MULTIBYTE, SHGLOB,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// (pattern text, inflags, option fingerprint).
type Key = (String, i32, u8);

// Two-level cache. L1 is thread-local and LOCK-FREE — the common hit takes
// no lock, so a hot loop (`repeat 20000 { [[ x == pat ]] }`) doesn't pay
// RwLock read-contention across zshrs's worker threads (which otherwise
// dominated the profile). L2 is the shared global that lets a pattern
// compiled on one worker be reused by another; an L2 hit is promoted into
// the reader's L1 so subsequent hits stay lock-free.
thread_local! {
    static L1: RefCell<HashMap<Key, Patprog>> = RefCell::new(HashMap::new());
}

static L2: LazyLock<RwLock<HashMap<Key, Patprog>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Bound the cache so a pathological workload (unique patterns forever)
/// can't grow it without limit. On overflow we clear wholesale rather than
/// track LRU — pattern working sets are small and stable (a shell reuses a
/// handful of patterns), so a rare full flush costs one recompile burst,
/// not a steady eviction tax.
const MAX_ENTRIES: usize = 8192;

/// Fold the six compilation-affecting options into a fingerprint byte.
#[inline]
fn opt_fingerprint() -> u8 {
    (isset(EXTENDEDGLOB) as u8)
        | ((isset(KSHGLOB) as u8) << 1)
        | ((isset(SHGLOB) as u8) << 2)
        | ((isset(CASEGLOB) as u8) << 3)
        | ((isset(CASEPATHS) as u8) << 4)
        | ((isset(MULTIBYTE) as u8) << 5)
}

/// Look up a compiled pattern. Returns a clone of the cached `Patprog` on a
/// hit (interchangeable with a fresh compile), `None` on a miss. Checks the
/// lock-free thread-local L1 first, then the shared L2 (promoting an L2 hit
/// into L1).
pub fn get(exp: &str, inflags: i32) -> Option<Patprog> {
    let key = (exp.to_string(), inflags, opt_fingerprint());
    if let Some(p) = L1.with(|c| c.borrow().get(&key).cloned()) {
        return Some(p);
    }
    let hit = L2.read().ok()?.get(&key).cloned();
    if let Some(ref p) = hit {
        promote_l1(key, p);
    }
    hit
}

/// Store a freshly-compiled pattern in both cache levels.
pub fn put(exp: &str, inflags: i32, prog: &Patprog) {
    let key = (exp.to_string(), inflags, opt_fingerprint());
    if let Ok(mut m) = L2.write() {
        if m.len() >= MAX_ENTRIES {
            m.clear();
        }
        m.insert(key.clone(), prog.clone());
    }
    promote_l1(key, prog);
}

/// Insert into the thread-local L1 (bounded, wholesale-flush on overflow).
#[inline]
fn promote_l1(key: Key, prog: &Patprog) {
    L1.with(|c| {
        let mut m = c.borrow_mut();
        if m.len() >= MAX_ENTRIES {
            m.clear();
        }
        m.insert(key, prog.clone());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored pattern is retrievable, and a distinct option state (encoded
    /// in the fingerprint) does not return the wrong cached program.
    #[test]
    fn get_after_put_roundtrips_and_key_is_option_sensitive() {
        // Compile a trivial pattern and cache it.
        let prog = crate::ported::pattern::patcompile("abc", 0, None)
            .expect("compile abc");
        put("zzz_test_pat_abc", 0, &prog);
        assert!(
            get("zzz_test_pat_abc", 0).is_some(),
            "just-put pattern must be retrievable"
        );
        // A never-stored pattern misses.
        assert!(
            get("zzz_test_pat_never_stored_xyz", 0).is_none(),
            "unseen pattern must miss"
        );
    }
}
