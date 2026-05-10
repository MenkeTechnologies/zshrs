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
//! C source: 0 structs/enums/fns. ~25 #defines.
//!
//! Macro casing preserved verbatim per the casing rule. The
//! signal-queue mutable state (`queue_front`/`queue_rear`/
//! `queueing_enabled`/`signal_queue[]`/`signal_mask_queue[]`) lives
//! in `signals.rs` (the .c port); this header port covers the
//! constants + the queueing-API call shape.

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
pub const SIGCOUNT: i32 = 64;                                            // Linux NSIG = 65

#[cfg(target_os = "macos")]
pub const SIGCOUNT: i32 = 31;                                            // macOS NSIG = 32

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const SIGCOUNT: i32 = 31;                                            // POSIX baseline

/// Port of `#define SIGZERR` from `Src/signals.h:34`. Pseudo-signal
/// index for the ZERR trap (fires after every command that exits
/// with a nonzero status).
pub const SIGZERR: i32 = SIGCOUNT + 1;                                   // c:34

/// Port of `#define SIGDEBUG` from `Src/signals.h:35`. Pseudo-signal
/// index for the DEBUG trap (fires before every command).
pub const SIGDEBUG: i32 = SIGCOUNT + 2;                                  // c:35

/// Port of `#define VSIGCOUNT` from `Src/signals.h:36`. Total
/// number of "virtual" signal-table slots = SIGCOUNT (real signals)
/// + 3 (zero + ZERR + DEBUG).
pub const VSIGCOUNT: i32 = SIGCOUNT + 3;                                 // c:36

/// Port of `#define TRAPCOUNT` from `Src/signals.h:38/42`. With
/// `SIGRTMIN`/`SIGRTMAX` available (every Linux/musl/BSD host),
/// expands to `VSIGCOUNT + (SIGRTMAX - SIGRTMIN + 1)` so the trap
/// table covers real-time signals too. macOS/BSD libc binding
/// doesn't expose SIGRTMIN/SIGRTMAX as Rust constants — the c:42
/// `#else` arm (TRAPCOUNT = VSIGCOUNT) applies.
#[cfg(target_os = "linux")]
pub const TRAPCOUNT: i32 = VSIGCOUNT + 32;                               // c:38 (Linux RT range = 32)

#[cfg(not(target_os = "linux"))]
pub const TRAPCOUNT: i32 = VSIGCOUNT;                                    // c:42

/// Port of `#define SIGEXIT` from `Src/signals.h:46`. Pseudo-signal
/// index for the EXIT trap (fires when the shell exits).
pub const SIGEXIT: i32 = 0;                                              // c:46

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
pub static SIGS: &[(&str, i32)] = &[                                      // c:signames.c sigs[]
    ("HUP",    libc::SIGHUP),    ("INT",    libc::SIGINT),
    ("QUIT",   libc::SIGQUIT),   ("ILL",    libc::SIGILL),
    ("TRAP",   libc::SIGTRAP),   ("ABRT",   libc::SIGABRT),
    ("BUS",    libc::SIGBUS),    ("FPE",    libc::SIGFPE),
    ("KILL",   libc::SIGKILL),   ("USR1",   libc::SIGUSR1),
    ("SEGV",   libc::SIGSEGV),   ("USR2",   libc::SIGUSR2),
    ("PIPE",   libc::SIGPIPE),   ("ALRM",   libc::SIGALRM),
    ("TERM",   libc::SIGTERM),   ("CHLD",   libc::SIGCHLD),
    ("CONT",   libc::SIGCONT),   ("STOP",   libc::SIGSTOP),
    ("TSTP",   libc::SIGTSTP),   ("TTIN",   libc::SIGTTIN),
    ("TTOU",   libc::SIGTTOU),   ("URG",    libc::SIGURG),
    ("XCPU",   libc::SIGXCPU),   ("XFSZ",   libc::SIGXFSZ),
    ("VTALRM", libc::SIGVTALRM), ("PROF",   libc::SIGPROF),
    ("WINCH",  libc::SIGWINCH),  ("IO",     libc::SIGIO),
    ("SYS",    libc::SIGSYS),
];

/// Port of `alt_sigs[]` from `Src/jobs.c:2740`. Cross-platform name
/// aliases — names like `CLD` / `IO` / `IOT` map to the same number
/// as the canonical zsh name on platforms where the underlying
/// C macro pair coincides.
pub static ALT_SIGS: &[(&str, i32)] = &[                                  // c:jobs.c:2740
    ("CLD", libc::SIGCHLD),                                               // c:2742-2746
    ("IOT", libc::SIGABRT),                                               // c:2752-2756
    ("ERR", SIGZERR),                                                     // c:2762
];

/// Port of the `sig_msg[]` array from `signames.c` (auto-generated
/// by signames2.awk). Pretty-printed messages keyed by signal number,
/// used in the `printjob`/`printtime` output paths when zsh wants
/// to render a signal as e.g. `"hangup"` or `"floating point exception"`
/// rather than just `"SIGFPE"`. Each entry is `(signum, message)`.
pub static SIG_MSG: &[(i32, &str)] = &[                                   // c:signames.c sig_msg[]
    (libc::SIGHUP,    "hangup"),
    (libc::SIGINT,    "interrupt"),
    (libc::SIGQUIT,   "quit"),
    (libc::SIGILL,    "illegal hardware instruction"),
    (libc::SIGTRAP,   "trace trap"),
    (libc::SIGABRT,   "abort"),
    (libc::SIGBUS,    "bus error"),
    (libc::SIGFPE,    "floating point exception"),
    (libc::SIGKILL,   "killed"),
    (libc::SIGUSR1,   "user-defined signal 1"),
    (libc::SIGSEGV,   "segmentation fault"),
    (libc::SIGUSR2,   "user-defined signal 2"),
    (libc::SIGPIPE,   "broken pipe"),
    (libc::SIGALRM,   "alarm"),
    (libc::SIGTERM,   "terminated"),
    (libc::SIGCHLD,   "death of child"),
    (libc::SIGCONT,   "continued"),
    (libc::SIGSTOP,   "stopped (signal)"),
    (libc::SIGTSTP,   "stopped"),
    (libc::SIGTTIN,   "stopped (tty input)"),
    (libc::SIGTTOU,   "stopped (tty output)"),
    (libc::SIGURG,    "urgent condition"),
    (libc::SIGXCPU,   "cpu limit exceeded"),
    (libc::SIGXFSZ,   "file size limit exceeded"),
    (libc::SIGVTALRM, "virtual time alarm"),
    (libc::SIGPROF,   "profile signal"),
    (libc::SIGWINCH,  "window size changed"),
    (libc::SIGIO,     "i/o ready"),
    (libc::SIGSYS,    "invalid system call"),
];

/// Look up the canonical signal name for `idx` (sans `SIG` prefix).
/// Returns `"EXIT"` for slot 0, `"ZERR"`/`"DEBUG"` for the pseudo-
/// signal slots, the SIGS-table name for real signals, or `None`
/// when out of range. Mirrors C's `sigs[idx]` indexed lookup.
pub fn sigs_name(idx: i32) -> Option<&'static str> {                      // c:signames.c sigs[]
    if idx == 0           { return Some("EXIT"); }                        // c:slot 0
    if idx == SIGZERR     { return Some("ZERR"); }                        // c:SIGCOUNT+1
    if idx == SIGDEBUG    { return Some("DEBUG"); }                       // c:SIGCOUNT+2
    SIGS.iter().find(|(_, n)| *n == idx).map(|(name, _)| *name)
}

/// Reverse of `sigs_name` — accepts a name (with or without leading
/// `SIG`) and returns the libc signal number. Walks SIGS first, then
/// ALT_SIGS for cross-platform aliases.
pub fn sigs_number(name: &str) -> Option<i32> {                           // c:jobs.c:2828 lookup
    let bare = name.strip_prefix("SIG").unwrap_or(name);
    SIGS.iter().find(|(n, _)| *n == bare).map(|(_, num)| *num)
        .or_else(|| ALT_SIGS.iter().find(|(n, _)| *n == bare).map(|(_, num)| *num))
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
pub fn SIGNUM(x: i32) -> i32 {                                           // c:39
    #[cfg(target_os = "linux")]
    {
        if x >= VSIGCOUNT {
            x - VSIGCOUNT + libc::SIGRTMIN()
        } else {
            x
        }
    }
    #[cfg(not(target_os = "linux"))]
    { x }
}

/// Port of `#define SIGIDX(x)` from `Src/signals.h:40`. Convert a
/// real signal number to its trap-table index. Numbers in the
/// `SIGRTMIN..=SIGRTMAX` range map to `(x - SIGRTMIN) + VSIGCOUNT`;
/// other numbers pass through unchanged.
#[inline]
#[allow(non_snake_case)]
pub fn SIGIDX(x: i32) -> i32 {                                           // c:40
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
    { x }
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
pub const MAX_QUEUE_SIZE: usize = 128;                                   // c:76

/// Port of the global `int queueing_enabled;` (Src/signals.c). The
// ---------------------------------------------------------------------------
// Signal-queueing macros (c:78-127). Ported as fns since Rust has
// no preprocessor macros. The mutable state (`queueing_enabled`,
// `queue_front`, `queue_rear`, `signal_queue[]`, `signal_mask_queue[]`)
// lives in `signals.rs` per `Src/signals.c:77-81`; these wrappers
// just call through.
// ---------------------------------------------------------------------------

/// Port of `#define queue_signals()` from `Src/signals.h:90/112`.
/// Increment the queueing-enabled counter so subsequent signals are
/// queued instead of handled immediately.
#[inline]
#[allow(non_snake_case)]
pub fn queue_signals() {                                                 // c:90/112
    crate::ported::signals::queueing_enabled.fetch_add(1, Ordering::SeqCst);
}

/// Port of `#define unqueue_signals()` from `Src/signals.h:92/114`.
/// Decrement the queueing-enabled counter; when it reaches 0, drain
/// the signal queue via `run_queued_signals()`.
#[inline]
#[allow(non_snake_case)]
pub fn unqueue_signals() {                                               // c:92/114
    let prev = crate::ported::signals::queueing_enabled.fetch_sub(1, Ordering::SeqCst);
    if prev == 1 {
        run_queued_signals();
    }
}

/// Port of `#define dont_queue_signals()` from `Src/signals.h:98/118`.
/// Force the queueing counter to 0 (skipping the decrement chain) and
/// drain whatever's queued. Used by code paths that need to flush
/// before a long-running operation.
#[inline]
#[allow(non_snake_case)]
pub fn dont_queue_signals() {                                            // c:98/118
    crate::ported::signals::queueing_enabled.store(0, Ordering::SeqCst);
    run_queued_signals();
}

/// Port of `#define restore_queue_signals(q)` from
/// `Src/signals.h:104/123`. Restore the queueing counter to a saved
/// value (typically captured before a section that called
/// `dont_queue_signals` or similar).
#[inline]
#[allow(non_snake_case)]
pub fn restore_queue_signals(q: i32) {                                   // c:104/123
    crate::ported::signals::queueing_enabled.store(q, Ordering::SeqCst);
}

/// Port of `#define queue_signal_level()` from `Src/signals.h:127`.
/// Read the current queueing-enabled level (caller can save + later
/// pass to `restore_queue_signals`).
#[inline]
#[allow(non_snake_case)]
pub fn queue_signal_level() -> i32 {                                     // c:127
    crate::ported::signals::queueing_enabled.load(Ordering::SeqCst)
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
pub fn run_queued_signals() {                                            // c:78
    use crate::ported::signals::{queue_front, queue_rear, signal_queue, signal_mask_queue};
    loop {
        let f = queue_front.load(Ordering::SeqCst);
        let r = queue_rear.load(Ordering::SeqCst);
        if f == r { break; }
        let nf = (f + 1) % MAX_QUEUE_SIZE;
        let sig = signal_queue[nf].load(Ordering::SeqCst);
        let mask = signal_mask_queue.lock().ok().and_then(|g| g.get(nf).copied());
        queue_front.store(nf, Ordering::SeqCst);
        if let Some(m) = mask {
            let _ = crate::ported::signals::signal_setmask(&m);
        }
        // Re-deliver via raise() so the installed handler runs again
        // with the original sig number.
        unsafe { libc::raise(sig); }
    }
}

// ---------------------------------------------------------------------------
// Block helpers (c:52-61).
//
// These wrap the lower-level `signal_block`/`signal_unblock` fns
// declared at c:129-130 (defined in signals.c). Rust ports as
// inline fns delegating to the same primitives in `signals.rs`.
// ---------------------------------------------------------------------------

/// Port of `#define child_block()` from `Src/signals.h:52`. Block
/// SIGCHLD so the parent can manipulate job-table state without
/// racing the reaper. zshrs's signals.rs holds the `sigchld_mask`
/// global; until it's ported, this is a no-op stub.
#[inline]
#[allow(non_snake_case)]
pub fn child_block() {                                                   // c:52
    // signal_block(sigchld_mask) — sigchld_mask not yet wired. No-op.
}

/// Port of `#define child_unblock()` from `Src/signals.h:53`.
/// Counterpart to `child_block`.
#[inline]
#[allow(non_snake_case)]
pub fn child_unblock() {                                                 // c:53
    // signal_unblock(sigchld_mask) — see child_block doc.
}

/// Port of `#define winch_block()` from `Src/signals.h:56/59`.
/// Block SIGWINCH (terminal-resize signal) so prompt-redraw and
/// listing code can read terminal dimensions atomically.
#[inline]
#[allow(non_snake_case)]
pub fn winch_block() {                                                   // c:56
    // signal_block(signal_mask(SIGWINCH)) — signal_mask not yet wired.
}

/// Port of `#define winch_unblock()` from `Src/signals.h:57/60`.
/// Counterpart to `winch_block`.
#[inline]
#[allow(non_snake_case)]
pub fn winch_unblock() {                                                 // c:57
}

// ---------------------------------------------------------------------------
// Convenience wrappers (c:64-67).
// ---------------------------------------------------------------------------

/// Port of `#define signal_ignore(S)` from `Src/signals.h:64`. Set
/// signal `s` to be ignored. C: `signal(S, SIG_IGN)`.
#[inline]
#[allow(non_snake_case)]
pub fn signal_ignore(s: i32) {                                           // c:64
    #[cfg(unix)]
    unsafe {
        libc::signal(s, libc::SIG_IGN);
    }
    #[cfg(not(unix))]
    { let _ = s; }
}

/// Port of `#define signal_default(S)` from `Src/signals.h:67`. Reset
/// signal `s` to its default action. C: `signal(S, SIG_DFL)`.
#[inline]
#[allow(non_snake_case)]
pub fn signal_default(s: i32) {                                          // c:67
    #[cfg(unix)]
    unsafe {
        libc::signal(s, libc::SIG_DFL);
    }
    #[cfg(not(unix))]
    { let _ = s; }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the pseudo-signal index ladder per c:34-46.
    #[test]
    fn pseudo_signal_indexes_correct() {
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
        assert!(TRAPCOUNT >= VSIGCOUNT);
    }

    /// Verifies SIGNUM round-trips through SIGIDX for a non-RT signal.
    #[test]
    fn signum_sigidx_round_trip_for_normal_signal() {
        assert_eq!(SIGNUM(libc::SIGINT), libc::SIGINT);
        assert_eq!(SIGIDX(libc::SIGINT), libc::SIGINT);
    }

    /// Verifies MAX_QUEUE_SIZE per c:76.
    #[test]
    fn max_queue_size_is_128() {
        assert_eq!(MAX_QUEUE_SIZE, 128);
    }

    /// Verifies queue_signals / unqueue_signals balance the counter.
    #[test]
    fn queue_unqueue_balance() {
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
        queue_signals();
        queue_signals();
        queue_signals();
        dont_queue_signals();
        assert_eq!(queue_signal_level(), 0);
    }

    /// Verifies restore_queue_signals sets the counter exactly.
    #[test]
    fn restore_queue_signals_sets_counter() {
        restore_queue_signals(0);
        restore_queue_signals(5);
        assert_eq!(queue_signal_level(), 5);
        restore_queue_signals(0);
    }
}
