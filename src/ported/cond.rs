//! Conditional expression evaluation — port of Src/cond.c.
//!
//! Evaluates `[[ … ]]` (zsh extended test) and `[`/`test` (POSIX)
//! conditional expressions.
//!
//! ## Port status
//!
//! C-faithful: `evalcond` (c:70) walks pre-compiled wordcode (`Estate state`,
//! opcodes via `WC_COND_TYPE`, operand strings via `ecgetstr`) and is called
//! from `execcond` (exec.rs) for `[[ ]]` programs and from `bin_test`
//! (builtin.rs) for `test` / `[` programs built by `parse_cond`. The per-test
//! helpers `doaccess` (c:438), `getstat` (c:452), `dostat` (c:474), `dolstat`
//! (c:488), `optison` (c:502), `cond_str` (c:525), `cond_val` (c:539),
//! `cond_match` (c:552), `tracemodcond` (c:563) are direct ports with C-named
//! signatures.

use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::Ordering;

use crate::glob::matchpat;
use crate::ported::exec::quote_tokenized_output;
use crate::ported::lex::untokenize;
use crate::ported::math::{matheval, mathevali};
use crate::ported::module::{ensurefeature, getconddef, MODULESTAB};
use crate::ported::options::{optlookup, optlookupc};
use crate::ported::params::{issetvar, setaparam};
use crate::ported::parse::{ecgetarr, ecgetstr, ecrawstr};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::string::dupstring;
use crate::ported::glob::{checkglobqual, zglob};
use crate::ported::linklist::hlinklist2array;
use crate::ported::subst::{prefork, singsub};
use crate::ported::utils::{
    has_token, privasserted, quotedzputs, sepjoin, unmeta, zerr, zerrnam, zstrtol, zwarn, zwarnnam,
};
use crate::ported::zsh_h::{
    conddef, estate, isset, mnumber, unset, CASEGLOB, COND_AND, COND_EF, COND_EQ, COND_GE, COND_GT,
    COND_LE, COND_LT, COND_MOD, COND_MODI, COND_NE, COND_NOT, COND_NT, COND_OR, COND_OT,
    COND_REGEX, COND_STRDEQ, COND_STREQ, COND_STRGTR, COND_STRLT, COND_STRNEQ, EC_DUP, EC_DUPTOK,
    EC_NODUP, EXTENDEDGLOB, IS_DASH, MN_FLOAT, MN_INTEGER, PAT_STATIC, POSIXBUILTINS, REMATCHPCRE,
    WC_COND_SKIP, WC_COND_TYPE,
};
use std::io::Write;
use std::os::unix::io::FromRawFd;
// C-style i32 return codes from `evalcond` (mirroring cond.c:70):
//   0 — condition true
//   1 — condition false
//   2 — syntax error
//   3 — option tested with -o does not exist
//
// `evalcond`'s integer return values are documented in the C source
// at cond.c:62-66; we use bare i32 throughout (no enum wrapper).

/// Port of `int tracingcond` from `Src/cond.c:33` — "updated by
/// execcond() in exec.c": non-zero while `set -x` is tracing a `[[ ]]`.
#[allow(non_upper_case_globals)]
pub static tracingcond: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:33

/// Port of `static char *condstr[COND_MOD]` from `Src/cond.c:35-38`.
#[allow(non_upper_case_globals)]
const condstr: [&str; 18] = [
    "!", "&&", "||", "=", "==", "!=", "<", ">", "-nt", "-ot", "-ef", "-eq", "-ne", "-lt", "-gt",
    "-le", "-ge", "=~",
]; // c:35

/// Port of `cond_subst(char **strp, int glob_ok)` from `Src/cond.c:41`.
/// Substitute (and, when `glob_ok` and the word ends in a glob
/// qualifier, glob) one `[[ ]]` operand in place.
pub fn cond_subst(strp: &mut String, glob_ok: i32) {
    // c:41
    let chars: Vec<char> = strp.chars().collect();
    let mut sp: Option<usize> = None;
    if glob_ok != 0 && checkglobqual(&chars, chars.len() as i32, 1, &mut sp) != 0 {
        // c:43-44
        let mut args = crate::ported::subst::LinkList::default();
        args.push_back(strp.clone());
        let mut ret_flags = 0i32;
        prefork(&mut args, 0, &mut ret_flags);
        let mut v: Vec<String> = hlinklist2array(&args);
        while crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) == 0
            && !v.is_empty()
            && has_token(&v[0])
        {
            // c:48-50
            zglob(&mut v, 0, 0);
        }
        *strp = sepjoin(&v, None);
    } else {
        *strp = singsub(strp);
    }
}

/// Port of `evalcond(Estate state, char *fromtest)` from
/// `Src/cond.c:70`. Walks the `WC_COND` wordcode tree at `state.pc`.
/// `fromtest` is `Some(name)` when called from `test` / `[`.
///
/// Return status is the final shell status, i.e. 0 for true, 1 for
/// false, 2 for syntax error, 3 for "option in tested in -o does not
/// exist".
pub fn evalcond(state: &mut estate, fromtest: Option<&str>) -> i32 {
    // c:70
    // zwarnnam(fromtest, ...) with fromtest == NULL prints no command name.
    let warnnam = |msg: &str| match fromtest {
        Some(n) => zwarnnam(n, msg),
        None => zwarn(msg),
    };
    // rec: (c:77)
    loop {
        let mut overridename: Option<String> = None;
        let pcode = state.pc;
        state.pc += 1;
        let code = state.prog.prog[pcode];
        let mut ctype = WC_COND_TYPE(code) as i32;
        let mut htok: i32 = 0;

        match ctype {
            COND_NOT => {
                // c:86
                if tracingcond.load(Ordering::Relaxed) != 0 {
                    eprint!(" {}", condstr[ctype as usize]);
                }
                let ret = evalcond(state, fromtest);
                if ret == 0 || ret == 1 {
                    return (ret == 0) as i32;
                }
                return ret;
            }
            COND_AND => {
                // c:94
                let ret = evalcond(state, fromtest);
                if ret == 0 {
                    if tracingcond.load(Ordering::Relaxed) != 0 {
                        eprint!(" {}", condstr[ctype as usize]);
                    }
                    continue;
                }
                state.pc = pcode + (WC_COND_SKIP(code) as usize + 1);
                return ret;
            }
            COND_OR => {
                // c:103
                let ret = evalcond(state, fromtest);
                if ret == 1 || ret == 3 {
                    if tracingcond.load(Ordering::Relaxed) != 0 {
                        eprint!(" {}", condstr[ctype as usize]);
                    }
                    continue;
                }
                state.pc = pcode + (WC_COND_SKIP(code) as usize + 1);
                return ret;
            }
            _ => {}
        }

        if ctype == COND_REGEX {
            // c:113
            let modname = if isset(REMATCHPCRE) {
                "zsh/pcre"
            } else {
                "zsh/regex"
            };
            let on = format!("-{}-match", &modname[4..]);
            if let Ok(mut tab) = MODULESTAB.try_lock() {
                let _ = ensurefeature(&mut tab, modname, "C:", Some(&on[1..]));
            }
            overridename = Some(on);
            ctype = COND_MODI;
        }
        if ctype == COND_MOD || ctype == COND_MODI {
            // c:121-194
            let mut l = WC_COND_SKIP(code) as usize;
            let mut name: String = match overridename.clone() {
                Some(n) => n,
                None => ecgetstr(state, EC_NODUP, None),
            };
            let mut strs: Vec<String>;
            if ctype == COND_MOD {
                strs = ecgetarr(state, l, EC_DUP, None);
            } else {
                let s0 = ecgetstr(state, EC_NODUP, None);
                let s1 = ecgetstr(state, EC_NODUP, None);
                strs = vec![s0, s1];
                l = 2;
            }
            let errname: String = if name.chars().next().map_or(false, IS_DASH) {
                untokenize(&name)
            } else if strs
                .first()
                .and_then(|s| s.chars().next())
                .map_or(false, IS_DASH)
            {
                untokenize(&strs[0])
            } else {
                "<null>".to_string()
            };
            // getconddef() may load a module, which re-enters MODULESTAB;
            // the lock is released before any handler runs.
            let lookup = |inf: i32, nm: &str| -> Option<conddef> {
                match MODULESTAB.try_lock() {
                    Ok(mut tab) => getconddef(inf, nm, 1, &mut tab),
                    Err(_) => None,
                }
            };
            if name.chars().next().map_or(false, IS_DASH) {
                let rest: String = name.chars().skip(1).collect(); // name + 1
                if let Some(cd) = lookup((ctype == COND_MODI) as i32, &rest) {
                                            if ctype == COND_MOD
                        && ((l as i64) < cd.min as i64
                            || (cd.max >= 0 && (l as i64) > cd.max as i64))
                    {
                        warnnam(&format!("unknown condition: {}", name));
                        return 2;
                    }
                    if tracingcond.load(Ordering::Relaxed) != 0 {
                        tracemodcond(&name, &strs, ctype == COND_MODI);
                    }
                    let r = cd.handler.map_or(0, |h| h(&strs, cd.condid));
                    return (r == 0) as i32;
                }
            }

            let s: Option<String> = strs.first().cloned();
            if let Some(ov) = overridename {
                // standard regex function not available: hard error.
                let msg = format!("{} not available for regex", ov);
                match fromtest {
                    Some(n) => zerrnam(n, &msg),
                    None => zerr(&msg),
                }
                return 2;
            }
            if !strs.is_empty() {
                strs[0] = dupstring(&name);
            }
            let first_dash = s.as_deref().and_then(|x| x.chars().next());
            if let Some(c0) = first_dash {
                if IS_DASH(c0) {

                    name = s.clone().unwrap_or_default();
                    let rest: String = name.chars().skip(1).collect();
                    if let Some(cd) = lookup(0, &rest) {
                        if (l as i64) < cd.min as i64
                            || (cd.max >= 0 && (l as i64) > cd.max as i64)
                        {
                            warnnam(&format!("unknown condition: {}", errname));
                            return 2;
                        }
                        if tracingcond.load(Ordering::Relaxed) != 0 {
                            tracemodcond(&name, &strs, ctype == COND_MODI);
                        }
                        let r = cd.handler.map_or(0, |h| h(&strs, cd.condid));
                        return (r == 0) as i32;
                    }
                }
            }
            warnnam(&format!("unknown condition: {}", errname));
            return 2;
        }

        // c:196
        let mut left = ecgetstr(state, EC_DUPTOK, Some(&mut htok));
        if htok != 0 {
            cond_subst(&mut left, fromtest.is_none() as i32);
            left = untokenize(&left);
        }
        let mut right = String::new();
        if ctype <= COND_GE && ctype != COND_STREQ && ctype != COND_STRDEQ && ctype != COND_STRNEQ
        {

            right = ecgetstr(state, EC_DUPTOK, Some(&mut htok));
            if htok != 0 {
                cond_subst(&mut right, fromtest.is_none() as i32);
                right = untokenize(&right);
            }
        }
        if tracingcond.load(Ordering::Relaxed) != 0 {
            // c:209
            if ctype < COND_MOD {
                eprint!(" {} {} ", quotedzputs(&left), condstr[ctype as usize]);
                if ctype == COND_STREQ || ctype == COND_STRDEQ || ctype == COND_STRNEQ {
                    let mut rt = ecrawstr(&state.prog, state.pc, None);
                    cond_subst(&mut rt, fromtest.is_none() as i32);
                    let _ = quote_tokenized_output(&rt, &mut std::io::stderr());
                } else {
                    eprint!("{}", quotedzputs(&right));
                }
            } else {
                eprint!(" -{} {}", ctype as u8 as char, quotedzputs(&left));
            }
        }

        if ctype >= COND_EQ && ctype <= COND_GE {
            // c:228
            let mut mn1: mnumber;
            let mut mn2: mnumber;
            if fromtest.is_some() {

                let (l1, eptr) = zstrtol(&left, 10);
                let mut err: &str = &left;
                let mut l2: i64 = 0;
                let mut bad = !eptr.is_empty();
                if !bad {
                    let (v2, e2) = zstrtol(&right, 10);
                    l2 = v2;
                    err = &right;
                    bad = !e2.is_empty();
                }
                if bad {

                    warnnam(&format!("integer expression expected: {}", err));
                    return 2;
                }
                mn1 = mnumber { l: l1, d: 0.0, type_: MN_INTEGER };
                mn2 = mnumber { l: l2, d: 0.0, type_: MN_INTEGER };
            } else {
                let zero = mnumber { l: 0, d: 0.0, type_: MN_INTEGER };
                mn1 = matheval(&left).unwrap_or(zero);
                mn2 = matheval(&right).unwrap_or(zero);
            }
            if ((mn1.type_ | mn2.type_) & (MN_INTEGER | MN_FLOAT)) == (MN_INTEGER | MN_FLOAT) {

                if mn1.type_ & MN_INTEGER != 0 {
                    mn1.type_ = MN_FLOAT;
                    mn1.d = mn1.l as f64;
                }
                if mn2.type_ & MN_INTEGER != 0 {
                    mn2.type_ = MN_FLOAT;
                    mn2.d = mn2.l as f64;
                }
            }
            let fl = mn1.type_ & MN_FLOAT != 0;
            let t = match ctype {
                COND_EQ => {
                    if fl { mn1.d == mn2.d } else { mn1.l == mn2.l }
                }
                COND_NE => {
                    if fl { mn1.d != mn2.d } else { mn1.l != mn2.l }
                }
                COND_LT => {
                    if fl { mn1.d < mn2.d } else { mn1.l < mn2.l }
                }
                COND_GT => {
                    if fl { mn1.d > mn2.d } else { mn1.l > mn2.l }
                }
                COND_LE => {
                    if fl { mn1.d <= mn2.d } else { mn1.l <= mn2.l }
                }
                _ => {
                    if fl { mn1.d >= mn2.d } else { mn1.l >= mn2.l } // COND_GE
                }
            };
            return (!t) as i32;
        }

        // `!x` over a C truth value: 0 when true, 1 when false.
        let nz = |t: bool| -> i32 { (!t) as i32 };
        let fmt = libc::S_IFMT as u32;
        return match ctype {
            COND_STREQ | COND_STRDEQ | COND_STRNEQ => {
                // c:293-327
                queue_signals();
                // c:302-318 — every pattern slot is a dummy_patprog here (the Rust
                // `pats` slots hold no compiled pattern), so take the dummy-pattern
                // compile path: substitute the raw pattern, then compile it.
                let opat = ecrawstr(&state.prog, state.pc, Some(&mut htok));
                right = dupstring(&opat);
                right = singsub(&right);
                let pprog = patcompile(&right, PAT_STATIC, None);
                let ret = match pprog {
                    None => {
                        warnnam(&format!("bad pattern: {}", right));
                        unqueue_signals();
                        return 2;
                    }
                    Some(p) => {
                        state.pc += 2;
                        let test = pattry(&p, &left);
                        let test = if ctype == COND_STRNEQ { !test } else { test };
                        nz(test)
                    }
                };
                unqueue_signals();
                ret
            }
            // c:328
            COND_STRLT => nz(left.as_bytes() < right.as_bytes()),
            // c:330
            COND_STRGTR => nz(left.as_bytes() > right.as_bytes()),
            // c:332-428 — the single-letter and file-comparison cases
            _ => match ctype as u8 as char {
                'e' | 'a' => nz(doaccess(&left, libc::F_OK) != 0),
                'b' => nz((dostat(&left) & fmt) == libc::S_IFBLK as u32),
                'c' => nz((dostat(&left) & fmt) == libc::S_IFCHR as u32),
                'd' => nz((dostat(&left) & fmt) == libc::S_IFDIR as u32),
                'f' => nz((dostat(&left) & fmt) == libc::S_IFREG as u32),
                'g' => nz((dostat(&left) & (libc::S_ISGID as u32)) != 0),
                'k' => nz((dostat(&left) & (libc::S_ISVTX as u32)) != 0),
                'n' => nz(!left.is_empty()),
                'o' => optison(fromtest, &left),
                'p' => nz((dostat(&left) & fmt) == libc::S_IFIFO as u32),
                'r' => nz(doaccess(&left, libc::R_OK) != 0),
                's' => nz(getstat(&left).map_or(false, |m| m.size() != 0)),
                'S' => nz((dostat(&left) & fmt) == libc::S_IFSOCK as u32),
                'u' => nz((dostat(&left) & (libc::S_ISUID as u32)) != 0),
                'v' => nz(issetvar(&left) != 0),
                'w' => nz(doaccess(&left, libc::W_OK) != 0),
                'x' => {
                    // c:365
                    if privasserted() {
                        let mode = dostat(&left);
                        nz(((mode & 0o111) != 0) || ((mode & fmt) == libc::S_IFDIR as u32))
                    } else {
                        nz(doaccess(&left, libc::X_OK) != 0)
                    }
                }
                'z' => (!left.is_empty()) as i32,
                'h' | 'L' => nz((dolstat(&left) & fmt) == libc::S_IFLNK as u32),
                'O' => nz(getstat(&left).map_or(false, |m| m.uid() == unsafe { libc::geteuid() })),
                'G' => nz(getstat(&left).map_or(false, |m| m.gid() == unsafe { libc::getegid() })),
                'N' => match getstat(&left) {
                    // c:380
                    None => 1,
                    Some(m) => {
                        if m.atime() == m.mtime() {
                            (m.atime_nsec() > m.mtime_nsec()) as i32
                        } else {
                            (m.atime() > m.mtime()) as i32
                        }
                    }
                },
                't' => nz(unsafe { libc::isatty(mathevali(&left).unwrap_or(0) as i32) } != 0),
                _ if ctype == COND_NT || ctype == COND_OT => {
                    // c:400
                    let (a, nsecs) = match getstat(&left) {
                        None => return 1,
                        Some(m) => (m.mtime(), m.mtime_nsec()),
                    };
                    let m2 = match getstat(&right) {
                        None => return 1,
                        Some(m) => m,
                    };
                    if a == m2.mtime() {
                        // c:419
                        nz(if ctype == COND_NT {
                            nsecs > m2.mtime_nsec()
                        } else {
                            nsecs < m2.mtime_nsec()
                        })
                    } else {
                        nz(if ctype == COND_NT { a > m2.mtime() } else { a < m2.mtime() })
                    }
                }
                _ if ctype == COND_EF => {
                    // c:426
                    let (d, i) = match getstat(&left) {
                        None => return 1,
                        Some(m) => (m.dev(), m.ino()),
                    };
                    match getstat(&right) {
                        None => 1,
                        Some(m) => nz(d == m.dev() && i == m.ino()),
                    }
                }
                _ => {                        warnnam("bad cond code");
                    2
                }
            },
        };
    }
}

// ===========================================================
// Direct-port helpers used internally by evalcond. These mirror
// the C helpers in cond.c that wrap stat()/access()/option lookup
// and the cond_str/cond_val/cond_match argument-coercion trio.
// ===========================================================

/// Port of `doaccess(char *s, int c)` from Src/cond.c:438 — `[[ -r/-w/-x ]]` test.
/// Returns true (non-zero) when `access(2)` reports the file is
/// reachable for the requested mode.
///
/// C body (c:438-446):
///     #ifdef HAVE_FACCESSX
///         if (!strncmp(s, "/dev/fd/", 8))
///             return !faccessx(atoi(s + 8), c, ACC_SELF);
///     #endif
///     return !access(unmeta(s), c);
///
/// The HAVE_FACCESSX branch is Solaris-only (not available on
/// Linux/macOS via libc-rs). On Linux/macOS C falls through to
/// `access(unmeta(s), c)` which uses the kernel-provided
/// `/dev/fd/N` symlink resolution. Rust port mirrors this with
/// `libc::access(unmeta(s), c)` directly — the kernel handles
/// `/dev/fd/N` transparently.
pub fn doaccess(s: &str, c: i32) -> i32 {
    // c:438
    let cs = match std::ffi::CString::new(unmeta(s)) {
        // c:445 unmeta(s)
        Ok(v) => v,
        Err(_) => return 0,
    };
    (unsafe { libc::access(cs.as_ptr(), c) } == 0) as i32 // c:445 !access(...)
}

/// Port of `getstat(char *s)` from Src/cond.c:452 — `stat(2)` wrapper that
/// special-cases `/dev/fd/N` with `fstat()`. Returns the metadata or
/// `None` on error. Replaces the C global `static struct stat st`
/// with a returned `Metadata` value (Rust avoids globals here).
///
/// **C-faithful semantics**:
///   1. `/dev/fd/N` → `fstat(N, &st)` per c:458-461. C does NOT dup
///      the fd; the previous Rust port dup'd it unnecessarily, which
///      could fail at the open-fd limit AND created an owned File
///      that would close the duplicate when dropped (harmless on
///      success path but wasteful syscall).
///   2. Regular path → `stat(unmeta(s), &st)` per c:464-467. The
///      previous Rust port used `fs::metadata(s)` directly which
///      doesn't run `unmeta` — paths containing Meta-encoded bytes
///      would fail to resolve.
pub fn getstat(s: &str) -> Option<Metadata> {
    // c:452
    if let Some(rest) = s.strip_prefix("/dev/fd/") {
        // c:458
        if let Ok(fd) = rest.parse::<i32>() {
            // c:459 atoi(s+8)
            // c:459 — `fstat(fd, &st)`. Pre-check via fstat to verify
            // the fd is valid BEFORE dup'ing (avoid wasting an fd slot
            // on a bad fd). The dup is a Rust adaptation: `Metadata`
            // requires an owned `File`, but `File::from_raw_fd` would
            // close the user's fd on drop — so we dup to give the
            // File its own owned copy and the user keeps their fd.
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(fd, &mut st) } != 0 {
                return None;
            }
            let dup_fd = unsafe { libc::dup(fd) };
            if dup_fd < 0 {
                return None;
            }
            let f = unsafe { std::fs::File::from_raw_fd(dup_fd) };
            return f.metadata().ok();
        }
    }
    // c:464 — `if (!(us = unmeta(s))) return NULL;`
    let us = unmeta(s);
    fs::metadata(&us).ok() // c:466
}

/// Port of `dostat(char *s)` from Src/cond.c:474 — returns the file's
/// `st_mode` or 0 on error. Used by `[[ -b/-c/-d/-f/-g/-h/-k/-p
/// /-S/-u/-w/-x ]]` to inspect mode bits.
pub fn dostat(s: &str) -> u32 {
    // c:474
    getstat(s).map(|m| m.mode()).unwrap_or(0)
}

/// Port of `dolstat(char *s)` from Src/cond.c:488 — like `dostat()` but
/// uses `lstat(2)` so symlinks are *not* followed. Underpins
/// `[[ -h ]]` / `[[ -L ]]`.
///
/// C body (c:489): `if (lstat(unmeta(s), &st) < 0) return 0;`.
/// The previous Rust port passed `s` directly to `fs::symlink_metadata`,
/// missing the `unmeta(s)` step — paths containing Meta-encoded bytes
/// would fail to resolve. Same divergence as the now-fixed `getstat`.
pub fn dolstat(s: &str) -> u32 {
    // c:488
    let us = unmeta(s); // c:489 unmeta(s)
    fs::symlink_metadata(&us).map(|m| m.mode()).unwrap_or(0)
}

/// Port of `optison(char *name, char *s)` from Src/cond.c:502 — `[[ -o NAME ]]` shell-
/// option test. Returns 0 (true) when the option is set, 1 (false)
/// when unset, 3 (error) when the name is unrecognised. Routes
/// through the canonical option table via `optlookup` /
/// `optlookupc` (Src/options.c:684 / :721).
pub fn optison(name: Option<&str>, s: &str) -> i32 {
    // c:502
    let i: i32 = if s.len() == 1 {
        // c:502
        optlookupc(s.as_bytes()[0] as char) // c:507
    } else {
        optlookup(s) // c:509
    };
    if i == 0 {
        // c:510
        if isset(POSIXBUILTINS) {
            // c:511
            return 1; // c:512
        } else {
            // c:514 `zwarnnam(name, "no such option: %s", s)` — `name` is
            // C's `fromtest`: NULL for `[[ -o X ]]` (so the diagnostic has
            // NO command-name prefix — `zsh:1: no such option: …`) and
            // "test"/"[" only from the test/[ builtin. zshrs hardcoded
            // "test", so `[[ -o bad ]]` wrongly printed `…:test:1:…`.
            let msg = format!("no such option: {}", s);
            match name {
                Some(n) => zwarnnam(n, &msg),
                None => zwarn(&msg),
            }
            return 3; // c:515
        }
    } else if i < 0 {
        // c:517
        if unset(-i) {
            0
        } else {
            1
        } // c:518 !unset(-i)
    } else if isset(i) {
        0
    } else {
        1
    } // c:520 !isset(i)
}

// `isset` / `unset` macros from `Src/options.h:62-63` — `(opts[X])`
// / `(!opts[X])`. Re-exported from the canonical port in zsh_h.rs
// which reads the live opt_state, NOT a fresh `ShellOptions::new()`
// (the latter returns defaults and would be wrong).

/// Port of `cond_str(char **args, int num, int raw)` from `Src/cond.c:525-535`.
///
/// C body (c:527-534):
/// ```c
/// char *s = args[num];
/// if (has_token(s)) {
///     singsub(&s);
///     if (!raw)
///         untokenize(s);
/// }
/// return s;
/// ```
///
/// The previous Rust port stubbed this to a plain indexed read,
/// claiming "stores already-expanded argument strings" — but the
/// in-tree evalcond walker at cond.rs:62 passes raw `&str` slices
/// from the argv; no upstream expansion happens. Any cond op that
/// calls cond_str (e.g. module-defined ops like `Src/Modules/files.c`'s
/// `[[ -X file ]]`) would see un-singsub'd argument strings.
///
/// Port the full C body: if the arg contains tokens, route through
/// `singsub` (for $var / $(cmd) / arithmetic expansion), then
/// `untokenize` unless raw mode was requested.
pub fn cond_str(args: &[String], num: usize, raw: bool) -> String {
    // c:525
    let s = match args.get(num) {
        // c:527
        Some(v) => v.clone(),
        None => return String::new(),
    };
    if has_token(&s) {
        // c:529
        let expanded = singsub(&s); // c:530
        if !raw {
            untokenize(&expanded) // c:532
        } else {
            expanded
        }
    } else {
        s // c:534
    }
}

/// Port of `cond_val(char **args, int num)` from Src/cond.c:539 — `[[ N -eq M ]]`
/// integer-comparison side. Returns the integer value of the
/// numth argument, **routing through `mathevali`** per c:548. This
/// is how `[[ 1+2 -eq 3 ]]` evaluates the LHS string `1+2` as the
/// arithmetic expression yielding `3` rather than failing to parse
/// `"1+2"` as a base-10 integer.
///
/// Previously the Rust port called `s.trim().parse::<i64>()` which
/// silently returned 0 for any non-trivial arithmetic on either
/// side of a `-eq` / `-ne` / `-lt` / `-gt` / `-le` / `-ge` test, a
/// divergence that breaks `[[ $((LINENO)) -eq 1+0 ]]`-style asserts
/// in user scripts.
pub fn cond_val(args: &[String], num: usize) -> i64 {
    // c:539
    let raw = match args.get(num) {
        Some(v) => v.clone(),
        None => return 0,
    };
    // c:543-547 — `if (has_token(s)) { singsub(&s); untokenize(s); }`.
    // The previous Rust port claimed "args are pre-expanded" and
    // jumped straight to mathevali. The evalcond walker passes raw
    // slices; module-defined ops calling cond_val would see un-
    // singsub'd tokens (Inpar/Outpar/Dnull/etc) reach mathevali and
    // fail to parse \`$((x))\`-containing operands.
    let s = if has_token(&raw) {
        // c:543
        let expanded = singsub(&raw); // c:544
        untokenize(&expanded) // c:545
    } else {
        raw
    };
    // c:548 — `mathevali(s)`.
    mathevali(&s).unwrap_or(0) // c:548
}

/// Port of `cond_match(char **args, int num, char *str)` from Src/cond.c:552 — `[[ str = pat ]]`
/// pattern test. Runs `singsub()` on the pattern, then defers to
/// `matchpat()` (Src/glob.c).
///
/// C's `matchpat` reads `EXTENDED_GLOB` and case sensitivity from
/// global option state. The Rust `matchpat` extends the signature to
/// take these as explicit args (a structural Rust adaptation), so we
/// read the live option state here and pass it through.
pub fn cond_match(args: &[String], num: usize, str: &str) -> bool {
    // c:552
    // c:556 — `char *s = args[num]; singsub(&s); return matchpat(str, s);`
    // `singsub(&s)` performs parameter / arithmetic / command
    // substitution on the pattern BEFORE matching so `[[ $x = $pat ]]`
    // matches the value of $pat, not the literal "$pat".
    let p_raw = match args.get(num) {
        Some(v) => v,
        None => return false,
    };
    let p = singsub(p_raw); // c:556
                            // c:2519 (glob.c) — `if (isset(EXTENDED_GLOB)) ...` controls #/~ syntax.
    let extended = isset(EXTENDEDGLOB);
    // c:2519 — case sensitivity reads `isset(CASEGLOB)` (with the
    // canonical-name spelling, NOT a "no_case_glob" variant).
    let case_sensitive = isset(CASEGLOB);
    // C: `matchpat(str, s)` where `str` is the text being matched
    // and `s` is the pattern. Rust matchpat's signature is REVERSED:
    // `matchpat(pattern, text, ...)`. The previous Rust port called
    // `matchpat(str, p, ...)` which passed text as pattern AND
    // pattern as text — silently mis-routing every `[[ a = pat ]]`
    // glob test against the wrong side. Pass in Rust order.
    // (#m) / (#b) publish $MATCH, $MBEGIN, $MEND, $match, $mbegin and
    // $mend inside pattryrefs (c:Src/pattern.c:2526-2621), which matchpat
    // reaches through pattry.
    matchpat(&p, str, extended, case_sensitive) // c:557
}

/// Port of `tracemodcond(char *name, char **args, int inf)` from Src/cond.c:563 — `xtrace`-mode
/// pretty-printer for module-defined cond operators. Emits the
/// op + args to stderr in the same shape the C source uses (infix
/// for binary, prefix for unary). Used only when the `XTRACE`
/// option is enabled and a third-party module supplies a cond.
pub fn tracemodcond(name: &str, args: &[String], inf: bool) {
    // c:563
    // c:566-570 — `args = arrdup(args); for (aptr = args; *aptr; aptr++) untokenize(*aptr);`
    let args: Vec<String> = args.iter().map(|a| untokenize(a)).collect();
    let stderr = std::io::stderr();
    let mut out = stderr.lock();
    if inf {
        let _ = write!(
            out,
            " {} {} {}",
            args.first().map(|s| s.as_str()).unwrap_or(""),
            name,
            args.get(1).map(|s| s.as_str()).unwrap_or("")
        );
    } else {
        let _ = write!(out, " {}", name);
        for a in args {
            let _ = write!(out, " {}", a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{getaparam, getsparam};
    use std::fs::File;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;
    use std::collections::HashMap;

    /// Test shim with the pre-port argv signature: drives the same pipeline
    /// `bin_test` uses (test-mode `parse_cond` over the argv, then the wordcode
    /// `evalcond`). `from_test` is C`s `fromtest`; the option and variable maps
    /// are ignored because `evalcond` reads shell state directly.
    fn evalcond(
        args: &[&str],
        _options: &HashMap<String, bool>,
        _variables: &HashMap<String, String>,
        _posix: bool,
        from_test: Option<&str>,
    ) -> i32 {
        use crate::ported::builtin::{testlex, CURTESTARG, TESTARGS, TESTARGS_IDX};
        use crate::ported::parse::{parse_cond, CONDLEX_TESTLEX};
        use crate::ported::zsh_h::{eprog, ERRFLAG_ERROR, NULLTOK};
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        crate::ported::context::zcontext_save();
        TESTARGS.with_borrow_mut(|a| *a = argv);
        TESTARGS_IDX.set(0);
        CURTESTARG.set(0);
        crate::ported::lex::set_tok(NULLTOK);
        CONDLEX_TESTLEX.set(true);
        testlex();
        let prog: Option<eprog> = parse_cond();
        CONDLEX_TESTLEX.set(false);
        let failed = crate::ported::utils::errflag.load(Ordering::Relaxed) != 0
            || prog.is_none()
            || crate::ported::lex::tok() == crate::ported::zsh_h::LEXERR;
        crate::ported::utils::errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::context::zcontext_restore();
        let leftover = TESTARGS.with_borrow(|a| CURTESTARG.get() < a.len());
        if failed || leftover {
            return 2;
        }
        let p = prog.unwrap();
        let strs = p.strs.clone();
        let mut st = estate { prog: Box::new(p), pc: 0, strs, strs_offset: 0 };
        super::evalcond(&mut st, from_test)
    }

    fn empty_maps() -> (HashMap<String, bool>, HashMap<String, String>) {
        (HashMap::new(), HashMap::new())
    }

    // ═══════════════════════════════════════════════════════════════════
    // `=~` capture-group wire-up — Src/cond.c:113-119 dispatches to
    // zsh/regex module which sets $MATCH / $match[N] / $mbegin[N] /
    // $mend[N]. Rust port uses the `regex` crate directly and writes
    // the same arrays. Tests pin the canonical zsh observable shape.
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_string_empty() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["-z", ""], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["-z", "hello"], &opts, &vars, true, None), 1);
        assert_eq!(evalcond(&["-n", "hello"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["-n", ""], &opts, &vars, true, None), 1);
    }

    #[test]
    fn test_string_compare() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["hello", "=", "hello"], &opts, &vars, true, None),
            0
        );
        assert_eq!(
            evalcond(&["hello", "!=", "world"], &opts, &vars, true, None),
            0
        );
        // parse.c:2668-2673: in `test`/`[` both `<` and `>` compile to COND_STRGTR
        // (the reference build evaluates `a < b` as `a > b`).
        assert_eq!(evalcond(&["abc", "<", "def"], &opts, &vars, true, None), 1);
        assert_eq!(evalcond(&["def", "<", "abc"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["xyz", ">", "abc"], &opts, &vars, true, None), 0);
    }

    #[test]
    fn test_numeric_compare() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["5", "-eq", "5"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["5", "-ne", "3"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["3", "-lt", "5"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["5", "-gt", "3"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["5", "-le", "5"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["5", "-ge", "5"], &opts, &vars, true, None), 0);
    }

    #[test]
    fn test_file_exists() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("testfile");
        File::create(&file_path).unwrap();
        let (opts, vars) = empty_maps();
        let path_str = file_path.to_str().unwrap();
        assert_eq!(evalcond(&["-e", path_str], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["-f", path_str], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["-d", path_str], &opts, &vars, true, None), 1);
    }

    #[test]
    fn test_directory() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();
        let path_str = dir.path().to_str().unwrap();
        assert_eq!(evalcond(&["-d", path_str], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["-f", path_str], &opts, &vars, true, None), 1);
    }

    #[test]
    fn test_logical_not() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["!", "-z", "hello"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["!", "-n", ""], &opts, &vars, true, None), 0);
    }

    #[test]
    fn test_logical_and() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["-n", "a", "-a", "-n", "b"], &opts, &vars, true, None),
            0
        );
        assert_eq!(
            evalcond(&["-n", "a", "-a", "-z", "b"], &opts, &vars, true, None),
            1
        );
    }

    #[test]
    fn test_logical_or() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["-z", "a", "-o", "-n", "b"], &opts, &vars, true, None),
            0
        );
        assert_eq!(
            evalcond(&["-z", "a", "-o", "-z", "b"], &opts, &vars, true, None),
            1
        );
    }

    #[test]
    fn test_variable_exists() {
        let _g = crate::test_util::global_state_lock();
        let opts = HashMap::new();
        let vars = HashMap::new();
        crate::ported::params::setsparam("MYVAR", "value");
        assert_eq!(evalcond(&["-v", "MYVAR"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["-v", "NOTEXIST"], &opts, &vars, true, None), 1);
    }

    /// `Src/cond.c:179-180` — `[[ -s file ]]` is true iff stat succeeds
    /// AND `st_size > 0`. Empty file → false; non-empty → true; missing → false.
    #[test]
    fn test_minus_s_size_gt_zero() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let empty = dir.path().join("empty");
        File::create(&empty).unwrap();
        assert_eq!(
            evalcond(&["-s", empty.to_str().unwrap()], &opts, &vars, true, None),
            1,
            "c:179 — `-s` must be false for 0-byte file"
        );

        let nonempty = dir.path().join("nonempty");
        let mut f = File::create(&nonempty).unwrap();
        f.write_all(b"data").unwrap();
        assert_eq!(
            evalcond(
                &["-s", nonempty.to_str().unwrap()],
                &opts,
                &vars,
                true,
                None
            ),
            0,
            "c:179 — `-s` must be true for non-empty file"
        );

        let missing = dir.path().join("not_there");
        assert_eq!(
            evalcond(&["-s", missing.to_str().unwrap()], &opts, &vars, true, None),
            1,
            "c:179 — `-s` must be false when stat fails (missing file)"
        );
    }

    /// `Src/cond.c:488` — `dolstat` uses `lstat(2)` so `-h` / `-L`
    /// returns true for the LINK itself, even when the link target
    /// doesn't exist. `-f` / `-d` against the same link returns false
    /// (since they follow the link via `stat(2)` and find nothing).
    #[cfg(unix)]
    #[test]
    fn test_minus_h_minus_L_detect_symlink_via_lstat() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let target = dir.path().join("nonexistent_target");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let link_s = link.to_str().unwrap();
        assert_eq!(
            evalcond(&["-h", link_s], &opts, &vars, true, None),
            0,
            "c:488 — `-h` uses lstat; detects symlink even with missing target"
        );
        assert_eq!(
            evalcond(&["-L", link_s], &opts, &vars, true, None),
            0,
            "c:488 — `-L` is same as `-h`"
        );
        // -f / -d follow the link → false because target doesn't exist.
        assert_eq!(evalcond(&["-f", link_s], &opts, &vars, true, None), 1);
        assert_eq!(evalcond(&["-d", link_s], &opts, &vars, true, None), 1);
    }

    /// `Src/cond.c:179-180` — `[[ -ef ]]` returns true iff two paths
    /// resolve to the same inode (`st_dev` AND `st_ino` match). A
    /// hardlink to the same file passes; an unrelated file fails.
    #[cfg(unix)]
    #[test]
    fn test_dash_ef_same_inode() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let a = dir.path().join("a");
        let b = dir.path().join("b");
        let c = dir.path().join("c");
        File::create(&a).unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        File::create(&c).unwrap();

        let as_ = a.to_str().unwrap();
        let bs_ = b.to_str().unwrap();
        let cs_ = c.to_str().unwrap();
        assert_eq!(
            evalcond(&[as_, "-ef", bs_], &opts, &vars, true, None),
            0,
            "c:179 — hardlinks share st_ino + st_dev → -ef true"
        );
        assert_eq!(
            evalcond(&[as_, "-ef", cs_], &opts, &vars, true, None),
            1,
            "c:179 — distinct files → -ef false"
        );
    }

    /// `Src/cond.c:179` — `-nt` / `-ot` compare st_mtime. Newer file
    /// is `-nt` the older; same direction `-ot` is reversed.
    #[cfg(unix)]
    #[test]
    fn test_dash_nt_dash_ot_compare_mtime() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let older = dir.path().join("older");
        let newer = dir.path().join("newer");
        File::create(&older).unwrap();
        // Sleep is needed because some FS have 1s mtime granularity.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let mut f = File::create(&newer).unwrap();
        f.write_all(b"x").unwrap();

        let o = older.to_str().unwrap();
        let n = newer.to_str().unwrap();
        assert_eq!(
            evalcond(&[n, "-nt", o], &opts, &vars, true, None),
            0,
            "c:179 — newer -nt older → true"
        );
        assert_eq!(
            evalcond(&[o, "-nt", n], &opts, &vars, true, None),
            1,
            "c:179 — older -nt newer → false"
        );
        assert_eq!(
            evalcond(&[o, "-ot", n], &opts, &vars, true, None),
            0,
            "c:179 — older -ot newer → true"
        );
    }

    /// `Src/cond.c:179` — `-r` / `-w` map to access(F, R_OK)/W_OK.
    /// Created files inherit rw permissions; chmod 0 strips them.
    #[cfg(unix)]
    #[test]
    fn test_dash_r_dash_w_access_check() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let file = dir.path().join("rw");
        File::create(&file).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let p = file.to_str().unwrap();
        assert_eq!(
            evalcond(&["-r", p], &opts, &vars, true, None),
            0,
            "c:438 — mode 0600 → readable"
        );
        assert_eq!(
            evalcond(&["-w", p], &opts, &vars, true, None),
            0,
            "c:438 — mode 0600 → writable"
        );

        // Root can read anything; skip the strip-permissions check there.
        if unsafe { libc::geteuid() } != 0 {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
            assert_eq!(
                evalcond(&["-r", p], &opts, &vars, true, None),
                1,
                "c:438 — mode 0000 → not readable (non-root)"
            );
            assert_eq!(
                evalcond(&["-w", p], &opts, &vars, true, None),
                1,
                "c:438 — mode 0000 → not writable (non-root)"
            );
        }
    }

    /// `Src/cond.c:81-185` — Double-negation: `! ! foo` cancels out
    /// at the COND_NOT recursion level. `!` parses right-associative.
    #[test]
    fn test_double_negation_cancels() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["!", "!", "-n", "x"], &opts, &vars, true, None),
            0
        );
        assert_eq!(
            evalcond(&["!", "!", "-z", "x"], &opts, &vars, true, None),
            1
        );
    }

    /// `Src/cond.c:81-185` — Implicit `-n` for a bare arg. `[[ foo ]]`
    /// is the same as `[[ -n foo ]]`. Empty bare arg → false.
    #[test]
    fn test_implicit_minus_n_for_bare_arg() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["foo"], &opts, &vars, true, None),
            0,
            "non-empty bare arg → true (implicit -n)"
        );
        assert_eq!(
            evalcond(&[""], &opts, &vars, true, None),
            1,
            "empty bare arg → false (implicit -n)"
        );
    }

    /// `Src/cond.c:525-540` — `cond_str(args, num)` returns the arg
    /// at `num` after singsub. Out-of-bounds → empty string (no panic).
    #[test]
    fn test_cond_str_index_lookup() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        assert_eq!(cond_str(&args, 0, false), "alpha");
        assert_eq!(cond_str(&args, 2, false), "gamma");
        assert_eq!(
            cond_str(&args, 99, false),
            "",
            "c:525 — out-of-bounds index returns empty (Rust safety)"
        );
    }

    /// `Src/cond.c:539-554` — `cond_val(args, num)` parses arg as int.
    /// Non-numeric → 0. Trimmed whitespace allowed.
    #[test]
    fn test_cond_val_int_coerce() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["42".to_string(), "  -7 ".to_string(), "abc".to_string()];
        assert_eq!(cond_val(&args, 0), 42);
        assert_eq!(
            cond_val(&args, 1),
            -7,
            "c:539 — whitespace must trim; negative supported"
        );
        assert_eq!(cond_val(&args, 2), 0, "c:539 — non-numeric returns 0");
        assert_eq!(cond_val(&args, 99), 0, "c:539 — out-of-bounds returns 0");
    }

    /// `Src/cond.c:81` — Parenthesised grouping: `( expr )` evaluates
    /// `expr` in isolation. Missing closing paren → return 2 (error).
    #[test]
    fn test_paren_grouping_and_error_on_missing_close() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        // Balanced: ! ( -z "" )  →  ! true → false (1)
        assert_eq!(
            evalcond(&["!", "(", "-z", "", ")"], &opts, &vars, true, None),
            1
        );
        // Missing close paren: error
        assert_eq!(
            evalcond(&["(", "-z", ""], &opts, &vars, true, None),
            2,
            "missing closing `)` must return 2 (cond error)"
        );
    }

    /// `Src/cond.c:452-468` — `getstat(s)` special-cases `/dev/fd/N`
    /// with `fstat(N, &st)`. Regular paths run through `unmeta(s)`
    /// before `stat(2)`. The previous Rust port used `fs::metadata(s)`
    /// directly, missing the unmeta pass. Pin the regular-path
    /// contract (existence check on `/`).
    #[test]
    fn getstat_resolves_regular_path() {
        let _g = crate::test_util::global_state_lock();
        // Regular path: root exists, must return Some.
        assert!(getstat("/").is_some(), "c:466 — stat('/') must succeed");
        // Nonexistent path returns None.
        assert!(
            getstat("/nonexistent/path/zzz").is_none(),
            "c:464 — nonexistent path returns None"
        );
    }

    /// `Src/cond.c:458-461` — `/dev/fd/N` syntax routes through
    /// `fstat(N, &st)`. The previous Rust port wasted an fd via
    /// unconditional `dup` BEFORE checking fd validity. Fixed: fstat
    /// pre-check, then dup only for the File-ownership wrapper.
    /// Pin: /dev/fd/<stdin-fd> when stdin is a tty doesn't panic.
    #[cfg(unix)]
    #[test]
    fn getstat_dev_fd_path_doesnt_dup_bad_fds() {
        let _g = crate::test_util::global_state_lock();
        // /dev/fd/99 is almost certainly an invalid fd in test env.
        // Pre-fix behavior: would dup it (succeeds or fails), then
        // File::from_raw_fd(<bad fd>), then metadata fails. Net result
        // is still None, but it wasted a dup syscall.
        // Post-fix: fstat fails first → return None without dup.
        let _ = getstat("/dev/fd/99"); // must not panic
                                       // /dev/fd/0 is stdin — usually valid. Test that it doesn't
                                       // panic regardless of stdin shape.
        let _ = getstat("/dev/fd/0");
    }

    /// `Src/cond.c:552-562` — `cond_match` runs `matchpat`. C
    /// `matchpat` reads `EXTENDED_GLOB` and `CASEGLOB` from globals.
    /// The Rust port previously hardcoded `(extended=true,
    /// case_sensitive=true)`, ignoring the option state. Pin the
    /// option-respect contract: after the fix, calling cond_match
    /// reads the live option flags, so a future regression to
    /// hardcoded booleans would be silent without this test.
    ///
    /// Test the function itself (cond_match) — exercises the path
    /// the evalcond `=` operator uses when `posix=false`.
    /// Pin `cond_str` to its canonical C body at `Src/cond.c:525-535`.
    /// Token-free strings pass through unchanged; token-bearing
    /// strings go through singsub + (unless raw) untokenize.
    #[test]
    fn cond_str_passes_through_when_no_tokens() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["hello".to_string()];
        // c:529 — has_token false → return as-is.
        assert_eq!(
            cond_str(&args, 0, false),
            "hello",
            "c:534 — token-free string returned as-is"
        );
        // raw=true also returns same when no tokens.
        assert_eq!(
            cond_str(&args, 0, true),
            "hello",
            "c:534 — token-free string returned as-is regardless of raw"
        );
        // Out-of-bounds → "".
        assert_eq!(
            cond_str(&args, 99, false),
            "",
            "out-of-bounds num returns empty string"
        );
    }

    #[test]
    fn cond_match_runs_matchpat_through_args_indirection() {
        let _g = crate::test_util::global_state_lock();
        // Literal equality always matches regardless of options.
        let args = vec!["hello".to_string()];
        assert!(
            cond_match(&args, 0, "hello"),
            "literal pattern matches identical text"
        );
        assert!(
            !cond_match(&args, 0, "world"),
            "literal pattern rejects non-match"
        );
        // Out-of-bounds index → false (no panic, args.get returns None).
        assert!(
            !cond_match(&args, 99, "hello"),
            "out-of-bounds num returns false"
        );

        // c:556-557 — pattern goes through matchpat in (pattern, text)
        // order. Asymmetric glob: `*.txt` is a pattern that matches
        // `file.txt` (text) but NOT vice-versa. Previous Rust port had
        // args reversed: matchpat(str, p, ...) = matchpat(text-as-pattern,
        // pattern-as-text) — `[[ file.txt = *.txt ]]` would silently
        // match `*.txt` against text="file.txt" treating "*.txt" as a
        // string and "file.txt" as a glob, mis-routing every glob test.
        let args = vec!["*.txt".to_string()];
        assert!(
            cond_match(&args, 0, "file.txt"),
            "c:556-557 — pattern `*.txt` matches text `file.txt`"
        );
        // The reverse direction MUST NOT match — `file.txt` is not a
        // glob pattern that matches `*.txt` (the asterisk would be a
        // literal). If args were reversed, this would pass too.
        let args = vec!["file.txt".to_string()];
        assert!(
            !cond_match(&args, 0, "*.txt"),
            "c:556-557 — literal pattern `file.txt` does NOT match text `*.txt` \
             (this catches the swapped-arg regression)"
        );
    }

    /// Pin: `cond_val` routes through `mathevali` per `Src/cond.c:548`.
    /// A `[[ -eq ]]` operand of `"1+2"` must evaluate to 3, not 0.
    /// `[[ N -eq M+0 ]]`-style asserts must work.
    #[test]
    fn cond_val_routes_through_mathevali() {
        let _g = crate::test_util::global_state_lock();
        let args = vec![
            "1+2".to_string(),
            "10/2".to_string(),
            "0x10".to_string(),
            "2**8".to_string(),
        ];
        // c:548 — `1+2` → 3 (addition)
        assert_eq!(
            cond_val(&args, 0),
            3,
            "c:548 — `mathevali(\"1+2\")` evaluates the expression"
        );
        // c:548 — `10/2` → 5 (integer division)
        assert_eq!(
            cond_val(&args, 1),
            5,
            "c:548 — `mathevali(\"10/2\")` evaluates the expression"
        );
        // c:548 — `0x10` → 16 (hex literal via mathevali)
        assert_eq!(
            cond_val(&args, 2),
            16,
            "c:548 — `mathevali(\"0x10\")` parses hex"
        );
        // c:548 — `2**8` → 256 (exponent operator)
        assert_eq!(
            cond_val(&args, 3),
            256,
            "c:548 — `mathevali(\"2**8\")` evaluates the expression"
        );
    }

    /// Pin: `cond_val` with plain integer string returns that integer.
    /// Boundary cases — empty string, negative numbers, plain digits.
    #[test]
    fn cond_val_plain_integers() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["0".to_string(), "-42".to_string(), "123456789".to_string()];
        assert_eq!(cond_val(&args, 0), 0);
        assert_eq!(cond_val(&args, 1), -42);
        assert_eq!(cond_val(&args, 2), 123456789);
        // Out-of-bounds index → 0 (args.get returns None).
        assert_eq!(cond_val(&args, 99), 0, "out-of-bounds num returns 0");
    }

    /// Pin: `[[ -t fd ]]` routes the fd through `mathevali` per
    /// Src/cond.c:330+, accepting any arithmetic expression. The
    /// previous Rust port used `.parse::<i32>()` which only
    /// accepted plain decimal digits, rejecting valid forms like
    /// `[[ -t 1+0 ]]` or `[[ -t $((1)) ]]` (`$(())` expansion
    /// already happens before the test, but other arith forms
    /// like `2-1` would reach the test verbatim).
    ///
    /// fd 1 (stdout) under cargo test is typically not a tty
    /// (piped to test harness), so the result is "not a tty"
    /// (return code 1). Any non-tty fd returns 1; this confirms
    /// the mathevali path resolved the expression — the previous
    /// `.parse()` would have returned 2 (syntax error) for `1+0`.
    #[test]
    fn evalcond_dash_t_accepts_arithmetic_per_cond_c() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        // c:330 — `[[ -t 1+0 ]]`. Should evaluate to 0 or 1 (tty
        // check), NOT 2 (syntax error). The previous Rust port
        // would return 2 because `.parse::<i32>("1+0")` fails.
        let result = evalcond(&["-t", "1+0"], &opts, &vars, true, None);
        assert!(
            result == 0 || result == 1,
            "c:330 — `-t 1+0` must mathevali to fd 1 (not parse fail), got {}",
            result
        );
        // c:330 — `[[ -t 0 ]]` plain digit also works.
        let result = evalcond(&["-t", "0"], &opts, &vars, true, None);
        assert!(
            result == 0 || result == 1,
            "c:330 — `-t 0` plain digit still works, got {}",
            result
        );
    }

    // ─── Test/C02cond.ztst:175-193 — string/numeric cond pins ─────────

    /// `Test/C02cond.ztst:175-176` — `[[ '' = '' ]]`, `[[ a == a ]]`,
    /// `[[ x != y ]]` — equality operators.
    #[test]
    fn cond_corpus_equality_operators() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["", "=", ""], &opts, &vars, false, None),
            0,
            "= empty=empty"
        );
        assert_eq!(
            evalcond(&["a", "==", "a"], &opts, &vars, false, None),
            0,
            "== equal"
        );
        assert_eq!(
            evalcond(&["x", "!=", "y"], &opts, &vars, false, None),
            0,
            "!= unequal"
        );
        assert_eq!(
            evalcond(&["x", "==", "y"], &opts, &vars, false, None),
            1,
            "== unequal false"
        );
    }

    /// `Test/C02cond.ztst:180-181` — `[[ 7 -eq 0x07 ]]` — hex constants
    /// in `-eq` comparisons via mathevali.
    #[test]
    fn cond_corpus_eq_hex_constant() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["7", "-eq", "0x07"], &opts, &vars, false, None),
            0,
            "7 -eq 0x07"
        );
        assert_eq!(
            evalcond(&["10", "-ne", "0x10"], &opts, &vars, false, None),
            0,
            "10 -ne 0x10 (16)"
        );
        assert_eq!(
            evalcond(&["16", "-eq", "0x10"], &opts, &vars, false, None),
            0,
            "16 -eq 0x10"
        );
    }

    /// `Test/C02cond.ztst:183-184` — `[[ 3 -lt 04 ]]` — leading-zero
    /// numerics treated as octal under mathevali (04 = 4 decimal).
    #[test]
    fn cond_corpus_lt_gt_with_leading_zero() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["3", "-lt", "04"], &opts, &vars, false, None),
            0,
            "3 -lt 4"
        );
        assert_eq!(
            evalcond(&["05", "-gt", "2"], &opts, &vars, false, None),
            0,
            "5 -gt 2"
        );
    }

    /// `Test/C02cond.ztst:186-187` — `[[ 3 -le 3 && ! (4 -le 3) ]]`
    /// equal/lesser boundary + negation.
    #[test]
    fn cond_corpus_le_equal_boundary() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["3", "-le", "3"], &opts, &vars, false, None),
            0,
            "3 -le 3"
        );
        assert_eq!(
            evalcond(&["4", "-le", "3"], &opts, &vars, false, None),
            1,
            "4 -le 3 false"
        );
    }

    /// `Test/C02cond.ztst:189-190` — `[[ 3 -ge 3 && ! (3 -ge 4) ]]`.
    #[test]
    fn cond_corpus_ge_equal_boundary() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["3", "-ge", "3"], &opts, &vars, false, None),
            0,
            "3 -ge 3"
        );
        assert_eq!(
            evalcond(&["3", "-ge", "4"], &opts, &vars, false, None),
            1,
            "3 -ge 4 false"
        );
    }

    /// `Test/C02cond.ztst:128` — `[[ -x file ]]` — true when file has
    /// execute permission for current user.
    #[test]
    fn cond_corpus_minus_x_executable() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        let dir = TempDir::new().unwrap();
        let exe = dir.path().join("exe");
        File::create(&exe).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            evalcond(&["-x", exe.to_str().unwrap()], &opts, &vars, false, None),
            0,
            "ztst:128 — -x true for 0755 file",
        );
        let noexe = dir.path().join("noexe");
        File::create(&noexe).unwrap();
        std::fs::set_permissions(&noexe, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            evalcond(&["-x", noexe.to_str().unwrap()], &opts, &vars, false, None),
            1,
            "ztst:128 — -x false for 0644 file",
        );
    }

    /// `Test/C02cond.ztst:117` — `[[ -r file ]]` — true when file is readable.
    #[test]
    fn cond_corpus_minus_r_readable() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        let dir = TempDir::new().unwrap();
        let readable = dir.path().join("r");
        File::create(&readable).unwrap();
        std::fs::set_permissions(&readable, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            evalcond(
                &["-r", readable.to_str().unwrap()],
                &opts,
                &vars,
                false,
                None
            ),
            0,
            "ztst:117 — -r true for 0644 file",
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/cond.c helper fns.
    // ═══════════════════════════════════════════════════════════════════

    /// `doaccess("/", F_OK)` returns 1 (true) — root always exists.
    /// C `Src/cond.c:doaccess` — `!access(unmeta(s), c)` — F_OK=0
    /// passes for any existing path.
    #[cfg(unix)]
    #[test]
    fn doaccess_root_dir_exists_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(doaccess("/", libc::F_OK), 1, "/ exists → true");
    }

    /// `doaccess(non-existent, F_OK)` returns 0.
    #[cfg(unix)]
    #[test]
    fn doaccess_missing_path_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            doaccess("/no/such/path/zshrs_test_xyz", libc::F_OK),
            0,
            "non-existent path → false"
        );
    }

    /// `dostat` on a missing path returns 0 (no mode bits).
    #[cfg(unix)]
    #[test]
    fn dostat_missing_path_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mode = dostat("/no/such/path/zshrs_test_xyz");
        assert_eq!(mode, 0, "missing path → mode=0");
    }

    /// `dostat("/")` returns a mode with S_IFDIR bit set.
    #[cfg(unix)]
    #[test]
    fn dostat_root_dir_has_ifdir_bit() {
        let _g = crate::test_util::global_state_lock();
        let mode = dostat("/");
        let is_dir = (mode & libc::S_IFMT as u32) == libc::S_IFDIR as u32;
        assert!(is_dir, "/ should have S_IFDIR; got mode=0o{mode:o}");
    }

    /// `optison(Some("test"), "definitely_not_an_option")` returns nonzero.
    /// C: with POSIXBUILTINS off → 3 (with zwarnnam); with on → 1.
    /// Either way: nonzero.
    #[test]
    fn optison_unknown_option_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = optison(Some("test"), "zshrs_definitely_not_an_option_xyz");
        assert_ne!(r, 0, "unknown option → nonzero error (1 or 3)");
    }

    /// `optison(Some("test"), "x")` for `set -x` — never panics, returns
    /// 0/1 based on current xtrace state.
    #[test]
    fn optison_single_char_xtrace_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let r = optison(Some("test"), "x");
        assert!(r == 0 || r == 1, "single-char x must return 0/1; got {r}");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/cond.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:438 — `doaccess("/tmp", F_OK)` returns 1 (/tmp exists everywhere).
    #[test]
    #[cfg(unix)]
    fn doaccess_existing_dir_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(doaccess("/tmp", libc::F_OK), 1, "/tmp must exist");
    }

    /// c:438 — `doaccess` on nonexistent path returns 0.
    #[test]
    #[cfg(unix)]
    fn doaccess_nonexistent_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            doaccess("/__never_exists_zshrs_xyz__", libc::F_OK),
            0,
            "missing path → 0"
        );
    }

    /// c:474 — `dostat` on /tmp returns nonzero mode (it's a dir).
    #[test]
    #[cfg(unix)]
    fn dostat_directory_returns_nonzero_mode() {
        let _g = crate::test_util::global_state_lock();
        let mode = dostat("/tmp");
        assert_ne!(mode, 0, "/tmp must have non-zero mode bits");
    }

    /// c:474 — `dostat` on nonexistent returns 0.
    #[test]
    fn dostat_nonexistent_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dostat("/__never_exists_zshrs_xyz__"), 0);
    }

    /// c:488 — `dolstat` on /tmp returns nonzero mode.
    #[test]
    #[cfg(unix)]
    fn dolstat_directory_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let m = dolstat("/tmp");
        assert_ne!(m, 0);
    }

    /// c:488 — `dolstat` on missing path returns 0.
    #[test]
    fn dolstat_nonexistent_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dolstat("/__never_exists_zshrs_xyz__"), 0);
    }

    /// c:653 — `cond_str` with out-of-range index returns empty string.
    #[test]
    fn cond_str_out_of_range_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["a".to_string(), "b".to_string()];
        assert_eq!(cond_str(&args, 5, false), "", "idx 5 of 2 args → empty");
        assert_eq!(cond_str(&args, 100, false), "");
    }

    /// c:653 — `cond_str` in-range index returns the arg.
    #[test]
    fn cond_str_in_range_returns_arg() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["zero".to_string(), "one".to_string()];
        assert_eq!(cond_str(&args, 0, false), "zero");
        assert_eq!(cond_str(&args, 1, false), "one");
    }

    /// c:685 — `cond_val` for out-of-range returns 0.
    #[test]
    fn cond_val_out_of_range_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["42".to_string()];
        assert_eq!(cond_val(&args, 5), 0);
    }

    /// c:685 — `cond_val("42")` parses to 42.
    #[test]
    fn cond_val_parses_canonical_decimal() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["42".to_string()];
        assert_eq!(cond_val(&args, 0), 42);
    }

    /// c:716 — `cond_match` with empty pattern returns false / true
    /// per fnmatch semantics. Pin no panic.
    #[test]
    fn cond_match_empty_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let args: Vec<String> = vec![];
        let _ = cond_match(&args, 0, "anything");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/cond.c
    // c:513 doaccess / c:538 getstat / c:570 dostat / c:583 dolstat
    // c:594 optison / c:653 cond_str / c:685 cond_val / c:716 cond_match
    // c:758 tracemodcond
    // ═══════════════════════════════════════════════════════════════════

    /// c:513 — `doaccess` returns i32 (compile-time type pin).
    #[test]
    fn doaccess_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = doaccess("/tmp", 0);
    }

    /// c:513 — `doaccess` is deterministic.
    #[test]
    fn doaccess_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for (p, c) in [("/tmp", 0), ("/nonexistent_xyz", 0), ("", 0)] {
            let first = doaccess(p, c);
            for _ in 0..3 {
                assert_eq!(
                    doaccess(p, c),
                    first,
                    "doaccess({:?}, {}) must be deterministic",
                    p,
                    c
                );
            }
        }
    }

    /// c:570 — `dostat("")` empty path returns 0 (file not found).
    #[test]
    fn dostat_empty_path_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dostat(""), 0, "empty path → 0");
    }

    /// c:583 — `dolstat("")` empty path returns 0.
    #[test]
    fn dolstat_empty_path_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dolstat(""), 0, "empty path → 0");
    }

    /// c:570 — `dostat` returns u32 (compile-time type pin).
    #[test]
    fn dostat_returns_u32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: u32 = dostat("/tmp");
    }

    /// c:594 — `optison` returns i32 (compile-time type pin).
    #[test]
    fn optison_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = optison(Some("test"), "x");
    }

    /// c:594 — `optison` is deterministic.
    #[test]
    fn optison_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for s in ["x", "y", "v"] {
            let first = optison(Some("test"), s);
            for _ in 0..3 {
                assert_eq!(
                    optison(Some("test"), s),
                    first,
                    "optison(test, {:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:653 — `cond_str` returns String (compile-time type pin).
    #[test]
    fn cond_str_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["x".to_string()];
        let _: String = cond_str(&args, 0, false);
    }

    /// c:685 — `cond_val` returns i64 (compile-time type pin).
    #[test]
    fn cond_val_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["42".to_string()];
        let _: i64 = cond_val(&args, 0);
    }

    /// c:716 — `cond_match` returns bool (compile-time type pin).
    #[test]
    fn cond_match_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["*".to_string()];
        let _: bool = cond_match(&args, 0, "abc");
    }

    /// c:758 — `tracemodcond` empty args no panic.
    #[test]
    fn tracemodcond_empty_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        tracemodcond("test", &[], false);
        tracemodcond("test", &[], true);
    }

    /// c:653 — `cond_str` with empty args returns empty (no out-of-range).
    #[test]
    fn cond_str_empty_args_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let args: Vec<String> = vec![];
        let r = cond_str(&args, 0, false);
        assert!(r.is_empty(), "empty args + idx 0 → empty");
    }

    /// c:685 — `cond_val` with empty args returns 0.
    #[test]
    fn cond_val_empty_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let args: Vec<String> = vec![];
        assert_eq!(cond_val(&args, 0), 0, "empty args + idx 0 → 0");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/cond.c
    // c:513 doaccess / c:538 getstat / c:570 dostat / c:583 dolstat /
    // c:594 optison / c:653 cond_str / c:685 cond_val / c:716 cond_match /
    // c:758 tracemodcond
    // ═══════════════════════════════════════════════════════════════════

    /// c:513 — `doaccess("")` empty path doesn't panic.
    #[test]
    fn doaccess_empty_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = doaccess("", libc::F_OK);
    }

    /// c:513 — `doaccess` various access modes safe for nonexistent path.
    #[test]
    fn doaccess_nonexistent_path_various_modes_safe() {
        let _g = crate::test_util::global_state_lock();
        for mode in [libc::F_OK, libc::R_OK, libc::W_OK, libc::X_OK] {
            let _ = doaccess("__never_real_path_xyz_abc__", mode);
        }
    }

    /// c:538 — `getstat("")` empty path returns None.
    #[test]
    fn getstat_empty_path_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getstat("").is_none(), "getstat(\"\") must be None");
    }

    /// c:538 — `getstat` for "/" (root) returns Some on Unix.
    #[cfg(unix)]
    #[test]
    fn getstat_root_returns_some() {
        let _g = crate::test_util::global_state_lock();
        assert!(getstat("/").is_some(), "/ must exist and return Some");
    }

    /// c:570/583 — dostat == dolstat for non-symlink paths.
    #[cfg(unix)]
    #[test]
    fn dostat_dolstat_equal_for_regular_path() {
        let _g = crate::test_util::global_state_lock();
        let a = dostat("/");
        let b = dolstat("/");
        assert_eq!(a, b, "dostat and dolstat must agree on / (non-symlink)");
    }

    /// c:570 — `dostat` is deterministic.
    #[test]
    fn dostat_deterministic_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        let a = dostat("/tmp");
        let b = dostat("/tmp");
        assert_eq!(a, b, "dostat must be pure");
    }

    /// c:594 — `optison("")` empty name doesn't panic.
    #[test]
    fn optison_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = optison(None, "");
    }

    /// c:653 — `cond_str` out-of-bounds num returns empty.
    #[test]
    fn cond_str_oob_num_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let args: Vec<String> = vec!["a".into(), "b".into()];
        let r = cond_str(&args, 999, false);
        assert!(r.is_empty(), "OOB num must return empty, got {:?}", r);
    }

    /// c:685 — `cond_val` out-of-bounds num returns 0.
    #[test]
    fn cond_val_oob_num_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let args: Vec<String> = vec!["a".into(), "b".into()];
        let r = cond_val(&args, 999);
        assert_eq!(r, 0, "OOB num must return 0");
    }

    /// c:716 — `cond_match("", 0, "")` (empty match) doesn't panic.
    #[test]
    fn cond_match_empty_inputs_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let args: Vec<String> = vec![];
        let _ = cond_match(&args, 0, "");
    }

    /// c:758 — `tracemodcond` with various flag values safe.
    #[test]
    fn tracemodcond_various_flag_combinations_safe() {
        let _g = crate::test_util::global_state_lock();
        let args = vec!["a".into(), "b".into()];
        tracemodcond("test", &args, true);
        tracemodcond("test", &args, false);
        tracemodcond("", &[], true);
    }

    /// c:653/685/716 — args is borrowed (no mutation across calls).
    #[test]
    fn cond_str_val_match_dont_mutate_args() {
        let _g = crate::test_util::global_state_lock();
        let mut args: Vec<String> = vec!["x".into(), "y".into()];
        let before = args.clone();
        let _ = cond_str(&args, 0, false);
        let _ = cond_val(&args, 0);
        let _ = cond_match(&args, 0, "");
        args.shrink_to_fit();
        assert_eq!(args, before, "args must remain unchanged");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Wordcode form of evalcond (Src/cond.c:70) — hand-assembled WC_COND
    // programs in the layout par_cond_* emits (parse.c:2422-2723).
    // ═══════════════════════════════════════════════════════════════════

    /// Append one string operand as `ecstr` would: empty -> 6, up to three
    /// bytes inline (bit 1), otherwise an offset into the string pool.
    fn wc_str(code: &mut Vec<u32>, pool: &mut String, s: &str) {
        let b = s.as_bytes();
        if b.is_empty() {
            code.push(6);
        } else if b.len() <= 3 {
            let g = |i: usize| *b.get(i).unwrap_or(&0) as u32;
            code.push(2 | (g(0) << 3) | (g(1) << 11) | (g(2) << 19));
        } else {
            code.push((pool.len() as u32) << 2);
            pool.push_str(s);
            pool.push('\0');
        }
    }

    /// Wrap an assembled program in the `estate` `execcond` would hand over.
    fn wc_state(code: Vec<u32>, pool: String) -> crate::ported::zsh_h::estate {
        let p = crate::ported::zsh_h::eprog {
            flags: 0,
            len: code.len() as i32,
            npats: 0,
            nref: 0,
            pats: Vec::new(),
            prog: code,
            strs: Some(pool.clone()),
            shf: None,
            dump: None,
            strs_metafied: false,
        };
        crate::ported::zsh_h::estate {
            prog: Box::new(p),
            pc: 0,
            strs: Some(pool),
            strs_offset: 0,
        }
    }

    /// Unary `-X arg` node: `WCB_COND(X, 0)` then the operand.
    fn wc_unary(op: char, arg: &str) -> (Vec<u32>, String) {
        let (mut code, mut pool) = (vec![crate::ported::zsh_h::WCB_COND(op as u32, 0)], String::new());
        wc_str(&mut code, &mut pool, arg);
        (code, pool)
    }

    /// Binary node: `WCB_COND(ty, 0)`, both operands, plus the pattern slot
    /// word the string-compare types carry (parse.c:2664-2681).
    fn wc_binary(ty: i32, l: &str, r: &str) -> (Vec<u32>, String) {
        let (mut code, mut pool) = (vec![crate::ported::zsh_h::WCB_COND(ty as u32, 0)], String::new());
        wc_str(&mut code, &mut pool, l);
        wc_str(&mut code, &mut pool, r);
        if ty == COND_STREQ || ty == COND_STRDEQ || ty == COND_STRNEQ {
            code.push(0);
        }
        (code, pool)
    }

    /// Run one program through the wordcode evalcond; returns (status, pc, len).
    fn wc_run(code: Vec<u32>, pool: String, fromtest: Option<&str>) -> (i32, usize, usize) {
        let len = code.len();
        let mut st = wc_state(code, pool);
        let r = super::evalcond(&mut st, fromtest);
        (r, st.pc, len)
    }

    /// Concatenate `head` and sub-programs; operands here are all inline
    /// strings, so the string pools stay empty and need no rebasing.
    fn wc_join(head: u32, parts: &[(Vec<u32>, String)]) -> (Vec<u32>, String) {
        let mut code = vec![head];
        for (c, p) in parts {
            assert!(p.is_empty(), "wc_join only handles inline operands");
            code.extend_from_slice(c);
        }
        (code, String::new())
    }

    #[test]
    fn wordcode_unary_string_tests() {
        let _g = crate::test_util::global_state_lock();
        let (c, p) = wc_unary('n', "abc");
        let (r, pc, len) = wc_run(c, p, None);
        assert_eq!((r, pc), (0, len), "-n abc: true, operand consumed");
        let (c, p) = wc_unary('z', "abc");
        assert_eq!(wc_run(c, p, None).0, 1, "-z abc: false");
        let (c, p) = wc_unary('z', "");
        assert_eq!(wc_run(c, p, None).0, 0, "-z '': true");
        let (c, p) = wc_unary('n', "");
        assert_eq!(wc_run(c, p, None).0, 1, "-n '': false");
    }

    #[test]
    fn wordcode_file_tests_use_stat_and_access() {
        let _g = crate::test_util::global_state_lock();
        let (c, p) = wc_unary('d', "/");
        assert_eq!(wc_run(c, p, None).0, 0, "-d /");
        let (c, p) = wc_unary('f', "/");
        assert_eq!(wc_run(c, p, None).0, 1, "-f /");
        let (c, p) = wc_unary('e', "/nonexistent/zshrs/probe");
        assert_eq!(wc_run(c, p, None).0, 1, "-e missing");
        let (c, p) = wc_binary(COND_EF, "/", "/");
        assert_eq!(wc_run(c, p, None).0, 0, "/ -ef /");
        let (c, p) = wc_binary(COND_NT, "/", "/");
        assert_eq!(wc_run(c, p, None).0, 1, "/ -nt / is false (equal mtimes)");
        let (c, p) = wc_binary(COND_NT, "/nonexistent/zshrs/probe", "/");
        assert_eq!(wc_run(c, p, None).0, 1, "missing left operand -> 1");
    }

    #[test]
    fn wordcode_string_equality_consumes_pattern_slot() {
        let _g = crate::test_util::global_state_lock();
        let (c, p) = wc_binary(COND_STREQ, "abcd", "abcd");
        let (r, pc, len) = wc_run(c, p, None);
        assert_eq!((r, pc), (0, len), "c:325 `state->pc += 2` leaves pc at program end");
        let (c, p) = wc_binary(COND_STRDEQ, "abc", "abd");
        assert_eq!(wc_run(c, p, None).0, 1, "abc == abd");
        let (c, p) = wc_binary(COND_STRNEQ, "abc", "abd");
        assert_eq!(wc_run(c, p, None).0, 0, "abc != abd");
    }

    #[test]
    fn wordcode_numeric_compare_math_vs_fromtest() {
        let _g = crate::test_util::global_state_lock();
        let (c, p) = wc_binary(COND_LT, "1", "2");
        assert_eq!(wc_run(c, p, None).0, 0, "1 -lt 2");
        let (c, p) = wc_binary(COND_EQ, "2", "10");
        assert_eq!(wc_run(c, p, None).0, 1, "2 -eq 10 compares numbers, not strings");
        let (c, p) = wc_binary(COND_EQ, "1+2", "3");
        assert_eq!(wc_run(c, p, None).0, 0, "[[ ]] evaluates operands as math (c:257)");
        let (c, p) = wc_binary(COND_LT, "1.5", "2");
        assert_eq!(wc_run(c, p, None).0, 0, "mixed float/int promotes to float (c:261)");
        let (c, p) = wc_binary(COND_EQ, "1+2", "3");
        assert_eq!(wc_run(c, p, Some("test")).0, 2, "test/[ require base-10 integers (c:232)");
    }

    #[test]
    fn wordcode_not_and_or_short_circuit_and_skip() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::{WCB_COND, COND_AND, COND_MOD, COND_NOT, COND_OR};
        let t = || wc_unary('n', "x");
        let f = || wc_unary('z', "x");
        // NOT
        let (code, pool) = wc_join(WCB_COND(COND_NOT as u32, 0), &[f()]);
        assert_eq!(wc_run(code, pool, None).0, 0, "! false");
        // AND: skip = words after the AND word (parse.c:2447).
        for (l, r, want) in [(t(), t(), 0), (t(), f(), 1), (f(), t(), 1), (f(), f(), 1)] {
            let skip = (l.0.len() + r.0.len()) as u32;
            let (code, pool) = wc_join(WCB_COND(COND_AND as u32, skip), &[l, r]);
            let (res, pc, len) = wc_run(code, pool, None);
            assert_eq!(res, want, "AND result");
            assert_eq!(pc, len, "short-circuit must still land pc past the AND node (c:100)");
        }
        // OR
        for (l, r, want) in [(t(), t(), 0), (t(), f(), 0), (f(), t(), 0), (f(), f(), 1)] {
            let skip = (l.0.len() + r.0.len()) as u32;
            let (code, pool) = wc_join(WCB_COND(COND_OR as u32, skip), &[l, r]);
            let (res, pc, len) = wc_run(code, pool, None);
            assert_eq!(res, want, "OR result");
            assert_eq!(pc, len, "short-circuit must still land pc past the OR node (c:110)");
        }
    }

    #[test]
    fn wordcode_unknown_condition_is_status_2() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::{WCB_COND, COND_AND, COND_MOD, COND_NOT, COND_OR};
        // COND_MOD with one operand and an operator no module defines.
        let (mut code, mut pool) = (vec![WCB_COND(COND_MOD as u32, 1)], String::new());
        wc_str(&mut code, &mut pool, "-zq");
        wc_str(&mut code, &mut pool, "arg");
        assert_eq!(wc_run(code, pool, None).0, 2, "c:190-194");
    }
}
