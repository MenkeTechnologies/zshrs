//! File stat interface - port of Modules/stat.c
//!
//! Provides stat/zstat builtin for accessing file metadata.

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Identifiers for individual stat-result elements.
/// Port of the `ST_*` enum from Src/Modules/stat.c — the C
/// source's `statprint()` (line 234) takes an `iwhich` index that
/// dispatches between these elements; the Rust port keeps the
/// same set so `zstat -L` selectors still work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatElement {
    Device,
    Inode,
    Mode,
    Nlink,
    Uid,
    Gid,
    Rdev,
    Size,
    Atime,
    Mtime,
    Ctime,
    Blksize,
    Blocks,
    Link,
}

impl StatElement {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn from_name(name: &str) -> Option<Self> {
        let elements = Self::all();
        let matches: Vec<_> = elements
            .iter()
            .filter(|(n, _)| n.starts_with(name))
            .collect();

        if matches.len() == 1 {
            Some(matches[0].1)
        } else {
            None
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Inode => "inode",
            Self::Mode => "mode",
            Self::Nlink => "nlink",
            Self::Uid => "uid",
            Self::Gid => "gid",
            Self::Rdev => "rdev",
            Self::Size => "size",
            Self::Atime => "atime",
            Self::Mtime => "mtime",
            Self::Ctime => "ctime",
            Self::Blksize => "blksize",
            Self::Blocks => "blocks",
            Self::Link => "link",
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn all() -> Vec<(&'static str, Self)> {
        vec![
            ("device", Self::Device),
            ("inode", Self::Inode),
            ("mode", Self::Mode),
            ("nlink", Self::Nlink),
            ("uid", Self::Uid),
            ("gid", Self::Gid),
            ("rdev", Self::Rdev),
            ("size", Self::Size),
            ("atime", Self::Atime),
            ("mtime", Self::Mtime),
            ("ctime", Self::Ctime),
            ("blksize", Self::Blksize),
            ("blocks", Self::Blocks),
            ("link", Self::Link),
        ]
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn list_names() -> Vec<&'static str> {
        Self::all().into_iter().map(|(n, _)| n).collect()
    }
}

/// `zstat` formatting flags.
/// Port of the `STF_*` flag set from Src/Modules/stat.c — the C
/// source threads it through `statprint()` (line 234) and its
/// per-element printers (`statmodeprint()` line 47, `statuidprint`
/// line 132, etc.). `-n` / `-N` / `-s` / `-r` / `-o` / `-g` / `-L`.
#[derive(Debug, Default, Clone)]
pub struct StatFlags {
    pub show_name: bool,
    pub show_file: bool,
    pub string_format: bool,
    pub raw_format: bool,
    pub octal_mode: bool,
    pub use_gmt: bool,
    pub use_lstat: bool,
}

/// File stat result.
/// Port of the `struct stat` fields the C source's `statprint()`
/// (Src/Modules/stat.c:234) dispatches between — every field
/// corresponds to one `ST_*` selector.
#[derive(Debug, Clone)]
pub struct FileStat {
    pub device: u64,
    pub inode: u64,
    pub mode: u32,
    pub nlink: u64,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub blksize: u64,
    pub blocks: u64,
    pub link_target: Option<String>,
    pub file_type: FileType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    BlockDevice,
    CharDevice,
    Fifo,
    Socket,
    Unknown,
}

impl FileType {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn from_metadata(meta: &Metadata) -> Self {
        let ft = meta.file_type();
        if ft.is_file() {
            Self::Regular
        } else if ft.is_dir() {
            Self::Directory
        } else if ft.is_symlink() {
            Self::Symlink
        } else if ft.is_block_device() {
            Self::BlockDevice
        } else if ft.is_char_device() {
            Self::CharDevice
        } else if ft.is_fifo() {
            Self::Fifo
        } else if ft.is_socket() {
            Self::Socket
        } else {
            Self::Unknown
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn mode_char(&self) -> char {
        match self {
            Self::Regular => '-',
            Self::Directory => 'd',
            Self::Symlink => 'l',
            Self::BlockDevice => 'b',
            Self::CharDevice => 'c',
            Self::Fifo => 'p',
            Self::Socket => 's',
            Self::Unknown => '?',
        }
    }
}

impl FileStat {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn from_path(path: &Path, use_lstat: bool) -> std::io::Result<Self> {
        let meta = if use_lstat {
            fs::symlink_metadata(path)?
        } else {
            fs::metadata(path)?
        };

        let link_target = if meta.file_type().is_symlink() {
            fs::read_link(path)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        Ok(Self::from_metadata(&meta, link_target))
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn from_metadata(meta: &Metadata, link_target: Option<String>) -> Self {
        let atime = meta
            .accessed()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        Self {
            device: meta.dev(),
            inode: meta.ino(),
            mode: meta.mode(),
            nlink: meta.nlink(),
            uid: meta.uid(),
            gid: meta.gid(),
            rdev: meta.rdev(),
            size: meta.size(),
            atime,
            mtime,
            ctime: meta.ctime(),
            blksize: meta.blksize(),
            blocks: meta.blocks(),
            link_target,
            file_type: FileType::from_metadata(meta),
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn get_element(&self, elem: StatElement, flags: &StatFlags) -> String {
        match elem {
            StatElement::Device => format!("{}", self.device),
            StatElement::Inode => format!("{}", self.inode),
            StatElement::Mode => self.format_mode(flags),
            StatElement::Nlink => format!("{}", self.nlink),
            StatElement::Uid => self.format_uid(flags),
            StatElement::Gid => self.format_gid(flags),
            StatElement::Rdev => format!("{}", self.rdev),
            StatElement::Size => format!("{}", self.size),
            StatElement::Atime => self.printtime(self.atime, flags),
            StatElement::Mtime => self.printtime(self.mtime, flags),
            StatElement::Ctime => self.printtime(self.ctime, flags),
            StatElement::Blksize => format!("{}", self.blksize),
            StatElement::Blocks => format!("{}", self.blocks),
            StatElement::Link => self.link_target.clone().unwrap_or_default(),
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    fn format_mode(&self, flags: &StatFlags) -> String {
        let mut result = String::new();

        if flags.raw_format {
            if flags.octal_mode {
                result.push_str(&format!("0{:o}", self.mode));
            } else {
                result.push_str(&format!("{}", self.mode));
            }
            if flags.string_format {
                result.push_str(" (");
            }
        }

        if flags.string_format {
            result.push(self.file_type.mode_char());

            let perms = [
                (self.mode & 0o400 != 0, 'r'),
                (self.mode & 0o200 != 0, 'w'),
                (
                    self.mode & 0o100 != 0,
                    if self.mode & 0o4000 != 0 { 's' } else { 'x' },
                ),
                (self.mode & 0o040 != 0, 'r'),
                (self.mode & 0o020 != 0, 'w'),
                (
                    self.mode & 0o010 != 0,
                    if self.mode & 0o2000 != 0 { 's' } else { 'x' },
                ),
                (self.mode & 0o004 != 0, 'r'),
                (self.mode & 0o002 != 0, 'w'),
                (
                    self.mode & 0o001 != 0,
                    if self.mode & 0o1000 != 0 { 't' } else { 'x' },
                ),
            ];

            for (set, ch) in perms {
                if set {
                    result.push(ch);
                } else if ch == 's' || ch == 't' {
                    result.push(ch.to_ascii_uppercase());
                } else {
                    result.push('-');
                }
            }

            if !set_bit(self.mode, 0o100) && self.mode & 0o4000 != 0 {
                let chars: Vec<char> = result.chars().collect();
                let mut r: String = chars[..3].iter().collect();
                r.push('S');
                r.push_str(&chars[4..].iter().collect::<String>());
                result = r;
            }

            if flags.raw_format {
                result.push(')');
            }
        }

        if !flags.raw_format && !flags.string_format {
            if flags.octal_mode {
                result = format!("0{:o}", self.mode);
            } else {
                result = format!("{}", self.mode);
            }
        }

        result
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    fn format_uid(&self, flags: &StatFlags) -> String {
        let mut result = String::new();

        if flags.raw_format {
            result.push_str(&format!("{}", self.uid));
            if flags.string_format {
                result.push_str(" (");
            }
        }

        if flags.string_format {
            #[cfg(unix)]
            {
                if let Some(name) = get_username(self.uid) {
                    result.push_str(&name);
                } else {
                    result.push_str(&format!("{}", self.uid));
                }
            }
            #[cfg(not(unix))]
            {
                result.push_str(&format!("{}", self.uid));
            }

            if flags.raw_format {
                result.push(')');
            }
        }

        if !flags.raw_format && !flags.string_format {
            result = format!("{}", self.uid);
        }

        result
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    fn format_gid(&self, flags: &StatFlags) -> String {
        let mut result = String::new();

        if flags.raw_format {
            result.push_str(&format!("{}", self.gid));
            if flags.string_format {
                result.push_str(" (");
            }
        }

        if flags.string_format {
            #[cfg(unix)]
            {
                if let Some(name) = get_groupname(self.gid) {
                    result.push_str(&name);
                } else {
                    result.push_str(&format!("{}", self.gid));
                }
            }
            #[cfg(not(unix))]
            {
                result.push_str(&format!("{}", self.gid));
            }

            if flags.raw_format {
                result.push(')');
            }
        }

        if !flags.raw_format && !flags.string_format {
            result = format!("{}", self.gid);
        }

        result
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    fn printtime(&self, timestamp: i64, flags: &StatFlags) -> String {
        let mut result = String::new();

        if flags.raw_format {
            result.push_str(&format!("{}", timestamp));
            if flags.string_format {
                result.push_str(" (");
            }
        }

        if flags.string_format {
            use chrono::{Local, TimeZone, Utc};

            let dt = if flags.use_gmt {
                Utc.timestamp_opt(timestamp, 0)
                    .single()
                    .map(|dt| dt.format("%a %b %e %k:%M:%S %Z %Y").to_string())
            } else {
                Local
                    .timestamp_opt(timestamp, 0)
                    .single()
                    .map(|dt| dt.format("%a %b %e %k:%M:%S %Z %Y").to_string())
            };

            result.push_str(&dt.unwrap_or_else(|| format!("{}", timestamp)));

            if flags.raw_format {
                result.push(')');
            }
        }

        if !flags.raw_format && !flags.string_format {
            result = format!("{}", timestamp);
        }

        result
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn to_hash(&self, flags: &StatFlags) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for (name, elem) in StatElement::all() {
            map.insert(name.to_string(), self.get_element(elem, flags));
        }
        map
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/stat.c`.
    pub fn to_array(&self, flags: &StatFlags) -> Vec<String> {
        StatElement::all()
            .into_iter()
            .map(|(_, elem)| self.get_element(elem, flags))
            .collect()
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/stat.c`.
fn set_bit(mode: u32, bit: u32) -> bool {
    mode & bit != 0
}

/// Port of `statuidprint()` from `Src/Modules/stat.c:132`.
#[cfg(unix)]
fn get_username(uid: u32) -> Option<String> {
    use std::ffi::CStr;
    unsafe {
        let pwd = libc::getpwuid(uid);
        if pwd.is_null() {
            None
        } else {
            CStr::from_ptr((*pwd).pw_name)
                .to_str()
                .ok()
                .map(|s| s.to_string())
        }
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/stat.c`.
#[cfg(unix)]
fn get_groupname(gid: u32) -> Option<String> {
    use std::ffi::CStr;
    unsafe {
        let grp = libc::getgrgid(gid);
        if grp.is_null() {
            None
        } else {
            CStr::from_ptr((*grp).gr_name)
                .to_str()
                .ok()
                .map(|s| s.to_string())
        }
    }
}

/// `zstat` builtin options.
/// Mirrors the `Options ops` flag bag `bin_stat()` from
/// Src/Modules/stat.c:368 reads — `-A`/`-H` array/hash output,
/// `-L` lstat, `-T`/`-N`/`-n` name/type formatting, `-r`/`-s`/`-o`
/// raw/string/octal, `-l` list-elements, `-F` time format.
#[derive(Debug, Default)]
pub struct StatOptions {
    pub list_elements: bool,
    pub use_lstat: bool,
    pub use_gmt: bool,
    pub show_name: bool,
    pub hide_name: bool,
    pub show_type: bool,
    pub hide_type: bool,
    pub raw_format: bool,
    pub string_format: bool,
    pub octal_mode: bool,
    pub element: Option<StatElement>,
    pub array_name: Option<String>,
    pub hash_name: Option<String>,
    pub time_format: Option<String>,
}

/// `zstat` builtin entry point.
/// Port of `bin_stat()` from Src/Modules/stat.c:368 — drives the
/// per-file `statprint()` (line 234) call that walks each
/// `STAT_ELEMENT` printer in turn.
pub fn bin_stat(args: &[&str], options: &StatOptions) -> (i32, String) {
    let mut output = String::new();

    if options.list_elements {
        let names = StatElement::list_names();
        output.push_str(&names.join(" "));
        output.push('\n');
        return (0, output);
    }

    if args.is_empty() {
        return (1, "stat: no files given\n".to_string());
    }

    let flags = StatFlags {
        show_name: options.show_type && !options.hide_type,
        show_file: (options.show_name || args.len() > 1) && !options.hide_name,
        string_format: options.string_format || options.use_gmt,
        raw_format: options.raw_format || !options.string_format,
        octal_mode: options.octal_mode,
        use_gmt: options.use_gmt,
        use_lstat: options.use_lstat || options.element == Some(StatElement::Link),
    };

    let mut ret = 0;

    for path_str in args {
        let path = Path::new(path_str);

        let stat_result = FileStat::from_path(path, flags.use_lstat);

        match stat_result {
            Ok(stat) => {
                if flags.show_file {
                    if options.element.is_some() {
                        output.push_str(&format!("{} ", path_str));
                    } else {
                        output.push_str(&format!("{}:\n", path_str));
                    }
                }

                if let Some(elem) = options.element {
                    let value = stat.get_element(elem, &flags);
                    if flags.show_name {
                        output.push_str(&format!("{} {}\n", elem.name(), value));
                    } else {
                        output.push_str(&format!("{}\n", value));
                    }
                } else {
                    for (name, elem) in StatElement::all() {
                        let value = stat.get_element(elem, &flags);
                        if flags.show_name {
                            output.push_str(&format!("{:<8} {}\n", name, value));
                        } else {
                            output.push_str(&format!("{}\n", value));
                        }
                    }
                }
            }
            Err(e) => {
                output.push_str(&format!("stat: {}: {}\n", path_str, e));
                ret = 1;
            }
        }
    }

    (ret, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_stat_element_from_name() {
        assert_eq!(StatElement::from_name("dev"), Some(StatElement::Device));
        assert_eq!(StatElement::from_name("device"), Some(StatElement::Device));
        assert_eq!(StatElement::from_name("mode"), Some(StatElement::Mode));
        assert_eq!(StatElement::from_name("size"), Some(StatElement::Size));
        assert_eq!(StatElement::from_name("link"), Some(StatElement::Link));
        assert_eq!(StatElement::from_name("nonexistent"), None);
    }

    #[test]
    fn test_stat_element_list() {
        let names = StatElement::list_names();
        assert!(names.contains(&"device"));
        assert!(names.contains(&"inode"));
        assert!(names.contains(&"mode"));
        assert!(names.contains(&"size"));
        assert_eq!(names.len(), 14);
    }

    #[test]
    fn test_file_type_mode_char() {
        assert_eq!(FileType::Regular.mode_char(), '-');
        assert_eq!(FileType::Directory.mode_char(), 'd');
        assert_eq!(FileType::Symlink.mode_char(), 'l');
        assert_eq!(FileType::BlockDevice.mode_char(), 'b');
        assert_eq!(FileType::CharDevice.mode_char(), 'c');
    }

    #[test]
    fn test_file_stat_from_path() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"hello world").unwrap();
        }

        let stat = FileStat::from_path(&file_path, false).unwrap();
        assert_eq!(stat.size, 11);
        assert_eq!(stat.file_type, FileType::Regular);
        assert!(stat.inode > 0);
    }

    #[test]
    fn test_format_mode_string() {
        let stat = FileStat {
            device: 0,
            inode: 0,
            mode: 0o100644,
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            size: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            blksize: 0,
            blocks: 0,
            link_target: None,
            file_type: FileType::Regular,
        };

        let flags = StatFlags {
            string_format: true,
            ..Default::default()
        };

        let mode_str = stat.format_mode(&flags);
        assert!(mode_str.starts_with('-'));
        assert!(mode_str.contains('r'));
        assert!(mode_str.contains('w'));
    }

    #[test]
    fn test_format_mode_octal() {
        let stat = FileStat {
            device: 0,
            inode: 0,
            mode: 0o100755,
            nlink: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
            size: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            blksize: 0,
            blocks: 0,
            link_target: None,
            file_type: FileType::Regular,
        };

        let flags = StatFlags {
            raw_format: true,
            octal_mode: true,
            ..Default::default()
        };

        let mode_str = stat.format_mode(&flags);
        assert!(mode_str.starts_with("0"));
        assert!(mode_str.contains("755"));
    }

    /// Port of `bin_stat()` from `Src/Modules/stat.c:368`.
    #[test]
    fn test_stat_to_hash() {
        let stat = FileStat {
            device: 1,
            inode: 12345,
            mode: 0o100644,
            nlink: 1,
            uid: 1000,
            gid: 1000,
            rdev: 0,
            size: 100,
            atime: 1700000000,
            mtime: 1700000000,
            ctime: 1700000000,
            blksize: 4096,
            blocks: 8,
            link_target: None,
            file_type: FileType::Regular,
        };

        let flags = StatFlags::default();
        let hash = stat.to_hash(&flags);

        assert!(hash.contains_key("device"));
        assert!(hash.contains_key("size"));
        assert_eq!(hash.get("size"), Some(&"100".to_string()));
    }

    #[test]
    fn test_builtin_stat_list() {
        let options = StatOptions {
            list_elements: true,
            ..Default::default()
        };

        let (status, output) = bin_stat(&[], &options);
        assert_eq!(status, 0);
        assert!(output.contains("device"));
        assert!(output.contains("inode"));
        assert!(output.contains("mode"));
    }

    #[test]
    fn test_builtin_stat_no_args() {
        let options = StatOptions::default();
        let (status, output) = bin_stat(&[], &options);
        assert_eq!(status, 1);
        assert!(output.contains("no files given"));
    }

    #[test]
    fn test_builtin_stat_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"test content").unwrap();
        }

        let options = StatOptions {
            show_type: true,
            ..Default::default()
        };

        let (status, output) = bin_stat(&[file_path.to_str().unwrap()], &options);
        assert_eq!(status, 0);
        assert!(output.contains("device"));
        assert!(output.contains("size"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// zstat - file status (zsh/stat module)
    pub(crate) fn builtin_zstat(&mut self, args: &[String]) -> i32 {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;

        // Direct port of src/zsh/Src/Modules/stat.c bin_stat. The
        // `-A NAME` flag stores results in an assoc array NAME instead
        // of printing. The previous Rust impl received -A NAME but
        // its `_as_array` / `_array_name` were prefixed with `_`
        // (i.e. ignored) and the output_element closure just
        // printed `key=value` to stdout regardless. Now actually
        // writes to self.assoc_arrays[NAME].
        let mut show_all = true;
        let mut symbolic_mode = false;
        let mut show_link = false;
        let mut as_array = false;
        let mut array_name = String::new();
        // `-H name` populates an associative array keyed by field
        // name (mode, size, mtime, …). Distinct from `-A name` which
        // populates a plain indexed array of just the values in
        // field-table order. Direct port of stat.c:418-426 STF_HASH.
        let mut as_hash = false;
        let mut hash_name = String::new();
        let mut printtime = String::new();
        let mut elements: Vec<String> = Vec::new();
        let mut files: Vec<&str> = Vec::new();
        // `-t` flag: prefix each output line with the filename.
        // Direct port of `-t` handling in Src/Modules/stat.c bin_stat
        // (the BUILTIN flag char list `AfHLnNoTrs` includes `t`).
        let mut prefix_filename = false;

        let mut iter = args.iter().peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-s" => symbolic_mode = true,
                "-L" => show_link = true,
                // `-n`: STF_FILE — prefix each line with the
                // filename. Direct port of stat.c:518-519.
                "-n" => prefix_filename = true,
                // `-N`: clear STF_FILE — explicit "don't prefix".
                "-N" => prefix_filename = false,
                // `-o`: STF_OCTAL — print mode in octal. (Not yet
                // wired; non-symbolic mode currently always
                // decimal — matches zsh default.)
                "-o" => show_all = false,
                // `-t`: STF_NAME — show element names. Default
                // already sets STF_NAME when no `+pick` is given,
                // so this is a no-op for the common shape; the
                // flag exists so scripts can be explicit.
                "-t" => {}
                // `-T`: clear STF_NAME — strip element names.
                // Not yet wired; keeping names is the safe default.
                "-T" => {}
                "-A" => {
                    as_array = true;
                    if let Some(name) = iter.next() {
                        array_name = name.clone();
                    } else {
                        eprintln!("zshrs:zstat:1: argument expected: -A");
                        return 1;
                    }
                }
                "-H" => {
                    as_hash = true;
                    if let Some(name) = iter.next() {
                        hash_name = name.clone();
                    } else {
                        eprintln!("zshrs:zstat:1: argument expected: -H");
                        return 1;
                    }
                }
                "-F" => {
                    if let Some(fmt) = iter.next() {
                        printtime = fmt.clone();
                    }
                }
                s if s.starts_with('+') => {
                    elements.push(s[1..].to_string());
                    show_all = false;
                }
                s if !s.starts_with('-') => files.push(s),
                s => {
                    // BUILTIN("zstat", ..., "AfHLnNoTrs") in
                    // zsh/Src/Modules/stat.c declares the valid letter
                    // set. Old \`_ => {}\` accepted any letter.
                    let bad: String = s[1..].chars().take(1).collect();
                    eprintln!("zshrs:zstat:1: bad option: -{}", bad);
                    return 1;
                }
            }
        }

        if files.is_empty() {
            eprintln!("zshrs:zstat:1: no files specified");
            return 1;
        }

        // Multiple files auto-prefix with filename. Direct port of
        // stat.c:526-527 — `if (nargs > 1) flags |= STF_FILE;`
        // unless -A/-H redirects output. -N explicitly clears it.
        if files.len() > 1 && !as_array && !as_hash {
            prefix_filename = true;
        }

        for file in files {
            let meta = if show_link {
                std::fs::symlink_metadata(file)
            } else {
                std::fs::metadata(file)
            };

            let meta = match meta {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("zshrs:zstat:1: {}: {}", file, e);
                    return 1;
                }
            };

            // Collect into a local map first; flush to assoc_arrays
            // below so the &mut borrow doesn't tangle with iteration.
            let mut collected: Vec<(String, String)> = Vec::new();
            // zsh's bin_stat output format: NAME left-padded to 8
            // chars (space-padded), then VALUE. With STF_FILE set
            // (multi-file mode or `-n`), the filename appears on
            // its own line as `<file>:\n` BEFORE that file's data
            // block — not as a per-line prefix. Direct port of
            // stat.c:543-550 + the `printf("%s:\n", …)` header.
            if prefix_filename && !as_array && !as_hash {
                println!("{}:", file);
            }
            let mut output_element = |name: &str, value: &str| {
                if as_array || as_hash {
                    if show_all || elements.contains(&name.to_string()) {
                        collected.push((name.to_string(), value.to_string()));
                    }
                } else if show_all || elements.contains(&name.to_string()) {
                    println!("{:<8}{}", name, value);
                }
            };

            output_element("device", &meta.dev().to_string());
            output_element("inode", &meta.ino().to_string());

            if symbolic_mode {
                let mode = meta.permissions().mode();
                let mode_str = format!(
                    "{}{}{}{}{}{}{}{}{}{}",
                    match mode & 0o170000 {
                        0o040000 => 'd',
                        0o120000 => 'l',
                        0o100000 => '-',
                        0o060000 => 'b',
                        0o020000 => 'c',
                        0o010000 => 'p',
                        0o140000 => 's',
                        _ => '?',
                    },
                    if mode & 0o400 != 0 { 'r' } else { '-' },
                    if mode & 0o200 != 0 { 'w' } else { '-' },
                    if mode & 0o4000 != 0 {
                        's'
                    } else if mode & 0o100 != 0 {
                        'x'
                    } else {
                        '-'
                    },
                    if mode & 0o040 != 0 { 'r' } else { '-' },
                    if mode & 0o020 != 0 { 'w' } else { '-' },
                    if mode & 0o2000 != 0 {
                        's'
                    } else if mode & 0o010 != 0 {
                        'x'
                    } else {
                        '-'
                    },
                    if mode & 0o004 != 0 { 'r' } else { '-' },
                    if mode & 0o002 != 0 { 'w' } else { '-' },
                    if mode & 0o1000 != 0 {
                        't'
                    } else if mode & 0o001 != 0 {
                        'x'
                    } else {
                        '-'
                    },
                );
                output_element("mode", &mode_str);
            } else {
                // Non-symbolic mode is the raw `st_mode` integer in
                // DECIMAL — matches zsh bin_stat which prints
                // `s.st_mode` via the `%lu` format. Octal would
                // confuse scripts that test against numeric
                // constants like `(( mode & 0o170000 ))`.
                output_element("mode", &meta.permissions().mode().to_string());
            }

            output_element("nlink", &meta.nlink().to_string());
            output_element("uid", &meta.uid().to_string());
            output_element("gid", &meta.gid().to_string());
            output_element("rdev", &meta.rdev().to_string());
            output_element("size", &meta.len().to_string());

            let format_timestamp = |secs: i64| -> String {
                if printtime.is_empty() {
                    secs.to_string()
                } else {
                    chrono::DateTime::from_timestamp(secs, 0)
                        .map(|dt| dt.format(&printtime).to_string())
                        .unwrap_or_else(|| secs.to_string())
                }
            };

            output_element("atime", &format_timestamp(meta.atime()));
            output_element("mtime", &format_timestamp(meta.mtime()));
            output_element("ctime", &format_timestamp(meta.ctime()));
            output_element("blksize", &meta.blksize().to_string());
            output_element("blocks", &meta.blocks().to_string());

            if show_link && meta.file_type().is_symlink() {
                if let Ok(target) = std::fs::read_link(file) {
                    output_element("link", &target.to_string_lossy());
                }
            }

            // Direct port of stat.c:566+ setiparam/setaparam paths:
            //   `-A name` → indexed array, just the values in
            //              field-table order. STF_ARRAY.
            //   `-H name` → hash, name=>value pairs. STF_HASH.
            // Without the per-flag distinction, `-A` was masquerading
            // as `-H` and `${arr[size]}` worked when it shouldn't.
            if as_array {
                let values: Vec<String> = collected
                    .iter()
                    .map(|(_, v)| v.clone())
                    .collect();
                self.arrays.insert(array_name.clone(), values);
            }
            if as_hash {
                let map: indexmap::IndexMap<String, String> = collected.into_iter().collect();
                self.assoc_arrays.insert(hash_name.clone(), map);
            }
        }

        0
    }
}
// END moved-from-exec-rs
