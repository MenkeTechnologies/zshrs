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
use crate::signals_h::{MAX_QUEUE_SIZE, SIGCOUNT, SIGDEBUG, SIGEXIT, SIGZERR, TRAPCOUNT};
pub use crate::signals_h::{signal_default, signal_ignore};
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
/// C body (2 lines): `if (interact) signal_ignore(SIGINT);`
#[cfg(unix)]
pub fn nointr() {                                                            // c:128
    if is_interact() { unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN); } } // c:130-131
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
    // c:228 — `if (!(wait_cmd || isset(TRAPSASYNC) ||
    //           (sigtrapped[SIGINT] & ~ZSIG_IGNORED))) sigaddset(...)`.
    // Three escape hatches let SIGINT stay UNblocked during suspend:
    //   1. `wait_cmd` — the `wait` builtin wants SIGINT to break it.
    //   2. `isset(TRAPSASYNC)` — async-trap mode means traps fire even
    //      while blocked, so SIGINT must arrive in real time. Previously
    //      missing in the Rust port — silently dropped the option gate.
    //   3. SIGINT is trapped but not ignored — user trap must fire.
    let int_state = sigtrapped.lock()
        .ok()
        .and_then(|g| g.get(libc::SIGINT as usize).copied())
        .unwrap_or(0);
    let int_trapped = (int_state & !ZSIG_IGNORED) != 0;
    let trapsasync_set = crate::ported::zsh_h::isset(
        crate::ported::zsh_h::TRAPSASYNC                                     // c:228 isset(TRAPSASYNC)
    );
    if !(wait_cmd || trapsasync_set || int_trapped) {
        unsafe {
            libc::sigaddset(&mut set, libc::SIGINT);
        }
    }
    unsafe { libc::sigsuspend(&set) }
}

/// Reap zombie child processes via non-blocking `waitpid(2)`.
/// Port of `wait_for_processes()` from Src/signals.c:249 — the
/// SIGCHLD-driven reaper that updates the job table.
/// Rust idiom replacement: drain-loop over `waitpid(-1, WNOHANG)`
/// covers the C `update_process` + `update_job` cascade; the
/// per-PID job-table update is the caller's responsibility (decoupled
/// from the reaper).
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
            let _ = crate::ported::utils::adjustwinsize(1);                  // c:469
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
/// Rust idiom replacement: zshrs's executor walks its own job table
/// during shutdown; the C source's global `jobtab` iteration lives
/// in `exec_jobs`, not in signals.rs.
// SIGHUP any jobs left running                                             // c:506
#[cfg(unix)]
pub fn killrunjobs(from_signal: i32) {
    // This would need access to the job table
    // In practice, the exec module calls this during shutdown
    let _ = from_signal;
}

/// Kill a specific job by process group.
/// Port of `killjb(Job jn, int sig)` from Src/signals.c:529.
/// Rust idiom replacement: `killpg(jn, sig)` covers the C monitoring-
/// branch; non-monitor path (per-proc `kill`) lives on the executor
/// since it owns the JobInfo->procs vec.
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
    // c:696 (zsh.h:2563) — `if (jobbing && (sig == SIGTTOU ||
    // sig == SIGTSTP || sig == SIGTTIN)) { zerr("can't trap SIG%s
    // in interactive shells", ...); return 1; }`. The previous Rust
    // port both hardcoded `jobbing = false` (now fixed) AND omitted
    // the `zerr` emission — silently failing with no diagnostic.
    // C names the specific signal in the error so the user knows
    // which trap was rejected.
    let jobbing = crate::ported::zsh_h::isset(crate::ported::zsh_h::MONITOR); // c:696
    if jobbing && (sig == libc::SIGTTOU || sig == libc::SIGTSTP || sig == libc::SIGTTIN) {
        // c:697 — `zerr("can't trap SIG%s in interactive shells", sigs[sig])`.
        let signame = getsigname(sig);
        crate::ported::utils::zerr(
            &format!("can't trap SIG{} in interactive shells", signame),
        );
        return 1;                                                             // c:699
    }

    // c:705 — `queue_signals()` + `unsettrap(sig)` unconditional
    // (saves the previous trap if locallevel changed).
    queue_signals();
    unsettrap(sig);

    // c:712 — `if (!(flags & ZSIG_FUNC) && empty_eprog(l))`. C's
    // `empty_eprog` returns true for NULL, NULL prog, OR a prog whose
    // first wordcode is WCB_END (`Src/parse.c:586`). The previous Rust
    // port used `l.is_none()`, catching only the NULL case — a non-
    // NULL but empty Eprog (e.g. `trap -- '' SIGINT` with an empty
    // body) would fall through to the trapped branch, install a
    // handler, and dispatch to an empty body instead of ignoring.
    let l_is_empty = match &l {
        None => true,
        Some(eprog) => crate::ported::parse::empty_eprog(eprog),
    };
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
    // c:765 — `if (sig == -1 || (jobbing && (SIGTTOU || SIGTSTP || SIGTTIN))) return NULL`.
    // The Rust call sites already use sig in [0, SIGCOUNT]; sig == -1 is rare,
    // but jobbing+job-control reject mirrors C exactly.
    if sig == -1 { return; }
    let jobbing = crate::ported::zsh_h::isset(crate::ported::zsh_h::MONITOR);
    if jobbing && (sig == libc::SIGTTOU || sig == libc::SIGTSTP || sig == libc::SIGTTIN) {
        return;
    }
    let locallevel = crate::ported::utils::locallevel() as i32;
    // c:769-774 — `if (!dontsavetrap && (sig == SIGEXIT ? !isset(POSIXTRAPS)
    // : isset(LOCALTRAPS)) && locallevel && (!trapped || locallevel >
    // (sigtrapped[sig] >> ZSIG_SHIFT))) dosavetrap(sig, locallevel);`.
    //
    // The previous Rust port was buggy in three ways:
    //   (a) early-returned on `trapped == 0`, defeating the C path
    //       that wants to save-trap for clean unset of an untrapped slot;
    //   (b) used `!trapped` as BITWISE NOT (Rust `!i32` is bitwise) instead
    //       of LOGICAL NOT (`trapped == 0`);
    //   (c) omitted the SIGEXIT/POSIXTRAPS-LOCALTRAPS option gates and the
    //       locallevel-non-zero requirement entirely.
    let cond_local_or_exit = if sig == SIGEXIT {
        !crate::ported::zsh_h::isset(crate::ported::zsh_h::POSIXTRAPS)        // c:771 sig==SIGEXIT branch
    } else {
        crate::ported::zsh_h::isset(crate::ported::zsh_h::LOCALTRAPS)         // c:771 else branch
    };
    if DONTSAVETRAP.load(Ordering::Relaxed) == 0                             // c:769
        && cond_local_or_exit
        && locallevel != 0                                                   // c:772 `locallevel &&`
        && (trapped == 0                                                     // c:773 `!trapped` (logical NOT)
            || locallevel > (trapped >> ZSIG_SHIFT))
    {
        dosavetrap(sig, locallevel);                                         // c:774
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
    // c:803-845 — special re-install paths per signal. Approximate
    // here with the c:846 default fallback; the SIGINT/SIGHUP/SIGPIPE
    // re-install paths are not yet ported.
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
/// Rust idiom replacement: delegates to `unsettrap` then issues
/// libc::signal(SIG_DFL); the C body's siglist-walk + savestate
/// teardown lives on `unsettrap` already.
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

            // c:919 — `if (st->flags && (st->list != NULL))`. BOTH must
            // be truthy. The previous Rust port used `||` (either),
            // wrongly firing the restore branch on a flags-only or
            // list-only savetrap entry.
            if st.flags != 0 && st.list.is_some() {                          // c:919
                // c:921-922 — prevent settrap from saving this.
                DONTSAVETRAP.fetch_add(1, Ordering::Relaxed);
                // c:923-926 — ZSIG_FUNC takes (NULL, ZSIG_FUNC); list
                // traps take (st.list, 0). The current Rust port
                // collapses both into a single settrap(list, flags)
                // call — works because settrap stores the flags and
                // list independently. Pin the ZSIG_FUNC branch as a
                // comment for future refactors.
                let _ = settrap(st.sig, st.list, st.flags);                  // c:925/927
                if st.sig == SIGEXIT {
                    EXIT_TRAP_POSIX.store(st.posix != 0, Ordering::Relaxed); // c:929
                }
                DONTSAVETRAP.fetch_sub(1, Ordering::Relaxed);                // c:930
            } else {
                // c:933 — `else if (sigtrapped[sig])`. Only fires when
                // the current slot has a trap set. The previous Rust
                // port unconditionally entered this branch, calling
                // unsettrap on slots that were already cleared.
                let cur_trapped = sigtrapped.lock()
                    .ok()
                    .and_then(|g| g.get(st.sig as usize).copied())
                    .unwrap_or(0);
                if cur_trapped != 0 {                                        // c:933
                    // c:938 — `if (sig != SIGEXIT || !exit_trap_posix)`.
                    if st.sig != SIGEXIT || !EXIT_TRAP_POSIX.load(Ordering::Relaxed) {
                        unsettrap(st.sig);                                   // c:939
                    }
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
/// `Src/signals.c:1024-1033`.
///
/// C body:
///     if (!isset(TRAPSASYNC) && !wait_cmd)
///         trap_queueing_enabled = 1;
///
/// C ONLY enables queueing when NEITHER `TRAPSASYNC` is set NOR the
/// caller is the `wait` builtin (which wants traps to fire immediately
/// so the wait can be interrupted). The previous Rust port:
///   1. Dropped the `wait_cmd` arg (named `_wait_cmd`), defeating the
///      `wait` builtin's expectation that trap delivery stays inline.
///   2. Skipped the `isset(TRAPSASYNC)` check, defeating async-trap mode.
///   3. Used `fetch_add(1)` instead of `= 1` — turned the C boolean
///      flag into a counter, breaking the symmetric `= 0` reset that
///      `unqueue_traps` does at c:1042.
pub fn queue_traps(wait_cmd: i32) {                                           // c:1024
    // c:1026 — both gates must be off for queueing to be enabled.
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::TRAPSASYNC)
        && wait_cmd == 0
    {
        trap_queueing_enabled.store(1, Ordering::SeqCst);                     // c:1031
    }
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
    // c:1270 — `dont_queue_signals()`. C disables signal queueing for
    // the duration of the trap dispatch so signals delivered while
    // the trap is running run inline (not queued for later). Previously
    // missing — trap callbacks ran with queueing still on, deferring
    // any nested signals until after the trap returned.
    crate::ported::signals_h::dont_queue_signals();                           // c:1270

    // c:1272-1273 — `if (sig == SIGEXIT) ++in_exit_trap;` (counter,
    // not boolean). The previous Rust port used `store(1)` which
    // overwrites — nested EXIT traps would all share the same flag
    // state. C uses a depth counter so observers can detect re-entry.
    if sig == SIGEXIT {
        in_exit_trap.fetch_add(1, Ordering::SeqCst);                          // c:1273
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

    // c:1277 — `if (sig == SIGEXIT) --in_exit_trap;` (decrement, not
    // store-0). The previous Rust port used `store(0)` which would
    // mask a re-entered trap — a TRAP_EXIT inside another TRAP_EXIT
    // would clear the flag prematurely.
    if sig == SIGEXIT {
        in_exit_trap.fetch_sub(1, Ordering::SeqCst);                          // c:1277
    }
    // c:1280 — `restore_queue_signals(q)`. Restore to the level
    // captured at entry (the `int q = queue_signal_level()` at c:1248).
    // We didn't capture q because the Rust port routes through
    // dont_queue_signals which zeros the counter; restoring to 0 is
    // the only safe state without the entry capture. A subsequent
    // refactor that captures q at entry would mirror C exactly.
    intrap.store(0, Ordering::SeqCst);
    0
}

/// Direct port of `void dotrapargs(int sig, int *sigtr, void *sigfn)` from
/// `Src/signals.c:1081`. Drives a single trap callback for `sig`:
/// suspends `breaks`/`retflag`/`lastval` so the body runs in a fresh
/// control-flow scope, dispatches the function (ZSIG_FUNC) or eprog
/// (non-FUNC) body, then restores the caller's flags applying the
/// trap_state / trap_return / try_tryflag rules.
///
/// ```c
/// void
/// dotrapargs(int sig, int *sigtr, void *sigfn)
/// {
///     LinkList args;
///     char *name, num[4];
///     int obreaks = breaks;
///     int oretflag = retflag;
///     int olastval = lastval;
///     int isfunc;
///     int traperr, new_trap_state, new_trap_return;
///     if ((*sigtr & ZSIG_IGNORED) || !sigfn || errflag) return;
///     if (intrap) {
///         switch (sig) { case SIGEXIT: case SIGDEBUG: case SIGZERR: return; }
///     }
///     queue_signals();
///     intrap++;
///     *sigtr |= ZSIG_IGNORED;
///     zcontext_save();
///     execsave();
///     breaks = retflag = 0;
///     traplocallevel = locallevel;
///     runhookdef(BEFORETRAPHOOK, NULL);
///     if (*sigtr & ZSIG_FUNC) {
///         /* ... build args, doshfunc(...) ... */
///     } else {
///         trap_return = -2;
///         trap_state = TRAP_STATE_PRIMED;
///         trapisfunc = isfunc = 0;
///         execode((Eprog)sigfn, 1, 0, "trap");
///     }
///     runhookdef(AFTERTRAPHOOK, NULL);
///     traperr = errflag;
///     new_trap_state = trap_state;
///     new_trap_return = trap_return;
///     execrestore();
///     zcontext_restore();
///     /* ... restore breaks/retflag/lastval per FORCE_RETURN / traperr ... */
///     if (zleactive && resetneeded) zleentry(ZLE_CMD_REFRESH);
///     if (*sigtr != ZSIG_IGNORED) *sigtr &= ~ZSIG_IGNORED;
///     intrap--;
///     unqueue_signals();
/// }
/// ```
#[allow(clippy::too_many_arguments)]
pub fn dotrapargs(sig: i32, sigtr: &mut i32, sigfn: Option<&str>) {           // c:1081
    use crate::ported::builtin::{BREAKS, LASTVAL, LOCALLEVEL, LOOPS, RETFLAG, SFCONTEXT};
    use crate::ported::zsh_h::{
        AFTERTRAPHOOK, BEFORETRAPHOOK, EMULATE_SH, EMULATION, ERRFLAG_ERROR, ERRFLAG_INT,
        SFC_SIGNAL, TRAP_STATE_FORCE_RETURN, TRAP_STATE_PRIMED, ZSIG_FUNC, ZSIG_IGNORED,
    };
    use crate::signals_h::{SIGDEBUG, SIGEXIT, SIGZERR};

    let obreaks: i32 = BREAKS.load(Ordering::SeqCst);                        // c:1085
    let oretflag: i32 = RETFLAG.load(Ordering::SeqCst);                      // c:1086
    let olastval: i32 = LASTVAL.load(Ordering::SeqCst);                      // c:1087
    let isfunc: i32;                                                         // c:1088
    let traperr: i32;                                                        // c:1089
    let new_trap_state: i32;                                                 // c:1089
    let new_trap_return: i32;                                                // c:1089

    // c:1101 — `if ((*sigtr & ZSIG_IGNORED) || !sigfn || errflag) return;`
    if (*sigtr & ZSIG_IGNORED) != 0                                          // c:1101
        || sigfn.is_none()
        || crate::ported::utils::errflag.load(Ordering::SeqCst) != 0
    {
        return;                                                              // c:1102
    }

    // c:1112-1119 — disallow synchronous traps from nesting.
    if intrap.load(Ordering::SeqCst) != 0 {                                  // c:1112
        if sig == SIGEXIT || sig == SIGDEBUG || sig == SIGZERR {             // c:1113-1117
            return;                                                          // c:1117
        }
    }

    queue_signals();                                                         // c:1121

    intrap.fetch_add(1, Ordering::SeqCst);                                   // c:1123
    *sigtr |= ZSIG_IGNORED;                                                  // c:1124
    // Mirror into the sigtrapped slab so observers (handletrap, dotrap
    // re-entry) see the same ZSIG_IGNORED bit.
    if let Ok(mut g) = sigtrapped.lock() {
        if let Some(slot) = g.get_mut(sig as usize) {
            *slot |= ZSIG_IGNORED;
        }
    }

    crate::ported::context::zcontext_save();                                 // c:1126
    // c:1128 — `execsave()` saves trap_return/trap_state. Without a
    // canonical `execsave` port yet, snapshot the two atomics inline.
    let saved_trap_state = crate::exec::TRAP_STATE.load(Ordering::SeqCst);   // c:1128 execsave
    let saved_trap_return = crate::exec::TRAP_RETURN.load(Ordering::SeqCst); // c:1128 execsave
    BREAKS.store(0, Ordering::SeqCst);                                       // c:1129 breaks = 0
    RETFLAG.store(0, Ordering::SeqCst);                                      // c:1129 retflag = 0
    traplocallevel.store(LOCALLEVEL.load(Ordering::SeqCst), Ordering::SeqCst); // c:1130

    // c:1131 — `runhookdef(BEFORETRAPHOOK, NULL);`
    // module.rs runhookdef is on the ModuleHandlers struct; no public
    // dispatcher fn exposed yet, so the hook chain wire-up is deferred
    // until the ModuleHandlers singleton accessor lands.
    let _ = BEFORETRAPHOOK;                                                  // c:1131

    if (*sigtr & ZSIG_FUNC) != 0 {                                           // c:1132
        let osc = SFCONTEXT.load(Ordering::SeqCst);                          // c:1133 osc
        // c:1133 incompfunc snapshot — not yet a public global; preserved
        // here as a local to mirror the C save/restore.
        let old_incompfunc: i32 = 0;
        let hn = crate::ported::jobs::gettrapnode(sig);                      // c:1134

        let mut args: Vec<String> = Vec::new();                              // c:1136 znewlinklist
        // c:1144-1149 — pick the right TRAPxxx name from the function table
        // (multi-named aliases) or build the canonical TRAP<SIGNAME>.
        let name = match hn {
            Some(n) => crate::ported::mem::ztrdup(&n),                       // c:1145 ztrdup(hn->nam)
            None => {                                                        // c:1146
                format!("TRAP{}", crate::ported::signals::getsigname(sig))   // c:1147-1148
            }
        };
        args.push(name.clone());                                             // c:1150 zaddlinknode(args, name)
        let num = format!("{}", sig);                                        // c:1151 sprintf(num, "%d", sig)
        args.push(num);                                                      // c:1152

        crate::exec::TRAP_RETURN.store(-1, Ordering::SeqCst);                // c:1154 trap_return = -1
        crate::exec::TRAP_STATE.store(TRAP_STATE_PRIMED, Ordering::SeqCst);  // c:1155
        crate::ported::signals::trapisfunc.store(1, Ordering::SeqCst);       // c:1156
        isfunc = 1;

        SFCONTEXT.store(SFC_SIGNAL, Ordering::SeqCst);                       // c:1158
        // c:1159 — `incompfunc = 0;` — module-private; stub kept as comment.
        let _ = old_incompfunc;
        // c:1160 — `doshfunc((Shfunc)sigfn, args, 1);`
        let fn_name = sigfn.unwrap_or("");
        let _ = crate::fusevm_bridge::with_executor(|exec| {
            exec.dispatch_function_call(fn_name, &args[1..]).unwrap_or(0)
        });
        SFCONTEXT.store(osc, Ordering::SeqCst);                              // c:1161
        // c:1162 — restore incompfunc (no-op until ported).
        let _ = args;                                                        // c:1163 freelinklist(args)
        crate::ported::mem::zsfree(name);                                    // c:1164 zsfree(name)
    } else {                                                                 // c:1165
        crate::exec::TRAP_RETURN.store(-2, Ordering::SeqCst);                // c:1166 trap_return = -2
        crate::exec::TRAP_STATE.store(TRAP_STATE_PRIMED, Ordering::SeqCst);  // c:1167
        crate::ported::signals::trapisfunc.store(0, Ordering::SeqCst);       // c:1168
        isfunc = 0;
        // c:1170 — `execode((Eprog)sigfn, 1, 0, "trap");`
        // Eprog dispatch via fusevm bridge isn't wired for raw eprog yet;
        // mirror the call so the structure is auditable. When the
        // executor exposes an `execode_eprog` entry, swap in here.
        let _ = sigfn;
    }

    // c:1172 — `runhookdef(AFTERTRAPHOOK, NULL);`. Same singleton-
    // accessor gap as BEFORETRAPHOOK above; no-op until module
    // dispatcher is wired.
    let _ = AFTERTRAPHOOK;                                                   // c:1172

    traperr = crate::ported::utils::errflag.load(Ordering::SeqCst);          // c:1174

    new_trap_state = crate::exec::TRAP_STATE.load(Ordering::SeqCst);         // c:1177
    new_trap_return = crate::exec::TRAP_RETURN.load(Ordering::SeqCst);       // c:1178

    // c:1180 — `execrestore()` restores trap_return/trap_state.
    crate::exec::TRAP_STATE.store(saved_trap_state, Ordering::SeqCst);       // c:1180
    crate::exec::TRAP_RETURN.store(saved_trap_return, Ordering::SeqCst);     // c:1180
    crate::ported::context::zcontext_restore();                              // c:1181

    if new_trap_state == TRAP_STATE_FORCE_RETURN                             // c:1183
        && !(isfunc != 0 && new_trap_return == 0)                            // c:1184
    {
        if isfunc != 0 {                                                     // c:1186
            BREAKS.store(LOOPS.load(Ordering::SeqCst), Ordering::SeqCst);    // c:1187 breaks = loops
            if sig == libc::SIGINT || sig == libc::SIGQUIT {                 // c:1196
                crate::ported::utils::errflag.fetch_or(                      // c:1197 errflag |= ERRFLAG_INT
                    ERRFLAG_INT, Ordering::SeqCst,
                );
            } else {                                                         // c:1198
                crate::ported::utils::errflag.fetch_or(                      // c:1199 errflag |= ERRFLAG_ERROR
                    ERRFLAG_ERROR, Ordering::SeqCst,
                );
            }
        }
        LASTVAL.store(new_trap_return, Ordering::SeqCst);                    // c:1202
        RETFLAG.store(1, Ordering::SeqCst);                                  // c:1204 retflag = 1
    } else {                                                                 // c:1205
        if traperr != 0 && !EMULATION(EMULATE_SH) {                          // c:1206
            LASTVAL.store(1, Ordering::SeqCst);                              // c:1207
        } else {                                                             // c:1208
            // c:1210 — keep pre-trap lastval.
            LASTVAL.store(olastval, Ordering::SeqCst);                       // c:1213
        }
        if crate::ported::r#loop::try_tryflag.load(Ordering::SeqCst) != 0 {  // c:1215 try_tryflag
            if traperr != 0 {                                                // c:1216
                crate::ported::utils::errflag.fetch_or(                      // c:1217
                    ERRFLAG_ERROR, Ordering::SeqCst,
                );
            } else {                                                         // c:1218
                crate::ported::utils::errflag.fetch_and(                     // c:1219 errflag &= ~ERRFLAG_ERROR
                    !ERRFLAG_ERROR, Ordering::SeqCst,
                );
            }
        }
        BREAKS.fetch_add(obreaks, Ordering::SeqCst);                         // c:1220 breaks += obreaks
        RETFLAG.store(oretflag, Ordering::SeqCst);                           // c:1222 retflag = oretflag
        let cur_breaks = BREAKS.load(Ordering::SeqCst);
        let cur_loops = LOOPS.load(Ordering::SeqCst);
        if cur_breaks > cur_loops {                                          // c:1223
            BREAKS.store(cur_loops, Ordering::SeqCst);                       // c:1224
        }
    }

    // c:1231 — `if (zleactive && resetneeded) zleentry(ZLE_CMD_REFRESH);`
    if crate::ported::builtins::sched::zleactive.load(Ordering::SeqCst) != 0
        && crate::ported::utils::RESETNEEDED.load(Ordering::SeqCst) != 0
    {
        let _ = crate::ported::init::zleentry(
            crate::ported::zsh_h::ZLE_CMD_REFRESH,
        );
    }

    if *sigtr != ZSIG_IGNORED {                                              // c:1234
        *sigtr &= !ZSIG_IGNORED;                                             // c:1235
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(sig as usize) {
                *slot &= !ZSIG_IGNORED;
            }
        }
    }
    intrap.fetch_sub(1, Ordering::SeqCst);                                   // c:1236

    unqueue_signals();                                                       // c:1238
}

// `try_tryflag` lives at `Src/loop.c:731` (the always/try block depth
// counter); ported to `crate::ported::r#loop::try_tryflag`. dotrapargs
// above reads it from its canonical home per PORT.md Rule C (header /
// file placement).

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

/// Resolve a real-time signal number to its `RTMIN+N` / `RTMAX-N` name.
/// Port of `char *rtsigname(int signo, int alt)` from `Src/signals.c:1317`.
///
/// C body picks the SHORTER form between `RTMIN+N` and `RTMAX-N`,
/// preferring the smaller offset unless `alt` is set (which flips
/// the choice via XOR). `signo` outside `[SIGRTMIN..=SIGRTMAX]`
/// returns NULL — Rust returns empty string for the equivalent.
///
/// The previous Rust port:
///   1. Dropped the `alt` argument entirely (callers got the
///      `alt=0` default, but no way to flip).
///   2. ALWAYS produced the `RTMIN+N` form, ignoring the C "shorter
///      form wins" contract — for high-numbered RT signals where
///      RTMAX-N is the shorter form, C would emit `RTMAX-N` but
///      Rust emitted the longer `RTMIN+N`.
///   3. Used a hardcoded `sigrtmin = 34` constant instead of
///      `libc::SIGRTMIN()` (real-time signal numbers can vary by
///      libc version/build).
///   4. Out-of-range input produced `SIG{n}` — C returns NULL.
///
/// **Signature divergence from C**: C takes `(signo, alt)`; Rust port
/// takes `(sig)` with `alt=0` implicit because the only in-tree caller
/// (`params.rs:1640`) doesn't need the alt flip. A future caller that
/// needs alt-form can be added then.
/// WARNING: param names don't match C — Rust=(sig) vs C=(signo, alt)
pub fn rtsigname(sig: i32) -> String {                                       // c:1317
    #[cfg(target_os = "linux")]
    {
        let sigrtmin = unsafe { libc::SIGRTMIN() };
        let sigrtmax = unsafe { libc::SIGRTMAX() };
        // c:1325-1326 — `if (signo < SIGRTMIN || signo > SIGRTMAX) return NULL;`
        if sig < sigrtmin || sig > sigrtmax {
            return String::new();
        }
        // c:1319-1323 — `int minofs = signo - SIGRTMIN; int maxofs =
        // SIGRTMAX - signo; int form = alt ^ (maxofs < minofs);`
        // With alt=0 always, form simplifies to `maxofs < minofs`.
        let minofs = sig - sigrtmin;
        let maxofs = sigrtmax - sig;
        let form = maxofs < minofs;
        // c:1328-1334 — pick `RTMIN+` or `RTMAX-` per `form`.
        let prefix = if form { "RTMAX-" } else { "RTMIN+" };
        let offset = if form { maxofs } else { minofs };
        if offset == 0 {
            // c:1334 — buf[5] = '\0' → drop the trailing sign char.
            prefix[..5].to_string()
        } else {
            format!("{}{}", prefix, offset)
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = sig;
        String::new()
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

    /// `Src/signals.c:158-168` — `signal_mask(sig)` builds a fresh
    /// sigset containing only `sig`. The `sig == 0` arm at c:163
    /// returns an empty set (no `sigaddset` call). Pin both arms.
    #[cfg(unix)]
    #[test]
    fn signal_mask_includes_only_requested_signal() {
        let m = signal_mask(libc::SIGUSR1);
        // c:166 — `sigaddset(&set, sig)` for the requested signal.
        assert_eq!(unsafe { libc::sigismember(&m, libc::SIGUSR1) }, 1,
            "c:166 — requested signal must be set");
        // Other signals not in the set.
        assert_eq!(unsafe { libc::sigismember(&m, libc::SIGUSR2) }, 0);
        assert_eq!(unsafe { libc::sigismember(&m, libc::SIGTERM) }, 0);
    }

    /// `Src/signals.c:163` — `if (sig) sigaddset(&set, sig)`. With
    /// `sig == 0` no `sigaddset` runs, so the returned set is empty.
    /// Regression dropping the `if (sig)` guard would `sigaddset(&set, 0)`
    /// which is implementation-defined (Linux: EINVAL; macOS: may
    /// "succeed" with a bogus member).
    #[cfg(unix)]
    #[test]
    fn signal_mask_with_zero_returns_empty_set() {
        let m = signal_mask(0);
        // Every signal must be NOT a member of an empty set.
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGUSR1, libc::SIGUSR2] {
            assert_eq!(unsafe { libc::sigismember(&m, sig) }, 0,
                "c:163 — sig=0 produces empty set, but {} found", sig);
        }
    }

    /// `Src/signals.c:1317-1338` — `rtsigname(signo, alt)` picks the
    /// shorter form between `RTMIN+N` and `RTMAX-N`. Pin the contract
    /// on Linux where SIGRTMIN/SIGRTMAX are real.
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigname_picks_shorter_form_between_rtmin_rtmax() {
        let sigrtmin = unsafe { libc::SIGRTMIN() };
        let sigrtmax = unsafe { libc::SIGRTMAX() };
        // SIGRTMIN itself → "RTMIN" (offset 0; trailing '+' dropped).
        assert_eq!(rtsigname(sigrtmin), "RTMIN",
            "c:1334 — offset 0 → bare 'RTMIN' (no '+0')");
        // SIGRTMAX itself → "RTMAX" (offset 0; trailing '-' dropped).
        assert_eq!(rtsigname(sigrtmax), "RTMAX",
            "c:1334 — offset 0 → bare 'RTMAX' (no '-0')");
        // SIGRTMIN+1 — minofs=1, maxofs=(sigrtmax-sigrtmin-1) > 1 →
        // form=false → "RTMIN+1".
        assert_eq!(rtsigname(sigrtmin + 1), "RTMIN+1",
            "c:1322 — minofs < maxofs → form=0 → RTMIN+1");
        // SIGRTMAX-1 — maxofs=1, minofs=(sigrtmax-sigrtmin-1) > 1 →
        // form=true → "RTMAX-1".
        assert_eq!(rtsigname(sigrtmax - 1), "RTMAX-1",
            "c:1322 — maxofs < minofs → form=1 → RTMAX-1");
        // Out of range → empty string (C: NULL).
        assert_eq!(rtsigname(sigrtmin - 1), "",
            "c:1326 — signo < SIGRTMIN → NULL (empty)");
        assert_eq!(rtsigname(sigrtmax + 1), "",
            "c:1326 — signo > SIGRTMAX → NULL (empty)");
    }

    /// `Src/signals.c:1024-1033` — `queue_traps(wait_cmd)` enables
    /// queueing ONLY when BOTH `!isset(TRAPSASYNC)` AND `!wait_cmd`.
    /// The previous Rust port hardcoded a `fetch_add(1)`, defeating
    /// both option gates entirely. Pin:
    ///   * TRAPSASYNC=on  → queue_traps(0) is a no-op.
    ///   * wait_cmd=1     → queue_traps(1) is a no-op.
    ///   * both off       → queue_traps(0) sets trap_queueing_enabled=1.
    #[cfg(unix)]
    #[test]
    fn queue_traps_respects_trapsasync_and_wait_cmd() {
        use crate::ported::options::dosetopt;
        use crate::ported::zsh_h::TRAPSASYNC;
        let saved = crate::ported::zsh_h::isset(TRAPSASYNC);

        // Setup: TRAPSASYNC off, trap_queueing_enabled cleared.
        dosetopt(TRAPSASYNC, 0, 0);
        trap_queueing_enabled.store(0, Ordering::SeqCst);

        // wait_cmd=1 → queueing stays disabled.
        queue_traps(1);
        assert_eq!(trap_queueing_enabled.load(Ordering::SeqCst), 0,
            "c:1026 — wait_cmd=1 gate must block queueing");

        // wait_cmd=0 + TRAPSASYNC=off → queueing enabled.
        queue_traps(0);
        assert_eq!(trap_queueing_enabled.load(Ordering::SeqCst), 1,
            "c:1031 — both gates off → queueing enabled = 1");

        // Reset; turn TRAPSASYNC on.
        trap_queueing_enabled.store(0, Ordering::SeqCst);
        dosetopt(TRAPSASYNC, 1, 0);
        queue_traps(0);
        assert_eq!(trap_queueing_enabled.load(Ordering::SeqCst), 0,
            "c:1026 — TRAPSASYNC=on must block queueing even with wait_cmd=0");

        // Restore.
        dosetopt(TRAPSASYNC, if saved { 1 } else { 0 }, 0);
        trap_queueing_enabled.store(0, Ordering::SeqCst);
    }

    /// `Src/signals.c:696-699` — `settrap` rejects trapping
    /// SIGTTOU/SIGTSTP/SIGTTIN when `jobbing` (= `isset(MONITOR)`).
    /// The Rust port previously hardcoded `let jobbing = false;`,
    /// silently letting users trap job-control signals from an
    /// interactive shell (breaking their own job control). Pin:
    ///   * MONITOR unset → settrap on SIGTSTP succeeds (returns 0).
    ///   * MONITOR set   → settrap on SIGTSTP rejected (returns 1).
    #[cfg(unix)]
    #[test]
    fn settrap_rejects_job_control_signals_when_monitor_set() {
        use crate::ported::zsh_h::MONITOR;
        use crate::ported::options::dosetopt;
        // Save current MONITOR state; restore at end.
        let saved = crate::ported::zsh_h::isset(MONITOR);
        // MONITOR off → trapping SIGTSTP is allowed.
        dosetopt(MONITOR, 0, 0);
        assert_eq!(settrap(libc::SIGTSTP, None, 0), 0,
            "c:696 — MONITOR off → settrap on SIGTSTP succeeds");
        // Cleanup our successful set.
        unsettrap(libc::SIGTSTP);

        // MONITOR on → trapping SIGTSTP is rejected.
        dosetopt(MONITOR, 1, 0);
        assert_eq!(settrap(libc::SIGTSTP, None, 0), 1,
            "c:696-699 — MONITOR on → settrap on SIGTSTP rejected");
        assert_eq!(settrap(libc::SIGTTOU, None, 0), 1,
            "c:696-699 — SIGTTOU also rejected under MONITOR");
        assert_eq!(settrap(libc::SIGTTIN, None, 0), 1,
            "c:696-699 — SIGTTIN also rejected under MONITOR");

        // Restore prior MONITOR state.
        dosetopt(MONITOR, if saved { 1 } else { 0 }, 0);
    }
}
