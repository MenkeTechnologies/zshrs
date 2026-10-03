//! rkyv-backed cache of deparsed function bodies — the text `$functions`
//! and `${functions[name]}` print.
//!
//! **zshrs-original — no C counterpart.** C stores a function as wordcode
//! and `getpermtext` walks it (c:Src/Modules/parameter.c:495), which is
//! cheap. zshrs stores the body as text, so producing the same output means
//! re-parsing and deparsing it. `modules/parameter.rs` memoizes that per
//! process; this file keeps the results across processes, so a new shell's
//! first `${functions}` read (zpwr's `_parameters` does one on every
//! command-position `<TAB>`) does not re-parse every function it has.
//!
//! Single-file shard at `$ZSHRS_HOME/deparse.rkyv`, same discipline as
//! [`autoload_cache`](crate::autoload_cache): versioned header, exact
//! producing-binary identity, atomic-rename writes, a `pending` buffer
//! flushed once per prompt and at exit.
//!
//! # What identifies an entry
//!
//! The deparse is a function of everything the re-lex sees, so the key is a
//! SHA-256 over all of it, taken while `funcdef_lex_pin` is in force:
//!
//!   * the function name and the exact body text;
//!   * every option bit (`isset(1..OPT_SIZE)`), which covers the pinned
//!     RCQUOTES and a sticky emulation's option set;
//!   * the `emulation` value the parser consults directly.
//!
//! A body re-lexed with aliases ON also depends on the whole alias table.
//! Those bodies are not stored here ([`key`] returns `None`) and keep only
//! the per-process memo. Bodies the parser captured and `autoload -U`
//! bodies re-lex with aliases off, and they are the bulk.
//!
//! The whole shard is stamped with the producing binary's `(mtime, len)`:
//! deparse output is a property of the build, so a shard from any other
//! binary is discarded rather than read.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use parking_lot::Mutex;
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};

/// "ZRDP" little-endian.
pub const SHARD_MAGIC: u32 = 0x5A52_4450;
/// Bump when the key recipe or the entry layout changes.
/// v2: token chars moved to the Private Use Area (crate::token_char).
pub const SHARD_FORMAT_VERSION: u32 = 2;
/// Entries buffered before a flush is forced. A memory bound for a process
/// that never reaches a prompt; the real batch boundary is the prompt.
const PENDING_FLUSH_MAX: usize = 4096;
/// A shard that grows past this is started over. Old keys are never removed
/// otherwise (an edited function leaves its previous body's entry behind),
/// and a rebuild costs one re-deparse per live function.
const SHARD_MAX_ENTRIES: usize = 100_000;

/// On-disk shard. `entries` maps a [`key`] digest to the deparsed body.
#[derive(Archive, RkyvDeserialize, RkyvSerialize, Debug, Clone)]
#[archive(check_bytes)]
pub struct DeparseShard {
    /// [`SHARD_MAGIC`].
    pub magic: u32,
    /// [`SHARD_FORMAT_VERSION`].
    pub format_version: u32,
    /// `size_of::<usize>()` of the writer.
    pub pointer_width: u32,
    /// mtime of the binary that produced every entry.
    pub binary_mtime: i64,
    /// Byte length of that binary.
    pub binary_len: u64,
    /// key digest → deparsed body.
    pub entries: HashMap<[u8; 32], String>,
}

struct MmappedShard {
    _mmap: Mmap,
    archived: *const ArchivedDeparseShard,
}

// The mapping is read-only and lives as long as the struct.
unsafe impl Send for MmappedShard {}
unsafe impl Sync for MmappedShard {}

impl MmappedShard {
    fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let mmap = unsafe { Mmap::map(&file).ok()? };
        let archived = rkyv::check_archived_root::<DeparseShard>(&mmap[..]).ok()?;
        let archived = archived as *const ArchivedDeparseShard;
        let shard = Self { _mmap: mmap, archived };
        shard.header_ok().then_some(shard)
    }

    fn shard(&self) -> &ArchivedDeparseShard {
        unsafe { &*self.archived }
    }

    fn header_ok(&self) -> bool {
        let s = self.shard();
        let Some((mtime, len)) = current_binary_identity() else {
            return false;
        };
        let magic: u32 = s.magic.into();
        let fv: u32 = s.format_version.into();
        let pw: u32 = s.pointer_width.into();
        let bm: i64 = s.binary_mtime.into();
        let bl: u64 = s.binary_len.into();
        magic == SHARD_MAGIC
            && fv == SHARD_FORMAT_VERSION
            && pw as usize == std::mem::size_of::<usize>()
            && bm == mtime
            && bl == len
    }

    fn get(&self, key: &[u8; 32]) -> Option<String> {
        self.shard().entries.get(key).map(|s| s.as_str().to_string())
    }
}

/// The per-process handle: the mapped shard plus writes not yet on disk.
pub struct DeparseCache {
    path: PathBuf,
    lock_path: PathBuf,
    /// Opened on first lookup; `Some(None)` = no usable shard on disk.
    mmap: Mutex<Option<Option<MmappedShard>>>,
    pending: Mutex<HashMap<[u8; 32], String>>,
}

impl DeparseCache {
    fn open(path: &Path) -> Self {
        let lock_path = path.with_extension("rkyv.lock");
        Self {
            path: path.to_path_buf(),
            lock_path,
            mmap: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, key: &[u8; 32]) -> Option<String> {
        if let Some(hit) = self.pending.lock().get(key) {
            return Some(hit.clone());
        }
        let mut guard = self.mmap.lock();
        let shard = guard.get_or_insert_with(|| MmappedShard::open(&self.path));
        shard.as_ref()?.get(key)
    }

    fn put(&self, key: [u8; 32], deparsed: String) {
        let full = {
            let mut pending = self.pending.lock();
            pending.insert(key, deparsed);
            pending.len() >= PENDING_FLUSH_MAX
        };
        if full {
            let _ = self.flush_pending();
        }
    }

    /// Fold every buffered entry into the shard in one rewrite. A no-op when
    /// nothing is buffered. A failed write drops the batch instead of
    /// retrying at every prompt; the cost is one re-deparse per entry.
    ///
    /// The lock is taken without blocking: this runs before every prompt,
    /// and waiting on another process's rewrite left a new shell with no
    /// prompt. A busy lock puts the batch back for the next prompt.
    fn flush_pending(&self) -> Result<(), String> {
        let batch = {
            let mut pending = self.pending.lock();
            if pending.is_empty() {
                return Ok(());
            }
            std::mem::take(&mut *pending)
        };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let Some(_lock) = try_acquire_lock(&self.lock_path) else {
            // Entries buffered since the take are newer; keep them. Bounded
            // by PENDING_FLUSH_MAX, past which the rest cost a re-deparse.
            let mut pending = self.pending.lock();
            for (key, deparsed) in batch {
                if pending.len() >= PENDING_FLUSH_MAX {
                    break;
                }
                pending.entry(key).or_insert(deparsed);
            }
            return Ok(());
        };
        let Some((mtime, len)) = current_binary_identity() else {
            return Ok(());
        };
        let mut shard = read_owned_shard(&self.path)
            .filter(|s| {
                s.magic == SHARD_MAGIC
                    && s.format_version == SHARD_FORMAT_VERSION
                    && s.pointer_width as usize == std::mem::size_of::<usize>()
                    && s.binary_mtime == mtime
                    && s.binary_len == len
                    && s.entries.len() < SHARD_MAX_ENTRIES
            })
            .unwrap_or_else(|| DeparseShard {
                magic: SHARD_MAGIC,
                format_version: SHARD_FORMAT_VERSION,
                pointer_width: std::mem::size_of::<usize>() as u32,
                binary_mtime: mtime,
                binary_len: len,
                entries: HashMap::new(),
            });
        shard.entries.extend(batch);
        let bytes =
            rkyv::to_bytes::<_, 4096>(&shard).map_err(|e| format!("rkyv serialize: {}", e))?;
        crate::atomic_write::write_bytes_atomic(&self.path, &bytes)?;
        // The next lookup maps the file just written.
        *self.mmap.lock() = None;
        Ok(())
    }
}

/// Exclusive lock on the shard's side file, or `None` when another
/// process holds it.
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

fn read_owned_shard(path: &Path) -> Option<DeparseShard> {
    let bytes = std::fs::read(path).ok()?;
    let archived = rkyv::check_archived_root::<DeparseShard>(&bytes[..]).ok()?;
    archived.deserialize(&mut rkyv::Infallible).ok()
}

fn current_binary_identity() -> Option<(i64, u64)> {
    use std::os::unix::fs::MetadataExt;
    static BIN_ID: std::sync::OnceLock<Option<(i64, u64)>> = std::sync::OnceLock::new();
    *BIN_ID.get_or_init(|| {
        let meta = std::fs::metadata(std::env::current_exe().ok()?).ok()?;
        Some((meta.mtime(), meta.len()))
    })
}

fn default_cache_path() -> PathBuf {
    let root = match std::env::var_os("ZSHRS_HOME") {
        Some(custom) => PathBuf::from(custom),
        None => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".zshrs"),
    };
    root.join("deparse.rkyv")
}

static CACHE: once_cell::sync::Lazy<Option<DeparseCache>> = once_cell::sync::Lazy::new(|| {
    crate::autoload_cache::cache_enabled().then(|| DeparseCache::open(&default_cache_path()))
});

/// The cache key for deparsing `text` as the body of `name`, or `None` when
/// the result also depends on the live alias table (see the module doc).
/// Call it while `funcdef_lex_pin` is held, so the pinned options are the
/// ones hashed.
pub fn key(name: &str, text: &str) -> Option<[u8; 32]> {
    use sha2::Digest;
    if !crate::ported::lex::noaliases() {
        return None;
    }
    let mut bits = [0u8; (crate::ported::zsh_h::OPT_SIZE as usize).div_ceil(8)];
    for optno in 1..crate::ported::zsh_h::OPT_SIZE {
        if crate::ported::zsh_h::isset(optno) {
            bits[optno as usize / 8] |= 1 << (optno % 8);
        }
    }
    let emulation = crate::ported::options::emulation.load(std::sync::atomic::Ordering::Relaxed);
    let mut h = sha2::Sha256::new();
    h.update(name.as_bytes());
    h.update([0]);
    h.update(text.as_bytes());
    h.update([0]);
    h.update(bits);
    h.update(emulation.to_le_bytes());
    Some(h.finalize().into())
}

/// A previously stored deparse for `key`.
pub fn lookup(key: &[u8; 32]) -> Option<String> {
    CACHE.as_ref()?.get(key)
}

/// Remember `deparsed` for `key`; it reaches disk at the next flush.
pub fn record(key: [u8; 32], deparsed: &str) {
    if let Some(cache) = CACHE.as_ref() {
        cache.put(key, deparsed.to_string());
    }
}

/// Write out everything [`record`] buffered. Called beside
/// `autoload_cache::try_flush_pending` at the prompt, at `zexit`, and from
/// the `atexit` hook, for the same reasons given there.
pub fn try_flush_pending() {
    if let Some(cache) = CACHE.as_ref() {
        if let Err(e) = cache.flush_pending() {
            if !crate::atexit_teardown::active() {
                tracing::warn!(error = %e, "deparse cache: could not flush");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flushed entry is served by a fresh handle on the same file, which is
    /// what a new shell sees.
    #[test]
    fn flushed_entry_survives_into_a_new_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deparse.rkyv");
        let key = [7u8; 32];
        let writer = DeparseCache::open(&path);
        writer.put(key, "print hi".to_string());
        assert_eq!(writer.get(&key).as_deref(), Some("print hi"), "pending read");
        writer.flush_pending().unwrap();
        let reader = DeparseCache::open(&path);
        assert_eq!(reader.get(&key).as_deref(), Some("print hi"));
        assert_eq!(reader.get(&[8u8; 32]), None);
    }

    /// A second flush keeps what the first one wrote.
    #[test]
    fn flush_merges_with_the_shard_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deparse.rkyv");
        let a = DeparseCache::open(&path);
        a.put([1u8; 32], "one".to_string());
        a.flush_pending().unwrap();
        let b = DeparseCache::open(&path);
        b.put([2u8; 32], "two".to_string());
        b.flush_pending().unwrap();
        let c = DeparseCache::open(&path);
        assert_eq!(c.get(&[1u8; 32]).as_deref(), Some("one"));
        assert_eq!(c.get(&[2u8; 32]).as_deref(), Some("two"));
    }

    /// A shard written under a different binary identity is not read.
    #[test]
    fn shard_from_another_binary_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deparse.rkyv");
        let (mtime, len) = current_binary_identity().unwrap();
        let mut entries = HashMap::new();
        entries.insert([3u8; 32], "stale".to_string());
        let shard = DeparseShard {
            magic: SHARD_MAGIC,
            format_version: SHARD_FORMAT_VERSION,
            pointer_width: std::mem::size_of::<usize>() as u32,
            binary_mtime: mtime,
            binary_len: len + 1,
            entries,
        };
        let bytes = rkyv::to_bytes::<_, 4096>(&shard).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(DeparseCache::open(&path).get(&[3u8; 32]), None);
    }
}
