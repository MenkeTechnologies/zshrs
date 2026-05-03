//! zshrs logging & profiling framework
//!
//! **Logging** (always on):
//!   - File: $HOME/.zshrs/<binary>.log — three separate files so the
//!     three processes don't interleave their tracing output:
//!       - `zshrs.log`           (the shell — thin clients)
//!       - `zshrs-daemon.log`    (daemon, set up via daemon/log.rs)
//!       - `zshrs-recorder.log`  (single-shot recorder runs)
//!   - Level: ZSHRS_LOG env var (default: info)
//!   - Structured key=value fields, ISO timestamps, thread names, module paths
//!
//! **Profiling** (feature-gated, zero cost when off):
//!   - `--features profiling`  → chrome://tracing JSON  → $HOME/.zshrs/trace-{PID}.json
//!   - `--features flamegraph` → folded stacks          → $HOME/.zshrs/flame-{PID}.folded
//!   - `--features prometheus` → metrics on :9090/metrics
//!
//! Call `zsh::log::init()` (defaults to `zshrs.log`) or
//! `zsh::log::init_named("…")` once at startup. Use
//! `tracing::{info,debug,trace,warn,error}!` everywhere. Use
//! `#[tracing::instrument]` or `zsh::log::span!` for timed sections.

use std::path::PathBuf;
use std::sync::OnceLock;
use tracing_subscriber::prelude::*;

/// Guards that must live for the duration of the process.
/// Dropping any of these flushes and stops the associated writer.
struct Guards {
    #[cfg(feature = "profiling")]
    _chrome: tracing_chrome::FlushGuard,
    #[cfg(feature = "flamegraph")]
    _flame: tracing_flame::FlushGuard<std::io::BufWriter<std::fs::File>>,
}

static GUARDS: OnceLock<Guards> = OnceLock::new();

/// Resolve log/profile output directory: $ZSHRS_HOME or $HOME/.zshrs
pub fn log_dir() -> PathBuf {
    if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        return PathBuf::from(custom);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".zshrs")
}

/// Resolve full log path for the shell binary (zshrs.log).
pub fn log_path() -> PathBuf {
    log_dir().join("zshrs.log")
}

/// Initialize logging + optional profiling subscribers using the default
/// `zshrs.log` filename. Equivalent to `init_named("zshrs.log")`.
/// Safe to call multiple times — only the first call takes effect.
///
/// Env vars:
///   ZSHRS_LOG=debug|trace|info|warn|error  (default: info)
pub fn init() {
    init_named("zshrs.log");
}

/// Initialize logging + optional profiling subscribers, writing to
/// `$ZSHRS_HOME/<filename>` (or `$HOME/.zshrs/<filename>`). Used by
/// `zshrs-recorder` to land tracing in `zshrs-recorder.log` instead of
/// the shell's `zshrs.log`. The daemon owns its own subscriber via
/// `daemon/log.rs` and uses the path from `paths.log_file_name`.
///
/// Safe to call multiple times — only the first call takes effect.
pub fn init_named(filename: &str) {
    GUARDS.get_or_init(|| {
        let dir = log_dir();
        let _ = std::fs::create_dir_all(&dir);
        let pid = std::process::id();

        // --- File log layer (always on) ---
        // Use a blocking Mutex<File> writer — log writes are microseconds and this
        // guarantees data reaches disk even when std::process::exit() skips destructors.
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(filename))
            .unwrap_or_else(|_| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(format!("/tmp/{}", filename))
                    .expect("cannot open any log file")
            });
        let log_writer = std::sync::Mutex::new(log_file);

        // Filter precedence:
        //   1. $ZSHRS_LOG (env var, ad-hoc / one-shot debugging)
        //   2. [log] level in $ZSHRS_HOME/zshrs.toml (the persistent
        //      knob — set once, no env var needed every session)
        //   3. "info" hard default
        let env_filter = std::env::var("ZSHRS_LOG")
            .unwrap_or_else(|_| crate::daemon_presence::read_log_directive());

        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(log_writer)
            .with_ansi(false)
            .with_target(true)
            .with_thread_names(true)
            .compact();

        // --- Chrome tracing layer (--features profiling) ---
        #[cfg(feature = "profiling")]
        let (chrome_layer, chrome_guard) = {
            let trace_path = dir.join(format!("trace-{}.json", pid));
            let (layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
                .file(trace_path)
                .include_args(true)
                .build();
            (Some(layer), guard)
        };
        #[cfg(not(feature = "profiling"))]
        let chrome_layer: Option<tracing_subscriber::layer::Identity> = None;

        // --- Flamegraph layer (--features flamegraph) ---
        #[cfg(feature = "flamegraph")]
        let (flame_layer, flame_guard) = {
            let flame_path = dir.join(format!("flame-{}.folded", pid));
            let file =
                std::fs::File::create(&flame_path).expect("cannot create flamegraph output file");
            let writer = std::io::BufWriter::new(file);
            let (layer, guard) = tracing_flame::FlameLayer::with_writer(writer).build();
            (Some(layer), guard)
        };
        #[cfg(not(feature = "flamegraph"))]
        let flame_layer: Option<tracing_subscriber::layer::Identity> = None;

        // --- Prometheus metrics (--features prometheus) ---
        #[cfg(feature = "prometheus")]
        {
            // Spawn metrics HTTP server on :9090 in background
            let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
            if let Err(e) = builder.with_http_listener(([127, 0, 0, 1], 9090)).install() {
                eprintln!("zshrs: failed to start prometheus exporter: {}", e);
            }
        }

        // --- Assemble the subscriber registry ---
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(&env_filter))
            .with(file_layer)
            .with(chrome_layer)
            .with(flame_layer);

        let _ = tracing::subscriber::set_global_default(subscriber);

        Guards {
            #[cfg(feature = "profiling")]
            _chrome: chrome_guard,
            #[cfg(feature = "flamegraph")]
            _flame: flame_guard,
        }
    });
}

/// Flush all log writers. Call before std::process::exit() to ensure
/// buffered log data reaches disk — exit() doesn't run destructors.
pub fn flush() {
    // The WorkerGuard flushes on drop, but we can't drop a static.
    // Instead, give the non-blocking writer time to drain its buffer.
    // 50ms is more than enough for any reasonable log volume.
    std::thread::sleep(std::time::Duration::from_millis(50));
}

/// Convenience: check if profiling features are compiled in
pub fn profiling_enabled() -> bool {
    cfg!(feature = "profiling")
}

pub fn flamegraph_enabled() -> bool {
    cfg!(feature = "flamegraph")
}

pub fn prometheus_enabled() -> bool {
    cfg!(feature = "prometheus")
}
