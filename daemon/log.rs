// Daemon logging — tracing-subscriber → ~/.zshrs/zshrs-daemon.log via tracing-appender.
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
type ReloadFn = Box<dyn Fn(&str) -> std::result::Result<(), String> + Send + Sync + 'static>;

/// Set during `init` only when `try_init` actually installs the global
/// subscriber. Consumed by `set_runtime_level` (the `log_level` IPC op).
static FILTER_RELOAD: OnceLock<ReloadFn> = OnceLock::new();

/// Initialize daemon-wide tracing. Returns a guard whose drop flushes the appender.
///
/// When the env var `ZSHRS_LOG_STDERR=1` is set (or the daemon binary was
/// launched with `--log-stderr`), tracing also fans out to stderr — useful
/// for `daemon-reset.sh` style live-debugging where you want to see every
/// IPC op, fsnotify event, and walk pass scrolling in your terminal.
///
/// Single log file at `~/.zshrs/zshrs-daemon.log`. No daily date suffix.
/// Size-based rotation (10 MB → `.1`, `.1` → `.2`, … up to `.4`) is owned
/// by `ticker::rotate_logs_if_needed`. The appender holds an `O_APPEND` fd
/// against the active file; ticker truncates that file when it crosses
/// the size cap, and the appender resumes writing at offset 0 cleanly.
pub fn init(paths: &CachePaths) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let appender = tracing_appender::rolling::never(&paths.log_dir, &paths.log_file_name);
    // Coerce the log file to 0600 if the appender created it at the umask
    // default (typically 644). Best-effort — done once at init.
    let _ = super::paths::ensure_file_600(&paths.log);

    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    // Filter precedence:
    //   1. $ZSHRS_LOG (env var, ad-hoc / one-shot debugging)
    //   2. [log] level in $ZSHRS_HOME/zshrs-daemon.toml (the persistent
    //      knob — set once, no env var needed every session)
    //   3. "info" hard default
    // Same syntax everywhere — accepts EnvFilter directives like
    // `info`, `debug`, `info,fsnotify=trace,ipc=debug`. `zlog level
    // <directive>` (the IPC op) overrides at runtime via the reload
    // handle below.
    let env_filter = tracing_subscriber::EnvFilter::try_from_env("ZSHRS_LOG")
        .or_else(|_| {
            let directive = super::paths::load_log_directive(paths);
            tracing_subscriber::EnvFilter::try_new(&directive)
        })
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::reload;
    use tracing_subscriber::util::SubscriberInitExt;

    let (filter_layer, filter_handle) = reload::Layer::new(env_filter);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_level(true);

    let stderr_enabled = matches!(
        std::env::var("ZSHRS_LOG_STDERR").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    );

    // Build the registry differently depending on whether the user asked for
    // stderr fanout. The two paths are otherwise identical — same env-filter,
    // same reload handle, same file layer.
    let init_result = if stderr_enabled {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(io::stderr)
            .with_ansi(true)
            .with_target(true)
            .with_level(true);
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(file_layer)
            .with(stderr_layer)
            .try_init()
    } else {
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(file_layer)
            .try_init()
    };

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
