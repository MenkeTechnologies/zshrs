// ~/.cache/zshrs/* path resolution + permission enforcement.
//
// Layout (matches docs/DAEMON.md):
//   ~/.cache/zshrs/
//   ├── index.rkyv
//   ├── images/                  ← 0700 dir, 0600 files
//   ├── catalog.db               ← 0600 (daemon-only writer)
//   ├── history.db               ← 0600
//   ├── zshrs.log                ← 0600 (10MB rotation)
//   ├── daemon.sock              ← 0600 Unix domain socket
//   └── daemon.pid               ← 0600 singleton flock
//
// Cache directory is 0700 (user-only). Files inside are 0600. Verified by
// `zcache verify` on every integrity scan; drift triggers WARN in zshrs.log.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::{DaemonError, Result};

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
    /// Resolve cache paths from `XDG_CACHE_HOME` or `~/.cache/zshrs`.
    ///
    /// **Not** `dirs::cache_dir()`, which on macOS returns
    /// `~/Library/Caches/`. The spec (docs/DAEMON.md) says `~/.cache/zshrs/`
    /// on every platform, and so do every doc, script, and example. Cross-
    /// platform consistency wins over Apple's HIG.
    pub fn resolve() -> Result<Self> {
        let root = if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
            PathBuf::from(xdg).join("zshrs")
        } else {
            let home = std::env::var_os("HOME")
                .or_else(|| dirs::home_dir().map(|p| p.into_os_string()))
                .ok_or_else(|| DaemonError::other("could not resolve $HOME"))?;
            PathBuf::from(home).join(".cache").join("zshrs")
        };
        Ok(Self::with_root(root))
    }

    /// Build paths anchored at an explicit root. Useful for tests with tempdirs.
    pub fn with_root<P: Into<PathBuf>>(root: P) -> Self {
        let root = root.into();
        let images = root.join("images");
        let catalog_db = root.join("catalog.db");
        let history_db = root.join("history.db");
        let log = root.join("zshrs.log");
        let log_dir = root.clone();
        let log_file_name = "zshrs.log".to_string();
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

/// Resolve `~/.cache/zshrs/daemon.toml` (respecting `$XDG_CACHE_HOME`).
/// Single-directory rule: every zshrs file — config, cache, sockets,
/// rkyv shards, log — lives under `~/.cache/zshrs/`. Returns the path
/// even if the file does not exist; callers handle the not-present
/// case as "no overrides" rather than as an error.
pub fn daemon_config_file() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))
        .ok_or_else(|| DaemonError::other("no $HOME / $XDG_CACHE_HOME for daemon.toml"))?;
    Ok(base.join("zshrs").join("daemon.toml"))
}

/// Load `[http]` section from `~/.cache/zshrs/daemon.toml` into the
/// `HttpConfig` consumed by `daemon::http::serve_http`. The file is
/// optional; a missing file or a missing `[http]` section both produce
/// the default (HTTP listener disabled).
///
/// `[http.tokens]` accepts two value shapes per key:
///
///     [http]
///     listen = "127.0.0.1:7733"
///
///     [http.tokens]
///     # Legacy / unscoped — flat string. Token grants full access.
///     mybox      = "0123abcd..."
///     # Scoped — inline table. Token only grants the listed scopes.
///     # See `daemon::auth::op_scope` for the area.verb namespace.
///     vim-lsp    = { token = "feedface...", scopes = ["defs.read", "snapshot.read"] }
///     ci-pipe    = { token = "deadbeef...", scopes = ["job.write", "cache.*"] }
///
/// Wildcards in `scopes`: `*` (everything), `<area>.*` (every verb in
/// an area), `*.<verb>` (every area's `<verb>`).
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
