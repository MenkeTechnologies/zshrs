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
use crate::ported::zsh_h::{module, options};

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
        pub fn cap_get_proc() -> CapT;
        pub fn cap_set_proc(cap_p: CapT) -> c_int;
        pub fn cap_get_file(path: *const c_char) -> CapT;
        pub fn cap_set_file(path: *const c_char, cap_p: CapT) -> c_int;
        pub fn cap_from_text(buf: *const c_char) -> CapT;
        pub fn cap_to_text(caps: CapT, length: *mut ssize_t) -> *mut c_char;
        pub fn cap_free(obj: *mut c_void) -> c_int;
    }
}

// =====================================================================
// Port of `bin_cap()` from Src/Modules/cap.c:36.
// =====================================================================

/// Port of `bin_cap()` from `Src/Modules/cap.c:36`.
///
/// `cap [STRING]`: with `STRING`, parse via `cap_from_text` and
/// install via `cap_set_proc`; without args, print the current
/// process's capability set.
#[cfg(all(target_os = "linux", feature = "libcap"))]
pub(crate) fn bin_cap(nam: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    use std::ffi::{CStr, CString};

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
                    &format!("can't change capabilities: {}", std::io::Error::last_os_error()),
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
                    &format!("can't get capabilities: {}", std::io::Error::last_os_error()),
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
// Port of `bin_getcap()` from Src/Modules/cap.c:68.
// =====================================================================

/// Port of `bin_getcap()` from `Src/Modules/cap.c:68`.
///
/// `getcap FILE...`: print each file's capability set as
/// `FILE CAPS`. C bails on the first error but iterates the rest;
/// the Rust port mirrors that exact loop.
#[cfg(all(target_os = "linux", feature = "libcap"))]
pub(crate) fn bin_getcap(nam: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    use std::ffi::{CStr, CString};

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
                zwarnnam(nam, &format!("{}: {}", file, std::io::Error::last_os_error()));
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
// Port of `bin_setcap()` from Src/Modules/cap.c:91.
// =====================================================================

/// Port of `bin_setcap()` from `Src/Modules/cap.c:91`.
///
/// `setcap STRING FILE...`: parse `STRING` via `cap_from_text`, then
/// apply via `cap_set_file` to each remaining file argument. Mirrors
/// C's loop: free `caps` once at end, advance `argv` per iteration.
#[cfg(all(target_os = "linux", feature = "libcap"))]
pub(crate) fn bin_setcap(nam: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    use std::ffi::CString;

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
                zwarnnam(nam, &format!("{}: {}", file, std::io::Error::last_os_error()));
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

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::features as features_t;

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,                                                // c:130 bintab[3]
        bn_size: 3,
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
// Module entry points (cap.c:138-178).
// =====================================================================

/// Port of `setup_()` from `Src/Modules/cap.c:139`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:139
    0                                                                  // c:141
}

/// Port of `features_()` from `Src/Modules/cap.c:146`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {      // c:146
    *features = featuresarray(m, module_features());                  // c:148
    0                                                                  // c:149
}

/// Port of `enables_()` from `Src/Modules/cap.c:154`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {   // c:154
    handlefeatures(m, module_features(), enables)                     // c:156
}

/// Port of `boot_()` from `Src/Modules/cap.c:161`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:161
    0                                                                  // c:163
}

/// Port of `cleanup_()` from `Src/Modules/cap.c:168`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                                   // c:168
    setfeatureenables(m, module_features(), None)                     // c:170
}

/// Port of `finish_()` from `Src/Modules/cap.c:175`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:175
    0                                                                  // c:177
}

// `featuresarray` — Src/module.c:3275.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:cap".to_string(), "b:getcap".to_string(), "b:setcap".to_string()]
}

// `handlefeatures` — Src/module.c:3370.
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

// `setfeatureenables` — Src/module.c:3445.
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

// =====================================================================
// ShellExecutor bridge — sanctioned PORT.md exception. Wires the
// internal builtin dispatcher to the canonical free fns above.
// =====================================================================

// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)


// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ops() -> options {
        options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }
    }

    #[test]
    fn test_features_returns_bintab_names() {
        let m: *const module = std::ptr::null();
        let mut features: Vec<String> = Vec::new();
        let rc = features_(m, &mut features);
        assert_eq!(rc, 0);
        assert_eq!(features, vec!["b:cap", "b:getcap", "b:setcap"]);
    }

    #[test]
    fn test_enables_get_then_set() {
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
        let m: *const module = std::ptr::null();
        assert_eq!(cleanup_(m), 0);
    }

    #[test]
    #[cfg(not(all(target_os = "linux", feature = "libcap")))]
    fn test_bin_cap_unsupported_on_macos() {
        let ops = empty_ops();
        // Without libcap, all three bin_* return 1 (notavail).
        assert_eq!(bin_cap("cap", &[], &ops, 0), 1);
        assert_eq!(bin_getcap("getcap", &["/etc/passwd".into()], &ops, 0), 1);
        assert_eq!(
            bin_setcap("setcap", &["cap_net_admin+ep".into(), "/tmp/x".into()], &ops, 0),
            1
        );
    }
}
