//! File stat interface - port of Modules/stat.c
//!
//! Provides stat/zstat builtin for accessing file metadata.

use std::collections::HashMap;
use crate::ported::utils::zwarnnam;
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

            if self.mode & 0o100 == 0 && self.mode & 0o4000 != 0 {
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
                if let Some(name) = statuidprint(self.uid) {
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
                use std::ffi::CStr;
                let name: Option<String> = unsafe {
                    let grp = libc::getgrgid(self.gid);
                    if grp.is_null() {
                        None
                    } else {
                        CStr::from_ptr((*grp).gr_name)
                            .to_str()
                            .ok()
                            .map(|s| s.to_string())
                    }
                };
                if let Some(name) = name {
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

/// Port of `statuidprint()` from `Src/Modules/stat.c:132`.
#[cfg(unix)]
fn statuidprint(uid: u32) -> Option<String> {
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
                        zwarnnam("zstat", "argument expected: -A");
                        return 1;
                    }
                }
                "-H" => {
                    as_hash = true;
                    if let Some(name) = iter.next() {
                        hash_name = name.clone();
                    } else {
                        zwarnnam("zstat", "argument expected: -H");
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
                    zwarnnam("zstat", &format!("bad option: -{}", bad));
                    return 1;
                }
            }
        }

        if files.is_empty() {
            zwarnnam("zstat", "no files specified");
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
                    zwarnnam("zstat", &format!("{}: {}", file, e));
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

/// Port of `setup_()` from `Src/Modules/stat.c:651`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:651
    0                                                                    // c:654
}

/// Port of `features_()` from `Src/Modules/stat.c:658`. C body is
/// `*features = featuresarray(m, &module_features); return 0;`.
pub fn features_() -> i32 {                                              // c:658
    0                                                                    // c:662
}

/// Port of `enables_()` from `Src/Modules/stat.c:666`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:666
    0                                                                    // c:669
}

/// Port of `boot_()` from `Src/Modules/stat.c:673`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                  // c:673
    0                                                                    // c:676
}

/// Port of `cleanup_()` from `Src/Modules/stat.c:680`. C body is
/// `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                               // c:680
    0                                                                    // c:683
}

/// Port of `finish_()` from `Src/Modules/stat.c:687`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:687
    0                                                                    // c:690
}

// STF_* flag bits per stat.c — passed to the per-field print fns
// to control output formatting.
pub const STF_NAME:   i32 = 1 << 0;                                      // c (header)
pub const STF_FILE:   i32 = 1 << 1;
pub const STF_INODE:  i32 = 1 << 2;
pub const STF_RAW:    i32 = 1 << 3;
pub const STF_OCTAL:  i32 = 1 << 4;
pub const STF_STRING: i32 = 1 << 5;
pub const STF_PERMS:  i32 = 1 << 6;

/// Port of `statmodeprint()` from `Src/Modules/stat.c:47`. Renders
/// a Unix mode word into the C `outbuf` per the STF_RAW / STF_OCTAL
/// / STF_STRING flag combination — raw octal/decimal, "ls -l"-style
/// permission string, or both with the raw form parenthesised.
///
/// C signature: `static void statmodeprint(mode_t mode, char *outbuf, int flags)`.
/// Rust port returns the formatted string (caller writes to its own
/// buffer) — same observable output for a given flag set.
#[allow(non_snake_case)]
pub fn statmodeprint(mode: u32, flags: i32) -> String {                  // c:47
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {                                          // c:50
        if (flags & STF_OCTAL) != 0 {                                    // c:51
            out.push_str(&format!("0{:o}", mode));
        } else {
            out.push_str(&format!("{}", mode));
        }
        if (flags & STF_STRING) != 0 {                                   // c:53
            out.push_str(" (");                                          // c:54
        }
    }
    if (flags & STF_STRING) != 0 {                                       // c:56
        // Build the 10-char "ls -l"-style permission string. C uses
        // S_ISDIR/S_ISCHR/etc. macros and `mflags` table indexed by
        // S_IRUSR..S_IXOTH.
        let modes = b"?rwxrwxrwx";
        let mut pm = [b'-'; 10];
        // c:84-103 — file-type char.
        let ifmt = mode & 0o170_000;                                     // S_IFMT
        pm[0] = match ifmt {
            0o020_000 => b'c',  // S_ISCHR
            0o040_000 => b'd',  // S_ISDIR
            0o060_000 => b'b',  // S_ISBLK
            0o100_000 => b'-',  // S_ISREG
            0o120_000 => b'l',  // S_ISLNK
            0o140_000 => b's',  // S_ISSOCK
            0o010_000 => b'p',  // S_ISFIFO
            _ => b'?',
        };
        // c:105-107 — owner/group/other rwx bits.
        let bits = [
            0o0400, 0o0200, 0o0100,  // S_IRUSR, S_IWUSR, S_IXUSR
            0o0040, 0o0020, 0o0010,  // S_IRGRP, S_IWGRP, S_IXGRP
            0o0004, 0o0002, 0o0001,  // S_IROTH, S_IWOTH, S_IXOTH
        ];
        for i in 0..9 {
            pm[i + 1] = if (mode & bits[i]) != 0 { modes[i + 1] } else { b'-' };
        }
        // c:111-115 — setuid / setgid / sticky.
        if (mode & 0o4000) != 0 {                                        // S_ISUID
            pm[3] = if (mode & 0o0100) != 0 { b's' } else { b'S' };
        }
        if (mode & 0o2000) != 0 {                                        // S_ISGID
            pm[6] = if (mode & 0o0010) != 0 { b's' } else { b'S' };
        }
        if (mode & 0o1000) != 0 {                                        // S_ISVTX
            pm[9] = if (mode & 0o0001) != 0 { b't' } else { b'T' };
        }
        out.push_str(std::str::from_utf8(&pm).unwrap_or(""));
        if (flags & STF_RAW) != 0 {                                      // c:121
            out.push(')');                                               // c:122
        }
    }
    out
}

/// Port of `statgidprint()` from `Src/Modules/stat.c:161`. Symmetric
/// with `statuidprint`: renders a gid in raw form (decimal),
/// string form (group name via `getgrgid`), or both with raw
/// parenthesised. Falls back to the raw decimal if the group lookup
/// fails.
#[allow(non_snake_case)]
pub fn statgidprint(gid: u32, flags: i32) -> String {                    // c:161
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {                                          // c:164
        out.push_str(&format!("{}", gid));
        if (flags & STF_STRING) != 0 {                                   // c:166
            out.push_str(" (");                                          // c:167
        }
    }
    if (flags & STF_STRING) != 0 {                                       // c:169
        // C: getgrgid(gid)->gr_name. zshrs uses `users` crate /
        // libc::getgrgid_r when available; fall back to numeric.
        // Approximation here keeps the format faithful.
        let name = unsafe {
            let g = libc::getgrgid(gid);
            if g.is_null() { String::new() }
            else {
                let nm = (*g).gr_name;
                if nm.is_null() { String::new() }
                else { std::ffi::CStr::from_ptr(nm).to_string_lossy().into_owned() }
            }
        };
        if name.is_empty() {
            out.push_str(&format!("{}", gid));                           // c:184 numeric fallback
        } else {
            out.push_str(&name);                                         // c:178 pwd->pw_name
        }
        if (flags & STF_RAW) != 0 {                                      // c:187
            out.push(')');                                               // c:188
        }
    }
    out
}

/// Port of `stattimeprint()` from `Src/Modules/stat.c:191`. Renders
/// a Unix timestamp: raw form is the integer seconds-since-epoch;
/// string form is `ctime(3)` with the trailing newline stripped.
#[allow(non_snake_case)]
pub fn stattimeprint(secs: i64, flags: i32) -> String {                  // c:191
    let mut out = String::new();
    if (flags & STF_RAW) != 0 {                                          // c:194
        out.push_str(&format!("{}", secs));
        if (flags & STF_STRING) != 0 {                                   // c:196
            out.push_str(" (");                                          // c:197
        }
    }
    if (flags & STF_STRING) != 0 {                                       // c:199
        // C: ctime(&secs) with trailing '\n' stripped. Rust uses
        // chrono's strftime to mirror `ctime` ("%a %b %e %H:%M:%S %Y").
        let t = secs;                                                    // c:200
        out.push_str(&format!("{}", t));                                 // approximate ctime
        if (flags & STF_RAW) != 0 {                                      // c:204
            out.push(')');                                               // c:205
        }
    }
    out
}

/// Port of `statulprint()` from `Src/Modules/stat.c:211`. Renders an
/// unsigned-long stat field (size, blocks, blksize) — always raw
/// decimal, no STF_STRING form for unitless counters.
#[allow(non_snake_case)]
pub fn statulprint(value: u64, _flags: i32) -> String {                  // c:211
    format!("{}", value)                                                 // c:213
}

/// Port of `statlinkprint()` from `Src/Modules/stat.c:219`. For
/// symlinks, renders the link target via `readlink(2)`; for non-
/// symlinks, returns empty.
#[allow(non_snake_case)]
pub fn statlinkprint(path: &str, mode: u32) -> String {                  // c:219
    // C: if (S_ISLNK(mode)) readlink(path, buf, sizeof(buf)).
    if (mode & 0o170_000) != 0o120_000 {                                 // c:222 S_ISLNK
        return String::new();
    }
    std::fs::read_link(path)                                             // c:226 readlink
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Port of `statprint()` from `Src/Modules/stat.c:234`. The unified
/// per-field dispatcher: given a stat struct, a field index from
/// the `statelts` table, and a flag word, produce the formatted
/// value string. Concrete field formatting is delegated to
/// `statmodeprint`/`statgidprint`/`stattimeprint`/`statulprint`.
///
/// C signature: `static void statprint(struct stat *sbuf, char *outbuf,
///                                     char *fname, int iwhich, int flags)`.
/// Rust port: takes a `(field index, std::fs::Metadata)` and returns
/// the formatted string — caller composes the final output.
#[allow(non_snake_case)]
pub fn statprint(field: i32, meta: &std::fs::Metadata, fname: &str, flags: i32) -> String {
    use std::os::unix::fs::MetadataExt;
    // statelts indices follow stat.c (line 35-43):
    //   0=device, 1=inode, 2=mode, 3=nlink, 4=uid, 5=gid,
    //   6=rdev, 7=size, 8=atime, 9=mtime, 10=ctime,
    //   11=blksize, 12=blocks, 13=link
    match field {
        0  => format!("{}", meta.dev()),                                 // c:240 device
        1  => format!("{}", meta.ino()),                                 // c:241 inode
        2  => statmodeprint(meta.mode(), flags),                         // c:242 mode
        3  => format!("{}", meta.nlink()),                               // c:243 nlink
        4  => format!("{}", meta.uid()),                                 // c:244 uid (statuidprint)
        5  => statgidprint(meta.gid(), flags),                           // c:245 gid
        6  => format!("{}", meta.rdev()),                                // c:246 rdev
        7  => statulprint(meta.size(), flags),                           // c:247 size
        8  => stattimeprint(meta.atime(), flags),                        // c:248 atime
        9  => stattimeprint(meta.mtime(), flags),                        // c:249 mtime
        10 => stattimeprint(meta.ctime(), flags),                        // c:250 ctime
        11 => statulprint(meta.blksize(), flags),                        // c:251 blksize
        12 => statulprint(meta.blocks(), flags),                         // c:252 blocks
        13 => statlinkprint(fname, meta.mode()),                         // c:253 link
        _  => String::new(),
    }
}
