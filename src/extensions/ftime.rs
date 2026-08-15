//! TEMPORARY measurement scaffold — per-shell-function inclusive timing.
//!
//! `zsh/zprof` cannot be used to profile zshrs: `zprof_wrapper` is ported
//! but never registered (`boot_` returns 0 where C returns
//! `addwrapper(m, wrapper)`, zprof.c:362) and there is no `funcwrap`
//! infrastructure to register it with. This stands in until that is built.
//!
//! Gated on `ZSHRS_LOG` containing `ftime`; when the gate is off every
//! entry point is a single relaxed atomic load and returns `None`.
//!
//! Times are INCLUSIVE (like zprof's `time` column, not `self`): a nested
//! call is counted in its own row and in every ancestor's.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("ZSHRS_LOG").is_ok_and(|v| v.contains("ftime"))
    })
}

static DIRTY: AtomicBool = AtomicBool::new(false);

#[allow(clippy::type_complexity)]
fn table() -> &'static Mutex<HashMap<String, (u128, u32)>> {
    static T: OnceLock<Mutex<HashMap<String, (u128, u32)>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Begin timing `name`; `None` when the gate is off.
pub fn start(name: &str) -> Option<(String, Instant)> {
    if !enabled() {
        return None;
    }
    Some((name.to_string(), Instant::now()))
}

/// Accumulate the elapsed time for a span opened by [`start`].
pub fn stop(span: Option<(String, Instant)>) {
    let Some((name, t0)) = span else { return };
    let ns = t0.elapsed().as_nanos();
    if let Ok(mut t) = table().lock() {
        let e = t.entry(name).or_insert((0, 0));
        e.0 += ns;
        e.1 += 1;
    }
    DIRTY.store(true, Ordering::Relaxed);
}

/// Write the aggregate to `/tmp/ftime.log`, highest total first, and reset.
/// Called at the end of a completion so one TAB yields one report.
pub fn dump_and_reset() {
    if !enabled() || !DIRTY.swap(false, Ordering::Relaxed) {
        return;
    }
    let Ok(mut t) = table().lock() else { return };
    let mut rows: Vec<(String, u128, u32)> =
        t.iter().map(|(k, v)| (k.clone(), v.0, v.1)).collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    let mut out = String::from("  total_ms   calls  name (inclusive)\n");
    for (name, ns, calls) in rows.iter().take(40) {
        out.push_str(&format!(
            "{:10.3} {:7}  {}\n",
            *ns as f64 / 1e6,
            calls,
            name
        ));
    }
    let _ = std::fs::write("/tmp/ftime.log", out);
    t.clear();
}
