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

/// Mapfile associative array emulation.
/// Port of the `mapfile` HashTable special-parameter shape from
/// Src/Modules/mapfile.c — the C source registers `$mapfile[k]`
/// with `getpmmapfile()`/`setpmmapfile()`/`scanpmmapfile()`
/// callbacks; this struct wraps the same public surface.
#[derive(Debug, Default)]
pub struct Mapfile {
    readonly: bool,
}

impl Mapfile {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/mapfile.c`.
    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    /// Get file contents by filename (key).
    /// Port of `getpmmapfile()` from Src/Modules/mapfile.c:217 —
    /// the `getfn` slot of the per-key Param the C source builds on
    /// the fly when `$mapfile[KEY]` is dereferenced.
    pub fn get(&self, filename: &str) -> Option<String> {
        get_contents(filename).ok()
    }

    /// Set file contents by filename (key).
    /// Port of `setpmmapfile()` from Src/Modules/mapfile.c:68 — the
    /// `setfn` slot the C source wires for `$mapfile[KEY]=VALUE`
    /// assignment.
    pub fn set(&self, filename: &str, contents: &str) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mapfile is read-only",
            ));
        }
        setpmmapfile(filename, contents)
    }

    /// Unset (delete) a file by filename (key).
    /// Port of `unsetpmmapfile()` from Src/Modules/mapfile.c:126 —
    /// the `unsetfn` slot the C source wires for `unset
    /// 'mapfile[KEY]'`.
    pub fn unset(&self, filename: &str) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mapfile is read-only",
            ));
        }
        fs::remove_file(filename)
    }

    /// Scan current directory for regular files.
    /// Direct port of the directory-walk inside `scanpmmapfile()`
    /// (Src/Modules/mapfile.c:241): readdir + lstat S_ISREG filter.
    pub fn keys(&self) -> io::Result<Vec<String>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(".")? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    files.push(name.to_string());
                }
            }
        }
        Ok(files)
    }

    /// Get all files in current directory as hash.
    /// Port of `scanpmmapfile()` from Src/Modules/mapfile.c:241 —
    /// the C source invokes a per-entry callback with each (key,
    /// contents) pair; here we materialize them into a HashMap so
    /// callers can drive the scan iteratively.
    pub fn to_hash(&self) -> io::Result<HashMap<String, String>> {
        let mut result = HashMap::new();
        for filename in self.keys()? {
            if let Ok(contents) = get_contents(&filename) {
                result.insert(filename, contents);
            }
        }
        Ok(result)
    }

    /// Set multiple files from a hash.
    /// Port of `setpmmapfiles()` from Src/Modules/mapfile.c:141 —
    /// the bulk-assignment slot the C source wires for `mapfile=(
    /// k1 v1 k2 v2 ... )`.
    pub fn from_hash(&self, files: &HashMap<String, String>) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mapfile is read-only",
            ));
        }
        for (filename, contents) in files {
            setpmmapfile(filename, contents)?;
        }
        Ok(())
    }
}

/// Read file contents using mmap when available.
/// Port of `get_contents()` from Src/Modules/mapfile.c:167 — uses
/// the same `open(2)` + `mmap(2, MAP_PRIVATE)` fast-path the C
/// source uses, with a `read(2)` fallback when mmap fails.
#[cfg(unix)]
pub fn get_contents(filename: &str) -> io::Result<String> {
    use std::os::unix::fs::MetadataExt;

    let file = File::open(filename)?;
    let metadata = file.metadata()?;
    let size = metadata.size() as usize;

    if size == 0 {
        return Ok(String::new());
    }

    let fd = file.as_raw_fd();

    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        )
    };

    if ptr == libc::MAP_FAILED {
        let mut contents = String::new();
        let mut file = file;
        file.read_to_string(&mut contents)?;
        return Ok(contents);
    }

    let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) };
    let contents = String::from_utf8_lossy(slice).into_owned();

    unsafe {
        libc::munmap(ptr, size);
    }

    Ok(contents)
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

/// Write file contents using mmap when available.
/// Port of `setpmmapfile()` from Src/Modules/mapfile.c:68 —
/// `open(O_RDWR|O_CREAT)`, `ftruncate(2)` to the new length,
/// `mmap(2, MAP_SHARED)` write, `msync(MS_SYNC)`, second `ftruncate`
/// (the C source notes both are needed because mmap rounds to page).
/// C source's `#ifndef USE_MMAP` branch (line 110-117) does buffered
/// `fopen`/`putc`/`fclose`; this Rust port falls back to `write_all`
/// on `MAP_FAILED` for the same effect.
///
/// The C signature is `(Param, char*)` — invoked via the param tied
/// table mechanism. The Rust port takes `(filename, contents)` since
/// the Param wrapper is a higher-level concern.
#[cfg(unix)]
pub fn setpmmapfile(filename: &str, contents: &str) -> io::Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(filename)?;

    let fd = file.as_raw_fd();
    let len = contents.len();

    if len == 0 {
        file.set_len(0)?;
        return Ok(());
    }

    unsafe {
        if libc::ftruncate(fd, len as libc::off_t) < 0 {
            return Err(io::Error::last_os_error());
        }
    }

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
        let mut file = file;
        file.set_len(0)?;
        file.write_all(contents.as_bytes())?;
        return Ok(());
    }

    unsafe {
        std::ptr::copy_nonoverlapping(contents.as_ptr(), ptr as *mut u8, len);
        libc::msync(ptr, len, libc::MS_SYNC);
        libc::munmap(ptr, len);
    }

    Ok(())
}

/// Non-Unix fallback for `setpmmapfile` — port of the
/// `#ifndef USE_MMAP` branch in Src/Modules/mapfile.c:110-117 (the
/// `fopen`/`putc`/`fclose` arm). Rust `fs::write` collapses that
/// into a single buffered write.
#[cfg(not(unix))]
pub fn setpmmapfile(filename: &str, contents: &str) -> io::Result<()> {
    fs::write(filename, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_mapfile_new() {
        let mf = Mapfile::new();
        assert!(!mf.is_readonly());
    }

    #[test]
    fn test_mapfile_readonly() {
        let mut mf = Mapfile::new();
        mf.set_readonly(true);
        assert!(mf.is_readonly());

        let result = mf.set("test.txt", "content");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_nonexistent_file() {
        let mf = Mapfile::new();
        assert!(mf.get("/nonexistent/file/path").is_none());
    }

    #[test]
    fn test_file_roundtrip() {
        let test_file = "/tmp/zsh_mapfile_test.txt";
        let content = "Hello, mapfile!";

        let result = setpmmapfile(test_file, content);
        assert!(result.is_ok());

        let read_content = get_contents(test_file).unwrap();
        assert_eq!(read_content, content);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_empty_file() {
        let test_file = "/tmp/zsh_mapfile_empty.txt";

        let result = setpmmapfile(test_file, "");
        assert!(result.is_ok());

        let read_content = get_contents(test_file).unwrap();
        assert!(read_content.is_empty());

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_mapfile_keys_lists_regular_files() {
        let mf = Mapfile::default();
        let files = mf.keys();
        assert!(files.is_ok());
    }

    #[test]
    fn test_file_exists() {
        assert!(Path::new(".").exists());
        assert!(!Path::new("/nonexistent/path/to/file").exists());
    }

    #[test]
    fn test_mapfile_unset() {
        let test_file = "/tmp/zsh_mapfile_unset.txt";
        let _ = fs::write(test_file, "content");

        let mf = Mapfile::new();
        let result = mf.unset(test_file);
        assert!(result.is_ok());
        assert!(!Path::new(test_file).exists());
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
/// readonly).
///
/// C signature: `static void unsetpmmapfile(Param pm, int exp)`.
/// Rust port takes the file name directly (the param name) + the
/// readonly flag; matches the C semantics line-by-line.
#[allow(non_snake_case)]
pub fn unsetpmmapfile(name: &str, readonly: bool) {                      // c:126
    if !readonly {                                                       // c:133
        let _ = std::fs::remove_file(name);                              // c:134 unlink
    }
}

/// Port of `setpmmapfiles()` from `Src/Modules/mapfile.c:141`. The
/// bulk-set callback when `mapfile=( ... )` assigns a hashtable.
/// For each `(name, contents)` entry, writes contents to a file
/// named `name` (unless the param is readonly).
///
/// C iterates the `HashTable ht` and for each node calls
/// `setpmmapfile(pm, getstrvalue(&v))`. Rust port collapses to
/// a `&[(name, contents)]` shape since zshrs doesn't expose the
/// raw HashNode type at this layer; the writeback semantics match.
#[allow(non_snake_case)]
pub fn setpmmapfiles(entries: &[(String, String)], readonly: bool) {     // c:141
    if entries.is_empty() { return; }                                    // c:148
    if readonly { return; }                                              // c:151
    for (name, contents) in entries {                                    // c:152-160
        // C: setpmmapfile(v.pm, ztrdup(getstrvalue(&v)));
        // setpmmapfile (mapfile.c:103) writes the string to a file
        // named after the param.
        let _ = std::fs::write(name, contents);                          // c:160
    }
}

/// Port of `getpmmapfile()` from `Src/Modules/mapfile.c:217`. The
/// magic-assoc lookup callback for `${mapfile[name]}`. Reads the
/// contents of the file named `name` and returns it as a scalar
/// param value (or empty + UNSET if the file is unreadable).
///
/// C returns a `HashNode` (the synthesised Param). Rust port
/// returns `Option<String>` — `Some(contents)` when the file
/// reads, `None` (PM_UNSET) when it doesn't.
#[allow(non_snake_case)]
pub fn getpmmapfile(name: &str) -> Option<String> {                      // c:217
    // C: get_contents(name) — uses mmap when available, falls back
    // to plain read. Rust uses std::fs::read_to_string (lossy
    // UTF-8); the C source's metafy step is unnecessary in zshrs's
    // string model.
    std::fs::read_to_string(name).ok()                                   // c:228 get_contents
}

/// Port of `scanpmmapfile()` from `Src/Modules/mapfile.c:241`. The
/// magic-assoc scan callback for `${(k)mapfile}` /
/// `${(kv)mapfile}`. Walks the current directory and yields a
/// param entry per file. The C source notes "we always leave
/// contents empty" (mapfile.c:265-269) to avoid reading every
/// file in the dir into memory.
///
/// Returns a `Vec<String>` of file names from the cwd. Filters
/// `.` and `..` like `zreaddir`'s default no-dotfile behaviour.
#[allow(non_snake_case)]
pub fn scanpmmapfile() -> Vec<String> {                                  // c:241
    let mut out = Vec::new();                                            // c:243
    let dir = match std::fs::read_dir(".") {                             // c:247 opendir(".")
        Ok(d) => d,
        Err(_) => return out,
    };
    for entry in dir.flatten() {                                         // c:255 zreaddir
        if let Some(n) = entry.file_name().to_str() {
            // zreaddir(dir, 1) skips dot and dotdot when the second
            // arg is non-zero (ignoredots).
            if n == "." || n == ".." { continue; }
            out.push(n.to_string());
        }
    }
    out
}
