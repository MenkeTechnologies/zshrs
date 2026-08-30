//! Ship zsh's function tree with the binary and materialise it into
//! `~/.zshrs/functions`.
//!
//! **zshrs-original — no C counterpart.** zsh finds `is-at-least`,
//! `colors`, `add-zsh-hook`, `_git`, … through an fpath baked in by
//! `configure` (`<prefix>/share/zsh/<version>/functions`). zshrs is a
//! drop-in binary that is not installed under a zsh prefix, so it had
//! nothing of its own to fall back on: a shell started without `FPATH`
//! in the environment could not autoload anything at all, which is
//! exactly what `exec zshrs` produced --
//!
//! ```text
//! zsh: is-at-least: function definition file not found
//! zsh: colors: function definition file not found
//! zsh: add-zsh-hook: function definition file not found
//! ```
//!
//! The vendored `src/zsh/{Completion,Functions}` trees are packed by
//! `build.rs` into a single zstd blob (1245 files, ~1.1 MiB) and written
//! out here on first run, or after an upgrade, guarded by a version
//! stamp.
//!
//! # Layout
//!
//! FLAT, matching what zsh's own `make install` produces --
//! `/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions` holds 1235 files
//! and zero subdirectories. Keyed by basename, so `Base/Utility/_describe`
//! and `Misc/is-at-least` both land at the top level and a plain
//! `fpath=(~/.zshrs/functions $fpath)` resolves them.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The packed tree: `u32 name_len | name | u32 body_len | body`, repeated,
/// little-endian, zstd-compressed. Written by `bundle_zsh_functions` in
/// build.rs.
static BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/zsh_functions.zst"));

/// Written into the directory so an upgraded zshrs refreshes the tree
/// instead of leaving a stale one from an older build.
const STAMP: &str = ".zshrs-bundle-version";

/// `~/.zshrs/functions`.
pub fn functions_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".zshrs").join("functions"))
}

/// True when the directory is absent or was written by another version.
fn needs_write(dir: &Path) -> bool {
    match std::fs::read_to_string(dir.join(STAMP)) {
        Ok(s) => s.trim() != env!("CARGO_PKG_VERSION"),
        Err(_) => true,
    }
}

/// Materialise the bundle when missing or stale. Returns how many files
/// were written; `Some(0)` means the tree was already current.
///
/// Errors are swallowed on purpose: a read-only or full `$HOME` must not
/// stop the shell from starting. The cost on the common path is one
/// `read_to_string` of a ~7-byte stamp.
pub fn ensure_installed() -> Option<usize> {
    let dir = functions_dir()?;
    if !needs_write(&dir) {
        return Some(0);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let raw = zstd::decode_all(BUNDLE).ok()?;
    let mut n = 0usize;
    let mut i = 0usize;
    while i + 4 <= raw.len() {
        let nl = u32::from_le_bytes(raw[i..i + 4].try_into().ok()?) as usize;
        i += 4;
        if i + nl + 4 > raw.len() {
            break;
        }
        let name = String::from_utf8_lossy(&raw[i..i + nl]).into_owned();
        i += nl;
        let bl = u32::from_le_bytes(raw[i..i + 4].try_into().ok()?) as usize;
        i += 4;
        if i + bl > raw.len() {
            break;
        }
        let body = &raw[i..i + bl];
        i += bl;
        // Basename only: the bundle is flat, and a name with a separator
        // would otherwise escape the directory.
        if name.contains('/') || name.contains("..") || name.is_empty() {
            continue;
        }
        let dest = dir.join(&name);
        if std::fs::write(&dest, body).is_ok() {
            n += 1;
        }
    }
    if let Ok(mut f) = std::fs::File::create(dir.join(STAMP)) {
        let _ = f.write_all(env!("CARGO_PKG_VERSION").as_bytes());
    }
    tracing::info!(target: "bundled_functions", written = n, dir = %dir.display(),
                   "materialised bundled zsh functions");
    Some(n)
}
