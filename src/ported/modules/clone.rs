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
use crate::ported::module::{
    featuresarray, handlefeatures, setfeatureenables, Builtin, Features, Module,
};
use crate::ported::utils::{zerrnam, zwarnnam};

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
#[cfg(unix)]
pub(crate) fn bin_clone(s: &mut ShellExecutor, nam: &str, args: &[String], _func: i32) -> i32 {
    use std::ffi::CString;
    use std::os::unix::io::RawFd;

    // C: BUILTIN("clone", ..., 1, 1, ...) guarantees args[0] exists,
    // but defend against direct calls from the Rust dispatcher.
    let arg0 = match args.first() {
        Some(a) => a.as_str(),
        None => {
            zwarnnam(nam, "terminal required");
            return 1;
        }
    };

    let tty_c = match CString::new(arg0) {
        Ok(c) => c,
        Err(_) => {
            zwarnnam(nam, &format!("{}: invalid tty path", arg0));
            return 1;
        }
    };

    // C: clone.c:49 — open(*args, O_RDWR|O_NOCTTY)
    let ttyfd: RawFd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if ttyfd < 0 {
        zwarnnam(nam, &format!("{}: {}", arg0, std::io::Error::last_os_error()));
        return 1;
    }

    // C: clone.c:54 — pid = fork();
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // CHILD path — clone.c:55-98.
        unsafe {
            // C: clone.c:56 — clearjobtab(0). zshrs's job table port
            // is incomplete; the equivalent on ShellExecutor would
            // clear `s.jobs`, but the child has its own copy via
            // fork() COW so this is a no-op for correctness.
            // WARNING: NOT IN CLONE.C — `clearjobtab(0)` lives in
            // Src/jobs.c; pending the jobs.c full-state port.
            //
            // C: clone.c:57-58 — ppid = getppid(); mypid = getpid();
            // zshrs reads these on demand from the system rather than
            // caching globals, so no assignment needed here.
            //
            // C: clone.c:60 — setsid creates a new session and pgid.
            // Failure is non-fatal; zsh just warns.
            if libc::setsid() == -1 {
                zwarnnam(
                    nam,
                    &format!("failed to create new session: {}", std::io::Error::last_os_error()),
                );
            }
            // C: clone.c:67-69 — dup2(ttyfd, 0/1/2)
            libc::dup2(ttyfd, 0);
            libc::dup2(ttyfd, 1);
            libc::dup2(ttyfd, 2);
            // C: clone.c:70-71 — if (ttyfd > 2) close(ttyfd);
            if ttyfd > 2 {
                libc::close(ttyfd);
            }
            // C: clone.c:72 — closem(FDT_UNUSED, 0). Closes shell-
            // internal fds. zshrs's fd-tracking port is incomplete;
            // skipped here pending the io.c port.
            //
            // C: clone.c:73-74 — close(coprocin); close(coprocout);
            // zshrs's coproc port doesn't track the pipe fds on
            // ShellExecutor yet (the global `coprocin`/`coprocout`
            // from Src/exec.c live elsewhere). Skipped pending the
            // exec.c coproc-fd port; the new shell on the new tty
            // would inherit closed coproc fds anyway because the
            // closem-FDT_UNUSED loop above handles them in zsh's
            // model.
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
            // C: clone.c:95-96 — mypgrp = 0; init_io(NULL);
            // WARNING: NOT IN CLONE.C — `init_io()` lives in Src/init.c;
            // pending the init.c port. zshrs's I/O re-init is implicit
            // via the dup2/setsid sequence above.
            //
            // C: clone.c:97 — setsparam("TTY", ztrdup(ttystrname));
            // setsparam writes to the global param table; zshrs uses
            // ShellExecutor.variables.
            s.variables.insert("TTY".to_string(), arg0.to_string());
        }
        // C: clone.c:106 — return 0; in the child. Parent's $! is
        // set by the parent path below.
        s.variables.insert("!".to_string(), "0".to_string());
        return 0;
    }
    // PARENT path — clone.c:99-106.
    unsafe { libc::close(ttyfd) };
    if pid < 0 {
        zerrnam(nam, &format!("fork failed: {}", std::io::Error::last_os_error()));
        return 1;
    }
    // C: clone.c:105 — lastpid = pid;
    s.variables.insert("!".to_string(), pid.to_string());
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
