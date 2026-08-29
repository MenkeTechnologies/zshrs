//! zshrs plugin package manager (`znative`) — a GLOBAL-only package manager for
//! zsh script plugins AND native (Rust cdylib) plugins.
//!
//! Ported from strykelang's package manager (`strykelang/pkg/`), retargeted
//! from `.stk` modules to zshrs plugins and simplified to the global-store
//! model (no per-project manifest/lockfile — the global install index at
//! `$ZSHRS_HOME/pkg/installed.toml` is the single source of truth).
//!
//! World's first: a JIT-compiled Unix shell whose package manager installs and
//! loads BOTH interpreted (zsh) and compiled (Rust cdylib) plugins from one
//! content-addressed global store, with SHA-256 integrity pinning.
//!
//! Surface:
//! - [`manifest`] — a plugin's optional `znative.toml` (`[plugin]`/`[native]`/
//!   `[script]`); auto-detected when absent.
//! - [`store`]    — `$ZSHRS_HOME/pkg/{store,cache,git,bin}/` layout + the
//!   `installed.toml` global index.
//! - [`resolver`] — turn a source spec (`owner/repo`, `git+URL`, `path:DIR`)
//!   into a staged directory ready to install.
//! - [`commands`] — `znative {add,remove,list,info,load,update}` implementations.
//! - [`builtin`]  — the `znative` builtin dispatcher (wired from `fusevm_bridge`).

pub mod builtin;
pub mod commands;
pub mod manifest;
pub mod resolver;
pub mod store;

/// Result alias used throughout the package manager. Errors are stringly-typed
/// (one user-facing diagnostic per failure path), emitted to stderr as
/// `znative: <reason>` with exit code 1 — matching the shell's terse error style.
pub type PkgResult<T> = Result<T, PkgError>;

/// Errors emitted by the package manager. `Display` produces the one-line
/// reason (no `znative:` prefix — the builtin adds it).
#[derive(Debug)]
pub enum PkgError {
    /// File I/O — read/write/create/copy.
    Io(String),
    /// Manifest parse error (bad TOML in a plugin's `znative.toml`).
    Manifest(String),
    /// Resolver error — unknown source form, clone/build/download failure.
    Resolve(String),
    /// The plugin kind could not be determined (no `znative.toml`, no
    /// `*.plugin.zsh`, no cdylib/`Cargo.toml`).
    Unknown(String),
    /// Generic runtime error.
    Other(String),
}

impl std::fmt::Display for PkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PkgError::Io(s)
            | PkgError::Manifest(s)
            | PkgError::Resolve(s)
            | PkgError::Unknown(s)
            | PkgError::Other(s) => write!(f, "{}", s),
        }
    }
}

impl From<std::io::Error> for PkgError {
    fn from(e: std::io::Error) -> Self {
        PkgError::Io(e.to_string())
    }
}

/// Deterministic SHA-256 of a directory tree, `sha256-<hex>`. Ported from
/// strykelang `pkg/lockfile.rs::integrity_for_directory`. Files are walked in
/// sorted order so the hash is stable regardless of filesystem iteration; each
/// file contributes `<relpath>\0F\0<len>\n<bytes>\n`, symlinks their target.
/// Recorded in the install index for change detection / audit.
pub fn store_integrity(root: &std::path::Path) -> PkgResult<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    fn walk(
        root: &std::path::Path,
        cur: &std::path::Path,
        out: &mut Vec<std::path::PathBuf>,
    ) -> PkgResult<()> {
        for entry in std::fs::read_dir(cur)? {
            let entry = entry?;
            let path = entry.path();
            let meta = entry.metadata()?;
            if meta.is_dir() && !meta.file_type().is_symlink() {
                walk(root, &path, out)?;
            } else {
                out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
        Ok(())
    }
    walk(root, root, &mut entries)?;
    entries.sort();
    for rel in &entries {
        let abs = root.join(rel);
        let meta = std::fs::symlink_metadata(&abs)?;
        let rel_s = rel.to_string_lossy();
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&abs)?;
            hasher.update(rel_s.as_bytes());
            hasher.update(b"\0L\0");
            hasher.update(target.to_string_lossy().as_bytes());
            hasher.update(b"\n");
        } else if meta.is_file() {
            let bytes = std::fs::read(&abs)?;
            hasher.update(rel_s.as_bytes());
            hasher.update(b"\0F\0");
            hasher.update(bytes.len().to_string().as_bytes());
            hasher.update(b"\n");
            hasher.update(&bytes);
            hasher.update(b"\n");
        }
    }
    Ok(format!("sha256-{:x}", hasher.finalize()))
}
