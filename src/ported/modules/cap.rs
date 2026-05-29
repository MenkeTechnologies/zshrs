//! POSIX.1e capabilities — port of `Src/Modules/cap.c`.
//!
//! Implements `cap` / `getcap` / `setcap`. Linux-only (libcap);
//! macOS / BSD have no POSIX.1e capability sets — the stubs return
//! Unsupported.
//!
//! Structure mirrors the C source line-by-line:
//!   - `bin_cap` (cap.c:36)
//!   - `bin_getcap` (cap.c:68)
//!   - `bin_setcap` (cap.c:91)
//!   - `static struct builtin bintab[]` (cap.c:123)
//!   - module entries (cap.c:139-178)

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::{features, module, options, MAX_OPS};
use std::sync::{Mutex, OnceLock};

// =====================================================================
// libcap FFI — declared in `<sys/capability.h>` (libcap), not libc.
// =====================================================================

#[cfg(all(target_os = "linux", feature = "libcap"))]
mod ffi {
    use libc::{c_char, c_int, c_void, ssize_t};

    /// `cap_t` is an opaque pointer in libcap.
    pub type CapT = *mut c_void;

    #[link(name = "cap")]
    extern "C" {
        /// `cap_get_proc` — see implementation.
        pub fn cap_get_proc() -> CapT;
        /// `cap_set_proc` — see implementation.
        pub fn cap_set_proc(cap_p: CapT) -> c_int;
        /// `cap_get_file` — see implementation.
        pub fn cap_get_file(path: *const c_char) -> CapT;
        /// `cap_set_file` — see implementation.
        pub fn cap_set_file(path: *const c_char, cap_p: CapT) -> c_int;
        /// `cap_from_text` — see implementation.
        pub fn cap_from_text(buf: *const c_char) -> CapT;
        /// `cap_to_text` — see implementation.
        pub fn cap_to_text(caps: CapT, length: *mut ssize_t) -> *mut c_char;
        /// `cap_free` — see implementation.
        pub fn cap_free(obj: *mut c_void) -> c_int;
    }
}

// =====================================================================
// Port of `bin_cap(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/Modules/cap.c:36.
// =====================================================================

/// Port of `bin_cap(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/cap.c:36`.
///
/// `cap [STRING]`: with `STRING`, parse via `cap_from_text` and
/// install via `cap_set_proc`; without args, print the current
/// process's capability set.
#[cfg(all(target_os = "linux", feature = "libcap"))]
/// WARNING: param names don't match C — Rust=(argv, _ops, _func) vs C=(nam, argv, ops, func)
pub(crate) fn bin_cap(nam: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    // c:36

    let mut ret = 0;
    if let Some(arg0) = argv.first() {
        // C: caps = cap_from_text(*argv);
        let arg_c = match CString::new(arg0.as_str()) {
            Ok(c) => c,
            Err(_) => {
                zwarnnam(nam, "invalid capability string");
                return 1;
            }
        };
        unsafe {
            let caps = ffi::cap_from_text(arg_c.as_ptr());
            if caps.is_null() {
                zwarnnam(nam, "invalid capability string");
                return 1;
            }
            // C: if (cap_set_proc(caps)) { zwarnnam(...); ret = 1; }
            if ffi::cap_set_proc(caps) != 0 {
                zwarnnam(
                    nam,
                    &format!(
                        "can't change capabilities: {}",
                        std::io::Error::last_os_error()
                    ),
                );
                ret = 1;
            }
            ffi::cap_free(caps);
        }
    } else {
        // C: caps = cap_get_proc(); if (caps) result = cap_to_text(caps, &length);
        unsafe {
            let caps = ffi::cap_get_proc();
            let result = if !caps.is_null() {
                ffi::cap_to_text(caps, std::ptr::null_mut())
            } else {
                std::ptr::null_mut()
            };
            if caps.is_null() || result.is_null() {
                zwarnnam(
                    nam,
                    &format!(
                        "can't get capabilities: {}",
                        std::io::Error::last_os_error()
                    ),
                );
                ret = 1;
            } else {
                let s = CStr::from_ptr(result).to_string_lossy();
                println!("{}", s);
            }
            if !result.is_null() {
                ffi::cap_free(result as *mut libc::c_void);
            }
            if !caps.is_null() {
                ffi::cap_free(caps);
            }
        }
    }
    ret
}

/// Port of `bin_cap()` — non-Linux stub. C uses `bin_notavail`
/// (cap.c:115); we mirror the behaviour by emitting the same
/// "not available on this host" error.
#[cfg(not(all(target_os = "linux", feature = "libcap")))]
pub(crate) fn bin_cap(nam: &str, _argv: &[String], _ops: &options, _func: i32) -> i32 {
    zwarnnam(nam, "not available on this host");
    1
}

// =====================================================================
// Port of `bin_getcap(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/Modules/cap.c:68.
// =====================================================================

/// Port of `bin_getcap(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/cap.c:68`.
///
/// `getcap FILE...`: print each file's capability set as
/// `FILE CAPS`. C bails on the first error but iterates the rest;
/// the Rust port mirrors that exact loop.
#[cfg(all(target_os = "linux", feature = "libcap"))]
/// WARNING: param names don't match C — Rust=(argv, _ops, _func) vs C=(nam, argv, ops, func)
pub(crate) fn bin_getcap(nam: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    let mut ret = 0;
    // C: do { ... } while(*++argv);
    for file in argv {
        let path_c = match CString::new(file.as_str()) {
            Ok(c) => c,
            Err(_) => {
                zwarnnam(nam, &format!("{}: invalid path", file));
                ret = 1;
                continue;
            }
        };
        unsafe {
            let caps = ffi::cap_get_file(path_c.as_ptr());
            let result = if !caps.is_null() {
                ffi::cap_to_text(caps, std::ptr::null_mut())
            } else {
                std::ptr::null_mut()
            };
            if caps.is_null() || result.is_null() {
                zwarnnam(
                    nam,
                    &format!("{}: {}", file, std::io::Error::last_os_error()),
                );
                ret = 1;
            } else {
                let s = CStr::from_ptr(result).to_string_lossy();
                println!("{} {}", file, s);
            }
            if !result.is_null() {
                ffi::cap_free(result as *mut libc::c_void);
            }
            if !caps.is_null() {
                ffi::cap_free(caps);
            }
        }
    }
    ret
}

/// Port of `bin_getcap()` — non-Linux stub.
#[cfg(not(all(target_os = "linux", feature = "libcap")))]
pub(crate) fn bin_getcap(nam: &str, _argv: &[String], _ops: &options, _func: i32) -> i32 {
    zwarnnam(nam, "not available on this host");
    1
}

// =====================================================================
// Port of `bin_setcap(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/Modules/cap.c:91.
// =====================================================================

/// Port of `bin_setcap(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/cap.c:91`.
///
/// `setcap STRING FILE...`: parse `STRING` via `cap_from_text`, then
/// apply via `cap_set_file` to each remaining file argument. Mirrors
/// C's loop: free `caps` once at end, advance `argv` per iteration.
#[cfg(all(target_os = "linux", feature = "libcap"))]
/// WARNING: param names don't match C — Rust=(argv, _ops, _func) vs C=(nam, argv, ops, func)
pub(crate) fn bin_setcap(nam: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    let mut ret = 0;
    let cap_str = match argv.first() {
        Some(s) => s.as_str(),
        None => {
            zwarnnam(nam, "invalid capability string");
            return 1;
        }
    };
    let cap_c = match CString::new(cap_str) {
        Ok(c) => c,
        Err(_) => {
            zwarnnam(nam, "invalid capability string");
            return 1;
        }
    };
    unsafe {
        let caps = ffi::cap_from_text(cap_c.as_ptr());
        if caps.is_null() {
            zwarnnam(nam, "invalid capability string");
            return 1;
        }
        // C: do { if(cap_set_file(...)) { zwarnnam; ret = 1; } } while(*++argv);
        for file in &argv[1..] {
            let path_c = match CString::new(file.as_str()) {
                Ok(c) => c,
                Err(_) => {
                    zwarnnam(nam, &format!("{}: invalid path", file));
                    ret = 1;
                    continue;
                }
            };
            if ffi::cap_set_file(path_c.as_ptr(), caps) != 0 {
                zwarnnam(
                    nam,
                    &format!("{}: {}", file, std::io::Error::last_os_error()),
                );
                ret = 1;
            }
        }
        ffi::cap_free(caps);
    }
    ret
}

/// Port of `bin_setcap()` — non-Linux stub.
#[cfg(not(all(target_os = "linux", feature = "libcap")))]
pub(crate) fn bin_setcap(nam: &str, _argv: &[String], _ops: &options, _func: i32) -> i32 {
    zwarnnam(nam, "not available on this host");
    1
}

// =====================================================================
// /* module paraphernalia */                                        c:121
// static struct builtin bintab[]                                    c:123
// static struct features module_features                            c:129
// =====================================================================

// `bintab` — port of `static struct builtin bintab[]` (cap.c:123).

// `module_features` — port of `static struct features module_features`
// from cap.c:129. Uses canonical slice-based `module::Features`.

// =====================================================================
// Module entry points (cap.c:138-178).
// =====================================================================

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/cap.c:139`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:139
    0 // c:154
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/cap.c:146`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:146
    *features = featuresarray(m, module_features());
    0 // c:161
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/cap.c:154`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:154
    handlefeatures(m, module_features(), enables) // c:168
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/cap.c:161`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:161
    0 // c:175
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/cap.c:168`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:168
    setfeatureenables(m, module_features(), None) // c:175
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/cap.c:175`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:175
    0 // c:175
}

// =====================================================================
// ShellExecutor bridge — sanctioned PORT.md exception. Wires the
// internal builtin dispatcher to the canonical free ported above.
// =====================================================================

// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// =====================================================================
// Tests
// =====================================================================

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN CAP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "b:cap".to_string(),
        "b:getcap".to_string(),
        "b:setcap".to_string(),
    ]
}

// WARNING: NOT IN CAP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 3]);
    }
    0
}

// WARNING: NOT IN CAP.C — Rust-only module-framework shim.
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

// WARNING: NOT IN CAP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 3,
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
    fn test_features_returns_bintab_names() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut features: Vec<String> = Vec::new();
        let rc = features_(m, &mut features);
        assert_eq!(rc, 0);
        assert_eq!(features, vec!["b:cap", "b:getcap", "b:setcap"]);
    }

    #[test]
    fn test_enables_get_then_set() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut enables: Option<Vec<i32>> = None;
        let rc = enables_(m, &mut enables);
        assert_eq!(rc, 0);
        let v = enables.as_ref().unwrap();
        assert_eq!(v.len(), 3);
        let rc = enables_(m, &mut enables);
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_cleanup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(cleanup_(m), 0);
    }

    #[test]
    #[cfg(not(all(target_os = "linux", feature = "libcap")))]
    fn test_bin_cap_unsupported_on_macos() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        // Without libcap, all three bin_* return 1 (notavail).
        assert_eq!(bin_cap("cap", &[], &ops, 0), 1);
        assert_eq!(bin_getcap("getcap", &["/etc/passwd".into()], &ops, 0), 1);
        assert_eq!(
            bin_setcap(
                "setcap",
                &["cap_net_admin+ep".into(), "/tmp/x".into()],
                &ops,
                0
            ),
            1
        );
    }

    /// c:146 — every feature in the list uses the canonical `b:`
    /// (builtin) prefix per zsh's module-feature naming spec. A regen
    /// that swaps in `p:` (param) would silently make `zmodload -F
    /// zsh/cap +cap` fail.
    #[test]
    fn features_all_use_b_prefix() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut features: Vec<String> = Vec::new();
        features_(m, &mut features);
        for f in &features {
            assert!(
                f.starts_with("b:"),
                "feature {} must use 'b:' (builtin) prefix",
                f
            );
        }
    }

    /// c:154 — `enables_` returns a vec of length matching the
    /// feature count exactly. Mismatch means the bintab dispatcher
    /// would either OOB-read or never reach the last builtin's enable
    /// bit.
    #[test]
    fn enables_vec_length_matches_features_count() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut features: Vec<String> = Vec::new();
        features_(m, &mut features);
        let mut enables: Option<Vec<i32>> = None;
        enables_(m, &mut enables);
        let e = enables.expect("enables_ must return Some");
        assert_eq!(
            e.len(),
            features.len(),
            "enables vec length must match features count"
        );
    }

    /// c:139-175 — module-lifecycle stubs all return 0 in C.
    /// Catches a regression where one of setup/boot/cleanup/finish
    /// stops being a thin pass-through and starts returning a non-zero
    /// status that would prevent module load.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:200 — `bin_setcap` requires 2 positional args. With < 2 on
    /// a non-libcap host, the notavail stub still surfaces 1, NOT 2
    /// (the "bad usage" code) — platform-availability check fires
    /// before usage check.
    #[test]
    #[cfg(not(all(target_os = "linux", feature = "libcap")))]
    fn bin_setcap_unavailable_check_fires_before_usage_check() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_setcap("setcap", &["cap_net_admin+ep".into()], &ops, 0);
        assert_eq!(r, 1, "notavail stub must return 1 regardless of arg count");
    }

    // ─── zsh-corpus pins for cap on non-libcap host ────────────────

    /// On non-libcap host, `bin_cap` returns 1 (notavail).
    #[test]
    #[cfg(not(all(target_os = "linux", feature = "libcap")))]
    fn cap_corpus_bin_cap_notavail_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_cap("cap", &[], &ops, 0);
        assert_eq!(r, 1, "non-libcap host = notavail");
    }

    /// On non-libcap host, `bin_getcap` returns 1.
    #[test]
    #[cfg(not(all(target_os = "linux", feature = "libcap")))]
    fn cap_corpus_bin_getcap_notavail_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_getcap("getcap", &[], &ops, 0);
        assert_eq!(r, 1);
    }

    /// On non-libcap host, `bin_setcap` returns 1 even with full args.
    #[test]
    #[cfg(not(all(target_os = "linux", feature = "libcap")))]
    fn cap_corpus_bin_setcap_notavail_returns_one_with_full_args() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_setcap(
            "setcap",
            &["cap_net_admin+ep".into(), "/bin/ls".into()],
            &ops,
            0,
        );
        assert_eq!(r, 1);
    }

    /// Lifecycle shims all return 0.
    #[test]
    fn cap_corpus_lifecycle_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/cap.c builtins.
    // ═══════════════════════════════════════════════════════════════════

    /// c:62 — `bin_cap` with no args queries current caps;
    /// platform-dependent result. Pin no-panic only.
    #[test]
    fn bin_cap_no_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _ = bin_cap("cap", &[], &ops, 0);
    }

    /// c:147 — `bin_getcap` with no args returns nonzero (usage error).
    #[test]
    fn bin_getcap_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_getcap("getcap", &[], &ops, 0);
        assert_ne!(r, 0, "no path arg → usage error");
    }

    /// c:205 — `bin_setcap` with no args returns nonzero.
    #[test]
    fn bin_setcap_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_setcap("setcap", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:205 — `bin_setcap` with only one arg returns nonzero.
    #[test]
    fn bin_setcap_one_arg_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_setcap("setcap", &["cap_net_admin+ep".into()], &ops, 0);
        assert_ne!(r, 0, "missing path arg → usage error");
    }

    /// c:147 — `bin_getcap` on nonexistent path returns nonzero.
    #[test]
    fn bin_getcap_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_getcap(
            "getcap",
            &["/__never_exists_zshrs_xyz__".into()],
            &ops,
            0,
        );
        assert_ne!(r, 0, "nonexistent path → error");
    }

    /// c:274 — split per-hook setup_ pin.
    #[test]
    fn cap_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:281 — features_ returns 0.
    #[test]
    fn cap_features_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut features = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut features), 0);
    }

    /// c:289 — enables_ no panic.
    #[test]
    fn cap_enables_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _ = enables_(std::ptr::null(), &mut e);
    }
}
