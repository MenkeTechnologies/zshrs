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

/// Written into the directory so a zshrs carrying a different tree
/// refreshes it instead of leaving a stale one from an older build.
const STAMP: &str = ".zshrs-bundle-version";

include!(concat!(env!("OUT_DIR"), "/zsh_functions_id.rs"));

/// What [`STAMP`] holds: crate version plus the bundle's content hash.
/// The version alone is not enough -- the tree can change within a
/// version, and then a version-only stamp never triggers a rewrite.
fn stamp_value() -> String {
    format!("{}-{}", env!("CARGO_PKG_VERSION"), BUNDLE_ID)
}

/// `~/.zshrs/functions`.
pub fn functions_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".zshrs").join("functions"))
}

/// True when `dir` is a materialised bundle -- it carries [`STAMP`].
///
/// Identity by content, not by path: [`functions_dir`] is derived from the
/// LIVE `$HOME`, so after `export HOME=…` it names a directory that is not
/// the one startup put on `$fpath`. zsh's own stock tree is a configure-time
/// constant that no assignment moves, and the router's stock-tree test has
/// to be just as stable.
pub fn is_bundle_dir(dir: &Path) -> bool {
    dir.join(STAMP).is_file()
}

/// True when the directory is absent or holds a different bundle.
fn needs_write(dir: &Path) -> bool {
    match std::fs::read_to_string(dir.join(STAMP)) {
        Ok(s) => s.trim() != stamp_value(),
        Err(_) => true,
    }
}

/// Write `body` to `dir/name` so that no reader ever sees it half-written.
///
/// Every shell that starts without a current bundle installs it, and shells
/// start together (sixteen of them share one `$HOME`), so one shell's
/// `compinit` can scan this directory while another is still writing it. A
/// plain `fs::write` truncates the file and then fills it: a scan landing in
/// between reads an empty completer, finds no `#compdef` line and registers
/// nothing for it -- measured as two `$_comps` entries missing from one shell
/// of eight started at once. The body goes to a process-private dot-file first
/// and is renamed over the destination, which is atomic within a filesystem; the
/// dot prefix keeps the scan (`_*` only) from ever opening the temporary.
fn write_atomically(dir: &Path, name: &str, body: &[u8]) -> bool {
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let dest = dir.join(name);
    let ok = std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(body))
        .and_then(|()| std::fs::rename(&tmp, &dest))
        .is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
    ok
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
        if write_atomically(&dir, &name, body) {
            n += 1;
        }
    }
    write_atomically(&dir, STAMP, stamp_value().as_bytes());
    tracing::info!(target: "bundled_functions", written = n, dir = %dir.display(),
                   "materialised bundled zsh functions");
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader polling a file while another process rewrites it must only ever
    /// see a complete body. `fs::write` (truncate, then fill) fails this within a
    /// few hundred iterations; rename-into-place never does.
    #[test]
    fn a_reader_never_sees_a_partly_written_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let body = vec![b'x'; 256 * 1024];
        assert!(write_atomically(dir.path(), "_probe", &body));
        let path = dir.path().join("_probe");

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (path, stop, len) = (path.clone(), stop.clone(), body.len());
            std::thread::spawn(move || {
                let mut torn = 0usize;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(read) = std::fs::read(&path) {
                        if read.len() != len {
                            torn += 1;
                        }
                    }
                }
                torn
            })
        };
        for _ in 0..400 {
            assert!(write_atomically(dir.path(), "_probe", &body));
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(reader.join().expect("reader"), 0, "a read saw a truncated or partial file");
    }

    #[test]
    fn the_temporary_is_gone_after_the_rename() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(write_atomically(dir.path(), "_one", b"#compdef one\n"));
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["_one".to_string()]);
    }
}
