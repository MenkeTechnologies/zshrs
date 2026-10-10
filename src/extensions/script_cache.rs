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
/// v3: token chars moved to the Private Use Area (crate::token_char).
/// v5: `ScriptEntry::env_fingerprint`, and a one-byte kind tag in front of
/// every `chunk_blob` (a script's single chunk vs a sourced file's events).
pub const SHARD_FORMAT_VERSION: u32 = 5;
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
    /// Hash of the lexer-visible shell state (aliases, options, emulation) the
    /// program was lexed and compiled under. The compiled chunks have alias
    /// expansions and option-dependent code baked in, so an entry is served
    /// only to a run that starts from the same state.
    pub env_fingerprint: u64,
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
    /// `(inode, len, mtime_ns)` of the file this mapping was taken from. The
    /// shard is replaced by atomic rename, so another process's write leaves
    /// this mapping on the OLD inode; comparing against the path's current
    /// identity is how a long-lived shell notices.
    identity: (u64, u64, i64),
}

// SAFETY: the pointer aliases an immutable mmap that lives as long as Self.
// rkyv-validated reads are immutable.
unsafe impl Send for MmappedShard {}
unsafe impl Sync for MmappedShard {}

impl MmappedShard {
    /// `open` — see implementation.
    pub fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let identity = file_identity(&file.metadata().ok()?);
        let mmap = unsafe { Mmap::map(&file).ok()? };
        let archived = rkyv::check_archived_root::<ScriptShard>(&mmap[..]).ok()?;
        let archived_ptr = archived as *const ArchivedScriptShard;
        Some(Self {
            _mmap: mmap,
            archived: archived_ptr,
            identity,
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

/// One buffered write: what `try_save_bytes` queues until the next flush.
#[derive(Clone)]
pub struct PendingPut {
    path: String,
    mtime_secs: i64,
    mtime_nsecs: i64,
    env_fingerprint: u64,
    /// Kind-tagged blob, as stored.
    blob: Vec<u8>,
}

/// Writes buffered since the last flush; see [`try_flush_pending`].
static PENDING: Mutex<Vec<PendingPut>> = Mutex::new(Vec::new());

/// Past this many buffered entries `try_save_bytes` flushes by itself, so a
/// long non-interactive run cannot hold an unbounded amount of bytecode.
const PENDING_FLUSH_MAX: usize = 128;

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
        // One `stat`: remap when another process replaced the shard (or it
        // vanished) since this mapping was taken. `put` and `clear` in THIS
        // process invalidate directly; a write from a script run elsewhere
        // only shows up here.
        let on_disk = std::fs::metadata(&self.path).ok().map(|m| file_identity(&m));
        let stale = match (guard.as_ref(), on_disk) {
            (Some(m), Some(id)) => m.identity != id,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if stale {
            *guard = None;
        }
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
    pub fn get(
        &self,
        path: &str,
        mtime_secs: i64,
        mtime_nsecs: i64,
        env_fingerprint: u64,
    ) -> Option<Vec<u8>> {
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
        let entry_fp: u64 = entry.env_fingerprint.into();
        if entry_fp != env_fingerprint {
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
        env_fingerprint: u64,
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
            env_fingerprint,
            chunk_blob,
        };
        shard.entries.insert(path.to_string(), entry);
        shard.header.built_at_secs = now_secs() as u64;

        write_shard_atomic(&self.path, &shard)?;
        self.invalidate_mmap();
        Ok(())
    }

    /// Insert / replace several entries with ONE lock, read and rewrite of the
    /// shard. `put` costs a full shard rewrite and fsync per call, so a shell
    /// that sources a hundred files for the first time would pay that a hundred
    /// times; the sourced-file path buffers and flushes here instead.
    pub fn put_many(&self, batch: Vec<PendingPut>) -> Result<(), String> {
        if batch.is_empty() {
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
        let (bin_mtime, bin_len) = current_binary_identity().unwrap_or((0, 0));
        for p in batch {
            shard.entries.insert(
                p.path,
                ScriptEntry {
                    mtime_secs: p.mtime_secs,
                    mtime_nsecs: p.mtime_nsecs,
                    binary_mtime_at_cache: bin_mtime,
                    binary_len_at_cache: bin_len,
                    cached_at_secs: now_secs(),
                    env_fingerprint: p.env_fingerprint,
                    chunk_blob: p.blob,
                },
            );
        }
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

/// `(inode, len, mtime_ns)` — what distinguishes one atomically-renamed shard
/// from the next.
fn file_identity(m: &std::fs::Metadata) -> (u64, u64, i64) {
    (m.ino(), m.len(), m.mtime() * 1_000_000_000 + m.mtime_nsec())
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

/// Default shard path: `$ZSHRS_HOME/scripts.rkyv` (default
/// `~/.zshrs/scripts.rkyv`).
///
/// This was the one cache that ignored `$ZSHRS_HOME`. Every other
/// store honours it — `autoload_cache::default_cache_path`,
/// `compsys::cache::default_cache_path`, the daemon's `CachePaths`
/// (daemon/paths.rs) — so a test or a session pointed at an isolated
/// home still read and WROTE the real `~/.zshrs/scripts.rkyv`,
/// which is both a leak out of the isolation and a writer the
/// isolated run never accounted for.
pub fn default_cache_path() -> PathBuf {
    let root = if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        PathBuf::from(custom)
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".zshrs")
    };
    root.join("scripts.rkyv")
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

/// What a cached blob holds. A top-level script is one chunk; a sourced file is
/// the sequence of per-event chunks the `loop()` of c:Src/init.c:155-220
/// compiled, each lexed under the state the previous ones left behind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlobKind {
    /// bincode of one `fusevm::Chunk`.
    Script = 0,
    /// bincode of a `Vec<fusevm::Chunk>`, one per executed event.
    Events = 1,
}

/// Hash of everything the lexer and compiler read from the live shell: the
/// alias tables, the option store and the emulation mode. Two runs that start
/// from equal fingerprints lex the same bytes into the same chunks.
///
/// The alias tables are large and change rarely, so their hash is reused until
/// `ALIAS_GEN` moves; the option store is a few hundred bytes and is hashed
/// whenever `opts_cache::generation` moved since the last call.
pub fn env_fingerprint() -> u64 {
    use std::sync::atomic::Ordering;
    static MEMO: Mutex<Option<(u64, u64, u64, u64)>> = Mutex::new(None);
    let alias_gen = crate::ported::hashtable::ALIAS_GEN.load(Ordering::Relaxed);
    let opt_gen = crate::opts_cache::generation();
    let mut memo = MEMO.lock();
    if let Some((ag, og, alias_hash, opt_hash)) = *memo {
        if ag == alias_gen && og == opt_gen {
            return fnv_mix(alias_hash, opt_hash);
        }
    }
    let alias_hash = hash_alias_tables();
    let opt_hash = hash_option_store();
    *memo = Some((alias_gen, opt_gen, alias_hash, opt_hash));
    fnv_mix(alias_hash, opt_hash)
}

/// `(ALIAS_GEN, option generation)` — compared before and after a run to learn
/// whether the run itself changed lexer-visible state.
pub fn env_generation() -> (u64, u64) {
    (
        crate::ported::hashtable::ALIAS_GEN.load(std::sync::atomic::Ordering::Relaxed),
        crate::opts_cache::generation(),
    )
}

fn fnv_mix(a: u64, b: u64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in a.to_le_bytes().into_iter().chain(b.to_le_bytes()) {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn fnv_bytes(mut h: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Field separator, so ("ab","c") and ("a","bc") hash apart.
    h ^= 0xff;
    h.wrapping_mul(0x0000_0100_0000_01b3)
}

fn hash_alias_tables() -> u64 {
    use crate::ported::hashtable::{aliastab_lock, sufaliastab_lock};
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for table in [aliastab_lock(), sufaliastab_lock()] {
        let Ok(t) = table.read() else { continue };
        // Bucket order is deterministic for a given insertion history, but the
        // fingerprint must not depend on history — sort by name.
        let mut rows: Vec<(&String, &crate::ported::zsh_h::alias)> = t.iter().collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        for (name, a) in rows {
            h = fnv_bytes(h, name.as_bytes());
            h = fnv_bytes(h, a.text.as_bytes());
            h = fnv_bytes(h, &a.node.flags.to_le_bytes());
        }
        h = fnv_bytes(h, b"|");
    }
    h
}

fn hash_option_store() -> u64 {
    let mut rows: Vec<(String, bool)> = crate::ported::options::opt_state_snapshot()
        .into_iter()
        .collect();
    rows.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (name, on) in rows {
        h = fnv_bytes(h, name.as_bytes());
        h = fnv_bytes(h, &[on as u8]);
    }
    let emul = crate::ported::options::emulation.load(std::sync::atomic::Ordering::Relaxed);
    fnv_bytes(h, &emul.to_le_bytes())
}

/// Try to load a cached program by source path. Returns `None` on any miss,
/// including a different lexer-state fingerprint or a blob of another kind.
pub fn try_load_bytes(path: &Path, kind: BlobKind, env_fp: u64) -> Option<Vec<u8>> {
    let cache = CACHE.as_ref()?;
    let canonical = path.canonicalize().ok()?;
    let path_str = canonical.to_string_lossy();
    let (mtime_s, mtime_ns) = file_mtime(&canonical)?;
    // A file sourced twice in one session: the first run's entry is still
    // buffered, and it is the newest word on the path.
    let buffered = PENDING.lock().iter().rev().find_map(|p| {
        (p.path == path_str && p.mtime_secs == mtime_s && p.mtime_nsecs == mtime_ns && p.env_fingerprint == env_fp)
            .then(|| p.blob.clone())
    });
    let blob = match buffered {
        Some(b) => b,
        None => cache.get(&path_str, mtime_s, mtime_ns, env_fp)?,
    };
    let (tag, body) = blob.split_first()?;
    (*tag == kind as u8).then(|| body.to_vec())
}

/// Store the bincode-encoded program for a script path. `env_fp` is the
/// fingerprint taken BEFORE the run: the state the lexer started from, not the
/// state the file left behind. Best-effort — cache disabled / canonicalize
/// failure / mtime stat failure all return `Ok(())` silently so the caller can
/// fire-and-forget.
pub fn try_save_bytes(
    path: &Path,
    kind: BlobKind,
    env_fp: u64,
    chunk_blob: &[u8],
) -> Result<(), String> {
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
    let mut tagged = Vec::with_capacity(chunk_blob.len() + 1);
    tagged.push(kind as u8);
    tagged.extend_from_slice(chunk_blob);
    let over = {
        let mut pending = PENDING.lock();
        pending.push(PendingPut {
            path: path_str.into_owned(),
            mtime_secs: mtime_s,
            mtime_nsecs: mtime_ns,
            env_fingerprint: env_fp,
            blob: tagged,
        });
        pending.len() >= PENDING_FLUSH_MAX
    };
    if over {
        try_flush_pending();
    }
    Ok(())
}

/// Write out everything `try_save_bytes` buffered, in one shard rewrite. Called
/// at the prompt, from `zexit` and from the `atexit` hook, alongside the
/// autoload and deparse caches. Cheap when nothing is buffered.
pub fn try_flush_pending() {
    let batch = std::mem::take(&mut *PENDING.lock());
    if batch.is_empty() {
        return;
    }
    if let Some(cache) = CACHE.as_ref() {
        let _ = cache.put_many(batch);
    }
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
            .put(&path_str, mtime_s, mtime_ns, 0, blob.clone())
            .unwrap();

        let loaded = cache.get(&path_str, mtime_s, mtime_ns, 0).unwrap();
        assert_eq!(loaded, blob);

        let (count, _bytes) = cache.stats();
        assert_eq!(count, 1);
    }

    /// `dbview scripts` in a long-lived shell reads through a mapping taken
    /// earlier. A script run by ANOTHER process replaces the shard by rename,
    /// which this process's mapping never sees on its own: the listing froze
    /// at whatever the shard held when it was first read.
    #[test]
    fn a_write_from_another_process_shows_up_in_a_mapped_reader() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        let reader = ScriptCache::open(&cache_path).unwrap();
        let writer = ScriptCache::open(&cache_path).unwrap();

        let first = dir.path().join("first.zsh");
        let second = dir.path().join("second.zsh");
        std::fs::write(&first, "echo 1").unwrap();
        std::fs::write(&second, "echo 2").unwrap();
        let (s1, n1) = file_mtime(&first).unwrap();
        let (s2, n2) = file_mtime(&second).unwrap();

        writer.put(&first.to_string_lossy(), s1, n1, 0, vec![1]).unwrap();
        assert_eq!(reader.list_scripts().len(), 1, "reader maps the shard");

        writer.put(&second.to_string_lossy(), s2, n2, 0, vec![2]).unwrap();
        let rows = reader.list_scripts();
        assert_eq!(rows.len(), 2, "the replaced shard must be remapped: {rows:?}");
        assert_eq!(reader.stats().0, 2);
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
        cache.put(&path_str, mtime_s, mtime_ns, 0, vec![9u8]).unwrap();

        assert!(cache.get(&path_str, mtime_s + 1, mtime_ns, 0).is_none());
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
        cache.put(&path_str, mtime_s, mtime_ns, 0, vec![9u8]).unwrap();
        assert_eq!(
            cache.get(&path_str, mtime_s, mtime_ns, 0),
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
            reopened.get(&path_str, mtime_s, mtime_ns, 0).is_none(),
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
            reopened.get(&path_str, mtime_s, mtime_ns, 0).is_none(),
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
            .put(&p1.to_string_lossy(), m1s, m1n, 0, vec![1u8])
            .unwrap();
        cache
            .put(&p2.to_string_lossy(), m2s, m2n, 0, vec![2u8])
            .unwrap();

        let (count, _) = cache.stats();
        assert_eq!(count, 2);
        assert!(cache.get(&p1.to_string_lossy(), m1s, m1n, 0).is_some());
        assert!(cache.get(&p2.to_string_lossy(), m2s, m2n, 0).is_some());
    }

    #[test]
    fn corrupt_file_returns_no_mmap() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("scripts.rkyv");
        std::fs::write(&cache_path, b"this is not a valid rkyv archive").unwrap();
        let cache = ScriptCache::open(&cache_path).unwrap();
        assert!(cache.get("/nope", 0, 0, 0).is_none());
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
            .put(&script_path.to_string_lossy(), mtime_s, mtime_ns, 0, vec![7u8])
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
