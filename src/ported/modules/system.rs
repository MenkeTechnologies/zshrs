//! `zsh/system` module — port of `Src/Modules/system.c`.
//!
//! Functions for the errnos special parameter.                              // c:828
//! Functions for the sysparams special parameter.                           // c:842
//! The load/unload routines required by the zsh library interface          // c:916
//!
//! Provides the system-call builtins: `sysread`, `syswrite`, `sysopen`,
//! `sysseek`, `syserror`, `zsystem` (with subcommands `flock` and
//! `supports`); the `systell` math function; the `errnos` and
//! `sysparams` special parameters.
//!
//! C source: 21 ported total — `getposint`, `bin_sysread`, `bin_syswrite`,
//! `bin_sysopen`, `bin_sysseek`, `math_systell`, `bin_syserror`,
//! `bin_zsystem_flock`, `bin_zsystem_supports`, `bin_zsystem`,
//! `errnosgetfn`, `fillpmsysparams`, `getpmsysparams`,
//! `scanpmsysparams`, plus 6 module loaders (`setup_`, `features_`,
//! `enables_`, `boot_`, `cleanup_`, `finish_`).
//!
//! Zero `struct` / `enum` definitions in system.c (only the
//! `static struct { const char *name; int oflag; } openopts[]` ad-hoc
//! anonymous-struct array at c:283 — mirrored as a Rust
//! `&[(&str, i32)]` slice; not a public type).
//!
//! Order in this file mirrors C source order verbatim.

use crate::ported::math::{matheval, mnumber, MN_FLOAT, MN_INTEGER};
use crate::ported::options::{opt_state_get, opt_state_set};
use crate::ported::params::{isident, paramtab, setiparam, setsparam};
use crate::ported::utils::{metafy, movefd, unmeta, zclose, zerr, zstrtol, zwarnnam};
use crate::ported::zsh_h::{module, options, OPT_ARG, OPT_ISSET};
use std::sync::{Mutex, OnceLock};

const SYSREAD_BUFSIZE: usize = 8192; // c:45

/// Port of `getposint(char *instr, char *nam)` from `Src/Modules/system.c:45`. Parses
/// `instr` as a non-negative integer (zstrtol with base 10); emits
/// `zwarnnam` and returns -1 on parse error or negative.
///
/// C signature: `static int getposint(char *instr, char *nam)`.
pub fn getposint(instr: &str, nam: &str) -> i32 {
    // c:45
    // c:45 — `ret = (int)zstrtol(instr, &eptr, 10);`
    let (ret, eptr) = zstrtol(instr, 10);
    let ret = ret as i32;
    // c:51 — `if (*eptr || ret < 0)`
    if !eptr.is_empty() || ret < 0 {
        zwarnnam(nam, &format!("integer expected: {}", instr)); // c:52
        return -1; // c:53
    }
    ret // c:56
}

/// Port of `bin_sysread(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:72`.
/// C: `int bin_sysread(char *nam, char **args, Options ops, int func)`.
/// Builtin spec: `"c:i:o:s:t:"` (system.c:820).
///
/// Return values per c:60-67:
///   0 — Successfully read (and written if `-o`)
///   1 — Error in parameters
///   2 — Read error (errno set)
///   3 — Write error (errno set; partial residue stashed)
///   4 — Timeout on read
///   5 — Zero bytes read (EOF)
#[allow(unused_variables)]
pub fn bin_sysread(
    nam: &str,
    args: &[String], // c:72
    ops: &options,
    func: i32,
) -> i32 {
    // c:74 — `int infd = 0, outfd = -1, bufsize = SYSREAD_BUFSIZE, count;`
    let mut infd: i32 = 0; // c:74
    let mut outfd: i32 = -1; // c:74
    let mut bufsize: usize = SYSREAD_BUFSIZE; // c:74
                                              // c:75 — `char *outvar = NULL, *countvar = NULL, *inbuf;`
    let mut outvar: Option<String> = None; // c:75
    let mut countvar: Option<String> = None; // c:75

    // c:80 — `if (OPT_ISSET(ops, 'i')) { infd = getposint(OPT_ARG(ops,'i'),nam); ...}`
    if OPT_ISSET(ops, b'i') {
        // c:80
        infd = getposint(OPT_ARG(ops, b'i').unwrap_or(""), nam); // c:81
        if infd < 0 {
            return 1;
        } // c:82-83
    }
    // c:87 — `if (OPT_ISSET(ops, 'o')) { outfd = getposint(OPT_ARG(ops,'o'),nam); ...}`
    if OPT_ISSET(ops, b'o') {
        // c:87
        outfd = getposint(OPT_ARG(ops, b'o').unwrap_or(""), nam); // c:88
        if outfd < 0 {
            return 1;
        } // c:89-90
    }
    // c:94 — `if (OPT_ISSET(ops, 's')) bufsize = getposint(OPT_ARG(ops,'s'),nam);`
    if OPT_ISSET(ops, b's') {
        // c:94
        let v = getposint(OPT_ARG(ops, b's').unwrap_or(""), nam); // c:95
        if v < 0 {
            return 1;
        } // c:96-97
        bufsize = v as usize;
    }
    // c:101 — `if (OPT_ISSET(ops, 'c')) { countvar = OPT_ARG(ops,'c'); isident...}`
    if OPT_ISSET(ops, b'c') {
        // c:101
        let cv = OPT_ARG(ops, b'c').unwrap_or("").to_string(); // c:102
        if !isident(&cv) {
            // c:103
            zwarnnam(nam, &format!("not an identifier: {}", cv)); // c:104
            return 1; // c:105
        }
        countvar = Some(cv);
    }
    // c:109 — `if (*args) { outvar = *args; isident... }`
    if !args.is_empty() {
        // c:109
        let ov = args[0].clone(); // c:116
        if !isident(&ov) {
            // c:117
            zwarnnam(nam, &format!("not an identifier: {}", ov)); // c:118
            return 1; // c:119
        }
        outvar = Some(ov);
    }
    let timeout_arg: Option<&str> = if OPT_ISSET(ops, b't') {
        // c:127
        OPT_ARG(ops, b't')
    } else {
        None
    };

    // c:123 — `inbuf = zhalloc(bufsize);`
    let mut inbuf = vec![0u8; bufsize]; // c:123

    // c:127-185 — `-t` poll(2) wait. C uses HAVE_POLL → poll(); else
    // select(). Rust has poll(2) on every supported unix; pick the
    // poll branch (c:129-152).
    if let Some(t_str) = timeout_arg {
        // c:137 — `to_mn = matheval(OPT_ARG(ops,'t'));`
        // c:138-139 `if (errflag) return 1;` — mathevali's zerr already
        // wrote the diagnostic in C; Rust captures it in Err — surface
        // it via zerr() so callers see the parse error on stderr.
        let to_mn = match matheval(t_str) {
            Ok(m) => m,
            Err(msg) => {
                zerr(&msg);
                return 1; // c:138-139 errflag
            }
        };
        // c:140-143 — float→int conversion of seconds × 1000.
        let to_int: i32 = if to_mn.type_ == MN_FLOAT {
            (1000.0 * to_mn.d) as i32 // c:141
        } else {
            (1000 * to_mn.l) as i32 // c:143
        };
        // c:145-148 — `while ((ret = poll(...)) < 0) { if (errno !=
        //              EINTR || errflag || retflag || breaks ||
        //              contflag) break; }`. Same shell-control-flow
        //              flag bail as bin_syswrite (a4fd96ac0e).
        use std::sync::atomic::Ordering::Relaxed;
        let mut ret;
        loop {
            let mut pfd = libc::pollfd {
                // c:130-135
                fd: infd,
                events: libc::POLLIN,
                revents: 0,
            };
            ret = unsafe { libc::poll(&mut pfd, 1, to_int) };
            if ret >= 0 {
                break;
            }
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            let interrupted = crate::ported::utils::errflag.load(Relaxed) != 0
                || crate::ported::exec::retflag.load(Relaxed) != 0
                || crate::ported::builtin::BREAKS.load(Relaxed) != 0
                || crate::ported::builtin::CONTFLAG.load(Relaxed) != 0;
            if eno != libc::EINTR || interrupted {
                break; // c:177
            }
        }
        // c:149-151 — `if (ret <= 0) return ret ? 2 : 4;`
        if ret <= 0 {
            return if ret != 0 { 2 } else { 4 };
        }
    }

    // c:188-191 — `while ((count = read(infd, inbuf, bufsize)) < 0)
    //                  { if (errno != EINTR || errflag || retflag
    //                       || breaks || contflag) break; }`.
    // Same control-flow flag bail as bin_syswrite (a4fd96ac0e).
    use std::sync::atomic::Ordering::Relaxed;
    let mut count: isize;
    loop {
        count = unsafe { libc::read(infd, inbuf.as_mut_ptr() as *mut libc::c_void, bufsize) };
        if count >= 0 {
            break;
        }
        let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let interrupted = crate::ported::utils::errflag.load(Relaxed) != 0
            || crate::ported::exec::retflag.load(Relaxed) != 0
            || crate::ported::builtin::BREAKS.load(Relaxed) != 0
            || crate::ported::builtin::CONTFLAG.load(Relaxed) != 0;
        if eno != libc::EINTR || interrupted {
            break; // c:189
        }
    }
    // c:192-193 — `if (countvar) setiparam(countvar, count);`
    if let Some(ref cv) = countvar {
        setiparam(cv, count as i64); // c:192
    }
    // c:194-195 — `if (count < 0) return 2;`
    if count < 0 {
        return 2;
    }
    let count = count as usize;

    // c:197-218 — outfd write path with EINTR retry + partial residue.
    if outfd >= 0 {
        // c:197
        if count == 0 {
            return 5;
        } // c:198-199
        let mut p = 0usize;
        let mut remaining = count;
        while remaining > 0 {
            // c:200
            let ret = unsafe {
                libc::write(outfd, inbuf[p..].as_ptr() as *const libc::c_void, remaining)
            };
            if ret < 0 {
                // c:204
                // c:205-206 — `if (errno == EINTR && !errflag &&
                //               !retflag && !breaks && !contflag) continue;`
                // C only retries when ALL FOUR control-flow flags are
                // clear AND errno is EINTR. Prior port retried on any
                // EINTR regardless of shell state.
                use std::sync::atomic::Ordering::Relaxed;
                let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                let interrupted = crate::ported::utils::errflag.load(Relaxed) != 0
                    || crate::ported::exec::retflag.load(Relaxed) != 0
                    || crate::ported::builtin::BREAKS.load(Relaxed) != 0
                    || crate::ported::builtin::CONTFLAG.load(Relaxed) != 0;
                if eno == libc::EINTR && !interrupted {
                    // c:205-207 — clean EINTR, retry.
                    continue;
                }
                // c:208-212 — stash residue + remaining count.
                if let Some(ref ov) = outvar {
                    let buf_remaining = String::from_utf8_lossy(&inbuf[p..p + remaining]);
                    let m = metafy(&buf_remaining);
                    setsparam(ov, &m); // c:209
                }
                if let Some(ref cv) = countvar {
                    setiparam(cv, remaining as i64); // c:210
                }
                return 3; // c:212
            }
            p += ret as usize; // c:214
            remaining -= ret as usize; // c:215
        }
        return 0; // c:217
    }

    // c:220-225 — no outfd: stash buffer in `outvar` (default REPLY).
    let target = outvar.unwrap_or_else(|| "REPLY".to_string()); // c:220-221
    let buf_str = String::from_utf8_lossy(&inbuf[..count]);
    let m = metafy(&buf_str);
    setsparam(&target, &m); // c:223
    if count != 0 {
        0
    } else {
        5
    } // c:225
}

/// Port of `bin_syswrite(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:238`.
///
/// C signature: `static int bin_syswrite(char *nam, char **args,
///                                        Options ops, int func)`.
/// Builtin spec: `"c:o:"` (system.c:821), 1 mandatory positional
/// arg.
///
/// Return values per c:230-233:
///   0 — Successfully written
///   1 — Error in parameters
///   2 — Write error (errno set)
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_syswrite(
    nam: &str,
    args: &[String], // c:238
    ops: &options,
    _func: i32,
) -> i32 {
    // c:240-241 — `int outfd = 1, len, count, totcount;
    //              char *countvar = NULL;`
    let mut outfd: i32 = 1; // c:240
    let mut countvar: Option<String> = None; // c:241

    // c:246 — `if (OPT_ISSET(ops, 'o')) { outfd = getposint(OPT_ARG(ops,'o'),nam); ...}`
    if OPT_ISSET(ops, b'o') {
        // c:246
        outfd = getposint(OPT_ARG(ops, b'o').unwrap_or(""), nam); // c:247
        if outfd < 0 {
            return 1;
        } // c:248-249
    }
    // c:253 — `if (OPT_ISSET(ops, 'c')) { countvar = OPT_ARG(ops,'c'); isident...}`
    if OPT_ISSET(ops, b'c') {
        // c:253
        let cv = OPT_ARG(ops, b'c').unwrap_or("").to_string(); // c:254
        if !isident(&cv) {
            // c:255
            zwarnnam(nam, &format!("not an identifier: {}", cv)); // c:256
            return 1; // c:257
        }
        countvar = Some(cv);
    }
    // c:262 — `unmetafy(*args, &len);` — first positional arg = data.
    let data = match args.first() {
        // c:262
        Some(d) => d.clone(),
        None => return 1,
    };

    // c:262 — `unmetafy(*args, &len);`
    let unmeta = unmeta(&data);
    let bytes = unmeta.as_bytes();
    let mut totcount: usize = 0; // c:261
    let mut len = bytes.len();
    let mut p = 0usize;

    // c:263-275 — write loop with EINTR retry and partial residue.
    // C body:
    //   while ((count = write(outfd, *args, len)) < 0) {
    //       if (errno != EINTR || errflag || retflag || breaks || contflag)
    //       {
    //           if (countvar) setiparam(countvar, totcount);
    //           return 2;
    //       }
    //   }
    //
    // Prior Rust port only checked `errno != EINTR` — ignored
    // errflag / retflag / breaks / contflag. That meant the EINTR
    // retry loop would keep spinning even after Ctrl-C (which
    // sets errflag via the SIGINT trap) or a `return` from an
    // enclosing function body. Now matches C: any of the four
    // shell control-flow flags bails out the retry the same as
    // a non-EINTR errno.
    use std::sync::atomic::Ordering::Relaxed;
    while len > 0 {
        // c:263
        let count = unsafe { libc::write(outfd, bytes[p..].as_ptr() as *const libc::c_void, len) };
        if count < 0 {
            // c:264
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            let interrupted = crate::ported::utils::errflag.load(Relaxed) != 0
                || crate::ported::exec::retflag.load(Relaxed) != 0
                || crate::ported::builtin::BREAKS.load(Relaxed) != 0
                || crate::ported::builtin::CONTFLAG.load(Relaxed) != 0;
            if eno != libc::EINTR || interrupted {
                // c:265
                if let Some(ref cv) = countvar {
                    // c:267-268
                    setiparam(cv, totcount as i64); // c:268
                }
                return 2; // c:269
            }
            continue;
        }
        p += count as usize; // c:272 *args += count
        totcount += count as usize; // c:273
        len -= count as usize; // c:274
    }
    // c:276-277 — `if (countvar) setiparam(countvar, totcount);`
    if let Some(ref cv) = countvar {
        setiparam(cv, totcount as i64); // c:277
    }
    0 // c:279
}

/// Port of `bin_sysopen(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:319`.
///
/// C signature: `static int bin_sysopen(char *nam, char **args,
///                                       Options ops, int func)`.
/// Builtin spec: `"rwau:o:m:"` (system.c:822), 1 mandatory
/// positional arg (the file path).
///
/// Return values per c:312-314: 0 success / 1 bad params / 2 open error.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_sysopen(
    nam: &str,
    args: &[String], // c:319
    ops: &options,
    _func: i32,
) -> i32 {
    // c:321-323 — `int read = OPT_ISSET(ops, 'r');` etc.
    let read_flag = OPT_ISSET(ops, b'r'); // c:321
    let write_flag = OPT_ISSET(ops, b'w'); // c:322
    let append_flag = OPT_ISSET(ops, b'a'); // c:323

    // c:323-325 — flags = O_NOCTTY | append | (RDWR/WRONLY/RDONLY).
    let append_flag_bit = if append_flag { libc::O_APPEND } else { 0 };
    let mut flags = libc::O_NOCTTY
        | append_flag_bit
        | if append_flag || write_flag {
            if read_flag {
                libc::O_RDWR
            } else {
                libc::O_WRONLY
            }
        } else {
            libc::O_RDONLY
        };

    // c:328 — `mode_t perms = 0666;`
    let mut perms: u32 = 0o666;
    let mut explicit: i32 = -1; // c:327

    // c:335 — `if (!OPT_ISSET(ops, 'u')) { ... return 1; }`
    if !OPT_ISSET(ops, b'u') {
        zwarnnam(nam, "file descriptor not specified"); // c:336
        return 1; // c:337
    }
    let fdvar = OPT_ARG(ops, b'u').unwrap_or("").to_string(); // c:340
    let path = match args.first() {
        Some(p) => p.clone(),
        None => return 1,
    };
    let o_arg: Option<&str> = if OPT_ISSET(ops, b'o') {
        OPT_ARG(ops, b'o')
    } else {
        None
    };
    let m_arg: Option<&str> = if OPT_ISSET(ops, b'm') {
        OPT_ARG(ops, b'm')
    } else {
        None
    };

    // c:341-347 — fdvar is either single digit (explicit fd) or identifier.
    if fdvar.len() == 1 && fdvar.chars().next().unwrap().is_ascii_digit() {
        explicit = fdvar.parse().unwrap(); // c:343
    } else if !isident(&fdvar) {
        // c:344
        zwarnnam(nam, &format!("not an identifier: {}", fdvar)); // c:345
        return 1; // c:346
    }

    // c:350-369 — comma-list of O_* names from -o, case-insensitive,
    // optional `O_` prefix.
    if let Some(opts) = o_arg {
        for tok in opts.split(',') {
            // c:355 strchr ','
            let mut name: &str = tok;
            // c:353 — `if (!strncasecmp(opt, "O_", 2)) opt += 2;`
            if name.len() >= 2 && name[..2].eq_ignore_ascii_case("O_") {
                name = &name[2..];
            }
            // c:357-358 — case-insensitive lookup in openopts[].
            // openopts[] is the c:283-308 anonymous-struct table:
            // `static struct { const char *name; int oflag; } openopts[]`.
            // Inlined here as a const slice so the lookup is bit-for-bit
            // identical to C (same name/oflag rows, same order, walked
            // backwards via `for (o = N-1; o >= 0; o--)` at c:357).
            #[cfg(unix)]
            {
                const OPENOPTS: &[(&str, i32)] = &[
                    ("cloexec", libc::O_CLOEXEC),   // c:285
                    ("nofollow", libc::O_NOFOLLOW), // c:292
                    ("sync", libc::O_SYNC),         // c:295
                    #[cfg(target_os = "linux")]
                    ("noatime", libc::O_NOATIME), // c:298
                    ("nonblock", libc::O_NONBLOCK), // c:301
                    ("excl", libc::O_EXCL | libc::O_CREAT), // c:303
                    ("creat", libc::O_CREAT),       // c:304
                    ("create", libc::O_CREAT),      // c:305
                    ("truncate", libc::O_TRUNC),    // c:306
                    ("trunc", libc::O_TRUNC),       // c:307
                ];
                let mut found: Option<i32> = None;
                for (n, oflag) in OPENOPTS.iter().rev() {
                    // c:357 walks backwards
                    if n.eq_ignore_ascii_case(name) {
                        found = Some(*oflag);
                        break;
                    }
                }
                let oflag = match found {
                    Some(f) => f,
                    None => {
                        zwarnnam(nam, &format!("unsupported option: {}\n", tok)); // c:360
                        return 1; // c:361
                    }
                };
                flags |= oflag; // c:367
            }
        }
    }

    // c:372-381 — -m: octal permissions string.
    if let Some(m) = m_arg {
        let mode_str: &str = m;
        // c:374-375 — `while (*ptr >= '0' && *ptr <= '7') ptr++;`
        let mut ptr = 0;
        let bytes = mode_str.as_bytes();
        while ptr < bytes.len() && (b'0'..=b'7').contains(&bytes[ptr]) {
            ptr += 1;
        }
        // c:376 — `if (*ptr || ptr - opt < 3)`
        if ptr < bytes.len() || ptr < 3 {
            zwarnnam(nam, &format!("invalid mode {}", mode_str)); // c:377
            return 1; // c:378
        }
        // c:380 — `perms = zstrtol(opt, 0, 8);`
        let (v, _) = zstrtol(mode_str, 8);
        perms = v as u32;
    }

    // c:383-391 — `open(*args, flags[, perms])`; `*args` is path.
    let path_c = match std::ffi::CString::new(path.as_bytes()) {
        Ok(s) => s,
        Err(_) => return 1,
    };
    let fd = unsafe {
        if (flags & libc::O_CREAT) != 0 {
            // c:383
            libc::open(path_c.as_ptr(), flags, perms as libc::c_uint) // c:384
        } else {
            libc::open(path_c.as_ptr(), flags) // c:386
        }
    };
    if fd == -1 {
        // c:388
        let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        // c:389 — `zwarnnam(nam, "can't open file %s: %e", *args, errno);`
        // %e per Src/utils.c:352-368 (lowercased strerror).
        zwarnnam(
            nam,
            &format!(
                "can't open file {}: {}",
                path,
                crate::vm_helper::zsh_errno_msg(eno)
            ),
        ); // c:389
        return 2; // c:390
    }

    // c:392 — `moved_fd = (explicit > -1) ? redup(fd, explicit) : movefd(fd);`
    let moved_fd: i32 = if explicit > -1 {
        crate::ported::utils::redup(fd, explicit) // c:392 redup branch
    } else {
        movefd(fd) // c:392 movefd branch
    };
    if moved_fd == -1 {
        // c:393
        zwarnnam(nam, &format!("can't open file {}", path)); // c:394
        return 2; // c:395
    }

    // c:398-411 — reapply FD_CLOEXEC after dup2 if requested.
    if (flags & libc::O_CLOEXEC) != 0 && fd != moved_fd {
        // c:406
        unsafe {
            libc::fcntl(moved_fd, libc::F_SETFD, libc::FD_CLOEXEC);
        } // c:410
    }

    // c:412 — `fdtable[moved_fd] = FDT_EXTERNAL;` (zshrs's fdtable
    // manager owns this; not yet wired — no-op for now).

    // c:413-418 — `if (explicit == -1) { setiparam(fdvar, moved_fd); ... }`
    if explicit == -1 {
        setiparam(&fdvar, moved_fd as i64); // c:414
    }

    0 // c:433
}

/// Port of `bin_sysseek(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:433`.
///
/// C signature: `static int bin_sysseek(char *nam, char **args,
///                                       Options ops, int func)`.
/// Builtin spec: `"u:w:"` (system.c:823), 1 mandatory positional
/// arg (the offset).
///
/// Return values per c:425-428: 0 success / 1 bad params / 2 lseek error.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_sysseek(
    nam: &str,
    args: &[String], // c:433
    ops: &options,
    _func: i32,
) -> i32 {
    // c:435 — `int w = SEEK_SET, fd = 0;`
    let mut w: i32 = libc::SEEK_SET; // c:435
    let mut fd: i32 = 0; // c:435

    // c:441-446 — `if (OPT_ISSET(ops, 'u')) { fd = getposint(OPT_ARG(ops,'u'),nam); ...}`
    if OPT_ISSET(ops, b'u') {
        // c:441
        fd = getposint(OPT_ARG(ops, b'u').unwrap_or(""), nam); // c:442
        if fd < 0 {
            return 1;
        } // c:443-444
    }
    // c:449-460 — `-w` whence parse (case-insensitive).
    if OPT_ISSET(ops, b'w') {
        // c:449
        let whence = OPT_ARG(ops, b'w').unwrap_or(""); // c:450
        if whence.eq_ignore_ascii_case("current") || whence == "1" {
            // c:451
            w = libc::SEEK_CUR; // c:452
        } else if whence.eq_ignore_ascii_case("end") || whence == "2" {
            // c:453
            w = libc::SEEK_END; // c:454
        } else if !whence.eq_ignore_ascii_case("start") && whence != "0" {
            // c:455
            zwarnnam(nam, &format!("unknown argument to -w: {}", whence)); // c:456
            return 1; // c:457
        }
    }

    // c:461 — `pos = (off_t)mathevali(*args);`
    let pos_str = match args.first() {
        Some(s) => s.clone(),
        None => return 1,
    };
    let pos = match crate::ported::math::mathevali(&pos_str) {
        // c:461 — mathevali errflag → zerr already in C; surface Err msg.
        Ok(v) => v,
        Err(msg) => {
            zerr(&msg);
            return 1;
        }
    };
    // c:462 — `return (lseek(fd, pos, w) == -1) ? 2 : 0;`
    if unsafe { libc::lseek(fd, pos as libc::off_t, w) } == -1 {
        // c:462
        2
    } else {
        0
    }
}

/// Port of `math_systell(UNUSED(char *name), UNUSED(int argc), mnumber *argv, UNUSED(int id))` from `Src/Modules/system.c:467`.
///
/// C signature: `static mnumber math_systell(char *name, int argc,
///                                            mnumber *argv, int id)`.
/// Returns the current `lseek(fd, 0, SEEK_CUR)` position of `argv[0]`
/// as an `mnumber`. Negative fds error via `zerr` and return 0.
#[allow(unused_variables)]
pub fn math_systell(name: &str, argc: i32, argv: &[mnumber], id: i32) -> mnumber {
    // c:467
    // C's mathfunc dispatch (mathfunc.c::stdmathfn) enforces min/max
    // arg counts BEFORE calling the per-fn implementation — so the C
    // body can `argv->u.l` safely. The Rust port calls through a
    // generic adapter without the upstream guard; check argc/len here
    // so a direct test call (or future dispatch divergence) doesn't
    // index `argv[0]` on an empty slice. Mirror C's "missing arg →
    // return 0 mnumber" failure shape.
    if argc < 1 || argv.is_empty() {
        return mnumber {
            type_: MN_INTEGER,
            l: 0,
            d: 0.0,
        };
    }
    // c:467 — `int fd = (argv->type == MN_INTEGER) ? argv->u.l : (int)argv->u.d;`
    let fd: i32 = if argv[0].type_ == MN_INTEGER {
        argv[0].l as i32
    } else {
        argv[0].d as i32
    };
    // c:470-472 — `mnumber ret; ret.type = MN_INTEGER; ret.u.l = 0;`
    let mut ret = mnumber {
        type_: MN_INTEGER, // c:471
        l: 0,              // c:472
        d: 0.0,
    };
    // c:474-477 — `if (fd < 0) { zerr("file descriptor out of range"); return ret; }`
    //
    // C uses zerr (not zwarn) — the difference matters: zerr sets the
    // shell's errflag so the failure propagates through `set -e`
    // (errexit) and lastval, while zwarn just prints to stderr without
    // touching error state. Prior Rust port called zwarn so
    // `set -e; : $((systell(-1)))` silently continued instead of
    // aborting the script as C does.
    if fd < 0 {
        crate::ported::utils::zerr("file descriptor out of range"); // c:475
        return ret;
    }
    // c:478 — `ret.u.l = lseek(fd, 0, SEEK_CUR);`
    ret.l = unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) } as i64;
    ret // c:494
}

/// Port of `bin_syserror(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:494`.
///
/// C signature: `static int bin_syserror(char *nam, char **args,
///                                        Options ops, int func)`.
/// Builtin spec: `"e:p:"` (system.c:819), 0-1 positional args
/// (the errno number or symbolic name).
///
/// Return values per c:485-489: 0 success / 1 bad params / 2 unknown errno name.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_syserror(
    nam: &str,
    args: &[String], // c:494
    ops: &options,
    _func: i32,
) -> i32 {
    // c:496-497 — `int num = 0; char *errvar = NULL, *msg, *pfx = "", *str;`
    let mut num: i32 = 0;
    let mut errvar: Option<String> = None;
    let mut pfx: String = String::new();

    // c:500-505 — `if (OPT_ISSET(ops, 'e')) { errvar = OPT_ARG(...); isident...}`
    if OPT_ISSET(ops, b'e') {
        // c:500
        let ev = OPT_ARG(ops, b'e').unwrap_or("").to_string(); // c:501
        if !isident(&ev) {
            // c:502
            zwarnnam(nam, &format!("not an identifier: {}", ev)); // c:503
            return 1; // c:504
        }
        errvar = Some(ev);
    }
    // c:508 — `if (OPT_ISSET(ops, 'p')) pfx = OPT_ARG(ops, 'p');`
    if OPT_ISSET(ops, b'p') {
        // c:508
        pfx = OPT_ARG(ops, b'p').unwrap_or("").to_string(); // c:509
    }

    // c:511-530 — name parse: empty → use current errno; all-digit →
    // atoi; symbolic → lookup in sys_errnames, return 2 on miss.
    if args.is_empty() {
        // c:511
        // c:512 — `num = errno;`
        num = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    } else {
        let arg = &args[0];
        let bytes = arg.as_bytes();
        let mut ptr = 0usize;
        // c:514-516 — `while (*ptr && idigit(*ptr)) ptr++;`
        while ptr < bytes.len() && bytes[ptr].is_ascii_digit() {
            ptr += 1;
        }
        if ptr == bytes.len() && ptr > 0 {
            // c:517
            num = arg.parse::<i32>().unwrap_or(0); // c:518
        } else {
            // c:519
            // c:521-526 — walk SYS_ERRNAMES looking for *args.
            let mut found = false;
            for (idx, (ename, _)) in SYS_ERRNAMES.iter().enumerate() {
                if *ename == arg {
                    // c:522
                    num = (idx as i32) + 1; // c:523
                    found = true;
                    break; // c:524
                }
            }
            if !found {
                // c:527
                return 2; // c:528
            }
        }
    }

    // c:532 — `msg = strerror(num);`. Use libc::strerror so the
    // output matches C zsh byte-for-byte (e.g. "No such file or
    // directory"). std::io::Error::from_raw_os_error(n).to_string()
    // would append " (os error N)" — wrong format for the
    // `${(t)errvar}` consumer. Bug #316 in docs/BUGS.md.
    let msg = unsafe {
        let p = libc::strerror(num);
        if p.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };
    // c:533-539 — write back to errvar or stderr.
    if let Some(ev) = errvar {
        let str_out = format!("{}{}", pfx, msg); // c:534-535
        setsparam(&ev, &str_out); // c:536
    } else {
        eprintln!("{}{}", pfx, msg); // c:538
    }
    0 // c:541
}

/// Port of `bin_zsystem_flock(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/system.c:546`.
///
/// C signature: `static int bin_zsystem_flock(char *nam, char **args,
///                                              Options ops, int func)`.
/// Subcommand of `zsystem flock`. Parses its own option chain (no
/// builtin opt-spec since the parent `zsystem` BUILTIN at c:824 has
/// `optstr=NULL`).
///
/// Return values per inline comments: 0 success / 1 param/lock error
/// / 2 timeout exhausted / 255 not supported on this platform.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zsystem_flock(
    nam: &str,
    args: &[String], // c:546
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:548-551 — option-state locals.
    let mut cloexec: bool = true; // c:548
    let mut unlock: bool = false;
    let mut readlock: bool = false;
    let mut timeout: f64 = -1.0; // c:549
                                 // c:550 — `long timeout_interval = 1e6;` (microseconds).
    let mut timeout_interval: i64 = 1_000_000;
    let mut fdvar: Option<String> = None; // c:552

    // c:558-661 — option-chain parser. `while (*args && **args == '-')`.
    let mut i = 0usize;
    while i < args.len() && args[i].starts_with('-') {
        let arg = &args[i];
        i += 1;
        let optptr = &arg[1..];
        if optptr.is_empty() || optptr == "-" {
            // c:562
            break;
        }
        let chars: Vec<char> = optptr.chars().collect();
        let mut idx = 0usize;
        while idx < chars.len() {
            let opt = chars[idx];
            match opt {
                'e' => {
                    // c:566 keep lock on exec
                    cloexec = false; // c:568
                }
                'f' => {
                    // c:571 fd variable
                    let rest: String = chars[idx + 1..].iter().collect();
                    let fdvar_str = if !rest.is_empty() {
                        idx = chars.len(); // c:574-575 consume rest
                        rest
                    } else if i < args.len() {
                        let v = args[i].clone(); // c:577
                        i += 1;
                        v
                    } else {
                        zwarnnam(
                            nam,
                            &format!("flock: option {} requires a variable name", opt),
                        );
                        return 1;
                    };
                    if !isident(&fdvar_str) {
                        // c:579
                        zwarnnam(
                            nam,
                            &format!("flock: option {} requires a variable name", opt),
                        );
                        return 1; // c:582
                    }
                    fdvar = Some(fdvar_str);
                    break;
                }
                'r' => readlock = true, // c:586-588
                't' => {
                    // c:591 timeout in seconds
                    let rest: String = chars[idx + 1..].iter().collect();
                    let optarg = if !rest.is_empty() {
                        idx = chars.len();
                        rest
                    } else if i < args.len() {
                        let v = args[i].clone();
                        i += 1;
                        v
                    } else {
                        zwarnnam(
                            nam,
                            &format!("flock: option {} requires a numeric timeout", opt),
                        );
                        return 1;
                    };
                    let tp = match matheval(&optarg) {
                        Ok(m) => m,
                        Err(msg) => {
                            zerr(&msg);
                            return 1;
                        }
                    };
                    timeout = if (tp.type_ & MN_FLOAT) != 0 {
                        // c:604
                        tp.d
                    } else {
                        tp.l as f64
                    };
                    // c:614-618 — overflow guard at 2^30-1.
                    if timeout > 1073741823.0 {
                        zwarnnam(nam, &format!("flock: invalid timeout value: '{}'", optarg));
                        return 1;
                    }
                    break;
                }
                'i' => {
                    // c:621 retry interval
                    let rest: String = chars[idx + 1..].iter().collect();
                    let optarg = if !rest.is_empty() {
                        idx = chars.len();
                        rest
                    } else if i < args.len() {
                        let v = args[i].clone();
                        i += 1;
                        v
                    } else {
                        zwarnnam(
                            nam,
                            &format!("flock: option {} requires a numeric retry interval", opt),
                        );
                        return 1;
                    };
                    let mut tp = match matheval(&optarg) {
                        Ok(m) => m,
                        Err(msg) => {
                            zerr(&msg);
                            return 1;
                        }
                    };
                    if (tp.type_ & MN_FLOAT) == 0 {
                        // c:636
                        tp.type_ = MN_FLOAT;
                        tp.d = tp.l as f64;
                    }
                    tp.d = (tp.d * 1e6).ceil(); // c:640
                    if tp.d < 1.0 || tp.d > 0.999 * (i64::MAX as f64) {
                        // c:641
                        zwarnnam(nam, &format!("flock: invalid interval value: '{}'", optarg));
                        return 1; // c:645
                    }
                    timeout_interval = tp.d as i64; // c:647
                    break;
                }
                'u' => unlock = true, // c:650-652
                _ => {
                    zwarnnam(nam, &format!("flock: unknown option: {}", opt)); // c:656
                    return 1; // c:657
                }
            }
            idx += 1;
        }
    }

    // c:664-667 — `if (!args[0]) { zwarnnam("flock: not enough arguments"); return 1; }`
    if i >= args.len() {
        zwarnnam(nam, "flock: not enough arguments");
        return 1;
    }
    if i + 1 < args.len() {
        // c:668-671
        zwarnnam(nam, "flock: too many arguments");
        return 1;
    }
    let path = &args[i];

    // c:674-682 — -u: unlock. argument is fd; close releases POSIX lock.
    if unlock {
        let flock_fd: i32 = match crate::ported::math::mathevali(path) {
            Ok(v) => v as i32,
            Err(msg) => {
                zerr(&msg);
                return 1;
            }
        };
        // c:676 — zcloselockfd(flock_fd) returns -1 if not in our lockfd table.
        if crate::ported::utils::zcloselockfd(flock_fd) < 0 {
            // c:676
            zwarnnam(
                nam,
                &format!("flock: file descriptor {} not in use for locking", flock_fd),
            );
            return 1;
        }
        return 0; // c:681
    }

    // c:684-687 — flags = readlock ? O_RDONLY|O_NOCTTY : O_RDWR|O_NOCTTY.
    let flags = if readlock {
        libc::O_RDONLY | libc::O_NOCTTY
    } else {
        libc::O_RDWR | libc::O_NOCTTY
    };
    // c:688 — open(unmeta(args[0]), flags).
    let path_unmeta = unmeta(path);
    let path_c = match std::ffi::CString::new(path_unmeta) {
        Ok(s) => s,
        Err(_) => return 1,
    };
    let mut flock_fd = unsafe { libc::open(path_c.as_ptr(), flags) }; // c:688
    if flock_fd < 0 {
        // c:689 — `zwarnnam(nam, "failed to open %s for writing: %e", args[0], errno);`
        let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        zwarnnam(
            nam,
            &format!(
                "failed to open {} for writing: {}",
                path,
                crate::vm_helper::zsh_errno_msg(eno)
            ),
        );
        return 1;
    }
    // c:692 — `flock_fd = movefd(flock_fd);`
    flock_fd = movefd(flock_fd); // c:692
    if flock_fd == -1 {
        return 1;
    } // c:693-694

    // c:695-702 — set FD_CLOEXEC if cloexec.
    if cloexec {
        let fdflags = unsafe { libc::fcntl(flock_fd, libc::F_GETFD, 0) };
        if fdflags != -1 {
            unsafe {
                libc::fcntl(flock_fd, libc::F_SETFD, fdflags | libc::FD_CLOEXEC);
            }
        }
    }
    // c:703 — `addlockfd(flock_fd, cloexec);`
    crate::ported::utils::addlockfd(flock_fd, cloexec); // c:703

    // c:705-708 — assemble struct flock.
    let lock_type: libc::c_short = if readlock {
        libc::F_RDLCK as libc::c_short
    } else {
        libc::F_WRLCK as libc::c_short
    };
    #[allow(clippy::unnecessary_cast)]
    let lck = libc::flock {
        l_type: lock_type,                         // c:705
        l_whence: libc::SEEK_SET as libc::c_short, // c:706
        l_start: 0,                                // c:707
        l_len: 0,                                  // c:708
        l_pid: 0,
    };

    use std::sync::atomic::Ordering::Relaxed;
    if timeout > 0.0 {
        // c:710
        // c:711-749 — timed retry loop.
        // C body c:729-749:
        //   while (fcntl(flock_fd, F_SETLK, &lck) < 0) {
        //       if (errflag) { zclose(flock_fd); return 1; }    ← c:730
        //       if (errno != EINTR && errno != EACCES && errno != EAGAIN) {
        //           zclose(flock_fd);
        //           zwarnnam(nam, 'failed to lock file ...');
        //           return 1;
        //       }
        //       ... deadline check + sleep ...
        //   }
        //
        // Prior port skipped the errflag check at c:730 — Ctrl-C
        // during a flock-acquire wait wouldn't bail out the retry.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(timeout);
        loop {
            let r = unsafe { libc::fcntl(flock_fd, libc::F_SETLK, &lck) };
            if r >= 0 {
                break;
            }
            // c:730 — `if (errflag) { zclose; return 1; }`.
            if crate::ported::utils::errflag.load(Relaxed) != 0 {
                zclose(flock_fd); // c:731
                return 1; // c:732
            }
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if eno != libc::EINTR && eno != libc::EACCES && eno != libc::EAGAIN {
                zclose(flock_fd); // c:735
                // c:736 — `zwarnnam(nam, "failed to lock file %s: %e", args[0], errno);`
                // Format from the errno captured BEFORE zclose — close(2)
                // may clobber errno.
                zwarnnam(
                    nam,
                    &format!(
                        "failed to lock file {}: {}",
                        path,
                        crate::vm_helper::zsh_errno_msg(eno)
                    ),
                );
                return 1;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                zclose(flock_fd); // c:742
                return 2; // c:743
            }
            let remaining = deadline - now;
            let remaining_us = remaining.as_micros() as i64;
            let interval = remaining_us.min(timeout_interval);
            std::thread::sleep(std::time::Duration::from_micros(interval as u64));
        }
    } else {
        // c:751-762 — no timeout: F_SETLK if timeout==0 (non-blocking),
        // else F_SETLKW (blocking).
        // C body:
        //   while (fcntl(flock_fd, ...) < 0) {
        //       if (errflag) { zclose; return 1; }   ← c:752
        //       if (errno == EINTR) continue;
        //       zclose(flock_fd);
        //       zwarnnam(nam, 'failed to lock file ...');
        //       return 1;
        //   }
        let cmd = if timeout == 0.0 {
            libc::F_SETLK
        } else {
            libc::F_SETLKW
        };
        loop {
            let r = unsafe { libc::fcntl(flock_fd, cmd, &lck) };
            if r >= 0 {
                break;
            }
            // c:752 — errflag bail-out.
            if crate::ported::utils::errflag.load(Relaxed) != 0 {
                zclose(flock_fd); // c:753
                return 1; // c:754
            }
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if eno == libc::EINTR {
                continue;
            } // c:756-757
            zclose(flock_fd); // c:758
            // c:759 — `zwarnnam(nam, "failed to lock file %s: %e", args[0], errno);`
            // Format from the errno captured BEFORE zclose.
            zwarnnam(
                nam,
                &format!(
                    "failed to lock file {}: {}",
                    path,
                    crate::vm_helper::zsh_errno_msg(eno)
                ),
            );
            return 1;
        }
    }

    // c:764-765 — `if (fdvar) setiparam(fdvar, flock_fd);`
    if let Some(ref var) = fdvar {
        setiparam(var, flock_fd as i64); // c:765
    }
    0 // c:781
}

/// Port of `bin_zsystem_supports(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/system.c:781`.
///
/// C signature: `static int bin_zsystem_supports(char *nam, char **args,
///                                                 Options ops, int func)`.
///
/// Returns 0 if the named feature is supported, 1 if not, 255 on
/// argument-count error.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zsystem_supports(
    nam: &str,
    args: &[String], // c:781
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:784-787 — `if (!args[0]) ... return 255;`
    if args.is_empty() {
        zwarnnam(nam, "supports: not enough arguments");
        return 255;
    }
    // c:788-791 — `if (args[1]) ... return 255;`
    if args.len() > 1 {
        zwarnnam(nam, "supports: too many arguments");
        return 255;
    }
    // c:794 — `if (!strcmp(*args, "supports")) return 0;`
    if args[0] == "supports" {
        return 0;
    } // c:794-795
      // c:796-799 — HAVE_FCNTL_H gate; flock is universal on supported unix.
    #[cfg(unix)]
    if args[0] == "flock" {
        return 0;
    } // c:806-798
    1 // c:806
}

/// Port of `bin_zsystem(char *nam, char **args, Options ops, int func)` from `Src/Modules/system.c:806`.
///
/// C signature: `static int bin_zsystem(char *nam, char **args,
///                                       Options ops, int func)`.
/// The `zsystem` builtin dispatcher — peels the first arg and routes
/// to `bin_zsystem_flock` or `bin_zsystem_supports`.
/// WARNING: param names don't match C — Rust=(nam, args, func) vs C=(nam, args, ops, func)
pub fn bin_zsystem(
    nam: &str,
    args: &[String], // c:806
    ops: &options,
    func: i32,
) -> i32 {
    if args.is_empty() {
        zwarnnam(nam, "subcommand expected");
        return 1;
    }
    // c:809 — `if (!strcmp(*args, "flock"))`
    if args[0] == "flock" {
        return bin_zsystem_flock(nam, &args[1..], ops, func); // c:810
    }
    // c:811 — `else if (!strcmp(*args, "supports"))`
    if args[0] == "supports" {
        return bin_zsystem_supports(nam, &args[1..], ops, func); // c:812
    }
    zwarnnam(nam, &format!("unknown subcommand: {}", args[0])); // c:814
    1 // c:815
}

// ---------------------------------------------------------------------------
// Special-parameter callbacks (errnos + sysparams).
// ---------------------------------------------------------------------------

/// Port of `errnosgetfn(UNUSED(Param pm))` from `Src/Modules/system.c:832`. The
/// getter for the `${errnos}` special array. C body returns
/// `arrdup((char **)sys_errnames)` — a fresh duplicate of the
/// errno-name table. Rust port returns the names as `Vec<String>`.
///
/// Port of `static char **errnosgetfn(Param pm)` from `Src/Modules/system.c:832`.
pub fn errnosgetfn(_pm: *mut crate::ported::zsh_h::param) -> Vec<String> {
    // c:832
    SYS_ERRNAMES.iter().map(|(n, _)| n.to_string()).collect() // c:846 arrdup
}

/// Port of `fillpmsysparams(Param pm, const char *name)` from `Src/Modules/system.c:846`.
/// Populates a synthesised Param node for one of the three
/// `${sysparams[NAME]}` keys: `pid` / `ppid` / `procsubstpid`.
///
/// C signature: `static void fillpmsysparams(Param pm, const char *name)`.
/// Rust port returns the rendered string (or None for PM_UNSET) since
/// zshrs's magic-assoc dispatcher reads the value directly.
/// WARNING: param names don't match C — Rust=(name) vs C=(pm, name)
pub fn fillpmsysparams(name: &str) -> Option<String> {
    // c:846
    // Faithful port of c:854-867:
    //   if (!strcmp(name, 'pid')) num = (int)getpid();
    //   else if (!strcmp(name, 'ppid')) num = (int)getppid();
    //   else if (!strcmp(name, 'procsubstpid')) num = (int)procsubstpid;
    //   else { pm->u.str = ''; pm->node.flags |= PM_UNSET; return; }
    //   sprintf(buf, '%d', num); pm->u.str = dupstring(buf);
    //
    // Prior port hardcoded procsubstpid=0 — \$sysparams[procsubstpid]
    // always read 0 even after a process substitution fired. The
    // canonical procsubstpid lives in exec.rs as an AtomicI32 (c:220
    // port) and gets stamped at every <(...) / >(...) invocation
    // (c:5092 / c:5143). Read it directly.
    let num: i32 = match name {
        "pid" => unsafe { libc::getpid() },   // c:854-855
        "ppid" => unsafe { libc::getppid() }, // c:856-857
        // c:858-859 — `procsubstpid` from the live atomic so the
        // value matches what was last assigned by the exec-side
        // process-substitution code.
        "procsubstpid" => {
            crate::ported::exec::procsubstpid.load(std::sync::atomic::Ordering::Relaxed)
        }
        _ => return None, // c:861-863 PM_UNSET
    };
    Some(format!("{}", num)) // c:866-867 sprintf %d
}

/// Port of `static HashNode getpmsysparams(UNUSED(HashTable ht), const char *name)`
/// from `Src/Modules/system.c:873-883`. Returns a synthesised Param
/// with u_str set via fillpmsysparams, or PM_UNSET when name isn't
/// pid/ppid/procsubstpid.
pub fn getpmsysparams(
    _ht: *mut crate::ported::zsh_h::HashTable,
    name: &str,
) -> Option<crate::ported::zsh_h::Param> {
    // c:873
    use crate::ported::zsh_h::{hashnode, param, Param, PM_READONLY, PM_SCALAR, PM_UNSET};
    let mk = |s: String, extra: i32| -> Param {
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: PM_SCALAR as i32 | PM_READONLY as i32 | extra,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(s),
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        })
    };
    // c:879 — `fillpmsysparams(pm, name)`. Wrap the rendered value
    // in a Param; PM_UNSET when name didn't match a known key.
    match fillpmsysparams(name) {
        Some(v) => Some(mk(v, 0)),
        None => Some(mk(String::new(), PM_UNSET as i32)),
    }
}

/// Port of `static void scanpmsysparams(UNUSED(HashTable ht), ScanFunc func, int flags)`
/// from `Src/Modules/system.c:885-895`. Walks the three fixed keys
/// (pid/ppid/procsubstpid) and invokes the callback with a transient
/// Param per entry.
pub fn scanpmsysparams(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ScanFunc>,
    flags: i32,
) {
    // c:885
    use crate::ported::zsh_h::{hashnode, param, PM_READONLY, PM_SCALAR};
    let f = match func {
        Some(f) => f,
        None => return,
    };
    for n in ["pid", "ppid", "procsubstpid"] {
        if let Some(v) = fillpmsysparams(n) {
            let pm = param {
                node: hashnode {
                    next: None,
                    nam: n.to_string(),
                    flags: PM_SCALAR as i32 | PM_READONLY as i32,
                },
                u_data: 0,
                u_tied: None,
                u_arr: None,
                u_str: Some(v),
                u_val: 0,
                u_dval: 0.0,
                u_hash: None,
                gsu_s: None,
                gsu_i: None,
                gsu_f: None,
                gsu_a: None,
                gsu_h: None,
                base: 0,
                width: 0,
                env: None,
                ename: None,
                old: None,
                level: 0,
            };
            let node_box = Box::new(pm.node.clone());
            f(&node_box, flags); // c:891 / c:893 func call
        }
    }
}

// ---------------------------------------------------------------------------
// Module loaders.
// ---------------------------------------------------------------------------

// =====================================================================
// static struct features module_features                            c:910 (system.c)
// =====================================================================

// `bintab` — port of `static struct builtin bintab[]` (system.c).

// `mftab` — port of `static struct mathfunc mftab[]` (system.c).

// `partab` — port of `static struct paramdef partab[]` (system.c).

// `module_features` — port of `static struct features module_features`
// from system.c:910.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/system.c:920`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:920
    // C body c:922-923 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/system.c:927`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0 // c:942
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/system.c:935`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables) // c:942
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/system.c:942`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:942
    // C body c:944-945 — `return 0`. Faithful empty-body port; the
    //                    syserror/sysread/syswrite/zsystem builtins
    //                    register via the bn_list feature dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/system.c:950`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None) // c:957
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/system.c:957`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:957
    // C body c:959-960 — `return 0`. Faithful empty-body port; the
    //                    builtins unregister via cleanup_'s setfeatureenables.
    0
}

// ---------------------------------------------------------------------------
// `sys_errnames[]` table — port of `Src/Modules/errnames.c:9` (which
// is auto-generated at build time by `Src/Modules/errnames2.awk`
// from the platform's `<errno.h>`).
//
// PORT.md ABSOLUTE FREEZE forbids creating `src/ported/modules/
// errnames.rs`, so the table lives in this file. Other modules
// (`fusevm_bridge`, `params`, `parameter`) read it via
// `crate::modules::system::SYS_ERRNAMES`. The previous table name
// `ERRNO_NAMES` is kept as an alias for those existing call sites.
// ---------------------------------------------------------------------------

/// Linux errno table — kernel order, sourced from
/// `<asm-generic/errno.h>` + `<asm-generic/errno-base.h>`.
#[cfg(target_os = "linux")]
pub static SYS_ERRNAMES: &[(&str, i32)] = &[
    ("EPERM", 1),
    ("ENOENT", 2),
    ("ESRCH", 3),
    ("EINTR", 4),
    ("EIO", 5),
    ("ENXIO", 6),
    ("E2BIG", 7),
    ("ENOEXEC", 8),
    ("EBADF", 9),
    ("ECHILD", 10),
    ("EAGAIN", 11),
    ("ENOMEM", 12),
    ("EACCES", 13),
    ("EFAULT", 14),
    ("ENOTBLK", 15),
    ("EBUSY", 16),
    ("EEXIST", 17),
    ("EXDEV", 18),
    ("ENODEV", 19),
    ("ENOTDIR", 20),
    ("EISDIR", 21),
    ("EINVAL", 22),
    ("ENFILE", 23),
    ("EMFILE", 24),
    ("ENOTTY", 25),
    ("ETXTBSY", 26),
    ("EFBIG", 27),
    ("ENOSPC", 28),
    ("ESPIPE", 29),
    ("EROFS", 30),
    ("EMLINK", 31),
    ("EPIPE", 32),
    ("EDOM", 33),
    ("ERANGE", 34),
    ("EDEADLK", 35),
    ("ENAMETOOLONG", 36),
    ("ENOLCK", 37),
    ("ENOSYS", 38),
    ("ENOTEMPTY", 39),
    ("ELOOP", 40),
];

/// macOS errno table — Apple's `<sys/errno.h>` (Homebrew/older-SDK shape).
#[cfg(target_os = "macos")]
pub static SYS_ERRNAMES: &[(&str, i32)] = &[
    ("EPERM", 1),
    ("ENOENT", 2),
    ("ESRCH", 3),
    ("EINTR", 4),
    ("EIO", 5),
    ("ENXIO", 6),
    ("E2BIG", 7),
    ("ENOEXEC", 8),
    ("EBADF", 9),
    ("ECHILD", 10),
    ("EDEADLK", 11),
    ("ENOMEM", 12),
    ("EACCES", 13),
    ("EFAULT", 14),
    ("ENOTBLK", 15),
    ("EBUSY", 16),
    ("EEXIST", 17),
    ("EXDEV", 18),
    ("ENODEV", 19),
    ("ENOTDIR", 20),
    ("EISDIR", 21),
    ("EINVAL", 22),
    ("ENFILE", 23),
    ("EMFILE", 24),
    ("ENOTTY", 25),
    ("ETXTBSY", 26),
    ("EFBIG", 27),
    ("ENOSPC", 28),
    ("ESPIPE", 29),
    ("EROFS", 30),
    ("EMLINK", 31),
    ("EPIPE", 32),
    ("EDOM", 33),
    ("ERANGE", 34),
    ("EAGAIN", 35),
    ("EINPROGRESS", 36),
    ("EALREADY", 37),
    ("ENOTSOCK", 38),
    ("EDESTADDRREQ", 39),
    ("EMSGSIZE", 40),
    ("EPROTOTYPE", 41),
    ("ENOPROTOOPT", 42),
    ("EPROTONOSUPPORT", 43),
    ("ESOCKTNOSUPPORT", 44),
    ("ENOTSUP", 45),
    ("EPFNOSUPPORT", 46),
    ("EAFNOSUPPORT", 47),
    ("EADDRINUSE", 48),
    ("EADDRNOTAVAIL", 49),
    ("ENETDOWN", 50),
    ("ENETUNREACH", 51),
    ("ENETRESET", 52),
    ("ECONNABORTED", 53),
    ("ECONNRESET", 54),
    ("ENOBUFS", 55),
    ("EISCONN", 56),
    ("ENOTCONN", 57),
    ("ESHUTDOWN", 58),
    ("ETOOMANYREFS", 59),
    ("ETIMEDOUT", 60),
    ("ECONNREFUSED", 61),
    ("ELOOP", 62),
    ("ENAMETOOLONG", 63),
    ("EHOSTDOWN", 64),
    ("EHOSTUNREACH", 65),
    ("ENOTEMPTY", 66),
    ("EPROCLIM", 67),
    ("EUSERS", 68),
    ("EDQUOT", 69),
    ("ESTALE", 70),
    ("EREMOTE", 71),
    ("EBADRPC", 72),
    ("ERPCMISMATCH", 73),
    ("EPROGUNAVAIL", 74),
    ("EPROGMISMATCH", 75),
    ("EPROCUNAVAIL", 76),
    ("ENOLCK", 77),
    ("ENOSYS", 78),
    ("EFTYPE", 79),
    ("EAUTH", 80),
    ("ENEEDAUTH", 81),
    ("EPWROFF", 82),
    ("EDEVERR", 83),
    ("EOVERFLOW", 84),
    ("EBADEXEC", 85),
    ("EBADARCH", 86),
    ("ESHLIBVERS", 87),
    ("EBADMACHO", 88),
    ("ECANCELED", 89),
    ("EIDRM", 90),
    ("ENOMSG", 91),
    ("EILSEQ", 92),
    ("ENOATTR", 93),
    ("EBADMSG", 94),
    ("EMULTIHOP", 95),
    ("ENODATA", 96),
    ("ENOLINK", 97),
    ("ENOSR", 98),
    ("ENOSTR", 99),
    ("EPROTO", 100),
    ("ETIME", 101),
    ("EOPNOTSUPP", 102),
    ("ENOPOLICY", 103),
    ("ENOTRECOVERABLE", 104),
    ("EOWNERDEAD", 105),
    ("EQFULL", 106),
    ("ENOTCAPABLE", 107), // sys/errno.h:265 — ELAST on current macOS SDKs
];

/// Fallback for platforms zshrs doesn't have a verified table for —
/// the POSIX-portable subset (errnos 1-34).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub static SYS_ERRNAMES: &[(&str, i32)] = &[
    ("EPERM", 1),
    ("ENOENT", 2),
    ("ESRCH", 3),
    ("EINTR", 4),
    ("EIO", 5),
    ("ENXIO", 6),
    ("E2BIG", 7),
    ("ENOEXEC", 8),
    ("EBADF", 9),
    ("ECHILD", 10),
    ("ENOMEM", 12),
    ("EACCES", 13),
    ("EFAULT", 14),
    ("EBUSY", 16),
    ("EEXIST", 17),
    ("EXDEV", 18),
    ("ENODEV", 19),
    ("ENOTDIR", 20),
    ("EISDIR", 21),
    ("EINVAL", 22),
    ("ENFILE", 23),
    ("EMFILE", 24),
    ("ENOTTY", 25),
    ("EFBIG", 27),
    ("ENOSPC", 28),
    ("ESPIPE", 29),
    ("EROFS", 30),
    ("EMLINK", 31),
    ("EPIPE", 32),
    ("EDOM", 33),
    ("ERANGE", 34),
];

/// Back-compat alias: pre-rewrite call sites in `fusevm_bridge`,
/// `params`, and `parameter` reference the table as `ERRNO_NAMES`.
/// New code should use `SYS_ERRNAMES` (matches the C identifier).
pub static ERRNO_NAMES: &[(&str, i32)] = SYS_ERRNAMES;

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN SYSTEM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec![
        "b:syserror".to_string(),
        "b:sysread".to_string(),
        "b:syswrite".to_string(),
        "b:sysopen".to_string(),
        "b:sysseek".to_string(),
        "b:zsystem".to_string(),
        "f:systell".to_string(),
        "p:errnos".to_string(),
        "p:sysparams".to_string(),
    ]
}

// WARNING: NOT IN SYSTEM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 9]);
    }
    0
}

// WARNING: NOT IN SYSTEM.C — Rust-only module-framework shim.
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

// WARNING: NOT IN SYSTEM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
            bn_list: None,
            bn_size: 6,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 1,
            pd_list: None,
            pd_size: 2,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::math::{mnumber, MN_INTEGER};
    use crate::zsh_h::{options, MAX_OPS};
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    /// Verifies `getposint` parses non-negative ints and rejects
    /// negatives + trailing garbage per c:51.
    #[test]
    fn getposint_basic() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("42", "test"), 42);
        assert_eq!(getposint("0", "test"), 0);
        assert_eq!(getposint("-1", "test"), -1); // negative → -1
        assert_eq!(getposint("abc", "test"), -1); // garbage → -1
    }

    /// Port of `bin_zsystem(char *nam, char **args, Options ops, int func)` from `Src/Modules/system.c:806`.
    /// Verifies `bin_zsystem_supports` per c:794-800.
    #[test]
    fn bin_zsystem_supports_self() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        assert_eq!(
            bin_zsystem_supports("zsystem", &["supports".to_string()], &ops, 0),
            0
        );
        #[cfg(unix)]
        assert_eq!(
            bin_zsystem_supports("zsystem", &["flock".to_string()], &ops, 0),
            0
        );
        assert_eq!(
            bin_zsystem_supports("zsystem", &["nosuchfeature".to_string()], &ops, 0),
            1
        );
    }

    /// Verifies `bin_zsystem_supports` arg-count guards (c:784-791).
    #[test]
    fn bin_zsystem_supports_arg_count() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        assert_eq!(bin_zsystem_supports("zsystem", &[], &ops, 0), 255);
        assert_eq!(
            bin_zsystem_supports("zsystem", &["a".to_string(), "b".to_string()], &ops, 0),
            255
        );
    }

    /// Verifies `bin_zsystem` dispatches to the right subcommand
    /// (c:809/811/814).
    #[test]
    fn bin_zsystem_dispatch() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        assert_eq!(
            bin_zsystem(
                "zsystem",
                &["supports".to_string(), "supports".to_string()],
                &ops,
                0
            ),
            0
        );
        assert_eq!(bin_zsystem("zsystem", &["unknown".to_string()], &ops, 0), 1);
        assert_eq!(bin_zsystem("zsystem", &[], &ops, 0), 1);
    }

    /// Verifies `errnosgetfn` returns the dup'd table (c:835).
    #[test]
    fn errnosgetfn_returns_table() {
        let _g = crate::test_util::global_state_lock();
        let names = errnosgetfn(std::ptr::null_mut());
        assert!(names.contains(&"EPERM".to_string()));
        assert!(names.contains(&"ENOENT".to_string()));
        assert!(names.contains(&"EINVAL".to_string()));
    }

    /// Port of `scanpmsysparams(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/system.c:885`.
    /// Verifies `fillpmsysparams` for the three known keys
    /// (c:854-862) and PM_UNSET fallback (c:861-863).
    #[test]
    fn fillpmsysparams_keys() {
        let _g = crate::test_util::global_state_lock();
        assert!(fillpmsysparams("pid").is_some());
        assert!(fillpmsysparams("ppid").is_some());
        assert!(fillpmsysparams("procsubstpid").is_some());
        assert!(fillpmsysparams("nonsense").is_none());
    }

    /// Verifies `getpmsysparams` proxies through fillpmsysparams
    /// (c:878).
    #[test]
    fn getpmsysparams_pid_set() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        let pm_pid = getpmsysparams(std::ptr::null_mut(), "pid").expect("pid Param");
        assert!(pm_pid.node.flags & PM_UNSET as i32 == 0, "pid must be set");
        let pm_bad = getpmsysparams(std::ptr::null_mut(), "nonsense").expect("Param");
        assert!(
            pm_bad.node.flags & PM_UNSET as i32 != 0,
            "unknown key PM_UNSET"
        );
    }

    /// Verifies `scanpmsysparams` yields all three known keys
    /// (c:889-894).
    #[test]
    fn scanpmsysparams_three_entries() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::Mutex;
        static KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
        KEYS.lock().unwrap().clear();
        fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
            KEYS.lock().unwrap().push(node.nam.clone());
        }
        scanpmsysparams(std::ptr::null_mut(), Some(cb), 0);
        let collected = KEYS.lock().unwrap().clone();
        assert!(collected.iter().any(|k| k == "pid"));
        assert!(collected.iter().any(|k| k == "ppid"));
        assert!(collected.iter().any(|k| k == "procsubstpid"));
    }

    fn empty_ops() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }
    /// Port of `bin_sysopen(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:319`.
    fn ops_with(args: &[(u8, &str)]) -> options {
        // ind[c] encodes "set" in low 2 bits (1 = -X, 2 = +X) plus the
        // 1-based args[] slot shifted up by 2 (per zsh.h:1412 OPT_ARG
        // macro `args[(ind[c]>>2) - 1]`). idx=0 → ind=4 (slot 1, set
        // via -), idx=1 → ind=8 (slot 2), etc.
        let mut ops = empty_ops();
        for (idx, (opt, val)) in args.iter().enumerate() {
            ops.ind[*opt as usize] = (((idx + 1) << 2) | 1) as u8;
            ops.args.push(val.to_string());
            ops.argscount = (idx + 1) as i32;
            ops.argsalloc = (idx + 1) as i32;
        }
        ops
    }

    /// Verifies `bin_syserror` writes message to errvar with prefix
    /// (c:533-536).
    #[test]
    fn bin_syserror_to_errvar_with_prefix() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_with(&[(b'e', "myerr"), (b'p', "PFX:")]);
        let r = bin_syserror("syserror", &["ENOENT".to_string()], &ops, 0);
        assert_eq!(r, 0);
        // Side-effect param flows through params::setsparam → paramtab().
        let val = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get("myerr").and_then(|p| p.u_str.clone()))
            .unwrap_or_default();
        assert!(
            val.starts_with("PFX:"),
            "expected PFX: prefix, got {:?}",
            val
        );
    }

    /// Verifies `bin_syserror` returns 2 for unknown errno name
    /// (c:527-528).
    #[test]
    fn bin_syserror_unknown_name_returns_2() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_with(&[(b'e', "myerr2")]);
        assert_eq!(
            bin_syserror("syserror", &["ENOTAREALERROR".to_string()], &ops, 0),
            2
        );
    }

    /// Port of `bin_sysopen(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:319`.
    /// Verifies `bin_sysopen` opens a file and stores fd in the
    /// named variable (c:413-414) when -u is a non-digit identifier.
    #[test]
    #[cfg(unix)]
    fn bin_sysopen_writes_fd_to_var() {
        let _g = crate::test_util::global_state_lock();
        // `assignnparam` (params.rs:4464) short-circuits with
        // `if unset(EXECOPT) { return None; }`. The Rust options table
        // doesn't pre-populate EXECOPT=true the way C's
        // createoptiontable does at shell start, so the test must do
        // it manually — same pattern as the existing tests in
        // params.rs (8212/8547/9392/9442).
        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);

        let dir = TempDir::new().unwrap();
        let p = dir.path().join("a.txt");
        let ops = ops_with(&[(b'u', "MYFD"), (b'o', "creat")]);
        // Set the -w flag manually (no arg).
        let mut ops = ops;
        ops.ind[b'w' as usize] = 1;
        let r = bin_sysopen("sysopen", &[p.to_str().unwrap().to_string()], &ops, 0);
        assert_eq!(r, 0);
        // Side-effect param flows through params::setiparam → paramtab().
        // `setiparam` (params.rs:4649) builds an `mnumber{ MN_INTEGER, .l = fd }`
        // and routes via `assignnparam` — the value lands in `u_val` (i64),
        // NOT `u_str`. The original test read `u_str`, got `""`, and
        // `parse::<i32>()` returned ParseIntError::Empty. Read u_val.
        let fd: i32 = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get("MYFD").map(|p| p.u_val as i32))
            .expect("MYFD not set by sysopen");
        assert!(fd >= 10, "movefd should lift fd to 10+, got {}", fd);
        unsafe {
            libc::close(fd);
        }
        opt_state_set("exec", saved_exec);
    }

    /// Port of `bin_sysopen(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/system.c:319`.
    /// Verifies `bin_sysseek` lseek + return-code shape (c:461-462).
    #[test]
    #[cfg(unix)]
    fn bin_sysseek_basic() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("b.txt");
        {
            let mut f = File::create(&p).unwrap();
            f.write_all(b"hello world").unwrap();
        }
        let path_c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        let fd = unsafe { libc::open(path_c.as_ptr(), libc::O_RDONLY) };
        assert!(fd >= 0);
        let ops = ops_with(&[(b'u', &fd.to_string()), (b'w', "start")]);
        let r = bin_sysseek("sysseek", &["5".to_string()], &ops, 0);
        assert_eq!(r, 0);
        unsafe {
            libc::close(fd);
        }
    }

    /// Direct test of `setiparam` writeback into `paramtab()` — pins
    /// the exact contract `bin_sysopen` relies on at c:413-414. The
    /// `assignnparam` short-circuits with `unset(EXECOPT) → return
    /// None`, so the test must set "exec" true first (same pattern as
    /// the params.rs internal tests at 8212/8547/9392).
    #[test]
    fn setiparam_writes_integer_to_paramtab() {
        let _g = crate::test_util::global_state_lock();
        let saved_exec = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);
        let name = "ZSHRS_TEST_SETIPARAM_FD_INT";
        let _ = setiparam(name, 12345);
        let val = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(name).map(|p| p.u_val));
        opt_state_set("exec", saved_exec);
        assert_eq!(
            val,
            Some(12345),
            "setiparam should put the integer in paramtab().get(name).u_val"
        );
    }

    /// Verifies `math_systell` returns lseek(SEEK_CUR) (c:478).
    #[test]
    #[cfg(unix)]
    fn math_systell_returns_lseek_cur() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("c.txt");
        {
            let mut f = File::create(&p).unwrap();
            f.write_all(b"hello world").unwrap();
        }
        let path_c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        let fd = unsafe { libc::open(path_c.as_ptr(), libc::O_RDONLY) };
        unsafe {
            libc::lseek(fd, 7, libc::SEEK_SET);
        }
        let argv = vec![mnumber {
            l: fd as i64,
            d: 0.0,
            type_: MN_INTEGER,
        }];
        let r = math_systell("systell", 1, &argv, 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 7);
        unsafe {
            libc::close(fd);
        }
    }

    // ─── zsh-corpus pins for getposint ──────────────────────────────

    /// `getposint("42", "name")` returns 42.
    #[test]
    fn system_corpus_getposint_decimal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("42", "test"), 42);
    }

    /// `getposint("0", "name")` returns 0.
    #[test]
    fn system_corpus_getposint_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("0", "test"), 0);
    }

    /// `getposint("-5", "name")` returns -1 (error per c:51).
    #[test]
    fn system_corpus_getposint_negative_returns_error() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("-5", "test"),
            -1,
            "negative input rejected per c:51"
        );
    }

    /// `getposint("abc", "name")` returns -1 (non-integer).
    #[test]
    fn system_corpus_getposint_non_numeric_returns_error() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("abc", "test"), -1);
    }

    /// `getposint("42abc", "name")` returns -1 (trailing garbage).
    #[test]
    fn system_corpus_getposint_trailing_garbage_returns_error() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("42abc", "test"),
            -1,
            "trailing non-digits rejected per c:51 eptr check"
        );
    }

    /// `getposint("")` returns 0 (zstrtol parses empty as 0,
    /// eptr is empty too, ret is 0, so neither error branch fires —
    /// matches C behavior at system.c:51).
    #[test]
    fn system_corpus_getposint_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("", "test"),
            0,
            "empty input: zstrtol returns 0, neither error branch hits"
        );
    }

    /// `getposint("1000000", "name")` returns 1000000 (large positive).
    #[test]
    fn system_corpus_getposint_large_positive() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("1000000", "test"), 1_000_000);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/system.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:45 — `getposint("0")` returns 0 (zero is non-negative, valid).
    #[test]
    fn getposint_zero_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("0", "test"), 0, "zero is valid positive int");
    }

    /// c:51 — `getposint("-5")` returns -1 (negative rejected).
    #[test]
    fn getposint_negative_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("-5", "test"),
            -1,
            "negative rejected per ret < 0 branch"
        );
    }

    /// c:51 — `getposint("12abc")` returns -1 (trailing garbage).
    #[test]
    fn getposint_trailing_garbage_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("12abc", "test"),
            -1,
            "trailing non-digit rejected per *eptr != \\0"
        );
    }

    /// c:45 — `getposint("42")` returns 42 (canonical positive int).
    #[test]
    fn getposint_canonical_positive() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("42", "test"), 42);
        assert_eq!(getposint("1", "test"), 1);
    }

    /// c:45 — `getposint` is deterministic.
    #[test]
    fn getposint_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for s in &["0", "42", "1000", "-1", "abc"] {
            let first = getposint(s, "test");
            for _ in 0..5 {
                assert_eq!(getposint(s, "test"), first, "{:?} must be pure", s);
            }
        }
    }

    /// c:1085 — `errnosgetfn` returns a vec of error name strings
    /// (errno names like "EACCES", "ENOENT").
    #[test]
    fn errnosgetfn_returns_nonempty_vec() {
        let _g = crate::test_util::global_state_lock();
        let names = errnosgetfn(std::ptr::null_mut());
        // Must contain at least the common POSIX errnos.
        assert!(!names.is_empty(), "errno table must not be empty");
    }

    /// c:1085 — errnosgetfn output should contain "EACCES" (a POSIX
    /// errno guaranteed on every Unix system).
    #[test]
    #[cfg(unix)]
    fn errnosgetfn_includes_eacces() {
        let _g = crate::test_util::global_state_lock();
        let names = errnosgetfn(std::ptr::null_mut());
        assert!(
            names.iter().any(|n| n == "EACCES"),
            "errno table must include EACCES, got {:?}",
            names
        );
    }

    /// c:1098 — `fillpmsysparams("anything")` returns Option<String>
    /// (no panic on lookup of arbitrary key).
    #[test]
    fn fillpmsysparams_does_not_panic_on_arbitrary_key() {
        let _g = crate::test_util::global_state_lock();
        let _ = fillpmsysparams("zshrs_never_real_sysparam_key");
    }

    /// c:1224 — `setup_(NULL)` returns 0 (no-op).
    #[test]
    fn system_setup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:1245-1261 — lifecycle stubs all return 0.
    #[test]
    fn system_lifecycle_stubs_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/system.c
    // c:40 getposint / c:1085 errnosgetfn / c:1098 fillpmsysparams
    // c:1118 getpmsysparams / c:1162 scanpmsysparams / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:40 — `getposint` accepts leading whitespace per strtol(3) convention.
    /// C body uses zstrtol which skips leading whitespace.
    #[test]
    fn getposint_leading_whitespace_accepted_per_strtol() {
        let _g = crate::test_util::global_state_lock();
        let r = getposint(" 5", "test");
        assert_eq!(r, 5, "strtol skips leading whitespace, parses 5");
    }

    /// c:40 — `getposint` rejects hex-prefix (only decimal allowed).
    #[test]
    fn getposint_hex_prefix_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        let r = getposint("0x10", "test");
        assert_eq!(r, -1, "hex prefix must reject");
    }

    /// c:40 — `getposint` is deterministic for arbitrary input.
    #[test]
    fn getposint_deterministic_for_any_input() {
        let _g = crate::test_util::global_state_lock();
        for input in ["42", "-1", "abc", "", "0", "999999999"] {
            let first = getposint(input, "test");
            for _ in 0..3 {
                assert_eq!(
                    getposint(input, "test"),
                    first,
                    "must be deterministic for {:?}",
                    input
                );
            }
        }
    }

    /// c:1085 — `errnosgetfn(null)` is safe.
    #[test]
    fn errnosgetfn_null_pm_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = errnosgetfn(std::ptr::null_mut());
    }

    /// c:1085 — `errnosgetfn` entries all start with 'E' (errno names).
    #[test]
    fn errnosgetfn_all_entries_start_with_e() {
        let _g = crate::test_util::global_state_lock();
        let v = errnosgetfn(std::ptr::null_mut());
        for entry in &v {
            assert!(
                entry.starts_with('E'),
                "errno name {:?} must start with 'E'",
                entry
            );
        }
    }

    /// c:1085 — `errnosgetfn` is deterministic.
    #[test]
    fn errnosgetfn_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = errnosgetfn(std::ptr::null_mut());
        for _ in 0..5 {
            assert_eq!(errnosgetfn(std::ptr::null_mut()), first);
        }
    }

    /// c:1098 — `fillpmsysparams("nothing_real")` returns None.
    #[test]
    fn fillpmsysparams_unknown_key_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = fillpmsysparams("definitely_not_a_sysparam_xyz");
        assert!(r.is_none(), "unknown key → None");
    }

    /// c:1098 — `fillpmsysparams("pid")` returns Some.
    #[test]
    fn fillpmsysparams_pid_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let r = fillpmsysparams("pid");
        assert!(r.is_some(), "pid key must resolve");
    }

    /// c:1162 — `scanpmsysparams` with None callback safe.
    #[test]
    fn scanpmsysparams_none_callback_no_panic() {
        let _g = crate::test_util::global_state_lock();
        scanpmsysparams(std::ptr::null_mut(), None, 0);
    }

    /// c:1224-1261 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn system_full_lifecycle_returns_zero_for_all() {
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

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/system.c
    // c:40 getposint / c:1085 errnosgetfn / c:1098 fillpmsysparams /
    // c:1118 getpmsysparams / c:1162 scanpmsysparams
    // ═══════════════════════════════════════════════════════════════════

    /// c:40 — `getposint` returns i32 (compile-time type pin).
    #[test]
    fn getposint_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = getposint("0", "test");
    }

    /// c:40 — `getposint("999999999999")` overflow returns -1 (per C).
    #[test]
    fn getposint_overflow_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        let r = getposint("999999999999999999999", "test");
        // C body uses strtol which clamps + sets errno; expected -1 sentinel.
        assert_eq!(r, -1, "overflow → -1");
    }

    /// c:1085 — `errnosgetfn` returns Vec<String> (compile-time pin).
    #[test]
    fn errnosgetfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = errnosgetfn(std::ptr::null_mut());
    }

    /// c:1085 — `errnosgetfn` returns non-empty vec on Unix.
    #[cfg(unix)]
    #[test]
    fn errnosgetfn_unix_non_empty() {
        let _g = crate::test_util::global_state_lock();
        let v = errnosgetfn(std::ptr::null_mut());
        assert!(!v.is_empty(), "Unix must have errnos");
    }

    /// c:1098 — `fillpmsysparams` returns Option<String> (compile-time pin).
    #[test]
    fn fillpmsysparams_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = fillpmsysparams("anything");
    }

    /// c:1098 — `fillpmsysparams("ppid")` returns Some.
    #[test]
    fn fillpmsysparams_ppid_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let r = fillpmsysparams("ppid");
        assert!(r.is_some(), "ppid is known sysparam");
    }

    /// c:1098 — `fillpmsysparams("")` empty name returns None.
    #[test]
    fn fillpmsysparams_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(fillpmsysparams("").is_none(), "empty → None");
    }

    /// c:40 — `getposint` is deterministic for negative input.
    #[test]
    fn getposint_negative_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = getposint("-42", "test");
        for _ in 0..3 {
            assert_eq!(
                getposint("-42", "test"),
                first,
                "getposint(-42) must be deterministic"
            );
        }
    }

    /// c:1162 — `scanpmsysparams(null, None, 0)` is safe (returns void).
    #[test]
    fn scanpmsysparams_returns_void_signature() {
        let _g = crate::test_util::global_state_lock();
        let _: () = scanpmsysparams(std::ptr::null_mut(), None, 0);
    }

    /// c:40 — `getposint("0", _)` returns 0 (zero is valid pos int).
    #[test]
    fn getposint_zero_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("0", "test"), 0, "0 is valid pos int");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/system.c
    // c:65 bin_sysread / c:264 bin_syswrite / c:345 bin_sysopen /
    // c:534 bin_sysseek / c:631 bin_syserror / c:718 bin_zsystem_flock /
    // c:1018 bin_zsystem_supports / c:1053 bin_zsystem +
    // c:1224-1261 lifecycle type pins
    // ═══════════════════════════════════════════════════════════════════

    fn empty_ops_sys() -> crate::ported::zsh_h::options {
        crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// c:65 — `bin_sysread` returns i32 (compile-time type pin).
    #[test]
    fn bin_sysread_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_sysread("sysread", &[], &ops, 0);
    }

    /// c:264 — `bin_syswrite` returns i32.
    #[test]
    fn bin_syswrite_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_syswrite("syswrite", &[], &ops, 0);
    }

    /// c:345 — `bin_sysopen` returns i32.
    #[test]
    fn bin_sysopen_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_sysopen("sysopen", &[], &ops, 0);
    }

    /// c:534 — `bin_sysseek` returns i32.
    #[test]
    fn bin_sysseek_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_sysseek("sysseek", &[], &ops, 0);
    }

    /// c:631 — `bin_syserror` returns i32.
    #[test]
    fn bin_syserror_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_syserror("syserror", &[], &ops, 0);
    }

    /// c:718 — `bin_zsystem_flock` returns i32.
    #[test]
    fn bin_zsystem_flock_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_zsystem_flock("zsystem", &[], &ops, 0);
    }

    /// c:1018 — `bin_zsystem_supports` returns i32.
    #[test]
    fn bin_zsystem_supports_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_zsystem_supports("zsystem", &[], &ops, 0);
    }

    /// c:1053 — `bin_zsystem` returns i32.
    #[test]
    fn bin_zsystem_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_zsystem("zsystem", &[], &ops, 0);
    }

    /// c:1053 — `bin_zsystem` with no args returns nonzero (usage error).
    #[test]
    fn bin_zsystem_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let r = bin_zsystem("zsystem", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:1018 — `bin_zsystem_supports` is deterministic for same input.
    #[test]
    fn bin_zsystem_supports_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let first = bin_zsystem_supports("zsystem", &["flock".into()], &ops, 0);
        for _ in 0..3 {
            assert_eq!(
                bin_zsystem_supports("zsystem", &["flock".into()], &ops, 0),
                first,
                "bin_zsystem_supports must be deterministic",
            );
        }
    }

    /// c:1224 — `setup_` returns i32 (compile-time type pin).
    #[test]
    fn system_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:1255 + c:1261 — cleanup/finish idempotent.
    #[test]
    fn system_cleanup_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:1232 — features non-empty + use canonical b:/p:/f:/c: prefix
    /// per zsh's module-feature naming spec (b=builtin, p=param, f=mathfunc,
    /// c=condition).
    #[test]
    fn system_features_nonempty_canonical_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        assert!(!feats.is_empty(), "system advertises ≥1 feature");
        for f in &feats {
            let ok = f.starts_with("b:")
                || f.starts_with("p:")
                || f.starts_with("f:")
                || f.starts_with("c:");
            assert!(ok, "feature {:?} must use b:/p:/f:/c: prefix", f);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/system.c
    // c:40 getposint / c:65 bin_sysread / c:264 bin_syswrite /
    // c:345 bin_sysopen / c:534 bin_sysseek / c:598 math_systell /
    // c:631 bin_syserror / c:1085 errnosgetfn
    // ═══════════════════════════════════════════════════════════════════

    /// c:40 — `getposint` returns i32 (compile-time pin).
    #[test]
    fn system_getposint_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = getposint("42", "test");
    }

    /// c:40 — `getposint("42", _)` returns 42.
    #[test]
    fn system_getposint_42_returns_42() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("42", "test"), 42, "'42' parses to 42");
    }

    /// c:40 — `getposint("garbage", _)` is non-positive.
    #[test]
    fn system_getposint_garbage_non_positive() {
        let _g = crate::test_util::global_state_lock();
        let r = getposint("garbage", "test");
        assert!(
            r <= 0,
            "garbage must return non-positive sentinel; got {}",
            r
        );
    }

    /// c:65 — `bin_sysread` no-args returns nonzero. The C source reads
    /// stdin into `REPLY` when no outvar is given, so this test asserts
    /// non-success — which is only true when stdin yields no data
    /// (EOF → rc=5). Under `cargo test` in a terminal, stdin is the
    /// inherited TTY and a read might succeed with TTY chatter → rc=0,
    /// flaking the test. Pin stdin to a closed pipe so the read
    /// deterministically returns 0 bytes → bin_sysread returns 5 (EOF).
    #[test]
    fn bin_sysread_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        // Create a pipe and close the write end so reading the read
        // end returns 0 bytes (EOF). dup2 the read end over fd 0 for
        // the duration of the call.
        let mut pipefds: [libc::c_int; 2] = [0, 0];
        let pipe_rc = unsafe { libc::pipe(pipefds.as_mut_ptr()) };
        if pipe_rc != 0 {
            // pipe(2) failed — fall back to original behavior; the
            // test will still pass if stdin happens to be empty.
            let ops = empty_ops_sys();
            let r = bin_sysread("sysread", &[], &ops, 0);
            assert_ne!(r, 0, "sysread no args → usage error");
            return;
        }
        let read_fd = pipefds[0];
        let write_fd = pipefds[1];
        unsafe { libc::close(write_fd) }; // close write → EOF on read
        let saved_stdin = unsafe { libc::dup(0) };
        unsafe { libc::dup2(read_fd, 0) };
        let ops = empty_ops_sys();
        let r = bin_sysread("sysread", &[], &ops, 0);
        // Restore stdin and clean up.
        unsafe { libc::dup2(saved_stdin, 0) };
        unsafe { libc::close(saved_stdin) };
        unsafe { libc::close(read_fd) };
        assert_ne!(r, 0, "sysread no args + closed stdin → EOF (rc=5)");
    }

    /// c:264 — `bin_syswrite` no-args returns nonzero (usage error).
    #[test]
    fn bin_syswrite_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let r = bin_syswrite("syswrite", &[], &ops, 0);
        assert_ne!(r, 0, "syswrite no args → usage error");
    }

    /// c:345 — `bin_sysopen` no-args returns nonzero (usage error).
    #[test]
    fn bin_sysopen_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let r = bin_sysopen("sysopen", &[], &ops, 0);
        assert_ne!(r, 0, "sysopen no args → usage error");
    }

    /// c:534 — `bin_sysseek` no-args returns nonzero (usage error).
    #[test]
    fn bin_sysseek_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let r = bin_sysseek("sysseek", &[], &ops, 0);
        assert_ne!(r, 0, "sysseek no args → usage error");
    }

    /// c:598 — `math_systell` MUST safely return mnumber without
    /// panicking; C source validates argc before argv[0] access.
    /// In zshrs the port indexes `argv[0]` without bounds check at c:601.
    #[test]
    fn math_systell_returns_mnumber_type() {
        let _g = crate::test_util::global_state_lock();
        let _: crate::ported::zsh_h::mnumber = math_systell("systell", 0, &[], 0);
    }

    /// c:1085 — `errnosgetfn(null)` returns Vec<String> (compile-time pin).
    #[test]
    fn errnosgetfn_null_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = errnosgetfn(std::ptr::null_mut());
    }

    /// c:1085 — `errnosgetfn(null)` is non-empty (POSIX errno table
    /// has dozens of entries).
    #[test]
    fn errnosgetfn_null_non_empty() {
        let _g = crate::test_util::global_state_lock();
        let v = errnosgetfn(std::ptr::null_mut());
        assert!(!v.is_empty(), "errnos table must have ≥1 entry on POSIX");
    }

    /// c:631 — `bin_syserror` returns i32 (compile-time pin, alt).
    #[test]
    fn bin_syserror_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_sys();
        let _: i32 = bin_syserror("syserror", &[], &ops, 0);
    }
}
