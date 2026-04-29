// Daemon for zshrs — see docs/DAEMON.md.
//
// Module layout:
//   paths.rs   — ~/.cache/zshrs/* paths + 0700/0600 permissions
//   log.rs     — tracing-subscriber setup, rolling file at zshrs.log
//   ipc.rs     — u32-BE-length-prefixed JSON framing + message types
//   pidlock.rs — singleton flock on daemon.pid + spawn-on-demand
//   server.rs  — tokio accept loop + per-session connection handler
//   state.rs   — DaemonState (sessions, tags, registry, broadcast channels)
//   ops.rs     — IPC operation dispatch
//   client.rs  — client-side IPC helpers used by z* builtins
//   firstrun.rs — first-run detection + 6-line stderr notice

pub mod builtins;
pub mod catalog;
pub mod client;
pub mod firstrun;
pub mod ipc;
pub mod log;
pub mod ops;
pub mod paths;
pub mod pidlock;
pub mod server;
pub mod shard;
pub mod source_resolver;
pub mod state;

pub use ipc::{Event, Frame, Hello, Welcome, ProtocolVersion, PROTOCOL_VERSION};
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
