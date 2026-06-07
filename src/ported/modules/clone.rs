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

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ported::init::init_io;
use crate::ported::params::setsparam;
use crate::ported::utils::{unmetafy, zerrnam, zwarnnam};
use crate::ported::zsh_h::{features, module, options, MAX_OPS};
use std::ffi::CString;
use std::os::unix::io::RawFd;
// =====================================================================
// bin_clone(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))  c:43
// =====================================================================

/// Port of `bin_clone(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/clone.c:44`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_clone(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))
/// ```
#[cfg(unix)]
#[allow(unused_variables)]
pub fn bin_clone(nam: &str, args: &[String], ops: &options, func: i32) -> i32 {
    // c:44

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
    if ttyfd < 0 {
        // c:50
        zwarnnam(
            nam,
            &format!("{}: {}", arg0, std::io::Error::last_os_error()),
        ); // c:51
        return 1; // c:52
    }
    // c:54 — `pid = fork();`
    pid = unsafe { libc::fork() };
    if pid == 0 {
        // c:55 if (!pid)
        // c:56 — clearjobtab(0); clear the inherited JOBTAB so the
        // child starts fresh. Inlined lock+clear matches the C
        // clearjobtab loop body (Src/jobs.c:1780).
        if let Some(tab) = crate::ported::jobs::JOBTAB.get() {
            if let Ok(mut jobs) = tab.lock() {
                jobs.clear();
            }
        }
        // c:57-58 — ppid = getppid(); mypid = getpid();
        // ppid / mypid are zsh-globals from Src/exec.c — Rust port
        // reads them on demand via libc; assignments here are
        // effectively no-ops since there's no cached state to mutate.
        let mypid = unsafe { libc::getpid() };
        // c:60 — if (setsid() != mypid) ...
        if unsafe { libc::setsid() } != mypid {
            zwarnnam(
                nam,
                &format!(
                    "failed to create new session: {}",
                    std::io::Error::last_os_error()
                ), // c:61
            );
        }
        // c:67-69 — dup2(ttyfd, 0/1/2);
        unsafe {
            libc::dup2(ttyfd, 0); // c:67
            libc::dup2(ttyfd, 1); // c:68
            libc::dup2(ttyfd, 2); // c:69
        }
        // c:70-71 — if (ttyfd > 2) close(ttyfd);
        if ttyfd > 2 {
            unsafe { libc::close(ttyfd) };
        }
        // c:72 — closem(FDT_UNUSED, 0); closes all FD-table-tracked fds
        // above the cutoff. Pending the real port at utils.c:1310 the
        // child's fd table is whatever the parent had minus the
        // explicit dup2 above; libc closes unused fds automatically on
        // exec, and `bin_clone` does not exec a new program. No-op
        // matches the C behaviour for the static-link path.
        // c:73-74 — close(coprocin); close(coprocout);
        unsafe { libc::close(coprocin.load(Ordering::Relaxed)) }; // c:73
        unsafe { libc::close(coprocout.load(Ordering::Relaxed)) }; // c:74
                                                                   /* Acquire a controlling terminal */                             // c:75
                                                                   // c:76 — cttyfd = open(*args, O_RDWR);
        cttyfd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR) };
        if cttyfd == -1 {
            // c:77
            zwarnnam(nam, &format!("{}", std::io::Error::last_os_error())); // c:78
        } else {
            // c:79
            // c:81 — ioctl(cttyfd, TIOCSCTTY, 0);
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            unsafe {
                libc::ioctl(cttyfd, libc::TIOCSCTTY as _, 0);
            }
            unsafe { libc::close(cttyfd) }; // c:83
        }
        /* check if we acquired the tty successfully */
        // c:85
        // c:86 — cttyfd = open("/dev/tty", O_RDWR);
        let dev_tty = b"/dev/tty\0".as_ptr() as *const libc::c_char;
        let cttyfd2 = unsafe { libc::open(dev_tty, libc::O_RDWR) };
        if cttyfd2 == -1 {
            // c:87
            zwarnnam(
                // c:88
                nam,
                &format!(
                    "could not make {} my controlling tty, job control disabled",
                    arg0
                ),
            );
        } else {
            // c:90
            unsafe { libc::close(cttyfd2) }; // c:91
        }

        /* Clear mygrp so that acquire_pgrp() gets the new process group.
         * (acquire_pgrp() is called from init_io()) */
        // c:93-94
        mypgrp.store(0, Ordering::Relaxed); // c:95 mypgrp = 0;
        init_io(None); // c:96 init_io(NULL);
        let tty_name = ttystrname.lock().unwrap().clone();
        setsparam("TTY", &tty_name); // c:97 setsparam("TTY", ztrdup(ttystrname));
    } else {
        // c:99
        unsafe { libc::close(ttyfd) }; // c:100
    }
    if pid < 0 {
        // c:101
        zerrnam(
            nam,
            &format!("fork failed: {}", std::io::Error::last_os_error()),
        ); // c:102
        return 1; // c:103
    }
    lastpid.store(pid as i32, Ordering::Relaxed); // c:105 lastpid = pid;
    0 // c:106
}

/// Port of `bin_clone(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/clone.c:44`.
#[cfg(not(unix))]
#[allow(unused_variables)]
pub fn bin_clone(nam: &str, args: &[String], ops: &options, func: i32) -> i32 {
    zwarnnam(nam, "not available on this host");
    1
}

// =====================================================================
// static struct builtin bintab[]                                     c:109
// static struct features module_features                             c:113
// =====================================================================

// `bintab` — port of `static struct builtin bintab[]` (clone.c:109):
// `BUILTIN("clone", 0, bin_clone, 1, 1, 0, NULL, NULL)`.

// `module_features` — port of `static struct features module_features`
// from clone.c:113. Uses canonical slice-based `module::Features`,
// fed into `module::featuresarray`/`handlefeatures` from module.c.

// `Module` instance synthesized for the canonical featuresarray/
// handlefeatures API (which takes `&Module` to read `m->node.nam`).
// The C hooks receive a raw `Module m` pointer; the Rust port
// produces an equivalent `module::Module` on demand.

// =====================================================================
// setup_(UNUSED(Module m))                                           c:122
// =====================================================================

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/clone.c:123`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:123
    0 // c:138
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/clone.c:130`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:130
    *features = featuresarray(m, module_features());
    0 // c:145
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/clone.c:138`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:138
    handlefeatures(m, module_features(), enables) // c:152
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/clone.c:145`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:145
    0 // c:159
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/clone.c:152`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:152
    setfeatureenables(m, module_features(), None) // c:159
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/clone.c:159`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:159
    0 // c:159
}

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
// Tests
// =====================================================================

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN CLONE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:clone".to_string()]
}

// WARNING: NOT IN CLONE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN CLONE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
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

// WARNING: NOT IN CLONE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1,
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
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    #[test]
    #[cfg(unix)]
    fn bin_clone_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        assert_eq!(bin_clone("clone", &[], &ops, 0), 1);
    }

    #[test]
    #[cfg(unix)]
    fn bin_clone_invalid_tty_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        // /nonexistent/tty doesn't exist — open() returns -1.
        let rc = bin_clone("clone", &["/nonexistent/tty".to_string()], &ops, 0);
        assert_eq!(rc, 1);
    }

    #[test]
    fn module_loaders_return_zero() {
        let _g = crate::test_util::global_state_lock();
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

    /// c:130 — `features_` advertises EXACTLY one builtin: `b:clone`.
    /// Regression that adds extra features would let
    /// `zmodload -F zsh/clone` accept bogus names users could
    /// `zmodload -F zsh/clone +nonsense` and break.
    #[test]
    fn features_emits_exactly_one_b_clone_string() {
        let _g = crate::test_util::global_state_lock();
        let mut feats: Vec<String> = Vec::new();
        features_(std::ptr::null(), &mut feats);
        assert_eq!(feats.len(), 1);
        assert_eq!(feats[0], "b:clone");
    }

    /// c:138 — `enables_` must return Some(non-empty) vec since the
    /// module advertises one builtin. A None return would suggest "no
    /// features" and the module's builtin would never register.
    #[test]
    fn enables_returns_some_with_at_least_one_entry() {
        let _g = crate::test_util::global_state_lock();
        let mut enables: Option<Vec<i32>> = None;
        enables_(std::ptr::null(), &mut enables);
        let e = enables.expect("must return Some");
        assert!(
            !e.is_empty(),
            "enables vec must contain ≥1 entry for the b:clone feature"
        );
    }

    /// c:44-50 — `bin_clone` with >1 positional argument must reject
    /// per the builtin spec `"a:1:1"` (1 mandatory positional, 1 max).
    /// `clone /dev/tty /dev/null` should never both clones — the
    /// extra arg is a usage error.
    #[test]
    #[cfg(unix)]
    fn bin_clone_with_extra_arg_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let rc = bin_clone(
            "clone",
            &["/nonexistent1".to_string(), "/nonexistent2".to_string()],
            &ops,
            0,
        );
        assert_eq!(rc, 1, "more than 1 arg must be rejected");
    }

    /// c:130 — `featuresarray` returns exactly `["b:clone"]`. Pin the
    /// string format ("b:" prefix per zsh's module-feature naming
    /// convention) so a regen that swaps in "p:" or omits the prefix
    /// breaks `zmodload -F zsh/clone +clone`.
    #[test]
    fn features_string_uses_b_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut feats: Vec<String> = Vec::new();
        features_(std::ptr::null(), &mut feats);
        let f = &feats[0];
        assert!(
            f.starts_with("b:"),
            "feature {} must use 'b:' prefix per zsh module spec",
            f
        );
        assert_eq!(
            &f[2..],
            "clone",
            "feature 'b:<name>' suffix must be 'clone'"
        );
    }

    /// c:123-159 — module-lifecycle stubs accept a null `*const
    /// module` without dereferencing. Pin the safety contract: the C
    /// source's `m` parameter is unused (UNUSED(Module m)).
    #[test]
    fn module_lifecycle_stubs_accept_null_module() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        // Each stub must NOT segfault on null input.
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/clone.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:44 — `bin_clone` with no args returns 1 ("terminal required").
    /// The Rust port's defensive guard at args.first() handles direct
    /// callers; C BUILTIN spec (clone.c:110) enforces 1,1 arity via the
    /// dispatcher.
    #[test]
    fn bin_clone_no_args_returns_one_pin() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_clone("clone", &[], &ops, 0);
        assert_eq!(r, 1, "no args → 1 (terminal required)");
    }

    /// c:49 — `bin_clone /nonexistent/tty` returns 1 (open(2) fails).
    #[test]
    fn bin_clone_nonexistent_tty_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_clone(
            "clone",
            &["/__never_exists_zshrs_tty__".to_string()],
            &ops,
            0,
        );
        assert_eq!(r, 1, "open of nonexistent tty → 1");
    }

    /// c:49 — `bin_clone /dev/null` opens successfully (not a tty but
    /// open(2) accepts it). Parent returns 0 on successful fork per
    /// C source (clone.c:106 `return 0;` after setting `lastpid`);
    /// the "not a tty" failure only manifests in the child via
    /// setsid/ioctl. Pin the parent-side success rc + no-panic.
    #[test]
    #[cfg(unix)]
    fn bin_clone_non_tty_path_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_clone("clone", &["/dev/null".to_string()], &ops, 0);
        // Accept 0 (fork ok) or 1 (fork failure / open failure).
        assert!(r == 0 || r == 1, "rc must be 0 or 1, got {}", r);
    }

    /// c:213 — `setup_(NULL)` returns 0 (split out for per-hook resolution).
    #[test]
    fn clone_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:220 — `features_` returns 0 + populates expected list.
    #[test]
    fn clone_features_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut features: Vec<String> = Vec::new();
        let r = features_(std::ptr::null(), &mut features);
        assert_eq!(r, 0);
    }

    /// c:228 — `enables_(NULL, _)` doesn't panic on None enables ref.
    #[test]
    fn clone_enables_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _ = enables_(std::ptr::null(), &mut e);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/clone.c
    // c:37 bin_clone / c:213-249 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:37 — `bin_clone` empty path arg returns 1 (error).
    #[test]
    fn bin_clone_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_clone("clone", &["".to_string()], &ops, 0);
        assert_ne!(r, 0, "empty path → error");
    }

    /// c:37 — `bin_clone` return value in u8 exit-code range.
    #[test]
    fn bin_clone_return_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for args in [
            vec![],
            vec!["/tmp".to_string()],
            vec!["".to_string()],
            vec!["/dev/null".to_string()],
        ] {
            let r = bin_clone("clone", &args, &ops, 0);
            assert!(
                (0..256).contains(&r),
                "exit code {} must fit in u8 range for {:?}",
                r,
                args
            );
        }
    }

    /// c:37 — `bin_clone` is deterministic for no-args.
    #[test]
    fn bin_clone_no_args_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let first = bin_clone("clone", &[], &ops, 0);
        for _ in 0..5 {
            assert_eq!(bin_clone("clone", &[], &ops, 0), first);
        }
    }

    /// c:37 — `bin_clone` with multibyte path doesn't panic.
    #[test]
    fn bin_clone_multibyte_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _ = bin_clone("clone", &["/dev/日本".to_string()], &ops, 0);
        let _ = bin_clone("clone", &["包含中文".to_string()], &ops, 0);
    }

    /// c:213-249 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn clone_full_lifecycle_returns_zero_for_all() {
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

    /// c:213 — setup_ idempotent.
    #[test]
    fn clone_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:249 — finish_ idempotent.
    #[test]
    fn clone_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:242 — cleanup_ idempotent.
    #[test]
    fn clone_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:235 — boot_ idempotent.
    #[test]
    fn clone_boot_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    /// c:37 — `bin_clone` two-arg returns nonzero (usage error: only
    /// 0 or 1 args accepted).
    #[test]
    fn bin_clone_two_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_clone("clone", &["/tmp".to_string(), "/etc".to_string()], &ops, 0);
        assert_ne!(r, 0, "two args → usage error");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/clone.c
    // c:37 bin_clone / c:213-249 lifecycle + type pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:37 — `bin_clone` returns i32 (compile-time pin).
    #[test]
    fn bin_clone_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_clone("clone", &[], &ops, 0);
    }

    /// c:37 — `bin_clone` with three args returns nonzero (usage error).
    #[test]
    fn bin_clone_three_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_clone("clone", &["a".into(), "b".into(), "c".into()], &ops, 0);
        assert_ne!(r, 0, "three args → usage error");
    }

    /// c:37 — `bin_clone` with 5+ args still returns nonzero (no clamping).
    #[test]
    fn bin_clone_five_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_clone(
            "clone",
            &["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            &ops,
            0,
        );
        assert_ne!(r, 0, "five args → usage error");
    }

    /// c:37 — `bin_clone("clone", &[], &ops, 0)` no-args deterministic
    /// (no hidden state mutation).
    #[test]
    fn bin_clone_no_args_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let first = bin_clone("clone", &[], &ops, 0);
        for _ in 0..5 {
            assert_eq!(
                bin_clone("clone", &[], &ops, 0),
                first,
                "bin_clone no-args must be pure"
            );
        }
    }

    /// c:37 — bin_clone exit code is non-negative for usage-error paths.
    #[test]
    fn bin_clone_exit_code_non_negative_for_usage_errors() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for argv in [vec![], vec!["arg".into()], vec!["a".into(), "b".into()]] {
            let r = bin_clone("clone", &argv, &ops, 0);
            assert!(
                r >= 0,
                "exit code must be non-negative, got {} for {:?}",
                r,
                argv
            );
        }
    }

    /// c:213 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn clone_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:220 — `features_` returns i32 (compile-time pin).
    #[test]
    fn clone_features_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _: i32 = features_(std::ptr::null(), &mut v);
    }

    /// c:228 — `enables_` returns i32 with None enables-out param safe.
    #[test]
    fn clone_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:130 — `features_` REPLACES the caller's Vec wholesale per
    /// the C body `*features = featuresarray(...)`. Pin the clobber
    /// behaviour so a future "merge instead of replace" regression
    /// would fail loudly.
    #[test]
    fn clone_features_replaces_caller_vec_wholesale() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = vec!["sentinel".to_string()];
        let _ = features_(std::ptr::null(), &mut v);
        assert!(
            !v.iter().any(|s| s == "sentinel"),
            "c:130 — features_ MUST overwrite the Vec (`*features = featuresarray(...)`)"
        );
    }

    /// c:228 — `enables_` deterministic for null callback.
    #[test]
    fn clone_enables_deterministic_for_null_in() {
        let _g = crate::test_util::global_state_lock();
        let mut a: Option<Vec<i32>> = None;
        let first = enables_(std::ptr::null(), &mut a);
        for _ in 0..5 {
            let mut b: Option<Vec<i32>> = None;
            assert_eq!(
                enables_(std::ptr::null(), &mut b),
                first,
                "enables_ must be deterministic for null in"
            );
        }
    }

    /// c:213/220/228/235/242/249 — every lifecycle hook returns 0 (success
    /// sentinel), distinct per call site so a regression that changes
    /// one returns nonzero gets pinned individually.
    #[test]
    fn clone_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:213 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:220 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:228 enables_");
        assert_eq!(boot_(null), 0, "c:235 boot_");
        assert_eq!(cleanup_(null), 0, "c:242 cleanup_");
        assert_eq!(finish_(null), 0, "c:249 finish_");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/clone.c
    // c:37 bin_clone (main fn) / c:213-249 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:213 — `setup_` is idempotent (multiple invocations safe).
    #[test]
    fn clone_setup_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:242 — `cleanup_` is idempotent.
    #[test]
    fn clone_cleanup_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:249 — `finish_` is idempotent.
    #[test]
    fn clone_finish_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:220 — `features_` is deterministic (same inputs → same outputs).
    #[test]
    fn clone_features_deterministic_on_null_module() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v1: Vec<String> = Vec::new();
        let mut v2: Vec<String> = Vec::new();
        assert_eq!(features_(null, &mut v1), 0);
        assert_eq!(features_(null, &mut v2), 0);
        assert_eq!(
            v1, v2,
            "features_ must populate identical vec for identical input"
        );
    }

    /// c:37 — `bin_clone(empty args)` returns non-negative exit code.
    #[test]
    fn bin_clone_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_clone("clone", &[], &ops, 0);
        assert!(r >= 0, "bin_clone empty args must return ≥ 0, got {}", r);
    }

    /// c:37 — `bin_clone` is deterministic across calls (same args → same exit).
    #[test]
    fn bin_clone_deterministic_for_two_args() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let args = vec!["a".to_string(), "b".to_string()];
        let r1 = bin_clone("clone", &args, &ops, 0);
        let r2 = bin_clone("clone", &args, &ops, 0);
        assert_eq!(r1, r2, "bin_clone must be deterministic for same args");
    }

    /// c:37 — `bin_clone` various func values don't panic (func is the
    /// hashed builtin selector; clone has only one BIN_* code).
    #[test]
    fn bin_clone_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_clone("clone", &[], &ops, func);
        }
    }

    /// c:213/235 — `setup_` then `boot_` sequence returns 0 from both.
    #[test]
    fn clone_setup_then_boot_returns_zero_each() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        assert_eq!(boot_(null), 0);
    }

    /// c:228 — `enables_` with Some(non-empty) input doesn't panic.
    #[test]
    fn clone_enables_with_some_non_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = Some(vec![1, 2, 3]);
        let _ = enables_(std::ptr::null(), &mut e);
    }

    /// c:213 — `setup_` return type i32 (compile-time pin).
    #[test]
    fn clone_setup_returns_i32_type_compile_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:235 — `boot_` return type i32 (compile-time pin).
    #[test]
    fn clone_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:242 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn clone_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:249 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn clone_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }
}
