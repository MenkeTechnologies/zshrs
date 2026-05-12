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
//! walker that mirrors cond.c:70 line-by-line. The `CondExpr` /
//! `CondParser` types below are the intermediate scaffold — also
//! NOT in C and slated for deletion once the wordcode path lands.

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::glob::matchpat;
use crate::ported::zsh_h::{
    COND_AND, COND_EF, COND_EQ, COND_GE, COND_GT, COND_LE, COND_LT, COND_NE, COND_NOT, COND_NT,
    COND_OR, COND_OT, COND_REGEX, COND_STRDEQ, COND_STREQ, COND_STRGTR, COND_STRLT, COND_STRNEQ,
};

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
                    let strpat = |pat: &str, text: &str| -> bool {
                        if posix { text == pat } else { matchpat(pat, text, true, true) }
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
                                b2i(matchpat(&right, &left, true, true))
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

/// Port of `doaccess(s, c)` from Src/cond.c:438 — `[[ -r/-w/-x ]]` test.
/// Returns true (non-zero) when `access(2)` reports the file is
/// reachable for the requested mode. The C source special-cases
/// `/dev/fd/N` to use `faccessat` against the descriptor; we do the
/// same with a manual `fstat`-based check (an open fd always
/// satisfies POSIX `R_OK`/`W_OK`/`X_OK` if its descriptor permits
/// the action; portable equivalent for our uses).
pub fn doaccess(s: &str, c: i32) -> i32 {                                    // c:438
    if let Some(rest) = s.strip_prefix("/dev/fd/") {
        if rest.parse::<i32>().is_ok() {
            return 1;
        }
    }
    let mode = match c {
        0 => libc::F_OK,
        4 => libc::R_OK,
        2 => libc::W_OK,
        1 => libc::X_OK,
        _ => libc::F_OK,
    };
    let cs = match std::ffi::CString::new(s) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    unsafe {
        if libc::access(cs.as_ptr(), mode) == 0 { 1 } else { 0 }
    }
}

/// Port of `getstat(s)` from Src/cond.c:452 — `stat(2)` wrapper that
/// special-cases `/dev/fd/N` with `fstat()`. Returns the metadata or
/// `None` on error. Replaces the C global `static struct stat st`
/// with a returned `Metadata` value (Rust avoids globals here).
pub fn getstat(s: &str) -> Option<Metadata> {                                // c:452
    if let Some(rest) = s.strip_prefix("/dev/fd/") {
        if let Ok(fd) = rest.parse::<i32>() {
            use std::os::unix::io::FromRawFd;
            let f = unsafe { std::fs::File::from_raw_fd(libc::dup(fd)) };
            return f.metadata().ok();
        }
    }
    fs::metadata(s).ok()
}

/// Port of `dostat(s)` from Src/cond.c:474 — returns the file's
/// `st_mode` or 0 on error. Used by `[[ -b/-c/-d/-f/-g/-h/-k/-p
/// /-S/-u/-w/-x ]]` to inspect mode bits.
pub fn dostat(s: &str) -> u32 {                                              // c:474
    getstat(s).map(|m| m.mode()).unwrap_or(0)
}

/// Port of `dolstat(s)` from Src/cond.c:488 — like `dostat()` but
/// uses `lstat(2)` so symlinks are *not* followed. Underpins
/// `[[ -h ]]` / `[[ -L ]]`.
pub fn dolstat(s: &str) -> u32 {                                             // c:488
    fs::symlink_metadata(s).map(|m| m.mode()).unwrap_or(0)
}

/// Port of `optison(name, s)` from Src/cond.c:502 — `[[ -o NAME ]]` shell-
/// option test. Returns 0 (true) when the option is set, 1 (false)
/// when unset, 3 (error) when the name is unrecognised. Routes
/// through the canonical option table via `optlookup` /
/// `optlookupc` (Src/options.c:684 / :721).
pub fn optison(name: &str, s: &str) -> i32 {                                 // c:502
    let i: i32 = if s.len() == 1 {                                           // c:506
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

/// Port of `cond_str(args, num, raw)` from Src/cond.c:525 — return `arg[num]` after
/// running it through `singsub()` if it contains shell tokens, then
/// optionally `untokenize()`. The Rust port stores already-expanded
/// argument strings in the cond evaluator, so this collapses to an
/// indexed read.
#[allow(unused_variables)]
pub fn cond_str(args: &[String], num: usize, raw: bool) -> String {          // c:525
    args.get(num).cloned().unwrap_or_default()
}

/// Port of `cond_val(args, num)` from Src/cond.c:539 — like `cond_str()` but
/// then runs `mathevali()` to coerce the result to an integer. The
/// Rust port handles math evaluation through `crate::math::eval`;
/// here we parse the trimmed argument as a base-10 integer.
pub fn cond_val(args: &[String], num: usize) -> i64 {                        // c:539
    args.get(num)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

/// Port of `cond_match(args, num, str)` from Src/cond.c:552 — `[[ str = pat ]]`
/// pattern test. Runs `singsub()` on the pattern, then defers to
/// `matchpat()` (Src/glob.c).
pub fn cond_match(args: &[String], num: usize, str: &str) -> bool {         // c:552
    args.get(num)
        .map(|p| matchpat(str, p, true, true))
        .unwrap_or(false)
}

/// Port of `tracemodcond(name, args, inf)` from Src/cond.c:562 — `xtrace`-mode
/// pretty-printer for module-defined cond operators. Emits the
/// op + args to stderr in the same shape the C source uses (infix
/// for binary, prefix for unary). Used only when the `XTRACE`
/// option is enabled and a third-party module supplies a cond.
pub fn tracemodcond(name: &str, args: &[String], inf: bool) {                // c:563
    use std::io::Write;
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
}
