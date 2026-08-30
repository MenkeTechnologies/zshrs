//! Startup phase timer for time-to-first-prompt work.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C zsh
//! has no startup instrumentation; profiling its init means attaching
//! `dtrace`/`perf` from outside. zshrs marks its own init phases so a
//! regression in time-to-first-prompt can be attributed to a phase
//! without an external profiler.
//!
//! [`mark`] is called from the init path (`ShellExecutor::new`,
//! `ShellExecutor::exec_init`) at each phase boundary. It records the
//! elapsed time since process start and since the previous mark.
//!
//! Tracing is **off** unless `ZSHRS_STARTUP_TRACE` is set in the
//! environment, so the steady-state cost of a mark is one relaxed
//! atomic load. Per the project's "no startup chatter" rule, marks go
//! to `~/.zshrs/zshrs.log` via `tracing::info!`; setting
//! `ZSHRS_STARTUP_TRACE=stderr` additionally mirrors them to stderr,
//! which is explicit user-requested output.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Where marks go, decided once from `ZSHRS_STARTUP_TRACE`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sink {
    /// Tracing disabled — `mark` returns immediately.
    Off,
    /// `~/.zshrs/zshrs.log` only.
    Log,
    /// The log plus stderr.
    LogAndStderr,
}

static SINK: OnceLock<Sink> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

/// Micros since [`START`] at the previous mark, for the delta column.
static PREV_US: AtomicU64 = AtomicU64::new(0);

fn sink() -> Sink {
    *SINK.get_or_init(|| match std::env::var("ZSHRS_STARTUP_TRACE") {
        Err(_) => Sink::Off,
        Ok(v) if v.is_empty() || v == "0" => Sink::Off,
        Ok(v) if v.eq_ignore_ascii_case("stderr") => Sink::LogAndStderr,
        Ok(_) => Sink::Log,
    })
}

/// Process start, as first observed. Called from [`mark`] and from
/// [`start`]; whichever runs first wins.
fn origin() -> Instant {
    *START.get_or_init(Instant::now)
}

/// Pin the zero point of the trace. Optional — the first [`mark`]
/// pins it otherwise. Call from `main` to include pre-init work.
pub fn start() {
    let _ = origin();
}

/// Micros since the zero point, regardless of whether tracing is on.
pub fn elapsed_us() -> u64 {
    u64::try_from(origin().elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Record a startup phase boundary.
///
/// No-op unless `ZSHRS_STARTUP_TRACE` is set. `label` names the phase
/// that just *finished*.
pub fn mark(label: &str) {
    let sink = sink();
    if sink == Sink::Off {
        return;
    }
    let total_us = elapsed_us();
    let prev_us = PREV_US.swap(total_us, Ordering::Relaxed);
    let delta_us = total_us.saturating_sub(prev_us);
    tracing::info!(
        target: "startup",
        phase = label,
        total_us,
        delta_us,
        "startup phase"
    );
    if sink == Sink::LogAndStderr {
        eprintln!(
            "startup: {:>9.3}ms (+{:>8.3}ms)  {label}",
            total_us as f64 / 1000.0,
            delta_us as f64 / 1000.0
        );
    }
}
