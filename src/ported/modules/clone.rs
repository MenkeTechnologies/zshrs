//! `zsh/clone` module — port of `Src/Modules/clone.c`.
//!
//! Top-level declaration order matches C source line-by-line:
//!   - `bin_clone(nam, args, ops, func)`            c:43
//!   - `static struct builtin bintab[]`             c:109
//!   - `static struct features module_features`     c:113
//!   - `setup_(m)` / `features_(m, features)` /
//!     `enables_(m, enables)` / `boot_(m)` /
//!     `cleanup_(m)` / `finish_(m)`                 c:122-162

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::ported::utils::{unmetafy, zerrnam, zwarnnam};
use crate::ported::zsh_h::{module, options};

// =====================================================================
// External C globals from other Src/*.c files. Mirrored as atomic /
// Mutex statics with the same case-sensitive C name; the eventual real
// ports of jobs.c / exec.c / init.c / params.c will replace these
// stubs in-place without touching call sites.
// =====================================================================

// `coprocin` / `coprocout` — `int` globals in `Src/exec.c:430-431`.
pub static coprocin: AtomicI32 = AtomicI32::new(-1);
pub static coprocout: AtomicI32 = AtomicI32::new(-1);

// `mypgrp` — `pid_t` global in `Src/jobs.c:60`.
pub static mypgrp: AtomicI32 = AtomicI32::new(0);

// `lastpid` — `pid_t` global in `Src/jobs.c:73` (zsh's `$!`).
pub static lastpid: AtomicI32 = AtomicI32::new(0);

// `ttystrname` — `char *` global in `Src/init.c:248`, set by
// `init_io` from `ttyname(SHTTY)`. Mirrored as `Mutex<String>` since
// the value is mutated at runtime by init_io.
pub static ttystrname: Mutex<String> = Mutex::new(String::new());

// =====================================================================
// bin_clone(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))  c:43
// =====================================================================

/// Port of `bin_clone()` from `Src/Modules/clone.c:44`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_clone(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))
/// ```
#[cfg(unix)]
pub fn bin_clone(nam: &str, args: &[String], _ops: &options, _func: i32) -> i32 { // c:44
    use std::ffi::CString;
    use std::os::unix::io::RawFd;

    // c:46 — `int ttyfd, pid, cttyfd;`
    let ttyfd: RawFd;
    let pid: libc::pid_t;
    let cttyfd: RawFd;

    // C: BUILTIN("clone", 0, bin_clone, 1, 1, 0, NULL, NULL) (clone.c:110)
    // guarantees args[0] exists; defend against direct calls anyway.
    let arg0_in: &str = match args.first() {
        Some(a) => a.as_str(),
        None => {
            zwarnnam(nam, "terminal required");
            return 1;
        }
    };

    // c:48 — `unmetafy(*args, NULL);` strip Meta escapes before open(2).
    let mut arg0_bytes = arg0_in.as_bytes().to_vec();
    unmetafy(&mut arg0_bytes);
    let arg0: String = String::from_utf8_lossy(&arg0_bytes).into_owned();

    let tty_c = match CString::new(arg0.clone()) {
        Ok(c) => c,
        Err(_) => {
            zwarnnam(nam, &format!("{}: invalid tty path", arg0));
            return 1;
        }
    };

    // c:49 — `ttyfd = open(*args, O_RDWR|O_NOCTTY);`
    ttyfd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if ttyfd < 0 {                                                       // c:50
        zwarnnam(nam, &format!("{}: {}", arg0, std::io::Error::last_os_error())); // c:51
        return 1;                                                        // c:52
    }
    // c:54 — `pid = fork();`
    pid = unsafe { libc::fork() };
    if pid == 0 {                                                        // c:55 if (!pid)
        // c:56 — clearjobtab(0);
        clearjobtab(0);
        // c:57-58 — ppid = getppid(); mypid = getpid();
        // ppid / mypid are zsh-globals from Src/exec.c — Rust port
        // reads them on demand via libc; assignments here are
        // effectively no-ops since there's no cached state to mutate.
        let mypid = unsafe { libc::getpid() };
        // c:60 — if (setsid() != mypid) ...
        if unsafe { libc::setsid() } != mypid {
            zwarnnam(
                nam,
                &format!("failed to create new session: {}", std::io::Error::last_os_error()), // c:61
            );
        }
        // c:67-69 — dup2(ttyfd, 0/1/2);
        unsafe {
            libc::dup2(ttyfd, 0);                                        // c:67
            libc::dup2(ttyfd, 1);                                        // c:68
            libc::dup2(ttyfd, 2);                                        // c:69
        }
        // c:70-71 — if (ttyfd > 2) close(ttyfd);
        if ttyfd > 2 {
            unsafe { libc::close(ttyfd) };
        }
        // c:72 — closem(FDT_UNUSED, 0);
        closem(FDT_UNUSED, 0);
        // c:73-74 — close(coprocin); close(coprocout);
        unsafe { libc::close(coprocin.load(Ordering::Relaxed)) };        // c:73
        unsafe { libc::close(coprocout.load(Ordering::Relaxed)) };       // c:74
        /* Acquire a controlling terminal */                             // c:75
        // c:76 — cttyfd = open(*args, O_RDWR);
        cttyfd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR) };
        if cttyfd == -1 {                                                // c:77
            zwarnnam(nam, &format!("{}", std::io::Error::last_os_error())); // c:78
        } else {                                                         // c:79
            // c:81 — ioctl(cttyfd, TIOCSCTTY, 0);
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            unsafe {
                libc::ioctl(cttyfd, libc::TIOCSCTTY as _, 0);
            }
            unsafe { libc::close(cttyfd) };                              // c:83
        }
        /* check if we acquired the tty successfully */                  // c:85
        // c:86 — cttyfd = open("/dev/tty", O_RDWR);
        let dev_tty = b"/dev/tty\0".as_ptr() as *const libc::c_char;
        let cttyfd2 = unsafe { libc::open(dev_tty, libc::O_RDWR) };
        if cttyfd2 == -1 {                                               // c:87
            zwarnnam(                                                     // c:88
                nam,
                &format!("could not make {} my controlling tty, job control disabled", arg0),
            );
        } else {                                                         // c:90
            unsafe { libc::close(cttyfd2) };                             // c:91
        }

        /* Clear mygrp so that acquire_pgrp() gets the new process group.
         * (acquire_pgrp() is called from init_io()) */                  // c:93-94
        mypgrp.store(0, Ordering::Relaxed);                              // c:95 mypgrp = 0;
        init_io(std::ptr::null_mut());                                   // c:96 init_io(NULL);
        let tty_name = ttystrname.lock().unwrap().clone();
        setsparam("TTY", ztrdup(&tty_name));                             // c:97
    } else {                                                             // c:99
        unsafe { libc::close(ttyfd) };                                   // c:100
    }
    if pid < 0 {                                                         // c:101
        zerrnam(nam, &format!("fork failed: {}", std::io::Error::last_os_error())); // c:102
        return 1;                                                        // c:103
    }
    lastpid.store(pid as i32, Ordering::Relaxed);                        // c:105 lastpid = pid;
    0                                                                    // c:106
}

#[cfg(not(unix))]
pub fn bin_clone(nam: &str, _args: &[String], _ops: &options, _func: i32) -> i32 {
    zwarnnam(nam, "not available on this host");
    1
}

// =====================================================================
// static struct builtin bintab[]                                     c:109
// static struct features module_features                             c:113
// =====================================================================

use std::sync::OnceLock;
use crate::ported::zsh_h::features as features_t;

// `module_features` — port of `static struct features module_features`
// from clone.c:113.
static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,                                                   // c:114 bintab
        bn_size: 1,                                                       // sizeof(bintab)/sizeof(*bintab)
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

// =====================================================================
// setup_(UNUSED(Module m))                                           c:122
// =====================================================================

/// Port of `setup_()` from `Src/Modules/clone.c:123`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:123
    0                                                                    // c:125
}

/// Port of `features_()` from `Src/Modules/clone.c:130`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {      // c:130
    *features = featuresarray(m, module_features());                     // c:132
    0                                                                    // c:133
}

/// Port of `enables_()` from `Src/Modules/clone.c:138`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {   // c:138
    handlefeatures(m, module_features(), enables)                        // c:140
}

/// Port of `boot_()` from `Src/Modules/clone.c:145`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:145
    0                                                                    // c:147
}

/// Port of `cleanup_()` from `Src/Modules/clone.c:152`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                                   // c:152
    setfeatureenables(m, module_features(), None)                        // c:154
}

/// Port of `finish_()` from `Src/Modules/clone.c:159`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:159
    0                                                                    // c:161
}

// `featuresarray` lives in `Src/module.c:3275`.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:clone".to_string()]
}

// `handlefeatures` lives in `Src/module.c:3370`.
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
    let total = g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract;
    vec![0; total as usize]
}

// `setfeatureenables` lives in `Src/module.c:3445`.
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

// =====================================================================
// External fns from other Src/*.c files. Stubbed locally pending the
// proper ports of their home files (Src/jobs.c, Src/exec.c, Src/init.c,
// Src/params.c, Src/string.c).
// =====================================================================

// `clearjobtab` lives in `Src/jobs.c:1780`. Clears the global JOBTAB
// so the cloned child starts with no inherited jobs (mirrors C's
// `clearjobtab(0)` in clone.c c:78).
fn clearjobtab(_monitor: i32) {                                              // c:1780
    if let Some(tab) = crate::ported::jobs::JOBTAB.get() {
        if let Ok(mut jobs) = tab.lock() {
            jobs.clear();
        }
    }
}

// `closem` lives in `Src/utils.c:1310`. `FDT_UNUSED` from zsh.h:1115.
const FDT_UNUSED: i32 = 0;
fn closem(_typ: i32, _all: i32) {}

// `init_io` lives in `Src/init.c:577`. Wires through the canonical
// port at init.rs:295.
fn init_io(cmd: *mut libc::c_char) {                                         // c:577
    let s: Option<String> = if cmd.is_null() {
        None
    } else {
        unsafe { std::ffi::CStr::from_ptr(cmd).to_str().ok().map(|s| s.to_string()) }
    };
    crate::ported::init::init_io(s.as_deref());
}

// `setsparam` lives in `Src/params.c:3380`. Stub for the static-link
// path — TTY/$! assignment in the cloned child can't yet feed the
// real param table.
fn setsparam(_name: &str, _value: &str) {}

// `ztrdup` lives in `Src/string.c:18` — `strdup` clone using zsh's
// allocator. Pass-through here; the `setsparam` stub copies anyway.
fn ztrdup(s: &str) -> &str { s }

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::MAX_OPS;

    fn empty_ops() -> options {
        options { ind: [0u8; MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }
    }

    #[test]
    #[cfg(unix)]
    fn bin_clone_no_args_returns_one() {
        let ops = empty_ops();
        assert_eq!(bin_clone("clone", &[], &ops, 0), 1);
    }

    #[test]
    #[cfg(unix)]
    fn bin_clone_invalid_tty_returns_one() {
        let ops = empty_ops();
        // /nonexistent/tty doesn't exist — open() returns -1.
        let rc = bin_clone("clone", &["/nonexistent/tty".to_string()], &ops, 0);
        assert_eq!(rc, 1);
    }

    #[test]
    fn module_loaders_return_zero() {
        let mut features: Vec<String> = Vec::new();
        let mut enables: Option<Vec<i32>> = None;
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(features_(m, &mut features), 0);
        assert_eq!(features, vec!["b:clone"]);
        assert_eq!(enables_(m, &mut enables), 0);
        assert!(enables.is_some());
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }
}
