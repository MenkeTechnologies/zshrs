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
    pub client_id: u64,
    pub session_id: String,
    pub pid: i32,
    pub tty: Option<String>,
    pub cwd: Option<String>,
    pub argv0: Option<String>,
    pub tags: BTreeSet<String>,
    pub connected_at: Instant,
    pub login_time: chrono::DateTime<chrono::Utc>,
    /// Outbound channel — daemon writes async events / responses here, the connection
    /// handler task drains and sends them on the wire.
    pub outbound: mpsc::UnboundedSender<Frame>,
}

impl Session {
    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            client_id: self.client_id,
            session_id: self.session_id.clone(),
            pid: self.pid,
            tty: self.tty.clone(),
            cwd: self.cwd.clone(),
            argv0: self.argv0.clone(),
            tags: self.tags.iter().cloned().collect(),
            login_time: self.login_time.to_rfc3339(),
            uptime_secs: self.connected_at.elapsed().as_secs(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SessionSnapshot {
    pub client_id: u64,
    pub session_id: String,
    pub pid: i32,
    pub tty: Option<String>,
    pub cwd: Option<String>,
    pub argv0: Option<String>,
    pub tags: Vec<String>,
    pub login_time: String,
    pub uptime_secs: u64,
}

/// Inner mutable state behind a single mutex.
pub struct DaemonStateInner {
    pub sessions: BTreeMap<u64, Session>,
    pub next_client_id: u64,
    pub subscriptions: BTreeMap<u64, Subscription>,
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
            subscriptions: BTreeMap::new(),
            next_subscription_id: 1,
            pending_responses: HashMap::new(),
            config: HashMap::new(),
        }
    }
}

/// Shared handle — clone freely; every clone holds the same Arc<Mutex<...>> + paths.
pub struct DaemonState {
    inner: Mutex<DaemonStateInner>,
    catalog: Mutex<Connection>,
    history_db: Mutex<Connection>,
    pub fs_watcher: Arc<super::fsnotify::FsWatcher>,
    pub ask_inbox: Arc<super::zask::AskInbox>,
    pub jobs: Arc<super::jobs::Supervisor>,
    pub canonical: Arc<super::canonical::CanonicalEngine>,
    pub paths: CachePaths,
    pub started_at: Instant,
    pub start_wall: chrono::DateTime<chrono::Utc>,
    pub pid: i32,
}

impl DaemonState {
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

    pub fn uptime_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    /// Register a new session post-handshake. Returns (client_id, session_id) assigned.
    pub fn register_session(
        &self,
        pid: i32,
        tty: Option<String>,
        cwd: Option<String>,
        argv0: Option<String>,
        outbound: mpsc::UnboundedSender<Frame>,
    ) -> (u64, String) {
        let session_id = uuid_like();
        let mut g = self.inner.lock();
        let client_id = g.next_client_id;
        g.next_client_id += 1;

        let session = Session {
            client_id,
            session_id: session_id.clone(),
            pid,
            tty,
            cwd,
            argv0,
            tags: BTreeSet::new(),
            connected_at: Instant::now(),
            login_time: chrono::Utc::now(),
            outbound,
        };
        g.sessions.insert(client_id, session);

        (client_id, session_id)
    }

    pub fn unregister_session(&self, client_id: u64) {
        {
            let mut g = self.inner.lock();
            g.sessions.remove(&client_id);
            // Drop every subscription belonging to this client.
            g.subscriptions.retain(|_, s| s.client_id != client_id);
        }
        // Also drop any pending zask requests targeting this disconnected shell.
        self.ask_inbox.drop_for_shell(client_id);
    }

    /// Add a subscription. Returns the assigned subscription id, or None if the
    /// pattern is malformed (caller surfaces the parse error).
    pub fn add_subscription(
        &self,
        client_id: u64,
        pattern: &str,
    ) -> std::result::Result<u64, String> {
        let mut g = self.inner.lock();
        let id = g.next_subscription_id;
        g.next_subscription_id += 1;
        let sub = Subscription::parse(client_id, id, pattern)?;
        g.subscriptions.insert(id, sub);
        Ok(id)
    }

    /// Remove subscriptions matching pattern (exact pattern match). Returns the count
    /// removed.
    pub fn remove_subscription_by_pattern(&self, client_id: u64, pattern: &str) -> usize {
        let mut g = self.inner.lock();
        let before = g.subscriptions.len();
        g.subscriptions
            .retain(|_, s| !(s.client_id == client_id && s.pattern == pattern));
        before - g.subscriptions.len()
    }

    /// Remove a subscription by id (only the owning client may unsubscribe).
    pub fn remove_subscription_by_id(&self, client_id: u64, sub_id: u64) -> bool {
        let mut g = self.inner.lock();
        match g.subscriptions.get(&sub_id) {
            Some(s) if s.client_id == client_id => {
                g.subscriptions.remove(&sub_id);
                true
            }
            _ => false,
        }
    }

    /// List a client's active subscriptions.
    pub fn list_subscriptions_for(&self, client_id: u64) -> Vec<Subscription> {
        let g = self.inner.lock();
        g.subscriptions
            .values()
            .filter(|s| s.client_id == client_id)
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

    /// Pause a subscription owned by the given client. Returns true if the
    /// subscription existed and was found owned by this client (or already paused).
    pub fn set_subscription_paused(&self, client_id: u64, sub_id: u64, paused: bool) -> bool {
        let mut g = self.inner.lock();
        match g.subscriptions.get_mut(&sub_id) {
            Some(s) if s.client_id == client_id => {
                s.paused = paused;
                true
            }
            _ => false,
        }
    }

    /// Pause every subscription owned by the given client. Returns the number
    /// of subscriptions affected.
    pub fn pause_all_subscriptions(&self, client_id: u64, paused: bool) -> usize {
        let mut g = self.inner.lock();
        let mut n = 0;
        for s in g.subscriptions.values_mut() {
            if s.client_id == client_id && s.paused != paused {
                s.paused = paused;
                n += 1;
            }
        }
        n
    }

    /// Build a Scope from a session id (for use as event origin).
    pub fn origin_scope(&self, client_id: u64) -> Option<Scope> {
        let g = self.inner.lock();
        let s = g.sessions.get(&client_id)?;
        Some(Scope {
            shell_id: s.client_id,
            tags: s.tags.clone(),
            user: None,
            job_id: None,
        })
    }

    pub fn snapshot_sessions(&self) -> Vec<SessionSnapshot> {
        let g = self.inner.lock();
        g.sessions.values().map(Session::snapshot).collect()
    }

    pub fn session_count(&self) -> usize {
        self.inner.lock().sessions.len()
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

    pub fn add_tags(&self, client_id: u64, tags: &[String]) -> Option<Vec<String>> {
        let mut g = self.inner.lock();
        let s = g.sessions.get_mut(&client_id)?;
        for t in tags {
            s.tags.insert(t.clone());
        }
        Some(s.tags.iter().cloned().collect())
    }

    pub fn remove_tags(&self, client_id: u64, tags: &[String]) -> Option<Vec<String>> {
        let mut g = self.inner.lock();
        let s = g.sessions.get_mut(&client_id)?;
        if tags.is_empty() {
            s.tags.clear();
        } else {
            for t in tags {
                s.tags.remove(t);
            }
        }
        Some(s.tags.iter().cloned().collect())
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

    pub fn shells_with_tag(&self, tag: &str) -> Vec<u64> {
        let g = self.inner.lock();
        g.sessions
            .values()
            .filter(|s| s.tags.contains(tag))
            .map(|s| s.client_id)
            .collect()
    }

    /// Send a frame to a specific client. Returns false if the client is unknown or
    /// its outbound channel is closed.
    pub fn send_to(&self, client_id: u64, frame: Frame) -> bool {
        let g = self.inner.lock();
        match g.sessions.get(&client_id) {
            Some(s) => s.outbound.send(frame).is_ok(),
            None => false,
        }
    }

    /// Broadcast a frame to every connected session except optionally excluded ones.
    /// Returns the number of clients the frame was queued to.
    pub fn broadcast(&self, frame: Frame, exclude: &[u64]) -> usize {
        let g = self.inner.lock();
        let mut count = 0;
        for (id, s) in g.sessions.iter() {
            if exclude.contains(id) {
                continue;
            }
            if s.outbound.send(frame.clone()).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Broadcast to every session matching a tag. Returns the recipient ids.
    pub fn send_tag(&self, tag: &str, frame: Frame) -> Vec<u64> {
        let g = self.inner.lock();
        let mut out = Vec::new();
        for s in g.sessions.values() {
            if s.tags.contains(tag) && s.outbound.send(frame.clone()).is_ok() {
                out.push(s.client_id);
            }
        }
        out
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
}
