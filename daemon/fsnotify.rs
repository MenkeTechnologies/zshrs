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

use notify::{RecommendedWatcher, RecursiveMode};
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

    /// JSON snapshot used by `zcache doctor` — includes a `running` flag
    /// derived from whether the debouncer was successfully installed.
    pub fn stats_json(&self) -> serde_json::Value {
        let g = self.inner.lock();
        serde_json::json!({
            "running": g.debouncer.is_some(),
            "events_received": g.stats.events_received,
            "events_routed": g.stats.events_routed,
            "watched_path_count": g.stats.watched_path_count,
        })
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

        // The static `reanalyze_zshrc` (ZshrcSource) and `rewalk_for_fpath`
        // (FpathDir) branches were removed with the AST-walk pipeline.
        // The daemon no longer re-derives state from sources on file change
        // — `zshrs-recorder` does that on demand. We still broadcast
        // `shard_updated` so subscribers know to re-mmap their cache;
        // re-indexing of state mutations is the user's recorder run.
        let _ = kind;

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
