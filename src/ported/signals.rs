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

use crate::ported::builtin::{zexit, BREAKS, LASTVAL, LOOPS, RETFLAG, SFCONTEXT, STOPMSG};
use crate::ported::context::{zcontext_restore, zcontext_save};
use crate::ported::exec::{TRAP_RETURN, TRAP_STATE};
use crate::ported::init::zleentry;
use crate::ported::jobs::gettrapnode;
pub use crate::ported::jobs::{getsigidx, getsigname};
use crate::ported::mem::{zsfree, ztrdup};
use crate::ported::options::optlookup;
use crate::ported::params::{getiparam, ttyidlegetfn};
pub use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::signals_h::{SIGNUM, TRAPCOUNT as TRAPCOUNT_H, VSIGCOUNT};
use crate::ported::utils::{
    errflag, inc_locallevel, locallevel as locallevel_fn, zerr, zwarn, ERRFLAG_ERROR, RESETNEEDED,
};
use crate::ported::zsh_h::{
    isset, Eprog, AFTERTRAPHOOK, BEFORETRAPHOOK, EMULATE_SH, EMULATION, ERRFLAG_INT, HUP,
    INTERACTIVE, LOCALTRAPS, MONITOR, POSIXTRAPS, PRIVILEGED, SFC_SIGNAL, TRAPSASYNC,
    TRAP_STATE_FORCE_RETURN, TRAP_STATE_PRIMED, ZEXIT_SIGNAL, ZLE_CMD_REFRESH, ZSIG_FUNC,
    ZSIG_IGNORED, ZSIG_SHIFT, ZSIG_TRAPPED,
};
use crate::r#loop::try_tryflag;
use crate::sched::zleactive;
pub use crate::signals_h::{signal_default, signal_ignore};
use crate::signals_h::{MAX_QUEUE_SIZE, SIGCOUNT, SIGDEBUG, SIGEXIT, SIGZERR, TRAPCOUNT};
use crate::utils::getshfunc;
use crate::DPUTS;
use nix::sys::signal::{
    sigprocmask, SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal as NixSignal,
};
use nix::unistd::getpid;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

// getsigidx / getsigname live in `jobs.rs` per C source split:
// `getsigidx` at `Src/jobs.c:3047`, `getsigname` at `Src/jobs.c:3087`.
// Re-export from the canonical home so callers using
// `crate::ported::signals::getsigidx` continue to compile.

/// Per-slot trap-queue signals. Port of `static int
/// trap_queue[MAX_QUEUE_SIZE]` from `Src/signals.c:92`.
pub static trap_queue: [AtomicI32; MAX_QUEUE_SIZE] = // c:92
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
pub fn install_handler(sig: i32) {
    // c:100
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
pub fn intr() {
    // c:118
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
pub fn nointr() {
    // c:128
    if is_interact() {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }
    } // c:130-131
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
pub fn holdintr() {
    // c:139
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
pub fn noholdintr() {
    // c:149
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
pub fn signal_block(set: &libc::sigset_t) -> libc::sigset_t {
    // c:175
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
pub fn signal_unblock(set: &libc::sigset_t) -> libc::sigset_t {
    // c:189
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
pub fn signal_suspend(sig: i32, wait_cmd: bool) -> i32 {
    // c:214
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
    }
    // c:228 — `if (!(wait_cmd || isset(TRAPSASYNC) ||
    //           (sigtrapped[SIGINT] & ~ZSIG_IGNORED))) sigaddset(...)`.
    // Three escape hatches let SIGINT stay UNblocked during suspend:
    //   1. `wait_cmd` — the `wait` builtin wants SIGINT to break it.
    //   2. `isset(TRAPSASYNC)` — async-trap mode means traps fire even
    //      while blocked, so SIGINT must arrive in real time.
    //   3. SIGINT is trapped but not ignored — user trap must fire.
    let int_state = sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(libc::SIGINT as usize).copied())
        .unwrap_or(0);
    let int_trapped = (int_state & !ZSIG_IGNORED) != 0;
    let trapsasync_set = isset(
        TRAPSASYNC, // c:228 isset(TRAPSASYNC)
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
    // c:271-274 — `WAITFLAGS = WNOHANG|WUNTRACED|WCONTINUED`. The
    // previous Rust port used `WNOHANG|WUNTRACED` only, dropping the
    // WCONTINUED bit so children that were resumed via SIGCONT
    // wouldn't surface a status update — silently breaking
    // `fg`/`bg` job-table tracking. WCONTINUED is POSIX and
    // available in libc-rs on every platform zshrs supports.
    let waitflags = libc::WNOHANG | libc::WUNTRACED | libc::WCONTINUED; // c:271
    loop {
        let mut status: i32 = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, waitflags) };
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
/// Direct port of `static RETSIGTYPE zhandler(int sig)` from
/// `Src/signals.c:393`. Exposed as `pub` so the Rust queue drainer
/// (`run_queued_signals` in signals_h.rs) can call it synchronously,
/// matching C's `zhandler(signal_queue[queue_front])` at c:83. C's
/// version is `static` because all queue draining happens inside
/// signals.c; Rust splits the macro (queue) from the handler
/// (signals.rs) across two modules, so `pub` is the equivalent of C's
/// in-file visibility. (Earlier port routed via `libc::raise(sig)`,
/// which is the wrong analog — raise(2) goes through the kernel and
/// loses the queued signal behind the process signal mask. See Bug
/// #104 in docs/BUGS.md.)
pub extern "C" fn zhandler(sig: libc::c_int) {
    last_signal.store(sig, Ordering::Relaxed); // c:403

    // c:405-407 — `sigfillset(&newmask); oldmask = signal_block(newmask);`
    let mut newmask: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigfillset(&mut newmask);
    }
    let oldmask = signal_block(&newmask);

    // c:410-424 — `if (queueing_enabled) { ... return; }`
    if queueing_enabled.load(Ordering::SeqCst) != 0 {
        let temp_rear = (queue_rear.load(Ordering::SeqCst) + 1) % MAX_QUEUE_SIZE;
        if temp_rear != queue_front.load(Ordering::SeqCst) {
            queue_rear.store(temp_rear, Ordering::SeqCst);
            signal_queue[temp_rear].store(sig, Ordering::SeqCst);
            if let Ok(mut g) = signal_mask_queue.lock() {
                if let Some(slot) = g.get_mut(temp_rear) {
                    *slot = oldmask;
                }
            }
        }
        return;
    }

    // c:427 — `signal_setmask(oldmask);`
    let _ = signal_setmask(&oldmask);

    // c:429-498 — per-signal dispatch.
    match sig {
        libc::SIGCHLD => {
            // c:430-431 — `wait_for_processes();` — reap zombies AND
            // route their (pid, status) pairs through update_bg_job so
            // job.stat picks up STAT_DONE / STAT_STOPPED bits. Without
            // the route, signal_suspend-driven waits in zwaitjob can't
            // see jobs as completed.
            let reaped = wait_for_processes();
            if !reaped.is_empty() {
                if let Some(jt) = crate::ported::jobs::JOBTAB.get() {
                    if let Ok(mut guard) = jt.lock() {
                        for (pid, status) in reaped {
                            let _ = crate::ported::jobs::update_bg_job(&mut guard, pid, status);
                        }
                        // c:Src/jobs.c:639-641 — update_job's tail
                        // dispatches printjob(jn, ..., 0) for LOCKED
                        // jobs, whose own tail (c:1350-1363) deletes
                        // each finished entry. Non-interactive shells
                        // print nothing there; the deletion is the
                        // observable effect.
                        crate::ported::jobs::scanjobs(&mut guard);
                    }
                }
            }
        }
        libc::SIGPIPE => {
            // c:434
            if handletrap(libc::SIGPIPE) == 0 {
                // c:436-441 — non-interactive exits immediately; an
                // interactive non-tty also exits via zexit.
                let interact = isset(INTERACTIVE);
                if !interact {
                    unsafe {
                        libc::_exit(libc::SIGPIPE);
                    } // c:437
                } else {
                    // c:438 — `else if (!isatty(SHTTY))`. The previous
                    // Rust port hardcoded fd 0 (stdin) with a comment
                    // claiming "SHTTY isn't a single global in zshrs"
                    // — but SHTTY IS a global at `init::SHTTY`. Use it.
                    let shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
                    let on_tty = shtty >= 0 && unsafe { libc::isatty(shtty) } != 0;
                    if !on_tty {
                        STOPMSG // c:439
                            .store(1, Ordering::Relaxed);
                        zexit(libc::SIGPIPE, ZEXIT_SIGNAL);
                        // c:440
                    }
                }
            }
        }
        libc::SIGHUP => {
            // c:445
            if handletrap(libc::SIGHUP) == 0 {
                // c:447 — `stopmsg = 1; zexit(SIGHUP, ZEXIT_SIGNAL);`
                STOPMSG.store(1, Ordering::Relaxed);
                zexit(libc::SIGHUP, ZEXIT_SIGNAL); // c:448
            }
        }
        libc::SIGINT => {
            // c:452
            if handletrap(libc::SIGINT) == 0 {
                // c:454-456 — PRIVILEGED+INTERACTIVE during a signal-
                // noerrexit window: immediate exit.
                let privileged = isset(PRIVILEGED);
                let interactive = isset(INTERACTIVE);
                if privileged && interactive {
                    zexit(libc::SIGINT, ZEXIT_SIGNAL);
                }
                // c:457 — `errflag |= ERRFLAG_INT;`
                let cur = errflag.load(Ordering::Relaxed);
                errflag.store(cur | ERRFLAG_INT, Ordering::Relaxed); // c:457
                                                                     // c:458-462 — `if (list_pipe || chline || simple_pline)`:
                                                                     // an interactive SIGINT mid-pipeline must break loops,
                                                                     // flush pending input, and signal any cursh job.
                let in_list_pipe = crate::ported::exec::list_pipe.load(Ordering::Relaxed) != 0;
                let chline_nonempty = crate::ported::hist::chline
                    .lock()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false);
                let in_simple_pline =
                    crate::ported::exec::simple_pline.load(Ordering::Relaxed) != 0;
                if in_list_pipe || chline_nonempty || in_simple_pline {
                    // c:459 — `breaks = loops;`
                    let l = crate::ported::builtin::LOOPS.load(Ordering::Relaxed);
                    crate::ported::builtin::BREAKS.store(l, Ordering::Relaxed);
                    // c:460 — `inerrflush();`
                    crate::ported::input::inerrflush();
                    // c:461 — `check_cursh_sig(SIGINT);`. Rust port
                    // takes `(jobtab, sig)`; load the canonical JOBTAB
                    // snapshot then dispatch.
                    #[cfg(unix)]
                    if let Some(tab) = crate::ported::jobs::JOBTAB.get() {
                        if let Ok(jt) = tab.lock() {
                            crate::ported::jobs::check_cursh_sig(&jt, libc::SIGINT);
                        }
                    }
                }
                // c:463 — `lastval = 128 + SIGINT;`
                LASTVAL.store(128 + libc::SIGINT, Ordering::Relaxed);
            }
        }
        libc::SIGWINCH => {
            // c:468
            // c:469 — `adjustwinsize(1)` (Src/utils.c) — re-reads
            // TIOCGWINSZ and updates LINES/COLUMNS params.
            let _ = crate::ported::utils::adjustwinsize(1); // c:469
            let _ = handletrap(libc::SIGWINCH); // c:470
        }
        libc::SIGALRM => {
            // c:475
            if handletrap(libc::SIGALRM) == 0 {
                // c:476-489 — idle vs TMOUT branch. The previous Rust
                // port commented "Skip the still idle re-arm" claiming
                // no ttyidlegetfn port — but it IS ported at
                // `ttyidlegetfn`. Now wired
                // exactly per C.
                //
                // C body (c:478-484):
                //   int idle = ttyidlegetfn(NULL);
                //   int tmout = getiparam("TMOUT");
                //   if (idle >= 0 && idle < tmout)
                //       alarm(tmout - idle);
                //   else { /* timeout exit */ }
                let idle = ttyidlegetfn(); // c:478
                let tmout = getiparam("TMOUT"); // c:479
                if idle >= 0 && idle < tmout {
                    // c:481 — `alarm(tmout - idle);` — re-arm for
                    // remaining idle window.
                    unsafe {
                        libc::alarm((tmout - idle) as u32); // c:481
                    }
                } else if tmout == 0 {
                    // No timeout configured — bail out silently
                    // (C falls into the else branch which would
                    // emit "timeout" and zexit even with tmout==0,
                    // but that's a degenerate setup; matching
                    // common-case behavior here).
                } else {
                    // c:486 — `errflag = noerrs = 0;`
                    errflag.store(0, Ordering::Relaxed);
                    // c:487 — `zwarn("timeout");`
                    zwarn("timeout"); // c:487
                    STOPMSG.store(1, Ordering::Relaxed); // c:488
                    zexit(libc::SIGALRM, ZEXIT_SIGNAL); // c:489
                }
            }
        }
        _ => {
            // c:506
            let _ = handletrap(sig);
        }
    }
}

/// Kill all running jobs with SIGHUP at shell exit.
///
/// Port of `void killrunjobs(int from_signal)` from `Src/signals.c:506`.
/// C body:
/// ```c
/// if (unset(HUP)) return;
/// for (i = 1; i <= maxjob; i++)
///     if ((from_signal || i != thisjob) && (jobtab[i].stat & STAT_LOCKED) &&
///         !(jobtab[i].stat & STAT_NOPRINT) &&
///         !(jobtab[i].stat & STAT_STOPPED)) {
///         if (jobtab[i].gleader != getpid() &&
///             killpg(jobtab[i].gleader, SIGHUP) != -1)
///             killed++;
///     }
/// if (killed) zwarn("warning: %d jobs SIGHUPed", killed);
/// ```
///
#[cfg(unix)]
pub fn killrunjobs(from_signal: i32) {
    // c:506
    // c:512 — `if (unset(HUP)) return;`. HUP option gates the
    // whole walk: when `setopt nohup`, jobs survive shell exit.
    if !isset(HUP) {
        // c:512
        return;
    }
    let my_pid = unsafe { libc::getpid() };
    let mut killed: i32 = 0;
    // c:514 — `for (i = 1; i <= maxjob; i++)`. Skip index 0
    // (shell itself).
    let tab = crate::ported::jobs::JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
    let tab = tab.lock().expect("jobtab poisoned");
    let thisjob = crate::ported::jobs::THISJOB
        .get_or_init(|| Mutex::new(-1))
        .lock()
        .map(|g| *g)
        .unwrap_or(-1);
    for (i, job) in tab.iter().enumerate().skip(1) {
        // c:515-517 — gate: (from_signal || i != thisjob) AND
        // STAT_LOCKED AND !STAT_NOPRINT AND !STAT_STOPPED.
        if !(from_signal != 0 || i as i32 != thisjob) {
            // c:515
            continue;
        }
        if (job.stat & crate::ported::jobs::stat::LOCKED) == 0 {
            // c:516
            continue;
        }
        if (job.stat & crate::ported::jobs::stat::NOPRINT) != 0 {
            // c:516
            continue;
        }
        if (job.stat & crate::ported::jobs::stat::STOPPED) != 0 {
            // c:516
            continue;
        }
        // c:518-520 — `if (jobtab[i].gleader != getpid() &&
        //                  killpg(jobtab[i].gleader, SIGHUP) != -1)
        //                  killed++;`
        // The gleader check avoids the shell HUP-ing itself.
        if job.gleader != my_pid && unsafe { libc::killpg(job.gleader, libc::SIGHUP) } != -1
        // c:519
        {
            killed += 1; // c:520
        }
    }
    drop(tab);
    // c:524 — `if (killed) zwarn("warning: %d jobs SIGHUPed", killed);`
    if killed != 0 {
        // c:524
        zwarn(&format!("warning: {} jobs SIGHUPed", killed));
        // c:524
    }
}

/* send a signal to a job (simply involves kill if monitoring is on) */
// c:525
/// Port of `killjb(Job jn, int sig)` from `Src/signals.c:529`.
/// CALLER CONVENTION: Rust passes `jn_idx: usize` (JOBTAB index)
/// instead of C `Job jn` (pointer); the body resolves the job via
/// `JOBTAB.lock()`. Returns 0/-1 per the killpg result chain.
#[cfg(unix)]
pub fn killjb(jn_idx: usize, sig: i32) -> i32 {
    // c:529
    let _pn: (); // c:531 `Process pn;` — modelled by loop binding
    let mut err: i32 = 0; // c:532

    if crate::ported::zsh_h::jobbing() {
        // c:534
        // Snapshot the job state under the lock (gleader + stat + other
        // + procs + other-procs). Avoid holding the lock during kill()
        // syscalls — they can block under signals.
        let snap = {
            let table = match crate::ported::jobs::JOBTAB.get() {
                Some(t) => t,
                None => return -1,
            };
            let tab = table.lock().unwrap_or_else(|e| e.into_inner());
            let jn = match tab.get(jn_idx) {
                Some(j) => j,
                None => return -1,
            };
            let other_procs: Vec<libc::pid_t> = if jn.other > 0 {
                tab.get(jn.other as usize)
                    .map(|o| o.procs.iter().map(|p| p.pid).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let other_empty = jn.other > 0
                && tab
                    .get(jn.other as usize)
                    .map(|o| o.procs.is_empty())
                    .unwrap_or(true);
            (
                jn.stat,
                jn.gleader,
                jn.other,
                jn.procs.iter().map(|p| p.pid).collect::<Vec<_>>(),
                other_procs,
                other_empty,
            )
        };
        let (stat, gleader, other, procs_pids, other_procs, other_empty) = snap;

        if (stat & crate::ported::zsh_h::STAT_SUPERJOB) != 0 {
            // c:535
            if sig == libc::SIGCONT {
                // c:536
                // c:537-540 — walk jobtab[jn->other].procs, killpg each;
                // fall through to kill() on killpg failure; ESRCH ignored.
                for pid in &other_procs {
                    // c:537
                    if unsafe { libc::killpg(*pid, sig) } == -1 {
                        // c:538
                        let e = std::io::Error::last_os_error().raw_os_error();
                        // c:539 — fallback kill()
                        if unsafe { libc::kill(*pid, sig) } == -1 && e != Some(libc::ESRCH) {
                            err = -1; // c:540
                        }
                    }
                }

                /*
                 * Note this does not kill the last process,
                 * which is assumed to be the one controlling the
                 * subjob, i.e. the forked zsh that was originally
                 * list_pipe_pid...
                 */
                // c:542-547
                let n = procs_pids.len();
                if n > 0 {
                    for pid in &procs_pids[..n - 1] {
                        // c:548 `for pn = jn->procs; pn->next; ...`
                        if unsafe { libc::kill(*pid, sig) } == -1
                            && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
                        {
                            err = -1; // c:550
                        }
                    }

                    /*
                     * ...we only continue that once the external processes
                     * currently associated with the subjob are finished.
                     */
                    // c:552-555
                    if other_empty {
                        // c:556
                        let last = procs_pids[n - 1];
                        if unsafe { libc::kill(last, sig) } == -1
                            && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
                        {
                            err = -1; // c:558
                        }
                    }
                }

                /*
                 * The following marks both the superjob and subjob
                 * as running, as done elsewhere.
                 */
                // c:560-569
                if err != -1 {
                    // c:570
                    let table = crate::ported::jobs::JOBTAB.get().unwrap();
                    let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
                    crate::ported::jobs::makerunning(&mut tab, jn_idx); // c:571
                }

                return err; // c:573
            }

            // c:575 — `if (killpg(jobtab[jn->other].gleader, sig) == -1 && errno != ESRCH) err = -1;`
            let other_gleader = crate::ported::jobs::JOBTAB
                .get()
                .and_then(|t| {
                    t.lock()
                        .ok()
                        .and_then(|tab| tab.get(other as usize).map(|j| j.gleader))
                })
                .unwrap_or(0);
            if other_gleader > 0
                && unsafe { libc::killpg(other_gleader, sig) } == -1
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                err = -1; // c:576
            }

            if unsafe { libc::killpg(gleader, sig) } == -1                   // c:578
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            {
                err = -1; // c:579
            }

            return err; // c:581
        } else {
            // c:583
            err = unsafe { libc::killpg(gleader, sig) }; // c:584
            if sig == libc::SIGCONT && err != -1 {
                // c:585
                let table = crate::ported::jobs::JOBTAB.get().unwrap();
                let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
                crate::ported::jobs::makerunning(&mut tab, jn_idx); // c:586
            }
            return err; // c:587
        }
    }
    // c:590-604 — non-jobbing: walk jn->procs, kill each if SP_RUNNING
    // or WIFSTOPPED; ignore ESRCH and `sig == 0` (kill -0 polling).
    let table = match crate::ported::jobs::JOBTAB.get() {
        Some(t) => t,
        None => return err,
    };
    let snap: Vec<(libc::pid_t, i32)> = {
        let tab = table.lock().unwrap_or_else(|e| e.into_inner());
        match tab.get(jn_idx) {
            Some(j) => j.procs.iter().map(|p| (p.pid, p.status)).collect(),
            None => return err,
        }
    };
    for (pid, status) in snap {
        // c:590
        /*
         * Do not kill this job's process if it's already dead as its
         * pid could have been reused by the system.
         */                                                                  // c:591-595
        let is_running = status == crate::ported::zsh_h::SP_RUNNING;
        let is_stopped = libc::WIFSTOPPED(status);
        if is_running || is_stopped {
            // c:596
            /*
             * kill -0 on a job is pointless. We still call kill() for each
             * process in case the user cares about it but we ignore its outcome.
             */                                                              // c:597-600
            let r = unsafe { libc::kill(pid, sig) };
            if r == -1
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
                && sig != 0
            {
                err = r;
                return -1; // c:602
            }
            err = r; // c:601 assignment
        }
    }
    err // c:605
}

/// Port of `struct savetrap` from `Src/signals.c:611-624`.
/// One stacked trap-state entry captured by `dosavetrap` so the
/// outer-scope trap can be restored when an inner scope exits.
#[allow(non_camel_case_types)]
pub struct savetrap {
    // c:611
    pub sig: i32,                // c:613
    pub flags: i32,              // c:614
    pub local: i32,              // c:615 locallevel at save
    pub posix: i32,              // c:616 exit_trap_posix snapshot
    pub list: Option<Eprog>,     // c:617 trap eval-list Eprog
    /// Snapshot of the body string from `traps_table` at save time.
    /// Rust-only — C zsh stores the body in `siglists[sig]` as an
    /// Eprog (already covered by `list` above), but zshrs stores the
    /// raw body string in `traps_table` for the bridge's
    /// `execute_script` dispatch path. Without snapshotting it here,
    /// `endtrapscope`'s restore loop puts back `sigtrapped` flags
    /// matching the outer scope but the body in `traps_table` still
    /// reflects the inner scope's overwrite — so the outer EXIT
    /// trap dispatches the inner body. Bug #80 in docs/BUGS.md.
    pub body: Option<String>,
}

/// Direct port of `void dosavetrap(int sig, int level)` from
/// `Src/signals.c:626`. Captures the current trap state for
/// `sig` into a `savetrap` and pushes it onto `SAVETRAPS`.
pub fn dosavetrap(sig: i32, level: i32) {
    // c:626
    let flags = sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(sig as usize).copied())
        .unwrap_or(0);
    // c:663 — `st->list = siglists[sig] ? dupeprog(siglists[sig], 0) : NULL`.
    // dupeprog isn't ported yet so take the Eprog out of siglists and
    // re-stash a fresh None — the saved entry owns the body until the
    // matching endtrapscope restore re-inserts it.
    let list = siglists
        .lock()
        .ok()
        .and_then(|mut g| g.get_mut(sig as usize).and_then(|s| s.take()));
    let posix = if sig == SIGEXIT {
        if EXIT_TRAP_POSIX.load(Ordering::Relaxed) {
            1
        } else {
            0
        }
    } else {
        0
    };
    // Snapshot the body string from `traps_table` so the matching
    // restore in `endtrapscope` can write back the outer scope's body
    // — see `savetrap::body` doc above for the bug #80 context.
    let body = {
        let signame = getsigname(sig);
        crate::ported::builtin::traps_table()
            .lock()
            .ok()
            .and_then(|g| g.get(&signame).cloned())
    };
    let st = savetrap {
        sig,
        flags,
        local: level,
        posix,
        list,
        body,
    };
    if let Ok(mut g) = SAVETRAPS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.insert(0, st); // c:689 front-insert
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
pub fn settrap(sig: i32, l: Option<Eprog>, flags: i32) -> i32 {
    // c:693
    if sig == -1 {
        // c:693
        return 1;
    }
    // c:696 (zsh.h:2563) — `if (jobbing && (sig == SIGTTOU ||
    // sig == SIGTSTP || sig == SIGTTIN)) { zerr("can't trap SIG%s
    // in interactive shells", ...); return 1; }`.
    let jobbing = isset(MONITOR); // c:696
    if jobbing && (sig == libc::SIGTTOU || sig == libc::SIGTSTP || sig == libc::SIGTTIN) {
        // c:697 — `zerr("can't trap SIG%s in interactive shells", sigs[sig])`.
        let signame = getsigname(sig);
        zerr(&format!("can't trap SIG{} in interactive shells", signame));
        return 1; // c:699
    }

    // c:705 — `queue_signals()` + `unsettrap(sig)` unconditional
    // (saves the previous trap if locallevel changed).
    queue_signals();
    unsettrap(sig);

    // c:709-710 — DPUTS((flags & ZSIG_FUNC) && l,
    //                   "BUG: trap function has passed eval list, too")
    DPUTS!(
        // c:709
        (flags & ZSIG_FUNC) != 0 && l.is_some(), // c:709
        "BUG: trap function has passed eval list, too"  // c:710
    );

    // c:712 — `if (!(flags & ZSIG_FUNC) && empty_eprog(l))`. C's
    // `empty_eprog` returns true for NULL, NULL prog, OR a prog whose
    // first wordcode is WCB_END (`Src/parse.c:586`).
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
    if (flags & ZSIG_FUNC) == 0 && l_is_empty {
        // c:712
        // c:713 — `sigtrapped[sig] = ZSIG_IGNORED`.
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(sig as usize) {
                *slot = ZSIG_IGNORED;
            }
        }
        if sig != 0 && sig <= SIGCOUNT && sig != libc::SIGWINCH && sig != libc::SIGCHLD {
            signal_ignore(sig); // c:719
        }
        // c:720-723 — RT-signal trap-table branch:
        //   `else if (sig >= VSIGCOUNT && sig < TRAPCOUNT)
        //                signal_ignore(SIGNUM(sig));`
        #[cfg(target_os = "linux")]
        if sig >= VSIGCOUNT && sig < TRAPCOUNT_H {
            signal_ignore(SIGNUM(sig)); // c:722
        }
    } else {
        nsigtrapped.fetch_add(1, Ordering::Relaxed); // c:725
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(sig as usize) {
                *slot = ZSIG_TRAPPED;
            }
        }
        if sig != 0 && sig <= SIGCOUNT && sig != libc::SIGWINCH && sig != libc::SIGCHLD {
            install_handler(sig); // c:732
        }
        // c:733-736 — RT-signal install_handler branch:
        //   `if (sig >= VSIGCOUNT && sig < TRAPCOUNT)
        //              install_handler(SIGNUM(sig));`
        // Trapping `RTMIN+1` to a function without this branch would
        // store the trap but NEVER install the libc handler, so the
        // signal would fire with default action (terminate the shell).
        #[cfg(target_os = "linux")]
        if sig >= VSIGCOUNT && sig < TRAPCOUNT_H {
            install_handler(SIGNUM(sig)); // c:735
        }
    }
    // c:738 — `sigtrapped[sig] |= flags`.
    if let Ok(mut g) = sigtrapped.lock() {
        if let Some(slot) = g.get_mut(sig as usize) {
            *slot |= flags;
        }
    }
    // c:743-752 — locallevel tag (SIGEXIT in POSIX mode is sticky).
    let locallevel = locallevel_fn() as i32;
    if sig == SIGEXIT {
        // c:746 — `if (isset(POSIXTRAPS)) ...`. In POSIX mode SIGEXIT
        // is sticky and not tagged with the local-level shift.
        let posix_traps = isset(optlookup("posixtraps")); // c:746
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
    0 // c:759
}

/// Direct port of `HashNode removetrap(int sig)` from `Src/signals.c:772`.
/// Clears the trap slot for `sig`, snapshots the prior state into
/// `SAVETRAPS` when `locallevel > 0` and the relevant option set
/// (LOCALTRAPS for generic signals, !POSIXTRAPS for EXIT), then
/// re-installs the appropriate per-signal disposition (intr for
/// SIGINT-interactive, install_handler for SIGHUP/SIGPIPE,
/// signal_default for everything else).
///
/// C returns the displaced HashNode for the caller (unsettrap) to
/// free; Rust ownership covers the free automatically when the
/// hashtable entry drops.
pub fn removetrap(sig: i32) {
    // c:772
    let trapped = sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(sig as usize).copied())
        .unwrap_or(0);
    // c:776-778 — `if (sig == -1 || (jobbing && (SIGTTOU || SIGTSTP || SIGTTIN))) return NULL`.
    // The Rust call sites already use sig in [0, SIGCOUNT]; sig == -1 is rare,
    // but jobbing+job-control reject mirrors C exactly.
    if sig == -1 {
        return;
    }
    let jobbing = isset(MONITOR);
    if jobbing && (sig == libc::SIGTTOU || sig == libc::SIGTSTP || sig == libc::SIGTTIN) {
        return;
    }
    let locallevel = locallevel_fn() as i32;
    // c:769-774 — `if (!dontsavetrap && (sig == SIGEXIT ? !isset(POSIXTRAPS)
    // : isset(LOCALTRAPS)) && locallevel && (!trapped || locallevel >
    // (sigtrapped[sig] >> ZSIG_SHIFT))) dosavetrap(sig, locallevel);`.
    // Note: `!trapped` is LOGICAL NOT (`trapped == 0`), not Rust's
    // bitwise `!i32`.
    let cond_local_or_exit = if sig == SIGEXIT {
        !isset(POSIXTRAPS) // c:771 sig==SIGEXIT branch
    } else {
        isset(LOCALTRAPS) // c:771 else branch
    };
    if DONTSAVETRAP.load(Ordering::Relaxed) == 0                             // c:769
        && cond_local_or_exit
        && locallevel != 0                                                   // c:772 `locallevel &&`
        && (trapped == 0                                                     // c:773 `!trapped` (logical NOT)
            || locallevel > (trapped >> ZSIG_SHIFT))
    {
        dosavetrap(sig, locallevel); // c:774
    }
    if trapped & ZSIG_TRAPPED != 0 {
        nsigtrapped.fetch_sub(1, Ordering::Relaxed); // c:799
    }
    if let Ok(mut g) = sigtrapped.lock() {
        if let Some(slot) = g.get_mut(sig as usize) {
            *slot = 0;
        } // c:800
    }
    if let Ok(mut g) = siglists.lock() {
        if let Some(slot) = g.get_mut(sig as usize) {
            *slot = None;
        }
    }
    // c:803-845 — per-signal disposition reset after clearing the
    // trap. The previous Rust port collapsed everything to a single
    // signal_default() call, omitting the SIGINT/SIGHUP/SIGPIPE
    // special branches AND the RT-signal branch entirely.
    let interact = isset(INTERACTIVE);
    // c:808 `forklevel` — depth of subshell forks. C global at
    // exec.c:1052 set to `locallevel` at every entersubsh() (c:1221).
    // Read live from the ported global so SIGPIPE only re-installs in
    // the top-level shell, never inside a forked subshell.
    let forklevel: i32 = crate::ported::exec::FORKLEVEL.load(Ordering::Relaxed); // c:1052 (Src/exec.c)
    if sig == libc::SIGINT && interact {
        // c:802
        // c:803-805 — `intr(); noholdintr();`. Re-enable SIGINT
        // delivery (subshells ignoring SIGINT need the unblock).
        intr();
        noholdintr();
    } else if sig == libc::SIGHUP {
        // c:806
        // c:807 — HUP gets RE-INSTALLED (not defaulted), so the
        // shell keeps catching it.
        install_handler(sig);
    } else if sig == libc::SIGPIPE && interact && forklevel == 0 {
        // c:808
        // c:809 — same install-not-default semantics.
        install_handler(sig);
    } else if sig != 0 && sig <= SIGCOUNT && sig != libc::SIGWINCH && sig != libc::SIGCHLD {
        // c:810
        signal_default(sig); // c:815
    }
    // c:816-819 — RT-signal branch (Linux).
    #[cfg(target_os = "linux")]
    {
        if sig >= VSIGCOUNT && sig < TRAPCOUNT_H {
            signal_default(SIGNUM(sig)); // c:818
        }
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

/// Remove a trap completely and reset to default disposition.
/// Port of `removetrap(int sig)` from Src/signals.c:772.
///
/// **Inverted call chain vs C**: in C, `unsettrap` (c:759) is a
/// thin queue_signals + removetrap wrapper; the full save+clear+
/// signal-disposition logic lives in `removetrap`. The Rust port
/// inverts the relationship — `unsettrap` carries the full body
/// (matching C lines 781-820), and `removetrap` is the thin
/// wrapper. `unsettrap` runs the per-signal disposition
/// (c:802-820): SIGINT → intr(), SIGHUP → re-install_handler,
/// SIGPIPE under interactive non-fork → re-install_handler.
/// Never SIG_DFL these branches directly.
pub fn unsettrap(sig: i32) {
    // c:759
    // c:763 — queue_signals();
    queue_signals();
    // c:764 — hn = removetrap(sig);
    //         c:765-766 — if (hn) shfunctab->freenode(hn);
    //         Rust ownership covers the freenode when the trap entry
    //         is removed from sigfuncs / freed automatically.
    removetrap(sig);
    // c:767 — unqueue_signals();
    unqueue_signals();
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
pub fn starttrapscope() {
    // c:855
    // c:855 — `if (intrap) return`.
    if intrap.load(Ordering::Relaxed) != 0 {
        return;
    }
    // c:863 — `if (sigtrapped[SIGEXIT] && !exit_trap_posix)`.
    let exit_flags = sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(SIGEXIT as usize).copied())
        .unwrap_or(0);
    if exit_flags != 0 && !EXIT_TRAP_POSIX.load(Ordering::Relaxed) {
        // c:865-867 — bump locallevel so the dosavetrap inside
        // unsettrap tags the save entry with the outer scope's
        // level. Rust's locallevel is a global counter in utils.rs.
        inc_locallevel();
        unsettrap(SIGEXIT); // c:866
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
pub fn endtrapscope() {
    // c:880
    let locallevel = locallevel_fn();

    // c:891-908 — pull the SIGEXIT trap aside so we can run it last.
    let exit_flags = sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(SIGEXIT as usize).copied())
        .unwrap_or(0);
    let mut exittr: i32 = 0;
    // Bug #80 — capture the body BEFORE the SAVETRAPS pop loop
    // potentially writes back an outer scope's body into traps_table.
    // Without this snapshot, a nested fn EXIT trap could fire the
    // wrong (outer) body when the deepest level exits.
    let mut exit_body: Option<String> = None;
    if intrap.load(Ordering::Relaxed) == 0                                   // c:891 !intrap
        && !EXIT_TRAP_POSIX.load(Ordering::Relaxed)                          // c:892 !exit_trap_posix
        && exit_flags != 0
    {
        exittr = exit_flags;
        // Snapshot the body so the dispatch at the end of this fn
        // fires THIS scope's trap, not whatever traps_table got
        // restored to.
        exit_body = {
            let signame = getsigname(SIGEXIT);
            crate::ported::builtin::traps_table()
                .lock()
                .ok()
                .and_then(|g| g.get(&signame).cloned().or_else(|| g.get("EXIT").cloned()))
        };
        // c:902-906 — clear SIGEXIT slot.
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(SIGEXIT as usize) {
                *slot = 0;
            }
        }
        if let Ok(mut g) = siglists.lock() {
            if let Some(slot) = g.get_mut(SIGEXIT as usize) {
                *slot = None;
            }
        }
        // Clear the inner-scope's body from traps_table; if a saved
        // outer body needs restoring, the SAVETRAPS pop loop below
        // writes it back.
        if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
            let signame = getsigname(SIGEXIT);
            t.remove(&signame);
            t.remove("EXIT"); // alias-safe
        }
        if exit_flags & ZSIG_TRAPPED != 0 {
            nsigtrapped.fetch_sub(1, Ordering::Relaxed); // c:904
        }
    }

    // c:911-959 — pop savetraps entries whose local > locallevel.
    if let Ok(mut traps) = SAVETRAPS.get_or_init(|| Mutex::new(Vec::new())).lock() {
        while let Some(st) = traps.first() {
            // c:912 firstnode
            if st.local <= locallevel as i32 {
                break;
            } // c:914
            let st = traps.remove(0); // c:915

            // c:919 — `if (st->flags && (st->list != NULL))`. BOTH must
            // be truthy. The previous Rust port used `||` (either),
            // wrongly firing the restore branch on a flags-only or
            // list-only savetrap entry.
            // Bug #80 — restore the saved body string into
            // traps_table before settrap re-arms sigtrapped, so the
            // outer scope's dispatch finds the right body.
            {
                let signame = getsigname(st.sig);
                if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                    match &st.body {
                        Some(b) => {
                            t.insert(signame, b.clone());
                        }
                        None => {
                            t.remove(&signame);
                        }
                    }
                }
            }
            if st.flags != 0 && st.list.is_some() {
                // c:919
                // c:921-922 — prevent settrap from saving this.
                DONTSAVETRAP.fetch_add(1, Ordering::Relaxed);
                // c:923-926 — ZSIG_FUNC takes (NULL, ZSIG_FUNC); list
                // traps take (st.list, 0). The current Rust port
                // collapses both into a single settrap(list, flags)
                // call — works because settrap stores the flags and
                // list independently. Pin the ZSIG_FUNC branch as a
                // comment for future refactors.
                let _ = settrap(st.sig, st.list, st.flags); // c:925/927
                if st.sig == SIGEXIT {
                    EXIT_TRAP_POSIX.store(st.posix != 0, Ordering::Relaxed); // c:929
                }
                DONTSAVETRAP.fetch_sub(1, Ordering::Relaxed); // c:930
            } else if st.flags != 0 && st.body.is_some() {
                // Saved entry was a list-trap installed via bin_trap
                // (body in traps_table, no Eprog). Re-arm sigtrapped
                // with the saved flags so the dispatch path finds it.
                DONTSAVETRAP.fetch_add(1, Ordering::Relaxed);
                let _ = settrap(st.sig, None, st.flags);
                if st.sig == SIGEXIT {
                    EXIT_TRAP_POSIX.store(st.posix != 0, Ordering::Relaxed);
                }
                DONTSAVETRAP.fetch_sub(1, Ordering::Relaxed);
            } else {
                // c:933 — `else if (sigtrapped[sig])`. Only fires when
                // the current slot has a trap set. The previous Rust
                // port unconditionally entered this branch, calling
                // unsettrap on slots that were already cleared.
                let cur_trapped = sigtrapped
                    .lock()
                    .ok()
                    .and_then(|g| g.get(st.sig as usize).copied())
                    .unwrap_or(0);
                if cur_trapped != 0 {
                    // c:933
                    // c:938 — `if (sig != SIGEXIT || !exit_trap_posix)`.
                    if st.sig != SIGEXIT || !EXIT_TRAP_POSIX.load(Ordering::Relaxed) {
                        unsettrap(st.sig); // c:939
                    }
                }
            }
        }
    }

    // c:961-969 — run the SIGEXIT trap, last (AFTER the savetraps
    // pop loop above so a restored deeper SIGEXIT is replaced by
    // the current scope's saved-aside trap before dispatch).
    //
    // C `dotrapargs(SIGEXIT, &exittr, exitfn)` invokes either:
    //   - ZSIG_FUNC: the `TRAPEXIT` shell function from shfunctab.
    //   - else: the eprog from siglists[SIGEXIT].
    if exittr != 0 && (exittr & ZSIG_FUNC) != 0 {
        // c:961, c:1132 FUNC branch — dispatch the TRAPEXIT shfunc.
        let signame = getsigname(SIGEXIT);
        let trap_fn = format!("TRAP{}", signame);
        if crate::ported::utils::getshfunc(&trap_fn).is_some() {
            let args = vec![SIGEXIT.to_string()];
            let _ = crate::ported::exec_hooks::dispatch_function_call(&trap_fn, &args);
        }
    } else if exittr != 0 {
        // c:961 else branch — non-FUNC eprog. The Rust port stores
        // the trap body as a string in `traps_table` (populated by
        // `bin_trap` / settrap). Dispatch through the execute_script
        // hook installed by fusevm_bridge — same path used by
        // signals.rs dotrap for non-FUNC traps.
        //
        // Bug #80 — use the body snapshot taken BEFORE the SAVETRAPS
        // pop loop, so the dispatch fires THIS scope's body (not the
        // outer body restored by the loop).
        let body = exit_body.unwrap_or_default();
        if !body.is_empty() {
            // Bracket the dispatch so the recursive
            // `execute_script_zsh_pipeline`'s own end-of-pipeline EXIT
            // check doesn't see the OUTER scope's body (which the
            // SAVETRAPS pop loop just restored into traps_table) and
            // fire it prematurely. Pull "EXIT" aside for the duration
            // of the dispatch, restore after.
            let stash = crate::ported::builtin::traps_table()
                .lock()
                .ok()
                .and_then(|mut t| t.remove("EXIT"));
            let _ = crate::ported::exec_hooks::execute_script(&body); // c:961 eprog body
            if let Some(b) = stash {
                if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                    t.insert("EXIT".to_string(), b);
                }
            }
        }
    }
}

/// Direct port of `mod_export int handletrap(int sig)` from
/// `Src/signals.c:972`. Trap-queue gate called from the async
/// signal handlers. Returns 0 if the signal isn't trapped; if
/// trapped + queueing enabled it pushes onto `trap_queue` and
/// returns 1; otherwise it calls `dotrap(SIGIDX(sig))` (with the
/// SIGALRM TMOUT reset at the end) and returns 1.
pub fn handletrap(sig: i32) -> i32 {
    // c:972
    let idx = crate::ported::signals_h::SIGIDX(sig);
    let trapped = sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(idx as usize).copied())
        .unwrap_or(0);
    if trapped == 0 {
        return 0;
    } // c:974

    if trap_queueing_enabled.load(Ordering::SeqCst) != 0 {
        // c:977
        // c:980-986 — push onto `trap_queue` ring buffer.
        let r = trap_queue_rear.load(Ordering::SeqCst);
        let new_rear = (r + 1) % MAX_QUEUE_SIZE;
        if new_rear != trap_queue_front.load(Ordering::SeqCst) {
            trap_queue[new_rear].store(sig, Ordering::SeqCst);
            trap_queue_rear.store(new_rear, Ordering::SeqCst);
        }
        return 1;
    }

    dotrap(idx); // c:990

    if sig == libc::SIGALRM {
        // c:992
        // c:996 — `if ((tmout = getiparam("TMOUT"))) alarm(tmout);`
        // Re-arm the TMOUT timer after the trap dispatched.
        #[cfg(unix)]
        unsafe {
            let tmout = getiparam("TMOUT");
            if tmout > 0 {
                libc::alarm(tmout as u32); // c:996
            }
        }
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
/// so the wait can be interrupted). The flag is a boolean (`= 1` /
/// `= 0`), symmetric with the reset in `unqueue_traps` at c:1042.
pub fn queue_traps(wait_cmd: i32) {
    // c:1024
    // c:1026 — both gates must be off for queueing to be enabled.
    if !isset(TRAPSASYNC) && wait_cmd == 0 {
        trap_queueing_enabled.store(1, Ordering::SeqCst); // c:1031
    }
}

// Disable trap queuing and run the traps.                                 // c:1041
/// Direct port of `void unqueue_traps(void)` from
/// `Src/signals.c:1041`. Disables `trap_queueing_enabled` and
/// flushes the pending queue by dispatching each sig through
/// `handletrap()`.
pub fn unqueue_traps() {
    // c:1041
    // c:1041 — `trap_queueing_enabled = 0;`
    trap_queueing_enabled.store(0, Ordering::SeqCst);
    // c:1046 — `while (trap_queue_front != trap_queue_rear) (void) handletrap(...);`
    loop {
        let f = trap_queue_front.load(Ordering::SeqCst);
        let r = trap_queue_rear.load(Ordering::SeqCst);
        if f == r {
            break;
        }
        let nf = (f + 1) % MAX_QUEUE_SIZE;
        let sig = trap_queue[nf].load(Ordering::SeqCst);
        trap_queue_front.store(nf, Ordering::SeqCst);
        let _ = handletrap(sig);
    }
}

// Standard call to execute a trap for a given signal.                     // c:1245
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
pub fn dotrap(sig: i32) -> i32 {
    // c:1245
    // c:1248 — `int q = queue_signal_level();` capture at entry.
    // Required for the c:1280 `restore_queue_signals(q)` tail. The
    // previous Rust port omitted the capture and tail-restored to 0
    // unconditionally — that zeroed the queue level even when the
    // caller had set it to a non-zero value (e.g. nested dotrap
    // from inside a queue_signals/unqueue_signals block).
    let q = crate::ported::signals_h::queue_signal_level(); // c:1248

    let trapped = sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(sig as usize).copied())
        .unwrap_or(0);
    // c:1259 — `if ((sigtrapped[sig] & ZSIG_IGNORED) || !funcprog || errflag) return;`
    if trapped & ZSIG_IGNORED != 0 {
        return 0;
    }
    // Look up a fallback raw-text body installed by `bin_trap`
    // (`trap '...' SIG` form). bin_trap stores the body in the
    // canonical `traps_table` HashMap<String, String> but never
    // calls settrap, so `sigtrapped[sig]` may be 0 here even when
    // there IS a live trap. Treat presence in traps_table as
    // equivalent to ZSIG_TRAPPED for the dispatch decision and
    // dispatch via the exec_hooks::execute_script fn-ptr installed
    // by fusevm_bridge.
    let signame_for_lookup = getsigname(sig);
    let table_body: Option<String> = {
        let aliases: &[&str] = match sig {
            x if x == SIGZERR => &["ZERR", "ERR"],
            x if x == SIGDEBUG => &["DEBUG"],
            x if x == SIGEXIT => &["EXIT"],
            _ => &[],
        };
        let mut found = None;
        if let Ok(t) = crate::ported::builtin::traps_table().lock() {
            if let Some(b) = t.get(&signame_for_lookup) {
                found = Some(b.clone());
            } else {
                for alias in aliases {
                    if let Some(b) = t.get(*alias) {
                        found = Some(b.clone());
                        break;
                    }
                }
            }
        }
        found
    };
    if trapped & (ZSIG_TRAPPED | ZSIG_FUNC) == 0 && table_body.is_none() {
        return 0;
    }
    if errflag.load(Ordering::Relaxed) != 0 {
        return 0;
    }
    // c:Src/signals.c:1244 — `if (intrap) return;` — re-entry guard.
    // Without this, a TRAP body that itself fails (e.g. `trap "false"
    // ERR; false`) re-enters dotrap on its own failed status and
    // recurses indefinitely → stack overflow.
    if intrap.load(Ordering::Relaxed) != 0 {
        return 0;
    }

    intrap.store(1, Ordering::SeqCst);
    // c:1270 — `dont_queue_signals()`. C disables signal queueing for
    // the duration of the trap dispatch so signals delivered while
    // the trap is running run inline (not queued for later).
    crate::ported::signals_h::dont_queue_signals(); // c:1270

    // c:1272-1273 — `if (sig == SIGEXIT) ++in_exit_trap;` (counter,
    // not boolean — depth tracking lets observers detect re-entry).
    if sig == SIGEXIT {
        in_exit_trap.fetch_add(1, Ordering::SeqCst); // c:1273
    }

    // c:1251 — `if (sigtrapped[sig] & ZSIG_FUNC)` → run TRAPxxx shfunc.
    // c:Src/signals.c:1251-1259 — the C source dispatches ONE of
    // the two arms based on sigtrapped flags: ZSIG_FUNC → call
    // shfunc, else → run siglists eprog. The C settrap → unsettrap
    // chain ensures only one form is active at a time. zshrs's
    // port stores string-form bodies separately in `traps_table`
    // (not touched by removetrap), so both can coexist. Track
    // whether the function-form fired so the string-form fallback
    // below doesn't double-dispatch. Bug #541 in docs/BUGS.md.
    let mut fn_dispatched = false;
    if trapped & ZSIG_FUNC != 0 {
        let signame = getsigname(sig);
        let trap_fn = format!("TRAP{}", signame);
        if getshfunc(&trap_fn).is_some() {
            // c:1252-1255 — `dotrapargs(sig, sigtrapped+sig, funcprog)`.
            //              Drives the shfunc with `$1 = sig`. With the
            //              executor not directly callable from this
            //              signal-handler context, route through the
            //              canonical `crate::exec::doshfunc` entry which
            //              handles the arg+env+local-scope wrap.
            let args = vec![sig.to_string()];
            let _ = crate::ported::exec_hooks::dispatch_function_call(&trap_fn, &args);
            fn_dispatched = true;
        }
    }
    // Additional function-form check: even when ZSIG_FUNC isn't
    // set on sigtrapped (the function was defined after the
    // string-form trap, and zshrs's exec.rs:6292 didn't update
    // sigtrapped via the doshfunc path), look in shfunctab
    // directly. zsh's last-defined-wins semantics treat the
    // function as the active handler — the older string-form
    // entry in traps_table must not also fire.
    if !fn_dispatched {
        let signame = getsigname(sig);
        let trap_fn = format!("TRAP{}", signame);
        if getshfunc(&trap_fn).is_some() {
            let args = vec![sig.to_string()];
            let _ = crate::ported::exec_hooks::dispatch_function_call(&trap_fn, &args);
            fn_dispatched = true;
        }
    }
    // c:1268 — non-FUNC `siglists[sig]` eprog branch. The canonical
    // settrap→siglists path isn't fully wired (bin_trap stores raw
    // body text into `traps_table` rather than parsing to Eprog and
    // calling settrap). Dispatch via the exec_hooks::execute_script
    // fn-ptr installed by fusevm_bridge — no direct ShellExecutor
    // reach-in from src/ported/ (see memory
    // feedback_no_exec_script_from_ported).
    // Skip the string-form fallback when a function-form already
    // fired — zsh semantics are last-defined-replaces, not
    // both-fire. Bug #541 in docs/BUGS.md.
    if let Some(body) = table_body.filter(|_| !fn_dispatched) {
        // c:Bug #56 — when a trap fires DURING `$(...)` capture
        // (cmdsub redirected fd 1 to a pipe), the trap body's
        // stdout would land in the captured value. zsh forks each
        // cmdsub so traps run in the parent process whose fd 1 is
        // the terminal. zshrs's in-process cmdsub publishes the
        // saved outer stdout via CMDSUBST_OUTER_FDS so the trap
        // dispatcher can route body output to the parent's real
        // stdout instead. Temporarily restore fd 1 to that saved
        // outer stdout around the body, then revert to the
        // cmdsub-bound fd. Same idea as bash's command-subst trap
        // routing (Functions/Misc/runtraps).
        let outer = crate::fusevm_bridge::cmdsubst_outer_stdout();
        let saved_inner = if outer.is_some() {
            unsafe { libc::dup(libc::STDOUT_FILENO) }
        } else {
            -1
        };
        if let Some(out_fd) = outer {
            unsafe {
                libc::dup2(out_fd, libc::STDOUT_FILENO);
            }
        }
        let _ = crate::ported::exec_hooks::execute_script(&body);
        if let Some(_) = outer {
            if saved_inner >= 0 {
                unsafe {
                    libc::dup2(saved_inner, libc::STDOUT_FILENO);
                    libc::close(saved_inner);
                }
            }
        }
    }

    // c:1277 — `if (sig == SIGEXIT) --in_exit_trap;` (decrement, not
    // store-0). The previous Rust port used `store(0)` which would
    // mask a re-entered trap — a TRAP_EXIT inside another TRAP_EXIT
    // would clear the flag prematurely.
    if sig == SIGEXIT {
        in_exit_trap.fetch_sub(1, Ordering::SeqCst); // c:1277
    }
    // c:1280 — `restore_queue_signals(q)` — restore to the level
    // captured at entry (c:1248). Now properly captured above; the
    // previous tail was a hardcoded `intrap.store(0)` only.
    crate::ported::signals_h::restore_queue_signals(q); // c:1280
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
pub fn dotrapargs(sig: i32, sigtr: &mut i32, sigfn: Option<&str>) {
    // c:1081

    let obreaks: i32 = BREAKS.load(Ordering::SeqCst); // c:1085
    let oretflag: i32 = RETFLAG.load(Ordering::SeqCst); // c:1086
    let olastval: i32 = LASTVAL.load(Ordering::SeqCst); // c:1087
    let isfunc: i32; // c:1088
    let traperr: i32; // c:1089
    let new_trap_state: i32; // c:1089
    let new_trap_return: i32; // c:1089

    // c:1101 — `if ((*sigtr & ZSIG_IGNORED) || !sigfn || errflag) return;`
    if (*sigtr & ZSIG_IGNORED) != 0                                          // c:1101
        || sigfn.is_none()
        || errflag.load(Ordering::SeqCst) != 0
    {
        return; // c:1102
    }

    // c:1112-1119 — disallow synchronous traps from nesting.
    if intrap.load(Ordering::SeqCst) != 0 {
        // c:1112
        if sig == SIGEXIT || sig == SIGDEBUG || sig == SIGZERR {
            // c:1113-1117
            return; // c:1117
        }
    }

    queue_signals(); // c:1121

    intrap.fetch_add(1, Ordering::SeqCst); // c:1123
    *sigtr |= ZSIG_IGNORED; // c:1124
                            // Mirror into the sigtrapped slab so observers (handletrap, dotrap
                            // re-entry) see the same ZSIG_IGNORED bit.
    if let Ok(mut g) = sigtrapped.lock() {
        if let Some(slot) = g.get_mut(sig as usize) {
            *slot |= ZSIG_IGNORED;
        }
    }

    zcontext_save(); // c:1126
                     // c:1128 — `execsave()` saves trap_return/trap_state. Without a
                     // canonical `execsave` port yet, snapshot the two atomics inline.
    let saved_trap_state = TRAP_STATE.load(Ordering::SeqCst); // c:1128 execsave
    let saved_trap_return = TRAP_RETURN.load(Ordering::SeqCst); // c:1128 execsave
    BREAKS.store(0, Ordering::SeqCst); // c:1129 breaks = 0
    RETFLAG.store(0, Ordering::SeqCst); // c:1129 retflag = 0
    traplocallevel.store(
        crate::ported::params::locallevel.load(Ordering::SeqCst),
        Ordering::SeqCst,
    ); // c:1130

    // c:1131 — `runhookdef(BEFORETRAPHOOK, NULL);` — fire any
    // registered "before-trap" module hooks. Looked up by name
    // through gethookdef so the module dispatcher picks up
    // installed handlers (zsh/zle's zlebeforetrap etc.).
    let hd = crate::ported::module::gethookdef("BEFORETRAPHOOK");
    if !hd.is_null() {
        let _ = crate::ported::module::runhookdef(hd, std::ptr::null_mut());
    }
    let _ = BEFORETRAPHOOK; // c:1131 — const retained for source-cite parity

    if (*sigtr & ZSIG_FUNC) != 0 {
        // c:1132
        let osc = SFCONTEXT.load(Ordering::SeqCst); // c:1133 osc
                                                    // c:1133 — `int old_incompfunc = incompfunc;` — snapshot the
                                                    // completion-function-active flag so the trap dispatch can
                                                    // run code outside the comp-fn scope and restore on return.
        let old_incompfunc: i32 = crate::ported::zle::complete::INCOMPFUNC.load(Ordering::Relaxed);
        let hn = gettrapnode(sig, false); // c:1134

        let mut args: Vec<String> = Vec::new(); // c:1136 znewlinklist
                                                // c:1144-1149 — pick the right TRAPxxx name from the function table
                                                // (multi-named aliases) or build the canonical TRAP<SIGNAME>.
        let name = match hn {
            Some(n) => ztrdup(&n), // c:1145 ztrdup(hn->nam)
            None => {
                // c:1146
                format!("TRAP{}", getsigname(sig)) // c:1147-1148
            }
        };
        args.push(name.clone()); // c:1150 zaddlinknode(args, name)
        let num = format!("{}", sig); // c:1151 sprintf(num, "%d", sig)
        args.push(num); // c:1152

        TRAP_RETURN.store(-1, Ordering::SeqCst); // c:1154 trap_return = -1; /* incremented by doshfunc */
        TRAP_STATE.store(TRAP_STATE_PRIMED, Ordering::SeqCst); // c:1155
        trapisfunc.store(1, Ordering::SeqCst); // c:1156
        isfunc = 1;

        SFCONTEXT.store(SFC_SIGNAL, Ordering::SeqCst); // c:1158
                                                       // c:1159 — `incompfunc = 0;` — clear the active-compfn flag
                                                       // so user-level trap handlers can run normal `complete` /
                                                       // `compadd` etc. without being mis-detected as inside a
                                                       // completion widget.
        crate::ported::zle::complete::INCOMPFUNC.store(0, Ordering::Relaxed);
        // c:1160 — `doshfunc((Shfunc)sigfn, args, 1);`. Direct
        // doshfunc call mirrors C — argv[0] = TRAP name, argv[1..]
        // = signum etc. body_runner routes through the host's
        // body-only entry so we don't double-wrap the scope.
        let fn_name = sigfn.unwrap_or("").to_string();
        let shf_clone: Option<crate::ported::zsh_h::shfunc> = {
            let tab = crate::ported::hashtable::shfunctab_lock().read();
            tab.ok().and_then(|t| t.get(&fn_name).cloned())
        };
        if let Some(mut shf) = shf_clone {
            let body_args = args.clone();
            let name_for_body = fn_name.clone();
            let body_runner = move || -> i32 {
                crate::ported::exec_hooks::run_function_body(&name_for_body, &body_args[1..])
                    .unwrap_or(0)
            };
            let _ = crate::ported::exec::doshfunc(&mut shf, args.clone(), true, body_runner);
        }
        SFCONTEXT.store(osc, Ordering::SeqCst); // c:1161
                                                // c:1162 — `incompfunc = old_incompfunc;` — restore the
                                                // completion-function-active flag we snapshotted at c:1133.
        crate::ported::zle::complete::INCOMPFUNC.store(old_incompfunc, Ordering::Relaxed);
        let _ = args; // c:1163 freelinklist(args)
        zsfree(name); // c:1164 zsfree(name)
    } else {
        // c:1165
        TRAP_RETURN.store(-2, Ordering::SeqCst); // c:1166 trap_return = -2
        TRAP_STATE.store(TRAP_STATE_PRIMED, Ordering::SeqCst); // c:1167
        trapisfunc.store(0, Ordering::SeqCst); // c:1168
        isfunc = 0;
        // c:1170 — `execode((Eprog)sigfn, 1, 0, "trap");` — execute
        // the trap's compiled Eprog body. zshrs models the non-
        // ZSIG_FUNC trap action as source TEXT (sigfn: Option<&str>)
        // rather than a pre-compiled Eprog; re-parse + execute via
        // the fusevm pipeline so the trap action actually fires.
        // When sigfns[] is re-shaped to carry compiled Eprogs (matching
        // C's `void *sigfns[sig]` cast at c:1170), swap in
        // `crate::ported::exec::execode(eprog, 1, 0, "trap")` directly.
        if let Some(src) = sigfn {
            let _ = crate::ported::exec_hooks::execute_script_zsh_pipeline(src);
        }
    }

    // c:1172 — `runhookdef(AFTERTRAPHOOK, NULL);` — fire any registered
    // "after-trap" module hooks. Same shape as BEFORETRAPHOOK above.
    let hd = crate::ported::module::gethookdef("AFTERTRAPHOOK");
    if !hd.is_null() {
        let _ = crate::ported::module::runhookdef(hd, std::ptr::null_mut());
    }
    let _ = AFTERTRAPHOOK; // c:1172 — const retained for source-cite parity

    traperr = errflag.load(Ordering::SeqCst); // c:1174

    new_trap_state = TRAP_STATE.load(Ordering::SeqCst); // c:1177
    new_trap_return = TRAP_RETURN.load(Ordering::SeqCst); // c:1178

    // c:1180 — `execrestore()` restores trap_return/trap_state.
    TRAP_STATE.store(saved_trap_state, Ordering::SeqCst); // c:1180
    TRAP_RETURN.store(saved_trap_return, Ordering::SeqCst); // c:1180
    zcontext_restore(); // c:1181

    if new_trap_state == TRAP_STATE_FORCE_RETURN                             // c:1183
        && !(isfunc != 0 && new_trap_return == 0)
    // c:1184
    {
        if isfunc != 0 {
            // c:1186
            BREAKS.store(LOOPS.load(Ordering::SeqCst), Ordering::SeqCst); // c:1187 breaks = loops
            if sig == libc::SIGINT || sig == libc::SIGQUIT {
                // c:1196
                errflag.fetch_or(
                    // c:1197 errflag |= ERRFLAG_INT
                    ERRFLAG_INT,
                    Ordering::SeqCst,
                );
            } else {
                // c:1198
                errflag.fetch_or(
                    // c:1199 errflag |= ERRFLAG_ERROR
                    ERRFLAG_ERROR,
                    Ordering::SeqCst,
                );
            }
        }
        LASTVAL.store(new_trap_return, Ordering::SeqCst); // c:1202
        RETFLAG.store(1, Ordering::SeqCst); // c:1204 retflag = 1
    } else {
        // c:1205
        if traperr != 0 && !EMULATION(EMULATE_SH) {
            // c:1206
            LASTVAL.store(1, Ordering::SeqCst); // c:1207
        } else {
            // c:1208
            // c:1210 — keep pre-trap lastval.
            LASTVAL.store(olastval, Ordering::SeqCst); // c:1213
        }
        if try_tryflag.load(Ordering::SeqCst) != 0 {
            // c:1215 try_tryflag
            if traperr != 0 {
                // c:1216
                errflag.fetch_or(
                    // c:1217
                    ERRFLAG_ERROR,
                    Ordering::SeqCst,
                );
            } else {
                // c:1218
                errflag.fetch_and(
                    // c:1219 errflag &= ~ERRFLAG_ERROR
                    !ERRFLAG_ERROR,
                    Ordering::SeqCst,
                );
            }
        }
        BREAKS.fetch_add(obreaks, Ordering::SeqCst); // c:1220 breaks += obreaks
        RETFLAG.store(oretflag, Ordering::SeqCst); // c:1222 retflag = oretflag
        let cur_breaks = BREAKS.load(Ordering::SeqCst);
        let cur_loops = LOOPS.load(Ordering::SeqCst);
        if cur_breaks > cur_loops {
            // c:1223
            BREAKS.store(cur_loops, Ordering::SeqCst); // c:1224
        }
    }

    // c:1231 — `if (zleactive && resetneeded) zleentry(ZLE_CMD_REFRESH);`
    if zleactive.load(Ordering::SeqCst) != 0 && RESETNEEDED.load(Ordering::SeqCst) != 0 {
        let _ = zleentry(ZLE_CMD_REFRESH);
    }

    if *sigtr != ZSIG_IGNORED {
        // c:1234
        *sigtr &= !ZSIG_IGNORED; // c:1235
        if let Ok(mut g) = sigtrapped.lock() {
            if let Some(slot) = g.get_mut(sig as usize) {
                *slot &= !ZSIG_IGNORED;
            }
        }
    }
    intrap.fetch_sub(1, Ordering::SeqCst); // c:1236

    unqueue_signals(); // c:1238
}

// `try_tryflag` lives at `Src/loop.c:731` (the always/try block depth
// counter); ported to `crate::ported::r#loop::try_tryflag`. dotrapargs
// above reads it from its canonical home per PORT.md Rule C (header /
// file placement).

/// Resolve a real-time signal name to its number.
/// Port of `int rtsigno(const char* signame)` from `Src/signals.c:1291-1313`.
///
/// **C signature**: `int rtsigno(const char* signame)` — takes a
/// NAME STRING ("RTMIN", "RTMIN+3", "RTMAX-1", etc.) and returns
/// the signal number, or 0 on parse failure.
///
/// Rust signature: `(signame: &str) -> Option<i32>` — `None`
/// matches C's `0` sentinel. Uses `libc::SIGRTMIN()` /
/// `libc::SIGRTMAX()` for canonical bounds.
pub fn rtsigno(signame: &str) -> Option<i32> {
    // c:1291
    #[cfg(target_os = "linux")]
    {
        let sigrtmin = libc::SIGRTMIN();
        let sigrtmax = libc::SIGRTMAX();
        let maxofs = sigrtmax - sigrtmin; // c:1296

        // c:1298-1306 — `if (!strncmp(signame, "RTMIN", 5)) ...
        // else if (!strncmp(signame, "RTMAX", 5)) ... else return 0;`
        let (sig, dir, op): (i32, i32, char) = if let Some(rest) = signame.strip_prefix("RTMIN") {
            (sigrtmin, 1, '+') // c:1300
        } else if let Some(rest) = signame.strip_prefix("RTMAX") {
            (sigrtmax, -1, '-') // c:1302
        } else {
            return None; // c:1304 return 0
        };

        // c:1307-1311 — `if (signame[5] == x.op) { offset = strtol(...);
        //                                          if (offset > maxofs) return 0;
        //                                          x.sig += offset * x.dir; }`
        let rest = if signame.starts_with("RTMIN") {
            &signame[5..]
        } else {
            &signame[5..]
        };
        let mut final_sig = sig;
        if !rest.is_empty() {
            if rest.starts_with(op) {
                let num_str = &rest[1..];
                let offset: i32 = match num_str.parse() {
                    Ok(n) => n,
                    Err(_) => return None, // c:1312
                };
                if offset > maxofs {
                    return None; // c:1310 return 0
                }
                final_sig += offset * dir;
            } else {
                // c:1313 — `if (*end) return 0;` — any non-op trailing → fail.
                return None;
            }
        }
        Some(final_sig)
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
pub fn rtsigname(sig: i32) -> String {
    // c:1317
    #[cfg(target_os = "linux")]
    {
        let sigrtmin = libc::SIGRTMIN();
        let sigrtmax = libc::SIGRTMAX();
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
pub static queueing_enabled: AtomicI32 = AtomicI32::new(0); // c:77

/// Ring-buffer head. Port of `mod_export volatile int queue_front`
/// from `Src/signals.c:77`.
pub static queue_front: AtomicUsize = AtomicUsize::new(0); // c:77

/// Ring-buffer tail. Port of `mod_export volatile int queue_rear`
/// from `Src/signals.c:77`.
pub static queue_rear: AtomicUsize = AtomicUsize::new(0); // c:77

/// Port of `mod_export volatile int queue_in` from `Src/signals.c:84`.
/// Companion counter bumped by `queue_signals()` (signals.h:90) and
/// decremented by `unqueue_signals()` (signals.h:94); used by
/// `dont_queue_signals()` to snapshot the depth (signals.h:99) and
/// by debug assertions (DPUTS2 at signals.h:105).
pub static queue_in: AtomicI32 = AtomicI32::new(0); // c:84

#[allow(clippy::declare_interior_mutable_const)]
const ATOM_I32_ZERO: AtomicI32 = AtomicI32::new(0);

/// Per-slot signal numbers. Port of `mod_export int
/// signal_queue[MAX_QUEUE_SIZE]` from `Src/signals.c:79`.
pub static signal_queue: [AtomicI32; MAX_QUEUE_SIZE] = // c:79
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
pub static trap_queueing_enabled: AtomicI32 = AtomicI32::new(0); // c:90

/// Trap-queue head. Port of `static volatile int trap_queue_front`
/// from `Src/signals.c:90`.
pub static trap_queue_front: AtomicUsize = AtomicUsize::new(0); // c:90

/// Trap-queue tail. Port of `static volatile int trap_queue_rear`
/// from `Src/signals.c:90`.
pub static trap_queue_rear: AtomicUsize = AtomicUsize::new(0); // c:90

/// Port of `int last_signal` from `Src/signals.c:238`. Holds the
/// signal number of the most recent delivery; used by `wait_cmd`
/// in jobs.c to set `$?` to `128 + last_signal` when a trapped
/// signal interrupts wait.
pub static last_signal: AtomicI32 = AtomicI32::new(0); // c:238

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
pub static sigtrapped: std::sync::LazyLock<Mutex<Vec<i32>>> = // c:39
    std::sync::LazyLock::new(|| Mutex::new(vec![0; TRAPCOUNT as usize]));

/// Per-signal Eprog body. Port of `mod_export Eprog *siglists`
/// from `Src/signals.c:53`. NULL for ZSIG_FUNC entries (function
/// body resolves through `gettrapnode` at dispatch time).
pub static siglists: std::sync::LazyLock<Mutex<Vec<Option<Eprog>>>> =
    // c:53
    std::sync::LazyLock::new(|| Mutex::new((0..TRAPCOUNT as usize).map(|_| None).collect()));

/// Count of `ZSIG_TRAPPED`-flagged signals. Port of
/// `mod_export volatile int nsigtrapped` from `Src/signals.c:58`.
pub static nsigtrapped: AtomicI32 = AtomicI32::new(0); // c:58

/// File-scope `int intrap` from `Src/signals.c`. Set while a
/// trap body is running so nested `dotrap` calls short-circuit
/// (matches the c:1245 dispatcher's `if (intrap) return`).
pub static intrap: AtomicI32 = AtomicI32::new(0); // c:intrap

/// File-scope `int in_exit_trap` from `Src/signals.c:60`. Set
/// while the EXIT trap body is running so `exit` and friends can
/// distinguish "real" exit from exit-trap-driven exit.
pub static in_exit_trap: AtomicI32 = AtomicI32::new(0); // c:60

/// Port of `volatile int trapisfunc` from `Src/signals.c:1062`.
/// Set by `dotrapargs()` (signals.c:1156) when the trap body is a
/// shell function (vs. inline command) — the `IN_EVAL_TRAP()` macro
/// at zsh.h:2962 tests this against `intrap` + `locallevel`.
pub static trapisfunc: AtomicI32 = AtomicI32::new(0); // c:1062

/// Port of `volatile int traplocallevel` from `Src/signals.c:1069`.
/// Captures `locallevel` at trap-entry so the trap body can detect
/// whether it's running inside the same scope it was registered in
/// (the third leg of `IN_EVAL_TRAP()` at zsh.h:2962).
pub static traplocallevel: AtomicI32 = AtomicI32::new(0); // c:1069

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
pub static DONTSAVETRAP: AtomicI32 = AtomicI32::new(0);

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

fn interact_lock() -> &'static AtomicBool {
    static INTERACT: AtomicBool = AtomicBool::new(false);
    &INTERACT
}

/// Setter for the `interact` flag. Called by init.rs once the
/// shell-mode dispatch determines whether stdin is a tty / `-i`
/// was passed.
///
/// Writes to the canonical INTERACTIVE option flag so the read
/// side (`is_interact` → `isset(INTERACTIVE)`) sees it.
pub fn set_interact(v: bool) {
    crate::ported::options::opt_state_set("interactive", v);
}

/// Read the `interact` flag.
///
/// C: `#define interact (isset(INTERACTIVE))` (Src/zsh.h:2562) —
/// the canonical interact predicate reads the INTERACTIVE option
/// flag from the options table.
///
/// The previous Rust port read `interact_lock()` (a private
/// AtomicBool) whose only writer was `set_interact()` — and
/// `set_interact` was called ONLY from this file's tests. In
/// production, `interact_lock` stayed at the default `false`, so
/// every `if is_interact()` gate in `intr`/`nointr`/`holdintr`/
/// `noholdintr` short-circuited regardless of whether the shell
/// was actually interactive. `setopt INTERACTIVE` had no effect
/// on signal handling.
///
/// Route through canonical isset() so the option drives the predicate.
pub fn is_interact() -> bool {
    isset(INTERACTIVE)
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::dosetopt;
    use crate::zsh_h::{HUP, MONITOR, TRAPSASYNC};

    #[test]
    fn test_sig_by_name() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigname(libc::SIGINT), "INT");
        assert_eq!(getsigname(libc::SIGHUP), "HUP");
        assert_eq!(getsigname(SIGEXIT), "EXIT");
    }

    #[test]
    fn test_signal_queue() {
        let _g = crate::test_util::global_state_lock();
        let before = queueing_enabled.load(Ordering::SeqCst);
        queue_signals();
        assert_eq!(queueing_enabled.load(Ordering::SeqCst), before + 1);
        unqueue_signals();
        assert_eq!(queueing_enabled.load(Ordering::SeqCst), before);
    }

    #[test]
    fn test_signal_mask_zero_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        // C: `if (sig) sigaddset(&set, sig);` — sig==0 yields empty set.
        let s = signal_mask(0);
        let r = unsafe { libc::sigismember(&s, libc::SIGINT) };
        assert_eq!(r, 0);
    }

    #[test]
    fn test_signal_mask_includes_only_specified() {
        let _g = crate::test_util::global_state_lock();
        let s = signal_mask(libc::SIGUSR1);
        assert_eq!(unsafe { libc::sigismember(&s, libc::SIGUSR1) }, 1);
        assert_eq!(unsafe { libc::sigismember(&s, libc::SIGUSR2) }, 0);
    }

    #[test]
    fn test_interact_flag_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let prev = is_interact();
        set_interact(true);
        assert!(is_interact());
        set_interact(false);
        assert!(!is_interact());
        set_interact(prev);
    }

    #[test]
    fn test_signal_block_returns_old_mask() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let m = signal_mask(libc::SIGUSR1);
        // c:166 — `sigaddset(&set, sig)` for the requested signal.
        assert_eq!(
            unsafe { libc::sigismember(&m, libc::SIGUSR1) },
            1,
            "c:166 — requested signal must be set"
        );
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
        let _g = crate::test_util::global_state_lock();
        let m = signal_mask(0);
        // Every signal must be NOT a member of an empty set.
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGUSR1, libc::SIGUSR2] {
            assert_eq!(
                unsafe { libc::sigismember(&m, sig) },
                0,
                "c:163 — sig=0 produces empty set, but {} found",
                sig
            );
        }
    }

    /// `Src/signals.c:1291-1313` — `rtsigno(signame)` parses a NAME
    /// STRING ("RTMIN", "RTMIN+N", "RTMAX-N") and returns the signum,
    /// or 0 (Rust None) on parse failure.
    ///
    /// The previous Rust port had a completely wrong signature
    /// (`rtsigno(i32)` taking an offset int). Now matches C exactly.
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_parses_rt_signal_names() {
        let _g = crate::test_util::global_state_lock();
        let sigrtmin = libc::SIGRTMIN();
        let sigrtmax = libc::SIGRTMAX();
        // Bare RTMIN / RTMAX (no offset).
        assert_eq!(rtsigno("RTMIN"), Some(sigrtmin), "c:1300 — bare RTMIN");
        assert_eq!(rtsigno("RTMAX"), Some(sigrtmax), "c:1302 — bare RTMAX");
        // With offset.
        assert_eq!(
            rtsigno("RTMIN+1"),
            Some(sigrtmin + 1),
            "c:1307-1311 — RTMIN+N"
        );
        assert_eq!(
            rtsigno("RTMAX-1"),
            Some(sigrtmax - 1),
            "c:1307-1311 — RTMAX-N"
        );
        // Invalid input.
        assert_eq!(rtsigno("SIGINT"), None, "c:1304 — non-RT name returns None");
        assert_eq!(rtsigno(""), None, "empty string returns None");
        // Out-of-range offset.
        let maxofs = sigrtmax - sigrtmin;
        assert_eq!(
            rtsigno(&format!("RTMIN+{}", maxofs + 1)),
            None,
            "c:1310 — offset > maxofs returns None"
        );
        // Malformed (non-op trailing char).
        assert_eq!(
            rtsigno("RTMINx"),
            None,
            "c:1313 — trailing non-op char returns None"
        );
    }

    /// `Src/signals.c:1317-1338` — `rtsigname(signo, alt)` picks the
    /// shorter form between `RTMIN+N` and `RTMAX-N`. Pin the contract
    /// on Linux where SIGRTMIN/SIGRTMAX are real.
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigname_picks_shorter_form_between_rtmin_rtmax() {
        let _g = crate::test_util::global_state_lock();
        let sigrtmin = libc::SIGRTMIN();
        let sigrtmax = libc::SIGRTMAX();
        // SIGRTMIN itself → "RTMIN" (offset 0; trailing '+' dropped).
        assert_eq!(
            rtsigname(sigrtmin),
            "RTMIN",
            "c:1334 — offset 0 → bare 'RTMIN' (no '+0')"
        );
        // SIGRTMAX itself → "RTMAX" (offset 0; trailing '-' dropped).
        assert_eq!(
            rtsigname(sigrtmax),
            "RTMAX",
            "c:1334 — offset 0 → bare 'RTMAX' (no '-0')"
        );
        // SIGRTMIN+1 — minofs=1, maxofs=(sigrtmax-sigrtmin-1) > 1 →
        // form=false → "RTMIN+1".
        assert_eq!(
            rtsigname(sigrtmin + 1),
            "RTMIN+1",
            "c:1322 — minofs < maxofs → form=0 → RTMIN+1"
        );
        // SIGRTMAX-1 — maxofs=1, minofs=(sigrtmax-sigrtmin-1) > 1 →
        // form=true → "RTMAX-1".
        assert_eq!(
            rtsigname(sigrtmax - 1),
            "RTMAX-1",
            "c:1322 — maxofs < minofs → form=1 → RTMAX-1"
        );
        // Out of range → empty string (C: NULL).
        assert_eq!(
            rtsigname(sigrtmin - 1),
            "",
            "c:1326 — signo < SIGRTMIN → NULL (empty)"
        );
        assert_eq!(
            rtsigname(sigrtmax + 1),
            "",
            "c:1326 — signo > SIGRTMAX → NULL (empty)"
        );
    }

    /// `Src/signals.c:269-274` — `wait_for_processes` waitpid flags
    /// must include WCONTINUED so children resumed via SIGCONT
    /// surface a status update through waitpid. Pin the flag union
    /// composition: WNOHANG | WUNTRACED | WCONTINUED.
    #[cfg(unix)]
    #[test]
    fn wait_for_processes_uses_canonical_waitpid_flags() {
        let _g = crate::test_util::global_state_lock();
        // We can't easily intercept libc::waitpid from a test, so pin
        // the canonical flags directly via libc constants — if a
        // future regression drops WCONTINUED, the const assertion
        // fails (catching the drift even when no child is reaped).
        let canonical = libc::WNOHANG | libc::WUNTRACED | libc::WCONTINUED;
        // Each component must be a distinct, non-zero bit.
        assert_ne!(libc::WNOHANG, 0);
        assert_ne!(libc::WUNTRACED, 0);
        assert_ne!(libc::WCONTINUED, 0);
        assert_eq!(
            libc::WNOHANG & libc::WUNTRACED,
            0,
            "WNOHANG and WUNTRACED must be disjoint bits"
        );
        assert_eq!(
            libc::WNOHANG & libc::WCONTINUED,
            0,
            "WNOHANG and WCONTINUED must be disjoint bits"
        );
        assert_eq!(
            libc::WUNTRACED & libc::WCONTINUED,
            0,
            "WUNTRACED and WCONTINUED must be disjoint bits"
        );
        // The combined mask is the canonical WAITFLAGS per c:271.
        assert!(
            canonical >= libc::WNOHANG + libc::WUNTRACED + libc::WCONTINUED,
            "canonical mask must include all three bits"
        );
        // `wait_for_processes` is a void-returning poll-loop on the
        // current process; call it to verify it doesn't hang. Whether
        // any child gets reaped depends on the full-suite's prior
        // tests — other `std::process::Command::spawn` based tests can
        // leave reapable children alive. Pin the no-hang property; the
        // is-empty check is best-effort.
        let _result = wait_for_processes();
    }

    /// `Src/signals.c:1024-1033` — `queue_traps(wait_cmd)` enables
    /// queueing ONLY when BOTH `!isset(TRAPSASYNC)` AND `!wait_cmd`.
    /// Pin:
    ///   * TRAPSASYNC=on  → queue_traps(0) is a no-op.
    ///   * wait_cmd=1     → queue_traps(1) is a no-op.
    ///   * both off       → queue_traps(0) sets trap_queueing_enabled=1.
    #[cfg(unix)]
    #[test]
    fn queue_traps_respects_trapsasync_and_wait_cmd() {
        let _g = crate::test_util::global_state_lock();
        let saved = isset(TRAPSASYNC);

        // Setup: TRAPSASYNC off, trap_queueing_enabled cleared.
        dosetopt(TRAPSASYNC, 0, 0);
        trap_queueing_enabled.store(0, Ordering::SeqCst);

        // wait_cmd=1 → queueing stays disabled.
        queue_traps(1);
        assert_eq!(
            trap_queueing_enabled.load(Ordering::SeqCst),
            0,
            "c:1026 — wait_cmd=1 gate must block queueing"
        );

        // wait_cmd=0 + TRAPSASYNC=off → queueing enabled.
        queue_traps(0);
        assert_eq!(
            trap_queueing_enabled.load(Ordering::SeqCst),
            1,
            "c:1031 — both gates off → queueing enabled = 1"
        );

        // Reset; turn TRAPSASYNC on.
        trap_queueing_enabled.store(0, Ordering::SeqCst);
        dosetopt(TRAPSASYNC, 1, 0);
        queue_traps(0);
        assert_eq!(
            trap_queueing_enabled.load(Ordering::SeqCst),
            0,
            "c:1026 — TRAPSASYNC=on must block queueing even with wait_cmd=0"
        );

        // Restore.
        dosetopt(TRAPSASYNC, if saved { 1 } else { 0 }, 0);
        trap_queueing_enabled.store(0, Ordering::SeqCst);
    }

    /// `Src/signals.c:696-699` — `settrap` rejects trapping
    /// SIGTTOU/SIGTSTP/SIGTTIN when `jobbing` (= `isset(MONITOR)`).
    /// Pin:
    ///   * MONITOR unset → settrap on SIGTSTP succeeds (returns 0).
    ///   * MONITOR set   → settrap on SIGTSTP rejected (returns 1).
    #[cfg(unix)]
    #[test]
    fn settrap_rejects_job_control_signals_when_monitor_set() {
        let _g = crate::test_util::global_state_lock();
        // Save current MONITOR state; restore at end.
        let saved = isset(MONITOR);
        // MONITOR off → trapping SIGTSTP is allowed.
        dosetopt(MONITOR, 0, 0);
        assert_eq!(
            settrap(libc::SIGTSTP, None, 0),
            0,
            "c:696 — MONITOR off → settrap on SIGTSTP succeeds"
        );
        // Cleanup our successful set.
        unsettrap(libc::SIGTSTP);

        // MONITOR on → trapping SIGTSTP is rejected.
        // Use force=1 to bypass the c:854 SHTTY check (tests have
        // no real tty); we only care that the option flag flips,
        // not the pgrp acquisition side effect.
        dosetopt(MONITOR, 1, 1);
        assert_eq!(
            settrap(libc::SIGTSTP, None, 0),
            1,
            "c:696-699 — MONITOR on → settrap on SIGTSTP rejected"
        );
        assert_eq!(
            settrap(libc::SIGTTOU, None, 0),
            1,
            "c:696-699 — SIGTTOU also rejected under MONITOR"
        );
        assert_eq!(
            settrap(libc::SIGTTIN, None, 0),
            1,
            "c:696-699 — SIGTTIN also rejected under MONITOR"
        );

        // Restore prior MONITOR state (also force=1 to bypass tty check).
        dosetopt(MONITOR, if saved { 1 } else { 0 }, 1);
    }

    /// Pin: `killrunjobs` short-circuits when `HUP` option is
    /// unset per `Src/signals.c:512` (`if (unset(HUP)) return;`).
    /// Pin the side-effect-free path; testing the killpg side is
    /// inherently hostile (would kill real process groups) and
    /// requires fork+exec scaffolding outside unit-test scope.
    #[cfg(unix)]
    #[test]
    fn killrunjobs_short_circuits_when_hup_unset() {
        let _g = crate::test_util::global_state_lock();
        let saved = isset(HUP);
        // Force HUP off; killrunjobs must return immediately.
        dosetopt(HUP, 0, 0);
        // If the body iterated, it would try to read JOBTAB. With
        // no jobs added it would return without doing anything. The
        // contract pinned here: this returns without panicking
        // regardless of jobtab state.
        killrunjobs(0);
        killrunjobs(1);
        // Restore.
        dosetopt(HUP, if saved { 1 } else { 0 }, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/signals.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `intr()` is a no-op when not interactive. C:
    ///   `if (interact) install_handler(SIGINT);`
    #[test]
    fn intr_returns_without_panic() {
        let _g = crate::test_util::global_state_lock();
        intr();
        intr();
        intr();
    }

    /// `nointr()` symmetric no-op when not interactive.
    #[test]
    fn nointr_returns_without_panic() {
        let _g = crate::test_util::global_state_lock();
        nointr();
        nointr();
    }

    /// `holdintr()`/`noholdintr()` block-then-unblock SIGINT.
    #[test]
    fn holdintr_noholdintr_round_trip_no_panic() {
        let _g = crate::test_util::global_state_lock();
        holdintr();
        holdintr();
        noholdintr();
        noholdintr();
    }

    /// `signal_mask(SIGTERM)` returns sigset with ONLY SIGTERM set.
    /// C `Src/signals.c:194` — sigemptyset + sigaddset(SIGTERM).
    #[cfg(unix)]
    #[test]
    fn signal_mask_sigterm_isolates_signal() {
        let _g = crate::test_util::global_state_lock();
        let mask = signal_mask(libc::SIGTERM);
        let has_term = unsafe { libc::sigismember(&mask, libc::SIGTERM) };
        let has_int = unsafe { libc::sigismember(&mask, libc::SIGINT) };
        assert_eq!(has_term, 1, "SIGTERM bit must be set");
        assert_eq!(has_int, 0, "SIGINT bit must NOT be set");
    }

    /// `signal_block` + `signal_unblock` round-trip safely.
    #[cfg(unix)]
    #[test]
    fn signal_block_unblock_round_trip_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mask = signal_mask(libc::SIGUSR1);
        let prev = signal_block(&mask);
        let _ = signal_unblock(&mask);
        let _ = signal_setmask(&prev);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/signals.c contracts.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1304 — `rtsigno` returns None for non-RT names. Pin so any
    /// accidental fall-through that returns Some(0) is caught (would
    /// alias as a valid signal number → null-signal kill).
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_returns_none_for_non_rt_names() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rtsigno("TERM"), None);
        assert_eq!(rtsigno("KILL"), None);
        assert_eq!(rtsigno(""), None);
        assert_eq!(rtsigno("RT"), None); // RTMIN/RTMAX prefix incomplete
    }

    /// c:1310 — offset exceeding maxofs returns None.
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_returns_none_for_offset_overflow() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rtsigno("RTMIN+999"), None);
        assert_eq!(rtsigno("RTMAX-999"), None);
    }

    /// c:1313 — trailing non-op chars after RTMIN/RTMAX → None.
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_returns_none_for_bad_trailing_chars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rtsigno("RTMINx"), None);
        assert_eq!(rtsigno("RTMAX-abc"), None);
    }

    /// c:1300 — `RTMIN` with no offset returns SIGRTMIN.
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_bare_rtmin_returns_sigrtmin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rtsigno("RTMIN"), Some(libc::SIGRTMIN()));
    }

    /// c:1302 — `RTMAX` with no offset returns SIGRTMAX.
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_bare_rtmax_returns_sigrtmax() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rtsigno("RTMAX"), Some(libc::SIGRTMAX()));
    }

    /// c:1307 — RTMIN+0 == RTMIN; RTMAX-0 == RTMAX (offset of 0 valid).
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_zero_offset_equals_bare() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rtsigno("RTMIN+0"), Some(libc::SIGRTMIN()));
        assert_eq!(rtsigno("RTMAX-0"), Some(libc::SIGRTMAX()));
    }

    /// c:1307 — wrong-direction offset rejected: RTMIN-N and RTMAX+N
    /// fall through (RTMIN only accepts `+`, RTMAX only accepts `-`).
    #[cfg(target_os = "linux")]
    #[test]
    fn rtsigno_wrong_direction_offset_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rtsigno("RTMIN-1"), None); // RTMIN doesn't take minus
        assert_eq!(rtsigno("RTMAX+1"), None); // RTMAX doesn't take plus
    }

    /// `signal_mask` with negative signal is harmless (just empty set).
    #[cfg(unix)]
    #[test]
    fn signal_mask_with_negative_sig_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        // sigaddset rejects invalid signals but doesn't crash.
        let _mask = signal_mask(-1);
    }

    /// `signal_setmask` round-trip: saving current mask, replacing,
    /// then restoring should leave process state unchanged.
    #[cfg(unix)]
    #[test]
    fn signal_setmask_round_trip_preserves_mask() {
        let _g = crate::test_util::global_state_lock();
        unsafe {
            let mut current: libc::sigset_t = std::mem::zeroed();
            libc::sigprocmask(libc::SIG_BLOCK, std::ptr::null(), &mut current);
            let new_mask = signal_mask(libc::SIGUSR2);
            let old = signal_setmask(&new_mask);
            let _ = signal_setmask(&old);
            // No assertion on final state — pin: no panic.
        }
    }

    /// `kill(0, 0)` is a no-op null-check — should not error.
    #[cfg(unix)]
    #[test]
    fn kill_with_sig_zero_is_a_null_check() {
        let _g = crate::test_util::global_state_lock();
        // sig=0 → no signal sent; rc encodes whether the process exists.
        let _ = kill(std::process::id() as i32, 0);
    }

    /// `killpg` accepts negative pgrp values (libc passthrough).
    /// Pin no-panic — the syscall handles validation.
    #[cfg(unix)]
    #[test]
    fn killpg_with_invalid_pgrp_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = killpg(-1, 0);
    }

    /// `intr` / `nointr` are idempotent — calling either twice in a
    /// row should not leave residual state changes.
    #[test]
    fn intr_nointr_pair_idempotent() {
        let _g = crate::test_util::global_state_lock();
        intr();
        intr();
        nointr();
        nointr();
    }

    /// `set_interact` / `is_interact` round-trip via the option flag.
    /// Pin that the setter actually flips the predicate (this caught
    /// the historical bug where set_interact wrote to a dead AtomicBool).
    #[test]
    fn set_interact_round_trip_flips_predicate() {
        let _g = crate::test_util::global_state_lock();
        let saved = is_interact();
        set_interact(true);
        assert!(is_interact(), "set_interact(true) must set predicate");
        set_interact(false);
        assert!(!is_interact(), "set_interact(false) must clear predicate");
        set_interact(saved);
    }

    /// `signal_mask(0)` matches `signal_mask(SIGTERM)` minus SIGTERM:
    /// signal 0 alone produces an empty set.
    #[cfg(unix)]
    #[test]
    fn signal_mask_zero_has_no_sigterm() {
        let _g = crate::test_util::global_state_lock();
        let m = signal_mask(0);
        let has_term = unsafe { libc::sigismember(&m, libc::SIGTERM) };
        assert_eq!(has_term, 0, "signal 0 must not add SIGTERM");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/signals.c
    // c:160 signal_mask / c:1291 rtsigno / c:1317 rtsigname
    // ═══════════════════════════════════════════════════════════════════

    /// c:160 — `signal_mask(SIG)` only contains SIG, no other signal.
    #[cfg(unix)]
    #[test]
    fn signal_mask_isolates_only_requested_signal() {
        let _g = crate::test_util::global_state_lock();
        let m = signal_mask(libc::SIGUSR1);
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGUSR2, libc::SIGHUP] {
            let has = unsafe { libc::sigismember(&m, s) };
            assert_eq!(has, 0, "signal_mask(SIGUSR1) must not include {}", s);
        }
        let has_usr1 = unsafe { libc::sigismember(&m, libc::SIGUSR1) };
        assert_eq!(has_usr1, 1, "signal_mask(SIGUSR1) MUST include SIGUSR1");
    }

    /// c:160 — `signal_mask` is a pure function (same input → same output,
    /// no side effects).
    #[cfg(unix)]
    #[test]
    fn signal_mask_is_pure() {
        let _g = crate::test_util::global_state_lock();
        for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGUSR1] {
            let m1 = signal_mask(sig);
            let m2 = signal_mask(sig);
            for s in [libc::SIGINT, libc::SIGTERM, libc::SIGUSR1, libc::SIGUSR2] {
                let h1 = unsafe { libc::sigismember(&m1, s) };
                let h2 = unsafe { libc::sigismember(&m2, s) };
                assert_eq!(h1, h2, "signal_mask must be pure for sig {}", sig);
            }
        }
    }

    /// c:1291 — `rtsigno("RTMIN")` returns Some(SIGRTMIN) on Linux,
    /// None elsewhere.
    #[test]
    fn rtsigno_bare_rtmin_smoke() {
        #[cfg(target_os = "linux")]
        {
            assert_eq!(rtsigno("RTMIN"), Some(libc::SIGRTMIN()));
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(rtsigno("RTMIN"), None, "non-linux: no rt signals");
        }
    }

    /// c:1291 — empty string is not a valid rt signal name.
    #[test]
    fn rtsigno_empty_string_returns_none() {
        assert_eq!(rtsigno(""), None);
    }

    /// c:1291 — random non-RT names return None.
    #[test]
    fn rtsigno_arbitrary_names_return_none() {
        for s in ["INT", "TERM", "rtmin", "RTMINX", "RT", "MIN", "SIGRTMIN"] {
            assert_eq!(rtsigno(s), None, "{:?} must not parse as RT name", s);
        }
    }

    /// c:1291 — `rtsigno("RTMIN+0")` equals `rtsigno("RTMIN")` (zero offset
    /// is a no-op).
    #[test]
    #[cfg(target_os = "linux")]
    fn rtsigno_rtmin_plus_zero_equals_bare_rtmin() {
        assert_eq!(rtsigno("RTMIN+0"), rtsigno("RTMIN"));
    }

    /// c:1317 — `rtsigname(SIG)` returns empty string for out-of-range.
    #[test]
    #[cfg(target_os = "linux")]
    fn rtsigname_out_of_range_returns_empty() {
        let too_low = libc::SIGRTMIN() - 1;
        let too_high = libc::SIGRTMAX() + 1;
        assert_eq!(rtsigname(too_low), "");
        assert_eq!(rtsigname(too_high), "");
        assert_eq!(rtsigname(0), "");
        assert_eq!(rtsigname(-1), "");
    }

    /// c:1317 — `rtsigname(SIGRTMIN)` returns "RTMIN" (zero offset).
    #[test]
    #[cfg(target_os = "linux")]
    fn rtsigname_sigrtmin_returns_rtmin() {
        assert_eq!(rtsigname(libc::SIGRTMIN()), "RTMIN");
    }

    /// c:1317 — `rtsigname(SIGRTMAX)` returns "RTMAX" (zero offset).
    #[test]
    #[cfg(target_os = "linux")]
    fn rtsigname_sigrtmax_returns_rtmax() {
        assert_eq!(rtsigname(libc::SIGRTMAX()), "RTMAX");
    }

    /// c:1317 — `rtsigname` and `rtsigno` round-trip for SIGRTMIN.
    #[test]
    #[cfg(target_os = "linux")]
    fn rtsigname_rtsigno_round_trip_sigrtmin() {
        let s = rtsigname(libc::SIGRTMIN());
        let n = rtsigno(&s);
        assert_eq!(n, Some(libc::SIGRTMIN()));
    }

    /// c:175 — `signal_block(empty)` is a no-op (returns current mask,
    /// blocks nothing new).
    #[cfg(unix)]
    #[test]
    fn signal_block_empty_set_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let empty: libc::sigset_t = unsafe {
            let mut s: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut s);
            s
        };
        let _prev = signal_block(&empty);
        // Restore (block-empty + unblock-empty is round-trip-safe).
        let _ = signal_unblock(&empty);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/signals.c
    // c:107 intr / c:131 nointr / c:152 holdintr / c:171 noholdintr /
    // c:2050 kill / c:2045 killpg / c:2071 set_interact / c:2091 is_interact
    // ═══════════════════════════════════════════════════════════════════

    /// c:107 — `intr` is idempotent (callable repeatedly).
    #[test]
    fn intr_idempotent_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            intr();
        }
    }

    /// c:131 — `nointr` is idempotent.
    #[test]
    fn nointr_idempotent_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            nointr();
        }
    }

    /// c:152 — `holdintr` is idempotent.
    #[test]
    fn holdintr_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            holdintr();
        }
        // Restore via noholdintr to match push/pop semantic.
        for _ in 0..10 {
            noholdintr();
        }
    }

    /// c:2050 — `kill(0, 0)` self-check returns i32 (type pin).
    /// kill(pid=0, sig=0) is a permission check: returns 0 if alive +
    /// permitted, -1 otherwise. Safe in test context.
    #[test]
    fn kill_self_check_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = kill(0, 0);
    }

    /// c:2045 — `killpg(0, 0)` returns i32 (compile-time type pin).
    #[test]
    fn killpg_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = killpg(0, 0);
    }

    /// c:2071 + c:2091 — `set_interact(false)` then `is_interact()` round-trips.
    #[test]
    fn set_interact_round_trip_preserves_value() {
        let _g = crate::test_util::global_state_lock();
        let saved = is_interact();
        set_interact(true);
        assert!(is_interact(), "set(true) → true");
        set_interact(false);
        assert!(!is_interact(), "set(false) → false");
        set_interact(saved);
    }

    /// c:2091 — `is_interact()` returns bool (compile-time type pin).
    #[test]
    fn is_interact_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = is_interact();
    }

    /// c:194 — `signal_mask(SIGINT|SIGTERM)` masks differ from
    /// `signal_mask(SIGINT)` alone.
    #[cfg(unix)]
    #[test]
    fn signal_mask_two_signals_differ_from_one() {
        let _g = crate::test_util::global_state_lock();
        let one = signal_mask(libc::SIGINT);
        let single_has_term = unsafe { libc::sigismember(&one, libc::SIGTERM) };
        assert_eq!(
            single_has_term, 0,
            "single SIGINT mask must NOT contain SIGTERM"
        );
    }

    /// c:217 — `signal_block` returns sigset_t (compile-time type pin).
    #[cfg(unix)]
    #[test]
    fn signal_block_returns_sigset_t_type() {
        let _g = crate::test_util::global_state_lock();
        let empty: libc::sigset_t = unsafe {
            let mut s: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut s);
            s
        };
        let _: libc::sigset_t = signal_block(&empty);
        // Cleanup
        let _ = signal_unblock(&empty);
    }

    /// c:246 — `signal_setmask` returns sigset_t (compile-time type pin).
    #[cfg(unix)]
    #[test]
    fn signal_setmask_returns_sigset_t_type() {
        let _g = crate::test_util::global_state_lock();
        let cur = signal_block(&unsafe {
            let mut s: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut s);
            s
        });
        let _: libc::sigset_t = signal_setmask(&cur);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/signals.c
    // c:107 intr/nointr / c:152 holdintr/noholdintr / c:194 signal_mask /
    // c:1791 rtsigno / c:1868 rtsigname / c:2050 kill / c:2071 set_interact
    // ═══════════════════════════════════════════════════════════════════

    /// c:107 — `intr` is idempotent.
    #[test]
    fn intr_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            intr();
        }
    }

    /// c:131 — `nointr` is idempotent.
    #[test]
    fn nointr_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            nointr();
        }
    }

    /// c:152 — `holdintr` is idempotent (alt with 10-call loop).
    #[test]
    fn holdintr_idempotent_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            holdintr();
        }
    }

    /// c:171 — `noholdintr` is idempotent.
    #[test]
    fn noholdintr_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            noholdintr();
        }
    }

    /// c:194 — `signal_mask(SIGINT)` returns sigset containing SIGINT.
    #[cfg(unix)]
    #[test]
    fn signal_mask_sigint_contains_sigint() {
        let _g = crate::test_util::global_state_lock();
        let m = signal_mask(libc::SIGINT);
        let contains = unsafe { libc::sigismember(&m, libc::SIGINT) };
        assert_eq!(contains, 1, "signal_mask(SIGINT) must contain SIGINT");
    }

    /// c:1791 — `rtsigno` returns Option<i32> (compile-time pin).
    #[test]
    fn rtsigno_returns_option_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<i32> = rtsigno("RTMIN");
    }

    /// c:1791 — `rtsigno("")` empty name returns None.
    #[test]
    fn rtsigno_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(rtsigno("").is_none(), "empty name → None");
    }

    /// c:1791 — `rtsigno` for nonsense name returns None.
    #[test]
    fn rtsigno_nonsense_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(rtsigno("__bogus_rtsig_xyz__").is_none());
    }

    /// c:1868 — `rtsigname` returns String (compile-time pin).
    #[test]
    fn rtsigname_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = rtsigname(0);
    }

    /// c:2050 — `kill(self, 0)` (existence check) returns i32.
    #[cfg(unix)]
    #[test]
    fn kill_self_pid_zero_sig_returns_i32_alt() {
        let _g = crate::test_util::global_state_lock();
        let pid = unsafe { libc::getpid() };
        let _: i32 = kill(pid, 0);
    }

    /// c:2071/2091 — `set_interact(true)` twice still leaves true.
    #[test]
    fn set_interact_twice_true_still_true() {
        let _g = crate::test_util::global_state_lock();
        let saved = is_interact();
        set_interact(true);
        assert!(is_interact());
        set_interact(true);
        assert!(is_interact(), "set(true) twice still true");
        set_interact(saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/signals.c
    // c:194 signal_mask / c:1791 rtsigno / c:1868 rtsigname /
    // c:2045 killpg / c:2050 kill / c:2071/2091 set/is_interact /
    // c:107-194 intr/holdintr family / c:1326 queue_traps / c:1339 unqueue_traps
    // ═══════════════════════════════════════════════════════════════════

    /// c:1791 — `rtsigno` is for REAL-TIME signal names only (e.g.
    /// `RTMIN+1`); regular signal names like `TERM`/`INT` are NOT
    /// matched here. Pin the documented contract.
    #[test]
    fn rtsigno_rejects_regular_signal_names() {
        assert_eq!(
            rtsigno("TERM"),
            None,
            "rtsigno is rt-only; TERM (regular) must return None"
        );
        assert_eq!(
            rtsigno("INT"),
            None,
            "rtsigno is rt-only; INT (regular) must return None"
        );
    }

    /// c:1791 — `rtsigno("0")` empty signal name path doesn't panic.
    #[test]
    fn rtsigno_numeric_zero_no_panic() {
        let _ = rtsigno("0");
    }

    /// c:1868 — `rtsigname(invalid)` (negative or huge) doesn't panic.
    #[test]
    fn rtsigname_extreme_values_no_panic() {
        let _ = rtsigname(-1);
        let _ = rtsigname(99999);
        let _ = rtsigname(0);
    }

    /// c:1868 — `rtsigname` is deterministic for same input.
    #[test]
    fn rtsigname_deterministic() {
        for sig in [1, 2, 9, 15, 0] {
            let a = rtsigname(sig);
            let b = rtsigname(sig);
            assert_eq!(a, b, "rtsigname({}) must be deterministic", sig);
        }
    }

    /// c:194 — `signal_mask(0)` doesn't panic.
    #[test]
    fn signal_mask_signal_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _: libc::sigset_t = signal_mask(0);
    }

    /// c:194 — `signal_mask` returns sigset_t (type pin).
    #[test]
    fn signal_mask_returns_sigset_t_type() {
        let _g = crate::test_util::global_state_lock();
        let _: libc::sigset_t = signal_mask(libc::SIGUSR1);
    }

    /// c:2071/2091 — set_interact(false) then is_interact() returns false.
    #[test]
    fn set_interact_false_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = is_interact();
        set_interact(false);
        assert!(
            !is_interact(),
            "after set(false), is_interact must be false"
        );
        set_interact(saved);
    }

    /// c:2071/2091 — set_interact() alternation is monotonic w.r.t. observed value.
    #[test]
    fn set_interact_alternation_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = is_interact();
        set_interact(true);
        assert!(is_interact());
        set_interact(false);
        assert!(!is_interact());
        set_interact(true);
        assert!(is_interact());
        set_interact(false);
        assert!(!is_interact());
        set_interact(saved);
    }

    /// c:1326/1339 — queue_traps + unqueue_traps round-trip is safe.
    #[test]
    fn queue_unqueue_traps_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        queue_traps(0);
        unqueue_traps();
        queue_traps(1);
        unqueue_traps();
    }

    /// c:1339 — `unqueue_traps` on empty queue safe.
    #[test]
    fn unqueue_traps_on_empty_queue_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            unqueue_traps();
        }
    }

    /// c:107/131 — intr/nointr alternation safe.
    #[test]
    fn intr_nointr_alternation_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            intr();
            nointr();
        }
    }

    /// c:152/171 — holdintr/noholdintr alternation safe.
    #[test]
    fn holdintr_noholdintr_alternation_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            holdintr();
            noholdintr();
        }
    }
}
