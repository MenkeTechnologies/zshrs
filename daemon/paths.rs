// ~/.zshrs/* path resolution + permission enforcement.
//
// Layout (matches docs/DAEMON.md):
//   ~/.zshrs/
//   ├── index.rkyv
//   ├── images/                  ← 0700 dir, 0600 files
//   ├── catalog.db               ← 0600 (daemon-only writer)
//   ├── history.db               ← 0600
//   ├── zshrs.log                ← 0600 — shell (thin client) tracing
//   ├── zshrs-daemon.log         ← 0600 — daemon tracing
//   ├── zshrs-recorder.log       ← 0600 — recorder tracing
//   ├── daemon.sock              ← 0600 Unix domain socket
//   └── daemon.pid               ← 0600 singleton flock
//
// Cache directory is 0700 (user-only). Files inside are 0600. Verified by
// `zcache verify` on every integrity scan; drift triggers WARN in
// zshrs-daemon.log. All three log files are size-rotated together by the
// daemon's ticker via `is_zshrs_log_file` — adding a fourth log file
// (e.g. for a future `zshrs-test-runner` binary) only needs editing
// that one helper.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{DaemonError, Result};

/// Default `zshrs-daemon.toml` written on first daemon startup. HTTP
/// is enabled on loopback so `zd health` / `zd ops` / `zd op …` work
/// without the user having to edit anything. Loopback bind requires
/// no auth (per `http::serve_http`); switching `listen` to a non-
/// loopback address WILL refuse to start until `[http.tokens]` is
/// populated.
const DEFAULT_DAEMON_TOML: &str = "\
# zshrs-daemon configuration. Lives at $ZSHRS_HOME/zshrs-daemon.toml
# (~/.zshrs/zshrs-daemon.toml by default). Auto-seeded on first
# start; edit freely — the daemon never overwrites it.

[log]
# Tracing filter directive. Same syntax as $ZSHRS_LOG (which
# always wins when set). Accepts simple levels (info / debug /
# trace / warn / error) or per-module overrides
# (info,fsnotify=trace,ipc=debug). `zlog level <directive>`
# overrides this at runtime without a daemon restart.
#
# Default `info` keeps the log file lean; bump to `trace` or
# `debug,zshrs_daemon=trace` for first-pass diagnostics
# (resolved root, shard count, db sizes, watch list, token
# scopes, schedule rows etc — all gated to TRACE so they
# don't flood at INFO).
level = \"info\"

[http]
# HTTP listener for the `zd` CLI + curl + any HTTP client.
# Loopback-only by default so no token is required. Set to
# 0.0.0.0:7733 (or a non-loopback IP) to expose to your network —
# the daemon will refuse that bind unless [http.tokens] has at
# least one entry below.
listen = \"127.0.0.1:7733\"

# Bearer-token registry. Required for non-loopback listeners.
# Two value shapes per key:
#
#   [http.tokens]
#   # Legacy / unscoped — flat string. Token grants full access.
#   mybox    = \"replace-with-32-byte-random-hex\"
#
#   # Scoped — only grants the listed scopes.
#   # Wildcards: `*` (everything), `<area>.*`, `*.<verb>`.
#   vim-lsp  = { token = \"…\", scopes = [\"defs.read\", \"snapshot.read\"] }
#   ci-pipe  = { token = \"…\", scopes = [\"job.write\", \"cache.*\"] }
[http.tokens]
";

/// Default `zshrs.toml` written on first daemon startup. The daemon
/// owns the cache root, so seeding the shell-side file here too
/// means a single `zshrs-daemon` invocation leaves both knobs
/// discoverable + editable.
const DEFAULT_SHELL_TOML: &str = "\
# zshrs (shell) configuration. Lives at $ZSHRS_HOME/zshrs.toml
# (~/.zshrs/zshrs.toml by default). Auto-seeded on first run of
# any zshrs binary (zshrs / zshrs-daemon / zshrs-recorder / zd);
# edit freely.

[log]
# Tracing filter directive for the SHELL side (writes to
# zshrs.log + zshrs-recorder.log). Same syntax as $ZSHRS_LOG
# (which always wins when set). Accepts simple levels (info /
# debug / trace / warn / error) or per-module overrides
# (info,zsh::exec=debug,zsh::recorder=trace).
#
# Default `info`. Bump to `trace` for first-pass diagnostics
# of executor / parser / canonical_apply paths.
level = \"info\"

[daemon]
# Whether the shell talks to zshrs-daemon at startup:
#   auto     — connect if the socket is reachable, else go vanilla
#   on       — require a daemon, fail loudly if missing
#   off      — never connect, even if the daemon is running
enabled = \"auto\"

[shell]
# Whether to skip sourcing /etc/zshenv + ~/.{zshenv,zprofile,zshrc,
# zlogin} and rebuild executor state from the daemon's canonical
# rkyv shard instead. Saves ~150ms on shells with heavy plugin
# loads.
#   auto — skip iff daemon is up AND has a recorded zshrs shard
#   on   — always skip (config files become inert; recorder owns
#          all state)
#   off  — never skip (canonical state ignored even when present)
skip_configs = \"auto\"

# Optional: zsh script run once after dotfiles (or after canonical_apply
# when skip_configs applies). Omit entirely if unused.
# startup_config = \"~/.zshrs/shell-init.zsh\"
";

/// Default `zshrs-recorder.toml` written on first start. The recorder
/// today reads only `[log] level` from this file; future runtime
/// defaults (default `--shell-id`, default `--quiet`, default
/// `--no-daemon`) land here as the surface grows.
const DEFAULT_RECORDER_TOML: &str = "\
# zshrs-recorder configuration. Lives at
# $ZSHRS_HOME/zshrs-recorder.toml (~/.zshrs/zshrs-recorder.toml by
# default). Auto-seeded on first run of any zshrs binary; edit freely.

[log]
# Tracing filter directive for the RECORDER log
# (zshrs-recorder.log). Same syntax as $ZSHRS_LOG (which always
# wins when set). Accepts simple levels (info / debug / trace /
# warn / error) or per-module overrides
# (info,zsh::recorder=trace).
#
# Default `info`. Bump to `trace` to see every captured event +
# the source-resolver decisions for each captured file:line.
level = \"info\"
";

/// All cache-related paths for the running user.
#[derive(Clone, Debug)]
pub struct CachePaths {
    pub root: PathBuf,
    pub images: PathBuf,
    pub catalog_db: PathBuf,
    pub history_db: PathBuf,
    pub log: PathBuf,
    pub log_dir: PathBuf,
    pub log_file_name: String,
    pub socket: PathBuf,
    pub pid_file: PathBuf,
    pub index_rkyv: PathBuf,
    /// `replay/` — per-shell scripts holding the non-deterministic
    /// fragments of `.zshrc` that the daemon couldn't bake into canonical
    /// state. Per docs/DAEMON.md "Determinism boundary" (line 278).
    pub replay_dir: PathBuf,
    /// `cache.db` — daemon-as-service KV cache, namespaced. See
    /// docs/DAEMON_AS_SERVICE.md `daemon.cache.*` ops + daemon/cache.rs.
    pub cache_db: PathBuf,
    /// `artifacts/` — content-addressed artifact cache. Files live at
    /// `artifacts/<sha256_prefix>/<sha256_hex>`. See
    /// docs/DAEMON_AS_SERVICE.md `daemon.artifact.*` ops.
    pub artifacts_dir: PathBuf,
    /// `snapshots/` — tag-based canonical-state snapshots. See
    /// docs/DAEMON_AS_SERVICE.md `daemon.snapshot.*` ops.
    pub snapshots_dir: PathBuf,
}

impl CachePaths {
    /// Resolve to `$ZSHRS_HOME` or `$HOME/.zshrs`.
    ///
    /// Single top-level directory holds everything: rkyv shards, sqlite
    /// caches, sockets, pid file, logs, AND config
    /// (`zshrs.toml`, `zshrs-daemon.toml`, `zshrs-recorder.toml`).
    /// The directory is NOT cache-semantic — it survives
    /// OS cache eviction; that's why we don't use `XDG_CACHE_HOME`.
    /// Matches the convention of `~/.zinit/`, `~/.zpwr/`, `~/.oh-my-zsh/`.
    ///
    /// Override via `$ZSHRS_HOME` for tests / hermetic harnesses /
    /// users who want a non-default location.
    pub fn resolve() -> Result<Self> {
        let root = if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
            PathBuf::from(custom)
        } else {
            let home = std::env::var_os("HOME")
                .or_else(|| dirs::home_dir().map(|p| p.into_os_string()))
                .ok_or_else(|| DaemonError::other("could not resolve $HOME"))?;
            PathBuf::from(home).join(".zshrs")
        };
        Ok(Self::with_root(root))
    }

    /// Build paths anchored at an explicit root. Useful for tests with tempdirs.
    pub fn with_root<P: Into<PathBuf>>(root: P) -> Self {
        let root = root.into();
        let images = root.join("images");
        let catalog_db = root.join("catalog.db");
        let history_db = root.join("history.db");
        let log = root.join("zshrs-daemon.log");
        let log_dir = root.clone();
        let log_file_name = "zshrs-daemon.log".to_string();
        let socket = root.join("daemon.sock");
        let pid_file = root.join("daemon.pid");
        let index_rkyv = root.join("index.rkyv");
        let replay_dir = root.join("replay");
        let cache_db = root.join("cache.db");
        let artifacts_dir = root.join("artifacts");
        let snapshots_dir = root.join("snapshots");

        Self {
            root,
            images,
            catalog_db,
            history_db,
            log,
            log_dir,
            log_file_name,
            socket,
            pid_file,
            index_rkyv,
            replay_dir,
            cache_db,
            artifacts_dir,
            snapshots_dir,
        }
    }

    /// Create the cache directory tree with 0700 perms.
    pub fn ensure_dirs(&self) -> Result<()> {
        ensure_dir_700(&self.root)?;
        ensure_dir_700(&self.images)?;
        ensure_dir_700(&self.replay_dir)?;
        ensure_dir_700(&self.artifacts_dir)?;
        ensure_dir_700(&self.snapshots_dir)?;
        Ok(())
    }

    /// Path to `zshrs-daemon.toml` — the daemon's HTTP/auth/log/cache
    /// config. Renamed from the bare `daemon.toml` (briefly used during
    /// the initial seeding work) so every config file in the dir
    /// shares the `zshrs[-binary].toml` naming convention. Migration
    /// from the old name is in `ensure_default_configs`.
    pub fn daemon_config_path(&self) -> std::path::PathBuf {
        self.root.join("zshrs-daemon.toml")
    }

    /// Path to `zshrs.toml` — shell-side daemon-presence + skip-configs
    /// + log-level knobs (consumed by the `zshrs` binary).
    pub fn shell_config_path(&self) -> std::path::PathBuf {
        self.root.join("zshrs.toml")
    }

    /// Path to `zshrs-recorder.toml` — recorder-side log level + future
    /// runtime knobs (e.g. default --shell-id, default --quiet, etc.).
    pub fn recorder_config_path(&self) -> std::path::PathBuf {
        self.root.join("zshrs-recorder.toml")
    }

    /// Seed every default config file under `~/.zshrs/` if absent:
    ///   * `zshrs.toml`            — shell knobs
    ///   * `zshrs-daemon.toml`     — daemon knobs (auto-migrates from
    ///     the legacy `daemon.toml`)
    ///   * `zshrs-recorder.toml`   — recorder knobs
    ///
    /// Idempotent — never overwrites a user-edited file. Files are
    /// chmod'd 0600 because the same root holds secrets-bearing tokens
    /// once the user opts in. Called by every binary
    /// (`zshrs` / `zshrs-daemon` / `zshrs-recorder` / `zd`) at startup
    /// so whichever runs first gets the user a fully-populated
    /// `~/.zshrs/` tree without manual intervention.
    pub fn ensure_default_configs(&self) -> Result<()> {
        // Step 1: rename the legacy `daemon.toml` if present + the new
        // path doesn't exist yet. Single-shot, never overwrites the new
        // file. After this, the rest of the function treats the new
        // path as authoritative.
        let legacy_daemon = self.root.join("daemon.toml");
        let daemon_cfg = self.daemon_config_path();
        if legacy_daemon.exists() && !daemon_cfg.exists() {
            match std::fs::rename(&legacy_daemon, &daemon_cfg) {
                Ok(()) => tracing::info!(
                    from = %legacy_daemon.display(),
                    to = %daemon_cfg.display(),
                    "renamed legacy daemon.toml -> zshrs-daemon.toml"
                ),
                Err(e) => tracing::warn!(?e, "rename legacy daemon.toml failed"),
            }
        }

        if !daemon_cfg.exists() {
            std::fs::write(&daemon_cfg, DEFAULT_DAEMON_TOML)?;
            ensure_file_600(&daemon_cfg)?;
            tracing::info!(path = %daemon_cfg.display(), "seeded default zshrs-daemon.toml");
        }
        let shell_cfg = self.shell_config_path();
        if !shell_cfg.exists() {
            std::fs::write(&shell_cfg, DEFAULT_SHELL_TOML)?;
            ensure_file_600(&shell_cfg)?;
            tracing::info!(path = %shell_cfg.display(), "seeded default zshrs.toml");
        }
        let recorder_cfg = self.recorder_config_path();
        if !recorder_cfg.exists() {
            std::fs::write(&recorder_cfg, DEFAULT_RECORDER_TOML)?;
            ensure_file_600(&recorder_cfg)?;
            tracing::info!(path = %recorder_cfg.display(), "seeded default zshrs-recorder.toml");
        }
        Ok(())
    }

    /// Has the daemon ever completed an init pass on this machine for this user?
    /// First-run = no daemon.pid AND no index.rkyv AND no shards in images/.
    pub fn is_first_run(&self) -> bool {
        if self.pid_file.exists() {
            return false;
        }
        if self.index_rkyv.exists() {
            return false;
        }
        if let Ok(mut iter) = std::fs::read_dir(&self.images) {
            if iter.next().is_some() {
                return false;
            }
        }
        true
    }
}

fn ensure_dir_700(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    let mut perms = std::fs::metadata(path)?.permissions();
    if perms.mode() & 0o777 != 0o700 {
        perms.set_mode(0o700);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/// Resolve `$ZSHRS_HOME/zshrs-daemon.toml` (or
/// `~/.zshrs/zshrs-daemon.toml`). Single-directory rule: every zshrs
/// file — config, cache, sockets, rkyv shards, log — lives under one
/// root. Returns the path even if the file does not exist; callers
/// handle the not-present case as "no overrides" rather than as an
/// error.
pub fn daemon_config_file() -> Result<PathBuf> {
    let root = if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        PathBuf::from(custom)
    } else {
        dirs::home_dir()
            .map(|h| h.join(".zshrs"))
            .ok_or_else(|| DaemonError::other("no $HOME / $ZSHRS_HOME for zshrs-daemon.toml"))?
    };
    Ok(root.join("zshrs-daemon.toml"))
}

/// Load `[http]` section from `~/.zshrs/zshrs-daemon.toml` into the
/// `HttpConfig` consumed by `daemon::http::serve_http`. The file is
/// optional; a missing file or a missing `[http]` section both produce
/// the default (HTTP listener disabled).
///
/// `[http.tokens]` accepts two value shapes per key:
///
/// ```toml
/// [http]
/// listen = "127.0.0.1:7733"
///
/// [http.tokens]
/// # Legacy / unscoped — flat string. Token grants full access.
/// mybox      = "0123abcd..."
/// # Scoped — inline table. Token only grants the listed scopes.
/// # See `daemon::auth::op_scope` for the area.verb namespace.
/// vim-lsp    = { token = "feedface...", scopes = ["defs.read", "snapshot.read"] }
/// ci-pipe    = { token = "deadbeef...", scopes = ["job.write", "cache.*"] }
/// ```
///
/// Wildcards in `scopes`: `*` (everything), `<area>.*` (every verb in
/// an area), `*.<verb>` (every area's `<verb>`).
/// Read `[log] level` from `$ZSHRS_HOME/zshrs-daemon.toml`. Returns the
/// directive string (e.g. `"info"`, `"debug,fsnotify=trace"`) so the
/// caller can hand it straight to `EnvFilter::try_new`. Falls back
/// to `"info"` on any error: missing file, missing section, missing
/// key, parse error, non-string value. Errors aren't surfaced — the
/// caller will hit them again via the EnvFilter parse if the
/// directive is malformed, with a more useful "bad directive `xxx`"
/// message at THAT layer.
///
/// Honors the same `$ZSHRS_HOME` override every other path resolver
/// uses, and takes the resolved `CachePaths` so test harnesses can
/// point at a tempdir without poking env.
pub fn load_log_directive(paths: &CachePaths) -> String {
    const DEFAULT: &str = "info";
    let path = paths.daemon_config_path();
    if !path.exists() {
        return DEFAULT.into();
    }
    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(_) => return DEFAULT.into(),
    };
    let parsed: toml::Value = match body
        .parse::<toml::Table>()
        .map(toml::Value::Table)
    {
        Ok(v) => v,
        Err(_) => return DEFAULT.into(),
    };
    parsed
        .get("log")
        .and_then(|s| s.get("level"))
        .and_then(toml::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT.into())
}

pub fn load_http_config() -> Result<super::http::HttpConfig> {
    let path = daemon_config_file()?;
    if !path.exists() {
        return Ok(super::http::HttpConfig::default());
    }
    let body = std::fs::read_to_string(&path)?;
    // toml v1 reserves the FromStr impl on `Value` for SCALARS only;
    // documents must go through `Table::from_str`. Routing through
    // Value::from_str produces "unexpected content, expected nothing"
    // the moment the parser sees a table header.
    let parsed: toml::Value = body
        .parse::<toml::Table>()
        .map(toml::Value::Table)
        .map_err(|e| DaemonError::other(format!("daemon.toml parse: {e}")))?;
    let http_section = match parsed.get("http") {
        Some(v) => v,
        None => return Ok(super::http::HttpConfig::default()),
    };
    let listen = http_section
        .get("listen")
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let mut tokens: Vec<super::auth::Token> = Vec::new();
    if let Some(tok_table) = http_section.get("tokens").and_then(toml::Value::as_table) {
        for (label, val) in tok_table {
            // Legacy form: `name = "secret"` (full access).
            if let Some(secret) = val.as_str() {
                if !secret.is_empty() {
                    tokens.push(super::auth::Token {
                        label: label.clone(),
                        secret: secret.to_string(),
                        scopes: super::auth::ScopeMatcher::default(),
                    });
                }
                continue;
            }
            // Scoped form: `name = { token = "secret", scopes = [...] }`.
            if let Some(inner) = val.as_table() {
                let secret = match inner.get("token").and_then(toml::Value::as_str) {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => {
                        return Err(DaemonError::other(format!(
                            "daemon.toml: [http.tokens].{label} missing or empty `token` field"
                        )));
                    }
                };
                let scopes = inner
                    .get("scopes")
                    .and_then(toml::Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                tokens.push(super::auth::Token {
                    label: label.clone(),
                    secret,
                    scopes: super::auth::ScopeMatcher::from_strings(scopes),
                });
            }
        }
    }
    Ok(super::http::HttpConfig {
        listen,
        tokens: super::auth::TokenRegistry::new(tokens),
    })
}

/// True if `name` is the prefix of one of the three zshrs log files,
/// or any of their `.N` rotation siblings:
///
///   * `zshrs.log` / `zshrs.log.1` / `zshrs.log.2` …  (shell)
///   * `zshrs-daemon.log{,.N}`                       (daemon)
///   * `zshrs-recorder.log{,.N}`                     (recorder)
///
/// Used by ticker rotation, `zlog` builtin scanners, and snapshot
/// bundling. Centralized here so adding a fourth log file (e.g. for a
/// future `zshrs-test-runner` binary) only needs editing this one
/// function.
pub fn is_zshrs_log_file(name: &str) -> bool {
    name.starts_with("zshrs.log")
        || name.starts_with("zshrs-daemon.log")
        || name.starts_with("zshrs-recorder.log")
}

/// Set 0600 on a file path that already exists. Logs a warning on drift detection.
pub fn ensure_file_600(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut perms = std::fs::metadata(path)?.permissions();
    let mode = perms.mode() & 0o777;
    if mode != 0o600 {
        tracing::warn!(
            path = %path.display(),
            current_mode = format!("{:o}", mode),
            "file mode drift; coercing to 0600"
        );
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn paths_relative_to_root() {
        let tmp = TempDir::new().unwrap();
        let p = CachePaths::with_root(tmp.path());
        assert!(p.images.starts_with(tmp.path()));
        assert_eq!(p.images.file_name().unwrap(), "images");
        assert_eq!(p.socket.file_name().unwrap(), "daemon.sock");
        assert_eq!(p.pid_file.file_name().unwrap(), "daemon.pid");
    }

    #[test]
    fn ensure_dirs_sets_0700() {
        let tmp = TempDir::new().unwrap();
        let p = CachePaths::with_root(tmp.path().join("zshrs"));
        p.ensure_dirs().unwrap();

        let mode = std::fs::metadata(&p.root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);

        let mode = std::fs::metadata(&p.images).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn first_run_detected_on_empty_root() {
        let tmp = TempDir::new().unwrap();
        let p = CachePaths::with_root(tmp.path().join("zshrs"));
        p.ensure_dirs().unwrap();
        assert!(p.is_first_run());
    }

    #[test]
    fn first_run_false_when_pid_exists() {
        let tmp = TempDir::new().unwrap();
        let p = CachePaths::with_root(tmp.path().join("zshrs"));
        p.ensure_dirs().unwrap();
        std::fs::write(&p.pid_file, "12345").unwrap();
        assert!(!p.is_first_run());
    }
}
