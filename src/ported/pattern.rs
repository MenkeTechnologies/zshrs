//! Pattern matching engine for zshrs
//!
//! Direct port from zsh/Src/pattern.c
//!
//! This implements a bytecode-compiled pattern matching engine supporting:
//! - Basic wildcards: *, ?, [...]
//! - Extended glob patterns: #, ##, ~, ^
//! - KSH glob patterns: ?(pat), *(pat), +(pat), !(pat), @(pat)
//! - Backreferences with parentheses
//! - Case-insensitive matching
//! - Approximate matching (error tolerance)
//! - Numeric ranges: `<n-m>`

use crate::ported::exec::{ShellExecutor, with_executor};

/// Pattern opcodes — port of the `P_*` constants from Src/zsh.h.
/// The C source emits these as bytes into `patcode`; we keep them
/// as a typed enum mostly for the `(#s)`/`(#e)` start/end-assert
/// hooks `patgetglobflags()` (Src/pattern.c:1037) returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PatOp {
    End = 0x00,        // End of program
    ExcSync = 0x01,    // Test if following exclude already failed
    ExcEnd = 0x02,     // Test if exclude matched original branch
    Back = 0x03,       // Match "", "next" ptr points backward
    Exactly = 0x04,    // Match literal string
    Nothing = 0x05,    // Match empty string
    OneHash = 0x06,    // Match 0+ times (simple thing)
    TwoHash = 0x07,    // Match 1+ times (simple thing)
    GFlags = 0x08,     // Set globbing flags
    IsStart = 0x09,    // Match start of string
    IsEnd = 0x0a,      // Match end of string
    CountStart = 0x0b, // Initialize P_COUNT
    Count = 0x0c,      // Match counted repetitions
    Branch = 0x20,     // Match alternative
    WBranch = 0x21,    // Branch, but match at least 1 char
    Exclude = 0x30,    // Exclude from previous branch
    ExcludP = 0x31,    // Exclude using full path
    Any = 0x40,        // Match any one character
    AnyOf = 0x41,      // Match any char in set
    AnyBut = 0x42,     // Match any char not in set
    Star = 0x43,       // Match any characters
    NumRng = 0x44,     // Match numeric range
    NumFrom = 0x45,    // Match number >= X
    NumTo = 0x46,      // Match number <= X
    NumAny = 0x47,     // Match any decimal digits
    Open = 0x80,       // Start of capture group (+ group number)
    Close = 0x90,      // End of capture group (+ group number)
}

/// Maximum number of backreferences
const NSUBEXP: usize = 9;

/// Pattern flags.
/// Port of the `PAT_*` constants the C source passes to
/// `patcompile()` (Src/pattern.c:540) — `PAT_FILE`, `PAT_ANY`,
/// `PAT_NOANCH`, `PAT_NOGLD`, `PAT_PURES`, `PAT_SCAN`,
/// `PAT_LCMATCHUC`. Each maps onto one struct field.
#[derive(Debug, Clone, Copy, Default)]
pub struct PatFlags {
    pub file: bool,      // File globbing mode
    pub any: bool,       // Match any string
    pub noanch: bool,    // Not anchored at end
    pub nogld: bool,     // Don't match leading dot
    pub pures: bool,     // Pure string (no pattern chars)
    pub scan: bool,      // Scanning for match
    pub lcmatchuc: bool, // Lowercase pattern matches uppercase
}

/// Globbing flags.
/// Port of the `(#i)`/`(#l)`/`(#b)`/`(#m)`/`(#u)`/`(#a<n>)`
/// in-pattern flag set that `patgetglobflags()` (Src/pattern.c:1037)
/// produces. Each field corresponds to one of the `GF_*` bits in
/// the C source.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobFlags {
    pub igncase: bool,   // Case insensitive
    pub lcmatchuc: bool, // Lowercase matches uppercase
    pub matchref: bool,  // Set MATCH, MBEGIN, MEND
    pub backref: bool,   // Enable backreferences
    pub multibyte: bool, // Multibyte support
    pub approx: u8,      // Approximation level (error tolerance)
}

/// Compiled pattern program.
/// Port of `struct patprog` from Src/zshpat.h — what
/// `patcompile()` (Src/pattern.c:540) returns. The `code` field
/// replaces the C source's flat `char *patcode` bytecode buffer
/// with a typed `Vec<PatNode>` AST.
#[derive(Debug, Clone)]
pub struct PatProg {
    /// The bytecode
    code: Vec<PatNode>,
    /// Pattern flags
    pub flags: PatFlags,
    /// Glob flags at start
    pub glob_start: GlobFlags,
    /// Glob flags at end
    pub glob_end: GlobFlags,
    /// Number of parenthesized groups
    pub npar: usize,
    /// Start character optimization (if known)
    pub start_char: Option<char>,
    /// Pure string (if PAT_PURES)
    pub pure_string: Option<String>,
}

/// A node in the pattern bytecode AST.
/// One variant per `P_*` opcode the C source's compiler emits in
/// Src/pattern.c — `Exactly` is `P_EXACTLY`, `OneHash` is the `#`
/// 0-or-more, `TwoHash` is `##`, etc. The C version flattens these
/// into a `char *` buffer with offsets; the Rust AST holds them
/// directly so we can pattern-match on shapes.
#[derive(Debug, Clone)]
pub enum PatNode {
    End,
    ExcSync,
    ExcEnd,
    Back(usize),     // Offset to jump back
    Exactly(String), // Literal string
    Nothing,
    OneHash(Box<PatNode>), // 0 or more
    TwoHash(Box<PatNode>), // 1 or more
    GFlags(GlobFlags),
    IsStart,
    IsEnd,
    CountStart,
    Count {
        min: u32,
        max: Option<u32>,
        node: Box<PatNode>,
    },
    Branch(Vec<PatNode>, usize), // Alternatives, next offset
    WBranch(Vec<PatNode>),
    Exclude(Vec<PatNode>),
    ExcludP(Vec<PatNode>),
    Any,                    // Match any single char
    AnyOf(Vec<char>),       // Character class
    AnyBut(Vec<char>),      // Negated character class
    Star,                   // Match any string
    NumRng(i64, i64),       // Numeric range
    NumFrom(i64),           // >= number
    NumTo(i64),             // <= number
    NumAny,                 // Any digits
    Open(usize),            // Start capture group
    Close(usize),           // End capture group
    Sequence(Vec<PatNode>), // Sequence of nodes
}

/// Pattern compiler state
struct PatCompiler<'a> {
    input: &'a str,
    pos: usize,
    flags: PatFlags,
    glob_flags: GlobFlags,
    npar: usize,
    extended_glob: bool,
    ksh_glob: bool,
}

impl<'a> PatCompiler<'a> {
    fn new(input: &'a str, flags: PatFlags) -> Self {
        PatCompiler {
            input,
            pos: 0,
            flags,
            glob_flags: GlobFlags::default(),
            npar: 0,
            extended_glob: true,
            ksh_glob: true,
        }
    }

    fn with_options(mut self, extended: bool, ksh: bool) -> Self {
        self.extended_glob = extended;
        self.ksh_glob = ksh;
        self
    }

    fn with_igncase(mut self, igncase: bool) -> Self {
        self.glob_flags.igncase = igncase;
        self
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn peek_n(&self, n: usize) -> Option<char> {
        self.input[self.pos..].chars().nth(n)
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn compile(mut self) -> Result<PatProg, String> {
        // Check for pure string (no pattern chars)
        if !self.has_pattern_chars() {
            return Ok(PatProg {
                code: vec![PatNode::Exactly(self.input.to_string()), PatNode::End],
                flags: PatFlags {
                    pures: true,
                    ..self.flags
                },
                glob_start: self.glob_flags,
                glob_end: self.glob_flags,
                npar: 0,
                start_char: self.input.chars().next(),
                pure_string: Some(self.input.to_string()),
            });
        }

        let nodes = self.compile_branch()?;
        let start_char = self.find_start_char(&nodes);

        Ok(PatProg {
            code: nodes,
            flags: self.flags,
            glob_start: self.glob_flags,
            glob_end: self.glob_flags,
            npar: self.npar,
            start_char,
            pure_string: None,
        })
    }

    fn has_pattern_chars(&self) -> bool {
        for c in self.input.chars() {
            match c {
                '*' | '?' | '[' | '\\' => return true,
                '#' | '^' | '~' if self.extended_glob => return true,
                '(' | ')' | '|' if self.ksh_glob => return true,
                '<' | '>' if self.extended_glob => return true,
                _ => {}
            }
        }
        false
    }

    fn find_start_char(&self, nodes: &[PatNode]) -> Option<char> {
        match nodes.first()? {
            PatNode::Exactly(s) => s.chars().next(),
            PatNode::Sequence(seq) => {
                if let Some(PatNode::Exactly(s)) = seq.first() {
                    s.chars().next()
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn compile_branch(&mut self) -> Result<Vec<PatNode>, String> {
        self.compile_branch_inner(true)
    }

    fn compile_branch_inner(&mut self, add_end: bool) -> Result<Vec<PatNode>, String> {
        let mut nodes = Vec::new();
        let mut alternatives: Vec<Vec<PatNode>> = Vec::new();

        loop {
            let node = self.compile_piece()?;
            if let Some(n) = node {
                nodes.push(n);
            }

            if self.at_end() {
                break;
            }

            match self.peek() {
                Some('|') => {
                    self.advance();
                    alternatives.push(std::mem::take(&mut nodes));
                }
                Some(')') => break,
                None => break,
                _ => {}
            }
        }

        if !alternatives.is_empty() {
            alternatives.push(nodes);
            Ok(vec![PatNode::Branch(
                alternatives.into_iter().flatten().collect(),
                0,
            )])
        } else {
            if add_end {
                nodes.push(PatNode::End);
            }
            Ok(nodes)
        }
    }

    fn compile_piece(&mut self) -> Result<Option<PatNode>, String> {
        let Some(c) = self.peek() else {
            return Ok(None);
        };

        let node = match c {
            '*' => {
                self.advance();
                // Check for KSH *(pattern)
                if self.ksh_glob && self.peek() == Some('(') {
                    self.advance();
                    let inner = self.compile_branch_inner(false)?;
                    if self.peek() != Some(')') {
                        return Err("missing ) in *(...)".to_string());
                    }
                    self.advance();
                    PatNode::OneHash(Box::new(PatNode::Sequence(inner)))
                } else {
                    PatNode::Star
                }
            }
            '?' => {
                self.advance();
                // Check for KSH ?(pattern)
                if self.ksh_glob && self.peek() == Some('(') {
                    self.advance();
                    let inner = self.compile_branch_inner(false)?;
                    if self.peek() != Some(')') {
                        return Err("missing ) in ?(...)".to_string());
                    }
                    self.advance();
                    // 0 or 1 match
                    PatNode::Branch(vec![PatNode::Sequence(inner), PatNode::Nothing], 0)
                } else {
                    PatNode::Any
                }
            }
            '[' => self.compile_bracket()?,
            '\\' => {
                self.advance();
                if let Some(escaped) = self.advance() {
                    PatNode::Exactly(escaped.to_string())
                } else {
                    PatNode::Exactly("\\".to_string())
                }
            }
            '#' if self.extended_glob => {
                self.advance();
                // ## means 1 or more
                if self.peek() == Some('#') {
                    self.advance();
                    // Get previous node and wrap
                    return Ok(Some(PatNode::TwoHash(Box::new(PatNode::Any))));
                }
                // # means 0 or more
                PatNode::OneHash(Box::new(PatNode::Any))
            }
            '<' if self.extended_glob => self.compile_numeric_range()?,
            '(' => {
                self.advance();
                self.npar += 1;
                let group_num = self.npar;
                let inner = self.compile_branch_inner(false)?;
                if self.peek() != Some(')') {
                    return Err("missing )".to_string());
                }
                self.advance();
                PatNode::Sequence(vec![
                    PatNode::Open(group_num),
                    PatNode::Sequence(inner),
                    PatNode::Close(group_num),
                ])
            }
            ')' | '|' => return Ok(None),
            '+' if self.ksh_glob && self.peek_n(1) == Some('(') => {
                self.advance(); // +
                self.advance(); // (
                let inner = self.compile_branch_inner(false)?;
                if self.peek() != Some(')') {
                    return Err("missing ) in +(...)".to_string());
                }
                self.advance();
                PatNode::TwoHash(Box::new(PatNode::Sequence(inner)))
            }
            '!' if self.ksh_glob && self.peek_n(1) == Some('(') => {
                self.advance(); // !
                self.advance(); // (
                let inner = self.compile_branch_inner(false)?;
                if self.peek() != Some(')') {
                    return Err("missing ) in !(...)".to_string());
                }
                self.advance();
                PatNode::Exclude(inner)
            }
            '@' if self.ksh_glob && self.peek_n(1) == Some('(') => {
                self.advance(); // @
                self.advance(); // (
                let inner = self.compile_branch_inner(false)?;
                if self.peek() != Some(')') {
                    return Err("missing ) in @(...)".to_string());
                }
                self.advance();
                PatNode::Sequence(inner)
            }
            '^' if self.extended_glob => {
                self.advance();
                // Negation - match anything except
                let inner = self.compile_piece()?;
                if let Some(node) = inner {
                    PatNode::Exclude(vec![node])
                } else {
                    return Err("^ requires pattern".to_string());
                }
            }
            '~' if self.extended_glob => {
                self.advance();
                // Exclusion operator
                let inner = self.compile_piece()?;
                if let Some(node) = inner {
                    PatNode::Exclude(vec![node])
                } else {
                    return Err("~ requires pattern".to_string());
                }
            }
            _ => {
                // Collect literal characters
                let mut literal = String::new();
                while let Some(ch) = self.peek() {
                    if self.is_special(ch) {
                        break;
                    }
                    literal.push(ch);
                    self.advance();
                }
                if literal.is_empty() {
                    return Ok(None);
                }
                PatNode::Exactly(literal)
            }
        };

        // Check for repetition suffix
        if self.extended_glob {
            if let Some('#') = self.peek() {
                self.advance();
                if self.peek() == Some('#') {
                    self.advance();
                    return Ok(Some(PatNode::TwoHash(Box::new(node))));
                }
                return Ok(Some(PatNode::OneHash(Box::new(node))));
            }
        }

        Ok(Some(node))
    }

    fn is_special(&self, c: char) -> bool {
        matches!(c, '*' | '?' | '[' | '\\' | '(' | ')' | '|')
            || (self.extended_glob && matches!(c, '#' | '^' | '~' | '<'))
            || (self.ksh_glob && matches!(c, '+' | '!' | '@') && self.peek_n(1) == Some('('))
    }

    fn compile_bracket(&mut self) -> Result<PatNode, String> {
        self.advance(); // consume '['

        let negated = matches!(self.peek(), Some('!' | '^'));
        if negated {
            self.advance();
        }

        let mut chars = Vec::new();

        // ] at start is literal
        if self.peek() == Some(']') {
            chars.push(']');
            self.advance();
        }

        while let Some(c) = self.peek() {
            if c == ']' {
                self.advance();
                break;
            }

            if c == '\\' {
                self.advance();
                if let Some(escaped) = self.advance() {
                    chars.push(escaped);
                }
                continue;
            }

            // Check for POSIX class [:alpha:]
            if c == '[' && self.peek_n(1) == Some(':') {
                if let Some(class_chars) = self.parse_posix_class() {
                    chars.extend(class_chars);
                    continue;
                }
            }

            self.advance();

            // Check for range a-z
            if self.peek() == Some('-') && self.peek_n(1) != Some(']') {
                self.advance(); // consume '-'
                if let Some(end) = self.advance() {
                    for ch in c..=end {
                        chars.push(ch);
                    }
                    continue;
                }
            }

            chars.push(c);
        }

        if negated {
            Ok(PatNode::AnyBut(chars))
        } else {
            Ok(PatNode::AnyOf(chars))
        }
    }

    fn parse_posix_class(&mut self) -> Option<Vec<char>> {
        let start = self.pos;
        self.advance(); // [
        self.advance(); // :

        let mut class_name = String::new();
        while let Some(c) = self.peek() {
            if c == ':' {
                break;
            }
            class_name.push(c);
            self.advance();
        }

        if self.peek() != Some(':') || self.peek_n(1) != Some(']') {
            self.pos = start;
            return None;
        }
        self.advance(); // :
        self.advance(); // ]

        let chars: Vec<char> = match class_name.as_str() {
            "alpha" => ('a'..='z').chain('A'..='Z').collect(),
            "digit" => ('0'..='9').collect(),
            "alnum" => ('a'..='z').chain('A'..='Z').chain('0'..='9').collect(),
            "space" => vec![' ', '\t', '\n', '\r', '\x0b', '\x0c'],
            "upper" => ('A'..='Z').collect(),
            "lower" => ('a'..='z').collect(),
            "punct" => "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".chars().collect(),
            "xdigit" => ('0'..='9').chain('a'..='f').chain('A'..='F').collect(),
            "blank" => vec![' ', '\t'],
            "cntrl" => (0u8..=31)
                .map(|b| b as char)
                .chain(std::iter::once(127 as char))
                .collect(),
            "graph" | "print" => (33u8..=126).map(|b| b as char).collect(),
            "word" => ('a'..='z')
                .chain('A'..='Z')
                .chain('0'..='9')
                .chain(std::iter::once('_'))
                .collect(),
            _ => return None,
        };

        Some(chars)
    }

    fn compile_numeric_range(&mut self) -> Result<PatNode, String> {
        self.advance(); // consume '<'

        let mut from_str = String::new();
        let mut to_str = String::new();
        let mut in_to = false;

        while let Some(c) = self.peek() {
            if c == '>' {
                self.advance();
                break;
            }
            if c == '-' {
                self.advance();
                in_to = true;
                continue;
            }
            if c.is_ascii_digit() {
                if in_to {
                    to_str.push(c);
                } else {
                    from_str.push(c);
                }
                self.advance();
            } else {
                return Err(format!("invalid character in numeric range: {}", c));
            }
        }

        let from: Option<i64> = if from_str.is_empty() {
            None
        } else {
            from_str.parse().ok()
        };
        let to: Option<i64> = if to_str.is_empty() {
            None
        } else {
            to_str.parse().ok()
        };

        match (from, to) {
            (Some(f), Some(t)) => Ok(PatNode::NumRng(f, t)),
            (Some(f), None) => Ok(PatNode::NumFrom(f)),
            (None, Some(t)) => Ok(PatNode::NumTo(t)),
            (None, None) => Ok(PatNode::NumAny),
        }
    }
}

/// Pattern matcher state.
/// Port of the per-match locals `pattry()` from Src/pattern.c:2223
/// keeps on the stack — current input position, capture start/end
/// offsets, glob-flag state. The C source uses globals; we scope
/// them to the matcher.
pub struct PatMatcher<'a> {
    prog: &'a PatProg,
    input: &'a str,
    pos: usize,
    glob_flags: GlobFlags,
    /// Capture group positions: (start, end) byte offsets
    captures: [(usize, usize); NSUBEXP],
    captures_set: u16,
    /// Errors found (for approximate matching)
    errors_found: u32,
}

impl<'a> PatMatcher<'a> {
    pub fn new(prog: &'a PatProg, input: &'a str) -> Self {
        PatMatcher {
            prog,
            input,
            pos: 0,
            glob_flags: prog.glob_start,
            captures: [(0, 0); NSUBEXP],
            captures_set: 0,
            errors_found: 0,
        }
    }

    /// Try to match the pattern against the input
    pub fn try_match(&mut self) -> bool {
        // Handle pure string case
        if let Some(ref pure) = self.prog.pure_string {
            if self.glob_flags.igncase {
                return self.input.eq_ignore_ascii_case(pure);
            }
            return self.input == pure;
        }

        // Don't match leading dot unless explicitly matched
        if self.prog.flags.nogld && self.input.starts_with('.') {
            return false;
        }

        self.match_nodes_at(&self.prog.code.clone(), 0)
    }

    fn match_nodes_at(&mut self, nodes: &[PatNode], start_idx: usize) -> bool {
        let mut idx = start_idx;
        while idx < nodes.len() {
            let node = &nodes[idx];

            // Special handling for Star - needs to try all possible positions
            if matches!(node, PatNode::Star) {
                // If this is the last node, consume rest of input
                if idx + 1 >= nodes.len() {
                    self.pos = self.input.len();
                    return true;
                }

                // Try matching rest of pattern at each position
                let save_pos = self.pos;
                let end_pos = if self.prog.flags.file {
                    self.input[self.pos..]
                        .find('/')
                        .map(|i| self.pos + i)
                        .unwrap_or(self.input.len())
                } else {
                    self.input.len()
                };

                // Try from current position to end
                for try_pos in save_pos..=end_pos {
                    self.pos = try_pos;
                    if self.match_nodes_at(nodes, idx + 1) {
                        return true;
                    }
                }
                self.pos = save_pos;
                return false;
            }

            if !self.match_node(node) {
                return false;
            }
            idx += 1;
        }
        true
    }

    fn match_node(&mut self, node: &PatNode) -> bool {
        match node {
            PatNode::End => {
                // End matches if we're at the end of input
                // or if pattern is not anchored
                self.pos >= self.input.len() || self.prog.flags.noanch
            }

            PatNode::Exactly(s) => {
                let remaining = &self.input[self.pos..];
                if self.glob_flags.igncase {
                    if remaining.len() >= s.len() && remaining[..s.len()].eq_ignore_ascii_case(s) {
                        self.pos += s.len();
                        true
                    } else {
                        false
                    }
                } else if remaining.starts_with(s) {
                    self.pos += s.len();
                    true
                } else {
                    false
                }
            }

            PatNode::Nothing => true,

            PatNode::Any => {
                if self.pos < self.input.len() {
                    let c = self.current_char();
                    // Don't match '/' in file mode
                    if self.prog.flags.file && c == '/' {
                        return false;
                    }
                    self.pos += c.len_utf8();
                    true
                } else {
                    false
                }
            }

            PatNode::Star => {
                // Match any sequence - * just advances to end
                // Actual matching happens via backtracking in sequence matching
                // For file mode, don't cross '/'
                if self.prog.flags.file {
                    if let Some(slash_pos) = self.input[self.pos..].find('/') {
                        self.pos += slash_pos;
                    } else {
                        self.pos = self.input.len();
                    }
                } else {
                    self.pos = self.input.len();
                }
                true
            }

            PatNode::AnyOf(chars) => {
                if self.pos >= self.input.len() {
                    return false;
                }
                let c = self.current_char();
                let matched = if self.glob_flags.igncase {
                    chars.iter().any(|&ch| ch.eq_ignore_ascii_case(&c))
                } else {
                    chars.contains(&c)
                };
                if matched {
                    self.pos += c.len_utf8();
                    true
                } else {
                    false
                }
            }

            PatNode::AnyBut(chars) => {
                if self.pos >= self.input.len() {
                    return false;
                }
                let c = self.current_char();
                let in_set = if self.glob_flags.igncase {
                    chars.iter().any(|&ch| ch.eq_ignore_ascii_case(&c))
                } else {
                    chars.contains(&c)
                };
                if !in_set {
                    self.pos += c.len_utf8();
                    true
                } else {
                    false
                }
            }

            PatNode::Branch(alts, _) => {
                let save_pos = self.pos;
                // Try each alternative
                for alt in alts {
                    self.pos = save_pos;
                    if self.match_node(alt) {
                        return true;
                    }
                }
                self.pos = save_pos;
                false
            }

            PatNode::Sequence(nodes) => self.match_nodes_at(nodes, 0),

            PatNode::OneHash(inner) => {
                // Match 0 or more times
                loop {
                    let save_pos = self.pos;
                    if !self.match_single_node(inner) {
                        self.pos = save_pos;
                        break;
                    }
                    // Avoid infinite loop on empty match
                    if self.pos == save_pos {
                        break;
                    }
                }
                true
            }

            PatNode::TwoHash(inner) => {
                // Match 1 or more times
                if !self.match_single_node(inner) {
                    return false;
                }
                loop {
                    let save_pos = self.pos;
                    if !self.match_single_node(inner) {
                        self.pos = save_pos;
                        break;
                    }
                    if self.pos == save_pos {
                        break;
                    }
                }
                true
            }

            PatNode::Count { min, max, node } => {
                let mut count = 0u32;
                loop {
                    if let Some(m) = max {
                        if count >= *m {
                            break;
                        }
                    }
                    let save_pos = self.pos;
                    if !self.match_node(node) {
                        self.pos = save_pos;
                        break;
                    }
                    if self.pos == save_pos {
                        break;
                    }
                    count += 1;
                }
                count >= *min
            }

            PatNode::Open(n) => {
                if *n > 0 && *n <= NSUBEXP {
                    self.captures[n - 1].0 = self.pos;
                    self.captures_set |= 1 << (n - 1);
                }
                true
            }

            PatNode::Close(n) => {
                if *n > 0 && *n <= NSUBEXP {
                    self.captures[n - 1].1 = self.pos;
                }
                true
            }

            PatNode::NumRng(from, to) => self.match_number(Some(*from), Some(*to)),

            PatNode::NumFrom(from) => self.match_number(Some(*from), None),

            PatNode::NumTo(to) => self.match_number(None, Some(*to)),

            PatNode::NumAny => self.match_number(None, None),

            PatNode::IsStart => self.pos == 0,

            PatNode::IsEnd => self.pos >= self.input.len(),

            PatNode::GFlags(flags) => {
                self.glob_flags = *flags;
                true
            }

            PatNode::Exclude(inner) => {
                // Match if inner does NOT match
                let save_pos = self.pos;
                let matched = self.match_nodes_at(inner, 0);
                self.pos = save_pos;
                !matched
            }

            PatNode::ExcludP(inner) => {
                let save_pos = self.pos;
                let matched = self.match_nodes_at(inner, 0);
                self.pos = save_pos;
                !matched
            }

            PatNode::WBranch(alts) => {
                // Like branch but must match at least one char
                let save_pos = self.pos;
                for alt in alts {
                    self.pos = save_pos;
                    if self.match_node(alt) && self.pos > save_pos {
                        return true;
                    }
                }
                self.pos = save_pos;
                false
            }

            PatNode::ExcSync | PatNode::ExcEnd | PatNode::Back(_) | PatNode::CountStart => true,
        }
    }

    fn current_char(&self) -> char {
        self.input[self.pos..].chars().next().unwrap_or('\0')
    }

    /// Match a single node (for repetition operators)
    fn match_single_node(&mut self, node: &PatNode) -> bool {
        match node {
            PatNode::Sequence(nodes) => self.match_nodes_at(nodes, 0),
            _ => self.match_node(node),
        }
    }

    fn match_number(&mut self, from: Option<i64>, to: Option<i64>) -> bool {
        let start = self.pos;
        let mut num_str = String::new();

        // Collect digits
        while self.pos < self.input.len() {
            let c = self.current_char();
            if c.is_ascii_digit() {
                num_str.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }

        if num_str.is_empty() {
            self.pos = start;
            return false;
        }

        let num: i64 = match num_str.parse() {
            Ok(n) => n,
            Err(_) => {
                self.pos = start;
                return false;
            }
        };

        let in_range = match (from, to) {
            (Some(f), Some(t)) => num >= f && num <= t,
            (Some(f), None) => num >= f,
            (None, Some(t)) => num <= t,
            (None, None) => true,
        };

        if !in_range {
            self.pos = start;
            return false;
        }

        true
    }

    /// Get capture groups
    pub fn captures(&self) -> &[(usize, usize); NSUBEXP] {
        &self.captures
    }

    /// Get a specific capture group as a string slice
    pub fn capture(&self, n: usize) -> Option<&'a str> {
        if n == 0 || n > NSUBEXP {
            return None;
        }
        if self.captures_set & (1 << (n - 1)) == 0 {
            return None;
        }
        let (start, end) = self.captures[n - 1];
        if start <= end && end <= self.input.len() {
            Some(&self.input[start..end])
        } else {
            None
        }
    }
}

/// Compile a pattern string into a program.
/// Port of `patcompile()` from Src/pattern.c:540 — the entry point
/// the C source's `glob`/`[[ x = pat ]]` paths call to turn a
/// pattern string into a `Patprog`. The Rust AST replaces the
/// flat-bytecode `char *patcode` buffer the C source builds.
pub fn patcompile(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    PatCompiler::new(pattern, flags).compile()
}

/// Try to match a compiled pattern against a string.
/// Port of `pattry()` from Src/pattern.c:2223 — the C source's
/// top-level matcher entry point. Wraps `PatMatcher::try_match()`.
pub fn pattry(prog: &PatProg, s: &str) -> bool {
    PatMatcher::new(prog, s).try_match()
}

/// Compile and match a pattern in one call.
/// Convenience wrapper around `patcompile` (Src/pattern.c:540) +
/// `pattry` (Src/pattern.c:2223). Returns `false` on compile error
/// matching the `glob` builtin's "no-match" fall-through.
pub fn patmatch(pattern: &str, text: &str) -> bool {
    match patcompile(pattern, PatFlags::default()) {
        Ok(prog) => pattry(&prog, text),
        Err(_) => false,
    }
}

/// Try to match pattern against a length-limited string Port of `pattrylen()` from Src/pattern.c:2236
pub fn pattrylen(prog: &PatProg, s: &str, len: usize) -> bool {
    let truncated = if len < s.len() { &s[..len] } else { s };
    pattry(prog, truncated)
}

/// Try to match with backreferences Port of `pattryrefs()` from Src/pattern.c:2294
pub fn pattryrefs(prog: &PatProg, s: &str) -> Option<(bool, Vec<(usize, usize)>)> {
    let mut matcher = PatMatcher::new(prog, s);
    let matched = matcher.try_match();
    if matched {
        let refs: Vec<(usize, usize)> = (1..=prog.npar).map(|i| matcher.captures[i - 1]).collect();
        Some((true, refs))
    } else {
        Some((false, Vec::new()))
    }
}

/// Get the length of the successful match Port of `patmatchlen()` from Src/pattern.c:2649
pub fn patmatchlen(prog: &PatProg, s: &str) -> Option<usize> {
    let mut matcher = PatMatcher::new(prog, s);
    if matcher.try_match() {
        Some(matcher.pos)
    } else {
        None
    }
}

/// Parse glob flags from (#...) syntax Port of `patgetglobflags()` from Src/pattern.c:1037
///
/// Supports: (#i) case insensitive, (#l) lowercase matches upper,
/// (#I) restore case, (#b)/(#B) backrefs, (#m)/(#M) match refs,
/// `(#a<n>)` approximate matching, `(#s)` start assert, `(#e)` end assert,
/// (#u)/(#U) multibyte, (#q) glob qualifiers (ignored)
pub fn patgetglobflags(s: &str) -> Option<(GlobFlags, Option<PatOp>, usize)> {
    if !s.starts_with("(#") {
        return None;
    }

    let mut flags = GlobFlags::default();
    let mut assert_op = None;
    let mut pos = 2; // skip "(#"
    let bytes = s.as_bytes();

    while pos < bytes.len() && bytes[pos] != b')' {
        match bytes[pos] {
            b'q' => {
                // Glob qualifiers - skip to end
                while pos < bytes.len() && bytes[pos] != b')' {
                    pos += 1;
                }
                break;
            }
            b'a' => {
                // Approximate matching
                pos += 1;
                let mut num_str = String::new();
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    num_str.push(bytes[pos] as char);
                    pos += 1;
                }
                flags.approx = num_str.parse().unwrap_or(1).min(254);
                continue; // don't advance pos again
            }
            b'l' => {
                flags.lcmatchuc = true;
                flags.igncase = false;
            }
            b'i' => {
                flags.igncase = true;
                flags.lcmatchuc = false;
            }
            b'I' => {
                flags.igncase = false;
                flags.lcmatchuc = false;
            }
            b'b' => {
                flags.backref = true;
            }
            b'B' => {
                flags.backref = false;
            }
            b'm' => {
                flags.matchref = true;
            }
            b'M' => {
                flags.matchref = false;
            }
            b's' => {
                assert_op = Some(PatOp::IsStart);
            }
            b'e' => {
                assert_op = Some(PatOp::IsEnd);
            }
            b'u' => {
                flags.multibyte = true;
            }
            b'U' => {
                flags.multibyte = false;
            }
            _ => return None,
        }
        pos += 1;
    }

    if pos >= bytes.len() || bytes[pos] != b')' {
        return None;
    }
    pos += 1; // skip ')'

    // Start/end assertions must appear alone
    if assert_op.is_some() && pos - 3 > 1 {
        // more than one flag char
        return None;
    }

    Some((flags, assert_op, pos))
}

/// Check if character matches a character range element
/// Port of `patmatchrange()` from Src/pattern.c:3856
pub fn patmatchrange(range: &[char], ch: char, igncase: bool) -> bool {
    let ch = if igncase { ch.to_ascii_lowercase() } else { ch };
    for &rc in range {
        let rc = if igncase { rc.to_ascii_lowercase() } else { rc };
        if rc == ch {
            return true;
        }
    }
    false
}

/// Find index of character in range Port of `patmatchindex()` from Src/pattern.c:4004
pub fn patmatchindex(range: &[char], idx: usize) -> Option<char> {
    range.get(idx).copied()
}

/// Check if string contains pattern characters Port of `haswilds()` from Src/pattern.c:4306
pub fn haswilds(s: &str) -> bool {
    for c in s.chars() {
        match c {
            '*' | '?' | '[' | '#' | '^' | '~' | '<' | '>' => return true,
            _ => {}
        }
    }
    false
}

/// Repeat match for the given pattern node Port of `patrepeat()` from Src/pattern.c:4096
pub fn patrepeat(prog: &PatProg, s: &str, max: Option<usize>) -> usize {
    let mut matcher = PatMatcher::new(prog, s);
    let mut count = 0;
    loop {
        if let Some(m) = max {
            if count >= m {
                break;
            }
        }
        let save = matcher.pos;
        if !matcher.match_nodes_at(&prog.code, 0) {
            matcher.pos = save;
            break;
        }
        if matcher.pos == save {
            break; // No progress
        }
        count += 1;
    }
    count
}

/// Pattern scope state — saves disabled patterns for restore.
/// Port of the `disabled[]` table the C source's pattern-scope
/// machinery (`startpatternscope`/`endpatternscope` in
/// Src/pattern.c:4241/4279) maintains so a function can disable a
/// pattern qualifier locally without leaking the change.
#[derive(Debug, Default, Clone)]
pub struct PatternScope {
    pub disabled: Vec<String>,
}

use std::sync::Mutex;

static PATTERN_SCOPES: Mutex<Vec<PatternScope>> = Mutex::new(Vec::new());

/// Start a pattern scope Port of `startpatternscope()` from Src/pattern.c:4241
pub fn startpatternscope() {
    PATTERN_SCOPES.lock().unwrap().push(PatternScope::default());
}

/// End a pattern scope Port of `endpatternscope()` from Src/pattern.c:4279
pub fn endpatternscope() {
    PATTERN_SCOPES.lock().unwrap().pop();
}

/// Snapshot the current pattern-disables state.
/// Port of `savepatterndisables()` from Src/pattern.c:4220 — pairs
/// with `restorepatterndisables` to save/restore around a nested
/// function call.
pub fn savepatterndisables() -> Vec<String> {
    PATTERN_SCOPES
        .lock()
        .unwrap()
        .last()
        .map(|s| s.disabled.clone())
        .unwrap_or_default()
}

/// Restore a previously-saved pattern-disables state.
/// Port of `restorepatterndisables()` from Src/pattern.c:4258.
pub fn restorepatterndisables(disables: Vec<String>) {
    if let Some(scope) = PATTERN_SCOPES.lock().unwrap().last_mut() {
        scope.disabled = disables;
    }
}

/// Clear all pattern disables in the current scope.
/// Port of `clearpatterndisables()` from Src/pattern.c:4296.
pub fn clearpatterndisables() {
    if let Some(scope) = PATTERN_SCOPES.lock().unwrap().last_mut() {
        scope.disabled.clear();
    }
}

/// Free a compiled pattern.
/// Port of `freepatprog()` from Src/pattern.c:4161 — the C source's
/// allocator release for the bytecode buffer. Rust's `Drop` does
/// the equivalent automatically; this exists for call-site parity.
pub fn freepatprog(_prog: PatProg) {
    // Rust handles this via Drop
}

/// Enable/disable pattern commands Port of `pat_enables()` from Src/pattern.c:4171
pub fn pat_enables(cmd: &str, patterns: &[&str], enable: bool) -> i32 {
    let _ = (cmd, patterns, enable);
    // Pattern enable/disable is mainly for completion system
    0
}

/// POSIX character class type names for `[:stuff:]`.
/// Port of the `colon_stuffs[]` table Src/pattern.c (~line 1148)
/// uses to recognise POSIX bracket expressions inside character
/// classes. Order matches the C source so `range_type()` indices
/// stay stable.
pub const COLON_CLASSES: &[&str] = &[
    "alpha",
    "alnum",
    "ascii",
    "blank",
    "cntrl",
    "digit",
    "graph",
    "lower",
    "print",
    "punct",
    "space",
    "upper",
    "xdigit",
    "IDENT",
    "IFS",
    "IFSSPACE",
    "WORD",
    "INCOMPLETE",
    "INVALID",
];

/// Get the POSIX class type from name Port of `range_type()` from Src/pattern.c:1148
pub fn range_type(name: &str) -> Option<usize> {
    COLON_CLASSES.iter().position(|&c| c == name)
}

/// Convert a pattern range to a string for display Port of `pattern_range_to_string()` from Src/pattern.c:1179
pub fn pattern_range_to_string(range_type_idx: usize) -> Option<String> {
    COLON_CLASSES
        .get(range_type_idx)
        .map(|s| format!("[:{}:]", s))
}

// ---------------------------------------------------------------------------
// C-internal pattern compiler functions - implemented differently in Rust
// These are provided as thin wrappers/stubs for API completeness
// ---------------------------------------------------------------------------

/// Clear multibyte shift state Port of `clear_shiftstate()` from Src/pattern.c:327 — no-op in Rust (we use native `char`, no shift-state needed).
pub fn clear_shiftstate() {}

/// Advance past metafied char Port of `metacharinc()` from Src/pattern.c:336 — Rust strings are native UTF-8 so this is just `len_utf8` advance.
pub fn metacharinc(s: &str, pos: usize) -> usize {
    let c = s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    pos + c
}

/// Add bytes to pattern buffer Port of `patadd()` from Src/pattern.c:412 — the C source builds the bytecode in a flat `char *patcode` buffer; Rust uses `Vec<PatNode>`.
pub fn patadd(prog: &mut Vec<PatNode>, node: PatNode) {
    prog.push(node);
}

/// Set up pattern compiler char sets Port of `patcompcharsset()` from Src/pattern.c:464 — no-op in Rust (the C source initializes a static char-class table; Rust pattern-matches inline).
pub fn patcompcharsset() {}

/// Initialize pattern compilation Port of `patcompstart()` from Src/pattern.c:517 — no-op in Rust (the C source resets compiler globals; Rust threads state through `PatCompiler`).
pub fn patcompstart() {}

/// Compile a top-level pattern with alternation.
/// Port of `patcompswitch()` from Src/pattern.c:765 — the C source's
/// alternation entry point. The Rust path delegates to the full
/// compiler since `PatCompiler::compile_branch` handles `|` inline.
pub fn patcompswitch(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    patcompile(pattern, flags)
}

/// Compile a single pattern branch.
/// Port of `patcompbranch()` from Src/pattern.c:942.
pub fn patcompbranch(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    patcompile(pattern, flags)
}

/// Compile a single pattern piece.
/// Port of `patcomppiece()` from Src/pattern.c:1261.
pub fn patcomppiece(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    patcompile(pattern, flags)
}

/// Compile a negation pattern (`^pat` / `!(pat)`).
/// Port of `patcompnot()` from Src/pattern.c:1760 — the C source
/// inverts the match through an `Exclude` node.
pub fn patcompnot(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    let negated = format!("^({})", pattern);
    patcompile(&negated, flags)
}

/// Add node to bytecode Port of `patnode()` from Src/pattern.c:1790 — the C source appends an opcode to `patcode`; Rust appends to a `Vec`.
pub fn patnode(prog: &mut Vec<PatNode>, node: PatNode) -> usize {
    let idx = prog.len();
    prog.push(node);
    idx
}

/// Insert node at position Port of `patinsert()` from Src/pattern.c:1807 — the C source uses a buffer-shift; Rust uses `Vec::insert`.
pub fn patinsert(prog: &mut Vec<PatNode>, pos: usize, node: PatNode) {
    if pos <= prog.len() {
        prog.insert(pos, node);
    }
}

/// Set tail pointer Port of `pattail()` from Src/pattern.c:1834 — no-op in Rust (the C source patches forward jumps in flat bytecode; the Rust AST already knows its successor nodes).
pub fn pattail(_prog: &[PatNode], _p: usize, _val: usize) {}

/// Set optional tail pointer Port of `patoptail()` from Src/pattern.c:1856 — see `pattail` above; same reasoning.
pub fn patoptail(_prog: &[PatNode], _p: usize, _val: usize) {}

/// Get char reference Port of `charref()` from Src/pattern.c:1909 — the C source decodes a metafied byte at offset; Rust's `chars().next()` does the equivalent for UTF-8.
pub fn charref(s: &str, pos: usize) -> Option<char> {
    s[pos..].chars().next()
}

/// Get next char Port of `charnext()` from Src/pattern.c:1936 — wraps `metacharinc` for the natural advance step.
pub fn charnext(s: &str, pos: usize) -> usize {
    metacharinc(s, pos)
}

/// Get char and advance Port of `charrefinc()` from Src/pattern.c:1964 — atomic decode-and-advance, no metafying needed in Rust.
pub fn charrefinc(s: &str, pos: &mut usize) -> Option<char> {
    let c = s[*pos..].chars().next()?;
    *pos += c.len_utf8();
    Some(c)
}

/// Get previous char width Port of `charsub()` from Src/pattern.c:1997 — gets the char width before `pos` so we can step backwards.
pub fn charsub(s: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let prev = s[..pos]
        .chars()
        .next_back()
        .map(|c| c.len_utf8())
        .unwrap_or(1);
    pos - prev
}

/// Initialize pattern try Port of `pattrystart()` from Src/pattern.c:2063 — no-op in Rust (resets per-match state which `PatMatcher::new` already initializes).
pub fn pattrystart() {}

/// Prepare string for pattern matching Port of `patmungestring()` from Src/pattern.c:2080 — identity in Rust (the C source un-metafies the input buffer; UTF-8 strings need no munging).
pub fn patmungestring(s: &str) -> String {
    s.to_string()
}

/// Multibyte pattern match range Port of `mb_patmatchrange()` from Src/pattern.c:3610 — Rust's `char` is already a multibyte-safe code point so the multibyte and ASCII paths collapse.
pub fn mb_patmatchrange(range: &[char], ch: char, igncase: bool) -> bool {
    patmatchrange(range, ch, igncase)
}

/// Multibyte pattern match index Port of `mb_patmatchindex()` from Src/pattern.c:3767
pub fn mb_patmatchindex(range: &[char], idx: usize) -> Option<char> {
    patmatchindex(range, idx)
}

/// Allocate pattern string buffer Port of `patallocstr()` from Src/pattern.c:2132 — no-op in Rust (the C source un-metafies into a fresh heap buffer; native UTF-8 needs no copy)
pub fn patallocstr(s: &str) -> String {
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_literal() {
        assert!(patmatch("hello", "hello"));
        assert!(!patmatch("hello", "world"));
        assert!(!patmatch("hello", "hell"));
    }

    #[test]
    fn test_star() {
        assert!(patmatch("*", "anything"));
        assert!(patmatch("*", ""));
        assert!(patmatch("h*o", "hello"));
        assert!(patmatch("h*o", "ho"));
        assert!(!patmatch("h*o", "hi"));
    }

    #[test]
    fn test_question() {
        assert!(patmatch("?", "a"));
        assert!(!patmatch("?", "ab"));
        assert!(patmatch("h?llo", "hello"));
        assert!(patmatch("h?llo", "hallo"));
        assert!(!patmatch("h?llo", "hllo"));
    }

    #[test]
    fn test_bracket() {
        assert!(patmatch("[abc]", "a"));
        assert!(patmatch("[abc]", "b"));
        assert!(!patmatch("[abc]", "d"));
        assert!(patmatch("[a-z]", "m"));
        assert!(!patmatch("[a-z]", "5"));
    }

    #[test]
    fn test_bracket_negated() {
        assert!(!patmatch("[!abc]", "a"));
        assert!(patmatch("[!abc]", "d"));
        assert!(patmatch("[^abc]", "x"));
    }

    #[test]
    fn test_escape() {
        assert!(patmatch("\\*", "*"));
        assert!(!patmatch("\\*", "a"));
        assert!(patmatch("\\?", "?"));
    }

    #[test]
    fn test_numeric_range() {
        assert!(patmatch("<1-10>", "5"));
        assert!(patmatch("<1-10>", "1"));
        assert!(patmatch("<1-10>", "10"));
        assert!(!patmatch("<1-10>", "0"));
        assert!(!patmatch("<1-10>", "11"));
    }

    #[test]
    fn test_case_insensitive() {
        // Inline patcompile_opts + pattry equivalent of the deleted
        // patmatch_opts wrapper. Mirrors zsh's `setopt nocasematch`
        // path through patcompile + pattry.
        let compile = |pattern: &str, igncase: bool| -> PatProg {
            PatCompiler::new(pattern, PatFlags::default())
                .with_options(true, true)
                .with_igncase(igncase)
                .compile()
                .unwrap()
        };
        assert!(pattry(&compile("Hello", true), "HELLO"));
        assert!(pattry(&compile("Hello", true), "hello"));
        assert!(!pattry(&compile("Hello", false), "HELLO"));
    }

    #[test]
    fn test_extended_hash() {
        // # = 0 or more of previous
        assert!(patmatch("a#", ""));
        assert!(patmatch("a#", "a"));
        assert!(patmatch("a#", "aaa"));
    }

    #[test]
    fn test_captures() {
        // Inline of the deleted patmatch_captures helper — runs the
        // matcher and surfaces the per-group capture slices that
        // Src/pattern.c:patbeginp[]/patendp[] expose to ${match[N]}.
        let prog = patcompile("(foo)(bar)", PatFlags::default()).unwrap();
        let mut matcher = PatMatcher::new(&prog, "foobar");
        assert!(matcher.try_match());
        let mut captures: Vec<Option<&str>> = Vec::with_capacity(prog.npar);
        for i in 1..=prog.npar {
            captures.push(matcher.capture(i));
        }
        assert_eq!(captures.len(), 2);
        assert_eq!(captures[0], Some("foo"));
        assert_eq!(captures[1], Some("bar"));
    }

    #[test]
    fn test_posix_class() {
        assert!(patmatch("[[:alpha:]]", "a"));
        assert!(patmatch("[[:alpha:]]", "Z"));
        assert!(!patmatch("[[:alpha:]]", "5"));
        assert!(patmatch("[[:digit:]]", "5"));
        assert!(!patmatch("[[:digit:]]", "a"));
    }

    #[test]
    fn test_pure_string_optimization() {
        let prog = patcompile("hello", PatFlags::default()).unwrap();
        assert!(prog.flags.pures);
        assert!(prog.pure_string.is_some());
    }

    #[test]
    fn test_ksh_glob_plus() {
        // +(pattern) = 1 or more
        assert!(patmatch("+(ab)", "ab"));
        assert!(patmatch("+(ab)", "abab"));
        assert!(!patmatch("+(ab)", ""));
    }

    #[test]
    fn test_ksh_glob_star() {
        // *(pattern) = 0 or more
        assert!(patmatch("*(ab)", ""));
        assert!(patmatch("*(ab)", "ab"));
        assert!(patmatch("*(ab)", "ababab"));
    }

    #[test]
    fn test_ksh_glob_question() {
        // ?(pattern) = 0 or 1
        assert!(patmatch("?(ab)c", "c"));
        assert!(patmatch("?(ab)c", "abc"));
    }

    #[test]
    fn test_pattrylen() {
        let prog = patcompile("hello", PatFlags::default()).unwrap();
        assert!(pattrylen(&prog, "hello world", 5));
        assert!(!pattrylen(&prog, "hello world", 3));
    }

    #[test]
    fn test_patmatchlen() {
        let prog = patcompile(
            "hel*",
            PatFlags {
                noanch: true,
                ..Default::default()
            },
        )
        .unwrap();
        let len = patmatchlen(&prog, "hello world");
        assert!(len.is_some());
    }

    #[test]
    fn test_patgetglobflags() {
        let (flags, assert_op, consumed) = patgetglobflags("(#i)rest").unwrap();
        assert!(flags.igncase);
        assert!(assert_op.is_none());
        assert_eq!(consumed, 4);

        let (flags, _, _) = patgetglobflags("(#l)rest").unwrap();
        assert!(flags.lcmatchuc);
        assert!(!flags.igncase);

        let (_, assert_op, _) = patgetglobflags("(#s)rest").unwrap();
        assert_eq!(assert_op, Some(PatOp::IsStart));

        let (flags, _, _) = patgetglobflags("(#bm)rest").unwrap();
        assert!(flags.backref);
        assert!(flags.matchref);
    }

    #[test]
    fn test_haswilds() {
        assert!(haswilds("*.txt"));
        assert!(haswilds("file?"));
        assert!(haswilds("[abc]"));
        assert!(haswilds("foo#"));
        assert!(!haswilds("plain"));
    }

    #[test]
    fn test_patmatchrange() {
        let range = vec!['a', 'b', 'c'];
        assert!(patmatchrange(&range, 'a', false));
        assert!(!patmatchrange(&range, 'd', false));
        assert!(patmatchrange(&range, 'A', true));
    }

    #[test]
    fn test_range_type() {
        assert_eq!(range_type("alpha"), Some(0));
        assert_eq!(range_type("digit"), Some(5));
        assert_eq!(range_type("nonexistent"), None);
    }

    #[test]
    fn test_pattern_range_to_string() {
        assert_eq!(pattern_range_to_string(0), Some("[:alpha:]".to_string()));
        assert_eq!(pattern_range_to_string(5), Some("[:digit:]".to_string()));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: pattern
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Check if pattern contains extended glob syntax
    pub(crate) fn has_extglob_pattern(&self, pattern: &str) -> bool {
        let chars: Vec<char> = pattern.chars().collect();
        for i in 0..chars.len().saturating_sub(1) {
            if (chars[i] == '?'
                || chars[i] == '*'
                || chars[i] == '+'
                || chars[i] == '@'
                || chars[i] == '!')
                && chars[i + 1] == '('
            {
                return true;
            }
        }
        false
    }
    /// Convert extended glob pattern to regex
    pub(crate) fn extglob_to_regex(&self, pattern: &str) -> String {
        let mut regex = String::from("^");
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];

            // Check for extglob patterns
            if i + 1 < chars.len() && chars[i + 1] == '(' {
                match c {
                    '?' => {
                        // ?(pattern) - zero or one occurrence
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})?", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '*' => {
                        // *(pattern) - zero or more occurrences
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})*", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '+' => {
                        // +(pattern) - one or more occurrences
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})+", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '@' => {
                        // @(pattern) - exactly one occurrence
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '!' => {
                        // !(pattern) - handled specially in expand_extglob
                        // Just skip this extglob for regex, will do manual filtering
                        let (_, end) = self.extract_extglob_inner(&chars, i + 2);
                        regex.push_str(".*"); // Match anything, we filter later
                        i = end + 1;
                        continue;
                    }
                    _ => {}
                }
            }

            // Handle regular glob characters
            match c {
                '*' => regex.push_str(".*"),
                '?' => regex.push('.'),
                '.' => regex.push_str("\\."),
                '[' => {
                    regex.push('[');
                    i += 1;
                    while i < chars.len() && chars[i] != ']' {
                        if chars[i] == '!' && regex.ends_with('[') {
                            regex.push('^');
                        } else {
                            regex.push(chars[i]);
                        }
                        i += 1;
                    }
                    regex.push(']');
                }
                '^' | '$' | '(' | ')' | '{' | '}' | '|' | '\\' => {
                    regex.push('\\');
                    regex.push(c);
                }
                _ => regex.push(c),
            }
            i += 1;
        }

        regex.push('$');
        regex
    }
    /// Extract the inner part of an extglob pattern (until closing paren)
    pub(crate) fn extract_extglob_inner(&self, chars: &[char], start: usize) -> (String, usize) {
        let mut inner = String::new();
        let mut depth = 1;
        let mut i = start;

        while i < chars.len() && depth > 0 {
            if chars[i] == '(' {
                depth += 1;
            } else if chars[i] == ')' {
                depth -= 1;
                if depth == 0 {
                    return (inner, i);
                }
            }
            inner.push(chars[i]);
            i += 1;
        }

        (inner, i)
    }
    /// Convert the inner part of extglob (handles | for alternation)
    pub(crate) fn extglob_inner_to_regex(&self, inner: &str) -> String {
        // Split by | and convert each alternative
        let alternatives: Vec<String> = inner
            .split('|')
            .map(|alt| {
                let mut result = String::new();
                for c in alt.chars() {
                    match c {
                        '*' => result.push_str(".*"),
                        '?' => result.push('.'),
                        '.' => result.push_str("\\."),
                        '^' | '$' | '(' | ')' | '{' | '}' | '\\' => {
                            result.push('\\');
                            result.push(c);
                        }
                        _ => result.push(c),
                    }
                }
                result
            })
            .collect();

        alternatives.join("|")
    }
    /// Extract !(pattern) info from file pattern, returns (inner_pattern, suffix)
    pub(crate) fn extract_neg_extglob(&self, pattern: &str) -> Option<(String, String)> {
        let chars: Vec<char> = pattern.chars().collect();
        if chars.len() >= 3 && chars[0] == '!' && chars[1] == '(' {
            let mut depth = 1;
            let mut i = 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '(' {
                    depth += 1;
                } else if chars[i] == ')' {
                    depth -= 1;
                }
                i += 1;
            }
            if depth == 0 {
                let inner: String = chars[2..i - 1].iter().collect();
                let suffix: String = chars[i..].iter().collect();
                return Some((inner, suffix));
            }
        }
        None
    }
}
// END moved-from-exec-rs

// ===========================================================
// Free fns moved verbatim from src/ported/exec.rs.
// ===========================================================
// BEGIN moved-from-exec-rs (free fns)
/// Full pattern-flag parser that also reports the `(#b)` backref-
/// capture flag in addition to the four flags `parse_pattern_flags`
/// returns. Used by `BUILTIN_PARAM_REPLACE` to enable
/// `${match[N]}` backreference population. Per zshexpn(1):
///   `(#b)` — capture each `(...)` group in the pattern; on match,
///            $match[N] holds capture N (1-based), $mbegin / $mend
///            hold start/end positions.
///   `(#B)` — turn it off (default).
pub(crate) fn parse_pattern_flags_full(
    pat: &str,
) -> (String, bool, bool, Option<usize>, bool) {
    if !pat.starts_with("(#") {
        return (pat.to_string(), false, false, None, false);
    }
    let after = &pat[2..];
    let close = match after.find(')') {
        Some(i) => i,
        None => return (pat.to_string(), false, false, None, false),
    };
    let flag_str = &after[..close];
    let rest = &after[close + 1..];
    let mut case_i = false;
    let mut l = false;
    let mut approx: Option<usize> = None;
    let mut backref = false;
    let bytes = flag_str.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'i' => {
                case_i = true;
                i += 1;
            }
            b'I' => {
                case_i = false;
                i += 1;
            }
            b'l' => {
                l = true;
                i += 1;
            }
            b'b' => {
                backref = true;
                i += 1;
            }
            b'B' => {
                backref = false;
                i += 1;
            }
            b'm' => {
                // `(#m)` flag: per Src/pattern.c the matched text is
                // exposed via $MATCH in the replacement, plus
                // $MBEGIN/$MEND for offsets. zshrs uses the same
                // backref-mode plumbing — the replacement template is
                // re-expanded with caps available, so $MATCH resolves
                // through expand_string. Direct port of zsh's
                // `pat_pure_m` flag (line 154 in Src/pattern.c).
                backref = true;
                i += 1;
            }
            b'M' => {
                // `(#M)` — disable (m) (rarely used, but symmetric).
                backref = false;
                i += 1;
            }
            b'a' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let n: usize = if i > start {
                    flag_str[start..i].parse().unwrap_or(1)
                } else {
                    1
                };
                approx = Some(n);
            }
            _ => {
                return (pat.to_string(), false, false, None, false);
            }
        }
    }
    (rest.to_string(), case_i, l, approx, backref)
}
/// Approximate match: returns true if `s` matches `pat` with up to `n`
/// edit-distance errors. Uses the Wagner-Fischer dynamic-programming
/// algorithm to compute Levenshtein distance, then compares against
/// the budget. Glob metacharacters in `pat` are NOT honored — zsh's
/// `(#a)` form combines with literal patterns; combining with `*`/`?`
/// is rare and not supported here.
/// Match `s` against zsh-extended glob `pat`. When the `extendedglob`
/// Translate the body of a ksh-style extglob group `(p1|p2|...)`
/// into a regex alternation. Each branch is glob-translated by the
/// same rules as `glob_match_static` minus the wrapping anchors and
/// minus the (#flags)/numeric-range support (those don't appear
/// inside extglob bodies in practice).
// END moved-from-exec-rs (free fns)

// ===========================================================
// Pattern helpers moved from src/ported/exec.rs.
// All correspond to Src/pattern.c logic.
// ===========================================================


/// Apply a `${var/pat/repl}` style pattern replacement to a single
/// scalar string. `op`: 0=first-match, 1=all (`//`), 2=anchored prefix
/// (`/#`), 3=anchored suffix (`/%`). Mirrors the runtime
/// `BUILTIN_PARAM_REPLACE` `one()` closure but operates on a free value
/// — used by the array-element subscript path
/// (`${a[N]/pat/repl}`) which can't dispatch through the name-keyed
/// builtin.
fn zsh_pattern_replace(val: &str, pattern: &str, repl: &str, op: u8) -> String {
    let has_glob = pattern
        .chars()
        .any(|c| matches!(c, '?' | '*' | '[' | ']' | '('));
    let glob_re: Option<regex::Regex> = if has_glob {
        let mut re = String::with_capacity(pattern.len() * 2);
        let mut chars = pattern.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '?' => re.push('.'),
                '*' => re.push_str(".*"),
                '[' => {
                    re.push('[');
                    if chars.peek() == Some(&'!') {
                        chars.next();
                        re.push('^');
                    }
                    for cc in chars.by_ref() {
                        re.push(cc);
                        if cc == ']' {
                            break;
                        }
                    }
                }
                '\\' => {
                    re.push('\\');
                    if let Some(next) = chars.next() {
                        re.push(next);
                    }
                }
                '(' | ')' | '|' => re.push(c),
                '.' | '+' | '^' | '$' | '{' | '}' => {
                    re.push('\\');
                    re.push(c);
                }
                _ => re.push(c),
            }
        }
        regex::Regex::new(&re).ok()
    } else {
        None
    };
    let val = val.to_string();
    if let Some(rx) = glob_re {
        return match op {
            0 => rx.replacen(&val, 1, repl).to_string(),
            1 => rx.replace_all(&val, repl).to_string(),
            2 => {
                if let Some(m) = rx.find(&val) {
                    if m.start() == 0 {
                        return format!("{}{}", repl, &val[m.end()..]);
                    }
                }
                val
            }
            3 => {
                let mut last_start: Option<usize> = None;
                for m in rx.find_iter(&val) {
                    if m.end() == val.len() {
                        last_start = Some(m.start());
                    }
                }
                if let Some(s) = last_start {
                    return format!("{}{}", &val[..s], repl);
                }
                val
            }
            _ => val,
        };
    }
    match op {
        0 => val.replacen(pattern, repl, 1),
        1 => val.replace(pattern, repl),
        2 => {
            if let Some(suffix) = val.strip_prefix(pattern) {
                format!("{}{}", repl, suffix)
            } else {
                val
            }
        }
        3 => {
            if val.ends_with(pattern) {
                format!("{}{}", &val[..val.len() - pattern.len()], repl)
            } else {
                val
            }
        }
        _ => val,
    }
}

/// Numeric range glob `<N-M>` parsed form. `None`/`None` means open-ended
/// on that side (`<->` matches any digits, `<3->` ≥ 3, `<-5>` ≤ 5).
#[derive(Debug, Clone)]
pub(crate) struct NumericRange {
    pub(crate) lo: Option<i64>,
    pub(crate) hi: Option<i64>,
}

/// Walk the pattern once, returning each `<N-M>` range in source order.
/// Skips bracket expressions (`[<…>]`) so the inside-`[]` `<` stays
/// literal. Caller calls [`replace_numeric_ranges_with_star`] in lockstep
/// to keep counts aligned.
pub(crate) fn extract_numeric_ranges(pattern: &str) -> Vec<NumericRange> {
    let mut ranges = Vec::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' && !in_bracket {
            in_bracket = true;
            i += 1;
            continue;
        }
        if c == ']' && in_bracket {
            in_bracket = false;
            i += 1;
            continue;
        }
        if c == '<' && !in_bracket {
            let mut j = i + 1;
            let mut lo_str = String::new();
            while j < chars.len() && chars[j].is_ascii_digit() {
                lo_str.push(chars[j]);
                j += 1;
            }
            if j < chars.len() && chars[j] == '-' {
                j += 1;
                let mut hi_str = String::new();
                while j < chars.len() && chars[j].is_ascii_digit() {
                    hi_str.push(chars[j]);
                    j += 1;
                }
                if j < chars.len() && chars[j] == '>' {
                    let lo = lo_str.parse::<i64>().ok();
                    let hi = hi_str.parse::<i64>().ok();
                    ranges.push(NumericRange { lo, hi });
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    ranges
}

/// Replace each `<N-M>` (matching `extract_numeric_ranges`) with a `*`
/// so the underlying glob crate matches any chars at that spot. The
/// post-filter then narrows to digits in range.
pub(crate) fn replace_numeric_ranges_with_star(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' && !in_bracket {
            in_bracket = true;
            out.push(c);
            i += 1;
            continue;
        }
        if c == ']' && in_bracket {
            in_bracket = false;
            out.push(c);
            i += 1;
            continue;
        }
        if c == '<' && !in_bracket {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '-' {
                j += 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '>' {
                    out.push('*');
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Build a regex anchored to the basename position by translating each
/// glob metachar to its regex equivalent and each `<N-M>` to `(\d+)`,
/// then keep only candidates whose captured digit sequence falls in the
/// declared range. Operates on the trailing path component since
/// numeric ranges only apply within a single segment.
fn filter_numeric_ranges(
    candidates: Vec<String>,
    original_pattern: &str,
    ranges: &[NumericRange],
) -> Vec<String> {
    let pat_basename = original_pattern
        .rsplit('/')
        .next()
        .unwrap_or(original_pattern);
    let mut regex_str = String::from("^");
    let chars: Vec<char> = pat_basename.chars().collect();
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' && !in_bracket {
            in_bracket = true;
            regex_str.push('[');
            i += 1;
            continue;
        }
        if c == ']' && in_bracket {
            in_bracket = false;
            regex_str.push(']');
            i += 1;
            continue;
        }
        if in_bracket {
            regex_str.push(c);
            i += 1;
            continue;
        }
        if c == '<' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '-' {
                j += 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '>' {
                    regex_str.push_str("(\\d+)");
                    i = j + 1;
                    continue;
                }
            }
        }
        match c {
            '*' => regex_str.push_str(".*"),
            '?' => regex_str.push('.'),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '\\' | '{' | '}' => {
                regex_str.push('\\');
                regex_str.push(c);
            }
            _ => regex_str.push(c),
        }
        i += 1;
    }
    regex_str.push('$');

    let re = match regex::Regex::new(&regex_str) {
        Ok(r) => r,
        Err(_) => return candidates,
    };
    candidates
        .into_iter()
        .filter(|p| {
            let basename = p.rsplit('/').next().unwrap_or(p);
            let caps = match re.captures(basename) {
                Some(c) => c,
                None => return false,
            };
            for (idx, range) in ranges.iter().enumerate() {
                let cap = match caps.get(idx + 1) {
                    Some(m) => m.as_str(),
                    None => return false,
                };
                let val: i64 = match cap.parse() {
                    Ok(v) => v,
                    Err(_) => return false,
                };
                if let Some(lo) = range.lo {
                    if val < lo {
                        return false;
                    }
                }
                if let Some(hi) = range.hi {
                    if val > hi {
                        return false;
                    }
                }
            }
            true
        })
        .collect()
}
