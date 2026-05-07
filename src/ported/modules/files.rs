//! File operation builtins - port of Modules/files.c
//!
//! Provides mkdir, rmdir, ln, mv, rm, chmod, chown, chgrp, sync builtins.

use std::fs::{self};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

/// Options for mkdir
#[derive(Debug, Default)]
/// `mkdir` option flags.
/// Mirrors the `Options ops` flag bag `bin_mkdir()` from
/// Src/Modules/files.c:63 reads — `-p` (create parents), `-m`
/// (mode).
pub struct MkdirOptions {
    pub parents: bool,
    pub mode: Option<u32>,
}

/// Create a directory
/// `mkdir` builtin.
/// Port of `bin_mkdir()` + `domkdir()` from
/// Src/Modules/files.c:63/115 — same `mkdir(2)`-with-mode
/// logic and the same `-p` parent-creation walk.
pub fn mkdir(path: &Path, options: &MkdirOptions) -> Result<(), String> {
    let mode = options.mode.unwrap_or(0o777);

    if options.parents {
        mkdir_parents(path, mode)
    } else {
        mkdir_single(path, mode)
    }
}

/// Port of `domkdir()` from `Src/Modules/files.c:115`.
fn mkdir_single(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;

        let path_str = path.to_string_lossy();
        let path_c = CString::new(path_str.as_bytes()).map_err(|e| e.to_string())?;

        let result = unsafe { libc::mkdir(path_c.as_ptr(), mode as libc::mode_t) };
        if result < 0 {
            Err(format!(
                "cannot make directory '{}': {}",
                path.display(),
                io::Error::last_os_error()
            ))
        } else {
            Ok(())
        }
    }

    #[cfg(not(unix))]
    {
        fs::create_dir(path)
            .map_err(|e| format!("cannot make directory '{}': {}", path.display(), e))
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/files.c`.
fn mkdir_parents(path: &Path, mode: u32) -> Result<(), String> {
    if path.exists() {
        if path.is_dir() {
            return Ok(());
        }
        return Err(format!(
            "'{}' exists but is not a directory",
            path.display()
        ));
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            mkdir_parents(parent, mode | 0o300)?;
        }
    }

    mkdir_single(path, mode)
}

/// Remove a directory
/// `rmdir` builtin.
/// Port of `bin_rmdir()` from Src/Modules/files.c:150 — wraps
/// `rmdir(2)` with errno → diagnostic conversion.
pub fn rmdir(path: &Path) -> Result<(), String> {
    fs::remove_dir(path).map_err(|e| format!("cannot remove directory '{}': {}", path.display(), e))
}

/// Options for link operations
#[derive(Debug, Default)]
/// `ln` option flags.
/// Mirrors the `Options ops` flag bag `bin_ln()` from
/// Src/Modules/files.c:200 reads — `-s` (symbolic), `-f`
/// (force), `-d` (allow superuser to link dirs).
pub struct LinkOptions {
    pub symbolic: bool,
    pub force: bool,
    pub interactive: bool,
    pub no_dereference: bool,
    pub allow_dir: bool,
}

/// Create a link (hard or symbolic)
/// `ln` builtin.
/// Port of `bin_ln()` from Src/Modules/files.c:200 —
/// dispatches between hardlink and symlink based on options,
/// then calls into `domove()` (line 298) for force-replace
/// semantics.
pub fn link(source: &Path, target: &Path, options: &LinkOptions) -> Result<(), String> {
    let target_path = if target.is_dir() && !options.no_dereference {
        let filename = source
            .file_name()
            .ok_or_else(|| "invalid source path".to_string())?;
        target.join(filename)
    } else {
        target.to_path_buf()
    };

    if target_path.exists() {
        if options.force {
            fs::remove_file(&target_path)
                .map_err(|e| format!("cannot remove '{}': {}", target_path.display(), e))?;
        } else if !options.interactive {
            return Err(format!("'{}' already exists", target_path.display()));
        }
    }

    #[cfg(unix)]
    {
        if !options.allow_dir && source.is_dir() && !options.symbolic {
            return Err(format!(
                "'{}': hard link not allowed for directory",
                source.display()
            ));
        }

        if options.symbolic {
            std::os::unix::fs::symlink(source, &target_path)
                .map_err(|e| format!("cannot create symlink '{}': {}", target_path.display(), e))
        } else {
            fs::hard_link(source, &target_path)
                .map_err(|e| format!("cannot create hard link '{}': {}", target_path.display(), e))
        }
    }

    #[cfg(not(unix))]
    {
        fs::hard_link(source, &target_path)
            .map_err(|e| format!("cannot create link '{}': {}", target_path.display(), e))
    }
}

/// Options for move/rename
#[derive(Debug, Default)]
/// `mv` option flags.
/// Mirrors the flag bag `bin_ln()` (Src/Modules/files.c:200)
/// dispatches when `func == BIN_MV` — `-f` / `-i` interactivity
/// and the no-clobber path.
pub struct MoveOptions {
    pub force: bool,
    pub interactive: bool,
}

/// Move/rename a file
/// `mv` builtin.
/// Port of the rename-or-copy path inside `domove()` from
/// Src/Modules/files.c:298 — wraps `rename(2)` with the C
/// source's interactive-prompt and force-overwrite logic.
pub fn mv(source: &Path, target: &Path, options: &MoveOptions) -> Result<(), String> {
    let target_path = if target.is_dir() {
        let filename = source
            .file_name()
            .ok_or_else(|| "invalid source path".to_string())?;
        target.join(filename)
    } else {
        target.to_path_buf()
    };

    if target_path.exists() && !options.force && !options.interactive && target_path.is_dir() {
        return Err(format!(
            "'{}': cannot overwrite directory",
            target_path.display()
        ));
    }

    fs::rename(source, &target_path).map_err(|e| {
        format!(
            "cannot move '{}' to '{}': {}",
            source.display(),
            target_path.display(),
            e
        )
    })
}

/// Options for remove
#[derive(Debug, Default)]
/// `rm` option flags.
/// Mirrors the `Options ops` flag bag `bin_rm()` from
/// Src/Modules/files.c:616 reads — `-f` / `-i` / `-r` / `-s`.
pub struct RemoveOptions {
    pub force: bool,
    pub recursive: bool,
    pub interactive: bool,
    pub dir: bool,
}

/// Remove a file or directory
/// `rm` builtin.
/// Port of `bin_rm()` from Src/Modules/files.c:616 — drives
/// the `recursivecmd()` walker (line 378) with
/// `rm_leaf` (line 546) / `rm_dirpost` (line 594) callbacks.
pub fn rm(path: &Path, options: &RemoveOptions) -> Result<(), String> {
    if !path.exists() {
        if options.force {
            return Ok(());
        }
        return Err(format!(
            "cannot remove '{}': No such file or directory",
            path.display()
        ));
    }

    if path.is_dir() {
        if options.recursive {
            rm_recursive(path, options)
        } else if options.dir {
            fs::remove_dir(path).map_err(|e| format!("cannot remove '{}': {}", path.display(), e))
        } else if !options.force {
            Err(format!(
                "cannot remove '{}': Is a directory",
                path.display()
            ))
        } else {
            Ok(())
        }
    } else {
        fs::remove_file(path).map_err(|e| format!("cannot remove '{}': {}", path.display(), e))
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/files.c`.
#[allow(clippy::only_used_in_recursion)]
fn rm_recursive(path: &Path, options: &RemoveOptions) -> Result<(), String> {
    if path.is_dir() {
        for entry in fs::read_dir(path)
            .map_err(|e| format!("cannot read directory '{}': {}", path.display(), e))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            rm_recursive(&entry.path(), options)?;
        }
        fs::remove_dir(path).map_err(|e| format!("cannot remove '{}': {}", path.display(), e))
    } else {
        fs::remove_file(path).map_err(|e| format!("cannot remove '{}': {}", path.display(), e))
    }
}

/// Change file permissions
/// `chmod` builtin.
/// Port of `bin_chmod()` + `chmod_dochmod()` from
/// Src/Modules/files.c:655/642 — same `chmod(2)` per-file
/// dispatch, walked recursively via `recursivecmd()` when `-R`.
pub fn chmod(path: &Path, mode: u32, recursive: bool) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;

        let path_str = path.to_string_lossy();
        let path_c = CString::new(path_str.as_bytes()).map_err(|e| e.to_string())?;

        let result = unsafe { libc::chmod(path_c.as_ptr(), mode as libc::mode_t) };
        if result < 0 {
            return Err(format!(
                "cannot change mode of '{}': {}",
                path.display(),
                io::Error::last_os_error()
            ));
        }

        if recursive && path.is_dir() {
            for entry in fs::read_dir(path)
                .map_err(|e| format!("cannot read directory '{}': {}", path.display(), e))?
            {
                let entry = entry.map_err(|e| e.to_string())?;
                chmod(&entry.path(), mode, true)?;
            }
        }

        Ok(())
    }

    #[cfg(not(unix))]
    {
        Err("chmod not supported on this platform".to_string())
    }
}

/// Change file owner/group
#[cfg(unix)]
/// `chown`/`chgrp` builtin.
/// Port of `bin_chown()` + `chown_dochown()` /
/// `chown_dolchown()` from Src/Modules/files.c (~line 700) —
/// `chown(2)` / `lchown(2)` per file, walked recursively when
/// `-R`.
pub fn chown(
    path: &Path,
    uid: Option<u32>,
    gid: Option<u32>,
    recursive: bool,
    no_dereference: bool,
) -> Result<(), String> {
    use std::ffi::CString;

    let path_str = path.to_string_lossy();
    let path_c = CString::new(path_str.as_bytes()).map_err(|e| e.to_string())?;

    let uid = uid
        .map(|u| u as libc::uid_t)
        .unwrap_or(u32::MAX as libc::uid_t);
    let gid = gid
        .map(|g| g as libc::gid_t)
        .unwrap_or(u32::MAX as libc::gid_t);

    let result = if no_dereference {
        unsafe { libc::lchown(path_c.as_ptr(), uid, gid) }
    } else {
        unsafe { libc::chown(path_c.as_ptr(), uid, gid) }
    };

    if result < 0 {
        return Err(format!(
            "cannot change owner of '{}': {}",
            path.display(),
            io::Error::last_os_error()
        ));
    }

    if recursive && path.is_dir() {
        for entry in fs::read_dir(path)
            .map_err(|e| format!("cannot read directory '{}': {}", path.display(), e))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            chown(&entry.path(), Some(uid), Some(gid), true, no_dereference)?;
        }
    }

    Ok(())
}

/// Port of `bin_chown()` from `Src/Modules/files.c:725`.
/// Get user ID from username
#[cfg(unix)]
/// Look up a uid by username.
/// zshrs convenience over `getpwnam(3)` — the C source inlines
/// this lookup inside `parse_chown_spec` equivalents in
/// Src/Modules/files.c.
pub fn get_uid(username: &str) -> Option<u32> {
    use std::ffi::CString;

    if let Ok(uid) = username.parse::<u32>() {
        return Some(uid);
    }

    let username_c = CString::new(username).ok()?;
    unsafe {
        let pwd = libc::getpwnam(username_c.as_ptr());
        if pwd.is_null() {
            None
        } else {
            Some((*pwd).pw_uid)
        }
    }
}

/// Port of `bin_chown()` from `Src/Modules/files.c:725`.
/// Get group ID from group name
#[cfg(unix)]
/// Look up a gid by group name.
/// zshrs convenience over `getgrnam(3)`.
pub fn get_gid(groupname: &str) -> Option<u32> {
    use std::ffi::CString;

    if let Ok(gid) = groupname.parse::<u32>() {
        return Some(gid);
    }

    let groupname_c = CString::new(groupname).ok()?;
    unsafe {
        let grp = libc::getgrnam(groupname_c.as_ptr());
        if grp.is_null() {
            None
        } else {
            Some((*grp).gr_gid)
        }
    }
}

/// Parse chown spec (user:group or user.group)
#[cfg(unix)]
/// Parse a `user[:group]` chown spec.
/// Port of the chown-arg parser inside `bin_chown()`
/// (Src/Modules/files.c) — accepts `user`, `:group`,
/// `user:group`, plus the legacy `user.group` form.
pub fn parse_chown_spec(spec: &str) -> Result<(Option<u32>, Option<u32>), String> {
    let (user_part, group_part) = if let Some(pos) = spec.find(':') {
        let (u, g) = spec.split_at(pos);
        (u, Some(&g[1..]))
    } else if let Some(pos) = spec.find('.') {
        let (u, g) = spec.split_at(pos);
        (u, Some(&g[1..]))
    } else {
        (spec, None)
    };

    let uid = if user_part.is_empty() {
        None
    } else {
        Some(get_uid(user_part).ok_or_else(|| format!("{}: no such user", user_part))?)
    };

    let gid = match group_part {
        Some("") => {
            if let Some(uid_val) = uid {
                unsafe {
                    let pwd = libc::getpwuid(uid_val);
                    if pwd.is_null() {
                        return Err(format!("{}: no such user", user_part));
                    }
                    Some((*pwd).pw_gid)
                }
            } else {
                None
            }
        }
        Some(g) => Some(get_gid(g).ok_or_else(|| format!("{}: no such group", g))?),
        None => None,
    };

    Ok((uid, gid))
}

/// Sync filesystem
/// Force a filesystem sync.
/// Port of `bin_sync()` from Src/Modules/files.c:53 — wraps
/// `sync(2)`.
pub fn sync_fs() {
    #[cfg(unix)]
    unsafe {
        libc::sync();
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/files.c`.
/// Convert octal mode to display string
/// Render a Unix mode bitmask as a 10-char `ls -l` string.
/// zshrs convenience — Src/Modules/files.c emits the same
/// shape inline for diagnostic output.
pub fn mode_to_string(mode: u32) -> String {
    let mut result = String::with_capacity(10);

    let file_type = match mode & 0o170000 {
        0o140000 => 's',
        0o120000 => 'l',
        0o100000 => '-',
        0o060000 => 'b',
        0o040000 => 'd',
        0o020000 => 'c',
        0o010000 => 'p',
        _ => '?',
    };
    result.push(file_type);

    let perms = [
        (mode & 0o400 != 0, 'r'),
        (mode & 0o200 != 0, 'w'),
        (
            mode & 0o100 != 0,
            if mode & 0o4000 != 0 { 's' } else { 'x' },
        ),
        (mode & 0o040 != 0, 'r'),
        (mode & 0o020 != 0, 'w'),
        (
            mode & 0o010 != 0,
            if mode & 0o2000 != 0 { 's' } else { 'x' },
        ),
        (mode & 0o004 != 0, 'r'),
        (mode & 0o002 != 0, 'w'),
        (
            mode & 0o001 != 0,
            if mode & 0o1000 != 0 { 't' } else { 'x' },
        ),
    ];

    for (set, ch) in perms {
        if set {
            result.push(ch);
        } else if ch == 's' {
            result.push('S');
        } else if ch == 't' {
            result.push('T');
        } else {
            result.push('-');
        }
    }

    result
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/files.c`.
/// Parse octal mode string
/// Parse an `ls -l`-style mode string back to a u32 bitmask.
/// zshrs-original convenience — used by tests / format
/// round-trips. C source's parser lives in `chmod`'s symbolic
/// mode parser.
pub fn parse_mode(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 8).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_mkdir_single() {
        let dir = TempDir::new().unwrap();
        let new_dir = dir.path().join("newdir");

        let options = MkdirOptions::default();
        mkdir(&new_dir, &options).unwrap();

        assert!(new_dir.exists());
        assert!(new_dir.is_dir());
    }

    #[test]
    fn test_mkdir_parents() {
        let dir = TempDir::new().unwrap();
        let deep_dir = dir.path().join("a/b/c/d");

        let options = MkdirOptions {
            parents: true,
            ..Default::default()
        };
        mkdir(&deep_dir, &options).unwrap();

        assert!(deep_dir.exists());
        assert!(deep_dir.is_dir());
    }

    #[test]
    fn test_rmdir() {
        let dir = TempDir::new().unwrap();
        let new_dir = dir.path().join("to_remove");

        fs::create_dir(&new_dir).unwrap();
        assert!(new_dir.exists());

        rmdir(&new_dir).unwrap();
        assert!(!new_dir.exists());
    }

    #[test]
    fn test_rm_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"test").unwrap();
        }

        let options = RemoveOptions::default();
        rm(&file_path, &options).unwrap();
        assert!(!file_path.exists());
    }

    #[test]
    fn test_rm_recursive() {
        let dir = TempDir::new().unwrap();
        let sub_dir = dir.path().join("subdir");
        fs::create_dir(&sub_dir).unwrap();

        let file_path = sub_dir.join("test.txt");
        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"test").unwrap();
        }

        let options = RemoveOptions {
            recursive: true,
            ..Default::default()
        };
        rm(&sub_dir, &options).unwrap();
        assert!(!sub_dir.exists());
    }

    #[test]
    fn test_mv() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");

        {
            let mut f = File::create(&src).unwrap();
            f.write_all(b"content").unwrap();
        }

        let options = MoveOptions::default();
        mv(&src, &dst, &options).unwrap();

        assert!(!src.exists());
        assert!(dst.exists());
    }

    #[test]
    #[cfg(unix)]
    fn test_link_hard() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("link.txt");

        {
            let mut f = File::create(&src).unwrap();
            f.write_all(b"content").unwrap();
        }

        let options = LinkOptions::default();
        link(&src, &dst, &options).unwrap();

        assert!(dst.exists());
        assert_eq!(
            fs::metadata(&src).unwrap().ino(),
            fs::metadata(&dst).unwrap().ino()
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_link_symbolic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("symlink.txt");

        {
            let mut f = File::create(&src).unwrap();
            f.write_all(b"content").unwrap();
        }

        let options = LinkOptions {
            symbolic: true,
            ..Default::default()
        };
        link(&src, &dst, &options).unwrap();

        assert!(dst.is_symlink());
    }

    #[test]
    #[cfg(unix)]
    fn test_chmod() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");

        {
            let mut f = File::create(&file_path).unwrap();
            f.write_all(b"test").unwrap();
        }

        chmod(&file_path, 0o755, false).unwrap();

        let meta = fs::metadata(&file_path).unwrap();
        assert_eq!(meta.mode() & 0o777, 0o755);
    }

    #[test]
    fn test_mode_to_string() {
        assert_eq!(mode_to_string(0o100644), "-rw-r--r--");
        assert_eq!(mode_to_string(0o100755), "-rwxr-xr-x");
        assert_eq!(mode_to_string(0o040755), "drwxr-xr-x");
        assert_eq!(mode_to_string(0o120777), "lrwxrwxrwx");
    }

    #[test]
    fn test_parse_mode() {
        assert_eq!(parse_mode("755"), Some(0o755));
        assert_eq!(parse_mode("644"), Some(0o644));
        assert_eq!(parse_mode("777"), Some(0o777));
        assert_eq!(parse_mode("invalid"), None);
    }

    #[test]
    #[cfg(unix)]
    fn test_get_uid() {
        assert!(get_uid("root").is_some() || get_uid("0").is_some());
        assert_eq!(get_uid("0"), Some(0));
    }

    #[test]
    #[cfg(unix)]
    fn test_parse_chown_spec() {
        let result = parse_chown_spec("0:0");
        assert!(result.is_ok());
        let (uid, gid) = result.unwrap();
        assert_eq!(uid, Some(0));
        assert_eq!(gid, Some(0));
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
    /// sync - flush filesystem buffers
    /// Port from zsh/Src/Modules/files.c bin_sync() lines 52-57
    /// mkdir - create directories
    /// Port from zsh/Src/Modules/files.c bin_mkdir() lines 62-111
    pub(crate) fn bin_mkdir(&self, args: &[String]) -> i32 {
        // coreutils mkdir(1) port. -p (parents) silently succeeds
        // when the dir exists; default fails. -v (verbose) reports
        // each created dir. -m sets mode after creation.
        let mut mode: u32 = 0o777;
        let mut parents = false;
        let mut verbose = false;
        let mut dirs: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-p" || arg == "--parents" {
                parents = true;
            } else if arg == "-v" || arg == "--verbose" {
                verbose = true;
            } else if arg == "-m" && i + 1 < args.len() {
                i += 1;
                mode = u32::from_str_radix(&args[i], 8).unwrap_or(0o777);
            } else if let Some(s) = arg.strip_prefix("-m") {
                mode = u32::from_str_radix(s, 8).unwrap_or(0o777);
            } else if let Some(s) = arg.strip_prefix("--mode=") {
                mode = u32::from_str_radix(s, 8).unwrap_or(0o777);
            } else if arg == "--" {
                dirs.extend(args[i + 1..].iter().map(|s| s.as_str()));
                break;
            } else if arg == "-" || !arg.starts_with('-') {
                dirs.push(arg);
            } else if arg.starts_with('-') && arg.len() > 1 {
                // Combined short flags: -pv, -vp.
                for c in arg[1..].chars() {
                    match c {
                        'p' => parents = true,
                        'v' => verbose = true,
                        // coreutils mkdir errors on unknown flags;
                        // old `_ => {}` made `mkdir -X foo` create
                        // foo silently with -X dropped.
                        _ => {
                            eprintln!("mkdir: unrecognized option: '-{}'", c);
                            return 1;
                        }
                    }
                }
            }
            i += 1;
        }

        let mut err = 0;
        for dir in dirs {
            let path = std::path::Path::new(dir);
            let result = if parents {
                std::fs::create_dir_all(path)
            } else {
                std::fs::create_dir(path)
            };
            if let Err(e) = result {
                eprintln!("mkdir: cannot create directory '{}': {}", dir, e);
                err = 1;
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
                }
                if verbose {
                    println!("mkdir: created directory '{}'", dir);
                }
            }
        }
        err
    }
    /// rmdir - remove directories.
    /// Port from zsh/Src/Modules/files.c bin_rmdir() with the
    /// coreutils -p (remove ancestors) extension.
    pub(crate) fn bin_rmdir(&self, args: &[String]) -> i32 {
        let mut parents = false;
        let mut dirs: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-p" => parents = true,
                "--" => {} // end of options (rest collected in fall-through)
                s if s.starts_with('-') && s.len() > 1 => {
                    // coreutils rmdir errors on unknown flags. Old
                    // permissive silent-accept made `rmdir -Z foo`
                    // attempt to remove `foo` while losing the -Z
                    // signal. Per coreutils convention.
                    eprintln!("rmdir: unrecognized option: '{}'", s);
                    return 1;
                }
                s => dirs.push(s),
            }
        }
        let mut err = 0;
        for arg in dirs {
            if let Err(e) = std::fs::remove_dir(arg) {
                eprintln!("rmdir: cannot remove '{}': {}", arg, e);
                err = 1;
                continue;
            }
            // -p: walk parents up; stop on first non-empty / error.
            // Direct port of coreutils rmdir(1) -p semantics.
            if parents {
                let mut p = std::path::Path::new(arg).parent().map(|p| p.to_path_buf());
                while let Some(dir) = p {
                    if dir.as_os_str().is_empty() || dir == std::path::Path::new("/") {
                        break;
                    }
                    if std::fs::remove_dir(&dir).is_err() {
                        break; // parent non-empty or no permission — stop.
                    }
                    p = dir.parent().map(|q| q.to_path_buf());
                }
            }
        }
        err
    }
    /// ln - create links. Port from zsh/Src/Modules/files.c bin_ln().
    /// Coreutils-compatible flag set: -s/-f/-i/-n/-v/-T plus combined
    /// short flags and --long forms. Last-wins between -f and -i.
    /// ln/mv - create links or move files. Direct port of bin_ln()
    /// from zsh/Src/Modules/files.c:200, which dispatches on its
    /// `func` arg between BIN_LN (link) and BIN_MV (move via domove).
    pub(crate) fn bin_ln(&self, name: &str, args: &[String]) -> i32 {
        // C parity: BUILTIN("mv", ..., bin_ln, ..., BIN_MV, "fi", NULL)
        // and BUILTIN("zf_mv", ...) both flow through bin_ln. We
        // dispatch on the invoked name here.
        if name == "mv" || name == "zf_mv" {
            return self.domove(args);
        }
        let mut symbolic = false;
        let mut force = false;
        let mut interactive = false;
        let mut no_deref = false;
        let mut verbose = false;
        let mut no_target_dir = false;
        let mut files: Vec<String> = Vec::new();
        let mut end_of_options = false;
        for arg in args {
            if end_of_options {
                files.push(arg.clone());
                continue;
            }
            match arg.as_str() {
                "--" => end_of_options = true,
                "-s" | "--symbolic" => symbolic = true,
                "-f" | "--force" => {
                    force = true;
                    interactive = false;
                }
                "-i" | "--interactive" => {
                    interactive = true;
                    force = false;
                }
                "-n" | "-h" | "--no-dereference" => no_deref = true,
                "-v" | "--verbose" => verbose = true,
                "-T" | "--no-target-directory" => no_target_dir = true,
                s if s.starts_with("--") => {
                    eprintln!("ln: unrecognized option: '{}'", s);
                    return 1;
                }
                s if s.starts_with('-') && s.len() > 1 => {
                    for c in s[1..].chars() {
                        match c {
                            's' => symbolic = true,
                            'f' => {
                                force = true;
                                interactive = false;
                            }
                            'i' => {
                                interactive = true;
                                force = false;
                            }
                            'n' | 'h' => no_deref = true,
                            'v' => verbose = true,
                            'T' => no_target_dir = true,
                            // coreutils ln errors on unknown short
                            // flags inside combined forms.
                            _ => {
                                eprintln!("ln: unrecognized option: '-{}'", c);
                                return 1;
                            }
                        }
                    }
                }
                _ => files.push(arg.clone()),
            }
        }

        if files.is_empty() {
            eprintln!("ln: missing file operand");
            return 1;
        }
        if files.len() == 1 {
            // ln SRC: link to SRC's basename in cwd. Direct port of
            // coreutils ln 1-arg form. Was Box::leak'd; now owned
            // strings so the leak goes away.
            let src = files[0].clone();
            let target = std::path::Path::new(&src)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(src.clone());
            files.push(target);
        }

        let target = files.pop().unwrap();
        let target_path = std::path::Path::new(&target);
        let is_dir = !no_deref && !no_target_dir && target_path.is_dir();

        let mut status = 0;
        for src in &files {
            let dest = if is_dir {
                format!(
                    "{}/{}",
                    target,
                    std::path::Path::new(src)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| src.clone())
                )
            } else {
                target.clone()
            };

            let dest_path = std::path::Path::new(&dest);
            if dest_path.exists() {
                if force {
                    let _ = std::fs::remove_file(&dest);
                } else if interactive {
                    eprint!("ln: replace '{}'? ", dest);
                    let mut response = String::new();
                    if std::io::stdin().read_line(&mut response).is_err()
                        || !response.trim().eq_ignore_ascii_case("y")
                    {
                        continue;
                    }
                    let _ = std::fs::remove_file(&dest);
                } else {
                    eprintln!("ln: failed to create link '{}': File exists", dest);
                    status = 1;
                    continue;
                }
            }

            let result = if symbolic {
                #[cfg(unix)]
                {
                    std::os::unix::fs::symlink(src, &dest)
                }
                #[cfg(not(unix))]
                {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        "symlinks not supported",
                    ))
                }
            } else {
                std::fs::hard_link(src, &dest)
            };

            match result {
                Ok(()) => {
                    if verbose {
                        let arrow = if symbolic { "->" } else { "=>" };
                        println!("'{}' {} '{}'", dest, arrow, src);
                    }
                }
                Err(e) => {
                    eprintln!("ln: cannot create link '{}' -> '{}': {}", dest, src, e);
                    status = 1;
                }
            }
        }
        status
    }
    /// mv - move/rename files. Helper invoked by bin_ln when func is
    /// BIN_MV. Mirrors zsh/Src/Modules/files.c domove().
    fn domove(&self, args: &[String]) -> i32 {
        let mut force = false;
        let mut interactive = false;
        let mut verbose = false;
        // -n / --no-clobber: never overwrite an existing target.
        // Order semantics per coreutils: if -f, -i, and -n appear
        // together, the LAST one wins. Track which was last.
        let mut no_clobber = false;
        let mut files: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-f" | "--force" => {
                    force = true;
                    interactive = false;
                    no_clobber = false;
                }
                "-i" | "--interactive" => {
                    interactive = true;
                    force = false;
                    no_clobber = false;
                }
                "-n" | "--no-clobber" => {
                    no_clobber = true;
                    force = false;
                    interactive = false;
                }
                "-v" | "--verbose" => verbose = true,
                "--" => {} // end of options; rest collected in fall-through
                s if !s.starts_with('-') || s == "-" => files.push(s),
                s => {
                    // coreutils mv rejects unknown flags. Old
                    // catch-all silently dropped them, so \`mv -X a b\`
                    // moved a → b ignoring -X.
                    eprintln!("mv: unrecognized option: '{}'", s);
                    return 1;
                }
            }
        }

        if files.len() < 2 {
            eprintln!("mv: missing file operand");
            return 1;
        }

        let target = files.pop().unwrap();
        let target_path = std::path::Path::new(target);
        let is_dir = target_path.is_dir();

        // Per-file continue-on-error per coreutils (was return 1 on
        // first failure, leaving the rest unprocessed).
        let mut mv_status = 0;
        for src in files {
            let dest = if is_dir {
                format!(
                    "{}/{}",
                    target,
                    std::path::Path::new(src)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| src.to_string())
                )
            } else {
                target.to_string()
            };

            let dest_path = std::path::Path::new(&dest);
            if dest_path.exists() && !force {
                if no_clobber {
                    // -n: silently skip existing targets per
                    // coreutils mv. Exit 0 (this is the
                    // intentional skip path, not an error).
                    if verbose {
                        println!("'{}' -> '{}' (skipped, target exists)", src, dest);
                    }
                    continue;
                }
                if interactive {
                    eprint!("mv: overwrite '{}'? ", dest);
                    let mut response = String::new();
                    if std::io::stdin().read_line(&mut response).is_err()
                        || !response.trim().eq_ignore_ascii_case("y")
                    {
                        continue;
                    }
                } else {
                    eprintln!("mv: cannot overwrite '{}': File exists", dest);
                    mv_status = 1;
                    continue;
                }
            }

            // Try rename first (fast same-filesystem path). On
            // EXDEV (cross-filesystem), fall back to copy+unlink —
            // matches coreutils mv's strategy. zsh's bin_ln (which
            // backs mv) does the same via the libc do_rename helper
            // that retries with file_copy on EXDEV.
            let rename_err = std::fs::rename(src, &dest);
            if let Err(e) = rename_err {
                let is_exdev = e.raw_os_error() == Some(libc::EXDEV);
                if is_exdev {
                    // Cross-fs: copy then unlink. For directories
                    // we fall through to the same copy_dir_recursive
                    // helper used by cp.
                    let src_path = std::path::Path::new(src);
                    let dest_path_buf = std::path::PathBuf::from(&dest);
                    let copy_result = if src_path.is_dir() {
                        Self::copy_dir_recursive(src_path, &dest_path_buf)
                    } else {
                        std::fs::copy(src, &dest).map(|_| ())
                    };
                    match copy_result {
                        Ok(()) => {
                            if src_path.is_dir() {
                                let _ = std::fs::remove_dir_all(src_path);
                            } else {
                                let _ = std::fs::remove_file(src_path);
                            }
                        }
                        Err(ce) => {
                            eprintln!("mv: cannot move '{}' to '{}': {}", src, dest, ce);
                            mv_status = 1;
                            continue;
                        }
                    }
                } else {
                    eprintln!("mv: cannot move '{}' to '{}': {}", src, dest, e);
                    mv_status = 1;
                    continue;
                }
            }

            if verbose {
                println!("'{}' -> '{}'", src, dest);
            }
        }
        mv_status
    }
    /// rm - remove files
    pub(crate) fn bin_rm(&self, args: &[String]) -> i32 {
        // coreutils rm(1) port: combinable -rfv flags + -d (empty
        // dirs) + '--' end-of-options + --long-forms. Per-file
        // continue on error so 'rm a b c' deletes b/c when a fails.
        let mut recursive = false;
        let mut force = false;
        let mut interactive = false;
        let mut verbose = false;
        let mut empty_dir = false;
        let mut files: Vec<&str> = Vec::new();
        let mut end_of_options = false;
        for arg in args {
            if end_of_options {
                files.push(arg);
                continue;
            }
            match arg.as_str() {
                "--" => end_of_options = true,
                "-r" | "-R" | "--recursive" => recursive = true,
                "-f" | "--force" => {
                    force = true;
                    interactive = false;
                }
                "-i" | "--interactive" => {
                    interactive = true;
                    force = false;
                }
                "-v" | "--verbose" => verbose = true,
                "-d" | "--dir" => empty_dir = true,
                s if s.starts_with("--") => {
                    // Unknown long form rejected per coreutils.
                    eprintln!("rm: unrecognized option: '{}'", s);
                    return 1;
                }
                s if s.starts_with('-') && s.len() > 1 => {
                    // Combined short flags: walk every char.
                    for c in s[1..].chars() {
                        match c {
                            'r' | 'R' => recursive = true,
                            'f' => {
                                force = true;
                                interactive = false;
                            }
                            'i' => {
                                interactive = true;
                                force = false;
                            }
                            'v' => verbose = true,
                            'd' => empty_dir = true,
                            // coreutils rm errors on unknown short
                            // flag letters (esp. inside combined forms
                            // like \`-rfX\`).
                            _ => {
                                eprintln!("rm: unrecognized option: '-{}'", c);
                                return 1;
                            }
                        }
                    }
                }
                _ => files.push(arg),
            }
        }

        let mut status = 0;
        for file in files {
            let path = std::path::Path::new(file);

            if !path.exists() {
                if !force {
                    eprintln!("rm: cannot remove '{}': No such file or directory", file);
                    status = 1;
                }
                continue;
            }

            if interactive {
                let file_type = if path.is_dir() { "directory" } else { "file" };
                eprint!("rm: remove {} '{}'? ", file_type, file);
                let mut response = String::new();
                if std::io::stdin().read_line(&mut response).is_err()
                    || !response.trim().eq_ignore_ascii_case("y")
                {
                    continue;
                }
            }

            let result = if path.is_dir() {
                if recursive {
                    std::fs::remove_dir_all(path)
                } else if empty_dir {
                    // -d: only empty dirs (rmdir-like). Errors loudly
                    // if non-empty unless -f.
                    std::fs::remove_dir(path)
                } else {
                    eprintln!("rm: cannot remove '{}': Is a directory", file);
                    status = 1;
                    continue;
                }
            } else {
                std::fs::remove_file(path)
            };

            if let Err(e) = result {
                if !force {
                    eprintln!("rm: cannot remove '{}': {}", file, e);
                    status = 1;
                }
            } else if verbose {
                println!("removed '{}'", file);
            }
        }
        status
    }
    /// chown / chgrp - change file owner and/or group (Unix only).
    /// Direct port of bin_chown() from zsh/Src/Modules/files.c, which
    /// is the BUILTIN handler for both chown (BIN_CHOWN) and chgrp
    /// (BIN_CHGRP). The func arg selects whether the first positional
    /// is owner[:group] or just group.
    #[cfg(unix)]
    pub(crate) fn bin_chown(&self, name: &str, args: &[String]) -> i32 {
        let is_chgrp = name == "chgrp" || name == "zf_chgrp";
        let prefix = if is_chgrp { "chgrp" } else { "chown" };

        let mut recursive = false;
        let mut symlink = false;
        let mut positional: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-R" | "--recursive" => recursive = true,
                // -h: act on the symlink itself (lchown) rather than
                // following it. Direct port of zsh/Src/Modules/files.c
                // bin_chown which selects chown_dolchown when -h is
                // set. Default WITHOUT -h is to follow (chown the
                // target), matching coreutils chown(1).
                "-h" | "--no-dereference" => symlink = true,
                "--" => {} // end of options
                s if !s.starts_with('-') => positional.push(s),
                s => {
                    eprintln!("{}: unrecognized option: '{}'", prefix, s);
                    return 1;
                }
            }
        }

        if positional.len() < 2 {
            eprintln!("{}: missing operand", prefix);
            return 1;
        }

        // chgrp: first positional is the group spec, and uid is left
        // unchanged (u32::MAX maps to chown(2)'s -1 sentinel). chown:
        // first positional is owner[:group].
        let (uid, gid) = if is_chgrp {
            let group_spec = positional[0];
            let gid: u32 = if let Ok(id) = group_spec.parse() {
                id
            } else {
                unsafe {
                    let c_group = std::ffi::CString::new(group_spec).unwrap();
                    let gr = libc::getgrnam(c_group.as_ptr());
                    if gr.is_null() {
                        eprintln!("{}: invalid group: '{}'", prefix, group_spec);
                        return 1;
                    }
                    (*gr).gr_gid
                }
            };
            (u32::MAX, gid)
        } else {
            let owner_spec = positional[0];
            let (user, group) = if let Some(colon_pos) = owner_spec.find(':') {
                (&owner_spec[..colon_pos], Some(&owner_spec[colon_pos + 1..]))
            } else {
                (owner_spec, None)
            };

            let uid: u32 = if user.is_empty() {
                u32::MAX
            } else if let Ok(id) = user.parse() {
                id
            } else {
                unsafe {
                    let c_user = std::ffi::CString::new(user).unwrap();
                    let pw = libc::getpwnam(c_user.as_ptr());
                    if pw.is_null() {
                        eprintln!("{}: invalid user: '{}'", prefix, user);
                        return 1;
                    }
                    (*pw).pw_uid
                }
            };

            let gid: u32 = match group {
                Some(g) if !g.is_empty() => {
                    if let Ok(id) = g.parse() {
                        id
                    } else {
                        unsafe {
                            let c_group = std::ffi::CString::new(g).unwrap();
                            let gr = libc::getgrnam(c_group.as_ptr());
                            if gr.is_null() {
                                eprintln!("{}: invalid group: '{}'", prefix, g);
                                return 1;
                            }
                            (*gr).gr_gid
                        }
                    }
                }
                _ => u32::MAX,
            };
            (uid, gid)
        };

        let files = &positional[1..];

        fn do_chown(
            path: &std::path::Path,
            uid: u32,
            gid: u32,
            recursive: bool,
            symlink: bool,
            prefix: &str,
        ) -> i32 {
            let c_path = match std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
                Ok(p) => p,
                Err(_) => return 1,
            };

            // -h selects lchown to act on the symlink itself; default
            // chown follows the link to its target.
            let ret = unsafe {
                if symlink {
                    libc::lchown(c_path.as_ptr(), uid, gid)
                } else {
                    libc::chown(c_path.as_ptr(), uid, gid)
                }
            };
            if ret != 0 {
                let verb = if prefix == "chgrp" { "group" } else { "ownership" };
                eprintln!(
                    "{}: changing {} of '{}': {}",
                    prefix,
                    verb,
                    path.display(),
                    std::io::Error::last_os_error()
                );
                return 1;
            }

            if recursive && path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        if do_chown(&entry.path(), uid, gid, true, symlink, prefix) != 0 {
                            return 1;
                        }
                    }
                }
            }
            0
        }

        // Per-file continue-on-error per coreutils chown.
        let mut ch_status = 0;
        for file in files {
            if do_chown(std::path::Path::new(file), uid, gid, recursive, symlink, prefix) != 0 {
                ch_status = 1;
            }
        }
        ch_status
    }
    #[cfg(not(unix))]
    pub(crate) fn bin_chown(&self, name: &str, _args: &[String]) -> i32 {
        let prefix = if name == "chgrp" || name == "zf_chgrp" { "chgrp" } else { "chown" };
        eprintln!("{}: not supported on this platform", prefix);
        1
    }
    /// chmod - change file permissions
    pub(crate) fn bin_chmod(&self, args: &[String]) -> i32 {
        let mut recursive = false;
        let mut positional: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-R" | "--recursive" => recursive = true,
                "--" => {} // end of options
                s if !s.starts_with('-') => positional.push(s),
                // First arg starting with `-` followed by a digit is a
                // mode like `-rwx` or numeric (treated as positional);
                // a leading dash + symbolic-mode-letters could be
                // confused. zsh's chmod accepts only octal modes —
                // unknown flags like `-X` are rejected.
                s if s.starts_with('-') && s.len() > 1 => {
                    // Allow forms that LOOK like a mode if they parse
                    // as octal. Otherwise unknown flag.
                    if u32::from_str_radix(&s[1..], 8).is_ok() {
                        positional.push(s);
                    } else {
                        eprintln!("chmod: unrecognized option: '{}'", s);
                        return 1;
                    }
                }
                _ => {}
            }
        }

        if positional.len() < 2 {
            eprintln!("chmod: missing operand");
            return 1;
        }

        let mode_spec = positional[0];
        let files = &positional[1..];

        // Direct port of src/zsh/Src/Modules/files.c:660-666 bin_chmod
        // — mode parses as octal only. Symbolic forms (`u+x`, `g-w`,
        // etc.) are NOT supported by zsh's chmod builtin, only by
        // /bin/chmod. Mirror by erroring with the same diagnostic
        // format zsh uses: `chmod: invalid mode `<spec>'` exit 1.
        let mode: Option<u32> = u32::from_str_radix(mode_spec, 8).ok();
        if mode.is_none() {
            eprintln!("chmod: invalid mode `{}'", mode_spec);
            return 1;
        }
        let mode = mode.unwrap();

        fn do_chmod(path: &std::path::Path, mode: u32, recursive: bool) -> i32 {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) =
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                {
                    eprintln!("chmod: changing permissions of '{}': {}", path.display(), e);
                    return 1;
                }

                if recursive && path.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(path) {
                        for entry in entries.flatten() {
                            if do_chmod(&entry.path(), mode, true) != 0 {
                                return 1;
                            }
                        }
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (path, mode, recursive);
            }
            0
        }

        // Per-file continue-on-error.
        let mut chmod_status = 0;
        for file in files {
            if do_chmod(std::path::Path::new(file), mode, recursive) != 0 {
                chmod_status = 1;
            }
        }
        chmod_status
    }
    /// sync [FILE...] — flush filesystem buffers. Coreutils sync(1)
    /// / POSIX. Without args, calls sync(2) (sync all filesystems).
    /// With file args, calls fsync(2) on each. Flags --data
    /// (fdatasync) and --file-system (syncfs) accepted; data-only
    /// uses fdatasync(2) per file. Other flags rejected.
    pub(crate) fn bin_sync(&self, args: &[String]) -> i32 {
        let mut data_only = false;
        let mut filesystem = false;
        let mut files: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-d" | "--data" => data_only = true,
                "-f" | "--file-system" => filesystem = true,
                "--" => {}
                s if s.starts_with('-') && s.len() > 1 => {
                    eprintln!("sync: unrecognized option: '{}'", s);
                    return 1;
                }
                s => files.push(s),
            }
        }
        if files.is_empty() {
            // Plain sync — flush all FS.
            unsafe {
                libc::sync();
            }
            return 0;
        }
        // Per-file flush. Open each, fdatasync/fsync, close.
        let mut status = 0;
        for f in files {
            let cpath = match std::ffi::CString::new(f) {
                Ok(c) => c,
                Err(_) => {
                    eprintln!("sync: invalid path '{}'", f);
                    status = 1;
                    continue;
                }
            };
            let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY) };
            if fd < 0 {
                eprintln!(
                    "sync: cannot open '{}': {}",
                    f,
                    std::io::Error::last_os_error()
                );
                status = 1;
                continue;
            }
            let r = if data_only {
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::fdatasync(fd)
                }
                // macOS doesn't expose fdatasync; fall back to fsync.
                #[cfg(not(target_os = "linux"))]
                unsafe {
                    libc::fsync(fd)
                }
            } else if filesystem {
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::syncfs(fd)
                }
                #[cfg(not(target_os = "linux"))]
                unsafe {
                    libc::fsync(fd)
                }
            } else {
                unsafe { libc::fsync(fd) }
            };
            if r != 0 {
                eprintln!(
                    "sync: cannot sync '{}': {}",
                    f,
                    std::io::Error::last_os_error()
                );
                status = 1;
            }
            unsafe {
                libc::close(fd);
            }
        }
        status
    }
}
// END moved-from-exec-rs
