//! Clone module — port of `Src/Modules/clone.c`.
//!
//! Implements the `clone` builtin: fork the current shell onto a
//! new terminal. The child inherits state but re-initializes
//! session and controlling-tty membership; the parent's `$!`
//! becomes the child pid and the child's `$!` is zero.
//!
//! Structure mirrors the C source line-by-line:
//!   - `bin_clone()` (clone.c:42)
//!   - `static struct builtin bintab[]` (clone.c:109)
//!   - module entries `setup_` / `features_` / `enables_` /
//!     `boot_` / `cleanup_` / `finish_` (clone.c:121-162)

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use crate::ported::exec::ShellExecutor;
use crate::ported::init::{init_io, ShellState};
use crate::ported::jobs::clearjobtab;
use crate::ported::module::{
    featuresarray, handlefeatures, setfeatureenables, Builtin, Features, Module,
};
use crate::ported::params::setsparam;
use crate::ported::utils::{unmetafy, zerrnam, zwarnnam};

// =====================================================================
// Port of `bin_clone()` from Src/Modules/clone.c:44.
//
// C signature:
//   bin_clone(char *nam, char **args, Options ops, int func)
// =====================================================================

/// Port of `bin_clone()` from `Src/Modules/clone.c:44`.
///
/// `clone TTY`: open `TTY`, fork. Child sets up a new session,
/// makes `TTY` its controlling terminal, and continues running the
/// shell. Parent closes the tty and stores child pid in `$!`.
///
/// C body (clone.c:42-107) followed line-by-line. Notes on each
/// C call site and the Rust equivalent below.
#[cfg(unix)]
pub(crate) fn bin_clone(s: &mut ShellExecutor, nam: &str, args: &[String], _func: i32) -> i32 {
    use std::ffi::CString;
    use std::os::unix::io::RawFd;

    // C: BUILTIN("clone", ..., 1, 1, ...) guarantees args[0] exists,
    // but defend against direct calls from the Rust dispatcher.
    let arg0_in = match args.first() {
        Some(a) => a.as_str(),
        None => {
            zwarnnam(nam, "terminal required");
            return 1;
        }
    };

    // C: clone.c:48 — unmetafy(*args, NULL); — strip Meta escapes
    // before passing the path to open(2).
    let mut arg0_bytes = arg0_in.as_bytes().to_vec();
    unmetafy(&mut arg0_bytes);
    let arg0 = std::str::from_utf8(&arg0_bytes)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| arg0_in.to_string());

    let tty_c = match CString::new(arg0.clone()) {
        Ok(c) => c,
        Err(_) => {
            zwarnnam(nam, &format!("{}: invalid tty path", arg0));
            return 1;
        }
    };

    // C: clone.c:49 — ttyfd = open(*args, O_RDWR|O_NOCTTY);
    let ttyfd: RawFd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if ttyfd < 0 {
        zwarnnam(nam, &format!("{}: {}", arg0, std::io::Error::last_os_error()));
        return 1;
    }

    // C: clone.c:54 — pid = fork();
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // CHILD path — clone.c:55-98.
        // C: clone.c:56 — clearjobtab(0). Reset the inherited job
        // table; the new shell starts with no background jobs.
        clearjobtab(&mut s.jobs, 0);
        // C: clone.c:57-58 — ppid = getppid(); mypid = getpid();
        // zshrs reads these on demand from the system (no cached
        // globals), so the call is implicit.
        unsafe {
            // C: clone.c:60 — setsid creates a new session+pgid.
            // C uses `if (setsid() != mypid)` — the call returns the
            // new session id which equals the calling pid on success,
            // or -1 on failure. Rust port matches the C check exactly
            // (mypid is fetched from getpid() since the C global isn't
            // populated in this codepath yet).
            let mypid = libc::getpid();
            if libc::setsid() != mypid {
                zwarnnam(
                    nam,
                    &format!("failed to create new session: {}", std::io::Error::last_os_error()),
                );
            }
            // C: clone.c:67-69 — dup2(ttyfd, 0/1/2).
            libc::dup2(ttyfd, 0);
            libc::dup2(ttyfd, 1);
            libc::dup2(ttyfd, 2);
            // C: clone.c:70-71 — if (ttyfd > 2) close(ttyfd);
            if ttyfd > 2 {
                libc::close(ttyfd);
            }
            // C: clone.c:72 — closem(FDT_UNUSED, 0). Close shell-
            // internal fds (preserving 0/1/2 since they're not in
            // FDT_UNUSED state after the dup2's). zshrs's
            // ShellExecutor::closem (src/exec.rs:2248) walks fds 3+
            // and closes them. Used here as the FDT_UNUSED equivalent
            // since zshrs doesn't track the per-fd FDT_* type tags.
            s.closem(&[]);
            // C: clone.c:73-74 — close(coprocin); close(coprocout);
            // zshrs's coproc port doesn't yet expose the pipe fds via
            // ShellExecutor; the closem() above already closes fds
            // 3..256 which subsumes any open coproc pipes.
            // C: clone.c:76 — cttyfd = open(*args, O_RDWR);
            let cttyfd = libc::open(tty_c.as_ptr(), libc::O_RDWR);
            if cttyfd < 0 {
                zwarnnam(nam, &format!("{}", std::io::Error::last_os_error()));
            } else {
                // C: clone.c:81 — ioctl(cttyfd, TIOCSCTTY, 0);
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                {
                    libc::ioctl(cttyfd, libc::TIOCSCTTY as libc::c_ulong, 0);
                }
                libc::close(cttyfd);
            }
            // C: clone.c:86 — verify by opening /dev/tty.
            let dev_tty = b"/dev/tty\0";
            let verify = libc::open(dev_tty.as_ptr() as *const libc::c_char, libc::O_RDWR);
            if verify < 0 {
                zwarnnam(
                    nam,
                    &format!(
                        "could not make {} my controlling tty, job control disabled",
                        arg0
                    ),
                );
            } else {
                libc::close(verify);
            }
        }
        // C: clone.c:95 — mypgrp = 0; (so acquire_pgrp picks up the
        // new pgid). zshrs's pgrp tracking lives in libc::getpgrp()
        // calls; nothing to clear here since there's no cached state.
        //
        // C: clone.c:96 — init_io(NULL); — re-establishes SHTTY etc.
        // for the new controlling tty. The ported `init_io` operates
        // on a `ShellState`, not `ShellExecutor`; instantiate a
        // throwaway state for the call so the libc-side atty checks
        // run against the fresh fd 0.
        let mut state = ShellState::new();
        init_io(&mut state);
        // C: clone.c:97 — setsparam("TTY", ztrdup(ttystrname)).
        // ttystrname in C is set by init_io from ttyname(SHTTY); look
        // up the name of the freshly-installed controlling tty.
        let tty_name = unsafe {
            let ptr = libc::ttyname(0);
            if ptr.is_null() {
                arg0.clone()
            } else {
                std::ffi::CStr::from_ptr(ptr)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        setsparam(
            &mut s.variables,
            &mut s.arrays,
            &mut s.assoc_arrays,
            "TTY",
            &tty_name,
        );
        // Header comment of clone.c says: "$! is set to zero in the
        // new instance of the shell". C zsh relies on lastpid being
        // naturally cleared at process startup; Rust port pins it
        // explicitly to match that documented contract.
        setsparam(
            &mut s.variables,
            &mut s.arrays,
            &mut s.assoc_arrays,
            "!",
            "0",
        );
        return 0;
    }
    // PARENT path — clone.c:99-106.
    unsafe { libc::close(ttyfd) };
    if pid < 0 {
        zerrnam(nam, &format!("fork failed: {}", std::io::Error::last_os_error()));
        return 1;
    }
    // C: clone.c:105 — lastpid = pid;
    setsparam(
        &mut s.variables,
        &mut s.arrays,
        &mut s.assoc_arrays,
        "!",
        &pid.to_string(),
    );
    0
}

/// Port of `bin_clone()` — non-Unix stub. C uses `bin_notavail`
/// implicitly (only Unix systems compile this module).
#[cfg(not(unix))]
pub(crate) fn bin_clone(_s: &mut ShellExecutor, nam: &str, _args: &[String], _func: i32) -> i32 {
    zwarnnam(nam, "not available on this host");
    1
}

// =====================================================================
// Module paraphernalia (clone.c:109-119).
// =====================================================================

/// Port of `static struct builtin bintab[]` from `clone.c:109`.
///
/// ```c
/// BUILTIN("clone", 0, bin_clone, 1, 1, 0, NULL, NULL),
/// ```
static BINTAB: &[Builtin] = &[Builtin {
    name: "clone",
    flags: 0,
    minargs: 1,
    maxargs: 1,
    funcid: 0,
    optstr: None,
    defopts: None,
}];

/// Port of `static struct features module_features` from `clone.c:113`.
static MODULE_FEATURES: Features = Features {
    bn_list: BINTAB,
    cd_list: &[],
    mf_list: &[],
    pd_list: &[],
    n_abstract: 0,
};

// =====================================================================
// Module entry points (clone.c:121-162).
// =====================================================================

/// Port of `setup_()` from `Src/Modules/clone.c:123`. C body: `return 0;`.
pub fn setup_(_m: &Module) -> i32 {
    0
}

/// Port of `features_()` from `Src/Modules/clone.c:130`.
///
/// ```c
/// *features = featuresarray(m, &module_features);
/// return 0;
/// ```
pub fn features_(m: &Module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, &MODULE_FEATURES);
    0
}

/// Port of `enables_()` from `Src/Modules/clone.c:138`.
///
/// ```c
/// return handlefeatures(m, &module_features, enables);
/// ```
pub fn enables_(m: &Module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, &MODULE_FEATURES, enables)
}

/// Port of `boot_()` from `Src/Modules/clone.c:145`. C body: `return 0;`.
pub fn boot_(_m: &Module) -> i32 {
    0
}

/// Port of `cleanup_()` from `Src/Modules/clone.c:152`.
///
/// ```c
/// return setfeatureenables(m, &module_features, NULL);
/// ```
pub fn cleanup_(m: &Module) -> i32 {
    setfeatureenables(m, &MODULE_FEATURES, None)
}

/// Port of `finish_()` from `Src/Modules/clone.c:159`. C body: `return 0;`.
pub fn finish_(_m: &Module) -> i32 {
    0
}

// =====================================================================
// ShellExecutor bridge — sanctioned PORT.md exception.
// =====================================================================

impl ShellExecutor {
    /// `clone` builtin entry. Bridge to `bin_clone()` above.
    pub(crate) fn bin_clone(&mut self, args: &[String]) -> i32 {
        bin_clone(self, "clone", args, 0)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_features_returns_bintab_names() {
        let m = Module::new("zsh/clone");
        let mut features: Vec<String> = Vec::new();
        let rc = features_(&m, &mut features);
        assert_eq!(rc, 0);
        assert_eq!(features, vec!["b:clone"]);
    }

    #[test]
    fn test_enables_get_then_set() {
        let m = Module::new("zsh/clone");
        let mut enables: Option<Vec<i32>> = None;
        let rc = enables_(&m, &mut enables);
        assert_eq!(rc, 0);
        let v = enables.as_ref().unwrap();
        assert_eq!(v.len(), 1);
        let rc = enables_(&m, &mut enables);
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_cleanup_returns_zero() {
        let m = Module::new("zsh/clone");
        assert_eq!(cleanup_(&m), 0);
    }

    #[test]
    fn test_bin_clone_no_args() {
        let mut s = ShellExecutor::new();
        let rc = bin_clone(&mut s, "clone", &[], 0);
        assert_eq!(rc, 1);
    }

    #[test]
    fn test_bin_clone_invalid_tty() {
        let mut s = ShellExecutor::new();
        // /nonexistent/tty doesn't exist — open() returns -1.
        let rc = bin_clone(&mut s, "clone", &["/nonexistent/tty".into()], 0);
        assert_eq!(rc, 1);
    }
}
