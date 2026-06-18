//! Serializes in-process lexer/parser access across the parity binary's
//! parallel test threads.
//!
//! `zsh::parse`/`zsh::lex` mutate PROCESS-GLOBAL state (`chwords`,
//! `chwordpos`, `chline`, the lexer/heredoc buffers, etc.) that is correct for
//! a single shell but not for many parsers running at once. The corpus parity
//! tests (`corpus_parity`, `corpus_wordcode_parity`, `zpwr_real_world_parity`)
//! call the parser in-process on cargo's test thread-pool, so without
//! serialization concurrent parses overflow `chwordpos` (a `raw_vec` capacity
//! panic) and poison the `chwords` mutex.
//!
//! The crate's own `test_util::global_state_lock` is `#[cfg(test)]`-only and
//! thus invisible to this integration binary, so we keep an equivalent lock
//! here. Every in-process parse in this binary must hold it.

use std::sync::{Mutex, MutexGuard, OnceLock};

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Acquire the parity-binary in-process parser lock. Held for the duration of
/// a single `parse_init`/`parse` (or `lex_init`/`par_list_wordcode`) call so
/// the global lexer/history state is touched by one thread at a time.
pub fn parser_guard() -> MutexGuard<'static, ()> {
    match lock().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}
