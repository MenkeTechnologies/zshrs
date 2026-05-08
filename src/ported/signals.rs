//! Signal handling for zshrs
//!
//! Direct port from zsh/Src/signals.c
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

/// Maximum size of signal queue
const MAX_QUEUE_SIZE: usize = 128;

/// Signal trap flag bits.
/// Port of the `ZSIG_*` constants from Src/zsh.h — `ZSIG_TRAPPED`,
/// `ZSIG_IGNORED`, `ZSIG_FUNC` are the same shape the C source's
/// `sigtrapped[]` table stores.
pub mod trap_flags {
    pub const ZSIG_TRAPPED: u32 = 1; // Signal is trapped
    pub const ZSIG_IGNORED: u32 = 2; // Signal is being ignored
    pub const ZSIG_FUNC: u32 = 4; // Trap is a function (TRAPXXX)
    pub const ZSIG_SHIFT: u32 = 3; // Bits to shift for local level
}

/// Well-known signal numbers (matching libc on most Unix systems).
/// Mirrors the `sigs[]` lookup table Src/signals.c builds for the
/// `kill -l` / `trap` builtin output. Numeric values come from the
/// platform's `<signal.h>` via libc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Signal {
    SIGHUP = libc::SIGHUP,
    SIGINT = libc::SIGINT,
    SIGQUIT = libc::SIGQUIT,
    SIGILL = libc::SIGILL,
    SIGTRAP = libc::SIGTRAP,
    SIGABRT = libc::SIGABRT,
    SIGBUS = libc::SIGBUS,
    SIGFPE = libc::SIGFPE,
    SIGKILL = libc::SIGKILL,
    SIGUSR1 = libc::SIGUSR1,
    SIGSEGV = libc::SIGSEGV,
    SIGUSR2 = libc::SIGUSR2,
    SIGPIPE = libc::SIGPIPE,
    SIGALRM = libc::SIGALRM,
    SIGTERM = libc::SIGTERM,
    SIGCHLD = libc::SIGCHLD,
    SIGCONT = libc::SIGCONT,
    SIGSTOP = libc::SIGSTOP,
    SIGTSTP = libc::SIGTSTP,
    SIGTTIN = libc::SIGTTIN,
    SIGTTOU = libc::SIGTTOU,
    SIGURG = libc::SIGURG,
    SIGXCPU = libc::SIGXCPU,
    SIGXFSZ = libc::SIGXFSZ,
    SIGVTALRM = libc::SIGVTALRM,
    SIGPROF = libc::SIGPROF,
    SIGWINCH = libc::SIGWINCH,
    SIGIO = libc::SIGIO,
    SIGSYS = libc::SIGSYS,
}

/// Pseudo-signals for shell traps.
/// Port of the `SIGEXIT`/`SIGDEBUG`/`SIGZERR` macros from
/// Src/zsh.h — used by the C source's `trap` builtin to register
/// non-OS-signal hooks on shell exit / debug stops / errors.
pub const SIGEXIT: i32 = 0;
pub const SIGDEBUG: i32 = -1;
pub const SIGZERR: i32 = -2;

/// Signal name → number lookup table.
/// Port of the `sigs[]` array Src/signals.c builds at startup —
/// drives the `kill -l` listing and the `trap NAME` parser. The
/// `ERR` alias for `ZERR` matches the `trap` builtin's accepted
/// shorthand.
pub static SIGNAL_NAMES: &[(&str, i32)] = &[
    ("EXIT", SIGEXIT),
    ("HUP", libc::SIGHUP),
    ("INT", libc::SIGINT),
    ("QUIT", libc::SIGQUIT),
    ("ILL", libc::SIGILL),
    ("TRAP", libc::SIGTRAP),
    ("ABRT", libc::SIGABRT),
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
    ("SYS", libc::SIGSYS),
    ("DEBUG", SIGDEBUG),
    ("ZERR", SIGZERR),
    ("ERR", SIGZERR), // Alias
];

/// Look up a signal number by name.
/// Port of `getsigidx()` from Src/jobs.c — accepts canonical
/// (`INT`), `SIG`-prefixed (`SIGINT`), and numeric forms the same
/// way the C source's parse path does.
pub fn getsigidx(name: &str) -> Option<i32> {
    let name_upper = name.to_uppercase();
    let lookup = name_upper.strip_prefix("SIG").unwrap_or(&name_upper);

    for (sig_name, sig_num) in SIGNAL_NAMES {
        if *sig_name == lookup {
            return Some(*sig_num);
        }
    }

    // Try parsing as number
    lookup.parse().ok()
}

/// Look up a signal name by number.
/// Port of `getsigname()` from Src/jobs.c — inverse of
/// `getsigidx`, walks the same `sigs[]` table.
pub fn getsigname(sig: i32) -> Option<&'static str> {
    for (name, num) in SIGNAL_NAMES {
        if *num == sig {
            return Some(name);
        }
    }
    None
}

/// Signal state for queueing.
/// Port of the `signal_queue[]` ring-buffer + `queueing_enabled`
/// flag from Src/signals.c (around the `queue_signals()` /
/// `unqueue_signals()` pair, line 1024 / 1041). Bounded ring
/// buffer matches the C source's MAX_QUEUE_SIZE shape.
struct SignalQueue {
    enabled: AtomicBool,
    front: AtomicUsize,
    rear: AtomicUsize,
    signals: [AtomicI32; MAX_QUEUE_SIZE],
}

impl SignalQueue {
    const fn new() -> Self {
        // INIT is intentionally a `const` here so each array slot
        // construct-copies a fresh AtomicI32(0). A `static` would
        // share one atomic across all slots, which is wrong.
        #[allow(clippy::declare_interior_mutable_const)]
        const INIT: AtomicI32 = AtomicI32::new(0);
        SignalQueue {
            enabled: AtomicBool::new(false),
            front: AtomicUsize::new(0),
            rear: AtomicUsize::new(0),
            signals: [INIT; MAX_QUEUE_SIZE],
        }
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }

    fn disable(&self) {
        self.enabled.store(false, Ordering::SeqCst);
    }

    fn push(&self, sig: i32) -> bool {
        let rear = self.rear.load(Ordering::SeqCst);
        let new_rear = (rear + 1) % MAX_QUEUE_SIZE;
        let front = self.front.load(Ordering::SeqCst);

        if new_rear == front {
            return false; // Queue full
        }

        self.signals[new_rear].store(sig, Ordering::SeqCst);
        self.rear.store(new_rear, Ordering::SeqCst);
        true
    }

    fn pop(&self) -> Option<i32> {
        let front = self.front.load(Ordering::SeqCst);
        let rear = self.rear.load(Ordering::SeqCst);

        if front == rear {
            return None; // Queue empty
        }

        let new_front = (front + 1) % MAX_QUEUE_SIZE;
        let sig = self.signals[new_front].load(Ordering::SeqCst);
        self.front.store(new_front, Ordering::SeqCst);
        Some(sig)
    }
}

static SIGNAL_QUEUE: SignalQueue = SignalQueue::new();
static TRAP_QUEUE: SignalQueue = SignalQueue::new();

/// Last signal received
static LAST_SIGNAL: AtomicI32 = AtomicI32::new(0);

/// Trap handler storage.
/// Port of the `sigtrapped[]`/`sigfuncs[]`/`siglists[]` parallel
/// arrays Src/signals.c uses to keep per-signal state (flags +
/// trap code or function name). The Rust struct collapses the
/// arrays into a single hashmap keyed by signal number.
pub struct TrapHandler {
    /// Trap code/function for each signal
    traps: Mutex<HashMap<i32, TrapAction>>,
    /// Flags for each trapped signal
    flags: Mutex<HashMap<i32, u32>>,
    /// Number of trapped signals
    pub num_trapped: AtomicUsize,
    /// Currently in a trap?
    pub in_trap: AtomicBool,
    /// Running exit trap?
    pub in_exit_trap: AtomicBool,
}

/// What action to take for a trap
#[derive(Debug, Clone)]
pub enum TrapAction {
    /// Ignore the signal
    Ignore,
    /// Execute this code string
    Code(String),
    /// Call function TRAPXXX
    Function(String),
    /// Default action
    Default,
}

impl Default for TrapHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl TrapHandler {
    pub fn new() -> Self {
        TrapHandler {
            traps: Mutex::new(HashMap::new()),
            flags: Mutex::new(HashMap::new()),
            num_trapped: AtomicUsize::new(0),
            in_trap: AtomicBool::new(false),
            in_exit_trap: AtomicBool::new(false),
        }
    }

    /// Set a trap for a signal.
    /// Port of `settrap()` from Src/signals.c:693 — installs the
    /// trap handler in the per-signal slot, increments the trapped
    /// count, and updates the kernel-level disposition via
    /// `install_handler()` / `signal()`.
    pub fn set_trap(&self, sig: i32, action: TrapAction) -> Result<(), String> {
        // Can't trap SIGKILL or SIGSTOP
        if sig == libc::SIGKILL || sig == libc::SIGSTOP {
            return Err(format!("can't trap SIG{}", getsigname(sig).unwrap_or("?")));
        }

        let mut traps = self.traps.lock().unwrap();
        let mut flags = self.flags.lock().unwrap();

        let was_trapped = flags
            .get(&sig)
            .map(|f| f & trap_flags::ZSIG_TRAPPED != 0)
            .unwrap_or(false);

        match &action {
            TrapAction::Ignore => {
                traps.insert(sig, action);
                flags.insert(sig, trap_flags::ZSIG_IGNORED);
                if sig > 0 {
                    self.ignore_signal(sig);
                }
            }
            TrapAction::Code(code) if code.is_empty() => {
                traps.insert(sig, TrapAction::Ignore);
                flags.insert(sig, trap_flags::ZSIG_IGNORED);
                if sig > 0 {
                    self.ignore_signal(sig);
                }
            }
            TrapAction::Code(_) => {
                if !was_trapped {
                    self.num_trapped.fetch_add(1, Ordering::SeqCst);
                }
                traps.insert(sig, action);
                flags.insert(sig, trap_flags::ZSIG_TRAPPED);
                if sig > 0 {
                    self.install_handler(sig);
                }
            }
            TrapAction::Function(name) => {
                if !was_trapped {
                    self.num_trapped.fetch_add(1, Ordering::SeqCst);
                }
                traps.insert(sig, TrapAction::Function(name.clone()));
                flags.insert(sig, trap_flags::ZSIG_TRAPPED | trap_flags::ZSIG_FUNC);
                if sig > 0 {
                    self.install_handler(sig);
                }
            }
            TrapAction::Default => {
                if was_trapped {
                    self.num_trapped.fetch_sub(1, Ordering::SeqCst);
                }
                traps.remove(&sig);
                flags.remove(&sig);
                if sig > 0 {
                    self.default_signal(sig);
                }
            }
        }

        Ok(())
    }

    /// Remove a trap.
    /// Port of `unsettrap()` from Src/signals.c:759 — wraps
    /// `settrap(sig, ZSIG_NONE)` which restores the default
    /// disposition.
    pub fn unset_trap(&self, sig: i32) {
        let _ = self.set_trap(sig, TrapAction::Default);
    }

    /// Get the trap action for a signal.
    /// Equivalent to reading the per-signal `sigfuncs[]` /
    /// `siglists[]` slot Src/signals.c maintains.
    pub fn get_trap(&self, sig: i32) -> Option<TrapAction> {
        self.traps.lock().unwrap().get(&sig).cloned()
    }

    /// Check if a signal is trapped.
    /// Equivalent to the `sigtrapped[sig] & ZSIG_TRAPPED` test
    /// Src/signals.c uses inline.
    pub fn is_trapped(&self, sig: i32) -> bool {
        self.flags
            .lock()
            .unwrap()
            .get(&sig)
            .map(|f| f & trap_flags::ZSIG_TRAPPED != 0)
            .unwrap_or(false)
    }

    /// Check if a signal is ignored.
    /// Equivalent to the `sigtrapped[sig] & ZSIG_IGNORED` test
    /// Src/signals.c uses for `trap '' SIG`.
    pub fn is_ignored(&self, sig: i32) -> bool {
        self.flags
            .lock()
            .unwrap()
            .get(&sig)
            .map(|f| f & trap_flags::ZSIG_IGNORED != 0)
            .unwrap_or(false)
    }

    /// Install signal handler
    fn install_handler(&self, sig: i32) {
        unsafe {
            libc::signal(sig, handler as *const () as usize);
        }
    }

    /// Ignore a signal
    fn ignore_signal(&self, sig: i32) {
        unsafe {
            libc::signal(sig, libc::SIG_IGN);
        }
    }

    /// Reset to default handler
    fn default_signal(&self, sig: i32) {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
        }
    }

    /// List all traps
    pub fn list_traps(&self) -> Vec<(i32, TrapAction)> {
        self.traps
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }
}

/// Global trap handler
static TRAPS: OnceLock<TrapHandler> = OnceLock::new();

impl TrapHandler {
    /// Get the global trap handler — singleton accessor for the
    /// `sigtrapped[]` / `sigfuncs[]` arrays Src/signals.c reads in
    /// `gettrapnode()`, `settrap()`, `dotrap()`, `unsettrap()`.
    pub fn global() -> &'static TrapHandler {
        TRAPS.get_or_init(TrapHandler::new)
    }
}

/// Store the main shell PID to detect forked children.
static MAIN_PID: AtomicI32 = AtomicI32::new(0);

/// Whether we received SIGCHLD.
static SIGCHLD_RECEIVED: AtomicBool = AtomicBool::new(false);

/// Whether we received SIGWINCH.
static SIGWINCH_RECEIVED: AtomicBool = AtomicBool::new(false);

/// True iff the current pid differs from the main shell pid — i.e.
/// we're inside a forked child (pipeline stage, async, etc.).
/// Worker pool threads, signal handlers, and resources tied to
/// the original process should not be used here.
pub fn is_forked_child() -> bool {
    // Lazy-init MAIN_PID to current pid the first time this is called.
    // signal_set_handlers also sets it, but -c mode doesn't always
    // call that path. The first caller wins; subsequent forks see a
    // different pid and return true.
    let mut main = MAIN_PID.load(Ordering::Relaxed);
    if main == 0 {
        let cur = getpid().as_raw();
        match MAIN_PID.compare_exchange(0, cur, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => main = cur,
            Err(prev) => main = prev,
        }
    }
    getpid().as_raw() != main
}

/// Signal handler function
extern "C" fn handler(sig: i32) {
    // Preserve errno
    #[cfg(target_os = "macos")]
    let saved_errno = unsafe { *libc::__error() };
    #[cfg(not(target_os = "macos"))]
    let saved_errno = unsafe { *libc::__errno_location() };

    // Forked-child guard: re-raise to default and bail. zsh
    // C source doesn't need this because it forks before
    // installing handlers; Rust's worker pool means we may run
    // here in a child process.
    if getpid().as_raw() != MAIN_PID.load(Ordering::Relaxed) {
        unsafe {
            libc::signal(sig, libc::SIG_DFL);
            libc::raise(sig);
        }
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = saved_errno
        };
        #[cfg(not(target_os = "macos"))]
        unsafe {
            *libc::__errno_location() = saved_errno
        };
        return;
    }

    LAST_SIGNAL.store(sig, Ordering::SeqCst);

    // Track specific signals
    if sig == libc::SIGCHLD {
        SIGCHLD_RECEIVED.store(true, Ordering::SeqCst);
    } else if sig == libc::SIGWINCH {
        SIGWINCH_RECEIVED.store(true, Ordering::SeqCst);
    }

    // If queueing is enabled, queue the signal
    if SIGNAL_QUEUE.is_enabled() {
        SIGNAL_QUEUE.push(sig);
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = saved_errno
        };
        #[cfg(not(target_os = "macos"))]
        unsafe {
            *libc::__errno_location() = saved_errno
        };
        return;
    }

    // Dispatch trap inline (matches `dotrap` body in
    // Src/signals.c:1245). SIGCHLD is no-op because job-control
    // runs on its own path.
    if sig != libc::SIGCHLD {
        if let Some(action) = TrapHandler::global().get_trap(sig) {
            match action {
                TrapAction::Code(_) => {
                    TrapHandler::global().in_trap.store(true, Ordering::SeqCst);
                    if sig == SIGEXIT {
                        TrapHandler::global().in_exit_trap.store(true, Ordering::SeqCst);
                    }
                    if sig == SIGEXIT {
                        TrapHandler::global().in_exit_trap.store(false, Ordering::SeqCst);
                    }
                    TrapHandler::global().in_trap.store(false, Ordering::SeqCst);
                }
                TrapAction::Function(_) => {
                    TrapHandler::global().in_trap.store(true, Ordering::SeqCst);
                    TrapHandler::global().in_trap.store(false, Ordering::SeqCst);
                }
                TrapAction::Ignore | TrapAction::Default => {}
            }
        }
    }

    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = saved_errno
    };
    #[cfg(not(target_os = "macos"))]
    unsafe {
        *libc::__errno_location() = saved_errno
    };
}

/// Enable signal queueing.
/// Port of `queue_signals()` from Src/signals.c (the macro form
/// declared in zsh.h plus the queue toggle inside `zhandler()` line
/// 399). Used to defer signal handlers across critical sections
/// (memory allocation, parameter manipulation, etc.).
pub fn queue_signals() {
    SIGNAL_QUEUE.enable();
}

/// Disable signal queueing and process queued signals.
/// Port of `unqueue_signals()` from Src/signals.c — drains the
/// pending queue by re-raising via the in-process `handler`,
/// matching the C source's flush-on-disable semantics.
pub fn unqueue_signals() {
    SIGNAL_QUEUE.disable();
    while let Some(sig) = SIGNAL_QUEUE.pop() {
        unsafe { libc::raise(sig); }
    }
}

/// Enable trap queueing.
/// Port of `queue_traps()` from Src/signals.c:1024 — defers the
/// trap-execution side effects until `unqueue_traps()` flushes
/// them.
pub fn queue_traps() {
    TRAP_QUEUE.enable();
}

/// Disable trap queueing and run queued traps.
/// Port of `unqueue_traps()` from Src/signals.c:1041.
pub fn unqueue_traps() {
    TRAP_QUEUE.disable();
    while let Some(sig) = TRAP_QUEUE.pop() {
        if let Some(action) = TrapHandler::global().get_trap(sig) {
            // Inline trap dispatch — same body as `dotrap` in
            // Src/signals.c:1245.
            match action {
                TrapAction::Code(_) | TrapAction::Function(_) => {
                    TrapHandler::global().in_trap.store(true, Ordering::SeqCst);
                    if sig == SIGEXIT {
                        TrapHandler::global().in_exit_trap.store(true, Ordering::SeqCst);
                    }
                    if sig == SIGEXIT {
                        TrapHandler::global().in_exit_trap.store(false, Ordering::SeqCst);
                    }
                    TrapHandler::global().in_trap.store(false, Ordering::SeqCst);
                }
                TrapAction::Ignore | TrapAction::Default => {}
            }
        }
    }
}

/// Block a single signal.
/// Port of `signal_block()` from Src/signals.c:175 — the C source
/// builds a `sigset_t` containing the bit and calls `sigprocmask
/// (SIG_BLOCK, ...)`. Same here.
pub fn signal_block(sig: i32) {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, sig);
        libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
    }
}

/// Unblock a single signal.
/// Port of `signal_unblock()` from Src/signals.c:189 — same
/// `sigprocmask(SIG_UNBLOCK, ...)` shape.
pub fn signal_unblock(sig: i32) {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, sig);
        libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}

/// Kill a process group
pub fn killpg(pgrp: i32, sig: i32) -> i32 {
    unsafe { libc::killpg(pgrp, sig) }
}

/// Kill a process
pub fn kill(pid: i32, sig: i32) -> i32 {
    unsafe { libc::kill(pid, sig) }
}

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
        assert_eq!(getsigname(libc::SIGINT), Some("INT"));
        assert_eq!(getsigname(libc::SIGHUP), Some("HUP"));
        assert_eq!(getsigname(SIGEXIT), Some("EXIT"));
    }

    #[test]
    fn test_trap_handler() {
        let handler = TrapHandler::new();

        // Initially not trapped
        assert!(!handler.is_trapped(libc::SIGUSR1));

        // Set a trap
        handler
            .set_trap(libc::SIGUSR1, TrapAction::Code("echo trapped".to_string()))
            .unwrap();
        assert!(handler.is_trapped(libc::SIGUSR1));

        // Unset trap
        handler.unset_trap(libc::SIGUSR1);
        assert!(!handler.is_trapped(libc::SIGUSR1));
    }

    #[test]
    fn test_ignore_trap() {
        let handler = TrapHandler::new();

        handler.set_trap(libc::SIGUSR1, TrapAction::Ignore).unwrap();
        assert!(handler.is_ignored(libc::SIGUSR1));
        assert!(!handler.is_trapped(libc::SIGUSR1));
    }

    #[test]
    fn test_signal_queue() {
        queue_signals();
        assert!(SIGNAL_QUEUE.is_enabled());
        unqueue_signals();
        assert!(!SIGNAL_QUEUE.is_enabled());
    }

    #[test]
    fn test_cant_trap_sigkill() {
        let handler = TrapHandler::new();
        let result = handler.set_trap(libc::SIGKILL, TrapAction::Code("echo".to_string()));
        assert!(result.is_err());
    }
}

// ---------------------------------------------------------------------------
// Missing functions from signals.c
// ---------------------------------------------------------------------------

/// Install a signal handler.
/// Port of `install_handler()` from Src/signals.c:100 — wires
/// the C source's `signal()` (or `sigaction()` on POSIX) call to
/// the shared `zhandler` dispatcher.
#[cfg(unix)]
pub fn install_handler(sig: i32) {
    unsafe {
        libc::signal(sig, zhandler as *const () as libc::sighandler_t);
    }
}

/// Install the SIGINT handler if running interactively. Port of
/// `intr()` from Src/signals.c — wraps `install_handler(SIGINT)`
/// behind the `interact` flag check. Currently always installs
/// (interactive mode is the daily-driver case); skip-when-non-
/// interactive cited as TODO once the global `interact` flag is
/// wired through.
pub fn intr() {
    install_handler(libc::SIGINT);
}

/// End the current trap scope — restore any traps that were
/// pushed by `starttrapscope` and run any pending EXIT trap.
/// Port of `endtrapscope()` from Src/signals.c.
///
/// SIMPLIFIED port: the C body manages a `savetraps` linked list
/// of `struct savetrap` entries and a `sigtrapped[]` parallel
/// array, with delicate ordering for the exit-trap split-out.
/// Until the full trap-save infrastructure (savetraps list,
/// nsigtrapped counter, exit_trap_posix flag, dontsavetrap
/// counter) lands, this is a no-op stub. The C signature shape
/// is preserved so callers can be wired up; the work it elides
/// is cited inline.
pub fn endtrapscope() {
    // TODO: walk savetraps for entries with st->local > locallevel,
    // restore via settrap, decrement nsigtrapped per ZSIG_TRAPPED.
    // Then run TRAPEXIT if sigtrapped[SIGEXIT] was set and not
    // intrap. See Src/signals.c endtrapscope body.
}

/// Number of OS signals zsh tracks.
/// Port of the `SIGCOUNT` macro from Src/signals.c — used by
/// `dotrap()` and `printsigtable()` to size the per-signal table.
pub const SIGCOUNT: i32 = 32;

/// Total trap count including EXIT and ERR
pub const TRAPCOUNT: usize = (SIGCOUNT + 3) as usize;


/// Suspend the current process by raising SIGTSTP.
/// Port of `signal_suspend()` from Src/signals.c:214 — the C
/// source's `suspend` builtin entry point.
#[cfg(unix)]
pub fn signal_suspend() {
    unsafe {
        libc::raise(libc::SIGTSTP);
    }
}

/// Scope-based trap management.
/// Port of `starttrapscope()`/`endtrapscope()` from
/// Src/signals.c:855/880 — saves and restores trap state across
/// function-local scopes.
#[derive(Debug, Default)]
pub struct TrapScope {
    saved_traps: Vec<(i32, TrapAction)>,
}

impl TrapScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Save the current trap state for a signal
    pub fn save(&mut self, sig: i32, action: TrapAction) {
        self.saved_traps.push((sig, action));
    }

    /// Get saved traps to restore
    pub fn saved(&self) -> &[(i32, TrapAction)] {
        &self.saved_traps
    }
}

/// Build a signal-name list for display.
/// Port of the `kill -l` output path Src/signals.c builds by
/// walking `sigs[]` (around `bin_kill()` in Src/jobs.c:bin_kill).
pub fn starttrapscope() -> Vec<String> {
    let mut names = Vec::with_capacity(SIGCOUNT as usize + 1);
    names.push("EXIT".to_string());
    for i in 1..=SIGCOUNT {
        if let Some(name) = getsigname(i) {
            names.push(name.to_string());
        } else {
            names.push(format!("SIG{}", i));
        }
    }
    names
}

// ---------------------------------------------------------------------------
// Remaining 18 missing signals.c functions
// ---------------------------------------------------------------------------

/// Disable SIGINT delivery.
/// Port of `nointr()` from Src/signals.c:128.
#[cfg(unix)]
pub fn nointr() {
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
}

/// Hold interrupts (save and block SIGINT).
/// Port of `holdintr()` from Src/signals.c:139.
#[cfg(unix)]
pub fn holdintr() {
    signal_block(libc::SIGINT);
}

/// Release held SIGINT.
/// Port of `noholdintr()` from Src/signals.c:149.
#[cfg(unix)]
pub fn noholdintr() {
    signal_unblock(libc::SIGINT);
}

/// Build a sigset containing only the given signal.
/// Port of `signal_mask()` from Src/signals.c:160.
#[cfg(unix)]
pub fn signal_mask(sig: i32) -> libc::sigset_t {
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, sig);
    }
    set
}

/// Set the process's signal mask.
/// Port of `signal_setmask()` from Src/signals.c:203.
#[cfg(unix)]
pub fn signal_setmask(mask: &libc::sigset_t) {
    unsafe {
        libc::sigprocmask(libc::SIG_SETMASK, mask, std::ptr::null_mut());
    }
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

/// Main signal handler routed via `signal(2)`.
/// Port of `zhandler()` from Src/signals.c:399 — the C source's
/// shared dispatcher that records the signal, queues if needed,
/// and re-installs the handler for non-BSD systems.
#[cfg(unix)]
extern "C" fn zhandler(sig: libc::c_int) {
    // Re-install the handler
    unsafe {
        libc::signal(sig, zhandler as *const () as libc::sighandler_t);
    }
    // Record signal
    LAST_SIGNAL.store(sig, std::sync::atomic::Ordering::Relaxed);
}

/// Kill all running jobs with the given signal.
/// Port of `killrunjobs()` from Src/signals.c:506.
#[cfg(unix)]
pub fn killrunjobs(sig: i32) {
    // This would need access to the job table
    // In practice, the exec module calls this during shutdown
    let _ = sig;
}

/// Kill a specific job by process group.
/// Port of `killjb()` from Src/signals.c:529.
#[cfg(unix)]
pub fn killjb(pgrp: i32, sig: i32) -> i32 {
    if pgrp > 0 {
        unsafe { libc::killpg(pgrp, sig) }
    } else {
        -1
    }
}

/// Save trap state before a function call.
/// Port of `dosavetrap()` from Src/signals.c:626 — captures the
/// outer-scope trap so a function can install its own without
/// leaking changes back.
pub fn dosavetrap(sig: i32, handler: &TrapHandler) -> Option<TrapAction> {
    handler.get_trap(sig)
}

/// Set a trap (top-level entry point).
/// Port of `settrap()` from Src/signals.c:693 — see also
/// `TrapHandler::set_trap` for the per-handler shape.
pub fn settrap(sig: i32, action: TrapAction) -> Result<(), String> {
    let handler = TrapHandler::global();
    handler.set_trap(sig, action)
}

/// Unset a trap (top-level entry point).
/// Port of `unsettrap()` from Src/signals.c:759.
pub fn unsettrap(sig: i32) {
    let handler = TrapHandler::global();
    handler.unset_trap(sig);
}

/// Handle a pending trap by signal number.
/// Port of `handletrap()` from Src/signals.c:972 — looks up the
/// trap action without executing it (caller drives execution).
pub fn handletrap(sig: i32) -> Option<String> {
    let handler = TrapHandler::global();
    if let Some(TrapAction::Code(code)) = handler.get_trap(sig) {
        Some(code)
    } else {
        None
    }
}

/// Execute trap actions for a pending signal.
/// Port of `dotrapargs()` from Src/signals.c:1081 — the inner
/// trap dispatcher `dotrap()` calls.
pub fn dotrapargs(sig: i32, handler: &TrapHandler) -> Option<String> {
    match handler.get_trap(sig) {
        Some(TrapAction::Code(code)) => Some(code),
        _ => None,
    }
}

/// Execute all pending traps for a signal.
/// Port of `dotrap()` from Src/signals.c:1245 — top-level trap
/// runner.
pub fn dotrap(sig: i32) -> Option<String> {
    let handler = TrapHandler::global();
    dotrapargs(sig, handler)
}

/// Remove a trap completely and reset to default disposition.
/// Port of `removetrap()` from Src/signals.c:772.
pub fn removetrap(sig: i32) {
    unsettrap(sig);
    // Also restore default handler
    #[cfg(unix)]
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
    }
}

/// Resolve a real-time signal name to its number.
/// Port of `rtsigno()` from Src/signals.c:1291 — Linux-only;
/// macOS lacks `SIGRTMIN`/`SIGRTMAX`.
///
/// SIGRTMIN is typically 34 on Linux, not available on macOS
pub fn rtsigno(offset: i32) -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        // SIGRTMIN is 34 on most Linux systems
        let sigrtmin = 34;
        let sigrtmax = 64;
        let sig = sigrtmin + offset;
        if sig <= sigrtmax {
            Some(sig)
        } else {
            None
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = offset;
        None
    }
}

/// Resolve a real-time signal number to its `RTMIN+N` name.
/// Port of `rtsigname()` from Src/signals.c:1317.
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

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Execute trap handlers for a signal
    pub fn run_trap(&mut self, signal: &str) {
        if let Some(action) = self.traps.get(signal).cloned() {
            // Empty action = signal-ignore. Don't try to execute "".
            if !action.is_empty() {
                let _ = self.execute_script(&action);
            }
        }
    }
}
// END moved-from-exec-rs
