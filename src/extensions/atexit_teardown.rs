//! Whether the process is inside a libc `atexit` hook.
//!
//! **zshrs-original — no C counterpart.** zsh's exit path is plain C and
//! has no runtime that tears itself down underneath it.
//!
//! zshrs registers two libc `atexit` hooks — `autoload_cache::
//! atexit_flush_pending` (the shard flush that makes the autoload cache
//! work for `zshrs -c` and for the `exit` builtin, neither of which
//! unwinds) and `recorder::atexit_finalize`. libc runs both AFTER the
//! Rust runtime has begun destroying thread-locals, so anything
//! TLS-backed raises
//!
//!     cannot access a Thread Local Storage value during or after
//!     destruction: AccessError
//!
//! `tracing`'s fmt layer formats every event into a thread-local buffer
//! (`tracing_subscriber::fmt::fmt_layer`), so a single `tracing::info!`
//! on that path panics. `once_cell::Lazy` has the same hazard.
//!
//! Both hooks already wrap their bodies in `catch_unwind`, which stops
//! an unwinding panic out of an `extern "C"` function from aborting the
//! process. That is not sufficient: `catch_unwind` runs AFTER the panic
//! hook, so `panicked at ...` plus the `RUST_BACKTRACE` note have
//! already been written to stderr — on the terminal the user is exiting.
//! The unwind also abandons whatever the hook had left to do.
//!
//! Measured 2026-09-09, and it reproduces every time:
//!
//! ```text
//! Z=$(mktemp -d); : > $Z/autoloads.rkyv.tmp.999991.1
//! ZSHRS_HOME=$Z zshrs -f -c 'autoload -Uz is-at-least; is-at-least 5.0'
//!     yes
//!     thread 'main' panicked at .../thread/local.rs:429:25:
//!     cannot access a Thread Local Storage value during or after
//!     destruction: AccessError
//! ```
//!
//! The temp file is what arms it: the `tracing::info!` in
//! `atomic_write::write_bytes_atomic` fires only when a reap actually
//! removed something, so an ordinary exit never reaches it and a machine
//! running several shells at once does. It first showed up as a crashed
//! cell in a four-way-parallel completion parity sweep.
//!
//! So the hooks mark the teardown here, and the log sites they can reach
//! ask before emitting. Non-atexit callers of those same functions are
//! unaffected — the flag is only ever set once the process is already on
//! its way out.

use std::sync::atomic::{AtomicBool, Ordering};

/// Set once a libc `atexit` hook starts running. Never cleared: there is
/// nothing after `atexit` to clear it for.
static IN_ATEXIT: AtomicBool = AtomicBool::new(false);

/// Called first thing by every `extern "C"` `atexit` hook.
pub fn mark() {
    IN_ATEXIT.store(true, Ordering::SeqCst);
}

/// True once [`mark`] has run, i.e. thread-locals may already be gone.
///
/// Guard every `tracing::*` (or other TLS-backed) call reachable from an
/// `atexit` hook with this. The event is DROPPED rather than deferred:
/// the log is a diagnostic and the process is one instruction from
/// `_exit`, whereas the work the event interrupts is the cache write the
/// hook exists to perform.
pub fn active() -> bool {
    IN_ATEXIT.load(Ordering::SeqCst)
}
