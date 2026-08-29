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
    // Seed the option table with zsh's own defaults. A test binary
    // never runs `parseargs`/`setupvals`, so `OPTS_LIVE` starts EMPTY
    // and every `isset(X)` reports false — including options zsh
    // enables by default (UNSET, EXECOPT, PROMPTPERCENT, CASEMATCH…).
    // That made `let ZL_X=5` fail with "parameter not set" (NO_UNSET
    // semantics) whenever no earlier test had happened to populate the
    // table as a side effect. `emulate("zsh", fully)` is the canonical
    // populator (Src/options.c:533 → installemulation c:523), and
    // running it per-test also rolls back whatever options the previous
    // test flipped.
    crate::ported::options::emulate("zsh", true);
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

/// Reset the completion-machinery globals a `compadd`-driven completion
/// unit test depends on, so its result reflects the test's own inputs
/// rather than whatever the previous test in the same binary left behind.
///
/// [`global_state_lock`] serialises stateful tests but restores nothing,
/// and the completion subsystem is almost entirely process-wide state:
///
/// * `$PREFIX` / `$SUFFIX` / `$IPREFIX` / `$ISUFFIX`. `addmatches`
///   re-seeds the `compprefix` / `compsuffix` / `compiprefix` /
///   `compisuffix` globals from those parameters whenever `incompfunc`
///   is set (`src/ported/zle/compcore.rs:4334-4347`) and then matches
///   every `CAF_MATCH` candidate against them
///   (`compcore.rs:4360-4373`, `Src/Zle/compcore.c:2253-2300`). A dozen
///   completion tests park a non-empty word there and never clear it
///   (`_absolute_command_paths.rs:168` `"ls"`, `_ldap_filters.rs:267`
///   `"-x"`, `_debbugs_bugnumber.rs:142` `"notabug"`, …); after any of
///   them EVERY candidate a later `compadd` offers fails to match, so
///   `compadd` returns 1 and `_all_labels` / `_wanted` report "no
///   matches" for a tag set that was registered perfectly well.
/// * `comptags[]`, which `bin_comptags` indexes by `locallevel`
///   (`Src/Zle/computil.c:3782` "Array of tag-set infos. Index is the
///   locallevel", ported at `computil.rs:6886`), plus `locallevel`
///   itself — nothing unwinds it when a test panics out of a
///   `doshfunc`-shaped port.
/// * the `zstyle` table, which `_hosts` / `_domains` / `_completers` /
///   `_call_program` all consult for their candidate lists.
/// * the three process-wide `compadd` shadows `_approximate` and
///   `_complete_help` install (`src/ported/zle/complete.rs:975-982`,
///   `:1043-1048`).
///
/// Every half of this is the boot/teardown entry point zsh itself uses,
/// not a bespoke reset: `Src/Zle/complete.c:1788 finish_` zsfree's each
/// `comp*` string global, `Src/Zle/computil.c:5124 setup_` zeroes
/// `comptags[]` and `lasttaglevel`, and `zstyle -d` with no pattern is
/// `zstyletab->emptytable` (`Src/Modules/zutil.c:639-640`).
pub fn reset_completion_state() {
    use std::sync::atomic::Ordering;
    // c:Src/Zle/complete.c:1788 — clears compprefix/compsuffix/compiprefix/
    // compisuffix/compqiprefix/compqisuffix/compquote/compqstack/complist
    // and compwords.
    let _ = crate::ported::zle::complete::finish_(std::ptr::null());
    // c:Src/Zle/computil.c:5124 — `memset(comptags, 0, sizeof(comptags))`
    // plus `lasttaglevel = 0`.
    let _ = crate::ported::zle::computil::setup_();
    // The globals cleared above are re-seeded FROM these parameters at
    // compcore.rs:4334-4347, so clearing only the globals would leave the
    // stale word to come straight back. `curcontext` selects every style
    // context; `expl`, `_comp_tags`, `_tags_level`, `_next_tags_not` and
    // `_sort_tags` are the bookkeeping parameters `_tags` / `_all_labels`
    // read (`_all_labels.rs:291-323`, `_tags.rs:157`, `_tags.rs:213`).
    for p in [
        "PREFIX",
        "SUFFIX",
        "IPREFIX",
        "ISUFFIX",
        "QIPREFIX",
        "QISUFFIX",
        "curcontext",
        "expl",
        "_comp_tags",
        "_tags_level",
        "_next_tags_not",
        "_sort_tags",
    ] {
        crate::ported::params::unsetparam(p);
    }
    // c:Src/params.c:54 `locallevel` — the `comptags[]` index.
    crate::ported::params::locallevel.store(0, Ordering::Relaxed);
    // c:Src/Zle/complete.c:41 `compignored` — `$compstate[ignored]`, the
    // gate `_ignored` fires on.
    crate::ported::zle::complete::COMPIGNORED.store(0, Ordering::Relaxed);
    // complete.rs:975-982 / :983-1002 / :1043-1048 — the shell-function,
    // argv-rewrite and trace shadows over `compadd`. All three make the
    // builtin return 1 (or swallow the call) for reasons that have
    // nothing to do with the caller under test.
    crate::ported::zle::complete::set_compadd_trace(false);
    crate::ported::zle::complete::clear_compadd_prefix_injector();
    if let Ok(mut g) = crate::ported::zle::complete::COMPADD_ARGV_SHADOW.lock() {
        *g = None;
    }
    // c:Src/Modules/zutil.c:639-640 — `zstyle -d` with no pattern is
    // `zstyletab->emptytable`, i.e. drop every style.
    let ops = crate::ported::zsh_h::options {
        ind: [0u8; crate::ported::zsh_h::MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    let _ = crate::ported::modules::zutil::bin_zstyle("zstyle", &["-d".to_string()], &ops, 0);
}

/// Install one `zstyle` for the duration of a test, the way a user would
/// write it on the command line: `zstyle <context> <style> <value…>`
/// (`Src/Modules/zutil.c:606-616` — the no-flag arm is `setstyle`).
///
/// Completion functions that shell out through `_call_program` take the
/// command line from the `command` style
/// (`Completion/Base/Utility/_call_program:26`, ported at
/// `_call_program.rs:74-101`), so this is the upstream-sanctioned way to
/// give such a port a fixed candidate list instead of whatever the host
/// machine's `ifconfig` / `global` happens to print.
pub fn set_test_zstyle(context: &str, style: &str, value: &str) {
    let ops = crate::ported::zsh_h::options {
        ind: [0u8; crate::ported::zsh_h::MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    let _ = crate::ported::modules::zutil::bin_zstyle(
        "zstyle",
        &[context.to_string(), style.to_string(), value.to_string()],
        &ops,
        0,
    );
}
