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
}

impl CachePaths {
    /// Resolve cache paths from the user's `XDG_CACHE_HOME` or `~/.cache`.
    pub fn resolve() -> Result<Self> {
        let cache_home = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::cache_dir())
            .ok_or_else(|| DaemonError::other("could not resolve cache home"))?;

        let root = cache_home.join("zshrs");
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
        }
    }

    /// Create the cache directory tree with 0700 perms.
    pub fn ensure_dirs(&self) -> Result<()> {
        ensure_dir_700(&self.root)?;
        ensure_dir_700(&self.images)?;
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
