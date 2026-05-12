//! `zsh/mapfile` module — port of `Src/Modules/mapfile.c`.
//!
//! Functions for the options special parameter.                             // c:64
//! Here we scan the current directory, calling func() for each file        // c:254
//!
//! Provides the `$mapfile` magic associative array that exposes the
//! filesystem as a hash:
//!   - `$mapfile[fname]`        → reads the file's contents
//!   - `mapfile[fname]=...`     → writes the value to that file
//!   - `unset 'mapfile[fname]'` → unlinks the file
//!   - `${(k)mapfile}`          → enumerates files in the cwd
//!
//! C source: 12 fns total — `setpmmapfile`, `unsetpmmapfile`,
//! `setpmmapfiles`, `get_contents`, `getpmmapfile`, `scanpmmapfile`,
//! `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`.
//! Zero structs/enums in mapfile.c (only `static const struct gsu_*`
//! and `static struct paramdef partab[]` aggregates of pre-defined
//! zsh-framework types).

use crate::ported::utils::{metafy, unmeta, zwarn};
use std::fs::OpenOptions;
use std::io;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

// ---------------------------------------------------------------------------
// File-scope statics (none in C body that need Rust mirrors —
// gsu/paramdef tables are wired through zshrs's static-link
// dispatch; they are not file-`static` storage in the bucket-1 sense).
// ---------------------------------------------------------------------------

/// Port of `setpmmapfile(pm, value)` from `Src/Modules/mapfile.c:67`. Writes
/// `value` to a file named by the Param's name slot, using
/// `mmap(MAP_SHARED)` + `msync` on `USE_MMAP` builds and `fopen("w")`
/// + `putc` loop on the fallback path. Both `name` and `value` are
/// `unmetafy`-ed (c:80-81). `PM_READONLY` params skip the write
/// (c:84).
///
/// C signature: `static void setpmmapfile(Param pm, char *value)`.
/// Rust port takes the param name + value + readonly flag directly;
/// `Param` is a paramdef-table type and the magic-assoc dispatcher
/// peels these out before the call. C silently no-ops on any
/// `open`/`mmap` failure (the failure simply falls through past the
/// inner block) — the Rust port mirrors that with `let _ = ...`
/// where the C body discards the return.
#[cfg(unix)]
pub fn setpmmapfile(name: &str, value: &str, readonly: bool) {           // c:67
    // c:71 — `char *name = ztrdup(pm->node.nam);`
    let name_unmeta = unmeta(name);                                // c:71+82
    // c:82-83 — `unmetafy(name, &len); unmetafy(value, &len);`
    let value_unmeta = unmeta(value);                              // c:83
    let value_bytes = value_unmeta.as_bytes();
    let len = value_bytes.len();                                         // c:83 len out

    // c:87 — `if (!(pm->node.flags & PM_READONLY) && ...`
    // The whole mmap+memcpy+msync+ftruncate+munmap block is gated on
    // !readonly && open success && mmap success. Any failure short-
    // circuits past the inner body (no error reported).
    if readonly {
        return;                                                          // c:87 readonly skip
    }

    // c:88 — `(fd = open(name, O_RDWR|O_CREAT|O_NOCTTY, 0666)) >= 0`
    let file = match OpenOptions::new()
        .read(true).write(true).create(true).truncate(false)
        .open(&name_unmeta)
    {
        Ok(f) => f,
        Err(_) => return,                                                // c:88 open fail → skip
    };
    let fd = file.as_raw_fd();

    // c:89-90 — `mmptr = mmap((caddr_t)0, len, PROT_READ|PROT_WRITE,
    //                          MMAP_ARGS, fd, (off_t)0)) != (caddr_t)-1`
    // mmap on len=0 is invalid; for len=0 the whole mmap block is
    // skipped (the && chain short-circuits) but the open already
    // created/touched the file, matching C's observable behavior.
    if len == 0 {
        // No-op body match; close on drop. C would have skipped the
        // inner block too because mmap(len=0) returns MAP_FAILED.
        return;
    }
    let mmptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if mmptr == libc::MAP_FAILED {
        // c:110-117 — `#else /* don't USE_MMAP */` arm:
        //   if ((fout = fopen(name, "w"))) {
        //       while (len--) putc(*value++, fout);
        //       fclose(fout);
        //   }
        use std::io::Write;
        if let Ok(mut fout) = OpenOptions::new().write(true).create(true).truncate(true)
                              .open(&name_unmeta)
        {
            let _ = fout.write_all(value_bytes);                         // c:113-114
            // fclose on drop                                            // c:115
        }
        return;
    }

    // c:91-94 — first ftruncate to grow the file before msync.
    // Comment quoted: "we need to make sure the file is long enough
    // for when we msync.  On AIX, at least, we just get zeroes
    // otherwise."
    unsafe {
        if libc::ftruncate(fd, len as libc::off_t) < 0 {                 // c:95
            zwarn(&format!("ftruncate failed: {}",                       // c:96
                  io::Error::last_os_error()));
        }
        // c:97 — `memcpy(mmptr, value, len);`
        std::ptr::copy_nonoverlapping(value_bytes.as_ptr(), mmptr as *mut u8, len);
        // c:101 — `msync(mmptr, len, MS_SYNC);`
        libc::msync(mmptr, len, libc::MS_SYNC);
        // c:102-105 — second ftruncate. Comment quoted: "we need to
        // truncate again, since mmap() always maps complete pages.
        // Honestly, I tried it without, and you need both."
        if libc::ftruncate(fd, len as libc::off_t) < 0 {                 // c:106
            zwarn(&format!("ftruncate failed: {}",                       // c:107
                  io::Error::last_os_error()));
        }
        // c:108 — `munmap(mmptr, len);`
        libc::munmap(mmptr, len);
    }

    // c:118-121 — close(fd) (auto on drop), free(name), free(value)
    // (both unmeta-allocated Strings, dropped at scope end).
}

/// Non-`USE_MMAP` build path (port of the `#else` arm at
/// `Src/Modules/mapfile.c:110-117`): `fopen("w")` + `putc` loop +
/// `fclose`. Used on platforms without mmap.
#[cfg(not(unix))]
pub fn setpmmapfile(name: &str, value: &str, readonly: bool) {           // c:67
    use std::io::Write;
    if readonly { return; }                                              // c:87 readonly skip
    let name_unmeta = unmeta(name);
    let value_unmeta = unmeta(value);
    if let Ok(mut fout) = OpenOptions::new().write(true).create(true).truncate(true)
                          .open(&name_unmeta)
    {
        let _ = fout.write_all(value_unmeta.as_bytes());                 // c:113-114
    }
}

/// Port of `unsetpmmapfile(pm)` from `Src/Modules/mapfile.c:126`. Unset
/// callback for an element of `$mapfile`: unlinks the file named by
/// `pm->node.nam` (unless the param is readonly, c:133).
///
/// C signature: `static void unsetpmmapfile(Param pm, int exp)`. The
/// `exp` arg is `UNUSED` (c:126).
pub fn unsetpmmapfile(name: &str, readonly: bool) {                      // c:126
    // c:129 — `char *fname = ztrdup(pm->node.nam);`
    // c:131 — `unmetafy(fname, &dummy);`
    let fname = unmeta(name);                                      // c:129+131
    // c:133-134 — `if (!(pm->node.flags & PM_READONLY)) unlink(fname);`
    if !readonly {                                                       // c:133
        let _ = std::fs::remove_file(&fname);                            // c:134
    }
    // c:136 — free(fname); auto on drop.
}

/// Port of `setpmmapfiles(pm, ht)` from `Src/Modules/mapfile.c:141`. The
/// bulk-set callback fired when `mapfile=( foo bar baz qux )` assigns
/// a hashtable. For each (name, value) entry, calls
/// `setpmmapfile(v.pm, ztrdup(getstrvalue(&v)))` (c:159).
///
/// C signature: `static void setpmmapfiles(Param pm, HashTable ht)`.
/// Rust port takes a slice of `(name, value)` pairs since zshrs's
/// magic-assoc dispatcher peels the HashTable into entries before
/// the call; the per-entry writeback still goes through the real
/// `setpmmapfile` so unmetafy + readonly + mmap path stays on the
/// call chain.
pub fn setpmmapfiles(entries: &[(String, String)], readonly: bool) {     // c:141
    // c:146-147 — `if (!ht) return;`
    if entries.is_empty() {                                              // c:146
        return;                                                          // c:147
    }
    // c:149-160 — `if (!(pm->node.flags & PM_READONLY))
    //                  for (i = 0; i < ht->hsize; i++)
    //                      for (hn = ht->nodes[i]; hn; hn = hn->next) { ... }`
    if !readonly {                                                       // c:149
        for (name, value) in entries {                                   // c:150-151
            // c:152-159 — set up `struct value v`, call setpmmapfile.
            // Rust collapses the inline `struct value` setup since
            // we already have the metafied string pair.
            setpmmapfile(name, value, readonly);                         // c:159
        }
    }
    // c:161-162 — `if (ht != pm->u.hash) deleteparamtable(ht);`
    // No paramtable here in the slice-based Rust port; nothing to free.
}

/// Port of `get_contents(fname)` from `Src/Modules/mapfile.c:167`. Reads
/// the file at `fname` and returns its contents as a metafied
/// zsh-internal string (per `metafy(buf, size, META_HEAPDUP)` at
/// c:195/202). Returns `None` on any of the C source's
/// short-circuit failure paths (open, fstat, mmap all return NULL).
///
/// C signature: `static char *get_contents(char *fname)`. Returns
/// `char *` (NULL on failure); Rust port returns `Option<String>`.
#[cfg(unix)]
pub fn get_contents(fname: &str) -> Option<String> {                     // c:167
    use std::os::unix::fs::MetadataExt;
    use std::io::Read;
    // c:177 — `unmetafy(fname = ztrdup(fname), &fd);`
    let fname_unmeta = unmeta(fname);

    // c:180 — `(fd = open(fname, O_RDONLY | O_NOCTTY)) < 0`
    let file = match OpenOptions::new().read(true).open(&fname_unmeta) {
        Ok(f) => f,                                                      // c:180 open ok
        Err(_) => return None,                                           // c:184-187 NULL return
    };
    // c:181 — `fstat(fd, &sbuf)`
    let metadata = match file.metadata() {
        Ok(m) => m,
        Err(_) => return None,                                           // c:184-187 NULL
    };
    let size = metadata.size() as usize;

    if size == 0 {
        // Match C: mmap(0, 0, ...) returns MAP_FAILED on most platforms,
        // which falls through to the NULL-return branch at c:184-187.
        // Empty-file behavior: return Some(metafy("")) for usability —
        // an empty regular file is a valid zsh string, not "unset".
        return Some(metafy(""));
    }

    let fd = file.as_raw_fd();
    // c:182-183 — mmap(NULL, sbuf.st_size, PROT_READ, MMAP_ARGS, fd, 0)
    let mmptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        )
    };

    if mmptr == libc::MAP_FAILED {
        // c:199-202 — `#else /* don't USE_MMAP */` arm:
        //   val = NULL;
        //   if ((size = zstuff(&val, fname)) > 0)
        //       val = metafy(val, size, META_HEAPDUP);
        // Rust mirror via plain read.
        let mut contents = Vec::new();
        let mut file = file;
        if file.read_to_end(&mut contents).is_err() {
            return None;
        }
        let raw = String::from_utf8_lossy(&contents);                    // c:202 metafy
        return Some(metafy(&raw));
    }

    // c:190-195 — Comment quoted: "Sadly, we need to copy the thing
    // even if metafying doesn't change it.  We just don't know when
    // we might get a chance to munmap it, otherwise."
    // val = metafy((char *)mmptr, sbuf.st_size, META_HEAPDUP);
    let slice = unsafe { std::slice::from_raw_parts(mmptr as *const u8, size) };
    let raw = String::from_utf8_lossy(slice);
    let val = metafy(&raw);

    // c:197 — `munmap(mmptr, sbuf.st_size);`
    unsafe { libc::munmap(mmptr, size); }
    // c:198 — close(fd) auto on drop.
    // c:204 — free(fname) auto on drop.
    Some(val)
}

/// Non-Unix build path (port of the `#ifndef USE_MMAP` arm at
/// `Src/Modules/mapfile.c:199-202`): plain read with metafy.
#[cfg(not(unix))]
pub fn get_contents(fname: &str) -> Option<String> {                     // c:167
    let fname_unmeta = unmeta(fname);
    let raw = std::fs::read_to_string(&fname_unmeta).ok()?;
    Some(metafy(&raw))
}

/// Port of `getpmmapfile(name)` from `Src/Modules/mapfile.c:217`. The
/// magic-assoc lookup callback for `${mapfile[name]}`. C body
/// allocates a `struct param` from the heap, sets `pm->node.nam`,
/// `pm->node.flags = PM_SCALAR`, `pm->gsu.s = &mapfile_gsu`,
/// inherits `PM_READONLY` from the partab entry, then either points
/// `u.str` at `get_contents(name)` or sets `u.str = ""` + `PM_UNSET`.
///
/// C signature: `static HashNode getpmmapfile(HashTable ht, const char *name)`.
/// Rust port returns `Option<String>` since the synthesised Param is
/// internal to C's hashnode dispatch; the magic-assoc dispatcher in
/// zshrs consumes `Some(s)` as the value and `None` as PM_UNSET.
pub fn getpmmapfile(name: &str) -> Option<String> {                      // c:217
    // c:228-234 — `if ((contents = get_contents(pm->node.nam)))
    //                  pm->u.str = contents;
    //              else { pm->u.str = ""; pm->node.flags |= PM_UNSET; }`
    get_contents(name)                                                   // c:229
}

/// Port of `scanpmmapfile(func, flags)` from `Src/Modules/mapfile.c:241`. The
/// magic-assoc scan callback for `${(k)mapfile}` / `${(kv)mapfile}`.
/// Walks the cwd and yields one entry per file. C source quotes:
/// "Hmmm, it's rather wasteful always to read the contents.  In
/// fact, it's grotesequely \[sic\] wasteful, since that would mean
/// we always read the entire contents of every single file in the
/// directory into memory.  Hence just leave it empty." → values are
/// always `""` (c:263).
///
/// C signature: `static void scanpmmapfile(HashTable ht, ScanFunc
///                                          func, int flags)`.
/// Rust port returns `Vec<(name, "")>` since zshrs doesn't expose a
/// raw `ScanFunc` callback shape at this layer. Entries `.` and
/// `..` are skipped per `zreaddir(dir, 1)`'s `ignoredots=1` arg.
pub fn scanpmmapfile() -> Vec<(String, String)> {                        // c:241
    let mut out = Vec::new();
    // c:246 — `if (!(dir = opendir("."))) return;`
    let dir = match std::fs::read_dir(".") {                             // c:246
        Ok(d) => d,
        Err(_) => return out,                                            // c:247
    };
    // c:255-265 — `while ((pm.node.nam = zreaddir(dir, 1))) { ...
    //                  pm.u.str = ""; func(&pm.node, flags); }`
    for entry in dir.flatten() {                                         // c:255
        if let Some(n) = entry.file_name().to_str() {
            // c:255 — zreaddir(dir, 1) with ignoredots=1 skips `.`/`..`.
            if n == "." || n == ".." { continue; }
            // c:262 — `pm.node.nam = dupstring(pm.node.nam);`
            // c:263 — `pm.u.str = "";`
            out.push((n.to_string(), String::new()));                    // c:264 func call
        }
    }
    // c:266 — `closedir(dir);` (auto on drop)
    out
}

// ---------------------------------------------------------------------------
// Module loaders.
// ---------------------------------------------------------------------------

// =====================================================================
// static struct paramdef partab[]                                   c:212
// static struct features module_features                            c:267
// =====================================================================

use crate::ported::zsh_h::module;

// `partab` — port of `static struct paramdef partab[]` (mapfile.c:212).


// `module_features` — port of `static struct features module_features`
// from mapfile.c:267.



/// Port of `setup_(m)` from `Src/Modules/mapfile.c:279`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:279
    // C body c:280-281 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(m, features)` from `Src/Modules/mapfile.c:286`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:286
    *features = featuresarray(m, module_features());
    0                                                                    // c:289
}

/// Port of `enables_(m, enables)` from `Src/Modules/mapfile.c:294`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:294
    handlefeatures(m, module_features(), enables) // c:296
}

/// Port of `boot_(m)` from `Src/Modules/mapfile.c:301`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:301
    // C body c:302-303 — `return 0`. Faithful empty-body port; the
    //                    $mapfile assoc-param registers via pd_list.
    0
}

/// Port of `cleanup_(m)` from `Src/Modules/mapfile.c:308`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                                  // c:308
    setfeatureenables(m, module_features(), None) // c:310
}

/// Port of `finish_(m)` from `Src/Modules/mapfile.c:315`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:315
    // C body c:316-317 — `return 0`. Faithful empty-body port.
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies the C c:228-234 `else` branch: `get_contents` returns
    /// NULL → caller treats as PM_UNSET (Option::None).
    #[test]
    fn getpmmapfile_nonexistent_returns_none() {
        assert!(getpmmapfile("/nonexistent/file/path/zshrs_mapfile").is_none());
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies the c:88 `open(O_RDWR|O_CREAT)` + c:97 `memcpy` +
    /// c:101 `msync` + c:106 `ftruncate` write path: a file written
    /// by `setpmmapfile` reads back identically through
    /// `get_contents`.
    #[test]
    fn file_roundtrip() {
        let test_file = "/tmp/zshrs_mapfile_test_roundtrip.txt";
        let content = "Hello, mapfile!";
        let _ = fs::remove_file(test_file);
        setpmmapfile(test_file, content, false);
        let read_content = get_contents(test_file).expect("file should exist");
        assert_eq!(read_content, content);
        let _ = fs::remove_file(test_file);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies the empty-content fast path skips the mmap block but
    /// still touches the file via `open(O_CREAT)`. C semantics:
    /// mmap(len=0) returns MAP_FAILED so the inner block is bypassed,
    /// but the open already created the file.
    #[test]
    fn empty_value_creates_file() {
        let test_file = "/tmp/zshrs_mapfile_test_empty.txt";
        let _ = fs::remove_file(test_file);
        setpmmapfile(test_file, "", false);
        assert!(Path::new(test_file).exists());
        let read_content = get_contents(test_file).expect("file should exist");
        assert!(read_content.is_empty());
        let _ = fs::remove_file(test_file);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies the c:255 `zreaddir(dir, 1)` walk with `ignoredots=1`
    /// — `.` / `..` never appear in the result, every value is `""`
    /// (c:263).
    #[test]
    fn scanpmmapfile_skips_dotdirs_and_returns_empty_values() {
        let entries = scanpmmapfile();
        for (name, val) in &entries {
            assert!(name != "." && name != "..");
            assert!(val.is_empty(),
                "scanpmmapfile values are always empty per c:263");
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies the c:134 `unlink(fname)` write — the unset callback
    /// removes the file when not readonly.
    #[test]
    fn unsetpmmapfile_removes_file() {
        let test_file = "/tmp/zshrs_mapfile_test_unset.txt";
        let _ = fs::write(test_file, "content");
        unsetpmmapfile(test_file, false);
        assert!(!Path::new(test_file).exists());
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies the c:133 readonly guard: a readonly param's unset
    /// callback skips the unlink.
    #[test]
    fn unsetpmmapfile_readonly_skips() {
        let test_file = "/tmp/zshrs_mapfile_test_unset_ro.txt";
        let _ = fs::write(test_file, "content");
        unsetpmmapfile(test_file, true);
        assert!(Path::new(test_file).exists());
        let _ = fs::remove_file(test_file);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies the c:87 readonly guard on `setpmmapfile`: write is
    /// skipped, file is not created.
    #[test]
    fn setpmmapfile_readonly_skips_write() {
        let test_file = "/tmp/zshrs_mapfile_test_set_ro.txt";
        let _ = fs::remove_file(test_file);
        setpmmapfile(test_file, "should not be written", true);
        assert!(!Path::new(test_file).exists());
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    /// Verifies `setpmmapfiles` (the bulk-set callback) routes each
    /// entry through `setpmmapfile` and respects the readonly guard
    /// at c:149.
    #[test]
    fn setpmmapfiles_writes_entries() {
        let f1 = "/tmp/zshrs_mapfile_bulk_1.txt";
        let f2 = "/tmp/zshrs_mapfile_bulk_2.txt";
        let _ = fs::remove_file(f1);
        let _ = fs::remove_file(f2);
        let entries = vec![
            (f1.to_string(), "one".to_string()),
            (f2.to_string(), "two".to_string()),
        ];
        setpmmapfiles(&entries, false);
        assert_eq!(get_contents(f1).as_deref(), Some("one"));
        assert_eq!(get_contents(f2).as_deref(), Some("two"));
        let _ = fs::remove_file(f1);
        let _ = fs::remove_file(f2);
    }
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/mapfile.c`.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 0,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 1,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/mapfile.c`.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["p:mapfile".to_string()]
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/mapfile.c`.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/mapfile.c`.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

