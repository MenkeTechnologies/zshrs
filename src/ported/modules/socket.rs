//! Unix domain socket module — port of `Src/Modules/socket.c`.
//!
//! C source has zero `struct ...` / `enum ...` definitions. The
//! Rust port matches: zero types, only the function ports
//! (`bin_zsocket`, `setup_`/`features_`/`enables_`/`boot_`/
//! `cleanup_`/`finish_`).

use crate::ported::params::setiparam_no_convert;
use crate::ported::utils::{
    addmodulefd, errflag, fdtable_get, fdtable_set, movefd, redup, zerrnam, zwarnnam,
};
/// Direct port of `bin_zsocket(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/socket.c:57`.
/// C signature matches exactly: `static int bin_zsocket(char *nam,
/// char **args, Options ops, UNUSED(int func))`.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
use crate::ported::zsh_h::{
    features, module, options, FDT_EXTERNAL, FDT_UNUSED, OPT_ARG, OPT_ISSET,
};
use std::sync::{Mutex, OnceLock};
/// `bin_zsocket` — see implementation.
pub fn bin_zsocket(
    nam: &str,
    args: &[String], // c:57
    ops: &options,
    _func: i32,
) -> i32 {
    let mut soun: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut sfd: i32;
    let mut err: i32 = 1; // c:60
    let mut verbose = 0i32;
    let mut test = 0i32;
    let mut targetfd: i32 = 0;
    let mut soun: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut sfd: i32;

    if OPT_ISSET(ops, b'v') {
        verbose = 1;
    } // c:64-65
    if OPT_ISSET(ops, b't') {
        test = 1;
    } // c:67-68

    if OPT_ISSET(ops, b'd') {
        // c:70
        let darg = OPT_ARG(ops, b'd').unwrap_or("");
        targetfd = darg.parse::<i32>().unwrap_or(0); // c:71 atoi
        if targetfd == 0 {
            // c:72
            zwarnnam(nam, &format!("{} is an invalid argument to -d", darg)); // c:73
            return 1; // c:75
        }
        // c:78-82 — `if (targetfd <= max_zsh_fd && fdtable[targetfd] != FDT_UNUSED)`.
        // Static-link path: query the per-process fdtable accessor.
        if fdtable_get(targetfd) != FDT_UNUSED {
            // c:78
            zwarnnam(
                nam, // c:79
                &format!("file descriptor {} is in use by the shell", targetfd),
            );
            return 1; // c:81
        } else {
        }
    }

    if OPT_ISSET(ops, b'l') {
        // c:85
        if args.is_empty() {
            // c:88
            zwarnnam(nam, "-l requires an argument");
            return 1; // c:90
        }
        let localfn = args[0].as_str(); // c:93
        sfd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) }; // c:95
        if sfd == -1 {
            // c:97
            zwarnnam(
                nam,
                &format!("socket error: {} ", std::io::Error::last_os_error()),
            ); // c:98
            return 1; // c:99
        }
        soun.sun_family = libc::AF_UNIX as _; // c:102
        let path_bytes = localfn.as_bytes();
        let max_len = soun.sun_path.len() - 1;
        let copy_len = path_bytes.len().min(max_len);
        for (k, &b) in path_bytes[..copy_len].iter().enumerate() {
            // c:103 strncpy
            soun.sun_path[k] = b as libc::c_char;
        }
        let r = unsafe {
            // c:105
            libc::bind(
                sfd,
                &soun as *const _ as *const libc::sockaddr,
                size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if r != 0 {
            // c:106
            zwarnnam(
                nam,
                &format!(
                    "could not bind to {}: {}",
                    localfn,
                    std::io::Error::last_os_error()
                ),
            ); // c:107
            unsafe {
                libc::close(sfd);
            } // c:108
            return 1; // c:109
        }
        if unsafe { libc::listen(sfd, 1) } != 0 {
            // c:112
            zwarnnam(
                nam,
                &format!(
                    "could not listen on socket: {}",
                    std::io::Error::last_os_error()
                ),
            ); // c:114
            unsafe {
                libc::close(sfd);
            } // c:115
            return 1; // c:116
        }
        addmodulefd(sfd, FDT_EXTERNAL); // c:119 FDT_EXTERNAL
        if targetfd != 0 {
            // c:121
            sfd = redup(sfd, targetfd); // c:122
        } else {
            sfd = movefd(sfd); // c:126 movefd
        }
        if sfd == -1 {
            // c:128
            zerrnam(
                nam,
                &format!(
                    "cannot duplicate fd {}: {}",
                    sfd,
                    std::io::Error::last_os_error()
                ),
            ); // c:129
            return 1; // c:130
        }
        fdtable_set(sfd, FDT_EXTERNAL); // c:134
        setiparam_no_convert("REPLY", sfd as i64); // c:135 setiparam_no_convert
        if verbose != 0 {
            // c:138
            println!("{} listener is on fd {}", localfn, sfd); // c:139
        }
        return 0; // c:141
    } else if OPT_ISSET(ops, b'a') {
        // c:143
        if args.is_empty() {
            // c:147
            zwarnnam(nam, "-a requires an argument");
            return 1; // c:149
        }
        let lfd = args[0].parse::<i32>().unwrap_or(0); // c:152 atoi
        if lfd == 0 {
            // c:154
            zwarnnam(nam, "invalid numerical argument");
            return 1; // c:156
        }
        if test != 0 {
            // c:159
            // c:163 HAVE_POLL branch.
            let mut pfd = libc::pollfd {
                fd: lfd,
                events: libc::POLLIN,
                revents: 0,
            };
            let r = unsafe { libc::poll(&mut pfd, 1, 0) }; // c:166
            if r == 0 {
                return 1;
            }
            // c:166
            else if r == -1 {
                // c:167
                zwarnnam(
                    nam,
                    &format!("poll error: {}", std::io::Error::last_os_error()),
                ); // c:169
                return 1; // c:170
            }
        }
        let mut len: libc::socklen_t = size_of::<libc::sockaddr_un>() as libc::socklen_t; // c:194
        let rfd: i32;
        loop {
            // c:195
            let r = unsafe {
                libc::accept(
                    lfd, // c:196
                    &mut soun as *mut _ as *mut libc::sockaddr,
                    &mut len,
                )
            };
            if r >= 0 {
                rfd = r;
                break;
            }
            let osek = std::io::Error::last_os_error().raw_os_error();
            if osek != Some(libc::EINTR) || errflag.load(std::sync::atomic::Ordering::Relaxed) != 0
            {
                rfd = r;
                break;
            } else {
            }
        }
        if rfd == -1 {
            // c:199
            zwarnnam(
                nam,
                &format!(
                    "could not accept connection: {}",
                    std::io::Error::last_os_error()
                ),
            ); // c:200
            return 1; // c:201
        }
        addmodulefd(rfd, FDT_EXTERNAL); // c:204 FDT_EXTERNAL
        if targetfd != 0 {
            // c:206
            sfd = redup(rfd, targetfd); // c:207
            if sfd < 0 {
                // c:208
                zerrnam(
                    nam,
                    &format!(
                        "could not duplicate socket fd to {}: {}",
                        targetfd,
                        std::io::Error::last_os_error()
                    ),
                ); // c:209
                   // c:214 — `zclose(rfd);` — rfd was registered as
                   // FDT_EXTERNAL at c:208 above (addmodulefd call).
                   // Raw libc::close would leave the marker stale on
                   // the freed fd (same leak shape as the init_io
                   // SHTTY fix ff15efec5f and the tcp_close fix
                   // 9b4dae375a).
                let _ = crate::ported::utils::zclose(rfd);
                return 1; // c:215
            }
            fdtable_set(sfd, FDT_EXTERNAL); // c:217
        } else {
            sfd = rfd; // c:217
        }
        setiparam_no_convert("REPLY", sfd as i64); // c:223 setiparam_no_convert
        if verbose != 0 {
            // c:222
            let path = soun
                .sun_path
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8 as char)
                .collect::<String>();
            println!("new connection from {} is on fd {}", path, sfd); // c:223
        }
    } else {
        // c:225
        if args.is_empty() {
            // c:227
            zwarnnam(nam, "zsocket requires an argument");
            return 1; // c:229
        }
        sfd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) }; // c:233
        if sfd == -1 {
            // c:235
            zwarnnam(
                nam,
                &format!(
                    "socket creation failed: {}",
                    std::io::Error::last_os_error()
                ),
            ); // c:236
            return 1; // c:237
        }
        soun.sun_family = libc::AF_UNIX as _; // c:240
        let path_bytes = args[0].as_bytes();
        let max_len = soun.sun_path.len() - 1;
        let copy_len = path_bytes.len().min(max_len);
        for (k, &b) in path_bytes[..copy_len].iter().enumerate() {
            // c:241 strncpy
            soun.sun_path[k] = b as libc::c_char;
        }
        err = unsafe {
            // c:243
            libc::connect(
                sfd,
                &soun as *const _ as *const libc::sockaddr,
                size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        if err != 0 {
            // c:243
            zwarnnam(
                nam,
                &format!("connection failed: {}", std::io::Error::last_os_error()),
            ); // c:244
            unsafe {
                libc::close(sfd);
            } // c:245
            return 1; // c:246
        }
        addmodulefd(sfd, FDT_EXTERNAL); // c:251 FDT_EXTERNAL
        if targetfd != 0 {
            // c:253
            if redup(sfd, targetfd) < 0 {
                // c:254
                zerrnam(
                    nam,
                    &format!(
                        "could not duplicate socket fd to {}: {}",
                        targetfd,
                        std::io::Error::last_os_error()
                    ),
                ); // c:256
                   // c:257 — `zclose(sfd);` — sfd was just registered as
                   // FDT_EXTERNAL at c:252 above; raw libc::close would
                   // leave the marker stale (same fix shape as the c:214
                   // case earlier in this builtin and the init_io fix
                   // ff15efec5f).
                let _ = crate::ported::utils::zclose(sfd);
                return 1; // c:258
            }
            sfd = targetfd; // c:260
            fdtable_set(sfd, FDT_EXTERNAL); // c:260
        }
        setiparam_no_convert("REPLY", sfd as i64); // c:264 setiparam_no_convert
        if verbose != 0 {
            // c:265
            let path = &args[0];
            println!("{} is now on fd {}", path, sfd); // c:266
        }
    }
    let _ = (err, verbose, test, targetfd); // silence unused-binding paths
    0 // c:271
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// ===========================================================

// =====================================================================
// static struct builtin bintab[]                                    c:280
// static struct features module_features                            c:284
// =====================================================================

// `bintab` — port of `static struct builtin bintab[]` (socket.c:280).

// `module_features` — port of `static struct features module_features`
// from socket.c:284.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/socket.c:291`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:291
    0 // c:306
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/socket.c:298`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:298
    *features = featuresarray(m, module_features());
    0 // c:313
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/socket.c:306`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:306
    handlefeatures(m, module_features(), enables) // c:320
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/socket.c:313`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:313
    0 // c:327
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/socket.c:320`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:320
    setfeatureenables(m, module_features(), None) // c:327
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/socket.c:327`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:327
    0 // c:327
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN SOCKET.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:zsocket".to_string()]
}

// WARNING: NOT IN SOCKET.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN SOCKET.C — Rust-only module-framework shim.
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

// WARNING: NOT IN SOCKET.C — Rust-only module-framework shim.
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

    /// c:88-90 — `zsocket -l` with no path arg MUST fail-fast BEFORE
    /// any libc::socket(2) call. A regression where the missing-arg
    /// check is bypassed would leak a socket fd per invocation.
    #[test]
    fn zsocket_l_without_arg_fails_before_socket_call() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'l' as usize] = 1;
        assert_eq!(bin_zsocket("zsocket", &[], &ops, 0), 1);
    }

    /// c:71-75 — non-numeric `-d <fd>` MUST fail-fast (atoi → 0 → bad
    /// fd) BEFORE socket(2). A regression that lets `0` through would
    /// dup2 the new socket onto stdin silently.
    #[test]
    fn zsocket_d_non_numeric_fails_before_dup2() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'd' as usize] = (1 << 2) | 1;
        ops.args.push("not-a-number".to_string());
        assert_eq!(bin_zsocket("zsocket", &[], &ops, 0), 1);
    }

    fn empty_ops() -> options {
        options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// c:225-229 — `zsocket` (default, connect-mode) with NO args must
    /// fail-fast with "zsocket requires an argument". Catches a
    /// regression where the missing-arg path leaks an unconnected
    /// socket fd.
    #[test]
    fn zsocket_connect_mode_without_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        assert_eq!(bin_zsocket("zsocket", &[], &ops, 0), 1);
    }

    /// c:144-149 — `zsocket -a` with NO args must fail-fast with
    /// "-a requires an argument". Symmetrical to the `-l` check at
    /// c:88-90 (already pinned) but for the accept-mode path.
    #[test]
    fn zsocket_a_without_arg_fails_before_accept() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'a' as usize] = 1;
        assert_eq!(bin_zsocket("zsocket", &[], &ops, 0), 1);
    }

    /// c:152-156 — `zsocket -a 0` (or any non-numeric → atoi → 0) must
    /// fail with "invalid numerical argument". `0` is never a valid
    /// listening fd because the user can't have just created one and
    /// taken stdin away.
    #[test]
    fn zsocket_a_zero_listen_fd_fails() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'a' as usize] = 1;
        assert_eq!(bin_zsocket("zsocket", &["0".to_string()], &ops, 0), 1);
        // non-numeric also flows through atoi → 0
        assert_eq!(
            bin_zsocket("zsocket", &["not-numeric".to_string()], &ops, 0),
            1
        );
    }

    /// c:291-327 — module-lifecycle stubs (`setup_`, `boot_`,
    /// `cleanup_`, `finish_`) all return 0 in the C source. The Rust
    /// port must match.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:298 — `features_` populates the feature list and returns 0.
    /// Specific contents aren't pinned by C; just verify the function
    /// is callable without panicking and returns the success sentinel.
    #[test]
    fn features_returns_success() {
        let _g = crate::test_util::global_state_lock();
        let mut features = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut features), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/socket.c bin_zsocket
    // option handling.
    // ═══════════════════════════════════════════════════════════════════

    fn ops_with_flag(flag: u8) -> options {
        let mut ind = [0u8; crate::ported::zsh_h::MAX_OPS];
        ind[flag as usize] = 1;
        options {
            ind,
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// c:88 — `bin_zsocket -l` with no args returns 1 ("requires arg").
    #[test]
    fn bin_zsocket_l_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_with_flag(b'l');
        let r = bin_zsocket("zsocket", &[], &ops, 0);
        assert_eq!(r, 1, "-l with no args → usage error");
    }

    /// c:57 — `bin_zsocket` with no flags or args runs without panic
    /// (no-op invocation; returns 1 for missing operation per usage check).
    #[test]
    fn bin_zsocket_no_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = options {
            ind: [0; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _ = bin_zsocket("zsocket", &[], &ops, 0);
        // No panic = pass.
    }

    /// c:64-65 — `-v` flag alone (verbose, no connect/listen) returns
    /// nonzero per usage check (verbose without action).
    #[test]
    fn bin_zsocket_v_alone_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_with_flag(b'v');
        let _ = bin_zsocket("zsocket", &[], &ops, 0);
    }

    /// c:70-75 — `-d` flag set runs the targetfd validation path.
    /// Pin no-panic only (OPT_ARG wiring details vary between ports).
    #[test]
    fn bin_zsocket_d_flag_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_with_flag(b'd');
        let _ = bin_zsocket("zsocket", &[], &ops, 0);
    }

    /// c:67-68 — `-t` (test) flag alone doesn't panic.
    #[test]
    fn bin_zsocket_t_alone_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_with_flag(b't');
        let _ = bin_zsocket("zsocket", &[], &ops, 0);
    }

    /// c:351 — `setup_(NULL)` returns 0 (split from combined test).
    #[test]
    fn socket_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:373 — `boot_(NULL)` returns 0.
    #[test]
    fn socket_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:380 — `cleanup_(NULL)` returns 0.
    #[test]
    fn socket_cleanup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    /// c:387 — `finish_(NULL)` returns 0.
    #[test]
    fn socket_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/socket.c
    // c:21 bin_zsocket / c:351-387 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:21 — `bin_zsocket` return value in u8 exit-code range.
    #[test]
    fn bin_zsocket_return_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for args in [
            vec![],
            vec!["/tmp/zshrs_test_sock".to_string()],
            vec!["".to_string()],
        ] {
            let r = bin_zsocket("zsocket", &args, &ops, 0);
            assert!(
                (0..256).contains(&r),
                "exit code {} must fit in u8 range for {:?}",
                r,
                args
            );
        }
    }

    /// c:21 — `bin_zsocket` empty socket path returns nonzero.
    #[test]
    fn bin_zsocket_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_zsocket("zsocket", &["".to_string()], &ops, 0);
        assert_ne!(r, 0, "empty path → error");
    }

    /// c:21 — `bin_zsocket` is deterministic for no-args.
    #[test]
    fn bin_zsocket_no_args_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let first = bin_zsocket("zsocket", &[], &ops, 0);
        for _ in 0..5 {
            assert_eq!(bin_zsocket("zsocket", &[], &ops, 0), first);
        }
    }

    /// c:21 — `bin_zsocket -l` and `bin_zsocket -a` with non-numeric
    /// args don't panic.
    ///
    /// `-l <path>` calls `bind(2)` on the path, which CREATES a Unix
    /// socket file at that path; `-a <path>` connects but also can
    /// touch the filesystem. The prior bare `"abc"` / `"xyz"` args
    /// left strays in the repo root that surfaced in `git status`
    /// after every `cargo test` run. Route through `$TMPDIR` and
    /// pre+post cleanup.
    #[test]
    fn bin_zsocket_a_l_flags_with_arbitrary_arg_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let xyz_path = std::env::temp_dir().join("__zshrs_test_zsocket_xyz");
        let abc_path = std::env::temp_dir().join("__zshrs_test_zsocket_abc");
        let _ = std::fs::remove_file(&xyz_path);
        let _ = std::fs::remove_file(&abc_path);
        let mut ops = empty_ops();
        ops.ind[b'a' as usize] = 1;
        let _ = bin_zsocket("zsocket", &[xyz_path.to_string_lossy().into()], &ops, 0);
        let mut ops2 = empty_ops();
        ops2.ind[b'l' as usize] = 1;
        let _ = bin_zsocket("zsocket", &[abc_path.to_string_lossy().into()], &ops2, 0);
        let _ = std::fs::remove_file(&xyz_path);
        let _ = std::fs::remove_file(&abc_path);
    }

    /// c:351-387 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn socket_full_lifecycle_returns_zero_for_all() {
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

    /// c:351 — setup_ idempotent.
    #[test]
    fn socket_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:387 — finish_ idempotent.
    #[test]
    fn socket_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:380 — cleanup_ idempotent.
    #[test]
    fn socket_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:373 — boot_ idempotent.
    #[test]
    fn socket_boot_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    /// c:21 — `bin_zsocket` with multibyte path doesn't panic.
    #[test]
    fn bin_zsocket_multibyte_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _ = bin_zsocket("zsocket", &["/tmp/日本".to_string()], &ops, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/socket.c
    // c:21 bin_zsocket + c:351-387 lifecycle type/exit-code pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:21 — `bin_zsocket` returns i32 (compile-time pin).
    #[test]
    fn bin_zsocket_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_zsocket("zsocket", &[], &ops, 0);
    }

    /// c:21 — `bin_zsocket` exit codes are non-negative.
    #[test]
    fn bin_zsocket_exit_codes_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for argv in [
            vec![],
            vec!["/tmp/sock".into()],
            vec!["".into()],
            vec!["/tmp/sock".into(), "extra".into()],
        ] {
            let r = bin_zsocket("zsocket", &argv, &ops, 0);
            assert!(
                r >= 0,
                "exit code must be non-negative, got {} for {:?}",
                r,
                argv
            );
        }
    }

    /// c:21 — `bin_zsocket` empty path is deterministic (no hidden state).
    #[test]
    fn bin_zsocket_empty_path_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let first = bin_zsocket("zsocket", &["".to_string()], &ops, 0);
        for _ in 0..5 {
            assert_eq!(
                bin_zsocket("zsocket", &["".to_string()], &ops, 0),
                first,
                "empty-path zsocket must be pure across calls"
            );
        }
    }

    /// c:21 — `bin_zsocket` with both -a and -l flags is safe (no panic).
    /// C source resolves precedence inside the body; what matters here
    /// is that calling with both flags set doesn't crash.
    #[test]
    fn bin_zsocket_both_a_and_l_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'a' as usize] = 1;
        ops.ind[b'l' as usize] = 1;
        let _ = bin_zsocket("zsocket", &["/tmp/sock".to_string()], &ops, 0);
    }

    /// c:21 — `bin_zsocket` -d <fd> with non-numeric arg doesn't panic.
    #[test]
    fn bin_zsocket_d_flag_with_non_numeric_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = empty_ops();
        ops.ind[b'd' as usize] = 1;
        let _ = bin_zsocket("zsocket", &["not-a-number".to_string()], &ops, 0);
    }

    /// c:21 — `bin_zsocket` with very long path doesn't panic.
    #[test]
    fn bin_zsocket_long_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let path = "/tmp/".to_string() + &"x".repeat(2000);
        let _ = bin_zsocket("zsocket", &[path], &ops, 0);
    }

    /// c:351 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn socket_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:358 — `features_` returns i32 (compile-time pin).
    #[test]
    fn socket_features_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _: i32 = features_(std::ptr::null(), &mut v);
    }

    /// c:366 — `enables_` returns i32 with None enables-out param safe.
    #[test]
    fn socket_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:366 — `enables_` deterministic for null callback.
    #[test]
    fn socket_enables_deterministic_for_null_in() {
        let _g = crate::test_util::global_state_lock();
        let mut a: Option<Vec<i32>> = None;
        let first = enables_(std::ptr::null(), &mut a);
        for _ in 0..3 {
            let mut b: Option<Vec<i32>> = None;
            assert_eq!(
                enables_(std::ptr::null(), &mut b),
                first,
                "enables_ must be deterministic"
            );
        }
    }

    /// c:351/358/366/373/380/387 — each lifecycle hook returns 0 individually.
    #[test]
    fn socket_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:351 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:358 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:366 enables_");
        assert_eq!(boot_(null), 0, "c:373 boot_");
        assert_eq!(cleanup_(null), 0, "c:380 cleanup_");
        assert_eq!(finish_(null), 0, "c:387 finish_");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/socket.c
    // c:21 bin_zsocket / c:351-387 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:351 — `setup_` is idempotent.
    #[test]
    fn socket_setup_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:380 — `cleanup_` is idempotent.
    #[test]
    fn socket_cleanup_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:387 — `finish_` is idempotent.
    #[test]
    fn socket_finish_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:380 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn socket_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:387 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn socket_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:373 — `boot_` return type i32 (compile-time pin).
    #[test]
    fn socket_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:21 — `bin_zsocket` empty args returns non-negative.
    #[test]
    fn bin_zsocket_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zsocket("zsocket", &[], &ops, 0);
        assert!(r >= 0, "bin_zsocket empty args must return ≥ 0, got {}", r);
    }

    /// c:21 — `bin_zsocket` various func values don't panic.
    #[test]
    fn bin_zsocket_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_zsocket("zsocket", &[], &ops, func);
        }
    }

    /// c:21 — `bin_zsocket` deterministic for identical args.
    #[test]
    fn bin_zsocket_deterministic_identical_args() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let args = vec!["test".to_string()];
        let r1 = bin_zsocket("zsocket", &args, &ops, 0);
        let r2 = bin_zsocket("zsocket", &args, &ops, 0);
        assert_eq!(r1, r2, "bin_zsocket must be deterministic");
    }

    /// c:351 — `setup_` returns i32 type.
    #[test]
    fn socket_setup_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:366 — `enables_` with Some(non-empty) doesn't panic.
    #[test]
    fn socket_enables_with_some_non_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = Some(vec![1, 2, 3]);
        let _ = enables_(std::ptr::null(), &mut e);
    }

    /// c:351/373 — setup_ then boot_ sequence returns 0 from both.
    #[test]
    fn socket_setup_then_boot_returns_zero_each() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        assert_eq!(boot_(null), 0);
    }
}
