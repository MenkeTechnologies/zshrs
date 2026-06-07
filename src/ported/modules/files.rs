//! Direct port of `Src/Modules/files.c` — the `zsh/files` module.
//!
//! Provides built-in implementations of: chgrp, chmod, chown, ln,
//! mkdir, mv, rm, rmdir, sync (plus the `zf_*` safe-named aliases).
//! Every function below maps 1:1 to its C counterpart with a `// c:NNN`
//! citation against the upstream source.

#![allow(non_camel_case_types, non_snake_case)]

use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::{module, options, OPT_ARG, OPT_ISSET};
use std::io::Read;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::sync::{Mutex, OnceLock};

/// Direct port of `recursivecmd(char *nam, int opt_noerr, int opt_recurse, int opt_safe, char **args, RecurseFunc dirpre_func, RecurseFunc dirpost_func, RecurseFunc leaf_func, void *magic)` from `Src/Modules/files.c:378`.
/// C body (c:381-446): walk argv, dispatch each via recursivecmd_doone.
/// The dirsav-based chdir-back stack (c:396-399, c:438-446) is omitted
/// in the Rust port — std::fs operations take absolute paths so the
/// chdir dance C uses to safely descend isn't needed.
/// WARNING: param names don't match C — Rust=(opt_noerr, opt_recurse, opt_safe, args, dirpre_func, dirpost_func, leaf_func) vs C=(nam, opt_noerr, opt_recurse, opt_safe, args, dirpre_func, dirpost_func, leaf_func, magic)
pub fn recursivecmd<P, R, L>(
    // c:378
    nam: &str,
    opt_noerr: i32,
    opt_recurse: i32,
    opt_safe: i32,
    args: &[String],
    dirpre_func: P,
    dirpost_func: R,
    leaf_func: L,
) -> i32
// c:378
where
    P: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    R: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    L: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
{
    let _ = opt_noerr;
    let reccmd = recursivecmd {
        nam,
        opt_noerr,
        opt_recurse,
        opt_safe,
        dirpre_func,
        dirpost_func,
        leaf_func,
    };
    let mut err = 0i32;
    for arg in args {
        // c:401
        if (err & 2) != 0 {
            break;
        }
        let first = if opt_safe != 0 { 0 } else { 1 }; // c:421/c:434
        err |= recursivecmd_doone(&reccmd, arg, arg, first); // c:450/c:434
    }
    if err != 0 {
        1
    } else {
        0
    } // c:450 !!err
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
    pub nam: &'a str,     // c:366
    pub opt_noerr: i32,   // c:367
    pub opt_recurse: i32, // c:368
    pub opt_safe: i32,    // c:378
    pub dirpre_func: P,   // c:378
    pub dirpost_func: R,  // c:378
    pub leaf_func: L,     // c:378
}

// =====================================================================
// ask() — `Src/Modules/files.c:41`.
// =====================================================================

/// Direct port of `ask()` from `Src/Modules/files.c:41`.
/// C body (c:43-46): read one char from stdin; consume the rest of
/// the line; return 1 for `y`/`Y`, 0 otherwise.
pub fn ask() -> i32 {
    // c:41
    let mut bytes = std::io::stdin().lock().bytes();
    let a = bytes.next().and_then(|r| r.ok()).unwrap_or(0); // c:43 getchar
    for c in bytes.by_ref() {
        // c:44-45
        if matches!(c, Ok(b'\n') | Err(_)) {
            break;
        }
    }
    (a == b'y' || a == b'Y') as i32 // c:46
}

// =====================================================================
// bin_sync — `Src/Modules/files.c:53`.
// =====================================================================

/// Direct port of `bin_sync(UNUSED(char *nam), UNUSED(char **args), UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/files.c:53`.
/// C body (c:55-57): `sync(); return 0;`.
// sync builtin                                                             // c:53
/// WARNING: param names don't match C — Rust=(_nam, _args, _func) vs C=(nam, args, ops, func)
pub fn bin_sync(
    _nam: &str,
    _args: &[String], // c:53
    _ops: &options,
    _func: i32,
) -> i32 {
    unsafe {
        libc::sync();
    } // c:55
    0 // c:63
}

// =====================================================================
// bin_mkdir + domkdir — `Src/Modules/files.c:63`, `:115`.
// =====================================================================

/// Direct port of `bin_mkdir(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/files.c:63`.
/// C body (c:65-110): default mode = 0777 & ~umask; parse -m; for
/// each arg, strip trailing slashes; with -p walk each `/` segment.
// mkdir builtin                                                            // c:63
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_mkdir(
    nam: &str,
    args: &[String], // c:63
    ops: &options,
    _func: i32,
) -> i32 {
    // C's `BUILTIN("mkdir", 0, bin_mkdir, 1, -1, ...)` table entry
    // (Src/Modules/files.c:810) declares `minargs=1`; the builtin
    // dispatcher (Src/builtin.c:430) enforces it BEFORE calling
    // bin_mkdir. zshrs may call bin_mkdir directly (test paths /
    // fusevm bridge), so self-validate to keep the C-observable
    // "not enough arguments → exit 1" contract intact.
    if args.is_empty() {
        zwarnnam(nam, "not enough arguments"); // c:builtin.c:434
        return 1;
    }
    let oumask = unsafe { libc::umask(0) }; // c:65
    let mut mode: u32 = 0o777 & !(oumask as u32); // c:66
    let mut err = 0i32;
    unsafe {
        libc::umask(oumask);
    } // c:69
    if OPT_ISSET(ops, b'm') {
        // c:70
        let str_arg = OPT_ARG(ops, b'm').unwrap_or(""); // c:71
        match i64::from_str_radix(str_arg, 8) {
            // c:73 zstrtol base 8
            Ok(m) => mode = m as u32,
            Err(_) => {
                zwarnnam(
                    nam, // c:75
                    &format!("invalid mode `{}'", str_arg),
                );
                return 1; // c:76
            }
        }
    }
    let p_flag = if OPT_ISSET(ops, b'p') { 1 } else { 0 }; // c:84
    for arg in args {
        // c:80
        let trimmed: String = if arg.starts_with('/') {
            // c:81-83
            let body = arg.trim_end_matches('/');
            if body.is_empty() {
                "/".to_string()
            } else {
                body.to_string()
            }
        } else {
            arg.trim_end_matches('/').to_string()
        };
        if p_flag != 0 {
            // c:84
            let bytes = trimmed.as_bytes();
            let mut i = 0usize;
            loop {
                while i < bytes.len() && bytes[i] == b'/' {
                    i += 1;
                } // c:88-89
                while i < bytes.len() && bytes[i] != b'/' {
                    i += 1;
                } // c:90-91
                if i >= bytes.len() {
                    // c:92
                    err |= domkdir(nam, &trimmed, mode, 1); // c:93
                    break;
                }
                let prefix = &trimmed[..i]; // c:97
                let e = domkdir(nam, prefix, mode | 0o300, 1); // c:98
                if e != 0 {
                    // c:99
                    err = 1; // c:100
                    break; // c:101
                }
            }
        } else {
            err |= domkdir(nam, &trimmed, mode, 0); // c:115
        }
    }
    err // c:115
}

/// Direct port of `domkdir(char *nam, char *path, mode_t mode, int p)` from `Src/Modules/files.c:115`.
/// C body (c:120-141): retry up to 8 times if EEXIST + p && stat
/// shows existing entry is itself a directory.
pub fn domkdir(nam: &str, path: &str, mode: u32, p: i32) -> i32 {
    // c:115
    let mut n = 8; // c:120
    let mut last_err: i32 = 0;
    while n > 0 {
        // c:122
        n -= 1;
        let oumask = unsafe { libc::umask(0) }; // c:123
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(mode);
        // c:Src/Modules/files.c:domkdir — when `-p` is set, the C body
        // walks each intermediate path component, creating each as
        // needed (the loop at c:131 increments past ENOENT and retries).
        // The Rust port previously called `DirBuilder::create` which
        // creates ONLY the leaf and fails with ENOENT for missing
        // parents. Use `recursive(true)` so `mkdir -p a/b/c` works with
        // missing `a` and `b`. The non-`-p` path (`p == 0`) keeps the
        // strict single-level create so `mkdir x/y` with no `x` still
        // errors the same way C does.
        if p != 0 {
            builder.recursive(true);
        }
        let result = builder.create(path); // c:124 mkdir
        unsafe {
            libc::umask(oumask);
        } // c:125
        match result {
            Ok(()) => return 0, // c:127
            Err(e) => last_err = e.raw_os_error().unwrap_or(0),
        }
        if p == 0 || last_err != libc::EEXIST {
            break;
        } // c:129
        match std::fs::metadata(path) {
            // c:130 stat
            Ok(meta) if meta.is_dir() => return 0, // c:138
            Ok(_) => break,                        // c:139
            Err(e) => {
                last_err = e.raw_os_error().unwrap_or(0);
                if last_err == libc::ENOENT {
                    continue;
                } // c:131
                break; // c:135
            }
        }
    }
    zwarnnam(
        nam, // c:142
        &format!(
            "cannot make directory `{}': {}",
            path,
            std::io::Error::from_raw_os_error(last_err)
        ),
    );
    1 // c:150
}

// =====================================================================
// bin_rmdir — `Src/Modules/files.c:150`.
// =====================================================================

/// Direct port of `bin_rmdir(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/files.c:150`.
/// C body (c:154-164): for each arg, call rmdir(2); accumulate err.
// rmdir builtin                                                            // c:150
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_rmdir(
    nam: &str,
    args: &[String], // c:150
    _ops: &options,
    _func: i32,
) -> i32 {
    // C `BUILTIN("rmdir", 0, bin_rmdir, 1, -1, ...)` (files.c:813).
    // See bin_mkdir for why the minargs self-check sits here.
    if args.is_empty() {
        zwarnnam(nam, "not enough arguments"); // c:builtin.c:434
        return 1;
    }
    let mut err = 0i32;
    for arg in args {
        // c:154
        let cpath = match std::ffi::CString::new(arg.as_str()) {
            // c:155
            Ok(c) => c,
            Err(_) => {
                zwarnnam(
                    nam, // c:158
                    &format!("{}: {}", arg, "name too long"),
                );
                err = 1;
                continue;
            }
        };
        let r = unsafe { libc::rmdir(cpath.as_ptr()) }; // c:160
        if r != 0 {
            // c:160
            zwarnnam(
                nam, // c:161
                &format!(
                    "cannot remove directory `{}': {}",
                    arg,
                    crate::ported::compat::last_errstr()
                ),
            );
            err = 1; // c:162
        }
    }
    err // c:165
}

// =====================================================================
// BIN_* / MV_* constants — `Src/Modules/files.c:170-178`.
// =====================================================================
/// `BIN_LN` constant.
pub const BIN_LN: i32 = 0; // c:170
/// `BIN_MV` constant.
pub const BIN_MV: i32 = 1; // c:171
/// `MV_NODIRS` constant.
pub const MV_NODIRS: i32 = 1 << 0; // c:173
/// `MV_FORCE` constant.
pub const MV_FORCE: i32 = 1 << 1; // c:174
/// `MV_INTERACTIVE` constant.
pub const MV_INTERACTIVE: i32 = 1 << 2; // c:175
/// `MV_ASKNW` constant.
pub const MV_ASKNW: i32 = 1 << 3; // c:176
/// `MV_ATOMIC` constant.
pub const MV_ATOMIC: i32 = 1 << 4; // c:177
/// `MV_NOCHASETARGET` constant.
pub const MV_NOCHASETARGET: i32 = 1 << 5; // c:178

/// Direct port of `bin_ln(char *nam, char **args, Options ops, int func)` from `Src/Modules/files.c:200`.
/// C body (c:209-296):
///   - func == BIN_MV → movefn = rename, MV_ASKNW unless -f, MV_ATOMIC
///   - else → MV_FORCE if -f; -h/-n adds MV_NOCHASETARGET; -s →
///     symlink; otherwise link with MV_NODIRS unless -d
///   - -i without -f → MV_INTERACTIVE
///   - last-arg-is-dir handling: chase into the dir for each src
/// WARNING: param names don't match C — Rust=(nam, args, func) vs C=(nam, args, ops, func)
pub fn bin_ln(
    nam: &str,
    args: &[String], // c:200
    ops: &options,
    func: i32,
) -> i32 {
    let movefn: MoveFunc;
    let mut flags: i32;
    let mut err = 0i32;
    if func == BIN_MV {
        // c:209
        movefn = mv_rename; // c:210
        flags = if OPT_ISSET(ops, b'f') { 0 } else { MV_ASKNW }; // c:211
        flags |= MV_ATOMIC; // c:212
    } else {
        flags = if OPT_ISSET(ops, b'f') { MV_FORCE } else { 0 }; // c:215
        if OPT_ISSET(ops, b'h') || OPT_ISSET(ops, b'n') {
            // c:217
            flags |= MV_NOCHASETARGET;
        }
        if OPT_ISSET(ops, b's') {
            // c:219
            movefn = mv_symlink; // c:220
        } else {
            movefn = mv_link; // c:226
            if !OPT_ISSET(ops, b'd') {
                // c:227
                flags |= MV_NODIRS;
            }
        }
    }
    if OPT_ISSET(ops, b'i') && !OPT_ISSET(ops, b'f') {
        // c:230
        flags |= MV_INTERACTIVE;
    }
    if args.is_empty() {
        zwarnnam(nam, "missing file argument");
        return 1;
    }
    let last_idx = args.len() - 1; // c:232 a = args; for(; a[1]; a++)
    let mut have_dir = false;
    if last_idx > 0 {
        // c:233
        let target = &args[last_idx];
        if let Ok(meta) = std::fs::metadata(target) {
            // c:235 stat
            if meta.is_dir() {
                // c:235 S_ISDIR
                have_dir = true;
                if (flags & MV_NOCHASETARGET) != 0 {
                    // c:237
                    if let Ok(lmeta) = std::fs::symlink_metadata(target) {
                        if lmeta.file_type().is_symlink() {
                            // c:237 S_ISLNK
                            // c:245-256 — multi-source symlink-to-dir
                            // resolution: error unless -f and exactly
                            // one source.
                            if last_idx > 1 {
                                // c:245
                                zwarnnam(
                                    nam, // c:247
                                    &format!("{}: not a directory", target),
                                );
                                return 1; // c:248
                            }
                            if (flags & MV_FORCE) != 0 {
                                // c:250
                                let _ = std::fs::remove_file(target); // c:251 unlink
                                have_dir = false; // c:252
                            } else {
                                zwarnnam(
                                    nam, // c:255
                                    &format!("{}: file exists", target),
                                );
                                return 1; // c:256
                            }
                        }
                    }
                }
            }
        }
    }
    if have_dir {
        // c:havedir branch
        // c:276-294 — target is dir, chase into it for each source.
        let dir = args[last_idx].trim_end_matches('/').to_string();
        for src in &args[..last_idx] {
            // c:281
            let basename = match src.rsplit_once('/') {
                // c:283-285 strrchr
                Some((_, n)) => n,
                None => src.as_str(),
            };
            let dest = format!("{}/{}", dir, basename); // c:289 strcat
            err |= domove(nam, movefn, src, &dest, flags); // c:290
        }
        return err; // c:295
    }
    if last_idx > 1 {
        // c:265
        zwarnnam(nam, "last of many arguments must be a directory"); // c:266
        return 1; // c:267
    }
    let (src, dest) = if args.len() < 2 {
        // c:269 !args[1]
        let basename = match args[0].rsplit_once('/') {
            // c:270 strrchr
            Some((_, n)) => n,
            None => args[0].as_str(),
        };
        (args[0].clone(), basename.to_string()) // c:272 args[1] = ptr+1
    } else {
        (args[0].clone(), args[1].clone())
    };
    domove(nam, movefn, &src, &dest, flags) // c:275
}

/// Direct port of `domove(char *nam, MoveFunc movefn, char *p, char *q, int flags)` from `Src/Modules/files.c:298`.
/// C body (c:300-360): if MV_NODIRS, refuse src that is dir; if dest
/// exists, force/interactive/asknw checks; unlink dest if not atomic;
/// then call movefn(src, dest) and report errno on failure.
pub fn domove(nam: &str, movefn: MoveFunc, p: &str, q: &str, flags: i32) -> i32 {
    // c:298
    if (flags & MV_NODIRS) != 0 {
        // c:298
        match std::fs::symlink_metadata(p) {
            // c:308 lstat
            Ok(meta) if meta.is_dir() => {
                // c:308 S_ISDIR
                zwarnnam(nam, &format!("{}: is a directory", p)); // c:310
                return 1; // c:311
            }
            Err(e) => {
                zwarnnam(nam, &format!("{}: {}", p, e)); // c:310
                return 1;
            }
            _ => {}
        }
    }
    if let Ok(qmeta) = std::fs::symlink_metadata(q) {
        // c:315 lstat
        let mut doit = (flags & MV_FORCE) != 0; // c:316
        if qmeta.is_dir() {
            // c:317 S_ISDIR
            zwarnnam(nam, &format!("{}: cannot overwrite directory", q)); // c:319
            return 1; // c:320
        } else if (flags & MV_INTERACTIVE) != 0 {
            // c:322
            eprint!("{}: replace `{}'? ", nam, q); // c:324-326
            if ask() == 0 {
                // c:328
                return 0; // c:329
            }
            doit = true; // c:331
        } else if (flags & MV_ASKNW) != 0                                    // c:333
                && !qmeta.file_type().is_symlink()                           // c:334 !S_ISLNK
                && unsafe {                                                  // c:335 access W_OK
                    let cq = std::ffi::CString::new(q).ok();
                    cq.map(|c| libc::access(c.as_ptr(), libc::W_OK))
                        .unwrap_or(-1) != 0
                }
        {
            let mode = qmeta.permissions().mode() & 0o7777;
            eprint!("{}: replace `{}', overriding mode {:04o}? ", nam, q, mode); // c:337-340
            if ask() == 0 {
                // c:342
                return 0; // c:343
            }
            doit = true; // c:345
        }
        if doit && (flags & MV_ATOMIC) == 0 {
            // c:347
            let _ = std::fs::remove_file(q); // c:348 unlink
        }
    }
    let r = {
        // c:350 movefn(p, q)
        let cp = std::ffi::CString::new(p).unwrap_or_default();
        let cq = std::ffi::CString::new(q).unwrap_or_default();
        movefn(&cp, &cq)
    };
    if r != 0 {
        // c:350
        let osek = std::io::Error::last_os_error();
        let errfile = if osek.raw_os_error() == Some(libc::ENOENT)           // c:352-355
            && std::fs::symlink_metadata(p).is_ok()
        {
            q
        } else {
            p
        };
        // Bug #112 — use C strerror via last_errstr to avoid the
        // " (os error N)" Rust suffix that Display appends.
        zwarnnam(
            nam,
            &format!("`{}': {}", errfile, crate::ported::compat::last_errstr()),
        ); // c:357
        return 1; // c:358
    }
    0 // c:362
}

/// Direct port of `recursivecmd_doone(struct recursivecmd const *reccmd, char *arg, char *rp, struct dirsav *ds, int first)` from `Src/Modules/files.c:450`.
/// C body (c:455-462): lstat the path; if recurse + S_ISDIR → dive
/// via recursivecmd_dorec; else call leaf_func.
/// WARNING: param names don't match C — Rust=(arg, rp, first) vs C=(reccmd, arg, rp, ds, first)
pub fn recursivecmd_doone<P, R, L>(
    // c:450
    reccmd: &recursivecmd<P, R, L>,
    arg: &str,
    rp: &str,
    first: i32,
) -> i32
// c:450
where
    P: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    R: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    L: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
{
    let st = std::fs::symlink_metadata(rp); // c:455 lstat
    if reccmd.opt_recurse != 0 {
        // c:457
        if let Ok(ref meta) = st {
            if meta.is_dir() {
                // c:458 S_ISDIR
                return recursivecmd_dorec(reccmd, arg, rp, meta, first); // c:465
            }
        }
    }
    let sp = st.as_ref().ok(); // c:465 sp
    (reccmd.leaf_func)(arg, rp, sp) // c:465
}

/// Direct port of `recursivecmd_dorec(struct recursivecmd const *reccmd, char *arg, char *rp, struct stat const *sp, struct dirsav *ds, int first)` from `Src/Modules/files.c:465`.
/// C body (c:475-525): dirpre callback, opendir + readdir each entry,
/// recurse via recursivecmd_doone, then dirpost callback.
/// WARNING: param names don't match C — Rust=(arg, rp, sp, _first) vs C=(reccmd, arg, rp, sp, ds, first)
pub fn recursivecmd_dorec<P, R, L>(
    // c:465
    reccmd: &recursivecmd<P, R, L>,
    arg: &str,
    rp: &str,
    sp: &std::fs::Metadata,
    _first: i32,
) -> i32
// c:465
where
    P: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    R: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
    L: Fn(&str, &str, Option<&std::fs::Metadata>) -> i32,
{
    let err1 = (reccmd.dirpre_func)(arg, rp, Some(sp)); // c:475 dirpre_func
    if (err1 & 2) != 0 {
        return 2;
    } // c:476
    let dir = match std::fs::read_dir(rp) {
        // c:489 opendir
        Ok(d) => d,
        Err(e) => {
            if reccmd.opt_noerr == 0 {
                // c:491
                zwarnnam(reccmd.nam, &format!("{}: {}", arg, e)); // c:492
            }
            return err1 | 1; // c:493
        }
    };
    let mut err = err1;
    for entry in dir.flatten() {
        // c:497 readdir
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "." || name_str == ".." {
            continue;
        } // c:498
        let narg = format!("{}/{}", arg.trim_end_matches('/'), name_str); // c:507-510
        let nrp = entry.path();
        let nrp_str = nrp.to_string_lossy();
        if (err & 2) != 0 {
            break;
        } // c:503
        err |= recursivecmd_doone(reccmd, &narg, &nrp_str, 0); // c:530
    }
    if (err & 2) != 0 {
        return 2;
    } // c:530
    err | (reccmd.dirpost_func)(arg, rp, Some(sp)) // c:530
}

/// Direct port of `recurse_donothing(UNUSED(char *arg), UNUSED(char *rp), UNUSED(struct stat const *sp), UNUSED(void *magic))` from `Src/Modules/files.c:530`.
/// C body: `return 0;`.
/// WARNING: param names don't match C — Rust=(_arg, _rp) vs C=(arg, rp, sp, magic)
pub fn recurse_donothing(
    _arg: &str,
    _rp: &str, // c:530
    _sp: Option<&std::fs::Metadata>,
) -> i32 {
    0 // c:533
}

// =====================================================================
// bin_rm — `Src/Modules/files.c:537-630`.
// =====================================================================

/// Port of `struct rmmagic` from `Src/Modules/files.c:537`.
pub struct rmmagic<'a> {
    pub nam: &'a str,       // c:546
    pub opt_force: i32,     // c:546
    pub opt_interact: i32,  // c:546
    pub opt_unlinkdir: i32, // c:546
}

/// Direct port of `rm_leaf(char *arg, char *rp, struct stat const *sp, void *magic)` from `Src/Modules/files.c:546`.
/// C body (c:551-589):
///   - if !opt_unlinkdir || !opt_force: lstat (if not provided);
///     refuse directories; ask if interactive; warn if read-only
///   - unlink(rp); error path returns 1 unless -f
/// WARNING: param names don't match C — Rust=(arg, rp, sp) vs C=(arg, rp, sp, magic)
pub fn rm_leaf(
    arg: &str,
    rp: &str,
    sp: Option<&std::fs::Metadata>, // c:546
    rmm: &rmmagic,
) -> i32 {
    if rmm.opt_unlinkdir == 0 || rmm.opt_force == 0 {
        // c:551
        let owned;
        let sp_use = if let Some(s) = sp {
            Some(s)
        } else {
            // c:552-554
            owned = std::fs::symlink_metadata(rp).ok(); // c:553 lstat
            owned.as_ref()
        };
        if let Some(meta) = sp_use {
            // c:556
            if rmm.opt_unlinkdir == 0 && meta.is_dir() {
                // c:557 S_ISDIR
                if rmm.opt_force != 0 {
                    return 0;
                } // c:558-559
                zwarnnam(
                    rmm.nam, // c:560
                    &format!("{}: is a directory", arg),
                );
                return 1; // c:561
            }
            if rmm.opt_interact != 0 {
                // c:563
                eprint!("{}: remove `{}'? ", rmm.nam, arg); // c:564-568
                if ask() == 0 {
                    return 0;
                } // c:570
            } else if rmm.opt_force == 0                                     // c:571
                    && !meta.file_type().is_symlink()                        // c:572 !S_ISLNK
                    && unsafe {                                              // c:573 access W_OK
                        let crp = std::ffi::CString::new(rp).ok();
                        crp.map(|c| libc::access(c.as_ptr(), libc::W_OK))
                            .unwrap_or(-1) != 0
                    }
            {
                let mode = meta.permissions().mode() & 0o7777;
                eprint!(
                    "{}: remove `{}', overriding mode {:04o}? ",
                    rmm.nam, arg, mode
                ); // c:574-579
                if ask() == 0 {
                    return 0;
                } // c:581
            }
        }
    }
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    if unsafe { libc::unlink(crp.as_ptr()) } != 0 && rmm.opt_force == 0 {
        // c:594
        zwarnnam(
            rmm.nam, // c:594
            &format!("{}: {}", arg, crate::ported::compat::last_errstr()),
        );
        return 1; // c:594
    }
    0 // c:594
}

/// Direct port of `rm_dirpost(char *arg, char *rp, UNUSED(struct stat const *sp), void *magic)` from `Src/Modules/files.c:594`.
/// C body (c:599-613): rmdir(rp); error path returns 1 unless -f.
/// WARNING: param names don't match C — Rust=(arg, rp, _sp) vs C=(arg, rp, sp, magic)
pub fn rm_dirpost(
    arg: &str,
    rp: &str,
    _sp: Option<&std::fs::Metadata>, // c:594
    rmm: &rmmagic,
) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    if unsafe { libc::rmdir(crp.as_ptr()) } != 0 && rmm.opt_force == 0 {
        // c:608
        zwarnnam(
            rmm.nam, // c:616
            &format!("{}: {}", arg, crate::ported::compat::last_errstr()),
        );
        return 1; // c:616
    }
    0 // c:616
}

/// Direct port of `bin_rm(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/files.c:616`.
/// C body (c:621-633): build rmmagic; recursivecmd with rm_dirpost
/// + rm_leaf; -f swallows the err code.
// rm builtin                                                               // c:535
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_rm(
    nam: &str,
    args: &[String], // c:616
    ops: &options,
    _func: i32,
) -> i32 {
    // C `BUILTIN("rm", 0, bin_rm, 1, -1, ...)` (files.c:814).
    // See bin_mkdir for why the minargs self-check sits here.
    if args.is_empty() {
        zwarnnam(nam, "not enough arguments"); // c:builtin.c:434
        return 1;
    }
    let rmm = rmmagic {
        nam,                                                 // c:621
        opt_force: if OPT_ISSET(ops, b'f') { 1 } else { 0 }, // c:622
        opt_interact: if OPT_ISSET(ops, b'i') && !OPT_ISSET(ops, b'f')
        // c:623
        {
            1
        } else {
            0
        },
        opt_unlinkdir: if OPT_ISSET(ops, b'd') { 1 } else { 0 }, // c:624
    };
    let recurse = if !OPT_ISSET(ops, b'd')                                   // c:626
        && (OPT_ISSET(ops, b'R') || OPT_ISSET(ops, b'r'))
    {
        1
    } else {
        0
    };
    let safe = if OPT_ISSET(ops, b's') { 1 } else { 0 }; // c:627
    let err = recursivecmd(
        nam,
        rmm.opt_force,
        recurse,
        safe,
        args,                                // c:625
        |_a, _r, _s| 0,                      // dirpre = recurse_donothing
        |a, r, s| rm_dirpost(a, r, s, &rmm), // dirpost
        |a, r, s| rm_leaf(a, r, s, &rmm),
    ); // leaf
    if rmm.opt_force != 0 {
        0
    } else {
        err
    } // c:631
}

// =====================================================================
// bin_chmod — `Src/Modules/files.c:642`.
// =====================================================================

/// Port of `struct chmodmagic` from `Src/Modules/files.c:642`.
pub struct chmodmagic<'a> {
    pub nam: &'a str, // c:642
    pub mode: u32,    // c:642
}

/// Direct port of `chmod_dochmod(char *arg, char *rp, UNUSED(struct stat const *sp), void *magic)` from `Src/Modules/files.c:642`.
/// C body (c:646-652): `chmod(rp, mode)`; warn + return 1 on failure.
/// WARNING: param names don't match C — Rust=(arg, rp, _sp) vs C=(arg, rp, sp, magic)
pub fn chmod_dochmod(
    arg: &str,
    rp: &str,
    _sp: Option<&std::fs::Metadata>, // c:642
    chm: &chmodmagic,
) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    if unsafe { libc::chmod(crp.as_ptr(), chm.mode as libc::mode_t) } != 0 {
        // c:646
        zwarnnam(
            chm.nam, // c:655
            &format!("{}: {}", arg, crate::ported::compat::last_errstr()),
        );
        return 1; // c:655
    }
    0 // c:655
}

/// Direct port of `bin_chmod(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/files.c:655`.
/// C body (c:659-672): parse `args[0]` as octal mode; recursivecmd
/// over `args[1..]` applying chmod_dochmod.
// chmod builtin                                                            // c:633
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_chmod(
    nam: &str,
    args: &[String], // c:655
    ops: &options,
    _func: i32,
) -> i32 {
    if args.is_empty() {
        zwarnnam(nam, "missing mode");
        return 1;
    }
    let mode = match i64::from_str_radix(&args[0], 8) {
        // c:663 zstrtol base 8
        Ok(m) => m as u32,
        Err(_) => {
            zwarnnam(nam, &format!("invalid mode `{}'", args[0])); // c:665
            return 1; // c:666
        }
    };
    let chm = chmodmagic { nam, mode };
    let recurse = if OPT_ISSET(ops, b'R') { 1 } else { 0 }; // c:670
    let safe = if OPT_ISSET(ops, b's') { 1 } else { 0 }; // c:670
    recursivecmd(
        nam,
        0,
        recurse,
        safe,
        &args[1..],                             // c:669
        |a, r, s| chmod_dochmod(a, r, s, &chm), // dirpre
        |_a, _r, _s| 0,                         // dirpost = recurse_donothing
        |a, r, s| chmod_dochmod(a, r, s, &chm),
    ) // leaf
}

// =====================================================================
// bin_chown — `Src/Modules/files.c:674-801`.
// =====================================================================

/// Port of `struct chownmagic` from `Src/Modules/files.c:682`.
pub struct chownmagic<'a> {
    pub nam: &'a str, // c:682
    pub uid: i64,     // c:682 (uid_t but -1 sentinel)
    pub gid: i64,     // c:682
}

/// Direct port of `chown_dochown(char *arg, char *rp, UNUSED(struct stat const *sp), void *magic)` from `Src/Modules/files.c:682`.
/// C body (c:686-692): `chown(rp, uid, gid)`; warn + return 1 on failure.
/// WARNING: param names don't match C — Rust=(arg, rp, _sp) vs C=(arg, rp, sp, magic)
pub fn chown_dochown(
    arg: &str,
    rp: &str,
    _sp: Option<&std::fs::Metadata>, // c:682
    chm: &chownmagic,
) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let uid = if chm.uid < 0 {
        libc::uid_t::MAX
    } else {
        chm.uid as libc::uid_t
    };
    let gid = if chm.gid < 0 {
        libc::gid_t::MAX
    } else {
        chm.gid as libc::gid_t
    };
    if unsafe { libc::chown(crp.as_ptr(), uid, gid) } != 0 {
        // c:695
        zwarnnam(
            chm.nam, // c:695
            &format!("{}: {}", arg, crate::ported::compat::last_errstr()),
        );
        return 1; // c:695
    }
    0 // c:695
}

/// Direct port of `chown_dolchown(char *arg, char *rp, UNUSED(struct stat const *sp), void *magic)` from `Src/Modules/files.c:695`.
/// C body (c:699-705): `lchown(rp, uid, gid)`.
/// WARNING: param names don't match C — Rust=(arg, rp, _sp) vs C=(arg, rp, sp, magic)
pub fn chown_dolchown(
    arg: &str,
    rp: &str,
    _sp: Option<&std::fs::Metadata>, // c:695
    chm: &chownmagic,
) -> i32 {
    let crp = match std::ffi::CString::new(rp) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let uid = if chm.uid < 0 {
        libc::uid_t::MAX
    } else {
        chm.uid as libc::uid_t
    };
    let gid = if chm.gid < 0 {
        libc::gid_t::MAX
    } else {
        chm.gid as libc::gid_t
    };
    if unsafe { libc::lchown(crp.as_ptr(), uid, gid) } != 0 {
        // c:708
        zwarnnam(
            chm.nam, // c:708
            &format!("{}: {}", arg, crate::ported::compat::last_errstr()),
        );
        return 1; // c:708
    }
    0 // c:708
}

/// Direct port of `bin_chown(char *nam, char **args, Options ops, int func)` from `Src/Modules/files.c:725`.
/// C body (c:729-797): parse `user[:group]` spec; for chgrp, skip the
/// user half; getpwnam / getnumeric / getgrnam fallbacks; recursivecmd
/// with chown_dochown or chown_dolchown for `-h`.
// chown builtin                                                            // c:672
/// WARNING: param names don't match C — Rust=(nam, args, func) vs C=(nam, args, ops, func)
pub fn bin_chown(
    nam: &str,
    args: &[String], // c:725
    ops: &options,
    func: i32,
) -> i32 {
    if args.is_empty() {
        zwarnnam(nam, "missing argument");
        return 1;
    }
    let uspec = args[0].clone(); // c:728
    let mut chm = chownmagic {
        nam,
        uid: -1,
        gid: -1,
    };
    let mut p_idx = 0usize;
    let mut do_group_only = false;
    if func == BIN_CHGRP {
        // c:733
        chm.uid = -1; // c:734
        do_group_only = true; // c:735 goto dogroup
    } else {
        // c:737-741 — locate `:` or `.` separator.
        let end = uspec.find(':').or_else(|| uspec.find('.')); // c:737-738
        if end == Some(0) {
            // c:739
            chm.uid = -1; // c:740
            p_idx = 1; // c:741
            do_group_only = true; // c:742 goto dogroup
        } else {
            let user_part = if let Some(e) = end {
                &uspec[..e]
            } else {
                &uspec[..]
            };
            // c:746 — getpwnam(p)
            let cuser = std::ffi::CString::new(user_part).unwrap_or_default();
            let pwd = unsafe { libc::getpwnam(cuser.as_ptr()) }; // c:746
            let uid = if !pwd.is_null() {
                unsafe { (*pwd).pw_uid as i64 } // c:748
            } else {
                let mut errp = 0i32;
                let n = getnumeric(user_part, &mut errp); // c:751
                if errp != 0 {
                    // c:752
                    zwarnnam(
                        nam, // c:753
                        &format!("{}: no such user", user_part),
                    );
                    return 1; // c:755
                }
                n as i64
            };
            chm.uid = uid;
            if let Some(e) = end {
                // c:759
                let group_part = &uspec[e + 1..];
                if group_part.is_empty() {
                    // c:761
                    let p2 = if !pwd.is_null() {
                        pwd
                    }
                    // c:762
                    else {
                        unsafe { libc::getpwuid(uid as libc::uid_t) }
                    };
                    if p2.is_null() {
                        // c:763
                        zwarnnam(
                            nam, // c:764
                            &format!("{}: no such user", uspec),
                        );
                        return 1; // c:766
                    }
                    chm.gid = unsafe { (*p2).pw_gid as i64 }; // c:768
                } else if group_part == ":" {
                    // c:769
                    chm.gid = -1; // c:770
                } else {
                    p_idx = 0; // not used past this point
                    let cgrp = std::ffi::CString::new(group_part).unwrap_or_default();
                    let grp = unsafe { libc::getgrnam(cgrp.as_ptr()) }; // c:773 dogroup
                    if !grp.is_null() {
                        chm.gid = unsafe { (*grp).gr_gid as i64 }; // c:775
                    } else {
                        let mut errp = 0i32;
                        let n = getnumeric(group_part, &mut errp); // c:778
                        if errp != 0 {
                            // c:779
                            zwarnnam(
                                nam, // c:780
                                &format!("{}: no such group", group_part),
                            );
                            return 1; // c:782
                        }
                        chm.gid = n as i64;
                    }
                }
            }
        }
    }
    if do_group_only {
        // c:773 dogroup label
        let group_part = &uspec[p_idx..];
        let cgrp = std::ffi::CString::new(group_part).unwrap_or_default();
        let grp = unsafe { libc::getgrnam(cgrp.as_ptr()) }; // c:773
        if !grp.is_null() {
            chm.gid = unsafe { (*grp).gr_gid as i64 }; // c:775
        } else {
            let mut errp = 0i32;
            let n = getnumeric(group_part, &mut errp); // c:778
            if errp != 0 {
                // c:779
                zwarnnam(
                    nam, // c:780
                    &format!("{}: no such group", group_part),
                );
                return 1; // c:782
            }
            chm.gid = n as i64;
        }
    }
    let recurse = if OPT_ISSET(ops, b'R') { 1 } else { 0 }; // c:792
    let safe = if OPT_ISSET(ops, b's') { 1 } else { 0 }; // c:792
    let h_flag = OPT_ISSET(ops, b'h'); // c:793
    recursivecmd(
        nam,
        0,
        recurse,
        safe,
        &args[1..], // c:791
        |a, r, s| {
            if h_flag {
                chown_dolchown(a, r, s, &chm)
            } else {
                chown_dochown(a, r, s, &chm)
            }
        }, // dirpre
        |_a, _r, _s| 0, // dirpost = recurse_donothing
        |a, r, s| {
            if h_flag {
                chown_dolchown(a, r, s, &chm)
            } else {
                chown_dochown(a, r, s, &chm)
            }
        },
    ) // leaf
}

// =====================================================================
// `LN_OPTS` — `Src/Modules/files.c:799`.
// =====================================================================

/// `LN_OPTS` — `Src/Modules/files.c:799` (`"dfhins"` when HAVE_LSTAT,
/// `"dfi"` otherwise). zshrs targets POSIX hosts where lstat is
/// always present, so the long form is canonical.
pub const LN_OPTS: &str = "dfhins"; // c:799

// `module_bintab` — port of `static struct builtin bintab[]` (files.c:803)
// in the canonical `module::Builtin` shape (the local `FilesBuiltin`
// table above is kept for the internal dispatcher in
// `src/extensions/` which needs the `funcid` int).

// `module_features` — port of `static struct features module_features`
// from files.c:828.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/files.c:838`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:838
    // C body c:840-841 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/files.c:845`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:845
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/files.c:853`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:853
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/files.c:860`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:860
    // C body c:862-863 — `return 0`. Faithful empty-body port; the
    //                    chmod/chown/chgrp/sync/etc. builtins register
    //                    via the bn_list feature dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/files.c:867`.
pub fn cleanup_(m: *const module) -> i32 {
    // c:867
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/files.c:874`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:874
    // C body c:876-877 — `return 0`. Faithful empty-body port; the
    //                    builtins unregister via cleanup_'s setfeatureenables.
    0
}

/// `bin_chown` func discriminant — `Src/Modules/files.c:41`
/// (`enum { BIN_CHOWN, BIN_CHGRP };`).
pub const BIN_CHOWN: i32 = 0; // c:719

// bintab[] (`static struct builtin bintab[]` at files.c:803) lives in
// the `MODULE_BINTAB` slice below in canonical `module::Builtin` shape;
// the dispatcher in src/extensions/ reads it through the singleton
// rather than a private aggregate type.

// =====================================================================
// module entries — `Src/Modules/files.c:828-876`.
// =====================================================================
/// `BIN_CHGRP` constant.
pub const BIN_CHGRP: i32 = 1; // c:719

// =====================================================================
// bin_ln + domove — `Src/Modules/files.c:200`, `:298`.
// =====================================================================

// `enum MoveFunc` deleted — C uses a bare function-pointer typedef
// `typedef int (*MoveFunc)(char const *, char const *);` at
// `Src/Modules/files.c:32`. Rust ports it directly as the same
// function-pointer type alias below, so call sites can do
// `movefn = mv_rename;` matching C's `movefn = rename;`.
/// `MoveFunc` type alias.
#[allow(non_camel_case_types)]
pub type MoveFunc = fn(p: &std::ffi::CStr, q: &std::ffi::CStr) -> i32;

/// Adapter for `rename(2)` — used by `bin_mv`.
pub fn mv_rename(p: &std::ffi::CStr, q: &std::ffi::CStr) -> i32 {
    unsafe { libc::rename(p.as_ptr(), q.as_ptr()) }
}

/// Adapter for `symlink(2)` — used by `bin_ln -s`.
pub fn mv_symlink(p: &std::ffi::CStr, q: &std::ffi::CStr) -> i32 {
    unsafe { libc::symlink(p.as_ptr(), q.as_ptr()) }
}

/// Adapter for `link(2)` — used by `bin_ln` default.
pub fn mv_link(p: &std::ffi::CStr, q: &std::ffi::CStr) -> i32 {
    unsafe { libc::link(p.as_ptr(), q.as_ptr()) }
}

/// Direct port of `getnumeric(char *p, int *errp)` from `Src/Modules/files.c:708`.
/// C body (c:712-719): parse leading digits as base-10 unsigned long;
/// `*errp = !!*p` after parse — set when there are trailing non-digits.
pub fn getnumeric(p: &str, errp: &mut i32) -> u64 {
    // c:708
    if !p.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        // c:708
        *errp = 1; // c:713
        return 0; // c:714
    }
    let end = p.find(|c: char| !c.is_ascii_digit()).unwrap_or(p.len());
    let ret = p[..end].parse::<u64>().unwrap_or(0); // c:725 strtoul
    *errp = if end < p.len() { 1 } else { 0 }; // c:725
    ret // c:725
}

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN FILES.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec![
        "b:chgrp".to_string(),
        "b:chmod".to_string(),
        "b:chown".to_string(),
        "b:ln".to_string(),
        "b:mkdir".to_string(),
        "b:mv".to_string(),
        "b:rm".to_string(),
        "b:rmdir".to_string(),
        "b:sync".to_string(),
        "b:zf_chgrp".to_string(),
        "b:zf_chmod".to_string(),
        "b:zf_chown".to_string(),
        "b:zf_ln".to_string(),
        "b:zf_mkdir".to_string(),
        "b:zf_mv".to_string(),
        "b:zf_rm".to_string(),
        "b:zf_rmdir".to_string(),
        "b:zf_sync".to_string(),
    ]
}

// WARNING: NOT IN FILES.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 18]);
    }
    0
}

// WARNING: NOT IN FILES.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN FILES.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
            bn_list: None,
            bn_size: 18,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ops() -> options {
        options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// c:115 — `domkdir` against a path that already exists must
    /// surface mkdir(2)'s EEXIST. Silent success would mask
    /// existing-directory clobbering — a real safety regression.
    #[test]
    fn domkdir_fails_when_path_already_exists() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir();
        assert_ne!(domkdir("mkdir", tmp.to_str().unwrap(), 0o755, 0), 0);
    }

    /// c:75-76 — `mkdir -m <invalid-octal>` early-returns 1 BEFORE
    /// the per-arg mkdir loop. Catches a regression where the bad
    /// mode silently falls through to the umask-derived default.
    #[test]
    fn mkdir_with_invalid_mode_short_circuits_before_filesystem_op() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'm' as usize] = (1 << 2) | 1;
        ops.args.push("not-octal".to_string());
        // Path is bogus on purpose: if the mode-parse early-return
        // failed, mkdir(2) would still try this path and fail
        // differently. The assertion catches the right error origin.
        let r = bin_mkdir(
            "mkdir",
            &["/tmp/zshrs_test_invalid_mode".to_string()],
            &ops,
            0,
        );
        assert_eq!(r, 1);
    }

    /// c:115-150 — `domkdir` with `p=1` against a path that already
    /// exists AS A DIRECTORY must return 0 (success). This is the
    /// `mkdir -p` idempotence rule: re-running `mkdir -p /existing`
    /// must NOT error.
    #[test]
    fn domkdir_with_p_flag_on_existing_dir_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir();
        let r = domkdir("mkdir", tmp.to_str().unwrap(), 0o755, /*p=*/ 1);
        assert_eq!(r, 0, "mkdir -p /tmp must succeed when /tmp already exists");
    }

    /// c:115-150 — same path with `p=0` (`mkdir` without -p) must
    /// FAIL when the path already exists. Symmetrical to the above:
    /// behavior pivots entirely on the `p` flag.
    #[test]
    fn domkdir_without_p_on_existing_dir_fails() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir();
        let r = domkdir("mkdir", tmp.to_str().unwrap(), 0o755, /*p=*/ 0);
        assert_ne!(r, 0, "mkdir (no -p) on existing dir must fail per POSIX");
    }

    /// c:80-115 + c:builtin.c:430-435 — `bin_mkdir` with no args.
    /// The C body itself returns 0 (per-arg loop has no iterations),
    /// but the BUILTIN dispatch table declares `minargs=1`
    /// (Src/Modules/files.c:810), so the framework rejects empty
    /// argv with "not enough arguments" → exit 1 before the body
    /// runs. zshrs's bin_mkdir self-enforces the same minargs check
    /// for direct-call paths (test / fusevm bridge) so the
    /// user-visible `mkdir; echo $?` parity holds.
    #[test]
    fn bin_mkdir_with_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_mkdir("mkdir", &[], &ops, 0);
        assert_ne!(r, 0, "bin_mkdir on empty argv → minargs error");
    }

    /// c:80-115 — `bin_mkdir -p` should successfully create a deeply
    /// nested path. Catches a regression where the nested-walk logic
    /// (c:88-101) leaves a parent half-created and reports failure.
    #[test]
    fn bin_mkdir_p_creates_nested_path() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'p' as usize] = (1 << 2) | 1;
        // Unique under /tmp to keep tests independent of one another
        let pid = std::process::id();
        let base = format!("/tmp/zshrs_test_mkdir_p_{}", pid);
        let nested = format!("{}/a/b/c", base);
        let _ = std::fs::remove_dir_all(&base);
        let r = bin_mkdir("mkdir", &[nested.clone()], &ops, 0);
        assert_eq!(r, 0, "mkdir -p should create the whole chain");
        assert!(
            std::path::Path::new(&nested).is_dir(),
            "leaf dir {} should exist after mkdir -p",
            nested
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// c:81-83 — bin_mkdir's trailing-slash trim. `bin_mkdir /tmp/foo/`
    /// must call domkdir with the trailing slash stripped. Verified
    /// through the happy-path: the directory ends up at the trimmed
    /// path, even though the user supplied the trailing slash.
    #[test]
    fn bin_mkdir_strips_trailing_slashes() {
        let _g = crate::test_util::global_state_lock();
        let pid = std::process::id();
        let target = format!("/tmp/zshrs_test_mkdir_trailing_{}/", pid);
        let _ = std::fs::remove_dir_all(target.trim_end_matches('/'));
        let ops = empty_ops();
        let r = bin_mkdir("mkdir", &[target.clone()], &ops, 0);
        assert_eq!(r, 0, "trailing slash should not break mkdir");
        assert!(std::path::Path::new(target.trim_end_matches('/')).is_dir());
        let _ = std::fs::remove_dir_all(target.trim_end_matches('/'));
    }

    /// c:150-200 — `bin_rmdir` on a path that does not exist returns
    /// non-zero, matching rmdir(2)'s ENOENT contract. Regression
    /// suppressing the per-arg error accumulator would mask this.
    #[test]
    fn bin_rmdir_on_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rmdir(
            "rmdir",
            &["/__definitely_not_a_dir_xyzzy_zshrs_test__".to_string()],
            &ops,
            0,
        );
        assert_ne!(r, 0, "rmdir of nonexistent path must report failure");
    }

    /// c:150-200 + c:builtin.c:430-435 — same framework minargs
    /// story as bin_mkdir above: BUILTIN table sets `minargs=1`,
    /// dispatch rejects empty argv → exit 1. zshrs self-enforces.
    #[test]
    fn bin_rmdir_with_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rmdir("rmdir", &[], &ops, 0);
        assert_ne!(r, 0, "rmdir empty argv → minargs error");
    }

    /// c:150-200 — `bin_rmdir` happy path: create then remove. Proves
    /// the rmdir(2) call is actually wired through.
    #[test]
    fn bin_rmdir_removes_an_empty_directory() {
        let _g = crate::test_util::global_state_lock();
        let pid = std::process::id();
        let target = format!("/tmp/zshrs_test_rmdir_{}", pid);
        let _ = std::fs::remove_dir_all(&target);
        std::fs::create_dir(&target).expect("setup mkdir");
        let ops = empty_ops();
        let r = bin_rmdir("rmdir", &[target.clone()], &ops, 0);
        assert_eq!(r, 0, "rmdir on existing empty dir should succeed");
        assert!(
            !std::path::Path::new(&target).exists(),
            "directory should be gone after rmdir"
        );
    }

    /// c:838-879 — `setup_` / `boot_` / `cleanup_` / `finish_` module
    /// lifecycle stubs. C versions are empty (`return 0`). The Rust
    /// port must match.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let null_module = std::ptr::null();
        assert_eq!(setup_(null_module), 0);
        assert_eq!(boot_(null_module), 0);
        assert_eq!(cleanup_(null_module), 0);
        assert_eq!(finish_(null_module), 0);
    }

    /// c:616 + c:builtin.c:430-435 — same framework minargs story
    /// as bin_mkdir above. zshrs self-enforces.
    #[test]
    fn bin_rm_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rm("rm", &[], &ops, 0);
        assert_ne!(r, 0, "rm empty argv → minargs error");
    }

    /// c:616 — `bin_rm` on a nonexistent path returns nonzero.
    #[test]
    fn bin_rm_nonexistent_path_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rm("rm", &["/__zshrs_test_no_such_path__".to_string()], &ops, 0);
        assert_ne!(r, 0, "rm on nonexistent path must error");
    }

    /// c:546 — `rm_leaf` on a real file removes it. Uses rmmagic
    /// struct (c:475-480) matching the port signature.
    #[test]
    fn rm_leaf_removes_existing_file() {
        let _g = crate::test_util::global_state_lock();
        let pid = std::process::id();
        let f = format!("/tmp/zshrs_test_rm_leaf_{}.txt", pid);
        let _ = std::fs::write(&f, "x");
        assert!(std::path::Path::new(&f).exists());
        let rmm = rmmagic {
            nam: "rm",
            opt_force: 1,
            opt_interact: 0,
            opt_unlinkdir: 0,
        };
        let r = rm_leaf(&f, &f, None, &rmm);
        assert_eq!(r, 0, "rm_leaf on existing file must succeed");
        assert!(
            !std::path::Path::new(&f).exists(),
            "file must be gone after rm_leaf"
        );
    }

    /// c:546 — `rm_leaf` on a nonexistent path with opt_force=1
    /// returns 0 silently (matches POSIX `rm -f`). Pin so a regen
    /// that ignores force flag silently errors on `rm -f /missing`.
    #[test]
    fn rm_leaf_force_silently_succeeds_on_missing() {
        let _g = crate::test_util::global_state_lock();
        let rmm = rmmagic {
            nam: "rm",
            opt_force: 1,
            opt_interact: 0,
            opt_unlinkdir: 0,
        };
        let r = rm_leaf("/__never__", "/__never__", None, &rmm);
        assert_eq!(r, 0, "rm -f on missing path must succeed silently");
    }

    /// c:655 — `bin_chmod` with no args returns 1 (the port has a
    /// "missing mode" arity guard at line 607). Pin the explicit
    /// error so a regen removing the guard silently mis-routes.
    #[test]
    fn bin_chmod_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_chmod("chmod", &[], &ops, 0);
        assert_eq!(r, 1, "missing mode must surface as error");
    }

    /// c:725 — `bin_chown` with no args returns 1 (port arity guard).
    #[test]
    fn bin_chown_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_chown("chown", &[], &ops, 0);
        assert_eq!(r, 1, "missing owner must surface as error");
    }

    /// c:200 — `bin_ln` with FEWER than 2 args is a usage error.
    #[test]
    fn bin_ln_one_arg_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_ln("ln", &["/tmp".to_string()], &ops, 0);
        assert_ne!(r, 0, "ln with <2 args must error");
    }

    /// c:53 — `bin_sync` ignores all args, returns 0 (fire-and-
    /// forget sync(2)).
    #[test]
    fn bin_sync_returns_zero_regardless_of_args() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        assert_eq!(bin_sync("sync", &[], &ops, 0), 0);
        assert_eq!(bin_sync("sync", &["ignored".to_string()], &ops, 0), 0);
        assert_eq!(
            bin_sync("sync", &["a".to_string(), "b".to_string()], &ops, 0),
            0
        );
    }

    // ─── zsh-corpus pins for bin_mkdir / bin_rmdir / bin_rm ─────────

    /// `bin_mkdir` with no args returns non-zero (usage error).
    #[test]
    fn files_corpus_bin_mkdir_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_mkdir("mkdir", &[], &ops, 0);
        assert_ne!(r, 0, "mkdir with no args = usage error");
    }

    /// `bin_rmdir` with no args returns non-zero.
    #[test]
    fn files_corpus_bin_rmdir_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rmdir("rmdir", &[], &ops, 0);
        assert_ne!(r, 0, "rmdir with no args = usage error");
    }

    /// `bin_rm` with no args returns non-zero (usage error).
    #[test]
    fn files_corpus_bin_rm_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rm("rm", &[], &ops, 0);
        assert_ne!(r, 0, "rm with no args = usage error");
    }

    /// `bin_mkdir` creates a new directory successfully.
    #[test]
    fn files_corpus_bin_mkdir_creates_directory() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("newdir");
        let target_str = target.to_str().unwrap().to_string();
        let r = bin_mkdir("mkdir", &[target_str], &ops, 0);
        assert_eq!(r, 0, "mkdir on new path succeeds");
        assert!(target.exists(), "new dir actually created");
        assert!(target.is_dir(), "new path is a directory");
    }

    /// `bin_rmdir` removes an empty dir.
    #[test]
    fn files_corpus_bin_rmdir_removes_empty_dir() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("toremove");
        std::fs::create_dir(&target).unwrap();
        assert!(target.exists());
        let r = bin_rmdir("rmdir", &[target.to_str().unwrap().to_string()], &ops, 0);
        assert_eq!(r, 0, "rmdir on empty dir succeeds");
        assert!(!target.exists(), "dir removed");
    }

    /// `domkdir` creates a dir with the given mode.
    #[test]
    fn files_corpus_domkdir_creates_with_mode() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("modedir");
        let r = domkdir("mkdir", target.to_str().unwrap(), 0o755, 0);
        assert_eq!(r, 0);
        assert!(target.is_dir());
    }

    /// `domkdir` fails if dir already exists (without -p flag).
    #[test]
    fn files_corpus_domkdir_existing_path_fails_without_p() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let r = domkdir("mkdir", dir.path().to_str().unwrap(), 0o755, 0);
        assert_ne!(r, 0, "mkdir on existing dir without -p = error");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Modules/files.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `domkdir` creates a fresh dir and returns 0.
    /// C `Src/Modules/files.c:domkdir` — mkdir() syscall path.
    #[test]
    fn domkdir_fresh_path_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("newdir");
        let r = domkdir("mkdir", path.to_str().unwrap(), 0o755, 0);
        assert_eq!(r, 0, "fresh mkdir returns 0 (success)");
        assert!(path.exists(), "directory should exist after mkdir");
    }

    /// `domkdir -p` creates parents as needed.
    /// C `Src/Modules/files.c:domkdir` with `p=1` flag:
    ///   recursively creates each intermediate directory if missing.
    /// ZSHRS BUG: Rust port at modules/files.rs:222 does NOT walk
    /// intermediate dirs — `mkdir -p a/b/c` fails when `a` and `b`
    /// don't exist. C-compatible `mkdir -p` is mandatory shell
    /// behavior; this breaks any script that relies on it.
    #[test]
    fn domkdir_with_p_creates_parents() {
        let _g = crate::test_util::global_state_lock();
        let parent = tempfile::tempdir().unwrap();
        let nested = parent.path().join("a/b/c");
        let r = domkdir("mkdir", nested.to_str().unwrap(), 0o755, 1);
        assert_eq!(r, 0, "mkdir -p with nested path returns 0");
        assert!(nested.exists(), "nested dir should exist after mkdir -p");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/files.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:708 — `getnumeric("0")` returns 0 with err=0.
    #[test]
    fn getnumeric_zero_returns_zero_no_err() {
        let mut err = 0;
        let r = getnumeric("0", &mut err);
        assert_eq!(r, 0);
        assert_eq!(err, 0);
    }

    /// c:708 — `getnumeric("12345")` returns 12345 with err=0.
    #[test]
    fn getnumeric_canonical_decimal() {
        let mut err = 0;
        let r = getnumeric("12345", &mut err);
        assert_eq!(r, 12345);
        assert_eq!(err, 0);
    }

    /// c:725 — `getnumeric("100abc")` returns 100 but sets err=1
    /// (trailing non-digit).
    #[test]
    fn getnumeric_trailing_garbage_sets_err() {
        let mut err = 0;
        let r = getnumeric("100abc", &mut err);
        assert_eq!(r, 100, "digits consumed");
        assert_eq!(err, 1, "trailing non-digit sets err");
    }

    /// c:713 — `getnumeric("abc")` returns 0 and sets err=1 (no leading digit).
    #[test]
    fn getnumeric_no_leading_digit_sets_err() {
        let mut err = 0;
        let r = getnumeric("abc", &mut err);
        assert_eq!(r, 0);
        assert_eq!(err, 1);
    }

    /// c:708 — `getnumeric("")` empty input: no leading digit → err=1, ret=0.
    #[test]
    fn getnumeric_empty_sets_err() {
        let mut err = 0;
        let r = getnumeric("", &mut err);
        assert_eq!(r, 0);
        assert_eq!(err, 1, "empty input is invalid");
    }

    /// c:713 — getnumeric with negative-sign prefix sets err (no leading digit).
    #[test]
    fn getnumeric_negative_sign_sets_err() {
        let mut err = 0;
        let r = getnumeric("-5", &mut err);
        assert_eq!(r, 0);
        assert_eq!(err, 1, "minus sign isn't a digit");
    }

    /// c:53 — `bin_sync` always returns 0 (no-op success).
    #[test]
    fn bin_sync_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_sync("sync", &[], &ops, 0);
        assert_eq!(r, 0, "sync builtin must return 0");
    }

    /// c:222 — `domkdir` on existing dir returns nonzero (already exists).
    #[test]
    fn domkdir_existing_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        // Attempt mkdir of the already-existing temp dir.
        let r = domkdir("mkdir", dir.path().to_str().unwrap(), 0o755, 0);
        assert_ne!(r, 0, "existing dir → nonzero error");
    }

    /// c:1205 — `mv_rename` on missing source path returns nonzero.
    #[test]
    fn mv_rename_missing_source_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let src = std::ffi::CString::new("/__nonexistent_zshrs_xyz__").unwrap();
        let dst = std::ffi::CString::new("/tmp/zshrs_mv_dst").unwrap();
        let r = mv_rename(&src, &dst);
        assert_ne!(r, 0, "missing source → error");
    }

    /// c:1210 — `mv_symlink` to nonexistent target returns nonzero or
    /// succeeds creating dangling link. Test no panic.
    #[test]
    fn mv_symlink_no_panic_on_dangling() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let dst_path = dir.path().join("zshrs_symlink_test");
        let src = std::ffi::CString::new("/__nonexistent_zshrs_xyz__").unwrap();
        let dst = std::ffi::CString::new(dst_path.to_str().unwrap()).unwrap();
        let _ = mv_symlink(&src, &dst);
    }

    /// Lifecycle stubs return 0.
    #[test]
    fn files_lifecycle_stubs_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
        assert_eq!(boot_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/files.c
    // c:116 bin_sync / c:137 bin_mkdir / c:275 bin_rmdir / c:347 bin_ln /
    // c:757 bin_rm / c:845 bin_chmod / c:963 bin_chown / c:1132+ lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:116 — `bin_sync` returns u8 exit-code range.
    #[test]
    fn bin_sync_returns_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_sync("sync", &[], &ops, 0);
        assert!((0..256).contains(&r), "exit code {} must fit in u8", r);
    }

    /// c:116 — `bin_sync` is idempotent (safe to call many times).
    #[test]
    fn bin_sync_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for _ in 0..5 {
            let r = bin_sync("sync", &[], &ops, 0);
            assert_eq!(r, 0, "sync must return 0");
        }
    }

    /// c:137 — `bin_mkdir` empty path returns nonzero.
    #[test]
    fn bin_mkdir_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_mkdir("mkdir", &["".to_string()], &ops, 0);
        assert_ne!(r, 0, "empty path → error");
    }

    /// c:275 — `bin_rmdir` empty path returns nonzero.
    #[test]
    fn bin_rmdir_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rmdir("rmdir", &["".to_string()], &ops, 0);
        assert_ne!(r, 0, "empty path → error");
    }

    /// c:757 — `bin_rm` empty path returns nonzero.
    #[test]
    fn bin_rm_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rm("rm", &["".to_string()], &ops, 0);
        assert_ne!(r, 0, "empty path → error");
    }

    /// c:347 — `bin_ln` no args returns nonzero (usage error).
    #[test]
    fn bin_ln_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_ln("ln", &[], &ops, 0);
        assert_ne!(r, 0, "no args → error");
    }

    /// c:845 — `bin_chmod` no args returns nonzero.
    #[test]
    fn bin_chmod_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_chmod("chmod", &[], &ops, 0);
        assert_ne!(r, 0, "no args → error");
    }

    /// c:963 — `bin_chown` no args returns nonzero.
    #[test]
    fn bin_chown_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_chown("chown", &[], &ops, 0);
        assert_ne!(r, 0, "no args → error");
    }

    /// c:137-963 — all file-op builtin returns fit in u8 exit-code range.
    #[test]
    fn files_builtins_return_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for r in [
            bin_mkdir("mkdir", &[], &ops, 0),
            bin_rmdir("rmdir", &[], &ops, 0),
            bin_rm("rm", &[], &ops, 0),
            bin_ln("ln", &[], &ops, 0),
            bin_chmod("chmod", &[], &ops, 0),
            bin_chown("chown", &[], &ops, 0),
        ] {
            assert!(
                (0..256).contains(&r),
                "exit code {} must fit in u8 range",
                r
            );
        }
    }

    /// c:1132+ — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn files_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/files.c
    // c:116 bin_sync / c:137 bin_mkdir / c:222 domkdir / c:1222 getnumeric +
    // c:1132-1169 lifecycle type pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:116 — `bin_sync` returns i32 (compile-time type pin).
    #[test]
    fn bin_sync_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_sync("sync", &[], &ops, 0);
    }

    /// c:137 — `bin_mkdir` returns i32 (compile-time type pin).
    #[test]
    fn bin_mkdir_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_mkdir("mkdir", &[], &ops, 0);
    }

    /// c:222 — `domkdir` returns i32 (compile-time type pin).
    #[test]
    fn domkdir_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = domkdir("mkdir", "/__never_zshrs_xyz__", 0o755, 0);
    }

    /// c:222 — `domkdir` for nonexistent parent fails.
    #[test]
    fn domkdir_nonexistent_parent_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = domkdir("mkdir", "/__never_real_parent__/foo", 0o755, 0);
        assert_ne!(r, 0, "nonexistent parent → error");
    }

    /// c:1222 — `getnumeric("0")` returns (0, no-error).
    #[test]
    fn getnumeric_zero_no_error_pin() {
        let mut err = 0i32;
        let v = getnumeric("0", &mut err);
        assert_eq!(v, 0, "0 → 0");
        assert_eq!(err, 0, "no error flag on 0");
    }

    /// c:1222 — `getnumeric` returns u64 (compile-time type pin).
    #[test]
    fn getnumeric_returns_u64_type() {
        let mut err = 0i32;
        let _: u64 = getnumeric("0", &mut err);
    }

    /// c:1222 — `getnumeric` is deterministic for stable input.
    #[test]
    fn getnumeric_is_deterministic() {
        for s in ["0", "42", "100", "garbage", ""] {
            let mut err = 0i32;
            let first = getnumeric(s, &mut err);
            for _ in 0..3 {
                let mut err2 = 0i32;
                let v = getnumeric(s, &mut err2);
                assert_eq!(v, first, "getnumeric({:?}) must be deterministic", s);
            }
        }
    }

    /// c:1132 — `setup_` returns i32 (compile-time type pin).
    #[test]
    fn files_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:1139 — features list non-empty.
    #[test]
    fn files_features_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        assert!(!feats.is_empty(), "files module advertises ≥1 feature");
    }

    /// c:1139 — features use b:/p: prefix per zsh module spec.
    #[test]
    fn files_features_use_canonical_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        for f in &feats {
            assert!(
                f.starts_with("b:") || f.starts_with("p:"),
                "feature {:?} must use b:/p: prefix",
                f
            );
        }
    }

    /// c:116 — `bin_sync` is idempotent (safe to call repeatedly).
    #[test]
    fn bin_sync_idempotent_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for _ in 0..5 {
            let r = bin_sync("sync", &[], &ops, 0);
            assert_eq!(r, 0, "sync always returns 0");
        }
    }

    /// c:1162 + c:1169 — cleanup/finish idempotent.
    #[test]
    fn files_cleanup_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:222 — `domkdir` creates real directory then succeeds.
    #[test]
    fn domkdir_real_path_succeeds() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("new_dir");
        let r = domkdir("mkdir", target.to_str().unwrap(), 0o755, 0);
        assert_eq!(r, 0, "creating new dir under tempdir → 0");
        assert!(target.exists(), "directory was created");
        assert!(target.is_dir(), "result is a directory");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/files.c
    // c:116 bin_sync / c:137 bin_mkdir / c:222 domkdir / c:275 bin_rmdir /
    // c:347 bin_ln / c:757 bin_rm / c:845 bin_chmod / c:963 bin_chown
    // ═══════════════════════════════════════════════════════════════════

    /// c:116 — `bin_sync` returns i32 (compile-time pin, alt).
    #[test]
    fn bin_sync_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_sync("sync", &[], &ops, 0);
    }

    /// c:116 — `bin_sync` is deterministic (always returns 0).
    #[test]
    fn bin_sync_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for _ in 0..10 {
            assert_eq!(bin_sync("sync", &[], &ops, 0), 0, "sync always returns 0");
        }
    }

    /// c:137 — `bin_mkdir` no-args returns nonzero (usage error).
    #[test]
    fn bin_mkdir_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_mkdir("mkdir", &[], &ops, 0);
        assert_ne!(r, 0, "mkdir no args → usage error");
    }

    /// c:275 — `bin_rmdir` no-args returns nonzero (usage error).
    #[test]
    fn bin_rmdir_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rmdir("rmdir", &[], &ops, 0);
        assert_ne!(r, 0, "rmdir no args → usage error");
    }

    /// c:347 — `bin_ln` no-args returns nonzero (usage error, alt).
    #[test]
    fn bin_ln_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_ln("ln", &[], &ops, 0);
        assert_ne!(r, 0, "ln no args → usage error");
    }

    /// c:757 — `bin_rm` no-args returns nonzero (usage error, alt).
    #[test]
    fn bin_rm_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_rm("rm", &[], &ops, 0);
        assert_ne!(r, 0, "rm no args → usage error");
    }

    /// c:845 — `bin_chmod` no-args returns nonzero (usage error, alt).
    #[test]
    fn bin_chmod_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_chmod("chmod", &[], &ops, 0);
        assert_ne!(r, 0, "chmod no args → usage error");
    }

    /// c:963 — `bin_chown` no-args returns nonzero (usage error, alt).
    #[test]
    fn bin_chown_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_chown("chown", &[], &ops, 0);
        assert_ne!(r, 0, "chown no args → usage error");
    }

    /// c:222 — `domkdir` on already-existing dir returns nonzero (EEXIST).
    #[test]
    fn domkdir_existing_dir_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = domkdir("mkdir", "/tmp", 0o755, 0);
        assert_ne!(r, 0, "mkdir /tmp (already exists) must error");
    }

    /// c:222 — `domkdir` empty path returns nonzero (no path).
    #[test]
    fn domkdir_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = domkdir("mkdir", "", 0o755, 0);
        assert_ne!(r, 0, "mkdir empty path must error");
    }

    /// c:222 — `domkdir` returns i32 (compile-time pin, alt).
    #[test]
    fn domkdir_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = domkdir("mkdir", "/__nonexistent_xyz__", 0o755, 0);
    }
}
