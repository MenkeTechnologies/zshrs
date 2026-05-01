// fsnotify watcher — single instance per daemon, debounced, routes to rebuild jobs.
//
// Per docs/DAEMON.md "Steady state: fsnotify only" + "ANY periodic re-walk ... REJECT":
//   - One watcher across the machine (notify::RecommendedWatcher).
//   - Events are debounced (notify-debouncer-mini) so a burst of writes from
//     `git checkout` etc. produces ONE rebuild per file, not one per inotify event.
//   - On qualifying event: identify the affected shard, enqueue a rebuild job,
//     bump generation, atomic-rename, push shard_updated event to subscribers.
//
// V1 implementation:
//   - Watcher started in a dedicated tokio task.
//   - Maintains a registry of watched paths → (shard_slug, source_root).
//   - On event: log it, emit a `shard_updated` event to subscribers tracking that shard,
//     and (for completeness — not yet wired into a rebuild pipeline) record the
//     event in a per-watcher counter accessible via `info`.
//   - Real rebuild dispatch arrives with the walk-lifecycle evaluator (task 24).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, Debouncer};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::ipc::Frame;
use super::state::DaemonState;
use super::Result;

/// Outcome of one debounced fsnotify cycle — used by tests + observability.
#[derive(Clone, Debug, serde::Serialize, Default)]
pub struct WatcherStats {
    pub events_received: u64,
    pub events_routed: u64,
    pub watched_path_count: usize,
}

/// What action a watched path triggers when it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchKind {
    /// Default: just emit `shard_updated` and let subscribers react.
    Generic,
    /// `.zshrc` (or transitively-sourced file): re-run analyze_with_sources
    /// and update the canonical engine on every change.
    ZshrcSource,
    /// `$FPATH` directory: re-walk on changes so command_hash /
    /// autoload_table catch up. (v1: just emits shard_updated; rebuild
    /// dispatch is task #24-class work.)
    FpathDir,
}

/// One watched-path registration: maps a filesystem path to the shard it belongs to.
#[derive(Clone, Debug)]
pub struct WatchedPath {
    pub path: PathBuf,
    pub shard_slug: String,
    pub source_root: String,
    pub kind: WatchKind,
}

/// Top-level fsnotify state owned by DaemonState. Holds the debouncer + the
/// registry; the actual events are dispatched in a tokio task spawned by `start`.
pub struct FsWatcher {
    inner: Mutex<FsWatcherInner>,
}

struct FsWatcherInner {
    /// Map: absolute path → registration. We use a flat HashMap (small N — fpath dirs +
    /// .zshrc + .tokens.sh + plugin trees ≈ low hundreds).
    registered: HashMap<PathBuf, WatchedPath>,
    stats: WatcherStats,
    debouncer: Option<Debouncer<RecommendedWatcher>>,
}

impl FsWatcher {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(FsWatcherInner {
                registered: HashMap::new(),
                stats: WatcherStats::default(),
                debouncer: None,
            }),
        }
    }

    /// Spawn the dedicated fsnotify task. Must be called from inside a tokio runtime.
    pub fn start(self: &Arc<Self>, state: Arc<DaemonState>) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel::<DebouncedResult>();
        let debouncer = new_debouncer(
            Duration::from_millis(150),
            move |res: notify_debouncer_mini::DebounceEventResult| {
                let _ = tx.send(res);
            },
        )
        .map_err(|e| super::DaemonError::other(format!("fsnotify init: {e}")))?;

        {
            let mut g = self.inner.lock();
            g.debouncer = Some(debouncer);
        }

        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(res) = rx.recv().await {
                me.handle_debounced(res, &state);
            }
        });

        tracing::info!("fsnotify watcher started");
        Ok(())
    }

    /// Register a path to watch. Idempotent. Canonicalizes the path so
    /// macOS-FSEvents-style /private prefix or other symlink resolution doesn't
    /// cause subsequent events to fail to match the registration.
    pub fn watch_path(&self, mut wp: WatchedPath, recursive: bool) -> Result<()> {
        // Canonicalize for consistent prefix-matching against fsnotify's reported paths.
        let canonical = std::fs::canonicalize(&wp.path).unwrap_or(wp.path.clone());
        wp.path = canonical;

        let mut g = self.inner.lock();
        let key = wp.path.clone();
        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        if let Some(deb) = g.debouncer.as_mut() {
            // notify::Watcher::watch is exposed via the debouncer's `watcher()` accessor.
            deb.watcher()
                .watch(&wp.path, mode)
                .map_err(|e| super::DaemonError::other(format!("fsnotify watch: {e}")))?;
        }
        g.registered.insert(key, wp);
        g.stats.watched_path_count = g.registered.len();
        Ok(())
    }

    /// Stop watching a path. No-op if not registered. Canonicalizes the path so the
    /// caller can pass either the original or the resolved form.
    pub fn unwatch_path(&self, path: &Path) -> Result<()> {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let mut g = self.inner.lock();
        if let Some(deb) = g.debouncer.as_mut() {
            let _ = deb.watcher().unwatch(&canonical);
        }
        g.registered.remove(&canonical);
        g.registered.remove(path); // also try the as-passed key, just in case
        g.stats.watched_path_count = g.registered.len();
        Ok(())
    }

    pub fn stats(&self) -> WatcherStats {
        let g = self.inner.lock();
        g.stats.clone()
    }

    pub fn registered_paths(&self) -> Vec<WatchedPath> {
        let g = self.inner.lock();
        g.registered.values().cloned().collect()
    }

    fn handle_debounced(&self, res: DebouncedResult, state: &Arc<DaemonState>) {
        let events = match res {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!(?e, "fsnotify error");
                return;
            }
        };

        for ev in events {
            self.handle_one_event(&ev, state);
        }
    }

    fn handle_one_event(&self, ev: &DebouncedEvent, state: &Arc<DaemonState>) {
        let mut g = self.inner.lock();
        g.stats.events_received += 1;

        // Find the longest-prefix watched-path for the changed file. Slow O(N) walk over
        // the registry but N is small for typical setups.
        let path = &ev.path;
        let best_match: Option<WatchedPath> = g
            .registered
            .values()
            .filter(|wp| path.starts_with(&wp.path))
            .max_by_key(|wp| wp.path.as_os_str().len())
            .cloned();

        let Some(wp) = best_match else {
            tracing::debug!(path = %path.display(), "fsnotify event with no matching registration");
            return;
        };

        g.stats.events_routed += 1;
        let kind = wp.kind;
        drop(g);

        tracing::info!(
            path = %path.display(),
            shard = %wp.shard_slug,
            source_root = %wp.source_root,
            kind = ?kind,
            "fsnotify event routed"
        );

        // Re-analyze on .zshrc-style changes: re-run analyze_with_sources
        // and reseed the canonical engine. This is the steady-state piece of
        // the walk lifecycle — file modified → daemon re-parses just that
        // file → updates canonical → broadcasts canonical_changed.
        if kind == WatchKind::ZshrcSource {
            self.reanalyze_zshrc(&wp.source_root, state);
        }

        // Emit `shard_updated` event so any subscriber tracking this shard knows
        // the daemon picked up the change.
        let payload = serde_json::json!({
            "shard": wp.shard_slug,
            "source_root": wp.source_root,
            "trigger_path": path.display().to_string(),
            "generation": null,
        });
        let frame = Frame::event("shard_updated", payload);
        let _ = state.broadcast(frame, &[]);
    }

    fn reanalyze_zshrc(&self, source_root: &str, state: &Arc<DaemonState>) {
        let path = std::path::Path::new(source_root);
        if !path.exists() {
            return;
        }
        let analysis = match super::zshrc_analysis::analyze_with_sources(path) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(?e, source = %source_root, "fsnotify reanalyze failed");
                return;
            }
        };
        let canon = &state.canonical;
        // Replace per-subsystem rows so removed declarations actually disappear.
        canon.replace_subsystem(
            "alias",
            analysis.aliases.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "galias",
            analysis.global_aliases.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "salias",
            analysis.suffix_aliases.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "named_dir",
            analysis.named_dirs.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "compdef",
            analysis.compdef.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "bindkey",
            analysis.bindkeys.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "env",
            analysis.env_exports.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "params",
            analysis.params.iter().map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "function",
            analysis
                .functions
                .iter()
                .map(|(k, v)| (k.clone(), json_string_local(v))),
            None,
        );
        canon.replace_subsystem(
            "path",
            analysis
                .path_additions
                .iter()
                .enumerate()
                .map(|(i, d)| (i.to_string(), json_string_local(d))),
            None,
        );
        canon.replace_subsystem(
            "fpath",
            analysis
                .fpath_additions
                .iter()
                .enumerate()
                .map(|(i, d)| (i.to_string(), json_string_local(d))),
            None,
        );
        canon.replace_subsystem(
            "manpath",
            analysis
                .manpath_additions
                .iter()
                .enumerate()
                .map(|(i, d)| (i.to_string(), json_string_local(d))),
            None,
        );
        // setopt is a union of setopts + unsetopts (latter wins for matching keys).
        let mut setopt_iter: Vec<(String, String)> = Vec::new();
        for opt in &analysis.setopts {
            setopt_iter.push((opt.clone(), "\"on\"".to_string()));
        }
        for opt in &analysis.unsetopts {
            setopt_iter.push((opt.clone(), "\"off\"".to_string()));
        }
        canon.replace_subsystem("setopt", setopt_iter, None);

        let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
        if let Err(e) = canon.persist(generation) {
            tracing::warn!(?e, "fsnotify reanalyze: persist failed");
        }
        let event = serde_json::json!({
            "subsystem": "*",
            "row_count": canon.total_rows(),
            "set_at_ns": chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            "set_by_shell": 0,
            "trigger": "fsnotify_reanalyze",
            "source": source_root,
        });
        state.broadcast(Frame::event("canonical_changed", event), &[]);
        tracing::info!(
            source = %source_root,
            captured = canon.total_rows(),
            "fsnotify reanalyze complete"
        );
    }
}

fn json_string_local(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

type DebouncedResult = notify_debouncer_mini::DebounceEventResult;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[tokio::test(flavor = "multi_thread")]
    async fn watch_register_unregister() {
        let tmp = TempDir::new().unwrap();
        let watch_dir = tmp.path().join("zsh_funcs");
        std::fs::create_dir_all(&watch_dir).unwrap();

        let paths = super::super::paths::CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let state = super::super::state::DaemonState::new(paths).unwrap();

        let watcher = Arc::new(FsWatcher::new());
        watcher.start(Arc::clone(&state)).unwrap();

        let wp = WatchedPath {
            path: watch_dir.clone(),
            shard_slug: "test".to_string(),
            source_root: watch_dir.display().to_string(),
            kind: WatchKind::Generic,
        };
        watcher.watch_path(wp.clone(), false).unwrap();

        let stats = watcher.stats();
        assert_eq!(stats.watched_path_count, 1);

        let registered = watcher.registered_paths();
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].shard_slug, "test");

        watcher.unwatch_path(&watch_dir).unwrap();
        assert_eq!(watcher.stats().watched_path_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fsnotify_routes_event_to_shard() {
        let tmp = TempDir::new().unwrap();
        let watch_dir = tmp.path().join("zsh_funcs");
        std::fs::create_dir_all(&watch_dir).unwrap();

        let paths = super::super::paths::CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let state = super::super::state::DaemonState::new(paths).unwrap();

        let watcher = Arc::new(FsWatcher::new());
        watcher.start(Arc::clone(&state)).unwrap();

        let wp = WatchedPath {
            path: watch_dir.clone(),
            shard_slug: "test".to_string(),
            source_root: watch_dir.display().to_string(),
            kind: WatchKind::Generic,
        };
        watcher.watch_path(wp, false).unwrap();

        // Trigger an event by writing a file inside the watched dir.
        std::fs::write(watch_dir.join("_git"), b"# completion file").unwrap();

        // Wait for debouncer to fire.
        tokio::time::sleep(Duration::from_millis(400)).await;

        let stats = watcher.stats();
        assert!(
            stats.events_received >= 1,
            "events_received = {}",
            stats.events_received
        );
        assert!(
            stats.events_routed >= 1,
            "events_routed = {}",
            stats.events_routed
        );
    }
}
