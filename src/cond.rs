//! Conditional expression evaluation for zshrs
//!
//! Direct port from zsh/Src/cond.c
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

use crate::glob::pattern_match;

/// Condition type codes matching zsh's COND_* constants
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

/// Result of condition evaluation
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

/// Conditional expression evaluator
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
                    CondResult::from_bool(pattern_match(right, left, true, true))
                } else {
                    CondResult::from_bool(left == right)
                }
            }
            CondType::StrNeq => {
                if !self.posix_mode {
                    CondResult::from_bool(!pattern_match(right, left, true, true))
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
        // Single character option
        if name.len() == 1 {
            let ch = name.chars().next().unwrap();
            if let Some(opt_name) = short_option_name(ch) {
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
            CondResult::from_bool(pattern_match(pattern, text, true, true))
        }
    }
}

/// Map single-character option codes to option names
fn short_option_name(c: char) -> Option<&'static str> {
    Some(match c {
        'a' => "allexport",
        'B' => "braceccl",
        'C' => "noclobber",
        'e' => "errexit",
        'f' => "noglob",
        'g' => "histignorespace",
        'h' => "hashcmds",
        'H' => "histexpand",
        'i' => "interactive",
        'I' => "ignoreeof",
        'j' => "monitor",
        'k' => "keywordargs",
        'l' => "login",
        'm' => "monitor",
        'n' => "noexec",
        'p' => "privileged",
        'P' => "physical",
        'r' => "restricted",
        's' => "stdin",
        't' => "singlecommand",
        'u' => "nounset",
        'v' => "verbose",
        'w' => "chaselinks",
        'x' => "xtrace",
        'X' => "listtypes",
        'Y' => "menucomplete",
        'Z' => "zle",
        '0' => "correct",
        '1' => "printexitvalue",
        '2' => "autolist",
        '3' => "autocontinue",
        '4' => "autoparamslash",
        '5' => "autopushd",
        '6' => "autoremoveslash",
        '7' => "bsdecho",
        '8' => "nocaseglob",
        '9' => "cdablevars",
        _ => return None,
    })
}

/// Parsed conditional expression
#[derive(Debug, Clone)]
pub enum CondExpr {
    Not(Box<CondExpr>),
    And(Box<CondExpr>, Box<CondExpr>),
    Or(Box<CondExpr>, Box<CondExpr>),
    Unary(char, String),
    Binary(CondType, String, String),
    Ternary(CondType, String, String, String),
}

/// Parser for conditional expressions
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

        // Check for unary operators
        if let Some(tok) = self.peek() {
            if tok.starts_with('-') && tok.len() == 2 {
                let op = tok.chars().nth(1).unwrap();
                // Check if this is a unary file/string test
                if is_unary_op(op) {
                    self.advance();
                    let arg = self.expect_arg()?;
                    return Ok(CondExpr::Unary(op, arg.to_string()));
                }
            }
        }

        // Binary expression: left op right
        let left = self.expect_arg()?;

        if let Some(op) = self.peek() {
            if let Some(cond_type) = parse_binary_op(op) {
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

fn is_unary_op(c: char) -> bool {
    matches!(
        c,
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
    )
}

fn parse_binary_op(s: &str) -> Option<CondType> {
    Some(match s {
        "=" | "==" => CondType::StrEq,
        "!=" => CondType::StrNeq,
        "<" => CondType::StrLt,
        ">" => CondType::StrGt,
        "-eq" => CondType::Eq,
        "-ne" => CondType::Ne,
        "-lt" => CondType::Lt,
        "-gt" => CondType::Gt,
        "-le" => CondType::Le,
        "-ge" => CondType::Ge,
        "-nt" => CondType::Nt,
        "-ot" => CondType::Ot,
        "-ef" => CondType::Ef,
        "=~" => CondType::Regex,
        _ => return None,
    })
}

/// Convenience function to evaluate a test expression
pub fn eval_test(
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
        assert_eq!(eval_test(&["-z", ""], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["-z", "hello"], &opts, &vars, true), 1);
        assert_eq!(eval_test(&["-n", "hello"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["-n", ""], &opts, &vars, true), 1);
    }

    #[test]
    fn test_string_compare() {
        let (opts, vars) = empty_maps();
        assert_eq!(eval_test(&["hello", "=", "hello"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["hello", "!=", "world"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["abc", "<", "def"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["xyz", ">", "abc"], &opts, &vars, true), 0);
    }

    #[test]
    fn test_numeric_compare() {
        let (opts, vars) = empty_maps();
        assert_eq!(eval_test(&["5", "-eq", "5"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["5", "-ne", "3"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["3", "-lt", "5"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["5", "-gt", "3"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["5", "-le", "5"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["5", "-ge", "5"], &opts, &vars, true), 0);
    }

    #[test]
    fn test_file_exists() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("testfile");
        File::create(&file_path).unwrap();

        let (opts, vars) = empty_maps();
        let path_str = file_path.to_str().unwrap();

        assert_eq!(eval_test(&["-e", path_str], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["-f", path_str], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["-d", path_str], &opts, &vars, true), 1);
    }

    #[test]
    fn test_directory() {
        let dir = TempDir::new().unwrap();
        let (opts, vars) = empty_maps();
        let path_str = dir.path().to_str().unwrap();

        assert_eq!(eval_test(&["-d", path_str], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["-f", path_str], &opts, &vars, true), 1);
    }

    #[test]
    fn test_logical_not() {
        let (opts, vars) = empty_maps();
        assert_eq!(eval_test(&["!", "-z", "hello"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["!", "-n", ""], &opts, &vars, true), 0);
    }

    #[test]
    fn test_logical_and() {
        let (opts, vars) = empty_maps();
        assert_eq!(
            eval_test(&["-n", "a", "-a", "-n", "b"], &opts, &vars, true),
            0
        );
        assert_eq!(
            eval_test(&["-n", "a", "-a", "-z", "b"], &opts, &vars, true),
            1
        );
    }

    #[test]
    fn test_logical_or() {
        let (opts, vars) = empty_maps();
        assert_eq!(
            eval_test(&["-z", "a", "-o", "-n", "b"], &opts, &vars, true),
            0
        );
        assert_eq!(
            eval_test(&["-z", "a", "-o", "-z", "b"], &opts, &vars, true),
            1
        );
    }

    #[test]
    fn test_variable_exists() {
        let opts = HashMap::new();
        let mut vars = HashMap::new();
        vars.insert("MYVAR".to_string(), "value".to_string());

        assert_eq!(eval_test(&["-v", "MYVAR"], &opts, &vars, true), 0);
        assert_eq!(eval_test(&["-v", "NOTEXIST"], &opts, &vars, true), 1);
    }
}
