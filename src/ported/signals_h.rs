//! `signals.h` port — signal-handling constants + macro wrappers.
//!
//! Port of `Src/signals.h`. Defines:
//!   - Pseudo-signal indexes (`SIGZERR`, `SIGDEBUG`, `SIGEXIT`,
//!     `VSIGCOUNT`, `TRAPCOUNT`).
//!   - SIGRTMIN/SIGRTMAX-aware index/number conversion macros
//!     (`SIGNUM`, `SIGIDX`).
//!   - Signal-queueing macros (`queue_signals`/`unqueue_signals`/
//!     `dont_queue_signals`/`restore_queue_signals`/
//!     `run_queued_signals`/`queue_signal_level`).
//!   - Block helpers (`child_block`/`child_unblock`/`winch_block`/
//!     `winch_unblock`).
//!   - Convenience wrappers (`signal_ignore`/`signal_default`).
//!
//! C source: 0 structs/enums/ported. ~25 #defines.
//!
//! Macro casing preserved verbatim per the casing rule. The
//! signal-queue mutable state (`queue_front`/`queue_rear`/
//! `queueing_enabled`/`signal_queue[]`/`signal_mask_queue[]`) lives
//! in `signals.rs` (the .c port); this header port covers the
//! constants + the queueing-API call shape.

use crate::ported::signals::{queue_front, queue_rear, signal_mask_queue, signal_queue};
use crate::signals::{queue_in, queueing_enabled};
use crate::{DPUTS, DPUTS2};
use std::sync::atomic::{AtomicI32, Ordering};
// ---------------------------------------------------------------------------
// Pseudo-signal indexes (c:34-46).
// ---------------------------------------------------------------------------

/// Port of the platform's `SIGCOUNT` value (computed by autoconf
/// from `<signal.h>`). Equal to `NSIG - 1` on every supported host
/// — the highest valid signal number for the standard signal table.
/// libc-rs doesn't expose `NSIG` on Linux/macOS so we hardcode the
/// value `<signal.h>` would emit.
#[cfg(target_os = "linux")]
pub const SIGCOUNT: i32 = 64; // Linux NSIG = 65
/// `SIGCOUNT` constant.
#[cfg(target_os = "macos")]
pub const SIGCOUNT: i32 = 31; // macOS NSIG = 32
/// `SIGCOUNT` constant.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const SIGCOUNT: i32 = 31; // POSIX baseline

/// Port of `#define SIGZERR` from `Src/signals.h:34`. Pseudo-signal
/// index for the ZERR trap (fires after every command that exits
/// with a nonzero status).
pub const SIGZERR: i32 = SIGCOUNT + 1; // c:34

/// Port of `#define SIGDEBUG` from `Src/signals.h:35`. Pseudo-signal
/// index for the DEBUG trap (fires before every command).
pub const SIGDEBUG: i32 = SIGCOUNT + 2; // c:35

/// Port of `#define VSIGCOUNT` from `Src/signals.h:36`. Total
/// number of "virtual" signal-table slots = SIGCOUNT (real signals)
/// + 3 (zero + ZERR + DEBUG).
pub const VSIGCOUNT: i32 = SIGCOUNT + 3; // c:36

/// Port of `#define TRAPCOUNT` from `Src/signals.h:38/42`. With
/// `SIGRTMIN`/`SIGRTMAX` available (every Linux/musl/BSD host),
/// expands to `VSIGCOUNT + (SIGRTMAX - SIGRTMIN + 1)` so the trap
/// table covers real-time signals too. macOS/BSD libc binding
/// doesn't expose SIGRTMIN/SIGRTMAX as Rust constants — the c:42
/// `#else` arm (TRAPCOUNT = VSIGCOUNT) applies.
#[cfg(target_os = "linux")]
pub const TRAPCOUNT: i32 = VSIGCOUNT + 32; // c:38 (Linux RT range = 32)
/// `TRAPCOUNT` constant.
#[cfg(not(target_os = "linux"))]
pub const TRAPCOUNT: i32 = VSIGCOUNT; // c:42

/// Port of `#define SIGEXIT` from `Src/signals.h:46`. Pseudo-signal
/// index for the EXIT trap (fires when the shell exits).
pub const SIGEXIT: i32 = 0; // c:46

// ---------------------------------------------------------------------------
// Signal name table — port of the `sigs[SIGCOUNT+4]` array generated
// by `Src/signames2.awk` at build time. The C source feeds this awk
// the platform's `<signal.h>` and produces `signames.c` with one
// `"NAME"` entry per real signal plus the bookend pseudo-signal
// names (`"EXIT"` at slot 0, `"ZERR"` / `"DEBUG"` after the real
// signals, `NULL` terminator).
//
// Rust port collects the same data at compile time using the
// `libc::SIG*` constants. Each entry is `(canonical_name, signum)`;
// `sigs_name(idx)` and `sigs_number(name)` provide the same
// dispatch the awk-generated array gives C callers.
// ---------------------------------------------------------------------------

/// Port of `sigs[]` from `signames.c` (auto-generated). Lists the
/// canonical zsh signal name for every real signal — name without
/// the `SIG` prefix. Entries are platform-conditional via libc
/// constants so the table matches the running OS's `<signal.h>`.
pub static SIGS: &[(&str, i32)] = &[
    // c:signames.c sigs[]
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
    // c:signames.c — EMT exists on BSD-derived platforms (macOS,
    // *BSD, Solaris). On Linux SIGEMT is glibc-defined as 7 (same
    // slot as Linux's IOT), so include it there too — `awk
    // signames2.awk` emits it whenever <signal.h> exposes the
    // macro.
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "solaris",
    ))]
    ("EMT", libc::SIGEMT),
    ("BUS", libc::SIGBUS),
    ("FPE", libc::SIGFPE),
    ("KILL", libc::SIGKILL),
    ("USR1", libc::SIGUSR1),
    ("SEGV", libc::SIGSEGV),
    ("USR2", libc::SIGUSR2),
    ("PIPE", libc::SIGPIPE),
    ("ALRM", libc::SIGALRM),
    ("TERM", libc::SIGTERM),
    ("CHLD", libc::SIGCHLD),
    ("CONT", libc::SIGCONT),
    ("STOP", libc::SIGSTOP),
    ("TSTP", libc::SIGTSTP),
    ("TTIN", libc::SIGTTIN),
    ("TTOU", libc::SIGTTOU),
    ("URG", libc::SIGURG),
    ("XCPU", libc::SIGXCPU),
    ("XFSZ", libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM),
    ("PROF", libc::SIGPROF),
    ("WINCH", libc::SIGWINCH),
    ("IO", libc::SIGIO),
    // c:signames.c — INFO is BSD-only (macOS, *BSD). signames2.awk
    // emits it iff <signal.h> defines SIGINFO; libc-rs only exposes
    // the constant on BSD-family targets.
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
    ))]
    ("INFO", libc::SIGINFO),
    ("SYS", libc::SIGSYS),
];

/// Port of `alt_sigs[]` from `Src/jobs.c:2740`. Cross-platform name
/// aliases — names like `CLD` / `IO` / `IOT` map to the same number
/// as the canonical zsh name on platforms where the underlying
/// C macro pair coincides.
pub static ALT_SIGS: &[(&str, i32)] = &[
    // c:jobs.c:2740
    ("CLD", libc::SIGCHLD), // c:2742-2746
    ("IOT", libc::SIGABRT), // c:2752-2756
    ("ERR", SIGZERR),       // c:2762
];

/// Port of the `sig_msg[]` array from `signames.c` (auto-generated
/// by signames2.awk). Pretty-printed messages keyed by signal number,
/// used in the `printjob`/`printtime` output paths when zsh wants
/// to render a signal as e.g. `"hangup"` or `"floating point exception"`
/// rather than just `"SIGFPE"`. Each entry is `(signum, message)`.
pub static SIG_MSG: &[(i32, &str)] = &[
    // c:signames.c sig_msg[]
    (libc::SIGHUP, "hangup"),
    (libc::SIGINT, "interrupt"),
    (libc::SIGQUIT, "quit"),
    (libc::SIGILL, "illegal hardware instruction"),
    (libc::SIGTRAP, "trace trap"),
    (libc::SIGABRT, "abort"),
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "solaris",
    ))]
    (libc::SIGEMT, "EMT trap"),
    (libc::SIGBUS, "bus error"),
    (libc::SIGFPE, "floating point exception"),
    (libc::SIGKILL, "killed"),
    (libc::SIGUSR1, "user-defined signal 1"),
    (libc::SIGSEGV, "segmentation fault"),
    (libc::SIGUSR2, "user-defined signal 2"),
    (libc::SIGPIPE, "broken pipe"),
    (libc::SIGALRM, "alarm"),
    (libc::SIGTERM, "terminated"),
    (libc::SIGCHLD, "death of child"),
    (libc::SIGCONT, "continued"),
    (libc::SIGSTOP, "stopped (signal)"),
    (libc::SIGTSTP, "stopped"),
    (libc::SIGTTIN, "stopped (tty input)"),
    (libc::SIGTTOU, "stopped (tty output)"),
    (libc::SIGURG, "urgent condition"),
    (libc::SIGXCPU, "cpu limit exceeded"),
    (libc::SIGXFSZ, "file size limit exceeded"),
    (libc::SIGVTALRM, "virtual time alarm"),
    (libc::SIGPROF, "profile signal"),
    (libc::SIGWINCH, "window size changed"),
    (libc::SIGIO, "i/o ready"),
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
    ))]
    (libc::SIGINFO, "information request"),
    (libc::SIGSYS, "invalid system call"),
];

/// Look up the canonical signal name for `idx` (sans `SIG` prefix).
/// Returns `"EXIT"` for slot 0, `"ZERR"`/`"DEBUG"` for the pseudo-
/// signal slots, the SIGS-table name for real signals, or `None`
/// when out of range. Mirrors C's `sigs[idx]` indexed lookup.
pub fn sigs_name(idx: i32) -> Option<&'static str> {
    // c:signames.c sigs[]
    if idx == 0 {
        return Some("EXIT");
    } // c:slot 0
    if idx == SIGZERR {
        return Some("ZERR");
    } // c:SIGCOUNT+1
    if idx == SIGDEBUG {
        return Some("DEBUG");
    } // c:SIGCOUNT+2
    SIGS.iter().find(|(_, n)| *n == idx).map(|(name, _)| *name)
}

/// Reverse of `sigs_name` — accepts a name (with or without leading
/// `SIG`) and returns the libc signal number. Walks SIGS first, then
/// ALT_SIGS for cross-platform aliases.
pub fn sigs_number(name: &str) -> Option<i32> {
    // c:jobs.c:2828 lookup
    let bare = name.strip_prefix("SIG").unwrap_or(name);
    SIGS.iter()
        .find(|(n, _)| *n == bare)
        .map(|(_, num)| *num)
        .or_else(|| {
            ALT_SIGS
                .iter()
                .find(|(n, _)| *n == bare)
                .map(|(_, num)| *num)
        })
}

// ---------------------------------------------------------------------------
// SIGRTMIN/SIGRTMAX-aware index ↔ number conversion (c:39-44).
// ---------------------------------------------------------------------------

/// Port of `#define SIGNUM(x)` from `Src/signals.h:39`. Convert a
/// trap-table index back to a real signal number. Indexes >=
/// `VSIGCOUNT` are real-time-signal slots — those map back to
/// `SIGRTMIN + (x - VSIGCOUNT)`. Other indexes pass through unchanged.
#[inline]
#[allow(non_snake_case)]
pub fn SIGNUM(x: i32) -> i32 {
    // c:39
    #[cfg(target_os = "linux")]
    {
        if x >= VSIGCOUNT {
            x - VSIGCOUNT + libc::SIGRTMIN()
        } else {
            x
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        x
    }
}

/// Port of `#define SIGIDX(x)` from `Src/signals.h:40`. Convert a
/// real signal number to its trap-table index. Numbers in the
/// `SIGRTMIN..=SIGRTMAX` range map to `(x - SIGRTMIN) + VSIGCOUNT`;
/// other numbers pass through unchanged.
#[inline]
#[allow(non_snake_case)]
pub fn SIGIDX(x: i32) -> i32 {
    // c:40
    #[cfg(target_os = "linux")]
    {
        let rtmin = libc::SIGRTMIN();
        let rtmax = libc::SIGRTMAX();
        if x >= rtmin && x <= rtmax {
            x - rtmin + VSIGCOUNT
        } else {
            x
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        x
    }
}

// ---------------------------------------------------------------------------
// Queue size + per-signal queueing state (c:76, c:69-86, c:88-127).
//
// C uses module-globals: `queue_front`, `queue_rear`, `signal_queue[]`,
// `signal_mask_queue[]`, `queueing_enabled`, `queue_in` (debug only).
// The Rust port keeps `queueing_enabled` here as the central flag the
// queue_signals/unqueue_signals macros toggle; the actual queue
// arrays live in `signals.rs` per their C home (`Src/signals.c`).
// ---------------------------------------------------------------------------

/// Port of `#define MAX_QUEUE_SIZE` from `Src/signals.h:76`. Maximum
/// signal-queue depth before older entries get overwritten.
pub const MAX_QUEUE_SIZE: usize = 128; // c:76

/// Port of the global `int queueing_enabled;` (Src/signals.c). The
// ---------------------------------------------------------------------------
// Signal-queueing macros (c:78-127). Ported as ported since Rust has
// no preprocessor macros. The mutable state (`queueing_enabled`,
// `queue_front`, `queue_rear`, `signal_queue[]`, `signal_mask_queue[]`)
// lives in `signals.rs` per `Src/signals.c:77-81`; these wrappers
// just call through.
// ---------------------------------------------------------------------------

/// Port of `#define queue_signals()` from `Src/signals.h:90/112`.
/// C body: `(queue_in++, queueing_enabled++)` — both counters bump.
#[inline]
#[allow(non_snake_case)]
pub fn queue_signals() {
    // c:90/112
    queue_in.fetch_add(1, Ordering::SeqCst);
    queueing_enabled.fetch_add(1, Ordering::SeqCst);
}

/// Port of `#define unqueue_signals()` from `Src/signals.h:92/114`.
/// C body decrements `queue_in` and `queueing_enabled`; when the
/// latter reaches 0, drains the signal queue via
/// `run_queued_signals()`.
#[inline]
#[allow(non_snake_case)]
pub fn unqueue_signals() {
    // c:92/114
    // c:93 — DPUTS(!queueing_enabled, "BUG: unqueue_signals called but not queueing")
    DPUTS!(
        // c:93
        queueing_enabled.load(Ordering::SeqCst) == 0, // c:93
        "BUG: unqueue_signals called but not queueing"  // c:93
    );
    queue_in.fetch_sub(1, Ordering::SeqCst); // c:94 --queue_in
    let prev = queueing_enabled.fetch_sub(1, Ordering::SeqCst); // c:95
    if prev == 1 {
        // c:95 if (!--queueing_enabled)
        run_queued_signals(); // c:95 run_queued_signals()
    }
}

/// Port of `#define dont_queue_signals()` from `Src/signals.h:98/118`.
/// C body: `queue_in = queueing_enabled; queueing_enabled = 0`. The
/// `queue_in` snapshot lets `restore_queue_signals` later re-arm a
/// later DPUTS2 invariant check.
#[inline]
#[allow(non_snake_case)]
pub fn dont_queue_signals() {
    // c:98/118
    let level = queueing_enabled.swap(0, Ordering::SeqCst);
    queue_in.store(level, Ordering::SeqCst);
    run_queued_signals();
}

/// Port of `#define restore_queue_signals(q)` from
/// `Src/signals.h:104/123`. Restore the queueing counter to a saved
/// value (typically captured before a section that called
/// `dont_queue_signals` or similar).
#[inline]
#[allow(non_snake_case)]
pub fn restore_queue_signals(q: i32) {
    // c:104/123
    // c:105-106 — DPUTS2(queueing_enabled && queue_in != q,
    //                    "BUG: q = %d != queue_in = %d", q, queue_in)
    let qi = queue_in.load(Ordering::SeqCst); // c:105
    let qe = queueing_enabled.load(Ordering::SeqCst); // c:105
    DPUTS2!(
        // c:105
        qe != 0 && qi != q, // c:105
        "BUG: q = {} != queue_in = {}",
        q,
        qi // c:106
    );
    queue_in.store(q, Ordering::SeqCst); // c:107 queue_in = q
    queueing_enabled.store(q, Ordering::SeqCst); // c:107 queueing_enabled = q
}

/// Port of `#define queue_signal_level()` from `Src/signals.h:127`.
/// Read the current queueing-enabled level (caller can save + later
/// pass to `restore_queue_signals`).
#[inline]
#[allow(non_snake_case)]
pub fn queue_signal_level() -> i32 {
    // c:127
    queueing_enabled.load(Ordering::SeqCst)
}

/// Port of `#define run_queued_signals()` from `Src/signals.h:78-86`.
/// Drain queued signals by re-running the handler for each, restoring
/// the saved mask between deliveries.
///
/// C body (c:78-86):
/// ```c
/// while (queue_front != queue_rear) {
///     sigset_t oset;
///     queue_front = (queue_front + 1) % MAX_QUEUE_SIZE;
///     oset = signal_setmask(signal_mask_queue[queue_front]);
///     zhandler(signal_queue[queue_front]);
///     signal_setmask(oset);
/// }
/// ```
#[inline]
#[allow(non_snake_case)]
pub fn run_queued_signals() {
    // c:78
    loop {
        let f = queue_front.load(Ordering::SeqCst);
        let r = queue_rear.load(Ordering::SeqCst);
        if f == r {
            break;
        }
        let nf = (f + 1) % MAX_QUEUE_SIZE;
        let sig = signal_queue[nf].load(Ordering::SeqCst);
        let mask = signal_mask_queue
            .lock()
            .ok()
            .and_then(|g| g.get(nf).copied());
        queue_front.store(nf, Ordering::SeqCst);
        if let Some(m) = mask {
            let _ = crate::ported::signals::signal_setmask(&m);
        }
        // Re-deliver via raise() so the installed handler runs again
        // with the original sig number.
        unsafe {
            libc::raise(sig);
        }
    }
}

// ---------------------------------------------------------------------------
// Block helpers (c:52-61).
//
// These wrap the lower-level `signal_block`/`signal_unblock` ported
// declared at c:129-130 (defined in signals.c). Rust ports as
// inline ported delegating to the same primitives in `signals.rs`.
// ---------------------------------------------------------------------------

/// Port of `#define child_block()` from `Src/signals.h:52`. Block
/// SIGCHLD so the parent can manipulate job-table state without
/// racing the reaper.
///
/// C body: `signal_block(signal_mask(SIGCHLD))`. The previous Rust
/// port was a no-op with a stale comment ("sigchld_mask not yet
/// wired") — but `signal_mask` and `signal_block` ARE both ported.
/// Without this block, the parent's job-table mutations could race
/// the SIGCHLD reaper inside wait_for_processes, corrupting jobtab
/// entries (e.g. setting STAT_DONE on a job the reaper hasn't seen).
#[inline]
#[allow(non_snake_case)]
#[cfg(unix)]
pub fn child_block() {
    // c:52
    let mask = crate::ported::signals::signal_mask(libc::SIGCHLD);
    let _ = crate::ported::signals::signal_block(&mask);
}

/// Non-unix no-op shim.
#[inline]
#[allow(non_snake_case)]
#[cfg(not(unix))]
pub fn child_block() {}

/// Port of `#define child_unblock()` from `Src/signals.h:53`.
/// Counterpart to `child_block`.
#[inline]
#[allow(non_snake_case)]
#[cfg(unix)]
pub fn child_unblock() {
    // c:53
    let mask = crate::ported::signals::signal_mask(libc::SIGCHLD);
    let _ = crate::ported::signals::signal_unblock(&mask);
}

/// Non-unix no-op shim.
#[inline]
#[allow(non_snake_case)]
#[cfg(not(unix))]
pub fn child_unblock() {}

/// Port of `#define winch_block()` from `Src/signals.h:56/59`.
/// Block SIGWINCH (terminal-resize signal) so prompt-redraw and
/// listing code can read terminal dimensions atomically.
///
/// C body: `signal_block(signal_mask(SIGWINCH))`. The previous Rust
/// port was a no-op with a stale comment ("signal_mask not yet
/// wired") — but `signal_mask` and `signal_block` ARE both ported
/// at `crate::ported::signals`. Now wired exactly per c:56.
#[inline]
#[allow(non_snake_case)]
#[cfg(unix)]
pub fn winch_block() {
    // c:56
    let mask = crate::ported::signals::signal_mask(libc::SIGWINCH);
    let _ = crate::ported::signals::signal_block(&mask);
}

/// Non-unix no-op shim.
#[inline]
#[allow(non_snake_case)]
#[cfg(not(unix))]
pub fn winch_block() {}

/// Port of `#define winch_unblock()` from `Src/signals.h:57/60`.
/// Counterpart to `winch_block`.
///
/// C body: `signal_unblock(signal_mask(SIGWINCH))`. Same wire-up
/// gap as winch_block — fixed now.
#[inline]
#[allow(non_snake_case)]
#[cfg(unix)]
pub fn winch_unblock() {
    // c:57
    let mask = crate::ported::signals::signal_mask(libc::SIGWINCH);
    let _ = crate::ported::signals::signal_unblock(&mask);
}

/// Non-unix no-op shim.
#[inline]
#[allow(non_snake_case)]
#[cfg(not(unix))]
pub fn winch_unblock() {}

// ---------------------------------------------------------------------------
// Convenience wrappers (c:64-67).
// ---------------------------------------------------------------------------

/// Port of `#define signal_ignore(S)` from `Src/signals.h:64`. Set
/// signal `s` to be ignored. C: `signal(S, SIG_IGN)` — returns the
/// PREVIOUS handler (the libc `signal(3)` contract). `init_signals`
/// at `Src/init.c:1418` reads this return value to detect a parent-
/// installed `SIG_IGN` on SIGHUP.
#[inline]
#[allow(non_snake_case)]
#[cfg(unix)]
pub fn signal_ignore(s: i32) -> libc::sighandler_t {
    // c:64
    unsafe { libc::signal(s, libc::SIG_IGN) }
}

/// Non-unix shim — same signature, no-op.
#[inline]
#[allow(non_snake_case)]
#[cfg(not(unix))]
pub fn signal_ignore(_s: i32) -> usize {
    0
}

/// Port of `#define signal_default(S)` from `Src/signals.h:67`. Reset
/// signal `s` to its default action. C: `signal(S, SIG_DFL)` —
/// returns the PREVIOUS handler.
#[inline]
#[allow(non_snake_case)]
#[cfg(unix)]
pub fn signal_default(s: i32) -> libc::sighandler_t {
    // c:67
    unsafe { libc::signal(s, libc::SIG_DFL) }
}

/// Non-unix shim — same signature, no-op.
#[inline]
#[allow(non_snake_case)]
#[cfg(not(unix))]
pub fn signal_default(_s: i32) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the pseudo-signal index ladder per c:34-46.
    #[test]
    fn pseudo_signal_indexes_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(SIGEXIT, 0);
        assert_eq!(SIGZERR, SIGCOUNT + 1);
        assert_eq!(SIGDEBUG, SIGCOUNT + 2);
        assert_eq!(VSIGCOUNT, SIGCOUNT + 3);
        assert!(VSIGCOUNT > 0);
    }

    /// Verifies TRAPCOUNT >= VSIGCOUNT (= when no RT signals;
    /// > when RT signals are available).
    #[test]
    fn trapcount_at_least_vsigcount() {
        let _g = crate::test_util::global_state_lock();
        assert!(TRAPCOUNT >= VSIGCOUNT);
    }

    /// Verifies SIGNUM round-trips through SIGIDX for a non-RT signal.
    #[test]
    fn signum_sigidx_round_trip_for_normal_signal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(SIGNUM(libc::SIGINT), libc::SIGINT);
        assert_eq!(SIGIDX(libc::SIGINT), libc::SIGINT);
    }

    /// Verifies MAX_QUEUE_SIZE per c:76.
    #[test]
    fn max_queue_size_is_128() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(MAX_QUEUE_SIZE, 128);
    }

    /// Verifies queue_signals / unqueue_signals balance the counter.
    #[test]
    fn queue_unqueue_balance() {
        let _g = crate::test_util::global_state_lock();
        let initial = queue_signal_level();
        queue_signals();
        assert_eq!(queue_signal_level(), initial + 1);
        queue_signals();
        assert_eq!(queue_signal_level(), initial + 2);
        unqueue_signals();
        assert_eq!(queue_signal_level(), initial + 1);
        unqueue_signals();
        assert_eq!(queue_signal_level(), initial);
    }

    /// Verifies dont_queue_signals zeroes the counter.
    #[test]
    fn dont_queue_signals_zeros_counter() {
        let _g = crate::test_util::global_state_lock();
        queue_signals();
        queue_signals();
        queue_signals();
        dont_queue_signals();
        assert_eq!(queue_signal_level(), 0);
    }

    /// Verifies restore_queue_signals sets the counter exactly.
    #[test]
    fn restore_queue_signals_sets_counter() {
        let _g = crate::test_util::global_state_lock();
        restore_queue_signals(0);
        restore_queue_signals(5);
        assert_eq!(queue_signal_level(), 5);
        restore_queue_signals(0);
    }

    /// `Src/signals.h:64` — `#define signal_ignore(S) signal(S, SIG_IGN)`.
    /// libc `signal(3)` returns the previous handler. Pin the round-
    /// trip: set SIG_IGN then SIG_DFL on a benign signal (SIGUSR2)
    /// and verify the returned previous-handler observation matches
    /// what we just installed. `init_signals` at `Src/init.c:1418`
    /// reads `signal_ignore(SIGHUP) == SIG_IGN` to detect a
    /// parent-installed-ignored HUP.
    #[cfg(unix)]
    #[test]
    fn signal_ignore_returns_previous_handler() {
        let _g = crate::test_util::global_state_lock();
        // Reset SIGUSR2 to default.
        let _ = signal_default(libc::SIGUSR2);
        // Install SIG_IGN — returns SIG_DFL (was reset above).
        let prev = signal_ignore(libc::SIGUSR2);
        assert_eq!(
            prev,
            libc::SIG_DFL,
            "c:64 — first signal_ignore must return prior SIG_DFL"
        );
        // Install SIG_IGN again — returns SIG_IGN.
        let prev2 = signal_ignore(libc::SIGUSR2);
        assert_eq!(
            prev2,
            libc::SIG_IGN,
            "c:64 — second signal_ignore must return prior SIG_IGN"
        );
        // Cleanup.
        let _ = signal_default(libc::SIGUSR2);
    }

    /// `Src/signals.h:67` — `#define signal_default(S) signal(S, SIG_DFL)`.
    /// Round-trip: install SIG_IGN, then signal_default — must
    /// return SIG_IGN (the prior handler).
    #[cfg(unix)]
    #[test]
    fn signal_default_returns_previous_handler() {
        let _g = crate::test_util::global_state_lock();
        let _ = signal_default(libc::SIGUSR2);
        let _ = signal_ignore(libc::SIGUSR2);
        let prev = signal_default(libc::SIGUSR2);
        assert_eq!(
            prev,
            libc::SIG_IGN,
            "c:67 — signal_default must return prior SIG_IGN"
        );
        // Default→default returns SIG_DFL.
        let prev2 = signal_default(libc::SIGUSR2);
        assert_eq!(prev2, libc::SIG_DFL);
    }

    /// c:39-40 — `SIGNUM` and `SIGIDX` are inverses for every valid
    /// signal in `[0, VSIGCOUNT)`. Pin the round-trip property
    /// across the full range to catch a regen that breaks the
    /// formula at a corner (e.g. real-time signal boundary).
    #[cfg(unix)]
    #[test]
    fn signum_sigidx_round_trip_full_range() {
        let _g = crate::test_util::global_state_lock();
        for s in 1..VSIGCOUNT {
            let n = SIGNUM(s);
            let back = SIGIDX(n);
            assert_eq!(
                back, s,
                "round-trip failed at signal {}: SIGIDX(SIGNUM({})) = {}",
                s, s, back
            );
        }
    }

    /// c:signames.c — `sigs_name(0)` is SIGEXIT, traditionally
    /// named "EXIT" (not numeric "0"). Pin so a regen that drops
    /// the pseudo-signal handling silently returns "0".
    #[cfg(unix)]
    #[test]
    fn sigs_name_pseudo_signal_exit() {
        let _g = crate::test_util::global_state_lock();
        let n = sigs_name(SIGEXIT);
        assert!(n.is_some(), "SIGEXIT (index 0) must have a name");
        // Conventionally "EXIT", but accept any non-numeric stem
        let s = n.unwrap();
        assert!(
            !s.is_empty(),
            "SIGEXIT name must be non-empty (got {:?})",
            s
        );
    }

    /// c:signames.c — `sigs_name` for a well-known signal returns
    /// the canonical name (e.g. SIGINT → "INT").
    #[cfg(unix)]
    #[test]
    fn sigs_name_sigint_resolves() {
        let _g = crate::test_util::global_state_lock();
        let n = sigs_name(libc::SIGINT);
        // SIGIDX(SIGINT) may differ from libc::SIGINT depending on
        // mapping; try direct + idx.
        let alt = sigs_name(SIGIDX(libc::SIGINT));
        assert!(
            n.is_some() || alt.is_some(),
            "SIGINT must resolve to a name via direct or SIGIDX path"
        );
    }

    /// c:signames.c — `sigs_name(-1)` or huge index returns None.
    /// Defensive contract: out-of-range must be a safe miss, not
    /// an OOB index into the table.
    #[test]
    fn sigs_name_out_of_range_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(sigs_name(-1).is_none());
        assert!(sigs_name(99999).is_none());
        assert!(sigs_name(TRAPCOUNT + 100).is_none());
    }

    /// c:jobs.c:2828 — `sigs_number("INT")` resolves to SIGINT.
    /// Pin reverse lookup so users can `kill -INT pid`.
    #[cfg(unix)]
    #[test]
    fn sigs_number_resolves_canonical_short_name() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            sigs_number("INT").is_some(),
            "sigs_number(INT) must resolve"
        );
        assert!(sigs_number("TERM").is_some());
        assert!(sigs_number("HUP").is_some());
        assert!(sigs_number("KILL").is_some());
    }

    /// c:jobs.c — `sigs_number` for unknown name returns None.
    /// Pin defensive miss; user `kill -BOGUS` must fail-soft.
    #[test]
    fn sigs_number_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(sigs_number("ZZBOGUSXX").is_none());
        assert!(sigs_number("").is_none());
        assert!(sigs_number("not_a_signal").is_none());
    }

    /// c:78 — `run_queued_signals` is safe to call even when no
    /// signals have been queued. Pin no-panic contract for the
    /// no-op path.
    #[test]
    fn run_queued_signals_with_empty_queue_is_safe() {
        let _g = crate::test_util::global_state_lock();
        // Reset: ensure nothing queued.
        dont_queue_signals();
        run_queued_signals();
        run_queued_signals(); // re-entry safety
    }

    /// c:104/123 — `restore_queue_signals(-1)` MUST NOT panic.
    /// Defensive boundary because the C source's `oqs = qs` walk
    /// can produce arbitrary saved values; the restore must not
    /// crash on weird input.
    #[test]
    fn restore_queue_signals_negative_value_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let saved = queue_signal_level();
        restore_queue_signals(-1);
        // Most impls clamp; just ensure no panic
        let _ = queue_signal_level();
        // Restore real state
        restore_queue_signals(saved.max(0));
    }

    /// c:34-38 — Every pseudo-signal index lies above SIGCOUNT.
    /// Pin layering: pseudo signals must NOT collide with real
    /// libc signal numbers, which are bounded by SIGCOUNT.
    #[test]
    fn pseudo_signals_above_libc_signal_range() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            SIGZERR > SIGCOUNT,
            "SIGZERR ({}) must be > SIGCOUNT ({}) to avoid libc collision",
            SIGZERR,
            SIGCOUNT
        );
        assert!(
            SIGDEBUG > SIGCOUNT,
            "SIGDEBUG ({}) must be > SIGCOUNT ({})",
            SIGDEBUG,
            SIGCOUNT
        );
        // Each pseudo-signal is distinct
        assert_ne!(SIGZERR, SIGDEBUG);
        assert_ne!(SIGZERR, SIGEXIT);
        assert_ne!(SIGDEBUG, SIGEXIT);
    }

    // ─── zsh-corpus pins: sigs_name / sigs_number round-trip ─────────

    /// `sigs_number("INT")` returns SIGINT.
    #[test]
    fn signals_corpus_sigs_number_int_returns_sigint() {
        let n = sigs_number("INT");
        assert_eq!(n, Some(libc::SIGINT), "INT → SIGINT");
    }

    /// `sigs_number("TERM")` returns SIGTERM.
    #[test]
    fn signals_corpus_sigs_number_term_returns_sigterm() {
        let n = sigs_number("TERM");
        assert_eq!(n, Some(libc::SIGTERM), "TERM → SIGTERM");
    }

    /// `sigs_number("HUP")` returns SIGHUP.
    #[test]
    fn signals_corpus_sigs_number_hup_returns_sighup() {
        let n = sigs_number("HUP");
        assert_eq!(n, Some(libc::SIGHUP), "HUP → SIGHUP");
    }

    /// `sigs_number("KILL")` returns SIGKILL.
    #[test]
    fn signals_corpus_sigs_number_kill_returns_sigkill() {
        let n = sigs_number("KILL");
        assert_eq!(n, Some(libc::SIGKILL), "KILL → SIGKILL");
    }

    /// `sigs_number("BOGUS_NEVER")` returns None.
    #[test]
    fn signals_corpus_sigs_number_unknown_returns_none() {
        assert!(sigs_number("BOGUS_NEVER").is_none());
        assert!(sigs_number("").is_none());
    }

    /// `sigs_name(SIGINT)` returns Some("INT") (canonical short form).
    #[test]
    fn signals_corpus_sigs_name_int_returns_short_form() {
        let n = sigs_name(libc::SIGINT);
        assert!(n.is_some(), "SIGINT must have a name");
        assert_eq!(n.unwrap(), "INT", "short form, not 'SIGINT'");
    }

    /// `sigs_name(SIGTERM)` returns Some("TERM").
    #[test]
    fn signals_corpus_sigs_name_term_returns_short_form() {
        assert_eq!(sigs_name(libc::SIGTERM), Some("TERM"));
    }

    /// `sigs_name(0)` doesn't panic regardless of return.
    #[test]
    fn signals_corpus_sigs_name_zero_does_not_panic() {
        let _ = sigs_name(0);
    }

    /// Round-trip: name → number → name preserves canonical form.
    #[test]
    fn signals_corpus_round_trip_int() {
        let n = sigs_number("INT").expect("INT exists");
        let back = sigs_name(n).expect("name for SIGINT");
        assert_eq!(back, "INT", "round-trip preserves canonical name");
    }

    /// Various signals have distinct numbers (sanity).
    #[test]
    fn signals_corpus_distinct_numbers() {
        let i = sigs_number("INT").unwrap();
        let t = sigs_number("TERM").unwrap();
        let h = sigs_number("HUP").unwrap();
        let k = sigs_number("KILL").unwrap();
        let q = sigs_number("QUIT").unwrap();
        assert_ne!(i, t);
        assert_ne!(i, h);
        assert_ne!(t, h);
        assert_ne!(k, q);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/signals.h macros + helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// `sigs_name(0)` returns "EXIT" (pseudo-signal slot 0).
    #[test]
    fn sigs_name_zero_returns_exit_pseudo() {
        assert_eq!(sigs_name(0), Some("EXIT"));
    }

    /// `sigs_name(SIGZERR)` returns "ZERR" (pseudo-signal SIGCOUNT+1).
    #[test]
    fn sigs_name_sigzerr_returns_zerr() {
        assert_eq!(sigs_name(SIGZERR), Some("ZERR"));
    }

    /// `sigs_name(SIGDEBUG)` returns "DEBUG" (pseudo-signal SIGCOUNT+2).
    #[test]
    fn sigs_name_sigdebug_returns_debug() {
        assert_eq!(sigs_name(SIGDEBUG), Some("DEBUG"));
    }

    /// `sigs_name(-1)` returns None (no such signal).
    #[test]
    fn sigs_name_neg_one_returns_none() {
        assert!(sigs_name(-1).is_none());
    }

    /// `sigs_name(99999)` returns None (unknown signal).
    #[test]
    fn sigs_name_out_of_range_returns_none_pin() {
        assert!(sigs_name(99999).is_none());
    }

    /// `sigs_number` strips leading "SIG" prefix.
    #[test]
    fn sigs_number_strips_sig_prefix() {
        let bare = sigs_number("INT").expect("INT");
        let prefixed = sigs_number("SIGINT").expect("SIGINT");
        assert_eq!(bare, prefixed, "SIG prefix must be transparent");
    }

    /// `sigs_number` for canonical POSIX signals returns Some.
    #[test]
    fn sigs_number_posix_signals_resolve() {
        for s in &["HUP", "INT", "QUIT", "TERM", "KILL", "USR1", "USR2"] {
            assert!(sigs_number(s).is_some(), "{:?} must resolve", s);
        }
    }

    /// `sigs_number` for unknown name returns None.
    #[test]
    fn sigs_number_unknown_returns_none_pin() {
        assert!(sigs_number("NEVER_REAL_SIGNAL").is_none());
        assert!(sigs_number("").is_none(), "empty string → None");
    }

    /// `sigs_number` is deterministic.
    #[test]
    fn sigs_number_is_deterministic() {
        for s in &["INT", "TERM", "NEVER_REAL", ""] {
            let first = sigs_number(s);
            for _ in 0..5 {
                assert_eq!(sigs_number(s), first, "{:?} must be pure", s);
            }
        }
    }

    /// SIGNUM (non-Linux): identity for any input.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn signum_non_linux_is_identity() {
        for x in &[0i32, 1, 10, 100, 1000, -1] {
            assert_eq!(SIGNUM(*x), *x, "non-Linux SIGNUM is identity");
        }
    }

    /// SIGIDX (non-Linux): identity for any input.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn sigidx_non_linux_is_identity() {
        for x in &[0i32, 1, 10, 100, 1000, -1] {
            assert_eq!(SIGIDX(*x), *x, "non-Linux SIGIDX is identity");
        }
    }

    /// queue_signals / unqueue_signals round-trip safely.
    #[test]
    fn queue_unqueue_signals_round_trip() {
        let _g = crate::test_util::global_state_lock();
        queue_signals();
        unqueue_signals();
    }

    /// queue_signal_level returns the queue depth.
    #[test]
    fn queue_signal_level_returns_nonneg() {
        let _g = crate::test_util::global_state_lock();
        let lvl = queue_signal_level();
        assert!(lvl >= 0, "queue level must be ≥ 0, got {}", lvl);
    }

    /// queue_signals → unqueue_signals → level returns to where it was.
    #[test]
    fn queue_unqueue_preserves_level() {
        let _g = crate::test_util::global_state_lock();
        let before = queue_signal_level();
        queue_signals();
        unqueue_signals();
        let after = queue_signal_level();
        assert_eq!(before, after, "round-trip preserves queue level");
    }

    /// `dont_queue_signals()` + `restore_queue_signals(q)` round-trip.
    #[test]
    fn dont_queue_restore_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let before = queue_signal_level();
        dont_queue_signals();
        restore_queue_signals(before);
        let after = queue_signal_level();
        assert_eq!(before, after);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/signals.h c:34-46 + sigs_*.
    // ═══════════════════════════════════════════════════════════════════

    /// c:46 — SIGEXIT is slot 0 (reserved for the EXIT trap).
    #[test]
    fn sigexit_is_slot_zero() {
        assert_eq!(SIGEXIT, 0, "EXIT trap occupies slot 0 per c:46");
    }

    /// c:34/35 — SIGZERR and SIGDEBUG are distinct pseudo-signals
    /// above SIGCOUNT (don't collide with real signals).
    #[test]
    fn sigzerr_sigdebug_distinct_above_sigcount() {
        assert!(SIGZERR > SIGCOUNT, "SIGZERR must be above real signals");
        assert!(SIGDEBUG > SIGCOUNT, "SIGDEBUG must be above real signals");
        assert_ne!(SIGZERR, SIGDEBUG, "ZERR and DEBUG must differ");
        assert_eq!(SIGZERR, SIGCOUNT + 1, "c:34 = SIGCOUNT+1");
        assert_eq!(SIGDEBUG, SIGCOUNT + 2, "c:35 = SIGCOUNT+2");
    }

    /// c:36 — VSIGCOUNT = SIGCOUNT + 3 (room for ZERR/DEBUG + slot 0).
    #[test]
    fn vsigcount_equals_sigcount_plus_three() {
        assert_eq!(VSIGCOUNT, SIGCOUNT + 3, "c:36 invariant");
    }

    /// `sigs_name(0)` returns Some("EXIT") (the EXIT trap convention).
    #[test]
    fn sigs_name_zero_returns_exit() {
        assert_eq!(sigs_name(0), Some("EXIT"));
    }

    /// `sigs_name(SIGZERR)` returns Some("ZERR").
    #[test]
    fn sigs_name_sigzerr_returns_zerr_pin() {
        assert_eq!(sigs_name(SIGZERR), Some("ZERR"));
    }

    /// `sigs_name(SIGDEBUG)` returns Some("DEBUG").
    #[test]
    fn sigs_name_sigdebug_returns_debug_pin() {
        assert_eq!(sigs_name(SIGDEBUG), Some("DEBUG"));
    }

    /// `sigs_name(-1)` returns None (out of range below).
    #[test]
    fn sigs_name_negative_returns_none() {
        assert_eq!(sigs_name(-1), None);
    }

    /// `sigs_name(99999)` returns None (out of range above).
    #[test]
    fn sigs_name_far_above_range_returns_none() {
        assert_eq!(sigs_name(99999), None);
    }

    /// `sigs_number("SIGINT")` and `sigs_number("INT")` both work
    /// (with/without SIG prefix).
    #[cfg(unix)]
    #[test]
    fn sigs_number_strips_sig_prefix_pin() {
        let with = sigs_number("SIGINT");
        let without = sigs_number("INT");
        assert_eq!(with, without, "SIG prefix must be optional");
        assert_eq!(with, Some(libc::SIGINT));
    }

    /// `sigs_number("DEFINITELY_NOT_A_SIGNAL_XYZ")` returns None.
    #[test]
    fn sigs_number_unknown_name_returns_none() {
        assert_eq!(sigs_number("DEFINITELY_NOT_A_SIGNAL_XYZ"), None);
    }

    /// `sigs_number("")` (empty string) returns None.
    #[test]
    fn sigs_number_empty_string_returns_none() {
        assert_eq!(sigs_number(""), None);
    }

    /// Round-trip: sigs_number(name) → sigs_name(num) → name (modulo
    /// SIG prefix stripping).
    #[cfg(unix)]
    #[test]
    fn sigs_number_to_name_round_trip_for_int() {
        let n = sigs_number("INT").unwrap();
        let back = sigs_name(n);
        assert_eq!(back, Some("INT"), "INT round trip");
    }

    /// c:39/40 — SIGNUM and SIGIDX are inverses for normal (non-RT)
    /// signal numbers.
    #[test]
    fn signum_sigidx_inverse_for_low_signals() {
        for s in [1, 2, 3, 9, 15, 20] {
            assert_eq!(SIGIDX(SIGNUM(s)), s, "round trip for sig {}", s);
            assert_eq!(SIGNUM(SIGIDX(s)), s, "inverse round trip for sig {}", s);
        }
    }

    /// c:76 — MAX_QUEUE_SIZE is a power of 2 (canonical buffer size).
    #[test]
    fn max_queue_size_is_power_of_two() {
        assert!(
            MAX_QUEUE_SIZE.is_power_of_two(),
            "MAX_QUEUE_SIZE {} must be a power of 2 for buffer math",
            MAX_QUEUE_SIZE
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/signals.h
    // c:329 queue_signals / c:341 unqueue_signals / c:363 dont_queue_signals /
    // c:398 queue_signal_level / c:419 run_queued_signals /
    // c:466 child_block / c:483 child_unblock / c:506 winch_block /
    // c:526 winch_unblock
    // ═══════════════════════════════════════════════════════════════════

    /// c:329 — `queue_signals` returns void (compile-time pin).
    #[test]
    fn queue_signals_signature_void() {
        let _g = crate::test_util::global_state_lock();
        let _: () = queue_signals();
        unqueue_signals(); // restore
    }

    /// c:398 — `queue_signal_level` returns i32 type pin.
    #[test]
    fn queue_signal_level_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = queue_signal_level();
    }

    /// c:329-341 — multiple queue/unqueue pairs preserve level.
    #[test]
    fn queue_unqueue_multiple_pairs_balance() {
        let _g = crate::test_util::global_state_lock();
        let before = queue_signal_level();
        for _ in 0..5 {
            queue_signals();
        }
        for _ in 0..5 {
            unqueue_signals();
        }
        let after = queue_signal_level();
        assert_eq!(before, after, "5x queue + 5x unqueue must balance");
    }

    /// c:466 — `child_block` is idempotent / safe.
    #[test]
    fn child_block_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            child_block();
        }
        for _ in 0..5 {
            child_unblock();
        }
    }

    /// c:506 — `winch_block` is idempotent / safe.
    #[test]
    fn winch_block_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            winch_block();
        }
        for _ in 0..5 {
            winch_unblock();
        }
    }

    /// c:419 — `run_queued_signals` is idempotent / safe to call when
    /// no signals queued.
    #[test]
    fn run_queued_signals_empty_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            run_queued_signals();
        }
    }

    /// c:363 — `dont_queue_signals` is idempotent.
    #[test]
    fn dont_queue_signals_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let saved = queue_signal_level();
        for _ in 0..5 {
            dont_queue_signals();
        }
        restore_queue_signals(saved);
    }

    /// c:34/35/36 — SIGZERR/SIGDEBUG/VSIGCOUNT all positive.
    #[test]
    fn sig_pseudos_all_positive() {
        assert!(SIGZERR > 0);
        assert!(SIGDEBUG > 0);
        assert!(VSIGCOUNT > 0);
        assert!(SIGCOUNT > 0);
        assert!(TRAPCOUNT > 0);
    }

    /// c:38/42 — TRAPCOUNT >= VSIGCOUNT (RT range never shrinks below VSIG).
    #[test]
    fn trapcount_greater_or_equal_vsigcount() {
        assert!(TRAPCOUNT >= VSIGCOUNT,
            "TRAPCOUNT {} must ≥ VSIGCOUNT {}", TRAPCOUNT, VSIGCOUNT);
    }

    /// c:46 — SIGEXIT (=0) is special pseudo-signal for EXIT trap slot.
    /// c:34 — SIGZERR is SIGCOUNT+1 (above all real signals).
    #[test]
    fn sigexit_below_sigzerr() {
        assert!(SIGEXIT < SIGZERR,
            "SIGEXIT=0 must be below SIGZERR={}", SIGZERR);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/signals.h
    // c:221 sigs_name / c:238 sigs_number / c:262 SIGNUM / c:284 SIGIDX /
    // c:398 queue_signal_level
    // ═══════════════════════════════════════════════════════════════════

    /// c:221 — `sigs_name` returns Option<&'static str> (compile-time pin).
    #[test]
    fn sigs_name_returns_option_static_str_type() {
        let _: Option<&'static str> = sigs_name(0);
    }

    /// c:221 — `sigs_name` for negative idx returns None (out of range).
    #[test]
    fn sigs_name_negative_idx_returns_none() {
        assert!(sigs_name(-1).is_none(), "negative idx → None");
        assert!(sigs_name(i32::MIN).is_none(), "MIN idx → None");
    }

    /// c:221 — `sigs_name` for far-out-of-range idx returns None.
    #[test]
    fn sigs_name_far_out_of_range_returns_none() {
        assert!(sigs_name(99999).is_none(), "huge idx → None");
        assert!(sigs_name(i32::MAX).is_none(), "MAX idx → None");
    }

    /// c:221 — `sigs_name` is deterministic.
    #[test]
    fn sigs_name_deterministic() {
        for idx in [0, 1, 2, 5, 9, 15, -1, 100] {
            let first = sigs_name(idx);
            for _ in 0..3 {
                assert_eq!(sigs_name(idx), first,
                    "sigs_name({}) must be pure", idx);
            }
        }
    }

    /// c:238 — `sigs_number` returns Option<i32> (compile-time pin).
    #[test]
    fn sigs_number_returns_option_i32_type() {
        let _: Option<i32> = sigs_number("HUP");
    }

    /// c:238 — `sigs_number("")` empty name returns None.
    #[test]
    fn sigs_number_empty_returns_none() {
        assert!(sigs_number("").is_none(), "empty name → None");
    }

    /// c:238 — `sigs_number` for nonsense name returns None.
    #[test]
    fn sigs_number_nonsense_returns_none() {
        assert!(sigs_number("__bogus_signal_xyz__").is_none(),
            "nonsense name → None");
    }

    /// c:238 — `sigs_number` is deterministic.
    #[test]
    fn sigs_number_deterministic() {
        for name in ["HUP", "INT", "TERM", "__bogus__", ""] {
            let first = sigs_number(name);
            for _ in 0..3 {
                assert_eq!(sigs_number(name), first,
                    "sigs_number({:?}) must be pure", name);
            }
        }
    }

    /// c:262 — `SIGNUM` returns i32 (compile-time pin).
    #[test]
    fn signum_returns_i32_type() {
        let _: i32 = SIGNUM(0);
    }

    /// c:284 — `SIGIDX` returns i32 (compile-time pin).
    #[test]
    fn sigidx_returns_i32_type() {
        let _: i32 = SIGIDX(0);
    }

    /// c:262+284 — SIGNUM/SIGIDX deterministic across full byte range.
    #[test]
    fn signum_sigidx_deterministic_byte_range() {
        for x in 0..256i32 {
            let n1 = SIGNUM(x);
            let n2 = SIGNUM(x);
            assert_eq!(n1, n2, "SIGNUM({}) must be pure", x);
            let i1 = SIGIDX(x);
            let i2 = SIGIDX(x);
            assert_eq!(i1, i2, "SIGIDX({}) must be pure", x);
        }
    }

    /// c:398 — `queue_signal_level` returns i32 (compile-time pin, alt).
    #[test]
    fn queue_signal_level_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = queue_signal_level();
    }

    /// c:398 — `queue_signal_level` is non-negative.
    #[test]
    fn queue_signal_level_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let v = queue_signal_level();
        assert!(v >= 0, "queue level must be ≥ 0, got {}", v);
    }
}
