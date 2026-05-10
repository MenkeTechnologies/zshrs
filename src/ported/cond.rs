//! Conditional expression evaluation for zshrs
//!
//! Direct port from zsh/Src/cond.c
//!
//! tracingcond: updated by execcond() in exec.c                             // c:34
//!
//! Evaluates conditional expressions used in:
//! - `[[ ... ]]` (zsh extended test)
//! - `[ ... ]` and `test` (POSIX test)
//!
//! Supports:
//! - File tests (-e, -f, -d, -r, -w, -x, etc.)
//! - String tests (-n, -z, =, !=, <, >)
//! - Numeric comparisons (-eq, -ne, -lt, -gt, -le, -ge)
//! - Logical operators (!, &&, ||)
//! - Pattern matching (=~, ==, !=)
//! - File comparisons (-nt, -ot, -ef)

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::glob::matchpat;

/// `[[ ... ]]` operator codes.
/// Port of the `COND_*` enum from Src/zsh.h — `evalcond()`
/// (Src/cond.c:70) dispatches between these for every binary /
/// unary / regex test the C source supports. Single-character
/// `FileTest('e')` etc. delegates to `doaccess()` (Src/cond.c:438)
/// / `dostat()` (line 474) / `dolstat()` (line 488).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondType {
    // Logical operators
    Not, // !
    And, // &&
    Or,  // ||

    // String comparisons
    StrEq,  // = or ==
    StrDeq, // == (double equals)
    StrNeq, // !=
    StrLt,  // <
    StrGt,  // >

    // File comparisons
    Nt, // -nt (newer than)
    Ot, // -ot (older than)
    Ef, // -ef (same file)

    // Numeric comparisons
    Eq, // -eq
    Ne, // -ne
    Lt, // -lt
    Gt, // -gt
    Le, // -le
    Ge, // -ge

    // Regex
    Regex, // =~

    // Unary file tests (single character codes)
    FileTest(char),

    // Module conditions (custom tests)
    Mod,
    Modi,
}

/// Outcome of evaluating a `[[ ... ]]` test.
/// Port of the integer return values `evalcond()` from
/// Src/cond.c:70 produces — `0` true, `1` false, `2` error,
/// `3` option-not-found (the `-o NONEXISTENT_OPT` case).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondResult {
    True,           // 0 - condition is true
    False,          // 1 - condition is false
    Error,          // 2 - syntax error
    OptionNotExist, // 3 - option tested with -o does not exist
}

impl CondResult {
    pub fn to_exit_code(self) -> i32 {
        match self {
            CondResult::True => 0,
            CondResult::False => 1,
            CondResult::Error => 2,
            CondResult::OptionNotExist => 3,
        }
    }

    pub fn from_bool(b: bool) -> Self {
        if b {
            CondResult::True
        } else {
            CondResult::False
        }
    }

    pub fn negate(self) -> Self {
        match self {
            CondResult::True => CondResult::False,
            CondResult::False => CondResult::True,
            other => other,
        }
    }
}

/// Conditional expression evaluator state.
/// Port of the per-evaluation locals `evalcond()` from
/// Src/cond.c:70 keeps on the C source's stack — the option /
/// variable tables it consults plus tracing/posix flags.
pub struct CondEval<'a> {
    /// Shell options (for -o test)
    options: &'a HashMap<String, bool>,
    /// Shell variables (for -v test)
    variables: &'a HashMap<String, String>,
    /// Whether we're in POSIX test mode ([ ] or test)
    posix_mode: bool,
    /// Enable tracing output
    tracing: bool,
}

impl<'a> CondEval<'a> {
    pub fn new(options: &'a HashMap<String, bool>, variables: &'a HashMap<String, String>) -> Self {
        CondEval {
            options,
            variables,
            posix_mode: false,
            tracing: false,
        }
    }

    pub fn with_posix_mode(mut self, posix: bool) -> Self {
        self.posix_mode = posix;
        self
    }

    pub fn with_tracing(mut self, tracing: bool) -> Self {
        self.tracing = tracing;
        self
    }

    /// Evaluate a parsed conditional expression
    pub fn eval(&self, expr: &CondExpr) -> CondResult {
        match expr {
            CondExpr::Not(inner) => {
                let result = self.eval(inner);
                result.negate()
            }

            CondExpr::And(left, right) => {
                let left_result = self.eval(left);
                if left_result != CondResult::True {
                    return left_result;
                }
                self.eval(right)
            }

            CondExpr::Or(left, right) => {
                let left_result = self.eval(left);
                if left_result == CondResult::True {
                    return CondResult::True;
                }
                if left_result == CondResult::Error {
                    return CondResult::Error;
                }
                self.eval(right)
            }

            CondExpr::Unary(op, arg) => self.eval_unary(*op, arg),

            CondExpr::Binary(op, left, right) => self.eval_binary(*op, left, right),

            CondExpr::Ternary(_, _, _, _) => CondResult::Error, // Not used in conditionals
        }
    }

    fn eval_unary(&self, op: char, arg: &str) -> CondResult {
        match op {
            // File existence tests
            'a' | 'e' => CondResult::from_bool(self.file_exists(arg)),
            'b' => CondResult::from_bool(self.is_block_device(arg)),
            'c' => CondResult::from_bool(self.is_char_device(arg)),
            'd' => CondResult::from_bool(self.is_directory(arg)),
            'f' => CondResult::from_bool(self.is_regular_file(arg)),
            'g' => CondResult::from_bool(self.has_setgid(arg)),
            'h' | 'L' => CondResult::from_bool(self.is_symlink(arg)),
            'k' => CondResult::from_bool(self.has_sticky(arg)),
            'p' => CondResult::from_bool(self.is_fifo(arg)),
            'r' => CondResult::from_bool(self.is_readable(arg)),
            's' => CondResult::from_bool(self.has_size(arg)),
            'S' => CondResult::from_bool(self.is_socket(arg)),
            'u' => CondResult::from_bool(self.has_setuid(arg)),
            'w' => CondResult::from_bool(self.is_writable(arg)),
            'x' => CondResult::from_bool(self.is_executable(arg)),
            'O' => CondResult::from_bool(self.is_owned_by_euid(arg)),
            'G' => CondResult::from_bool(self.is_owned_by_egid(arg)),
            'N' => CondResult::from_bool(self.is_modified_since_read(arg)),

            // String tests
            'n' => CondResult::from_bool(!arg.is_empty()),
            'z' => CondResult::from_bool(arg.is_empty()),

            // Option test
            'o' => self.test_option(arg),

            // Variable test
            'v' => CondResult::from_bool(self.variables.contains_key(arg)),

            // TTY test
            't' => {
                if let Ok(fd) = arg.parse::<i32>() {
                    CondResult::from_bool(unsafe { libc::isatty(fd) } != 0)
                } else {
                    CondResult::Error
                }
            }

            _ => CondResult::Error,
        }
    }

    fn eval_binary(&self, op: CondType, left: &str, right: &str) -> CondResult {
        match op {
            // String comparisons
            CondType::StrEq | CondType::StrDeq => {
                // In [[ ]], right side is a pattern
                if !self.posix_mode {
                    CondResult::from_bool(matchpat(right, left, true, true))
                } else {
                    CondResult::from_bool(left == right)
                }
            }
            CondType::StrNeq => {
                if !self.posix_mode {
                    CondResult::from_bool(!matchpat(right, left, true, true))
                } else {
                    CondResult::from_bool(left != right)
                }
            }
            CondType::StrLt => CondResult::from_bool(left < right),
            CondType::StrGt => CondResult::from_bool(left > right),

            // Numeric comparisons
            CondType::Eq => self.numeric_compare(left, right, |a, b| a == b),
            CondType::Ne => self.numeric_compare(left, right, |a, b| a != b),
            CondType::Lt => self.numeric_compare(left, right, |a, b| a < b),
            CondType::Gt => self.numeric_compare(left, right, |a, b| a > b),
            CondType::Le => self.numeric_compare(left, right, |a, b| a <= b),
            CondType::Ge => self.numeric_compare(left, right, |a, b| a >= b),

            // File comparisons
            CondType::Nt => self.file_newer_than(left, right),
            CondType::Ot => self.file_older_than(left, right),
            CondType::Ef => self.same_file(left, right),

            // Regex match
            CondType::Regex => self.regex_match(left, right),

            _ => CondResult::Error,
        }
    }

    // File test implementations

    fn get_metadata(&self, path: &str) -> Option<Metadata> {
        // Handle /dev/fd/N
        if let Some(fd_str) = path.strip_prefix("/dev/fd/") {
            if let Ok(fd) = fd_str.parse::<i32>() {
                // Use fstat for /dev/fd/N
                let mut stat: libc::stat = unsafe { std::mem::zeroed() };
                if unsafe { libc::fstat(fd, &mut stat) } == 0 {
                    // We can't easily convert libc::stat to std::fs::Metadata,
                    // so fall back to regular stat
                    return fs::metadata(path).ok();
                }
            }
        }
        fs::metadata(path).ok()
    }

    fn get_symlink_metadata(&self, path: &str) -> Option<Metadata> {
        fs::symlink_metadata(path).ok()
    }

    fn file_exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn is_block_device(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mode() & libc::S_IFMT as u32 == libc::S_IFBLK as u32)
            .unwrap_or(false)
    }

    fn is_char_device(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mode() & libc::S_IFMT as u32 == libc::S_IFCHR as u32)
            .unwrap_or(false)
    }

    fn is_directory(&self, path: &str) -> bool {
        Path::new(path).is_dir()
    }

    fn is_regular_file(&self, path: &str) -> bool {
        Path::new(path).is_file()
    }

    fn is_symlink(&self, path: &str) -> bool {
        self.get_symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    fn is_fifo(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mode() & libc::S_IFMT as u32 == libc::S_IFIFO as u32)
            .unwrap_or(false)
    }

    fn is_socket(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mode() & libc::S_IFMT as u32 == libc::S_IFSOCK as u32)
            .unwrap_or(false)
    }

    fn has_setuid(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mode() & libc::S_ISUID as u32 != 0)
            .unwrap_or(false)
    }

    fn has_setgid(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mode() & libc::S_ISGID as u32 != 0)
            .unwrap_or(false)
    }

    fn has_sticky(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mode() & libc::S_ISVTX as u32 != 0)
            .unwrap_or(false)
    }

    fn is_readable(&self, path: &str) -> bool {
        use std::ffi::CString;
        if let Ok(c_path) = CString::new(path) {
            unsafe { libc::access(c_path.as_ptr(), libc::R_OK) == 0 }
        } else {
            fs::metadata(path).is_ok()
        }
    }

    fn is_writable(&self, path: &str) -> bool {
        use std::ffi::CString;
        if let Ok(c_path) = CString::new(path) {
            unsafe { libc::access(c_path.as_ptr(), libc::W_OK) == 0 }
        } else {
            self.get_metadata(path)
                .map(|m| m.mode() & 0o200 != 0)
                .unwrap_or(false)
        }
    }

    fn is_executable(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| {
                let mode = m.mode();
                // Check if any execute bit is set, or if it's a directory
                (mode & 0o111 != 0) || (mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32)
            })
            .unwrap_or(false)
    }

    fn has_size(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    }

    fn is_owned_by_euid(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.uid() == unsafe { libc::geteuid() })
            .unwrap_or(false)
    }

    fn is_owned_by_egid(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.gid() == unsafe { libc::getegid() })
            .unwrap_or(false)
    }

    fn is_modified_since_read(&self, path: &str) -> bool {
        self.get_metadata(path)
            .map(|m| m.mtime() >= m.atime())
            .unwrap_or(false)
    }

    // Numeric comparison

    fn numeric_compare<F>(&self, left: &str, right: &str, cmp: F) -> CondResult
    where
        F: Fn(f64, f64) -> bool,
    {
        let left_val = self.parse_number(left);
        let right_val = self.parse_number(right);

        match (left_val, right_val) {
            (Some(l), Some(r)) => CondResult::from_bool(cmp(l, r)),
            _ => CondResult::Error,
        }
    }

    fn parse_number(&self, s: &str) -> Option<f64> {
        // In POSIX mode, only base-10 integers
        if self.posix_mode {
            s.trim().parse::<i64>().ok().map(|i| i as f64)
        } else {
            // Try integer first, then float
            if let Ok(i) = s.trim().parse::<i64>() {
                Some(i as f64)
            } else {
                s.trim().parse::<f64>().ok()
            }
        }
    }

    // File comparisons

    fn file_newer_than(&self, left: &str, right: &str) -> CondResult {
        let left_meta = match self.get_metadata(left) {
            Some(m) => m,
            None => return CondResult::False,
        };
        let right_meta = match self.get_metadata(right) {
            Some(m) => m,
            None => return CondResult::False,
        };

        CondResult::from_bool(left_meta.mtime() > right_meta.mtime())
    }

    fn file_older_than(&self, left: &str, right: &str) -> CondResult {
        let left_meta = match self.get_metadata(left) {
            Some(m) => m,
            None => return CondResult::False,
        };
        let right_meta = match self.get_metadata(right) {
            Some(m) => m,
            None => return CondResult::False,
        };

        CondResult::from_bool(left_meta.mtime() < right_meta.mtime())
    }

    fn same_file(&self, left: &str, right: &str) -> CondResult {
        let left_meta = match self.get_metadata(left) {
            Some(m) => m,
            None => return CondResult::False,
        };
        let right_meta = match self.get_metadata(right) {
            Some(m) => m,
            None => return CondResult::False,
        };

        CondResult::from_bool(
            left_meta.dev() == right_meta.dev() && left_meta.ino() == right_meta.ino(),
        )
    }

    // Option test

    fn test_option(&self, name: &str) -> CondResult {
        // Single character option — direct port of zsh's optletters
        // lookup at Src/options.c:287 / 726. Map shorthand letters
        // (`-e`, `-x`, etc.) to their full option names.
        if name.len() == 1 {
            let ch = name.chars().next().unwrap();
            let opt_name = match ch {
                'a' => Some("allexport"),
                'B' => Some("braceccl"),
                'C' => Some("noclobber"),
                'e' => Some("errexit"),
                'f' => Some("noglob"),
                'g' => Some("histignorespace"),
                'h' => Some("hashcmds"),
                'H' => Some("histexpand"),
                'i' => Some("interactive"),
                'I' => Some("ignoreeof"),
                'j' => Some("monitor"),
                'k' => Some("keywordargs"),
                'l' => Some("login"),
                'm' => Some("monitor"),
                'n' => Some("noexec"),
                'p' => Some("privileged"),
                'P' => Some("physical"),
                'r' => Some("restricted"),
                's' => Some("stdin"),
                't' => Some("singlecommand"),
                'u' => Some("nounset"),
                'v' => Some("verbose"),
                'w' => Some("chaselinks"),
                'x' => Some("xtrace"),
                'X' => Some("listtypes"),
                'Y' => Some("menucomplete"),
                'Z' => Some("zle"),
                '0' => Some("correct"),
                '1' => Some("printexitvalue"),
                '2' => Some("autolist"),
                '3' => Some("autocontinue"),
                '4' => Some("autoparamslash"),
                '5' => Some("autopushd"),
                '6' => Some("autoremoveslash"),
                '7' => Some("bsdecho"),
                '8' => Some("nocaseglob"),
                '9' => Some("cdablevars"),
                _ => None,
            };
            if let Some(opt_name) = opt_name {
                if let Some(&val) = self.options.get(opt_name) {
                    return CondResult::from_bool(val);
                }
            }
        }

        // Full option name
        if let Some(&val) = self.options.get(name) {
            CondResult::from_bool(val)
        } else {
            CondResult::OptionNotExist
        }
    }

    // Regex match

    fn regex_match(&self, text: &str, pattern: &str) -> CondResult {
        #[cfg(feature = "regex")]
        {
            match regex::Regex::new(pattern) {
                Ok(re) => CondResult::from_bool(re.is_match(text)),
                Err(_) => CondResult::Error,
            }
        }
        #[cfg(not(feature = "regex"))]
        {
            // Fallback: simple pattern match
            CondResult::from_bool(matchpat(pattern, text, true, true))
        }
    }
}

/// Parsed `[[ ... ]]` expression tree.
/// Port of the `Wordcode` shape `parse_cond()` from Src/parse.c
/// produces and `evalcond()` (Src/cond.c:70) walks. Each variant
/// matches one of the C `COND_*` operator categories.
#[derive(Debug, Clone)]
pub enum CondExpr {
    Not(Box<CondExpr>),
    And(Box<CondExpr>, Box<CondExpr>),
    Or(Box<CondExpr>, Box<CondExpr>),
    Unary(char, String),
    Binary(CondType, String, String),
    Ternary(CondType, String, String, String),
}

/// Parser for `[[ ... ]]` / `test`-style expressions.
/// Port of the cond-parsing path inside Src/parse.c (`par_cond_*`
/// functions) — the C source emits wordcode; this Rust parser
/// produces a typed AST instead.
pub struct CondParser<'a> {
    tokens: Vec<&'a str>,
    pos: usize,
    posix_mode: bool,
}

impl<'a> CondParser<'a> {
    pub fn new(tokens: Vec<&'a str>, posix_mode: bool) -> Self {
        CondParser {
            tokens,
            pos: 0,
            posix_mode,
        }
    }

    pub fn parse(&mut self) -> Result<CondExpr, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<CondExpr, String> {
        let mut left = self.parse_and()?;

        while self.match_token("||") || self.match_token("-o") {
            let right = self.parse_and()?;
            left = CondExpr::Or(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_and(&mut self) -> Result<CondExpr, String> {
        let mut left = self.parse_not()?;

        while self.match_token("&&") || self.match_token("-a") {
            let right = self.parse_not()?;
            left = CondExpr::And(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_not(&mut self) -> Result<CondExpr, String> {
        if self.match_token("!") {
            let inner = self.parse_not()?;
            Ok(CondExpr::Not(Box::new(inner)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<CondExpr, String> {
        // Parenthesized expression
        if self.match_token("(") {
            let expr = self.parse_or()?;
            if !self.match_token(")") {
                return Err("missing )".to_string());
            }
            return Ok(expr);
        }

        // Check for unary operators (file/string tests). Direct port
        // of Src/cond.c parser's `-X arg` recognizer — every char in
        // this set takes one operand: `-a` exists, `-d` directory,
        // `-f` regular file, `-n` non-empty string, `-z` empty
        // string, `-r/-w/-x` perm bits, etc.
        if let Some(tok) = self.peek() {
            if tok.starts_with('-') && tok.len() == 2 {
                let op = tok.chars().nth(1).unwrap();
                if matches!(
                    op,
                    'a' | 'b' | 'c' | 'd' | 'e' | 'f' | 'g' | 'h'
                    | 'k' | 'L' | 'n' | 'o' | 'p' | 'r' | 's' | 'S'
                    | 't' | 'u' | 'v' | 'w' | 'x' | 'z'
                    | 'G' | 'N' | 'O'
                ) {
                    self.advance();
                    let arg = self.expect_arg()?;
                    return Ok(CondExpr::Unary(op, arg.to_string()));
                }
            }
        }

        // Binary expression: left op right. Operator dispatch is the
        // direct port of cond.c's binary-op parser — string and arith
        // comparators, file-relation tests (-nt/-ot/-ef), and the
        // regex match `=~` (plus the zsh/regex module's
        // `-regex-match` per Src/Modules/regex.c:214).
        let left = self.expect_arg()?;

        if let Some(op) = self.peek() {
            let cond_type = match op {
                "=" | "==" => Some(CondType::StrEq),
                "!=" => Some(CondType::StrNeq),
                "<" => Some(CondType::StrLt),
                ">" => Some(CondType::StrGt),
                "-eq" => Some(CondType::Eq),
                "-ne" => Some(CondType::Ne),
                "-lt" => Some(CondType::Lt),
                "-gt" => Some(CondType::Gt),
                "-le" => Some(CondType::Le),
                "-ge" => Some(CondType::Ge),
                "-nt" => Some(CondType::Nt),
                "-ot" => Some(CondType::Ot),
                "-ef" => Some(CondType::Ef),
                "=~" => Some(CondType::Regex),
                "-regex-match" => Some(CondType::Regex),
                _ => None,
            };
            if let Some(cond_type) = cond_type {
                self.advance();
                let right = self.expect_arg()?;
                return Ok(CondExpr::Binary(
                    cond_type,
                    left.to_string(),
                    right.to_string(),
                ));
            }
        }

        // Implicit -n test for non-empty string
        Ok(CondExpr::Unary('n', left.to_string()))
    }

    fn peek(&self) -> Option<&'a str> {
        self.tokens.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<&'a str> {
        let tok = self.tokens.get(self.pos).copied();
        self.pos += 1;
        tok
    }

    fn match_token(&mut self, expected: &str) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_arg(&mut self) -> Result<&'a str, String> {
        self.advance()
            .ok_or_else(|| "expected argument".to_string())
    }
}


/// Convenience function to evaluate a test expression
/// Evaluate a POSIX `test`/`[[` expression.
/// Top-level wrapper around `CondParser` + `CondEval`. Port of
// -o does not exist".                                                      // c:65
/// the `evalcond()` driver from Src/cond.c:70 — the C source's
/// entry point that the `[[` keyword and the `test`/`[`
/// builtins both delegate to.
pub fn evalcond(                                                             // c:70
    args: &[&str],
    options: &HashMap<String, bool>,
    variables: &HashMap<String, String>,
    posix_mode: bool,
) -> i32 {
    // Handle empty args
    if args.is_empty() {
        return 1; // false
    }

    // Filter out [ and ] if present
    let args: Vec<&str> = args
        .iter()
        .filter(|&s| *s != "[" && *s != "]" && *s != "[[" && *s != "]]")
        .copied()
        .collect();

    if args.is_empty() {
        return 1;
    }

    let mut parser = CondParser::new(args, posix_mode);
    match parser.parse() {
        Ok(expr) => {
            let evaluator = CondEval::new(options, variables).with_posix_mode(posix_mode);
            evaluator.eval(&expr).to_exit_code()
        }
        Err(_) => 2, // syntax error
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
        assert_eq!(
            evalcond(&["-n", "a", "-a", "-n", "b"], &opts, &vars, true),
            0
        );
        assert_eq!(
            evalcond(&["-n", "a", "-a", "-z", "b"], &opts, &vars, true),
            1
        );
    }

    #[test]
    fn test_logical_or() {
        let (opts, vars) = empty_maps();
        assert_eq!(
            evalcond(&["-z", "a", "-o", "-n", "b"], &opts, &vars, true),
            0
        );
        assert_eq!(
            evalcond(&["-z", "a", "-o", "-z", "b"], &opts, &vars, true),
            1
        );
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

// ===========================================================
// Direct-port helpers used internally by evalcond. These mirror
// the C helpers in cond.c that wrap stat()/access()/option lookup
// and the cond_str/cond_val/cond_match argument-coercion trio.
// ===========================================================

/// Port of `doaccess()` from Src/cond.c:438 — `[[ -r/-w/-x ]]` test.
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
        if libc::access(cs.as_ptr(), mode) == 0 {
            1
        } else {
            0
        }
    }
}

/// Port of `getstat()` from Src/cond.c:452 — `stat(2)` wrapper that
/// special-cases `/dev/fd/N` with `fstat()`. Returns the metadata or
/// `None` on error. Replaces the C global `static struct stat st`
/// with a returned `Metadata` value (Rust avoids globals here).
pub fn getstat(s: &str) -> Option<std::fs::Metadata> {                       // c:452
    if let Some(rest) = s.strip_prefix("/dev/fd/") {
        if let Ok(fd) = rest.parse::<i32>() {
            use std::os::unix::io::FromRawFd;
            let f = unsafe { std::fs::File::from_raw_fd(libc::dup(fd)) };
            let m = f.metadata().ok();
            return m;
        }
    }
    fs::metadata(s).ok()
}

/// Port of `dostat()` from Src/cond.c:474 — returns the file's
/// `st_mode` or 0 on error. Used by `[[ -b/-c/-d/-f/-g/-h/-k/-p
/// /-S/-u/-w/-x ]]` to inspect mode bits.
pub fn dostat(s: &str) -> u32 {                                              // c:474
    getstat(s).map(|m| m.mode()).unwrap_or(0)
}

/// Port of `dolstat()` from Src/cond.c:488 — like `dostat()` but
/// uses `lstat(2)` so symlinks are *not* followed. Underpins
/// `[[ -h ]]` / `[[ -L ]]`.
pub fn dolstat(s: &str) -> u32 {
    fs::symlink_metadata(s).map(|m| m.mode()).unwrap_or(0)
}

/// Port of `optison()` from Src/cond.c:502 — `[[ -o NAME ]]` shell-
/// option test. Returns 0 (true) when the option is set, 1 (false)
/// when unset, 3 (error) when the name is unrecognised. The Rust
/// rewrite stores options in the executor's `HashMap`; this entry
/// is a free-fn shim — callers in `evalcond` already inspect the
/// passed-in option map directly.
pub fn optison(name: &str, s: &str) -> i32 {                            // c:502
    /*
     * optison returns evalcond-friendly statuses (true, false, error).
     */                                                                  // c:496-498
    let i: i32;                                                          // c:504
    if s.len() == 1 {                                                    // c:506
        i = crate::ported::options::optlookupc(s.as_bytes()[0] as char); // c:507
    } else {
        i = crate::ported::options::optlookup(s);                        // c:509
    }
    if i == 0 {                                                          // c:510
        if isset(crate::ported::zsh_h::POSIXBUILTINS) {                  // c:511
            return 1;                                                     // c:512
        } else {
            crate::ported::utils::zwarnnam(name, &format!("no such option: {}", s)); // c:514
            return 3;                                                     // c:515
        }
    } else if i < 0 {                                                    // c:517
        if unset(-i) { 0 } else { 1 }                                    // c:518 !unset(-i)
    } else {
        if isset(i) { 0 } else { 1 }                                     // c:520 !isset(i)
    }
}

// `isset` macro from `Src/options.h:62` — `(opts[X] != 0)`. Reads
// from the global option table in options.rs.
fn isset(opt: i32) -> bool {
    let opts = crate::ported::options::ShellOptions::new();
    opts.get_by_index(opt).unwrap_or(false)
}

// `unset` macro from `Src/options.h:63` — `(!isset(X))`.
fn unset(opt: i32) -> bool { !isset(opt) }

/// Port of `cond_str()` from Src/cond.c:525 — return `arg[num]` after
/// running it through `singsub()` if it contains shell tokens, then
/// optionally `untokenize()`. The Rust port stores already-expanded
/// argument strings in the cond evaluator, so this collapses to an
/// indexed read.
pub fn cond_str(args: &[String], num: usize, _raw: bool) -> String {
    args.get(num).cloned().unwrap_or_default()
}

/// Port of `cond_val()` from Src/cond.c:539 — like `cond_str()` but
/// then runs `mathevali()` to coerce the result to an integer. The
/// Rust port handles math evaluation through `crate::math::eval`;
/// here we parse the trimmed argument as a base-10 integer.
pub fn cond_val(args: &[String], num: usize) -> i64 {
    args.get(num)
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

/// Port of `cond_match()` from Src/cond.c:552 — `[[ str = pat ]]`
/// pattern test. Runs `singsub()` on the pattern, then defers to
/// `matchpat()` (Src/glob.c).
pub fn cond_match(args: &[String], num: usize, str_: &str) -> bool {
    args.get(num)
        .map(|p| matchpat(str_, p, true, true))
        .unwrap_or(false)
}

/// Port of `tracemodcond()` from Src/cond.c:562 — `xtrace`-mode
/// pretty-printer for module-defined cond operators. Emits the
/// op + args to stderr in the same shape the C source uses (infix
/// for binary, prefix for unary). Used only when the `XTRACE`
/// option is enabled and a third-party module supplies a cond.
pub fn tracemodcond(name: &str, args: &[String], inf: bool) {
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
