//! Unix domain socket module — port of `Src/Modules/socket.c`.
//!
//! C source has zero `struct ...` / `enum ...` definitions. The
//! Rust port matches: zero types, only the function ports
//! (`bin_zsocket`, `setup_`/`features_`/`enables_`/`boot_`/
//! `cleanup_`/`finish_`).

/// Direct port of `bin_zsocket()` from `Src/Modules/socket.c:57`.
/// C signature matches exactly: `static int bin_zsocket(char *nam,
/// char **args, Options ops, UNUSED(int func))`.
pub fn bin_zsocket(nam: &str, args: &[String],                           // c:57
                   ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG, FDT_UNUSED, FDT_EXTERNAL};
    let mut soun: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut sfd: i32;
    let mut err: i32 = 1;                                                // c:60
    let mut verbose = 0i32;
    let mut test = 0i32;
    let mut targetfd: i32 = 0;
    let mut soun: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let mut sfd: i32;

    if OPT_ISSET(ops, b'v') { verbose = 1; }                            // c:64-65
    if OPT_ISSET(ops, b't') { test    = 1; }                            // c:67-68

    if OPT_ISSET(ops, b'd') {                                           // c:70
        let darg = OPT_ARG(ops, b'd').unwrap_or("");
        targetfd = darg.parse::<i32>().unwrap_or(0);                     // c:71 atoi
        if targetfd == 0 {                                               // c:72
            crate::ported::utils::zwarnnam(nam,
                &format!("{} is an invalid argument to -d", darg));      // c:73
            return 1;                                                    // c:75
        }
        // c:78-82 — `if (targetfd <= max_zsh_fd && fdtable[targetfd] != FDT_UNUSED)`.
        // Static-link path: query the per-process fdtable accessor.
        if crate::ported::utils::fdtable_get(targetfd) != FDT_UNUSED {   // c:78
            crate::ported::utils::zwarnnam(nam,                          // c:79
                &format!("file descriptor {} is in use by the shell", targetfd));
            return 1;                                                    // c:81
        }
    }

    if OPT_ISSET(ops, b'l') {                                           // c:85
        if args.is_empty() {                                             // c:88
            crate::ported::utils::zwarnnam(nam, "-l requires an argument");
            return 1;                                                    // c:90
        }
        let localfn = args[0].as_str();                                  // c:93
        sfd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) }; // c:95
        if sfd == -1 {                                                   // c:97
            crate::ported::utils::zwarnnam(nam,
                &format!("socket error: {} ", std::io::Error::last_os_error())); // c:98
            return 1;                                                    // c:99
        }
        soun.sun_family = libc::AF_UNIX as _;                            // c:102
        let path_bytes = localfn.as_bytes();
        let max_len = soun.sun_path.len() - 1;
        let copy_len = path_bytes.len().min(max_len);
        for (k, &b) in path_bytes[..copy_len].iter().enumerate() {       // c:103 strncpy
            soun.sun_path[k] = b as libc::c_char;
        }
        let r = unsafe {                                                 // c:105
            libc::bind(sfd, &soun as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t)
        };
        if r != 0 {                                                      // c:106
            crate::ported::utils::zwarnnam(nam,
                &format!("could not bind to {}: {}", localfn,
                    std::io::Error::last_os_error()));                   // c:107
            unsafe { libc::close(sfd); }                                 // c:108
            return 1;                                                    // c:109
        }
        if unsafe { libc::listen(sfd, 1) } != 0 {                        // c:112
            crate::ported::utils::zwarnnam(nam,
                &format!("could not listen on socket: {}",
                    std::io::Error::last_os_error()));                   // c:114
            unsafe { libc::close(sfd); }                                 // c:115
            return 1;                                                    // c:116
        }
        crate::ported::utils::addmodulefd(sfd);                          // c:119 FDT_EXTERNAL
        if targetfd != 0 {                                               // c:121
            sfd = crate::ported::utils::redup(sfd, targetfd);            // c:122
        } else {
            sfd = crate::ported::utils::movefd(sfd);                     // c:126 movefd
        }
        if sfd == -1 {                                                   // c:128
            crate::ported::utils::zerrnam(nam,
                &format!("cannot duplicate fd {}: {}", sfd,
                    std::io::Error::last_os_error()));                   // c:129
            return 1;                                                    // c:130
        }
        crate::ported::utils::fdtable_set(sfd, FDT_EXTERNAL);            // c:134
        crate::ported::modules::ksh93::setiparam("REPLY", sfd as i64);   // c:136 setiparam_no_convert
        if verbose != 0 {                                                // c:138
            println!("{} listener is on fd {}", localfn, sfd);           // c:139
        }
        return 0;                                                        // c:141
    } else if OPT_ISSET(ops, b'a') {                                    // c:143
        if args.is_empty() {                                             // c:147
            crate::ported::utils::zwarnnam(nam, "-a requires an argument");
            return 1;                                                    // c:149
        }
        let lfd = args[0].parse::<i32>().unwrap_or(0);                   // c:152 atoi
        if lfd == 0 {                                                    // c:154
            crate::ported::utils::zwarnnam(nam, "invalid numerical argument");
            return 1;                                                    // c:156
        }
        if test != 0 {                                                   // c:159
            // c:163 HAVE_POLL branch.
            let mut pfd = libc::pollfd { fd: lfd, events: libc::POLLIN, revents: 0 };
            let r = unsafe { libc::poll(&mut pfd, 1, 0) };               // c:166
            if r == 0 { return 1; }                                      // c:166
            else if r == -1 {                                            // c:167
                crate::ported::utils::zwarnnam(nam,
                    &format!("poll error: {}",
                        std::io::Error::last_os_error()));               // c:169
                return 1;                                                // c:170
            }
        }
        let mut len: libc::socklen_t =
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t; // c:194
        let rfd: i32;
        loop {                                                           // c:195
            let r = unsafe { libc::accept(lfd,                           // c:196
                &mut soun as *mut _ as *mut libc::sockaddr, &mut len) };
            if r >= 0 { rfd = r; break; }
            let osek = std::io::Error::last_os_error().raw_os_error();
            if osek != Some(libc::EINTR)
                || crate::ported::utils::errflag
                    .load(std::sync::atomic::Ordering::Relaxed) != 0 { rfd = r; break; }
        }
        if rfd == -1 {                                                   // c:199
            crate::ported::utils::zwarnnam(nam,
                &format!("could not accept connection: {}",
                    std::io::Error::last_os_error()));                   // c:200
            return 1;                                                    // c:201
        }
        crate::ported::utils::addmodulefd(rfd);                          // c:204 FDT_EXTERNAL
        if targetfd != 0 {                                               // c:206
            sfd = crate::ported::utils::redup(rfd, targetfd);            // c:207
            if sfd < 0 {                                                 // c:208
                crate::ported::utils::zerrnam(nam,
                    &format!("could not duplicate socket fd to {}: {}",
                        targetfd, std::io::Error::last_os_error()));     // c:209
                unsafe { libc::close(rfd); }                             // c:210
                return 1;                                                // c:211
            }
            crate::ported::utils::fdtable_set(sfd, FDT_EXTERNAL);        // c:213
        } else {
            sfd = rfd;                                                   // c:217
        }
        crate::ported::modules::ksh93::setiparam("REPLY", sfd as i64);   // c:220 setiparam_no_convert
        if verbose != 0 {                                                // c:222
            let path = soun.sun_path.iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8 as char).collect::<String>();
            println!("new connection from {} is on fd {}", path, sfd);   // c:223
        }
    } else {                                                             // c:225
        if args.is_empty() {                                             // c:227
            crate::ported::utils::zwarnnam(nam, "zsocket requires an argument");
            return 1;                                                    // c:229
        }
        sfd = unsafe { libc::socket(libc::PF_UNIX, libc::SOCK_STREAM, 0) }; // c:233
        if sfd == -1 {                                                   // c:235
            crate::ported::utils::zwarnnam(nam,
                &format!("socket creation failed: {}",
                    std::io::Error::last_os_error()));                   // c:236
            return 1;                                                    // c:237
        }
        soun.sun_family = libc::AF_UNIX as _;                            // c:240
        let path_bytes = args[0].as_bytes();
        let max_len = soun.sun_path.len() - 1;
        let copy_len = path_bytes.len().min(max_len);
        for (k, &b) in path_bytes[..copy_len].iter().enumerate() {       // c:241 strncpy
            soun.sun_path[k] = b as libc::c_char;
        }
        err = unsafe {                                                   // c:243
            libc::connect(sfd,
                &soun as *const _ as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t)
        };
        if err != 0 {                                                    // c:243
            crate::ported::utils::zwarnnam(nam,
                &format!("connection failed: {}",
                    std::io::Error::last_os_error()));                   // c:244
            unsafe { libc::close(sfd); }                                 // c:245
            return 1;                                                    // c:246
        }
        crate::ported::utils::addmodulefd(sfd);                          // c:251 FDT_EXTERNAL
        if targetfd != 0 {                                               // c:253
            if crate::ported::utils::redup(sfd, targetfd) < 0 {          // c:254
                crate::ported::utils::zerrnam(nam,
                    &format!("could not duplicate socket fd to {}: {}",
                        targetfd, std::io::Error::last_os_error()));     // c:255
                unsafe { libc::close(sfd); }                             // c:256
                return 1;                                                // c:257
            }
            sfd = targetfd;                                              // c:259
            crate::ported::utils::fdtable_set(sfd, FDT_EXTERNAL);        // c:260
        }
        crate::ported::modules::ksh93::setiparam("REPLY", sfd as i64);   // c:263 setiparam_no_convert
        if verbose != 0 {                                                // c:265
            let path = &args[0];
            println!("{} is now on fd {}", path, sfd);                   // c:266
        }
    }
    let _ = (err, verbose, test, targetfd);                              // silence unused-binding paths
    0                                                                    // c:271
}



// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// ===========================================================

// =====================================================================
// static struct builtin bintab[]                                    c:280
// static struct features module_features                            c:284
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,                                                   // c:285 bintab[1] (zsocket)
        bn_size: 1,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/socket.c:291`.
pub fn setup_(_m: *const module) -> i32 {                                // c:291
    0                                                                    // c:294
}

/// Port of `features_()` from `Src/Modules/socket.c:298`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {  // c:298
    *features = featuresarray(m, module_features());                    // c:301
    0                                                                    // c:302
}

/// Port of `enables_()` from `Src/Modules/socket.c:306`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 { // c:306
    handlefeatures(m, module_features(), enables)                       // c:310
}

/// Port of `boot_()` from `Src/Modules/socket.c:313`.
pub fn boot_(_m: *const module) -> i32 {                                 // c:313
    0                                                                    // c:316
}

/// Port of `cleanup_()` from `Src/Modules/socket.c:320`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                               // c:320
    setfeatureenables(m, module_features(), None)                       // c:323
}

/// Port of `finish_()` from `Src/Modules/socket.c:327`.
pub fn finish_(_m: *const module) -> i32 {                               // c:327
    0                                                                    // c:330
}

// `featuresarray` — Src/module.c:3275.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:zsocket".to_string()]
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
