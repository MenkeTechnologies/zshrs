//! rkyv-backed bytecode cache for autoload functions.
//!
//! Single-file shard at `~/.zshrs/autoloads.rkyv`, keyed by function
//! name. **zshrs-original — no C counterpart**, so the design rule here
//! is correctness first: an entry that cannot be PROVEN to describe the
//! body about to be installed is a miss.
//!
//! Storage layout (rkyv archived):
//!   AutoloadShard {
//!     header: { magic, format_version, zshrs_version, pointer_width, built_at_secs },
//!     entries: HashMap<function_name, AutoloadEntry>,
//!   }
//!
//! Inner `chunk_blob` is bincode-encoded `fusevm::Chunk` (same constraint as
//! [`script_cache`](crate::script_cache) module — `fusevm::Chunk` is upstream and only derives serde).
//!
//! # What identifies an entry
//!
//! Three things have to match before a chunk may be run, because a chunk
//! is a function of all three:
//!
//!   * **the producing binary**, as `(mtime, len)` of `current_exe()`.
//!     Bytecode is not a stable interchange format: the builtin index
//!     table and the opcode lowering both move between builds, so a
//!     chunk emitted by a different `zshrs` is meaningless here even
//!     when `zshrs_version` matches. It routinely does match — a debug
//!     build and the installed release build share `0.12.36` — which is
//!     why the version string cannot carry this check.
//!   * **the resolved fpath directory**, because the same function name
//!     lives in several directories on a real `$fpath` and the cache is
//!     keyed by name alone.
//!   * **the exact definition text**, as a SHA-256 of the very string
//!     that will be compiled. Not a `stat` of `<dir>/<name>`: the body
//!     the loader installs can come out of a `dir.zwc` digest instead of
//!     that file, and stamping a path that is not the source is how a
//!     chunk built from one text gets served for another.
//!
//! The previous scheme stamped `(mtime, len)` of `<dir>/<name>` and
//! treated the binary check as one-directional (`cached < current` =
//! stale), so a chunk written by a NEWER binary was served to an older
//! one. On the corpus this was built for that meant `_megacomplete`
//! being "installed" from a chunk that never defined it, and every
//! `<TAB>` producing `_megacomplete: function not defined by file` and
//! zero matches.
//!
//! Bulk-write: compinit prewarms 16k+ autoload bytecodes in one go. Per-batch
//! shard rewrites (the SQLite-era pattern) would re-serialize 16k entries
//! 160 times. Instead `put_many` accumulates all entries in memory and
//! writes the shard once. The single-add `put_one` path remains for the
//! cold-start case where one autoload at a time is compiled by the
//! interactive shell.
//!
//! The on-disk shape mirrors [`ScriptShard`](crate::script_cache::ScriptShard) — same header,
//! same magic-version-pointer_width discipline, same atomic-rename writes.

use std::collections::HashMap;
use std::fs::File;
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
///
/// v2 stamped an entry with `(mtime, len)` of `<loaddir>/<name>` and
/// accepted any chunk not strictly older than the running binary. Both
/// tests could pass for a chunk compiled from different text by a
/// different build, so v2 entries are not trustworthy and this bump
/// discards them. v3 stamps the resolved directory, a SHA-256 of the
/// exact definition text, and the producing binary's identity.
pub const SHARD_FORMAT_VERSION: u32 = 3;
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
    /// mtime of the `zshrs` binary that emitted `chunk_blob`.
    pub binary_mtime_at_cache: i64,
    /// Byte length of that binary. Paired with the mtime because two
    /// different builds can land in the same second, and a debug build
    /// and a release build differ enormously in size.
    pub binary_len_at_cache: u64,
    /// `cached_at_secs` field.
    pub cached_at_secs: i64,
    /// The fpath directory the definition was resolved from. The cache
    /// is keyed by function NAME, and a real `$fpath` has the same name
    /// in several directories, so the winner has to be recorded.
    pub source_dir: String,
    /// SHA-256 of the exact definition text that produced `chunk_blob`
    /// — `name() { <body> }` as the loader builds it. Hashing the text
    /// rather than stat-ing a file is what makes a `.zwc`-digest body
    /// and a plain-file body distinguishable.
    pub source_sha: [u8; 32],
    /// bincode of the `fusevm::Chunk` for the definition program.
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

/// Was this entry emitted by the binary that is running right now?
///
/// EXACT equality, not "not older". A chunk from any other build is
/// unusable whichever direction the timestamps point, and the older
/// `<` test was what let a newer build's bytecode be executed by an
/// older one.
fn entry_binary_matches(entry: &ArchivedAutoloadEntry) -> bool {
    let Some((mtime, len)) = current_binary_identity() else {
        // No `current_exe()` — nothing can be proven, so nothing is used.
        return false;
    };
    let cached_mtime: i64 = entry.binary_mtime_at_cache.into();
    let cached_len: u64 = entry.binary_len_at_cache.into();
    cached_mtime == mtime && cached_len == len
}

/// `AutoloadCache` — see fields for layout.
pub struct AutoloadCache {
    /// `path` field.
    path: PathBuf,
    /// `lock_path` field.
    lock_path: PathBuf,
    /// `mmap` field.
    mmap: Mutex<Option<MmappedShard>>,
    /// Cold-start writes not yet folded into the shard on disk.
    ///
    /// `put_one` is called once per autoload compile. Folding each one
    /// in immediately costs a full read + `bytecheck` + deserialize +
    /// re-serialize + write of the WHOLE file, so a single completion
    /// run that autoloads N helpers was O(N x shard). On a 46k-completer
    /// `$fpath` the shard reaches 40 MB and one `<TAB>` autoloads dozens
    /// of `_*` helpers, which is how a keypress came to cost 30+ s.
    /// Entries accumulate here and leave in ONE `put_many`.
    pending: Mutex<Vec<(String, Vec<u8>, String, [u8; 32])>>,
}

/// Cap on un-flushed [`AutoloadCache::pending`] entries.
///
/// A MEMORY bound, not a latency knob: the flush that matters is the one
/// at the next prompt (or at exit), and that is where the batching win
/// comes from. This only stops a script that autoloads without ever
/// reaching a prompt from growing the buffer without limit.
const PENDING_FLUSH_MAX: usize = 256;

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
            pending: Mutex::new(Vec::new()),
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
    /// Raw probe: the chunk for `name` with only the producing-binary
    /// check applied. `dbview autoloads <name>` uses it to report
    /// whether an entry exists. NOT for execution — use
    /// [`AutoloadCache::get_for_source`], which also proves the entry
    /// describes the definition text about to be installed.
    pub fn get(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(blob) = self.pending_lookup(name, None, None) {
            return Some(blob);
        }
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let shard = guard.as_ref()?;
        if !shard.header_ok() {
            return None;
        }
        let entry = shard.lookup(name)?;
        if !entry_binary_matches(entry) {
            return None;
        }
        Some(entry.chunk_blob.as_slice().to_vec())
    }

    /// The chunk for `name`, but only if this exact binary compiled it
    /// from this exact definition text found in this exact directory.
    /// Anything else — an edited file, a `.zwc` digest body, a chunk
    /// from another build, no entry at all — is a miss, and a miss just
    /// means "parse it yourself".
    pub fn get_for_source(
        &self,
        name: &str,
        source_dir: &str,
        source_sha: &[u8; 32],
    ) -> Option<Vec<u8>> {
        if let Some(blob) = self.pending_lookup(name, Some(source_dir), Some(source_sha)) {
            return Some(blob);
        }
        self.ensure_mmap();
        let guard = self.mmap.lock();
        let shard = guard.as_ref()?;
        if !shard.header_ok() {
            return None;
        }
        let entry = shard.lookup(name)?;
        if !entry_binary_matches(entry) {
            return None;
        }
        if entry.source_dir.as_str() != source_dir {
            return None;
        }
        if entry.source_sha.as_slice() != source_sha.as_slice() {
            return None;
        }
        Some(entry.chunk_blob.as_slice().to_vec())
    }

    /// Read the shard for mutation, discarding one whose header this
    /// build cannot write into.
    fn owned_shard_for_write(&self) -> AutoloadShard {
        match read_owned_shard(&self.path) {
            Some(s)
                if s.header.zshrs_version == env!("CARGO_PKG_VERSION")
                    && s.header.pointer_width as usize == std::mem::size_of::<usize>()
                    && s.header.format_version == SHARD_FORMAT_VERSION =>
            {
                s
            }
            _ => fresh_shard(),
        }
    }

    /// Buffer one entry from the cold-start path, where a function is
    /// autoloaded before compinit pre-warm has cached it.
    ///
    /// This used to call `put_many` with a single entry, which rewrote
    /// the entire shard per autoload. See [`AutoloadCache::pending`] for
    /// why that is quadratic in practice. The write now happens in
    /// [`AutoloadCache::flush_pending`], called at the next prompt and
    /// again on exit.
    pub fn put_one(
        &self,
        name: &str,
        chunk_blob: Vec<u8>,
        source_dir: &str,
        source_sha: [u8; 32],
    ) -> Result<(), String> {
        {
            let mut pending = self.pending.lock();
            pending.push((
                name.to_string(),
                chunk_blob,
                source_dir.to_string(),
                source_sha,
            ));
            if pending.len() < PENDING_FLUSH_MAX {
                return Ok(());
            }
        }
        self.flush_pending()
    }

    /// Write every buffered entry in one shard rewrite. A no-op when
    /// nothing is buffered, so it is cheap to call on every prompt.
    ///
    /// On failure the batch is NOT put back: a shard write that failed
    /// once (read-only home, full disk) will fail again, and retrying it
    /// at every prompt would turn a broken cache into a stall. Dropping
    /// the batch only costs a recompile.
    pub fn flush_pending(&self) -> Result<(), String> {
        let batch = {
            let mut pending = self.pending.lock();
            if pending.is_empty() {
                return Ok(());
            }
            std::mem::take(&mut *pending)
        };
        self.put_many(&batch)
    }

    /// Serve an entry that is buffered but not yet on disk.
    ///
    /// Without this a lookup between a `put_one` and the next flush
    /// would miss and recompile a body this very process just compiled.
    /// The binary-identity check the on-disk path applies is trivially
    /// true here — this process wrote these.
    fn pending_lookup(
        &self,
        name: &str,
        source_dir: Option<&str>,
        source_sha: Option<&[u8; 32]>,
    ) -> Option<Vec<u8>> {
        let pending = self.pending.lock();
        // Reverse: last write for a name wins, matching the repeated
        // `entries.insert` in `put_many`.
        pending.iter().rev().find_map(|(n, blob, dir, sha)| {
            if n != name {
                return None;
            }
            if source_dir.is_some_and(|d| d != dir) {
                return None;
            }
            if source_sha.is_some_and(|s| s != sha) {
                return None;
            }
            Some(blob.clone())
        })
    }

    /// Insert many entries in one read + one write of the shard.
    ///
    /// The bulk path for `zshrs --prewarm-autoloads`: compiling 46k
    /// completers one entry at a time would re-serialize the whole
    /// shard 46k times. Existing entries not named here are preserved,
    /// so a prewarm of one fpath dir does not discard the rest.
    pub fn put_many(&self, entries: &[(String, Vec<u8>, String, [u8; 32])]) -> Result<(), String> {
        if entries.is_empty() {
            return Ok(());
        }
        let _lock = match acquire_lock(&self.lock_path) {
            Some(l) => l,
            None => return Ok(()),
        };
        let mut shard = self.owned_shard_for_write();
        let (bin_mtime, bin_len) = current_binary_identity().unwrap_or((0, 0));
        let now = now_secs();
        for (name, chunk_blob, source_dir, source_sha) in entries {
            shard.entries.insert(
                name.clone(),
                AutoloadEntry {
                    binary_mtime_at_cache: bin_mtime,
                    binary_len_at_cache: bin_len,
                    cached_at_secs: now,
                    source_dir: source_dir.clone(),
                    source_sha: *source_sha,
                    chunk_blob: chunk_blob.clone(),
                },
            );
        }
        shard.header.built_at_secs = now as u64;
        write_shard_atomic(&self.path, &shard)?;
        self.invalidate_mmap();
        Ok(())
    }

    /// Drop the entry for `name`, if there is one.
    ///
    /// The loader calls this when a cached chunk ran without defining
    /// the function it was stored under — proof the entry is wrong. It
    /// has to go, or every later process pays the same failure before
    /// falling back.
    pub fn remove(&self, name: &str) -> Result<(), String> {
        // Drop any buffered copy first, or the next flush would write
        // back the very entry just proven wrong.
        self.pending.lock().retain(|(n, _, _, _)| n != name);
        let _lock = match acquire_lock(&self.lock_path) {
            Some(l) => l,
            None => return Ok(()),
        };
        let mut shard = self.owned_shard_for_write();
        if shard.entries.remove(name).is_none() {
            return Ok(());
        }
        shard.header.built_at_secs = now_secs() as u64;
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
        crate::atomic_write::reap_orphan_temps(&self.path);
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
    crate::atomic_write::write_bytes_atomic(path, &bytes)
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `(mtime_secs, len)` of the running `zshrs` binary — the identity a
/// cached chunk is stamped with. Read once; the executable does not
/// change under a live process.
fn current_binary_identity() -> Option<(i64, u64)> {
    static BIN_ID: OnceLock<Option<(i64, u64)>> = OnceLock::new();
    *BIN_ID.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let meta = std::fs::metadata(&exe).ok()?;
        Some((meta.mtime(), meta.len()))
    })
}

/// SHA-256 of the definition text a chunk was compiled from.
///
/// Both writers — the interactive loader and `--prewarm-autoloads` —
/// hash the string they are about to hand the compiler, so an entry is
/// only reused for byte-identical input. That is the whole guarantee:
/// no stat, no path, no assumption about where the bytes came from.
pub fn source_digest(text: &str) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(text.as_bytes());
    hasher.finalize().into()
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
/// Raw presence probe for `dbview autoloads <name>`. See
/// [`AutoloadCache::get`].
pub fn try_load(name: &str) -> Option<Vec<u8>> {
    let cache = CACHE.as_ref()?;
    cache.get(name)
}

/// Execution-path lookup: the compiled definition program for `name`,
/// valid only for this exact directory and definition text.
pub fn try_load_for_source(name: &str, source_dir: &str, source_sha: &[u8; 32]) -> Option<Vec<u8>> {
    let cache = CACHE.as_ref()?;
    cache.get_for_source(name, source_dir, source_sha)
}

/// Write-through after a real autoload compile.
pub fn try_save_one(
    name: &str,
    chunk_blob: &[u8],
    source_dir: &str,
    source_sha: [u8; 32],
) -> Result<(), String> {
    let Some(cache) = CACHE.as_ref() else {
        return Ok(());
    };
    cache.put_one(name, chunk_blob.to_vec(), source_dir, source_sha)
}

/// Bulk write-through for the prewarm path. See
/// [`AutoloadCache::put_many`].
pub fn try_put_many(entries: &[(String, Vec<u8>, String, [u8; 32])]) -> Result<(), String> {
    let Some(cache) = CACHE.as_ref() else {
        return Ok(());
    };
    cache.put_many(entries)
}

/// Write out everything `put_one` buffered. See
/// [`AutoloadCache::flush_pending`]. Called from `preprompt()` (the
/// batch boundary for an interactive shell — every autoload a command
/// or a `<TAB>` triggered lands in one write) and from `zexit` (the
/// boundary for a script, which never reaches a prompt).
pub fn try_flush_pending() {
    if let Some(cache) = CACHE.as_ref() {
        if let Err(e) = cache.flush_pending() {
            tracing::warn!(error = %e, "autoload: could not flush cache");
        }
    }
}

/// Drop a proven-wrong entry. See [`AutoloadCache::remove`].
pub fn try_remove(name: &str) {
    if let Some(cache) = CACHE.as_ref() {
        if let Err(e) = cache.remove(name) {
            tracing::warn!(name, error = %e, "autoload: could not drop bad cache entry");
        }
    }
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

    const DIR: &str = "/some/fpath/dir";

    #[test]
    fn round_trip_one() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        cache
            .put_one("foo", vec![1, 2, 3], DIR, source_digest("body"))
            .unwrap();
        // Served from the pending buffer, before any shard write.
        assert_eq!(cache.get("foo"), Some(vec![1, 2, 3]));
        cache.flush_pending().unwrap();
        assert_eq!(cache.get("foo"), Some(vec![1, 2, 3]));
        assert_eq!(cache.entry_count(), 1);
    }

    #[test]
    fn source_text_mismatch_is_a_miss() {
        // The whole point of the digest: an edited definition file must
        // not be served from a chunk compiled off the old bytes, and
        // neither must a same-named function from another directory.
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        let sha = source_digest("foo() {\nprint one\n}");
        cache.put_one("foo", vec![1, 2, 3], DIR, sha).unwrap();
        cache.flush_pending().unwrap();
        assert_eq!(cache.get_for_source("foo", DIR, &sha), Some(vec![1, 2, 3]));
        // Edited body.
        let edited = source_digest("foo() {\nprint two\n}");
        assert!(cache.get_for_source("foo", DIR, &edited).is_none());
        // Same text, different fpath directory.
        assert!(cache.get_for_source("foo", "/other/dir", &sha).is_none());
    }

    #[test]
    fn an_entry_from_another_binary_is_never_served() {
        // The regression that produced `function not defined by file`:
        // a chunk emitted by a DIFFERENT build must be refused whichever
        // way the timestamps point, because bytecode is not portable
        // between builds even at the same `zshrs_version`.
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        let sha = source_digest("foo() {\nprint one\n}");
        cache.put_one("foo", vec![1, 2, 3], DIR, sha).unwrap();
        cache.flush_pending().unwrap();

        // Rewrite the entry as if a newer build had produced it.
        let mut shard = read_owned_shard(&cache_path).expect("shard readable");
        let entry = shard.entries.get_mut("foo").expect("entry present");
        entry.binary_mtime_at_cache += 10_000;
        write_shard_atomic(&cache_path, &shard).unwrap();

        let reopened = AutoloadCache::open(&cache_path).unwrap();
        assert!(
            reopened.get_for_source("foo", DIR, &sha).is_none(),
            "a chunk from a newer build was accepted",
        );
        assert!(reopened.get("foo").is_none());
    }

    #[test]
    fn remove_drops_the_entry() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        let sha = source_digest("body");
        cache.put_one("foo", vec![1], DIR, sha).unwrap();
        cache.put_one("bar", vec![2], DIR, sha).unwrap();
        cache.remove("foo").unwrap();
        assert!(cache.get("foo").is_none());
        assert_eq!(cache.get("bar"), Some(vec![2]));
    }

    #[test]
    fn cached_names_returns_keys() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        let sha = source_digest("body");
        cache.put_one("alpha", vec![1], DIR, sha).unwrap();
        cache.put_one("beta", vec![2], DIR, sha).unwrap();
        cache.flush_pending().unwrap();
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
        cache
            .put_one("x", vec![1], DIR, source_digest("body"))
            .unwrap();
        cache.flush_pending().unwrap();
        assert!(cache_path.exists());
        cache.clear().unwrap();
        assert!(!cache_path.exists());
    }

    #[test]
    fn a_write_leaves_no_temp_file_behind() {
        // 517 MB of `autoloads.rkyv.tmp.<pid>.<ns>` orphans came from
        // writes that never reached the rename.
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        cache
            .put_one("x", vec![1], DIR, source_digest("body"))
            .unwrap();
        // Must be a REAL write, or this regression test goes vacuous.
        cache.flush_pending().unwrap();
        let temps: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(temps.is_empty(), "temp files left behind: {temps:?}");
    }

    #[test]
    fn put_one_defers_the_shard_write_until_flush() {
        // The whole point of buffering: N autoloads must not cost N
        // rewrites of a shard that reaches 40 MB on a real $fpath.
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        let sha = source_digest("body");

        for i in 0..8 {
            cache.put_one(&format!("f{i}"), vec![i], DIR, sha).unwrap();
        }
        assert!(
            !cache_path.exists(),
            "put_one wrote the shard before the flush boundary"
        );
        // Still readable while buffered — otherwise this process would
        // recompile a body it just compiled.
        assert_eq!(cache.get_for_source("f3", DIR, &sha), Some(vec![3]));

        cache.flush_pending().unwrap();
        assert_eq!(cache.entry_count(), 8);
        assert_eq!(cache.get_for_source("f7", DIR, &sha), Some(vec![7]));
    }

    #[test]
    fn flush_preserves_entries_written_by_another_process() {
        // Concurrent shells share one shard; the flush is a
        // read-modify-write under lock, so a peer's entries survive.
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let sha = source_digest("body");

        let peer = AutoloadCache::open(&cache_path).unwrap();
        peer.put_one("peer_fn", vec![9], DIR, sha).unwrap();
        peer.flush_pending().unwrap();

        let mine = AutoloadCache::open(&cache_path).unwrap();
        mine.put_one("my_fn", vec![1], DIR, sha).unwrap();
        mine.flush_pending().unwrap();

        assert_eq!(mine.get_for_source("peer_fn", DIR, &sha), Some(vec![9]));
        assert_eq!(mine.get_for_source("my_fn", DIR, &sha), Some(vec![1]));
    }

    #[test]
    fn remove_drops_a_buffered_entry_before_it_reaches_disk() {
        // The loader calls remove() when a cached chunk ran without
        // defining its function. If the buffer kept it, the next flush
        // would write back the entry just proven wrong.
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        let sha = source_digest("body");

        cache.put_one("bad", vec![1], DIR, sha).unwrap();
        cache.remove("bad").unwrap();
        cache.flush_pending().unwrap();

        assert!(cache.get("bad").is_none());
        let reopened = AutoloadCache::open(&cache_path).unwrap();
        assert!(reopened.get("bad").is_none());
    }
}
