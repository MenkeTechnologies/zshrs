// rkyv shard layer — daemon-prepared, mmap-ready bytecode storage.
//
// Per docs/DAEMON.md "Cache layout (locked)":
//   ~/.cache/zshrs/images/{hash8}-{slug}.rkyv
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
pub const SHARD_FORMAT_VERSION: u32 = 1;

/// Header of every shard. Generation is monotonic, bumped on each rebuild.
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct ShardHeader {
    pub magic: u32,
    pub format_version: u32,
    pub generation: u64,
    pub built_at_ns: u64,
    pub slug: String,
    pub source_root: String,
    pub entry_count: u32,
}

/// Whole shard: header + entry map (fq_name → bytecode bytes).
#[derive(Archive, Deserialize, Serialize, Clone, Debug)]
#[archive(check_bytes)]
pub struct Shard {
    pub header: ShardHeader,
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
    pub header: ShardHeader,
    pub aliases: HashMap<String, String>,
    pub global_aliases: HashMap<String, String>,
    pub suffix_aliases: HashMap<String, String>,
    pub functions: HashMap<String, String>,
    pub setopts: Vec<String>,
    pub unsetopts: Vec<String>,
    pub bindkeys: HashMap<String, String>,
    pub named_dirs: HashMap<String, String>,
    pub compdef: HashMap<String, String>,
    pub zstyle: Vec<(String, String)>,
    pub zmodload: Vec<String>,
    pub env_exports: HashMap<String, String>,
    pub params: HashMap<String, String>,
    pub path: Vec<String>,
    pub fpath: Vec<String>,
    pub manpath: Vec<String>,
    pub plugins: Vec<(String, String)>, // (manager, name)
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
            },
            aliases: HashMap::new(),
            global_aliases: HashMap::new(),
            suffix_aliases: HashMap::new(),
            functions: HashMap::new(),
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
            },
            entries: HashMap::new(),
        }
    }

    pub fn insert(&mut self, fq_name: impl Into<String>, bytecode: Vec<u8>) {
        self.entries.insert(fq_name.into(), bytecode);
        self.header.entry_count = self.entries.len() as u32;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

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

/// Atomic-rename writer for the canonical-state shard. Same crash-safety
/// guarantees as `write_shard` (tmp + fsync + rename).
pub fn write_canonical_shard(
    paths: &CachePaths,
    shard: &CanonicalShard,
) -> Result<PathBuf> {
    let final_path = shard_path(paths, &shard.header.source_root, &shard.header.slug);
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
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_path)?;
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
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_path)?;
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
    _mmap: Mmap,
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
    pub fn open(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        let archived = rkyv::check_archived_root::<Shard>(&mmap[..])
            .map_err(|e| DaemonError::other(format!("shard validation failed: {e}")))?;

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

    pub fn header(&self) -> &ArchivedShardHeader {
        &self.shard().header
    }

    pub fn generation(&self) -> u64 {
        self.shard().header.generation.into()
    }

    pub fn slug(&self) -> &str {
        self.shard().header.slug.as_str()
    }

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
