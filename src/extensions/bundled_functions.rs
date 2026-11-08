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

/// Held exclusively while the bundle is being written. A dot-file: the `_*` scan
/// and function lookup never open it.
const INSTALL_LOCK: &str = ".zshrs-bundle-install.lock";

include!(concat!(env!("OUT_DIR"), "/zsh_functions_id.rs"));

/// What [`STAMP`] holds: the bundle's content hash. A crate version alone
/// is not enough -- the tree can change within a version -- and the version
/// adds nothing once the hash is there.
fn stamp_value() -> String {
    // Content hash only. The crate version is not part of the stamp: two builds
    // with the same bundle must not rewrite each other's tree (and bump the
    // stamp's mtime, which `compinit` reads to decide a dump is stale) every
    // time shells of different versions start alternately.
    BUNDLE_ID.to_string()
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
    let dest = dir.join(name);
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    // Already the right bytes (most of the tree is unchanged from one release to the
    // next): leave the file, and its mtime, alone. Three metadata operations per file
    // across 1.2k files is seconds on a busy disk.
    if std::fs::read(&dest).is_ok_and(|have| have == body) {
        return true;
    }
    let ok = std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(body))
        .and_then(|()| std::fs::rename(&tmp, &dest))
        .is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
    ok
}

/// Create `dir/name` holding `body` in one `open(O_CREAT|O_EXCL)`; a file that is already
/// there (an install that died half way) goes through [`write_atomically`] instead.
fn write_new(dir: &Path, name: &str, body: &[u8]) -> bool {
    match std::fs::File::options().write(true).create_new(true).open(dir.join(name)) {
        Ok(mut f) => f.write_all(body).is_ok(),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => write_atomically(dir, name, body),
        Err(_) => false,
    }
}

/// Run `f` over `items` on a few plain scoped threads, all joined before this returns.
///
/// Creating 1.5k files is a syscall-bound loop; file creation in distinct dentries
/// scales across threads. Plain threads rather than rayon's global pool: nothing is left
/// running afterwards for a later `fork` to leave in a half-copied state.
pub(crate) fn par_each<T: Sync>(items: &[T], f: impl Fn(&T) + Sync) {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for part in items.chunks(chunk) {
            let f = &f;
            s.spawn(move || part.iter().for_each(f));
        }
    });
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
    // One installer at a time. Shells that start together all see a missing or stale
    // stamp, and each used to rewrite all ~1.2k files; on a busy disk that is seconds per
    // shell and the others read the half-installed tree meanwhile. The first takes the
    // lock and installs; the rest wait, find the stamp current and return.
    let _installing = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(INSTALL_LOCK))
        .ok()
        .and_then(|f| nix::fcntl::Flock::lock(f, nix::fcntl::FlockArg::LockExclusive).ok());
    if !needs_write(&dir) {
        return Some(0);
    }
    // No stamp at all: a first install. Every other shell is parked on the lock above
    // until the stamp lands, so nothing can scan a half-written file and the
    // tmp-file-plus-rename dance (two directory operations per file) is skipped.
    let fresh = !dir.join(STAMP).exists();
    let raw = zstd::decode_all(BUNDLE).ok()?;
    let mut files: Vec<(&str, &[u8])> = Vec::new();
    let mut i = 0usize;
    while i + 4 <= raw.len() {
        let nl = u32::from_le_bytes(raw[i..i + 4].try_into().ok()?) as usize;
        i += 4;
        if i + nl + 4 > raw.len() {
            break;
        }
        let name = std::str::from_utf8(&raw[i..i + nl]).ok();
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
        match name {
            Some(n) if !n.contains('/') && !n.contains("..") && !n.is_empty() => files.push((n, body)),
            _ => {}
        }
    }
    let written = std::sync::atomic::AtomicUsize::new(0);
    par_each(&files, |&(name, body)| {
        let ok = if fresh { write_new(&dir, name, body) } else { write_atomically(&dir, name, body) };
        if ok {
            written.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
    let n = written.into_inner();
    prune_stale(&dir, &stamp_value(), &files);
    write_atomically(&dir, STAMP, stamp_value().as_bytes());
    tracing::info!(target: "bundled_functions", written = n, dir = %dir.display(),
                   "materialised bundled zsh functions");
    Some(n)
}

/// Records which files each recently installed bundle carries, one file per
/// bundle id. A dot-directory, so the `_*` scan never opens it.
const MANIFESTS: &str = ".zshrs-bundle-manifests";

/// How long a bundle's file list protects its files after its last install.
/// Shells of different versions share this directory and each rewrites it when
/// it starts (the installed shell and a development build, say), so another
/// recent bundle's files may belong to a shell that is still running.
const MANIFEST_TTL: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);

/// Remove the files no recently installed bundle carries.
///
/// The directory sits at the front of `$fpath` as zshrs's stock tree, so a
/// completer that left the bundle (renamed, or dropped upstream) kept
/// claiming its `#compdef` commands after an upgrade: an old
/// `_analyseplugin` held `analyseplugin` ahead of the bundle's `_ladspa`, and
/// an old `_nameref` held `nameref` ahead of `_typeset`.
///
/// Removing everything this bundle does not carry would also remove the
/// files of another version installed alongside, which a shell of that
/// version may still autoload from. So this bundle's list is recorded under
/// [`MANIFESTS`], and a file goes only when no list written within
/// [`MANIFEST_TTL`] names it; older lists are dropped. Dot-files (the stamp,
/// the lock, in-flight temporaries, the lists) stay.
fn prune_stale(dir: &Path, id: &str, files: &[(&str, &[u8])]) {
    let manifests = dir.join(MANIFESTS);
    let _ = std::fs::create_dir_all(&manifests);
    let list: String = files.iter().map(|&(n, _)| format!("{n}\n")).collect();
    write_atomically(&manifests, id, list.as_bytes());

    let mut keep: std::collections::HashSet<String> = files.iter().map(|&(n, _)| n.to_string()).collect();
    for e in std::fs::read_dir(&manifests).into_iter().flatten().flatten() {
        let fresh = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < MANIFEST_TTL);
        if !fresh {
            let _ = std::fs::remove_file(e.path());
        } else if let Ok(text) = std::fs::read_to_string(e.path()) {
            keep.extend(text.lines().map(str::to_string));
        }
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with('.') && !keep.contains(name) && e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader polling a file while another process rewrites it must only ever
    /// see a complete body. `fs::write` (truncate, then fill) fails this within a
    /// few hundred iterations; rename-into-place never does. The two bodies differ
    /// (an identical rewrite is skipped) and have one length, so a short read is
    /// always a torn one.
    #[test]
    fn a_reader_never_sees_a_partly_written_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (a, b) = (vec![b'a'; 32 * 1024], vec![b'b'; 32 * 1024]);
        assert!(write_atomically(dir.path(), "_probe", &a));
        let path = dir.path().join("_probe");

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (path, stop, len) = (path.clone(), stop.clone(), a.len());
            std::thread::spawn(move || {
                let mut torn = 0usize;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Ok(read) = std::fs::read(&path) {
                        let whole = read.len() == len
                            && (read.iter().all(|&c| c == b'a') || read.iter().all(|&c| c == b'b'));
                        if !whole {
                            torn += 1;
                        }
                    }
                }
                torn
            })
        };
        for round in 0..200 {
            assert!(write_atomically(dir.path(), "_probe", if round % 2 == 0 { &b } else { &a }));
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(reader.join().expect("reader"), 0, "a read saw a truncated or partial file");
    }

    #[test]
    fn an_unchanged_file_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(write_atomically(dir.path(), "_same", b"#compdef same\n"));
        let first = std::fs::metadata(dir.path().join("_same")).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(write_atomically(dir.path(), "_same", b"#compdef same\n"));
        let second = std::fs::metadata(dir.path().join("_same")).unwrap().modified().unwrap();
        assert_eq!(first, second, "identical content must not be rewritten");
        assert!(write_atomically(dir.path(), "_same", b"#compdef other\n"));
        assert_eq!(std::fs::read(dir.path().join("_same")).unwrap(), b"#compdef other\n");
    }

    /// A first install creates files directly; a file left by a dead install is replaced
    /// whole, and every item reaches the worker threads exactly once.
    #[test]
    fn first_install_writes_every_file_and_replaces_leftovers() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("_left"), b"stale").unwrap();
        let names: Vec<String> = (0..100).map(|i| format!("_f{i}")).chain(["_left".to_string()]).collect();
        par_each(&names, |n| assert!(write_new(dir.path(), n, n.as_bytes())));
        for n in &names {
            assert_eq!(std::fs::read(dir.path().join(n)).unwrap(), n.as_bytes());
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), names.len());
    }

    /// An upgrade drops the completers the new bundle no longer carries, so a
    /// stale one cannot keep its `#compdef` commands, and keeps the dot-files.
    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A file no recent bundle carries goes; dot-files stay.
    #[test]
    fn an_upgrade_removes_files_no_recent_bundle_carries() {
        let dir = tempfile::tempdir().expect("tempdir");
        for n in ["_analyseplugin", "_ladspa", STAMP] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        prune_stale(dir.path(), "new", &[("_ladspa", b"#compdef analyseplugin\n")]);
        assert_eq!(listing(dir.path()), vec![MANIFESTS.to_string(), STAMP.to_string(), "_ladspa".to_string()]);
    }

    /// Two versions installing alternately keep each other's files: a shell of
    /// the other version may still be running and autoload from here.
    #[test]
    fn alternating_versions_keep_each_others_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Each install writes its own files, then prunes.
        for n in ["_old_only", "_shared"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        prune_stale(dir.path(), "old", &[("_old_only", b""), ("_shared", b"")]);
        std::fs::write(dir.path().join("_new_only"), b"x").unwrap();
        prune_stale(dir.path(), "new", &[("_shared", b""), ("_new_only", b"")]);
        prune_stale(dir.path(), "old", &[("_old_only", b""), ("_shared", b"")]);
        assert!(dir.path().join("_old_only").exists(), "the other version's file was removed");
        assert!(dir.path().join("_new_only").exists());
    }

    /// A bundle's list stops protecting its files once it has not been
    /// installed for MANIFEST_TTL, and the expired list itself is dropped.
    #[test]
    fn an_expired_bundle_list_protects_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("_gone"), b"x").unwrap();
        prune_stale(dir.path(), "old", &[("_gone", b"")]);
        let old = dir.path().join(MANIFESTS).join("old");
        let past = std::time::SystemTime::now() - MANIFEST_TTL - std::time::Duration::from_secs(60);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(past).unwrap();
        prune_stale(dir.path(), "new", &[("_kept", b"")]);
        assert!(!dir.path().join("_gone").exists());
        assert!(!old.exists());
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
