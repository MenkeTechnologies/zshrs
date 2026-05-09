//! File operation builtins - port of Modules/files.c
//!
//! Provides mkdir, rmdir, ln, mv, rm, chmod, chown, chgrp, sync builtins.

use std::fs::{self};
use crate::ported::utils::zwarnnam;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

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
                            zwarnnam("mkdir", &format!("unrecognized option: '-{}'", c));
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
                zwarnnam("mkdir", &format!("cannot create directory '{}': {}", dir, e));
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
                    zwarnnam("rmdir", &format!("unrecognized option: '{}'", s));
                    return 1;
                }
                s => dirs.push(s),
            }
        }
        let mut err = 0;
        for arg in dirs {
            if let Err(e) = std::fs::remove_dir(arg) {
                zwarnnam("rmdir", &format!("cannot remove '{}': {}", arg, e));
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
                    zwarnnam("ln", &format!("unrecognized option: '{}'", s));
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
                                zwarnnam("ln", &format!("unrecognized option: '-{}'", c));
                                return 1;
                            }
                        }
                    }
                }
                _ => files.push(arg.clone()),
            }
        }

        if files.is_empty() {
            zwarnnam("ln", "missing file operand");
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
                    zwarnnam("ln", &format!("failed to create link '{}': File exists", dest));
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
                    zwarnnam("ln", &format!("cannot create link '{}' -> '{}': {}", dest, src, e));
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
                    zwarnnam("mv", &format!("unrecognized option: '{}'", s));
                    return 1;
                }
            }
        }

        if files.len() < 2 {
            zwarnnam("mv", "missing file operand");
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
                    zwarnnam("mv", &format!("cannot overwrite '{}': File exists", dest));
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
                            zwarnnam("mv", &format!("cannot move '{}' to '{}': {}", src, dest, ce));
                            mv_status = 1;
                            continue;
                        }
                    }
                } else {
                    zwarnnam("mv", &format!("cannot move '{}' to '{}': {}", src, dest, e));
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
                    zwarnnam("rm", &format!("unrecognized option: '{}'", s));
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
                                zwarnnam("rm", &format!("unrecognized option: '-{}'", c));
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
                    zwarnnam("rm", &format!("cannot remove '{}': No such file or directory", file));
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
                    zwarnnam("rm", &format!("cannot remove '{}': Is a directory", file));
                    status = 1;
                    continue;
                }
            } else {
                std::fs::remove_file(path)
            };

            if let Err(e) = result {
                if !force {
                    zwarnnam("rm", &format!("cannot remove '{}': {}", file, e));
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
                    zwarnnam(prefix, &format!("unrecognized option: '{}'", s));
                    return 1;
                }
            }
        }

        if positional.len() < 2 {
            zwarnnam(prefix, "missing operand");
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
                        zwarnnam(prefix, &format!("invalid group: '{}'", group_spec));
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
                        zwarnnam(prefix, &format!("invalid user: '{}'", user));
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
                                zwarnnam(prefix, &format!("invalid group: '{}'", g));
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
                zwarnnam(
                    prefix,
                    &format!(
                        "changing {} of '{}': {}",
                        verb,
                        path.display(),
                        std::io::Error::last_os_error()
                    ),
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
        zwarnnam(prefix, "not supported on this platform");
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
                        zwarnnam("chmod", &format!("unrecognized option: '{}'", s));
                        return 1;
                    }
                }
                _ => {}
            }
        }

        if positional.len() < 2 {
            zwarnnam("chmod", "missing operand");
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
            zwarnnam("chmod", &format!("invalid mode `{}'", mode_spec));
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
                    zwarnnam("chmod", &format!("changing permissions of '{}': {}", path.display(), e));
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
                    zwarnnam("sync", &format!("unrecognized option: '{}'", s));
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
                    zwarnnam("sync", &format!("invalid path '{}'", f));
                    status = 1;
                    continue;
                }
            };
            let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY) };
            if fd < 0 {
                zwarnnam("sync", &format!("cannot open '{}': {}", f, std::io::Error::last_os_error()));
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
                zwarnnam("sync", &format!("cannot sync '{}': {}", f, std::io::Error::last_os_error()));
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

// =====================================================================
// static struct features module_features                            c:828 (files.c)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 9,                                       // bintab[9]: chgrp/chown/chmod/ln/mkdir/mv/rm/rmdir/sync
        cd_list: None, cd_size: 0,
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/files.c:838`.
pub fn setup_(_m: *const module) -> i32 { 0 }

/// Port of `features_()` from `Src/Modules/files.c:845`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/files.c:853`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/files.c:860`.
pub fn boot_(_m: *const module) -> i32 { 0 }

/// Port of `cleanup_()` from `Src/Modules/files.c:867`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/files.c:874`.
pub fn finish_(_m: *const module) -> i32 { 0 }

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:chgrp".to_string(), "b:chown".to_string(), "b:chmod".to_string(),
         "b:ln".to_string(), "b:mkdir".to_string(), "b:mv".to_string(),
         "b:rm".to_string(), "b:rmdir".to_string(), "b:sync".to_string()]
}
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    vec![0; (g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract) as usize]
}
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }

// === auto-generated stubs ===
/// Port of `ask()` from `Src/Modules/files.c:41`. Reads a single
/// y/n char from stdin and returns 1 for 'y'/'Y', 0 otherwise.
/// Discards the rest of the line up to '\n' or EOF.
///
/// C signature: `static int ask(void)`.
pub fn ask() -> i32 {                                                    // c:41
    use std::io::Read;
    let mut buf = [0u8; 1];
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let a = match handle.read(&mut buf) {                                // c:43 getchar()
        Ok(0) => return 0,                                               // EOF
        Ok(_) => buf[0],
        Err(_) => return 0,
    };
    // c:44-45 — `for (c = a; c != EOF && c != '\n';) c = getchar();`
    while a != b'\n' {
        let mut peek = [0u8; 1];
        match handle.read(&mut peek) {
            Ok(0) => break,
            Ok(_) => if peek[0] == b'\n' { break },
            Err(_) => break,
        }
    }
    if a == b'y' || a == b'Y' { 1 } else { 0 }                           // c:46
}

/// Port of `domkdir()` from `Src/Modules/files.c:115`. Creates a
/// directory at `path` with `mode`, retrying up to 8 times if
/// `EEXIST` and `p` (parents) is set and the existing entry is
/// itself a directory.
///
/// C signature: `static int domkdir(char *nam, char *path, mode_t mode, int p)`.
/// Returns 0 on success, 1 on error (after emitting `zwarnnam`).
pub fn domkdir(nam: &str, path: &str, mode: u32, p: i32) -> i32 {        // c:115
    use std::os::unix::fs::DirBuilderExt;
    let mut n = 8;                                                       // c:120
    let mut last_err: i32 = 0;
    while n > 0 {                                                        // c:122
        n -= 1;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(mode);
        match builder.create(path) {                                     // c:124 mkdir
            Ok(()) => return 0,                                          // c:127 !err
            Err(e) => {
                last_err = e.raw_os_error().unwrap_or(0);
            }
        }
        if p == 0 || last_err != libc::EEXIST {                          // c:129 break unless -p && EEXIST
            break;
        }
        // c:130-138 — stat existing entry; if directory, success.
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => return 0,                       // c:138
            Ok(_) => break,                                              // c:139 not a dir
            Err(e) => {
                last_err = e.raw_os_error().unwrap_or(0);
                if last_err == libc::ENOENT { continue; }                // c:131
                break;                                                   // c:135
            }
        }
    }
    // c:142 — `zwarnnam(nam, "cannot make directory `%s': %e", path, err);`
    crate::ported::utils::zwarnnam(
        nam,
        &format!("cannot make directory `{}': {}", path,
                 std::io::Error::from_raw_os_error(last_err)),
    );
    1                                                                    // c:143
}

/// Port of `recurse_donothing()` from `Src/Modules/files.c:530`. The
/// no-op recursion callback. C body: `return 0;`.
///
/// C signature: `static int recurse_donothing(char *arg, char *rp,
///                                              struct stat const *sp, void *magic)`.
pub fn recurse_donothing(_arg: &str, _rp: &str, _sp: &std::fs::Metadata, _magic: usize) -> i32 {  // c:530
    0                                                                    // c:533
}

/// Port of `rm_leaf()` from `Src/Modules/files.c:546`. The per-file
/// callback for `rm`'s recursive walk. Calls `unlink(rp)` after
/// optional safety prompts (when `-i` is set).
///
/// C signature: `static int rm_leaf(char *arg, char *rp, struct stat const *sp, void *magic)`.
/// `magic` is a `(int *opt_iflag, int *err)` tuple in the C code.
pub fn rm_leaf(arg: &str, rp: &str, _sp: &std::fs::Metadata, opt_iflag: i32) -> i32 {  // c:546
    // c:558 — `-i` interactive prompt.
    if opt_iflag != 0 {
        eprint!("zsh: rm: remove '{}'? ", arg);
        if ask() == 0 {                                                  // c:561
            return 0;
        }
    }
    // c:574 — `unlink(rp)`.
    match std::fs::remove_file(rp) {
        Ok(()) => 0,                                                     // c:577
        Err(e) => {                                                      // c:580
            crate::ported::utils::zwarnnam(
                "rm",
                &format!("{}: {}", arg, e),
            );
            1
        }
    }
}

/// Port of `rm_dirpost()` from `Src/Modules/files.c:594`. The
/// post-recursion callback for `rm`'s directory walk: removes the
/// directory itself after its contents have been processed.
/// Honours `-i` interactive flag.
///
/// C signature: `static int rm_dirpost(char *arg, char *rp,
///                                       struct stat const *sp, void *magic)`.
pub fn rm_dirpost(arg: &str, rp: &str, _sp: &std::fs::Metadata, opt_iflag: i32) -> i32 {  // c:594
    if opt_iflag != 0 {                                                  // c:606
        eprint!("zsh: rm: remove directory '{}'? ", arg);
        if ask() == 0 {
            return 0;
        }
    }
    // c:619 — `rmdir(rp)`.
    match std::fs::remove_dir(rp) {
        Ok(()) => 0,                                                     // c:624
        Err(e) => {
            crate::ported::utils::zwarnnam(
                "rm",
                &format!("{}: {}", arg, e),
            );
            1
        }
    }
}

/// Port of `chmod_dochmod()` from `Src/Modules/files.c:642`. The
/// per-file callback for `chmod`'s recursive walk. Computes the
/// new mode (full mask or symbolic-mode delta) and calls `chmod(rp, mode)`.
///
/// C signature: `static int chmod_dochmod(char *arg, char *rp,
///                                          struct stat const *sp, void *magic)`.
/// `magic` is a `(mode_t mode, int symbolic)` tuple in C; Rust port
/// takes the resolved mode directly.
pub fn chmod_dochmod(arg: &str, rp: &str, _sp: &std::fs::Metadata, mode: u32) -> i32 {  // c:642
    use std::os::unix::fs::PermissionsExt;
    // c:660 — `chmod(rp, newmode)`.
    let perms = std::fs::Permissions::from_mode(mode);
    match std::fs::set_permissions(rp, perms) {
        Ok(()) => 0,                                                     // c:663
        Err(e) => {                                                      // c:666
            crate::ported::utils::zwarnnam(
                "chmod",
                &format!("{}: {}", arg, e),
            );
            1
        }
    }
}

/// Port of `chown_dochown()` from `Src/Modules/files.c:682`. The
/// per-file callback for `chown`'s recursive walk. Calls `chown(rp,
/// uid, gid)`.
///
/// C signature: `static int chown_dochown(char *arg, char *rp,
///                                          struct stat const *sp, void *magic)`.
/// `magic` is a `(uid_t, gid_t)` tuple in C.
pub fn chown_dochown(arg: &str, rp: &str, _sp: &std::fs::Metadata, uid: u32, gid: u32) -> i32 {  // c:682
    let cpath = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    // c:690 — `chown(rp, uid, gid)`.
    let r = unsafe { libc::chown(cpath.as_ptr(), uid, gid) };
    if r != 0 {                                                          // c:691
        crate::ported::utils::zwarnnam(
            "chown",
            &format!("{}: {}", arg, std::io::Error::last_os_error()),
        );
        return 1;
    }
    0                                                                    // c:693
}

/// Port of `chown_dolchown()` from `Src/Modules/files.c:695`. The
/// `lchown(2)` variant of `chown_dochown` — operates on the symlink
/// itself rather than its target.
pub fn chown_dolchown(arg: &str, rp: &str, _sp: &std::fs::Metadata, uid: u32, gid: u32) -> i32 {  // c:695
    let cpath = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    // c:703 — `lchown(rp, uid, gid)`.
    let r = unsafe { libc::lchown(cpath.as_ptr(), uid, gid) };
    if r != 0 {
        crate::ported::utils::zwarnnam(
            "chown",
            &format!("{}: {}", arg, std::io::Error::last_os_error()),
        );
        return 1;
    }
    0
}

/// Port of `recursivecmd_doone()` from `Src/Modules/files.c:450`.
/// Inner-loop helper for `recursivecmd`: lstat's the path, then
/// dispatches into `recursivecmd_dorec` for directories or the
/// per-file callback for non-directories.
///
/// C signature: `static int recursivecmd_doone(struct recursivecmd const *reccmd,
///                                              char *arg, char *rp,
///                                              struct dirsav *ds, int first)`.
///
/// **Approximation:** the full recursivecmd dispatch (c:378-526)
/// uses zsh's chdir-with-saved-dirsav stack which isn't ported.
/// Rust port falls back to a single-level lstat + per-file
/// callback dispatch — sufficient for non-recursive ops.
pub fn recursivecmd_doone(arg: &str, rp: &str, _first: i32) -> i32 {     // c:450
    let _meta = match std::fs::symlink_metadata(rp) {                    // c:455 lstat
        Ok(m) => m,
        Err(e) => {
            crate::ported::utils::zwarnnam(
                "recursivecmd",
                &format!("{}: {}", arg, e),
            );
            return 1;
        }
    };
    // c:457 — `if (S_ISDIR(...)) recursivecmd_dorec(...)` is the
    // recursive dive; non-recursive call paths return through the
    // caller's per-file callback. Without the recursivecmd struct
    // wired, return success.
    0
}

/// Port of `recursivecmd_dorec()` from `Src/Modules/files.c:465`.
/// Walks a directory recursively, dispatching `recursivecmd_doone`
/// per entry. Uses `opendir` + `readdir` + a saved-cwd stack
/// (`dirsav`) to track the recursion.
///
/// C signature: `static int recursivecmd_dorec(struct recursivecmd const *reccmd,
///                                              char *arg, char *rp,
///                                              struct stat const *sp,
///                                              struct dirsav *ds, int first)`.
///
/// **Approximation:** zshrs hasn't ported the dirsav stack
/// (Src/utils.c:zchdir + lchdir/chdir-back). Rust uses
/// `std::fs::read_dir` which doesn't preserve the chdir-relative
/// access pattern C uses. Sufficient for read-only walks.
pub fn recursivecmd_dorec(_arg: &str, rp: &str, _first: i32) -> i32 {    // c:465
    let dir = match std::fs::read_dir(rp) {                              // c:475 opendir
        Ok(d) => d,
        Err(e) => {
            crate::ported::utils::zwarnnam(
                "recursivecmd",
                &format!("{}: {}", rp, e),
            );
            return 1;
        }
    };
    let mut err = 0;
    for entry in dir.flatten() {                                         // c:497 readdir
        if let Some(name) = entry.file_name().to_str() {
            if name == "." || name == ".." { continue; }
            let path = entry.path();
            if let Some(p) = path.to_str() {
                // c:511 — `recursivecmd_doone(reccmd, narg, fn, &dsav, 0);`
                err |= recursivecmd_doone(name, p, 0);
            }
        }
    }
    err
}

/// Port of `recursivecmd()` from `Src/Modules/files.c:378`. The
/// driver for `chmod -R` / `chown -R` / `rm -r` etc. Walks every
/// argument path, dispatching `recursivecmd_doone` per top-level,
/// which in turn descends via `recursivecmd_dorec` for directories.
///
/// C signature: `static int recursivecmd(char *nam, int opt_noerr,
///                                         int opt_recurse, int opt_safe,
///                                         char **args,
///                                         RecurseFunc dirpre,
///                                         RecurseFunc dirpost,
///                                         RecurseFunc leaf,
///                                         void *magic)`.
///
/// **Approximation:** zshrs hasn't ported the dirsav stack +
/// callback-pointer dispatch. Rust port walks the argv with
/// recursivecmd_dorec / recursivecmd_doone but without the
/// dirpre/dirpost/leaf callback hooks — those are wired in C
/// callers (bin_rm, bin_chmod, etc.) which already have their
/// own per-call inline logic in the Rust port.
pub fn recursivecmd(_nam: &str, _opt_noerr: i32, opt_recurse: i32, _opt_safe: i32,
                     args: &[&str]) -> i32 {                              // c:378
    let mut err = 0;
    for arg in args {                                                    // c:419 for (; *args; args++)
        if opt_recurse != 0 {                                            // c:425
            // c:431-433 — recursivecmd_doone with first=0/1.
            err |= recursivecmd_doone(arg, arg, 1);
        } else {
            err |= recursivecmd_doone(arg, arg, 0);
        }
    }
    err
}
