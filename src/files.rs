//! File operation builtins - port of Modules/files.c
//!
//! Provides mkdir, rmdir, ln, mv, rm, chmod, chown, chgrp, sync builtins.

use std::fs::{self};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

/// Options for mkdir
#[derive(Debug, Default)]
pub struct MkdirOptions {
    pub parents: bool,
    pub mode: Option<u32>,
}

/// Create a directory
pub fn mkdir(path: &Path, options: &MkdirOptions) -> Result<(), String> {
    let mode = options.mode.unwrap_or(0o777);

    if options.parents {
        mkdir_parents(path, mode)
    } else {
        mkdir_single(path, mode)
    }
}

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
pub fn rmdir(path: &Path) -> Result<(), String> {
    fs::remove_dir(path).map_err(|e| format!("cannot remove directory '{}': {}", path.display(), e))
}

/// Options for link operations
#[derive(Debug, Default)]
pub struct LinkOptions {
    pub symbolic: bool,
    pub force: bool,
    pub interactive: bool,
    pub no_dereference: bool,
    pub allow_dir: bool,
}

/// Create a link (hard or symbolic)
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
pub struct MoveOptions {
    pub force: bool,
    pub interactive: bool,
}

/// Move/rename a file
pub fn mv(source: &Path, target: &Path, options: &MoveOptions) -> Result<(), String> {
    let target_path = if target.is_dir() {
        let filename = source
            .file_name()
            .ok_or_else(|| "invalid source path".to_string())?;
        target.join(filename)
    } else {
        target.to_path_buf()
    };

    if target_path.exists() && !options.force && !options.interactive {
        if target_path.is_dir() {
            return Err(format!(
                "'{}': cannot overwrite directory",
                target_path.display()
            ));
        }
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
pub struct RemoveOptions {
    pub force: bool,
    pub recursive: bool,
    pub interactive: bool,
    pub dir: bool,
}

/// Remove a file or directory
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

/// Get user ID from username
#[cfg(unix)]
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

/// Get group ID from group name
#[cfg(unix)]
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
        Some(g) if g.is_empty() => {
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
pub fn sync_fs() {
    #[cfg(unix)]
    unsafe {
        libc::sync();
    }
}

/// Convert octal mode to display string
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

/// Parse octal mode string
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
