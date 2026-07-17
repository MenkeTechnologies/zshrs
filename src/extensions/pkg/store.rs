//! Global store + installed index. Ported from strykelang's `pkg/store.rs`,
//! retargeted to `$ZSHRS_HOME/pkg/` and reduced to the global-only model.
//!
//! Layout (`$ZSHRS_HOME` defaults to `~/.zshrs`, matching
//! `autoload_cache::zshrs_home`):
//! ```text
//! $ZSHRS_HOME/pkg/
//!   store/  name@version/     # one extracted copy per (name, version)
//!   cache/                    # download scratch
//!   git/                      # bare/working git clones
//!   bin/                      # launcher symlinks (future)
//!   installed.toml            # the global install index (source of truth)
//! ```
//! Human-readable `name@version` paths give Nix-grade reproducibility from the
//! index's content hashes without Nix-grade opaque path UX.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::{PkgError, PkgResult};

/// Installed-index filename under the store root.
pub const INSTALLED_FILE: &str = "installed.toml";

/// Resolves and lazily creates the `$ZSHRS_HOME/pkg/...` layout.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Construct a [`Store`] rooted at `$ZSHRS_HOME/pkg/` (default
    /// `~/.zshrs/pkg/`). Honors `ZSHRS_HOME` like the rest of zshrs.
    pub fn user_default() -> PkgResult<Store> {
        let home = if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
            PathBuf::from(custom)
        } else {
            let h = std::env::var_os("HOME")
                .ok_or_else(|| PkgError::Other("HOME environment variable not set".into()))?;
            PathBuf::from(h).join(".zshrs")
        };
        Ok(Store {
            root: home.join("pkg"),
        })
    }

    /// Root at an explicit path (tests).
    pub fn at(root: impl Into<PathBuf>) -> Store {
        Store { root: root.into() }
    }

    /// `store/` — extracted packages.
    pub fn store_dir(&self) -> PathBuf {
        self.root.join("store")
    }
    /// `cache/` — download scratch.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }
    /// `git/` — git clones.
    pub fn git_dir(&self) -> PathBuf {
        self.root.join("git")
    }
    /// `bin/` — launcher links.
    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }
    /// `$ZSHRS_HOME/pkg/` root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where a package extraction lives: `store/{name}@{version}/`.
    pub fn package_dir(&self, name: &str, version: &str) -> PathBuf {
        self.store_dir().join(format!("{}@{}", name, version))
    }

    /// Create the full directory layout. Idempotent.
    pub fn ensure_layout(&self) -> PkgResult<()> {
        for d in [
            self.store_dir(),
            self.cache_dir(),
            self.git_dir(),
            self.bin_dir(),
        ] {
            std::fs::create_dir_all(&d)
                .map_err(|e| PkgError::Io(format!("create {}: {}", d.display(), e)))?;
        }
        Ok(())
    }

    /// True if a `name@version` extraction already exists.
    pub fn has_package(&self, name: &str, version: &str) -> bool {
        self.package_dir(name, version).is_dir()
    }

    /// Copy a staged plugin tree wholesale into `store/{name}@{version}/`,
    /// excluding VCS/build scratch (`.git/`, `target/`) so the store holds only
    /// the loadable plugin. The destination is cleared first for fresh
    /// re-installs. Returns the store path.
    pub fn install_dir(&self, name: &str, version: &str, src: &Path) -> PkgResult<PathBuf> {
        let dst = self.package_dir(name, version);
        if dst.exists() {
            std::fs::remove_dir_all(&dst)
                .map_err(|e| PkgError::Io(format!("clear {}: {}", dst.display(), e)))?;
        }
        std::fs::create_dir_all(&dst)?;
        copy_dir_filtered(src, &dst)?;
        Ok(dst)
    }
}

/// The global install index at `$ZSHRS_HOME/pkg/installed.toml` — the single
/// source of truth for what's installed. `zpm load` reads it at shell startup
/// to load every plugin with zero network.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstalledIndex {
    /// Schema version.
    pub version: u32,
    /// One entry per installed plugin, sorted by name for deterministic diffs.
    #[serde(default, rename = "package")]
    pub packages: Vec<InstalledPlugin>,
}

/// One installed plugin: identity + provenance + everything `zpm load` needs
/// to load it without re-resolving.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstalledPlugin {
    /// Plugin name (store key `name@version`).
    pub name: String,
    /// Installed version (`0.0.0` when the plugin declared none).
    pub version: String,
    /// Provenance: `github:owner/repo`, `git+URL`, or `path+file://DIR`.
    pub source: String,
    /// `"native"` or `"script"`.
    pub kind: String,
    /// SHA-256 of the extracted tree, `sha256-<hex>` (audit / change detection).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub integrity: String,
    /// Native: the cdylib filename inside the store dir (e.g. `libfoo.dylib`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lib: String,
    /// Script: files to `source`, relative to the store dir.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_files: Vec<String>,
    /// Script: dirs to prepend to `$fpath`, relative to the store dir.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fpath: Vec<String>,
}

impl InstalledIndex {
    /// Empty index stamped with the current schema version.
    pub fn new() -> InstalledIndex {
        InstalledIndex {
            version: 1,
            packages: Vec::new(),
        }
    }

    /// Load the index from a [`Store`], or an empty index when it doesn't exist.
    pub fn load_from(store: &Store) -> PkgResult<InstalledIndex> {
        let path = store.root().join(INSTALLED_FILE);
        if !path.is_file() {
            return Ok(InstalledIndex::new());
        }
        let s = std::fs::read_to_string(&path)
            .map_err(|e| PkgError::Io(format!("read {}: {}", path.display(), e)))?;
        toml::from_str::<InstalledIndex>(&s)
            .map_err(|e| PkgError::Other(format!("parse {}: {}", path.display(), e.message())))
    }

    /// Write the index (packages sorted by name) under `store.root()`.
    pub fn save_to(&mut self, store: &Store) -> PkgResult<()> {
        self.packages.sort_by(|a, b| a.name.cmp(&b.name));
        let path = store.root().join(INSTALLED_FILE);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(&self)
            .map_err(|e| PkgError::Other(format!("serialize {}: {}", INSTALLED_FILE, e)))?;
        std::fs::write(&path, format!("# zpm — auto-generated. Do not edit.\n{}", body))
            .map_err(|e| PkgError::Io(format!("write {}: {}", path.display(), e)))?;
        Ok(())
    }

    /// Find an installed plugin by name.
    pub fn find(&self, name: &str) -> Option<&InstalledPlugin> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Insert or replace the entry for `p.name`.
    pub fn upsert(&mut self, p: InstalledPlugin) {
        if let Some(slot) = self.packages.iter_mut().find(|e| e.name == p.name) {
            *slot = p;
        } else {
            self.packages.push(p);
        }
    }

    /// Remove the entry named `name`; returns it if present.
    pub fn remove(&mut self, name: &str) -> Option<InstalledPlugin> {
        let idx = self.packages.iter().position(|p| p.name == name)?;
        Some(self.packages.remove(idx))
    }
}

/// Recursively copy `src` into `dst`, skipping `.git/` and `target/` (VCS +
/// Rust build scratch) at any depth.
fn copy_dir_filtered(src: &Path, dst: &Path) -> PkgResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_s = name.to_string_lossy();
        if name_s == ".git" || name_s == "target" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_filtered(&from, &to)?;
        } else if ft.is_symlink() {
            // Copy the resolved file content (plugins rarely ship symlinks; a
            // dangling link in the store would break loads).
            if let Ok(target) = std::fs::read(&from) {
                std::fs::write(&to, target)?;
            }
        } else {
            std::fs::copy(&from, &to)
                .map_err(|e| PkgError::Io(format!("copy {}: {}", from.display(), e)))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "zpm-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn install_dir_skips_git_and_target() {
        let src = tmp();
        std::fs::write(src.join("a.plugin.zsh"), b"echo hi").unwrap();
        std::fs::create_dir_all(src.join(".git")).unwrap();
        std::fs::write(src.join(".git/HEAD"), b"ref").unwrap();
        std::fs::create_dir_all(src.join("target")).unwrap();
        std::fs::write(src.join("target/junk"), b"x").unwrap();
        let store = Store::at(tmp().join("pkg"));
        store.ensure_layout().unwrap();
        let dst = store.install_dir("a", "0.1.0", &src).unwrap();
        assert!(dst.join("a.plugin.zsh").is_file());
        assert!(!dst.join(".git").exists());
        assert!(!dst.join("target").exists());
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn index_round_trip() {
        let store = Store::at(tmp().join("pkg"));
        let mut idx = InstalledIndex::new();
        idx.upsert(InstalledPlugin {
            name: "zed".into(),
            version: "1.0.0".into(),
            source: "github:o/zed".into(),
            kind: "script".into(),
            source_files: vec!["zed.plugin.zsh".into()],
            ..Default::default()
        });
        idx.upsert(InstalledPlugin {
            name: "abc".into(),
            version: "0.1.0".into(),
            source: "github:o/abc".into(),
            kind: "native".into(),
            lib: "libabc.dylib".into(),
            ..Default::default()
        });
        idx.save_to(&store).unwrap();
        let back = InstalledIndex::load_from(&store).unwrap();
        assert_eq!(back.packages.len(), 2);
        // Sorted by name: abc before zed.
        assert_eq!(back.packages[0].name, "abc");
        assert_eq!(back.find("zed").unwrap().kind, "script");
        let _ = std::fs::remove_dir_all(store.root());
    }
}
