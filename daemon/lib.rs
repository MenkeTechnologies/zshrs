// Daemon for zshrs — see docs/DAEMON.md.
//
// Module layout:
//   paths.rs   — ~/.zshrs/* paths + 0700/0600 permissions
//   log.rs     — tracing-subscriber setup, rolling file at zshrs-daemon.log
//   ipc.rs     — u32-BE-length-prefixed JSON framing + message types
//   pidlock.rs — singleton flock on daemon.pid + spawn-on-demand
//   server.rs  — tokio accept loop + per-session connection handler
//   state.rs   — DaemonState (sessions, tags, registry, broadcast channels)
//   ops.rs     — IPC operation dispatch
//   client.rs  — client-side IPC helpers used by z* builtins
//   firstrun.rs — first-run detection + 6-line stderr notice
/// `artifact` submodule.

pub mod artifact;
/// `auth` submodule.
pub mod auth;
/// `builtins` submodule.
pub mod builtins;
/// `cache` submodule.
pub mod cache;
/// `canonical` submodule.
pub mod canonical;
/// `catalog` submodule.
pub mod catalog;
/// `client` submodule.
pub mod client;
/// `definitions` submodule.
pub mod definitions;
/// `export` submodule.
pub mod export;
/// `firstrun` submodule.
pub mod firstrun;
/// `fsnotify` submodule.
pub mod fsnotify;
/// `history` submodule.
pub mod history;
/// `http` submodule.
pub mod http;
/// `ipc` submodule.
pub mod ipc;
/// `jobs` submodule.
pub mod jobs;
/// `lock` submodule.
pub mod lock;
/// `log` submodule.
pub mod log;
/// `metrics` submodule.
pub mod metrics;
/// `ops` submodule.
pub mod ops;
/// `paths` submodule.
pub mod paths;
/// `pidlock` submodule.
pub mod pidlock;
/// `pubsub` submodule.
pub mod pubsub;
/// `schedule` submodule.
pub mod schedule;
/// `server` submodule.
pub mod server;
/// `shard` submodule.
pub mod shard;
/// `snapshot` submodule.
pub mod snapshot;
/// `source_resolver` submodule.
pub mod source_resolver;
/// `state` submodule.
pub mod state;
/// `ticker` submodule.
pub mod ticker;
/// `zask` submodule.
pub mod zask;
/// `zask_builtin` submodule.
pub mod zask_builtin;
/// `zcomplete_builtin` submodule.
pub mod zcomplete_builtin;
/// `zd_dispatch` submodule.
pub mod zd_dispatch;
/// `zhistory_builtin` submodule.
pub mod zhistory_builtin;
/// `zjob_builtin` submodule.
pub mod zjob_builtin;
/// `zsource_builtin` submodule.
pub mod zsource_builtin;
/// `zsync` submodule.
pub mod zsync;
/// `zsync_builtin` submodule.
pub mod zsync_builtin;

// The static AST-walk pipeline (`ast_walker`, `walk`, `plugin_walk`,
// `zshrc_analysis`) was removed: state attribution comes from
// `zshrs-recorder` (runtime AOP intercept, see docs/RECORDER.md) instead
// of parsing user config files. The daemon still services cache ops and
// fsnotify-driven shard-update broadcasts, but never re-derives state by
// walking sources. New plugin installs require the user to re-run
// `zshrs-recorder`.

pub use ipc::{Event, Frame, Hello, ProtocolVersion, Welcome, PROTOCOL_VERSION};
pub use paths::CachePaths;

/// Result type used throughout the daemon.
pub type Result<T> = std::result::Result<T, DaemonError>;
/// `DaemonError` — see variants.

#[derive(thiserror::Error, Debug)]
pub enum DaemonError {
    /// `Io` variant.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// `Json` variant.

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// `Nix` variant.

    #[error("nix: {0}")]
    Nix(#[from] nix::Error),
    /// `Sqlite` variant.

    #[error("rusqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// `AlreadyRunning` variant.

    #[error("singleton: another daemon is running (pid {0})")]
    AlreadyRunning(i32),

    #[error("protocol: client v{client} incompatible with daemon v{daemon}")]
    ProtocolMismatch { client: u32, daemon: u32 },
    /// `BadHandshake` variant.

    #[error("protocol: malformed handshake")]
    BadHandshake,

    #[error("protocol: frame too large ({size} > {max})")]
    FrameTooLarge { size: usize, max: usize },
    /// `Shutdown` variant.

    #[error("daemon: shutting down")]
    Shutdown,
    /// `UnknownOp` variant.

    #[error("op: unknown opcode {0:?}")]
    UnknownOp(String),

    #[error("op: bad args for {op}: {reason}")]
    BadArgs { op: String, reason: String },
    /// `NotConnected` variant.

    #[error("client: not connected to daemon")]
    NotConnected,
    /// `Timeout` variant.

    #[error("client: timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// `Other` variant.

    #[error("{0}")]
    Other(String),
}

impl DaemonError {
    /// `other` — see implementation.
    pub fn other<S: Into<String>>(msg: S) -> Self {
        Self::Other(msg.into())
    }
}

/// Run the daemon to completion. Used by `zshrs --daemon` mode.
///
/// Acquires the singleton lock, opens IPC socket, sets up logging, runs the accept loop
/// until SIGTERM/SIGINT or a `daemon stop` IPC op.
pub fn run() -> Result<()> {
    let paths = paths::CachePaths::resolve()?;
    paths.ensure_dirs()?;

    // Seed zshrs.toml + zshrs-daemon.toml + zshrs-recorder.toml with defaults BEFORE log::init so
    // the new [log] level directive can be picked up on first run.
    // Idempotent — never overwrites user edits.
    if let Err(e) = paths.ensure_default_configs() {
        eprintln!("zshrs-daemon: failed to seed default configs: {e}");
    }

    // Logging next so subsequent setup is observable.
    let _log_guard = log::init(&paths)?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "zshrs-daemon starting");

    // First-pass diagnostics — gated to TRACE so they don't flood
    // the log at default INFO. Bump `[log] level = "trace"` in
    // zshrs-daemon.toml (or `ZSHRS_LOG=trace`) to see them.
    let pid = std::process::id();
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let zshrs_home = std::env::var("ZSHRS_HOME").unwrap_or_default();
    tracing::trace!(
        pid,
        cwd,
        zshrs_home = if zshrs_home.is_empty() { "<unset>" } else { &zshrs_home },
        root = %paths.root.display(),
        socket = %paths.socket.display(),
        pid_file = %paths.pid_file.display(),
        log = %paths.log.display(),
        daemon_toml = %paths.daemon_config_path().display(),
        shell_toml = %paths.shell_config_path().display(),
        catalog_db = %paths.catalog_db.display(),
        history_db = %paths.history_db.display(),
        cache_db = %paths.cache_db.display(),
        images_dir = %paths.images.display(),
        artifacts_dir = %paths.artifacts_dir.display(),
        snapshots_dir = %paths.snapshots_dir.display(),
        replay_dir = %paths.replay_dir.display(),
        "daemon: resolved environment"
    );

    // Singleton enforcement. Holds the lock for daemon lifetime.
    let _pid_lock = pidlock::acquire(&paths)?;
    tracing::trace!(pid_file = %paths.pid_file.display(), pid, "daemon: pidlock acquired");

    // Spin up tokio runtime + run server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("zshrs-daemon")
        .build()?;

    let result = rt.block_on(server::serve(paths.clone()));

    tracing::info!("zshrs-daemon exiting");

    result
}
