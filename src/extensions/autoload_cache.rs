//! rkyv-backed bytecode cache for autoload functions.
//!
//! Single-file shard at `~/.zshrs/autoloads.rkyv`. Keyed by function name
//! (not file path) — autoload bytecode is identified by the resolved function
//! name, regardless of which fpath dir or .zwc archive it came from.
//!
//! Storage layout (rkyv archived):
//!   AutoloadShard {
//!     header: { magic, format_version, zshrs_version, pointer_width, built_at_secs },
//!     entries: HashMap<function_name, AutoloadEntry>,
//!   }
//!   AutoloadEntry { binary_mtime_at_cache, cached_at_secs, chunk_blob: `Vec<u8>` }
//!
//! Inner `chunk_blob` is bincode-encoded `fusevm::Chunk` (same constraint as
//! [`script_cache`](crate::script_cache) module — `fusevm::Chunk` is upstream and only derives serde).
//!
//! Invalidation:
//!   - zshrs binary mtime newer than `binary_mtime_at_cache` ⇒ entry stale
//!     (any zshrs rebuild silently invalidates the whole shard).
//!   - There is no per-source-file mtime check here. Autoload bodies live in
//!     fpath dirs / .zwc archives and the existing `crate::compsys::cache::autoloads`
//!     SQLite row tracks the source file/offset/size. Rebuild logic relies on
//!     `compinit` clearing the whole rkyv shard at recompute time (see
//!     `AutoloadShardWriter` — used by the compinit bulk-prewarm path).
//!
//! Bulk-write: compinit prewarms 16k+ autoload bytecodes in one go. Per-batch
//! shard rewrites (the SQLite-era pattern) would re-serialize 16k entries
//! 160 times. Instead the new API exposes `AutoloadShardWriter`: accumulate
//! all `(name, blob)` pairs in memory, then `commit()` writes the shard once.
//! The single-add `try_save_one` path remains for the cold-start case where
//! one autoload at a time is compiled by the interactive shell.
//!
//! The on-disk shape mirrors [`ScriptShard`](crate::script_cache::ScriptShard) — same header,
//! same magic-version-pointer_width discipline, same atomic-rename writes.

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

/// "ZRAL" little-endian.
pub const SHARD_MAGIC: u32 = 0x5A52414C;
/// `SHARD_FORMAT_VERSION` constant.
pub const SHARD_FORMAT_VERSION: u32 = 1;
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
/// `AutoloadEntry` — see fields for layout.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct AutoloadEntry {
    /// `binary_mtime_at_cache` field.
    pub binary_mtime_at_cache: i64,
    /// `cached_at_secs` field.
    pub cached_at_secs: i64,
    /// `chunk_blob` field.
    pub chunk_blob: Vec<u8>,
}
/// `AutoloadShard` — see fields for layout.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct AutoloadShard {
    /// `header` field.
    pub header: ShardHeader,
    /// `entries` field.
    pub entries: HashMap<String, AutoloadEntry>,
}
/// `MmappedShard` — see fields for layout.
pub struct MmappedShard {
    /// `_mmap` field.
    _mmap: Mmap,
    /// `archived` field.
    archived: *const ArchivedAutoloadShard,
}

unsafe impl Send for MmappedShard {}
unsafe impl Sync for MmappedShard {}

impl MmappedShard {
    /// `open` — see implementation.
    pub fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let mmap = unsafe { Mmap::map(&file).ok()? };
        let archived = rkyv::check_archived_root::<AutoloadShard>(&mmap[..]).ok()?;
        let archived_ptr = archived as *const ArchivedAutoloadShard;
        Some(Self {
            _mmap: mmap,
            archived: archived_ptr,
        })
    }

    fn shard(&self) -> &ArchivedAutoloadShard {
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

    fn lookup(&self, name: &str) -> Option<&ArchivedAutoloadEntry> {
        self.shard().entries.get(name)
    }
}
/// `AutoloadCache` — see fields for layout.
pub struct AutoloadCache {
    /// `path` field.
    path: PathBuf,
    /// `lock_path` field.
    lock_path: PathBuf,
    /// `mmap` field.
    mmap: Mutex<Option<MmappedShard>>,
}

impl AutoloadCache {
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
                .unwrap_or("autoloads.rkyv")
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
    /// `get` — see implementation.
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let shard = guard.as_ref()?;
        if !shard.header_ok() {
            return None;
        }
        let entry = shard.lookup(name)?;
        if let Some(bin_mtime) = current_binary_mtime_secs() {
            let cached_bin_mtime: i64 = entry.binary_mtime_at_cache.into();
            if cached_bin_mtime < bin_mtime {
                return None;
            }
        }
        Some(entry.chunk_blob.as_slice().to_vec())
    }

    /// Single-write: read shard, insert one entry, write shard. Used by the
    /// cold-start path when a function is autoloaded before compinit
    /// pre-warm completes.
    pub fn put_one(&self, name: &str, chunk_blob: Vec<u8>) -> Result<(), String> {
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
        let bin_mtime = current_binary_mtime_secs().unwrap_or(0);
        shard.entries.insert(
            name.to_string(),
            AutoloadEntry {
                binary_mtime_at_cache: bin_mtime,
                cached_at_secs: now_secs(),
                chunk_blob,
            },
        );
        shard.header.built_at_secs = now_secs() as u64;
        write_shard_atomic(&self.path, &shard)?;
        self.invalidate_mmap();
        Ok(())
    }

    /// Merge `entries` into the existing shard, inserting/replacing each one.
    /// Used by compinit's BACKFILL path — when an existing shard is missing
    /// some entries (e.g. binary mtime bump invalidated a subset), the
    /// caller computes the missing names + chunks and merges them in
    /// without touching unrelated entries. Single read + single write,
    /// even for 16k entries.
    pub fn merge_in(&self, entries: HashMap<String, Vec<u8>>) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
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
        let bin_mtime = current_binary_mtime_secs().unwrap_or(0);
        let now = now_secs();
        for (name, chunk_blob) in entries {
            shard.entries.insert(
                name,
                AutoloadEntry {
                    binary_mtime_at_cache: bin_mtime,
                    cached_at_secs: now,
                    chunk_blob,
                },
            );
        }
        shard.header.built_at_secs = now as u64;
        write_shard_atomic(&self.path, &shard)?;
        self.invalidate_mmap();
        Ok(())
    }

    /// Replace the entire shard with the given entries. Used by compinit's
    /// bulk pre-warm — accumulate all (name, chunk_blob) pairs, then commit
    /// once. Avoids re-serializing 16k entries on every batch flush.
    pub fn replace_all(&self, entries: HashMap<String, Vec<u8>>) -> Result<(), String> {
        let _lock = match acquire_lock(&self.lock_path) {
            Some(l) => l,
            None => return Ok(()),
        };
        let bin_mtime = current_binary_mtime_secs().unwrap_or(0);
        let now = now_secs();
        let mut shard = fresh_shard();
        for (name, chunk_blob) in entries {
            shard.entries.insert(
                name,
                AutoloadEntry {
                    binary_mtime_at_cache: bin_mtime,
                    cached_at_secs: now,
                    chunk_blob,
                },
            );
        }
        write_shard_atomic(&self.path, &shard)?;
        self.invalidate_mmap();
        Ok(())
    }
    /// `entry_count` — see implementation.
    pub fn entry_count(&self) -> usize {
        self.ensure_mmap();
        let guard = self.mmap.lock();
        guard.as_ref().map(|s| s.shard().entries.len()).unwrap_or(0)
    }

    /// Set of cached function names — caller can subtract this from "all
    /// known autoload names" to compute the missing-bytecode set without a
    /// SQL JOIN.
    pub fn cached_names(&self) -> std::collections::HashSet<String> {
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let Some(shard) = guard.as_ref() else {
            return std::collections::HashSet::new();
        };
        shard
            .shard()
            .entries
            .keys()
            .map(|k| k.as_str().to_string())
            .collect()
    }
    /// `stats` — see implementation.
    pub fn stats(&self) -> (i64, i64) {
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let Some(shard) = guard.as_ref() else {
            return (0, 0);
        };
        let count = shard.shard().entries.len() as i64;
        let bytes: i64 = shard
            .shard()
            .entries
            .values()
            .map(|e| e.chunk_blob.len() as i64)
            .sum();
        (count, bytes)
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

fn fresh_shard() -> AutoloadShard {
    AutoloadShard {
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

fn read_owned_shard(path: &Path) -> Option<AutoloadShard> {
    let bytes = std::fs::read(path).ok()?;
    let archived = rkyv::check_archived_root::<AutoloadShard>(&bytes[..]).ok()?;
    archived.deserialize(&mut rkyv::Infallible).ok()
}

fn write_shard_atomic(path: &Path, shard: &AutoloadShard) -> Result<(), String> {
    let bytes = rkyv::to_bytes::<_, 4096>(shard).map_err(|e| format!("rkyv serialize: {}", e))?;
    let parent = path.parent().expect("cache path has parent");
    let _ = std::fs::create_dir_all(parent);
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(
        "{}.tmp.{}.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("autoloads.rkyv"),
        pid,
        nanos
    ));
    {
        let mut f = File::create(&tmp_path).map_err(|e| e.to_string())?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp_path, path).map_err(|e| e.to_string())?;
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn file_mtime(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.mtime(), meta.mtime_nsec()))
}

fn current_binary_mtime_secs() -> Option<i64> {
    static BIN_MTIME: OnceLock<Option<i64>> = OnceLock::new();
    *BIN_MTIME.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let (secs, _) = file_mtime(&exe)?;
        Some(secs)
    })
}
/// `default_cache_path` — see implementation.
///
/// All zshrs state lives under `$ZSHRS_HOME` (default `~/.zshrs`),
/// matching the daemon's `CachePaths` convention (daemon/paths.rs)
/// and the `~/.zinit`/`~/.zpwr`/`~/.oh-my-zsh` precedent. Project
/// policy forbids `~/.cache/zshrs/` and `~/Library/Caches/zshrs/`
/// — both of which `dirs::cache_dir()` resolves to.
pub fn default_cache_path() -> PathBuf {
    let root = if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        PathBuf::from(custom)
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".zshrs")
    };
    root.join("autoloads.rkyv")
}
/// `cache_enabled` — see implementation. Honors the process-local
/// `script_cache::CACHE_DISABLED` AtomicBool first so parity-mode
/// init can disable caches without exporting `ZSHRS_CACHE=0` (which
/// would otherwise leak into `${(k)parameters}`).
pub fn cache_enabled() -> bool {
    if crate::extensions::script_cache::CACHE_DISABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    !matches!(
        std::env::var("ZSHRS_CACHE").as_deref(),
        Ok("0") | Ok("false") | Ok("no")
    )
}
/// `CACHE` static.
pub static CACHE: once_cell::sync::Lazy<Option<AutoloadCache>> = once_cell::sync::Lazy::new(|| {
    if !cache_enabled() {
        return None;
    }
    AutoloadCache::open(&default_cache_path()).ok()
});
/// `try_load` — see implementation.
pub fn try_load(name: &str) -> Option<Vec<u8>> {
    let cache = CACHE.as_ref()?;
    cache.get(name)
}
/// `try_save_one` — see implementation.
pub fn try_save_one(name: &str, chunk_blob: &[u8]) -> Result<(), String> {
    let Some(cache) = CACHE.as_ref() else {
        return Ok(());
    };
    cache.put_one(name, chunk_blob.to_vec())
}

/// Replace the entire autoload shard with the given entries. Use this from
/// compinit's bulk pre-warm path — accumulates all `(name, chunk_blob)` in
/// the `entries` HashMap and writes the shard exactly once.
pub fn try_replace_all(entries: HashMap<String, Vec<u8>>) -> Result<(), String> {
    let Some(cache) = CACHE.as_ref() else {
        return Ok(());
    };
    cache.replace_all(entries)
}

/// Merge new entries into the existing shard. Use this from the compinit
/// BACKFILL path (existing shard has most entries, just adding the missing
/// ones).
pub fn try_merge_in(entries: HashMap<String, Vec<u8>>) -> Result<(), String> {
    let Some(cache) = CACHE.as_ref() else {
        return Ok(());
    };
    cache.merge_in(entries)
}
/// `cached_names` — see implementation.
pub fn cached_names() -> std::collections::HashSet<String> {
    CACHE.as_ref().map(|c| c.cached_names()).unwrap_or_default()
}
/// `entry_count` — see implementation.
pub fn entry_count() -> usize {
    CACHE.as_ref().map(|c| c.entry_count()).unwrap_or(0)
}
/// `stats` — see implementation.
pub fn stats() -> Option<(i64, i64)> {
    CACHE.as_ref().map(|c| c.stats())
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
    fn round_trip_one() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        cache.put_one("foo", vec![1, 2, 3]).unwrap();
        assert_eq!(cache.get("foo"), Some(vec![1, 2, 3]));
        assert_eq!(cache.entry_count(), 1);
    }

    #[test]
    fn replace_all_overwrites() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        cache.put_one("a", vec![10]).unwrap();
        cache.put_one("b", vec![20]).unwrap();
        assert_eq!(cache.entry_count(), 2);

        let mut new_entries = HashMap::new();
        new_entries.insert("c".to_string(), vec![30]);
        new_entries.insert("d".to_string(), vec![40]);
        cache.replace_all(new_entries).unwrap();

        assert_eq!(cache.entry_count(), 2);
        assert!(cache.get("a").is_none());
        assert!(cache.get("b").is_none());
        assert_eq!(cache.get("c"), Some(vec![30]));
        assert_eq!(cache.get("d"), Some(vec![40]));
    }

    #[test]
    fn cached_names_returns_keys() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        cache.put_one("alpha", vec![1]).unwrap();
        cache.put_one("beta", vec![2]).unwrap();
        let names = cache.cached_names();
        assert!(names.contains("alpha"));
        assert!(names.contains("beta"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn corrupt_shard_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        std::fs::write(&cache_path, b"garbage").unwrap();
        let cache = AutoloadCache::open(&cache_path).unwrap();
        assert!(cache.get("anything").is_none());
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn clear_removes_file() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        cache.put_one("x", vec![1]).unwrap();
        assert!(cache_path.exists());
        cache.clear().unwrap();
        assert!(!cache_path.exists());
    }
}
