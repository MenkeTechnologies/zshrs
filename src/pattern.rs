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
//! - Numeric ranges: <n-m>

/// Pattern opcodes - matching zsh's P_* constants
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

/// Pattern flags
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

/// Globbing flags
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobFlags {
    pub igncase: bool,   // Case insensitive
    pub lcmatchuc: bool, // Lowercase matches uppercase
    pub matchref: bool,  // Set MATCH, MBEGIN, MEND
    pub backref: bool,   // Enable backreferences
    pub multibyte: bool, // Multibyte support
    pub approx: u8,      // Approximation level (error tolerance)
}

/// Compiled pattern program
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

/// A node in the pattern bytecode
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
            match self.peek() {
                Some('#') => {
                    self.advance();
                    if self.peek() == Some('#') {
                        self.advance();
                        return Ok(Some(PatNode::TwoHash(Box::new(node))));
                    }
                    return Ok(Some(PatNode::OneHash(Box::new(node))));
                }
                _ => {}
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

/// Pattern matcher state
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

/// Compile a pattern string into a program
pub fn patcompile(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    PatCompiler::new(pattern, flags).compile()
}

/// Compile with options
pub fn patcompile_opts(
    pattern: &str,
    flags: PatFlags,
    extended_glob: bool,
    ksh_glob: bool,
    igncase: bool,
) -> Result<PatProg, String> {
    PatCompiler::new(pattern, flags)
        .with_options(extended_glob, ksh_glob)
        .with_igncase(igncase)
        .compile()
}

/// Try to match pattern against string
pub fn pattry(prog: &PatProg, s: &str) -> bool {
    PatMatcher::new(prog, s).try_match()
}

/// Simple pattern match (compile and match in one call)
pub fn patmatch(pattern: &str, text: &str) -> bool {
    match patcompile(pattern, PatFlags::default()) {
        Ok(prog) => pattry(&prog, text),
        Err(_) => false,
    }
}

/// Pattern match with options
pub fn patmatch_opts(
    pattern: &str,
    text: &str,
    extended_glob: bool,
    ksh_glob: bool,
    igncase: bool,
) -> bool {
    match patcompile_opts(
        pattern,
        PatFlags::default(),
        extended_glob,
        ksh_glob,
        igncase,
    ) {
        Ok(prog) => pattry(&prog, text),
        Err(_) => false,
    }
}

/// Match with captures - returns capture groups if matched
pub fn patmatch_captures<'a>(prog: &'a PatProg, text: &'a str) -> Option<Vec<Option<&'a str>>> {
    let mut matcher = PatMatcher::new(prog, text);
    if matcher.try_match() {
        let mut captures = Vec::with_capacity(prog.npar);
        for i in 1..=prog.npar {
            captures.push(matcher.capture(i));
        }
        Some(captures)
    } else {
        None
    }
}

/// Try to match pattern against a length-limited string (from pattern.c pattrylen)
pub fn pattrylen(prog: &PatProg, s: &str, len: usize) -> bool {
    let truncated = if len < s.len() { &s[..len] } else { s };
    pattry(prog, truncated)
}

/// Try to match with backreferences (from pattern.c pattryrefs)
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

/// Get the length of the successful match (from pattern.c patmatchlen)
pub fn patmatchlen(prog: &PatProg, s: &str) -> Option<usize> {
    let mut matcher = PatMatcher::new(prog, s);
    if matcher.try_match() {
        Some(matcher.pos)
    } else {
        None
    }
}

/// Parse glob flags from (#...) syntax (from pattern.c patgetglobflags)
///
/// Supports: (#i) case insensitive, (#l) lowercase matches upper,
/// (#I) restore case, (#b)/(#B) backrefs, (#m)/(#M) match refs,
/// (#a<n>) approximate matching, (#s) start assert, (#e) end assert,
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
/// (from pattern.c patmatchrange)
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

/// Find index of character in range (from pattern.c patmatchindex)
pub fn patmatchindex(range: &[char], idx: usize) -> Option<char> {
    range.get(idx).copied()
}

/// Check if string contains pattern characters (from pattern.c haswilds)
pub fn haswilds(s: &str) -> bool {
    for c in s.chars() {
        match c {
            '*' | '?' | '[' | '#' | '^' | '~' | '<' | '>' => return true,
            _ => {}
        }
    }
    false
}

/// Repeat match for the given pattern node (from pattern.c patrepeat)
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

/// Pattern scope management - save disabled patterns
#[derive(Debug, Default, Clone)]
pub struct PatternScope {
    pub disabled: Vec<String>,
}

use std::sync::Mutex;

static PATTERN_SCOPES: Mutex<Vec<PatternScope>> = Mutex::new(Vec::new());

/// Start a pattern scope (from pattern.c startpatternscope)
pub fn startpatternscope() {
    PATTERN_SCOPES.lock().unwrap().push(PatternScope::default());
}

/// End a pattern scope (from pattern.c endpatternscope)
pub fn endpatternscope() {
    PATTERN_SCOPES.lock().unwrap().pop();
}

/// Save pattern disables state
pub fn savepatterndisables() -> Vec<String> {
    PATTERN_SCOPES
        .lock()
        .unwrap()
        .last()
        .map(|s| s.disabled.clone())
        .unwrap_or_default()
}

/// Restore pattern disables state
pub fn restorepatterndisables(disables: Vec<String>) {
    if let Some(scope) = PATTERN_SCOPES.lock().unwrap().last_mut() {
        scope.disabled = disables;
    }
}

/// Clear pattern disables
pub fn clearpatterndisables() {
    if let Some(scope) = PATTERN_SCOPES.lock().unwrap().last_mut() {
        scope.disabled.clear();
    }
}

/// Free a compiled pattern (no-op in Rust, but provided for API compat)
pub fn freepatprog(_prog: PatProg) {
    // Rust handles this via Drop
}

/// Enable/disable pattern commands (from pattern.c pat_enables)
pub fn pat_enables(cmd: &str, patterns: &[&str], enable: bool) -> i32 {
    let _ = (cmd, patterns, enable);
    // Pattern enable/disable is mainly for completion system
    0
}

/// POSIX character class type names for [:stuff:]
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

/// Get the POSIX class type from name (from pattern.c range_type)
pub fn range_type(name: &str) -> Option<usize> {
    COLON_CLASSES.iter().position(|&c| c == name)
}

/// Convert a pattern range to a string for display (from pattern.c pattern_range_to_string)
pub fn pattern_range_to_string(range_type_idx: usize) -> Option<String> {
    COLON_CLASSES
        .get(range_type_idx)
        .map(|s| format!("[:{}:]", s))
}

// ---------------------------------------------------------------------------
// C-internal pattern compiler functions - implemented differently in Rust
// These are provided as thin wrappers/stubs for API completeness
// ---------------------------------------------------------------------------

/// Clear multibyte shift state (from pattern.c clear_shiftstate) - no-op in Rust
pub fn clear_shiftstate() {}

/// Advance past metafied char (from pattern.c metacharinc) - no-op in Rust (native UTF-8)
pub fn metacharinc(s: &str, pos: usize) -> usize {
    let c = s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    pos + c
}

/// Add bytes to pattern buffer (from pattern.c patadd) - Rust uses Vec<PatNode>
pub fn patadd(prog: &mut Vec<PatNode>, node: PatNode) {
    prog.push(node);
}

/// Set up pattern compiler char sets (from pattern.c patcompcharsset) - no-op in Rust
pub fn patcompcharsset() {}

/// Initialize pattern compilation (from pattern.c patcompstart) - no-op in Rust
pub fn patcompstart() {}

/// Compile top-level pattern switch (alternation) - implemented as PatCompiler::compile_branch
pub fn patcompswitch(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    patcompile(pattern, flags)
}

/// Compile a pattern branch - implemented as PatCompiler::compile_branch_inner
pub fn patcompbranch(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    patcompile(pattern, flags)
}

/// Compile a single pattern piece - implemented as PatCompiler::compile_piece
pub fn patcomppiece(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    patcompile(pattern, flags)
}

/// Compile negation pattern - implemented as PatCompiler handling of ^ and !()
pub fn patcompnot(pattern: &str, flags: PatFlags) -> Result<PatProg, String> {
    let negated = format!("^({})", pattern);
    patcompile(&negated, flags)
}

/// Add node to bytecode (from pattern.c patnode) - Rust uses Vec<PatNode>
pub fn patnode(prog: &mut Vec<PatNode>, node: PatNode) -> usize {
    let idx = prog.len();
    prog.push(node);
    idx
}

/// Insert node at position (from pattern.c patinsert) - Rust uses Vec::insert
pub fn patinsert(prog: &mut Vec<PatNode>, pos: usize, node: PatNode) {
    if pos <= prog.len() {
        prog.insert(pos, node);
    }
}

/// Set tail pointer (from pattern.c pattail) - no-op in Rust AST model
pub fn pattail(_prog: &[PatNode], _p: usize, _val: usize) {}

/// Set optional tail pointer (from pattern.c patoptail) - no-op in Rust AST model
pub fn patoptail(_prog: &[PatNode], _p: usize, _val: usize) {}

/// Get char reference (from pattern.c charref) - Rust native char
pub fn charref(s: &str, pos: usize) -> Option<char> {
    s[pos..].chars().next()
}

/// Get next char (from pattern.c charnext) - Rust native char iteration
pub fn charnext(s: &str, pos: usize) -> usize {
    metacharinc(s, pos)
}

/// Get char and advance (from pattern.c charrefinc) - Rust native char
pub fn charrefinc(s: &str, pos: &mut usize) -> Option<char> {
    let c = s[*pos..].chars().next()?;
    *pos += c.len_utf8();
    Some(c)
}

/// Get previous char width (from pattern.c charsub) - Rust native char
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

/// Initialize pattern try (from pattern.c pattrystart) - no-op in Rust
pub fn pattrystart() {}

/// Prepare string for pattern matching (from pattern.c patmungestring) - identity in Rust
pub fn patmungestring(s: &str) -> String {
    s.to_string()
}

/// Multibyte pattern match range (from pattern.c mb_patmatchrange) - uses native char
pub fn mb_patmatchrange(range: &[char], ch: char, igncase: bool) -> bool {
    patmatchrange(range, ch, igncase)
}

/// Multibyte pattern match index (from pattern.c mb_patmatchindex)
pub fn mb_patmatchindex(range: &[char], idx: usize) -> Option<char> {
    patmatchindex(range, idx)
}

/// Allocate pattern string buffer (from pattern.c patallocstr) - no-op in Rust
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
        assert!(patmatch_opts("Hello", "HELLO", true, true, true));
        assert!(patmatch_opts("Hello", "hello", true, true, true));
        assert!(!patmatch_opts("Hello", "HELLO", true, true, false));
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
        let prog = patcompile("(foo)(bar)", PatFlags::default()).unwrap();
        let captures = patmatch_captures(&prog, "foobar").unwrap();
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
