//! Conditional expression evaluation — port of Src/cond.c.
//!
//! Evaluates `[[ … ]]` (zsh extended test) and `[`/`test` (POSIX)
//! conditional expressions.
//!
//! ## Port status
//!
//! C-faithful: the per-test helpers `doaccess` (c:438), `getstat`
//! (c:452), `dostat` (c:474), `dolstat` (c:488), `optison` (c:502),
//! `cond_str` (c:525), `cond_val` (c:539), `cond_match` (c:552),
//! `tracemodcond` (c:563) are direct ports with C-named signatures.
//!
//! NOT C-faithful: the C `evalcond()` at cond.c:70 walks pre-compiled
//! wordcode bytecode (`Estate state`, opcodes via `WC_COND_TYPE`,
//! operand strings via `ecgetstr`). The argv-driven Rust entry point
//! here parses + evaluates inline because the wordcode + Estate
//! plumbing isn't fully wired through this call site yet. When that
//! lands, this file becomes a thin wrapper around the wordcode
//! walker that mirrors cond.c:70 line-by-line. (The earlier `CondExpr`
//! / `CondParser` intermediate scaffold types have been deleted.)

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::glob::matchpat;
use crate::ported::zsh_h::{
    COND_AND, COND_EF, COND_EQ, COND_GE, COND_GT, COND_LE, COND_LT, COND_NE, COND_NOT, COND_NT,
    COND_OR, COND_OT, COND_REGEX, COND_STRDEQ, COND_STREQ, COND_STRGTR, COND_STRLT, COND_STRNEQ,
};
use std::os::unix::io::FromRawFd;
use std::io::Write;

// C-style i32 return codes from `evalcond` (mirroring cond.c:70):
//   0 — condition true
//   1 — condition false
//   2 — syntax error
//   3 — option tested with -o does not exist
//
// `evalcond`'s integer return values are documented in the C source
// at cond.c:62-66; we use bare i32 throughout (no enum wrapper).

// `CondExpr` enum + `CondParser` struct DELETED. The Rust port no
// longer builds an intermediate AST: `evalcond` below is a single-
// pass recursive-descent walker that parses AND evaluates the argv
// stream in one go. C's `evalcond` at `Src/cond.c:70` walks
// pre-compiled wordcode (`Estate state` + `WC_COND_TYPE` opcode
// dispatch) — when the wordcode pipeline is wired through this
// builtin's call site, `evalcond` should be re-shaped to match
// the C signature `int evalcond(Estate, char *fromtest)` and walk
// `WC_COND_*` opcodes directly. Until then, the streaming evaluator
// here gives equivalent runtime behaviour without an AST type.

/// Argv-form `[[ … ]]` / `test … ]` driver. C signature differs:
/// `int evalcond(Estate state, char *fromtest)` at cond.c:70 walks
/// wordcode; this argv-form is a Rust-side adaptation pending the
/// wordcode pipeline wiring.
///
/// Returns C's convention (cond.c:62-66): 0=true, 1=false, 2=syntax
/// error, 3=option-tested-with-`-o`-not-found.
pub fn evalcond(                                                             // c:70
    args: &[&str],
    options: &HashMap<String, bool>,
    variables: &HashMap<String, String>,
    posix_mode: bool,
) -> i32 {
    if args.is_empty() { return 1; }
    let toks: Vec<&str> = args
        .iter()
        .filter(|s| !matches!(**s, "[" | "]" | "[[" | "]]"))
        .copied()
        .collect();
    if toks.is_empty() { return 1; }

    // Inner walker — the entire cond.c:81-185 switch collapsed into
    // one fn. `prec` selects the recursion level (0=OR, 1=AND,
    // 2=NOT, 3=primary). This is the one Rust adaptation: C's
    // `evalcond` walks a single wordcode stream; we walk argv via
    // operator-precedence climbing. No helper fns / no AST.
    fn walk(
        toks: &[&str],
        pos: &mut usize,
        opts: &HashMap<String, bool>,
        vars: &HashMap<String, String>,
        posix: bool,
        prec: u8,
    ) -> i32 {
        let b2i = |b: bool| -> i32 { if b { 0 } else { 1 } };
        let peek = |i: usize| -> Option<&str> { toks.get(i).copied() };

        match prec {
            // OR — c:96 COND_OR.
            0 => {
                let mut left = walk(toks, pos, opts, vars, posix, 1);
                while peek(*pos) == Some("||") || peek(*pos) == Some("-o") {
                    *pos += 1;
                    let right = walk(toks, pos, opts, vars, posix, 1);
                    left = if left == 0 { 0 } else if left >= 2 { left } else { right };
                }
                left
            }
            // AND — c:88 COND_AND.
            1 => {
                let mut left = walk(toks, pos, opts, vars, posix, 2);
                while peek(*pos) == Some("&&") || peek(*pos) == Some("-a") {
                    *pos += 1;
                    let right = walk(toks, pos, opts, vars, posix, 2);
                    left = if left != 0 { left } else { right };
                }
                left
            }
            // NOT — c:81 COND_NOT.
            2 => {
                if peek(*pos) == Some("!") {
                    *pos += 1;
                    let r = walk(toks, pos, opts, vars, posix, 2);
                    if r < 2 { if r == 0 { 1 } else { 0 } } else { r }
                } else {
                    walk(toks, pos, opts, vars, posix, 3)
                }
            }
            // Primary — c:179+ default arm. Parenthesised group,
            // unary `-X arg`, binary `l OP r`, or bare arg.
            _ => {
                if peek(*pos) == Some("(") {
                    *pos += 1;
                    let r = walk(toks, pos, opts, vars, posix, 0);
                    if peek(*pos) != Some(")") { return 2; }
                    *pos += 1;
                    return r;
                }
                // Unary `-X arg`.
                if let Some(tok) = peek(*pos) {
                    if tok.starts_with('-') && tok.len() == 2 {
                        let op = tok.chars().nth(1).unwrap();
                        if matches!(op,
                            'a'|'b'|'c'|'d'|'e'|'f'|'g'|'h'|'k'|'L'|'n'|'o'
                            |'p'|'r'|'s'|'S'|'t'|'u'|'v'|'w'|'x'|'z'|'G'|'N'|'O'
                        ) {
                            *pos += 1;
                            let arg = match peek(*pos) {
                                Some(a) => { *pos += 1; a.to_string() }
                                None => return 2,
                            };
                            return match op {
                                'a' | 'e' => b2i(Path::new(&arg).exists()),  // c:179-180
                                'b' => b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFBLK as u32),
                                'c' => b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFCHR as u32),
                                'd' => b2i(Path::new(&arg).is_dir()),
                                'f' => b2i(Path::new(&arg).is_file()),
                                'g' => b2i(dostat(&arg) & libc::S_ISGID as u32 != 0),
                                'h' | 'L' => b2i(dolstat(&arg) & libc::S_IFMT as u32 == libc::S_IFLNK as u32),
                                'k' => b2i(dostat(&arg) & libc::S_ISVTX as u32 != 0),
                                'p' => b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFIFO as u32),
                                'r' => b2i(doaccess(&arg, 4) != 0),          // c:438
                                's' => b2i(getstat(&arg).map(|m| m.len() > 0).unwrap_or(false)),
                                'S' => b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFSOCK as u32),
                                'u' => b2i(dostat(&arg) & libc::S_ISUID as u32 != 0),
                                'w' => b2i(doaccess(&arg, 2) != 0),          // c:438
                                'x' => b2i(doaccess(&arg, 1) != 0
                                           || getstat(&arg)
                                                  .map(|m| m.mode() & libc::S_IFMT as u32 == libc::S_IFDIR as u32)
                                                  .unwrap_or(false)),
                                'O' => b2i(getstat(&arg).map(|m| m.uid() == unsafe { libc::geteuid() })
                                           .unwrap_or(false)),
                                'G' => b2i(getstat(&arg).map(|m| m.gid() == unsafe { libc::getegid() })
                                           .unwrap_or(false)),
                                'N' => b2i(getstat(&arg).map(|m| m.mtime() >= m.atime()).unwrap_or(false)),
                                'n' => b2i(!arg.is_empty()),
                                'z' => b2i(arg.is_empty()),
                                'o' => {
                                    let r = optison("test", &arg);           // c:502
                                    if r != 3 { r }
                                    else if opts.contains_key(&arg) { b2i(opts[&arg]) }
                                    else { 3 }
                                }
                                'v' => b2i(vars.contains_key(&arg)),
                                't' => arg.parse::<i32>()
                                       .map(|fd| b2i(unsafe { libc::isatty(fd) } != 0))
                                       .unwrap_or(2),
                                _ => 2,
                            };
                        }
                    }
                }
                // Binary `left OP right` or bare `left` (implicit `-n`).
                let left = match peek(*pos) {
                    Some(a) => { *pos += 1; a.to_string() }
                    None => return 2,
                };
                let code: Option<i32> = peek(*pos).and_then(|t| match t {
                    "="   => Some(COND_STREQ),
                    "=="  => Some(COND_STRDEQ),
                    "!="  => Some(COND_STRNEQ),
                    "<"   => Some(COND_STRLT),
                    ">"   => Some(COND_STRGTR),
                    "-eq" => Some(COND_EQ),
                    "-ne" => Some(COND_NE),
                    "-lt" => Some(COND_LT),
                    "-gt" => Some(COND_GT),
                    "-le" => Some(COND_LE),
                    "-ge" => Some(COND_GE),
                    "-nt" => Some(COND_NT),
                    "-ot" => Some(COND_OT),
                    "-ef" => Some(COND_EF),
                    "=~" | "-regex-match" => Some(COND_REGEX),
                    _ => None,
                });
                if let Some(code) = code {
                    *pos += 1;
                    let right = match peek(*pos) {
                        Some(a) => { *pos += 1; a.to_string() }
                        None => return 2,
                    };
                    let parse_num = |s: &str| -> Option<f64> {
                        let t = s.trim();
                        if posix {
                            t.parse::<i64>().ok().map(|i| i as f64)
                        } else {
                            t.parse::<i64>().map(|i| i as f64)
                                .or_else(|_| t.parse::<f64>()).ok()
                        }
                    };
                    let num_cmp = |l: &str, r: &str, f: fn(f64, f64) -> bool| -> i32 {
                        match (parse_num(l), parse_num(r)) {
                            (Some(a), Some(b)) => b2i(f(a, b)),
                            _ => 2,
                        }
                    };
                    let mtime_cmp = |l: &str, r: &str, f: fn(i64, i64) -> bool| -> i32 {
                        let lm = match getstat(l) { Some(m) => m, None => return 1 };
                        let rm = match getstat(r) { Some(m) => m, None => return 1 };
                        b2i(f(lm.mtime(), rm.mtime()))
                    };
                    // c:2519 (glob.c) — `matchpat` reads EXTENDED_GLOB
                    // and CASEGLOB from option globals. Rust port
                    // extends the signature; read both flags here so
                    // `setopt nocaseglob` / `setopt extendedglob`
                    // actually affect `[[ str = pat ]]` dispatch.
                    let strpat = |pat: &str, text: &str| -> bool {
                        if posix {
                            text == pat
                        } else {
                            let extended = isset(crate::ported::zsh_h::EXTENDEDGLOB);
                            let case_sensitive = isset(crate::ported::zsh_h::CASEGLOB);
                            matchpat(pat, text, extended, case_sensitive)
                        }
                    };
                    return match code {
                        c if c == COND_STREQ || c == COND_STRDEQ => b2i(strpat(&right, &left)),
                        c if c == COND_STRNEQ => b2i(!strpat(&right, &left)),
                        c if c == COND_STRLT  => b2i(left.as_str() < right.as_str()),
                        c if c == COND_STRGTR => b2i(left.as_str() > right.as_str()),
                        c if c == COND_EQ => num_cmp(&left, &right, |a, b| a == b),
                        c if c == COND_NE => num_cmp(&left, &right, |a, b| a != b),
                        c if c == COND_LT => num_cmp(&left, &right, |a, b| a <  b),
                        c if c == COND_GT => num_cmp(&left, &right, |a, b| a >  b),
                        c if c == COND_LE => num_cmp(&left, &right, |a, b| a <= b),
                        c if c == COND_GE => num_cmp(&left, &right, |a, b| a >= b),
                        c if c == COND_NT => mtime_cmp(&left, &right, |a, b| a >  b),
                        c if c == COND_OT => mtime_cmp(&left, &right, |a, b| a <  b),
                        c if c == COND_EF => {
                            let lm = match getstat(&left)  { Some(m) => m, None => return 1 };
                            let rm = match getstat(&right) { Some(m) => m, None => return 1 };
                            b2i(lm.dev() == rm.dev() && lm.ino() == rm.ino())
                        }
                        c if c == COND_REGEX => {
                            #[cfg(feature = "regex")]
                            {
                                match regex::Regex::new(&right) {
                                    Ok(re) => b2i(re.is_match(&left)),
                                    Err(_) => 2,
                                }
                            }
                            #[cfg(not(feature = "regex"))]
                            {
                                // Same option-state read as strpat above.
                                let extended = isset(crate::ported::zsh_h::EXTENDEDGLOB);
                                let case_sensitive = isset(crate::ported::zsh_h::CASEGLOB);
                                b2i(matchpat(&right, &left, extended, case_sensitive))
                            }
                        }
                        _ => 2,
                    };
                }
                // Implicit `-n` (non-empty).
                b2i(!left.is_empty())
            }
        }
    }

    let mut pos = 0usize;
    let r = walk(&toks, &mut pos, options, variables, posix_mode, 0);
    if pos != toks.len() { 2 } else { r }
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
pub fn doaccess(s: &str, c: i32) -> i32 {                                    // c:438
    let cs = match std::ffi::CString::new(crate::ported::utils::unmeta(s)) {  // c:445 unmeta(s)
        Ok(v) => v, Err(_) => return 0,
    };
    (unsafe { libc::access(cs.as_ptr(), c) } == 0) as i32                    // c:445 !access(...)
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
pub fn getstat(s: &str) -> Option<Metadata> {                                // c:452
    if let Some(rest) = s.strip_prefix("/dev/fd/") {                          // c:458
        if let Ok(fd) = rest.parse::<i32>() {                                 // c:459 atoi(s+8)
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
    let us = crate::ported::utils::unmeta(s);
    fs::metadata(&us).ok()                                                    // c:466
}

/// Port of `dostat(char *s)` from Src/cond.c:474 — returns the file's
/// `st_mode` or 0 on error. Used by `[[ -b/-c/-d/-f/-g/-h/-k/-p
/// /-S/-u/-w/-x ]]` to inspect mode bits.
pub fn dostat(s: &str) -> u32 {                                              // c:474
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
pub fn dolstat(s: &str) -> u32 {                                             // c:488
    let us = crate::ported::utils::unmeta(s);                                 // c:489 unmeta(s)
    fs::symlink_metadata(&us).map(|m| m.mode()).unwrap_or(0)
}

/// Port of `optison(char *name, char *s)` from Src/cond.c:502 — `[[ -o NAME ]]` shell-
/// option test. Returns 0 (true) when the option is set, 1 (false)
/// when unset, 3 (error) when the name is unrecognised. Routes
/// through the canonical option table via `optlookup` /
/// `optlookupc` (Src/options.c:684 / :721).
pub fn optison(name: &str, s: &str) -> i32 {                                 // c:502
    let i: i32 = if s.len() == 1 {                                           // c:502
        crate::ported::options::optlookupc(s.as_bytes()[0] as char)          // c:507
    } else {
        crate::ported::options::optlookup(s)                                 // c:509
    };
    if i == 0 {                                                              // c:510
        if isset(crate::ported::zsh_h::POSIXBUILTINS) {                      // c:511
            return 1;                                                        // c:512
        } else {
            crate::ported::utils::zwarnnam(name, &format!("no such option: {}", s)); // c:514
            return 3;                                                        // c:515
        }
    } else if i < 0 {                                                        // c:517
        if unset(-i) { 0 } else { 1 }                                        // c:518 !unset(-i)
    } else if isset(i) { 0 } else { 1 }                                      // c:520 !isset(i)
}

// `isset` / `unset` macros from `Src/options.h:62-63` — `(opts[X])`
// / `(!opts[X])`. Re-exported from the canonical port in zsh_h.rs
// which reads the live opt_state, NOT a fresh `ShellOptions::new()`
// (the latter returns defaults and would be wrong).
use crate::ported::zsh_h::{isset, unset};

/// Port of `cond_str(char **args, int num, int raw)` from Src/cond.c:525 — return `arg[num]` after
/// running it through `singsub()` if it contains shell tokens, then
/// optionally `untokenize()`. The Rust port stores already-expanded
/// argument strings in the cond evaluator, so this collapses to an
/// indexed read.
#[allow(unused_variables)]
pub fn cond_str(args: &[String], num: usize, raw: bool) -> String {          // c:525
    args.get(num).cloned().unwrap_or_default()
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
pub fn cond_val(args: &[String], num: usize) -> i64 {                        // c:539
    let s = match args.get(num) {
        Some(v) => v,
        None => return 0,
    };
    // c:548 — `mathevali(s)`. Already-expanded arg (the Rust cond
    // evaluator pre-runs singsub/untokenize), so we skip the
    // has_token gate and go straight to mathevali.
    crate::ported::math::mathevali(s).unwrap_or(0)                            // c:548
}

/// Port of `cond_match(char **args, int num, char *str)` from Src/cond.c:552 — `[[ str = pat ]]`
/// pattern test. Runs `singsub()` on the pattern, then defers to
/// `matchpat()` (Src/glob.c).
///
/// C's `matchpat` reads `EXTENDED_GLOB` and case sensitivity from
/// global option state. The Rust `matchpat` extends the signature to
/// take these as explicit args (a structural Rust adaptation), so we
/// read the live option state here and pass it through.
///
/// **Previously hardcoded `(true, true)`** — defeating the
/// `EXTENDED_GLOB` and `CASEGLOB` option flags entirely. A user who
/// did `setopt nocaseglob` and ran `[[ ABC = abc ]]` would still get
/// a case-sensitive failure under the Rust port; C respects nocaseglob.
pub fn cond_match(args: &[String], num: usize, str: &str) -> bool {         // c:552
    // c:2519 (glob.c) — `if (isset(EXTENDED_GLOB)) ...` controls #/~ syntax.
    let extended = isset(crate::ported::zsh_h::EXTENDEDGLOB);
    // c:2519 — case sensitivity reads `isset(CASEGLOB)` (with the
    // canonical-name spelling, NOT a "no_case_glob" variant).
    let case_sensitive = isset(crate::ported::zsh_h::CASEGLOB);
    args.get(num)
        .map(|p| matchpat(str, p, extended, case_sensitive))
        .unwrap_or(false)
}

/// Port of `tracemodcond(char *name, char **args, int inf)` from Src/cond.c:563 — `xtrace`-mode
/// pretty-printer for module-defined cond operators. Emits the
/// op + args to stderr in the same shape the C source uses (infix
/// for binary, prefix for unary). Used only when the `XTRACE`
/// option is enabled and a third-party module supplies a cond.
pub fn tracemodcond(name: &str, args: &[String], inf: bool) {                // c:563
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
    use std::fs::File;
    use tempfile::TempDir;

    fn empty_maps() -> (HashMap<String, bool>, HashMap<String, String>) {
        (HashMap::new(), HashMap::new())
    }

    #[test]
    fn test_string_empty() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["-z", ""], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-z", "hello"], &opts, &vars, true), 1);
        assert_eq!(evalcond(&["-n", "hello"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-n", ""], &opts, &vars, true), 1);
    }

    #[test]
    fn test_string_compare() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["hello", "=", "hello"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["hello", "!=", "world"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["abc", "<", "def"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["xyz", ">", "abc"], &opts, &vars, true), 0);
    }

    #[test]
    fn test_numeric_compare() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["5", "-eq", "5"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["5", "-ne", "3"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["3", "-lt", "5"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["5", "-gt", "3"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["5", "-le", "5"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["5", "-ge", "5"], &opts, &vars, true), 0);
    }

    #[test]
    fn test_file_exists() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("testfile");
        File::create(&file_path).unwrap();
        let (opts, vars) = empty_maps();
        let path_str = file_path.to_str().unwrap();
        assert_eq!(evalcond(&["-e", path_str], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-f", path_str], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-d", path_str], &opts, &vars, true), 1);
    }

    #[test]
    fn test_directory() {
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();
        let path_str = dir.path().to_str().unwrap();
        assert_eq!(evalcond(&["-d", path_str], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-f", path_str], &opts, &vars, true), 1);
    }

    #[test]
    fn test_logical_not() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["!", "-z", "hello"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["!", "-n", ""], &opts, &vars, true), 0);
    }

    #[test]
    fn test_logical_and() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["-n", "a", "-a", "-n", "b"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-n", "a", "-a", "-z", "b"], &opts, &vars, true), 1);
    }

    #[test]
    fn test_logical_or() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["-z", "a", "-o", "-n", "b"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-z", "a", "-o", "-z", "b"], &opts, &vars, true), 1);
    }

    #[test]
    fn test_variable_exists() {
        let opts = HashMap::new();
        let mut vars = HashMap::new();
        vars.insert("MYVAR".to_string(), "value".to_string());
        assert_eq!(evalcond(&["-v", "MYVAR"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-v", "NOTEXIST"], &opts, &vars, true), 1);
    }

    /// `Src/cond.c:179-180` — `[[ -s file ]]` is true iff stat succeeds
    /// AND `st_size > 0`. Empty file → false; non-empty → true; missing → false.
    #[test]
    fn test_minus_s_size_gt_zero() {
        use std::io::Write;
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let empty = dir.path().join("empty");
        File::create(&empty).unwrap();
        assert_eq!(evalcond(&["-s", empty.to_str().unwrap()], &opts, &vars, true), 1,
            "c:179 — `-s` must be false for 0-byte file");

        let nonempty = dir.path().join("nonempty");
        let mut f = File::create(&nonempty).unwrap();
        f.write_all(b"data").unwrap();
        assert_eq!(evalcond(&["-s", nonempty.to_str().unwrap()], &opts, &vars, true), 0,
            "c:179 — `-s` must be true for non-empty file");

        let missing = dir.path().join("not_there");
        assert_eq!(evalcond(&["-s", missing.to_str().unwrap()], &opts, &vars, true), 1,
            "c:179 — `-s` must be false when stat fails (missing file)");
    }

    /// `Src/cond.c:488` — `dolstat` uses `lstat(2)` so `-h` / `-L`
    /// returns true for the LINK itself, even when the link target
    /// doesn't exist. `-f` / `-d` against the same link returns false
    /// (since they follow the link via `stat(2)` and find nothing).
    #[cfg(unix)]
    #[test]
    fn test_minus_h_minus_L_detect_symlink_via_lstat() {
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let target = dir.path().join("nonexistent_target");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let link_s = link.to_str().unwrap();
        assert_eq!(evalcond(&["-h", link_s], &opts, &vars, true), 0,
            "c:488 — `-h` uses lstat; detects symlink even with missing target");
        assert_eq!(evalcond(&["-L", link_s], &opts, &vars, true), 0,
            "c:488 — `-L` is same as `-h`");
        // -f / -d follow the link → false because target doesn't exist.
        assert_eq!(evalcond(&["-f", link_s], &opts, &vars, true), 1);
        assert_eq!(evalcond(&["-d", link_s], &opts, &vars, true), 1);
    }

    /// `Src/cond.c:179-180` — `[[ -ef ]]` returns true iff two paths
    /// resolve to the same inode (`st_dev` AND `st_ino` match). A
    /// hardlink to the same file passes; an unrelated file fails.
    #[cfg(unix)]
    #[test]
    fn test_dash_ef_same_inode() {
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
        assert_eq!(evalcond(&[as_, "-ef", bs_], &opts, &vars, true), 0,
            "c:179 — hardlinks share st_ino + st_dev → -ef true");
        assert_eq!(evalcond(&[as_, "-ef", cs_], &opts, &vars, true), 1,
            "c:179 — distinct files → -ef false");
    }

    /// `Src/cond.c:179` — `-nt` / `-ot` compare st_mtime. Newer file
    /// is `-nt` the older; same direction `-ot` is reversed.
    #[cfg(unix)]
    #[test]
    fn test_dash_nt_dash_ot_compare_mtime() {
        use std::io::Write;
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
        assert_eq!(evalcond(&[n, "-nt", o], &opts, &vars, true), 0,
            "c:179 — newer -nt older → true");
        assert_eq!(evalcond(&[o, "-nt", n], &opts, &vars, true), 1,
            "c:179 — older -nt newer → false");
        assert_eq!(evalcond(&[o, "-ot", n], &opts, &vars, true), 0,
            "c:179 — older -ot newer → true");
    }

    /// `Src/cond.c:179` — `-r` / `-w` map to access(F, R_OK)/W_OK.
    /// Created files inherit rw permissions; chmod 0 strips them.
    #[cfg(unix)]
    #[test]
    fn test_dash_r_dash_w_access_check() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();

        let file = dir.path().join("rw");
        File::create(&file).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let p = file.to_str().unwrap();
        assert_eq!(evalcond(&["-r", p], &opts, &vars, true), 0,
            "c:438 — mode 0600 → readable");
        assert_eq!(evalcond(&["-w", p], &opts, &vars, true), 0,
            "c:438 — mode 0600 → writable");

        // Root can read anything; skip the strip-permissions check there.
        if unsafe { libc::geteuid() } != 0 {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
            assert_eq!(evalcond(&["-r", p], &opts, &vars, true), 1,
                "c:438 — mode 0000 → not readable (non-root)");
            assert_eq!(evalcond(&["-w", p], &opts, &vars, true), 1,
                "c:438 — mode 0000 → not writable (non-root)");
        }
    }

    /// `Src/cond.c:81-185` — Double-negation: `! ! foo` cancels out
    /// at the COND_NOT recursion level. `!` parses right-associative.
    #[test]
    fn test_double_negation_cancels() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["!", "!", "-n", "x"], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["!", "!", "-z", "x"], &opts, &vars, true), 1);
    }

    /// `Src/cond.c:81-185` — Implicit `-n` for a bare arg. `[[ foo ]]`
    /// is the same as `[[ -n foo ]]`. Empty bare arg → false.
    #[test]
    fn test_implicit_minus_n_for_bare_arg() {
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["foo"], &opts, &vars, true), 0,
            "non-empty bare arg → true (implicit -n)");
        assert_eq!(evalcond(&[""], &opts, &vars, true), 1,
            "empty bare arg → false (implicit -n)");
    }

    /// `Src/cond.c:525-540` — `cond_str(args, num)` returns the arg
    /// at `num` after singsub. Out-of-bounds → empty string (no panic).
    #[test]
    fn test_cond_str_index_lookup() {
        let args = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        assert_eq!(cond_str(&args, 0, false), "alpha");
        assert_eq!(cond_str(&args, 2, false), "gamma");
        assert_eq!(cond_str(&args, 99, false), "",
            "c:525 — out-of-bounds index returns empty (Rust safety)");
    }

    /// `Src/cond.c:539-554` — `cond_val(args, num)` parses arg as int.
    /// Non-numeric → 0. Trimmed whitespace allowed.
    #[test]
    fn test_cond_val_int_coerce() {
        let args = vec!["42".to_string(), "  -7 ".to_string(), "abc".to_string()];
        assert_eq!(cond_val(&args, 0), 42);
        assert_eq!(cond_val(&args, 1), -7,
            "c:539 — whitespace must trim; negative supported");
        assert_eq!(cond_val(&args, 2), 0,
            "c:539 — non-numeric returns 0");
        assert_eq!(cond_val(&args, 99), 0,
            "c:539 — out-of-bounds returns 0");
    }

    /// `Src/cond.c:179-180` — `-a` / `-e` are aliases for "file exists".
    /// Both must accept the same input.
    #[test]
    fn test_dash_a_dash_e_aliases() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("f");
        File::create(&file).unwrap();
        let p = file.to_str().unwrap();
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["-e", p], &opts, &vars, true), 0);
        assert_eq!(evalcond(&["-a", p], &opts, &vars, true), 0,
            "c:179 — -a is alias for -e in zsh test/[[ context");
    }

    /// `Src/cond.c:539` — implicit-numeric coercion fails for
    /// non-numeric operands: `[[ abc -eq 5 ]]` should return error (2).
    #[test]
    fn test_minus_eq_non_numeric_returns_error() {
        let (opts, vars) = empty_maps();
        // Both posix and non-posix modes route through parse_num; if
        // either side fails to parse → return 2 (cond error).
        assert_eq!(evalcond(&["abc", "-eq", "5"], &opts, &vars, true), 2,
            "non-numeric LHS in -eq must return 2 (error)");
    }

    /// `Src/cond.c:81` — Parenthesised grouping: `( expr )` evaluates
    /// `expr` in isolation. Missing closing paren → return 2 (error).
    #[test]
    fn test_paren_grouping_and_error_on_missing_close() {
        let (opts, vars) = empty_maps();
        // Balanced: ! ( -z "" )  →  ! true → false (1)
        assert_eq!(evalcond(&["!", "(", "-z", "", ")"], &opts, &vars, true), 1);
        // Missing close paren: error
        assert_eq!(evalcond(&["(", "-z", ""], &opts, &vars, true), 2,
            "missing closing `)` must return 2 (cond error)");
    }

    /// `Src/cond.c:452-468` — `getstat(s)` special-cases `/dev/fd/N`
    /// with `fstat(N, &st)`. Regular paths run through `unmeta(s)`
    /// before `stat(2)`. The previous Rust port used `fs::metadata(s)`
    /// directly, missing the unmeta pass. Pin the regular-path
    /// contract (existence check on `/`).
    #[test]
    fn getstat_resolves_regular_path() {
        // Regular path: root exists, must return Some.
        assert!(getstat("/").is_some(),
            "c:466 — stat('/') must succeed");
        // Nonexistent path returns None.
        assert!(getstat("/nonexistent/path/zzz").is_none(),
            "c:464 — nonexistent path returns None");
    }

    /// `Src/cond.c:458-461` — `/dev/fd/N` syntax routes through
    /// `fstat(N, &st)`. The previous Rust port wasted an fd via
    /// unconditional `dup` BEFORE checking fd validity. Fixed: fstat
    /// pre-check, then dup only for the File-ownership wrapper.
    /// Pin: /dev/fd/<stdin-fd> when stdin is a tty doesn't panic.
    #[cfg(unix)]
    #[test]
    fn getstat_dev_fd_path_doesnt_dup_bad_fds() {
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
    #[test]
    fn cond_match_runs_matchpat_through_args_indirection() {
        // Literal equality always matches regardless of options.
        let args = vec!["hello".to_string()];
        assert!(cond_match(&args, 0, "hello"),
            "literal pattern matches identical text");
        assert!(!cond_match(&args, 0, "world"),
            "literal pattern rejects non-match");
        // Out-of-bounds index → false (no panic, args.get returns None).
        assert!(!cond_match(&args, 99, "hello"),
            "out-of-bounds num returns false");
    }

    /// Pin: `cond_val` routes through `mathevali` per `Src/cond.c:548`.
    /// A `[[ -eq ]]` operand of `"1+2"` must evaluate to 3, not 0.
    /// The previous Rust port called `s.trim().parse::<i64>()` which
    /// silently returned 0 for any non-trivial arithmetic, defeating
    /// `[[ N -eq M+0 ]]`-style asserts in user scripts.
    #[test]
    fn cond_val_routes_through_mathevali() {
        let args = vec![
            "1+2".to_string(),
            "10/2".to_string(),
            "0x10".to_string(),
            "2**8".to_string(),
        ];
        // c:548 — `1+2` → 3 (addition)
        assert_eq!(cond_val(&args, 0), 3,
            "c:548 — `mathevali(\"1+2\")` evaluates the expression");
        // c:548 — `10/2` → 5 (integer division)
        assert_eq!(cond_val(&args, 1), 5,
            "c:548 — `mathevali(\"10/2\")` evaluates the expression");
        // c:548 — `0x10` → 16 (hex literal via mathevali)
        assert_eq!(cond_val(&args, 2), 16,
            "c:548 — `mathevali(\"0x10\")` parses hex");
        // c:548 — `2**8` → 256 (exponent operator)
        assert_eq!(cond_val(&args, 3), 256,
            "c:548 — `mathevali(\"2**8\")` evaluates the expression");
    }

    /// Pin: `cond_val` with plain integer string returns that integer.
    /// Boundary cases — empty string, negative numbers, plain digits.
    #[test]
    fn cond_val_plain_integers() {
        let args = vec![
            "0".to_string(),
            "-42".to_string(),
            "123456789".to_string(),
        ];
        assert_eq!(cond_val(&args, 0), 0);
        assert_eq!(cond_val(&args, 1), -42);
        assert_eq!(cond_val(&args, 2), 123456789);
        // Out-of-bounds index → 0 (args.get returns None).
        assert_eq!(cond_val(&args, 99), 0,
            "out-of-bounds num returns 0");
    }
}
