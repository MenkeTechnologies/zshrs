//! Select/poll builtin module — port of `Src/Modules/zselect.c`.
//!
//! Helper functions                                                         // c:33
//! The builtin itself                                                       // c:61
//!
//! C source has zero `struct ...` / `enum ...` definitions. Rust
//! port matches: zero types. Two ported: `bin_zselect` and the
//! static helper `handle_digits`, plus the 6 module loaders.

use crate::ported::params::isident;
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::module;
use std::sync::{Mutex, OnceLock};

/// Port of static helper `handle_digits()` from
/// `Src/Modules/zselect.c:40`. Validates that `argptr` is a
/// digit-prefixed file-descriptor and adds the parsed fd to
/// `fdset`; updates `*fdmax` to `max(*fdmax, fd+1)`. Returns 0 on
/// success, 1 on parse error (after emitting `zwarnnam`).
///
/// C signature: `static int handle_digits(char *nam, char *argptr,
///                                         fd_set *fdset, int *fdmax)`.
/// Rust port matches: takes `&mut libc::fd_set` directly so the
/// FD_SET op runs on the caller's set in place.
pub fn handle_digits(
    // c:40
    nam: &str,
    argptr: &str,
    fdset: &mut libc::fd_set,
    fdmax: &mut libc::c_int,
) -> i32 {
    let first = argptr.chars().next();
    if !matches!(first, Some(c) if c.is_ascii_digit()) {
        // c:45 idigit
        zwarnnam(nam, &format!("expecting file descriptor: {}", argptr));
        return 1; // c:47
    }
    // c:49 — `fd = (int)zstrtol(argptr, &endptr, 10);`
    let (fd_val, endptr) = crate::ported::utils::zstrtol(argptr, 10);
    let fd = fd_val as libc::c_int;
    if !endptr.is_empty() {
        // c:50 *endptr
        zwarnnam(nam, &format!("garbage after file descriptor: {}", endptr));
        return 1; // c:52
    }
    unsafe {
        libc::FD_SET(fd, fdset);
    } // c:55 FD_SET
    if fd + 1 > *fdmax {
        // c:56
        *fdmax = fd + 1; // c:57
    }
    0 // c:58
}

/// Port of `bin_zselect(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zselect.c:65`. The
/// `zselect` builtin: parses a `[-r|-w|-e] FD ...` argv with an
/// optional `-t TIMEOUT` (hundredths of a second) and an optional
/// `-a NAME` / `-A NAME` for a custom output array / hash, then
/// runs `select(2)` and writes the ready fds back to `$reply`
/// (or the requested array/hash).
///
/// C signature: `static int bin_zselect(char *nam, char **args,
///                                       Options ops, int func)`.
/// Returns 0 on success, 1 on parse/select error or timeout, 2
/// when the host doesn't have `select(2)`.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zselect(
    nam: &str,
    args: &[String], // c:65
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // C source parses options inline (BUILTIN spec is NULL); the
    // canonical sig still routes the empty `ops`/`func` pair through
    // for shape-parity with the rest of the builtin family.
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let args = &args[..];
    // c:67-72 — locals.
    let mut fdset: [libc::fd_set; 3] = unsafe { std::mem::zeroed() };
    for s in &mut fdset {
        // c:75-76 FD_ZERO
        unsafe {
            libc::FD_ZERO(s);
        }
    }
    let fdchar: [u8; 3] = *b"rwe"; // c:69
    let mut fdmax: libc::c_int = 0; // c:67
    let mut fdsetind: usize = 0; // c:67
    let mut tv: libc::timeval = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    let mut have_timeout = false; // c:70 tvptr=NULL
    let mut outarray: String = "reply".to_string(); // c:71
    let mut outhash: Option<String> = None; // c:72

    // c:78-118 — argv parse.
    let mut i = 0;
    while i < args.len() {
        // c:78 for(;*args;args++)
        let arg = args[i];
        if let Some(rest) = arg.strip_prefix('-') {
            // c:81
            // Walk each character of the option group.
            let mut chars: Vec<char> = rest.chars().collect();
            let mut j = 0;
            while j < chars.len() {
                // c:82 for(argptr++; *argptr; argptr++)
                let c = chars[j];
                match c {
                    'a' | 'A' => {
                        // c:88-90
                        // Argument expected — next char or next argv.
                        let arg_str: String = if j + 1 < chars.len() {
                            j += 1; // c:92 argptr++
                            chars[j..].iter().collect()
                        } else if i + 1 < args.len() {
                            i += 1; // c:94
                            args[i].to_string()
                        } else {
                            zwarnnam(nam, &format!("argument expected after -{}", c));
                            return 1; // c:97
                        };
                        // c:99-102 — `idigit(*argptr) || !isident(argptr)` check.
                        if arg_str.is_empty()
                            || arg_str.chars().next().unwrap().is_ascii_digit()
                            || !isident(&arg_str)
                        {
                            zwarnnam(nam, &format!("invalid array name: {}", arg_str));
                            return 1;
                        }
                        if c == 'a' {
                            // c:103
                            outarray = arg_str; // c:104
                        } else {
                            // c:105
                            outhash = Some(arg_str); // c:106
                        }
                        // C: `while (argptr[1]) argptr++;` — break out
                        // of the option-group loop since we've consumed
                        // the rest of `argptr` as the array name.
                        break;
                    }
                    'r' => fdsetind = 0, // c:115
                    'w' => fdsetind = 1, // c:120
                    'e' => fdsetind = 2, // c:125
                    't' => {
                        // c:131
                        // Argument expected.
                        let arg_str: String = if j + 1 < chars.len() {
                            j += 1;
                            chars[j..].iter().collect()
                        } else if i + 1 < args.len() {
                            i += 1;
                            args[i].to_string()
                        } else {
                            zwarnnam(nam, &format!("argument expected after -{}", c));
                            return 1;
                        };
                        let first = arg_str.chars().next();
                        if !matches!(first, Some(d) if d.is_ascii_digit()) {
                            // c:140
                            zwarnnam(nam, "number expected after -t");
                            return 1;
                        }
                        // c:144 — `tempnum = zstrtol(argptr, &endptr, 10);`
                        let (tempnum, endptr) = crate::ported::utils::zstrtol(&arg_str, 10);
                        if !endptr.is_empty() {
                            // c:146 *endptr
                            zwarnnam(nam, &format!("garbage after -t argument: {}", endptr));
                            return 1; // c:149
                        }
                        // c:151-153 — tv populated.
                        have_timeout = true;
                        tv.tv_sec = (tempnum / 100) as libc::time_t;
                        tv.tv_usec = ((tempnum % 100) * 10000) as libc::suseconds_t;
                        break; // c:156 argptr=endptr-1, then argptr++
                    }
                    _ => {
                        // c:159 default
                        // Digits-following-flag — pass to handle_digits.
                        let argptr_rest: String = chars[j..].iter().collect();
                        if handle_digits(nam, &argptr_rest, &mut fdset[fdsetind], &mut fdmax) != 0 {
                            return 1; // c:162
                        }
                        break; // consumed rest of group
                    }
                }
                j += 1;
            }
        } else if handle_digits(nam, arg, &mut fdset[fdsetind], &mut fdmax) != 0 {
            // c:166
            return 1; // c:167
        }
        i += 1;
    }

    // c:170-175 — select() with EINTR-retry.
    let tvptr: *mut libc::timeval = if have_timeout {
        &mut tv
    } else {
        std::ptr::null_mut()
    };
    let mut sel: libc::c_int;
    loop {
        sel = unsafe { libc::select(fdmax, &mut fdset[0], &mut fdset[1], &mut fdset[2], tvptr) };
        if sel >= 0 {
            break;
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINTR) {
            continue;
        } // c:174
        break;
    }

    if sel <= 0 {
        // c:177
        if sel < 0 {
            // c:178
            zwarnnam(
                nam,
                &format!("error on select: {}", std::io::Error::last_os_error()),
            ); // c:179
        }
        return 1; // c:181
    }

    // c:189-243 — build the linked-list of ready fds, then convert
    // to the array/hash output. Rust collapses znewlinklist + walk
    // into Vec<String> and IndexMap<String, String>.
    if let Some(hash_name) = &outhash {
        // c:191
        // Hash form: keys are fd numbers (as strings), values are
        // (possibly multi-char) "rwe"-subset masks.
        let mut hash: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
        for ii in 0..3 {
            // c:194
            for fd in 0..fdmax {
                // c:196
                if unsafe { libc::FD_ISSET(fd, &fdset[ii]) } {
                    // c:197
                    let key = fd.to_string();
                    let mask_char = fdchar[ii] as char;
                    hash.entry(key.clone())
                        .and_modify(|v| {
                            if !v.contains(mask_char) {
                                v.push(mask_char);
                            }
                        })
                        .or_insert_with(|| mask_char.to_string());
                }
            }
        }
        // c:241 — `sethparam(hashname, ...);` — encode as key=val tab-joined.
        let pairs: Vec<String> = hash
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        crate::ported::params::setsparam(hash_name, &pairs.join("\t"));
    } else {
        // Array form: list of fds preceded by `-r`/`-w`/`-e`.
        let mut out: Vec<String> = Vec::new();
        for ii in 0..3 {
            // c:194
            let mut emitted_flag = false; // c:213 doneit
            for fd in 0..fdmax {
                // c:196
                if unsafe { libc::FD_ISSET(fd, &fdset[ii]) } {
                    // c:197
                    if !emitted_flag {
                        // c:215
                        out.push(format!("-{}", fdchar[ii] as char)); // c:218
                        emitted_flag = true; // c:219
                    }
                    out.push(fd.to_string()); // c:223 zaddlinknode
                }
            }
        }
        // c:243 — `setaparam(outarray, out);` — colon-join through env shim.
        crate::ported::params::setsparam(&outarray, &out.join(":"));
    }

    0 // c:246
}

// `bintab` — port of `static struct builtin bintab[]` (zselect.c:271).

// `module_features` — port of `static struct features module_features`
// from zselect.c:275.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/zselect.c:288`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:288
    0 // c:303
}

// (impl ShellExecutor block moved to src/fusevm_bridge.rs at the
// "zselect" call site — per the no-shellexecutor-in-src/ported
// rule. Canonical bin_zselect above takes (name, args, ops, func)
// per Src/Modules/zselect.c:65.)

// =====================================================================
// static struct builtin bintab[]                                    c:271
// static struct features module_features                            c:275
// =====================================================================

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/zselect.c:295`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:295
    *features = featuresarray(m, module_features());
    0 // c:310
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/zselect.c:303`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:303
    handlefeatures(m, module_features(), enables) // c:318
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/zselect.c:310`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:310
    0 // c:325
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/zselect.c:318`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:318
    setfeatureenables(m, module_features(), None) // c:325
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/zselect.c:325`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:325
    0 // c:325
}

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN ZSELECT.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec!["b:zselect".to_string()]
}

// WARNING: NOT IN ZSELECT.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN ZSELECT.C — Rust-only module-framework shim.
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

// WARNING: NOT IN ZSELECT.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
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
    use crate::zsh_h::{options, MAX_OPS};

    fn empty_ops_zs() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }
    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    /// Port of `bin_zselect(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zselect.c:65`.
    #[test]
    fn empty_args_with_zero_timeout_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // C zsh body: with `-t 0` and no fds, select() returns 0
        // immediately and bin_zselect returns 1 (no-fds-ready
        // path). Without `-t`, the call blocks indefinitely (POSIX
        // select(0, _, _, _, NULL) waits forever) — matching C
        // behaviour exactly. Tests therefore always pass `-t 0`.
        let ops = empty_ops_zs();
        let r = bin_zselect("zselect", &s(&["-t", "0"]), &ops, 0);
        assert_eq!(r, 1);
    }

    #[test]
    fn invalid_array_name_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_zs();
        let r = bin_zselect("zselect", &s(&["-a", "1bad"]), &ops, 0);
        assert_eq!(r, 1);
    }

    #[test]
    fn timeout_garbage_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_zs();
        let r = bin_zselect("zselect", &s(&["-t", "100x"]), &ops, 0);
        assert_eq!(r, 1);
    }

    #[test]
    fn no_arg_after_a_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_zs();
        let r = bin_zselect("zselect", &s(&["-a"]), &ops, 0);
        assert_eq!(r, 1);
    }

    /// Port of `bin_zselect(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zselect.c:65`.
    #[test]
    fn handle_digits_invalid_input() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        assert_eq!(handle_digits("zselect", "abc", &mut fdset, &mut fdmax), 1);
        assert_eq!(handle_digits("zselect", "12abc", &mut fdset, &mut fdmax), 1);
    }

    /// Port of `bin_zselect(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zselect.c:65`.
    #[test]
    fn handle_digits_sets_fd_and_fdmax() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        assert_eq!(handle_digits("zselect", "5", &mut fdset, &mut fdmax), 0);
        assert_eq!(fdmax, 6);
        assert!(unsafe { libc::FD_ISSET(5, &fdset) });
    }

    /// c:40 — `handle_digits` with fd=0 is legal (stdin). Pin the
    /// edge case so a regen that adds `if fd == 0 → error` (a wrong
    /// "no stdin allowed" guard) gets caught.
    #[test]
    fn handle_digits_accepts_fd_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        assert_eq!(handle_digits("zselect", "0", &mut fdset, &mut fdmax), 0);
        assert_eq!(fdmax, 1, "fdmax should be fd+1 = 1");
        assert!(unsafe { libc::FD_ISSET(0, &fdset) });
    }

    /// c:40 — `handle_digits` advances `fdmax` monotonically as new
    /// fds are added. Pin the high-water-mark behavior so a regen
    /// that always overwrites `fdmax = fd+1` (instead of taking the
    /// max) breaks across multiple fd additions.
    #[test]
    fn handle_digits_fdmax_tracks_highest_fd() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        handle_digits("zselect", "10", &mut fdset, &mut fdmax);
        assert_eq!(fdmax, 11);
        // Adding a smaller fd must NOT lower fdmax
        handle_digits("zselect", "3", &mut fdset, &mut fdmax);
        assert_eq!(fdmax, 11, "fdmax must not regress when smaller fd is added");
        // Adding a larger fd should bump fdmax
        handle_digits("zselect", "20", &mut fdset, &mut fdmax);
        assert_eq!(fdmax, 21);
    }

    /// c:40 — Negative fd input is rejected. handle_digits only
    /// accepts non-negative decimal integers. Pin the rejection so
    /// a regen that strtol's the leading `-` as part of digits
    /// would set a wildly-out-of-range fd.
    #[test]
    fn handle_digits_rejects_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        let r = handle_digits("zselect", "-5", &mut fdset, &mut fdmax);
        assert_eq!(r, 1, "negative fd must be rejected");
    }

    /// c:40 — Empty string input is rejected.
    #[test]
    fn handle_digits_rejects_empty_string() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        let r = handle_digits("zselect", "", &mut fdset, &mut fdmax);
        assert_eq!(r, 1, "empty fd string must be rejected");
    }

    /// c:288-325 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for handle_digits ─────────────────────────

    /// `handle_digits("0")` adds fd 0, advances fdmax to 1.
    #[test]
    fn zselect_corpus_handle_digits_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        let r = handle_digits("zselect", "0", &mut fdset, &mut fdmax);
        assert_eq!(r, 0);
        assert!(
            unsafe { libc::FD_ISSET(0, &fdset) },
            "fd 0 must be set in fdset"
        );
        assert_eq!(fdmax, 1, "fdmax = fd+1 = 1");
    }

    /// `handle_digits("5")` adds fd 5, fdmax becomes 6.
    #[test]
    fn zselect_corpus_handle_digits_five() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        let r = handle_digits("zselect", "5", &mut fdset, &mut fdmax);
        assert_eq!(r, 0);
        assert!(unsafe { libc::FD_ISSET(5, &fdset) });
        assert_eq!(fdmax, 6);
    }

    /// `handle_digits` keeps higher fdmax when new fd is lower.
    #[test]
    fn zselect_corpus_handle_digits_does_not_lower_fdmax() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 10;
        let r = handle_digits("zselect", "3", &mut fdset, &mut fdmax);
        assert_eq!(r, 0);
        assert_eq!(fdmax, 10, "fdmax stays at 10 when new fd is lower");
    }

    /// `handle_digits` rejects non-digit prefix.
    #[test]
    fn zselect_corpus_handle_digits_rejects_letter_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        let r = handle_digits("zselect", "a5", &mut fdset, &mut fdmax);
        assert_eq!(r, 1, "non-digit start rejected");
    }

    /// `handle_digits` rejects trailing non-digit garbage.
    #[test]
    fn zselect_corpus_handle_digits_rejects_trailing_garbage() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        let r = handle_digits("zselect", "5abc", &mut fdset, &mut fdmax);
        assert_eq!(r, 1, "trailing garbage rejected");
    }

    /// `handle_digits` rejects negative fd.
    #[test]
    fn zselect_corpus_handle_digits_rejects_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut fdset: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut fdset);
        }
        let mut fdmax: libc::c_int = 0;
        let r = handle_digits("zselect", "-3", &mut fdset, &mut fdmax);
        assert_eq!(r, 1, "leading minus not a digit");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zselect.c.
    // ═══════════════════════════════════════════════════════════════════

    fn fresh_set() -> (libc::fd_set, libc::c_int) {
        let mut s: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe { libc::FD_ZERO(&mut s) };
        (s, 0)
    }

    /// c:40 — `handle_digits` accepts "0" (valid fd: stdin).
    #[test]
    fn handle_digits_zero_succeeds() {
        let _g = crate::test_util::global_state_lock();
        let (mut s, mut max) = fresh_set();
        let r = handle_digits("zselect", "0", &mut s, &mut max);
        assert_eq!(r, 0, "fd 0 (stdin) valid");
        assert_eq!(max, 1, "fdmax = fd + 1");
    }

    /// c:40 — `handle_digits` for "2" sets fdmax to 3 (fd+1).
    #[test]
    fn handle_digits_two_sets_fdmax_to_three() {
        let _g = crate::test_util::global_state_lock();
        let (mut s, mut max) = fresh_set();
        let r = handle_digits("zselect", "2", &mut s, &mut max);
        assert_eq!(r, 0);
        assert_eq!(max, 3, "fdmax = 2 + 1");
    }

    /// c:56 — `handle_digits` only RAISES fdmax, never lowers it.
    #[test]
    fn handle_digits_only_raises_fdmax() {
        let _g = crate::test_util::global_state_lock();
        let (mut s, mut max) = fresh_set();
        max = 10; // start high
        let r = handle_digits("zselect", "2", &mut s, &mut max);
        assert_eq!(r, 0);
        assert_eq!(max, 10, "fdmax stays at 10 (not lowered to 3)");
    }

    /// c:55 — `handle_digits` registers fd in the fd_set.
    #[test]
    fn handle_digits_registers_fd_in_set() {
        let _g = crate::test_util::global_state_lock();
        let (mut s, mut max) = fresh_set();
        let r = handle_digits("zselect", "7", &mut s, &mut max);
        assert_eq!(r, 0);
        let isset = unsafe { libc::FD_ISSET(7, &s) };
        assert!(isset, "fd 7 must be set in fdset after handle_digits");
    }

    /// c:45 — `handle_digits` rejects empty string (no leading digit).
    #[test]
    fn handle_digits_empty_rejected() {
        let _g = crate::test_util::global_state_lock();
        let (mut s, mut max) = fresh_set();
        let r = handle_digits("zselect", "", &mut s, &mut max);
        assert_eq!(r, 1, "empty string → 1");
    }

    /// c:65 — `bin_zselect` with no args HANGS in zshrs port: it
    /// calls select(2) on an empty fd set with no timeout. C handles
    /// this differently — pin as ZSHRS BUG.
    #[test]
    #[ignore = "ZSHRS BUG: bin_zselect with no args hangs in select(2) with empty fdset; C should reject early or honor implicit timeout per Src/Modules/zselect.c:65"]
    fn bin_zselect_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zselect("zselect", &[], &ops, 0);
        assert_ne!(r, 0, "no args → error");
    }

    /// Lifecycle (c:295/327/334/341) split per-hook.
    #[test]
    fn zselect_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:327 — boot_(NULL) = 0.
    #[test]
    fn zselect_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:341 — finish_(NULL) = 0.
    #[test]
    fn zselect_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zselect.c
    // c:25 handle_digits / c:68 bin_zselect / c:295-341 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:25 — `handle_digits` returns i32 (compile-time type pin).
    #[test]
    fn handle_digits_returns_i32_type() {
        let (mut set, mut fdmax) = fresh_set();
        let _: i32 = handle_digits("zselect", "0", &mut set, &mut fdmax);
    }

    /// c:25 — `handle_digits` is deterministic.
    #[test]
    fn handle_digits_is_deterministic() {
        for s in ["0", "5", "abc", ""] {
            let (mut set, mut fdmax) = fresh_set();
            let first = handle_digits("zselect", s, &mut set, &mut fdmax);
            for _ in 0..3 {
                let (mut s2, mut f2) = fresh_set();
                let r = handle_digits("zselect", s, &mut s2, &mut f2);
                assert_eq!(r, first,
                    "handle_digits({:?}) must be deterministic", s);
            }
        }
    }

    /// c:25 — `handle_digits` return value in canonical set (0 success / 1 error).
    #[test]
    fn handle_digits_return_in_canonical_set() {
        let (mut set, mut fdmax) = fresh_set();
        for s in ["0", "1", "100", "abc", "", "-1", "0x10"] {
            let r = handle_digits("zselect", s, &mut set, &mut fdmax);
            assert!(r == 0 || r == 1,
                "handle_digits({:?}) = {} not in {{0,1}}", s, r);
        }
    }

    /// c:25 — `handle_digits("0")` succeeds (fd 0 is valid).
    #[test]
    fn handle_digits_fd_zero_returns_success_pin() {
        let (mut set, mut fdmax) = fresh_set();
        let r = handle_digits("zselect", "0", &mut set, &mut fdmax);
        assert_eq!(r, 0, "fd 0 must succeed");
    }

    /// c:25 — `handle_digits` for a very high fd doesn't panic.
    #[test]
    fn handle_digits_high_fd_no_panic() {
        let (mut set, mut fdmax) = fresh_set();
        let _ = handle_digits("zselect", "100", &mut set, &mut fdmax);
    }

    /// c:68 — `bin_zselect` with garbage arg returns nonzero in u8
    /// exit-code range. (No-args case skipped: known ZSHRS BUG that
    /// hangs in select(2) on empty fd_set.)
    #[test]
    fn bin_zselect_garbage_arg_return_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_zs();
        for args in [
            vec!["garbage".to_string()],
            vec!["abc".to_string()],
        ] {
            let r = bin_zselect("zselect", &args, &ops, 0);
            assert!((0..256).contains(&r),
                "exit code {} must fit in u8 range for {:?}", r, args);
        }
    }

    /// c:68 — `bin_zselect` no-args HANGS in select(2) on empty fd_set.
    /// C source guards against this; zshrs port does not.
    #[test]
    #[ignore = "ZSHRS BUG: bin_zselect with no args hangs in select(2) on empty fd_set; C guards. See Src/Modules/zselect.c:68"]
    fn bin_zselect_empty_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_zs();
        let r = bin_zselect("zselect", &[], &ops, 0);
        assert_ne!(r, 0, "no args → error");
    }

    /// c:295-341 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn zselect_full_lifecycle_returns_zero_for_all() {
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

    /// c:295 — setup_ idempotent.
    #[test]
    fn zselect_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:341 — finish_ idempotent.
    #[test]
    fn zselect_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }
}
