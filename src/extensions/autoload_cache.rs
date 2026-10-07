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
//! [`crate::script_cache`] module — `fusevm::Chunk` is upstream and only derives serde).
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
/// v4: token chars moved to the Private Use Area (crate::token_char); v3
/// chunks hold the old U+0084..=U+00A2 tokens.
/// v5: an entry can carry its [`ResolvedLoad`], so a load is served by name
/// with no `$fpath` search.
pub const SHARD_FORMAT_VERSION: u32 = 6;
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
    /// Everything the `$fpath` search and the file read produced for this
    /// function, so the next load needs neither (see [`try_resolve`]).
    /// `None` for an entry written without it; that entry is still served by
    /// `(dir, sha)` after a search.
    pub resolved: Option<ResolvedLoad>,
}

/// The outcome of resolving and reading one autoload, as `loadautofn` and
/// the registration step computed it: the body `getfpfunc` led to and the
/// definition text that body registers as.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone, PartialEq)]
#[archive(check_bytes)]
pub struct ResolvedLoad {
    /// The body `loadautofn` installs: the file text, or a `.zwc` deparse.
    pub body: String,
    /// The definition text the body registers as (the `name() { … }` wrap
    /// decision applied). Its SHA-256 is the entry's `source_sha`.
    pub registered: String,
    /// The dump's ksh style (`try_dump_file`'s `*ksh`), or -1 for a plain file.
    pub dump_ksh: i32,
    /// The body is a `.zwc` deparse.
    pub from_wordcode: bool,
    /// The registration parse rejected the body.
    pub parse_failed: bool,
    /// `autoload_is_ksh_style` when the body was registered; the wrap
    /// decision depends on it.
    pub ksh_style: bool,
    /// Every file in `source_dir` the resolution could have read
    /// ([`stamp_candidates`]), as it was then.
    pub stamps: Vec<FileStamp>,
}

/// One file's identity at resolution time.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone, PartialEq)]
#[archive(check_bytes)]
pub struct FileStamp {
    /// Absolute path.
    pub path: String,
    /// Whether it existed.
    pub exists: bool,
    /// mtime in nanoseconds.
    pub mtime_ns: i64,
    /// Size in bytes.
    pub len: u64,
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
    /// Resolutions waiting for their entry's write ([`note_resolved`]). The
    /// next write of that name's entry takes the one stored here.
    resolved_pending: Mutex<HashMap<String, ResolvedLoad>>,
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
            resolved_pending: Mutex::new(HashMap::new()),
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
        // An empty chunk is a probe-only entry (`record_port_probe`): no
        // bytecode.
        if let Some(blob) = self.pending_lookup(name, None, None) {
            return (!blob.is_empty()).then_some(blob);
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
        (!entry.chunk_blob.is_empty()).then(|| entry.chunk_blob.as_slice().to_vec())
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
    ///
    /// The lock is taken WITHOUT blocking. This runs before every prompt,
    /// and the holder may be another process rewriting a shard that has
    /// reached 1 GB, which a debug build takes minutes to do; a blocking
    /// `flock` here left a new shell with no prompt for that long. When
    /// the lock is busy the batch goes back into the buffer for the next
    /// prompt.
    pub fn flush_pending(&self) -> Result<(), String> {
        let batch = {
            let mut pending = self.pending.lock();
            // A resolution can be waiting without a chunk: the chunk was
            // already cached, so nothing called `put_one`.
            if pending.is_empty() && self.resolved_pending.lock().is_empty() {
                return Ok(());
            }
            std::mem::take(&mut *pending)
        };
        let Some(lock) = try_acquire_lock(&self.lock_path) else {
            self.requeue(batch);
            return Ok(());
        };
        self.put_many_locked(&batch, lock)
    }

    /// Put a batch whose flush found the lock busy back in front of
    /// anything buffered since, so `pending_lookup`'s last-write-wins
    /// order holds. Bounded by [`PENDING_FLUSH_MAX`]: past it the oldest
    /// entries are dropped, which only costs a recompile.
    fn requeue(&self, mut batch: Vec<(String, Vec<u8>, String, [u8; 32])>) {
        let mut pending = self.pending.lock();
        batch.append(&mut pending);
        let excess = batch.len().saturating_sub(PENDING_FLUSH_MAX);
        batch.drain(..excess);
        *pending = batch;
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
        if entries.is_empty() && self.resolved_pending.lock().is_empty() {
            return Ok(());
        }
        let Some(lock) = acquire_lock(&self.lock_path) else {
            return Ok(());
        };
        self.put_many_locked(entries, lock)
    }

    /// The read-modify-write of [`AutoloadCache::put_many`], for a caller
    /// that already holds the shard lock. The lock is released on return.
    fn put_many_locked(
        &self,
        entries: &[(String, Vec<u8>, String, [u8; 32])],
        _lock: nix::fcntl::Flock<File>,
    ) -> Result<(), String> {
        let mut shard = self.owned_shard_for_write();
        let (bin_mtime, bin_len) = current_binary_identity().unwrap_or((0, 0));
        let now = now_secs();
        for (name, chunk_blob, source_dir, source_sha) in entries {
            // A resolution noted for this name describes this write only if
            // it registers the same text; otherwise keep the one already on
            // disk when the entry itself is unchanged.
            // A probe-only write (`record_port_probe`: no chunk) never replaces
            // a compiled entry; its resolution is attached below instead.
            if chunk_blob.is_empty() && shard.entries.contains_key(name) {
                continue;
            }
            let noted = self
                .resolved_pending
                .lock()
                .remove(name)
                .filter(|r| r.registered.is_empty() || registered_digest(r) == *source_sha);
            let prior = shard
                .entries
                .get(name)
                .filter(|e| e.source_dir == *source_dir && e.source_sha == *source_sha)
                .and_then(|e| e.resolved.clone());
            shard.entries.insert(
                name.clone(),
                AutoloadEntry {
                    binary_mtime_at_cache: bin_mtime,
                    binary_len_at_cache: bin_len,
                    cached_at_secs: now,
                    source_dir: source_dir.clone(),
                    source_sha: *source_sha,
                    chunk_blob: chunk_blob.clone(),
                    resolved: noted.or(prior),
                },
            );
        }
        // Resolutions for entries this batch did not rewrite: attach each to
        // the entry it registers, when that entry still holds the same text
        // from the same directory.
        for (name, r) in std::mem::take(&mut *self.resolved_pending.lock()) {
            if let Some(e) = shard.entries.get_mut(&name) {
                let same_dir = r
                    .stamps
                    .first()
                    .is_some_and(|s| s.path == format!("{}/{}", e.source_dir, name));
                // A probe resolution (empty `registered`) only fills an entry
                // that has none; a load's resolution must register its text.
                let fits = if r.registered.is_empty() {
                    e.resolved.is_none()
                } else {
                    registered_digest(&r) == e.source_sha
                };
                if same_dir && fits {
                    e.resolved = Some(r);
                }
            }
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
        self.resolved_pending.lock().remove(name);
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

/// [`acquire_lock`] that returns `None` instead of waiting when another
/// process holds the lock.
fn try_acquire_lock(path: &Path) -> Option<nix::fcntl::Flock<File>> {
    let f = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .ok()?;
    nix::fcntl::Flock::lock(f, nix::fcntl::FlockArg::LockExclusiveNonblock).ok()
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

/// The identity of every file `getfpfunc` could have read for `name` in
/// `dir`: the plain file, the per-function dump and the directory digest
/// (c:Src/parse.c:3725 `try_dump_file` tries `<dir>.zwc` and
/// `<dir>/<name>.zwc` before `<dir>/<name>`).
pub fn stamp_candidates(dir: &str, name: &str) -> Vec<FileStamp> {
    [
        format!("{dir}/{name}"),
        format!("{dir}/{name}.zwc"),
        format!("{dir}.zwc"),
    ]
    .into_iter()
    .map(|path| match std::fs::metadata(&path) {
        Ok(m) => FileStamp {
            exists: true,
            mtime_ns: m.mtime() * 1_000_000_000 + m.mtime_nsec(),
            len: m.len(),
            path,
        },
        Err(_) => FileStamp {
            path,
            exists: false,
            mtime_ns: 0,
            len: 0,
        },
    })
    .collect()
}

/// The `source_sha` an entry for `r` carries: a `.zwc` deparse is hashed with
/// a `\0zwc\0` prefix (vm_helper `autoload_source_key`), so it never shares
/// a key with a plain file of the same text.
fn registered_digest(r: &ResolvedLoad) -> [u8; 32] {
    if r.from_wordcode {
        source_digest(&format!("\0zwc\0{}", r.registered))
    } else {
        source_digest(&r.registered)
    }
}

fn stamps_fresh(stamps: &[FileStamp]) -> bool {
    stamps.iter().all(|s| match std::fs::metadata(&s.path) {
        Ok(m) => {
            s.exists && m.mtime() * 1_000_000_000 + m.mtime_nsec() == s.mtime_ns && m.len() == s.len
        }
        Err(_) => !s.exists,
    })
}

impl AutoloadCache {
    /// The stored resolution for `name`, if it is still the one a search
    /// would produce. See [`try_resolve`].
    fn resolve(&self, name: &str, fpath: &[String]) -> Option<(String, ResolvedLoad)> {
        let pending_dir = self
            .pending
            .lock()
            .iter()
            .rev()
            .find(|(n, ..)| n == name)
            .map(|(_, _, dir, _)| dir.clone());
        let (dir, resolved) = match pending_dir.and_then(|d| {
            self.resolved_pending.lock().get(name).cloned().map(|r| (d, r))
        }) {
            Some(hit) => hit,
            None => {
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
                let resolved: ResolvedLoad =
                    entry.resolved.as_ref()?.deserialize(&mut rkyv::Infallible).ok()?;
                (entry.source_dir.as_str().to_string(), resolved)
            }
        };
        (fpath.iter().any(|d| *d == dir) && stamps_fresh(&resolved.stamps)).then_some((dir, resolved))
    }
}

/// The directory and resolved body for `name`, read from the shard instead
/// of searching `$fpath` and reading the file.
///
/// !!! RUST-ONLY — NO C COUNTERPART !!! C's `loadautofn` runs `getfpfunc`
/// (c:Src/exec.c:5759) on every first call of an autoload in every
/// process. The entry records which directory that search chose and the
/// state of every file it could have read there, so a hit costs three
/// `stat`s: the directory must still be on `fpath`, and those files must be
/// unchanged. A file added to an EARLIER `fpath` directory, which would now
/// shadow this one, is not seen until the entry is rewritten (a prewarm, or
/// a load after this entry goes stale).
pub fn try_resolve(name: &str, fpath: &[String]) -> Option<(String, ResolvedLoad)> {
    // A probe resolution (`record_port_probe`) has no registration to install.
    CACHE.as_ref()?.resolve(name, fpath).filter(|(_, r)| !r.registered.is_empty())
}

/// The directory and file text for a native completer `name`, read from the
/// shard: what a `getfpfunc` existence probe plus a read of `<dir>/<name>`
/// would return. `None` for a `.zwc`-resolved entry (its body is a deparse,
/// not the file) and for anything stale.
pub fn try_port_body(name: &str, fpath: &[String]) -> Option<(String, String)> {
    let (dir, r) = CACHE.as_ref()?.resolve(name, fpath)?;
    (!r.from_wordcode).then_some((dir, r.body))
}

/// Record what a native-completer probe found: `name` lives in `dir` and its
/// file reads `body`. Stored as a resolution with no registration and no
/// chunk, which [`try_port_body`] and [`try_source_dir`] serve and the loader
/// ignores.
pub fn record_port_probe(name: &str, dir: &str, body: &str) {
    let Some(cache) = CACHE.as_ref() else {
        return;
    };
    note_resolved(
        name,
        ResolvedLoad {
            body: body.to_string(),
            registered: String::new(),
            dump_ksh: -1,
            from_wordcode: false,
            parse_failed: false,
            ksh_style: false,
            stamps: stamp_candidates(dir, name),
        },
    );
    let _ = cache.put_one(name, Vec::new(), dir, [0u8; 32]);
}

/// The `$fpath` directory the entry for `name` was resolved from, while
/// that directory is still on `fpath` and still holds the file. The
/// shard-backed answer to a `getfpfunc` existence probe (test_only), with
/// the same caveat as [`try_resolve`] about a later shadowing file.
pub fn try_source_dir(name: &str, fpath: &[String]) -> Option<String> {
    let cache = CACHE.as_ref()?;
    if let Some((dir, _)) = cache.resolve(name, fpath) {
        return Some(dir);
    }
    cache.ensure_mmap();
    let guard = cache.mmap.lock();
    let shard = guard.as_ref()?;
    if !shard.header_ok() {
        return None;
    }
    let dir = shard.lookup(name)?.source_dir.as_str().to_string();
    let on_fpath = fpath.iter().any(|d| *d == dir);
    (on_fpath && std::path::Path::new(&format!("{dir}/{name}")).exists()).then_some(dir)
}

/// Remember `resolved` for `name`; it is written with the next write of
/// that name's entry (`put_one` / `put_many`).
pub fn note_resolved(name: &str, resolved: ResolvedLoad) {
    if let Some(cache) = CACHE.as_ref() {
        cache.resolved_pending.lock().insert(name.to_string(), resolved);
    }
}

/// What `loadautofn` resolved for one load, held until the registration
/// step adds the definition text (see [`stage_search`], [`commit_search`]).
struct StagedSearch {
    dir: String,
    body: String,
    dump_ksh: i32,
    from_wordcode: bool,
}

thread_local! {
    static STAGED_SEARCH: std::cell::RefCell<HashMap<String, StagedSearch>> =
        std::cell::RefCell::new(HashMap::new());
    static STAGED_HIT: std::cell::RefCell<HashMap<String, ResolvedLoad>> =
        std::cell::RefCell::new(HashMap::new());
}

/// `loadautofn` searched `$fpath` and read `body` out of `dir`.
pub fn stage_search(name: &str, dir: &str, body: &str, dump_ksh: Option<i32>, from_wordcode: bool) {
    STAGED_SEARCH.with(|s| {
        s.borrow_mut().insert(
            name.to_string(),
            StagedSearch {
                dir: dir.to_string(),
                body: body.to_string(),
                dump_ksh: dump_ksh.unwrap_or(-1),
                from_wordcode,
            },
        )
    });
}

/// The registration step turned the staged body into `registered`; record
/// the whole resolution for the entry that compile is about to write.
pub fn commit_search(name: &str, body: &str, registered: &str, parse_failed: bool, ksh_style: bool) {
    let Some(s) = STAGED_SEARCH.with(|s| s.borrow_mut().remove(name)) else {
        return;
    };
    if s.body != body {
        return;
    }
    let stamps = stamp_candidates(&s.dir, name);
    note_resolved(
        name,
        ResolvedLoad {
            body: s.body,
            registered: registered.to_string(),
            dump_ksh: s.dump_ksh,
            from_wordcode: s.from_wordcode,
            parse_failed,
            ksh_style,
            stamps,
        },
    );
}

/// `loadautofn` installed `resolved.body` from the shard.
pub fn stage_hit(name: &str, resolved: ResolvedLoad) {
    STAGED_HIT.with(|s| s.borrow_mut().insert(name.to_string(), resolved));
}

/// The stored registration for a body `loadautofn` took from the shard:
/// `(registered, from_wordcode, parse_failed)`. `None` unless `body` and
/// the ksh-style decision are the ones it was registered under.
pub fn take_hit(name: &str, body: &str, ksh_style: bool) -> Option<(String, bool, bool)> {
    let r = STAGED_HIT.with(|s| s.borrow_mut().remove(name))?;
    (r.body == body && r.ksh_style == ksh_style).then_some((r.registered, r.from_wordcode, r.parse_failed))
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
            // Same TLS hazard as the reap log in `atomic_write`: this runs from
            // the `atexit` hook below, where `tracing`'s thread-local buffer may
            // be gone.
            if !crate::atexit_teardown::active() {
                tracing::warn!(error = %e, "autoload: could not flush cache");
            }
        }
    }
}

/// libc `atexit` hook: flush whatever `put_one` buffered.
///
/// `preprompt()` covers an interactive shell and `zexit` covers a shell that
/// unwinds normally, but NEITHER runs on the paths that actually end a
/// one-shot: `zshrs -c SCRIPT` leaves through `std::process::exit` in
/// `bins/zshrs.rs`, and the `exit` builtin terminates the process directly.
/// `std::process::exit` runs no Rust destructors, so without this every
/// `zshrs -c` recompiled its autoloads and wrote nothing back — the cache was
/// dead for scripts.
///
/// libc atexit runs AFTER the Rust runtime begins tearing down thread-locals,
/// and `tracing::*` touches TLS, so the body is wrapped in `catch_unwind`: an
/// unwinding panic out of an `extern "C"` function aborts the process. Same
/// hazard and same mitigation as `recorder::atexit_finalize`.
///
/// `catch_unwind` alone is not enough, which is why `atexit_teardown::mark()`
/// comes first. `catch_unwind` runs after the panic hook, so by the time it
/// catches anything `panicked at ...` is already on the terminal the user is
/// exiting; the unwind then skips the rest of the flush (`invalidate_mmap`,
/// and any further work added here later). The mark tells the `tracing` sites
/// on this path to stay quiet so the panic never happens at all; the
/// `catch_unwind` stays as the backstop for anything else TLS-backed.
extern "C" fn atexit_flush_pending() {
    crate::atexit_teardown::mark();
    let _ = std::panic::catch_unwind(try_flush_pending);
    // The deparse cache buffers the same way and exits by the same paths.
    let _ = std::panic::catch_unwind(crate::deparse_cache::try_flush_pending);
}

/// Register [`atexit_flush_pending`]. Idempotent; call once from `main`.
pub fn install_atexit_flush() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // SAFETY: `atexit_flush_pending` is a plain `extern "C"` fn with the
        // signature libc requires.
        unsafe {
            libc::atexit(atexit_flush_pending);
        }
    });
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

    /// A load is served by name, with no `$fpath` search, only while the
    /// recorded resolution is current: its directory still on `fpath`, its
    /// files unchanged. A flushed entry carries it into a new handle.
    #[test]
    fn resolution_served_by_name_until_its_inputs_change() {
        let _g = crate::test_util::global_state_lock();
        let tmp = tempdir().unwrap();
        let fdir = tmp.path().join("fns");
        std::fs::create_dir(&fdir).unwrap();
        let file = fdir.join("_foo");
        std::fs::write(&file, "print one\n").unwrap();
        let fdir_s = fdir.to_string_lossy().to_string();
        let fpath = vec!["/elsewhere".to_string(), fdir_s.clone()];
        let resolved = ResolvedLoad {
            body: "print one\n".to_string(),
            registered: "_foo() {\nprint one\n}".to_string(),
            dump_ksh: -1,
            from_wordcode: false,
            parse_failed: false,
            ksh_style: false,
            stamps: stamp_candidates(&fdir_s, "_foo"),
        };
        let cache_path = tmp.path().join("autoloads.rkyv");
        let cache = AutoloadCache::open(&cache_path).unwrap();
        cache.resolved_pending.lock().insert("_foo".to_string(), resolved.clone());
        cache
            .put_one("_foo", vec![1], &fdir_s, registered_digest(&resolved))
            .unwrap();
        cache.flush_pending().unwrap();

        let fresh = AutoloadCache::open(&cache_path).unwrap();
        assert_eq!(fresh.resolve("_foo", &fpath), Some((fdir_s.clone(), resolved)));
        assert_eq!(fresh.resolve("_foo", &["/elsewhere".to_string()]), None, "dir left fpath");
        std::fs::write(&file, "print two!\n").unwrap();
        assert_eq!(fresh.resolve("_foo", &fpath), None, "file edited");
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

    #[test]
    fn flush_with_the_lock_held_elsewhere_returns_and_keeps_the_batch() {
        // flush_pending runs before every prompt. Another process holding
        // the shard lock (a long rewrite) must not stall the prompt, and
        // the batch must survive for the next one.
        let _g = crate::test_util::global_state_lock();
        let dir = tempdir().unwrap();
        let cache_path = dir.path().join("autoloads.rkyv");
        let cache = std::sync::Arc::new(AutoloadCache::open(&cache_path).unwrap());
        let sha = source_digest("body");
        cache.put_one("f", vec![1], DIR, sha).unwrap();

        // flock locks belong to the open file description, so a second
        // open in this process contends exactly like another process.
        let held = acquire_lock(&cache.lock_path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let flusher = std::sync::Arc::clone(&cache);
        // Detached rather than scoped: a flush that blocks must fail the
        // test, not hang it. The panic drops `held`, which frees it.
        std::thread::spawn(move || {
            let _ = tx.send(flusher.flush_pending());
        });
        let res = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("flush_pending blocked on a lock held elsewhere");
        assert!(res.is_ok());
        assert!(!cache_path.exists(), "wrote the shard without the lock");
        assert_eq!(cache.get_for_source("f", DIR, &sha), Some(vec![1]));

        drop(held);
        cache.flush_pending().unwrap();
        assert!(cache_path.exists());
        assert_eq!(cache.entry_count(), 1);
    }
}
