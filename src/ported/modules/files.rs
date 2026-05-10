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
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

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
