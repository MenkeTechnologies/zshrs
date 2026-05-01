// Daemon logging — tracing-subscriber → ~/.cache/zshrs/zshrs.log via tracing-appender.
//
// Per docs/DAEMON.md "Daemon logging (every action goes to logfile)":
//   - Default level: INFO
//   - Override: ZSHRS_LOG=debug or per-module (ZSHRS_LOG=info,fsnotify=debug,ipc=trace)
//   - Format: tracing default — [timestamp] LEVEL [module] msg {key=value}
//   - Rotation: 10 MB per file, 4 archives kept (handled by tracing-appender's daily roller
//     plus a future ticker-driven re-roll on size; for the v1 daemon we let
//     tracing-appender::rolling do daily rotation and rely on size-cap enforcement at
//     `zlog rotate` / ticker level)
//
// Returns a guard the caller must keep alive for the lifetime of the process; dropping it
// flushes the writer.

use std::io;

use super::{paths::CachePaths, Result};

/// Initialize daemon-wide tracing. Returns a guard whose drop flushes the appender.
pub fn init(paths: &CachePaths) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let appender = tracing_appender::rolling::Builder::new()
        .filename_prefix(&paths.log_file_name)
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .max_log_files(5)
        .build(&paths.log_dir)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("log appender: {e}")))?;

    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    let env_filter = tracing_subscriber::EnvFilter::try_from_env("ZSHRS_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_level(true);

    // Once per process. If a prior test run installed a global subscriber, the second
    // call returns Err — we ignore that path so daemon binary install always wins and tests
    // can reuse subscribers across runs without panicking.
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init();

    Ok(guard)
}
