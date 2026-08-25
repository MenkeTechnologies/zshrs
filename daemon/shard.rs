// rkyv shard layer — daemon-prepared, mmap-ready bytecode storage.
//
// Per docs/DAEMON.md "Cache layout (locked)":
//   ~/.zshrs/images/{hash8}-{slug}.rkyv
//
// Per "NO WALKING IN CLIENTS" + "Atomic-rename per shard":
//   - Shards use rkyv's ArchivedHashMap (O(1) lookup, zero-copy).
//   - Build path: (Vec<(String, Vec<u8>)>) → serialize to rkyv → atomic_rename.
//   - Read path: mmap file → archive root → HashMap lookup → bytecode slice.
//   - Atomic rename uses tmp.{pid}.{tid} naming so a daemon crash mid-write
//     leaves only orphaned .tmp.* files (cleaned by the ticker — see DAEMON.md
//     "Engineering details — Orphaned .tmp.{pid}.{tid} cleanup").
//
// Future iterations swap the rkyv HashMap for a perfect-hash function (PHF) for
// closer-to-150ns lookup; v1 uses ArchivedHashMap which is hashbrown-internally
// but still <1µs per lookup at our corpus sizes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use memmap2::Mmap;
use rkyv::{Archive, Deserialize, Serialize};

use super::{paths::CachePaths, DaemonError, Result};

/// Magic in the rkyv shard header — fail-fast if a wrong-format file is mmap'd.
pub const SHARD_MAGIC: u32 = 0x5A53_4853; // "ZSHS"

/// Bumped on incompatible rkyv schema changes.
///
/// 2 &mdash; `ShardHeader` gained the producing binary's identity
///     (`binary_mtime_secs` / `binary_mtime_nsecs` / `binary_len`). A
///     version-1 header has no place to put it, so v1 shards are
///     rejected wholesale rather than read with three zeroes that would
///     never match a real binary anyway.
pub const SHARD_FORMAT_VERSION: u32 = 2;

/// One row in `index.rkyv` — points at a single shard. Per docs/DAEMON.md
/// "Cache layout (locked)" line 192: `index.rkyv ← top-level fq_name →
/// (shard_id, generation, byte_offset)`. Today we index by *shard slug*
/// (one entry per shard file) rather than per fq_name. fq_name → shard
/// resolution is still O(1) on the client side because every fq_name lives
/// in exactly one shard whose name is recoverable from the catalog.entries
/// row's image_path column. Per-fq_name flattening becomes worthwhile once
/// PHF replaces the rkyv-internal HashMap.
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct IndexEntry {
    /// `slug` field.
    pub slug: String,
    /// `source_root` field.
    pub source_root: String,
    /// `generation` field.
    pub generation: u64,
    /// `built_at_ns` field.
    pub built_at_ns: u64,
    /// `entry_count` field.
    pub entry_count: u32,
    /// `byte_size` field.
    pub byte_size: u64,
    /// `path` field.
    pub path: String,
}

/// Top-level index file (`~/.zshrs/index.rkyv`). Written LAST in the
/// rebuild ordering — every shard atomic-renames into place, then this file
/// gets a fresh generation. Clients mmap this first to discover what shards
/// exist + their generations. Per DAEMON.md:184-185: "atomic-rename per
/// shard with strict ordering — shard rename FIRST, then `index.rkyv`
/// update — prevents torn reads."
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct IndexShard {
    /// `magic` field.
    pub magic: u32,
    /// `format_version` field.
    pub format_version: u32,
    /// Monotonic generation across the entire index. Each rebuild bumps it.
    pub generation: u64,
    /// `built_at_ns` field.
    pub built_at_ns: u64,
    /// `entries` field.
    pub entries: Vec<IndexEntry>,
}

impl Default for IndexShard {
    fn default() -> Self {
        Self {
            magic: SHARD_MAGIC,
            format_version: SHARD_FORMAT_VERSION,
            generation: 0,
            built_at_ns: 0,
            entries: Vec::new(),
        }
    }
}

/// Header of every shard. Generation is monotonic, bumped on each rebuild.
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct ShardHeader {
    /// `magic` field.
    pub magic: u32,
    /// `format_version` field.
    pub format_version: u32,
    /// `generation` field.
    pub generation: u64,
    /// `built_at_ns` field.
    pub built_at_ns: u64,
    /// `slug` field.
    pub slug: String,
    /// `source_root` field.
    pub source_root: String,
    /// `entry_count` field.
    pub entry_count: u32,
    /// `st_mtime` seconds of the `zshrs` binary that wrote this shard.
    pub binary_mtime_secs: i64,
    /// `st_mtime` nanoseconds of that binary. Seconds alone are not an
    /// identity: a rebuild landing in the same second as the shard write
    /// compares equal to its predecessor.
    pub binary_mtime_nsecs: i64,
    /// Length in bytes of that binary. Not sufficient alone either
    /// &mdash; two debug builds here measured byte-identical in size
    /// &mdash; but it separates same-mtime binaries of different content.
    pub binary_len: u64,
}

/// Whole shard: header + entry map (fq_name → bytecode bytes).
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct Shard {
    /// `header` field.
    pub header: ShardHeader,
    /// `entries` field.
    pub entries: HashMap<String, Vec<u8>>,
}

/// Per-source-root canonical-state shard. Holds the deterministic state
/// captured by the .zshrc analysis pass (or a `zsync up` promotion) — this
/// is the rkyv-backed source of truth for path/fpath/aliases/functions/etc.
/// SQLite catalog.db's `canonical` table is a hydrated mirror used only for
/// `zcache view` queries (`zcache hydrate-view` refreshes it from rkyv).
/// Bare ShardHeader::default-equivalent for CanonicalShard's Default impl —
/// we can't `#[derive(Default)]` on the rkyv-archived form, so the plain
/// `Default` impl below builds the header inline.
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct CanonicalShard {
    /// `header` field.
    pub header: ShardHeader,
    /// `aliases` field.
    pub aliases: HashMap<String, String>,
    /// `global_aliases` field.
    pub global_aliases: HashMap<String, String>,
    /// `suffix_aliases` field.
    pub suffix_aliases: HashMap<String, String>,
    /// Inline-defined functions captured from `.zshrc` + transitively
    /// sourced files. Replaced on every fsnotify reanalysis.
    pub functions: HashMap<String, String>,
    /// Autoload function bodies pre-loaded from $FPATH at first_init / full
    /// rebuild. Per docs/DAEMON.md "NO WALKING IN CLIENTS" — clients mmap
    /// these instead of reading $FPATH files at runtime. Never touched by
    /// .zshrc reanalysis (so a `.zshrc` save doesn't wipe 18k bodies).
    pub autoload_functions: HashMap<String, String>,
    /// `setopts` field.
    pub setopts: Vec<String>,
    /// `unsetopts` field.
    pub unsetopts: Vec<String>,
    /// `bindkeys` field.
    pub bindkeys: HashMap<String, String>,
    /// `named_dirs` field.
    pub named_dirs: HashMap<String, String>,
    /// `compdef` field.
    pub compdef: HashMap<String, String>,
    /// `zstyle` field.
    pub zstyle: Vec<(String, String)>,
    /// `zmodload` field.
    pub zmodload: Vec<String>,
    /// `env_exports` field.
    pub env_exports: HashMap<String, String>,
    /// `params` field.
    pub params: HashMap<String, String>,
    /// `path` field.
    pub path: Vec<String>,
    /// `fpath` field.
    pub fpath: Vec<String>,
    /// `manpath` field.
    pub manpath: Vec<String>,
    /// `plugins` field.
    pub plugins: Vec<(String, String)>, // (manager, name)
    /// `sourced_files` field.
    pub sourced_files: Vec<String>,
    /// Catch-all for subsystems not enumerated above (zcompdump_raw, service,
    /// patcomp, postpatcomp, autoload_completion, …). Keyed by subsystem
    /// name → entry map. Lets new subsystems land without a shard format
    /// version bump; readers iterate this and fold into the in-memory state.
    pub extras: HashMap<String, HashMap<String, String>>,
}

impl Default for CanonicalShard {
    fn default() -> Self {
        Self {
            header: ShardHeader {
                magic: SHARD_MAGIC,
                format_version: SHARD_FORMAT_VERSION,
                generation: 0,
                built_at_ns: 0,
                slug: String::new(),
                source_root: String::new(),
                entry_count: 0,
                binary_mtime_secs: current_binary_identity().0,
                binary_mtime_nsecs: current_binary_identity().1,
                binary_len: current_binary_identity().2,
            },
            aliases: HashMap::new(),
            global_aliases: HashMap::new(),
            suffix_aliases: HashMap::new(),
            functions: HashMap::new(),
            autoload_functions: HashMap::new(),
            setopts: Vec::new(),
            unsetopts: Vec::new(),
            bindkeys: HashMap::new(),
            named_dirs: HashMap::new(),
            compdef: HashMap::new(),
            zstyle: Vec::new(),
            zmodload: Vec::new(),
            env_exports: HashMap::new(),
            params: HashMap::new(),
            path: Vec::new(),
            fpath: Vec::new(),
            manpath: Vec::new(),
            plugins: Vec::new(),
            sourced_files: Vec::new(),
            extras: HashMap::new(),
        }
    }
}

impl Shard {
    /// `new` — see implementation.
    pub fn new(slug: impl Into<String>, source_root: impl Into<String>, generation: u64) -> Self {
        Self {
            header: ShardHeader {
                magic: SHARD_MAGIC,
                format_version: SHARD_FORMAT_VERSION,
                generation,
                built_at_ns: now_ns(),
                slug: slug.into(),
                source_root: source_root.into(),
                entry_count: 0,
                binary_mtime_secs: current_binary_identity().0,
                binary_mtime_nsecs: current_binary_identity().1,
                binary_len: current_binary_identity().2,
            },
            entries: HashMap::new(),
        }
    }
    /// `insert` — see implementation.
    pub fn insert(&mut self, fq_name: impl Into<String>, bytecode: Vec<u8>) {
        self.entries.insert(fq_name.into(), bytecode);
        self.header.entry_count = self.entries.len() as u32;
    }
    /// `len` — see implementation.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// `is_empty` — see implementation.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Build hash8 prefix for the shard filename — first 8 hex chars of source-root path hash.
pub fn hash8(source_root: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(source_root.as_bytes());
    digest
        .iter()
        .take(4)
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Compose the canonical shard filename: `{hash8}-{slug}.rkyv`.
pub fn shard_filename(source_root: &str, slug: &str) -> String {
    format!("{}-{}.rkyv", hash8(source_root), slug)
}

/// Compose the absolute shard path under a CachePaths root.
pub fn shard_path(paths: &CachePaths, source_root: &str, slug: &str) -> PathBuf {
    paths.images.join(shard_filename(source_root, slug))
}

/// Compose the per-shard advisory flock path.
pub fn shard_lock_path(paths: &CachePaths, source_root: &str, slug: &str) -> PathBuf {
    paths
        .images
        .join(format!("{}-{}.rkyv.lock", hash8(source_root), slug))
}

/// Take the blocking exclusive `flock` on a shard's lock file, held
/// across the serialize + `rename` below.
///
/// The lock path has existed since the shard layer was written and was
/// never taken. Without it the writers are last-rename-wins: a shard is
/// replaced WHOLE, so two processes that each built one from their own
/// view of the source root silently discard the other's entries, and a
/// reader between the two renames sees a generation that is about to
/// vanish. The lock lives on a side file precisely because `rename`
/// swaps the shard's inode &mdash; a lock on the shard itself would be
/// released the moment the new file landed.
fn lock_shard(paths: &CachePaths, source_root: &str, slug: &str) -> Option<nix::fcntl::Flock<std::fs::File>> {
    let path = shard_lock_path(paths, source_root, slug);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let f = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    nix::fcntl::Flock::lock(f, nix::fcntl::FlockArg::LockExclusive).ok()
}

/// `(mtime_secs, mtime_nsecs, len)` of the running binary &mdash; the
/// identity stamped into every shard header.
///
/// Equality, never ordering. A binary whose mtime moves BACKWARDS (an
/// older build restored over a newer one, a `cp -p`, a checkout of a
/// previously built `target/`) satisfies any "not older than" test while
/// being a different build; the nanoseconds term separates two builds
/// inside the same second, and the length separates two builds sharing
/// an mtime.
pub fn current_binary_identity() -> (i64, i64, u64) {
    use std::os::unix::fs::MetadataExt;
    static BIN_ID: std::sync::OnceLock<(i64, i64, u64)> = std::sync::OnceLock::new();
    *BIN_ID.get_or_init(|| {
        std::env::current_exe()
            .and_then(std::fs::metadata)
            .map(|m| (m.mtime(), m.mtime_nsec(), m.len()))
            // No `current_exe()`: an identity nothing can match, so every
            // shard is a miss rather than an unconditional accept.
            .unwrap_or((-1, -1, 0))
    })
}

/// Atomic-rename writer for the canonical-state shard. Same crash-safety
/// guarantees as `write_shard` (tmp + fsync + rename).
pub fn write_canonical_shard(paths: &CachePaths, shard: &CanonicalShard) -> Result<PathBuf> {
    let final_path = shard_path(paths, &shard.header.source_root, &shard.header.slug);
    let _lock = lock_shard(paths, &shard.header.source_root, &shard.header.slug);
    let pid = std::process::id();
    let nanos = now_ns();
    let tmp_path = paths.images.join(format!(
        "{}.tmp.{}.{}",
        shard_filename(&shard.header.source_root, &shard.header.slug),
        pid,
        nanos
    ));

    let bytes = rkyv::to_bytes::<_, 4096>(shard)
        .map_err(|e| DaemonError::other(format!("rkyv canonical serialize: {e}")))?;
    {
        // Born 0600. Without this OpenOptions block, File::create applies
        // the user's umask (typically 022 → 644), and ensure_file_600 then
        // coerces every shard with a WARN log line. With it, ensure_file_600
        // is silent on the happy path. O_NOFOLLOW + O_CREAT|O_EXCL prevents
        // tmp-file symlink races.
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o600)
            .open(&tmp_path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path)?;
    super::paths::ensure_file_600(&final_path)?;
    tracing::info!(
        slug = %shard.header.slug,
        generation = shard.header.generation,
        bytes = bytes.len(),
        path = %final_path.display(),
        "canonical shard written"
    );
    Ok(final_path)
}

/// Read a canonical-state shard back from disk. Decodes the archived form into
/// an owned `CanonicalShard` — daemon keeps that owned copy in memory; clients
/// that need raw mmap access go through `MmappedShard` instead.
pub fn read_canonical_shard(path: &Path) -> Result<CanonicalShard> {
    let bytes = std::fs::read(path)?;
    let archived = rkyv::check_archived_root::<CanonicalShard>(&bytes)
        .map_err(|e| DaemonError::other(format!("canonical shard validation: {e}")))?;
    let owned: CanonicalShard = archived
        .deserialize(&mut rkyv::Infallible)
        .map_err(|e| DaemonError::other(format!("canonical shard deserialize: {e}")))?;
    Ok(owned)
}

/// Serialize a shard and atomic-rename it into place.
///
/// Crash-safe: writes to `<final>.tmp.<pid>.<tid>` first, fsyncs, then renames over
/// the final path. The ticker sweeps orphaned `.tmp.*` files older than ~1 minute.
pub fn write_shard(paths: &CachePaths, shard: &Shard) -> Result<PathBuf> {
    let final_path = shard_path(paths, &shard.header.source_root, &shard.header.slug);
    let _lock = lock_shard(paths, &shard.header.source_root, &shard.header.slug);

    let pid = std::process::id();
    // Use thread id approximation — std::thread::current().id() doesn't expose a stable
    // u64 representation. We can use a thread_local counter or just fall back to a nanos
    // suffix; nanos is unique enough for tmp-file collision avoidance.
    let nanos = now_ns();
    let tmp_path = paths.images.join(format!(
        "{}.tmp.{}.{}",
        shard_filename(&shard.header.source_root, &shard.header.slug),
        pid,
        nanos
    ));

    let bytes = rkyv::to_bytes::<_, 4096>(shard)
        .map_err(|e| DaemonError::other(format!("rkyv serialize: {e}")))?;

    {
        // O_NOFOLLOW | O_CREAT | O_EXCL on the tmp path: prevents symlink
        // races where an attacker pre-creates the tmp file as a symlink to
        // somewhere else. Per docs/DAEMON.md "Sensitive content" — applied
        // to all shards (the cost is negligible) so sensitive flag is just
        // a downstream signal, not a separate write path.
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o600)
            .open(&tmp_path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }

    std::fs::rename(&tmp_path, &final_path)?;
    super::paths::ensure_file_600(&final_path)?;

    tracing::info!(
        slug = %shard.header.slug,
        generation = shard.header.generation,
        entries = shard.header.entry_count,
        bytes = bytes.len(),
        path = %final_path.display(),
        "shard written"
    );

    Ok(final_path)
}

/// mmap and validate a shard from disk. Returns an MmappedShard which holds the mmap
/// alive — drop it to release the mapping.
pub struct MmappedShard {
    /// `_mmap` field.
    _mmap: Mmap,
    /// `path` field.
    path: PathBuf,
    /// SAFETY-relevant: the archived reference points into `_mmap`, which lives as long
    /// as this struct. The pointer is valid for the lifetime of the struct.
    archived: *const ArchivedShard,
}

impl std::fmt::Debug for MmappedShard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MmappedShard")
            .field("path", &self.path)
            .field("entries", &self.entry_count())
            .field("generation", &self.generation())
            .field("slug", &self.slug())
            .finish()
    }
}

// SAFETY: MmappedShard is a self-referential struct (mmap + pointer into it). The
// pointer stays valid as long as the mmap. Send is safe because mmap owns its memory
// and no shared mutability is exposed; Sync is safe because reads through the pointer
// are immutable and rkyv-validated.
unsafe impl Send for MmappedShard {}
unsafe impl Sync for MmappedShard {}

impl MmappedShard {
    /// `open` — see implementation.
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        let archived = rkyv::check_archived_root::<Shard>(&mmap[..])
            .map_err(|e| DaemonError::other(format!("shard validation failed: {e}")))?;

        // The bytes below this header are EXECUTED: `entries` maps a
        // fq_name to fusevm bytecode a client replays. Bytecode is only
        // meaningful to the build that emitted it — opcode numbering,
        // builtin ids and chunk layout all move with the source tree —
        // so a shard stamped by any other binary is a miss, exactly as
        // `autoload_cache` and `script_cache` treat their chunks.
        let (secs, nsecs, len) = current_binary_identity();
        let h = &archived.header;
        let stamped: (i64, i64, u64) = (
            h.binary_mtime_secs.into(),
            h.binary_mtime_nsecs.into(),
            h.binary_len.into(),
        );
        let want_version: u32 = SHARD_FORMAT_VERSION;
        let got_version: u32 = h.format_version.into();
        if got_version != want_version || stamped != (secs, nsecs, len) {
            return Err(DaemonError::other(format!(
                "shard {} was built by a different zshrs (format v{}, binary {}.{}.{}); ignoring",
                path.display(),
                got_version,
                stamped.0,
                stamped.1,
                stamped.2
            )));
        }

        let archived_ptr = archived as *const ArchivedShard;

        Ok(Self {
            _mmap: mmap,
            path: path.to_path_buf(),
            archived: archived_ptr,
        })
    }

    /// Open a shard with `O_NOFOLLOW` + `MAP_PRIVATE` for sensitive content.
    /// Per docs/DAEMON.md "Sensitive content" (line 339-342). MAP_PRIVATE
    /// causes any writes (defensive programming — we don't write but the
    /// kernel guarantee is "no shared visibility") to stay copy-on-write
    /// inside this process. O_NOFOLLOW prevents symlink-swap attacks.
    pub fn open_sensitive(path: &Path) -> Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        // memmap2's `MmapOptions::map_copy_read_only` returns a private (COW)
        // read-only mapping → MAP_PRIVATE on Linux/macOS.
        let mmap = unsafe { memmap2::MmapOptions::new().map_copy_read_only(&file)? };
        let archived = rkyv::check_archived_root::<Shard>(&mmap[..])
            .map_err(|e| DaemonError::other(format!("shard validation failed: {e}")))?;
        // The bytes below this header are EXECUTED: `entries` maps a
        // fq_name to fusevm bytecode a client replays. Bytecode is only
        // meaningful to the build that emitted it — opcode numbering,
        // builtin ids and chunk layout all move with the source tree —
        // so a shard stamped by any other binary is a miss, exactly as
        // `autoload_cache` and `script_cache` treat their chunks.
        let (secs, nsecs, len) = current_binary_identity();
        let h = &archived.header;
        let stamped: (i64, i64, u64) = (
            h.binary_mtime_secs.into(),
            h.binary_mtime_nsecs.into(),
            h.binary_len.into(),
        );
        let want_version: u32 = SHARD_FORMAT_VERSION;
        let got_version: u32 = h.format_version.into();
        if got_version != want_version || stamped != (secs, nsecs, len) {
            return Err(DaemonError::other(format!(
                "shard {} was built by a different zshrs (format v{}, binary {}.{}.{}); ignoring",
                path.display(),
                got_version,
                stamped.0,
                stamped.1,
                stamped.2
            )));
        }
        let archived_ptr = archived as *const ArchivedShard;
        Ok(Self {
            _mmap: mmap,
            path: path.to_path_buf(),
            archived: archived_ptr,
        })
    }

    /// Reference to the validated archived shard root.
    pub fn shard(&self) -> &ArchivedShard {
        // SAFETY: archived_ptr points into _mmap, which lives as long as Self.
        unsafe { &*self.archived }
    }
    /// `header` — see implementation.
    pub fn header(&self) -> &ArchivedShardHeader {
        &self.shard().header
    }
    /// `generation` — see implementation.
    pub fn generation(&self) -> u64 {
        self.shard().header.generation.into()
    }
    /// `slug` — see implementation.
    pub fn slug(&self) -> &str {
        self.shard().header.slug.as_str()
    }
    /// `entry_count` — see implementation.
    pub fn entry_count(&self) -> u32 {
        self.shard().header.entry_count.into()
    }

    /// O(1) average lookup of a fq_name → bytecode bytes.
    pub fn get(&self, fq_name: &str) -> Option<&[u8]> {
        self.shard().entries.get(fq_name).map(|v| v.as_slice())
    }

    /// Iterate keys (for `${(k)_comps}` analogues — daemon-only, never exposed to
    /// clients per the no-walking rule).
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.shard().entries.keys().map(|s| s.as_str())
    }
    /// `path` — see implementation.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Sweep orphaned `.tmp.*` files older than the threshold from the images dir.
/// Used by the daemon's ticker job + `zcache verify`.
pub fn sweep_tmp_files(paths: &CachePaths, max_age: std::time::Duration) -> Result<usize> {
    let mut removed = 0usize;
    let now = SystemTime::now();
    if !paths.images.exists() {
        return Ok(0);
    }
    for entry in std::fs::read_dir(&paths.images)? {
        let entry = entry?;
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.contains(".tmp.") {
            continue;
        }
        let meta = entry.metadata()?;
        let modified = meta.modified()?;
        if now.duration_since(modified).unwrap_or_default() >= max_age {
            std::fs::remove_file(entry.path())?;
            removed += 1;
            tracing::warn!(file = %s, "removed orphaned tmp shard");
        }
    }
    Ok(removed)
}

/// Build an IndexShard from every existing shard in `paths.images/`. Walks
/// the dir, opens each shard via mmap to read its header (slug, source_root,
/// generation, entry_count), populates an IndexEntry, then writes
/// `~/.zshrs/index.rkyv` atomically. Per DAEMON.md:184-185: shard
/// rename FIRST, index.rkyv update LAST.
///
/// Returns the path the index was written to + the entry count.
pub fn rebuild_index(paths: &CachePaths) -> Result<(PathBuf, u32)> {
    let shard_paths = list_shards(paths)?;
    let mut entries: Vec<IndexEntry> = Vec::with_capacity(shard_paths.len());
    for sp in &shard_paths {
        // Try plain Shard first; if that fails, try CanonicalShard (per-plugin
        // shards use the canonical archive, not the bytecode-entries form).
        let byte_size = std::fs::metadata(sp).map(|m| m.len()).unwrap_or(0);
        let path_str = sp.display().to_string();

        if let Ok(m) = MmappedShard::open(sp) {
            entries.push(IndexEntry {
                slug: m.slug().to_string(),
                source_root: m.shard().header.source_root.as_str().to_string(),
                generation: m.generation(),
                built_at_ns: m.shard().header.built_at_ns.into(),
                entry_count: m.entry_count(),
                byte_size,
                path: path_str,
            });
            continue;
        }
        if let Ok(c) = read_canonical_shard(sp) {
            entries.push(IndexEntry {
                slug: c.header.slug.clone(),
                source_root: c.header.source_root.clone(),
                generation: c.header.generation,
                built_at_ns: c.header.built_at_ns,
                entry_count: c.header.entry_count,
                byte_size,
                path: path_str,
            });
            continue;
        }
        tracing::warn!(path = %sp.display(), "rebuild_index: shard unreadable, skipping");
    }
    entries.sort_by(|a, b| a.slug.cmp(&b.slug));

    let generation = now_ns();
    let index = IndexShard {
        magic: SHARD_MAGIC,
        format_version: SHARD_FORMAT_VERSION,
        generation,
        built_at_ns: generation,
        entries: entries.clone(),
    };

    let bytes = rkyv::to_bytes::<_, 4096>(&index)
        .map_err(|e| DaemonError::other(format!("rkyv index serialize: {e}")))?;
    let final_path = paths.index_rkyv.clone();
    let tmp_path = paths.root.join(format!(
        "index.rkyv.tmp.{}.{}",
        std::process::id(),
        generation
    ));
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o600)
            .open(&tmp_path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp_path, &final_path)?;

    let entry_count = entries.len() as u32;
    tracing::info!(
        path = %final_path.display(),
        generation,
        entries = entry_count,
        bytes = bytes.len(),
        "index.rkyv written"
    );
    Ok((final_path, entry_count))
}

/// Read `index.rkyv` back, returning the owned IndexShard. Used by `zcache
/// info`, `zcache view index`, and integrity checks. Clients that just want
/// "is shard X up-to-date" can mmap the file directly and check generation.
pub fn read_index(paths: &CachePaths) -> Result<IndexShard> {
    if !paths.index_rkyv.exists() {
        return Ok(IndexShard::default());
    }
    let bytes = std::fs::read(&paths.index_rkyv)?;
    let archived = rkyv::check_archived_root::<IndexShard>(&bytes)
        .map_err(|e| DaemonError::other(format!("index.rkyv validation: {e}")))?;
    let owned: IndexShard = archived
        .deserialize(&mut rkyv::Infallible)
        .map_err(|e| DaemonError::other(format!("index.rkyv deserialize: {e:?}")))?;
    Ok(owned)
}

/// List every shard file currently in the images dir (sorted by name).
pub fn list_shards(paths: &CachePaths) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !paths.images.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&paths.images)? {
        let entry = entry?;
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.ends_with(".rkyv") || s.contains(".tmp.") {
            continue;
        }
        out.push(entry.path());
    }
    out.sort();
    Ok(out)
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh() -> (TempDir, CachePaths) {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        (tmp, paths)
    }

    #[test]
    fn hash8_is_deterministic() {
        let h1 = hash8("/Users/wizard/.zpwr");
        let h2 = hash8("/Users/wizard/.zpwr");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 8);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash8_distinct_for_distinct_inputs() {
        let h1 = hash8("/Users/wizard/.zpwr");
        let h2 = hash8("/Users/wizard/.zpwrr");
        assert_ne!(h1, h2);
    }

    #[test]
    fn shard_filename_format() {
        let f = shard_filename("/some/path", "zpwr");
        assert!(f.ends_with("-zpwr.rkyv"));
        assert_eq!(f.split('-').next().unwrap().len(), 8);
    }

    #[test]
    fn write_then_read_roundtrip() {
        let (_tmp, paths) = fresh();
        let mut shard = Shard::new("test", "/Users/wizard/test", 1);
        shard.insert("_git", b"\x01\x02\x03 git bytecode".to_vec());
        shard.insert("_docker", b"\xaa\xbb\xcc docker bytecode".to_vec());
        shard.insert("_kubectl", b"\xff\xee\xdd kubectl bytecode".to_vec());

        let path = write_shard(&paths, &shard).unwrap();
        assert!(path.exists());
        let mode = std::fs::metadata(&path).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.mode() & 0o777, 0o600);

        let read = MmappedShard::open(&path).unwrap();
        assert_eq!(read.entry_count(), 3);
        assert_eq!(read.slug(), "test");
        assert_eq!(read.generation(), 1);
        assert_eq!(read.get("_git"), Some(&b"\x01\x02\x03 git bytecode"[..]));
        assert_eq!(
            read.get("_kubectl"),
            Some(&b"\xff\xee\xdd kubectl bytecode"[..])
        );
        assert_eq!(read.get("_nonexistent"), None);

        // Same shard via the sensitive open path (MAP_PRIVATE + O_NOFOLLOW).
        // Per docs/DAEMON.md:339-342. Read result must match exactly.
        let read_sensitive = MmappedShard::open_sensitive(&path).unwrap();
        assert_eq!(read_sensitive.entry_count(), 3);
        assert_eq!(read_sensitive.slug(), "test");
        assert_eq!(
            read_sensitive.get("_docker"),
            Some(&b"\xaa\xbb\xcc docker bytecode"[..])
        );
    }

    #[test]
    fn open_sensitive_rejects_symlink() {
        // O_NOFOLLOW means a symlink at the shard path can't be opened.
        // Defense-in-depth per DAEMON.md sensitive-content section.
        let (_tmp, paths) = fresh();
        let mut shard = Shard::new("real", "/Users/wizard/test", 1);
        shard.insert("_git", b"real bytecode".to_vec());
        let real_path = write_shard(&paths, &shard).unwrap();

        let symlink_path = paths.images.join("symlink-real.rkyv");
        std::os::unix::fs::symlink(&real_path, &symlink_path).unwrap();

        // Plain open follows the symlink; sensitive open refuses it.
        assert!(MmappedShard::open(&symlink_path).is_ok());
        let err = MmappedShard::open_sensitive(&symlink_path).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("symbolic link")
                || msg.contains("symlink")
                || msg.contains("ELOOP")
                || msg.contains("loop"),
            "expected symlink-related error, got: {}",
            msg
        );
    }

    #[test]
    fn rebuild_index_writes_index_rkyv() {
        let (_tmp, paths) = fresh();
        let mut s1 = Shard::new("system", "/Users/wizard/test", 100);
        s1.insert("_git", b"git bc".to_vec());
        write_shard(&paths, &s1).unwrap();

        let mut s2 = Shard::new("zpwr", "/Users/wizard/zpwr", 200);
        s2.insert("_zpwr", b"zpwr bc".to_vec());
        write_shard(&paths, &s2).unwrap();

        let (idx_path, count) = rebuild_index(&paths).unwrap();
        assert_eq!(idx_path, paths.index_rkyv);
        assert_eq!(count, 2);
        assert!(idx_path.exists());
        let mode = std::fs::metadata(&idx_path).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.mode() & 0o777, 0o600);

        let idx = read_index(&paths).unwrap();
        assert_eq!(idx.entries.len(), 2);
        let slugs: Vec<&str> = idx.entries.iter().map(|e| e.slug.as_str()).collect();
        assert!(
            slugs.contains(&"system"),
            "system slug missing: {:?}",
            slugs
        );
        assert!(slugs.contains(&"zpwr"), "zpwr slug missing: {:?}", slugs);
        assert!(idx.generation > 0);
    }

    #[test]
    fn read_index_returns_default_when_absent() {
        let (_tmp, paths) = fresh();
        let idx = read_index(&paths).unwrap();
        assert_eq!(idx.entries.len(), 0);
        assert_eq!(idx.generation, 0);
    }

    #[test]
    fn write_overwrite_via_atomic_rename() {
        let (_tmp, paths) = fresh();
        let mut shard1 = Shard::new("test", "/Users/wizard/test", 1);
        shard1.insert("_git", b"v1 bytecode".to_vec());
        write_shard(&paths, &shard1).unwrap();

        let mut shard2 = Shard::new("test", "/Users/wizard/test", 2);
        shard2.insert("_git", b"v2 bytecode".to_vec());
        shard2.insert("_docker", b"v2 docker".to_vec());
        let path = write_shard(&paths, &shard2).unwrap();

        let read = MmappedShard::open(&path).unwrap();
        assert_eq!(read.generation(), 2);
        assert_eq!(read.entry_count(), 2);
        assert_eq!(read.get("_git"), Some(&b"v2 bytecode"[..]));
    }

    #[test]
    fn sweep_removes_old_tmp_files() {
        let (_tmp, paths) = fresh();

        // Create a fake orphan that's "old".
        let orphan = paths.images.join("00000000-test.rkyv.tmp.99999.123");
        std::fs::write(&orphan, b"orphan").unwrap();
        // Backdate it.
        let past = filetime::FileTime::from_unix_time(1, 0);
        filetime::set_file_mtime(&orphan, past).unwrap();

        let removed = sweep_tmp_files(&paths, std::time::Duration::from_secs(60)).unwrap();
        assert_eq!(removed, 1);
        assert!(!orphan.exists());
    }

    #[test]
    fn sweep_skips_recent_tmp_files() {
        let (_tmp, paths) = fresh();
        let recent = paths.images.join("00000000-test.rkyv.tmp.99999.456");
        std::fs::write(&recent, b"recent").unwrap();
        let removed = sweep_tmp_files(&paths, std::time::Duration::from_secs(60)).unwrap();
        assert_eq!(removed, 0);
        assert!(recent.exists());
    }

    #[test]
    fn list_shards_filters_tmp_and_lock() {
        let (_tmp, paths) = fresh();
        std::fs::write(paths.images.join("aaaaaaaa-foo.rkyv"), b"x").unwrap();
        std::fs::write(paths.images.join("bbbbbbbb-bar.rkyv"), b"x").unwrap();
        std::fs::write(paths.images.join("cccccccc-baz.rkyv.tmp.1.2"), b"x").unwrap();
        std::fs::write(paths.images.join("dddddddd-zip.rkyv.lock"), b"x").unwrap();

        let listed = list_shards(&paths).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|p| p.extension().unwrap() == "rkyv"));
        assert!(listed
            .iter()
            .all(|p| !p.to_string_lossy().contains(".tmp.")));
    }

    #[test]
    fn empty_shard_roundtrip() {
        let (_tmp, paths) = fresh();
        let shard = Shard::new("empty", "/some/root", 1);
        let path = write_shard(&paths, &shard).unwrap();
        let read = MmappedShard::open(&path).unwrap();
        assert_eq!(read.entry_count(), 0);
        assert!(read.shard().entries.is_empty());
    }

    #[test]
    fn corrupt_file_rejected_on_open() {
        let (_tmp, paths) = fresh();
        let bogus = paths.images.join("zzzzzzzz-bogus.rkyv");
        std::fs::write(&bogus, b"this is not a valid rkyv archive").unwrap();
        let err = MmappedShard::open(&bogus).unwrap_err();
        assert!(format!("{}", err).contains("validation failed"));
    }
}
