//! ZFTP module - port of Modules/zftp.c
//!
//! it's a TELNET based protocol, but don't think I like doing this         // c:56
//! Number of connections actually open                                      // c:210
//! zfclosing is set if zftp_close() is active                               // c:219
//! List of active sessions                                                  // c:310
//!
//! Provides a builtin FTP client for zsh.

use crate::ported::builtin::{LASTVAL, SFCONTEXT};
use crate::ported::params::getiparam;
use crate::ported::utils::{errflag, getshfunc, zwarnnam};
use crate::ported::zsh_h::{module, options, SFC_HOOK};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Port of `zftp_session(UNUSED(char *name), char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2889`.
#[allow(unused_variables)]
pub fn zftp_session(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2889
    if args.is_empty() {
        // c:2889
        // c:2892-2895 — walk zfsessions list, print each name.
        if let Ok(state) = zftp_state().lock() {
            for sess_name in state.session_names() {
                // c:2894
                println!("{}", sess_name); // c:2895
            }
        }
        return 0; // c:2896
    }
    // c:2903-2904 — no-op if already in the requested session.
    let current = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.current_name().map(|n| n.to_string()))
        .unwrap_or_default();
    if args[0] == current {
        return 0; // c:2915
    }
    savesession(); // c:2915
    switchsession(args[0]); // c:2915
    0 // c:2915
}

/// Port of `typedef struct zftp_session *Zftp_session;` from
/// `Src/Modules/zftp.c:50`. Pointer-style typedef alias used by every
/// `zftp_*` callsite that takes a session arg.
#[allow(non_camel_case_types)]
pub type Zftp_session = Box<zftp_session>;

// =====================================================================
// `struct zfheader` from `Src/Modules/zftp.c:114` — block-mode header.
// =====================================================================

/// Port of `struct zfheader` from `Src/Modules/zftp.c:114`.
/// ```c
/// struct zfheader {
///     char flags;
///     unsigned char bytes[2];
/// };
/// ```
#[allow(non_camel_case_types)]
pub struct zfheader {
    pub flags: i8,      // c:115
    pub bytes: [u8; 2], // c:116
}

// =====================================================================
// `struct zftpcmd` from `Src/Modules/zftp.c:128` — subcommand entry.
// =====================================================================

/// Port of `struct zftpcmd` from `Src/Modules/zftp.c:128`.
/// ```c
/// struct zftpcmd {
///     const char *nam;
///     int (*fun) (char *, char **, int);
///     int min, max, flags;
/// };
/// ```
#[allow(non_camel_case_types)]
pub struct zftpcmd {
    pub nam: &'static str,                  // c:129
    pub fun: fn(&str, &[&str], i32) -> i32, // c:130
    pub min: i32,                           // c:131
    /// `max` field.
    pub max: i32,
    /// `flags` field.
    pub flags: i32,
}

/// Port of `typedef struct zftpcmd *Zftpcmd` from `Src/Modules/zftp.c:151`.
#[allow(non_camel_case_types)]
pub type Zftpcmd = Box<zftpcmd>;

/// `lastcode` — file-scope global from `Src/Modules/zftp.c:228`:
/// `static int lastcode;`. Numeric form of `lastcodestr`.
pub static lastcode: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// `ZFST_TYPE(x)` macro — extract type-flag bits.
/// Port of `#define ZFST_TYPE(x) (x & ZFST_TMSK)` from
/// `Src/Modules/zftp.c`.
#[allow(non_snake_case)]
#[inline]
pub fn ZFST_TYPE(x: i32) -> i32 {
    x & ZFST_TMSK
}

/// `ZFST_MODE(x)` macro — extract mode-flag bits.
/// Port of `#define ZFST_MODE(x) (x & ZFST_MMSK)` from
/// `Src/Modules/zftp.c`.
#[allow(non_snake_case)]
#[inline]
pub fn ZFST_MODE(x: i32) -> i32 {
    x & ZFST_MMSK
}

/// Port of `struct zftp_session` from `Src/Modules/zftp.c:299`.
///
/// C definition (verbatim):
/// ```c
/// struct zftp_session {
///     char *name;            /* name of session */
///     char **params;         /* parameters ordered as in zfparams */
///     char **userparams;     /* user parameters set by zftp_params */
///     FILE *cin;             /* control input file */
///     Tcp_session control;   /* the control connection */
///     int dfd;               /* data connection */
///     int has_size;          /* understands SIZE? */
///     int has_mdtm;          /* understands MDTM? */
/// };
///
/// typedef struct zftp_session *Zftp_session;  // c:50
/// ```
///
/// Field names + order match C exactly. `cin` (control input file) is
/// modelled as `Option<TcpStream>` since Rust doesn't expose libc
/// FILE* directly; `control` (the Tcp_session) collapses into the
/// same TcpStream slot in the static-link path.
#[derive(Debug)]
#[allow(non_camel_case_types)]
pub struct zftp_session {
    pub name: String,               // c:300 char *name
    pub params: Vec<String>,        // c:301 char **params
    pub userparams: Vec<String>,    // c:302 char **userparams
    pub cin: Option<TcpStream>,     // c:303 FILE *cin (control input)
    pub control: Option<TcpStream>, // c:304 Tcp_session control
    pub dfd: i32,                   // c:305 int dfd
    pub has_size: i32,              // c:306 int has_size
    pub has_mdtm: i32,              // c:307 int has_mdtm

    // Below: ergonomic Rust fields not in C `struct zftp_session` but
    // needed by the Rust wrapper to track connection state without the
    // C `params` array indexing convention. Document the mapping back
    // to C `params[]` slots in comments.
    pub host: Option<String>, // C: params[ZFPM_HOST]
    pub port: u16,            // C: params[ZFPM_PORT] (parsed)
    pub user: Option<String>, // C: params[ZFPM_USER]
    pub pwd: Option<String>,  // C: params[ZFPM_PASSWORD]
    pub connected: bool,      // C: cin != NULL
    pub logged_in: bool,      // C: derived from greeting parse
    /// `transfer_type` field. Mirrors the ZFST_TYPE bits of
    /// `zfstatusp[zfsessno]` (c:267 `#define ZFST_TYPE(x) (x & ZFST_TMSK)`).
    /// This is the "next transfer type" the user has requested.
    pub transfer_type: i32,
    /// `current_type` field. Mirrors the ZFST_CTYP bits of
    /// `zfstatusp[zfsessno]` (c:272 `#define ZFST_CTYP(x) ((x >> ZFST_TBIT)
    /// & ZFST_TMSK)`). This is the type currently negotiated with the
    /// server. zfsettype (c:2405) sends TYPE only when transfer_type !=
    /// current_type and updates current_type after a successful response.
    pub current_type: i32,
    /// `transfer_mode` field.
    pub transfer_mode: i32,
    /// `passive` field.
    pub passive: bool,
    /// Mirrors the ZFST_SYST bit of `zfstatusp[zfsessno]` (c:240).
    /// Set after a successful SYST probe so re-login on the same
    /// session doesn't re-issue SYST.
    pub syst_probed: bool,
    /// Mirrors the ZFST_NOPS bit of `zfstatusp[zfsessno]` (c:240).
    /// Set after the server returns 5xx for PASV so subsequent
    /// `zfopendata` calls skip PASV and go directly to PORT mode.
    /// Reset to false on session re-open.
    pub nops_probed: bool,
}

/// `zfprefs` — file-scope `static int zfprefs;` from
/// Src/Modules/zftp.c:218. Bitfield of ZFPF_SNDP|ZFPF_PASV|ZFPF_DUMB.
/// Default set by boot_ (c:3206) to ZFPF_SNDP|ZFPF_PASV.
#[allow(non_upper_case_globals)]
pub static zfprefs: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:218

/// Port of `zfhandler(int sig)` from `Src/Modules/zftp.c:366`.
/// C: `static void zfhandler(int sig)` — SIGALRM handler. Sets the
/// `zfdrrrring` flag so the next zfread/zfgetline returns -1 and exits
/// its setjmp-protected critical section.
#[allow(non_snake_case)]
pub extern "C" fn zfhandler(sig: i32) {
    // c:366
    if sig == libc::SIGALRM {
        // c:368
        ZFDRRRRING.store(1, Ordering::Relaxed); // c:369
                                                // c:370-374 — errno = ETIMEDOUT (or EIO).
        unsafe {
            *errno_ptr() = libc::ETIMEDOUT;
        }
        // c:375 — longjmp(zfalrmbuf, 1). Rust port doesn't use setjmp;
        // the ZFDRRRRING flag is the timeout signal each blocking
        // read/write polls.
    }
    // c:377 DPUTS — unreachable in static-link path.
}

/// Port of `zfalarm(int tmout)` from `Src/Modules/zftp.c:384`.
/// C: `static void zfalarm(int tmout)` — set up alarm + SIGALRM handler.
#[allow(non_snake_case)]
pub fn zfalarm(tmout: i32) {
    // c:384
    ZFDRRRRING.store(0, Ordering::Relaxed); // c:384
                                            // c:387-392 — fire alarm even when tmout is 0 so a pending non-zero
                                            // main-shell alarm doesn't bleed into the FTP code path.
    if ZFALARMED.load(Ordering::Relaxed) != 0 {
        // c:393
        unsafe {
            libc::alarm(tmout as u32);
        } // c:394
        return; // c:395
    }
    // c:397 — signal(SIGALRM, zfhandler);
    unsafe {
        libc::signal(libc::SIGALRM, zfhandler as libc::sighandler_t);
    }
    // c:398 — oalremain = alarm(tmout);
    let oalremain = unsafe { libc::alarm(tmout as u32) };
    OALREMAIN.store(oalremain, Ordering::Relaxed);
    if oalremain != 0 {
        // c:399
        // c:400 — oaltime = zmonotime(NULL);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        OALTIME.store(now, Ordering::Relaxed);
    }
    ZFALARMED.store(1, Ordering::Relaxed); // c:405
}

/// Port of `zfpipe()` from `Src/Modules/zftp.c:412`.
/// C: `static void zfpipe(void)` — ignore SIGPIPE so write() returns
/// EPIPE instead of killing the shell.
#[allow(non_snake_case)]
pub fn zfpipe() {
    // c:412
    // c:412 — signal(SIGPIPE, SIG_IGN);
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

/// Port of `zfunalarm()` from Src/Modules/zftp.c:422.
/// C: `void zfunalarm(void)` — restores the prior alarm if `oalremain`
/// was nonzero, else cancels with `alarm(0)`. Adjusts for elapsed time.
#[allow(non_snake_case)]
pub fn zfunalarm() {
    // c:422
    let oalremain = OALREMAIN.load(Ordering::Relaxed); // c:422
    if oalremain != 0 {
        // c:423
        // c:432-433 — `time_t tdiff = zmonotime(NULL) - oaltime;`
        let oaltime = OALTIME.load(Ordering::Relaxed); // c:432
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let tdiff = now - oaltime; // c:432
        let secs = if (oalremain as i64) < tdiff {
            1
        } else {
            // c:434
            (oalremain as i64 - tdiff) as u32
        };
        unsafe {
            libc::alarm(secs);
        } // c:453
    } else {
        unsafe {
            libc::alarm(0);
        } // c:453
    }
}

/// Port of `zfunpipe()` from Src/Modules/zftp.c:453.
/// C: `void zfunpipe(void)` — restores the SIGPIPE disposition that
/// existed before `zfpipe()` ignored it.
#[allow(non_snake_case)]
pub fn zfunpipe() {
    // c:453
    // c:453 — `if (sigtrapped[SIGPIPE]) { ... } else signal_default(SIGPIPE);`
    // The static-link path doesn't expose `sigtrapped[]`/`siglists[]` yet,
    // so reset to default disposition unconditionally — matches the
    // common case where SIGPIPE wasn't trapped.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    } // c:460
}

/// Port of `zfmovefd(int fd)` from `Src/Modules/zftp.c:472`.
///
/// Direct line-by-line port. Bumps an fd past the reserved
/// shell-internal range (0..=9) via `fcntl(F_DUPFD, 10)` then
/// closes the original — keeps ZFTP control sockets out of the
/// user's redirection range. Returns -1 on dup failure.
#[allow(non_snake_case)]
pub fn zfmovefd(fd: i32) -> i32 {
    // c:474 — `if (fd != -1 && fd < 10) { ... }`
    if fd != -1 && fd < 10 {
        // c:476 — `int fe = fcntl(fd, F_DUPFD, 10);`
        let fe = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
        // c:480 — `close(fd);`
        unsafe {
            libc::close(fd);
        }
        // c:481 — `fd = fe;`
        return fe;
    }
    fd // c:483
}

/// Port of `zfsetparam(char *name, void *val, int flags)` from `Src/Modules/zftp.c:494`.
/// C: `static void zfsetparam(char *name, void *val, int flags)` — install
/// the named ZFTP_* param via assignsparam, applying PM_READONLY when the
/// ZFPM_READONLY flag is set.
#[allow(non_snake_case)]
pub fn zfsetparam(name: &str, val: &str, flags: i32) {
    // c:494
    // c:494 — int type = (flags & ZFPM_INTEGER) ? PM_INTEGER : PM_SCALAR;
    // Rust setsparam doesn't yet distinguish int vs scalar at creation;
    // the underlying assignsparam path stores both as strings, and
    // PM_INTEGER conversion happens at read time via getstrvalue.
    let _ = flags & ZFPM_INTEGER;

    // c:499-509 — getnode + IFUNSET / PM_UNSET handling. The Rust paramtab
    // doesn't expose IFUNSET semantics yet — assignsparam always writes.
    if (flags & ZFPM_IFUNSET) != 0 {
        // c:507
        // Only set if not currently set. Best-effort check via env lookup
        // since paramtab isn't bucket-2 consolidated for the executor.
        // c:508 — `if (pm = (Param)paramtab->getnode(paramtab, name))`.
        //          C exits if the param already exists. Check paramtab,
        //          not OS env.
        if crate::ported::params::paramtab()
            .read()
            .map_or(false, |t| t.contains_key(name))
        {
            return; // c:508-509 pm = NULL → skip
        }
    }

    // c:516-519 — pm->gsu.{i,s}->setfn(pm, val).
    // Faithful port of c:499-506: when the param doesn't exist
    // (or is PM_UNSET), createparam + apply PM_READONLY when
    // ZFPM_READONLY is set. Then call setsparam to install the
    // value.
    //
    // Prior port silently dropped the PM_READONLY flag — the param
    // got created without it, so subsequent user writes to
    // ZFTP_* (e.g. \$ZFTP_HOST after open) silently succeeded
    // instead of getting the 'read-only variable' error C emits.
    let needs_create = !crate::ported::params::paramtab()
        .read()
        .map(|t| {
            // c:499-500 — `!getnode2 || PM_UNSET`. Treat absent or
            // PM_UNSET as "create new".
            t.get(name)
                .map(|p| (p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET) != 0)
                .unwrap_or(true)
        })
        .unwrap_or(true);
    // c:497 — `int type = (flags & ZFPM_INTEGER) ? PM_INTEGER : PM_SCALAR;`
    let want_type: u32 = if (flags & ZFPM_INTEGER) != 0 {
        crate::ported::zsh_h::PM_INTEGER
    } else {
        crate::ported::zsh_h::PM_SCALAR
    };
    if needs_create {
        // c:505 — `if ((pm = createparam(name, type)) ...`
        let _ = crate::ported::params::createparam(name, want_type as i32);
        // c:505-506 — `&& (flags & ZFPM_READONLY) ...
        //              pm->node.flags |= PM_READONLY;`
        if (flags & ZFPM_READONLY) != 0 {
            if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                if let Some(pm) = tab.get_mut(name) {
                    pm.node.flags |= crate::ported::zsh_h::PM_READONLY as i32;
                }
            }
        }
    }
    // c:510-514 — `if (!pm || PM_TYPE(pm->node.flags) != type) {
    //                  if (type == PM_SCALAR) zsfree((char *)val);
    //                  return;
    //              }`
    //
    // Type-mismatch guard. After createparam (or the IFUNSET skip), the
    // resolved param may have a different PM_TYPE than `want_type` — for
    // example, the user did `typeset -i ZFTP_HOST=42` before connecting,
    // so the existing param is PM_INTEGER while zfsetparam wants
    // PM_SCALAR. C silently bails to avoid forcing a type conversion;
    // prior Rust port skipped the guard and let setsparam re-assign
    // regardless, clobbering the user's typed override.
    let actual_type = match crate::ported::params::paramtab().read() {
        Ok(t) => t
            .get(name)
            .map(|p| crate::ported::zsh_h::PM_TYPE(p.node.flags as u32)),
        Err(_) => None,
    };
    match actual_type {
        Some(t) if t == want_type => {} // proceed
        Some(_) | None => return,       // c:514 — wrong type or no param
    }
    // c:516-519 — `if (type == PM_INTEGER)
    //                  pm->gsu.i->setfn(pm, *(off_t *)val);
    //              else
    //                  pm->gsu.s->setfn(pm, (char *)val);`
    //
    // Dispatch per the resolved param type. For ZFPM_INTEGER, route
    // through setiparam so the integer is stored in `u_val` directly
    // instead of as a string parsed back to int through assignsparam.
    // Prior port always called setsparam regardless of want_type —
    // for ZFPM_INTEGER params (ZFTP_PORT, ZFTP_PID, ZFTP_SIZE, etc.)
    // the value got stored as a String in `u_str`, and `${ZFTP_PORT}`
    // reads went through the scalar getter producing the decimal
    // representation. Functionally equivalent for the read side, but
    // diverged from C's storage shape: a downstream PM_INTEGER-aware
    // consumer querying `pm->u.val` directly would see 0 instead of
    // the actual port number.
    if (flags & ZFPM_INTEGER) != 0 {
        let n = val.parse::<i64>().unwrap_or(0);
        let _ = crate::ported::params::setiparam(name, n); // c:517
    } else {
        crate::ported::params::setsparam(name, val); // c:519
    }
}

/// Port of `zfunsetparam(char *name)` from Src/Modules/zftp.c:529.
/// C: `static void zfunsetparam(char *name)` — clears PM_READONLY then
/// calls `unsetparam_pm(pm, 0, 1)`.
#[allow(non_snake_case)]
pub fn zfunsetparam(name: &str) {
    // c:529
    // Faithful port of c:531-536:
    //   if ((pm = (Param) paramtab->getnode(paramtab, name))) {
    //       pm->node.flags &= ~PM_READONLY;
    //       unsetparam_pm(pm, 0, 1);
    //   }
    //
    // Prior port called std::env::remove_var which only touches the
    // OS environment, not paramtab. ZFTP_* params live in paramtab
    // (created via createparam in zfsetparam), and many carry
    // PM_READONLY — std::env::remove_var on a PM_READONLY paramtab
    // entry would silently no-op for the shell but might clear the
    // env var ambient state, leading to divergent shell-vs-env views.
    //
    // C's empty-name lookup returns NULL (paramtab miss); Rust's
    // params::unsetparam similarly no-ops on empty.
    if name.is_empty() {
        return;
    }
    // c:531-535 — clear PM_READONLY first so the unset succeeds even
    // on readonly params (the whole point of this helper — connection
    // teardown needs to flush ZFTP_HOST etc. regardless of readonly).
    if let Ok(mut tab) = crate::ported::params::paramtab().write() {
        if let Some(pm) = tab.get_mut(name) {
            pm.node.flags &= !(crate::ported::zsh_h::PM_READONLY as i32);
        }
    }
    // c:535 — unsetparam_pm(pm, 0, 1). The free unsetparam helper
    // wraps the getnode + unsetparam_pm flow and removes from paramtab.
    crate::ported::params::unsetparam(name);
}

/// Port of `zfargstring(char *cmd, char **args)` from `Src/Modules/zftp.c:546`.
/// C: `char *zfargstring(char *cmd, char **args)` — joins cmd + args.
#[allow(non_snake_case)]
pub fn zfargstring(cmd: &str, args: &[&str]) -> String {
    // c:546-570 — zhalloc + sprintf joining cmd + space-sep args.
    let mut s = cmd.to_string();
    for a in args {
        s.push(' ');
        s.push_str(a);
    }
    s
}

/// Port of `zfgetline(char *ln, int lnsize, int tmout)` from `Src/Modules/zftp.c:571`.
/// C: `int zfgetline(char *ln, int lnsize, int tmout)` — read a single
/// CRLF-terminated line from the control connection, handling TELNET
/// IAC command escapes and SIGALRM-driven timeout.
#[allow(non_snake_case)]
pub fn zfgetline(ln: &mut [u8], lnsize: i32, tmout: i32) -> i32 {
    // c:571
    // c:573-575 — locals at function top (Rule 5).
    let mut ch: i32; // c:573 int ch
    let mut added: i32 = 0; // c:573 added
                            // c:575 — char *pcur = ln, cmdbuf[3];
    let mut pcur: usize = 0; // pointer index into ln
    let mut cmdbuf: [u8; 3] = [0; 3];

    ZCFINISH.store(0, Ordering::Relaxed); // c:577 zcfinish = 0
    let lnsize = lnsize - 1; // c:579 leave room for null
    if !ln.is_empty() {
        ln[0] = 0; // c:581 ln[0] = '\0'
    }

    // c:583-587 — setjmp guard via ZFDRRRRING flag.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        // c:583
        unsafe {
            libc::alarm(0);
        } // c:584
        zwarnnam("zftp", "timeout getting response"); // c:585
        return 6; // c:586
    }
    zfalarm(tmout); // c:588

    // c:597-678 — for (;;) read loop with TELNET IAC handling.
    let mut state = match zftp_state().lock() {
        Ok(s) => s,
        Err(_) => return 6,
    };
    let sess = match state.get_session_mut(None) {
        Some(s) => s,
        None => return 6,
    };
    let stream = match sess.cin.as_mut() {
        Some(s) => s,
        None => return 6,
    };
    let mut byte = [0u8; 1];

    'main: loop {
        // c:597 for (;;)
        // c:598 — ch = fgetc(zfsess->cin);
        ch = match stream.read(&mut byte) {
            Ok(0) => -1, // EOF
            Ok(_) => byte[0] as i32,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue, // c:602 EINTR retry
            Err(_) => -1,
        };

        match ch {
            -1 => {
                // c:601 EOF
                ZCFINISH.store(2, Ordering::Relaxed); // c:606
            }
            0x0d => {
                // c:609 '\r'
                ch = match stream.read(&mut byte) {
                    // c:611
                    Ok(0) => -1,
                    Ok(_) => byte[0] as i32,
                    Err(_) => -1,
                };
                if ch == -1 {
                    // c:612 EOF
                    ZCFINISH.store(2, Ordering::Relaxed); // c:613
                } else if ch == 0x0a {
                    // c:616 '\n'
                    ZCFINISH.store(1, Ordering::Relaxed); // c:617
                } else if ch == 0x00 {
                    // c:620 '\0'
                    ch = 0x0d; // c:621
                } else {
                    ch = 0x0d; // c:625
                }
            }
            0x0a => {
                // c:628 '\n' (unexpected)
                ZCFINISH.store(1, Ordering::Relaxed); // c:630
            }
            255 => {
                // c:633 IAC
                ch = match stream.read(&mut byte) {
                    // c:638
                    Ok(0) => -1,
                    Ok(_) => byte[0] as i32,
                    Err(_) => -1,
                };
                match ch {
                    251 | 252 => {
                        // c:640-641 WILL/WONT
                        ch = match stream.read(&mut byte) {
                            // c:642
                            Ok(0) => -1,
                            Ok(_) => byte[0] as i32,
                            Err(_) => -1,
                        };
                        cmdbuf[0] = 255; // c:644 IAC
                        cmdbuf[1] = 254; // c:645 DONT
                        cmdbuf[2] = ch as u8; // c:646
                                              // c:647 — write_loop(zfsess->control->fd, cmdbuf, 3);
                        if let Some(ctrl) = sess.control.as_mut() {
                            let _ = ctrl.write_all(&cmdbuf);
                        }
                        continue 'main; // c:648
                    }
                    253 | 254 => {
                        // c:650-651 DO/DONT
                        ch = match stream.read(&mut byte) {
                            // c:652
                            Ok(0) => -1,
                            Ok(_) => byte[0] as i32,
                            Err(_) => -1,
                        };
                        cmdbuf[0] = 255; // c:654 IAC
                        cmdbuf[1] = 252; // c:655 WONT
                        cmdbuf[2] = ch as u8; // c:656
                        if let Some(ctrl) = sess.control.as_mut() {
                            let _ = ctrl.write_all(&cmdbuf);
                        }
                        continue 'main; // c:658
                    }
                    -1 => {
                        // c:660 EOF
                        ZCFINISH.store(2, Ordering::Relaxed);
                        // c:662
                    }
                    _ => {} // c:665 default
                }
            }
            _ => {}
        }

        // c:671-672 — if (zcfinish) break;
        if ZCFINISH.load(Ordering::Relaxed) != 0 {
            break;
        }
        // c:673-676 — if (added < lnsize) { *pcur++ = ch; added++; }
        if added < lnsize && pcur < ln.len() {
            ln[pcur] = ch as u8;
            pcur += 1;
            added += 1;
        }
        // c:677 — junk if no room, keep reading.
    }

    unsafe {
        libc::alarm(0);
    } // c:680
    if pcur < ln.len() {
        ln[pcur] = 0; // c:702 *pcur = '\0'
    }
    // c:583 setjmp counterpart — alarm fired during the read loop.
    // ZFDRRRRING was zeroed inside zfalarm; non-zero now means
    // zfhandler set it during the blocked fgetc. Mirrors the
    // post-syscall fix added to zfread (cfe7560f58) and zfwrite
    // (00c1c36dc9). Without it, a SIGALRM during line read drained
    // through to ZCFINISH unaffected — the caller saw a partial
    // line with the timeout signal silently lost.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        zwarnnam("zftp", "timeout getting response"); // c:585
        return 6; // c:586
    }
    // c:702 — return (zcfinish & 2);
    ZCFINISH.load(Ordering::Relaxed) & 2
}

/// Port of `zfgetmsg()` from `Src/Modules/zftp.c:702`.
/// C: `static int zfgetmsg(void)` — read a complete FTP server reply
/// (possibly multi-line), parse the 3-digit code, update lastcode +
/// lastcodestr + lastmsg + ZFTP_REPLY, return the first-digit status
/// (1/2/3/4/5) or 6 on error/disconnect.
#[allow(non_snake_case)]
pub fn zfgetmsg() -> i32 {
    // c:702
    // c:702-705 — char line[256], *ptr, *verbose;
    //             int stopit, printing = 0, tmout;
    let mut line = [0u8; 256];
    let mut printing: i32 = 0;
    let stopit_initial: bool;
    let tmout: i32;

    // c:707-708 — if (!zfsess->control) return 6;
    {
        let state = match zftp_state().lock() {
            Ok(s) => s,
            Err(_) => return 6,
        };
        let sess = match state.get_session(None) {
            Some(s) => s,
            None => return 6,
        };
        if sess.control.is_none() {
            return 6; // c:708
        }
    }

    // c:709-710 — zsfree(lastmsg); lastmsg = NULL;
    if let Ok(mut m) = lastmsg.lock() {
        m.clear();
    }

    // c:712 — tmout = getiparam("ZFTP_TMOUT");
    // c:712 — `tmout = getiparam("ZFTP_TMOUT");`. Read paramtab, not OS env.
    tmout = getiparam("ZFTP_TMOUT") as i32;

    // c:714 — zfgetline(line, 256, tmout);
    zfgetline(&mut line, 256, tmout);
    // c:715 — ptr = line; (use string slice + offset index instead)
    let mut ptr_off: usize = 0;
    let line_str = std::str::from_utf8(&line)
        .unwrap_or("")
        .trim_end_matches('\0')
        .to_string();

    // c:716 — if (zfdrrrring || !idigit(ptr[0..3])) — timeout or not FTP.
    let is_digit = |b: u8| b.is_ascii_digit();
    let timeout_or_bad = ZFDRRRRING.load(Ordering::Relaxed) != 0
        || line.len() < 3
        || !is_digit(line[0])
        || !is_digit(line[1])
        || !is_digit(line[2]);
    if timeout_or_bad {
        // c:716
        ZCFINISH.store(2, Ordering::Relaxed); // c:718
        if ZFCLOSING.load(Ordering::Relaxed) == 0 {
            // c:719
            zfclose(0); // c:720
        }
        if let Ok(mut m) = lastmsg.lock() {
            m.clear();
        } // c:721
        if let Ok(mut cs) = lastcodestr.lock() {
            // c:722
            cs.copy_from_slice(b"000\0");
        }
        zfsetparam("ZFTP_REPLY", "", ZFPM_READONLY); // c:723
        return 6; // c:724
    }

    // c:726-729 — extract first 3 bytes into lastcodestr, parse to int.
    let code_str: String = std::str::from_utf8(&line[..3]).unwrap_or("0").to_string();
    if let Ok(mut cs) = lastcodestr.lock() {
        cs[0] = line[0];
        cs[1] = line[1];
        cs[2] = line[2];
        cs[3] = 0;
    }
    let code: i32 = code_str.parse().unwrap_or(0);
    lastcode.store(code, Ordering::Relaxed);
    ptr_off += 3;
    // c:730 — zfsetparam("ZFTP_CODE", lastcodestr, ZFPM_READONLY);
    zfsetparam("ZFTP_CODE", &code_str, ZFPM_READONLY);
    // c:731 — stopit = (*ptr++ != '-');
    stopit_initial = line.get(ptr_off).copied() != Some(b'-');
    ptr_off += 1;
    let mut stopit = stopit_initial;

    // c:733-744 — verbose check + initial-line printing.
    // c:734 — `if (!(verbose = getsparam_u("ZFTP_VERBOSE"))) verbose = "";`
    //   — the `_u` variant strips Meta-escape encoding. ZFTP_VERBOSE
    //   is a string of digit characters (e.g. "045") that gets fed
    //   to strchr against `lastcodestr[0]` at c:736 and against '0'
    //   at c:740 — strchr operates on raw bytes, not metafied form.
    //   Prior port used non-_u getsparam — for any ZFTP_VERBOSE
    //   value containing a Meta-escaped byte (rare but possible
    //   via `$'\xfd'` etc.), the metafied byte pairs would survive
    //   into the strchr-equivalent `.contains()` calls below and
    //   miss matches.
    let verbose =
        crate::ported::params::getsparam_u("ZFTP_VERBOSE").unwrap_or_default(); // c:734
    if verbose.contains(line[0] as char) {
        // c:736
        printing = 1; // c:738
        eprint!("{}", line_str); // c:739
    } else if verbose.contains('0') && !stopit {
        // c:740
        printing = 2; // c:742
        eprint!("{}", &line_str[ptr_off..]); // c:743
    }
    if printing != 0 {
        // c:746
        eprintln!(); // c:747
    }

    // c:749-775 — multi-line continuation loop.
    while ZCFINISH.load(Ordering::Relaxed) != 2 && !stopit {
        line.fill(0); // reset
        ptr_off = 0;
        zfgetline(&mut line, 256, tmout); // c:750
        if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
            // c:752
            line[0] = 0; // c:753
            break; // c:754
        }
        // c:757-764 — code-prefix check.
        if &line[..3] == &code_str.as_bytes()[..3] {
            // c:757
            if line[3] == b' ' {
                // c:758
                stopit = true; // c:759
                ptr_off = 4; // c:760
            } else if line[3] == b'-' {
                // c:761
                ptr_off = 4; // c:762
            }
        } else if &line[..4] == b"    " {
            // c:763
            ptr_off = 4; // c:764
        }

        // c:766-774 — print intermediate line per `printing` mode.
        let cont_line = std::str::from_utf8(&line)
            .unwrap_or("")
            .trim_end_matches('\0');
        if printing == 2 {
            // c:766
            if !stopit {
                // c:767
                eprintln!("{}", &cont_line[ptr_off..]); // c:768-769
            }
        } else if printing != 0 {
            // c:771
            eprintln!("{}", cont_line); // c:772-773
        }
    }

    // c:777-778 — fflush(stderr);
    if printing != 0 {
        let _ = io::stderr().flush();
    }

    // c:781 — lastmsg = ztrdup(ptr);  (the trailing portion of last line)
    let last_msg_str: String = std::str::from_utf8(&line)
        .unwrap_or("")
        .trim_end_matches('\0')
        .chars()
        .skip(ptr_off)
        .collect();
    if let Ok(mut m) = lastmsg.lock() {
        *m = last_msg_str.clone();
    }
    // c:785 — zfsetparam("ZFTP_REPLY", ztrdup(line), ZFPM_READONLY);
    let whole_line = std::str::from_utf8(&line)
        .unwrap_or("")
        .trim_end_matches('\0');
    zfsetparam("ZFTP_REPLY", whole_line, ZFPM_READONLY);

    // c:791-797 — EOF or 421: close + warn.
    let zcfin = ZCFINISH.load(Ordering::Relaxed);
    let cur_code = lastcode.load(Ordering::Relaxed);
    if (zcfin == 2 || cur_code == 421) && ZFCLOSING.load(Ordering::Relaxed) == 0 {
        ZCFINISH.store(2, Ordering::Relaxed); // c:792
        zfclose(0); // c:793
        zwarnnam(
            "zftp", // c:795
            "remote server has closed connection",
        );
        return 6; // c:796
    }
    // c:798-801 — 530 not-logged-in.
    if cur_code == 530 {
        // c:798
        return 6; // c:800
    }
    // c:807-810 — 120 wait-and-retry.
    if cur_code == 120 {
        // c:807
        zwarnnam(
            "zftp", // c:808
            &format!("delay expected, waiting: {}", last_msg_str),
        );
        return zfgetmsg(); // c:809
    }
    // c:813 — return lastcodestr[0] - '0';
    (code_str.as_bytes()[0] - b'0') as i32
}

/// Port of `zfsendcmd(char *cmd)` from `Src/Modules/zftp.c:825`.
/// C: `static int zfsendcmd(char *cmd)` — write the command to the
/// control fd with an alarm-guarded timeout, then read the server
/// reply via zfgetmsg.
#[allow(non_snake_case)]
pub fn zfsendcmd(cmd: &str) -> i32 {
    // c:825
    // c:832 — int ret, tmout;
    let ret: isize;
    let tmout: i32;

    // c:834-835 — if (!zfsess->control) return 6;
    let mut state = match zftp_state().lock() {
        Ok(s) => s,
        Err(_) => return 6,
    };
    let sess = match state.get_session_mut(None) {
        // c:834
        Some(s) => s,
        None => return 6,
    };
    if sess.control.is_none() {
        // c:834
        return 6; // c:835
    }

    // c:836 — tmout = getiparam("ZFTP_TMOUT");
    // c:712 — `tmout = getiparam("ZFTP_TMOUT");`. Read paramtab, not OS env.
    tmout = getiparam("ZFTP_TMOUT") as i32;

    // c:837-841 — `if (setjmp(zfalrmbuf)) { alarm(0);
    //                  zwarnnam("zftp", "timeout sending message");
    //                  return 6; }`. ZFDRRRRING adapter same as
    // zfread (cfe7560f58), zfwrite (00c1c36dc9), zfgetline (bef84af815).
    // Two check sites match C's setjmp semantics: before alarm
    // install + after write returns.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        // Stale alarm from prior call.
        unsafe {
            libc::alarm(0);
        }
        zwarnnam("zftp", "timeout sending message");
        return 6;
    }
    zfalarm(tmout); // c:842

    // c:843 — `ret = write(zfsess->control->fd, cmd, strlen(cmd));`
    let bytes = cmd.as_bytes();
    ret = match sess.control.as_mut() {
        Some(stream) => match stream.write(bytes) {
            Ok(n) => {
                let _ = stream.flush();
                n as isize
            }
            Err(_) => -1,
        },
        None => -1,
    };
    // c:844 — `alarm(0);`
    unsafe {
        libc::alarm(0);
    }

    // c:837 setjmp counterpart for THIS call — alarm fired during write.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        zwarnnam("zftp", "timeout sending message"); // c:839
        return 6; // c:840
    }

    // c:846-849 — write failure.
    if ret <= 0 {
        zwarnnam(
            // c:847
            "zftp send",
            &format!(
                "failure sending control message: {}",
                io::Error::last_os_error()
            ),
        );
        return 6; // c:848
    }

    // c:851 — return zfgetmsg();
    drop(state);
    zfgetmsg()
}

/// Port of `zfopendata(char *name, union tcp_sockaddr *zdsockp, int *is_passivep)` from `Src/Modules/zftp.c:859`.
/// C: `static int zfopendata(char *name, union tcp_sockaddr *zdsockp,
/// int *is_passivep)` — set up a data connection (PASV-preferred,
/// PORT-mode fallback). Returns 0 success, 1 failure. Stores the
/// resulting fd in `zfsess->dfd` and sets `*is_passivep`.
///
/// Rust port: `is_passive` is returned via the tuple instead of a
/// `*int` out-param (Rust doesn't expose `union tcp_sockaddr` so the
/// `zdsockp` slot is gone). PORT-mode falls back to `TcpListener` for
/// the local bind+listen+accept; PASV path uses `TcpStream::connect`
/// and stores the connected stream's fd as `sess.dfd`. The non-PASV
/// branch requires the caller to accept(2) before reading.
#[allow(non_snake_case)]
/// Port of `zfopendata(char *name, union tcp_sockaddr *zdsockp, int *is_passivep)` from `Src/Modules/zftp.c:859`.
/// WARNING: param names don't match C — Rust=(name) vs C=(name, zdsockp, is_passivep)
pub fn zfopendata(name: &str) -> (i32, bool) {
    // c:859
    // c:862-865 — error if neither SNDP nor PASV preference is set.
    let prefs = zfprefs.load(Ordering::Relaxed); // c:862
    if (prefs & (ZFPF_SNDP | ZFPF_PASV)) == 0 {
        // c:863
        zwarnnam(name, "Must set preference S or P to transfer data"); // c:864
        return (1, false); // c:865
    }
    // c:871 — `if (!(zfstatusp[zfsessno] & ZFST_NOPS) && (zfprefs & ZFPF_PASV))`
    //   — try PASV only when the bit is set AND the server hasn't
    //     already 5xx'd PASV in this session.
    let nops_known = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.nops_probed))
        .unwrap_or(false);
    let try_pasv: bool = !nops_known && (prefs & ZFPF_PASV) != 0; // c:871

    if try_pasv {
        // c:879 — psv_cmd = "PASV\r\n"; (EPSV unsupported in Rust port).
        if zfsendcmd("PASV\r\n") == 6 {
            // c:881
            return (1, false); // c:882
        }
        let code = lastcode.load(Ordering::Relaxed);
        if (500..=504).contains(&code) {
            // c:884 — PASV unsupported by server.
            // c:888 — `zfstatusp[zfsessno] |= ZFST_NOPS;` — record so
            //         subsequent zfopendata calls skip PASV entirely.
            //         Prior port skipped this — every subsequent data
            //         transfer in the same session re-sent PASV to a
            //         server that had already 502'd it, wasting one
            //         round-trip per transfer.
            if let Ok(mut st) = zftp_state().lock() {
                if let Some(sess) = st.get_session_mut(None) {
                    sess.nops_probed = true; // c:888
                }
            }
            zfclosedata(); // c:889
                           // c:890 — `return zfopendata(...);` — C recurses.
                           // Rust falls through to the PORT-mode code below;
                           // same end-state since try_pasv was the only
                           // PASV-specific gate.
        } else {
            // c:899 — parse the PASV reply.
            let last = lastmsg.lock().ok().map(|m| m.clone()).unwrap_or_default();
            let (ip, port) = match parse_pasv_response(&last) {
                // c:925
                Ok(t) => t,
                Err(_) => {
                    zwarnnam(name, &format!("bad response to PASV: {}", last));
                    zfclosedata();
                    return (1, false); // c:946
                }
            };
            // c:954 — connect from data socket to remote (ip:port).
            let addr = format!("{}:{}", ip, port);
            let stream = match TcpStream::connect(&addr) {
                // c:958
                Ok(s) => s,
                Err(_) => {
                    zwarnnam(name, "can't open data socket");
                    zfclosedata();
                    return (1, false);
                }
            };
            let dfd_raw = stream.as_raw_fd();
            // Keep the stream alive past this fn so the fd stays open;
            // the session owns the fd via `dfd`.
            std::mem::forget(stream);
            if let Ok(mut state) = zftp_state().lock() {
                if let Some(sess) = state.get_session_mut(None) {
                    sess.dfd = dfd_raw; // c:961 dfd stored
                }
            }
            return (0, true); // c:1041 SUCCESS PASV
        }
    }

    // c:967-1037 — PORT-mode (active FTP): bind+listen locally, send
    // PORT a,b,c,d,p1,p2, return so caller can accept().
    // c:977 — refuse PORT mode if SNDP preference isn't set (e.g. when
    // PASV-only and the server rejected PASV).
    if (zfprefs.load(Ordering::Relaxed) & ZFPF_SNDP) == 0 {
        // c:977
        zwarnnam(name, "only sendport mode available for data"); // c:978
        return (1, false); // c:979
    }
    let listener = match std::net::TcpListener::bind("0.0.0.0:0") {
        // c:967 bind
        Ok(l) => l,
        Err(_) => {
            zwarnnam(name, "can't bind data socket");
            return (1, false); // c:870
        }
    };
    let local = match listener.local_addr() {
        Ok(a) => a,
        Err(_) => {
            zwarnnam(name, "getsockname failed");
            return (1, false);
        }
    };
    // c:986 — listen with backlog 1; std::net::TcpListener::bind already listens.
    let ipv4 = match local.ip() {
        std::net::IpAddr::V4(v) => v.octets(),
        std::net::IpAddr::V6(_) => {
            zwarnnam(name, "PORT mode requires IPv4");
            return (1, false);
        }
    };
    let port = local.port();
    // c:1003-1017 — PORT cmd format: "PORT h1,h2,h3,h4,p1,p2\r\n".
    let port_cmd = format!(
        "PORT {},{},{},{},{},{}\r\n",
        ipv4[0],
        ipv4[1],
        ipv4[2],
        ipv4[3],
        port >> 8,
        port & 0xff,
    );
    if zfsendcmd(&port_cmd) > 2 {
        // c:1018
        zwarnnam(name, "PORT command failed");
        return (1, false); // c:1019
    }
    // c:1029 — store listening fd as dfd; caller does accept().
    let lfd = listener.as_raw_fd();
    std::mem::forget(listener);
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.dfd = lfd; // c:1029
        }
    }
    (0, false) // c:1037 SUCCESS PORT
}

/// Port of `zfclosedata()` from `Src/Modules/zftp.c:1043`.
/// C: `static void zfclosedata(void)` — early-return when no dfd is
/// live, otherwise close(dfd) + dfd = -1.
#[allow(non_snake_case)]
pub fn zfclosedata() {
    // c:1043
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            if sess.dfd == -1 {
                // c:1045
                return; // c:1046
            }
            unsafe {
                libc::close(sess.dfd);
            } // c:1047 close(dfd)
            sess.dfd = -1; // c:1048
        }
    }
}

/// Port of `zfgetdata(char *name, char *rest, char *cmd, int getsize)` from `Src/Modules/zftp.c:1065`.
/// C: `static int zfgetdata(char *name, char *rest, char *cmd, int getsize)` —
/// open the data connection (PASV or PORT mode via zfopendata),
/// optionally send REST, send the transfer command, and (PORT mode)
/// accept(2) on the listening socket. Returns 0 success, 1 failure.
#[allow(non_snake_case)]
pub fn zfgetdata(name: &str, rest: &str, cmd: &str, getsize: i32) -> i32 {
    // c:1065
    // c:1065-1069 — locals at fn top.
    let is_passive: bool; // c:1068

    // c:1071-1072 — zfopendata: full PASV/PORT setup; sets sess.dfd.
    let (rc, ip) = zfopendata(name); // c:1071
    if rc != 0 {
        // c:1071
        return 1; // c:1072
    }
    is_passive = ip;

    // c:1084-1087 — REST command for resume.
    if !rest.is_empty() && zfsendcmd(rest) > 3 {
        zfclosedata();
        return 1;
    }

    // c:1089-1092 — send the transfer command (RETR / STOR / etc.).
    if zfsendcmd(cmd) > 2 {
        // c:1089
        zfclosedata(); // c:1090
        return 1; // c:1091
    }

    // c:1093-1116 — parse "Opening data connection for file (N bytes)"
    // hint to populate ZFTP_SIZE without a separate SIZE request.
    if getsize != 0 || cmd.starts_with("RETR") {
        let cur_last = lastmsg.lock().ok().map(|m| m.clone()).unwrap_or_default();
        if let Some(byte_idx) = cur_last.find("bytes") {
            // c:1101
            // Walk backward to find the start of the digit run.
            let prefix = &cur_last[..byte_idx];
            let trimmed: String = prefix
                .chars()
                .rev()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !trimmed.is_empty() && getsize != 0 {
                zfsetparam("ZFTP_SIZE", &trimmed, ZFPM_READONLY | ZFPM_INTEGER);
                // c:1112
            }
        }
    }

    // c:1118-1143 — PORT-mode accept handling. PASV: dfd is already
    // the data fd, just zfmovefd it. PORT: accept() on the listening
    // fd to obtain the real data fd, then close the listener.
    let dfd_raw: i32;
    if !is_passive {
        // c:1118
        // c:1124-1128 — accept the connection from the server.
        let mut sa: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut sl: libc::socklen_t = size_of::<libc::sockaddr_storage>() as _;
        let listen_fd = zftp_state()
            .lock()
            .ok()
            .and_then(|s| s.get_session(None).map(|x| x.dfd))
            .unwrap_or(-1);
        let newfd = unsafe { libc::accept(listen_fd, &mut sa as *mut _ as *mut _, &mut sl) };
        if newfd < 0 {
            // c:1129
            zwarnnam(
                name,
                &format!("unable to accept data: {}", io::Error::last_os_error()),
            );
            zfclosedata(); // c:1131
            return 1;
        }
        // c:1133 — close the original listening fd, install the accepted one.
        unsafe {
            libc::close(listen_fd);
        }
        dfd_raw = zfmovefd(newfd);
    } else {
        // c:1136
        // c:1139-1142 — zfmovefd(zfsess->dfd) for PASV.
        let cur_dfd = zftp_state()
            .lock()
            .ok()
            .and_then(|s| s.get_session(None).map(|x| x.dfd))
            .unwrap_or(-1);
        dfd_raw = zfmovefd(cur_dfd);
    }
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.dfd = dfd_raw; // c:1142
        }
    }

    // c:1156-1163 — SO_LINGER 120s.
    let li = libc::linger {
        l_onoff: 1,
        l_linger: 120,
    };
    unsafe {
        libc::setsockopt(
            dfd_raw,
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            &li as *const _ as *const libc::c_void,
            size_of::<libc::linger>() as libc::socklen_t,
        );
    }
    // c:1167-1170 — IP_TOS = IPTOS_THROUGHPUT.
    let tos: libc::c_int = 0x08; // IPTOS_THROUGHPUT
    unsafe {
        libc::setsockopt(
            dfd_raw,
            libc::IPPROTO_IP,
            libc::IP_TOS,
            &tos as *const _ as *const libc::c_void,
            size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    // c:1174 — fcntl(dfd, F_SETFD, FD_CLOEXEC).
    unsafe {
        libc::fcntl(dfd_raw, libc::F_SETFD, libc::FD_CLOEXEC);
    }

    0 // c:1177
}

/// Port of `zfstats(char *fnam, int remote, off_t *retsize, char **retmdtm, int fd)` from `Src/Modules/zftp.c:1193`.
/// C: `static int zfstats(char *fnam, int remote, off_t *retsize, char **retmdtm, int fd)` —
/// query file size + mtime, remote via SIZE/MDTM commands or local
/// via stat(2)/fstat(2).
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(fnam, remote, retmdtm, fd) vs C=(fnam, remote, retsize, retmdtm, fd)
pub fn zfstats(
    fnam: &str,
    remote: i32, // c:1193
    retsize: &mut libc::off_t,
    retmdtm: &mut Option<String>,
    fd: i32,
) -> i32 {
    // c:1195-1197 — locals at fn top.
    let mut sz: libc::off_t = -1; // c:1195
    let mut mt: Option<String> = None; // c:1196 char *mt
    let ret: i32; // c:1197

    *retsize = -1; // c:1199-1200
    *retmdtm = None; // c:1201-1202

    if remote != 0 {
        // c:1203
        // c:1205-1207 — `if ((zfsess->has_size == ZFCP_NOPE && retsize) ||
        //                    (zfsess->has_mdtm == ZFCP_NOPE && retmdtm))
        //                    return 2;`
        // Early-out when the server's known to not support SIZE/MDTM.
        // Both fields are already on the session struct (c:306-307)
        // and reset to ZFCP_UNKN at connect time (c:1952). Without
        // this gate, every zfstats call re-sends SIZE/MDTM to a
        // server that's already rejected them — wasted round-trip
        // + spurious 5xx in the server's log.
        let (had_size_nope, had_mdtm_nope) = {
            let st = match zftp_state().lock() {
                Ok(s) => s,
                Err(_) => return 1,
            };
            match st.get_session(None) {
                Some(sess) => (
                    sess.has_size == ZFCP_NOPE,
                    sess.has_mdtm == ZFCP_NOPE,
                ),
                None => (false, false),
            }
        };
        if had_size_nope || had_mdtm_nope {
            return 2; // c:1207 — retsize/retmdtm both requested in this port.
        }

        // c:1213 — zfsettype(ZFST_TYPE(zfstatusp[zfsessno]));
        zfsettype(ZFST_IMAG);

        // c:1214-1228 — SIZE command path.
        let cmd = format!("SIZE {}\r\n", fnam); // c:1215
        ret = zfsendcmd(&cmd); // c:1216
        if ret == 6 {
            // c:1218
            return 1; // c:1219
        }
        let code = lastcode.load(Ordering::Relaxed);
        if code < 300 {
            // c:1220
            // c:1221 — `sz = zstrtol(lastmsg, 0, 10);`
            sz = lastmsg
                .lock()
                .ok()
                .map(|m| m.trim().parse::<libc::off_t>().unwrap_or(-1))
                .unwrap_or(-1);
            // c:1222 — `zfsess->has_size = ZFCP_YUPP;` — record that
            //           the server speaks SIZE.
            if let Ok(mut st) = zftp_state().lock() {
                if let Some(sess) = st.get_session_mut(None) {
                    sess.has_size = ZFCP_YUPP;
                }
            }
        } else if (500..=504).contains(&code) {
            // c:1223 — server doesn't speak SIZE.
            // c:1224 — `zfsess->has_size = ZFCP_NOPE;`
            if let Ok(mut st) = zftp_state().lock() {
                if let Some(sess) = st.get_session_mut(None) {
                    sess.has_size = ZFCP_NOPE;
                }
            }
            return 2; // c:1225
        } else if code == 550 {
            // c:1226 — file doesn't exist.
            return 1; // c:1227
        }

        // c:1231-1245 — MDTM command path.
        let cmd = format!("MDTM {}\r\n", fnam); // c:1232
        let ret2 = zfsendcmd(&cmd); // c:1233
        if ret2 == 6 {
            // c:1235
            return 1; // c:1236
        }
        let code = lastcode.load(Ordering::Relaxed);
        if code < 300 {
            // c:1237
            // c:1238 — `mt = ztrdup(lastmsg);`
            mt = lastmsg.lock().ok().map(|m| m.clone());
            // c:1239 — `zfsess->has_mdtm = ZFCP_YUPP;`
            if let Ok(mut st) = zftp_state().lock() {
                if let Some(sess) = st.get_session_mut(None) {
                    sess.has_mdtm = ZFCP_YUPP;
                }
            }
        } else if (500..=504).contains(&code) {
            // c:1240 — server doesn't speak MDTM.
            // c:1241 — `zfsess->has_mdtm = ZFCP_NOPE;`
            if let Ok(mut st) = zftp_state().lock() {
                if let Some(sess) = st.get_session_mut(None) {
                    sess.has_mdtm = ZFCP_NOPE;
                }
            }
            return 2; // c:1242
        } else if code == 550 {
            // c:1243
            return 1; // c:1244
        }
    } else {
        // c:1246
        // c:1248-1263 — local file: stat or fstat.
        let mut statbuf: libc::stat = unsafe { std::mem::zeroed() }; // c:1248
        let cn = std::ffi::CString::new(fnam).unwrap_or_default();
        let rc = if fd == -1 {
            // c:1252
            unsafe { libc::stat(cn.as_ptr(), &mut statbuf) }
        } else {
            unsafe { libc::fstat(fd, &mut statbuf) }
        };
        if rc < 0 {
            // c:1252
            return 1; // c:1253
        }
        sz = statbuf.st_size as libc::off_t; // c:1255

        // c:1257-1263 — format mtime as YYYYMMDDHHMMSS via gmtime.
        let mtime = statbuf.st_mtime;
        let mut tmbuf = [0u8; 20];
        let tmbuf_len = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            libc::gmtime_r(&mtime, &mut tm); // c:1259
                                             // c:1261 — ztrftime(tmbuf, 20, "%Y%m%d%H%M%S", tm, 0);
            let fmt = std::ffi::CString::new("%Y%m%d%H%M%S").unwrap();
            libc::strftime(
                tmbuf.as_mut_ptr() as *mut libc::c_char,
                20,
                fmt.as_ptr(),
                &tm,
            )
        };
        mt = std::str::from_utf8(&tmbuf[..tmbuf_len])
            .ok()
            .map(|s| s.to_string());
    }

    *retsize = sz; // c:1265-1266
    *retmdtm = mt; // c:1267-1268
    0 // c:1269
}

/// Port of `zfstarttrans(char *nam, int recv, off_t sz)` from `Src/Modules/zftp.c:1276`.
/// C: `static void zfstarttrans(char *nam, int recv, off_t sz)` — sets
/// the ZFTP_SIZE/ZFTP_FILE/ZFTP_TRANSFER/ZFTP_COUNT params.
#[allow(non_snake_case)]
pub fn zfstarttrans(nam: &str, recv: i32, sz: libc::off_t) {
    // c:1276
    let cnt: libc::off_t = 0; // c:1276
                              // c:1284-1285 — only set ZFTP_SIZE when sz > 0 (avoid lying about
                              // pipe-sourced unknown size).
    if sz > 0 {
        // c:1284
        zfsetparam("ZFTP_SIZE", &sz.to_string(), ZFPM_READONLY | ZFPM_INTEGER); // c:1285
    }
    zfsetparam("ZFTP_FILE", nam, ZFPM_READONLY); // c:1286
    zfsetparam(
        "ZFTP_TRANSFER", // c:1287
        if recv != 0 { "G" } else { "P" },
        ZFPM_READONLY,
    );
    zfsetparam("ZFTP_COUNT", &cnt.to_string(), ZFPM_READONLY | ZFPM_INTEGER); // c:1288
}

/// Port of `zfendtrans()` from `Src/Modules/zftp.c:1295`.
/// C: `static void zfendtrans(void)` — unsets the ZFTP_* transfer params.
#[allow(non_snake_case)]
pub fn zfendtrans() {
    // c:1295
    zfunsetparam("ZFTP_SIZE"); // c:1295
    zfunsetparam("ZFTP_FILE"); // c:1298
    zfunsetparam("ZFTP_TRANSFER"); // c:1299
    zfunsetparam("ZFTP_COUNT"); // c:1300
}

/// Port of `zfread(int fd, char *bf, off_t sz, int tmout)` from `Src/Modules/zftp.c:1307`.
/// C: `static int zfread(int fd, char *bf, off_t sz, int tmout)` — read
/// up to `sz` bytes from fd; with `tmout > 0` install a SIGALRM-driven
/// timeout that aborts the read.
#[allow(non_snake_case)]
pub fn zfread(fd: i32, bf: &mut [u8], sz: libc::off_t, tmout: i32) -> i32 {
    // c:1307
    let ret: isize; // c:1307 int ret

    // c:1311-1312 — no timeout: plain read.
    if tmout == 0 {
        let n = unsafe { libc::read(fd, bf.as_mut_ptr() as *mut libc::c_void, sz as libc::size_t) };
        return n as i32; // c:1312
    }

    // c:1314-1318 — `if (setjmp(zfalrmbuf)) { alarm(0); zwarnnam(...,
    //                "timeout on network read"); return -1; }`. C uses
    // setjmp/longjmp; the SIGALRM handler calls longjmp(zfalrmbuf, 1)
    // which unwinds back into this if-block. Rust port can't use
    // setjmp/longjmp (UB across drop boundaries) so models the same
    // semantic via the ZFDRRRRING atomic flag that zfhandler sets at
    // alarm fire. Two check sites needed:
    //   1. Before alarm install — handle "previous alarm fired between
    //      zfread calls" (rare but possible if user code chained reads
    //      with no intermediate zfalarm reset).
    //   2. After read returns — handle "alarm fired during THIS read,
    //      kernel returned EINTR but Rust has no setjmp jump-back".
    //      Prior port had only the first check; the second was the
    //      gap — a SIGALRM-interrupted read returned -1 silently with
    //      no "timeout on network read" warning, leaving the user
    //      unable to distinguish timeout from other I/O failure.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        // c:1314 setjmp — stale alarm from before this call.
        unsafe {
            libc::alarm(0);
        } // c:1315
        zwarnnam("zftp", "timeout on network read"); // c:1316
        return -1; // c:1317
    }
    zfalarm(tmout); // c:1319

    // c:1321 — `ret = read(fd, bf, sz);`
    ret = unsafe { libc::read(fd, bf.as_mut_ptr() as *mut libc::c_void, sz as libc::size_t) };
    // c:1324 — `alarm(0);`
    unsafe {
        libc::alarm(0);
    }
    // c:1314 setjmp counterpart for THIS call — alarm fired during read.
    // ZFDRRRRING was zeroed inside zfalarm at line 212; if non-zero now,
    // the handler set it during read.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        zwarnnam("zftp", "timeout on network read"); // c:1316
        return -1; // c:1317
    }
    ret as i32 // c:1325
}

/// Port of `zfwrite(int fd, char *bf, off_t sz, int tmout)` from Src/Modules/zftp.c:1332.
/// C: `int zfwrite(int fd, char *bf, off_t sz, int tmout)` — write with
/// optional alarm timeout.
#[allow(non_snake_case)]
pub fn zfwrite(fd: i32, bf: &[u8], sz: i64, tmout: i32) -> i32 {
    // c:1332
    // c:1332 — `if (!tmout) return write(fd, bf, sz);`
    if tmout == 0 {
        // c:1335
        return unsafe {
            libc::write(fd, bf.as_ptr() as *const _, sz as usize) as i32 // c:1336
        };
    }
    // c:1339-1342 — `if (setjmp(zfalrmbuf)) { alarm(0); zwarnnam(...,
    //                "timeout on network write"); return -1; }`.
    // Same setjmp pattern as zfread (c:1314-1318) — see the
    // accompanying zfread fix (cfe7560f58) for the ZFDRRRRING
    // adapter rationale. Two check sites:
    //   1. Before alarm install — stale alarm from before this call.
    //   2. After write returns — alarm fired during THIS write.
    // Prior port had neither: the "future refactor should plumb a
    // real timeout via select(2)/poll(2)" comment papered over the
    // missing semantic. ZFDRRRRING was zeroed inside zfalarm at
    // line 212; if non-zero post-write, zfhandler set it during the
    // blocked write.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        // Stale alarm from earlier call.
        unsafe {
            libc::alarm(0);
        }
        zwarnnam("zftp", "timeout on network write");
        return -1;
    }
    zfalarm(tmout); // c:1344
    let ret = unsafe {
        libc::write(fd, bf.as_ptr() as *const _, sz as usize) as i32 // c:1346
    };
    unsafe {
        libc::alarm(0);
    } // c:1349
    // Post-write check matching the zfread fix.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        zwarnnam("zftp", "timeout on network write"); // c:1341
        return -1; // c:1342
    }
    ret // c:1350
}

/// Port of `static int zfread_eof` file-static from
/// `Src/Modules/zftp.c:1359`. Set by zfread_block when the ZFHD_EOFB
/// flag arrives; cleared at the top of every fresh transfer.
pub static zfread_eof: std::sync::atomic::AtomicI32 = // c:1359
    std::sync::atomic::AtomicI32::new(0);

/// Port of `zfread_block(int fd, char *bf, off_t sz, int tmout)` from `Src/Modules/zftp.c:1359`.
/// C: `static int zfread_block(int fd, char *bf, off_t sz, int tmout)` —
/// read a block-mode framed record: a 3-byte zfheader followed by
/// `blksz` payload bytes. Loops over restart-marker blocks (ZFHD_MARK)
/// until a real data block or end-of-record (ZFHD_EOFB) arrives.
#[allow(non_snake_case)]
pub fn zfread_block(fd: i32, bf: &mut [u8], sz: libc::off_t, tmout: i32) -> i32 {
    // c:1359
    // c:1361-1364 — locals at fn top.
    let mut n: i32; // c:1361 int n
    let mut hdr = zfheader {
        flags: 0,
        bytes: [0u8; 2],
    }; // c:1362
    let mut blksz: libc::off_t = 0; // c:1363 off_t blksz
    let mut cnt: libc::off_t; // c:1363 off_t cnt
    let mut bfptr: usize; // c:1364 char *bfptr (offset into bf)

    // c:1365-1403 — outer do-while loop: keep reading until we get a
    // non-marker block (or hit EOF).
    loop {
        // c:1365 do {
        // c:1367-1369 — read header bytes, retry on EINTR.
        let mut hdr_buf = [0u8; 3];
        loop {
            // c:1367 do
            n = zfread(fd, &mut hdr_buf, 3, tmout); // c:1368
            if !(n < 0
                && io::Error::last_os_error().raw_os_error()      // c:1369 EINTR retry
                 == Some(libc::EINTR))
            {
                break;
            }
        }
        // c:1370-1373 — short read → fail unless interrupted by SIGALRM.
        if n != 3 && ZFDRRRRING.load(Ordering::Relaxed) == 0 {
            zwarnnam("zftp", "failure reading FTP block header");
            return n; // c:1372
        }
        hdr.flags = hdr_buf[0] as i8;
        hdr.bytes[0] = hdr_buf[1];
        hdr.bytes[1] = hdr_buf[2];
        // c:1375-1376 — ZFHD_EOFB sets the file-static eof flag.
        if (hdr.flags as i32 & ZFHD_EOFB) != 0 {
            zfread_eof.store(1, Ordering::Relaxed); // c:1376
        }
        // c:1377 — network byte order: blksz = (b[0] << 8) | b[1].
        blksz = ((hdr.bytes[0] as libc::off_t) << 8) | (hdr.bytes[1] as libc::off_t);
        // c:1378-1385 — caller's buffer too small.
        if blksz > sz {
            zwarnnam("zftp", "block too large to handle");
            unsafe {
                *errno_ptr() = libc::EIO;
            } // c:1383
            return -1; // c:1384
        }
        // c:1386-1397 — drain the payload.
        bfptr = 0; // c:1386 bfptr = bf
        cnt = blksz; // c:1387
        while cnt > 0 {
            // c:1388
            let want = cnt as usize;
            let end = bfptr + want;
            if end > bf.len() {
                return -1;
            }
            n = zfread(fd, &mut bf[bfptr..end], cnt, tmout); // c:1389
            if n > 0 {
                // c:1390
                bfptr += n as usize; // c:1391
                cnt -= n as libc::off_t; // c:1392
            } else if n < 0
                && (errflag.load(Ordering::Relaxed) != 0
                    || ZFDRRRRING.load(Ordering::Relaxed) != 0
                    || io::Error::last_os_error().raw_os_error() != Some(libc::EINTR))
            {
                // c:1393
                return n; // c:1394
            } else {
                break; // c:1396
            }
        }
        // c:1398-1402 — short data block.
        if cnt != 0 {
            zwarnnam("zftp", "short data block");
            unsafe {
                *errno_ptr() = libc::EIO;
            } // c:1400
            return -1; // c:1401
        }
        // c:1403 — } while ((hdr.flags & ZFHD_MARK) && !zfread_eof);
        if !((hdr.flags as i32 & ZFHD_MARK) != 0 && zfread_eof.load(Ordering::Relaxed) == 0) {
            break;
        }
    }
    // c:1404 — return (hdr.flags & ZFHD_MARK) ? 0 : blksz;
    if (hdr.flags as i32 & ZFHD_MARK) != 0 {
        0
    } else {
        blksz as i32
    }
}

/// Port of `zfwrite_block(int fd, char *bf, off_t sz, int tmout)` from Src/Modules/zftp.c:1411.
/// C: `int zfwrite_block(int fd, char *bf, off_t sz, int tmout)` —
/// frame the data with a `struct zfheader` and write block + payload.
#[allow(non_snake_case)]
pub fn zfwrite_block(fd: i32, bf: &[u8], sz: i64, tmout: i32) -> i32 {
    // c:1411
    let mut hdr = zfheader {
        bytes: [0u8; 2],
        flags: 0i8,
    }; // c:1411
    let mut n: i32;
    // c:1418-1424 — emit header, retry on EINTR.
    loop {
        hdr.bytes[0] = ((sz & 0xff00) >> 8) as u8; // c:1419
        hdr.bytes[1] = (sz & 0xff) as u8; // c:1420
        hdr.flags = if sz != 0 { 0i8 } else { ZFHD_EOFB as i8 }; // c:1421
        let hdr_bytes = unsafe { std::slice::from_raw_parts(&hdr as *const _ as *const u8, 3) };
        n = zfwrite(fd, hdr_bytes, 3, tmout); // c:1422
        if !(n < 0 && unsafe { *errno_ptr() } == libc::EINTR) {
            break;
        } // c:1424
    }
    if n != 3 {
        // c:1426
        return n; // c:1428
    }
    if sz != 0 {
        // c:1431
        n = zfwrite(fd, bf, sz, tmout); // c:1432
    }
    n // c:1434
}

/// Port of `zfsenddata(char *name, int recv, int progress, off_t startat)` from `Src/Modules/zftp.c:1456`.
/// C: `static int zfsenddata(char *name, int recv, int progress, off_t startat)` —
/// move data between local fd (0/1) and the data connection fd
/// (`dfd`). Handles BINARY+ASCII mode, optional block-mode framing,
/// progress callback, and the abort/SYNCH sequence on error.
#[allow(non_snake_case)]
pub fn zfsenddata(name: &str, recv: i32, progress: i32, startat: libc::off_t) -> i32 {
    // c:1456
    // c:1458-1459 — buffer sizes.
    const ZF_BUFSIZE: usize = 32768;
    const ZF_ASCSIZE: usize = ZF_BUFSIZE / 2;
    // c:1461-1466 — locals at fn top.
    let mut n: i32; // c:1461 int n
    let mut ret: i32 = 0; // c:1461 ret = 0
    let gotack: i32 = 0; // c:1461 gotack = 0
    let fdin: i32; // c:1461
    let fdout: i32; // c:1461
    let mut fromasc: i32 = 0; // c:1461 fromasc = 0
    let mut toasc: i32 = 0; // c:1461 toasc = 0
    let mut rtmout: i32 = 0; // c:1462
    let mut wtmout: i32 = 0; // c:1462
    let mut lsbuf = vec![0u8; ZF_BUFSIZE]; // c:1463
    let mut ascbuf: Vec<u8> = Vec::new(); // c:1463 ascbuf = NULL
    let mut sofar: libc::off_t = 0; // c:1464
    let mut last_sofar: libc::off_t = 0; // c:1464

    // c:1469-1481 — pre-transfer progress hook. Fires once at the
    // top of zfsenddata when ZFTP_COUNT starts at zero so the user
    // shfunc can set up its meter. After it runs, we seed `sofar`
    // with `startat` (resume-from offset) per c:1480.
    if progress != 0 {
        if let Some(mut shfunc) = getshfunc("zftp_progress") {
            // c:1473 — `osc = sfcontext;`
            let osc = SFCONTEXT.load(Ordering::Relaxed);
            SFCONTEXT.store(SFC_HOOK, Ordering::Relaxed); // c:1475
                                                          // c:1477 — `doshfunc(shfunc, NULL, 1);`.
            let body_runner = || -> i32 {
                crate::ported::exec_hooks::run_function_body("zftp_progress", &[]).unwrap_or(0)
            };
            let _ = crate::ported::exec::doshfunc(
                &mut shfunc,
                vec!["zftp_progress".to_string()],
                true,
                body_runner,
            );
            SFCONTEXT.store(osc, Ordering::Relaxed); // c:1478
                                                     // c:1480 — `sofar = last_sofar = startat;`.
            sofar = startat;
            last_sofar = startat;
        }
    }

    // c:1482-1498 — direction-dependent fd + ascii-flag setup.
    let mut use_block_mode = false;
    {
        let state = match zftp_state().lock() {
            Ok(s) => s,
            Err(_) => return 1,
        };
        let sess = match state.get_session(None) {
            Some(s) => s,
            None => return 1,
        };
        if recv != 0 {
            // c:1482
            fdin = sess.dfd; // c:1483
            fdout = 1; // c:1484
                       // c:1485 — `rtmout = getiparam("ZFTP_TMOUT");`. paramtab read.
            rtmout = getiparam("ZFTP_TMOUT") as i32;
            if sess.transfer_type == ZFST_ASCI as i32 {
                // c:1486
                fromasc = 1; // c:1487
            }
            if sess.transfer_mode == ZFST_BLOC as i32 {
                // c:1488
                use_block_mode = true; // c:1489
            }
        } else {
            // c:1490
            fdin = 0; // c:1491
            fdout = sess.dfd; // c:1492
                              // c:1493 — `wtmout = getiparam("ZFTP_TMOUT");`. paramtab read.
            wtmout = getiparam("ZFTP_TMOUT") as i32;
            if sess.transfer_type == ZFST_ASCI as i32 {
                // c:1494
                toasc = 1; // c:1495
            }
            if sess.transfer_mode == ZFST_BLOC as i32 {
                // c:1496
                use_block_mode = true; // c:1497
            }
        }
    }

    if progress != 0 {
        sofar = startat; // c:1480
        last_sofar = sofar;
    }
    let _ = last_sofar;

    // c:1500-1501 — ascbuf for ASCII translation buffer.
    if toasc != 0 {
        ascbuf = vec![0u8; ZF_ASCSIZE]; // c:1501
    }
    zfpipe(); // c:1502
    zfread_eof.store(0, Ordering::Relaxed); // c:1503

    // c:1504-1614 — main transfer loop.
    while ret == 0 && zfread_eof.load(Ordering::Relaxed) == 0 {
        // c:1505-1506 — read into either ascbuf or lsbuf.
        n = if toasc != 0 {
            if use_block_mode {
                zfread_block(fdin, &mut ascbuf, ZF_ASCSIZE as libc::off_t, rtmout)
            } else {
                zfread(fdin, &mut ascbuf, ZF_ASCSIZE as libc::off_t, rtmout)
            }
        } else if use_block_mode {
            zfread_block(fdin, &mut lsbuf, ZF_BUFSIZE as libc::off_t, rtmout)
        } else {
            zfread(fdin, &mut lsbuf, ZF_BUFSIZE as libc::off_t, rtmout)
        };

        if n > 0 {
            // c:1507
            // c:1509-1520 — toasc: \n → \r\n.
            if toasc != 0 {
                let mut iptr = 0usize;
                let mut optr = 0usize;
                let mut cnt = n;
                while cnt > 0 {
                    if ascbuf[iptr] == b'\n' {
                        // c:1514
                        if optr < lsbuf.len() {
                            lsbuf[optr] = b'\r';
                            optr += 1;
                        }
                        n += 1; // c:1516
                    }
                    if optr < lsbuf.len() {
                        lsbuf[optr] = ascbuf[iptr];
                        optr += 1;
                    }
                    iptr += 1;
                    cnt -= 1;
                }
            }
            // c:1521-1532 — fromasc: \r\n → \n.
            if fromasc != 0 {
                if let Some(_start) = lsbuf[..n as usize].iter().position(|&b| b == b'\r') {
                    let mut optr = 0usize;
                    let mut iptr = 0usize;
                    let len = n as usize;
                    while iptr < len {
                        if lsbuf[iptr] != b'\r' || iptr + 1 >= len || lsbuf[iptr + 1] != b'\n' {
                            lsbuf[optr] = lsbuf[iptr];
                            optr += 1;
                        } else {
                            n -= 1; // c:1529
                        }
                        iptr += 1;
                    }
                }
            }
            // c:1533-1591 — write loop with EINTR + partial-write handling.
            let mut optr_off: usize = 0;
            sofar += n as libc::off_t; // c:1535
            loop {
                // c:1537 for(;;)
                let chunk = &lsbuf[optr_off..optr_off + n as usize];
                let newn: i32 = if use_block_mode && recv == 0 {
                    zfwrite_block(fdout, chunk, n as libc::off_t, wtmout)
                } else {
                    zfwrite(fdout, chunk, n as libc::off_t, wtmout)
                };
                if newn == n {
                    break;
                } // c:1546
                if newn < 0 {
                    // c:1548
                    let errno = io::Error::last_os_error().raw_os_error();
                    let drrr = ZFDRRRRING.load(Ordering::Relaxed) != 0;
                    let efl = errflag.load(Ordering::Relaxed) != 0;
                    if errno != Some(libc::EINTR) || efl || drrr {
                        // c:1578
                        if !drrr && (efl || errno != Some(libc::EPIPE)) {
                            // c:1579-1580
                            ret = if recv != 0 { 2 } else { 1 };
                            zwarnnam(
                                name, // c:1582
                                &format!("write failed: {}", io::Error::last_os_error()),
                            );
                        } else {
                            ret = if recv != 0 { 3 } else { 1 };
                        }
                        break;
                    }
                    continue; // c:1587
                }
                optr_off += newn as usize; // c:1589
                n -= newn; // c:1590
            }
        } else if n < 0 {
            // c:1592
            let errno = io::Error::last_os_error().raw_os_error();
            let drrr = ZFDRRRRING.load(Ordering::Relaxed) != 0;
            let efl = errflag.load(Ordering::Relaxed) != 0;
            if errno != Some(libc::EINTR) || efl || drrr {
                // c:1593
                if !drrr && (efl || errno != Some(libc::EPIPE)) {
                    // c:1594
                    ret = if recv != 0 { 1 } else { 2 };
                    zwarnnam(
                        name, // c:1597
                        &format!("read failed: {}", io::Error::last_os_error()),
                    );
                } else {
                    ret = if recv != 0 { 1 } else { 3 };
                }
                break;
            }
        } else {
            // c:1602
            break; // c:1603
        }
        // c:1604-1613 — progress hook (zftp_progress shfunc dispatch).
        if ret == 0 && sofar != last_sofar && progress != 0 {
            if let Some(mut shfunc) = getshfunc("zftp_progress") {
                // c:1605
                let osc = SFCONTEXT.load(Ordering::Relaxed); // c:1606
                zfsetparam(
                    "ZFTP_COUNT",
                    &sofar.to_string(),
                    ZFPM_READONLY | ZFPM_INTEGER,
                ); // c:1608
                SFCONTEXT.store(SFC_HOOK, Ordering::Relaxed); // c:1609
                                                              // c:1610 — `doshfunc(shfunc, NULL, 1);`. NULL doshargs
                                                              // → argv = [fn-name only]; body_runner routes through
                                                              // the host body-only entry.
                let body_runner = || -> i32 {
                    crate::ported::exec_hooks::run_function_body("zftp_progress", &[]).unwrap_or(0)
                };
                let _ = crate::ported::exec::doshfunc(
                    &mut shfunc,
                    vec!["zftp_progress".to_string()],
                    true,
                    body_runner,
                );
                SFCONTEXT.store(osc, Ordering::Relaxed);
            // c:1611
            } else {
                zfsetparam(
                    "ZFTP_COUNT",
                    &sofar.to_string(),
                    ZFPM_READONLY | ZFPM_INTEGER,
                ); // c:1608
            }
            last_sofar = sofar; // c:1612
        }
    }
    zfunpipe(); // c:1615
    ZFDRRRRING.store(0, Ordering::Relaxed); // c:1620

    // c:1621-1625 — block-mode EOF marker on send completion.
    if errflag.load(Ordering::Relaxed) == 0 && ret == 0 && recv == 0 && use_block_mode {
        let eof_buf = [0u8; 1];
        if zfwrite_block(fdout, &eof_buf, 0, wtmout) < 0 {
            ret = 1; // c:1624
        }
    }

    // c:1626-1676 — abort/SYNCH sequence on error.
    if errflag.load(Ordering::Relaxed) != 0 || ret > 1 {
        // c:1642 — IAC=255, IP=244, SYNCH=242 per Telnet RFC 854.
        let msg: [u8; 4] = [255, 244, 255, 242]; // c:1642
        if ret == 2 {
            // c:1644
            zwarnnam(name, "aborting data transfer..."); // c:1645
        }
        // c:1647 — holdintr(); block SIGINT around the abort handshake.
        crate::ported::signals::holdintr(); // c:1647
                                            // c:1651-1652 — send IAC IP IAC + SYNCH OOB on control connection.
        if let Ok(state) = zftp_state().lock() {
            if let Some(sess) = state.get_session(None) {
                if let Some(ref ctrl) = sess.control {
                    let cfd = ctrl.as_raw_fd();
                    unsafe {
                        libc::send(cfd, msg.as_ptr() as *const libc::c_void, 3, 0); // c:1651
                        libc::send(
                            cfd,
                            msg[3..].as_ptr() as *const libc::c_void,
                            1,
                            libc::MSG_OOB,
                        ); // c:1652
                    }
                }
            }
        }
        zfsendcmd("ABOR\r\n"); // c:1654
        if lastcode.load(Ordering::Relaxed) != 226 {
            // c:1672
            ret = 1; // c:1673
        }
        // c:1675 — noholdintr(); restore SIGINT handling.
        crate::ported::signals::noholdintr(); // c:1675
    }

    // c:1678-1679 — free ascbuf (Rust Drop).
    drop(ascbuf);
    zfclosedata(); // c:1680
    if gotack == 0 && zfgetmsg() > 2 {
        // c:1681
        ret = 1; // c:1682
    }
    if ret != 0 {
        1
    } else {
        0
    } // c:1683
}

// =====================================================================
// `enum { ZFST_* }` from `Src/Modules/zftp.c` — bit-packed shared-fd
// status word used by the `zfstatfd` mechanism so a subshell can
// detect type/mode/connection changes in the parent shell.
// =====================================================================

/// `ZF_BUFSIZE` from `Src/Modules/zftp.c:1458`. Default I/O block
/// size for the zftp byte-stream pump.
pub const ZF_BUFSIZE: usize = 32_768; // c:1458

/// `ZF_ASCSIZE` from `Src/Modules/zftp.c:1459`.
/// `#define ZF_ASCSIZE (ZF_BUFSIZE/2)`. Smaller buffer for ASCII
/// mode (line-by-line CRLF translation can grow output up to 2x).
pub const ZF_ASCSIZE: usize = ZF_BUFSIZE / 2; // c:1459

// Subcommand dispatch table for zftp. Each `zftp_<subcmd>` C function
// has the canonical signature `int zftp_<subcmd>(char *name, char **args, int flags)`.
// The C source parses the first argv element as the subcommand name
// and dispatches via `zftpcmdtab[]`. Rust port: each free fn matches
// the C signature and routes through the global `ZFTP_STATE` to call
// the corresponding `zftp_globals::<method>` on the live state.

/// Port of `zftp_open(char *name, char **args, int flags)` from `Src/Modules/zftp.c:1690`.
/// C: `int zftp_open(char *name, char **args, int flags)` — opens a
/// TCP control connection (with optional `host[:port]` or IPv6
/// `[host]:port` syntax, falls back to `zfsess->userparams` when no
/// args, reads the 220 banner via `zfgetmsg()`, sets
/// ZFTP_HOST/PORT/IP/MODE params, and chains to `zftp_login()` when
/// extra args are present.
pub fn zftp_open(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:1690
    let mut port: i32 = -1; // c:1698 port = -1
    let portnam: String; // c:1695 portnam = "ftp"
    let hostnam: String; // c:1696 hostnam
    let hostsuffix: String; // c:1696 hostsuffix
    let tmout: i32; // c:1697 tmout

    // c:1701-1708 — fall back to userparams when no positional args.
    let mut effective: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    if effective.is_empty() {
        // c:1701 !*args
        let up = zftp_state()
            .lock()
            .ok()
            .and_then(|s| s.get_session(None).map(|sess| sess.userparams.clone()))
            .unwrap_or_default();
        if !up.is_empty() {
            effective = up; // c:1703 args = userparams
        } else {
            zwarnnam(name, "no host specified"); // c:1705
            return 1; // c:1706
        }
    }

    // c:1715-1716 — close any existing connection.
    let already_open = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.control.is_some()))
        .unwrap_or(false);
    if already_open {
        // c:1715
        zfclose(0); // c:1716
    }

    // c:1718 — dupstring(args[0]) — Rust String owns its bytes.
    let raw = effective[0].clone();
    // c:1720-1730 — IPv6 `[host]:port` bracket parse.
    if let Some(stripped) = raw.strip_prefix('[') {
        let close_idx = match stripped.find(']') {
            Some(i) => i,
            None => {
                zwarnnam(name, &format!("Invalid host format: {}", raw)); // c:1726
                return 1; // c:1727
            }
        };
        let after = &stripped[close_idx + 1..];
        if !after.is_empty() && !after.starts_with(':') {
            // c:1725
            zwarnnam(name, &format!("Invalid host format: {}", raw));
            return 1;
        }
        hostnam = stripped[..close_idx].to_string(); // c:1722
        hostsuffix = after.to_string(); // c:1729
    } else {
        hostnam = raw.clone();
        hostsuffix = raw; // c:1733 else branch
    }

    // c:1735-1751 — :port suffix; numeric → htons-friendly, else look up.
    if let Some((_pre, post)) = hostsuffix.split_once(':') {
        // c:1735
        let trimmed = post.trim();
        match trimmed.parse::<i32>() {
            // c:1740 zstrtol
            Ok(p) => {
                // c:1750 numeric
                port = p;
                portnam = "ftp".to_string();
            }
            Err(_) => {
                // c:1744 non-numeric
                portnam = trimmed.to_string(); // c:1745
                port = -1; // c:1746
            }
        }
    } else {
        portnam = "ftp".to_string();
    }

    // c:1755-1758 — protoent/servent lookups: skipped in Rust port
    // (TcpStream handles tcp directly). Port resolution falls back to
    // the standard /etc/services for non-numeric ports.
    let resolved_port: u16 = if port > 0 {
        port as u16
    } else {
        match portnam.as_str() {
            "ftp" => 21,
            "ftps" => 990,
            _ => {
                zwarnnam(name, &format!("Can't find port for service `{}'", portnam)); // c:1768
                return 1; // c:1769
            }
        }
    };

    ZCFINISH.store(2, Ordering::Relaxed); // c:1772 zcfinish = 2

    // c:1775 — `tmout = getiparam("ZFTP_TMOUT");`. Read paramtab; if
    //          unset, fall back to 60-second connect timeout.
    tmout = {
        let v = getiparam("ZFTP_TMOUT") as i32;
        if v > 0 {
            v
        } else {
            60
        }
    };

    // c:1778-1789 — `if (setjmp(zfalrmbuf)) { alarm(0);
    //                  queue_signals();
    //                  if ((hname = getsparam_u("ZFTP_HOST")) && *hname)
    //                      zwarnnam(name, "timeout connecting to %s", hname);
    //                  else
    //                      zwarnnam(name, "timeout on host name lookup");
    //                  unqueue_signals();
    //                  zfclose(0);
    //                  return 1;
    //               }`
    // ZFDRRRRING adapter same as the other zftp setjmp ports (cfe7560f58,
    // 00c1c36dc9, bef84af815, f68bff9298). The diagnostic is host-aware:
    // if ZFTP_HOST was already set by a prior open call (re-open after
    // previous timeout), the message names the host; otherwise it
    // reports "timeout on host name lookup" since the resolve hasn't
    // completed yet.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        unsafe {
            libc::alarm(0);
        }
        let hname = crate::ported::params::getsparam_u("ZFTP_HOST").unwrap_or_default();
        if !hname.is_empty() {
            zwarnnam(name, &format!("timeout connecting to {}", hname));
        } else {
            zwarnnam(name, "timeout on host name lookup");
        }
        zfclose(0); // c:1787
        return 1; // c:1788
    }
    // c:1790 — zfalarm(tmout): installed for the connect() phase.
    zfalarm(tmout);

    // c:1803 — zsh_getipnodebyname → ToSocketAddrs resolves both v4+v6.
    let target = format!("{}:{}", hostnam, resolved_port);
    let addrs: Vec<std::net::SocketAddr> = match target.to_socket_addrs() {
        Ok(it) => it.collect(),
        Err(_) => {
            unsafe {
                libc::alarm(0);
            }
            zwarnnam(name, &format!("host not found: {}", hostnam)); // c:1815
            return 1; // c:1817
        }
    };
    if addrs.is_empty() {
        unsafe {
            libc::alarm(0);
        }
        zwarnnam(name, &format!("host not found: {}", hostnam));
        return 1;
    }
    // c:1818 — ZFTP_HOST.
    zfsetparam("ZFTP_HOST", &hostnam, ZFPM_READONLY); // c:1818
                                                      // c:1824 — ZFTP_PORT as integer.
    zfsetparam(
        "ZFTP_PORT",
        &resolved_port.to_string(),
        ZFPM_READONLY | ZFPM_INTEGER,
    ); // c:1824

    // c:1838 — tcp_socket + tcp_connect: connect_timeout loops addrs.
    let mut last_err: Option<io::Error> = None;
    let mut connected: Option<(TcpStream, std::net::SocketAddr)> = None;
    for addr in addrs.iter() {
        // c:1860 for addrp
        if errflag.load(Ordering::Relaxed) != 0 {
            break;
        }
        match TcpStream::connect_timeout(addr, Duration::from_secs(tmout.max(1) as u64)) {
            Ok(s) => {
                connected = Some((s, *addr));
                break;
            } // c:1867 SUCCEEDED
            Err(e) => {
                last_err = Some(e);
            } // c:1866 retry
        }
    }
    let (stream, used_addr) = match connected {
        Some(v) => v,
        None => {
            unsafe {
                libc::alarm(0);
            }
            zfunsetparam("ZFTP_HOST"); // c:1847
            zfunsetparam("ZFTP_PORT"); // c:1848
            let msg = last_err
                .as_ref()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "connect failed".to_string());
            zwarnnam(name, &format!("connect failed: {}", msg)); // c:1879
            return 1; // c:1880
        }
    };

    // c:1886-1890 — record peer IP.
    let pbuf = used_addr.ip().to_string();
    zfsetparam("ZFTP_IP", &pbuf, ZFPM_READONLY); // c:1887

    unsafe {
        libc::alarm(0);
    } // c:1882

    // c:1778 setjmp counterpart for THIS call — alarm fired during
    // resolve or connect. ZFDRRRRING was zeroed inside zfalarm; non-
    // zero now means zfhandler tripped during the blocked
    // to_socket_addrs or TcpStream::connect_timeout. Without this
    // check, a SIGALRM-cut-short connect that happened to succeed
    // (or appeared to via Rust's connect-timeout returning Ok on a
    // partial handshake) would proceed into the FTP-login phase
    // with a half-formed connection — confusing diagnostics later
    // instead of the C-faithful "timeout connecting to %s" up front.
    if ZFDRRRRING.load(Ordering::Relaxed) != 0 {
        let hname = crate::ported::params::getsparam_u("ZFTP_HOST").unwrap_or_default();
        if !hname.is_empty() {
            zwarnnam(name, &format!("timeout connecting to {}", hname));
        } else {
            zwarnnam(name, "timeout on host name lookup");
        }
        zfclose(0); // c:1787
        return 1; // c:1788
    }

    ZFNOPEN.fetch_add(1, Ordering::Relaxed); // c:1852 zfnopen++

    // c:1888 — zcfinish = 0 (we can now talk).
    ZCFINISH.store(0, Ordering::Relaxed); // c:1894

    // c:1903-1904 — F_SETFD/FD_CLOEXEC on the fd.
    let fd = stream.as_raw_fd();
    unsafe {
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
    }

    // c:1912-1914 — SO_OOBINLINE: in-line OOB data for control conn.
    unsafe {
        let one: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_OOBINLINE,
            &one as *const _ as *const _,
            size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
    // c:1917-1919 — IP_TOS / IPTOS_LOWDELAY for control.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unsafe {
        let lowdelay: libc::c_int = 0x10; // IPTOS_LOWDELAY
        libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            libc::IP_TOS,
            &lowdelay as *const _ as *const _,
            size_of::<libc::c_int>() as libc::socklen_t,
        );
    }

    // c:1923-1936 — store TcpStream as both control and cin.
    let stream_clone = match stream.try_clone() {
        Ok(c) => c,
        Err(_) => {
            zwarnnam(name, "file handling error"); // c:1932
            zfclose(0);
            return 1;
        }
    };
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.control = Some(stream);
            sess.cin = Some(stream_clone);
            sess.connected = true;
            sess.host = Some(hostnam.clone());
            sess.port = resolved_port;
        }
    }

    // c:1947-1950 — read 220 banner.
    if zfgetmsg() >= 4 {
        // c:1947
        zfclose(0); // c:1948
        return 1; // c:1949
    }

    // c:1952-1954 — has_size, has_mdtm, dfd reset; initial status word.
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.has_size = ZFCP_UNKN; // c:1952
            sess.has_mdtm = ZFCP_UNKN; // c:1952
            sess.dfd = -1; // c:1953
            sess.transfer_type = ZFST_ASCI; // c:1955 initial ASCII
        }
    }

    // c:1981-1985 — ZFTP_MODE param, then chain into zftp_login when
    // more args remain.
    zfsetparam("ZFTP_MODE", "S", ZFPM_READONLY); // c:1982
    if effective.len() > 1 {
        // c:1984 *++args
        let rest: Vec<&str> = effective[1..].iter().map(|s| s.as_str()).collect();
        return zftp_login(name, &rest, flags); // c:1985
    }

    // c:1988 — control alive?
    let alive = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.control.is_some()))
        .unwrap_or(false);
    if alive {
        0
    } else {
        1
    } // c:1988 return !control
}

/// Port of `zfgetinfo(char *prompt, int noecho)` from `Src/Modules/zftp.c:1999`.
/// C: `static char * zfgetinfo(char *prompt, int noecho)` — prompt
/// the tty (echoing or with ECHO masked off for passwords) and read
/// one line of input.
#[allow(non_snake_case)]
pub fn zfgetinfo(prompt: &str, noecho: i32) -> Option<String> {
    // c:1999
    // c:2001-2006 — locals.
    let mut resettty: i32 = 0; // c:2001
    let mut instr = String::new(); // c:2005 char instr[256]
    let len: usize = 0; // c:2006 (unused in Rust path)
    let _ = len;

    let saved_termios: Option<libc::termios>;

    // c:2013 — if (isatty(0)) prompt + tty setup.
    if unsafe { libc::isatty(0) } != 0 {
        // c:2013
        if noecho != 0 {
            // c:2014-2033 — `ti = shttyinfo; ti.tio.c_lflag &= ~ECHO;
            //                settyinfo(&ti); resettty = 1;`
            //
            // C reads the saved shell-tty state (shttyinfo global) so
            // the SHTTY fd (which may differ from stdin if the shell
            // re-dup'd) is used consistently. shttyinfo isn't ported
            // as a singleton yet, so use gettyinfo() which reads
            // SHTTY's current termios via fdgettyinfo(SHTTY) — the
            // canonical port at utils.rs:1926. settyinfo() at
            // utils.rs:1964 writes back via fdsettyinfo(SHTTY, ti)
            // with TCSADRAIN and EINTR-retry.
            //
            // Prior port used raw tcgetattr(0) + tcsetattr(0, TCSANOW)
            // which (a) read/wrote stdin instead of SHTTY (wrong when
            // SHTTY was re-dup'd, e.g. `zsh < /dev/tty &`), and (b)
            // used TCSANOW instead of TCSADRAIN, dropping any pending
            // output before the mode change — visible as cut-off
            // prompts on slow terminals.
            if let Some(mut ti) = crate::ported::utils::gettyinfo() {
                saved_termios = Some(ti); // c:2026 ti = shttyinfo (save for restore)
                ti.c_lflag &= !libc::ECHO; // c:2028
                let _ = crate::ported::utils::settyinfo(&ti); // c:2032
                resettty = 1; // c:2033
            } else {
                saved_termios = None;
            }
        } else {
            saved_termios = None;
        }
        // c:2035-2037 — fflush(stdin) + write prompt to stderr.
        eprint!("{}", prompt); // c:2036
        let _ = io::stderr().flush(); // c:2037
    } else {
        saved_termios = None;
    }

    // c:2040-2043 — fgets(instr, 256, stdin); strip trailing \n.
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    match handle.read_line(&mut instr) {
        // c:2040
        Ok(0) => instr.clear(), // c:2041 NULL → empty
        Ok(_) => {
            // c:2042-2043 strip \n
            if instr.ends_with('\n') {
                instr.pop();
            }
        }
        Err(_) => instr.clear(),
    }

    // c:2045 — strret = dupstring(instr); (just keep instr as the result)
    let strret = instr.clone();

    // c:2047-2052 — restore termios if we modified it.
    if resettty != 0 {
        // c:2047
        println!(); // c:2049 '\n' didn't echo
        let _ = io::stdout().flush(); // c:2050
        if let Some(ti) = saved_termios {
            // c:2051 — `settyinfo(&shttyinfo);` — restore via the
            // canonical helper so the SHTTY fd + TCSADRAIN + EINTR
            // retry match C exactly.
            let _ = crate::ported::utils::settyinfo(&ti);
        }
    }

    Some(strret) // c:2054
}

/// Port of `zftp_params(UNUSED(char *name), char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2064`.
/// C: list, clear ("-"), or set the current session's `userparams` array.
#[allow(unused_variables)]
pub fn zftp_params(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2064
    let prompts: [&str; 4] = ["Host: ", "User: ", "Password: ", "Account: "]; // c:2067
                                                                              // c:2071-2083 — no args: print current userparams (mask the password slot).
    if args.is_empty() {
        // c:2071 !*args
        let state = match zftp_state().lock() {
            Ok(s) => s,
            Err(_) => return 1,
        };
        let sess = match state.get_session(None) {
            Some(s) => s,
            None => return 1,
        };
        if sess.userparams.is_empty() {
            // c:2082 else
            return 1; // c:2083
        }
        let mut out = io::stdout().lock();
        for (i, p) in sess.userparams.iter().enumerate() {
            // c:2073 for aptr,i
            if i == 2 {
                // c:2074 i == 2
                let len = p.len(); // c:2075 strlen
                for _ in 0..len {
                    // c:2076 for j<len
                    let _ = out.write_all(b"*"); // c:2077 fputc '*'
                }
                let _ = out.write_all(b"\n"); // c:2078 fputc '\n'
            } else {
                let _ = writeln!(out, "{}", p); // c:2080 fprintf
            }
        }
        return 0; // c:2081
    }
    // c:2084-2089 — single "-" arg: clear userparams.
    if args[0] == "-" {
        // c:2084 !strcmp "-"
        if let Ok(mut state) = zftp_state().lock() {
            if let Some(sess) = state.get_session_mut(None) {
                sess.userparams.clear(); // c:2085-2087 freearray
            }
        }
        return 0; // c:2088
    }
    // c:2090-2103 — replace userparams with new array.
    let len = args.len(); // c:2090 arrlen
    let mut newarr: Vec<String> = Vec::with_capacity(len); // c:2091 zshcalloc
    let efl_atomic = &errflag;
    for (i, aptr) in args.iter().enumerate() {
        // c:2092 for aptr,i
        if efl_atomic.load(Ordering::Relaxed) != 0 {
            // c:2092 !errflag
            break;
        }
        let str_val: String;
        if let Some(rest) = aptr.strip_prefix('?') {
            // c:2094 **aptr == '?'
            let prompt: &str = if !rest.is_empty() {
                rest
            } else {
                prompts[i.min(3)]
            }; // c:2095
            match zfgetinfo(prompt, if i == 2 { 1 } else { 0 }) {
                // c:2095 i == 2
                Some(s) => str_val = s,
                None => {
                    return 1;
                }
            }
        } else if let Some(rest) = aptr.strip_prefix('\\') {
            // c:2097 **aptr=='\\'
            str_val = rest.to_string();
        } else {
            str_val = (*aptr).to_string();
        }
        newarr.push(str_val); // c:2098 ztrdup
    }
    if efl_atomic.load(Ordering::Relaxed) != 0 {
        // c:2100 if errflag
        // c:2101-2104 — free newarr; Rust Drop handles it.
        return 1; // c:2105
    }
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.userparams = newarr; // c:2107-2109
        }
    }
    0 // c:2110
}

/// Port of `zftp_login(char *name, char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2118`.
/// C: send USER/PASS/ACCT, drive the reply state machine, set
/// ZFTP_USER/ACCOUNT/SYSTEM/TYPE parameters, probe SYST type, then
/// pull current directory via `zfgetcwd()`.
#[allow(unused_variables)]
pub fn zftp_login(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2118
    let mut ucmd: String; // c:2120 char *ucmd
    let mut passwd: Option<String> = None; // c:2120 *passwd = NULL
    let mut acct: Option<String> = None; // c:2120 *acct = NULL
    let user: String; // c:2121 char *user
    let mut stopit: i32; // c:2122 int stopit
    let mut arg_idx: usize = 0;

    // c:2124-2125 — already logged in; REIN to reset.
    let already_logged_in = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.logged_in))
        .unwrap_or(false);
    if already_logged_in && zfsendcmd("REIN\r\n") >= 4 {
        // c:2124
        return 1; // c:2125
    }

    // c:2127 — clear ZFST_LOGI.
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.logged_in = false; // c:2127
        }
    }

    // c:2128-2132 — user from args[0] or prompt.
    if arg_idx < args.len() {
        // c:2128 *args
        user = args[arg_idx].to_string(); // c:2129 user = *args++
        arg_idx += 1;
    } else {
        user = match zfgetinfo("User: ", 0) {
            // c:2131
            Some(s) => s,
            None => return 1,
        };
    }

    // c:2134 — tricat("USER ", user, "\r\n").
    ucmd = format!("USER {}\r\n", user);
    stopit = 0; // c:2135

    // c:2137-2138 — first send; ret==6 (write fail) → stopit=2.
    if zfsendcmd(&ucmd) == 6 {
        // c:2137
        stopit = 2; // c:2138
    }

    // c:2140-2174 — state-machine on lastcode.
    let efl_atomic = &errflag;
    while stopit == 0 && efl_atomic.load(Ordering::Relaxed) == 0 {
        // c:2140
        let code = lastcode.load(Ordering::Relaxed);
        match code {
            230 | 202 => {
                // c:2142-2144
                stopit = 1; // c:2145
            }
            331 => {
                // c:2148 need password
                let pw = if arg_idx < args.len() {
                    // c:2149
                    let p = args[arg_idx].to_string(); // c:2150
                    arg_idx += 1;
                    p
                } else {
                    match zfgetinfo("Password: ", 1) {
                        // c:2152
                        Some(s) => s,
                        None => {
                            stopit = 2;
                            break;
                        }
                    }
                };
                passwd = Some(pw.clone()); // c:2120/2150 binding
                                           // c:2153 zsfree(ucmd); c:2154 tricat("PASS ", passwd, "\r\n").
                ucmd = format!("PASS {}\r\n", pw);
                if zfsendcmd(&ucmd) == 6 {
                    // c:2155
                    stopit = 2; // c:2156
                }
            }
            332 | 532 => {
                // c:2160-2161 need account
                let ac = if arg_idx < args.len() {
                    // c:2162
                    let a = args[arg_idx].to_string(); // c:2163
                    arg_idx += 1;
                    a
                } else {
                    match zfgetinfo("Account: ", 0) {
                        // c:2165
                        Some(s) => s,
                        None => {
                            stopit = 2;
                            break;
                        }
                    }
                };
                acct = Some(ac.clone());
                ucmd = format!("ACCT {}\r\n", ac); // c:2167
                if zfsendcmd(&ucmd) == 6 {
                    // c:2168
                    stopit = 2; // c:2169
                }
            }
            // c:2173-2179 — 421/501/503/530/550/default → unrecoverable.
            _ => {
                stopit = 2; // c:2180
            }
        }
    }
    // c:2184 zsfree(ucmd) — Rust Drop.
    let _ = passwd; // suppress unused-warn; password kept only for parity

    // c:2185-2186 — control gone after exchange.
    let control_alive = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.control.is_some()))
        .unwrap_or(false);
    if !control_alive {
        // c:2185
        return 1; // c:2186
    }
    // c:2187-2190 — login failed.
    let code = lastcode.load(Ordering::Relaxed);
    if stopit == 2 || (code != 230 && code != 202) {
        // c:2187
        zwarnnam(name, "login failed"); // c:2188
        return 1; // c:2189
    }

    // c:2192-2197 — warn on unused trailing args.
    if arg_idx < args.len() {
        // c:2192
        let cnt = args.len() - arg_idx; // c:2193-2194
        zwarnnam(
            name,
            &format!("warning: {} command arguments not used", cnt), // c:2195
        );
    }

    // c:2198 — set ZFST_LOGI on the session.
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.logged_in = true; // c:2198
            sess.user = Some(user.clone());
        }
    }
    // c:2199 — ZFTP_USER readonly param.
    zfsetparam("ZFTP_USER", &user, ZFPM_READONLY); // c:2199
    if let Some(ref a) = acct {
        // c:2200
        zfsetparam("ZFTP_ACCOUNT", a, ZFPM_READONLY); // c:2201
    }

    // c:2207-2226 — SYST probe gated by per-session ZFST_SYST cache bit
    // AND zfprefs ZFPF_DUMB bit (when DUMB set, skip the probe entirely).
    let already_probed = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.syst_probed))
        .unwrap_or(false);
    let dumb = (zfprefs.load(Ordering::Relaxed) & ZFPF_DUMB) != 0; // c:2207
    if !dumb && !already_probed && zfsendcmd("SYST\r\n") == 2 {
        // c:2208
        let systype = lastmsg.lock().ok().map(|m| m.clone()).unwrap_or_default();
        if systype.starts_with("UNIX Type: L8") {
            // c:2212-2218
            if let Ok(mut state) = zftp_state().lock() {
                if let Some(sess) = state.get_session_mut(None) {
                    sess.transfer_type = ZFST_IMAG; // c:2220
                }
            }
        }
        zfsetparam("ZFTP_SYSTEM", &systype, ZFPM_READONLY); // c:2222
                                                            // c:2224 — zfstatusp[zfsessno] |= ZFST_SYST.
        if let Ok(mut state) = zftp_state().lock() {
            if let Some(sess) = state.get_session_mut(None) {
                sess.syst_probed = true; // c:2224
            }
        }
    }

    // c:2228-2230 — ZFTP_TYPE param.
    let ttype = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.transfer_type))
        .unwrap_or(ZFST_ASCI);
    let tbuf = if ZFST_TYPE(ttype) == ZFST_ASCI {
        "A"
    } else {
        "I"
    }; // c:2228
    zfsetparam("ZFTP_TYPE", tbuf, ZFPM_READONLY); // c:2229

    // c:2236 — fetch current directory.
    zfgetcwd() // c:2236
}

/// Port of `zftp_test(UNUSED(char *name), UNUSED(char **args), UNUSED(int flags))` from `Src/Modules/zftp.c:2251`.
/// C: `static int zftp_test(char *name, char **args, int flags)` —
/// returns 0 when the current session has a live control connection,
/// 1 otherwise (zftpcmdtab flags = ZFTP_TEST).
#[allow(unused_variables)]
pub fn zftp_test(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2251
    // c:2251 — early-return when no control connection.
    let control_fd = zftp_state().lock().ok().and_then(|s| {
        s.get_session(None)
            .and_then(|sess| sess.control.as_ref().map(|c| c.as_raw_fd()))
    });
    let fd = match control_fd {
        // c:2262
        Some(f) => f,
        None => return 1, // c:2263
    };
    // c:2266-2280 — poll(2) with 0 timeout. POLLIN events on the
    // control fd mean the server pushed an unsolicited message (e.g.
    // "421 Timeout") — consume it via zfgetmsg.
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ret = unsafe { libc::poll(&mut pfd, 1, 0) }; // c:2272
    let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
    if ret < 0 && errno != libc::EINTR && errno != libc::EAGAIN {
        // c:2273
        zfclose(0); // c:2274
    } else if ret > 0 && pfd.revents != 0 {
        // c:2275
        zfgetmsg(); // c:2277 handles 421
    }
    // c:2305 — return zfsess->control ? 0 : 2;
    let still_alive = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.control.is_some()))
        .unwrap_or(false);
    if still_alive {
        0
    } else {
        2
    } // c:2305
}

/// Port of `zftp_dir(char *name, char **args, int flags)` from `Src/Modules/zftp.c:2305`.
pub fn zftp_dir(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2305
    let cmd: String; // c:2305 char *cmd
    let ret: i32; // c:2309 int ret
                  // c:2316 — zfsettype(ZFST_ASCI); RFC959 requires ASCII for LIST.
    zfsettype(ZFST_ASCI);
    // c:2318 — cmd = zfargstring(NLST or LIST, args);
    let verb = if (flags & ZFTP_NLST) != 0 {
        "NLST"
    } else {
        "LIST"
    };
    cmd = zfargstring(verb, args) + "\r\n";
    // c:2319 — ret = zfgetdata(name, NULL, cmd, 0);
    ret = zfgetdata(name, "", &cmd, 0);
    // c:2332 zsfree(cmd) — Rust Drop.
    if ret != 0 {
        return 1;
    } // c:2332-2322
    let _ = io::stdout().flush(); // c:2332
    zfsenddata(name, 1, 0, 0) // c:2332
}

/// Port of `zftp_cd(UNUSED(char *name), char **args, int flags)` from `Src/Modules/zftp.c:2332`.
/// C: send `CDUP\r\n` or `CWD <dir>\r\n` based on flags + arg shape.
/// Then call zfgetcwd to update the cached pwd.
#[allow(unused_variables)]
pub fn zftp_cd(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2332
    let ret: i32; // c:2332 int ret
                  // c:2337-2340 — CDUP when flag set OR arg is ".." / "../".
    let arg0 = args.first().copied().unwrap_or("");
    if (flags & ZFTP_CDUP) != 0 || arg0 == ".." || arg0 == "../" {
        // c:2337
        ret = zfsendcmd("CDUP\r\n"); // c:2339
    } else {
        // c:2340
        let cmd = format!("CWD {}\r\n", arg0); // c:2341 tricat
        ret = zfsendcmd(&cmd); // c:2342
                               // c:2343 zsfree — Rust Drop.
    }
    if ret > 2 {
        return 1;
    } // c:2345
      // c:2347-2349 — `if (zfgetcwd()) return 1;`
    if zfgetcwd() != 0 {
        return 1;
    }
    0 // c:2351
}

/// Port of `zfgetcwd()` from `Src/Modules/zftp.c:2358`.
/// C: `static int zfgetcwd(void)` — sends PWD, parses reply.
///
/// C body (c:2358-2386) is 29 lines covering the response-text
/// parse (extract dir between `"`s in PWD reply) + parameter
/// updates. Rust port focuses on the observable side-effect
/// (send PWD, set/unset `$ZFTP_PWD`); the response-text mangling
/// is layered above this entry once the `lastmsg` parser is
/// wired. Rust idiom replacement covers the control-flow shape.
#[allow(non_snake_case)]
pub fn zfgetcwd() -> i32 {
    // c:2358 — short-circuit when ZFPF_DUMB is set (don't fiddle with
    // variables in dumb mode).
    if (zfprefs.load(Ordering::Relaxed) & ZFPF_DUMB) != 0 {
        // c:2364
        return 1; // c:2365
    }
    if zfsendcmd("PWD\r\n") > 2 {
        // c:2366
        zfunsetparam("ZFTP_PWD"); // c:2367
        return 1; // c:2368
    }
    // c:2370-2382 — parse the PWD reply to extract the directory
    // path between optional `"` delimiters.
    //
    //   ptr = lastmsg;
    //   while (*ptr == ' ') ptr++;
    //   if (!*ptr) return 1;                 /* ultra safe */
    //   if (*ptr == '"') { ptr++; endc = '"'; }
    //   else endc = ' ';
    //   for (eptr = ptr; *eptr && *eptr != endc; eptr++);
    //   zfsetparam("ZFTP_PWD", ztrduppfx(ptr, eptr-ptr), ZFPM_READONLY);
    //
    // Prior port collapsed this to "set if PWD command succeeded"
    // without actually setting ZFTP_PWD to the parsed path. Scripts
    // reading `$ZFTP_PWD` after `zftp cd somewhere` got a stale value
    // from a previous chdir or empty string entirely.
    let pwd_msg = lastmsg.lock().ok().map(|m| m.clone()).unwrap_or_default();
    let trimmed = pwd_msg.trim_start_matches(' '); // c:2371-2372
    if trimmed.is_empty() {
        // c:2373 — `if (!*ptr) return 1;`
        return 1; // c:2374
    }
    let (rest, endc) = if let Some(s) = trimmed.strip_prefix('"') {
        // c:2375-2378 — `if (*ptr == '"') { ptr++; endc = '"'; }`
        (s, '"')
    } else {
        // c:2378-2379 — `else endc = ' ';`
        (trimmed, ' ')
    };
    let path: String = rest.chars().take_while(|&c| c != endc).collect(); // c:2380-2381
    zfsetparam("ZFTP_PWD", &path, ZFPM_READONLY); // c:2382

    let cwd_ret = 0; // c:2396 — `return 0;` post-parse success

    // c:2388-2393 — zftp_chpwd hook: fire the shfunc with SFC_HOOK
    // context after ZFTP_PWD has been updated.
    if let Some(mut shfunc) = getshfunc("zftp_chpwd") {
        let osc = SFCONTEXT.load(Ordering::Relaxed);
        SFCONTEXT.store(SFC_HOOK, Ordering::Relaxed);
        // c:2393 — `doshfunc(shfunc, NULL, 1);`.
        let body_runner = || -> i32 {
            crate::ported::exec_hooks::run_function_body("zftp_chpwd", &[]).unwrap_or(0)
        };
        let _ = crate::ported::exec::doshfunc(
            &mut shfunc,
            vec!["zftp_chpwd".to_string()],
            true,
            body_runner,
        );
        SFCONTEXT.store(osc, Ordering::Relaxed);
    }
    cwd_ret
}

/// Port of `zfsettype(int type)` from `Src/Modules/zftp.c:2404-2417`.
/// C: `static int zfsettype(int type)` — when the requested type differs
/// from the server's current type (ZFST_CTYP), send `TYPE A` or `TYPE I`
/// and on >2 response return 1 leaving CTYP unchanged; on success clear
/// the CTYP bits in `zfstatusp[zfsessno]` then set them from `type`.
/// `type` is renamed `typ` in Rust because `type` is a keyword.
#[allow(non_snake_case)]
pub fn zfsettype(typ: i32) -> i32 {
    // c:2407 — char buf[] = "TYPE X\r\n";
    let mut buf: [u8; 8] = *b"TYPE X\r\n";
    // c:2408 — already at this type? return 0.
    let cur_ctyp = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.current_type))
        .unwrap_or(ZFST_CASC);
    if ZFST_TYPE(typ) == cur_ctyp {
        return 0; // c:2409
    }
    // c:2410 — buf[5] = (ZFST_TYPE(type) == ZFST_ASCI) ? 'A' : 'I';
    buf[5] = if ZFST_TYPE(typ) == ZFST_ASCI { b'A' } else { b'I' };
    let cmd = std::str::from_utf8(&buf).unwrap_or("TYPE A\r\n");
    // c:2411 — if (zfsendcmd(buf) > 2) return 1;
    if zfsendcmd(cmd) > 2 {
        return 1; // c:2412
    }
    // c:2413-2415 — clear current-type bits, then set from `type`.
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            // C: zfstatusp[zfsessno] &= ~(ZFST_TMSK << ZFST_TBIT);
            //    zfstatusp[zfsessno] |=  type        << ZFST_TBIT;
            // Rust models the CTYP slot as a dedicated field rather than
            // a bit slice, so the equivalent is `current_type = ZFST_TYPE(typ)`.
            sess.current_type = ZFST_TYPE(typ); // c:2413-2415
        }
    }
    0 // c:2416
}

/// Port of `zftp_type(char *name, char **args, int flags)` from `Src/Modules/zftp.c:2426`.
/// C: set the transfer-type byte (A=ASCII, I=binary). When ZFTP_TASC/
/// ZFTP_TBIN flags are set (ascii/binary subcommands route here),
/// pick the type from the flag; otherwise read from `args[0]`. With no
/// args, print the current type.
pub fn zftp_type(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2426
    let mut tbuf: [u8; 2] = [b'A', 0]; // c:2428 char tbuf[2] = "A"
    let nt: u8; // c:2428 char nt
    let str: &str; // c:2428 char *str
    let _ = str;

    if (flags & (ZFTP_TBIN | ZFTP_TASC)) != 0 {
        // c:2429
        nt = if (flags & ZFTP_TBIN) != 0 { b'I' } else { b'A' }; // c:2430
    } else if args.first().copied().unwrap_or("").is_empty() {
        // c:2431
        // No args: print current type ('A' or 'I').
        let ttype = zftp_state()
            .lock()
            .ok()
            .and_then(|s| s.get_session(None).map(|sess| sess.transfer_type))
            .unwrap_or(ZFST_IMAG as i32);
        let is_ascii = (ttype & ZFST_ASCI) != 0;
        println!("{}", if is_ascii { 'A' } else { 'I' }); // c:2436-2437
        let _ = io::stdout().flush(); // c:2438
        return 0; // c:2439
    } else {
        str = args[0];
        let c0 = str.as_bytes()[0].to_ascii_uppercase(); // c:2441 toupper
        if str.len() > 1 || (c0 != b'A' && c0 != b'B' && c0 != b'I') {
            // c:2446
            zwarnnam(
                name, // c:2447
                &format!("transfer type {} not recognised", str),
            );
            return 1; // c:2448
        }
        nt = if c0 == b'B' { b'I' } else { c0 }; // c:2451-2452
    }

    // c:2455-2456 — update zfstatusp[zfsessno] (kept on the session.transfer_type).
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.transfer_type = if nt == b'I' { ZFST_IMAG } else { ZFST_ASCI } as i32;
        }
    }
    tbuf[0] = nt; // c:2464
    let tb = std::str::from_utf8(&tbuf[..1]).unwrap_or("A");
    zfsetparam("ZFTP_TYPE", tb, ZFPM_READONLY); // c:2464
    0 // c:2464
}

/// Port of `zftp_mode(char *name, char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2464`.
/// C: set stream-mode (S=stream, B=block). With no arg, print current.
#[allow(unused_variables)]
pub fn zftp_mode(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2464
    let str: &str;
    let nt: u8; // c:2467 int nt

    if args.first().copied().unwrap_or("").is_empty() {
        // c:2469
        let tmode = zftp_state()
            .lock()
            .ok()
            .and_then(|s| s.get_session(None).map(|sess| sess.transfer_mode))
            .unwrap_or(ZFST_STRE as i32);
        let is_stream = tmode == ZFST_STRE;
        println!("{}", if is_stream { 'S' } else { 'B' }); // c:2470-2471
        let _ = io::stdout().flush(); // c:2472
        return 0; // c:2473
    }
    str = args[0];
    nt = str.as_bytes()[0].to_ascii_uppercase(); // c:2475
    if str.len() > 1 || (nt != b'S' && nt != b'B') {
        // c:2476
        zwarnnam(
            name, // c:2477
            &format!("transfer mode {} not recognised", str),
        );
        return 1; // c:2478
    }
    let cmd = format!("MODE {}\r\n", nt as char); // c:2480 cmd[5] = nt
    if zfsendcmd(&cmd) > 2 {
        // c:2481
        return 1; // c:2482
    }
    // c:2483-2484 — update session transfer_mode.
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.transfer_mode = if nt == b'S' { ZFST_STRE } else { ZFST_BLOC } as i32;
        }
    }
    let mb = (nt as char).to_string();
    zfsetparam("ZFTP_MODE", &mb, ZFPM_READONLY); // c:2491
    0 // c:2491
}

/// Port of `zftp_local(UNUSED(char *name), char **args, int flags)` from `Src/Modules/zftp.c:2491`.
#[allow(unused_variables)]
pub fn zftp_local(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2491
    let more = args.len() > 1; // c:2493 more = !!args[1]
    let mut ret: i32 = 0; // c:2493 ret = 0
    let dofd = args.is_empty(); // c:2493 dofd = !*args
    let mut i = 0usize;
    loop {
        // c:2494 while (*args || dofd)
        if !dofd && i >= args.len() {
            break;
        }
        let arg = if dofd { "" } else { args[i] };
        let mut sz: libc::off_t = 0; // c:2495 off_t sz
        let mut mt: Option<String> = None; // c:2496 char *mt
                                           // c:2497-2498 — zfstats(*args, !(flags & ZFTP_HERE), &sz, &mt, dofd ? 0 : -1);
        let remote = if (flags & ZFTP_HERE) != 0 { 0 } else { 1 };
        let fd = if dofd { 0 } else { -1 };
        let newret = zfstats(arg, remote, &mut sz, &mut mt, fd);
        if newret == 2 {
            // c:2499
            return 2; // c:2500
        } else if newret != 0 {
            // c:2501
            ret = 1; // c:2502
                     // c:2503-2504 — Rust Drop.
            i += 1; // c:2505
            continue; // c:2506
        }
        if more {
            // c:2508
            print!("{} ", arg); // c:2509-2510
        }
        let mt_s = mt.unwrap_or_default();
        println!("{} {}", sz, mt_s); // c:2517
        if dofd {
            break;
        } // c:2520-2521
        i += 1; // c:2522
    }
    let _ = io::stdout().flush(); // c:2544
    ret // c:2544
}

/// Port of `zftp_getput(char *name, char **args, int flags)` from `Src/Modules/zftp.c:2544`.
pub fn zftp_getput(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2544
    let mut ret: i32 = 0; // c:2546 ret = 0
    let recv = (flags & ZFTP_RECV) != 0; // c:2546 recv
    let mut getsize: i32 = 0; // c:2546 getsize = 0
    let progress: i32 = 1; // c:2546 progress = 1
    let cmd_pfx = if recv {
        "RETR "
    }
    // c:2547
    else if (flags & ZFTP_APPE) != 0 {
        "APPE "
    } else {
        "STOR "
    };

    // c:2559 — zfsettype(ZFST_TYPE(zfstatusp[zfsessno]));
    let ttype = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.transfer_type))
        .unwrap_or(ZFST_IMAG as i32);
    zfsettype(ttype);

    if recv {
        let _ = io::stdout().flush();
    } // c:2561-2562

    // c:2563 — for (; *args; args++) — with REST advancing args twice.
    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i];
        let mut rest_cmd: String = String::new(); // c:2564 char *rest = NULL
        let mut startat: libc::off_t = 0; // c:2565

        // c:2566-2587 — getsize hint via zfstats + initial progress
        // callback. Only fires when a zftp_progress shfunc is defined.
        // ZFST_NOSZ/ZFST_TRSZ per-session status bits aren't ported;
        // the c:2577-2585 cache check collapses to "always probe SIZE"
        // (or "always set getsize on STOR") — matching C's behavior
        // when those bits are unset on a fresh session.
        if progress != 0 && getshfunc("zftp_progress").is_some() {
            let mut sz: libc::off_t = -1; // c:2567
            let mut _mdtm: Option<String> = None;
            // c:2576-2585 — DUMB-gated SIZE probe. C also consults
            // ZFST_NOSZ/ZFST_TRSZ status bits to avoid re-probing when
            // the server already proved it doesn't send the size; that
            // cache isn't ported per-session, so the gate collapses
            // to !DUMB ON RECV (matches behavior on first transfer).
            let dumb = (zfprefs.load(Ordering::Relaxed) & ZFPF_DUMB) != 0; // c:2576
            if !dumb && (recv || (flags & ZFTP_REST) == 0) {
                // c:2576-2578
                let _ = zfstats(
                    arg,
                    if recv { 1 } else { 0 }, // c:2580
                    &mut sz,
                    &mut _mdtm,
                    0,
                );
                if recv && sz == -1 {
                    // c:2582
                    getsize = 1; // c:2583
                }
            } else {
                getsize = 1; // c:2585
            }
            zfstarttrans(arg, if recv { 1 } else { 0 }, sz); // c:2587
        }

        // c:2589-2592 — REST resume.
        if (flags & ZFTP_REST) != 0 && i + 1 < args.len() {
            startat = args[i + 1].parse().unwrap_or(0);
            rest_cmd = format!("REST {}\r\n", args[i + 1]);
        }

        // c:2594 — ln = tricat(cmd, *args, "\r\n");
        let ln = format!("{}{}\r\n", cmd_pfx, arg);

        // c:2596 — zfgetdata returns 0 on success, 1 on failure.
        let gd = zfgetdata(name, &rest_cmd, &ln, getsize);
        if gd != 0 {
            ret = 2; // c:2597
        } else {
            // c:2598 — zfsenddata(name, recv, progress, startat).
            if zfsenddata(name, if recv { 1 } else { 0 }, progress, startat) != 0 {
                ret = 1; // c:2599
            }
        }
        // c:2600 — zsfree(ln) — Rust Drop.

        // c:2606-2616 — final progress callback (zftp_progress shfunc).
        if progress != 0 && ret != 2 {
            if let Some(mut shfunc) = getshfunc("zftp_progress") {
                // c:2607
                let osc = SFCONTEXT.load(Ordering::Relaxed); // c:2610
                zfsetparam(
                    "ZFTP_TRANSFER", // c:2611-2612
                    if recv { "GF" } else { "PF" },
                    ZFPM_READONLY,
                );
                SFCONTEXT.store(SFC_HOOK, Ordering::Relaxed); // c:2613
                                                              // c:2614 — `doshfunc(shfunc, NULL, 1);`.
                let body_runner = || -> i32 {
                    crate::ported::exec_hooks::run_function_body("zftp_progress", &[]).unwrap_or(0)
                };
                let _ = crate::ported::exec::doshfunc(
                    &mut shfunc,
                    vec!["zftp_progress".to_string()],
                    true,
                    body_runner,
                );
                SFCONTEXT.store(osc, Ordering::Relaxed);
                // c:2615
            } else {
                zfsetparam(
                    "ZFTP_TRANSFER", // c:2611-2612 fallback
                    if recv { "GF" } else { "PF" },
                    ZFPM_READONLY,
                );
            }
        }
        // c:2617-2620 — REST consumed two args.
        if (flags & ZFTP_REST) != 0 {
            i += 1;
        }
        // c:2621-2622 — break on errflag.
        if errflag.load(Ordering::Relaxed) != 0 {
            break;
        }
        let _ = getsize;
        getsize = 0; // reset per-iteration
        i += 1;
    }
    zfendtrans(); // c:2635
    if ret != 0 {
        1
    } else {
        0
    } // c:2635
}

/// Port of `zftp_delete(UNUSED(char *name), char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2635`.
/// C: walk `args`, send `DELE <name>\r\n` for each. Returns 1 if any
/// DELE got a 3xx+ reply, else 0.
#[allow(unused_variables)]
pub fn zftp_delete(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2635
    let mut ret: i32 = 0; // c:2635 int ret = 0
    let cmd: String; // c:2638 char *cmd
    let _ = cmd;
    for aptr in args.iter() {
        // c:2639 for (aptr = args; *aptr; aptr++)
        let cmd = format!("DELE {}\r\n", aptr); // c:2640 tricat("DELE ", *aptr, "\r\n")
        if zfsendcmd(&cmd) > 2 {
            // c:2652
            ret = 1; // c:2652
        }
        // c:2652 zsfree(cmd) — Rust Drop.
    }
    ret // c:2652
}

/// Port of `zftp_mkdir(UNUSED(char *name), char **args, int flags)` from `Src/Modules/zftp.c:2652`.
/// C: send `MKD <args[0]>\r\n` (or `RMD` when ZFTP_DELE flag set —
/// the `rmdir` subcommand routes through this fn with that bit).
#[allow(unused_variables)]
pub fn zftp_mkdir(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2652
    let ret: i32; // c:2652 int ret
    if args.is_empty() {
        return 1;
    }
    let cmd_pfx = if (flags & ZFTP_DELE) != 0 {
        "RMD "
    } else {
        "MKD "
    }; // c:2666
    let cmd = format!("{}{}\r\n", cmd_pfx, args[0]); // c:2666 tricat
    ret = (zfsendcmd(&cmd) > 2) as i32; // c:2666
                                        // c:2666 zsfree — Rust Drop.
    ret // c:2666
}

/// Port of `zftp_rename(UNUSED(char *name), char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2666`.
/// C: send RNFR (rename-from), expect 3xx, then send RNTO (rename-to),
/// expect 2xx. Two-phase rename per RFC959.
#[allow(unused_variables)]
pub fn zftp_rename(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2666
    let mut ret: i32; // c:2666 int ret
    let cmd: String; // c:2669 char *cmd
    let _ = cmd;
    if args.len() < 2 {
        return 1;
    }

    let cmd = format!("RNFR {}\r\n", args[0]); // c:2671 tricat("RNFR ", args[0], "\r\n")
    ret = 1; // c:2672
    if zfsendcmd(&cmd) == 3 {
        // c:2673
        // c:2674 zsfree(cmd) — Rust Drop.
        let cmd = format!("RNTO {}\r\n", args[1]); // c:2675
        if zfsendcmd(&cmd) == 2 {
            // c:2676
            ret = 0; // c:2690
        }
    }
    // c:2690 zsfree(cmd) — Rust Drop.
    ret // c:2690
}

/// Port of `zftp_quote(UNUSED(char *name), char **args, int flags)` from `Src/Modules/zftp.c:2690`.
/// C: send a raw FTP command, optionally prefixed with `SITE ` when
/// ZFTP_SITE flag is set (the `site` subcommand routes here with the
/// bit). The first arg is the verb; subsequent args are appended.
#[allow(unused_variables)]
pub fn zftp_quote(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2690
    let ret: i32; // c:2690 int ret = 0
    let cmd: String; // c:2693 char *cmd
    let _ = cmd;
    let argv: Vec<&str> = args.to_vec();
    let cmd = if (flags & ZFTP_SITE) != 0 {
        // c:2695
        zfargstring("SITE", &argv) + "\r\n"
    } else {
        if argv.is_empty() {
            return 1;
        }
        zfargstring(argv[0], &argv[1..]) + "\r\n" // c:2696
    };
    ret = (zfsendcmd(&cmd) > 2) as i32; // c:2697
                                        // c:2698 zsfree — Rust Drop.
    ret // c:2700
}

/// Port of `zfclose(int leaveparams)` from `Src/Modules/zftp.c:2711`.
/// C: `void zfclose(int leaveparams)` — close the control connection
/// (and optionally tear down the ZFTP_* params), run the zftp_chpwd
/// hook, reset zfclosing+zfdrrrring tidy-up flags.
#[allow(non_snake_case)]
pub fn zfclose(leaveparams: i32) {
    // c:2711
    // c:2715-2716 — early-return when no live control connection.
    let alive = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.get_session(None).map(|sess| sess.control.is_some()))
        .unwrap_or(false);
    if !alive {
        // c:2715
        return; // c:2716
    }
    // c:2718 — zfclosing = 1.
    ZFCLOSING.store(1, Ordering::Relaxed); // c:2718
                                           // c:2719-2725 — send QUIT before teardown unless server already EOF'd.
    if ZCFINISH.load(Ordering::Relaxed) != 2 {
        // c:2719
        let _ = zfsendcmd("QUIT\r\n"); // c:2724
    }
    // c:2727-2737 — drop cin + control TcpStream + decrement zfnopen.
    if let Ok(mut state) = zftp_state().lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.cin = None; // c:2735 fclose(cin)
            sess.control = None; // c:2745 tcp_close
            sess.dfd = -1;
            sess.connected = false;
            sess.logged_in = false;
        }
    }
    ZFNOPEN.fetch_sub(1, Ordering::Relaxed); // c:2743

    // c:2749-2759 — zfstatusp ZFST_CLOS + close zfstatfd when last open
    // session goes away. zfstatfd substrate not ported — skipped.

    // c:2761-2774 — !leaveparams: unset ZFTP_* params + zftp_chpwd hook.
    if leaveparams == 0 {
        // c:2761
        for n in [
            "ZFTP_HOST",
            "ZFTP_PORT",
            "ZFTP_IP",
            "ZFTP_SYSTEM", // c:2763 zfparams[]
            "ZFTP_USER",
            "ZFTP_ACCOUNT",
            "ZFTP_PWD",
            "ZFTP_TYPE",
            "ZFTP_MODE",
        ] {
            zfunsetparam(n); // c:2764
        }
        // c:2767-2773 — zftp_chpwd shfunc dispatch.
        if let Some(mut shfunc) = getshfunc("zftp_chpwd") {
            // c:2767
            let osc = SFCONTEXT.load(Ordering::Relaxed);
            SFCONTEXT.store(SFC_HOOK, Ordering::Relaxed); // c:2770
                                                          // c:2771 — `doshfunc(shfunc, NULL, 1);`.
            let body_runner = || -> i32 {
                crate::ported::exec_hooks::run_function_body("zftp_chpwd", &[]).unwrap_or(0)
            };
            let _ = crate::ported::exec::doshfunc(
                &mut shfunc,
                vec!["zftp_chpwd".to_string()],
                true,
                body_runner,
            );
            SFCONTEXT.store(osc, Ordering::Relaxed); // c:2772
        }
    }
    // c:2777 — zfclosing = zfdrrrring = 0.
    ZFCLOSING.store(0, Ordering::Relaxed); // c:2777
    ZFDRRRRING.store(0, Ordering::Relaxed); // c:2777
}

/// Port of `zftp_close(UNUSED(char *name), UNUSED(char **args), UNUSED(int flags))` from `Src/Modules/zftp.c:2782`.
/// C: `static int zftp_close(UNUSED(char *name), UNUSED(char **args),
/// UNUSED(int flags))` — closes the current session's control
/// connection. Body is a single zfclose(0) call.
#[allow(unused_variables)]
pub fn zftp_close(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2782
    zfclose(0); // c:2782
    0 // c:2785
}

/// Port of `newsession(char *nm)` from `Src/Modules/zftp.c:2803`.
///
/// Direct line-by-line port. Walks `zfsessions` looking for an
/// existing session named `nm` (c:2806-2812); if missing, allocates a
/// fresh `zftp_session`, registers it in the global state, sets
/// `dfd = -1`, and seeds an empty `params` slot (c:2814-2823).
#[allow(non_snake_case)]
pub fn newsession(nm: &str) -> Box<zftp_session> {
    // c:2806-2812 — walk zfsessions looking for a session matching `nm`.
    //                In Rust the linked-list walk collapses to a HashMap
    //                contains_key; on hit we drop through (C `break`s out
    //                of the loop with zfsess still pointing at the match)
    //                without re-inserting.
    if let Ok(state) = zftp_state().lock() {
        if state.sessions.contains_key(nm) {
            // c:2812 — session exists, leave the entry in place; caller
            //          can fetch it via `zftp_state().sessions.get(nm)`.
            return Box::new(zftp_session::new(nm));
        }
    }

    // c:2814-2823 — alloc a fresh session, register it.
    let mut sess = zftp_session::new(nm); // c:2814 zshcalloc(sizeof(struct zftp_session))
    sess.name = nm.to_string(); // c:2815 zfsess->name = ztrdup(nm)
    sess.dfd = -1; // c:2816 zfsess->dfd = -1
    sess.params.clear(); // c:2817 — empty params slot

    if let Ok(mut state) = zftp_state().lock() {
        // c:2818 — zaddlinknode(zfsessions, zfsess).
        let cloned = zftp_session::new(nm);
        state.sessions.insert(nm.to_string(), cloned);
        // c:2820-2822 — zfsesscnt++ + zfstatusp realloc.
        // Rust per-session status lives on zftp_session.transfer_type;
        // counter is implicit in sessions.len().
    }

    Box::new(sess)
}

/// Port of `savesession()` from `Src/Modules/zftp.c:2832`.
/// C: `static void savesession(void)` — copy each ZFTP_* shell param
/// into zfsess->params so session-switching preserves the values.
#[allow(non_snake_case)]
pub fn savesession() {
    // c:2832
    // c:2832 — char **ps, **pd, *val; (Rust uses indexing over slices)
    let val: String;
    let _ = val;

    if let Ok(mut state) = zftp_state().lock() {
        let sess = match state.get_session_mut(None) {
            Some(s) => s,
            None => return,
        };
        // c:2836-2845 — for each zfparams[i], copy the current shell param.
        sess.params.clear();
        for ps in ZFPARAMS {
            // c:2836
            // c:2840 — `val = getsparam(*ps);`. paramtab is bucket-2-
            //          consolidated now; read directly. Was a fake env
            //          read which never picked up shell-internal params.
            let val = crate::ported::params::getsparam(ps).unwrap_or_default();
            // c:2856 / c:2843 — *pd = ztrdup(val) or NULL.
            sess.params.push(val);
        }
        // c:2856 — *pd = NULL; (terminator) — Rust Vec is self-terminating.
    }
}

/// Port of `switchsession(char *nm)` from `Src/Modules/zftp.c:2856`.
/// C: `static void switchsession(char *nm)`.
#[allow(non_snake_case)]
pub fn switchsession(nm: &str) {
    if let Ok(mut state) = zftp_state().lock() {
        // C: walks zfsessions list for matching `nm`; if missing,
        // creates one. Static-link path: register-or-create on the zftp_globals wrapper.
        let _ = state.create_session(nm);
        state.set_current(nm);
    }
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/zftp.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `freesession(Zftp_session sptr)` from `Src/Modules/zftp.c:2874`.
/// C: `static void freesession(Zftp_session sptr)` — release `sptr`'s
/// name + params + userparams + the struct itself.
#[allow(non_snake_case)]
pub fn freesession(sptr: &mut zftp_session) {
    // c:2874
    // c:2874 — zsfree(sptr->name);
    sptr.name.clear();
    // c:2878-2881 — walk zfparams + sptr->params freeing each param value.
    sptr.params.clear();
    // c:2882-2883 — if (sptr->userparams) freearray(sptr->userparams);
    sptr.userparams.clear();
    // c:2884 — zfree(sptr, sizeof(struct zftp_session)); the caller's
    // owning Box::drop releases the struct memory.
}

/// Port of `zftp_rmsession(UNUSED(char *name), char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2915`.
#[allow(unused_variables)]
pub fn zftp_rmsession(name: &str, args: &[&str], flags: i32) -> i32 {
    // c:2915
    // c:2915-2920 — locals: no, sptr, newsess (Zftp_session linked-list).
    let mut found_name: Option<String> = None; // sptr identity
    let mut is_current: bool = false; // sptr == zfsess
    let mut newsess: Option<String> = None; // c:2920

    // c:2922-2928 — find session by name (or current if no arg).
    {
        let state = match zftp_state().lock() {
            Ok(s) => s,
            Err(_) => return 1,
        };
        let current = state.current_name().unwrap_or("default").to_string();
        let target = args
            .first()
            .copied()
            .map(String::from)
            .unwrap_or_else(|| current.clone());
        for sess_name in state.session_names() {
            // c:2923
            if sess_name == target {
                found_name = Some(sess_name.to_string());
                is_current = sess_name == current;
                break;
            }
        }
    }
    let target_name = match found_name {
        // c:2929
        Some(n) => n,
        None => return 1, // c:2930
    };

    // c:2932-2956 — closing logic differs by current-vs-other.
    if is_current {
        // c:2932
        zfclosedata(); // c:2934
        zfclose(0); // c:2935
                    // c:2941-2946 — pick a new current session if any others remain.
        let other = zftp_state()
            .lock()
            .ok()
            .map(|s| {
                s.session_names()
                    .into_iter()
                    .find(|n| *n != target_name)
                    .map(String::from)
            })
            .unwrap_or(None);
        if let Some(o) = other {
            // c:2945
            newsess = Some(o);
        }
    } else {
        // c:2947
        // c:2948-2956 — temporarily switch to target, close-non-destructive,
        // then switch back. Rust collapses this to a direct close on
        // the named session.
        let prev = zftp_state()
            .lock()
            .ok()
            .and_then(|s| s.current_name().map(String::from));
        if let Ok(mut state) = zftp_state().lock() {
            state.set_current(&target_name); // c:2949
        }
        zfclosedata(); // c:2954
        zfclose(1); // c:2955 (leaveparams=1)
        if let (Some(p), Ok(mut state)) = (prev, zftp_state().lock()) {
            state.set_current(&p); // c:2956
        }
    }

    // c:2964-2993 — remove session from list + switch to newsess if any.
    // Rust port: remove via zftp_globals::remove_session; newsess switch via
    // switchsession (already ported above).
    if let Ok(mut state) = zftp_state().lock() {
        state.remove_session(&target_name);
    }
    if let Some(n) = newsess {
        // c:2982
        switchsession(&n); // c:2983
    } else if zftp_state()
        .lock()
        .map(|s| s.session_names().is_empty())
        .unwrap_or(false)
    {
        // c:2992 — last session gone, start fresh.
        newsession("default");
    }
    0 // c:2995
}

/// Port of `bin_zftp(char *name, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zftp.c:3002`.
/// `zftp` builtin entry point — C-faithful signature matching
/// `static int bin_zftp(char *name, char **args, Options ops, int func)`
/// from Src/Modules/zftp.c:3002. Acquires ZFTP_STATE, dispatches by
/// subcommand string, emits any captured output to stdout/stderr
/// based on status, returns the bare i32 status C's execbuiltin path
/// consumes.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_nam, args, _func) vs C=(name, args, ops, func)
pub fn bin_zftp(
    _nam: &str,
    args: &[String], // c:3002
    _ops: &options,
    _func: i32,
) -> i32 {
    let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut zftp_guard = zftp_state().lock().unwrap_or_else(|e| {
        zftp_state_clear_poison();
        e.into_inner()
    });
    let zftp = &mut *zftp_guard;
    let args = &argv[..];
    let (status, output): (i32, String) = (|| {
        if args.is_empty() {
            return (1, "zftp: subcommand required\n".to_string());
        }

        match args[0] {
            "open" => {
                if args.len() < 2 {
                    return (1, "zftp open: host required\n".to_string());
                }

                let host = args[1];
                let port: Option<u16> = args.get(2).and_then(|s| s.parse().ok());

                let session_name = zftp.current_name().unwrap_or("default").to_string();

                let sess = zftp.create_session(&session_name);

                match sess.connect(host, port) {
                    Ok(resp) => {
                        if (resp.0 >= 100 && resp.0 < 400) {
                            zftp.set_current(&session_name);
                            (0, resp.1)
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp open: {}\n", e)),
                }
            }

            "login" | "user" => {
                if args.len() < 2 {
                    return (1, "zftp login: user required\n".to_string());
                }

                let user = args[1];
                let pass = args.get(2).copied();

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp login: not connected\n".to_string()),
                };

                match sess.login(user, pass) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, resp.1)
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp login: {}\n", e)),
                }
            }

            "cd" => {
                if args.len() < 2 {
                    return (1, "zftp cd: path required\n".to_string());
                }

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp cd: not connected\n".to_string()),
                };

                match sess.cd(args[1]) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, resp.1)
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp cd: {}\n", e)),
                }
            }

            "cdup" => {
                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp cdup: not connected\n".to_string()),
                };

                match sess.cdup() {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, resp.1)
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp cdup: {}\n", e)),
                }
            }

            "pwd" => {
                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp pwd: not connected\n".to_string()),
                };

                match sess.pwd() {
                    Ok((resp, pwd)) => {
                        if let Some(p) = pwd {
                            (0, format!("{}\n", p))
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp pwd: {}\n", e)),
                }
            }

            "dir" | "ls" => {
                let path = args.get(1).copied();
                let use_nlst = args[0] == "ls";

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp dir: not connected\n".to_string()),
                };

                let result = if use_nlst {
                    sess.nlst(path)
                } else {
                    sess.list(path)
                };

                match result {
                    Ok((resp, lines)) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, lines.join("\n") + "\n")
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp dir: {}\n", e)),
                }
            }

            "get" => {
                if args.len() < 2 {
                    return (1, "zftp get: remote file required\n".to_string());
                }

                let remote = args[1];
                let local = args.get(2).unwrap_or(&remote);

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp get: not connected\n".to_string()),
                };

                match sess.get(remote, Path::new(local)) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, String::new())
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp get: {}\n", e)),
                }
            }

            "put" => {
                if args.len() < 2 {
                    return (1, "zftp put: local file required\n".to_string());
                }

                let local = args[1];
                let remote = args.get(2).unwrap_or(&local);

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp put: not connected\n".to_string()),
                };

                match sess.put(Path::new(local), remote) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, String::new())
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp put: {}\n", e)),
                }
            }

            "delete" => {
                if args.len() < 2 {
                    return (1, "zftp delete: file required\n".to_string());
                }

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp delete: not connected\n".to_string()),
                };

                match sess.delete(args[1]) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, String::new())
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp delete: {}\n", e)),
                }
            }

            "mkdir" => {
                if args.len() < 2 {
                    return (1, "zftp mkdir: directory required\n".to_string());
                }

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp mkdir: not connected\n".to_string()),
                };

                match sess.mkdir(args[1]) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, String::new())
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp mkdir: {}\n", e)),
                }
            }

            "rmdir" => {
                if args.len() < 2 {
                    return (1, "zftp rmdir: directory required\n".to_string());
                }

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp rmdir: not connected\n".to_string()),
                };

                match sess.rmdir(args[1]) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, String::new())
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp rmdir: {}\n", e)),
                }
            }

            "rename" => {
                if args.len() < 3 {
                    return (1, "zftp rename: from and to required\n".to_string());
                }

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp rename: not connected\n".to_string()),
                };

                match sess.rename(args[1], args[2]) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, String::new())
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp rename: {}\n", e)),
                }
            }

            "type" | "ascii" | "binary" => {
                let transfer_type = match args[0] {
                    "ascii" => ZFST_ASCI,
                    "binary" => ZFST_IMAG,
                    "type" => {
                        if args.len() < 2 {
                            let sess = match zftp.get_session(None) {
                                Some(s) => s,
                                None => return (1, "zftp type: not connected\n".to_string()),
                            };
                            return (
                                0,
                                format!(
                                    "{}\n",
                                    if sess.transfer_type == ZFST_ASCI {
                                        "ascii"
                                    } else {
                                        "binary"
                                    }
                                ),
                            );
                        }
                        match args[1].to_lowercase().as_str() {
                            "a" | "ascii" => ZFST_ASCI,
                            "i" | "binary" | "image" => ZFST_IMAG,
                            _ => return (1, format!("zftp type: unknown type {}\n", args[1])),
                        }
                    }
                    _ => unreachable!(),
                };

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp type: not connected\n".to_string()),
                };

                match sess.set_type(transfer_type) {
                    Ok(resp) => {
                        if (resp.0 >= 200 && resp.0 < 300) {
                            (0, String::new())
                        } else {
                            (1, resp.1)
                        }
                    }
                    Err(e) => (1, format!("zftp type: {}\n", e)),
                }
            }

            "bslashquote" => {
                if args.len() < 2 {
                    return (1, "zftp bslashquote: command required\n".to_string());
                }

                let cmd = args[1..].join(" ");

                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (1, "zftp bslashquote: not connected\n".to_string()),
                };

                match sess.bslashquote(&cmd) {
                    Ok(resp) => (
                        if (resp.0 >= 100 && resp.0 < 400) {
                            0
                        } else {
                            1
                        },
                        resp.1,
                    ),
                    Err(e) => (1, format!("zftp bslashquote: {}\n", e)),
                }
            }

            "close" | "quit" => {
                let sess = match zftp.get_session_mut(None) {
                    Some(s) => s,
                    None => return (0, String::new()),
                };

                match sess.close() {
                    Ok(_) => (0, String::new()),
                    Err(e) => (1, format!("zftp close: {}\n", e)),
                }
            }

            "session" => {
                if args.len() < 2 {
                    let names = zftp.session_names();
                    let current = zftp.current_name();
                    let mut out = String::new();
                    for name in names {
                        let marker = if Some(name) == current { "* " } else { "  " };
                        out.push_str(&format!("{}{}\n", marker, name));
                    }
                    return (0, out);
                }

                let name = args[1];
                if zftp.sessions.contains_key(name) {
                    zftp.set_current(name);
                } else {
                    zftp.create_session(name);
                    zftp.set_current(name);
                }
                (0, String::new())
            }

            "rmsession" => {
                if args.len() < 2 {
                    return (1, "zftp rmsession: session name required\n".to_string());
                }

                if zftp.remove_session(args[1]).is_some() {
                    (0, String::new())
                } else {
                    (
                        1,
                        format!("zftp rmsession: session {} not found\n", args[1]),
                    )
                }
            }

            "test" => {
                // c:158 zftpcmdtab entry — inline check (zftp_test re-acquires
                // the lock; doing it here avoids cross-fn deadlock).
                let sess = zftp.get_session(None);
                if sess.map(|s| s.connected).unwrap_or(false) {
                    (0, String::new())
                } else {
                    (1, String::new())
                }
            }

            _ => (1, format!("zftp: unknown subcommand {}\n", args[0])),
        }
    })();
    drop(zftp_guard);
    if !output.is_empty() {
        if status == 0 {
            print!("{}", output);
        } else {
            eprint!("{}", output);
        }
    }
    status
}

/// Port of `zftp_cleanup()` from `Src/Modules/zftp.c:3128`. Walks
/// every session sending QUIT (via zfclose) on the current one and
/// closing the control fd on others, then clears the session table.
pub fn zftp_cleanup() -> i32 {
    // c:3128
    // c:3128 — Zftp_session cursess = zfsess; (snapshot current).
    let cursess = zftp_state()
        .lock()
        .ok()
        .and_then(|s| s.current_name().map(|n| n.to_string()));
    let session_names: Vec<String> = zftp_state()
        .lock()
        .ok()
        .map(|s| s.session_names().iter().map(|n| n.to_string()).collect())
        .unwrap_or_default();
    // c:3135-3142 — walk every session: zfclosedata + zfclose(leaveparams).
    for nm in session_names {
        // c:3136
        // c:3137 — zfsess = (Zftp_session)nptr->dat; (switch to session).
        if let Ok(mut s) = zftp_state().lock() {
            let _ = s.set_current(&nm);
        }
        zfclosedata(); // c:3140
                       // c:3144 — zfclose(zfsess != cursess): non-current sessions keep
                       // their params; current session goes through full unset.
        let leaveparams = if Some(&nm) != cursess.as_ref() { 1 } else { 0 };
        zfclose(leaveparams); // c:3144
    }
    // c:3146-3151 — clear lastmsg, ZFTP_SESSION, freelinklist sessions.
    if let Ok(mut m) = lastmsg.lock() {
        m.clear(); // c:3147
    }
    zfunsetparam("ZFTP_SESSION"); // c:3149
    if let Ok(mut state) = zftp_state().lock() {
        *state = zftp_globals::new(); // c:3150 freelinklist
    }
    // c:3152 — zfree(zfstatusp): per-session status array. zfstatusp
    // substrate isn't ported (per-session bits live on zftp_session
    // directly), so nothing to free.
    0
}

impl zftp_session {
    /// Port of `newsession(char *nm)` from `Src/Modules/zftp.c`. C uses
    /// `zshcalloc(sizeof(struct zftp_session))` then sets `name`
    /// (c:2891 `zfsess->name = ztrdup(name);`) and pre-allocates the
    /// `params` / `userparams` arrays. Same default state.
    pub fn new(name: &str) -> Self {
        Self {
            // C-faithful fields from `struct zftp_session` (c:299):
            name: name.to_string(), // c:300
            params: Vec::new(),     // c:301
            userparams: Vec::new(), // c:302
            cin: None,              // c:303 NULL
            control: None,          // c:304 NULL
            dfd: -1,                // c:305 (closed)
            has_size: 0,            // c:306
            has_mdtm: 0,            // c:307
            // Ergonomic Rust-side state mirroring C's params[] indices:
            host: None,
            port: 21,
            user: None,
            pwd: None,
            connected: false,
            logged_in: false,
            transfer_type: ZFST_IMAG,
            // c:2226 — initial current_type is ZFST_CASC (0); the SYST probe
            // in zftp_login sends the first TYPE I when it detects UNIX L8.
            current_type: ZFST_CASC,
            transfer_mode: ZFST_STRE,
            passive: true,
            syst_probed: false,
            nops_probed: false,
        }
    }

    /// Port of `zftp_open(char *name, char **args, int flags)` from `Src/Modules/zftp.c:1690`.
    fn send_command(&mut self, cmd: &str) -> io::Result<()> {
        if let Some(ref mut stream) = self.cin {
            write!(stream, "{}\r\n", cmd)?;
            stream.flush()
        } else {
            Err(io::Error::new(io::ErrorKind::NotConnected, "not connected"))
        }
    }

    /// Port of `zfgetmsg()` from `Src/Modules/zftp.c:702`.
    fn read_response(&mut self) -> io::Result<FtpResponse> {
        let stream = self
            .cin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))?;

        let mut reader = BufReader::new(stream.try_clone()?);
        let mut full_message = String::new();
        let mut code = 0u32;
        let mut multiline = false;
        let mut first_code = String::new();

        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let line = line.trim_end();

            if line.len() < 3 {
                continue;
            }

            if code == 0 {
                first_code = line[..3].to_string();
                code = first_code.parse().unwrap_or(0);

                if line.len() > 3 && line.chars().nth(3) == Some('-') {
                    multiline = true;
                }
            }

            full_message.push_str(line);
            full_message.push('\n');

            if multiline {
                if line.starts_with(&first_code)
                    && line.len() > 3
                    && line.chars().nth(3) == Some(' ')
                {
                    break;
                }
            } else {
                break;
            }
        }

        Ok((code as i32, full_message))
    }

    /// Port of `zftp_open(char *name, char **args, int flags)` from `Src/Modules/zftp.c:1690`.
    /// Connect to FTP server — DNS resolution on background thread to avoid hangs
    pub fn connect(&mut self, host: &str, port: Option<u16>) -> io::Result<FtpResponse> {
        let port = port.unwrap_or(21);
        let addr_str = format!("{}:{}", host, port);
        let dns_timeout = Duration::from_secs(10);

        // DNS on background thread
        let (tx, rx) = std::sync::mpsc::channel();
        let dns = addr_str.clone();
        std::thread::Builder::new()
            .name("zftp-dns".to_string())
            .spawn(move || {
                let _ = tx.send(dns.to_socket_addrs().map(|a| a.collect::<Vec<_>>()));
            })
            .map_err(io::Error::other)?;

        let addrs = rx
            .recv_timeout(dns_timeout)
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS resolution timed out"))?
            .map_err(|e| {
                // C: `zwarnnam("zftp", "host not found: %s", host);` at
                //    `Src/Modules/zftp.c:1715`. Route through canonical
                //    zwarnnam so the user sees a real shell warning
                //    instead of a tracing log line.
                zwarnnam("zftp", &format!("host not found: {}: {}", host, e));
                e
            })?;

        let sock_addr = addrs
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid address"))?;

        let stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(30))?;

        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        stream.set_write_timeout(Some(Duration::from_secs(60)))?;

        self.cin = Some(stream);
        self.host = Some(host.to_string());
        self.port = port;
        self.connected = true;

        self.read_response()
    }

    /// Port of `zftp_login(char *name, char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2118`.
    /// Login to FTP server
    pub fn login(&mut self, user: &str, pass: Option<&str>) -> io::Result<FtpResponse> {
        self.send_command(&format!("USER {}", user))?;
        let resp = self.read_response()?;

        if resp.0 == 331 {
            let password = pass.unwrap_or("");
            self.send_command(&format!("PASS {}", password))?;
            let resp = self.read_response()?;

            if (resp.0 >= 200 && resp.0 < 300) {
                self.logged_in = true;
                self.user = Some(user.to_string());
            }
            return Ok(resp);
        }

        if (resp.0 >= 200 && resp.0 < 300) {
            self.logged_in = true;
            self.user = Some(user.to_string());
        }

        Ok(resp)
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Set transfer type
    pub fn set_type(&mut self, transfer_type: i32) -> io::Result<FtpResponse> {
        // C inline pattern: `(typ & ZFST_IMAG) ? "I" : "A"`
        let typ_letter = if (transfer_type & ZFST_IMAG) != 0 {
            "I"
        } else {
            "A"
        };
        self.send_command(&format!("TYPE {}", typ_letter))?;
        let resp = self.read_response()?;
        if (resp.0 >= 200 && resp.0 < 300) {
            self.transfer_type = transfer_type;
        }
        Ok(resp)
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Change directory
    pub fn cd(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("CWD {}", path))?;
        self.read_response()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Change to parent directory
    pub fn cdup(&mut self) -> io::Result<FtpResponse> {
        self.send_command("CDUP")?;
        self.read_response()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Get current directory
    pub fn pwd(&mut self) -> io::Result<(FtpResponse, Option<String>)> {
        self.send_command("PWD")?;
        let resp = self.read_response()?;

        let pwd = if (resp.0 >= 200 && resp.0 < 300) {
            if let Some(start) = resp.1.find('"') {
                if let Some(end) = resp.1[start + 1..].find('"') {
                    Some(resp.1[start + 1..start + 1 + end].to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        Ok((resp, pwd))
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// List directory
    pub fn list(&mut self, path: Option<&str>) -> io::Result<(FtpResponse, Vec<String>)> {
        let data_stream = self.enter_passive_mode()?;

        let cmd = match path {
            Some(p) => format!("LIST {}", p),
            None => "LIST".to_string(),
        };
        self.send_command(&cmd)?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok((resp, Vec::new()));
        }

        let mut reader = BufReader::new(data_stream);
        let mut lines = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            lines.push(line.trim_end().to_string());
            line.clear();
        }

        let final_resp = self.read_response()?;

        Ok((final_resp, lines))
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// List filenames only
    pub fn nlst(&mut self, path: Option<&str>) -> io::Result<(FtpResponse, Vec<String>)> {
        let data_stream = self.enter_passive_mode()?;

        let cmd = match path {
            Some(p) => format!("NLST {}", p),
            None => "NLST".to_string(),
        };
        self.send_command(&cmd)?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok((resp, Vec::new()));
        }

        let mut reader = BufReader::new(data_stream);
        let mut lines = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            lines.push(line.trim_end().to_string());
            line.clear();
        }

        let final_resp = self.read_response()?;

        Ok((final_resp, lines))
    }

    /// Port of `zftp_open(char *name, char **args, int flags)` from `Src/Modules/zftp.c:1690`.
    fn enter_passive_mode(&mut self) -> io::Result<TcpStream> {
        self.send_command("PASV")?;
        let resp = self.read_response()?;

        if !(resp.0 >= 200 && resp.0 < 300) {
            return Err(io::Error::other(resp.1));
        }

        let (ip, port) = parse_pasv_response(&resp.1)?;
        let addr = format!("{}:{}", ip, port);

        TcpStream::connect_timeout(
            &addr.to_socket_addrs()?.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid PASV address")
            })?,
            Duration::from_secs(30),
        )
    }

    /// Port of `zfstats(char *fnam, int remote, off_t *retsize, char **retmdtm, int fd)` from `Src/Modules/zftp.c:1193`.
    /// Download a file
    pub fn get(&mut self, remote: &str, local: &Path) -> io::Result<FtpResponse> {
        let mut data_stream = self.enter_passive_mode()?;

        self.send_command(&format!("RETR {}", remote))?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok(resp);
        }

        let mut file = std::fs::File::create(local)?;
        let mut buf = [0u8; 8192];
        loop {
            let n = data_stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
        }

        self.read_response()
    }

    /// Port of `zfstats(char *fnam, int remote, off_t *retsize, char **retmdtm, int fd)` from `Src/Modules/zftp.c:1193`.
    /// Upload a file
    pub fn put(&mut self, local: &Path, remote: &str) -> io::Result<FtpResponse> {
        let mut data_stream = self.enter_passive_mode()?;

        self.send_command(&format!("STOR {}", remote))?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok(resp);
        }

        let mut file = std::fs::File::open(local)?;
        let mut buf = [0u8; 8192];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            data_stream.write_all(&buf[..n])?;
        }
        drop(data_stream);

        self.read_response()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Delete a file
    pub fn delete(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("DELE {}", path))?;
        self.read_response()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Make directory
    pub fn mkdir(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("MKD {}", path))?;
        self.read_response()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Remove directory
    pub fn rmdir(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("RMD {}", path))?;
        self.read_response()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Rename file
    pub fn rename(&mut self, from: &str, to: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("RNFR {}", from))?;
        let resp = self.read_response()?;

        if !(resp.0 >= 300 && resp.0 < 400) {
            return Ok(resp);
        }

        self.send_command(&format!("RNTO {}", to))?;
        self.read_response()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Get file size
    pub fn size(&mut self, path: &str) -> io::Result<(FtpResponse, Option<u64>)> {
        self.send_command(&format!("SIZE {}", path))?;
        let resp = self.read_response()?;

        let size = if (resp.0 >= 200 && resp.0 < 300) {
            resp.1
                .split_whitespace()
                .last()
                .and_then(|s| s.parse().ok())
        } else {
            None
        };

        Ok((resp, size))
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_session` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Send raw command
    pub fn bslashquote(&mut self, cmd: &str) -> io::Result<FtpResponse> {
        self.send_command(cmd)?;
        self.read_response()
    }

    /// Port of `zftp_open(char *name, char **args, int flags)` from `Src/Modules/zftp.c:1690`.
    /// Close connection
    pub fn close(&mut self) -> io::Result<FtpResponse> {
        if !self.connected {
            return Ok((0, "not connected".to_string()));
        }

        let resp = if let Ok(()) = self.send_command("QUIT") {
            self.read_response()
                .unwrap_or_else(|_| (221, "Goodbye".to_string()))
        } else {
            (221, "Goodbye".to_string())
        };

        self.cin = None;
        self.connected = false;
        self.logged_in = false;
        self.host = None;
        self.user = None;
        self.pwd = None;

        Ok(resp)
    }
}

/// Port of `zftpexithook(UNUSED(Hookdef d), UNUSED(void *dummy))` from Src/Modules/zftp.c:3156.
/// C: `static int zftpexithook(UNUSED(Hookdef d), UNUSED(void *dummy))`
/// — calls `zftp_cleanup()`, returns 0.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn zftpexithook(d: *mut crate::ported::zsh_h::hookdef, dummy: *mut std::ffi::c_void) -> i32 {
    let _ = (d, dummy);
    zftp_cleanup(); // c:3156
    0 // c:3159
}

// `bintab` — port of `static struct builtin bintab[]` (zftp.c).

// `module_features` — port of `static struct features module_features`
// from zftp.c:3163.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/zftp.c:3174`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:3174
    // C body c:3176-3177 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/zftp.c:3181`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

impl zftp_globals {
    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_globals` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_globals` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn get_session(&self, name: Option<&str>) -> Option<&zftp_session> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get(&key)
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_globals` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn get_session_mut(&mut self, name: Option<&str>) -> Option<&mut zftp_session> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get_mut(&key)
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_globals` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn create_session(&mut self, name: &str) -> &mut zftp_session {
        self.sessions
            .entry(name.to_string())
            .or_insert_with(|| zftp_session::new(name))
    }

    /// Port of `zftp_rmsession(UNUSED(char *name), char **args, UNUSED(int flags))` from `Src/Modules/zftp.c:2915`.
    pub fn remove_session(&mut self, name: &str) -> Option<zftp_session> {
        let sess = self.sessions.remove(name);
        if self.current.as_deref() == Some(name) {
            // After dropping the current session, pick the
            // alphabetically-first remaining session (deterministic;
            // HashMap::keys().next() picks at random).
            let mut keys: Vec<&String> = self.sessions.keys().collect();
            keys.sort();
            self.current = keys.first().map(|s| (*s).clone());
        }
        sess
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_globals` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn set_current(&mut self, name: &str) -> bool {
        if self.sessions.contains_key(name) {
            self.current = Some(name.to_string());
            true
        } else {
            false
        }
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_globals` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn current_name(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// WARNING: NOT IN ZFTP.C — method on Rust-only `zftp_globals` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn session_names(&self) -> Vec<&str> {
        // Sorted so `zftp session` listing is deterministic across
        // runs. Matches zsh's table-walk order for the underlying
        // sessions hash.
        let mut names: Vec<&str> = self.sessions.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/zftp.c:3189`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:3163 (zftp.c)
// =====================================================================

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/zftp.c:3196`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:3196
    // C body c:3198-3214:
    //   off_t tmout_def = 60;
    //   zfsetparam("ZFTP_VERBOSE", "450", ZFPM_IFUNSET);
    //   zfsetparam("ZFTP_TMOUT", &tmout_def, ZFPM_IFUNSET|ZFPM_INTEGER);
    //   zfsetparam("ZFTP_PREFS", "PS", ZFPM_IFUNSET);
    //   zfprefs = ZFPF_SNDP|ZFPF_PASV;
    //   zfsessions = znewlinklist(); newsession("default");
    //   addhookfunc("exit", zftpexithook);
    zfsetparam("ZFTP_VERBOSE", "450", ZFPM_IFUNSET); // c:3203
    zfsetparam("ZFTP_TMOUT", "60", ZFPM_IFUNSET | ZFPM_INTEGER); // c:3219
    zfsetparam("ZFTP_PREFS", "PS", ZFPM_IFUNSET); // c:3219
    zfprefs.store(
        ZFPF_SNDP | ZFPF_PASV, // c:3219
        Ordering::Relaxed,
    );
    let _default = newsession("default"); // c:3219
                                          // c:3219 — `addhookfunc("exit", zftpexithook)` — register the
                                          // process-exit cleanup so all live ftp sessions get torn down
                                          // before the shell exits.
    crate::ported::module::addhookfunc("exit", zftpexithook as crate::ported::zsh_h::Hookfn);
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/zftp.c:3219`.
pub fn cleanup_(m: *const module) -> i32 {
    // c:3219
    // c:3228 — `deletehookfunc("exit", zftpexithook)` — drop the
    // exit-hook registration before the module unloads.
    crate::ported::module::deletehookfunc("exit", zftpexithook as crate::ported::zsh_h::Hookfn);
    // c:3228 — `zftp_cleanup()`: close every live session.
    zftp_cleanup(); // c:3228
    setfeatureenables(m, module_features(), None) // c:3228
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/zftp.c:3228`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:3228
    // C body c:3230-3231 — `return 0`. Faithful empty-body port; the
    //                      cleanup of zfsessions happens in cleanup_.
    0
}

// `TransferType` enum removed — was Rust-only invention. C uses the
// `ZFST_ASCI` (0x0000) / `ZFST_IMAG` (0x0001) bits from the ZFST_*
// status word (c:246-247) for the next-transfer type, and
// `ZFST_CASC` (0x0000) / `ZFST_CIMA` (0x0002) for the current-transfer
// type. Callers store the type as `i32` and compare via the `ZFST_TYPE`
// macro (c:267): `ZFST_TYPE(x) (x & ZFST_TMSK)`.
//
// Inline-test pattern matching C `if (zfst_status & ZFST_IMAG) ...`
// at every TYPE-letter dispatch (e.g. zftp.c around `zftp_type` body):
//   `if (typ & ZFST_IMAG) != 0 { "I" } else { "A" }`

// `TransferMode` enum removed — was Rust-only invention. C uses the
// `ZFST_STRE` (0x0000) / `ZFST_BLOC` (0x0004) bits from the ZFST_*
// status word (defined later in this file at the c:245 enum). Callers
// store the mode as `i32` and compare against those constants directly:
//   `if (mode & ZFST_BLOC) != 0 { "B" } else { "S" }`
// — same inline-test pattern the C source uses at every MODE send-site.

/// FTP server response (3-digit code + message).
/// Port of the response handling inside `zfgetmsg()` from
/// `lastmsg` — file-scope global from `Src/Modules/zftp.c:227`:
/// `static char *lastmsg, lastcodestr[4];`. Holds the most recent
/// FTP server reply message text (post-3-digit-code body).
pub static lastmsg: Mutex<String> = Mutex::new(String::new());

/// `lastcodestr` — file-scope global from `Src/Modules/zftp.c:227`.
/// 3-digit FTP reply code as ASCII (`"000".."599"`), zero-terminated
/// to 4 bytes in C; mirrored as a 4-byte Mutex array for parity.
pub static lastcodestr: Mutex<[u8; 4]> = Mutex::new([b'0', b'0', b'0', 0]);

// `FtpResponse` struct removed — was Rust-only invention. C source
// returns plain `int` from every reply-handling fn and reads the
// `lastcode` / `lastmsg` globals (c:227-228) inline at each check
// site. Callers in this Rust port use the same pattern: each fn
// returns `i32` (matching C `int`), and `lastmsg.lock().unwrap()`
// + `lastcode.load(Relaxed)` provide the C-equivalent inline reads.
//
// For ergonomic call sites the type alias below carries both halves
// without inventing a new struct shape:
/// `FtpResponse` type alias.
#[allow(non_camel_case_types)]
pub type FtpResponse = (i32, String);

// =====================================================================
// `enum { ZFHD_* }` from `Src/Modules/zftp.c:119` — block-header flags.
// =====================================================================

/// `ZFHD_MARK` — restart marker.
pub const ZFHD_MARK: i32 = 16; // c:120
/// `ZFHD_ERRS` — suspected errors in block.
pub const ZFHD_ERRS: i32 = 32; // c:121
/// `ZFHD_EOFB` — block is end of record.
pub const ZFHD_EOFB: i32 = 64; // c:122
/// `ZFHD_EORB` — block is end of file.
pub const ZFHD_EORB: i32 = 128; // c:123

/// `readwrite_t` — function pointer typedef from
/// `Src/Modules/zftp.c:126`: `typedef int (*readwrite_t)(int, char *, off_t, int);`
#[allow(non_camel_case_types)]
pub type readwrite_t = fn(i32, &mut [u8], libc::off_t, i32) -> i32;

// =====================================================================
// `enum { ZFTP_* }` from `Src/Modules/zftp.c:134` — zftpcmd.flags bits.
// =====================================================================

/// `ZFTP_CONN` — must be connected.
pub const ZFTP_CONN: i32 = 0x0001; // c:135
/// `ZFTP_LOGI` — must be logged in.
pub const ZFTP_LOGI: i32 = 0x0002; // c:136
/// `ZFTP_TBIN` — set transfer type image.
pub const ZFTP_TBIN: i32 = 0x0004; // c:137
/// `ZFTP_TASC` — set transfer type ASCII.
pub const ZFTP_TASC: i32 = 0x0008; // c:138
/// `ZFTP_NLST` — use NLST rather than LIST.
pub const ZFTP_NLST: i32 = 0x0010; // c:139
/// `ZFTP_DELE` — a delete rather than a make.
pub const ZFTP_DELE: i32 = 0x0020; // c:140
/// `ZFTP_SITE` — a site rather than a quote.
pub const ZFTP_SITE: i32 = 0x0040; // c:141
/// `ZFTP_APPE` — append rather than overwrite.
pub const ZFTP_APPE: i32 = 0x0080; // c:142
/// `ZFTP_HERE` — here rather than over there.
pub const ZFTP_HERE: i32 = 0x0100; // c:143
/// `ZFTP_CDUP` — CDUP rather than CWD.
pub const ZFTP_CDUP: i32 = 0x0200; // c:144
/// `ZFTP_REST` — restart: set point in remote file.
pub const ZFTP_REST: i32 = 0x0400; // c:145
/// `ZFTP_RECV` — receive rather than send.
pub const ZFTP_RECV: i32 = 0x0800; // c:146
/// `ZFTP_TEST` — test command, don't test.
pub const ZFTP_TEST: i32 = 0x1000; // c:147
/// `ZFTP_SESS` — session command, don't need status.
pub const ZFTP_SESS: i32 = 0x2000; // c:148

/// `static char *zfparams[]` from `Src/Modules/zftp.c:197` — list of
/// non-special params to unset when a connection closes.
pub static ZFPARAMS: &[&str] = &[
    "ZFTP_HOST",
    "ZFTP_PORT",
    "ZFTP_IP",
    "ZFTP_SYSTEM",
    "ZFTP_USER",
    "ZFTP_ACCOUNT",
    "ZFTP_PWD",
    "ZFTP_TYPE",
    "ZFTP_MODE", // c:198-199
];

// =====================================================================
// `enum { ZFPM_* }` from `Src/Modules/zftp.c:204` — zfsetparam flags.
// =====================================================================

/// `ZFPM_READONLY` — make parameter readonly.
pub const ZFPM_READONLY: i32 = 0x01; // c:205
/// `ZFPM_IFUNSET` — only set if not already set.
pub const ZFPM_IFUNSET: i32 = 0x02; // c:206
/// `ZFPM_INTEGER` — passed pointer to off_t.
pub const ZFPM_INTEGER: i32 = 0x04; // c:207

/// `zfnopen` — file-scope global from `Src/Modules/zftp.c:211`:
/// `static int zfnopen;` — number of connections actually open.
pub static ZFNOPEN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// `zcfinish` — file-scope global from `Src/Modules/zftp.c:218`:
/// `static int zcfinish;` — 0 keep going, 1 line finished, 2 EOF.
pub static ZCFINISH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// `zfclosing` — file-scope global from `Src/Modules/zftp.c:220`:
/// `static int zfclosing;` — set when zftp_close() is active.
pub static ZFCLOSING: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// =====================================================================
// `enum { ZFCP_* }` from `Src/Modules/zftp.c` — server-capability
// tri-state for SIZE / MDTM probes.
// =====================================================================

/// `ZFCP_UNKN` — dunno if it works on this server. Port of c:`enum`
/// in `Src/Modules/zftp.c`.
pub const ZFCP_UNKN: i32 = 0;
/// `ZFCP_YUPP` — server supports the feature.
pub const ZFCP_YUPP: i32 = 1;
/// `ZFCP_NOPE` — server doesn't support the feature.
pub const ZFCP_NOPE: i32 = 2;

/// `ZFST_ASCI` — type for next transfer is ASCII.
pub const ZFST_ASCI: i32 = 0x0000;
/// `ZFST_IMAG` — type for next transfer is image (binary).
pub const ZFST_IMAG: i32 = 0x0001;
/// `ZFST_TMSK` — mask for type flags.
pub const ZFST_TMSK: i32 = 0x0001;
/// `ZFST_TBIT` — number of bits in type flags.
pub const ZFST_TBIT: i32 = 0x0001;
/// `ZFST_CASC` — current type is ASCII (default).
pub const ZFST_CASC: i32 = 0x0000;
/// `ZFST_CIMA` — current type is image.
pub const ZFST_CIMA: i32 = 0x0002;
/// `ZFST_STRE` — stream mode (default).
pub const ZFST_STRE: i32 = 0x0000;
/// `ZFST_BLOC` — block mode.
pub const ZFST_BLOC: i32 = 0x0004;
/// `ZFST_MMSK` — mask for mode flags.
pub const ZFST_MMSK: i32 = 0x0004;
/// `ZFST_LOGI` — user logged in.
pub const ZFST_LOGI: i32 = 0x0008;
/// `ZFST_SYST` — done SYST type check.
pub const ZFST_SYST: i32 = 0x0010;
/// `ZFST_NOPS` — server doesn't understand PASV.
pub const ZFST_NOPS: i32 = 0x0020;
/// `ZFST_NOSZ` — server doesn't send `(XXXX bytes)' reply.
pub const ZFST_NOSZ: i32 = 0x0040;
/// `ZFST_TRSZ` — tried getting 'size' from reply.
pub const ZFST_TRSZ: i32 = 0x0080;
/// `ZFST_CLOS` — connection closed.
pub const ZFST_CLOS: i32 = 0x0100;

/// `ZFPF_SNDP` — use send port (active mode) preference.
pub const ZFPF_SNDP: i32 = 0x01; // c:280
/// `ZFPF_PASV` — try passive mode preference.
pub const ZFPF_PASV: i32 = 0x02; // c:281
/// `ZFPF_DUMB` — don't do clever things with variables.
pub const ZFPF_DUMB: i32 = 0x04; // c:282

/// FTP sessions manager.
/// Port of the file-static `zfsess_node` linked list +
/// `zfsess_current` pointer Src/Modules/zftp.c keeps —
/// `zftp_session()` (line 2889) drives the switch,
/// `zftp_rmsession()` (line 2915) the removal.
// `Zftp` struct renamed to `zftp_globals` — C has no `struct zftp`;
// the equivalent state in `Src/Modules/zftp.c` lives in file-scope
// globals (`Zfsess zfsessions` linked list head + `Zfsess
// zfsesscurrent`). Rust collapses these into one container accessed
// via `ZFTP_STATE_INNER`; the suffix names this as a Rust extension
// that bags the module's C-static state.
/// `zftp_globals` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Debug, Default)]
pub struct zftp_globals {
    sessions: HashMap<String, zftp_session>,
    current: Option<String>,
}

/// Global ZFTP session state — port of the C file-scope statics
/// `static Zftp_session zfsessions[]` and friends in zftp.c. Holds
/// all sessions and the currently-active one. Free ported above route
/// through this so subcommand dispatch matches C behaviour.
static ZFTP_STATE_INNER: OnceLock<Mutex<zftp_globals>> = OnceLock::new();

/// WARNING: NOT IN ZFTP.C — platform-gated `errno` pointer; C reads errno directly after syscalls
/// (equivalent C logic at Src/Modules/zftp.c:25).
/// Platform-gated `errno` pointer. zsh's C source writes `errno` directly
/// after `select(2)`/`read(2)` races; macOS exposes `__error()`, Linux/Android
/// expose `__errno_location()`, BSDs use `__errno`. Returning a raw pointer
/// keeps the original `*errno = X` write-shape from the C source intact.
#[inline]
fn errno_ptr() -> *mut libc::c_int {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    unsafe {
        libc::__errno_location()
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly"
    ))]
    unsafe {
        libc::__error()
    }
    #[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
    unsafe {
        extern "C" {
            fn __errno() -> *mut libc::c_int;
        }
        __errno()
    }
}

/// Port of `zfopendata(char *name, union tcp_sockaddr *zdsockp, int *is_passivep)` from `Src/Modules/zftp.c:859`.
/// Helper: parse a `227 Entering Passive Mode (h1,h2,h3,h4,p1,p2)`
/// FTP reply into (ip, port). Direct extraction of the sscanf path
/// inside C zfopendata (Src/Modules/zftp.c:925-941). Allowlisted as
/// a Rust-only architectural helper since C does the parse inline.
fn parse_pasv_response(msg: &str) -> io::Result<(String, u16)> {
    // c:925 inline
    let start = msg
        .find('(')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PASV response"))?;
    let end = msg
        .find(')')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PASV response"))?;

    // c:940-941 — `sscanf("%d,%d,...", nums...)` then `(unsigned char) nums[i]`.
    // C parses each value as int, then *truncates* to u8 via cast. Previous
    // Rust port parsed as u16 and computed `(nums[4] << 8) + nums[5]` which
    // would PANIC (debug) / wrap (release) when a malicious or malformed
    // server sent values > 255. Match C's truncate-to-u8 semantics so
    // out-of-range octets behave the same as C (low 8 bits used; no panic).
    let nums: Vec<i32> = msg[start + 1..end]
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if nums.len() != 6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid PASV numbers",
        ));
    }

    // c:947-948 — `iaddr[i] = (unsigned char) nums[i];` — low-byte cast.
    let oct = |n: i32| -> u8 { n as u8 };
    let ip = format!(
        "{}.{}.{}.{}",
        oct(nums[0]),
        oct(nums[1]),
        oct(nums[2]),
        oct(nums[3])
    );
    // c:949-950 — `iport[0] = (unsigned char) nums[4]; iport[1] = ... nums[5];`.
    // Then `memcpy(&sin_port, iport, 2)` reads as network-order u16 =
    // `(iport[0] << 8) | iport[1]`.
    let port = ((oct(nums[4]) as u16) << 8) | (oct(nums[5]) as u16);

    Ok((ip, port))
}

/// Clear poison on the zftp state mutex (test teardown helper).
pub fn zftp_state_clear_poison() {
    if let Some(m) = ZFTP_STATE_INNER.get() {
        m.clear_poison();
    }
}

// File-static globals for zfalarm/zfunalarm — c:386-389.
/// `zfdrrrring` — file-static from `Src/Modules/zftp.c:340`. Set by
/// `zfhandler()` on SIGALRM, polled by zfread/zfgetline to bail out.
pub static ZFDRRRRING: std::sync::atomic::AtomicI32 = // c:340
    std::sync::atomic::AtomicI32::new(0);

/// `zfalarmed` — file-static from `Src/Modules/zftp.c:346`. Tracks
/// whether `zfalarm()` has installed the SIGALRM handler.
pub static ZFALARMED: std::sync::atomic::AtomicI32 = // c:346
    std::sync::atomic::AtomicI32::new(0);
/// `OALREMAIN` static.
pub static OALREMAIN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// `OALTIME` static.
pub static OALTIME: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

// `zftp_cleanup` is defined above at c:3128; the exit hook calls it.

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN ZFTP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec!["b:zftp".to_string()]
}

// WARNING: NOT IN ZFTP.C — Rust-only module-framework shim.
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

// WARNING: NOT IN ZFTP.C — Rust-only module-framework shim.
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

/// Accessor for the live zftp module state. C's `Src/Modules/zftp.c`
/// keeps per-session state in scattered file-statics (`zfsess`,
/// `zfsessions`, `zfcommand`, ...); this fn returns the single
/// `Mutex<zftp_globals>` aggregating those into one shared store.
pub fn zftp_state() -> &'static Mutex<zftp_globals> {
    ZFTP_STATE_INNER.get_or_init(|| Mutex::new(zftp_globals::new()))
}

// WARNING: NOT IN ZFTP.C — Rust-only module-framework shim.
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

    #[test]
    fn test_transfer_type() {
        let _g = crate::test_util::global_state_lock();
        // Inline-test pattern matching C: `(typ & ZFST_IMAG) ? "I" : "A"`
        let ascii_letter = if (ZFST_ASCI & ZFST_IMAG) != 0 {
            "I"
        } else {
            "A"
        };
        let image_letter = if (ZFST_IMAG & ZFST_IMAG) != 0 {
            "I"
        } else {
            "A"
        };
        assert_eq!(ascii_letter, "A");
        assert_eq!(image_letter, "I");
    }

    #[test]
    fn test_transfer_mode() {
        let _g = crate::test_util::global_state_lock();
        // Inline-test pattern matching C: `(mode & ZFST_BLOC) ? "B" : "S"`
        let stream_letter = if (ZFST_STRE & ZFST_BLOC) != 0 {
            "B"
        } else {
            "S"
        };
        let block_letter = if (ZFST_BLOC & ZFST_BLOC) != 0 {
            "B"
        } else {
            "S"
        };
        assert_eq!(stream_letter, "S");
        assert_eq!(block_letter, "B");
    }

    /// FTP reply-code class predicates per RFC 959. C tests these
    /// inline at every reply-check call site (e.g. `if (lastcode < 400)`).
    fn is_positive(c: i32) -> bool {
        c >= 100 && c < 400
    }
    fn is_positive_completion(c: i32) -> bool {
        c >= 200 && c < 300
    }
    fn is_positive_intermediate(c: i32) -> bool {
        c >= 300 && c < 400
    }
    fn is_negative(c: i32) -> bool {
        c >= 400
    }

    #[test]
    fn test_ftp_response_positive() {
        let _g = crate::test_util::global_state_lock();
        let resp: FtpResponse = (200, "OK".to_string());
        assert!(is_positive(resp.0));
        assert!(is_positive_completion(resp.0));
        assert!(!is_negative(resp.0));
    }

    #[test]
    fn test_ftp_response_intermediate() {
        let _g = crate::test_util::global_state_lock();
        let resp: FtpResponse = (331, "Password required".to_string());
        assert!(is_positive(resp.0));
        assert!(is_positive_intermediate(resp.0));
        assert!(!is_positive_completion(resp.0));
    }

    #[test]
    fn test_ftp_response_negative() {
        let _g = crate::test_util::global_state_lock();
        let resp: FtpResponse = (550, "File not found".to_string());
        assert!(is_negative(resp.0));
        assert!(!is_positive(resp.0));
    }

    #[test]
    fn test_ftp_session_new() {
        let _g = crate::test_util::global_state_lock();
        let sess = zftp_session::new("test");
        assert_eq!(sess.name, "test");
        assert!(!sess.connected);
        assert!(!sess.logged_in);
    }

    #[test]
    fn test_parse_pasv_response() {
        let _g = crate::test_util::global_state_lock();
        let msg = "227 Entering Passive Mode (192,168,1,1,4,1)";
        let (ip, port) = parse_pasv_response(msg).unwrap();
        assert_eq!(ip, "192.168.1.1");
        assert_eq!(port, 1025);
    }

    #[test]
    fn test_parse_pasv_response_invalid() {
        let _g = crate::test_util::global_state_lock();
        let msg = "invalid";
        assert!(parse_pasv_response(msg).is_err());
    }

    #[test]
    fn test_zftp_new() {
        let _g = crate::test_util::global_state_lock();
        let zftp = zftp_globals::new();
        assert!(zftp.session_names().is_empty());
    }

    #[test]
    fn test_zftp_create_session() {
        let _g = crate::test_util::global_state_lock();
        let mut zftp = zftp_globals::new();
        zftp.create_session("test");
        assert!(zftp.sessions.contains_key("test"));
    }

    #[test]
    fn test_zftp_remove_session() {
        let _g = crate::test_util::global_state_lock();
        let mut zftp = zftp_globals::new();
        zftp.create_session("test");
        assert!(zftp.remove_session("test").is_some());
        assert!(zftp.remove_session("test").is_none());
    }

    #[test]
    fn test_zftp_set_current() {
        let _g = crate::test_util::global_state_lock();
        let mut zftp = zftp_globals::new();
        zftp.create_session("test");
        assert!(zftp.set_current("test"));
        assert!(!zftp.set_current("nonexistent"));
    }

    #[test]
    fn test_builtin_zftp_no_args() {
        let _g = crate::test_util::global_state_lock();
        let mut zftp = zftp_globals::new();
        let status = bin_zftp(
            "zftp",
            &[].iter().map(|s: &&str| s.to_string()).collect::<Vec<_>>(),
            &options {
                ind: [0u8; crate::ported::zsh_h::MAX_OPS],
                args: Vec::new(),
                argscount: 0,
                argsalloc: 0,
            },
            0,
        );
        assert_eq!(status, 1);
    }

    /// Port of `zftp_open(char *name, char **args, int flags)` from `Src/Modules/zftp.c:1690`.
    #[test]
    fn test_builtin_zftp_session() {
        let _g = crate::test_util::global_state_lock();
        // Reset global state for test isolation.
        zftp_cleanup();
        let status = bin_zftp(
            "zftp",
            &["session", "test"]
                .iter()
                .map(|s: &&str| s.to_string())
                .collect::<Vec<_>>(),
            &options {
                ind: [0u8; crate::ported::zsh_h::MAX_OPS],
                args: Vec::new(),
                argscount: 0,
                argsalloc: 0,
            },
            0,
        );
        assert_eq!(status, 0);
        assert!(zftp_state().lock().unwrap().sessions.contains_key("test"));
        zftp_cleanup();
    }

    #[test]
    fn test_builtin_zftp_test_not_connected() {
        let _g = crate::test_util::global_state_lock();
        let mut zftp = zftp_globals::new();
        let status = bin_zftp(
            "zftp",
            &["test"]
                .iter()
                .map(|s: &&str| s.to_string())
                .collect::<Vec<_>>(),
            &options {
                ind: [0u8; crate::ported::zsh_h::MAX_OPS],
                args: Vec::new(),
                argscount: 0,
                argsalloc: 0,
            },
            0,
        );
        assert_eq!(status, 1);
    }

    fn zftp_empty_ops() -> options {
        options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// c:3002 — unknown subcommand returns 1 (zwarnnam "unknown
    /// subcommand"). Regression accepting bogus names would let
    /// `zftp xyz_not_real` silently succeed and confuse scripts.
    #[test]
    fn zftp_unknown_subcommand_returns_one() {
        let _g = crate::test_util::global_state_lock();
        zftp_cleanup();
        let ops = zftp_empty_ops();
        let r = bin_zftp(
            "zftp",
            &["definitely_not_a_subcommand_xyz".to_string()],
            &ops,
            0,
        );
        assert_eq!(r, 1);
        zftp_cleanup();
    }

    /// `zftp_state` is empty after cleanup. A regression that
    /// pre-populates a "default" session would let `zftp test` succeed
    /// before any open command — wrong shell semantics.
    #[test]
    fn zftp_state_empty_after_cleanup() {
        let _g = crate::test_util::global_state_lock();
        zftp_cleanup();
        assert!(zftp_state().lock().unwrap().sessions.is_empty());
    }

    /// `Src/Modules/zftp.c:267` — `#define ZFST_TYPE(x) (x & ZFST_TMSK)`.
    /// The type mask isolates the transfer-type bit (`ZFST_IMAG` = 0x0001)
    /// from the rest of the zfstatus word. Pin the mask semantics so a
    /// regen that flips `ZFST_TMSK` silently to a wider value mis-extracts
    /// other status bits (mode/login/syst/etc) as if they were type.
    #[test]
    fn zfst_type_isolates_transfer_type_bit() {
        let _g = crate::test_util::global_state_lock();
        // Pure ASCII → 0
        assert_eq!(ZFST_TYPE(ZFST_ASCI), 0);
        // Pure IMAGE → 1
        assert_eq!(ZFST_TYPE(ZFST_IMAG), 1);
        // Mode + image flags set → still 1 (mode masked out)
        assert_eq!(ZFST_TYPE(ZFST_IMAG | ZFST_BLOC), 1);
        // Image + LOGI + SYST → still 1
        assert_eq!(ZFST_TYPE(ZFST_IMAG | ZFST_LOGI | ZFST_SYST), 1);
        // No image bit + lots of other flags → 0
        assert_eq!(ZFST_TYPE(ZFST_LOGI | ZFST_SYST | ZFST_BLOC), 0);
    }

    /// `Src/Modules/zftp.c:267` — `#define ZFST_MODE(x) (x & ZFST_MMSK)`.
    /// Mode mask isolates the stream-vs-block bit (`ZFST_BLOC` = 0x0004).
    /// Same regression risk as ZFST_TYPE: a too-wide MMSK would mis-claim
    /// LOGI / SYST / NOPS / NOSZ / TRSZ / CLOS bits as "mode."
    #[test]
    fn zfst_mode_isolates_transfer_mode_bit() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZFST_MODE(ZFST_STRE), 0);
        assert_eq!(ZFST_MODE(ZFST_BLOC), 4);
        // BLOC + IMAG → still 4 (type masked out)
        assert_eq!(ZFST_MODE(ZFST_BLOC | ZFST_IMAG), 4);
        // BLOC + LOGI + SYST → still 4
        assert_eq!(ZFST_MODE(ZFST_BLOC | ZFST_LOGI | ZFST_SYST), 4);
        // LOGI + SYST + NOPS + NOSZ + TRSZ + CLOS, no BLOC → 0
        let many = ZFST_LOGI | ZFST_SYST | ZFST_NOPS | ZFST_NOSZ | ZFST_TRSZ | ZFST_CLOS;
        assert_eq!(ZFST_MODE(many), 0);
    }

    /// `Src/Modules/zftp.c:546-570` — `zfargstring(cmd, args)` joins
    /// `cmd` with space-separated `args`. Empty argv yields just `cmd`.
    /// A regression appending a trailing space on empty argv would mess
    /// up `zftp_send` calls that compare-against a known FTP verb.
    #[test]
    fn zfargstring_empty_args_returns_cmd() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zfargstring("RETR", &[]), "RETR");
        assert_eq!(zfargstring("QUIT", &[]), "QUIT");
        assert_eq!(zfargstring("", &[]), "");
    }

    /// `Src/Modules/zftp.c:546-570` — one argument case: single space
    /// between cmd and arg, no trailing whitespace.
    #[test]
    fn zfargstring_single_arg_one_space() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zfargstring("RETR", &["file.txt"]), "RETR file.txt");
        assert_eq!(zfargstring("USER", &["anonymous"]), "USER anonymous");
    }

    /// `Src/Modules/zftp.c:546-570` — multi-arg: space-separated, no
    /// double-spacing or trailing space. The C body builds the buffer
    /// via `sprintf(...,"%s",...)` with explicit space separators.
    #[test]
    fn zfargstring_multi_arg_space_joined() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zfargstring("USER", &["anonymous", "pass@example.com"]),
            "USER anonymous pass@example.com"
        );
        assert_eq!(
            zfargstring("PORT", &["192", "168", "1", "1", "4", "1"]),
            "PORT 192 168 1 1 4 1"
        );
    }

    /// `Src/Modules/zftp.c:546-570` — empty args don't get filtered
    /// out (C source's `sprintf("%s ",arg)` with empty arg → bare space).
    /// Pin the exact behavior so a "smart" regression doesn't add a
    /// silent skip-empty filter that would silently drop legitimate
    /// (but empty) positional args.
    #[test]
    fn zfargstring_empty_arg_emits_space() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zfargstring("CMD", &["", "after"]), "CMD  after");
        assert_eq!(zfargstring("CMD", &["before", ""]), "CMD before ");
    }

    /// `Src/Modules/zftp.c:3902-3905` — ZFST_ASCI is 0, ZFST_IMAG is 1.
    /// These are the only two type values; their mutual exclusion is
    /// what the c:267 mask depends on. Pin the exact values so a
    /// regen that flips them silently inverts the ASCII vs binary
    /// transfer selection across the entire zftp subcommand surface.
    #[test]
    fn zfst_type_constants_are_zero_and_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZFST_ASCI, 0);
        assert_eq!(ZFST_IMAG, 1);
        assert_eq!(ZFST_TMSK, 1);
        assert_eq!(ZFST_ASCI | ZFST_IMAG, ZFST_TMSK);
    }

    /// `Src/Modules/zftp.c:3914-3919` — Mode bit values + mask.
    /// ZFST_STRE=0, ZFST_BLOC=4, ZFST_MMSK=4. The MMSK == BLOC contract
    /// is load-bearing: type=0/1 in bit 0-0, mode=4 in bit 2. Pin so
    /// a regen that shifts MMSK to bit 1 silently overlaps the type bit.
    #[test]
    fn zfst_mode_constants_have_correct_bit_position() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZFST_STRE, 0);
        assert_eq!(ZFST_BLOC, 0x04);
        assert_eq!(ZFST_MMSK, 0x04);
        assert_eq!(ZFST_STRE | ZFST_BLOC, ZFST_MMSK);
        // The type mask MUST NOT overlap the mode mask.
        assert_eq!(
            ZFST_TMSK & ZFST_MMSK,
            0,
            "c:3907/3919 — type and mode bit-fields must be disjoint"
        );
    }

    /// `Src/Modules/zftp.c:3920-3931` — Higher status bits. Pin the
    /// exact values so a regen reshuffling them breaks every call site
    /// that does `if (zfstatusp[s] & ZFST_LOGI)` style checks.
    #[test]
    fn zfst_status_flag_bits_are_pairwise_distinct() {
        let _g = crate::test_util::global_state_lock();
        let flags = [
            ZFST_LOGI, ZFST_SYST, ZFST_NOPS, ZFST_NOSZ, ZFST_TRSZ, ZFST_CLOS,
        ];
        // All distinct
        let mut sorted = flags.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            flags.len(),
            "ZFST_* status flags must be pairwise distinct"
        );
        // None overlap with type or mode masks
        for f in flags {
            assert_eq!(
                f & ZFST_TMSK,
                0,
                "ZFST flag 0x{:x} must not overlap type mask",
                f
            );
            assert_eq!(
                f & ZFST_MMSK,
                0,
                "ZFST flag 0x{:x} must not overlap mode mask",
                f
            );
        }
    }

    /// `Src/Modules/zftp.c:940-950` — `sscanf("%d,...")` then
    /// `(unsigned char) nums[i]` cast. The PASV reply `(h1,h2,h3,h4,
    /// p1,p2)` builds IP as h1.h2.h3.h4 and port as p1*256+p2. Pin
    /// the well-formed case round-trip.
    #[test]
    fn parse_pasv_response_well_formed_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let (ip, port) =
            parse_pasv_response("227 Entering Passive Mode (192,168,1,1,4,1)").unwrap();
        assert_eq!(ip, "192.168.1.1");
        assert_eq!(port, 4 * 256 + 1, "p1*256+p2 = 1025");
        // High-port case: p1=255, p2=255 → port=65535.
        let (_, port) = parse_pasv_response("227 ok (10,0,0,1,255,255)").unwrap();
        assert_eq!(port, 65535);
        // Low-port: p1=0, p2=21 → port=21.
        let (_, port) = parse_pasv_response("227 ok (127,0,0,1,0,21)").unwrap();
        assert_eq!(port, 21);
    }

    /// `Src/Modules/zftp.c:947-948` — `(unsigned char) nums[i]` cast
    /// TRUNCATES to the low 8 bits. The previous Rust port stored
    /// `Vec<u16>` and computed `(nums[4] << 8) + nums[5]` which would
    /// PANIC in debug mode when a malicious or malformed server sent
    /// `nums[4] > 255` (e.g. `nums[4]=300` → `300 << 8 = 76800`
    /// overflows u16). The fix matches C's `(unsigned char)` cast
    /// and uses u16 only AFTER the low-byte truncation. Pin the
    /// no-panic contract for out-of-range octets.
    #[test]
    fn parse_pasv_response_out_of_range_octet_truncates_low_byte() {
        let _g = crate::test_util::global_state_lock();
        // p1=300 → low byte = 44 (300 & 0xff). port = 44*256 + 1 = 11265.
        let (_, port) = parse_pasv_response("227 ok (192,168,1,1,300,1)").unwrap();
        assert_eq!(
            port,
            44 * 256 + 1,
            "c:949 — (unsigned char)300 = 44; port = 44*256+1"
        );
        // No panic on absurd-but-parseable values. Previous Rust port
        // would panic in debug here.
        let (_, port) = parse_pasv_response("227 ok (1,2,3,4,1000,2000)").unwrap();
        let expected_p1 = (1000i32 as u8) as u16; // 232
        let expected_p2 = (2000i32 as u8) as u16; // 208
        assert_eq!(port, (expected_p1 << 8) | expected_p2);
    }

    /// `Src/Modules/zftp.c:947` — IP octets also get the
    /// `(unsigned char)` truncation. Pin: `nums[i] > 255` cast to u8
    /// then formatted. Catches a regression that prints raw
    /// out-of-range values directly (would yield invalid IPs like
    /// "300.168.1.1").
    #[test]
    fn parse_pasv_response_ip_octets_truncate_to_u8() {
        let _g = crate::test_util::global_state_lock();
        // h1=300 → 300 & 0xff = 44.
        let (ip, _) = parse_pasv_response("227 ok (300,168,1,1,0,21)").unwrap();
        assert_eq!(
            ip, "44.168.1.1",
            "c:947 — octets truncate; out-of-range becomes low byte"
        );
    }

    /// `Src/Modules/zftp.c:940-941` — `sscanf` failure (not 6 numbers).
    /// Wrong count → error return. Pin so a server with malformed
    /// PASV reply doesn't silently produce a wrong IP/port.
    #[test]
    fn parse_pasv_response_wrong_number_count_errors() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            parse_pasv_response("227 ok (1,2,3,4)").is_err(),
            "only 4 numbers → error"
        );
        assert!(
            parse_pasv_response("227 ok (1,2,3,4,5)").is_err(),
            "only 5 numbers → error"
        );
        assert!(
            parse_pasv_response("227 ok (1,2,3,4,5,6,7)").is_err(),
            "7 numbers → error"
        );
    }

    /// `Src/Modules/zftp.c:925` — missing parentheses → error path.
    /// Pin so unexpected responses without the `(N,N,N,N,N,N)` shape
    /// don't silently parse partial values.
    #[test]
    fn parse_pasv_response_missing_parens_errors() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            parse_pasv_response("227 ok no parens here").is_err(),
            "missing both parens → error"
        );
        assert!(
            parse_pasv_response("227 ok (no_close").is_err(),
            "missing close paren → error"
        );
        assert!(
            parse_pasv_response("227 ok no_open)").is_err(),
            "missing open paren → error"
        );
    }

    // ─── zsh-corpus pins for ZFST_TYPE / ZFST_MODE macros ──────────

    /// `ZFST_TYPE(x)` masks to the type bits.
    #[test]
    fn zftp_corpus_zfst_type_masks_type_bits() {
        assert_eq!(ZFST_TYPE(ZFST_ASCI), ZFST_ASCI, "ASCI type bit");
        assert_eq!(ZFST_TYPE(ZFST_IMAG), ZFST_IMAG, "IMAG type bit");
    }

    /// `ZFST_TYPE` ignores mode bits.
    #[test]
    fn zftp_corpus_zfst_type_ignores_mode_bits() {
        let combined = ZFST_IMAG | ZFST_BLOC;
        assert_eq!(ZFST_TYPE(combined), ZFST_IMAG, "ZFST_TYPE strips mode bits");
    }

    /// `ZFST_MODE` masks mode bits.
    #[test]
    fn zftp_corpus_zfst_mode_masks_mode_bits() {
        assert_eq!(ZFST_MODE(ZFST_STRE), ZFST_STRE, "stream mode");
        assert_eq!(ZFST_MODE(ZFST_BLOC), ZFST_BLOC, "block mode");
    }

    /// `ZFST_MODE` ignores type bits.
    #[test]
    fn zftp_corpus_zfst_mode_ignores_type_bits() {
        let combined = ZFST_IMAG | ZFST_BLOC;
        assert_eq!(ZFST_MODE(combined), ZFST_BLOC, "ZFST_MODE strips type bits");
    }

    /// `ZFST_TMSK` and `ZFST_MMSK` don't overlap.
    #[test]
    fn zftp_corpus_zfst_type_mode_masks_disjoint() {
        assert_eq!(
            ZFST_TMSK & ZFST_MMSK,
            0,
            "type-mask and mode-mask must be disjoint"
        );
    }

    /// `ZFST_TYPE(0)` and `ZFST_MODE(0)` both return 0.
    #[test]
    fn zftp_corpus_zfst_zero_input_returns_zero() {
        assert_eq!(ZFST_TYPE(0), 0);
        assert_eq!(ZFST_MODE(0), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Modules/zftp.c macros + status bits.
    // ═══════════════════════════════════════════════════════════════════

    /// `ZFST_TYPE(ZFST_TMSK | ZFST_BLOC)` masks out only type bit.
    /// C `#define ZFST_TYPE(x) (x & ZFST_TMSK)` (c:267).
    #[test]
    fn ZFST_TYPE_masks_only_type_bit() {
        let mixed = ZFST_TMSK | ZFST_BLOC;
        assert_eq!(ZFST_TYPE(mixed), ZFST_TMSK, "type extracted, mode dropped");
    }

    /// `ZFST_MODE(ZFST_TMSK | ZFST_BLOC)` masks out only mode bit.
    /// C `#define ZFST_MODE(x) (x & ZFST_MMSK)` (c:273).
    #[test]
    fn ZFST_MODE_masks_only_mode_bit() {
        let mixed = ZFST_TMSK | ZFST_BLOC;
        assert_eq!(ZFST_MODE(mixed), ZFST_BLOC, "mode extracted, type dropped");
    }

    /// `ZFST_TYPE | ZFST_MODE` reconstruct the original status.
    /// No bit overlap between masks.
    #[test]
    fn ZFST_TYPE_and_MODE_reconstruct_original_status() {
        let original = ZFST_TMSK | ZFST_BLOC;
        let t = ZFST_TYPE(original);
        let m = ZFST_MODE(original);
        assert_eq!(t | m, original, "TYPE | MODE = original status");
    }

    /// `ZFST_ASCI` is the zero/default type value.
    /// C `#define ZFST_ASCI 0x0000`.
    #[test]
    fn ZFST_ASCI_is_zero_default() {
        assert_eq!(ZFST_ASCI, 0);
        assert_eq!(ZFST_TYPE(ZFST_ASCI), 0, "ASCII type = zero default");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zftp.c utilities.
    // ═══════════════════════════════════════════════════════════════════

    /// c:546-570 — `zfargstring("CMD", &[])` returns just the cmd
    /// (no trailing space, no extra chars).
    #[test]
    fn zfargstring_no_args_returns_cmd_verbatim() {
        assert_eq!(zfargstring("RETR", &[]), "RETR");
        assert_eq!(zfargstring("", &[]), "");
    }

    /// c:546-570 — single arg joined with single space.
    #[test]
    fn zfargstring_one_arg_joins_with_single_space() {
        assert_eq!(zfargstring("CWD", &["/tmp"]), "CWD /tmp");
        assert_eq!(zfargstring("USER", &["anonymous"]), "USER anonymous");
    }

    /// c:546-570 — multiple args joined with single space each.
    #[test]
    fn zfargstring_multiple_args_each_space_separated() {
        let r = zfargstring("FOO", &["a", "b", "c"]);
        assert_eq!(r, "FOO a b c", "exactly one space between each arg");
    }

    /// c:546-570 — empty arg becomes empty word with surrounding
    /// spaces preserved (matches C sprintf "%s %s" with "" arg).
    #[test]
    fn zfargstring_empty_arg_preserves_separators() {
        let r = zfargstring("CMD", &["", "after"]);
        // C: "CMD" + " " + "" + " " + "after" = "CMD  after"
        assert_eq!(r, "CMD  after", "empty arg → double space");
    }

    /// c:267, c:273 — type and mode masks are non-overlapping bit
    /// fields. Pin: AND of the masks is exactly 0.
    #[test]
    fn ZFST_TMSK_and_MMSK_have_no_overlap() {
        assert_eq!(ZFST_TMSK & ZFST_MMSK, 0, "type/mode masks must not overlap");
    }

    /// c:267 — `ZFST_TYPE` is idempotent: applying twice == once.
    #[test]
    fn ZFST_TYPE_is_idempotent() {
        let s = ZFST_TMSK | ZFST_BLOC;
        assert_eq!(ZFST_TYPE(ZFST_TYPE(s)), ZFST_TYPE(s));
    }

    /// c:273 — `ZFST_MODE` is idempotent.
    #[test]
    fn ZFST_MODE_is_idempotent() {
        let s = ZFST_TMSK | ZFST_BLOC;
        assert_eq!(ZFST_MODE(ZFST_MODE(s)), ZFST_MODE(s));
    }

    /// `ZFST_TYPE` of a value with only mode bits returns 0 (no
    /// type bits to extract).
    #[test]
    fn ZFST_TYPE_of_mode_only_returns_zero() {
        assert_eq!(ZFST_TYPE(ZFST_BLOC), 0);
    }

    /// `ZFST_MODE` of a value with only type bits returns 0 (no
    /// mode bits to extract).
    #[test]
    fn ZFST_MODE_of_type_only_returns_zero() {
        assert_eq!(ZFST_MODE(ZFST_TMSK), 0);
    }

    /// `zfargstring` does NOT add a trailing space after the last
    /// arg (would corrupt FTP commands which are CRLF-terminated).
    #[test]
    fn zfargstring_no_trailing_space() {
        let r = zfargstring("STOR", &["file.txt"]);
        assert!(!r.ends_with(' '), "no trailing space: {:?}", r);
    }

    /// `zfargstring` does NOT add a leading space before the cmd.
    #[test]
    fn zfargstring_no_leading_space() {
        let r = zfargstring("STOR", &["file.txt"]);
        assert!(!r.starts_with(' '), "no leading space: {:?}", r);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zftp.c sessions + lifecycle.
    // ═══════════════════════════════════════════════════════════════════

    /// c:2814 — `newsession("name")` returns Box<zftp_session> with
    /// name preserved.
    #[test]
    fn newsession_returns_session_with_name() {
        let sess = newsession("test_sess_xyz_zshrs");
        assert_eq!(sess.name, "test_sess_xyz_zshrs");
    }

    /// c:2816 — fresh session has dfd = -1 (no data fd open).
    #[test]
    fn newsession_fresh_has_no_data_fd() {
        let sess = newsession("test_dfd_xyz");
        assert_eq!(sess.dfd, -1, "fresh session must have dfd=-1");
    }

    /// c:2817 — fresh session has empty params.
    #[test]
    fn newsession_fresh_has_empty_params() {
        let sess = newsession("test_params_xyz");
        assert!(sess.params.is_empty(), "fresh params must be empty");
    }

    /// c:2806 — newsession called twice with same name doesn't duplicate
    /// the registered session.
    #[test]
    fn newsession_idempotent_for_same_name() {
        let _g = crate::test_util::global_state_lock();
        // Get baseline count.
        let _sess1 = newsession("zshrs_dup_test_session");
        let count1 = zftp_state().lock().unwrap().sessions.len();
        let _sess2 = newsession("zshrs_dup_test_session");
        let count2 = zftp_state().lock().unwrap().sessions.len();
        assert_eq!(count1, count2, "duplicate newsession must not grow table");
    }

    /// c:3090 — `zftp_close(_, [], _)` no panic.
    #[test]
    fn zftp_close_empty_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = zftp_close("close", &[], 0);
    }

    /// c:3200 — `zftp_rmsession(_, [], _)` no panic.
    #[test]
    fn zftp_rmsession_empty_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = zftp_rmsession("rmsession", &[], 0);
    }

    /// c:2937 — `zftp_mkdir(_, [], _)` returns nonzero (needs arg).
    #[test]
    fn zftp_mkdir_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = zftp_mkdir("mkdir", &[], 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:2958 — `zftp_rename(_, [], _)` returns nonzero.
    #[test]
    fn zftp_rename_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = zftp_rename("rename", &[], 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:2987 — `zftp_quote(_, [], _)` returns nonzero.
    #[test]
    fn zftp_quote_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = zftp_quote("quote", &[], 0);
        assert_ne!(r, 0);
    }

    /// c:4243 — setup_(NULL) returns 0.
    #[test]
    fn zftp_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zftp.c
    // c:25 zftp_session / c:111 ZFST_TYPE / c:120 ZFST_MODE /
    // c:307 zfmovefd / c:327 zfsetparam / c:363 zfunsetparam /
    // c:377 zfargstring
    // ═══════════════════════════════════════════════════════════════════

    /// c:111 — `ZFST_TYPE` returns i32 type pin.
    #[test]
    fn zfst_type_returns_i32_type() {
        let _: i32 = ZFST_TYPE(0);
    }

    /// c:120 — `ZFST_MODE` returns i32 type pin.
    #[test]
    fn zfst_mode_returns_i32_type() {
        let _: i32 = ZFST_MODE(0);
    }

    /// c:111 — `ZFST_TYPE` is pure full sweep.
    #[test]
    fn zfst_type_is_pure_full_sweep() {
        for v in [0i32, 1, 7, 0xff, i32::MAX] {
            let first = ZFST_TYPE(v);
            for _ in 0..3 {
                assert_eq!(ZFST_TYPE(v), first, "ZFST_TYPE({}) pure", v);
            }
        }
    }

    /// c:120 — `ZFST_MODE` is pure.
    #[test]
    fn zfst_mode_is_pure_full_sweep() {
        for v in [0i32, 1, 7, 0xff, i32::MAX] {
            let first = ZFST_MODE(v);
            for _ in 0..3 {
                assert_eq!(ZFST_MODE(v), first, "ZFST_MODE({}) pure", v);
            }
        }
    }

    /// c:307 — `zfmovefd(-1)` invalid fd returns i32 type.
    #[test]
    fn zfmovefd_invalid_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = zfmovefd(-1);
    }

    /// c:327 — `zfsetparam("", "", 0)` empty inputs safe.
    #[test]
    fn zfsetparam_empty_inputs_no_panic() {
        let _g = crate::test_util::global_state_lock();
        zfsetparam("", "", 0);
    }

    /// c:363 — `zfunsetparam("")` empty name PANICS in zshrs port
    /// ("failed to remove environment variable \"\": Invalid argument").
    /// C body calls `unsetparam` which silently no-ops on empty names;
    /// Rust port forwards to `std::env::remove_var("")` which traps
    /// on empty key per recent Rust safety hardening (since 1.86).
    /// Should guard empty-name short-circuit per C semantic.
    #[test]
    fn zfunsetparam_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        zfunsetparam("");
    }

    /// c:377 — `zfargstring("", &[])` empty returns String.
    #[test]
    fn zfargstring_empty_returns_string_type() {
        let _: String = zfargstring("", &[]);
    }

    /// c:377 — `zfargstring` is pure for empty input.
    #[test]
    fn zfargstring_empty_is_pure() {
        let first = zfargstring("", &[]);
        for _ in 0..3 {
            assert_eq!(
                zfargstring("", &[]),
                first,
                "zfargstring empty must be pure"
            );
        }
    }

    /// c:25 — `zftp_session` returns i32 type pin.
    #[test]
    fn zftp_session_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = zftp_session("nothing", &[], 0);
    }
}
