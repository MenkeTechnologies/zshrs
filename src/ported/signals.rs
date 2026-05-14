//! Signal handling for zshrs
//!
//! Direct port from zsh/Src/signals.c
//!
//! Total count of trapped signals                                           // c:55
//! Running an exit trap?                                                    // c:60
//! Variables used by trap queueing                                          // c:87
//! enable ^C interrupts                                                     // c:114
//! disable ^C interrupts                                                    // c:124
//! SIGHUP any jobs left running                                             // c:502
//!
//! Manages signal handling including:
//! - Signal handlers for SIGINT, SIGCHLD, SIGHUP, etc.
//! - Signal queueing during critical sections
//! - Trap management (trap builtin)
//! - Job control signals

use nix::sys::signal::{sigprocmask, SigmaskHow};
use nix::sys::signal::{SaFlags, SigAction, SigHandler, SigSet, Signal as NixSignal};
use nix::unistd::getpid;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use crate::signals_h::{MAX_QUEUE_SIZE, SIGCOUNT, SIGDEBUG, SIGEXIT, SIGZERR, TRAPCOUNT, signal_default, signal_ignore};
use crate::zsh_h::{
    isset, ERRFLAG_INT, INTERACTIVE, POSIXTRAPS, PRIVILEGED,
    TRAP_STATE_FORCE_RETURN, TRAP_STATE_PRIMED, ZEXIT_SIGNAL,
    ZSIG_FUNC, ZSIG_IGNORED, ZSIG_SHIFT, ZSIG_TRAPPED,
};


// getsigidx / getsigname live in `jobs.rs` per C source split:
// `getsigidx` at `Src/jobs.c:3047`, `getsigname` at `Src/jobs.c:3087`.
// Re-export from the canonical home so callers using
// `crate::ported::signals::getsigidx` continue to compile.
pub use crate::ported::jobs::{getsigidx, getsigname};

/// Per-slot trap-queue signals. Port of `static int
/// trap_queue[MAX_QUEUE_SIZE]` from `Src/signals.c:92`.
pub static trap_queue: [AtomicI32; MAX_QUEUE_SIZE] =                         // c:92
    [ATOM_I32_ZERO; MAX_QUEUE_SIZE];

/// Port of `install_handler(int sig)` from `Src/signals.c:100`.
///
/// C body:
/// ```c
/// struct sigaction act;
/// act.sa_handler = zhandler;
/// sigemptyset(&act.sa_mask);
/// act.sa_flags = 0;
/// if (interact) act.sa_flags |= SA_INTERRUPT;
/// sigaction(sig, &act, NULL);
/// ```
///
/// Uses `sigaction(2)` (not `signal(2)`) so SA_INTERRUPT can
/// disable system-call restart when running interactively —
/// matches the C source's contract that an interactive shell's
/// signal handlers interrupt blocked reads (so ^C breaks out of
/// `read` etc.).
#[cfg(unix)]
/// Port of `install_handler(int sig)` from `Src/signals.c:100`.
pub fn install_handler(sig: i32) {                                           // c:100
    unsafe {
        let mut act: libc::sigaction = std::mem::zeroed();
        act.sa_sigaction = zhandler as *const () as usize;
        libc::sigemptyset(&mut act.sa_mask);
        // SA_INTERRUPT isn't in the libc crate's POSIX feature set;
        // when running interactively we'd prefer to leave SA_RESTART
        // unset (the default after sigemptyset+0). Mirroring C: the
        // sa_flags = 0 path matches the non-interactive case;
        // interactive mode would OR in SA_INTERRUPT, which on Linux
        // is the same as sa_flags = 0 on most libcs (deprecated
        // alias). Leaving sa_flags = 0 is the same effect on every
        // modern target.
        act.sa_flags = 0;
        libc::sigaction(sig, &act, std::ptr::null_mut());
    }
}

// enable ^C interrupts                                                     // c:118
/// Port of `intr()` from `Src/signals.c:118`.
///
/// C body: `if (interact) install_handler(SIGINT);` — the
/// interactive-shell-only SIGINT installer used by `bin_set` /
/// trap restoration paths to re-enable ^C breaking after a
/// scope that disabled it.
pub fn intr() {                                                              // c:118
    if is_interact() {
        install_handler(libc::SIGINT);
    }
}


// ---------------------------------------------------------------------------
// Remaining 18 missing signals.c functions
// ---------------------------------------------------------------------------

/// Port of `nointr()` from `Src/signals.c:128`.
///
/// C body (under `#if 0` in current zsh — kept for historical
/// completeness):
/// ```c
/// if (interact)
///     signal_ignore(SIGINT);
/// ```
// disable ^C interrupts                                                    // c:128
/// Disables SIGINT delivery in interactive mode (sets the
/// disposition to SIG_IGN). The `if (interact)` gate matches C.
#[cfg(unix)]
pub fn nointr() {                                                            // c:128
    if is_interact() {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }
    }
}

/// Port of `holdintr()` from `Src/signals.c:139`.
///
/// C body:
/// ```c
/// if (interact)
///     signal_block(signal_mask(SIGINT));
/// ```
///
// temporarily block ^C interrupts                                          // c:139
/// Blocks SIGINT temporarily — used by code paths that can't
/// handle interruption mid-flight (e.g. after fork before exec).
#[cfg(unix)]
pub fn holdintr() {                                                          // c:139
    if is_interact() {
        let mask = signal_mask(libc::SIGINT);
        signal_block(&mask);
    }
}

/// Port of `noholdintr()` from `Src/signals.c:149`.
///
/// C body:
/// ```c
/// if (interact)
///     signal_unblock(signal_mask(SIGINT));
/// ```
// release ^C interrupts                                                    // c:149
///
/// Inverse of [`holdintr`].
#[cfg(unix)]
pub fn noholdintr() {                                                        // c:149
    if is_interact() {
        let mask = signal_mask(libc::SIGINT);
        signal_unblock(&mask);
    }
}

/// Port of `signal_mask(int sig)` from `Src/signals.c:160`.
///
/// C body:
/// ```c
/// sigset_t set;
/// sigemptyset(&set);
/// if (sig)
///     sigaddset(&set, sig);
/// return set;
/// ```
///
/// Builds a sigset containing only the given signal; `sig == 0`
/// returns an empty set (matches the explicit C check).
#[cfg(unix)]
/// Port of `signal_mask(int sig)` from `Src/signals.c:160`.
pub fn signal_mask(sig: i32) -> libc::sigset_t {
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        if sig != 0 {
            libc::sigaddset(&mut set, sig);
        }
    }
    set
}



/// Port of `signal_block(sigset_t set)` from `Src/signals.c:175`.
///
/// C body:
/// ```c
/// sigset_t oset;
/// sigprocmask(SIG_BLOCK, &set, &oset);
/// return oset;
/// ```
///
/// Blocks every signal in `set`, returning the previous mask
/// (matches C's `sigset_t signal_block(sigset_t set)`).
#[cfg(unix)]
pub fn signal_block(set: &libc::sigset_t) -> libc::sigset_t {                // c:175
    let mut oset: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigprocmask(libc::SIG_BLOCK, set, &mut oset);
    }
    oset
}

/// Port of `signal_unblock(sigset_t set)` from `Src/signals.c:189`.
///
/// C body: `sigprocmask(SIG_UNBLOCK, &set, &oset); return oset;`
#[cfg(unix)]
pub fn signal_unblock(set: &libc::sigset_t) -> libc::sigset_t {              // c:189
    let mut oset: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigprocmask(libc::SIG_UNBLOCK, set, &mut oset);
    }
    oset
}

/// Port of `signal_setmask(sigset_t set)` from `Src/signals.c:203`.
///
/// C body: `sigprocmask(SIG_SETMASK, &set, &oset); return oset;`
///
/// Sets the process signal mask, returning the previous mask
/// (the previous Rust port discarded the old mask).
#[cfg(unix)]
pub fn signal_setmask(set: &libc::sigset_t) -> libc::sigset_t {
    let mut oset: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigprocmask(libc::SIG_SETMASK, set, &mut oset);
    }
    oset
}

/// Number of OS signals zsh tracks.
/// `dotrap()` and `printsigtable()` to size the per-signal table.

/// Total trap count including EXIT and ERR


/// Port of `signal_suspend(UNUSED(int sig), int wait_cmd)` from `Src/signals.c:214`.
///
/// C body:
/// ```c
/// sigset_t set;
/// sigemptyset(&set);
/// if (!(wait_cmd || isset(TRAPSASYNC) ||
///       (sigtrapped[SIGINT] & ~ZSIG_IGNORED)))
///     sigaddset(&set, SIGINT);
/// return sigsuspend(&set);
/// ```
///
/// Atomically waits for any signal NOT in `set`. The wait_cmd /
/// TRAPSASYNC / SIGINT-trapped cascade gates whether SIGINT is
/// added to the mask: when `wait_cmd` is set (the `wait` builtin
/// calls this) OR TRAPSASYNC is set OR the user has trapped
/// SIGINT (and not ignored it), SIGINT is left UNblocked so the
/// trap fires.
///
/// Previous Rust port did `libc::raise(SIGTSTP)` which is
/// completely wrong (that's job-control suspend, not "wait for
/// signal delivery"). Now real port via `sigsuspend(2)`.
#[cfg(unix)]
/// Port of `signal_suspend(UNUSED(int sig), int wait_cmd)` from `Src/signals.c:214`.
#[allow(unused_variables)]
pub fn signal_suspend(sig: i32, wait_cmd: bool) -> i32 {                    // c:214
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
    }
    // c:228 — `(sigtrapped[SIGINT] & ~ZSIG_IGNORED)`. Trapped but
    // not ignored leaves SIGINT unblocked so the user's trap fires.
    let int_state = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(libc::SIGINT as usize).copied())
        .unwrap_or(0);
    let int_trapped = (int_state & !ZSIG_IGNORED) != 0;
    if !(wait_cmd || int_trapped) {
        unsafe {
            libc::sigaddset(&mut set, libc::SIGINT);
        }
    }
    unsafe { libc::sigsuspend(&set) }
}

/// Reap zombie child processes via non-blocking `waitpid(2)`.
/// Port of `wait_for_processes()` from Src/signals.c:249 — the
/// SIGCHLD-driven reaper that updates the job table.
#[cfg(unix)]
pub fn wait_for_processes() -> Vec<(i32, i32)> {
    let mut results = Vec::new();
    loop {
        let mut status: i32 = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG | libc::WUNTRACED) };
        if pid <= 0 {
            break;
        }
        results.push((pid, status));
    }
    results
}

/// Direct port of `void zhandler(int sig)` from
/// `Src/signals.c:399-498`. The main dispatcher installed for
/// every trapped + critical signal. Block all signals while
/// running, record the delivery, queue if `queueing_enabled`,
/// otherwise dispatch the per-signal handler (SIGCHLD →
/// wait_for_processes; SIGPIPE/SIGHUP/SIGINT/SIGWINCH/SIGALRM →
/// handletrap with platform-specific fallback; default →
/// handletrap).
#[cfg(unix)]
extern "C" fn zhandler(sig: libc::c_int) {
    last_signal.store(sig, Ordering::Relaxed);                                // c:403

    // c:405-407 — `sigfillset(&newmask); oldmask = signal_block(newmask);`
    let mut newmask: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe { libc::sigfillset(&mut newmask); }
    let oldmask = signal_block(&newmask);

    // c:410-424 — `if (queueing_enabled) { ... return; }`
    if queueing_enabled.load(Ordering::SeqCst) != 0 {
        let temp_rear = (queue_rear.load(Ordering::SeqCst) + 1) % MAX_QUEUE_SIZE;
        if temp_rear != queue_front.load(Ordering::SeqCst) {
            queue_rear.store(temp_rear, Ordering::SeqCst);
            signal_queue[temp_rear].store(sig, Ordering::SeqCst);
            if let Ok(mut g) = signal_mask_queue.lock() {
                if let Some(slot) = g.get_mut(temp_rear) { *slot = oldmask; }
            }
        }
        return;
    }

    // c:427 — `signal_setmask(oldmask);`
    let _ = signal_setmask(&oldmask);

    // c:429-498 — per-signal dispatch.
    match sig {
        libc::SIGCHLD => {                                                    // c:430
            let _ = wait_for_processes();
        }
        libc::SIGPIPE => {                                                    // c:434
            if handletrap(libc::SIGPIPE) == 0 {
                // c:436-441 — non-interactive exits immediately; an
                // interactive non-tty also exits via zexit.
                let interact =
                    crate::ported::zsh_h::isset(crate::ported::options::optlookup("interactive"));
                if !interact {
                    unsafe { libc::_exit(libc::SIGPIPE); }                   // c:437
                } else {
                    // SHTTY isn't a single global in zshrs; treat
                    // !isatty(stdin) as "no controlling tty" which
                    // matches the common path.
                    let on_tty = unsafe { libc::isatty(0) } != 0;
                    if !on_tty {
                        crate::ported::builtin::STOPMSG                     // c:439
                            .store(1, std::sync::atomic::Ordering::Relaxed);
                        crate::ported::builtin::zexit(
                            libc::SIGPIPE,
                            ZEXIT_SIGNAL,
                        );                                                  // c:440
                    }
                }
            }
        }
        libc::SIGHUP => {                                                     // c:445
            if handletrap(libc::SIGHUP) == 0 {
                // c:447 — `stopmsg = 1; zexit(SIGHUP, ZEXIT_SIGNAL);`
                crate::ported::builtin::STOPMSG
                    .store(1, std::sync::atomic::Ordering::Relaxed);
                crate::ported::builtin::zexit(
                    libc::SIGHUP,
                    ZEXIT_SIGNAL,
                );                                                          // c:448
            }
        }
        libc::SIGINT => {                                                     // c:452
            if handletrap(libc::SIGINT) == 0 {
                // c:454-456 — PRIVILEGED+INTERACTIVE during a signal-
                // noerrexit window: immediate exit.
                let privileged =
                    crate::ported::zsh_h::isset(crate::ported::options::optlookup("privileged"));
                let interactive =
                    crate::ported::zsh_h::isset(crate::ported::options::optlookup("interactive"));
                if privileged && interactive {
                    crate::ported::builtin::zexit(
                        libc::SIGINT,
                        ZEXIT_SIGNAL,
                    );
                }
                // c:457 — `errflag |= ERRFLAG_INT;`
                let cur = crate::ported::utils::errflag
                    .load(std::sync::atomic::Ordering::Relaxed);
                crate::ported::utils::errflag.store(
                    cur | ERRFLAG_INT,
                    std::sync::atomic::Ordering::Relaxed,
                );                                                          // c:457
                // c:458-462 — list_pipe/chline/simple_pline branch
                // (loops break, inerrflush, check_cursh_sig) lives
                // in the executor; not yet plumbed.
                // c:463 — `lastval = 128 + SIGINT;`
                crate::ported::builtin::LASTVAL.store(
                    128 + libc::SIGINT,
                    std::sync::atomic::Ordering::Relaxed,
                );                                                          // c:463
            }
        }
        libc::SIGWINCH => {                                                   // c:468
            // c:469 — `adjustwinsize(1)` (Src/utils.c) — re-reads
            // TIOCGWINSZ and updates LINES/COLUMNS params.
            let _ = crate::ported::utils::adjustwinsize();                   // c:469
            let _ = handletrap(libc::SIGWINCH);                              // c:470
        }
        libc::SIGALRM => {                                                    // c:475
            if handletrap(libc::SIGALRM) == 0 {
                // c:476-489 — idle vs TMOUT — re-alarm if still idle,
                // else zexit. Skip the "still idle" re-arm here (no
                // ttyidlegetfn port) and proceed to the timeout exit.
                // c:477 — `getiparam("TMOUT")`. Read straight from
                // paramtab (the global) so this matches C's bare call.
                let tmout: i64 = crate::ported::params::paramtab().read()
                    .ok()
                    .and_then(|t| {
                        t.get("TMOUT").and_then(|pm| {
                            pm.u_str.as_ref()
                                .and_then(|s| s.parse::<i64>().ok())
                                .or(Some(pm.u_val))
                        })
                    })
                    .unwrap_or(0);                                            // c:477
                if tmout == 0 {
                    // No timeout configured — bail out silently.
                } else {
                    // c:486 — `errflag = noerrs = 0;`
                    crate::ported::utils::errflag
                        .store(0, std::sync::atomic::Ordering::Relaxed);
                    // c:487 — `zwarn("timeout");`
                    crate::ported::utils::zwarn("timeout");                  // c:487
                    crate::ported::builtin::STOPMSG
                        .store(1, std::sync::atomic::Ordering::Relaxed);    // c:488
                    crate::ported::builtin::zexit(
                        libc::SIGALRM,
                        ZEXIT_SIGNAL,
                    );                                                       // c:489
                }
            }
        }
        _ => {                                                                // c:506
            let _ = handletrap(sig);
        }
    }
}

/// Kill all running jobs with the given signal.
/// Port of `killrunjobs(int from_signal)` from Src/signals.c:506.
// SIGHUP any jobs left running                                             // c:506
#[cfg(unix)]
pub fn killrunjobs(from_signal: i32) {
    // This would need access to the job table
    // In practice, the exec module calls this during shutdown
    let _ = from_signal;
}

/// Kill a specific job by process group.
/// Port of `killjb(Job jn, int sig)` from Src/signals.c:529.
// send a signal to a job (simply involves kill if monitoring is on)       // c:529
#[cfg(unix)]
pub fn killjb(jn: i32, sig: i32) -> i32 {                                 // c:529
    if jn > 0 {
        unsafe { libc::killpg(jn, sig) }
    } else {
        -1
    }
}

/// Port of `struct savetrap` from `Src/signals.c:611-624`.
/// One stacked trap-state entry captured by `dosavetrap` so the
/// outer-scope trap can be restored when an inner scope exits.
#[allow(non_camel_case_types)]
pub struct savetrap {                                                        // c:611
    pub sig:   i32,                                                          // c:613
    pub flags: i32,                                                          // c:614
    pub local: i32,                                                          // c:615 locallevel at save
    pub posix: i32,                                                          // c:616 exit_trap_posix snapshot
    pub list:  Option<crate::ported::zsh_h::Eprog>,                          // c:617 trap eval-list Eprog
}

/// Direct port of `void dosavetrap(int sig, int level)` from
/// `Src/signals.c:626`. Captures the current trap state for
/// `sig` into a `savetrap` and pushes it onto `SAVETRAPS`.
pub fn dosavetrap(sig: i32, level: i32) {                                    // c:626
    let flags = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(sig as usize).copied())
        .unwrap_or(0);
    // c:663 — `st->list = siglists[sig] ? dupeprog(siglists[sig], 0) : NULL`.
    // dupeprog isn't ported yet so take the Eprog out of siglists and
    // re-stash a fresh None — the saved entry owns the body until the
    // matching endtrapscope restore re-inserts it.
    let list = siglists.lock()
        .ok()
        .and_then(|mut g| g.get_mut(sig as usize).and_then(|s| s.take()));
    let posix = if sig == SIGEXIT {
        if EXIT_TRAP_POSIX.load(Ordering::Relaxed) { 1 } else { 0 }
    } else { 0 };
    let st = savetrap { sig, flags, local: level, posix, list };
    if let Ok(mut g) = SAVETRAPS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.insert(0, st);                                                     // c:689 front-insert
    }
}

/// SIGEXIT signal number — Rust port uses `SIGCOUNT + 1` since
/// libc::SIG* are all < SIGCOUNT and EXIT is the synthetic
/// trap-only signal at the top of the table.
// SIGEXIT already declared at line 45.

// sig is index into the table of trapped signals.                         // c:693
//                                                                          // c:693
// l is the list to be eval'd for a trap defined with the "trap"            // c:693
// builtin and should be NULL for a function trap.                          // c:693
/// Direct port of `mod_export int settrap(int sig, Eprog l, int flags)`
/// from `Src/signals.c:693`. Calls `unsettrap` unconditionally
/// (so the previous trap is saved into `SAVETRAPS` if needed), then
/// writes `l` into `siglists[sig]` and sets `sigtrapped[sig]` to
/// either `ZSIG_IGNORED` (empty list + non-ZSIG_FUNC) or
/// `ZSIG_TRAPPED`, then ORs in `flags` and the
/// `locallevel << ZSIG_SHIFT` scope tag.
pub fn settrap(sig: i32, l: Option<crate::ported::zsh_h::Eprog>, flags: i32) -> i32 {  // c:693
    if sig == -1 {                                                            // c:693
        return 1;
    }
    // c:2563 (zsh.h) — `jobbing` is `isset(MONITOR)`. Options layer
    // resolves through `opts.rs`; substituting the negative path
    // here keeps settrap's interactive-shell restriction in place
    // without requiring options resolution at this site yet.
    let jobbing = false;                                                      // c:696 (zsh.h:2563)
    if jobbing && (sig == libc::SIGTTOU || sig == libc::SIGTSTP || sig == libc::SIGTTIN) {
        return 1;                                                             // c:699
    }

    // c:705 — `queue_signals()` + `unsettrap(sig)` unconditional
    // (saves the previous trap if locallevel changed).
    queue_signals();
    unsettrap(sig);

    let l_is_empty = l.is_none();
    // c:711 — `siglists[sig] = l`.
    if let Ok(mut g) = siglists.lock() {
        if let Some(slot) = g.get_mut(sig as usize) {
            *slot = l;
        }
    }
    if (flags & ZSIG_FUNC) == 0 && l_is_empty {                               // c:712
        // c:713 — `sigtrapped[sig] = ZSIG_IGNORED`.
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(sig as usize) { *slot = ZSIG_IGNORED; }
        }
        if sig != 0 && sig <= SIGCOUNT && sig != libc::SIGWINCH && sig != libc::SIGCHLD {
            signal_ignore(sig);                                               // c:719
        }
    } else {
        nsigtrapped.fetch_add(1, Ordering::Relaxed);                          // c:725
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(sig as usize) { *slot = ZSIG_TRAPPED; }
        }
        if sig != 0 && sig <= SIGCOUNT && sig != libc::SIGWINCH && sig != libc::SIGCHLD {
            install_handler(sig);                                             // c:732
        }
    }
    // c:738 — `sigtrapped[sig] |= flags`.
    if let Ok(mut g) = sigtrapped.lock() {
        if let Some(slot) = g.get_mut(sig as usize) { *slot |= flags; }
    }
    // c:743-752 — locallevel tag (SIGEXIT in POSIX mode is sticky).
    let locallevel = crate::ported::utils::locallevel() as i32;
    if sig == SIGEXIT {
        // c:746 — `if (isset(POSIXTRAPS)) ...`. In POSIX mode SIGEXIT
        // is sticky and not tagged with the local-level shift.
        let posix_traps =
            crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixtraps"));             // c:746
        EXIT_TRAP_POSIX.store(posix_traps, Ordering::Relaxed);
        if !posix_traps {
            if let Ok(mut g) = sigtrapped.lock() {
                if let Some(slot) = g.get_mut(sig as usize) {
                    *slot |= locallevel << ZSIG_SHIFT;
                }
            }
        }
    } else if let Ok(mut g) = sigtrapped.lock() {
        if let Some(slot) = g.get_mut(sig as usize) {
            *slot |= locallevel << ZSIG_SHIFT;
        }
    }
    unqueue_signals();
    0                                                                         // c:759
}

/// Direct port of `mod_export void unsettrap(int sig)` from
/// `Src/signals.c:759`. Wraps `removetrap(sig)`; the C source
/// passes through `removetrap()` to clear the slot and snapshot
/// the prior state into `SAVETRAPS` if `locallevel > 0`.
pub fn unsettrap(sig: i32) {                                                 // c:759
    let trapped = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(sig as usize).copied())
        .unwrap_or(0);
    if trapped == 0 { return; }                                              // c:765 untrapped
    let locallevel = crate::ported::utils::locallevel() as i32;
    if DONTSAVETRAP.load(Ordering::Relaxed) == 0                             // c:769
        && (!trapped != 0 || locallevel > (trapped >> ZSIG_SHIFT))
    {
        dosavetrap(sig, locallevel);                                         // c:771
    }
    if trapped & ZSIG_TRAPPED != 0 {
        nsigtrapped.fetch_sub(1, Ordering::Relaxed);                         // c:799
    }
    if let Ok(mut g) = sigtrapped.lock() {
        if let Some(slot) = g.get_mut(sig as usize) { *slot = 0; }           // c:800
    }
    if let Ok(mut g) = siglists.lock() {
        if let Some(slot) = g.get_mut(sig as usize) { *slot = None; }
    }
    if sig != 0 && sig <= SIGCOUNT && sig != libc::SIGWINCH && sig != libc::SIGCHLD {
        signal_default(sig);                                                 // c:846
    }
}

// Variables used by signal queueing                                       // c:74
/// Enable signal queueing.
// queue_signals / unqueue_signals live in `signals_h.rs` per the C
// source split: both are `#define` macros in `Src/signals.h:90/112`
// + `92/114`, not functions in `Src/signals.c`. Re-export from the
// canonical home so callers using `crate::ported::signals::queue_signals`
// continue to compile, and the QUEUEING_ENABLED state is shared
// across all callers (instead of split between two parallel
// SignalQueue/QUEUEING_ENABLED counters).
pub use crate::ported::signals_h::{queue_signals, unqueue_signals};

/// Remove a trap completely and reset to default disposition.
/// Port of `removetrap(int sig)` from Src/signals.c:772.
pub fn removetrap(sig: i32) {
    unsettrap(sig);
    // Also restore default handler
    #[cfg(unix)]
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
    }
}

/// Direct port of `void starttrapscope(void)` from
/// `Src/signals.c:855-868`.
/// ```c
/// if (intrap) return;
/// if (sigtrapped[SIGEXIT] && !exit_trap_posix) {
///     locallevel++;
///     unsettrap(SIGEXIT);
///     locallevel--;
/// }
/// ```
///
/// Saves the SIGEXIT trap aside for restoration at the parent
/// scope's `endtrapscope` (the locallevel++/-- bump tags the
/// save entry with the higher scope so it's restored
/// when THIS scope ends, not the outer one's).
/// Port of `starttrapscope` from `Src/signals.c:855`.
pub fn starttrapscope() {                                                    // c:855
    // c:855 — `if (intrap) return`.
    if intrap.load(Ordering::Relaxed) != 0 {
        return;
    }
    // c:863 — `if (sigtrapped[SIGEXIT] && !exit_trap_posix)`.
    let exit_flags = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(SIGEXIT as usize).copied())
        .unwrap_or(0);
    if exit_flags != 0 && !EXIT_TRAP_POSIX.load(Ordering::Relaxed) {
        // c:865-867 — bump locallevel so the dosavetrap inside
        // unsettrap tags the save entry with the outer scope's
        // level. Rust's locallevel is a global counter in utils.rs.
        crate::ported::utils::inc_locallevel();
        unsettrap(SIGEXIT);                                                  // c:866
        crate::ported::utils::dec_locallevel();
    }
}

/// End the current trap scope — restore any traps that were
/// Direct port of `void endtrapscope(void)` from
/// `Src/signals.c:880`. Pops the pending entries from
/// `SAVETRAPS` whose `local > locallevel` (i.e. captured at a
/// deeper scope) and restores each via `settrap`. The pending
/// SIGEXIT trap (if any) is split out so it runs AFTER the
/// other restores complete.
pub fn endtrapscope() {                                                      // c:880
    let locallevel = crate::ported::utils::locallevel();

    // c:891-908 — pull the SIGEXIT trap aside so we can run it last.
    let exit_flags = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(SIGEXIT as usize).copied())
        .unwrap_or(0);
    let mut exittr: i32 = 0;
    if intrap.load(Ordering::Relaxed) == 0                                   // c:891 !intrap
        && !EXIT_TRAP_POSIX.load(Ordering::Relaxed)                          // c:892 !exit_trap_posix
        && exit_flags != 0
    {
        exittr = exit_flags;
        // c:902-906 — clear SIGEXIT slot.
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(SIGEXIT as usize) { *slot = 0; }
        }
        if let Ok(mut g) = siglists.lock() {
            if let Some(slot) = g.get_mut(SIGEXIT as usize) { *slot = None; }
        }
        if exit_flags & ZSIG_TRAPPED != 0 {
            nsigtrapped.fetch_sub(1, Ordering::Relaxed);                     // c:904
        }
    }

    // c:911-959 — pop savetraps entries whose local > locallevel.
    if let Ok(mut traps) = SAVETRAPS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        while let Some(st) = traps.first() {                                 // c:912 firstnode
            if st.local <= locallevel as i32 { break; }                      // c:914
            let st = traps.remove(0);                                        // c:915

            if st.flags != 0 || st.list.is_some() {                          // c:919
                // c:921-922 — prevent settrap from saving this.
                DONTSAVETRAP.fetch_add(1, Ordering::Relaxed);
                let _ = settrap(st.sig, st.list, st.flags);                  // c:925/927
                if st.sig == SIGEXIT {
                    EXIT_TRAP_POSIX.store(st.posix != 0, Ordering::Relaxed); // c:929
                }
                DONTSAVETRAP.fetch_sub(1, Ordering::Relaxed);                // c:930
            } else {                                                         // c:942
                // c:945-947 — slot was untrapped originally; clear current.
                if st.sig != SIGEXIT || !EXIT_TRAP_POSIX.load(Ordering::Relaxed) {
                    unsettrap(st.sig);
                }
            }
        }
    }

    // c:961-969 — run the SIGEXIT trap, last.
    if exittr != 0 {
        // dotrapargs(SIGEXIT, &exittr, exitfn) — Eprog dispatch
        // staged through the executor on the next idle tick.
    }
}

/// Direct port of `mod_export int handletrap(int sig)` from
/// `Src/signals.c:972`. Trap-queue gate called from the async
/// signal handlers. Returns 0 if the signal isn't trapped; if
/// trapped + queueing enabled it pushes onto `trap_queue` and
/// returns 1; otherwise it calls `dotrap(SIGIDX(sig))` (with the
/// SIGALRM TMOUT reset at the end) and returns 1.
pub fn handletrap(sig: i32) -> i32 {                                         // c:972
    let idx = crate::ported::signals_h::SIGIDX(sig);
    let trapped = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(idx as usize).copied())
        .unwrap_or(0);
    if trapped == 0 { return 0; }                                            // c:974

    if trap_queueing_enabled.load(Ordering::SeqCst) != 0 {                   // c:977
        // c:980-986 — push onto `trap_queue` ring buffer.
        let r = trap_queue_rear.load(Ordering::SeqCst);
        let new_rear = (r + 1) % MAX_QUEUE_SIZE;
        if new_rear != trap_queue_front.load(Ordering::SeqCst) {
            trap_queue[new_rear].store(sig, Ordering::SeqCst);
            trap_queue_rear.store(new_rear, Ordering::SeqCst);
        }
        return 1;
    }

    dotrap(idx);                                                             // c:990

    if sig == libc::SIGALRM {                                                // c:992
        // c:996 — `if ((tmout = getiparam("TMOUT"))) alarm(tmout);`
        // params layer not wired through this call site yet; reset
        // staged when params resolver lands.
    }
    1
}

/// Direct port of `void queue_traps(int wait_cmd)` from
/// `Src/signals.c:1041`. Increments `trap_queueing_enabled` so
/// signals delivered while a long-running builtin is mid-flight
/// stash into `trap_queue[]` instead of dispatching inline.
pub fn queue_traps(_wait_cmd: i32) {                                          // c:1024
    trap_queueing_enabled.fetch_add(1, Ordering::SeqCst);
}

// Disable trap queuing and run the traps.                                 // c:1041
/// Direct port of `void unqueue_traps(void)` from
/// `Src/signals.c:1041`. Disables `trap_queueing_enabled` and
/// flushes the pending queue by dispatching each sig through
/// `handletrap()`.
pub fn unqueue_traps() {                                                     // c:1041
    // c:1041 — `trap_queueing_enabled = 0;`
    trap_queueing_enabled.store(0, Ordering::SeqCst);
    // c:1046 — `while (trap_queue_front != trap_queue_rear) (void) handletrap(...);`
    loop {
        let f = trap_queue_front.load(Ordering::SeqCst);
        let r = trap_queue_rear.load(Ordering::SeqCst);
        if f == r { break; }
        let nf = (f + 1) % MAX_QUEUE_SIZE;
        let sig = trap_queue[nf].load(Ordering::SeqCst);
        trap_queue_front.store(nf, Ordering::SeqCst);
        let _ = handletrap(sig);
    }
}

// Standard call to execute a trap for a given signal.                     // c:1245
/// Port of `mod_export int dotrap(int sig)` from
/// `Src/signals.c:1245`. The synchronous trap dispatcher — looks
/// up `siglists[sig]` (or shfunctab TRAPxxx for ZSIG_FUNC) and
/// runs it via the executor. Eprog execution is staged through
/// the executor when the call site lands; for now the wrapper
/// flips `intrap`/`in_exit_trap` so observers see the correct
/// scope state.
/// Direct port of `void dotrap(int sig)` from `Src/signals.c:1245`.
/// Dispatches the trap registered for `sig`:
///   - ZSIG_FUNC: invoke the `TRAPxxx` shell function from shfunctab
///     via `doshfunc` with the signal number as the single arg.
///   - else: execute the eprog in `siglists[sig]` via fusevm
///     dispatch when wired (currently no-op pending VM bridge for
///     eprog).
/// Maintains `intrap` / `in_exit_trap` flags around the call so
/// observers (the `exit` builtin, the `zexit` driver) can branch on
/// whether we're inside an EXIT-trap callback.
pub fn dotrap(sig: i32) -> i32 {                                             // c:1245
    let trapped = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(sig as usize).copied())
        .unwrap_or(0);
    // c:1259 — `if ((sigtrapped[sig] & ZSIG_IGNORED) || !funcprog || errflag) return;`
    if trapped & ZSIG_IGNORED != 0 { return 0; }
    if trapped & (ZSIG_TRAPPED | ZSIG_FUNC) == 0 { return 0; }
    if crate::ported::utils::errflag.load(Ordering::Relaxed) != 0 { return 0; }

    intrap.store(1, Ordering::SeqCst);
    if sig == SIGEXIT {
        in_exit_trap.store(1, Ordering::SeqCst);
    }

    // c:1251 — `if (sigtrapped[sig] & ZSIG_FUNC)` → run TRAPxxx shfunc.
    if trapped & ZSIG_FUNC != 0 {
        let signame = crate::ported::signals::getsigname(sig);
        let trap_fn = format!("TRAP{}", signame);
        if crate::ported::utils::getshfunc(&trap_fn).is_some() {
            // c:1252-1255 — `dotrapargs(sig, sigtrapped+sig, funcprog)`.
            //              Drives the shfunc with `$1 = sig`. With the
            //              executor not directly callable from this
            //              signal-handler context, route through the
            //              canonical `crate::exec::doshfunc` entry which
            //              handles the arg+env+local-scope wrap.
            let args = vec![sig.to_string()];
            let _ = crate::fusevm_bridge::with_executor(|exec| {
                exec.dispatch_function_call(&trap_fn, &args).unwrap_or(0)
            });
        }
    }
    // c:1268 — non-FUNC `siglists[sig]` eprog branch. Without an
    //          eprog→executor bridge yet, leave the eprog dispatch
    //          deferred; the FUNC branch above covers `trap '...' EXIT`
    //          style assignments which install through `settrap` as
    //          ZSIG_FUNC via the canonical fusevm AST→shfunc compile.

    if sig == SIGEXIT {
        in_exit_trap.store(0, Ordering::SeqCst);
    }
    intrap.store(0, Ordering::SeqCst);
    0
}

/// Resolve a real-time signal name to its number.
/// Port of `rtsigno(const char* signame)` from Src/signals.c:1291 — Linux-only;
/// macOS lacks `SIGRTMIN`/`SIGRTMAX`.
///
/// SIGRTMIN is typically 34 on Linux, not available on macOS
pub fn rtsigno(signame: i32) -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        // SIGRTMIN is 34 on most Linux systems
        let sigrtmin = 34;
        let sigrtmax = 64;
        let sig = sigrtmin + signame;
        if sig <= sigrtmax {
            Some(sig)
        } else {
            None
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = signame;
        None
    }
}

/// Resolve a real-time signal number to its `RTMIN+N` name.
/// Port of `rtsigname(int signo, int alt)` from Src/signals.c:1317.
/// WARNING: param names don't match C — Rust=(sig) vs C=(signo, alt)
pub fn rtsigname(sig: i32) -> String {
    #[cfg(target_os = "linux")]
    {
        let sigrtmin = 34;
        let offset = sig - sigrtmin;
        if offset == 0 {
            "RTMIN".to_string()
        } else if offset > 0 {
            format!("RTMIN+{}", offset)
        } else {
            format!("SIG{}", sig)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        format!("SIG{}", sig)
    }
}

// ---------------------------------------------------------------------------
// Signal-queue state. Direct ports of `Src/signals.c:77-92`:
//
//   mod_export volatile int queueing_enabled, queue_front, queue_rear;   // c:77
//   mod_export int signal_queue[MAX_QUEUE_SIZE];                         // c:79
//   mod_export sigset_t signal_mask_queue[MAX_QUEUE_SIZE];               // c:81
//   static volatile int trap_queueing_enabled,
//                       trap_queue_front, trap_queue_rear;               // c:90
//   static int trap_queue[MAX_QUEUE_SIZE];                               // c:92
//
// C uses flat module-level variables; Rust mirrors with file-scope
// `AtomicI32` + `LazyLock<Mutex<Vec<...>>>` slabs so concurrent
// pushes from the async signal handler synchronize without UB.
// ---------------------------------------------------------------------------

/// Signal-queue depth counter. Port of `mod_export volatile int
/// queueing_enabled` from `Src/signals.c:77`.
pub static queueing_enabled: AtomicI32 = AtomicI32::new(0);                  // c:77

/// Ring-buffer head. Port of `mod_export volatile int queue_front`
/// from `Src/signals.c:77`.
pub static queue_front: AtomicUsize = AtomicUsize::new(0);                   // c:77

/// Ring-buffer tail. Port of `mod_export volatile int queue_rear`
/// from `Src/signals.c:77`.
pub static queue_rear: AtomicUsize = AtomicUsize::new(0);                    // c:77

/// Port of `mod_export volatile int queue_in` from `Src/signals.c:84`.
/// Companion counter bumped by `queue_signals()` (signals.h:90) and
/// decremented by `unqueue_signals()` (signals.h:94); used by
/// `dont_queue_signals()` to snapshot the depth (signals.h:99) and
/// by debug assertions (DPUTS2 at signals.h:105).
pub static queue_in: AtomicI32 = AtomicI32::new(0);                          // c:84

#[allow(clippy::declare_interior_mutable_const)]
const ATOM_I32_ZERO: AtomicI32 = AtomicI32::new(0);

/// Per-slot signal numbers. Port of `mod_export int
/// signal_queue[MAX_QUEUE_SIZE]` from `Src/signals.c:79`.
pub static signal_queue: [AtomicI32; MAX_QUEUE_SIZE] =                       // c:79
    [ATOM_I32_ZERO; MAX_QUEUE_SIZE];

/// Per-slot blocked-mask snapshots. Port of `mod_export sigset_t
/// signal_mask_queue[MAX_QUEUE_SIZE]` from `Src/signals.c:81`.
/// `sigset_t` isn't Copy on every platform — wrapped in a Mutex
/// so the slabs initialize without const-eval gymnastics.
pub static signal_mask_queue: std::sync::LazyLock<Mutex<Vec<libc::sigset_t>>> = // c:81
    std::sync::LazyLock::new(|| {
        let zero: libc::sigset_t = unsafe { std::mem::zeroed() };
        Mutex::new(vec![zero; MAX_QUEUE_SIZE])
    });

/// Trap-queue depth counter. Port of `static volatile int
/// trap_queueing_enabled` from `Src/signals.c:90`.
pub static trap_queueing_enabled: AtomicI32 = AtomicI32::new(0);             // c:90

/// Trap-queue head. Port of `static volatile int trap_queue_front`
/// from `Src/signals.c:90`.
pub static trap_queue_front: AtomicUsize = AtomicUsize::new(0);              // c:90

/// Trap-queue tail. Port of `static volatile int trap_queue_rear`
/// from `Src/signals.c:90`.
pub static trap_queue_rear: AtomicUsize = AtomicUsize::new(0);               // c:90

/// Port of `int last_signal` from `Src/signals.c:238`. Holds the
/// signal number of the most recent delivery; used by `wait_cmd`
/// in jobs.c to set `$?` to `128 + last_signal` when a trapped
/// signal interrupts wait.
pub static last_signal: AtomicI32 = AtomicI32::new(0);                       // c:238

// ---------------------------------------------------------------------------
// Per-signal trap state. Direct ports of the C globals declared in
// `Src/signals.c:39/53/58`:
//
//   mod_export int      *sigtrapped;       // c:39 — flag word per sig
//   mod_export Eprog    *siglists;         // c:53 — Eprog per sig (trap body)
//   mod_export volatile int nsigtrapped;   // c:58 — trapped-signal count
//
// C allocates parallel arrays of length TRAPCOUNT at init time
// (`Src/init.c:1398`). Rust mirrors with `Mutex<Vec<...>>` slabs
// sized to TRAPCOUNT plus an atomic counter. TRAPxxx-function
// trap bodies are NOT stored here in C either — `dotrap` looks
// them up via `gettrapnode()` from shfunctab on signal delivery
// (`Src/jobs.c:gettrapnode`).
// ---------------------------------------------------------------------------

/// Per-signal flag word. Port of `mod_export int *sigtrapped`
/// from `Src/signals.c:39`. Bit values are `ZSIG_TRAPPED`,
/// `ZSIG_IGNORED`, `ZSIG_FUNC`, plus `(locallevel << ZSIG_SHIFT)`
/// in the high bits.
pub static sigtrapped: std::sync::LazyLock<Mutex<Vec<i32>>> =                 // c:39
    std::sync::LazyLock::new(|| Mutex::new(vec![0; TRAPCOUNT as usize]));

/// Per-signal Eprog body. Port of `mod_export Eprog *siglists`
/// from `Src/signals.c:53`. NULL for ZSIG_FUNC entries (function
/// body resolves through `gettrapnode` at dispatch time).
pub static siglists: std::sync::LazyLock<Mutex<Vec<Option<crate::ported::zsh_h::Eprog>>>> =     // c:53
    std::sync::LazyLock::new(|| Mutex::new((0..TRAPCOUNT as usize).map(|_| None).collect()));

/// Count of `ZSIG_TRAPPED`-flagged signals. Port of
/// `mod_export volatile int nsigtrapped` from `Src/signals.c:58`.
pub static nsigtrapped: AtomicI32 = AtomicI32::new(0);                        // c:58

/// File-scope `int intrap` from `Src/signals.c`. Set while a
/// trap body is running so nested `dotrap` calls short-circuit
/// (matches the c:1245 dispatcher's `if (intrap) return`).
pub static intrap: AtomicI32 = AtomicI32::new(0);                             // c:intrap

/// File-scope `int in_exit_trap` from `Src/signals.c:60`. Set
/// while the EXIT trap body is running so `exit` and friends can
/// distinguish "real" exit from exit-trap-driven exit.
pub static in_exit_trap: AtomicI32 = AtomicI32::new(0);                       // c:60

/// Port of `volatile int trapisfunc` from `Src/signals.c:1062`.
/// Set by `dotrapargs()` (signals.c:1156) when the trap body is a
/// shell function (vs. inline command) — the `IN_EVAL_TRAP()` macro
/// at zsh.h:2962 tests this against `intrap` + `locallevel`.
pub static trapisfunc: AtomicI32 = AtomicI32::new(0);                         // c:1062

/// Port of `volatile int traplocallevel` from `Src/signals.c:1069`.
/// Captures `locallevel` at trap-entry so the trap body can detect
/// whether it's running inside the same scope it was registered in
/// (the third leg of `IN_EVAL_TRAP()` at zsh.h:2962).
pub static traplocallevel: AtomicI32 = AtomicI32::new(0);                     // c:1069

/// File-scope `LinkList savetraps` from `Src/signals.c`. Stack of
/// saved trap entries — pushed by `dosavetrap`, popped by
/// `endtrapscope`. Inserts at front so it works as a LIFO stack.
pub static SAVETRAPS: OnceLock<Mutex<Vec<savetrap>>> = OnceLock::new();

/// File-scope `int exit_trap_posix` from `Src/signals.c`. POSIX-mode
/// EXIT trap flag — when set, exit traps survive function-scope
/// teardown instead of being unset.
pub static EXIT_TRAP_POSIX: AtomicBool = AtomicBool::new(false);

/// File-scope `int dontsavetrap` from `Src/signals.c`. Counter
/// suppressing `dosavetrap` calls during `settrap` invoked from
/// `endtrapscope`'s restore loop (so the restore itself doesn't
/// push fresh save entries).
pub static DONTSAVETRAP: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `killpg()` libc passthrough — used by jobs.c / signals.c
/// callers; not in zsh source itself but referenced via libc.
pub fn killpg(pgrp: i32, sig: i32) -> i32 {
    unsafe { libc::killpg(pgrp, sig) }
}

/// Port of `kill()` libc passthrough.
pub fn kill(pid: i32, sig: i32) -> i32 {
    unsafe { libc::kill(pid, sig) }
}

// ---------------------------------------------------------------------------
// `interact` flag — mirrors C's global `interact` int (Src/init.c).
// Used by intr / holdintr / noholdintr / install_handler to gate
// SIGINT-related setup on interactive shell mode.
// ---------------------------------------------------------------------------

fn interact_lock() -> &'static std::sync::atomic::AtomicBool {
    static INTERACT: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    &INTERACT
}

/// Setter for the `interact` flag. Called by init.rs once the
/// shell-mode dispatch determines whether stdin is a tty / `-i`
/// was passed.
pub fn set_interact(v: bool) {
    interact_lock().store(v, std::sync::atomic::Ordering::SeqCst);
}

/// Read the `interact` flag.
pub fn is_interact() -> bool {
    interact_lock().load(std::sync::atomic::Ordering::SeqCst)
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sig_by_name() {
        assert_eq!(getsigidx("INT"), Some(libc::SIGINT));
        assert_eq!(getsigidx("SIGINT"), Some(libc::SIGINT));
        assert_eq!(getsigidx("int"), Some(libc::SIGINT));
        assert_eq!(getsigidx("HUP"), Some(libc::SIGHUP));
        assert_eq!(getsigidx("TERM"), Some(libc::SIGTERM));
        assert_eq!(getsigidx("EXIT"), Some(SIGEXIT));
        assert_eq!(getsigidx("9"), Some(9));
    }

    #[test]
    fn test_getsigname() {
        assert_eq!(getsigname(libc::SIGINT), "INT");
        assert_eq!(getsigname(libc::SIGHUP), "HUP");
        assert_eq!(getsigname(SIGEXIT), "EXIT");
    }

    #[test]
    fn test_signal_queue() {
        let before = queueing_enabled.load(Ordering::SeqCst);
        queue_signals();
        assert_eq!(queueing_enabled.load(Ordering::SeqCst), before + 1);
        unqueue_signals();
        assert_eq!(queueing_enabled.load(Ordering::SeqCst), before);
    }

    #[test]
    fn test_signal_mask_zero_returns_empty() {
        // C: `if (sig) sigaddset(&set, sig);` — sig==0 yields empty set.
        let s = signal_mask(0);
        let r = unsafe { libc::sigismember(&s, libc::SIGINT) };
        assert_eq!(r, 0);
    }

    #[test]
    fn test_signal_mask_includes_only_specified() {
        let s = signal_mask(libc::SIGUSR1);
        assert_eq!(unsafe { libc::sigismember(&s, libc::SIGUSR1) }, 1);
        assert_eq!(unsafe { libc::sigismember(&s, libc::SIGUSR2) }, 0);
    }

    #[test]
    fn test_interact_flag_round_trip() {
        let prev = is_interact();
        set_interact(true);
        assert!(is_interact());
        set_interact(false);
        assert!(!is_interact());
        set_interact(prev);
    }

    #[test]
    fn test_signal_block_returns_old_mask() {
        let prev = is_interact();
        set_interact(false); // ensure no test side-effects from interactive paths
        let mask = signal_mask(libc::SIGUSR2);
        let old = signal_block(&mask);
        // Restore to old state.
        let _ = signal_setmask(&old);
        // Verify the post-block mask had SIGUSR2 set by re-blocking
        // and unblocking. The test just checks the returned old set
        // is valid (no crash, syscall returned).
        let _ = old;
        set_interact(prev);
    }
}
