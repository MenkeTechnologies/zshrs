//! Crash-safe atomic file replacement for the rkyv shard caches.
//!
//! **zshrs-original — no C counterpart.** zsh has no persistent bytecode
//! store, so nothing here mirrors a `Src/` function.
//!
//! Both shard caches replace their file the same way: serialize into a
//! sibling temp file, `fsync`, `rename` over the target. The temp name
//! embeds the writing pid (`<file>.tmp.<pid>.<nanos>`), which is what
//! lets a later writer decide whether an abandoned temp is safe to
//! delete.
//!
//! Abandonment was not hypothetical. One `~/.zshrs` accumulated 18
//! orphaned `autoloads.rkyv.tmp.<pid>.<nanos>` files totalling 517 MB.
//! Two holes produced them, and both are closed here:
//!
//!   * an error return between `File::create` and `rename` left the temp
//!     on disk — [`TempFileGuard`] now unlinks it on every exit path,
//!     including the `?` returns and a panic;
//!   * a process killed inside that window never reached the rename at
//!     all — [`reap_orphan_temps`] unlinks temps whose recorded pid no
//!     longer exists.
//!
//! A temp owned by a **live** pid is never touched: it belongs to a
//! sibling shell that is still mid-write, and deleting it would corrupt
//! that write. Same reasoning as the `.git/index.lock` rule — a lock (or
//! a temp) you did not create is not yours to remove.

use std::fs::File;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Unlinks the temp file unless [`TempFileGuard::disarm`] was called
/// after a successful rename.
///
/// The rename is the commit point: once it succeeds the temp path no
/// longer exists and the guard must not fire (a same-named temp from a
/// later write would be the victim). Every path that does NOT reach the
/// rename — an I/O error, an early return, a panic while serializing —
/// leaves the guard armed, so the partial file is removed.
struct TempFileGuard {
    /// Path to remove while armed.
    path: PathBuf,
    /// Cleared by [`TempFileGuard::disarm`] once the rename committed.
    armed: bool,
}

impl TempFileGuard {
    /// Arm cleanup for `path`.
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    /// Stop tracking the temp file — the rename has taken ownership.
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The temp path a write of `path` uses: `<file>.tmp.<pid>.<nanos>`.
///
/// The pid is the ownership record [`reap_orphan_temps`] reads back; the
/// nanosecond stamp keeps two writes from the same process distinct.
fn temp_path_for(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("shard.rkyv");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    parent.join(format!("{}.tmp.{}.{}", file, std::process::id(), nanos))
}

/// Replace `path` with `bytes` atomically, leaving no temp file behind
/// on any exit path, and reap temps abandoned by processes that are gone.
///
/// The caller is expected to already hold whatever lock serializes
/// writers of `path` (both shard caches take an `flock` first); this
/// function only guarantees that a reader never sees a half-written
/// file and that a failed write does not leak one.
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let _ = std::fs::create_dir_all(parent);

    let tmp_path = temp_path_for(path);
    let mut guard = TempFileGuard::new(tmp_path.clone());
    {
        let mut f = File::create(&tmp_path).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp_path, path).map_err(|e| e.to_string())?;
    guard.disarm();

    let reaped = reap_orphan_temps(path);
    if reaped > 0 {
        tracing::info!(
            path = %path.display(),
            reaped,
            "shard write: removed temp files left by dead processes"
        );
    }
    Ok(())
}

/// Delete `<file>.tmp.<pid>.<nanos>` siblings of `path` whose `<pid>` is
/// no longer running. Returns how many were removed.
///
/// Skips this process's own temps (one may be in flight above us on the
/// stack) and anything owned by a pid that still exists — a sibling
/// shell writing the same shard right now.
pub fn reap_orphan_temps(path: &Path) -> usize {
    let Some(parent) = path.parent() else {
        return 0;
    };
    let Some(file) = path.file_name().and_then(|s| s.to_str()) else {
        return 0;
    };
    let prefix = format!("{}.tmp.", file);
    let me = std::process::id() as i32;
    let Ok(dir) = std::fs::read_dir(parent) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in dir.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        // `<pid>.<nanos>` — both all-digits, or this is not one of ours.
        let Some((pid_str, nanos)) = rest.split_once('.') else {
            continue;
        };
        if nanos.is_empty() || !nanos.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        // Parsed as i32 because that is what `kill(2)` takes: a value
        // that does not fit is not a pid this shell ever wrote, and a
        // negative one would address a process GROUP.
        let Ok(pid) = pid_str.parse::<i32>() else {
            continue;
        };
        if pid <= 0 || pid == me || pid_is_alive(pid) {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Does a process with this pid exist? `kill(pid, 0)` is the portable
/// existence probe: `EPERM` means it exists but belongs to another user
/// (still alive — hands off), `ESRCH` means it is gone.
fn pid_is_alive(pid: i32) -> bool {
    match nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None) {
        Ok(()) => true,
        Err(nix::errno::Errno::EPERM) => true,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn a_successful_write_leaves_no_temp_behind() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("shard.rkyv");
        write_bytes_atomic(&path, b"payload").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"payload");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn a_dead_pids_temp_is_reaped_and_a_live_pids_is_not() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("shard.rkyv");
        // pid 1 (launchd/init) always exists; kill(1, 0) answers EPERM
        // for a non-root caller, which counts as alive.
        let live = dir.path().join("shard.rkyv.tmp.1.123");
        // A pid that cannot be running: well past every Unix pid_max
        // (macOS caps at 99998, Linux at 2^22), so `kill` answers ESRCH.
        let dead = dir.path().join("shard.rkyv.tmp.2147483646.456");
        // Not a temp of ours — different base name, must survive.
        let other = dir.path().join("other.rkyv.tmp.2147483646.789");
        std::fs::write(&live, b"x").unwrap();
        std::fs::write(&dead, b"x").unwrap();
        std::fs::write(&other, b"x").unwrap();

        assert_eq!(reap_orphan_temps(&path), 1);
        assert!(live.exists(), "a live process's temp was deleted");
        assert!(!dead.exists(), "a dead process's temp was not reaped");
        assert!(other.exists(), "an unrelated file was deleted");
    }
}
