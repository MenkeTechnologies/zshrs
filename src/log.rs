//! zshrs logging & profiling framework
//!
//! **Logging** (always on):
//!   - File: $HOME/.cache/zshrs/zshrs.log
//!   - Level: ZSHRS_LOG env var (default: info)
//!   - Structured key=value fields, ISO timestamps, thread names, module paths
//!
//! **Profiling** (feature-gated, zero cost when off):
//!   - `--features profiling`  → chrome://tracing JSON  → $HOME/.cache/zshrs/trace-{PID}.json
//!   - `--features flamegraph` → folded stacks          → $HOME/.cache/zshrs/flame-{PID}.folded
//!   - `--features prometheus` → metrics on :9090/metrics
//!
//! Call `zsh::log::init()` once at startup. Use `tracing::{info,debug,trace,warn,error}!`
//! everywhere. Use `#[tracing::instrument]` or `zsh::log::span!` for timed sections.

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

/// Resolve log/profile output directory: $HOME/.cache/zshrs/
pub fn log_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".cache/zshrs")
}

/// Resolve full log path: $HOME/.cache/zshrs/zshrs.log
pub fn log_path() -> PathBuf {
    log_dir().join("zshrs.log")
}

/// Initialize logging + optional profiling subscribers.
/// Safe to call multiple times — only the first call takes effect.
///
/// Env vars:
///   ZSHRS_LOG=debug|trace|info|warn|error  (default: info)
pub fn init() {
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
            .open(dir.join("zshrs.log"))
            .unwrap_or_else(|_| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/zshrs.log")
                    .expect("cannot open any log file")
            });
        let log_writer = std::sync::Mutex::new(log_file);

        let env_filter = std::env::var("ZSHRS_LOG").unwrap_or_else(|_| "info".to_string());

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
            let file = std::fs::File::create(&flame_path)
                .expect("cannot create flamegraph output file");
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
            if let Err(e) = builder
                .with_http_listener(([127, 0, 0, 1], 9090))
                .install()
            {
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
