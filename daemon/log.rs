// Daemon logging — tracing-subscriber → ~/.cache/zshrs/zshrs.log via tracing-appender.
//
// Per docs/DAEMON.md "Daemon logging (every action goes to logfile)":
//   - Default level: INFO
//   - Override: ZSHRS_LOG=debug or per-module (ZSHRS_LOG=info,fsnotify=debug,ipc=trace)
//   - Format: tracing default — [timestamp] LEVEL [module] msg {key=value}
//   - Rotation: 10 MB per file, 4 archives kept (the ticker handles size-based
//     rotation in-place; tracing-appender's DAILY roller continues running for
//     time-based archives)
//   - `zlog level <directive>` swaps the active EnvFilter at runtime via the
//     reload closure stored in the OnceLock below. No daemon restart required.
//
// Returns a guard the caller must keep alive for the lifetime of the process; dropping it
// flushes the writer.

use std::io;
use std::sync::OnceLock;

use super::{paths::CachePaths, Result};

/// Type-erased reload closure — boxed so the deeply-generic
/// `reload::Handle<EnvFilter, Layered<…>>` type doesn't leak through this API.
type ReloadFn =
    Box<dyn Fn(&str) -> std::result::Result<(), String> + Send + Sync + 'static>;

/// Set during `init` only when `try_init` actually installs the global
/// subscriber. Consumed by `set_runtime_level` (the `log_level` IPC op).
static FILTER_RELOAD: OnceLock<ReloadFn> = OnceLock::new();

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
    use tracing_subscriber::reload;
    use tracing_subscriber::util::SubscriberInitExt;

    let (filter_layer, filter_handle) = reload::Layer::new(env_filter);

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_level(true);

    // Only store the reload closure if try_init actually installed the global.
    // If a global was already set (test reuse, lib double-init), the layered
    // registry is dropped along with our reload-Layer's receiver — keeping the
    // closure would point at a dropped subscriber and break `zlog level`.
    let init_result = tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .try_init();

    if init_result.is_ok() {
        let reload_fn: ReloadFn = Box::new(move |directive: &str| {
            let new_filter = tracing_subscriber::EnvFilter::try_new(directive)
                .map_err(|e| format!("bad directive `{}`: {}", directive, e))?;
            filter_handle
                .reload(new_filter)
                .map_err(|e| format!("reload failed: {}", e))
        });
        let _ = FILTER_RELOAD.set(reload_fn);
    }

    Ok(guard)
}

/// Swap the active EnvFilter at runtime. `directive` accepts the same syntax
/// as `ZSHRS_LOG` (e.g. `info`, `debug`, `info,fsnotify=debug,ipc=trace`).
pub fn set_runtime_level(directive: &str) -> std::result::Result<(), String> {
    let f = FILTER_RELOAD
        .get()
        .ok_or_else(|| "log subsystem not reload-capable (init returned Err — likely a global subscriber was already set)".to_string())?;
    f(directive)
}
