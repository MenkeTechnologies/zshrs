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
//! from 0 to 139 divergences, `jobs` 0 -> 16, `errexit` 0 -> 13). So
//! `init_dispatch_signals` below replays `init_signals` line by line with
//! that ONE line left out, until those dispatch paths are converged onto
//! `zsh_main`.

/// Whether the inherited-signal bookkeeping below applies at all.
///
/// This is ZSH behaviour, not shared POSIX behaviour, so it must not
/// leak into the drop-in emulation modes. Measured with SIGQUIT and
/// SIGHUP both inherited as SIG_IGN, `<shell> -c trap` prints:
///   zsh   `trap -- '' QUIT`            (QUIT only, bare name)
///   bash  `trap -- '' SIGHUP` + `SIGQUIT`  (both, SIG-prefixed)
///   dash  nothing
///   ksh   nothing
///   sh    nothing
/// so recording it unconditionally made every POSIX-family drop-in
/// print a QUIT line the real shell never prints. bash's variant is a
/// different shape (every inherited ignore, SIG names) and is ported
/// separately as [`hard_ignored`].
#[cfg(unix)]
fn zsh_mode_only() -> bool {
    !crate::dash_mode::posix_faithful() && !crate::dash_mode::bash_mode()
}

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
    if !zsh_mode_only() {
        return;
    }
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

/// Bit `n` is set when signal `n` was `SIG_IGN` when the shell started.
static IGNORED_ON_ENTRY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// POSIX: "Signals that were ignored on entry to a non-interactive shell cannot be trapped
/// or reset." bash, dash, ksh93 and mksh all honour it; zsh does not, so only the real-shell
/// drop-ins consult this (see `settrap`).
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
#[cfg(unix)]
pub fn record_ignored_on_entry() {
    let mut mask = 0u64;
    for sig in 1..32 {
        if sig == libc::SIGKILL || sig == libc::SIGSTOP {
            continue;
        }
        let ignored = unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            libc::sigaction(sig, std::ptr::null(), &mut act) == 0 && act.sa_sigaction == libc::SIG_IGN
        };
        if ignored {
            mask |= 1u64 << sig;
        }
    }
    IGNORED_ON_ENTRY.store(mask, std::sync::atomic::Ordering::Relaxed);
}

/// Whether `sig` was ignored when the shell started.
pub fn ignored_on_entry(sig: i32) -> bool {
    (1..64).contains(&sig) && IGNORED_ON_ENTRY.load(std::sync::atomic::Ordering::Relaxed) & (1u64 << sig) != 0
}

/// Run `init_signals` (`Src/init.c:1427-1470`) on a dispatch path that
/// does not reach `ported::init::zsh_main`.
///
/// Every line of C's `init_signals` is replayed here in C's order bar
/// one, with the measurement at its site below: `install_handler(SIGCHLD)`
/// (`c:1455`). The `sigtrapped`/`siglists`
/// allocations (`c:1431-1432`) and the `sigchld_mask` cache (`c:1440`)
/// have no zshrs counterpart, same as in the ported `init_signals`
/// itself.
///
/// Without this, `zsh -f -i -c 'kill -ALRM $$'` printed `zsh:1: timeout`
/// and exited 14 while zshrs died from the raw signal with 142: SIGALRM,
/// SIGPIPE and the SIGTERM/SIGQUIT ignores were only ever armed for a
/// shell that read its input from stdin. SIGHUP was unarmed on BOTH the
/// interactive and non-interactive `-c` path (129 vs zsh's 1).
///
/// The ORDER matters beyond tidiness: C records an inherited `SIG_IGN` on
/// SIGQUIT (`c:1444-1445`) only AFTER the interactive branch has reset
/// every disposition to default (`c:1437-1438`), so an interactive shell
/// never records it. Doing the record first — which is what the two
/// dispatch sites used to do — made `nohup zshrs -fic trap` print
/// `trap -- '' QUIT` where zsh prints nothing.
#[cfg(unix)]
pub fn init_dispatch_signals() {
    record_ignored_on_entry();
    use crate::ported::signals::install_handler;
    use crate::ported::signals_h::{signal_default, signal_ignore, winch_block, SIGCOUNT};
    use crate::ported::zsh_h::{interact, jobbing};

    // Same gate as `record_inherited_sigquit_ignore`: this is ZSH's
    // startup signal policy, and the drop-in modes have their own.
    // Measured under `-i -c 'kill -<sig> $$; echo survived'`:
    //   TERM  bash survives, zsh survives      — agree
    //   QUIT  bash survives, zsh survives      — agree
    //   ALRM  bash dies (142), zsh prints "timeout" and exits 14
    //   HUP   bash dies (129), zsh exits 1
    // so arming zsh's SIGALRM/SIGHUP handlers in `--bash` would break the
    // two rows that used to agree. A drop-in mode that wants its own
    // arming needs its own port of it, not a share of this one.
    if !zsh_mode_only() {
        return;
    }

    // c:1434-1439 — `if (interact) { signal_setmask(signal_mask(0));
    // for (i=0; i<NSIG; ++i) signal_default(i); }`.
    if interact() {
        let empty = crate::ported::signals::signal_mask(0);
        let _ = crate::ported::signals::signal_setmask(&empty);
        // c:1437-1438 — `SIGCOUNT` is the port of NSIG-1; skip 0, whose
        // `signal_default(0)` is implementation-defined.
        for i in 1..=SIGCOUNT {
            let _ = signal_default(i);
        }
    }

    // c:1440 — `sigchld_mask = signal_mask(SIGCHLD);` not modeled.

    // c:1442 — `intr();`, i.e. `if (interact) install_handler(SIGINT);`.
    //
    // C's handler does not terminate the shell on an untrapped SIGINT: it
    // sets `errflag |= ERRFLAG_INT` and `lastval = 128 + SIGINT`
    // (c:Src/signals.c:457/463), and the ABORT comes from execlist's list
    // gate `while (… && !errflag)` (c:Src/exec.c:1443) ending the list.
    //
    // This line was left out while that gate was missing, because a handler
    // without it is strictly worse than the default disposition: the
    // interrupt was recorded and then ignored, so
    // `-i -c 'kill -INT $$; print survived'` printed `survived` and exited 0
    // where zsh is silent and exits 130. The gate now exists —
    // `BUILTIN_FATAL_ABORT_CHECK`, which `compile_program` emits between
    // every pair of top-level list elements, tests the WHOLE errflag word
    // and no longer exempts an interactive shell — so the handler is
    // installed as C installs it. Note `bin_trap` installs it itself when a
    // real INT trap is set, so trapped interrupts never depended on this.
    crate::ported::signals::intr();

    // c:1444-1445 — inherited SIG_IGN on SIGQUIT becomes ZSIG_IGNORED.
    record_inherited_sigquit_ignore();

    // c:1447-1449 — `#ifndef QDEBUG signal_ignore(SIGQUIT); #endif`
    signal_ignore(libc::SIGQUIT);

    // c:1451-1454 — an inherited SIG_IGN on SIGHUP clears the HUP option
    // (so `set -o` reports `nohup` under nohup/supervisors); otherwise
    // the handler takes over, which is what turns an untrapped HUP into
    // zsh's exit 1 instead of the raw 129.
    if signal_ignore(libc::SIGHUP) == libc::SIG_IGN {
        crate::ported::options::dosetopt(crate::ported::zsh_h::HUP, 0, 0); // c:1452
    } else {
        install_handler(libc::SIGHUP); // c:1454
    }

    // !!! DELIBERATE OMISSION — NO C COUNTERPART FOR THE ABSENCE !!!
    // c:1455 — `install_handler(SIGCHLD);`. zshrs's pipelines reap their
    // own children with `waitpid`; C's SIGCHLD reaper races them and
    // destroys `$pipestatus`. Module docs carry the measurement.

    // c:1456-1459 — `#ifdef SIGWINCH install_handler(SIGWINCH);
    // winch_block(); #endif`. The standing block is the delivery policy:
    // a resize stays PENDING until an explicit unblock window, and the
    // fork-child unblocks before `execve` (c:Src/exec.c:533, ported at
    // ported/exec.rs:4305) so the new program starts with it deliverable.
    #[cfg(not(target_os = "haiku"))]
    {
        install_handler(libc::SIGWINCH); // c:1457
        winch_block(); // c:1458
    }

    // c:1460-1464 — interactive-only: SIGPIPE and SIGALRM get handlers
    // (SIGALRM is how `$TMOUT` logs out with "timeout"), SIGTERM is
    // ignored outright so an interactive shell survives it.
    if interact() {
        install_handler(libc::SIGPIPE); // c:1461
        install_handler(libc::SIGALRM); // c:1462
        signal_ignore(libc::SIGTERM); // c:1463
    }

    // c:1465-1469 — job-control signals are ignored in the shell itself.
    if jobbing() {
        signal_ignore(libc::SIGTTOU); // c:1466
        signal_ignore(libc::SIGTSTP); // c:1467
        signal_ignore(libc::SIGTTIN); // c:1468
    }
}

/// Non-unix stub: none of these signals exist.
#[cfg(not(unix))]
pub fn init_dispatch_signals() {}

/// Signals that arrived with a `SIG_IGN` disposition (bit N = signal N),
/// captured by [`snapshot_inherited_ignores`] before the shell installs a
/// single handler of its own.
#[cfg(unix)]
static INHERITED_IGNORED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Record which signals the shell inherited as `SIG_IGN`.
///
/// Port of bash's `GETORIGSIG` (bash-5.3 `trap.c:134-140`), which reads
/// each signal's original disposition and marks an inherited `SIG_IGN`
/// as `SIG_HARD_IGNORE`. bash reads them lazily; reading them all here,
/// first thing in `main`, gives the same answer because nothing has
/// touched a disposition yet.
///
/// SIGPIPE is skipped: Rust's runtime sets it to `SIG_IGN` before `main`
/// runs, so its inherited disposition can no longer be read.
#[cfg(unix)]
pub fn snapshot_inherited_ignores() {
    let mut mask = 0u64;
    for sig in 1..=crate::ported::signals_h::SIGCOUNT.min(63) {
        if sig == libc::SIGPIPE {
            continue;
        }
        let ignored = unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            libc::sigaction(sig, std::ptr::null(), &mut act) == 0 && act.sa_sigaction == libc::SIG_IGN
        };
        if ignored {
            mask |= 1u64 << sig;
        }
    }
    INHERITED_IGNORED.store(mask, std::sync::atomic::Ordering::Relaxed);
}

/// Non-unix stub: there are no inherited dispositions to read.
#[cfg(not(unix))]
pub fn snapshot_inherited_ignores() {}

/// Whether `sig` is hard-ignored: a signal ignored on entry to a
/// NON-interactive bash, ksh93 or `/bin/sh` "cannot be trapped or reset, but
/// no error is reported when attempting to do so" (bash-5.3 `trap.c:850-853`,
/// `:954-957`, `:1001-1004`; ksh93 and bash-posix `/bin/sh` measured the same).
/// An interactive shell may trap it. zsh, dash and mksh have no such rule.
/// BUGS.md #1109.
#[cfg(unix)]
pub fn hard_ignored(sig: i32) -> bool {
    use crate::dash_mode::{bash_mode, ksh93_mode, sh_mode};
    (bash_mode() || ksh93_mode() || sh_mode())
        && !crate::ported::zsh_h::interact()
        && (1..64).contains(&sig)
        && INHERITED_IGNORED.load(std::sync::atomic::Ordering::Relaxed) & (1u64 << sig) != 0
}

/// Non-unix stub: nothing is ever inherited as ignored.
#[cfg(not(unix))]
pub fn hard_ignored(_sig: i32) -> bool {
    false
}

/// Hard-ignored signals a `trap` command has named (bit N = signal N).
static HARD_IGNORED_NAMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Record that a `trap` command named hard-ignored `sig` (set, `''` or `-`).
pub fn note_hard_ignored_named(sig: i32) {
    if (1..64).contains(&sig) {
        HARD_IGNORED_NAMED.fetch_or(1u64 << sig, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Whether the trap listing shows hard-ignored `sig` as `trap -- '' SIG`.
/// bash always does (`builtins/trap.def:302-303`); ksh93 only once a `trap`
/// command has named the signal; `/bin/sh` never does.
pub fn hard_ignored_listed(sig: i32) -> bool {
    hard_ignored(sig)
        && (crate::dash_mode::bash_mode()
            || (crate::dash_mode::ksh93_mode()
                && HARD_IGNORED_NAMED.load(std::sync::atomic::Ordering::Relaxed) & (1u64 << sig) != 0))
}
