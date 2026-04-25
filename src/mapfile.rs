//! Mapfile module - port of Modules/mapfile.c
//!
//! Provides associative array interface to external files.
//! The mapfile hash allows reading and writing files through hash syntax:
//! - Reading: $mapfile[filename] returns file contents
//! - Writing: mapfile[filename]=content writes to file
//! - Unsetting: unset 'mapfile[filename]' deletes the file

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

/// Mapfile associative array emulation
#[derive(Debug, Default)]
pub struct Mapfile {
    readonly: bool,
}

impl Mapfile {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }

    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    /// Get file contents by filename (key)
    pub fn get(&self, filename: &str) -> Option<String> {
        get_file_contents(filename).ok()
    }

    /// Set file contents by filename (key)
    pub fn set(&self, filename: &str, contents: &str) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mapfile is read-only",
            ));
        }
        set_file_contents(filename, contents)
    }

    /// Unset (delete) a file by filename (key)
    pub fn unset(&self, filename: &str) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mapfile is read-only",
            ));
        }
        fs::remove_file(filename)
    }

    /// Scan current directory for files
    pub fn keys(&self) -> io::Result<Vec<String>> {
        scan_directory(".")
    }

    /// Get all files in current directory as hash
    pub fn to_hash(&self) -> io::Result<HashMap<String, String>> {
        let mut result = HashMap::new();
        for filename in self.keys()? {
            if let Ok(contents) = get_file_contents(&filename) {
                result.insert(filename, contents);
            }
        }
        Ok(result)
    }

    /// Set multiple files from a hash
    pub fn from_hash(&self, files: &HashMap<String, String>) -> io::Result<()> {
        if self.readonly {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "mapfile is read-only",
            ));
        }
        for (filename, contents) in files {
            set_file_contents(filename, contents)?;
        }
        Ok(())
    }
}

/// Read file contents using mmap when available
#[cfg(unix)]
pub fn get_file_contents(filename: &str) -> io::Result<String> {
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

#[cfg(not(unix))]
pub fn get_file_contents(filename: &str) -> io::Result<String> {
    fs::read_to_string(filename)
}

/// Write file contents using mmap when available
#[cfg(unix)]
pub fn set_file_contents(filename: &str, contents: &str) -> io::Result<()> {
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

#[cfg(not(unix))]
pub fn set_file_contents(filename: &str, contents: &str) -> io::Result<()> {
    fs::write(filename, contents)
}

/// Scan directory for regular files
pub fn scan_directory(dir: &str) -> io::Result<Vec<String>> {
    let mut files = Vec::new();

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(name) = path.file_name() {
                if let Some(name_str) = name.to_str() {
                    files.push(name_str.to_string());
                }
            }
        }
    }

    Ok(files)
}

/// Check if a file exists
pub fn file_exists(filename: &str) -> bool {
    Path::new(filename).exists()
}

/// Get file size
pub fn file_size(filename: &str) -> io::Result<u64> {
    Ok(fs::metadata(filename)?.len())
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

        let result = set_file_contents(test_file, content);
        assert!(result.is_ok());

        let read_content = get_file_contents(test_file).unwrap();
        assert_eq!(read_content, content);

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_empty_file() {
        let test_file = "/tmp/zsh_mapfile_empty.txt";

        let result = set_file_contents(test_file, "");
        assert!(result.is_ok());

        let read_content = get_file_contents(test_file).unwrap();
        assert!(read_content.is_empty());

        let _ = fs::remove_file(test_file);
    }

    #[test]
    fn test_scan_directory() {
        let files = scan_directory(".");
        assert!(files.is_ok());
    }

    #[test]
    fn test_file_exists() {
        assert!(file_exists("."));
        assert!(!file_exists("/nonexistent/path/to/file"));
    }

    #[test]
    fn test_mapfile_unset() {
        let test_file = "/tmp/zsh_mapfile_unset.txt";
        let _ = fs::write(test_file, "content");

        let mf = Mapfile::new();
        let result = mf.unset(test_file);
        assert!(result.is_ok());
        assert!(!file_exists(test_file));
    }
}
