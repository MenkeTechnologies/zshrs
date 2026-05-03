// Daemon for zshrs — see docs/DAEMON.md.
//
// Module layout:
//   paths.rs   — ~/.zshrs/* paths + 0700/0600 permissions
//   log.rs     — tracing-subscriber setup, rolling file at zshrs.log
//   ipc.rs     — u32-BE-length-prefixed JSON framing + message types
//   pidlock.rs — singleton flock on daemon.pid + spawn-on-demand
//   server.rs  — tokio accept loop + per-session connection handler
//   state.rs   — DaemonState (sessions, tags, registry, broadcast channels)
//   ops.rs     — IPC operation dispatch
//   client.rs  — client-side IPC helpers used by z* builtins
//   firstrun.rs — first-run detection + 6-line stderr notice

pub mod artifact;
pub mod auth;
pub mod builtins;
pub mod cache;
pub mod canonical;
pub mod catalog;
pub mod client;
pub mod definitions;
pub mod export;
pub mod firstrun;
pub mod fsnotify;
pub mod history;
pub mod http;
pub mod ipc;
pub mod jobs;
pub mod lock;
pub mod log;
pub mod metrics;
pub mod ops;
pub mod paths;
pub mod pidlock;
pub mod pubsub;
pub mod schedule;
pub mod server;
pub mod shard;
pub mod snapshot;
pub mod source_resolver;
pub mod state;
pub mod ticker;
pub mod zask;
pub mod zask_builtin;
pub mod zcomplete_builtin;
pub mod zhistory_builtin;
pub mod zjob_builtin;
pub mod zsource_builtin;
pub mod zsync;
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

#[derive(thiserror::Error, Debug)]
pub enum DaemonError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("nix: {0}")]
    Nix(#[from] nix::Error),

    #[error("rusqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("singleton: another daemon is running (pid {0})")]
    AlreadyRunning(i32),

    #[error("protocol: client v{client} incompatible with daemon v{daemon}")]
    ProtocolMismatch { client: u32, daemon: u32 },

    #[error("protocol: malformed handshake")]
    BadHandshake,

    #[error("protocol: frame too large ({size} > {max})")]
    FrameTooLarge { size: usize, max: usize },

    #[error("daemon: shutting down")]
    Shutdown,

    #[error("op: unknown opcode {0:?}")]
    UnknownOp(String),

    #[error("op: bad args for {op}: {reason}")]
    BadArgs { op: String, reason: String },

    #[error("client: not connected to daemon")]
    NotConnected,

    #[error("client: timed out after {0:?}")]
    Timeout(std::time::Duration),

    #[error("{0}")]
    Other(String),
}

impl DaemonError {
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

    // Logging first so subsequent setup is observable.
    let _log_guard = log::init(&paths)?;

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "zshrs-daemon starting");

    // Singleton enforcement. Holds the lock for daemon lifetime.
    let _pid_lock = pidlock::acquire(&paths)?;

    // Spin up tokio runtime + run server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("zshrs-daemon")
        .build()?;

    let result = rt.block_on(server::serve(paths.clone()));

    tracing::info!("zshrs-daemon exiting");

    result
}
