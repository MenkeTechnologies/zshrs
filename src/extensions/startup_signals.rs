//! Startup signal-state recording for the dispatch paths that do not run
//! `ported::init::zsh_main`.
//!
//! !!! WARNING: RUST-ONLY MODULE — NO C COUNTERPART FILE !!!
//! The BEHAVIOUR here is a faithful port of two lines of
//! `Src/init.c:1444-1445`, but the standalone home is a Rust-only
//! adaptation, so it lives under `src/extensions/` rather than
//! `src/ported/` (where `tests/port_purity.rs` and the build gate require
//! every fn to map to a real `Src/<x>.c` function).
//!
//! Why it exists: C's `main` has a SINGLE path —
//! `parseargs` -> `init_io` -> `setupvals` -> `init_signals` -> run — so
//! every invocation, `-c` included, reaches `init_signals`. zshrs
//! dispatches `-c` and a script FILE inside `bins/zshrs.rs` without going
//! through `ported::init::zsh_main`, so `init_signals` never ran for them.
//!
//! Calling the WHOLE of `init_signals` from those paths is not a safe
//! substitute: it also installs C's SIGCHLD handler, whose reaper then
//! races the pipeline's own `waitpid` and destroys `$pipestatus`
//! (measured with that call wired in: `parity-fuzz --mode pipeline` went
//! from 0 to 139 divergences, `jobs` 0 -> 16, `errexit` 0 -> 13). Only the
//! inherited-SIGQUIT half is shared here until those dispatch paths are
//! converged onto `zsh_main`.

/// Record an INHERITED `SIG_IGN` disposition for `SIGQUIT`.
///
/// Port of `Src/init.c:1444-1445`:
/// ```c
/// if (!sigaction(SIGQUIT, NULL, &act) && act.sa_handler == SIG_IGN)
///     sigtrapped[SIGQUIT] = ZSIG_IGNORED;
/// ```
///
/// A shell that inherits SIGQUIT ignored — `nohup`, a `trap '' QUIT`
/// parent, most supervisors, and `cargo test`'s own spawn — records it as
/// an ignored trap. `trap` then lists `trap -- '' QUIT` and `entersubsh`
/// keeps it ignored in children (`c:Src/exec.c:1231`). Without this,
/// `nohup zshrs -fc trap` printed nothing where zsh printed the QUIT line.
///
/// Note this only RECORDS the inherited state; it deliberately does not
/// call `signal_ignore(SIGQUIT)` (`c:1448`), which would be a no-op for an
/// already-ignored signal anyway.
#[cfg(unix)]
pub fn record_inherited_sigquit_ignore() {
    // c:1444 — `if (!sigaction(SIGQUIT, NULL, &act) && act.sa_handler == SIG_IGN)`
    let is_ignored = unsafe {
        let mut act: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGQUIT, std::ptr::null(), &mut act) == 0
            && act.sa_sigaction == libc::SIG_IGN
    };
    if !is_ignored {
        return;
    }
    // c:1445 — `sigtrapped[SIGQUIT] = ZSIG_IGNORED;`
    if let Ok(mut guard) = crate::ported::signals::sigtrapped.lock() {
        if let Some(slot) = guard.get_mut(libc::SIGQUIT as usize) {
            *slot = crate::ported::zsh_h::ZSIG_IGNORED;
        }
    }
}

/// Non-unix stub: there is no SIGQUIT to inherit.
#[cfg(not(unix))]
pub fn record_inherited_sigquit_ignore() {}
