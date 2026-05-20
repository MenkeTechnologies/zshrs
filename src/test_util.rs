//! Shared test infrastructure for serializing tests that touch
//! crate-level global mutable state (errflag, paramtab, lex
//! buffers, zcontext stacks, paramtab_hashed_storage, ShellExecutor
//! singletons, etc.).
//!
//! Many ported subsystems intentionally mirror C's file-static
//! globals via `OnceLock<Mutex<…>>` / `AtomicI32`. Tests against
//! those subsystems share the same singletons. Parallel cargo
//! test execution races on read-modify-write patterns even when
//! each individual test cleans up afterwards, because two tests
//! observe each other's mid-test state.
//!
//! Pattern: tests that touch global state call
//! `let _g = crate::test_util::global_state_lock();` at entry.
//! The MutexGuard is held for the test's lifetime, serializing
//! against every other test that does the same — purely-functional
//! tests still run in parallel.
//!
//! Mutex-poisoning is recovered automatically so a single panicking
//! test doesn't break the whole suite.

use std::sync::{Mutex, MutexGuard, OnceLock};

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Acquire the crate-wide test mutex. Tests that touch global
/// state (errflag, paramtab, lex buffers, zcontext, ShellExecutor,
/// etc.) hold this guard for their duration to serialize against
/// other stateful tests.
pub fn global_state_lock() -> MutexGuard<'static, ()> {
    let g = match lock().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    // `assignstrvalue` (and downstream `setsparam`/`setiparam`/etc.)
    // bails out at the top with `if unset(EXECOPT) return;`. The
    // option default is OFF in test builds, so without enabling it
    // here every test that calls a param-setter sees a no-op write.
    // Real shells enable EXECOPT at init via `parseargs` (init.c:404).
    crate::ported::options::opt_state_set("exec", true);
    g
}
