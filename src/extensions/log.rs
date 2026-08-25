//! zshrs logging & profiling framework.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh emits all diagnostics through `zwarn()` / `zwarnnam()` /
//! `zerr()` / `zerrnam()` in Src/utils.c, which print directly to
//! stderr. This module replaces that pattern with a structured
//! `tracing`-based subscriber that writes to `~/.zshrs/zshrs.log`.
//! Per the project's "no startup chatter" rule, all init progress
//! goes here instead of stderr.
//!
//! **Logging** (always on):
//!   - File: `$HOME/.zshrs/{binary}.log` — five separate files so the
//!     processes don't interleave their tracing output:
//!       - `zshrs.log`           (the shell — interactive / scripted)
//!       - `zshrs-lsp.log`       (`zshrs --lsp` — IDE language server)
//!       - `zshrs-dap.log`       (`zshrs --dap` — IDE debug adapter)
//!       - `zshrs-daemon.log`    (daemon, set up via daemon/log.rs)
//!       - `zshrs-recorder.log`  (single-shot recorder runs)
//!     The IDE plugin side has its own `zshrs-plugin.log` written by
//!     `editors/intellij/.../ZshrsDebugLog.kt`.
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

/// Resolve log/profile output directory: `$ZSHRS_HOME` or
/// `$HOME/.zshrs`.
/// zshrs-original — no C counterpart. C zsh has no log file at all
/// (it writes to stderr).
pub fn log_dir() -> PathBuf {
    if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        return PathBuf::from(custom);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".zshrs")
}

/// Resolve the full log path for the shell binary
/// (`zshrs.log`).
/// zshrs-original — no C counterpart.
pub fn log_path() -> PathBuf {
    log_dir().join("zshrs.log")
}

/// Initialize logging + optional profiling subscribers using the
/// default `zshrs.log` filename. Equivalent to
/// `init_named("zshrs.log")`. Safe to call multiple times — only
/// the first call takes effect.
/// zshrs-original — no C counterpart. C zsh's `Src/init.c`
/// `setupshin()` doesn't open a log file.
///
/// Env vars:
///   ZSHRS_LOG=debug|trace|info|warn|error  (default: info)
pub fn init() {
    init_named("zshrs.log");
}

/// Initialize logging + optional profiling subscribers, writing to
/// `$ZSHRS_HOME/<filename>` (or `$HOME/.zshrs/<filename>`). Used
/// by `zshrs-recorder` to land tracing in `zshrs-recorder.log`
/// instead of the shell's `zshrs.log`. The daemon owns its own
/// subscriber via `daemon/log.rs` and uses the path from
/// `paths.log_file_name`.
/// zshrs-original — no C counterpart.
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
        // c:Src/utils.c:1990 movefd — the shell's own descriptors must not sit
        // in the script's fd range. Without this the log landed on fd 3, so
        // `print -u 3 -r -- x` appended to it (and reported success), and
        // `exec 3>file` would dup2 over the live log handle.
        let _lowfd = crate::lowfd::LowFdGuard::new();
        // Size-capped rotation at init (client-side, no background
        // threads — the daemon owns scheduled rotation, but daemonless
        // invocations must still bound the file). Millions of short
        // `-fc` runs (parity fuzz harnesses) each append startup lines;
        // unrotated this reached 18 GB / 142M lines and exhausted the
        // disk. One stat() per shell start; over the cap the current
        // file becomes `<name>.1` (previous `.1` is replaced) and a
        // fresh file starts. rename() keeps writers on the old inode
        // consistent (they finish writing to `.1`).
        const LOG_ROTATE_BYTES: u64 = 512 * 1024 * 1024;
        let log_path = dir.join(filename);
        if std::fs::metadata(&log_path)
            .map(|m| m.len() > LOG_ROTATE_BYTES)
            .unwrap_or(false)
        {
            let _ = std::fs::rename(&log_path, dir.join(format!("{filename}.1")));
        }
        // Logging is diagnostic infrastructure and must NEVER be able to stop
        // the shell from starting. This used to `.expect("cannot open any log
        // file")`, so an unwritable `$HOME` (unmounted NFS, read-only image,
        // `su` to an account whose home is absent, a container with no home)
        // killed zshrs outright where zsh starts fine.
        //
        // The fallback is also per-user. It was a fixed `/tmp/zshrs.log`,
        // which in a shared /tmp is owned by whoever ran zshrs first — every
        // other user then hit EACCES and, before this, the panic. A fixed name
        // in a world-writable directory is equally a squatting target: a
        // pre-created symlink there would have been appended to. `$TMPDIR` is
        // per-user on macOS, and the uid suffix keeps it unambiguous elsewhere.
        let tmp_dir = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        let uid = unsafe { libc::getuid() };
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .or_else(|_| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(std::path::Path::new(&tmp_dir).join(format!("{filename}.{uid}")))
            })
            .ok();
        // `None` => no file layer at all; the shell runs, just without a log.
        // Same `Option<Layer>` shape the chrome/flame layers below already use.
        let file_layer = log_file.map(|f| {
            tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(f))
                .with_ansi(false)
                .with_target(true)
                .with_thread_names(true)
                .compact()
        });

        // Filter precedence:
        //   1. $ZSHRS_LOG (env var, ad-hoc / one-shot debugging)
        //   2. [log] level in $ZSHRS_HOME/zshrs.toml (the persistent
        //      knob — set once, no env var needed every session)
        //   3. "info" hard default
        let env_filter = std::env::var("ZSHRS_LOG")
            .unwrap_or_else(|_| crate::daemon_presence::read_log_directive());

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

        // c:Src/utils.c:2007-2010 — record the log file as a descriptor
        // the shell owns. The appender never exposes it, so it cannot be
        // registered at the open the way `movefd` registers its result;
        // the sweep finds it by elimination. Done here, with the guard
        // above still held, because the sweep only inspects descriptors
        // at or above 10 and the guard's reservations are 3-9.
        crate::lowfd::register_internal_fds();

        Guards {
            #[cfg(feature = "profiling")]
            _chrome: chrome_guard,
            #[cfg(feature = "flamegraph")]
            _flame: flame_guard,
        }
    });
}

/// Flush all log writers. Call before `std::process::exit()` to
/// ensure buffered log data reaches disk — `exit()` doesn't run
/// destructors.
/// zshrs-original — no C counterpart. C zsh's stderr is line-
/// buffered (or unbuffered), so it doesn't need an explicit flush
/// step.
pub fn flush() {
    // The WorkerGuard flushes on drop, but we can't drop a static.
    // Instead, give the non-blocking writer time to drain its buffer.
    // 50ms is more than enough for any reasonable log volume.
    std::thread::sleep(std::time::Duration::from_millis(50));
}

/// Check if the chrome-tracing profiling feature is compiled in.
/// zshrs-original — no C counterpart. C zsh has the `zsh/zprof`
/// module (Src/Modules/zprof.c) for function-level profiling but
/// nothing as fine-grained as chrome:// tracing spans.
pub fn profiling_enabled() -> bool {
    cfg!(feature = "profiling")
}

/// Check if the flamegraph feature is compiled in.
/// zshrs-original — no C counterpart.
pub fn flamegraph_enabled() -> bool {
    cfg!(feature = "flamegraph")
}

/// Check if the Prometheus metrics exporter is compiled in.
/// zshrs-original — no C counterpart.
pub fn prometheus_enabled() -> bool {
    cfg!(feature = "prometheus")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env-mutating tests to avoid races. Other suites may
    // touch ZSHRS_HOME concurrently; this lock keeps us inside ours.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_zshrs_home<F: FnOnce()>(value: Option<&str>, f: F) {
        let _g = ENV_LOCK.lock().unwrap();
        let prev = std::env::var_os("ZSHRS_HOME");
        match value {
            Some(v) => std::env::set_var("ZSHRS_HOME", v),
            None => std::env::remove_var("ZSHRS_HOME"),
        }
        f();
        match prev {
            Some(v) => std::env::set_var("ZSHRS_HOME", v),
            None => std::env::remove_var("ZSHRS_HOME"),
        }
    }

    #[test]
    fn log_dir_honors_zshrs_home_env_var() {
        with_zshrs_home(Some("/tmp/zshrs-test-home"), || {
            assert_eq!(log_dir(), PathBuf::from("/tmp/zshrs-test-home"));
        });
    }

    #[test]
    fn log_path_appends_zshrs_log_filename() {
        with_zshrs_home(Some("/tmp/zshrs-test-path"), || {
            assert_eq!(log_path(), PathBuf::from("/tmp/zshrs-test-path/zshrs.log"));
        });
    }

    #[test]
    fn log_dir_falls_back_when_zshrs_home_unset() {
        with_zshrs_home(None, || {
            let dir = log_dir();
            // Must end in `.zshrs` regardless of whether HOME is set
            // (no HOME → /tmp/.zshrs).
            assert_eq!(dir.file_name().and_then(|s| s.to_str()), Some(".zshrs"));
        });
    }

    #[test]
    fn feature_flag_helpers_match_cfg() {
        // Each helper must agree with the underlying cfg!() macro —
        // a regression here would mean someone special-cased the
        // helper independent of the feature.
        assert_eq!(profiling_enabled(), cfg!(feature = "profiling"));
        assert_eq!(flamegraph_enabled(), cfg!(feature = "flamegraph"));
        assert_eq!(prometheus_enabled(), cfg!(feature = "prometheus"));
    }

    // ========================================================
    // log_dir — ZSHRS_HOME precedence and fallback
    // ========================================================

    #[test]
    fn log_dir_returns_absolute_when_zshrs_home_absolute() {
        with_zshrs_home(Some("/var/log/zshrs"), || {
            let d = log_dir();
            assert!(d.is_absolute(), "log_dir should be absolute: {:?}", d);
            assert_eq!(d, PathBuf::from("/var/log/zshrs"));
        });
    }

    #[test]
    fn log_dir_honors_relative_zshrs_home_verbatim() {
        // No magic normalization — whatever ZSHRS_HOME is, log_dir
        // returns it (with the `.zshrs` suffix replaced).
        with_zshrs_home(Some("relative/path"), || {
            assert_eq!(log_dir(), PathBuf::from("relative/path"));
        });
    }

    #[test]
    fn log_dir_distinct_from_log_path() {
        with_zshrs_home(Some("/tmp/zshrs-distinct"), || {
            let d = log_dir();
            let p = log_path();
            assert_ne!(d, p, "dir and path must differ");
            assert_eq!(p.parent(), Some(d.as_path()));
        });
    }

    #[test]
    fn log_path_filename_is_zshrs_dot_log() {
        with_zshrs_home(Some("/tmp/zshrs-fname"), || {
            assert_eq!(
                log_path().file_name().and_then(|s| s.to_str()),
                Some("zshrs.log")
            );
        });
    }

    #[test]
    fn log_dir_overrides_persist_for_repeated_calls() {
        // Calling twice in sequence should return identical values
        // — confirms there is no hidden caching that pins the first
        // call's result.
        with_zshrs_home(Some("/tmp/zshrs-cache-1"), || {
            assert_eq!(log_dir(), PathBuf::from("/tmp/zshrs-cache-1"));
        });
        with_zshrs_home(Some("/tmp/zshrs-cache-2"), || {
            assert_eq!(log_dir(), PathBuf::from("/tmp/zshrs-cache-2"));
        });
    }

    #[test]
    fn log_dir_empty_zshrs_home_is_taken_verbatim() {
        // env::var_os returns Some("") for empty — code path uses
        // PathBuf::from("") which still gets returned. Verify no
        // panic / silent fallback.
        with_zshrs_home(Some(""), || {
            assert_eq!(log_dir(), PathBuf::from(""));
        });
    }

    #[test]
    fn log_dir_fallback_path_ends_with_dot_zshrs_filename_component() {
        with_zshrs_home(None, || {
            let d = log_dir();
            assert_eq!(d.file_name().and_then(|s| s.to_str()), Some(".zshrs"));
        });
    }

    #[test]
    fn log_path_inside_zshrs_home_directory() {
        with_zshrs_home(Some("/tmp/zshrs-inside"), || {
            let p = log_path();
            assert!(
                p.starts_with("/tmp/zshrs-inside"),
                "log_path should live inside ZSHRS_HOME: {:?}",
                p
            );
        });
    }

    // ========================================================
    // Feature-flag helpers — interaction matrix
    // ========================================================

    #[test]
    fn feature_helpers_are_stable_within_one_call() {
        // Two back-to-back calls produce identical output (no random
        // toggling). Trivial sanity check, catches any future
        // accidental side-effect inside the helpers.
        assert_eq!(profiling_enabled(), profiling_enabled());
        assert_eq!(flamegraph_enabled(), flamegraph_enabled());
        assert_eq!(prometheus_enabled(), prometheus_enabled());
    }

    #[test]
    fn flush_is_idempotent_and_returns_unit() {
        // `flush` sleeps ~50ms but must always return cleanly. Two
        // back-to-back calls should still be safe.
        flush();
        flush();
    }
}
