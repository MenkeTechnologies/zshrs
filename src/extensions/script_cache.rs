//! rkyv-backed bytecode cache for zsh scripts.
//!
//! Single-file shard at `~/.zshrs/scripts.rkyv`. On 2+ runs of a given
//! script, lex/parse/compile is skipped — the cache hit is `mmap` + zero-copy
//! `ArchivedHashMap` lookup + bincode-decode of the inner `fusevm::Chunk` blob.
//!
//! Storage layout (rkyv archived):
//!   ScriptShard {
//!     header: { magic, format_version, zshrs_version, pointer_width, built_at_secs },
//!     entries: HashMap<canonical_path, ScriptEntry>,
//!   }
//!   ScriptEntry { mtime_secs, mtime_nsecs, binary_mtime_at_cache,
//!                 binary_len_at_cache, cached_at_secs, chunk_blob: `Vec<u8>` }
//!
//! Inner `chunk_blob` is bincode for now — `fusevm::Chunk` is owned by the
//! upstream `fusevm` crate and only derives `serde::Serialize`/`Deserialize`,
//! not `rkyv::Archive`, so the inner codec stays bincode inside the rkyv outer
//! container. Direct rkyv on Chunk would require either forking fusevm or a
//! mirror archived type — both are large refactors and not needed for the
//! current "kill SQLite for bytecode" goal.
//!
//! Read path:
//!   - Lazy `mmap` of the shard, kept alive for the process lifetime so repeat
//!     lookups pay validation once.
//!   - `rkyv::check_archived_root::<ScriptShard>` validates the byte image.
//!   - Header validated for magic / format_version / zshrs_version / pointer_width.
//!   - Per-entry: source mtime must match, and the entry's recorded binary
//!     identity (`binary_mtime_at_cache`, `binary_len_at_cache`) must EQUAL the
//!     running binary's (mtime, len). Bytecode is only ever replayed by the
//!     exact build that emitted it; any rebuild invalidates entries silently.
//!
//! Write path:
//!   - `bin_zsystem_flock(LOCK_EX)` on `scripts.rkyv.lock` so concurrent writers serialize.
//!   - Read existing shard into owned form, mutate, `rkyv::to_bytes`,
//!     write to `scripts.rkyv.tmp.<pid>.<nanos>`, fsync, atomic-rename.
//!   - Drop the in-process `mmap` so the next read picks up the new shard.
//!
//! Ported from `strykelang/strykelang/script_cache.rs` (the user's stryke
//! language has the same caching pattern; this is the same shape with `ZRSC`
//! magic, zshrs version pin, and a single `chunk_blob` per entry — zshrs has
//! no separate AST cache).

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use memmap2::Mmap;
use parking_lot::Mutex;
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use std::os::unix::fs::MetadataExt;

/// Magic header bytes — fail-fast if a wrong-format file is mmap'd.
/// "ZRSC" little-endian.
pub const SHARD_MAGIC: u32 = 0x5A525343;
/// Bumped on incompatible rkyv schema changes.
///
/// v2 added `ScriptEntry::binary_len_at_cache`; a v1 shard has no length to
/// compare against, so it is rejected wholesale rather than half-validated.
pub const SHARD_FORMAT_VERSION: u32 = 2;
/// `ShardHeader` — see fields for layout.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct ShardHeader {
    /// `magic` field.
    pub magic: u32,
    /// `format_version` field.
    pub format_version: u32,
    /// `zshrs_version` field.
    pub zshrs_version: String,
    /// `pointer_width` field.
    pub pointer_width: u32,
    /// `built_at_secs` field.
    pub built_at_secs: u64,
}
/// `ScriptEntry` — see fields for layout.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct ScriptEntry {
    /// `mtime_secs` field.
    pub mtime_secs: i64,
    /// `mtime_nsecs` field.
    pub mtime_nsecs: i64,
    /// mtime of the zshrs binary that compiled `chunk_blob`.
    pub binary_mtime_at_cache: i64,
    /// Size of the zshrs binary that compiled `chunk_blob`. Paired with
    /// `binary_mtime_at_cache` to identify the emitting build: mtime alone
    /// has one-second granularity and moves in both directions (an older
    /// binary restored over a newer one keeps its old timestamp), so it
    /// cannot by itself prove the running build emitted these bytes.
    pub binary_len_at_cache: u64,
    /// `cached_at_secs` field.
    pub cached_at_secs: i64,
    /// `chunk_blob` field.
    pub chunk_blob: Vec<u8>,
}
/// `ScriptShard` — see fields for layout.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct ScriptShard {
    /// `header` field.
    pub header: ShardHeader,
    /// `entries` field.
    pub entries: HashMap<String, ScriptEntry>,
}

/// mmap + validated `*const ArchivedScriptShard`. Self-referential — the pointer
/// is valid for the lifetime of the wrapping struct.
pub struct MmappedShard {
    /// `_mmap` field.
    _mmap: Mmap,
    /// `archived` field.
    archived: *const ArchivedScriptShard,
}

// SAFETY: the pointer aliases an immutable mmap that lives as long as Self.
// rkyv-validated reads are immutable.
unsafe impl Send for MmappedShard {}
unsafe impl Sync for MmappedShard {}

impl MmappedShard {
    /// `open` — see implementation.
    pub fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let mmap = unsafe { Mmap::map(&file).ok()? };
        let archived = rkyv::check_archived_root::<ScriptShard>(&mmap[..]).ok()?;
        let archived_ptr = archived as *const ArchivedScriptShard;
        Some(Self {
            _mmap: mmap,
            archived: archived_ptr,
        })
    }

    fn shard(&self) -> &ArchivedScriptShard {
        // SAFETY: see Self impl comment.
        unsafe { &*self.archived }
    }

    fn header_ok(&self) -> bool {
        let h = &self.shard().header;
        let magic: u32 = h.magic.into();
        let fv: u32 = h.format_version.into();
        let pw: u32 = h.pointer_width.into();
        magic == SHARD_MAGIC
            && fv == SHARD_FORMAT_VERSION
            && pw as usize == std::mem::size_of::<usize>()
            && h.zshrs_version.as_str() == env!("CARGO_PKG_VERSION")
    }

    fn lookup(&self, path: &str) -> Option<&ArchivedScriptEntry> {
        self.shard().entries.get(path)
    }

    fn entry_count(&self) -> usize {
        self.shard().entries.len()
    }
}

/// Shard cache keyed by canonical script path. One per shard file.
pub struct ScriptCache {
    /// `path` field.
    path: PathBuf,
    /// `lock_path` field.
    lock_path: PathBuf,
    /// `mmap` field.
    mmap: Mutex<Option<MmappedShard>>,
}

impl ScriptCache {
    /// `open` — see implementation.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let parent = path.parent().unwrap_or_else(|| Path::new("/tmp"));
        let lock_path = parent.join(format!(
            "{}.lock",
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("scripts.rkyv")
        ));
        Ok(Self {
            path: path.to_path_buf(),
            lock_path,
            mmap: Mutex::new(None),
        })
    }

    fn ensure_mmap(&self) {
        let mut guard = self.mmap.lock();
        if guard.is_none() {
            *guard = MmappedShard::open(&self.path);
        }
    }

    fn invalidate_mmap(&self) {
        let mut guard = self.mmap.lock();
        *guard = None;
    }

    /// Cache lookup. Returns `None` on miss, mtime mismatch, version drift, or
    /// zshrs binary newer than the cached entry.
    pub fn get(&self, path: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<Vec<u8>> {
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let shard = guard.as_ref()?;
        if !shard.header_ok() {
            return None;
        }
        let entry = shard.lookup(path)?;

        let entry_mtime_s: i64 = entry.mtime_secs.into();
        let entry_mtime_ns: i64 = entry.mtime_nsecs.into();
        if entry_mtime_s != mtime_secs || entry_mtime_ns != mtime_nsecs {
            return None;
        }

        // Was this chunk emitted by the binary that is running right now?
        //
        // EXACT equality on (mtime, len), the same test `autoload_cache`
        // already applies to its chunks. The previous `cached < running`
        // comparison only rejected a cache written by an OLDER build, so a
        // binary whose mtime went BACKWARDS — an earlier build restored over
        // a later one, a `cp` that preserves timestamps, a checkout of a
        // previously-built target dir — silently replayed the newer build's
        // bytecode. Proven by touching the binary's mtime into the past and
        // watching the shard still hit. The one-second granularity of the
        // stored mtime has the same effect when a rebuild lands in the same
        // second as the cache write, which is why the length is compared too.
        match current_binary_identity() {
            Some((bin_mtime, bin_len)) => {
                let cached_bin_mtime: i64 = entry.binary_mtime_at_cache.into();
                let cached_bin_len: u64 = entry.binary_len_at_cache.into();
                if cached_bin_mtime != bin_mtime || cached_bin_len != bin_len {
                    return None;
                }
            }
            // No `current_exe()` — nothing can be proven, so nothing is used.
            None => return None,
        }

        Some(entry.chunk_blob.as_slice().to_vec())
    }

    /// Insert / replace an entry. Serializes the whole shard and atomic-renames.
    pub fn put(
        &self,
        path: &str,
        mtime_secs: i64,
        mtime_nsecs: i64,
        chunk_blob: Vec<u8>,
    ) -> Result<(), String> {
        let _lock = match acquire_lock(&self.lock_path) {
            Some(l) => l,
            None => return Ok(()),
        };

        let mut shard = match read_owned_shard(&self.path) {
            Some(s)
                if s.header.zshrs_version == env!("CARGO_PKG_VERSION")
                    && s.header.pointer_width as usize == std::mem::size_of::<usize>()
                    && s.header.format_version == SHARD_FORMAT_VERSION =>
            {
                s
            }
            _ => fresh_shard(),
        };

        let (bin_mtime, bin_len) = current_binary_identity().unwrap_or((0, 0));
        let entry = ScriptEntry {
            mtime_secs,
            mtime_nsecs,
            binary_mtime_at_cache: bin_mtime,
            binary_len_at_cache: bin_len,
            cached_at_secs: now_secs(),
            chunk_blob,
        };
        shard.entries.insert(path.to_string(), entry);
        shard.header.built_at_secs = now_secs() as u64;

        write_shard_atomic(&self.path, &shard)?;
        self.invalidate_mmap();
        Ok(())
    }

    /// `(count, total_blob_bytes)` snapshot.
    pub fn stats(&self) -> (i64, i64) {
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let Some(shard) = guard.as_ref() else {
            return (0, 0);
        };
        let count = shard.entry_count() as i64;
        let bytes: i64 = shard
            .shard()
            .entries
            .values()
            .map(|e| e.chunk_blob.len() as i64)
            .sum();
        (count, bytes)
    }

    /// `(path, chunk_kb, version, cached_at_localstr)` per entry,
    /// sorted by `cached_at` desc.
    pub fn list_scripts(&self) -> Vec<(String, f64, String, String)> {
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let Some(shard) = guard.as_ref() else {
            return Vec::new();
        };
        let v = shard.shard().header.zshrs_version.as_str().to_string();
        let mut out: Vec<(String, f64, String, String, i64)> = shard
            .shard()
            .entries
            .iter()
            .map(|(k, e)| {
                let chunk_kb = e.chunk_blob.len() as f64 / 1024.0;
                let cached_at: i64 = e.cached_at_secs.into();
                let ts = format_local_ts(cached_at);
                (k.as_str().to_string(), chunk_kb, v.clone(), ts, cached_at)
            })
            .collect();
        out.sort_by_key(|x| std::cmp::Reverse(x.4));
        out.into_iter()
            .map(|(p, ck, ver, ts, _)| (p, ck, ver, ts))
            .collect()
    }

    /// Drop entries whose source file vanished or whose mtime changed.
    pub fn evict_stale(&self) -> usize {
        let _lock = match acquire_lock(&self.lock_path) {
            Some(l) => l,
            None => return 0,
        };
        let mut shard = match read_owned_shard(&self.path) {
            Some(s) => s,
            None => return 0,
        };
        let before = shard.entries.len();
        shard.entries.retain(|p, e| match file_mtime(Path::new(p)) {
            Some((s, ns)) => s == e.mtime_secs && ns == e.mtime_nsecs,
            None => false,
        });
        let evicted = before - shard.entries.len();
        if evicted > 0 {
            let _ = write_shard_atomic(&self.path, &shard);
            self.invalidate_mmap();
        }
        evicted
    }
    /// `clear` — see implementation.
    pub fn clear(&self) -> std::io::Result<()> {
        let _lock = acquire_lock(&self.lock_path);
        let res = match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        self.invalidate_mmap();
        res
    }
}

fn acquire_lock(path: &Path) -> Option<nix::fcntl::Flock<File>> {
    let f = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .ok()?;
    nix::fcntl::Flock::lock(f, nix::fcntl::FlockArg::LockExclusive).ok()
}

fn fresh_shard() -> ScriptShard {
    ScriptShard {
        header: ShardHeader {
            magic: SHARD_MAGIC,
            format_version: SHARD_FORMAT_VERSION,
            zshrs_version: env!("CARGO_PKG_VERSION").to_string(),
            pointer_width: std::mem::size_of::<usize>() as u32,
            built_at_secs: now_secs() as u64,
        },
        entries: HashMap::new(),
    }
}

fn read_owned_shard(path: &Path) -> Option<ScriptShard> {
    let bytes = std::fs::read(path).ok()?;
    let archived = rkyv::check_archived_root::<ScriptShard>(&bytes[..]).ok()?;
    archived.deserialize(&mut rkyv::Infallible).ok()
}

fn write_shard_atomic(path: &Path, shard: &ScriptShard) -> Result<(), String> {
    let bytes = rkyv::to_bytes::<_, 4096>(shard).map_err(|e| format!("rkyv serialize: {}", e))?;
    // Shared with `autoload_cache`: same temp-then-rename scheme, and
    // the same obligation to unlink the temp on a failed write and to
    // reap temps abandoned by processes that died mid-write.
    crate::atomic_write::write_bytes_atomic(path, &bytes)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn format_local_ts(secs: i64) -> String {
    let dt = chrono::DateTime::<chrono::Local>::from(
        UNIX_EPOCH + std::time::Duration::from_secs(secs.max(0) as u64),
    );
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}
/// `file_mtime` — see implementation.
pub fn file_mtime(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.mtime(), meta.mtime_nsec()))
}

/// `(mtime, len)` of the running zshrs binary — the identity a cached chunk
/// is stamped with, mirroring `autoload_cache::current_binary_identity`.
fn current_binary_identity() -> Option<(i64, u64)> {
    static BIN_ID: OnceLock<Option<(i64, u64)>> = OnceLock::new();
    *BIN_ID.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let meta = std::fs::metadata(&exe).ok()?;
        Some((meta.mtime(), meta.len()))
    })
}

/// Default shard path: `~/.zshrs/scripts.rkyv`.
pub fn default_cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".zshrs/scripts.rkyv")
}

/// Process-local disable flag set by parity-mode flags (`--zsh` etc.)
/// in bins/zshrs.rs. Preferred over `ZSHRS_CACHE=0` in env so the
/// env var doesn't leak into `${(k)parameters}` and inflate the
/// param count vs reference zsh.
pub static CACHE_DISABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `ZSHRS_CACHE=0|false|no` (env) or `CACHE_DISABLED=true` (process-
/// local) disables the cache entirely.
pub fn cache_enabled() -> bool {
    if CACHE_DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    !matches!(
        std::env::var("ZSHRS_CACHE").as_deref(),
        Ok("0") | Ok("false") | Ok("no")
    )
}

/// Process-wide `ScriptCache` rooted at `default_cache_path()`. `None` when the
/// cache is disabled or the path could not be opened.
pub static CACHE: once_cell::sync::Lazy<Option<ScriptCache>> = once_cell::sync::Lazy::new(|| {
    if !cache_enabled() {
        return None;
    }
    ScriptCache::open(&default_cache_path()).ok()
});

/// Try to load cached chunk-bytes by source path. Returns `None` on any miss.
pub fn try_load_bytes(path: &Path) -> Option<Vec<u8>> {
    let cache = CACHE.as_ref()?;
    let canonical = path.canonicalize().ok()?;
    let path_str = canonical.to_string_lossy();
    let (mtime_s, mtime_ns) = file_mtime(&canonical)?;
    cache.get(&path_str, mtime_s, mtime_ns)
}

/// Store bincode-encoded `fusevm::Chunk` bytes for a script path. Best-effort —
/// cache disabled / canonicalize failure / mtime stat failure all return
/// `Ok(())` silently so the caller can fire-and-forget.
pub fn try_save_bytes(path: &Path, chunk_blob: &[u8]) -> Result<(), String> {
    let Some(cache) = CACHE.as_ref() else {
        return Ok(());
    };
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };
    let path_str = canonical.to_string_lossy();
    let (mtime_s, mtime_ns) = match file_mtime(&canonical) {
        Some(m) => m,
        None => return Ok(()),
    };
    cache.put(&path_str, mtime_s, mtime_ns, chunk_blob.to_vec())
}
/// `stats` — see implementation.
pub fn stats() -> Option<(i64, i64)> {
    CACHE.as_ref().map(|c| c.stats())
}
/// `evict_stale` — see implementation.
pub fn evict_stale() -> usize {
    CACHE.as_ref().map(|c| c.evict_stale()).unwrap_or(0)
}
/// `clear` — see implementation.
pub fn clear() -> bool {
    CACHE.as_ref().map(|c| c.clear().is_ok()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        let cache = ScriptCache::open(&cache_path).unwrap();

        let script_path = dir.path().join("test.zsh");
        std::fs::write(&script_path, "echo hi").unwrap();

        let (mtime_s, mtime_ns) = file_mtime(&script_path).unwrap();
        let path_str = script_path.to_string_lossy().to_string();

        let blob = vec![1u8, 2, 3, 4, 5];
        cache
            .put(&path_str, mtime_s, mtime_ns, blob.clone())
            .unwrap();

        let loaded = cache.get(&path_str, mtime_s, mtime_ns).unwrap();
        assert_eq!(loaded, blob);

        let (count, _bytes) = cache.stats();
        assert_eq!(count, 1);
    }

    #[test]
    fn mtime_invalidation() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        let cache = ScriptCache::open(&cache_path).unwrap();

        let script_path = dir.path().join("test.zsh");
        std::fs::write(&script_path, "echo hi").unwrap();

        let (mtime_s, mtime_ns) = file_mtime(&script_path).unwrap();
        let path_str = script_path.to_string_lossy().to_string();
        cache.put(&path_str, mtime_s, mtime_ns, vec![9u8]).unwrap();

        assert!(cache.get(&path_str, mtime_s + 1, mtime_ns).is_none());
    }

    /// Bytecode is not portable between builds, so an entry must be refused
    /// whichever way the recorded binary timestamp points. The old
    /// `cached < running` test only caught the "cache written by an older
    /// build" direction, which meant a binary whose mtime moved BACKWARDS
    /// (an earlier build restored over a later one, a `cp -p`, a checkout of
    /// a previously-built target dir) replayed the newer build's chunks.
    #[test]
    fn an_entry_from_another_binary_is_never_served() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        let cache = ScriptCache::open(&cache_path).unwrap();

        let script_path = dir.path().join("test.zsh");
        std::fs::write(&script_path, "echo hi").unwrap();
        let (mtime_s, mtime_ns) = file_mtime(&script_path).unwrap();
        let path_str = script_path.to_string_lossy().to_string();
        cache.put(&path_str, mtime_s, mtime_ns, vec![9u8]).unwrap();
        assert_eq!(
            cache.get(&path_str, mtime_s, mtime_ns),
            Some(vec![9u8]),
            "the emitting binary must hit its own entry",
        );

        // Restamp the entry as if a NEWER build had produced it — the
        // direction the old comparison let through.
        let mut shard = read_owned_shard(&cache_path).expect("shard readable");
        shard
            .entries
            .get_mut(&path_str)
            .expect("entry present")
            .binary_mtime_at_cache += 10_000;
        write_shard_atomic(&cache_path, &shard).unwrap();
        let reopened = ScriptCache::open(&cache_path).unwrap();
        assert!(
            reopened.get(&path_str, mtime_s, mtime_ns).is_none(),
            "a chunk from a newer build was accepted",
        );

        // Same length, same-second mtime, different build: only the length
        // term can reject this one.
        let mut shard = read_owned_shard(&cache_path).expect("shard readable");
        let entry = shard.entries.get_mut(&path_str).expect("entry present");
        entry.binary_mtime_at_cache -= 10_000;
        entry.binary_len_at_cache += 1;
        write_shard_atomic(&cache_path, &shard).unwrap();
        let reopened = ScriptCache::open(&cache_path).unwrap();
        assert!(
            reopened.get(&path_str, mtime_s, mtime_ns).is_none(),
            "a chunk from a same-second build of a different size was accepted",
        );
    }

    #[test]
    fn second_put_replaces_first() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        let cache = ScriptCache::open(&cache_path).unwrap();

        let p1 = dir.path().join("a.zsh");
        let p2 = dir.path().join("b.zsh");
        std::fs::write(&p1, "1").unwrap();
        std::fs::write(&p2, "2").unwrap();

        let (m1s, m1n) = file_mtime(&p1).unwrap();
        let (m2s, m2n) = file_mtime(&p2).unwrap();

        cache
            .put(&p1.to_string_lossy(), m1s, m1n, vec![1u8])
            .unwrap();
        cache
            .put(&p2.to_string_lossy(), m2s, m2n, vec![2u8])
            .unwrap();

        let (count, _) = cache.stats();
        assert_eq!(count, 2);
        assert!(cache.get(&p1.to_string_lossy(), m1s, m1n).is_some());
        assert!(cache.get(&p2.to_string_lossy(), m2s, m2n).is_some());
    }

    #[test]
    fn corrupt_file_returns_no_mmap() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        std::fs::write(&cache_path, b"this is not a valid rkyv archive").unwrap();
        let cache = ScriptCache::open(&cache_path).unwrap();
        assert!(cache.get("/nope", 0, 0).is_none());
    }

    #[test]
    fn clear_removes_file() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        let cache = ScriptCache::open(&cache_path).unwrap();

        let script_path = dir.path().join("test.zsh");
        std::fs::write(&script_path, "echo hi").unwrap();
        let (mtime_s, mtime_ns) = file_mtime(&script_path).unwrap();
        cache
            .put(&script_path.to_string_lossy(), mtime_s, mtime_ns, vec![7u8])
            .unwrap();
        assert!(cache_path.exists());

        cache.clear().unwrap();
        assert!(!cache_path.exists());
    }

    // ========================================================
    // now_secs — monotonic-ish wall-clock
    // ========================================================

    #[test]
    fn now_secs_is_positive_and_within_realistic_range() {
        let _g = crate::test_util::global_state_lock();
        let n = now_secs();
        // Year 2020 = ~1.58e9 seconds. Year 2100 = ~4.1e9 seconds.
        assert!(
            (1_577_836_800..4_102_444_800).contains(&n),
            "now_secs out of plausible range: {}",
            n
        );
    }

    #[test]
    fn now_secs_does_not_go_backwards_in_quick_succession() {
        let _g = crate::test_util::global_state_lock();
        let a = now_secs();
        let b = now_secs();
        assert!(b >= a, "now_secs went backwards: {} -> {}", a, b);
    }

    // ========================================================
    // format_local_ts — human-readable timestamp
    // ========================================================

    #[test]
    fn format_local_ts_includes_year_and_punctuation() {
        let _g = crate::test_util::global_state_lock();
        // 2024-01-01 = 1704067200 UTC; local timezone shifts it but
        // the year prefix is stable regardless of TZ.
        let s = format_local_ts(1_704_067_200);
        assert!(s.starts_with("202"), "expected 21st century year: {}", s);
        assert!(s.contains('-'), "expected dash separator: {}", s);
        assert!(s.contains(':'), "expected colon separator: {}", s);
    }

    #[test]
    fn format_local_ts_length_matches_pattern() {
        let _g = crate::test_util::global_state_lock();
        let s = format_local_ts(1_700_000_000);
        // `YYYY-MM-DD HH:MM:SS` = 19 chars.
        assert_eq!(s.len(), 19, "unexpected width: {}", s);
    }

    #[test]
    fn format_local_ts_handles_zero_secs_via_clamp() {
        let _g = crate::test_util::global_state_lock();
        // 0 → 1970-01-01 in UTC, but local TZ may shift the date.
        let s = format_local_ts(0);
        assert_eq!(s.len(), 19);
        assert!(s.starts_with("19"), "expected 1970-ish year: {}", s);
    }

    #[test]
    fn format_local_ts_negative_clamped_to_zero() {
        let _g = crate::test_util::global_state_lock();
        // .max(0) prevents negative seconds reaching chrono.
        let s = format_local_ts(-1_000_000);
        assert_eq!(s.len(), 19);
    }

    // ========================================================
    // file_mtime — pure metadata sniff
    // ========================================================

    #[test]
    fn file_mtime_returns_some_for_real_file() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let p = dir.path().join("foo.zsh");
        std::fs::write(&p, b"x").unwrap();
        let (s, _ns) = file_mtime(&p).unwrap();
        assert!(s > 0);
    }

    #[test]
    fn file_mtime_returns_none_for_missing_path() {
        let _g = crate::test_util::global_state_lock();
        assert!(file_mtime(Path::new("/nonexistent/zshrs/script_cache_missing.bin")).is_none());
    }

    // ========================================================
    // default_cache_path / cache_enabled — config knobs
    // ========================================================

    #[test]
    fn default_cache_path_ends_in_scripts_rkyv() {
        let _g = crate::test_util::global_state_lock();
        let p = default_cache_path();
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("scripts.rkyv"));
    }

    #[test]
    fn cache_enabled_true_when_env_unset() {
        let _g = crate::test_util::global_state_lock();
        let prev = std::env::var_os("ZSHRS_CACHE");
        std::env::remove_var("ZSHRS_CACHE");
        let on = cache_enabled();
        if let Some(v) = prev {
            std::env::set_var("ZSHRS_CACHE", v);
        }
        assert!(on, "cache should be enabled when ZSHRS_CACHE is unset");
    }

    #[test]
    fn cache_enabled_false_when_env_is_zero_false_or_no() {
        let _g = crate::test_util::global_state_lock();
        let prev = std::env::var_os("ZSHRS_CACHE");
        for v in ["0", "false", "no"] {
            std::env::set_var("ZSHRS_CACHE", v);
            assert!(!cache_enabled(), "ZSHRS_CACHE={} must disable cache", v);
        }
        if let Some(v) = prev {
            std::env::set_var("ZSHRS_CACHE", v);
        } else {
            std::env::remove_var("ZSHRS_CACHE");
        }
    }

    #[test]
    fn cache_enabled_true_for_other_env_values() {
        // Truthiness model: only `0|false|no` disable. Anything else
        // (including empty string) leaves the cache on.
        let _g = crate::test_util::global_state_lock();
        let prev = std::env::var_os("ZSHRS_CACHE");
        for v in ["1", "true", "yes", "on", ""] {
            std::env::set_var("ZSHRS_CACHE", v);
            assert!(
                cache_enabled(),
                "ZSHRS_CACHE={:?} must NOT disable cache",
                v
            );
        }
        if let Some(v) = prev {
            std::env::set_var("ZSHRS_CACHE", v);
        } else {
            std::env::remove_var("ZSHRS_CACHE");
        }
    }
}
