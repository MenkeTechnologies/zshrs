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
    // `promptpercent` and `promptbang` are zsh-default-ON (Src/options.c
    // default_opts[] — both enabled in PROMPTPERCENT/PROMPTBANG default
    // state). The Rust port reads them via `isset(PROMPTPERCENT)` /
    // `isset(PROMPTBANG)` in the prompt expander; without this the
    // option table comes up clean and every `%X`/`!`-style expansion
    // is silently disabled.
    crate::ported::options::opt_state_set("promptpercent", true);
    crate::ported::options::opt_state_set("promptbang", true);
    // `inittyptab` (Src/utils.c:4155) populates the global character
    // classification table (`typtab[]`) — IIDENT, IALNUM, IDIGIT, ISEP,
    // IWORD, etc. C calls it once at startup in `setupvals` (init.c) —
    // without it `zistype('i', IIDENT)` returns false and `itype_end`
    // can't parse identifiers, so `fetchvalue("intvar", …)` returns
    // None even when the param exists in paramtab. Mirror the C startup
    // call so any test that touches the param/lex pipelines sees a
    // fully initialised table.
    crate::ported::utils::inittyptab();
    // Clear `errflag` so a previous test that errored doesn't leak its
    // ERRFLAG_ERROR / ERRFLAG_INT bits into this test's lex/parse/math
    // pipeline. `errflag` is a process-wide `AtomicI32` (utils.rs); any
    // test that calls `zerr()` sets it, and the next test sees a
    // non-zero value at every "if errflag != 0 return LEXERR" gate
    // (lex.rs:1064, parse.rs, math.rs, etc.). C zsh resets it at the
    // top of every `loop` iteration in `init.c::zsh_main`; mirror that
    // here so each test starts with a clean error state.
    crate::ported::utils::errflag.store(0, std::sync::atomic::Ordering::Relaxed);
    // Reset options that other tests temporarily flip. The lock
    // serialises but doesn't restore on panic — a test that sets
    // `octalzeroes=true` and panics before its restore leaves the
    // option ON for every subsequent test. Force OFF here so each test
    // starts from a deterministic default. Add other "test-toggled"
    // options here as they surface as cross-test interference.
    crate::ported::options::opt_state_set("octalzeroes", false);
    crate::ported::options::opt_state_set("cbases", false);
    // ksharrays flips array subscript base from 1 (zsh default) to 0
    // (ksh emulation). Other tests temporarily enable it and the lock
    // doesn't roll back on panic. Force OFF so `(( arr[2]=N ))` writes
    // the 2nd (1-indexed) element as zsh expects.
    crate::ported::options::opt_state_set("ksharrays", false);
    // `casematch` defaults ON in real zsh (verified via `/bin/zsh -fc
    // 'echo $options[casematch]'` → "on"). Without it stamped here,
    // `zcond_regex_match` and other case-sensitivity-gated paths see
    // the Rust-default `false` and silently swap to case-insensitive,
    // breaking the `[[ abc =~ ABC ]] → no-match` pin.
    crate::ported::options::opt_state_set("casematch", true);
    // Reset emulation to EMULATE_ZSH so `init_builtins` (called by
    // builtin tests) doesn't disable the `repeat` reswd in reswdtab
    // for every subsequent test that tries to parse `repeat N do …`.
    crate::ported::options::emulation.store(
        crate::ported::zsh_h::EMULATE_ZSH,
        std::sync::atomic::Ordering::Relaxed,
    );
    // Re-enable `repeat` in reswdtab — once a prior test ran
    // init_builtins under non-zsh emulation, the disable persists in
    // the process-wide table. tab.enable is the C `disablenode(hn, 1)`
    // equivalent (Src/hashtable.c::sethashnode_disable_state).
    if let Ok(mut tab) = crate::ported::hashtable::reswdtab_lock().write() {
        tab.enable("repeat");
    }
    g
}
