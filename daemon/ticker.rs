// ticker.rs — daemon's periodic background task.
//
// Per docs/DAEMON.md "ticker thread": runs in the daemon's tokio runtime,
// fires once a minute, owns the housekeeping nobody else does. Sub-second
// cadence is wrong (we'd burn CPU on a daemon that mostly sleeps); per-day
// cadence misses the kind of cleanup that needs to keep up with active load
// (tmp files, log size). One minute is the right floor.
//
// Jobs handled here:
//   - **tmp sweep** — orphaned `images/*.rkyv.tmp.{pid}.{tid}` left behind
//     by an interrupted compile. Anything older than 60s is unlinkable.
//   - **log size monitor** — warn (in-log) when zshrs.log exceeds 10 MB.
//     Real size-based rotation requires fd handoff with tracing-appender's
//     worker; not v1. Daily rotation by tracing-appender's `Rotation::DAILY`
//     is in place independently of this ticker.
//   - **catalog vacuum** — runs once per 24h to reclaim disk space and
//     tighten indexes after the daemon has churned.
//   - **zask timeouts** — drop ask requests whose timeout_ms has elapsed
//     and emit ask:dismissed events to keep status-line counters honest.
//
// All work is best-effort. Each job logs its failure and the ticker keeps
// running; one bad cycle never kills the loop.

use std::sync::Arc;
use std::time::Duration;

use super::state::DaemonState;

const TICK_INTERVAL: Duration = Duration::from_secs(60);
const TMP_AGE: Duration = Duration::from_secs(60);
const LOG_SIZE_WARN: u64 = 10 * 1024 * 1024;
const VACUUM_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Spawn the ticker as a tokio task. Returns immediately; the task lives for
/// daemon lifetime (or until DaemonState is dropped, since it holds only a
/// Weak reference).
pub fn spawn(state: Arc<DaemonState>) {
    let weak = Arc::downgrade(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICK_INTERVAL);
        // Skip the immediate tick — first useful work is one cycle in.
        interval.tick().await;
        let mut last_vacuum = std::time::Instant::now();

        loop {
            interval.tick().await;
            let Some(state) = weak.upgrade() else {
                tracing::info!("ticker: daemon dropped, exiting");
                break;
            };

            sweep_tmp(&state);
            check_log_size(&state);
            ask_timeouts(&state);

            if last_vacuum.elapsed() >= VACUUM_INTERVAL {
                vacuum_catalog(&state);
                last_vacuum = std::time::Instant::now();
            }
        }
    });
}

fn sweep_tmp(state: &DaemonState) {
    match super::shard::sweep_tmp_files(&state.paths, TMP_AGE) {
        Ok(0) => {}
        Ok(n) => tracing::info!(swept = n, "ticker: orphaned tmp shards cleaned"),
        Err(e) => tracing::warn!(?e, "ticker: tmp sweep failed"),
    }
}

fn check_log_size(state: &DaemonState) {
    // tracing-appender writes to a date-stamped file inside paths.root.
    // We don't know the exact name (it's regenerated on each call to
    // `Rotation::DAILY.suffix`), so walk the directory and find the bare prefix.
    let dir = match std::fs::read_dir(&state.paths.root) {
        Ok(d) => d,
        Err(_) => return,
    };
    let mut total_bytes: u64 = 0;
    let mut count = 0;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s.starts_with("zshrs.log") {
            if let Ok(meta) = entry.metadata() {
                total_bytes += meta.len();
                count += 1;
            }
        }
    }
    if total_bytes > LOG_SIZE_WARN {
        tracing::warn!(
            files = count,
            bytes = total_bytes,
            cap = LOG_SIZE_WARN,
            "ticker: zshrs.log family exceeds soft cap; consider `zlog clear` or rotation"
        );
    }
}

fn vacuum_catalog(state: &DaemonState) {
    let res: rusqlite::Result<()> = state.with_catalog(|conn| {
        conn.execute_batch("VACUUM;")?;
        Ok(())
    });
    match res {
        Ok(()) => tracing::info!("ticker: catalog VACUUM complete"),
        Err(e) => tracing::warn!(?e, "ticker: catalog VACUUM failed"),
    }
}

fn ask_timeouts(state: &DaemonState) {
    let dropped = state.ask_inbox.drop_expired();
    if dropped > 0 {
        tracing::info!(count = dropped, "ticker: ask requests timed out");
    }
}
