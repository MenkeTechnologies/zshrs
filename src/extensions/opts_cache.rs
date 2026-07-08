//! Fast-path option-state cache — a Rust-only optimization backing
//! `isset()`.
//!
//! # Why
//!
//! `isset(X)` is the single most-called primitive in a shell. In zsh it is
//! `#define isset(X) (opts[X])` (Src/zsh.h:2557) — one load from a
//! `char opts[OPT_SIZE]` array indexed by the option constant. zshrs stores
//! option state authoritatively in `ported::options::OPTS_LIVE`, a
//! `RwLock<HashMap<String, bool>>`. Routing every `isset` through that map
//! meant, PER CHECK: a constant→name linear match (`opt_name`), a
//! name→constant scan (`optlookup`), an RwLock acquisition, and a String
//! hash. Every parameter expansion checks a fistful of options
//! (`SHWORDSPLIT`, `KSHARRAYS`, `NOMATCH`, `EXTENDED_GLOB`, …), so a real
//! config's startup performs millions of these — and profiling showed the
//! option lookups dominating even a tight function-call loop.
//!
//! This module restores C's representation: a flat array indexed by option
//! number that `isset` reads with a single relaxed atomic load. The
//! `HashMap` remains the source of truth for name-based access (`setopt`,
//! `unsetopt`, `$options`); this cache is a read-through mirror for the hot
//! numeric read only.
//!
//! # Coherence
//!
//! Every option-store mutation funnels through exactly three functions —
//! `opt_state_set`, `opt_state_unset`, `opt_state_restore` (and
//! `opt_state_set_via_alias`, which routes through the first two). Each of
//! those calls [`invalidate_all`] after mutating the map, so the next read
//! of any slot repopulates from the authoritative value. Reads outnumber
//! writes by orders of magnitude, so a full invalidate on write is far
//! cheaper than doing the name lookup on every read.

use std::sync::atomic::{AtomicI8, Ordering};

use crate::ported::zsh_h::{opt_name, OPT_SIZE};

/// Slot values: `-1` = unknown (repopulate lazily), `0` = unset, `1` = set.
/// Indexed by option number, mirroring C's `char opts[OPT_SIZE]`.
static OPTS_CACHE: [AtomicI8; OPT_SIZE as usize] =
    [const { AtomicI8::new(-1) }; OPT_SIZE as usize];

/// Drop the entire cache. Used when a wholesale option-store change makes
/// per-slot tracking impossible (e.g. an unfiltered snapshot restore).
pub fn invalidate_all() {
    for slot in OPTS_CACHE.iter() {
        slot.store(-1, Ordering::Relaxed);
    }
}

/// Invalidate a SINGLE option's cached value by canonical number. Used by
/// the mutators so setting one option (e.g. `doshfunc`'s per-call
/// `printexitvalue` flip) doesn't cold-invalidate every other option's
/// slot — which is what made the cache useless inside function calls.
/// `optno` may be negative (a `no…` alias); the sign is dropped since the
/// alias and its canonical share a slot.
pub fn invalidate_one(optno: i32) {
    let idx = optno.unsigned_abs() as usize;
    if idx < OPT_SIZE as usize {
        OPTS_CACHE[idx].store(-1, Ordering::Relaxed);
    }
}

/// O(1) option read backing `isset(optno)` — mirrors C `opts[optno]`.
///
/// Reads the cache; on a miss, resolves the authoritative value once (the
/// old `opt_state_get(&opt_name(optno))` path) and memoizes it. Preserves
/// the historical `unwrap_or(false)` semantics for unset / out-of-range
/// option numbers (e.g. `OPT_INVALID`).
pub fn is_set(optno: i32) -> bool {
    if optno < 0 || optno as usize >= OPT_SIZE as usize {
        return authoritative(optno);
    }
    let slot = &OPTS_CACHE[optno as usize];
    let cached = slot.load(Ordering::Relaxed);
    if cached >= 0 {
        return cached != 0;
    }
    let v = authoritative(optno);
    slot.store(v as i8, Ordering::Relaxed);
    v
}

/// The pre-cache read: number → canonical name → authoritative map lookup.
#[inline]
fn authoritative(optno: i32) -> bool {
    crate::ported::options::opt_state_get(&opt_name(optno)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::options::{opt_state_set, opt_state_unset, optlookup};

    #[test]
    fn set_then_isset_reflects_change_through_cache() {
        let no = optlookup("ksharrays");
        assert!(no > 0, "ksharrays should resolve to a canonical optno");
        opt_state_set("ksharrays", true);
        assert!(is_set(no), "cache must observe the set");
        opt_state_unset("ksharrays");
        assert!(!is_set(no), "cache must observe the unset");
    }

    #[test]
    fn invalidate_all_is_idempotent() {
        invalidate_all();
        invalidate_all();
    }
}
