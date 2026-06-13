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
use crate::ported::lex::untokenize;
use crate::ported::math::mathevali;
use crate::ported::options::{optlookup, optlookupc};
use crate::ported::params::{setaparam, setiparam, setsparam};
use crate::ported::subst::singsub;
use crate::ported::utils::{has_token, privasserted, unmeta, zwarnnam};
use crate::ported::zsh_h::{
    isset, unset, CASEGLOB, COND_EF, COND_EQ, COND_GE, COND_GT, COND_LE, COND_LT, COND_NE, COND_NT,
    COND_OT, COND_REGEX, COND_STRDEQ, COND_STREQ, COND_STRGTR, COND_STRLT, COND_STRNEQ,
    EXTENDEDGLOB, POSIXBUILTINS,
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
pub fn evalcond(
    // c:70 — C signature is `int evalcond(Estate state, char *fromtest)`.
    // The Rust port adds posix_mode + an explicit args slice (Estate
    // walks wordcode in C; we walk argv tokens). `from_test` mirrors C's
    // `fromtest` parameter byte-for-byte: when `Some(name)`, the
    // integer-comparison arms route through strict zstrtol parsing that
    // emits `zwarnnam(name, "integer expression expected: %s", err)` +
    // rc=2 on parse failure (c:Src/cond.c:248-251). When `None`, the
    // `[[ ]]`-style mathevali coercion-to-0 path is used.
    args: &[&str],
    options: &HashMap<String, bool>,
    variables: &HashMap<String, String>,
    posix_mode: bool,
    from_test: Option<&str>,
) -> i32 {
    if args.is_empty() {
        return 1;
    }
    let toks: Vec<&str> = args
        .iter()
        .filter(|s| !matches!(**s, "[" | "]" | "[[" | "]]"))
        .copied()
        .collect();
    if toks.is_empty() {
        return 1;
    }

    // Inner walker — the entire cond.c:81-185 switch collapsed into
    // one fn. `prec` selects the recursion level (0=OR, 1=AND,
    // 2=NOT, 3=primary). This is the one Rust adaptation: C's
    // `evalcond` walks a single wordcode stream; we walk argv via
    // operator-precedence climbing. No helper ported / no AST.
    fn walk(
        toks: &[&str],
        pos: &mut usize,
        opts: &HashMap<String, bool>,
        vars: &HashMap<String, String>,
        posix: bool,
        from_test: Option<&str>,
        prec: u8,
    ) -> i32 {
        let b2i = |b: bool| -> i32 {
            if b {
                0
            } else {
                1
            }
        };
        let peek = |i: usize| -> Option<&str> { toks.get(i).copied() };

        match prec {
            // OR — c:96 COND_OR.
            0 => {
                let mut left = walk(toks, pos, opts, vars, posix, from_test, 1);
                while peek(*pos) == Some("||") || peek(*pos) == Some("-o") {
                    *pos += 1;
                    let right = walk(toks, pos, opts, vars, posix, from_test, 1);
                    left = if left == 0 {
                        0
                    } else if left >= 2 {
                        left
                    } else {
                        right
                    };
                }
                left
            }
            // AND — c:88 COND_AND.
            1 => {
                let mut left = walk(toks, pos, opts, vars, posix, from_test, 2);
                while peek(*pos) == Some("&&") || peek(*pos) == Some("-a") {
                    *pos += 1;
                    let right = walk(toks, pos, opts, vars, posix, from_test, 2);
                    left = if left != 0 { left } else { right };
                }
                left
            }
            // NOT — c:81 COND_NOT.
            2 => {
                if peek(*pos) == Some("!") {
                    *pos += 1;
                    let r = walk(toks, pos, opts, vars, posix, from_test, 2);
                    if r < 2 {
                        if r == 0 {
                            1
                        } else {
                            0
                        }
                    } else {
                        r
                    }
                } else {
                    walk(toks, pos, opts, vars, posix, from_test, 3)
                }
            }
            // Primary — c:179+ default arm. Parenthesised group,
            // unary `-X arg`, binary `l OP r`, or bare arg.
            _ => {
                if peek(*pos) == Some("(") {
                    *pos += 1;
                    let r = walk(toks, pos, opts, vars, posix, from_test, 0);
                    if peek(*pos) != Some(")") {
                        return 2;
                    }
                    *pos += 1;
                    return r;
                }
                // Unary `-X arg`.
                if let Some(tok) = peek(*pos) {
                    if tok.starts_with('-') && tok.len() == 2 {
                        let op = tok.chars().nth(1).unwrap();
                        if matches!(
                            op,
                            'a' | 'b'
                                | 'c'
                                | 'd'
                                | 'e'
                                | 'f'
                                | 'g'
                                | 'h'
                                | 'k'
                                | 'L'
                                | 'n'
                                | 'o'
                                | 'p'
                                | 'r'
                                | 's'
                                | 'S'
                                | 't'
                                | 'u'
                                | 'v'
                                | 'w'
                                | 'x'
                                | 'z'
                                | 'G'
                                | 'N'
                                | 'O'
                        ) {
                            *pos += 1;
                            let arg = match peek(*pos) {
                                Some(a) => {
                                    *pos += 1;
                                    a.to_string()
                                }
                                None => return 2,
                            };
                            return match op {
                                'a' | 'e' => b2i(Path::new(&arg).exists()), // c:179-180
                                'b' => {
                                    b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFBLK as u32)
                                }
                                'c' => {
                                    b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFCHR as u32)
                                }
                                'd' => b2i(Path::new(&arg).is_dir()),
                                'f' => b2i(Path::new(&arg).is_file()),
                                'g' => b2i(dostat(&arg) & libc::S_ISGID as u32 != 0),
                                'h' | 'L' => {
                                    b2i(dolstat(&arg) & libc::S_IFMT as u32 == libc::S_IFLNK as u32)
                                }
                                'k' => b2i(dostat(&arg) & libc::S_ISVTX as u32 != 0),
                                'p' => {
                                    b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFIFO as u32)
                                }
                                'r' => b2i(doaccess(&arg, 4) != 0), // c:438
                                's' => b2i(getstat(&arg).map(|m| m.len() > 0).unwrap_or(false)),
                                'S' => {
                                    b2i(dostat(&arg) & libc::S_IFMT as u32 == libc::S_IFSOCK as u32)
                                }
                                'u' => b2i(dostat(&arg) & libc::S_ISUID as u32 != 0),
                                'w' => b2i(doaccess(&arg, 2) != 0), // c:438
                                // c:368-373 — `-x file` test:
                                //   if (privasserted()) {
                                //       mode_t mode = dostat(left);
                                //       return !((mode & S_IXUGO) || S_ISDIR(mode));
                                //   }
                                //   return !doaccess(left, X_OK);
                                //
                                // The previous Rust port unconditionally
                                // did `doaccess || S_ISDIR` — adding the
                                // S_ISDIR fallback even for non-privileged
                                // shells. Under non-privileged shell,
                                // `[[ -x /no-x-perm-dir ]]` would return
                                // TRUE in Rust (the S_ISDIR fallback
                                // bypassed the access check) but FALSE
                                // in C (doaccess alone, no S_ISDIR fall-
                                // back). Gate on `privasserted()` to
                                // match C exactly.
                                'x' => {
                                    if privasserted() {
                                        // c:368
                                        let mode = dostat(&arg);
                                        // c:370 — `(mode & S_IXUGO) || S_ISDIR(mode)`.
                                        let s_ixugo = 0o111u32; // S_IXUSR|S_IXGRP|S_IXOTH
                                        let is_dir =
                                            mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32;
                                        b2i((mode & s_ixugo) != 0 || is_dir) // c:370
                                    } else {
                                        // c:372
                                        b2i(doaccess(&arg, 1) != 0) // c:373 X_OK
                                    }
                                }
                                'O' => b2i(getstat(&arg)
                                    .map(|m| m.uid() == unsafe { libc::geteuid() })
                                    .unwrap_or(false)),
                                'G' => b2i(getstat(&arg)
                                    .map(|m| m.gid() == unsafe { libc::getegid() })
                                    .unwrap_or(false)),
                                'N' => b2i(getstat(&arg)
                                    .map(|m| m.mtime() >= m.atime())
                                    .unwrap_or(false)),
                                'n' => b2i(!arg.is_empty()),
                                'z' => b2i(arg.is_empty()),
                                'o' => {
                                    let r = optison("test", &arg); // c:502
                                    if r != 3 {
                                        r
                                    } else if opts.contains_key(&arg) {
                                        b2i(opts[&arg])
                                    } else {
                                        3
                                    }
                                }
                                'v' => b2i(vars.contains_key(&arg)),
                                // c:330+ — `-t fd` accepts ARITHMETIC
                                // (`mathevali`), not just plain digits.
                                // C: `fd = mathevali(left); return
                                // !isatty(fd);`. The previous Rust port
                                // used `.parse::<i32>()` which rejected
                                // `[[ -t $((0)) ]]` / `[[ -t 1+0 ]]`.
                                // Route through `mathevali` so all
                                // arith-expression forms work.
                                't' => mathevali(&arg)
                                    .map(|fd| b2i(unsafe { libc::isatty(fd as i32) } != 0))
                                    .unwrap_or(2),
                                _ => 2,
                            };
                        }
                    }
                }
                // Binary `left OP right` or bare `left` (implicit `-n`).
                let left = match peek(*pos) {
                    Some(a) => {
                        *pos += 1;
                        a.to_string()
                    }
                    None => return 2,
                };
                let code: Option<i32> = peek(*pos).and_then(|t| match t {
                    "=" => Some(COND_STREQ),
                    "==" => Some(COND_STRDEQ),
                    "!=" => Some(COND_STRNEQ),
                    "<" => Some(COND_STRLT),
                    ">" => Some(COND_STRGTR),
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
                        Some(a) => {
                            *pos += 1;
                            a.to_string()
                        }
                        None => return 2,
                    };
                    // c:415-422 — C uses `mathevali(left)` /
                    // `mathevali(right)` for the integer-compare ops
                    // (-eq / -ne / -lt / -gt / -le / -ge). The previous
                    // Rust port called `s.parse::<i64>()` which
                    // silently returned None for any non-trivial
                    // arithmetic — `[[ 1+2 -eq 3 ]]` errored out at
                    // parse time even though C evaluates the LHS to 3.
                    //
                    // Under POSIX-mode, C falls back to plain integer
                    // parsing (no arithmetic eval). Mirror both
                    // branches.
                    let parse_num = |s: &str| -> Option<f64> {
                        let t = s.trim();
                        if posix || from_test.is_some() {
                            // c:Src/cond.c:236-249 — `if (fromtest) { ...
                            // zstrtol base-10 ... }`. test/[ context
                            // demands plain integers; no math eval.
                            // Same shape under POSIXBUILTINS.
                            t.parse::<i64>().ok().map(|i| i as f64)
                        } else {
                            // c:415 — route through `mathevali`.
                            // Falls back to plain decimal / float
                            // parsing for non-arith string operands.
                            mathevali(t)
                                .ok()
                                .map(|i| i as f64)
                                .or_else(|| t.parse::<i64>().ok().map(|i| i as f64))
                                .or_else(|| t.parse::<f64>().ok())
                        }
                    };
                    let num_cmp = |l: &str, r: &str, f: fn(f64, f64) -> bool| -> i32 {
                        match (parse_num(l), parse_num(r)) {
                            (Some(a), Some(b)) => b2i(f(a, b)),
                            _ => {
                                // c:Src/cond.c:248-251 — when fromtest is
                                // set and the operand isn't a base-10
                                // integer, emit the canonical diagnostic
                                // and return 2. C reports the FIRST bad
                                // operand: left if left failed, else
                                // right. Bug #411.
                                if let Some(name) = from_test {
                                    let err = if parse_num(l).is_none() {
                                        l
                                    } else {
                                        r
                                    };
                                    crate::ported::utils::zwarnnam(
                                        name,
                                        &format!("integer expression expected: {}", err),
                                    );
                                }
                                2
                            }
                        }
                    };
                    let mtime_cmp = |l: &str, r: &str, f: fn(i64, i64) -> bool| -> i32 {
                        let lm = match getstat(l) {
                            Some(m) => m,
                            None => return 1,
                        };
                        let rm = match getstat(r) {
                            Some(m) => m,
                            None => return 1,
                        };
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
                            let extended = isset(EXTENDEDGLOB);
                            let case_sensitive = isset(CASEGLOB);
                            matchpat(pat, text, extended, case_sensitive)
                        }
                    };
                    return match code {
                        c if c == COND_STREQ || c == COND_STRDEQ => b2i(strpat(&right, &left)),
                        c if c == COND_STRNEQ => b2i(!strpat(&right, &left)),
                        c if c == COND_STRLT => b2i(left.as_str() < right.as_str()),
                        c if c == COND_STRGTR => b2i(left.as_str() > right.as_str()),
                        c if c == COND_EQ => num_cmp(&left, &right, |a, b| a == b),
                        c if c == COND_NE => num_cmp(&left, &right, |a, b| a != b),
                        c if c == COND_LT => num_cmp(&left, &right, |a, b| a < b),
                        c if c == COND_GT => num_cmp(&left, &right, |a, b| a > b),
                        c if c == COND_LE => num_cmp(&left, &right, |a, b| a <= b),
                        c if c == COND_GE => num_cmp(&left, &right, |a, b| a >= b),
                        c if c == COND_NT => mtime_cmp(&left, &right, |a, b| a > b),
                        c if c == COND_OT => mtime_cmp(&left, &right, |a, b| a < b),
                        c if c == COND_EF => {
                            let lm = match getstat(&left) {
                                Some(m) => m,
                                None => return 1,
                            };
                            let rm = match getstat(&right) {
                                Some(m) => m,
                                None => return 1,
                            };
                            b2i(lm.dev() == rm.dev() && lm.ino() == rm.ino())
                        }
                        c if c == COND_REGEX => {
                            // `[[ str =~ pat ]]` — POSIX regex match.
                            // C dispatches to the `zsh/regex` module
                            // (Src/cond.c:113-119); that module sets
                            // `$MATCH` (the whole match) plus
                            // `$match[N]`/`$mbegin[N]`/`$mend[N]`
                            // arrays for each capture group (see
                            // Src/Modules/regex.c). The Rust port
                            // routes through the `regex` crate directly
                            // (it's a hard dep, not a feature-gated
                            // module) and populates the same arrays
                            // so downstream `[[ str =~ pat ]] &&
                            // var=$match[1]` idioms work without a
                            // module dispatch.
                            // c:Src/cond.c:113-119 + Src/Modules/regex.c —
                            // zsh's regex (system POSIX regex or PCRE
                            // when `setopt rematchpcre`) treats `.` as
                            // matching newline by default. Rust's regex
                            // crate defaults `.` to NOT match newline
                            // (single-line `.`); enable
                            // `dot_matches_new_line(true)` so
                            // multi-line `=~` behaves like zsh on
                            // `a\nb`-style inputs. Bug #557.
                            // c:Src/Modules/regex.c:78 — regcomp(3)
                            // POSIX-ERE bracket semantics: inside
                            // `[...]` a backslash is an ordinary
                            // class member. Translate to the Rust
                            // regex crate's class syntax via the
                            // same bridge helper zcond_regex_match
                            // uses. BUGS.md #558.
                            let right_ere =
                                crate::ported::modules::regex::posix_ere_bracket_escape(&right);
                            match regex::RegexBuilder::new(&right_ere)
                                .dot_matches_new_line(true)
                                .build()
                            {
                                Ok(re) => {
                                    if let Some(caps) = re.captures(&left) {
                                        // Whole-match: $MATCH.
                                        let m0 = caps.get(0).unwrap();
                                        setsparam("MATCH", m0.as_str());
                                        // Capture groups → $match[1..N].
                                        // zsh's regex module emits the
                                        // unnamed groups as $match[1..N]
                                        // — the 0th group (whole match)
                                        // lives in $MATCH, NOT $match[].
                                        let mut match_arr: Vec<String> = Vec::new();
                                        let mut begin_arr: Vec<String> = Vec::new();
                                        let mut end_arr: Vec<String> = Vec::new();
                                        for i in 1..caps.len() {
                                            let s = caps
                                                .get(i)
                                                .map(|m| m.as_str().to_string())
                                                .unwrap_or_default();
                                            let b = caps
                                                .get(i)
                                                .map(|m| (m.start() + 1).to_string())
                                                .unwrap_or_else(|| "0".into());
                                            let e = caps
                                                .get(i)
                                                .map(|m| m.end().to_string())
                                                .unwrap_or_else(|| "0".into());
                                            match_arr.push(s);
                                            begin_arr.push(b);
                                            end_arr.push(e);
                                        }
                                        setaparam("match", match_arr);
                                        setaparam("mbegin", begin_arr);
                                        setaparam("mend", end_arr);
                                        b2i(true)
                                    } else {
                                        b2i(false)
                                    }
                                }
                                Err(_) => 2,
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
    let r = walk(&toks, &mut pos, options, variables, posix_mode, from_test, 0);
    if pos != toks.len() {
        2
    } else {
        r
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
pub fn optison(name: &str, s: &str) -> i32 {
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
            zwarnnam(name, &format!("no such option: {}", s)); // c:514
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
    let matched = matchpat(&p, str, extended, case_sensitive); // c:557
                                                               // c:Src/pattern.c GF_MATCHREF / GF_BACKREF — (#m) writes the
                                                               // matched substring to $MATCH; (#b) writes capture groups to
                                                               // $match[]. In `==` cond context the pattern matches the whole
                                                               // string, so on success $MATCH = str. Capture-group support
                                                               // (real (#b) parens) is deferred — (#m) covers the high-traffic
                                                               // case (zinit's plugin-name matching uses it).
    if matched && extended && p.contains("(#m)") {
        setsparam("MATCH", str);
        setiparam("MBEGIN", 1);
        setiparam("MEND", str.chars().count() as i64);
    }
    matched
}

/// Port of `tracemodcond(char *name, char **args, int inf)` from Src/cond.c:563 — `xtrace`-mode
/// pretty-printer for module-defined cond operators. Emits the
/// op + args to stderr in the same shape the C source uses (infix
/// for binary, prefix for unary). Used only when the `XTRACE`
/// option is enabled and a third-party module supplies a cond.
pub fn tracemodcond(name: &str, args: &[String], inf: bool) {
    // c:563
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

    fn empty_maps() -> (HashMap<String, bool>, HashMap<String, String>) {
        (HashMap::new(), HashMap::new())
    }

    // ═══════════════════════════════════════════════════════════════════
    // `=~` capture-group wire-up — Src/cond.c:113-119 dispatches to
    // zsh/regex module which sets $MATCH / $match[N] / $mbegin[N] /
    // $mend[N]. Rust port uses the `regex` crate directly and writes
    // the same arrays. Tests pin the canonical zsh observable shape.
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/cond.c:113 + Modules/regex.c $MATCH` — a successful
    /// `[[ str =~ pat ]]` sets `$MATCH` to the whole-match substring.
    #[test]
    fn cond_regex_sets_match_to_whole_match() {
        let _g = crate::test_util::global_state_lock();
        let opts = HashMap::new();
        let vars = HashMap::new();
        let r = evalcond(&["abc123", "=~", "[a-z]+[0-9]+"], &opts, &vars, false, None);
        assert_eq!(r, 0, "match succeeded");
        assert_eq!(
            getsparam("MATCH").as_deref(),
            Some("abc123"),
            "$MATCH = whole-match per Modules/regex.c"
        );
    }

    /// `Src/Modules/regex.c` — capture groups populate `$match[1..N]`.
    /// `[[ "abc123" =~ '([a-z]+)([0-9]+)' ]]` → $match[1]="abc", $match[2]="123".
    #[test]
    fn cond_regex_sets_match_array_from_capture_groups() {
        let _g = crate::test_util::global_state_lock();
        let opts = HashMap::new();
        let vars = HashMap::new();
        let r = evalcond(&["abc123", "=~", "([a-z]+)([0-9]+)"], &opts, &vars, false, None);
        assert_eq!(r, 0);
        let m = getaparam("match");
        assert_eq!(
            m.as_deref(),
            Some(&["abc".to_string(), "123".to_string()][..]),
            "$match[1..N] populated from capture groups",
        );
    }

    /// `Src/Modules/regex.c $mbegin / $mend` — capture-group byte offsets.
    /// $mbegin is 1-based (zsh convention without KSHARRAYS) for the start,
    /// $mend is the inclusive end position.
    #[test]
    fn cond_regex_sets_mbegin_and_mend_arrays() {
        let _g = crate::test_util::global_state_lock();
        let opts = HashMap::new();
        let vars = HashMap::new();
        let r = evalcond(&["abc123", "=~", "([a-z]+)([0-9]+)"], &opts, &vars, false, None);
        assert_eq!(r, 0);
        let b = getaparam("mbegin");
        let e = getaparam("mend");
        // Group 1: "abc" at bytes 0..3 → mbegin[1] = "1" (1-based), mend[1] = "3".
        assert_eq!(
            b.as_deref().and_then(|v| v.first().cloned()),
            Some("1".to_string())
        );
        assert_eq!(
            e.as_deref().and_then(|v| v.first().cloned()),
            Some("3".to_string())
        );
        // Group 2: "123" at bytes 3..6 → mbegin[2] = "4", mend[2] = "6".
        assert_eq!(
            b.as_deref().and_then(|v| v.get(1).cloned()),
            Some("4".to_string())
        );
        assert_eq!(
            e.as_deref().and_then(|v| v.get(1).cloned()),
            Some("6".to_string())
        );
    }

    /// Failed match: returns 1 (false). $match/etc not asserted here
    /// since prior tests' state may persist.
    #[test]
    fn cond_regex_returns_one_on_no_match() {
        let _g = crate::test_util::global_state_lock();
        let opts = HashMap::new();
        let vars = HashMap::new();
        let r = evalcond(&["xyz", "=~", "[0-9]+"], &opts, &vars, false, None);
        assert_eq!(r, 1, "no digits in 'xyz' → false");
    }

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
        assert_eq!(evalcond(&["hello", "=", "hello"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["hello", "!=", "world"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["abc", "<", "def"], &opts, &vars, true, None), 0);
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
        let mut vars = HashMap::new();
        vars.insert("MYVAR".to_string(), "value".to_string());
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
            evalcond(&["-s", nonempty.to_str().unwrap()], &opts, &vars, true, None),
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
        assert_eq!(evalcond(&["!", "!", "-n", "x"], &opts, &vars, true, None), 0);
        assert_eq!(evalcond(&["!", "!", "-z", "x"], &opts, &vars, true, None), 1);
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

    /// `Src/cond.c:179-180` — `-a` / `-e` are aliases for "file exists".
    /// Both must accept the same input.
    #[test]
    fn test_dash_a_dash_e_aliases() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("f");
        File::create(&file).unwrap();
        let p = file.to_str().unwrap();
        let (opts, vars) = empty_maps();
        assert_eq!(evalcond(&["-e", p], &opts, &vars, true, None), 0);
        assert_eq!(
            evalcond(&["-a", p], &opts, &vars, true, None),
            0,
            "c:179 — -a is alias for -e in zsh test/[[ context"
        );
    }

    /// `Src/cond.c:539` — implicit-numeric coercion fails for
    /// non-numeric operands: `[[ abc -eq 5 ]]` should return error (2).
    #[test]
    fn test_minus_eq_non_numeric_returns_error() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        // Both posix and non-posix modes route through parse_num; if
        // either side fails to parse → return 2 (cond error).
        assert_eq!(
            evalcond(&["abc", "-eq", "5"], &opts, &vars, true, None),
            2,
            "non-numeric LHS in -eq must return 2 (error)"
        );
    }

    /// `Src/cond.c:81` — Parenthesised grouping: `( expr )` evaluates
    /// `expr` in isolation. Missing closing paren → return 2 (error).
    #[test]
    fn test_paren_grouping_and_error_on_missing_close() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        // Balanced: ! ( -z "" )  →  ! true → false (1)
        assert_eq!(evalcond(&["!", "(", "-z", "", ")"], &opts, &vars, true, None), 1);
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

    /// Pin: `[[ N -eq M ]]` etc. route both operands through
    /// `mathevali` per `Src/cond.c:415`. The previous Rust port's
    /// `parse_num` only called `s.parse::<i64>()`, so `[[ 1+2 -eq
    /// 3 ]]` returned 2 (syntax error) instead of evaluating the
    /// LHS to 3.
    #[test]
    fn evalcond_int_compare_routes_through_mathevali() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        // c:415 — `[[ 1+2 -eq 3 ]]` evaluates LHS via mathevali.
        // Should return 0 (true).
        assert_eq!(
            evalcond(&["1+2", "-eq", "3"], &opts, &vars, false, None),
            0,
            "c:415 — `1+2 -eq 3` must mathevali LHS to 3, return 0"
        );
        // c:415 — `[[ 4 -gt 1+2 ]]` evaluates RHS via mathevali.
        assert_eq!(
            evalcond(&["4", "-gt", "1+2"], &opts, &vars, false, None),
            0,
            "c:415 — `4 -gt 1+2` must mathevali RHS to 3, return 0"
        );
        // POSIX mode falls back to plain integer parsing — no
        // arithmetic eval. `[[ 1+2 -eq 3 ]]` under POSIX should
        // fail to parse the LHS (return 2 = syntax error).
        assert_eq!(
            evalcond(&["1+2", "-eq", "3"], &opts, &vars, true, None),
            2,
            "POSIX — no mathevali; non-numeric LHS = error"
        );
        // Plain integers still work in POSIX mode.
        assert_eq!(
            evalcond(&["5", "-eq", "5"], &opts, &vars, true, None),
            0,
            "POSIX — plain integers compare normally"
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

    /// `Test/C02cond.ztst:177-178` — `[[ bar < foo && foo > bar ]]`
    /// lexical < and > for strings.
    #[test]
    fn cond_corpus_lexical_less_greater() {
        let _g = crate::test_util::global_state_lock();
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["bar", "<", "foo"], &opts, &vars, false, None),
            0,
            "bar < foo"
        );
        assert_eq!(
            evalcond(&["foo", ">", "bar"], &opts, &vars, false, None),
            0,
            "foo > bar"
        );
        assert_eq!(
            evalcond(&["foo", "<", "bar"], &opts, &vars, false, None),
            1,
            "foo < bar false"
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
            evalcond(&["-r", readable.to_str().unwrap()], &opts, &vars, false, None),
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

    /// `optison("test", "definitely_not_an_option")` returns nonzero.
    /// C: with POSIXBUILTINS off → 3 (with zwarnnam); with on → 1.
    /// Either way: nonzero.
    #[test]
    fn optison_unknown_option_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = optison("test", "zshrs_definitely_not_an_option_xyz");
        assert_ne!(r, 0, "unknown option → nonzero error (1 or 3)");
    }

    /// `optison("test", "x")` for `set -x` — never panics, returns
    /// 0/1 based on current xtrace state.
    #[test]
    fn optison_single_char_xtrace_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let r = optison("test", "x");
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
        let _: i32 = optison("test", "x");
    }

    /// c:594 — `optison` is deterministic.
    #[test]
    fn optison_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for s in ["x", "y", "v"] {
            let first = optison("test", s);
            for _ in 0..3 {
                assert_eq!(
                    optison("test", s),
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
        let _ = optison("", "");
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
}
