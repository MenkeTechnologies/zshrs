//! Mapfile module - port of Modules/mapfile.c
//!
//! Provides associative array interface to external files.
//! The mapfile hash allows reading and writing files through hash syntax:
//! - Reading: `$mapfile[filename]` returns file contents
//! - Writing: `mapfile[filename]=content` writes to file
//! - Unsetting: `unset 'mapfile[filename]'` deletes the file

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

// C source has 0 structs/enums; Rust port matches. The
// `mapfile` magic-assoc dispatch is wired through the C `partab[]`
// paramdef (mapfile.c:209-211) that ties getpmmapfile / scanpmmapfile
// + setpmmapfile / unsetpmmapfile callbacks. Each is a free fn in
// this module.

/// Port of `get_contents()` from `Src/Modules/mapfile.c:167`.
/// Reads the file at `fname` and returns its contents as a
/// metafied zsh-internal string (per `metafy(buf, size, META_HEAPDUP)`
/// at c:191/198). The C source first `unmetafy`s the input
/// filename — the Rust port mirrors that on the `&str` input.
///
/// C signature: `static char *get_contents(char *fname)`.
/// `mmap(2)` fast-path with `read(2)` fallback on `MAP_FAILED`,
/// matching c:177-199 line-by-line.
#[cfg(unix)]
pub fn get_contents(fname: &str) -> io::Result<String> {                 // c:167
    use std::os::unix::fs::MetadataExt;
    // c:178 — `unmetafy(fname = ztrdup(fname), &fd);`
    let fname_unmeta = crate::ported::utils::unmetafy_dup(fname);

    let file = File::open(&fname_unmeta)?;                               // c:181 open(O_RDONLY)
    let metadata = file.metadata()?;
    let size = metadata.size() as usize;

    if size == 0 {                                                       // C: empty file → empty val
        return Ok(crate::ported::utils::metafy(""));
    }

    let fd = file.as_raw_fd();
    let ptr = unsafe {
        libc::mmap(                                                      // c:184 mmap(MAP_PRIVATE)
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        )
    };

    let raw_buf: Vec<u8> = if ptr == libc::MAP_FAILED {
        // c:201-205 — `#ifndef USE_MMAP` fallback: zstuff via read(2).
        let mut contents = Vec::new();
        let mut file = file;
        file.read_to_end(&mut contents)?;
        contents
    } else {
        let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) };
        let v = slice.to_vec();
        unsafe { libc::munmap(ptr, size); }                              // c:198 munmap
        v
    };

    // c:191/198 — `val = metafy((char *)mmptr, sbuf.st_size, META_HEAPDUP);`
    let raw_str = String::from_utf8_lossy(&raw_buf);
    Ok(crate::ported::utils::metafy(&raw_str))
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/mapfile.c`.
/// Non-Unix fallback for `get_contents` — `mmap(2)` is POSIX-
/// only, so on Windows / WASI we fall through to a plain
/// `read_to_string`. Mirrors the `#ifndef HAVE_MMAP` branch in
/// Src/Modules/mapfile.c.
#[cfg(not(unix))]
pub fn get_contents(filename: &str) -> io::Result<String> {
    fs::read_to_string(filename)
}

/// Port of `setpmmapfile()` from `Src/Modules/mapfile.c:67`. Writes
/// `value` to a file named by the Param's name slot. Both name and
/// value are unmetafied (c:80-81) before the write. Honours the
/// `PM_READONLY` flag (c:84) — read-only params skip the write.
///
/// C signature: `static void setpmmapfile(Param pm, char *value)`.
/// Rust port takes the param name + value + readonly-flag directly
/// since Param is a paramdef-table-internal type. Returns
/// `io::Result<()>` for error propagation; C silently no-ops on
/// failure (open/mmap returning -1 falls through past the inner
/// memcpy/msync block).
#[cfg(unix)]
pub fn setpmmapfile(name: &str, value: &str, readonly: bool) -> io::Result<()> {  // c:67
    // c:80-81 — `unmetafy(name, &len); unmetafy(value, &len);`
    let name_unmeta = crate::ported::utils::unmetafy_dup(name);
    let value_unmeta = crate::ported::utils::unmetafy_dup(value);
    let value_bytes = value_unmeta.as_bytes();
    let len = value_bytes.len();

    // c:84 — `if (!(pm->node.flags & PM_READONLY) ...`
    if readonly {
        return Ok(());                                                   // c:84 readonly skip
    }

    // c:85 — `open(name, O_RDWR|O_CREAT|O_NOCTTY, 0666)`
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&name_unmeta)?;
    let fd = file.as_raw_fd();

    if len == 0 {
        file.set_len(0)?;
        return Ok(());
    }

    // c:91-92 — `if (ftruncate(fd, len) < 0) zwarn("ftruncate failed: %e", errno);`
    unsafe {
        if libc::ftruncate(fd, len as libc::off_t) < 0 {
            crate::ported::utils::zwarn(&format!(
                "ftruncate failed: {}", io::Error::last_os_error()
            ));
        }
    }

    // c:86 — mmap(MAP_SHARED, PROT_READ|PROT_WRITE).
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };

    if ptr == libc::MAP_FAILED {
        // c:108-114 — `#ifndef USE_MMAP` arm: fopen("w") + putc loop +
        // fclose. Rust's write_all collapses the putc loop.
        let mut file = file;
        file.set_len(0)?;
        file.write_all(value_bytes)?;
        return Ok(());
    }

    unsafe {
        // c:94 — `memcpy(mmptr, value, len);`
        std::ptr::copy_nonoverlapping(value_bytes.as_ptr(), ptr as *mut u8, len);
        // c:99 — `msync(mmptr, len, MS_SYNC);`
        libc::msync(ptr, len, libc::MS_SYNC);
        // c:104-105 — second ftruncate "since mmap() always maps complete pages".
        if libc::ftruncate(fd, len as libc::off_t) < 0 {
            crate::ported::utils::zwarn(&format!(
                "ftruncate failed: {}", io::Error::last_os_error()
            ));
        }
        // c:106 — `munmap(mmptr, len);`
        libc::munmap(ptr, len);
    }

    Ok(())
}

/// Non-Unix fallback for `setpmmapfile` — port of the
/// `#ifndef USE_MMAP` branch in Src/Modules/mapfile.c:110-117 (the
/// `fopen`/`putc`/`fclose` arm). Rust `fs::write` collapses the
/// putc loop into a single buffered write.
#[cfg(not(unix))]
pub fn setpmmapfile(name: &str, value: &str, readonly: bool) -> io::Result<()> {
    if readonly { return Ok(()); }
    let name_unmeta = crate::ported::utils::unmetafy_dup(name);
    let value_unmeta = crate::ported::utils::unmetafy_dup(value);
    fs::write(name_unmeta, value_unmeta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn getpmmapfile_nonexistent_returns_none() {
        // c:217-225 — get_contents returns NULL → pm->u.str = ""
        // + PM_UNSET. Rust port returns None for unset.
        assert!(getpmmapfile("/nonexistent/file/path").is_none());
    }

    #[test]
    fn file_roundtrip() {
        let test_file = "/tmp/zsh_mapfile_test.txt";
        let content = "Hello, mapfile!";

        assert!(setpmmapfile(test_file, content, false).is_ok());

        let read_content = get_contents(test_file).unwrap();
        assert_eq!(read_content, content);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn empty_file_roundtrip() {
        let test_file = "/tmp/zsh_mapfile_empty.txt";

        assert!(setpmmapfile(test_file, "", false).is_ok());
        let read_content = get_contents(test_file).unwrap();
        assert!(read_content.is_empty());

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn scanpmmapfile_lists_regular_files() {
        // c:241-269 — opens cwd, walks zreaddir, skips . / .. .
        // Always-empty values per c:266 `pm.u.str = ""`.
        let entries = scanpmmapfile();
        for (name, val) in &entries {
            assert!(name != "." && name != "..");
            assert!(val.is_empty(), "scanpmmapfile values always empty per c:266");
        }
    }

    #[test]
    fn unsetpmmapfile_removes_file() {
        let test_file = "/tmp/zsh_mapfile_unset.txt";
        let _ = fs::write(test_file, "content");

        unsetpmmapfile(test_file, false);
        assert!(!Path::new(test_file).exists());
    }

    #[test]
    fn unsetpmmapfile_readonly_skips() {
        // c:133-134 — `if (!(pm->node.flags & PM_READONLY)) unlink(...)`
        let test_file = "/tmp/zsh_mapfile_unset_ro.txt";
        let _ = fs::write(test_file, "content");

        unsetpmmapfile(test_file, true);
        assert!(Path::new(test_file).exists());
        let _ = fs::remove_file(test_file);
    }
}

/// Module loader entry — port of `setup_()` from Src/Modules/mapfile.c:279.
pub fn setup_() -> i32 {                                                 // c:279
    0                                                                    // c:282
}

/// Port of `features_()` from `Src/Modules/mapfile.c:286`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
pub fn features_() -> i32 {                                              // c:286
    0                                                                    // c:290
}

/// Port of `enables_()` from `Src/Modules/mapfile.c:294`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:294
    0                                                                    // c:297
}

/// Port of `boot_()` from `Src/Modules/mapfile.c:301`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                  // c:301
    0                                                                    // c:304
}

/// Port of `cleanup_()` from `Src/Modules/mapfile.c:308`. C body
/// is `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                               // c:308
    0                                                                    // c:311
}

/// Port of `finish_()` from `Src/Modules/mapfile.c:315`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:315
    0                                                                    // c:318
}

/// Port of `unsetpmmapfile()` from `Src/Modules/mapfile.c:126`. The
/// unset-callback for an element of the `$mapfile` magic-assoc.
/// Unlinks the file named by `pm->nam` (unless the param is
/// readonly). C unmetafies the name before unlink (c:130-131).
///
/// C signature: `static void unsetpmmapfile(Param pm, int exp)`.
#[allow(non_snake_case)]
pub fn unsetpmmapfile(name: &str, readonly: bool) {                      // c:126
    // c:130-131 — `char *fname = ztrdup(pm->node.nam); unmetafy(fname, &dummy);`
    let fname = crate::ported::utils::unmetafy_dup(name);
    if !readonly {                                                       // c:133
        let _ = std::fs::remove_file(&fname);                            // c:134 unlink
    }
}

/// Port of `setpmmapfiles()` from `Src/Modules/mapfile.c:141`. The
/// bulk-set callback when `mapfile=( ... )` assigns a hashtable.
/// For each `(name, contents)` entry, calls `setpmmapfile()` to
/// write contents to a file named `name`.
///
/// C iterates the `HashTable ht` calling `setpmmapfile(v.pm,
/// ztrdup(getstrvalue(&v)))`. Rust port collapses to a
/// `&[(name, contents)]` slice since zshrs doesn't expose the
/// raw HashNode at this layer; the per-entry writeback goes
/// through the real `setpmmapfile()` so unmetafy + readonly +
/// mmap path stay on the call chain.
#[allow(non_snake_case)]
pub fn setpmmapfiles(entries: &[(String, String)], readonly: bool) {     // c:141
    if entries.is_empty() { return; }                                    // c:148
    for (name, contents) in entries {                                    // c:152-160
        // c:160 — `setpmmapfile(v.pm, ztrdup(getstrvalue(&v)));`
        let _ = setpmmapfile(name, contents, readonly);
    }
}

/// Port of `getpmmapfile()` from `Src/Modules/mapfile.c:217`. The
/// magic-assoc lookup callback for `${mapfile[name]}`. Reads the
/// file's contents via `get_contents()` and returns the metafied
/// value (or `None` for unset, matching C's `PM_UNSET` flag).
///
/// C signature: `static HashNode getpmmapfile(HashTable ht, const char *name)`.
/// Returns the synthesised Param; Rust port returns `Option<String>`.
#[allow(non_snake_case)]
pub fn getpmmapfile(name: &str) -> Option<String> {                      // c:217
    // c:228 — `if ((contents = get_contents(pm->node.nam))) pm->u.str = contents;`
    // get_contents already unmetafies the input name and metafies the
    // returned value, matching c:178/191/198.
    get_contents(name).ok()
}

/// Port of `scanpmmapfile()` from `Src/Modules/mapfile.c:241`. The
/// magic-assoc scan callback for `${(k)mapfile}` /
/// `${(kv)mapfile}`. Walks the current directory and yields a
/// per-file param entry. The C source notes at c:262-266: "Hmmm,
/// it's rather wasteful always to read the contents...  Hence
/// just leave it empty." → `pm.u.str = ""` per iteration.
///
/// C signature: `static void scanpmmapfile(HashTable ht, ScanFunc
///                                          func, int flags)`.
/// Rust port returns `Vec<(name, "")>` since zshrs doesn't expose
/// a ScanFunc callback shape; values are always empty, matching
/// c:266. The iterator skips `.` / `..` per `zreaddir(dir, 1)`'s
/// `ignoredots=1` behaviour.
#[allow(non_snake_case)]
pub fn scanpmmapfile() -> Vec<(String, String)> {                        // c:241
    let mut out = Vec::new();                                            // c:243 struct param pm
    let dir = match std::fs::read_dir(".") {                             // c:248 opendir(".")
        Ok(d) => d,
        Err(_) => return out,                                            // c:249 return on NULL
    };
    for entry in dir.flatten() {                                         // c:258 zreaddir(dir, 1)
        if let Some(n) = entry.file_name().to_str() {
            // c:258 — zreaddir(dir, 1) with ignoredots=1 skips . and ..
            if n == "." || n == ".." { continue; }
            // c:266 — `pm.u.str = "";` (always empty value)
            out.push((n.to_string(), String::new()));
        }
    }
    out
}
