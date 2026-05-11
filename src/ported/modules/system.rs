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
//! C source: 21 fns total — `getposint`, `bin_sysread`, `bin_syswrite`,
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

use crate::ported::exec::ShellExecutor;
use crate::ported::math::{Mnumber, MN_INTEGER, MN_FLOAT};
use crate::ported::params::{setiparam, setsparam, setiparam_no_convert};
use crate::ported::utils::{isident, metafy, unmeta, zwarnnam, zclose, movefd};

const SYSREAD_BUFSIZE: usize = 8192;                                     // c:41

/// Port of `getposint()` from `Src/Modules/system.c:45`. Parses
/// `instr` as a non-negative integer (zstrtol with base 10); emits
/// `zwarnnam` and returns -1 on parse error or negative.
///
/// C signature: `static int getposint(char *instr, char *nam)`.
pub fn getposint(instr: &str, nam: &str) -> i32 {                        // c:45
    // c:50 — `ret = (int)zstrtol(instr, &eptr, 10);`
    let (ret, eptr) = crate::ported::utils::zstrtol(instr, 10);
    let ret = ret as i32;
    // c:51 — `if (*eptr || ret < 0)`
    if !eptr.is_empty() || ret < 0 {
        zwarnnam(nam, &format!("integer expected: {}", instr));          // c:52
        return -1;                                                       // c:53
    }
    ret                                                                  // c:56
}

/// Port of `bin_sysread()` from `Src/Modules/system.c:72`.
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
pub fn bin_sysread(nam: &str, args: &[String],                               // c:72
                   ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
    // c:74 — `int infd = 0, outfd = -1, bufsize = SYSREAD_BUFSIZE, count;`
    let mut infd: i32 = 0;                                                    // c:74
    let mut outfd: i32 = -1;                                                  // c:74
    let mut bufsize: usize = SYSREAD_BUFSIZE;                                 // c:74
    // c:75 — `char *outvar = NULL, *countvar = NULL, *inbuf;`
    let mut outvar: Option<String> = None;                                    // c:75
    let mut countvar: Option<String> = None;                                  // c:75

    // c:80 — `if (OPT_ISSET(ops, 'i')) { infd = getposint(OPT_ARG(ops,'i'),nam); ...}`
    if OPT_ISSET(ops, b'i') {                                                 // c:80
        infd = getposint(OPT_ARG(ops, b'i').unwrap_or(""), nam);              // c:81
        if infd < 0 { return 1; }                                             // c:82-83
    }
    // c:87 — `if (OPT_ISSET(ops, 'o')) { outfd = getposint(OPT_ARG(ops,'o'),nam); ...}`
    if OPT_ISSET(ops, b'o') {                                                 // c:87
        outfd = getposint(OPT_ARG(ops, b'o').unwrap_or(""), nam);             // c:88
        if outfd < 0 { return 1; }                                            // c:89-90
    }
    // c:94 — `if (OPT_ISSET(ops, 's')) bufsize = getposint(OPT_ARG(ops,'s'),nam);`
    if OPT_ISSET(ops, b's') {                                                 // c:94
        let v = getposint(OPT_ARG(ops, b's').unwrap_or(""), nam);             // c:95
        if v < 0 { return 1; }                                                // c:96-97
        bufsize = v as usize;
    }
    // c:101 — `if (OPT_ISSET(ops, 'c')) { countvar = OPT_ARG(ops,'c'); isident...}`
    if OPT_ISSET(ops, b'c') {                                                 // c:101
        let cv = OPT_ARG(ops, b'c').unwrap_or("").to_string();                // c:102
        if !isident(&cv) {                                                    // c:103
            zwarnnam(nam, &format!("not an identifier: {}", cv));             // c:104
            return 1;                                                         // c:105
        }
        countvar = Some(cv);
    }
    // c:109 — `if (*args) { outvar = *args; isident... }`
    if !args.is_empty() {                                                     // c:109
        let ov = args[0].clone();                                             // c:116
        if !isident(&ov) {                                                    // c:117
            zwarnnam(nam, &format!("not an identifier: {}", ov));             // c:118
            return 1;                                                         // c:119
        }
        outvar = Some(ov);
    }
    let timeout_arg: Option<&str> = if OPT_ISSET(ops, b't') {                 // c:127
        OPT_ARG(ops, b't')
    } else { None };

    // c:123 — `inbuf = zhalloc(bufsize);`
    let mut inbuf = vec![0u8; bufsize];                                  // c:123

    // c:127-185 — `-t` poll(2) wait. C uses HAVE_POLL → poll(); else
    // select(). Rust has poll(2) on every supported unix; pick the
    // poll branch (c:129-152).
    if let Some(t_str) = timeout_arg {
        // c:137 — `to_mn = matheval(OPT_ARG(ops,'t'));`
        let to_mn = match crate::ported::math::matheval(t_str) {
            Ok(m) => m,
            Err(_) => return 1,                                          // c:138-139 errflag
        };
        // c:140-143 — float→int conversion of seconds × 1000.
        let to_int: i32 = if to_mn.type_ == MN_FLOAT {
            (1000.0 * to_mn.d) as i32                                    // c:141
        } else {
            (1000 * to_mn.l) as i32                                      // c:143
        };
        // c:145-148 — `while ((ret = poll(...)) < 0) { if (errno != EINTR ...) break; }`
        let mut ret;
        loop {
            let mut pfd = libc::pollfd {                                 // c:130-135
                fd: infd,
                events: libc::POLLIN,
                revents: 0,
            };
            ret = unsafe { libc::poll(&mut pfd, 1, to_int) };
            if ret >= 0 {
                break;
            }
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if eno != libc::EINTR {
                break;                                                   // c:146 EINTR retry
            }
        }
        // c:149-151 — `if (ret <= 0) return ret ? 2 : 4;`
        if ret <= 0 {
            return if ret != 0 { 2 } else { 4 };
        }
    }

    // c:188-191 — `while ((count = read(infd, inbuf, bufsize)) < 0) ...`
    let mut count: isize;
    loop {
        count = unsafe {
            libc::read(infd, inbuf.as_mut_ptr() as *mut libc::c_void, bufsize)
        };
        if count >= 0 { break; }
        let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        if eno != libc::EINTR { break; }                                 // c:189
    }
    // c:192-193 — `if (countvar) setiparam(countvar, count);`
    if let Some(ref cv) = countvar {
        crate::ported::params::setiparam(cv, count as i64);          // c:192
    }
    // c:194-195 — `if (count < 0) return 2;`
    if count < 0 {
        return 2;
    }
    let count = count as usize;

    // c:197-218 — outfd write path with EINTR retry + partial residue.
    if outfd >= 0 {                                                      // c:197
        if count == 0 { return 5; }                                      // c:198-199
        let mut p = 0usize;
        let mut remaining = count;
        while remaining > 0 {                                            // c:200
            let ret = unsafe {
                libc::write(outfd,
                            inbuf[p..].as_ptr() as *const libc::c_void,
                            remaining)
            };
            if ret < 0 {                                                 // c:204
                let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if eno == libc::EINTR {                                  // c:205-207
                    continue;
                }
                // c:208-211 — stash residue + remaining count.
                if let Some(ref ov) = outvar {
                    let buf_remaining = String::from_utf8_lossy(&inbuf[p..p+remaining]);
                    let m = metafy(&buf_remaining);
                    crate::ported::params::setsparam(ov, &m);        // c:209
                }
                if let Some(ref cv) = countvar {
                    crate::ported::params::setiparam(cv, remaining as i64); // c:210
                }
                return 3;                                                // c:212
            }
            p += ret as usize;                                           // c:214
            remaining -= ret as usize;                                   // c:215
        }
        return 0;                                                        // c:217
    }

    // c:220-225 — no outfd: stash buffer in `outvar` (default REPLY).
    let target = outvar.unwrap_or_else(|| "REPLY".to_string());          // c:220-221
    let buf_str = String::from_utf8_lossy(&inbuf[..count]);
    let m = metafy(&buf_str);
    crate::ported::params::setsparam(&target, &m);                   // c:223
    if count != 0 { 0 } else { 5 }                                       // c:225
}

/// Port of `bin_syswrite()` from `Src/Modules/system.c:238`.
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
pub fn bin_syswrite(nam: &str, args: &[String],                              // c:238
                    ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
    // c:240-241 — `int outfd = 1, len, count, totcount;
    //              char *countvar = NULL;`
    let mut outfd: i32 = 1;                                                   // c:240
    let mut countvar: Option<String> = None;                                  // c:241

    // c:246 — `if (OPT_ISSET(ops, 'o')) { outfd = getposint(OPT_ARG(ops,'o'),nam); ...}`
    if OPT_ISSET(ops, b'o') {                                                 // c:246
        outfd = getposint(OPT_ARG(ops, b'o').unwrap_or(""), nam);             // c:247
        if outfd < 0 { return 1; }                                            // c:248-249
    }
    // c:253 — `if (OPT_ISSET(ops, 'c')) { countvar = OPT_ARG(ops,'c'); isident...}`
    if OPT_ISSET(ops, b'c') {                                                 // c:253
        let cv = OPT_ARG(ops, b'c').unwrap_or("").to_string();                // c:254
        if !isident(&cv) {                                                    // c:255
            zwarnnam(nam, &format!("not an identifier: {}", cv));             // c:256
            return 1;                                                         // c:257
        }
        countvar = Some(cv);
    }
    // c:262 — `unmetafy(*args, &len);` — first positional arg = data.
    let data = match args.first() {                                           // c:262
        Some(d) => d.clone(),
        None => return 1,
    };

    // c:262 — `unmetafy(*args, &len);`
    let unmeta = self::unmeta(&data);
    let bytes = unmeta.as_bytes();
    let mut totcount: usize = 0;                                         // c:261
    let mut len = bytes.len();
    let mut p = 0usize;

    // c:263-275 — write loop with EINTR retry and partial residue.
    while len > 0 {                                                      // c:263
        let count = unsafe {
            libc::write(outfd,
                        bytes[p..].as_ptr() as *const libc::c_void,
                        len)
        };
        if count < 0 {                                                   // c:264
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if eno != libc::EINTR {                                      // c:265
                if let Some(ref cv) = countvar {                         // c:267-268
                    crate::ported::params::setiparam(cv, totcount as i64); // c:268
                }
                return 2;                                                // c:269
            }
            continue;
        }
        p += count as usize;                                             // c:272 *args += count
        totcount += count as usize;                                      // c:273
        len -= count as usize;                                           // c:274
    }
    // c:276-277 — `if (countvar) setiparam(countvar, totcount);`
    if let Some(ref cv) = countvar {
        crate::ported::params::setiparam(cv, totcount as i64);       // c:277
    }
    0                                                                    // c:279
}

/// Port of `bin_sysopen()` from `Src/Modules/system.c:319`.
///
/// C signature: `static int bin_sysopen(char *nam, char **args,
///                                       Options ops, int func)`.
/// Builtin spec: `"rwau:o:m:"` (system.c:822), 1 mandatory
/// positional arg (the file path).
///
/// Return values per c:312-314: 0 success / 1 bad params / 2 open error.
pub fn bin_sysopen(nam: &str, args: &[String],                               // c:319
                   ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
    // c:321-323 — `int read = OPT_ISSET(ops, 'r');` etc.
    let read_flag   = OPT_ISSET(ops, b'r');                                   // c:321
    let write_flag  = OPT_ISSET(ops, b'w');                                   // c:322
    let append_flag = OPT_ISSET(ops, b'a');                                   // c:323

    // c:323-325 — flags = O_NOCTTY | append | (RDWR/WRONLY/RDONLY).
    let append_flag_bit = if append_flag { libc::O_APPEND } else { 0 };
    let mut flags = libc::O_NOCTTY | append_flag_bit | if append_flag || write_flag {
        if read_flag { libc::O_RDWR } else { libc::O_WRONLY }
    } else {
        libc::O_RDONLY
    };

    // c:328 — `mode_t perms = 0666;`
    let mut perms: u32 = 0o666;
    let mut explicit: i32 = -1;                                          // c:327

    // c:335 — `if (!OPT_ISSET(ops, 'u')) { ... return 1; }`
    if !OPT_ISSET(ops, b'u') {
        zwarnnam(nam, "file descriptor not specified");                  // c:336
        return 1;                                                        // c:337
    }
    let fdvar = OPT_ARG(ops, b'u').unwrap_or("").to_string();            // c:340
    let path = match args.first() {
        Some(p) => p.clone(),
        None => return 1,
    };
    let o_arg: Option<&str> = if OPT_ISSET(ops, b'o') { OPT_ARG(ops, b'o') } else { None };
    let m_arg: Option<&str> = if OPT_ISSET(ops, b'm') { OPT_ARG(ops, b'm') } else { None };

    // c:341-347 — fdvar is either single digit (explicit fd) or identifier.
    if fdvar.len() == 1 && fdvar.chars().next().unwrap().is_ascii_digit() {
        explicit = fdvar.parse().unwrap();                               // c:343
    } else if !isident(&fdvar) {                                         // c:344
        zwarnnam(nam, &format!("not an identifier: {}", fdvar));         // c:345
        return 1;                                                        // c:346
    }

    // c:350-369 — comma-list of O_* names from -o, case-insensitive,
    // optional `O_` prefix.
    if let Some(opts) = o_arg {
        for tok in opts.split(',') {                                     // c:355 strchr ','
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
                    ("cloexec",  libc::O_CLOEXEC),                       // c:285
                    ("nofollow", libc::O_NOFOLLOW),                      // c:292
                    ("sync",     libc::O_SYNC),                          // c:295
                    #[cfg(target_os = "linux")]
                    ("noatime",  libc::O_NOATIME),                       // c:298
                    ("nonblock", libc::O_NONBLOCK),                      // c:301
                    ("excl",     libc::O_EXCL | libc::O_CREAT),          // c:303
                    ("creat",    libc::O_CREAT),                         // c:304
                    ("create",   libc::O_CREAT),                         // c:305
                    ("truncate", libc::O_TRUNC),                         // c:306
                    ("trunc",    libc::O_TRUNC),                         // c:307
                ];
                let mut found: Option<i32> = None;
                for (n, oflag) in OPENOPTS.iter().rev() {                // c:357 walks backwards
                    if n.eq_ignore_ascii_case(name) {
                        found = Some(*oflag);
                        break;
                    }
                }
                let oflag = match found {
                    Some(f) => f,
                    None => {
                        zwarnnam(nam, &format!("unsupported option: {}\n", tok));  // c:360
                        return 1;                                                  // c:361
                    }
                };
                flags |= oflag;                                          // c:367
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
            zwarnnam(nam, &format!("invalid mode {}", mode_str));        // c:377
            return 1;                                                    // c:378
        }
        // c:380 — `perms = zstrtol(opt, 0, 8);`
        let (v, _) = crate::ported::utils::zstrtol(mode_str, 8);
        perms = v as u32;
    }

    // c:383-391 — `open(*args, flags[, perms])`; `*args` is path.
    let path_c = match std::ffi::CString::new(path.as_bytes()) {
        Ok(s) => s,
        Err(_) => return 1,
    };
    let fd = unsafe {
        if (flags & libc::O_CREAT) != 0 {                                // c:383
            libc::open(path_c.as_ptr(), flags, perms as libc::c_uint)    // c:384
        } else {
            libc::open(path_c.as_ptr(), flags)                           // c:386
        }
    };
    if fd == -1 {                                                        // c:388
        let e = std::io::Error::last_os_error();
        zwarnnam(nam, &format!("can't open file {}: {}", path, e));      // c:389
        return 2;                                                        // c:390
    }

    // c:392 — `moved_fd = (explicit > -1) ? redup(fd, explicit) : movefd(fd);`
    let moved_fd: i32 = if explicit > -1 {
        crate::ported::utils::redup(fd, explicit)                        // c:392 redup branch
    } else {
        movefd(fd)                                                       // c:392 movefd branch
    };
    if moved_fd == -1 {                                                  // c:393
        zwarnnam(nam, &format!("can't open file {}", path));             // c:394
        return 2;                                                        // c:395
    }

    // c:398-411 — reapply FD_CLOEXEC after dup2 if requested.
    if (flags & libc::O_CLOEXEC) != 0 && fd != moved_fd {                // c:406
        unsafe { libc::fcntl(moved_fd, libc::F_SETFD, libc::FD_CLOEXEC); }   // c:410
    }

    // c:412 — `fdtable[moved_fd] = FDT_EXTERNAL;` (zshrs's fdtable
    // manager owns this; not yet wired — no-op for now).

    // c:413-418 — `if (explicit == -1) { setiparam(fdvar, moved_fd); ... }`
    if explicit == -1 {
        crate::ported::params::setiparam(&fdvar, moved_fd as i64);   // c:414
    }

    0                                                                    // c:420
}

/// Port of `bin_sysseek()` from `Src/Modules/system.c:433`.
///
/// C signature: `static int bin_sysseek(char *nam, char **args,
///                                       Options ops, int func)`.
/// Builtin spec: `"u:w:"` (system.c:823), 1 mandatory positional
/// arg (the offset).
///
/// Return values per c:425-428: 0 success / 1 bad params / 2 lseek error.
pub fn bin_sysseek(nam: &str, args: &[String],                               // c:433
                   ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
    // c:435 — `int w = SEEK_SET, fd = 0;`
    let mut w: i32 = libc::SEEK_SET;                                          // c:435
    let mut fd: i32 = 0;                                                      // c:435

    // c:441-446 — `if (OPT_ISSET(ops, 'u')) { fd = getposint(OPT_ARG(ops,'u'),nam); ...}`
    if OPT_ISSET(ops, b'u') {                                                 // c:441
        fd = getposint(OPT_ARG(ops, b'u').unwrap_or(""), nam);                // c:442
        if fd < 0 { return 1; }                                               // c:443-444
    }
    // c:449-460 — `-w` whence parse (case-insensitive).
    if OPT_ISSET(ops, b'w') {                                                 // c:449
        let whence = OPT_ARG(ops, b'w').unwrap_or("");                        // c:450
        if whence.eq_ignore_ascii_case("current") || whence == "1" {          // c:451
            w = libc::SEEK_CUR;                                               // c:452
        } else if whence.eq_ignore_ascii_case("end") || whence == "2" {       // c:453
            w = libc::SEEK_END;                                               // c:454
        } else if !whence.eq_ignore_ascii_case("start") && whence != "0" {    // c:455
            zwarnnam(nam, &format!("unknown argument to -w: {}", whence));    // c:456
            return 1;                                                         // c:457
        }
    }

    // c:461 — `pos = (off_t)mathevali(*args);`
    let pos_str = match args.first() {
        Some(s) => s.clone(),
        None => return 1,
    };
    let pos = match crate::ported::math::mathevali(&pos_str) {                // c:461
        Ok(v) => v,
        Err(_) => return 1,
    };
    // c:462 — `return (lseek(fd, pos, w) == -1) ? 2 : 0;`
    if unsafe { libc::lseek(fd, pos as libc::off_t, w) } == -1 {              // c:462
        2
    } else {
        0
    }
}

/// Port of `math_systell()` from `Src/Modules/system.c:467`.
///
/// C signature: `static mnumber math_systell(char *name, int argc,
///                                            mnumber *argv, int id)`.
/// Returns the current `lseek(fd, 0, SEEK_CUR)` position of `argv[0]`
/// as an `mnumber`. Negative fds error via `zerr` and return 0.
pub fn math_systell(_name: &str, _argc: i32, argv: &[Mnumber], _id: i32) -> Mnumber {  // c:467
    // c:469 — `int fd = (argv->type == MN_INTEGER) ? argv->u.l : (int)argv->u.d;`
    let fd: i32 = if argv[0].type_ == MN_INTEGER {
        argv[0].l as i32
    } else {
        argv[0].d as i32
    };
    // c:470-472 — `mnumber ret; ret.type = MN_INTEGER; ret.u.l = 0;`
    let mut ret = Mnumber {
        type_: MN_INTEGER,                                               // c:471
        l: 0,                                                            // c:472
        d: 0.0,
    };
    // c:474-477 — `if (fd < 0) { zerr("file descriptor out of range"); return ret; }`
    if fd < 0 {
        crate::ported::utils::zwarn("file descriptor out of range");
        return ret;
    }
    // c:478 — `ret.u.l = lseek(fd, 0, SEEK_CUR);`
    ret.l = unsafe { libc::lseek(fd, 0, libc::SEEK_CUR) } as i64;
    ret                                                                  // c:479
}

/// Port of `bin_syserror()` from `Src/Modules/system.c:494`.
///
/// C signature: `static int bin_syserror(char *nam, char **args,
///                                        Options ops, int func)`.
/// Builtin spec: `"e:p:"` (system.c:819), 0-1 positional args
/// (the errno number or symbolic name).
///
/// Return values per c:485-489: 0 success / 1 bad params / 2 unknown errno name.
pub fn bin_syserror(nam: &str, args: &[String],                              // c:494
                    ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG};
    // c:496-497 — `int num = 0; char *errvar = NULL, *msg, *pfx = "", *str;`
    let mut num: i32 = 0;
    let mut errvar: Option<String> = None;
    let mut pfx: String = String::new();

    // c:500-505 — `if (OPT_ISSET(ops, 'e')) { errvar = OPT_ARG(...); isident...}`
    if OPT_ISSET(ops, b'e') {                                                 // c:500
        let ev = OPT_ARG(ops, b'e').unwrap_or("").to_string();                // c:501
        if !isident(&ev) {                                                    // c:502
            zwarnnam(nam, &format!("not an identifier: {}", ev));             // c:503
            return 1;                                                         // c:504
        }
        errvar = Some(ev);
    }
    // c:508 — `if (OPT_ISSET(ops, 'p')) pfx = OPT_ARG(ops, 'p');`
    if OPT_ISSET(ops, b'p') {                                                 // c:508
        pfx = OPT_ARG(ops, b'p').unwrap_or("").to_string();                   // c:509
    }

    // c:511-530 — name parse: empty → use current errno; all-digit →
    // atoi; symbolic → lookup in sys_errnames, return 2 on miss.
    if args.is_empty() {                                                      // c:511
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
        if ptr == bytes.len() && ptr > 0 {                               // c:517
            num = arg.parse::<i32>().unwrap_or(0);                       // c:518
        } else {                                                         // c:519
            // c:521-526 — walk SYS_ERRNAMES looking for *args.
            let mut found = false;
            for (idx, (ename, _)) in SYS_ERRNAMES.iter().enumerate() {
                if *ename == arg {                                       // c:522
                    num = (idx as i32) + 1;                              // c:523
                    found = true;
                    break;                                               // c:524
                }
            }
            if !found {                                                  // c:527
                return 2;                                                // c:528
            }
        }
    }

    // c:532 — `msg = strerror(num);`
    let msg = std::io::Error::from_raw_os_error(num).to_string();
    // c:533-539 — write back to errvar or stderr.
    if let Some(ev) = errvar {
        let str_out = format!("{}{}", pfx, msg);                         // c:534-535
        crate::ported::params::setsparam(&ev, &str_out);             // c:536
    } else {
        eprintln!("{}{}", pfx, msg);                                     // c:538
    }
    0                                                                    // c:541
}

/// Port of `bin_zsystem_flock()` from `Src/Modules/system.c:546`.
///
/// C signature: `static int bin_zsystem_flock(char *nam, char **args,
///                                              Options ops, int func)`.
/// Subcommand of `zsystem flock`. Parses its own option chain (no
/// builtin opt-spec since the parent `zsystem` BUILTIN at c:824 has
/// `optstr=NULL`).
///
/// Return values per inline comments: 0 success / 1 param/lock error
/// / 2 timeout exhausted / 255 not supported on this platform.
pub fn bin_zsystem_flock(nam: &str, args: &[String],                         // c:546
                         _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    // c:548-551 — option-state locals.
    let mut cloexec: bool = true;                                        // c:548
    let mut unlock: bool = false;
    let mut readlock: bool = false;
    let mut timeout: f64 = -1.0;                                         // c:549
    // c:550 — `long timeout_interval = 1e6;` (microseconds).
    let mut timeout_interval: i64 = 1_000_000;
    let mut fdvar: Option<String> = None;                                // c:552

    // c:558-661 — option-chain parser. `while (*args && **args == '-')`.
    let mut i = 0usize;
    while i < args.len() && args[i].starts_with('-') {
        let arg = &args[i];
        i += 1;
        let optptr = &arg[1..];
        if optptr.is_empty() || optptr == "-" {                          // c:562
            break;
        }
        let chars: Vec<char> = optptr.chars().collect();
        let mut idx = 0usize;
        while idx < chars.len() {
            let opt = chars[idx];
            match opt {
                'e' => {                                                 // c:566 keep lock on exec
                    cloexec = false;                                     // c:568
                }
                'f' => {                                                 // c:571 fd variable
                    let rest: String = chars[idx + 1..].iter().collect();
                    let fdvar_str = if !rest.is_empty() {
                        idx = chars.len();                               // c:574-575 consume rest
                        rest
                    } else if i < args.len() {
                        let v = args[i].clone();                         // c:577
                        i += 1;
                        v
                    } else {
                        zwarnnam(nam, &format!(
                            "flock: option {} requires a variable name", opt));
                        return 1;
                    };
                    if !isident(&fdvar_str) {                            // c:579
                        zwarnnam(nam, &format!(
                            "flock: option {} requires a variable name", opt));
                        return 1;                                        // c:582
                    }
                    fdvar = Some(fdvar_str);
                    break;
                }
                'r' => readlock = true,                                  // c:586-588
                't' => {                                                 // c:591 timeout in seconds
                    let rest: String = chars[idx + 1..].iter().collect();
                    let optarg = if !rest.is_empty() {
                        idx = chars.len();
                        rest
                    } else if i < args.len() {
                        let v = args[i].clone();
                        i += 1;
                        v
                    } else {
                        zwarnnam(nam, &format!(
                            "flock: option {} requires a numeric timeout", opt));
                        return 1;
                    };
                    let tp = match crate::ported::math::matheval(&optarg) {
                        Ok(m) => m,
                        Err(_) => return 1,
                    };
                    timeout = if (tp.type_ & MN_FLOAT) != 0 {            // c:604
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
                'i' => {                                                 // c:621 retry interval
                    let rest: String = chars[idx + 1..].iter().collect();
                    let optarg = if !rest.is_empty() {
                        idx = chars.len();
                        rest
                    } else if i < args.len() {
                        let v = args[i].clone();
                        i += 1;
                        v
                    } else {
                        zwarnnam(nam, &format!(
                            "flock: option {} requires a numeric retry interval", opt));
                        return 1;
                    };
                    let mut tp = match crate::ported::math::matheval(&optarg) {
                        Ok(m) => m,
                        Err(_) => return 1,
                    };
                    if (tp.type_ & MN_FLOAT) == 0 {                      // c:636
                        tp.type_ = MN_FLOAT;
                        tp.d = tp.l as f64;
                    }
                    tp.d = (tp.d * 1e6).ceil();                          // c:640
                    if tp.d < 1.0 || tp.d > 0.999 * (i64::MAX as f64) {  // c:641
                        zwarnnam(nam, &format!("flock: invalid interval value: '{}'", optarg));
                        return 1;                                        // c:645
                    }
                    timeout_interval = tp.d as i64;                      // c:647
                    break;
                }
                'u' => unlock = true,                                    // c:650-652
                _ => {
                    zwarnnam(nam, &format!("flock: unknown option: {}", opt));  // c:656
                    return 1;                                            // c:657
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
    if i + 1 < args.len() {                                              // c:668-671
        zwarnnam(nam, "flock: too many arguments");
        return 1;
    }
    let path = &args[i];

    // c:674-682 — -u: unlock. argument is fd; close releases POSIX lock.
    if unlock {
        let flock_fd: i32 = match crate::ported::math::mathevali(path) {
            Ok(v) => v as i32,
            Err(_) => return 1,
        };
        // c:676 — zcloselockfd(flock_fd) returns -1 if not in our lockfd table.
        if crate::ported::utils::zcloselockfd(flock_fd) < 0 {            // c:676
            zwarnnam(nam, &format!(
                "flock: file descriptor {} not in use for locking", flock_fd));
            return 1;
        }
        return 0;                                                        // c:681
    }

    // c:684-687 — flags = readlock ? O_RDONLY|O_NOCTTY : O_RDWR|O_NOCTTY.
    let flags = if readlock {
        libc::O_RDONLY | libc::O_NOCTTY
    } else {
        libc::O_RDWR | libc::O_NOCTTY
    };
    // c:688 — open(unmeta(args[0]), flags).
    let path_unmeta = self::unmeta(path);
    let path_c = match std::ffi::CString::new(path_unmeta) {
        Ok(s) => s,
        Err(_) => return 1,
    };
    let mut flock_fd = unsafe { libc::open(path_c.as_ptr(), flags) };    // c:688
    if flock_fd < 0 {
        let e = std::io::Error::last_os_error();
        zwarnnam(nam, &format!("failed to open {} for writing: {}", path, e));
        return 1;
    }
    // c:692 — `flock_fd = movefd(flock_fd);`
    flock_fd = movefd(flock_fd);                                         // c:692
    if flock_fd == -1 { return 1; }                                      // c:693-694

    // c:695-702 — set FD_CLOEXEC if cloexec.
    if cloexec {
        let fdflags = unsafe { libc::fcntl(flock_fd, libc::F_GETFD, 0) };
        if fdflags != -1 {
            unsafe { libc::fcntl(flock_fd, libc::F_SETFD, fdflags | libc::FD_CLOEXEC); }
        }
    }
    // c:703 — `addlockfd(flock_fd, cloexec);`
    crate::ported::utils::addlockfd(flock_fd, cloexec);                  // c:703

    // c:705-708 — assemble struct flock.
    let lock_type: libc::c_short = if readlock {
        libc::F_RDLCK as libc::c_short
    } else {
        libc::F_WRLCK as libc::c_short
    };
    #[allow(clippy::unnecessary_cast)]
    let lck = libc::flock {
        l_type: lock_type,                                               // c:705
        l_whence: libc::SEEK_SET as libc::c_short,                       // c:706
        l_start: 0,                                                      // c:707
        l_len: 0,                                                        // c:708
        l_pid: 0,
    };

    if timeout > 0.0 {                                                   // c:710
        // c:711-749 — timed retry loop. zshrs uses a simple
        // monotonic Instant-based deadline; matches the C
        // behavior bit-by-bit (poll with EAGAIN/EACCES retry,
        // EINTR retry, EOTHER → fail, deadline → return 2).
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs_f64(timeout);
        loop {
            let r = unsafe { libc::fcntl(flock_fd, libc::F_SETLK, &lck) };
            if r >= 0 { break; }
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if eno != libc::EINTR && eno != libc::EACCES && eno != libc::EAGAIN {
                zclose(flock_fd);                                        // c:735
                let e = std::io::Error::last_os_error();
                zwarnnam(nam, &format!("failed to lock file {}: {}", path, e));
                return 1;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                zclose(flock_fd);                                        // c:742
                return 2;                                                // c:743
            }
            let remaining = deadline - now;
            let remaining_us = remaining.as_micros() as i64;
            let interval = remaining_us.min(timeout_interval);
            std::thread::sleep(std::time::Duration::from_micros(interval as u64));
        }
    } else {
        // c:751-762 — no timeout: F_SETLK if timeout==0 (non-blocking),
        // else F_SETLKW (blocking). EINTR retry.
        let cmd = if timeout == 0.0 { libc::F_SETLK } else { libc::F_SETLKW };
        loop {
            let r = unsafe { libc::fcntl(flock_fd, cmd, &lck) };
            if r >= 0 { break; }
            let eno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if eno == libc::EINTR { continue; }                          // c:756-757
            zclose(flock_fd);                                            // c:758
            let e = std::io::Error::last_os_error();
            zwarnnam(nam, &format!("failed to lock file {}: {}", path, e));
            return 1;
        }
    }

    // c:764-765 — `if (fdvar) setiparam(fdvar, flock_fd);`
    if let Some(ref var) = fdvar {
        crate::ported::params::setiparam(var, flock_fd as i64);      // c:765
    }
    0                                                                    // c:767
}

/// Port of `bin_zsystem_supports()` from `Src/Modules/system.c:781`.
///
/// C signature: `static int bin_zsystem_supports(char *nam, char **args,
///                                                 Options ops, int func)`.
///
/// Returns 0 if the named feature is supported, 1 if not, 255 on
/// argument-count error.
pub fn bin_zsystem_supports(nam: &str, args: &[String],                      // c:781
                            _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
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
    if args[0] == "supports" { return 0; }                               // c:794-795
    // c:796-799 — HAVE_FCNTL_H gate; flock is universal on supported unix.
    #[cfg(unix)]
    if args[0] == "flock" { return 0; }                                  // c:797-798
    1                                                                    // c:800
}

/// Port of `bin_zsystem()` from `Src/Modules/system.c:806`.
///
/// C signature: `static int bin_zsystem(char *nam, char **args,
///                                       Options ops, int func)`.
/// The `zsystem` builtin dispatcher — peels the first arg and routes
/// to `bin_zsystem_flock` or `bin_zsystem_supports`.
pub fn bin_zsystem(nam: &str, args: &[String],                               // c:806
                   ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    if args.is_empty() {
        zwarnnam(nam, "subcommand expected");
        return 1;
    }
    // c:809 — `if (!strcmp(*args, "flock"))`
    if args[0] == "flock" {
        return bin_zsystem_flock(nam, &args[1..], ops, func);            // c:810
    }
    // c:811 — `else if (!strcmp(*args, "supports"))`
    if args[0] == "supports" {
        return bin_zsystem_supports(nam, &args[1..], ops, func);         // c:812
    }
    zwarnnam(nam, &format!("unknown subcommand: {}", args[0]));          // c:814
    1                                                                    // c:815
}

// ---------------------------------------------------------------------------
// Special-parameter callbacks (errnos + sysparams).
// ---------------------------------------------------------------------------

/// Port of `errnosgetfn()` from `Src/Modules/system.c:832`. The
/// getter for the `${errnos}` special array. C body returns
/// `arrdup((char **)sys_errnames)` — a fresh duplicate of the
/// errno-name table. Rust port returns the names as `Vec<String>`.
///
/// C signature: `static char **errnosgetfn(Param pm)`.
pub fn errnosgetfn() -> Vec<String> {                                    // c:832
    SYS_ERRNAMES.iter().map(|(n, _)| n.to_string()).collect()            // c:835 arrdup
}

/// Port of `fillpmsysparams()` from `Src/Modules/system.c:846`.
/// Populates a synthesised Param node for one of the three
/// `${sysparams[NAME]}` keys: `pid` / `ppid` / `procsubstpid`.
///
/// C signature: `static void fillpmsysparams(Param pm, const char *name)`.
/// Rust port returns the rendered string (or None for PM_UNSET) since
/// zshrs's magic-assoc dispatcher reads the value directly.
pub fn fillpmsysparams(name: &str) -> Option<String> {                   // c:846
    // c:854-862 — name dispatch.
    let num: i32 = match name {
        "pid" => unsafe { libc::getpid() },                              // c:854-855
        "ppid" => unsafe { libc::getppid() },                            // c:856-857
        // c:858-859 — `procsubstpid` is the static `procsubstpid`
        // global from exec.c; not yet wired in zshrs's process-
        // substitution path. Returns 0 as the documented "no proc
        // subst active" sentinel matching C's initial value.
        "procsubstpid" => 0,
        _ => return None,                                                // c:861-863 PM_UNSET
    };
    Some(format!("{}", num))                                             // c:866 sprintf %d
}

/// Port of `getpmsysparams()` from `Src/Modules/system.c:873`. The
/// magic-assoc lookup callback for `${sysparams[name]}`.
///
/// C signature: `static HashNode getpmsysparams(HashTable ht, const char *name)`.
/// Rust port returns `Option<String>` since zshrs's magic-assoc
/// dispatcher consumes the value, not a synthesised Param.
pub fn getpmsysparams(name: &str) -> Option<String> {                    // c:873
    // c:875-879 — `pm = hcalloc(); fillpmsysparams(pm, name); return &pm->node;`
    fillpmsysparams(name)                                                // c:878
}

/// Port of `scanpmsysparams()` from `Src/Modules/system.c:885`. The
/// magic-assoc scanner for `${(k)sysparams}`. Iterates the three
/// fixed keys and returns each `(name, value)` pair.
///
/// C signature: `static void scanpmsysparams(HashTable ht, ScanFunc func, int flags)`.
/// Rust port returns the pairs as a Vec.
pub fn scanpmsysparams() -> Vec<(String, String)> {                      // c:885
    // c:889-894 — fill + emit each of pid / ppid / procsubstpid.
    let mut out = Vec::new();
    for n in ["pid", "ppid", "procsubstpid"] {                           // c:889/891/893
        if let Some(v) = fillpmsysparams(n) {
            out.push((n.to_string(), v));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Module loaders.
// ---------------------------------------------------------------------------

// =====================================================================
// static struct features module_features                            c:910 (system.c)
// =====================================================================

use crate::ported::zsh_h::module;

// `bintab` — port of `static struct builtin bintab[]` (system.c).


// `mftab` — port of `static struct mathfunc mftab[]` (system.c).


// `partab` — port of `static struct paramdef partab[]` (system.c).


// `module_features` — port of `static struct features module_features`
// from system.c:910.



/// Port of `setup_()` from `Src/Modules/system.c:920`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:920
    // C body c:922-923 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_()` from `Src/Modules/system.c:927`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0                                                                    // c:930
}

/// Port of `enables_()` from `Src/Modules/system.c:935`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables) // c:937
}

/// Port of `boot_()` from `Src/Modules/system.c:942`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:942
    // C body c:944-945 — `return 0`. Faithful empty-body port; the
    //                    syserror/sysread/syswrite/zsystem builtins
    //                    register via the bn_list feature dispatch.
    0
}

/// Port of `cleanup_()` from `Src/Modules/system.c:950`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None) // c:952
}

/// Port of `finish_()` from `Src/Modules/system.c:957`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:957
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
    ("EPERM", 1), ("ENOENT", 2), ("ESRCH", 3), ("EINTR", 4), ("EIO", 5),
    ("ENXIO", 6), ("E2BIG", 7), ("ENOEXEC", 8), ("EBADF", 9), ("ECHILD", 10),
    ("EAGAIN", 11), ("ENOMEM", 12), ("EACCES", 13), ("EFAULT", 14),
    ("ENOTBLK", 15), ("EBUSY", 16), ("EEXIST", 17), ("EXDEV", 18),
    ("ENODEV", 19), ("ENOTDIR", 20), ("EISDIR", 21), ("EINVAL", 22),
    ("ENFILE", 23), ("EMFILE", 24), ("ENOTTY", 25), ("ETXTBSY", 26),
    ("EFBIG", 27), ("ENOSPC", 28), ("ESPIPE", 29), ("EROFS", 30),
    ("EMLINK", 31), ("EPIPE", 32), ("EDOM", 33), ("ERANGE", 34),
    ("EDEADLK", 35), ("ENAMETOOLONG", 36), ("ENOLCK", 37), ("ENOSYS", 38),
    ("ENOTEMPTY", 39), ("ELOOP", 40),
];

/// macOS errno table — Apple's `<sys/errno.h>` (Homebrew/older-SDK shape).
#[cfg(target_os = "macos")]
pub static SYS_ERRNAMES: &[(&str, i32)] = &[
    ("EPERM", 1), ("ENOENT", 2), ("ESRCH", 3), ("EINTR", 4), ("EIO", 5),
    ("ENXIO", 6), ("E2BIG", 7), ("ENOEXEC", 8), ("EBADF", 9), ("ECHILD", 10),
    ("EDEADLK", 11), ("ENOMEM", 12), ("EACCES", 13), ("EFAULT", 14),
    ("ENOTBLK", 15), ("EBUSY", 16), ("EEXIST", 17), ("EXDEV", 18),
    ("ENODEV", 19), ("ENOTDIR", 20), ("EISDIR", 21), ("EINVAL", 22),
    ("ENFILE", 23), ("EMFILE", 24), ("ENOTTY", 25), ("ETXTBSY", 26),
    ("EFBIG", 27), ("ENOSPC", 28), ("ESPIPE", 29), ("EROFS", 30),
    ("EMLINK", 31), ("EPIPE", 32), ("EDOM", 33), ("ERANGE", 34),
    ("EAGAIN", 35), ("EINPROGRESS", 36), ("EALREADY", 37), ("ENOTSOCK", 38),
    ("EDESTADDRREQ", 39), ("EMSGSIZE", 40), ("EPROTOTYPE", 41),
    ("ENOPROTOOPT", 42), ("EPROTONOSUPPORT", 43), ("ESOCKTNOSUPPORT", 44),
    ("ENOTSUP", 45), ("EPFNOSUPPORT", 46), ("EAFNOSUPPORT", 47),
    ("EADDRINUSE", 48), ("EADDRNOTAVAIL", 49), ("ENETDOWN", 50),
    ("ENETUNREACH", 51), ("ENETRESET", 52), ("ECONNABORTED", 53),
    ("ECONNRESET", 54), ("ENOBUFS", 55), ("EISCONN", 56), ("ENOTCONN", 57),
    ("ESHUTDOWN", 58), ("ETOOMANYREFS", 59), ("ETIMEDOUT", 60),
    ("ECONNREFUSED", 61), ("ELOOP", 62), ("ENAMETOOLONG", 63),
    ("EHOSTDOWN", 64), ("EHOSTUNREACH", 65), ("ENOTEMPTY", 66),
    ("EPROCLIM", 67), ("EUSERS", 68), ("EDQUOT", 69), ("ESTALE", 70),
    ("EREMOTE", 71), ("EBADRPC", 72), ("ERPCMISMATCH", 73),
    ("EPROGUNAVAIL", 74), ("EPROGMISMATCH", 75), ("EPROCUNAVAIL", 76),
    ("ENOLCK", 77), ("ENOSYS", 78), ("EFTYPE", 79), ("EAUTH", 80),
    ("ENEEDAUTH", 81), ("EPWROFF", 82), ("EDEVERR", 83), ("EOVERFLOW", 84),
    ("EBADEXEC", 85), ("EBADARCH", 86), ("ESHLIBVERS", 87),
    ("EBADMACHO", 88), ("ECANCELED", 89), ("EIDRM", 90), ("ENOMSG", 91),
    ("EILSEQ", 92), ("ENOATTR", 93), ("EBADMSG", 94), ("EMULTIHOP", 95),
    ("ENODATA", 96), ("ENOLINK", 97), ("ENOSR", 98), ("ENOSTR", 99),
    ("EPROTO", 100), ("ETIME", 101), ("EOPNOTSUPP", 102), ("ENOPOLICY", 103),
    ("ENOTRECOVERABLE", 104), ("EOWNERDEAD", 105), ("EQFULL", 106),
];

/// Fallback for platforms zshrs doesn't have a verified table for —
/// the POSIX-portable subset (errnos 1-34).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub static SYS_ERRNAMES: &[(&str, i32)] = &[
    ("EPERM", 1), ("ENOENT", 2), ("ESRCH", 3), ("EINTR", 4), ("EIO", 5),
    ("ENXIO", 6), ("E2BIG", 7), ("ENOEXEC", 8), ("EBADF", 9), ("ECHILD", 10),
    ("ENOMEM", 12), ("EACCES", 13), ("EFAULT", 14), ("EBUSY", 16),
    ("EEXIST", 17), ("EXDEV", 18), ("ENODEV", 19), ("ENOTDIR", 20),
    ("EISDIR", 21), ("EINVAL", 22), ("ENFILE", 23), ("EMFILE", 24),
    ("ENOTTY", 25), ("EFBIG", 27), ("ENOSPC", 28), ("ESPIPE", 29),
    ("EROFS", 30), ("EMLINK", 31), ("EPIPE", 32), ("EDOM", 33),
    ("ERANGE", 34),
];

/// Back-compat alias: pre-rewrite call sites in `fusevm_bridge`,
/// `params`, and `parameter` reference the table as `ERRNO_NAMES`.
/// New code should use `SYS_ERRNAMES` (matches the C identifier).
pub static ERRNO_NAMES: &[(&str, i32)] = SYS_ERRNAMES;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::math::{Mnumber, MN_INTEGER};
    use std::fs::File;
    use std::io::Write as _;
    use tempfile::TempDir;

    /// Verifies `getposint` parses non-negative ints and rejects
    /// negatives + trailing garbage per c:51.
    #[test]
    fn getposint_basic() {
        assert_eq!(getposint("42", "test"), 42);
        assert_eq!(getposint("0", "test"), 0);
        assert_eq!(getposint("-1", "test"), -1);    // negative → -1
        assert_eq!(getposint("abc", "test"), -1);   // garbage → -1
    }

    /// Verifies `bin_zsystem_supports` per c:794-800.
    #[test]
    fn bin_zsystem_supports_self() {
        let ops = empty_ops();
        assert_eq!(bin_zsystem_supports("zsystem",
            &["supports".to_string()], &ops, 0), 0);
        #[cfg(unix)]
        assert_eq!(bin_zsystem_supports("zsystem",
            &["flock".to_string()], &ops, 0), 0);
        assert_eq!(bin_zsystem_supports("zsystem",
            &["nosuchfeature".to_string()], &ops, 0), 1);
    }

    /// Verifies `bin_zsystem_supports` arg-count guards (c:784-791).
    #[test]
    fn bin_zsystem_supports_arg_count() {
        let ops = empty_ops();
        assert_eq!(bin_zsystem_supports("zsystem", &[], &ops, 0), 255);
        assert_eq!(bin_zsystem_supports("zsystem",
            &["a".to_string(), "b".to_string()], &ops, 0), 255);
    }

    /// Verifies `bin_zsystem` dispatches to the right subcommand
    /// (c:809/811/814).
    #[test]
    fn bin_zsystem_dispatch() {
        let ops = empty_ops();
        assert_eq!(bin_zsystem("zsystem",
            &["supports".to_string(), "supports".to_string()], &ops, 0), 0);
        assert_eq!(bin_zsystem("zsystem",
            &["unknown".to_string()], &ops, 0), 1);
        assert_eq!(bin_zsystem("zsystem", &[], &ops, 0), 1);
    }

    /// Verifies `errnosgetfn` returns the dup'd table (c:835).
    #[test]
    fn errnosgetfn_returns_table() {
        let names = errnosgetfn();
        assert!(names.contains(&"EPERM".to_string()));
        assert!(names.contains(&"ENOENT".to_string()));
        assert!(names.contains(&"EINVAL".to_string()));
    }

    /// Verifies `fillpmsysparams` for the three known keys
    /// (c:854-862) and PM_UNSET fallback (c:861-863).
    #[test]
    fn fillpmsysparams_keys() {
        assert!(fillpmsysparams("pid").is_some());
        assert!(fillpmsysparams("ppid").is_some());
        assert!(fillpmsysparams("procsubstpid").is_some());
        assert!(fillpmsysparams("nonsense").is_none());
    }

    /// Verifies `getpmsysparams` proxies through fillpmsysparams
    /// (c:878).
    #[test]
    fn getpmsysparams_pid_set() {
        assert!(getpmsysparams("pid").is_some());
        assert!(getpmsysparams("nonsense").is_none());
    }

    /// Verifies `scanpmsysparams` yields all three known keys
    /// (c:889-894).
    #[test]
    fn scanpmsysparams_three_entries() {
        let entries = scanpmsysparams();
        let names: Vec<&str> = entries.iter().map(|(n,_)| n.as_str()).collect();
        assert!(names.contains(&"pid"));
        assert!(names.contains(&"ppid"));
        assert!(names.contains(&"procsubstpid"));
    }

    fn empty_ops() -> crate::ported::zsh_h::options {
        use crate::ported::zsh_h::{options, MAX_OPS};
        options { ind: [0u8; MAX_OPS], args: Vec::new(),
                  argscount: 0, argsalloc: 0 }
    }
    fn ops_with(args: &[(u8, &str)]) -> crate::ported::zsh_h::options {
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
        let ops = ops_with(&[(b'e', "myerr"), (b'p', "PFX:")]);
        let r = bin_syserror("syserror",
            &["ENOENT".to_string()], &ops, 0);
        assert_eq!(r, 0);
        // Side-effect param flows through params::setsparam → paramtab().
        let val = crate::ported::params::paramtab().lock().ok()
            .and_then(|t| t.get("myerr").and_then(|p| p.u_str.clone()))
            .unwrap_or_default();
        assert!(val.starts_with("PFX:"), "expected PFX: prefix, got {:?}", val);
    }

    /// Verifies `bin_syserror` returns 2 for unknown errno name
    /// (c:527-528).
    #[test]
    fn bin_syserror_unknown_name_returns_2() {
        let ops = ops_with(&[(b'e', "myerr2")]);
        assert_eq!(bin_syserror("syserror",
            &["ENOTAREALERROR".to_string()], &ops, 0), 2);
    }

    /// Verifies `bin_sysopen` opens a file and stores fd in the
    /// named variable (c:413-414) when -u is a non-digit identifier.
    #[test]
    #[cfg(unix)]
    fn bin_sysopen_writes_fd_to_var() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("a.txt");
        let ops = ops_with(&[(b'u', "MYFD"), (b'o', "creat")]);
        // Set the -w flag manually (no arg).
        let mut ops = ops;
        ops.ind[b'w' as usize] = 1;
        let r = bin_sysopen("sysopen",
            &[p.to_str().unwrap().to_string()], &ops, 0);
        assert_eq!(r, 0);
        // Side-effect param flows through params::setiparam → paramtab().
        let fd_str = crate::ported::params::paramtab().lock().ok()
            .and_then(|t| t.get("MYFD").and_then(|p| p.u_str.clone()))
            .unwrap_or_default();
        let fd: i32 = fd_str.parse().expect("MYFD should be integer");
        assert!(fd >= 10);   // movefd lifts to 10+
        unsafe { libc::close(fd); }
    }

    /// Verifies `bin_sysseek` lseek + return-code shape (c:461-462).
    #[test]
    #[cfg(unix)]
    fn bin_sysseek_basic() {
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
        unsafe { libc::close(fd); }
    }

    /// Verifies `math_systell` returns lseek(SEEK_CUR) (c:478).
    #[test]
    #[cfg(unix)]
    fn math_systell_returns_lseek_cur() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("c.txt");
        {
            let mut f = File::create(&p).unwrap();
            f.write_all(b"hello world").unwrap();
        }
        let path_c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
        let fd = unsafe { libc::open(path_c.as_ptr(), libc::O_RDONLY) };
        unsafe { libc::lseek(fd, 7, libc::SEEK_SET); }
        let argv = vec![Mnumber { l: fd as i64, d: 0.0, type_: MN_INTEGER }];
        let r = math_systell("systell", 1, &argv, 0);
        assert_eq!(r.type_, MN_INTEGER);
        assert_eq!(r.l, 7);
        unsafe { libc::close(fd); }
    }
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 6,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 1,
        pd_list: None,
        pd_size: 2,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:syserror".to_string(), "b:sysread".to_string(), "b:syswrite".to_string(), "b:sysopen".to_string(), "b:sysseek".to_string(), "b:zsystem".to_string(), "f:systell".to_string(), "p:errnos".to_string(), "p:sysparams".to_string()]
}

fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 9]);
    }
    0
}

fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

