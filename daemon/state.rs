// DaemonState — shared mutable state owned by the running daemon.
//
// Per docs/DAEMON.md "Daemon owns": session table, tag map, subscription map, broadcast
// channels for cross-shell pub/sub. All access is via parking_lot::Mutex (the daemon is
// fat — it can afford a global lock for control-plane operations; data-plane goes through
// mmap which doesn't touch this state).
//
// For v1 foundation we only implement the session registry (used by zls/zid/ztag/zsend);
// later iterations will fold in subscription map, fpath cache, FTS indexes, etc.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot};

use super::catalog::{self, CatalogSummary};
use super::history;
use super::ipc::Frame;
use super::paths::CachePaths;
use super::pubsub::{Scope, Subscription};
use super::Result;

/// One client/shell session.
pub struct Session {
    /// `client_id` field.
    pub client_id: u64,
    /// `session_id` field.
    pub session_id: String,
    /// `pid` field.
    pub pid: i32,
    /// `tty` field.
    pub tty: Option<String>,
    /// `cwd` field.
    pub cwd: Option<String>,
    /// `argv0` field.
    pub argv0: Option<String>,
    /// Stable shell id this connection belongs to (the record keyed by the
    /// shell pid from the Hello). `None` for HTTP synthetic sessions, which
    /// are not shells and never create or join a shell record.
    pub shell_id: Option<u64>,
    /// `connected_at` field.
    pub connected_at: Instant,
    /// `login_time` field.
    pub login_time: chrono::DateTime<chrono::Utc>,
    /// Outbound channel — daemon writes async events / responses here, the connection
    /// handler task drains and sends them on the wire.
    pub outbound: mpsc::UnboundedSender<Frame>,
    /// Per-session opt-in flag for `recorder_ingested` (DEFINITIONS) events.
    /// Off by default so silent IPC clients don't receive every recorder
    /// bundle's summary frame. Toggled by `definitions_subscribe` /
    /// `definitions_unsubscribe`. The HTTP `/stream/definitions` handler
    /// auto-subscribes its synthetic session for SSE delivery. See
    /// docs/DAEMON_AS_SERVICE.md §"Definitions" subscribe path.
    pub definitions_subscribed: bool,
}

impl Session {
    /// `snapshot` — see implementation.
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            client_id: self.client_id,
            shell_id: self.shell_id,
            session_id: self.session_id.clone(),
            pid: self.pid,
            tty: self.tty.clone(),
            cwd: self.cwd.clone(),
            argv0: self.argv0.clone(),
            login_time: self.login_time.to_rfc3339(),
            uptime_secs: self.connected_at.elapsed().as_secs(),
        }
    }
}
/// One live connection, as seen by `snapshot_sessions` (connection-level
/// view; `zls` lists shells via `ShellSnapshot` instead).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SessionSnapshot {
    /// Per-connection id (the `client_id` in the Welcome).
    pub client_id: u64,
    /// Stable shell id, `None` for HTTP synthetic sessions.
    pub shell_id: Option<u64>,
    /// `session_id` field.
    pub session_id: String,
    /// `pid` field.
    pub pid: i32,
    /// `tty` field.
    pub tty: Option<String>,
    /// `cwd` field.
    pub cwd: Option<String>,
    /// `argv0` field.
    pub argv0: Option<String>,
    /// `login_time` field.
    pub login_time: String,
    /// `uptime_secs` field.
    pub uptime_secs: u64,
}

/// One registered shell. Keyed by the shell pid the Hello carries
/// (`shell_pid`, falling back to `client_pid`); outlives the connections
/// that created it, so per-shell state set by one one-shot builtin call is
/// seen by the next. Removed only by `reap_dead_shells` once the pid is gone.
pub struct ShellRecord {
    /// Daemon-minted id, stable for the life of the pid. What `zid` prints,
    /// what `zls` lists, what `shell:N` resolves against.
    pub shell_id: u64,
    /// The shell's pid (`$$`).
    pub pid: i32,
    /// The shell's start time from the Hello (`shell_start`); with `pid`
    /// it tells a recycled pid from the shell that held it before.
    pub start: Option<u64>,
    /// Most recent tty reported by any of this shell's connections.
    pub tty: Option<String>,
    /// Most recent cwd reported by any of this shell's connections.
    pub cwd: Option<String>,
    /// Most recent argv0 reported by any of this shell's connections.
    pub argv0: Option<String>,
    /// `ztag` / `zuntag` tags.
    pub tags: BTreeSet<String>,
    /// First registration (wall clock).
    pub login_time: chrono::DateTime<chrono::Utc>,
    /// First registration (monotonic, for uptime).
    pub registered_at: Instant,
}

/// Serializable view of a `ShellRecord` — one `zls` row / `list_shells` entry.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ShellSnapshot {
    /// Stable shell id.
    pub shell_id: u64,
    /// The shell's pid.
    pub pid: i32,
    /// `tty` field.
    pub tty: Option<String>,
    /// `cwd` field.
    pub cwd: Option<String>,
    /// `argv0` field.
    pub argv0: Option<String>,
    /// `tags` field.
    pub tags: Vec<String>,
    /// `login_time` field.
    pub login_time: String,
    /// Seconds since the shell first registered.
    pub uptime_secs: u64,
    /// Live connections this shell holds right now (usually 0 between
    /// builtin calls — push events reach a shell only while this is > 0).
    pub connections: usize,
    /// Requests waiting in this shell's `zask` queue (`zls --ask-pending`).
    pub ask_pending: usize,
}

/// Inner mutable state behind a single mutex.
pub struct DaemonStateInner {
    /// Live connections, keyed by per-connection client_id.
    pub sessions: BTreeMap<u64, Session>,
    /// `next_client_id` field.
    pub next_client_id: u64,
    /// Registered shells, keyed by stable shell id.
    pub shells: BTreeMap<u64, ShellRecord>,
    /// Shell pid → stable shell id.
    pub shell_by_pid: HashMap<i32, u64>,
    /// `next_shell_id` field.
    pub next_shell_id: u64,
    /// `subscriptions` field.
    pub subscriptions: BTreeMap<u64, Subscription>,
    /// `next_subscription_id` field.
    pub next_subscription_id: u64,
    /// Pending zsend --wait responses, keyed by delivery_id. Sender side
    /// holds a oneshot::Receiver waiting for a `cmd_result` IPC from the
    /// target shell.
    pub pending_responses: HashMap<String, oneshot::Sender<serde_json::Value>>,
    /// Runtime-tunable config knobs. Per docs/DAEMON.md:905
    /// `zcache config set <key> <value>` mutates this map; readers fall back
    /// to env vars when the key isn't set. Keys: long_cmd_threshold (seconds),
    /// log_max_bytes, etc.
    pub config: HashMap<String, String>,
}

impl DaemonStateInner {
    fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            next_client_id: 1,
            shells: BTreeMap::new(),
            shell_by_pid: HashMap::new(),
            next_shell_id: 1,
            subscriptions: BTreeMap::new(),
            next_subscription_id: 1,
            pending_responses: HashMap::new(),
            config: HashMap::new(),
        }
    }

    fn shell_of(&self, client_id: u64) -> Option<u64> {
        self.sessions.get(&client_id).and_then(|s| s.shell_id)
    }

    fn shell_snapshot(&self, r: &ShellRecord) -> ShellSnapshot {
        ShellSnapshot {
            shell_id: r.shell_id,
            pid: r.pid,
            tty: r.tty.clone(),
            cwd: r.cwd.clone(),
            argv0: r.argv0.clone(),
            tags: r.tags.iter().cloned().collect(),
            login_time: r.login_time.to_rfc3339(),
            uptime_secs: r.registered_at.elapsed().as_secs(),
            connections: self
                .sessions
                .values()
                .filter(|s| s.shell_id == Some(r.shell_id))
                .count(),
            ask_pending: 0,
        }
    }

    /// Does the caller own this subscription? A shell caller owns every
    /// subscription its shell made (from any connection); an HTTP session
    /// owns only the ones made on that connection.
    fn owns_subscription(&self, client_id: u64, sub: &Subscription) -> bool {
        match self.shell_of(client_id) {
            Some(shell) => sub.shell_id == Some(shell),
            None => sub.shell_id.is_none() && sub.client_id == client_id,
        }
    }
}

/// `kill(pid, 0)` liveness: alive unless the pid does not exist (ESRCH).
/// EPERM means a process exists under another uid — still alive.
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Ok(()) => true,
        Err(e) => e != nix::errno::Errno::ESRCH,
    }
}

/// The kernel's start time for `pid`, as an opaque token: equal values mean
/// the same process, a different value on the same pid means the pid was
/// recycled. The shell sends its own in the Hello (`shell_start`); the daemon
/// reads it again for live pids when reaping. Units are per platform and
/// only ever compared on one machine:
/// - macOS: `proc_pidinfo(PROC_PIDTBSDINFO)` → `pbi_start_tvsec` * 10^6 +
///   `pbi_start_tvusec` (µs since the epoch).
/// - Linux: field 22 (`starttime`, clock ticks since boot) of
///   `/proc/<pid>/stat`.
/// - Elsewhere, or if the read fails: `None` (identity falls back to pid).
pub fn process_start_time(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }
    process_start_time_os(pid)
}

#[cfg(target_os = "macos")]
fn process_start_time_os(pid: i32) -> Option<u64> {
    // SAFETY: proc_pidinfo writes at most `size` bytes into `info`, a plain
    // C struct for which all-zero bytes are a valid value.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    if n != size {
        return None;
    }
    Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(target_os = "linux")]
fn process_start_time_os(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Field 2 (comm) is parenthesised and may hold spaces or `)`; fields
    // resume after the LAST `)`, starting at field 3, so field 22 is the
    // 20th token there.
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(22 - 3)?.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_start_time_os(_pid: i32) -> Option<u64> {
    None
}

/// Is the process a shell record describes still the one running? Its pid
/// must exist and, when both the record and the kernel know a start time,
/// the two must match (a mismatch = the pid now belongs to another process).
pub fn shell_alive(pid: i32, start: Option<u64>) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    match (start, process_start_time(pid)) {
        (Some(recorded), Some(now)) => recorded == now,
        _ => true,
    }
}

/// The SQLite databases the daemon holds open, for [`DaemonState::swap_db_file`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DbFile {
    /// `catalog.db`.
    Catalog,
    /// `history.db`.
    History,
}

/// Shared handle — clone freely; every clone holds the same Arc<Mutex<...>> + paths.
pub struct DaemonState {
    /// `inner` field.
    inner: Mutex<DaemonStateInner>,
    /// `catalog` field.
    catalog: Mutex<Connection>,
    /// `history_db` field.
    history_db: Mutex<Connection>,
    /// `fs_watcher` field.
    pub fs_watcher: Arc<super::fsnotify::FsWatcher>,
    /// `ask_inbox` field.
    pub ask_inbox: Arc<super::zask::AskInbox>,
    /// `jobs` field.
    pub jobs: Arc<super::jobs::Supervisor>,
    /// `canonical` field.
    pub canonical: Arc<super::canonical::CanonicalEngine>,
    /// Named cross-process locks (daemon.lock.* ops). In-memory only;
    /// daemon restart releases everything (intentional — locks held by
    /// processes that didn't get a release call were by definition
    /// crashed). PID-tied auto-release is per-acquire, not periodic.
    pub locks: super::lock::LockTable,
    /// In-process counters surfaced by `daemon.metrics` op + the
    /// `GET /metrics` Prometheus exposition. Bumped from
    /// `ops::dispatch` after each call and from `http::handler_op`
    /// after each HTTP response.
    pub metrics: super::metrics::Metrics,
    /// `paths` field.
    pub paths: CachePaths,
    /// `started_at` field.
    pub started_at: Instant,
    /// `start_wall` field.
    pub start_wall: chrono::DateTime<chrono::Utc>,
    /// `pid` field.
    pub pid: i32,
}

impl DaemonState {
    /// `new` — see implementation.
    pub fn new(paths: CachePaths) -> Result<Arc<Self>> {
        let catalog = catalog::open(&paths)?;
        let history_db = history::open(&paths)?;
        let fs_watcher = Arc::new(super::fsnotify::FsWatcher::new());
        let ask_inbox = super::zask::AskInbox::new();
        let jobs = super::jobs::Supervisor::new(paths.clone());
        let canonical = super::canonical::CanonicalEngine::new(paths.clone());
        // Eagerly load persisted canonical state from rkyv shard on disk —
        // missing shard = empty state (cold cache, first-run path).
        if let Err(e) = canonical.load_from_disk() {
            tracing::warn!(
                ?e,
                "canonical: load_from_disk failed (continuing with empty state)"
            );
        }
        let state = Arc::new(Self {
            inner: Mutex::new(DaemonStateInner::new()),
            locks: super::lock::new_table(),
            metrics: super::metrics::Metrics::new(),
            catalog: Mutex::new(catalog),
            history_db: Mutex::new(history_db),
            fs_watcher,
            ask_inbox,
            jobs: jobs.clone(),
            canonical,
            paths,
            started_at: Instant::now(),
            start_wall: chrono::Utc::now(),
            pid: std::process::id() as i32,
        });
        // Bind the supervisor to a weak ref of state so its async tasks can
        // publish events / persist to catalog without keeping state alive.
        jobs.bind_state(&state);
        let _ = jobs.ensure_schema(&state);
        Ok(state)
    }

    /// Run a closure with mutable access to the history connection.
    pub fn with_history<F, T, E>(&self, f: F) -> std::result::Result<T, E>
    where
        F: FnOnce(&Connection) -> std::result::Result<T, E>,
        E: From<rusqlite::Error>,
    {
        let conn = self.history_db.lock();
        f(&conn)
    }

    /// Total history row count (for `info` op).
    pub fn history_count(&self) -> rusqlite::Result<i64> {
        let conn = self.history_db.lock();
        history::count(&conn)
    }

    /// Read-only snapshot of catalog.db stats (table counts + file size).
    pub fn catalog_summary(&self) -> Result<CatalogSummary> {
        let conn = self.catalog.lock();
        catalog::summary(&conn, &self.paths.catalog_db)
    }

    /// Run PRAGMA integrity_check against catalog.db.
    pub fn catalog_integrity(&self) -> Result<bool> {
        let conn = self.catalog.lock();
        catalog::integrity_check(&conn)
    }

    /// Run a closure with mutable access to the catalog connection. The lock is held
    /// for the duration of the closure; keep it short.
    pub fn with_catalog<F, T, E>(&self, f: F) -> std::result::Result<T, E>
    where
        F: FnOnce(&Connection) -> std::result::Result<T, E>,
        E: From<rusqlite::Error>,
    {
        let conn = self.catalog.lock();
        f(&conn)
    }

    /// Fold both databases' WAL into their main files so a byte copy of
    /// `catalog.db` / `history.db` holds every committed row.
    pub fn checkpoint_dbs(&self) {
        for conn in [&self.catalog, &self.history_db] {
            if let Err(e) = conn.lock().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)") {
                tracing::warn!(?e, "wal checkpoint failed");
            }
        }
    }

    /// Replace a database file on disk (restore, import, clean) without
    /// leaving the daemon's connection on the old inode. Holding the
    /// connection lock: checkpoint, close, run `swap`, drop the old file's
    /// `-wal`/`-shm` sidecars (they would replay into the new file), then
    /// reopen — also when `swap` fails, so the daemon never runs without
    /// a connection.
    pub fn swap_db_file<T, E>(
        &self,
        db: DbFile,
        swap: impl FnOnce() -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<super::DaemonError>,
    {
        let (slot, path) = match db {
            DbFile::Catalog => (&self.catalog, &self.paths.catalog_db),
            DbFile::History => (&self.history_db, &self.paths.history_db),
        };
        let mut conn = slot.lock();
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        let placeholder = Connection::open_in_memory().map_err(super::DaemonError::from)?;
        drop(std::mem::replace(&mut *conn, placeholder));

        let swapped = swap();
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.clone().into_os_string();
            sidecar.push(suffix);
            let _ = std::fs::remove_file(sidecar);
        }
        *conn = match db {
            DbFile::Catalog => catalog::open(&self.paths),
            DbFile::History => history::open(&self.paths),
        }?;
        swapped
    }

    /// `uptime_ms` — see implementation.
    pub fn uptime_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Register a shell connection post-handshake: a Hello carrying
    /// `shell_pid` (`pid`) and, from current shells, `shell_start`
    /// (`start`). The connection joins that shell's record, minting one
    /// (with a fresh stable shell id) on the shell's first connection. A known
    /// pid with a different start time is a new shell on a recycled pid: the
    /// old record and everything it owned (tags, zask queue, subscriptions) is
    /// dropped and a new one minted. Returns (client_id, session_id); the
    /// stable id is `shell_id_of(client_id)`.
    pub fn register_shell_session(
        &self,
        pid: i32,
        start: Option<u64>,
        tty: Option<String>,
        cwd: Option<String>,
        argv0: Option<String>,
        outbound: mpsc::UnboundedSender<Frame>,
    ) -> (u64, String) {
        let mut g = self.inner.lock();
        let mut replaced = None;
        let known = g.shell_by_pid.get(&pid).copied();
        let reuse = match known.and_then(|id| g.shells.get_mut(&id)) {
            Some(r) => match (r.start, start) {
                (Some(old), Some(new)) if old != new => false,
                (None, Some(new)) => {
                    r.start = Some(new);
                    true
                }
                _ => true,
            },
            None => false,
        };
        let shell_id = match known {
            Some(id) if reuse => id,
            _ => {
                if let Some(old) = known {
                    Self::remove_shell_locked(&mut g, old);
                    replaced = Some(old);
                }
                let id = g.next_shell_id;
                g.next_shell_id += 1;
                g.shell_by_pid.insert(pid, id);
                g.shells.insert(
                    id,
                    ShellRecord {
                        shell_id: id,
                        pid,
                        start,
                        tty: None,
                        cwd: None,
                        argv0: None,
                        tags: BTreeSet::new(),
                        login_time: chrono::Utc::now(),
                        registered_at: Instant::now(),
                    },
                );
                id
            }
        };
        if let Some(r) = g.shells.get_mut(&shell_id) {
            if tty.is_some() {
                r.tty = tty.clone();
            }
            if cwd.is_some() {
                r.cwd = cwd.clone();
            }
            if argv0.is_some() {
                r.argv0 = argv0.clone();
            }
        }
        let out = Self::insert_session(&mut g, pid, tty, cwd, argv0, Some(shell_id), outbound);
        drop(g);
        if let Some(old) = replaced {
            self.ask_inbox.drop_for_shell(old);
            tracing::info!(pid, old_shell = old, new_shell = shell_id, "shell pid recycled; old record dropped");
        }
        out
    }

    /// `register_shell_session` without a start time (tests, old shells).
    pub fn register_session(
        &self,
        pid: i32,
        tty: Option<String>,
        cwd: Option<String>,
        argv0: Option<String>,
        outbound: mpsc::UnboundedSender<Frame>,
    ) -> (u64, String) {
        self.register_shell_session(pid, None, tty, cwd, argv0, outbound)
    }

    /// Register a session that is not a shell: HTTP synthetic sessions and
    /// socket clients whose Hello carries no `shell_pid` (the `zd` binary,
    /// the bench, anything that never called `set_shell_identity`). It never
    /// creates or joins a shell record, and everything it owns
    /// (subscriptions) is dropped when it unregisters.
    pub fn register_ephemeral_session(
        &self,
        pid: i32,
        tty: Option<String>,
        argv0: Option<String>,
        outbound: mpsc::UnboundedSender<Frame>,
    ) -> (u64, String) {
        let mut g = self.inner.lock();
        Self::insert_session(&mut g, pid, tty, None, argv0, None, outbound)
    }

    fn insert_session(
        g: &mut DaemonStateInner,
        pid: i32,
        tty: Option<String>,
        cwd: Option<String>,
        argv0: Option<String>,
        shell_id: Option<u64>,
        outbound: mpsc::UnboundedSender<Frame>,
    ) -> (u64, String) {
        let session_id = uuid_like();
        let client_id = g.next_client_id;
        g.next_client_id += 1;
        g.sessions.insert(
            client_id,
            Session {
                client_id,
                session_id: session_id.clone(),
                pid,
                tty,
                cwd,
                argv0,
                shell_id,
                connected_at: Instant::now(),
                login_time: chrono::Utc::now(),
                outbound,
                definitions_subscribed: false,
            },
        );
        (client_id, session_id)
    }

    /// Remove a shell record and its subscriptions (caller drops its zask
    /// queue once the lock is released).
    fn remove_shell_locked(g: &mut DaemonStateInner, shell_id: u64) {
        if let Some(r) = g.shells.remove(&shell_id) {
            if g.shell_by_pid.get(&r.pid) == Some(&shell_id) {
                g.shell_by_pid.remove(&r.pid);
            }
        }
        g.subscriptions.retain(|_, s| s.shell_id != Some(shell_id));
    }

    /// Drop a closed connection, and with it every subscription that
    /// delivers to it (it can never deliver again). The shell record it
    /// belonged to — tags, zask queue — stays until `reap_dead_shells` sees
    /// the pid gone.
    pub fn unregister_session(&self, client_id: u64) {
        let mut g = self.inner.lock();
        if g.sessions.remove(&client_id).is_some() {
            g.subscriptions.retain(|_, sub| sub.client_id != client_id);
        }
    }

    /// Stable shell id of a connection (`None` for HTTP sessions / unknown).
    pub fn shell_id_of(&self, client_id: u64) -> Option<u64> {
        self.inner.lock().shell_of(client_id)
    }

    /// Is `shell_id` a registered shell (live pid, connected or not)?
    pub fn shell_exists(&self, shell_id: u64) -> bool {
        self.inner.lock().shells.contains_key(&shell_id)
    }

    /// Every registered shell id.
    pub fn shell_ids(&self) -> Vec<u64> {
        self.inner.lock().shells.keys().copied().collect()
    }

    /// `zls` rows: every registered shell.
    pub fn snapshot_shells(&self) -> Vec<ShellSnapshot> {
        let mut rows: Vec<ShellSnapshot> = {
            let g = self.inner.lock();
            g.shells.values().map(|r| g.shell_snapshot(r)).collect()
        };
        for row in &mut rows {
            row.ask_pending = self.ask_inbox.pending_count(row.shell_id);
        }
        rows
    }

    /// Remove every shell record whose pid `is_alive` rejects, together with
    /// its subscriptions and zask queue. Returns the reaped shell ids. The
    /// ticker passes `pid_alive` (kill(pid, 0)).
    pub fn reap_dead_shells_with(&self, is_alive: impl Fn(i32, Option<u64>) -> bool) -> Vec<u64> {
        let dead: Vec<(u64, i32)> = {
            let g = self.inner.lock();
            g.shells
                .values()
                .filter(|r| !is_alive(r.pid, r.start))
                .map(|r| (r.shell_id, r.pid))
                .collect()
        };
        if dead.is_empty() {
            return Vec::new();
        }
        {
            let mut g = self.inner.lock();
            for (id, _) in &dead {
                Self::remove_shell_locked(&mut g, *id);
            }
        }
        for (id, _) in &dead {
            self.ask_inbox.drop_for_shell(*id);
        }
        dead.into_iter().map(|(id, _)| id).collect()
    }

    /// `reap_dead_shells_with(shell_alive)`: a record is dead when its pid is
    /// gone or now belongs to a process with a different start time.
    pub fn reap_dead_shells(&self) -> Vec<u64> {
        self.reap_dead_shells_with(shell_alive)
    }

    /// Add a subscription. Returns the assigned subscription id, or None if the
    /// pattern is malformed (caller surfaces the parse error). Delivery goes
    /// to the creating connection; ownership (list / pause / unsubscribe) is
    /// the caller's shell, so later connections of the same shell see it.
    pub fn add_subscription(
        &self,
        client_id: u64,
        pattern: &str,
    ) -> std::result::Result<u64, String> {
        let mut g = self.inner.lock();
        let id = g.next_subscription_id;
        g.next_subscription_id += 1;
        let mut sub = Subscription::parse(client_id, id, pattern)?;
        sub.shell_id = g.shell_of(client_id);
        g.subscriptions.insert(id, sub);
        Ok(id)
    }

    /// Remove the caller's subscriptions matching pattern (exact pattern
    /// match). Returns the count removed.
    pub fn remove_subscription_by_pattern(&self, client_id: u64, pattern: &str) -> usize {
        let mut g = self.inner.lock();
        let doomed: Vec<u64> = g
            .subscriptions
            .values()
            .filter(|s| s.pattern == pattern && g.owns_subscription(client_id, s))
            .map(|s| s.id)
            .collect();
        for id in &doomed {
            g.subscriptions.remove(id);
        }
        doomed.len()
    }

    /// Remove a subscription by id (only its owner may unsubscribe).
    pub fn remove_subscription_by_id(&self, client_id: u64, sub_id: u64) -> bool {
        let mut g = self.inner.lock();
        let owned = g
            .subscriptions
            .get(&sub_id)
            .is_some_and(|s| g.owns_subscription(client_id, s));
        if owned {
            g.subscriptions.remove(&sub_id);
        }
        owned
    }

    /// List the caller's subscriptions (its whole shell's, for a shell caller).
    pub fn list_subscriptions_for(&self, client_id: u64) -> Vec<Subscription> {
        let g = self.inner.lock();
        g.subscriptions
            .values()
            .filter(|s| g.owns_subscription(client_id, s))
            .cloned()
            .collect()
    }

    /// List every active subscription (for `zls --ui-pending` / debugging / `zsubscribe --list --all`).
    pub fn list_all_subscriptions(&self) -> Vec<Subscription> {
        let g = self.inner.lock();
        g.subscriptions.values().cloned().collect()
    }

    /// Publish an event: fan it out to every matching subscription. Returns the
    /// number of recipients the event was queued to. Paused subscriptions are
    /// silently skipped (the subscription stays registered, but no delivery).
    /// A subscription whose creating connection has closed gets no delivery.
    pub fn publish(&self, origin: &Scope, topic: &str, frame: Frame) -> usize {
        let g = self.inner.lock();
        let mut count = 0;
        for sub in g.subscriptions.values() {
            if sub.paused {
                continue;
            }
            if !origin.matches_scope(&sub.scope_pat) {
                continue;
            }
            if !super::pubsub::glob_match(&sub.topic_pat, topic) {
                continue;
            }
            if let Some(s) = g.sessions.get(&sub.client_id) {
                if s.outbound.send(frame.clone()).is_ok() {
                    count += 1;
                }
            }
        }
        count
    }

    /// Pause one of the caller's subscriptions. Returns true if it exists and
    /// the caller owns it.
    pub fn set_subscription_paused(&self, client_id: u64, sub_id: u64, paused: bool) -> bool {
        let mut g = self.inner.lock();
        let owned = g
            .subscriptions
            .get(&sub_id)
            .is_some_and(|s| g.owns_subscription(client_id, s));
        if owned {
            if let Some(s) = g.subscriptions.get_mut(&sub_id) {
                s.paused = paused;
            }
        }
        owned
    }

    /// Pause every subscription the caller owns. Returns the number of
    /// subscriptions affected.
    pub fn pause_all_subscriptions(&self, client_id: u64, paused: bool) -> usize {
        let mut g = self.inner.lock();
        let ids: Vec<u64> = g
            .subscriptions
            .values()
            .filter(|s| s.paused != paused && g.owns_subscription(client_id, s))
            .map(|s| s.id)
            .collect();
        for id in &ids {
            if let Some(s) = g.subscriptions.get_mut(id) {
                s.paused = paused;
            }
        }
        ids.len()
    }

    /// Build the event-origin Scope for a connection: its shell's stable id
    /// and tags. An HTTP session (not a shell) publishes as `shell:0`.
    pub fn origin_scope(&self, client_id: u64) -> Option<Scope> {
        let g = self.inner.lock();
        let s = g.sessions.get(&client_id)?;
        let (shell_id, tags) = match s.shell_id.and_then(|id| g.shells.get(&id)) {
            Some(r) => (r.shell_id, r.tags.clone()),
            None => (0, BTreeSet::new()),
        };
        Some(Scope {
            shell_id,
            tags,
            user: None,
            job_id: None,
        })
    }
    /// `snapshot_sessions` — see implementation.
    pub fn snapshot_sessions(&self) -> Vec<SessionSnapshot> {
        let g = self.inner.lock();
        g.sessions.values().map(Session::snapshot).collect()
    }
    /// `session_count` — see implementation.
    pub fn session_count(&self) -> usize {
        self.inner.lock().sessions.len()
    }

    /// Total active subscriptions across all sessions. Used by the
    /// `daemon.metrics` op + `/metrics` Prometheus exposition for
    /// the `daemon_active_subscriptions` gauge.
    pub fn subscription_count(&self) -> usize {
        self.inner.lock().subscriptions.len()
    }

    /// Persist canonical state to its rkyv shard AND immediately mirror it
    /// into the SQLite `canonical` view table. SQLite is the read-only
    /// inspection mirror per DAEMON.md "Canonical = source of truth (rkyv);
    /// SQLite is hydrated mirror." Every mutation of canonical state should
    /// flow through here so the mirror never goes stale.
    ///
    /// Returns the rkyv shard path on success. SQLite-hydrate failure logs
    /// a warning but does not abort — rkyv is authoritative; the mirror is
    /// best-effort.
    pub fn persist_canonical(&self, generation: u64) -> Result<std::path::PathBuf> {
        let path = self.canonical.persist(generation)?;
        if let Err(e) = self.canonical.hydrate_sqlite_view(self) {
            tracing::warn!(
                ?e,
                generation,
                "canonical: hydrate_sqlite_view failed (rkyv is authoritative)"
            );
        }
        Ok(path)
    }

    /// Read a runtime config knob. Returns the in-memory value if a client
    /// pushed one via `zcache config set`, else falls back to the env var
    /// (uppercased + `ZSHRS_` prefix), else `None`. Per DAEMON.md:905.
    pub fn config_get(&self, key: &str) -> Option<String> {
        let g = self.inner.lock();
        if let Some(v) = g.config.get(key) {
            return Some(v.clone());
        }
        drop(g);
        let env_key = format!("ZSHRS_{}", key.to_ascii_uppercase());
        std::env::var(&env_key).ok()
    }

    /// Set a runtime config knob. Returns the prior value, if any.
    pub fn config_set(&self, key: &str, value: String) -> Option<String> {
        let mut g = self.inner.lock();
        g.config.insert(key.to_string(), value)
    }

    /// Snapshot every config knob currently set in-memory (for `zcache
    /// config list` / view).
    pub fn config_snapshot(&self) -> std::collections::BTreeMap<String, String> {
        let g = self.inner.lock();
        g.config
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Update mutable per-session metadata (cwd, tty, argv0). Returns the
    /// post-update snapshot. Used by the `register` IPC op for mid-session
    /// updates (chpwd, tmux reattach, exec rebrand). None values leave the
    /// existing field untouched.
    pub fn update_session(
        &self,
        client_id: u64,
        cwd: Option<String>,
        tty: Option<String>,
        argv0: Option<String>,
    ) -> Option<SessionSnapshot> {
        let mut g = self.inner.lock();
        let shell_id = g.shell_of(client_id);
        if let Some(r) = shell_id.and_then(|id| g.shells.get_mut(&id)) {
            if cwd.is_some() {
                r.cwd = cwd.clone();
            }
            if tty.is_some() {
                r.tty = tty.clone();
            }
            if argv0.is_some() {
                r.argv0 = argv0.clone();
            }
        }
        let s = g.sessions.get_mut(&client_id)?;
        if let Some(c) = cwd {
            s.cwd = Some(c);
        }
        if let Some(t) = tty {
            s.tty = Some(t);
        }
        if let Some(a) = argv0 {
            s.argv0 = Some(a);
        }
        Some(s.snapshot())
    }
    /// Add tags to the caller's shell record. `None` when the caller is not
    /// a shell (HTTP session) or unknown.
    pub fn add_tags(&self, client_id: u64, tags: &[String]) -> Option<Vec<String>> {
        let mut g = self.inner.lock();
        let id = g.shell_of(client_id)?;
        let r = g.shells.get_mut(&id)?;
        for t in tags {
            r.tags.insert(t.clone());
        }
        Some(r.tags.iter().cloned().collect())
    }
    /// Remove tags from the caller's shell record (all of them when `tags`
    /// is empty).
    pub fn remove_tags(&self, client_id: u64, tags: &[String]) -> Option<Vec<String>> {
        let mut g = self.inner.lock();
        let id = g.shell_of(client_id)?;
        let r = g.shells.get_mut(&id)?;
        if tags.is_empty() {
            r.tags.clear();
        } else {
            for t in tags {
                r.tags.remove(t);
            }
        }
        Some(r.tags.iter().cloned().collect())
    }

    /// Register a pending zsend --wait response slot. Returns the receiver
    /// that the caller awaits; the sender is stored under delivery_id and
    /// fires when a `cmd_result` IPC matches.
    pub fn register_pending(&self, delivery_id: String) -> oneshot::Receiver<serde_json::Value> {
        let (tx, rx) = oneshot::channel();
        let mut g = self.inner.lock();
        g.pending_responses.insert(delivery_id, tx);
        rx
    }

    /// Resolve a pending zsend --wait response. Returns true if the slot
    /// existed (caller will be woken).
    pub fn resolve_pending(&self, delivery_id: &str, value: serde_json::Value) -> bool {
        let mut g = self.inner.lock();
        match g.pending_responses.remove(delivery_id) {
            Some(tx) => tx.send(value).is_ok(),
            None => false,
        }
    }
    /// Stable ids of every registered shell carrying `tag`.
    pub fn shells_with_tag(&self, tag: &str) -> Vec<u64> {
        let g = self.inner.lock();
        g.shells
            .values()
            .filter(|r| r.tags.contains(tag))
            .map(|r| r.shell_id)
            .collect()
    }

    /// Push a frame to every live connection of a shell. Returns how many
    /// connections it was queued to — 0 when the shell exists but holds no
    /// connection right now (no delivery; there is no long-lived client
    /// connection to hold it for).
    pub fn send_to_shell(&self, shell_id: u64, frame: Frame) -> usize {
        self.send_to_shell_except(shell_id, frame, None)
    }

    /// `send_to_shell`, skipping connection `except` (the caller, which gets
    /// its answer as the op response instead).
    pub fn send_to_shell_except(&self, shell_id: u64, frame: Frame, except: Option<u64>) -> usize {
        let g = self.inner.lock();
        g.sessions
            .values()
            .filter(|s| s.shell_id == Some(shell_id) && Some(s.client_id) != except)
            .filter(|s| s.outbound.send(frame.clone()).is_ok())
            .count()
    }

    /// Send a frame to a specific connection (responses). Returns false if the
    /// client is unknown or its outbound channel is closed.
    pub fn send_to(&self, client_id: u64, frame: Frame) -> bool {
        let g = self.inner.lock();
        match g.sessions.get(&client_id) {
            Some(s) => s.outbound.send(frame).is_ok(),
            None => false,
        }
    }

    /// Broadcast a frame to every live connection except those belonging to
    /// the excluded stable shell ids. Returns the number of connections the
    /// frame was queued to.
    pub fn broadcast(&self, frame: Frame, exclude_shells: &[u64]) -> usize {
        let g = self.inner.lock();
        let mut count = 0;
        for s in g.sessions.values() {
            if s.shell_id.is_some_and(|id| exclude_shells.contains(&id)) {
                continue;
            }
            if s.outbound.send(frame.clone()).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Targeted broadcast — only sessions that called `definitions_subscribe`
    /// receive this frame. Used by `op_recorder_ingest` so silent IPC
    /// clients (the common case) don't see every recorder bundle's
    /// summary frame on their socket.
    pub fn broadcast_to_definitions_subscribers(&self, frame: Frame) -> usize {
        let g = self.inner.lock();
        let mut count = 0;
        for s in g.sessions.values() {
            if s.definitions_subscribed && s.outbound.send(frame.clone()).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Toggle the per-session opt-in flag for DEFINITIONS events. Returns
    /// the prior value so the op handler can report whether anything
    /// actually changed.
    pub fn set_definitions_subscribed(&self, client_id: u64, subscribed: bool) -> Option<bool> {
        let mut g = self.inner.lock();
        let s = g.sessions.get_mut(&client_id)?;
        let prev = s.definitions_subscribed;
        s.definitions_subscribed = subscribed;
        Some(prev)
    }

    /// Push a frame to every live connection of every shell tagged `tag`.
    /// Returns the stable ids of the shells it reached.
    pub fn send_tag(&self, tag: &str, frame: Frame) -> Vec<u64> {
        self.shells_with_tag(tag)
            .into_iter()
            .filter(|&id| self.send_to_shell(id, frame.clone()) > 0)
            .collect()
    }
}

fn uuid_like() -> String {
    // Tiny opaque random id without a uuid crate dep — 8 hex bytes is enough for
    // session-uniqueness within a daemon process lifetime.
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 8] = rng.gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh() -> Arc<DaemonState> {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        // tempdir leaks here intentionally — test scope keeps it alive.
        std::mem::forget(tmp);
        DaemonState::new(paths).expect("DaemonState::new")
    }

    #[test]
    fn register_assigns_monotonic_ids() {
        let state = fresh();
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let (id1, _) = state.register_session(100, None, None, None, tx1);
        let (id2, _) = state.register_session(200, None, None, None, tx2);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(state.session_count(), 2);
    }

    #[test]
    fn unregister_removes_session() {
        let state = fresh();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (id, _) = state.register_session(100, None, None, None, tx);
        assert_eq!(state.session_count(), 1);
        state.unregister_session(id);
        assert_eq!(state.session_count(), 0);
    }

    #[test]
    fn add_then_remove_tags() {
        let state = fresh();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (id, _) = state.register_session(100, None, None, None, tx);
        let tags = state.add_tags(id, &["prod".into(), "dev".into()]).unwrap();
        assert_eq!(tags.len(), 2);

        let tags = state.remove_tags(id, &["prod".into()]).unwrap();
        assert_eq!(tags, vec!["dev".to_string()]);

        let cleared = state.remove_tags(id, &[]).unwrap();
        assert!(cleared.is_empty());
    }

    #[test]
    fn shells_with_tag_filters() {
        let state = fresh();
        let (tx1, _rx1) = mpsc::unbounded_channel();
        let (tx2, _rx2) = mpsc::unbounded_channel();
        let (tx3, _rx3) = mpsc::unbounded_channel();
        let (id1, _) = state.register_session(1, None, None, None, tx1);
        let (id2, _) = state.register_session(2, None, None, None, tx2);
        let (_, _) = state.register_session(3, None, None, None, tx3);

        state.add_tags(id1, &["prod".into()]).unwrap();
        state
            .add_tags(id2, &["prod".into(), "canary".into()])
            .unwrap();

        let prod = state.shells_with_tag("prod");
        assert_eq!(prod.len(), 2);
        assert!(prod.contains(&id1));
        assert!(prod.contains(&id2));

        let canary = state.shells_with_tag("canary");
        assert_eq!(canary, vec![id2]);
    }

    #[test]
    fn broadcast_excludes_self() {
        let state = fresh();
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let (id1, _) = state.register_session(1, None, None, None, tx1);
        let (id2, _) = state.register_session(2, None, None, None, tx2);

        let count = state.broadcast(
            Frame::event("notify", serde_json::json!({"m":"hi"})),
            &[id1],
        );
        assert_eq!(count, 1);
        assert!(rx1.try_recv().is_err());
        assert!(rx2.try_recv().is_ok());

        let _ = id2; // suppress unused warning if any
    }

    // ---- persistent shell registry (shell identity = pid, not connection) ----

    use super::super::ops::dispatch;
    use serde_json::json;

    /// One one-shot builtin call: connect as `pid`, run `op`, disconnect —
    /// the shape every z* builtin has from the command line.
    async fn one_shot(
        state: &Arc<DaemonState>,
        pid: i32,
        op: &str,
        args: serde_json::Value,
    ) -> serde_json::Value {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (cid, _) = state.register_session(pid, None, None, None, tx);
        let r = dispatch(state, cid, op, args).await;
        state.unregister_session(cid);
        r.unwrap_or_else(|e| panic!("{op} failed: {} ({})", e.msg, e.code))
    }

    /// A running `zsubscribe` stream: subscribe on a connection left open.
    /// Returns (connection id, subscription id).
    async fn open_stream(state: &Arc<DaemonState>, pid: i32, pattern: &str) -> (u64, u64) {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (cid, _) = state.register_session(pid, None, None, None, tx);
        let r = dispatch(state, cid, "subscribe", json!({ "pattern": pattern }))
            .await
            .unwrap();
        (cid, r["subscription_id"].as_u64().unwrap())
    }

    /// A pid that existed and is now gone: spawn `true` and reap it.
    fn dead_pid() -> i32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        child.wait().unwrap();
        assert!(!pid_alive(pid), "reaped child pid {pid} still alive");
        pid
    }

    fn shell_row(state: &Arc<DaemonState>, shell_id: u64) -> Option<ShellSnapshot> {
        state
            .snapshot_shells()
            .into_iter()
            .find(|s| s.shell_id == shell_id)
    }

    #[test]
    fn shell_registry_stable_id_across_connections() {
        let state = fresh();
        let (tx1, _r1) = mpsc::unbounded_channel();
        let (tx2, _r2) = mpsc::unbounded_channel();
        let (tx3, _r3) = mpsc::unbounded_channel();
        let (a, _) = state.register_session(4242, None, None, None, tx1);
        state.unregister_session(a);
        let (b, _) = state.register_session(4242, None, None, None, tx2);
        let (other, _) = state.register_session(4343, None, None, None, tx3);
        assert_ne!(a, b, "connection ids are per-connection");
        assert_eq!(state.shell_id_of(a), None, "closed connection is gone");
        let sb = state.shell_id_of(b).unwrap();
        let so = state.shell_id_of(other).unwrap();
        assert_ne!(sb, so);
        assert_eq!(state.snapshot_shells().len(), 2);
        // The pid's second connection rejoined the record minted by the first.
        let (tx4, _r4) = mpsc::unbounded_channel();
        let (c, _) = state.register_session(4242, None, None, None, tx4);
        assert_eq!(state.shell_id_of(c), Some(sb));
        assert_eq!(shell_row(&state, sb).unwrap().connections, 2);
    }

    #[tokio::test]
    async fn shell_registry_state_visible_to_later_connection() {
        let state = fresh();
        // Real live pids: `list_shells` reaps records whose pid is gone.
        let me = std::process::id() as i32;
        let mut other_proc = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let other = other_proc.id() as i32;

        // ztag, zask --target self: each its own connection. zsubscribe is a
        // stream: its connection stays open while the later calls run.
        one_shot(&state, me, "tag", json!({ "tags": ["prod"] })).await;
        one_shot(&state, other, "tag", json!({ "tags": ["dev"] })).await;
        let (stream, sub_id) = open_stream(&state, me, "*.chpwd").await;
        for _ in 0..2 {
            one_shot(
                &state,
                me,
                "ask_ask",
                json!({ "kind": "input", "target": { "self": true }, "payload": {} }),
            )
            .await;
        }
        one_shot(
            &state,
            other,
            "ask_ask",
            json!({ "kind": "input", "target": { "self": true }, "payload": {} }),
        )
        .await;

        // zls from a later connection sees the tag on the stable id.
        let shells = one_shot(&state, me, "list_shells", json!({ "tag": "prod" })).await;
        let rows = shells["shells"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["pid"].as_i64(), Some(me as i64));
        let my_shell = rows[0]["shell_id"].as_u64().unwrap();

        // zsubscribe --list from a later connection.
        let listed = one_shot(&state, me, "subscribe", json!({ "pattern": "--list" })).await;
        let ids: Vec<u64> = listed["subscriptions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_u64().unwrap())
            .collect();
        assert_eq!(ids, vec![sub_id]);
        let theirs = one_shot(&state, other, "subscribe", json!({ "pattern": "--list" })).await;
        assert!(theirs["subscriptions"].as_array().unwrap().is_empty());
        // Stream ends: its subscription can never deliver again, so it goes.
        state.unregister_session(stream);
        let listed = one_shot(&state, me, "subscribe", json!({ "pattern": "--list" })).await;
        assert!(listed["subscriptions"].as_array().unwrap().is_empty());

        // zask pending, then dismiss --all, from later connections.
        let p = one_shot(&state, me, "ask_pending", json!({})).await;
        assert_eq!(p["shell_id"].as_u64(), Some(my_shell));
        assert_eq!(p["pending_count"].as_u64(), Some(2));
        let d = one_shot(&state, me, "ask_dismiss", json!({ "all": true })).await;
        assert_eq!(d["dismissed"].as_u64(), Some(2));
        let p = one_shot(&state, me, "ask_pending", json!({})).await;
        assert_eq!(p["pending_count"].as_u64(), Some(0));

        // The other shell's tag and queue are untouched.
        let p = one_shot(&state, other, "ask_pending", json!({})).await;
        assert_eq!(p["pending_count"].as_u64(), Some(1));
        let dev = one_shot(&state, other, "list_shells", json!({ "tag": "dev" })).await;
        assert_eq!(dev["total"].as_u64(), Some(1));
        assert_eq!(dev["shells"][0]["pid"].as_i64(), Some(other as i64));
        let _ = other_proc.kill();
        let _ = other_proc.wait();
    }

    #[tokio::test]
    async fn shell_registry_reaps_dead_pid_with_tags_queue_and_subs() {
        let state = fresh();
        let gone = dead_pid();
        let alive = std::process::id() as i32;

        for pid in [gone, alive] {
            one_shot(&state, pid, "tag", json!({ "tags": ["t"] })).await;
            open_stream(&state, pid, "*.x").await;
            one_shot(
                &state,
                pid,
                "ask_ask",
                json!({ "kind": "menu", "target": { "self": true }, "payload": {} }),
            )
            .await;
        }
        assert_eq!(state.snapshot_shells().len(), 2);
        assert_eq!(state.subscription_count(), 2);
        let gone_shell = state
            .snapshot_shells()
            .into_iter()
            .find(|s| s.pid == gone)
            .unwrap()
            .shell_id;
        let alive_shell = state
            .snapshot_shells()
            .into_iter()
            .find(|s| s.pid == alive)
            .unwrap()
            .shell_id;
        assert_eq!(state.ask_inbox.pending_count(gone_shell), 1);

        assert_eq!(state.reap_dead_shells(), vec![gone_shell]);

        assert!(shell_row(&state, gone_shell).is_none());
        assert_eq!(state.ask_inbox.pending_count(gone_shell), 0);
        assert!(state.shells_with_tag("t") == vec![alive_shell]);
        assert_eq!(state.subscription_count(), 1);
        // The live shell keeps everything.
        assert_eq!(state.ask_inbox.pending_count(alive_shell), 1);
        assert_eq!(shell_row(&state, alive_shell).unwrap().tags, vec!["t"]);
        // A fresh shell reusing that pid number later gets a new id.
        let (tx, _rx) = mpsc::unbounded_channel();
        let (cid, _) = state.register_session(gone, None, None, None, tx);
        assert_ne!(state.shell_id_of(cid), Some(gone_shell));
    }

    #[tokio::test]
    async fn shell_registry_http_sessions_create_no_records() {
        let state = fresh();
        let (tx, _rx) = mpsc::unbounded_channel();
        let (cid, _) = state.register_ephemeral_session(state.pid, Some("http".into()), None, tx);
        assert_eq!(state.shell_id_of(cid), None);
        assert!(state.snapshot_shells().is_empty());
        // Shell-only ops refuse; pub/sub still works for the connection and
        // dies with it.
        assert!(dispatch(&state, cid, "tag", json!({ "tags": ["x"] })).await.is_err());
        assert!(dispatch(&state, cid, "ask_pending", json!({})).await.is_err());
        dispatch(&state, cid, "subscribe", json!({ "pattern": "*.x" }))
            .await
            .unwrap();
        assert_eq!(state.subscription_count(), 1);
        state.unregister_session(cid);
        assert_eq!(state.subscription_count(), 0);
        assert!(state.snapshot_shells().is_empty());
    }

    /// `zsubscribe --list` lists the shell's live streams: subscriptions on
    /// other open connections of the same shell. Closing a stream drops its
    /// subscription; another shell's stream is never listed or dropped.
    #[tokio::test]
    async fn shell_registry_subscription_dies_with_its_connection() {
        let state = fresh();
        let me = std::process::id() as i32;
        let mut other_proc = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let other = other_proc.id() as i32;

        let (s1, id1) = open_stream(&state, me, "*.chpwd").await;
        let (_s2, id2) = open_stream(&state, me, "tag:prod.commands").await;
        let (_s3, _) = open_stream(&state, other, "*.chpwd").await;

        let list = |v: serde_json::Value| -> Vec<u64> {
            v["subscriptions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].as_u64().unwrap())
                .collect()
        };
        let mine = one_shot(&state, me, "subscribe", json!({ "pattern": "--list" })).await;
        assert_eq!(list(mine), vec![id1, id2]);

        state.unregister_session(s1);
        let mine = one_shot(&state, me, "subscribe", json!({ "pattern": "--list" })).await;
        assert_eq!(list(mine), vec![id2]);
        let theirs = one_shot(&state, other, "subscribe", json!({ "pattern": "--list" })).await;
        assert_eq!(list(theirs).len(), 1);
        assert_eq!(state.subscription_count(), 2);

        let _ = other_proc.kill();
        let _ = other_proc.wait();
    }

    /// `list_shells` carries each shell's queued zask count (`zls --ask-pending`).
    #[tokio::test]
    async fn shell_registry_list_shells_reports_ask_pending() {
        let state = fresh();
        let me = std::process::id() as i32;
        let mut other_proc = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let other = other_proc.id() as i32;

        one_shot(&state, other, "tag", json!({ "tags": ["idle"] })).await;
        for _ in 0..3 {
            one_shot(
                &state,
                me,
                "ask_ask",
                json!({ "kind": "input", "target": { "self": true }, "payload": {} }),
            )
            .await;
        }
        let v = one_shot(&state, me, "list_shells", json!({})).await;
        let by_pid = |pid: i32| -> u64 {
            v["shells"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["pid"].as_i64() == Some(pid as i64))
                .unwrap()["ask_pending"]
                .as_u64()
                .unwrap()
        };
        assert_eq!(by_pid(me), 3);
        assert_eq!(by_pid(other), 0);

        let _ = other_proc.kill();
        let _ = other_proc.wait();
    }

    #[test]
    fn shell_registry_process_start_time_is_stable_and_per_process() {
        let me = std::process::id() as i32;
        let a = process_start_time(me).expect("start time readable on this platform");
        assert_eq!(process_start_time(me), Some(a));
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let c = process_start_time(child.id() as i32).unwrap();
        assert_ne!(c, a, "a later process must report a different start");
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(process_start_time(dead_pid()), None);
    }

    /// The reaper reads the live pid's start time: a record whose recorded
    /// start does not match the process now holding that pid is dead, even
    /// though the pid exists and no new shell has connected yet.
    #[test]
    fn shell_registry_reaper_detects_reused_pid_by_start_time() {
        let state = fresh();
        let mut reused = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let mut same = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let reused_pid = reused.id() as i32;
        let same_pid = same.id() as i32;
        let real = process_start_time(reused_pid).unwrap();

        // A record left by the "previous" holder of reused_pid.
        let (tx1, _r1) = mpsc::unbounded_channel();
        let (c1, _) =
            state.register_shell_session(reused_pid, Some(real + 1), None, None, None, tx1);
        let stale = state.shell_id_of(c1).unwrap();
        state.add_tags(c1, &["stale".into()]).unwrap();
        // A record that matches the live process.
        let (tx2, _r2) = mpsc::unbounded_channel();
        let (c2, _) = state.register_shell_session(
            same_pid,
            process_start_time(same_pid),
            None,
            None,
            None,
            tx2,
        );
        let live = state.shell_id_of(c2).unwrap();
        assert!(pid_alive(reused_pid));

        assert_eq!(state.reap_dead_shells(), vec![stale]);
        assert!(shell_row(&state, stale).is_none());
        assert!(state.shells_with_tag("stale").is_empty());
        assert!(shell_row(&state, live).is_some());

        for c in [&mut reused, &mut same] {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// A shell dismissing requests it asked itself: the cancel goes to its
    /// other live connections (a script waiting on the answer), never to the
    /// connection that issued the dismiss.
    #[tokio::test]
    async fn shell_registry_self_dismiss_skips_caller_connection() {
        let state = fresh();
        let me = std::process::id() as i32;
        let (tx_call, mut rx_call) = mpsc::unbounded_channel();
        let (tx_wait, mut rx_wait) = mpsc::unbounded_channel();
        let (caller, _) = state.register_session(me, None, None, None, tx_call);
        let (_waiter, _) = state.register_session(me, None, None, None, tx_wait);
        for _ in 0..2 {
            dispatch(
                &state,
                caller,
                "ask_ask",
                json!({ "kind": "input", "target": { "self": true }, "payload": {} }),
            )
            .await
            .unwrap();
        }
        let cancels = |rx: &mut mpsc::UnboundedReceiver<Frame>| -> usize {
            let mut n = 0;
            while let Ok(f) = rx.try_recv() {
                if let Frame::Event { event, payload } = f {
                    if event == "ask:response" && payload["cancelled"] == json!(true) {
                        n += 1;
                    }
                }
            }
            n
        };
        let d = dispatch(&state, caller, "ask_dismiss", json!({ "all": true }))
            .await
            .unwrap();
        assert_eq!(d["dismissed"].as_u64(), Some(2));
        assert_eq!(d["originators_notified"].as_u64(), Some(2));
        assert_eq!(cancels(&mut rx_call), 0);
        assert_eq!(cancels(&mut rx_wait), 2);
    }

    fn stat_names(path: &std::path::Path) -> Vec<String> {
        let conn = catalog::open_at(path).unwrap();
        let mut stmt = conn.prepare("SELECT fq_name FROM entry_stats ORDER BY fq_name").unwrap();
        let names = stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>();
        names.unwrap()
    }

    fn add_stat(conn: &Connection, name: &str) -> rusqlite::Result<()> {
        conn.execute("INSERT INTO entry_stats (fq_name, call_count) VALUES (?, 1)", [name])
            .map(drop)
    }

    // A restore renames catalog.db away and copies the backup in. The
    // daemon must write to the restored file afterwards, not to the
    // renamed inode its old connection still holds.
    #[test]
    fn swap_db_file_moves_the_connection_to_the_new_file() {
        let state = fresh();
        let catalog_db = state.paths.catalog_db.clone();
        state.with_catalog(|c| add_stat(c, "before_swap")).unwrap();

        let incoming = catalog_db.with_file_name("incoming.db");
        add_stat(&catalog::open_at(&incoming).unwrap(), "from_backup").unwrap();
        let displaced = catalog_db.with_file_name("displaced.db");
        state
            .swap_db_file(DbFile::Catalog, || -> Result<()> {
                std::fs::rename(&catalog_db, &displaced)?;
                std::fs::copy(&incoming, &catalog_db)?;
                Ok(())
            })
            .unwrap();
        state.with_catalog(|c| add_stat(c, "after_swap")).unwrap();

        assert_eq!(stat_names(&catalog_db), ["after_swap", "from_backup"]);
        assert_eq!(stat_names(&displaced), ["before_swap"]);
    }

    // A failed swap still leaves the daemon a working connection.
    #[test]
    fn swap_db_file_reopens_after_a_failed_swap() {
        let state = fresh();
        let failed = state.swap_db_file(DbFile::History, || -> Result<()> {
            Err(super::super::DaemonError::other("copy failed"))
        });
        assert!(failed.is_err());
        assert!(state.history_count().is_ok());
    }

    // `export_all` copies the database files byte for byte; rows still in
    // the WAL must be in the main file by then.
    #[test]
    fn checkpoint_dbs_leaves_every_row_in_the_main_file() {
        let state = fresh();
        state
            .with_history(|c| {
                c.execute("INSERT INTO history (line, ts_ns) VALUES ('ls', 1)", [])
            })
            .unwrap();
        state.checkpoint_dbs();

        let copy = state.paths.history_db.with_file_name("copy.db");
        std::fs::copy(&state.paths.history_db, &copy).unwrap();
        let conn = Connection::open(&copy).unwrap();
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 1);
    }
}
