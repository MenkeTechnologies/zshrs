//! !!! RUST-ONLY — NO C COUNTERPART !!!
//!
//! Phase timer for shell startup. Inert unless `ZSHRS_STARTUP_TRACE` is set,
//! in which case each `mark()` prints
//! `[startup <ms since process start>] <label>` to stderr, ending with
//! `FIRST PROMPT PAINTED`.
//!
//! This is the measurement tool for time-to-first-prompt: it is what found the
//! history `SELECT COUNT(*)` (37 ms) and the `module_path` shell-out (7 ms).
//! Output is opt-in and goes to stderr, so the no-startup-chatter rule holds
//! for every normal launch.

use std::sync::OnceLock;
use std::time::Instant;

static T0: OnceLock<Instant> = OnceLock::new();
static ON: OnceLock<bool> = OnceLock::new();

pub fn init(t0: Instant) {
    let _ = T0.set(t0);
    let _ = ON.set(std::env::var_os("ZSHRS_STARTUP_TRACE").is_some());
}

pub fn enabled() -> bool {
    *ON.get_or_init(|| std::env::var_os("ZSHRS_STARTUP_TRACE").is_some())
}

/// One-shot mark for the first prompt paint — the end of "time to first
/// prompt". `zrefresh` runs on every keystroke, so the mark is armed once and
/// costs a relaxed load afterwards.
pub fn mark_first_prompt() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static FIRED: AtomicBool = AtomicBool::new(false);
    if FIRED.swap(true, Ordering::Relaxed) {
        return;
    }
    mark("FIRST PROMPT PAINTED");
}

pub fn mark(label: &str) {
    if !enabled() {
        return;
    }
    let t0 = T0.get_or_init(Instant::now);
    eprintln!("[startup {:>8.3}ms] {}", t0.elapsed().as_secs_f64() * 1000.0, label);
}
