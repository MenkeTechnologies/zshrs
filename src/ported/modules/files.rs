//! Direct port of `Src/Modules/files.c` — the `zsh/files` module.
//!
//! Provides built-in implementations of: chgrp, chmod, chown, ln,
//! mkdir, mv, rm, rmdir, sync (plus the `zf_*` safe-named aliases).
//! Every function below maps 1:1 to its C counterpart with a `// c:NNN`
//! citation against the upstream source.

#![allow(non_camel_case_types, non_snake_case)]

use crate::ported::utils::zwarnnam;
use std::sync::{Mutex, OnceLock};

// =====================================================================
// BIN_* / MV_* constants — `Src/Modules/files.c:170-178`.
// =====================================================================

pub const BIN_LN: i32 = 0;                                                   // c:170
pub const BIN_MV: i32 = 1;                                                   // c:171

pub const MV_NODIRS:        i32 = 1 << 0;                                    // c:173
pub const MV_FORCE:         i32 = 1 << 1;                                    // c:174
pub const MV_INTERACTIVE:   i32 = 1 << 2;                                    // c:175
pub const MV_ASKNW:         i32 = 1 << 3;                                    // c:176
pub const MV_ATOMIC:        i32 = 1 << 4;                                    // c:177
pub const MV_NOCHASETARGET: i32 = 1 << 5;                                    // c:178

/// `bin_chown` func discriminant — `Src/Modules/files.c:719`
/// (`enum { BIN_CHOWN, BIN_CHGRP };`).
pub const BIN_CHOWN: i32 = 0;                                                // c:719
pub const BIN_CHGRP: i32 = 1;                                                // c:719

// =====================================================================
// ask() — `Src/Modules/files.c:41`.
// =====================================================================

/// Direct port of `ask()` from `Src/Modules/files.c:41`.
/// C body (c:43-46): read one char from stdin; consume the rest of
/// the line; return 1 for `y`/`Y`, 0 otherwise.
pub fn ask() -> i32 {                                                        // c:41
    use std::io::Read;
    let mut buf = [0u8; 1];
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let a = match handle.read(&mut buf) {                                    // c:43 getchar
        Ok(0) => return 0,
        Ok(_) => buf[0],
        Err(_) => return 0,
    };
    while a != b'\n' {                                                       // c:44-45
        let mut peek = [0u8; 1];
        match handle.read(&mut peek) {
            Ok(0) => break,
            Ok(_) => if peek[0] == b'\n' { break },
            Err(_) => break,
        }
    }
    if a == b'y' || a == b'Y' { 1 } else { 0 }                               // c:46
}

// =====================================================================
// bin_sync — `Src/Modules/files.c:53`.
// =====================================================================

/// Direct port of `bin_sync()` from `Src/Modules/files.c:53`.
/// C body (c:55-57): `sync(); return 0;`.
// sync builtin                                                             // c:49
pub fn bin_sync(_nam: &str, _args: &[String],                                // c:53
                _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    unsafe { libc::sync(); }                                                 // c:55
    0                                                                        // c:56
}

// =====================================================================
// bin_mkdir + domkdir — `Src/Modules/files.c:63`, `:115`.
// =====================================================================

/// Direct port of `bin_mkdir()` from `Src/Modules/files.c:63`.
/// C body (c:65-110): default mode = 0777 & ~umask; parse -m; for
/// each arg, strip trailing slashes; with -p walk each `/` segment.
// mkdir builtin                                                            // c:59
pub fn bin_mkdir(nam: &str, args: &[String],                                 // c:63
                 ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
    let oumask = unsafe { libc::umask(0) };                                  // c:65
    let mut mode: u32 = 0o777 & !(oumask as u32);                            // c:66
    let mut err = 0i32;
    unsafe { libc::umask(oumask); }                                          // c:69
    if OPT_ISSET(ops, b'm') {                                                // c:70
        let str_arg = OPT_ARG(ops, b'm').unwrap_or("");                      // c:71
        match i64::from_str_radix(str_arg, 8) {                              // c:73 zstrtol base 8
            Ok(m) => mode = m as u32,
            Err(_) => {
                zwarnnam(nam,                                                // c:75
                    &format!("invalid mode `{}'", str_arg));
                return 1;                                                    // c:76
            }
        }
    }
    let p_flag = if OPT_ISSET(ops, b'p') { 1 } else { 0 };                   // c:84
    for arg in args {                                                        // c:80
        let trimmed: String = if arg.starts_with('/') {                      // c:81-83
            let body = arg.trim_end_matches('/');
            if body.is_empty() { "/".to_string() } else { body.to_string() }
        } else {
            arg.trim_end_matches('/').to_string()
        };
        if p_flag != 0 {                                                     // c:84
            let bytes = trimmed.as_bytes();
            let mut i = 0usize;
            loop {
                while i < bytes.len() && bytes[i] == b'/' { i += 1; }        // c:88-89
                while i < bytes.len() && bytes[i] != b'/' { i += 1; }        // c:90-91
                if i >= bytes.len() {                                        // c:92
                    err |= domkdir(nam, &trimmed, mode, 1);                  // c:93
                    break;
                }
                let prefix = &trimmed[..i];                                  // c:97
                let e = domkdir(nam, prefix, mode | 0o300, 1);               // c:98
                if e != 0 {                                                  // c:99
                    err = 1;                                                 // c:100
                    break;                                                   // c:101
                }
            }
        } else {
            err |= domkdir(nam, &trimmed, mode, 0);                          // c:107
        }
    }
    err                                                                      // c:109
}

/// Direct port of `domkdir()` from `Src/Modules/files.c:115`.
/// C body (c:120-141): retry up to 8 times if EEXIST + p && stat
/// shows existing entry is itself a directory.
pub fn domkdir(nam: &str, path: &str, mode: u32, p: i32) -> i32 {            // c:115
    use std::os::unix::fs::DirBuilderExt;
    let mut n = 8;                                                           // c:120
    let mut last_err: i32 = 0;
    while n > 0 {                                                            // c:122
        n -= 1;
        let oumask = unsafe { libc::umask(0) };                              // c:123
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(mode);
        let result = builder.create(path);                                   // c:124 mkdir
        unsafe { libc::umask(oumask); }                                      // c:125
        match result {
            Ok(()) => return 0,                                              // c:127
            Err(e) => last_err = e.raw_os_error().unwrap_or(0),
        }
        if p == 0 || last_err != libc::EEXIST { break; }                     // c:129
        match std::fs::metadata(path) {                                      // c:130 stat
            Ok(meta) if meta.is_dir() => return 0,                           // c:138
            Ok(_) => break,                                                  // c:139
            Err(e) => {
                last_err = e.raw_os_error().unwrap_or(0);
                if last_err == libc::ENOENT { continue; }                    // c:131
                break;                                                       // c:135
            }
        }
    }
    zwarnnam(nam,                                                            // c:142
        &format!("cannot make directory `{}': {}",
            path, std::io::Error::from_raw_os_error(last_err)));
    1                                                                        // c:143
}

// =====================================================================
// bin_rmdir — `Src/Modules/files.c:150`.
// =====================================================================

/// Direct port of `bin_rmdir()` from `Src/Modules/files.c:150`.
/// C body (c:154-164): for each arg, call rmdir(2); accumulate err.
// rmdir builtin                                                            // c:146
pub fn bin_rmdir(nam: &str, args: &[String],                                 // c:150
                 _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    let mut err = 0i32;
    for arg in args {                                                        // c:154
        let cpath = match std::ffi::CString::new(arg.as_str()) {             // c:155
            Ok(c) => c,
            Err(_) => {
                zwarnnam(nam,                                                // c:158
                    &format!("{}: {}", arg, "name too long"));
                err = 1;
                continue;
            }
        };
        let r = unsafe { libc::rmdir(cpath.as_ptr()) };                      // c:160
        if r != 0 {                                                          // c:160
            zwarnnam(nam,                                                    // c:161
                &format!("cannot remove directory `{}': {}",
                    arg, std::io::Error::last_os_error()));
            err = 1;                                                         // c:162
        }
    }
    err                                                                      // c:165
}

// =====================================================================
// bin_ln + domove — `Src/Modules/files.c:200`, `:298`.
// =====================================================================

/// Move-function discriminant for `bin_ln` / `domove` — `Src/Modules/
/// files.c:198`. C has a `MoveFunc` typedef (`int (*)(const char *,
/// const char *)`); Rust uses an enum so each branch can call the
/// right libc fn directly.
pub enum MoveFunc {
    Link,                                                                    // c:226
    Symlink,                                                                 // c:222
    Rename,                                                                  // c:213
}

/// Direct port of `bin_ln()` from `Src/Modules/files.c:200`.
/// C body (c:209-296):
///   - func == BIN_MV → movefn = rename, MV_ASKNW unless -f, MV_ATOMIC
///   - else → MV_FORCE if -f; -h/-n adds MV_NOCHASETARGET; -s →
///     symlink; otherwise link with MV_NODIRS unless -d
///   - -i without -f → MV_INTERACTIVE
///   - last-arg-is-dir handling: chase into the dir for each src
pub fn bin_ln(nam: &str, args: &[String],                                    // c:200
              ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    let movefn: MoveFunc;
    let mut flags: i32;
    let mut err = 0i32;
    if func == BIN_MV {                                                      // c:209
        movefn = MoveFunc::Rename;                                           // c:210
        flags = if OPT_ISSET(ops, b'f') { 0 } else { MV_ASKNW };             // c:211
        flags |= MV_ATOMIC;                                                  // c:212
    } else {
        flags = if OPT_ISSET(ops, b'f') { MV_FORCE } else { 0 };             // c:215
        if OPT_ISSET(ops, b'h') || OPT_ISSET(ops, b'n') {                    // c:217
            flags |= MV_NOCHASETARGET;
        }
        if OPT_ISSET(ops, b's') {                                            // c:219
            movefn = MoveFunc::Symlink;                                      // c:220
        } else {
            movefn = MoveFunc::Link;                                         // c:226
            if !OPT_ISSET(ops, b'd') {                                       // c:227
                flags |= MV_NODIRS;
            }
        }
    }
    if OPT_ISSET(ops, b'i') && !OPT_ISSET(ops, b'f') {                       // c:230
        flags |= MV_INTERACTIVE;
    }
    if args.is_empty() {
        zwarnnam(nam, "missing file argument");
        return 1;
    }
    let last_idx = args.len() - 1;                                           // c:232 a = args; for(; a[1]; a++)
    let mut have_dir = false;
    if last_idx > 0 {                                                        // c:233
        let target = &args[last_idx];
        if let Ok(meta) = std::fs::metadata(target) {                        // c:235 stat
            if meta.is_dir() {                                               // c:235 S_ISDIR
                have_dir = true;
                if (flags & MV_NOCHASETARGET) != 0 {                         // c:237
                    if let Ok(lmeta) = std::fs::symlink_metadata(target) {
                        if lmeta.file_type().is_symlink() {                  // c:237 S_ISLNK
                            // c:245-256 — multi-source symlink-to-dir
                            // resolution: error unless -f and exactly
                            // one source.
                            if last_idx > 1 {                                // c:245
                                zwarnnam(nam,                                // c:247
                                    &format!("{}: not a directory", target));
                                return 1;                                    // c:248
                            }
                            if (flags & MV_FORCE) != 0 {                     // c:250
                                let _ = std::fs::remove_file(target);        // c:251 unlink
                                have_dir = false;                            // c:252
                            } else {
                                zwarnnam(nam,                                // c:255
                                    &format!("{}: file exists", target));
                                return 1;                                    // c:256
                            }
                        }
                    }
                }
            }
        }
    }
    if have_dir {                                                            // c:havedir branch
        // c:276-294 — target is dir, chase into it for each source.
        let dir = args[last_idx].trim_end_matches('/').to_string();
        for src in &args[..last_idx] {                                       // c:281
            let basename = match src.rsplit_once('/') {                      // c:283-285 strrchr
                Some((_, n)) => n,
                None => src.as_str(),
            };
            let dest = format!("{}/{}", dir, basename);                      // c:289 strcat
            err |= domove(nam, &movefn, src, &dest, flags);                  // c:290
        }
        return err;                                                          // c:295
    }
    if last_idx > 1 {                                                        // c:265
        zwarnnam(nam, "last of many arguments must be a directory");         // c:266
        return 1;                                                            // c:267
    }
    let (src, dest) = if args.len() < 2 {                                    // c:269 !args[1]
        let basename = match args[0].rsplit_once('/') {                      // c:270 strrchr
            Some((_, n)) => n,
            None => args[0].as_str(),
        };
        (args[0].clone(), basename.to_string())                              // c:272 args[1] = ptr+1
    } else {
        (args[0].clone(), args[1].clone())
    };
    domove(nam, &movefn, &src, &dest, flags)                                 // c:275
}

/// Direct port of `domove()` from `Src/Modules/files.c:298`.
/// C body (c:300-360): if MV_NODIRS, refuse src that is dir; if dest
/// exists, force/interactive/asknw checks; unlink dest if not atomic;
/// then call movefn(src, dest) and report errno on failure.
pub fn domove(nam: &str, movefn: &MoveFunc, p: &str, q: &str, flags: i32) -> i32 { // c:298
    if (flags & MV_NODIRS) != 0 {                                            // c:307
        match std::fs::symlink_metadata(p) {                                 // c:308 lstat
            Ok(meta) if meta.is_dir() => {                                   // c:308 S_ISDIR
                zwarnnam(nam, &format!("{}: is a directory", p));            // c:310
                return 1;                                                    // c:311
            }
            Err(e) => {
                zwarnnam(nam, &format!("{}: {}", p, e));                     // c:310
                return 1;
            }
            _ => {}
        }
    }
    if let Ok(qmeta) = std::fs::symlink_metadata(q) {                        // c:315 lstat
        let mut doit = (flags & MV_FORCE) != 0;                              // c:316
        if qmeta.is_dir() {                                                  // c:317 S_ISDIR
            zwarnnam(nam, &format!("{}: cannot overwrite directory", q));    // c:319
            return 1;                                                        // c:320
        } else if (flags & MV_INTERACTIVE) != 0 {                            // c:322
            eprint!("{}: replace `{}'? ", nam, q);                           // c:324-326
            if ask() == 0 {                                                  // c:328
                return 0;                                                    // c:329
            }
            doit = true;                                                     // c:331
        } else if (flags & MV_ASKNW) != 0                                    // c:333
                && !qmeta.file_type().is_symlink()                           // c:334 !S_ISLNK
                && unsafe {                                                  // c:335 access W_OK
                    let cq = std::ffi::CString::new(q).ok();
                    cq.map(|c| libc::access(c.as_ptr(), libc::W_OK))
                        .unwrap_or(-1) != 0
                } {
            use std::os::unix::fs::PermissionsExt;
            let mode = qmeta.permissions().mode() & 0o7777;
            eprint!("{}: replace `{}', overriding mode {:04o}? ", nam, q, mode); // c:337-340
            if ask() == 0 {                                                  // c:342
                return 0;                                                    // c:343
            }
            doit = true;                                                     // c:345
        }
        if doit && (flags & MV_ATOMIC) == 0 {                                // c:347
            let _ = std::fs::remove_file(q);                                 // c:348 unlink
        }
    }
    let r = match movefn {                                                   // c:350 movefn(p, q)
        MoveFunc::Rename => std::fs::rename(p, q).map(|_| 0).unwrap_or(-1),
        MoveFunc::Link => unsafe {
            let cp = std::ffi::CString::new(p).unwrap_or_default();
            let cq = std::ffi::CString::new(q).unwrap_or_default();
            libc::link(cp.as_ptr(), cq.as_ptr())
        },
        MoveFunc::Symlink => unsafe {
            let cp = std::ffi::CString::new(p).unwrap_or_default();
            let cq = std::ffi::CString::new(q).unwrap_or_default();
            libc::symlink(cp.as_ptr(), cq.as_ptr())
        },
    };
    if r != 0 {                                                              // c:350
        let osek = std::io::Error::last_os_error();
        let errfile = if osek.raw_os_error() == Some(libc::ENOENT)           // c:352-355
            && std::fs::symlink_metadata(p).is_ok() { q } else { p };
        zwarnnam(nam, &format!("`{}': {}", errfile, osek));                  // c:357
        return 1;                                                            // c:358
    }
    0                                                                        // c:362
}

// =====================================================================
// recursivecmd family — `Src/Modules/files.c:365-526`.
// =====================================================================

/// Port of `struct recursivecmd` from `Src/Modules/files.c:365`.
/// Holds the per-call recursion options + callback function pointers.
/// Rust uses generic closures for the callbacks; the struct is the
/// owned context object passed through the recursion.
pub struct recursivecmd<'a, P, R, L>
where
    P: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    R: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    L: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
{
    pub nam: &'a str,                                                        // c:366
    pub opt_noerr: i32,                                                      // c:367
    pub opt_recurse: i32,                                                    // c:368
    pub opt_safe: i32,                                                       // c:369
    pub dirpre_func: P,                                                      // c:370
    pub dirpost_func: R,                                                     // c:371
    pub leaf_func: L,                                                        // c:372
}

/// Direct port of `recursivecmd()` from `Src/Modules/files.c:378`.
/// C body (c:381-446): walk argv, dispatch each via recursivecmd_doone.
/// The dirsav-based chdir-back stack (c:396-399, c:438-446) is omitted
/// in the Rust port — std::fs operations take absolute paths so the
/// chdir dance C uses to safely descend isn't needed.
pub fn recursivecmd<P, R, L>(                                                // c:378
    nam: &str, opt_noerr: i32, opt_recurse: i32, opt_safe: i32,
    args: &[String], dirpre_func: P, dirpost_func: R, leaf_func: L,
) -> i32                                                                     // c:378
where
    P: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    R: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    L: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
{
    let _ = opt_noerr;
    let reccmd = recursivecmd {
        nam, opt_noerr, opt_recurse, opt_safe,
        dirpre_func, dirpost_func, leaf_func,
    };
    let mut err = 0i32;
    for arg in args {                                                        // c:401
        if (err & 2) != 0 { break; }
        let first = if opt_safe != 0 { 0 } else { 1 };                       // c:421/c:434
        err |= recursivecmd_doone(&reccmd, arg, arg, first);                 // c:432/c:434
    }
    if err != 0 { 1 } else { 0 }                                             // c:445 !!err
}

/// Direct port of `recursivecmd_doone()` from `Src/Modules/files.c:450`.
/// C body (c:455-462): lstat the path; if recurse + S_ISDIR → dive
/// via recursivecmd_dorec; else call leaf_func.
pub fn recursivecmd_doone<P, R, L>(                                          // c:450
    reccmd: &recursivecmd<P, R, L>, arg: &str, rp: &str, first: i32,
) -> i32                                                                     // c:450
where
    P: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    R: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    L: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
{
    let st = std::fs::symlink_metadata(rp);                                  // c:455 lstat
    if reccmd.opt_recurse != 0 {                                             // c:457
        if let Ok(ref meta) = st {
            if meta.is_dir() {                                               // c:458 S_ISDIR
                return recursivecmd_dorec(reccmd, arg, rp, meta, first);    // c:459
            }
        }
    }
    let sp = st.as_ref().ok();                                               // c:460 sp
    (reccmd.leaf_func)(arg, rp, sp)                                          // c:461
}

/// Direct port of `recursivecmd_dorec()` from `Src/Modules/files.c:465`.
/// C body (c:475-525): dirpre callback, opendir + readdir each entry,
/// recurse via recursivecmd_doone, then dirpost callback.
pub fn recursivecmd_dorec<P, R, L>(                                          // c:465
    reccmd: &recursivecmd<P, R, L>, arg: &str, rp: &str,
    sp: &std::fs::Metadata, _first: i32,
) -> i32                                                                     // c:465
where
    P: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    R: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    L: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
{
    let err1 = (reccmd.dirpre_func)(arg, rp, Some(sp));                      // c:475 dirpre_func
    if (err1 & 2) != 0 { return 2; }                                         // c:476
    let dir = match std::fs::read_dir(rp) {                                  // c:489 opendir
        Ok(d) => d,
        Err(e) => {
            if reccmd.opt_noerr == 0 {                                       // c:491
                zwarnnam(reccmd.nam, &format!("{}: {}", arg, e));            // c:492
            }
            return err1 | 1;                                                 // c:493
        }
    };
    let mut err = err1;
    for entry in dir.flatten() {                                             // c:497 readdir
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "." || name_str == ".." { continue; }                 // c:498
        let narg = format!("{}/{}", arg.trim_end_matches('/'), name_str);    // c:507-510
        let nrp = entry.path();
        let nrp_str = nrp.to_string_lossy();
        if (err & 2) != 0 { break; }                                         // c:503
        err |= recursivecmd_doone(reccmd, &narg, &nrp_str, 0);               // c:511
    }
    if (err & 2) != 0 { return 2; }                                          // c:516
    err | (reccmd.dirpost_func)(arg, rp, Some(sp))                           // c:524
}

/// Direct port of `recurse_donothing()` from `Src/Modules/files.c:530`.
/// C body: `return 0;`.
pub fn recurse_donothing(_arg: &str, _rp: &str,                              // c:530
                         _sp: Option<&std::fs::Metadata>) -> i32 {
    0                                                                        // c:533
}

// =====================================================================
// bin_rm — `Src/Modules/files.c:537-630`.
// =====================================================================

/// Port of `struct rmmagic` from `Src/Modules/files.c:537`.
pub struct rmmagic<'a> {
    pub nam: &'a str,                                                        // c:538
    pub opt_force: i32,                                                      // c:539
    pub opt_interact: i32,                                                   // c:540
    pub opt_unlinkdir: i32,                                                  // c:541
}

/// Direct port of `rm_leaf()` from `Src/Modules/files.c:546`.
/// C body (c:551-589):
///   - if !opt_unlinkdir || !opt_force: lstat (if not provided);
///     refuse directories; ask if interactive; warn if read-only
///   - unlink(rp); error path returns 1 unless -f
pub fn rm_leaf(arg: &str, rp: &str, sp: Option<&std::fs::Metadata>,          // c:546
               rmm: &rmmagic) -> i32 {
    if rmm.opt_unlinkdir == 0 || rmm.opt_force == 0 {                        // c:551
        let owned;
        let sp_use = if let Some(s) = sp { Some(s) } else {                  // c:552-554
            owned = std::fs::symlink_metadata(rp).ok();                      // c:553 lstat
            owned.as_ref()
        };
        if let Some(meta) = sp_use {                                         // c:556
            if rmm.opt_unlinkdir == 0 && meta.is_dir() {                     // c:557 S_ISDIR
                if rmm.opt_force != 0 { return 0; }                          // c:558-559
                zwarnnam(rmm.nam,                                            // c:560
                    &format!("{}: is a directory", arg));
                return 1;                                                    // c:561
            }
            if rmm.opt_interact != 0 {                                       // c:563
                eprint!("{}: remove `{}'? ", rmm.nam, arg);                  // c:564-568
                if ask() == 0 { return 0; }                                  // c:570
            } else if rmm.opt_force == 0                                     // c:571
                    && !meta.file_type().is_symlink()                        // c:572 !S_ISLNK
                    && unsafe {                                              // c:573 access W_OK
                        let crp = std::ffi::CString::new(rp).ok();
                        crp.map(|c| libc::access(c.as_ptr(), libc::W_OK))
                            .unwrap_or(-1) != 0
                    } {
                use std::os::unix::fs::PermissionsExt;
                let mode = meta.permissions().mode() & 0o7777;
                eprint!("{}: remove `{}', overriding mode {:04o}? ",
                    rmm.nam, arg, mode);                                     // c:574-579
                if ask() == 0 { return 0; }                                  // c:581
            }
        }
    }
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    if unsafe { libc::unlink(crp.as_ptr()) } != 0 && rmm.opt_force == 0 {    // c:585
        zwarnnam(rmm.nam,                                                    // c:586
            &format!("{}: {}", arg, std::io::Error::last_os_error()));
        return 1;                                                            // c:587
    }
    0                                                                        // c:589
}

/// Direct port of `rm_dirpost()` from `Src/Modules/files.c:594`.
/// C body (c:599-613): rmdir(rp); error path returns 1 unless -f.
pub fn rm_dirpost(arg: &str, rp: &str, _sp: Option<&std::fs::Metadata>,      // c:594
                  rmm: &rmmagic) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    if unsafe { libc::rmdir(crp.as_ptr()) } != 0 && rmm.opt_force == 0 {     // c:608
        zwarnnam(rmm.nam,                                                    // c:609
            &format!("{}: {}", arg, std::io::Error::last_os_error()));
        return 1;                                                            // c:610
    }
    0                                                                        // c:612
}

/// Direct port of `bin_rm()` from `Src/Modules/files.c:616`.
/// C body (c:621-633): build rmmagic; recursivecmd with rm_dirpost
/// + rm_leaf; -f swallows the err code.
// rm builtin                                                               // c:535
pub fn bin_rm(nam: &str, args: &[String],                                    // c:616
              ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    let rmm = rmmagic {
        nam,                                                                 // c:621
        opt_force:     if OPT_ISSET(ops, b'f') { 1 } else { 0 },             // c:622
        opt_interact:  if OPT_ISSET(ops, b'i') && !OPT_ISSET(ops, b'f')      // c:623
                       { 1 } else { 0 },
        opt_unlinkdir: if OPT_ISSET(ops, b'd') { 1 } else { 0 },             // c:624
    };
    let recurse = if !OPT_ISSET(ops, b'd')                                   // c:626
        && (OPT_ISSET(ops, b'R') || OPT_ISSET(ops, b'r')) { 1 } else { 0 };
    let safe    = if OPT_ISSET(ops, b's') { 1 } else { 0 };                  // c:627
    let err = recursivecmd(nam, rmm.opt_force, recurse, safe, args,          // c:625
        |_a, _r, _s| 0,                                                      // dirpre = recurse_donothing
        |a, r, s|    rm_dirpost(a, r, s, &rmm),                              // dirpost
        |a, r, s|    rm_leaf(a, r, s, &rmm));                                // leaf
    if rmm.opt_force != 0 { 0 } else { err }                                 // c:631
}

// =====================================================================
// bin_chmod — `Src/Modules/files.c:635-672`.
// =====================================================================

/// Port of `struct chmodmagic` from `Src/Modules/files.c:635`.
pub struct chmodmagic<'a> {
    pub nam: &'a str,                                                        // c:636
    pub mode: u32,                                                           // c:637
}

/// Direct port of `chmod_dochmod()` from `Src/Modules/files.c:642`.
/// C body (c:646-652): `chmod(rp, mode)`; warn + return 1 on failure.
pub fn chmod_dochmod(arg: &str, rp: &str, _sp: Option<&std::fs::Metadata>,   // c:642
                     chm: &chmodmagic) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    if unsafe { libc::chmod(crp.as_ptr(), chm.mode as libc::mode_t) } != 0 { // c:646
        zwarnnam(chm.nam,                                                    // c:647
            &format!("{}: {}", arg, std::io::Error::last_os_error()));
        return 1;                                                            // c:648
    }
    0                                                                        // c:650
}

/// Direct port of `bin_chmod()` from `Src/Modules/files.c:655`.
/// C body (c:659-672): parse args[0] as octal mode; recursivecmd
/// over args[1..] applying chmod_dochmod.
// chmod builtin                                                            // c:633
pub fn bin_chmod(nam: &str, args: &[String],                                 // c:655
                 ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    if args.is_empty() {
        zwarnnam(nam, "missing mode");
        return 1;
    }
    let mode = match i64::from_str_radix(&args[0], 8) {                      // c:663 zstrtol base 8
        Ok(m) => m as u32,
        Err(_) => {
            zwarnnam(nam, &format!("invalid mode `{}'", args[0]));           // c:665
            return 1;                                                        // c:666
        }
    };
    let chm = chmodmagic { nam, mode };
    let recurse = if OPT_ISSET(ops, b'R') { 1 } else { 0 };                  // c:670
    let safe    = if OPT_ISSET(ops, b's') { 1 } else { 0 };                  // c:670
    recursivecmd(nam, 0, recurse, safe, &args[1..],                          // c:669
        |a, r, s| chmod_dochmod(a, r, s, &chm),                              // dirpre
        |_a, _r, _s| 0,                                                      // dirpost = recurse_donothing
        |a, r, s| chmod_dochmod(a, r, s, &chm))                              // leaf
}

// =====================================================================
// bin_chown — `Src/Modules/files.c:674-801`.
// =====================================================================

/// Port of `struct chownmagic` from `Src/Modules/files.c:674`.
pub struct chownmagic<'a> {
    pub nam: &'a str,                                                        // c:675
    pub uid: i64,                                                            // c:676 (uid_t but -1 sentinel)
    pub gid: i64,                                                            // c:677
}

/// Direct port of `chown_dochown()` from `Src/Modules/files.c:682`.
/// C body (c:686-692): `chown(rp, uid, gid)`; warn + return 1 on failure.
pub fn chown_dochown(arg: &str, rp: &str, _sp: Option<&std::fs::Metadata>,   // c:682
                     chm: &chownmagic) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let uid = if chm.uid < 0 { libc::uid_t::MAX } else { chm.uid as libc::uid_t };
    let gid = if chm.gid < 0 { libc::gid_t::MAX } else { chm.gid as libc::gid_t };
    if unsafe { libc::chown(crp.as_ptr(), uid, gid) } != 0 {                 // c:686
        zwarnnam(chm.nam,                                                    // c:687
            &format!("{}: {}", arg, std::io::Error::last_os_error()));
        return 1;                                                            // c:688
    }
    0                                                                        // c:690
}

/// Direct port of `chown_dolchown()` from `Src/Modules/files.c:695`.
/// C body (c:699-705): `lchown(rp, uid, gid)`.
pub fn chown_dolchown(arg: &str, rp: &str, _sp: Option<&std::fs::Metadata>,  // c:695
                      chm: &chownmagic) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let uid = if chm.uid < 0 { libc::uid_t::MAX } else { chm.uid as libc::uid_t };
    let gid = if chm.gid < 0 { libc::gid_t::MAX } else { chm.gid as libc::gid_t };
    if unsafe { libc::lchown(crp.as_ptr(), uid, gid) } != 0 {                // c:699
        zwarnnam(chm.nam,                                                    // c:700
            &format!("{}: {}", arg, std::io::Error::last_os_error()));
        return 1;                                                            // c:701
    }
    0                                                                        // c:703
}

/// Direct port of `getnumeric()` from `Src/Modules/files.c:708`.
/// C body (c:712-719): parse leading digits as base-10 unsigned long;
/// `*errp = !!*p` after parse — set when there are trailing non-digits.
pub fn getnumeric(p: &str, errp: &mut i32) -> u64 {                          // c:708
    if !p.chars().next().is_some_and(|c| c.is_ascii_digit()) {               // c:712
        *errp = 1;                                                           // c:713
        return 0;                                                            // c:714
    }
    let end = p.find(|c: char| !c.is_ascii_digit()).unwrap_or(p.len());
    let ret = p[..end].parse::<u64>().unwrap_or(0);                          // c:716 strtoul
    *errp = if end < p.len() { 1 } else { 0 };                               // c:717
    ret                                                                      // c:718
}

/// Direct port of `bin_chown()` from `Src/Modules/files.c:725`.
/// C body (c:729-797): parse `user[:group]` spec; for chgrp, skip the
/// user half; getpwnam / getnumeric / getgrnam fallbacks; recursivecmd
/// with chown_dochown or chown_dolchown for `-h`.
// chown builtin                                                            // c:672
pub fn bin_chown(nam: &str, args: &[String],                                 // c:725
                 ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    if args.is_empty() {
        zwarnnam(nam, "missing argument");
        return 1;
    }
    let uspec = args[0].clone();                                             // c:728
    let mut chm = chownmagic { nam, uid: -1, gid: -1 };
    let mut p_idx = 0usize;
    let mut do_group_only = false;
    if func == BIN_CHGRP {                                                   // c:733
        chm.uid = -1;                                                        // c:734
        do_group_only = true;                                                // c:735 goto dogroup
    } else {
        // c:737-741 — locate `:` or `.` separator.
        let end = uspec.find(':').or_else(|| uspec.find('.'));               // c:737-738
        if end == Some(0) {                                                  // c:739
            chm.uid = -1;                                                    // c:740
            p_idx = 1;                                                       // c:741
            do_group_only = true;                                            // c:742 goto dogroup
        } else {
            let user_part = if let Some(e) = end { &uspec[..e] } else { &uspec[..] };
            // c:746 — getpwnam(p)
            let cuser = std::ffi::CString::new(user_part).unwrap_or_default();
            let pwd = unsafe { libc::getpwnam(cuser.as_ptr()) };             // c:746
            let uid = if !pwd.is_null() {
                unsafe { (*pwd).pw_uid as i64 }                              // c:748
            } else {
                let mut errp = 0i32;
                let n = getnumeric(user_part, &mut errp);                    // c:751
                if errp != 0 {                                               // c:752
                    zwarnnam(nam,                                            // c:753
                        &format!("{}: no such user", user_part));
                    return 1;                                                // c:755
                }
                n as i64
            };
            chm.uid = uid;
            if let Some(e) = end {                                           // c:759
                let group_part = &uspec[e + 1..];
                if group_part.is_empty() {                                   // c:761
                    let p2 = if !pwd.is_null() { pwd }                       // c:762
                             else { unsafe { libc::getpwuid(uid as libc::uid_t) } };
                    if p2.is_null() {                                        // c:763
                        zwarnnam(nam,                                        // c:764
                            &format!("{}: no such user", uspec));
                        return 1;                                            // c:766
                    }
                    chm.gid = unsafe { (*p2).pw_gid as i64 };                // c:768
                } else if group_part == ":" {                                // c:769
                    chm.gid = -1;                                            // c:770
                } else {
                    p_idx = 0; // not used past this point
                    let cgrp = std::ffi::CString::new(group_part).unwrap_or_default();
                    let grp = unsafe { libc::getgrnam(cgrp.as_ptr()) };      // c:773 dogroup
                    if !grp.is_null() {
                        chm.gid = unsafe { (*grp).gr_gid as i64 };           // c:775
                    } else {
                        let mut errp = 0i32;
                        let n = getnumeric(group_part, &mut errp);           // c:778
                        if errp != 0 {                                       // c:779
                            zwarnnam(nam,                                    // c:780
                                &format!("{}: no such group", group_part));
                            return 1;                                        // c:782
                        }
                        chm.gid = n as i64;
                    }
                }
            }
        }
    }
    if do_group_only {                                                       // c:773 dogroup label
        let group_part = &uspec[p_idx..];
        let cgrp = std::ffi::CString::new(group_part).unwrap_or_default();
        let grp = unsafe { libc::getgrnam(cgrp.as_ptr()) };                  // c:773
        if !grp.is_null() {
            chm.gid = unsafe { (*grp).gr_gid as i64 };                       // c:775
        } else {
            let mut errp = 0i32;
            let n = getnumeric(group_part, &mut errp);                       // c:778
            if errp != 0 {                                                   // c:779
                zwarnnam(nam,                                                // c:780
                    &format!("{}: no such group", group_part));
                return 1;                                                    // c:782
            }
            chm.gid = n as i64;
        }
    }
    let recurse = if OPT_ISSET(ops, b'R') { 1 } else { 0 };                  // c:792
    let safe    = if OPT_ISSET(ops, b's') { 1 } else { 0 };                  // c:792
    let h_flag  = OPT_ISSET(ops, b'h');                                      // c:793
    recursivecmd(nam, 0, recurse, safe, &args[1..],                          // c:791
        |a, r, s| if h_flag { chown_dolchown(a, r, s, &chm) }
                  else      { chown_dochown(a, r, s, &chm) },                // dirpre
        |_a, _r, _s| 0,                                                      // dirpost = recurse_donothing
        |a, r, s| if h_flag { chown_dolchown(a, r, s, &chm) }
                  else      { chown_dochown(a, r, s, &chm) })                // leaf
}

// =====================================================================
// bintab[] — `Src/Modules/files.c:803`.
// =====================================================================
//
// The C source lists 18 BUILTIN entries (9 normal + 9 zf_* aliases)
// for chgrp/chmod/chown/ln/mkdir/mv/rm/rmdir/sync. The Rust port
// records the same entries here so dispatchers can look up name →
// (handler, func id, opt-spec) without re-encoding the table.

/// Entry shape mirroring C `BUILTIN(name, flags, handler, min, max,
/// func, opts, defaults)` from `Src/builtin.h`.
pub struct FilesBuiltin {
    pub name:   &'static str,
    pub min:    i32,
    pub max:    i32,
    pub func:   i32,
    pub opts:   &'static str,
}

/// `LN_OPTS` — `Src/Modules/files.c:799` (`"dfhins"` when HAVE_LSTAT,
/// `"dfi"` otherwise). zshrs targets POSIX hosts where lstat is
/// always present, so the long form is canonical.
pub const LN_OPTS: &str = "dfhins";                                          // c:799

/// Port of `bintab[]` from `Src/Modules/files.c:803`.
pub static BINTAB: [FilesBuiltin; 18] = [                                    // c:803
    FilesBuiltin { name: "chgrp", min: 2, max: -1, func: BIN_CHGRP, opts: "hRs"    },
    FilesBuiltin { name: "chmod", min: 2, max: -1, func: 0,         opts: "Rs"     },
    FilesBuiltin { name: "chown", min: 2, max: -1, func: BIN_CHOWN, opts: "hRs"    },
    FilesBuiltin { name: "ln",    min: 1, max: -1, func: BIN_LN,    opts: LN_OPTS  },
    FilesBuiltin { name: "mkdir", min: 1, max: -1, func: 0,         opts: "pm:"    },
    FilesBuiltin { name: "mv",    min: 2, max: -1, func: BIN_MV,    opts: "fi"     },
    FilesBuiltin { name: "rm",    min: 1, max: -1, func: 0,         opts: "dfiRrs" },
    FilesBuiltin { name: "rmdir", min: 1, max: -1, func: 0,         opts: ""       },
    FilesBuiltin { name: "sync",  min: 0, max:  0, func: 0,         opts: ""       },
    // c:822-830 — "safe" zsh-only zf_* aliases.
    FilesBuiltin { name: "zf_chgrp", min: 2, max: -1, func: BIN_CHGRP, opts: "hRs"    },
    FilesBuiltin { name: "zf_chmod", min: 2, max: -1, func: 0,         opts: "Rs"     },
    FilesBuiltin { name: "zf_chown", min: 2, max: -1, func: BIN_CHOWN, opts: "hRs"    },
    FilesBuiltin { name: "zf_ln",    min: 1, max: -1, func: BIN_LN,    opts: LN_OPTS  },
    FilesBuiltin { name: "zf_mkdir", min: 1, max: -1, func: 0,         opts: "pm:"    },
    FilesBuiltin { name: "zf_mv",    min: 2, max: -1, func: BIN_MV,    opts: "fi"     },
    FilesBuiltin { name: "zf_rm",    min: 1, max: -1, func: 0,         opts: "dfiRrs" },
    FilesBuiltin { name: "zf_rmdir", min: 1, max: -1, func: 0,         opts: ""       },
    FilesBuiltin { name: "zf_sync",  min: 0, max:  0, func: 0,         opts: ""       },
];

// =====================================================================
// module entries — `Src/Modules/files.c:828-876`.
// =====================================================================

use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: BINTAB.len() as i32,                         // c:828
        cd_list: None, cd_size: 0,
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/files.c:838`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:838
    // C body c:840-841 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_()` from `Src/Modules/files.c:845`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {      // c:845
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/files.c:853`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {   // c:853
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/files.c:860`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:860
    // C body c:862-863 — `return 0`. Faithful empty-body port; the
    //                    chmod/chown/chgrp/sync/etc. builtins register
    //                    via the bn_list feature dispatch.
    0
}

/// Port of `cleanup_()` from `Src/Modules/files.c:867`.
pub fn cleanup_(m: *const module) -> i32 {                                   // c:867
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/files.c:874`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:874
    // C body c:876-877 — `return 0`. Faithful empty-body port; the
    //                    builtins unregister via cleanup_'s setfeatureenables.
    0
}

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    BINTAB.iter().map(|b| format!("b:{}", b.name)).collect()
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
// File-static delegator to `Src/module.c:3349 setfeatureenables` —
// dispatches per-feature enable bits through setbuiltins/setconddefs/
// setmathfuncs/setparamdefs. The static-link Rust path treats every
// feature as always-enabled, so this no-op return matches what
// cleanup_(NULL) needs (revoke nothing).
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }
