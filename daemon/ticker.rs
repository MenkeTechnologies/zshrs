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
/// Default 10 MB per docs/DAEMON.md. Override via env ZSHRS_LOG_MAX_BYTES.
const DEFAULT_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// Up to N rotated copies kept per active file (.1 .. .N). Default 4 per docs.
const DEFAULT_LOG_MAX_ROTATIONS: u32 = 4;
const VACUUM_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Soft cap for warning when zshrs.log family exceeds it.
const LOG_SIZE_WARN: u64 = DEFAULT_LOG_MAX_BYTES;

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
            rotate_logs_if_needed(&state);
            ask_timeouts(&state);

            if last_vacuum.elapsed() >= VACUUM_INTERVAL {
                vacuum_catalog(&state);
                last_vacuum = std::time::Instant::now();
            }
        }
    });
}

fn log_max_bytes() -> u64 {
    std::env::var("ZSHRS_LOG_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_LOG_MAX_BYTES)
}

fn log_max_rotations() -> u32 {
    std::env::var("ZSHRS_LOG_MAX_ROTATIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_LOG_MAX_ROTATIONS)
}

/// Force a rotation NOW for every active log file (size-cap is bypassed). Used
/// by the `log_rotate` IPC op (`zlog rotate` client). Returns the count of files
/// rotated.
pub fn force_rotate_now(state: &super::state::DaemonState) -> usize {
    let max_rot = log_max_rotations();
    let dir = match std::fs::read_dir(&state.paths.root) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let mut bases: Vec<std::path::PathBuf> = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.starts_with("zshrs.log") || is_rotation_suffix(&s) {
            continue;
        }
        bases.push(entry.path());
    }
    let mut count = 0;
    for base in bases {
        if rotate_one(&base, max_rot).is_ok() {
            count += 1;
        }
    }
    count
}

/// Walk every `zshrs.log*` file (daily-rolled by tracing-appender) and rotate
/// any one whose size exceeds the configured cap. Rotation is in-place: the
/// file is copied to `<basename>.1`, the original is truncated. Existing
/// `.1..N` files shift up; `.N+1` is removed.
///
/// The truncate works without the appender's open fd noticing because
/// tracing-appender opens the file with `O_APPEND`, which positions every
/// write at EOF — so after the file is set to 0 bytes, subsequent writes
/// resume cleanly at offset 0 rather than leaving a sparse gap.
fn rotate_logs_if_needed(state: &super::state::DaemonState) {
    let cap = log_max_bytes();
    let max_rot = log_max_rotations();
    let dir = match std::fs::read_dir(&state.paths.root) {
        Ok(d) => d,
        Err(_) => return,
    };
    let mut bases: Vec<std::path::PathBuf> = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        // The "active" files are those NOT matching the rolled-suffix pattern
        // `<base>.<digits>` — those are our own rotation outputs.
        if !s.starts_with("zshrs.log") {
            continue;
        }
        let path = entry.path();
        if is_rotation_suffix(&s) {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if meta.len() > cap {
                bases.push(path);
            }
        }
    }
    for base in bases {
        if let Err(e) = rotate_one(&base, max_rot) {
            tracing::warn!(?e, base=%base.display(), "ticker: log rotation failed");
        } else {
            tracing::info!(base=%base.display(), "ticker: log rotated (size cap exceeded)");
        }
    }
}

fn is_rotation_suffix(file_name: &str) -> bool {
    // Matches `zshrs.log.<base>.<N>` where N is digits — ours.
    // Daily-rolled looks like `zshrs.log.2026-05-01` — NOT digits-only at end.
    let last_dot = match file_name.rfind('.') {
        Some(i) => i,
        None => return false,
    };
    let suffix = &file_name[last_dot + 1..];
    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
}

fn rotate_one(base: &std::path::Path, max_rot: u32) -> std::io::Result<()> {
    let parent = base.parent().unwrap_or(std::path::Path::new("."));
    let base_name = match base.file_name().and_then(|n| n.to_str()) {
        Some(s) => s.to_string(),
        None => return Ok(()),
    };
    // Drop the oldest if at the cap.
    let oldest = parent.join(format!("{}.{}", base_name, max_rot));
    if oldest.exists() {
        let _ = std::fs::remove_file(&oldest);
    }
    // Shift .N-1 → .N down to .1 → .2.
    for n in (1..max_rot).rev() {
        let from = parent.join(format!("{}.{}", base_name, n));
        let to = parent.join(format!("{}.{}", base_name, n + 1));
        if from.exists() {
            let _ = std::fs::rename(&from, &to);
        }
    }
    // Copy active → .1, then truncate active.
    let dot1 = parent.join(format!("{}.1", base_name));
    std::fs::copy(base, &dot1)?;
    // O_TRUNC against the active path. tracing-appender's append mode means
    // its existing fd will resume writing at offset 0 cleanly.
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(base)?;
    Ok(())
}

fn sweep_tmp(state: &DaemonState) {
    match super::shard::sweep_tmp_files(&state.paths, TMP_AGE) {
        Ok(0) => {}
        Ok(n) => tracing::info!(swept = n, "ticker: orphaned tmp shards cleaned"),
        Err(e) => tracing::warn!(?e, "ticker: tmp sweep failed"),
    }
}

// (size monitoring is now handled inline by rotate_logs_if_needed; the warn-
// only path was replaced with actual rotation per docs/DAEMON.md.)

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
