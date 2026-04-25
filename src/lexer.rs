//! Zsh lexical analyzer - Direct port from zsh/Src/lex.c
//!
//! This lexer tokenizes zsh shell input into a stream of tokens.
//! It handles all zsh-specific syntax including:
//! - Single/double/dollar quotes
//! - Command substitution $(...)  and `...`
//! - Arithmetic $((...))
//! - Parameter expansion ${...}
//! - Process substitution <(...) >(...)
//! - Here documents
//! - All redirection operators
//! - Comments
//! - Continuation lines

use crate::tokens::{char_tokens, LexTok};
use std::collections::VecDeque;

/// Lexer flags controlling behavior
#[derive(Debug, Clone, Copy, Default)]
pub struct LexFlags {
    /// Parsing for ZLE (line editor) completion
    pub zle: bool,
    /// Return newlines as tokens
    pub newline: bool,
    /// Preserve comments in output
    pub comments_keep: bool,
    /// Strip comments from output
    pub comments_strip: bool,
    /// Active lexing (from bufferwords)
    pub active: bool,
}

/// Buffer state for building tokens
#[derive(Debug, Clone)]
struct LexBuf {
    data: String,
    siz: usize,
}

impl LexBuf {
    fn new() -> Self {
        LexBuf {
            data: String::with_capacity(256),
            siz: 256,
        }
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    fn add(&mut self, c: char) {
        self.data.push(c);
        if self.data.len() >= self.siz {
            self.siz *= 2;
            self.data.reserve(self.siz - self.data.len());
        }
    }

    #[allow(dead_code)]
    fn add_str(&mut self, s: &str) {
        self.data.push_str(s);
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn as_str(&self) -> &str {
        &self.data
    }

    #[allow(dead_code)]
    fn into_string(self) -> String {
        self.data
    }

    #[allow(dead_code)]
    fn last_char(&self) -> Option<char> {
        self.data.chars().last()
    }

    fn pop(&mut self) -> Option<char> {
        self.data.pop()
    }
}

/// Here-document state
#[derive(Debug, Clone)]
pub struct HereDoc {
    pub terminator: String,
    pub strip_tabs: bool,
    pub content: String,
}

/// The Zsh Lexer
pub struct ZshLexer<'a> {
    /// Input source
    input: &'a str,
    /// Current position in input
    pos: usize,
    /// Look-ahead buffer for ungotten characters
    unget_buf: VecDeque<char>,
    /// Current token string
    pub tokstr: Option<String>,
    /// Current token type
    pub tok: LexTok,
    /// File descriptor for redirections (e.g., 2> means fd=2)
    pub tokfd: i32,
    /// Line number at start of current token
    pub toklineno: u64,
    /// Current line number
    pub lineno: u64,
    /// Lexer has stopped (EOF or error)
    pub lexstop: bool,
    /// In command position (can accept reserved words)
    pub incmdpos: bool,
    /// In condition [[ ... ]]
    pub incond: i32,
    /// In pattern context (RHS of == != =~ in [[ ]])
    pub incondpat: bool,
    /// In case pattern
    pub incasepat: i32,
    /// In redirection
    pub inredir: bool,
    /// After 'for' keyword
    pub infor: i32,
    /// After 'repeat' keyword
    inrepeat: i32,
    /// Parsing typeset arguments
    pub intypeset: bool,
    /// Inside (( ... )) arithmetic
    dbparens: bool,
    /// Disable alias expansion
    pub noaliases: bool,
    /// Disable spelling correction
    pub nocorrect: i32,
    /// Disable comment recognition
    pub nocomments: bool,
    /// Lexer flags
    pub lexflags: LexFlags,
    /// Whether this is the first line
    pub isfirstln: bool,
    /// Whether this is the first char of command
    #[allow(dead_code)]
    isfirstch: bool,
    /// Pending here-documents
    pub heredocs: Vec<HereDoc>,
    /// Expecting heredoc terminator (0 = no, 1 = <<, 2 = <<-)
    heredoc_pending: u8,
    /// Token buffer
    lexbuf: LexBuf,
    /// After newline
    pub isnewlin: i32,
    /// Error message if any
    pub error: Option<String>,
    /// Global iteration counter for infinite loop detection
    global_iterations: usize,
    /// Recursion depth counter
    recursion_depth: usize,
}

const MAX_LEXER_RECURSION: usize = 200;

impl<'a> ZshLexer<'a> {
    /// Create a new lexer for the given input
    pub fn new(input: &'a str) -> Self {
        ZshLexer {
            input,
            pos: 0,
            unget_buf: VecDeque::new(),
            tokstr: None,
            tok: LexTok::Endinput,
            tokfd: -1,
            toklineno: 1,
            lineno: 1,
            lexstop: false,
            incmdpos: true,
            incond: 0,
            incondpat: false,
            incasepat: 0,
            inredir: false,
            infor: 0,
            inrepeat: 0,
            intypeset: false,
            dbparens: false,
            noaliases: false,
            nocorrect: 0,
            nocomments: false,
            lexflags: LexFlags::default(),
            isfirstln: true,
            isfirstch: true,
            heredocs: Vec::new(),
            heredoc_pending: 0,
            lexbuf: LexBuf::new(),
            isnewlin: 0,
            error: None,
            global_iterations: 0,
            recursion_depth: 0,
        }
    }

    /// Check recursion depth; returns true if exceeded
    #[inline]
    fn check_recursion(&mut self) -> bool {
        if self.recursion_depth > MAX_LEXER_RECURSION {
            self.error = Some("lexer exceeded max recursion depth".to_string());
            self.lexstop = true;
            true
        } else {
            false
        }
    }

    /// Check and increment global iteration counter; returns true if limit exceeded
    #[inline]
    fn check_iterations(&mut self) -> bool {
        self.global_iterations += 1;
        if self.global_iterations > 50_000 {
            self.error = Some("lexer exceeded 50K iterations".to_string());
            self.lexstop = true;
            self.tok = LexTok::Lexerr;
            true
        } else {
            false
        }
    }

    /// Get next character from input
    fn hgetc(&mut self) -> Option<char> {
        if self.check_iterations() {
            return None;
        }

        if let Some(c) = self.unget_buf.pop_front() {
            return Some(c);
        }

        let c = self.input[self.pos..].chars().next()?;
        self.pos += c.len_utf8();

        if c == '\n' {
            self.lineno += 1;
        }

        Some(c)
    }

    /// Put character back into input
    fn hungetc(&mut self, c: char) {
        self.unget_buf.push_front(c);
        if c == '\n' && self.lineno > 1 {
            self.lineno -= 1;
        }
        self.lexstop = false;
    }

    /// Peek at next character without consuming
    #[allow(dead_code)]
    fn peek(&mut self) -> Option<char> {
        if let Some(&c) = self.unget_buf.front() {
            return Some(c);
        }
        self.input[self.pos..].chars().next()
    }

    /// Add character to token buffer
    fn add(&mut self, c: char) {
        self.lexbuf.add(c);
    }

    /// Check if character is blank (space or tab)
    fn is_blank(c: char) -> bool {
        c == ' ' || c == '\t'
    }

    /// Check if character is blank (including other whitespace except newline)
    fn is_inblank(c: char) -> bool {
        matches!(c, ' ' | '\t' | '\x0b' | '\x0c' | '\r')
    }

    /// Check if character is a digit
    fn is_digit(c: char) -> bool {
        c.is_ascii_digit()
    }

    /// Check if character is identifier start
    #[allow(dead_code)]
    fn is_ident_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }

    /// Check if character is identifier continuation
    fn is_ident(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    /// Main lexer entry point - get next token
    pub fn zshlex(&mut self) {
        if self.tok == LexTok::Lexerr {
            return;
        }

        // Note: Do NOT reset global_iterations here - it must accumulate across all
        // zshlex calls in a parse to prevent infinite loops in the parser

        loop {
            if self.inrepeat > 0 {
                self.inrepeat += 1;
            }
            if self.inrepeat == 3 {
                self.incmdpos = true;
            }

            self.tok = self.gettok();

            // Handle alias expansion would go here
            break;
        }

        self.nocorrect &= 1;

        // Handle here-documents at end of line
        if self.tok == LexTok::Newlin || self.tok == LexTok::Endinput {
            self.process_heredocs();
        }

        if self.tok != LexTok::Newlin {
            self.isnewlin = 0;
        } else {
            self.isnewlin = if self.pos < self.input.len() { -1 } else { 1 };
        }

        if self.tok == LexTok::Semi || (self.tok == LexTok::Newlin && !self.lexflags.newline) {
            self.tok = LexTok::Seper;
        }

        // Check for reserved words when in command position
        // Also check for "{" and "}" which are special in many contexts
        if self.tok == LexTok::String {
            if let Some(ref s) = self.tokstr {
                if s == "{" {
                    self.tok = LexTok::Inbrace;
                } else if s == "}" {
                    self.tok = LexTok::Outbrace;
                } else if self.incasepat == 0 {
                    // Skip reserved word checking in case pattern context
                    // Words like "time", "end", etc. should be patterns, not reserved words
                    self.check_reserved_word();
                }
            }
        }

        // If we were expecting a heredoc terminator, register it now
        if self.heredoc_pending > 0 && self.tok == LexTok::String {
            if let Some(ref terminator) = self.tokstr {
                let strip_tabs = self.heredoc_pending == 2;
                // Handle quoted terminators (e.g., 'EOF' or "EOF")
                let term = terminator
                    .trim_matches(|c| c == '\'' || c == '"')
                    .to_string();
                self.heredocs.push(HereDoc {
                    terminator: term,
                    strip_tabs,
                    content: String::new(),
                });
            }
            self.heredoc_pending = 0;
        }

        // Track pattern context inside [[ ... ]] - after = == != =~ the RHS is a pattern
        if self.incond > 0 {
            if let Some(ref s) = self.tokstr {
                // Check if this token is a comparison operator
                // Note: single = is also a comparison operator in [[ ]]
                // The internal marker \u{8d} is used for =
                if s == "="
                    || s == "=="
                    || s == "!="
                    || s == "=~"
                    || s == "\u{8d}"
                    || s == "\u{8d}\u{8d}"
                    || s == "!\u{8d}"
                    || s == "\u{8d}~"
                {
                    self.incondpat = true;
                } else if self.incondpat {
                    // We were in pattern context, now we've consumed the pattern
                    // Reset after the pattern token is consumed
                    // But actually, pattern can span multiple tokens, so we should
                    // stay in pattern mode until ]] or && or ||
                }
            }
            // Reset pattern context on ]] or logical operators
            if self.tok == LexTok::Doutbrack {
                self.incondpat = false;
            }
        } else {
            self.incondpat = false;
        }

        // Update command position for next token based on current token
        // Note: In case patterns (incasepat > 0), | is a pattern separator, not pipeline,
        // so we don't set incmdpos after Bar in that context
        match self.tok {
            LexTok::Seper
            | LexTok::Newlin
            | LexTok::Semi
            | LexTok::Dsemi
            | LexTok::Semiamp
            | LexTok::Semibar
            | LexTok::Amper
            | LexTok::Amperbang
            | LexTok::Inpar
            | LexTok::Inbrace
            | LexTok::Dbar
            | LexTok::Damper
            | LexTok::Baramp
            | LexTok::Inoutpar
            | LexTok::Doloop
            | LexTok::Then
            | LexTok::Elif
            | LexTok::Else
            | LexTok::Doutbrack
            | LexTok::Func => {
                self.incmdpos = true;
            }
            LexTok::Bar => {
                // In case patterns, | is a pattern separator - don't change incmdpos
                if self.incasepat <= 0 {
                    self.incmdpos = true;
                }
            }
            LexTok::String
            | LexTok::Typeset
            | LexTok::Envarray
            | LexTok::Outpar
            | LexTok::Case
            | LexTok::Dinbrack => {
                self.incmdpos = false;
            }
            _ => {}
        }

        // Track 'for' keyword for C-style for loop: for (( init; cond; step ))
        // When we see 'for', set infor=2 to expect the init and cond parts
        // Each Dinpar (after semicolon in arithmetic) decrements it
        if self.tok != LexTok::Dinpar {
            self.infor = if self.tok == LexTok::For { 2 } else { 0 };
        }

        // Handle redirection context
        let oldpos = self.incmdpos;
        if self.tok.is_redirop()
            || self.tok == LexTok::For
            || self.tok == LexTok::Foreach
            || self.tok == LexTok::Select
        {
            self.inredir = true;
            self.incmdpos = false;
        } else if self.inredir {
            self.incmdpos = oldpos;
            self.inredir = false;
        }
    }

    /// Process pending here-documents
    fn process_heredocs(&mut self) {
        let heredocs = std::mem::take(&mut self.heredocs);

        for mut hdoc in heredocs {
            let mut content = String::new();
            let mut line_count = 0;

            loop {
                line_count += 1;
                if line_count > 10000 {
                    self.error = Some("heredoc exceeded 10000 lines".to_string());
                    self.tok = LexTok::Lexerr;
                    return;
                }

                let line = self.read_line();
                if line.is_none() {
                    self.error = Some("here document too large or unterminated".to_string());
                    self.tok = LexTok::Lexerr;
                    return;
                }

                let line = line.unwrap();
                let check_line = if hdoc.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    &line
                };

                if check_line.trim_end_matches('\n') == hdoc.terminator {
                    break;
                }

                content.push_str(&line);
            }

            hdoc.content = content;
        }
    }

    /// Read a line from input (returns partial line at EOF)
    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();

        loop {
            match self.hgetc() {
                Some(c) => {
                    line.push(c);
                    if c == '\n' {
                        break;
                    }
                }
                None => {
                    // EOF - return partial line if any
                    if line.is_empty() {
                        return None;
                    }
                    break;
                }
            }
        }

        Some(line)
    }

    /// Get the next token
    fn gettok(&mut self) -> LexTok {
        self.tokstr = None;
        self.tokfd = -1;

        // Skip whitespace
        let mut ws_iterations = 0;
        loop {
            ws_iterations += 1;
            if ws_iterations > 100_000 {
                self.error = Some("gettok: infinite loop in whitespace skip".to_string());
                return LexTok::Lexerr;
            }
            let c = match self.hgetc() {
                Some(c) => c,
                None => {
                    self.lexstop = true;
                    return if self.error.is_some() {
                        LexTok::Lexerr
                    } else {
                        LexTok::Endinput
                    };
                }
            };

            if !Self::is_blank(c) {
                self.hungetc(c);
                break;
            }
        }

        let c = match self.hgetc() {
            Some(c) => c,
            None => {
                self.lexstop = true;
                return LexTok::Endinput;
            }
        };

        self.toklineno = self.lineno;
        self.isfirstln = false;

        // Handle (( ... )) arithmetic
        if self.dbparens {
            return self.lex_arith(c);
        }

        // Handle digit followed by redirection
        if Self::is_digit(c) {
            let d = self.hgetc();
            match d {
                Some('&') => {
                    let e = self.hgetc();
                    if e == Some('>') {
                        self.tokfd = (c as u8 - b'0') as i32;
                        self.hungetc('>');
                        return self.lex_initial('&');
                    }
                    if let Some(e) = e {
                        self.hungetc(e);
                    }
                    self.hungetc('&');
                }
                Some('>') | Some('<') => {
                    self.tokfd = (c as u8 - b'0') as i32;
                    return self.lex_initial(d.unwrap());
                }
                Some(d) => {
                    self.hungetc(d);
                }
                None => {}
            }
            self.lexstop = false;
        }

        self.lex_initial(c)
    }

    /// Lex (( ... )) arithmetic expression
    fn lex_arith(&mut self, c: char) -> LexTok {
        self.lexbuf.clear();
        self.hungetc(c);

        let end_char = if self.infor > 0 { ';' } else { ')' };
        if self.dquote_parse(end_char, false).is_err() {
            return LexTok::Lexerr;
        }

        self.tokstr = Some(self.lexbuf.as_str().to_string());

        if !self.lexstop && self.infor > 0 {
            self.infor -= 1;
            return LexTok::Dinpar;
        }

        // Check for closing ))
        match self.hgetc() {
            Some(')') => {
                self.dbparens = false;
                LexTok::Doutpar
            }
            c => {
                if let Some(c) = c {
                    self.hungetc(c);
                }
                LexTok::Lexerr
            }
        }
    }

    /// Handle initial character of token
    fn lex_initial(&mut self, c: char) -> LexTok {
        // Handle comments
        if c == '#' && !self.nocomments {
            return self.lex_comment();
        }

        match c {
            '\\' => {
                let d = self.hgetc();
                if d == Some('\n') {
                    // Line continuation - get next token
                    return self.gettok();
                }
                if let Some(d) = d {
                    self.hungetc(d);
                }
                self.lexstop = false;
                self.gettokstr(c, false)
            }

            '\n' => LexTok::Newlin,

            ';' => {
                let d = self.hgetc();
                match d {
                    Some(';') => LexTok::Dsemi,
                    Some('&') => LexTok::Semiamp,
                    Some('|') => LexTok::Semibar,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        LexTok::Semi
                    }
                }
            }

            '&' => {
                let d = self.hgetc();
                match d {
                    Some('&') => LexTok::Damper,
                    Some('!') | Some('|') => LexTok::Amperbang,
                    Some('>') => {
                        self.tokfd = self.tokfd.max(0);
                        let e = self.hgetc();
                        match e {
                            Some('!') | Some('|') => LexTok::Outangampbang,
                            Some('>') => {
                                let f = self.hgetc();
                                match f {
                                    Some('!') | Some('|') => LexTok::Doutangampbang,
                                    _ => {
                                        if let Some(f) = f {
                                            self.hungetc(f);
                                        }
                                        self.lexstop = false;
                                        LexTok::Doutangamp
                                    }
                                }
                            }
                            _ => {
                                if let Some(e) = e {
                                    self.hungetc(e);
                                }
                                self.lexstop = false;
                                LexTok::Ampoutang
                            }
                        }
                    }
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        LexTok::Amper
                    }
                }
            }

            '|' => {
                let d = self.hgetc();
                match d {
                    Some('|') if self.incasepat <= 0 => LexTok::Dbar,
                    Some('&') => LexTok::Baramp,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        LexTok::Bar
                    }
                }
            }

            '(' => {
                let d = self.hgetc();
                match d {
                    Some('(') => {
                        if self.infor > 0 {
                            self.dbparens = true;
                            return LexTok::Dinpar;
                        }
                        if self.incmdpos {
                            // Could be (( arithmetic )) or ( subshell )
                            self.lexbuf.clear();
                            match self.cmd_or_math() {
                                CmdOrMath::Math => {
                                    self.tokstr = Some(self.lexbuf.as_str().to_string());
                                    return LexTok::Dinpar;
                                }
                                CmdOrMath::Cmd => {
                                    self.tokstr = None;
                                    return LexTok::Inpar;
                                }
                                CmdOrMath::Err => return LexTok::Lexerr,
                            }
                        }
                        self.hungetc('(');
                        self.lexstop = false;
                        self.gettokstr('(', false)
                    }
                    Some(')') => LexTok::Inoutpar,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        // In pattern context (after == != =~ in [[ ]]), ( is part of pattern
                        // In case pattern context, ( at start is optional delimiter, not pattern
                        // incasepat == 1 means "at start of pattern", > 1 means "inside pattern"
                        if self.incondpat || self.incasepat > 1 {
                            self.gettokstr('(', false)
                        } else if self.incond == 1 || self.incmdpos || self.incasepat == 1 {
                            LexTok::Inpar
                        } else {
                            self.gettokstr('(', false)
                        }
                    }
                }
            }

            ')' => LexTok::Outpar,

            '{' => {
                // { is a command group only if followed by whitespace or newline
                // {a,b} is brace expansion, not a command group
                if self.incmdpos {
                    let next = self.hgetc();
                    let is_brace_group = match next {
                        Some(' ') | Some('\t') | Some('\n') | None => true,
                        _ => false,
                    };
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    if is_brace_group {
                        self.tokstr = Some("{".to_string());
                        LexTok::Inbrace
                    } else {
                        self.gettokstr(c, false)
                    }
                } else {
                    self.gettokstr(c, false)
                }
            }

            '}' => {
                // } at start of token is always Outbrace (ends command group)
                // Inside a word, } would be handled by gettokstr but we never reach here mid-word
                self.tokstr = Some("}".to_string());
                LexTok::Outbrace
            }

            '[' => {
                // [[ is a conditional expression start
                // [ can also be a command (test builtin) or array subscript
                // In case patterns (incasepat > 0), [ is part of glob pattern like [yY]
                if self.incasepat > 0 {
                    self.gettokstr(c, false)
                } else if self.incmdpos {
                    let next = self.hgetc();
                    if next == Some('[') {
                        // [[ - double bracket conditional
                        self.tokstr = Some("[[".to_string());
                        self.incond = 1;
                        return LexTok::Dinbrack;
                    }
                    // Single [ - either test command or start of glob pattern
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    self.tokstr = Some("[".to_string());
                    LexTok::String
                } else {
                    self.gettokstr(c, false)
                }
            }

            ']' => {
                // ]] ends a conditional expression started by [[
                if self.incond > 0 {
                    let next = self.hgetc();
                    if next == Some(']') {
                        self.tokstr = Some("]]".to_string());
                        self.incond = 0;
                        return LexTok::Doutbrack;
                    }
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                }
                self.gettokstr(c, false)
            }

            '<' => {
                // In pattern context, < is literal (e.g., <-> in glob)
                if self.incondpat || self.incasepat > 0 {
                    self.gettokstr(c, false)
                } else {
                    self.lex_inang()
                }
            }

            '>' => {
                // In pattern context, > is literal
                if self.incondpat || self.incasepat > 0 {
                    self.gettokstr(c, false)
                } else {
                    self.lex_outang()
                }
            }

            _ => self.gettokstr(c, false),
        }
    }

    /// Lex comment
    fn lex_comment(&mut self) -> LexTok {
        if self.lexflags.comments_keep {
            self.lexbuf.clear();
            self.add('#');
        }

        loop {
            let c = self.hgetc();
            match c {
                Some('\n') | None => break,
                Some(c) => {
                    if self.lexflags.comments_keep {
                        self.add(c);
                    }
                }
            }
        }

        if self.lexflags.comments_keep {
            self.tokstr = Some(self.lexbuf.as_str().to_string());
            if !self.lexstop {
                self.hungetc('\n');
            }
            return LexTok::String;
        }

        if self.lexflags.comments_strip && self.lexstop {
            return LexTok::Endinput;
        }

        LexTok::Newlin
    }

    /// Lex < and variants
    fn lex_inang(&mut self) -> LexTok {
        let d = self.hgetc();
        match d {
            Some('(') => {
                // Process substitution <(...)
                self.hungetc('(');
                self.lexstop = false;
                return self.gettokstr('<', false);
            }
            Some('>') => return LexTok::Inoutang,
            Some('<') => {
                let e = self.hgetc();
                match e {
                    Some('(') => {
                        self.hungetc('(');
                        self.hungetc('<');
                        return LexTok::Inang;
                    }
                    Some('<') => return LexTok::Trinang,
                    Some('-') => {
                        self.heredoc_pending = 2; // <<- expects terminator next
                        return LexTok::Dinangdash;
                    }
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        self.lexstop = false;
                        self.heredoc_pending = 1; // << expects terminator next
                        return LexTok::Dinang;
                    }
                }
            }
            Some('&') => return LexTok::Inangamp,
            _ => {
                if let Some(d) = d {
                    self.hungetc(d);
                }
                self.lexstop = false;
                return LexTok::Inang;
            }
        }
    }

    /// Lex > and variants
    fn lex_outang(&mut self) -> LexTok {
        let d = self.hgetc();
        match d {
            Some('(') => {
                // Process substitution >(...)
                self.hungetc('(');
                self.lexstop = false;
                return self.gettokstr('>', false);
            }
            Some('&') => {
                let e = self.hgetc();
                match e {
                    Some('!') | Some('|') => return LexTok::Outangampbang,
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        self.lexstop = false;
                        return LexTok::Outangamp;
                    }
                }
            }
            Some('!') | Some('|') => return LexTok::Outangbang,
            Some('>') => {
                let e = self.hgetc();
                match e {
                    Some('&') => {
                        let f = self.hgetc();
                        match f {
                            Some('!') | Some('|') => return LexTok::Doutangampbang,
                            _ => {
                                if let Some(f) = f {
                                    self.hungetc(f);
                                }
                                self.lexstop = false;
                                return LexTok::Doutangamp;
                            }
                        }
                    }
                    Some('!') | Some('|') => return LexTok::Doutangbang,
                    Some('(') => {
                        self.hungetc('(');
                        self.hungetc('>');
                        return LexTok::Outang;
                    }
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        self.lexstop = false;
                        return LexTok::Doutang;
                    }
                }
            }
            _ => {
                if let Some(d) = d {
                    self.hungetc(d);
                }
                self.lexstop = false;
                return LexTok::Outang;
            }
        }
    }

    /// Get rest of token string
    fn gettokstr(&mut self, c: char, sub: bool) -> LexTok {
        let mut bct = 0; // brace count
        let mut pct = 0; // parenthesis count
        let mut brct = 0; // bracket count
        let mut in_brace_param = 0;
        let mut peek = LexTok::String;
        let mut intpos = 1;
        let mut unmatched = '\0';
        let mut c = c;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        if !sub {
            self.lexbuf.clear();
        }

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error = Some("gettokstr exceeded maximum iterations".to_string());
                return LexTok::Lexerr;
            }

            let inbl = Self::is_inblank(c);

            if inbl && in_brace_param == 0 && pct == 0 {
                // Whitespace outside brace param ends token
                break;
            }

            match c {
                // Whitespace is handled above for most cases
                ')' => {
                    if in_brace_param > 0 || sub {
                        self.add(char_tokens::OUTPAR);
                    } else if pct > 0 {
                        pct -= 1;
                        self.add(char_tokens::OUTPAR);
                    } else {
                        break;
                    }
                }

                '|' => {
                    if pct == 0 && in_brace_param == 0 {
                        if sub {
                            self.add(c);
                        } else {
                            break;
                        }
                    } else {
                        self.add(char_tokens::BAR);
                    }
                }

                '$' => {
                    let e = self.hgetc();
                    match e {
                        Some('\\') => {
                            let f = self.hgetc();
                            if f != Some('\n') {
                                if let Some(f) = f {
                                    self.hungetc(f);
                                }
                                self.hungetc('\\');
                                self.add(char_tokens::STRING);
                            } else {
                                // Line continuation after $
                                continue;
                            }
                        }
                        Some('[') => {
                            // $[...] arithmetic
                            self.add(char_tokens::STRING);
                            self.add(char_tokens::INBRACK);
                            if self.dquote_parse(']', sub).is_err() {
                                peek = LexTok::Lexerr;
                                break;
                            }
                            self.add(char_tokens::OUTBRACK);
                        }
                        Some('(') => {
                            // $(...) or $((...))
                            self.add(char_tokens::STRING);
                            match self.cmd_or_math_sub() {
                                CmdOrMath::Cmd => self.add(char_tokens::OUTPAR),
                                CmdOrMath::Math => self.add(char_tokens::OUTPARMATH),
                                CmdOrMath::Err => {
                                    peek = LexTok::Lexerr;
                                    break;
                                }
                            }
                        }
                        Some('{') => {
                            self.add(c);
                            self.add(char_tokens::INBRACE);
                            bct += 1;
                            if in_brace_param == 0 {
                                in_brace_param = bct;
                            }
                        }
                        _ => {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            self.lexstop = false;
                            self.add(char_tokens::STRING);
                        }
                    }
                }

                '[' => {
                    if in_brace_param == 0 {
                        brct += 1;
                    }
                    self.add(char_tokens::INBRACK);
                }

                ']' => {
                    if in_brace_param == 0 && brct > 0 {
                        brct -= 1;
                    }
                    self.add(char_tokens::OUTBRACK);
                }

                '(' => {
                    if in_brace_param == 0 {
                        pct += 1;
                    }
                    self.add(char_tokens::INPAR);
                }

                '{' => {
                    // Track braces for both ${...} param expansion and {...} brace expansion
                    bct += 1;
                    self.add(c);
                }

                '}' => {
                    if in_brace_param > 0 {
                        if bct == in_brace_param {
                            in_brace_param = 0;
                        }
                        bct -= 1;
                        self.add(char_tokens::OUTBRACE);
                    } else if bct > 0 {
                        // Closing a brace expansion like {a,b}
                        bct -= 1;
                        self.add(c);
                    } else {
                        break;
                    }
                }

                '>' => {
                    // In pattern context (incondpat), > is literal
                    if in_brace_param > 0 || sub || self.incondpat || self.incasepat > 0 {
                        self.add(c);
                    } else {
                        let e = self.hgetc();
                        if e != Some('(') {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            self.lexstop = false;
                            break;
                        }
                        // >(...)
                        self.add(char_tokens::OUTANGPROC);
                        if self.skip_command_sub().is_err() {
                            peek = LexTok::Lexerr;
                            break;
                        }
                        self.add(char_tokens::OUTPAR);
                    }
                }

                '<' => {
                    // In pattern context (incondpat), < is literal
                    if in_brace_param > 0 || sub || self.incondpat || self.incasepat > 0 {
                        self.add(c);
                    } else {
                        let e = self.hgetc();
                        if e != Some('(') {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            self.lexstop = false;
                            break;
                        }
                        // <(...)
                        self.add(char_tokens::INANG);
                        if self.skip_command_sub().is_err() {
                            peek = LexTok::Lexerr;
                            break;
                        }
                        self.add(char_tokens::OUTPAR);
                    }
                }

                '=' => {
                    if !sub {
                        if intpos > 0 {
                            // At start of token, check for =(...) process substitution
                            let e = self.hgetc();
                            if e == Some('(') {
                                self.add(char_tokens::EQUALS);
                                if self.skip_command_sub().is_err() {
                                    peek = LexTok::Lexerr;
                                    break;
                                }
                                self.add(char_tokens::OUTPAR);
                            } else {
                                if let Some(e) = e {
                                    self.hungetc(e);
                                }
                                self.lexstop = false;
                                self.add(char_tokens::EQUALS);
                            }
                        } else if peek != LexTok::Envstring
                            && (self.incmdpos || self.intypeset)
                            && bct == 0
                            && brct == 0
                            && self.incasepat == 0
                        {
                            // Check for VAR=value assignment (but not in case pattern context)
                            let tok_so_far = self.lexbuf.as_str().to_string();
                            if self.is_valid_assignment_target(&tok_so_far) {
                                let next = self.hgetc();
                                if next == Some('(') {
                                    // VAR=(...) array assignment - include '=' in tokstr
                                    self.add(char_tokens::EQUALS);
                                    self.tokstr = Some(self.lexbuf.as_str().to_string());
                                    return LexTok::Envarray;
                                }
                                if let Some(next) = next {
                                    self.hungetc(next);
                                }
                                self.lexstop = false;
                                peek = LexTok::Envstring;
                                intpos = 2;
                                self.add(char_tokens::EQUALS);
                            } else {
                                self.add(char_tokens::EQUALS);
                            }
                        } else {
                            self.add(char_tokens::EQUALS);
                        }
                    } else {
                        self.add(char_tokens::EQUALS);
                    }
                }

                '\\' => {
                    let next = self.hgetc();
                    if next == Some('\n') {
                        // Line continuation
                        let next = self.hgetc();
                        if let Some(next) = next {
                            c = next;
                            continue;
                        }
                        break;
                    } else {
                        self.add(char_tokens::BNULL);
                        if let Some(next) = next {
                            self.add(next);
                        }
                    }
                }

                '\'' => {
                    // Single quoted string - everything literal until '
                    self.add(char_tokens::SNULL);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\'') => break,
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                unmatched = '\'';
                                peek = LexTok::Lexerr;
                                break;
                            }
                        }
                    }
                    if unmatched != '\0' {
                        break;
                    }
                    self.add(char_tokens::SNULL);
                }

                '"' => {
                    // Double quoted string
                    self.add(char_tokens::DNULL);
                    if self.dquote_parse('"', sub).is_err() {
                        unmatched = '"';
                        if !self.lexflags.active {
                            peek = LexTok::Lexerr;
                        }
                        break;
                    }
                    self.add(char_tokens::DNULL);
                }

                '`' => {
                    // Backtick command substitution
                    self.add(char_tokens::TICK);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('`') => break,
                            Some('\\') => {
                                let next = self.hgetc();
                                match next {
                                    Some('\n') => continue, // Line continuation
                                    Some(c) if c == '`' || c == '\\' || c == '$' => {
                                        self.add(char_tokens::BNULL);
                                        self.add(c);
                                    }
                                    Some(c) => {
                                        self.add('\\');
                                        self.add(c);
                                    }
                                    None => break,
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                unmatched = '`';
                                peek = LexTok::Lexerr;
                                break;
                            }
                        }
                    }
                    if unmatched != '\0' {
                        break;
                    }
                    self.add(char_tokens::TICK);
                }

                '~' => {
                    self.add(char_tokens::TILDE);
                }

                '#' => {
                    self.add(char_tokens::POUND);
                }

                '^' => {
                    self.add(char_tokens::HAT);
                }

                '*' => {
                    self.add(char_tokens::STAR);
                }

                '?' => {
                    self.add(char_tokens::QUEST);
                }

                ',' => {
                    if bct > in_brace_param {
                        self.add(char_tokens::COMMA);
                    } else {
                        self.add(c);
                    }
                }

                '-' => {
                    self.add(char_tokens::DASH);
                }

                '!' => {
                    if brct > 0 {
                        self.add(char_tokens::BANG);
                    } else {
                        self.add(c);
                    }
                }

                // Terminators
                '\n' | ';' | '&' => {
                    break;
                }

                _ => {
                    self.add(c);
                }
            }

            c = match self.hgetc() {
                Some(c) => c,
                None => {
                    self.lexstop = true;
                    break;
                }
            };

            if intpos > 0 {
                intpos -= 1;
            }
        }

        // Put back the character that ended the token
        if !self.lexstop {
            self.hungetc(c);
        }

        if unmatched != '\0' && !self.lexflags.active {
            self.error = Some(format!("unmatched {}", unmatched));
        }

        if in_brace_param > 0 {
            self.error = Some("closing brace expected".to_string());
        }

        self.tokstr = Some(self.lexbuf.as_str().to_string());
        peek
    }

    /// Check if a string is a valid assignment target (identifier or array ref)
    fn is_valid_assignment_target(&self, s: &str) -> bool {
        let mut chars = s.chars().peekable();

        // Check for leading digit (invalid)
        if let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                // Could be array index, check rest
                while let Some(&c) = chars.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    chars.next();
                }
                return chars.peek().is_none();
            }
        }

        // Check identifier
        let mut has_ident = false;
        while let Some(&c) = chars.peek() {
            if c == char_tokens::INBRACK || c == '[' {
                break;
            }
            if c == '+' {
                // foo+=value
                chars.next();
                return chars.peek().is_none() || chars.peek() == Some(&'=');
            }
            if !Self::is_ident(c) && c != char_tokens::STRING && !char_tokens::is_token(c) {
                return false;
            }
            has_ident = true;
            chars.next();
        }

        has_ident
    }

    /// Parse double-quoted string content
    fn dquote_parse(&mut self, endchar: char, sub: bool) -> Result<(), ()> {
        self.recursion_depth += 1;
        if self.check_recursion() {
            self.recursion_depth -= 1;
            return Err(());
        }

        let result = self.dquote_parse_inner(endchar, sub);
        self.recursion_depth -= 1;
        result
    }

    fn dquote_parse_inner(&mut self, endchar: char, sub: bool) -> Result<(), ()> {
        let mut pct = 0; // parenthesis count
        let mut brct = 0; // bracket count
        let mut bct = 0; // brace count (for ${...})
        let mut intick = false; // inside backtick
        let is_math = endchar == ')' || endchar == ']' || self.infor > 0;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error = Some("dquote_parse exceeded maximum iterations".to_string());
                return Err(());
            }
            let c = self.hgetc();
            let c = match c {
                Some(c) if c == endchar && !intick && bct == 0 => {
                    if is_math && (pct > 0 || brct > 0) {
                        self.add(c);
                        if c == ')' {
                            pct -= 1;
                        } else if c == ']' {
                            brct -= 1;
                        }
                        continue;
                    }
                    return Ok(());
                }
                Some(c) => c,
                None => {
                    self.lexstop = true;
                    return Err(());
                }
            };

            match c {
                '\\' => {
                    let next = self.hgetc();
                    match next {
                        Some('\n') if !sub => continue, // Line continuation
                        Some(c)
                            if c == '$'
                                || c == '\\'
                                || (c == '}' && !intick && bct > 0)
                                || c == endchar
                                || c == '`'
                                || (endchar == ']'
                                    && (c == '['
                                        || c == ']'
                                        || c == '('
                                        || c == ')'
                                        || c == '{'
                                        || c == '}'
                                        || (c == '"' && sub))) =>
                        {
                            self.add(char_tokens::BNULL);
                            self.add(c);
                        }
                        Some(c) => {
                            self.add('\\');
                            self.hungetc(c);
                            continue;
                        }
                        None => {
                            self.add('\\');
                        }
                    }
                }

                '$' => {
                    if intick {
                        self.add(c);
                        continue;
                    }
                    let next = self.hgetc();
                    match next {
                        Some('(') => {
                            self.add(char_tokens::QSTRING);
                            match self.cmd_or_math_sub() {
                                CmdOrMath::Cmd => self.add(char_tokens::OUTPAR),
                                CmdOrMath::Math => self.add(char_tokens::OUTPARMATH),
                                CmdOrMath::Err => return Err(()),
                            }
                        }
                        Some('[') => {
                            self.add(char_tokens::STRING);
                            self.add(char_tokens::INBRACK);
                            self.dquote_parse(']', sub)?;
                            self.add(char_tokens::OUTBRACK);
                        }
                        Some('{') => {
                            self.add(char_tokens::QSTRING);
                            self.add(char_tokens::INBRACE);
                            bct += 1;
                        }
                        Some('$') => {
                            self.add(char_tokens::QSTRING);
                            self.add('$');
                        }
                        _ => {
                            if let Some(next) = next {
                                self.hungetc(next);
                            }
                            self.lexstop = false;
                            self.add(char_tokens::QSTRING);
                        }
                    }
                }

                '}' => {
                    if intick || bct == 0 {
                        self.add(c);
                    } else {
                        self.add(char_tokens::OUTBRACE);
                        bct -= 1;
                    }
                }

                '`' => {
                    self.add(char_tokens::QTICK);
                    intick = !intick;
                }

                '(' => {
                    if !is_math || bct == 0 {
                        pct += 1;
                    }
                    self.add(c);
                }

                ')' => {
                    if !is_math || bct == 0 {
                        if pct == 0 && is_math {
                            return Err(());
                        }
                        pct -= 1;
                    }
                    self.add(c);
                }

                '[' => {
                    if !is_math || bct == 0 {
                        brct += 1;
                    }
                    self.add(c);
                }

                ']' => {
                    if !is_math || bct == 0 {
                        if brct == 0 && is_math {
                            return Err(());
                        }
                        brct -= 1;
                    }
                    self.add(c);
                }

                '"' => {
                    if intick || (endchar != '"' && bct == 0) {
                        self.add(c);
                    } else if bct > 0 {
                        self.add(char_tokens::DNULL);
                        self.dquote_parse('"', sub)?;
                        self.add(char_tokens::DNULL);
                    } else {
                        return Err(());
                    }
                }

                _ => {
                    self.add(c);
                }
            }
        }
    }

    /// Determine if (( is arithmetic or command
    fn cmd_or_math(&mut self) -> CmdOrMath {
        let oldlen = self.lexbuf.len();

        self.add(char_tokens::INPAR);
        self.add('(');

        if self.dquote_parse(')', false).is_err() {
            // Back up and try as command
            while self.lexbuf.len() > oldlen {
                if let Some(c) = self.lexbuf.pop() {
                    self.hungetc(c);
                }
            }
            self.hungetc('(');
            self.lexstop = false;
            return if self.skip_command_sub().is_err() {
                CmdOrMath::Err
            } else {
                CmdOrMath::Cmd
            };
        }

        // Check for closing )
        let c = self.hgetc();
        if c == Some(')') {
            self.add(')');
            return CmdOrMath::Math;
        }

        // Not math, back up
        if let Some(c) = c {
            self.hungetc(c);
        }
        self.lexstop = false;

        // Back up token
        while self.lexbuf.len() > oldlen {
            if let Some(c) = self.lexbuf.pop() {
                self.hungetc(c);
            }
        }
        self.hungetc('(');

        if self.skip_command_sub().is_err() {
            CmdOrMath::Err
        } else {
            CmdOrMath::Cmd
        }
    }

    /// Parse $(...) or $((...))
    fn cmd_or_math_sub(&mut self) -> CmdOrMath {
        const MAX_CONTINUATIONS: usize = 10_000;
        let mut continuations = 0;

        loop {
            continuations += 1;
            if continuations > MAX_CONTINUATIONS {
                self.error = Some("cmd_or_math_sub: too many line continuations".to_string());
                return CmdOrMath::Err;
            }

            let c = self.hgetc();
            if c == Some('\\') {
                let c2 = self.hgetc();
                if c2 != Some('\n') {
                    if let Some(c2) = c2 {
                        self.hungetc(c2);
                    }
                    self.hungetc('\\');
                    self.lexstop = false;
                    return if self.skip_command_sub().is_err() {
                        CmdOrMath::Err
                    } else {
                        CmdOrMath::Cmd
                    };
                }
                // Line continuation, try again (loop instead of recursion)
                continue;
            }

            // Not a line continuation, process normally
            if c == Some('(') {
                // Might be $((...))
                let lexpos = self.lexbuf.len();
                self.add(char_tokens::INPAR);
                self.add('(');

                if self.dquote_parse(')', false).is_ok() {
                    let c2 = self.hgetc();
                    if c2 == Some(')') {
                        self.add(')');
                        return CmdOrMath::Math;
                    }
                    if let Some(c2) = c2 {
                        self.hungetc(c2);
                    }
                }

                // Not math, restore and parse as command
                while self.lexbuf.len() > lexpos {
                    if let Some(ch) = self.lexbuf.pop() {
                        self.hungetc(ch);
                    }
                }
                self.hungetc('(');
                self.lexstop = false;
            } else {
                if let Some(c) = c {
                    self.hungetc(c);
                }
                self.lexstop = false;
            }

            return if self.skip_command_sub().is_err() {
                CmdOrMath::Err
            } else {
                CmdOrMath::Cmd
            };
        }
    }

    /// Skip over command substitution (...), adding chars to token
    fn skip_command_sub(&mut self) -> Result<(), ()> {
        let mut pct = 1;
        let mut start = true;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        self.add(char_tokens::INPAR);

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error = Some("skip_command_sub exceeded maximum iterations".to_string());
                return Err(());
            }

            let c = self.hgetc();
            let c = match c {
                Some(c) => c,
                None => {
                    self.lexstop = true;
                    return Err(());
                }
            };

            let iswhite = Self::is_inblank(c);

            match c {
                '(' => {
                    pct += 1;
                    self.add(c);
                }
                ')' => {
                    pct -= 1;
                    if pct == 0 {
                        return Ok(());
                    }
                    self.add(c);
                }
                '\\' => {
                    self.add(c);
                    if let Some(c) = self.hgetc() {
                        self.add(c);
                    }
                }
                '\'' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\'') => {
                                self.add('\'');
                                break;
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                return Err(());
                            }
                        }
                    }
                }
                '"' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('"') => {
                                self.add('"');
                                break;
                            }
                            Some('\\') => {
                                self.add('\\');
                                if let Some(ch) = self.hgetc() {
                                    self.add(ch);
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                return Err(());
                            }
                        }
                    }
                }
                '`' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('`') => {
                                self.add('`');
                                break;
                            }
                            Some('\\') => {
                                self.add('\\');
                                if let Some(ch) = self.hgetc() {
                                    self.add(ch);
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                return Err(());
                            }
                        }
                    }
                }
                '#' => {
                    if start {
                        self.add(c);
                        // Skip comment to end of line
                        loop {
                            let ch = self.hgetc();
                            match ch {
                                Some('\n') => {
                                    self.add('\n');
                                    break;
                                }
                                Some(ch) => self.add(ch),
                                None => break,
                            }
                        }
                    } else {
                        self.add(c);
                    }
                }
                _ => {
                    self.add(c);
                }
            }

            start = iswhite;
        }
    }

    /// Update parser state after lexing based on token type
    pub fn ctxtlex(&mut self) {
        self.zshlex();

        match self.tok {
            LexTok::Seper
            | LexTok::Newlin
            | LexTok::Semi
            | LexTok::Dsemi
            | LexTok::Semiamp
            | LexTok::Semibar
            | LexTok::Amper
            | LexTok::Amperbang
            | LexTok::Inpar
            | LexTok::Inbrace
            | LexTok::Dbar
            | LexTok::Damper
            | LexTok::Bar
            | LexTok::Baramp
            | LexTok::Inoutpar
            | LexTok::Doloop
            | LexTok::Then
            | LexTok::Elif
            | LexTok::Else
            | LexTok::Doutbrack => {
                self.incmdpos = true;
            }

            LexTok::String
            | LexTok::Typeset
            | LexTok::Envarray
            | LexTok::Outpar
            | LexTok::Case
            | LexTok::Dinbrack => {
                self.incmdpos = false;
            }

            _ => {}
        }

        if self.tok != LexTok::Dinpar {
            self.infor = if self.tok == LexTok::For { 2 } else { 0 };
        }

        let oldpos = self.incmdpos;
        if self.tok.is_redirop()
            || self.tok == LexTok::For
            || self.tok == LexTok::Foreach
            || self.tok == LexTok::Select
        {
            self.inredir = true;
            self.incmdpos = false;
        } else if self.inredir {
            self.incmdpos = oldpos;
            self.inredir = false;
        }
    }

    /// Register a heredoc to be processed at next newline
    pub fn register_heredoc(&mut self, terminator: String, strip_tabs: bool) {
        self.heredocs.push(HereDoc {
            terminator,
            strip_tabs,
            content: String::new(),
        });
    }

    /// Check for reserved word
    pub fn check_reserved_word(&mut self) -> bool {
        if let Some(ref tokstr) = self.tokstr {
            if self.incmdpos || (tokstr == "}" && self.tok == LexTok::String) {
                if let Some(tok) = crate::tokens::lookup_reserved_word(tokstr) {
                    self.tok = tok;
                    if tok == LexTok::Repeat {
                        self.inrepeat = 1;
                    }
                    if tok == LexTok::Dinbrack {
                        self.incond = 1;
                    }
                    return true;
                }
                if tokstr == "]]" && self.incond > 0 {
                    self.tok = LexTok::Doutbrack;
                    self.incond = 0;
                    return true;
                }
            }
        }
        false
    }
}

/// Result of determining if (( is arithmetic or command
enum CmdOrMath {
    Cmd,
    Math,
    Err,
}

// ============================================================================
// Additional parsing functions ported from lex.c
// ============================================================================

/// Check whether we're looking at valid numeric globbing syntax
/// (/\<[0-9]*-[0-9]*\>/). Call pointing just after the opening "<".
/// Leaves the input in the same place, returning true or false.
///
/// Port of isnumglob() from lex.c
pub fn isnumglob(input: &str, pos: usize) -> bool {
    let chars: Vec<char> = input[pos..].chars().collect();
    let mut i = 0;
    let mut expect_close = false;

    // Look for digits, then -, then digits, then >
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            i += 1;
        } else if c == '-' && !expect_close {
            expect_close = true;
            i += 1;
        } else if c == '>' && expect_close {
            return true;
        } else {
            break;
        }
    }
    false
}

/// Tokenize a string as if in double quotes.
/// This is usually called before singsub().
///
/// Port of parsestr() / parsestrnoerr() from lex.c
pub fn parsestr(s: &str) -> Result<String, String> {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                i += 1;
                if i < chars.len() {
                    let next = chars[i];
                    match next {
                        '$' | '\\' | '`' | '"' | '\n' => {
                            result.push(char_tokens::BNULL);
                            result.push(next);
                        }
                        _ => {
                            result.push('\\');
                            result.push(next);
                        }
                    }
                } else {
                    result.push('\\');
                }
            }
            '$' => {
                result.push(char_tokens::QSTRING);
                if i + 1 < chars.len() {
                    let next = chars[i + 1];
                    if next == '{' {
                        result.push(char_tokens::INBRACE);
                        i += 1;
                    } else if next == '(' {
                        result.push(char_tokens::INPAR);
                        i += 1;
                    }
                }
            }
            '`' => {
                result.push(char_tokens::QTICK);
            }
            _ => {
                result.push(c);
            }
        }
        i += 1;
    }

    Ok(result)
}

/// Parse a subscript in string s.
/// Return the position after the closing bracket, or None on error.
///
/// Port of parse_subscript() from lex.c
pub fn parse_subscript(s: &str, endchar: char) -> Option<usize> {
    if s.is_empty() || s.starts_with(endchar) {
        return None;
    }

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut depth = 0;
    let mut in_dquote = false;
    let mut in_squote = false;

    while i < chars.len() {
        let c = chars[i];

        if in_squote {
            if c == '\'' {
                in_squote = false;
            }
            i += 1;
            continue;
        }

        if in_dquote {
            if c == '"' {
                in_dquote = false;
            } else if c == '\\' && i + 1 < chars.len() {
                i += 1; // skip escaped char
            }
            i += 1;
            continue;
        }

        match c {
            '\\' => {
                i += 1; // skip next char
            }
            '\'' => {
                in_squote = true;
            }
            '"' => {
                in_dquote = true;
            }
            '[' | '(' => {
                depth += 1;
            }
            ']' | ')' => {
                if depth > 0 {
                    depth -= 1;
                } else if c == endchar {
                    return Some(i);
                }
            }
            _ => {}
        }

        if c == endchar && depth == 0 {
            return Some(i);
        }

        i += 1;
    }

    None
}

/// Tokenize a string as if it were a normal command-line argument
/// but it may contain separators. Used for ${...%...} substitutions.
///
/// Port of parse_subst_string() from lex.c
pub fn parse_subst_string(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Ok(String::new());
    }

    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                result.push(char_tokens::BNULL);
                i += 1;
                if i < chars.len() {
                    result.push(chars[i]);
                }
            }
            '\'' => {
                result.push(char_tokens::SNULL);
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    result.push(chars[i]);
                    i += 1;
                }
                result.push(char_tokens::SNULL);
            }
            '"' => {
                result.push(char_tokens::DNULL);
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        result.push(char_tokens::BNULL);
                        i += 1;
                        result.push(chars[i]);
                    } else if chars[i] == '$' {
                        result.push(char_tokens::QSTRING);
                    } else {
                        result.push(chars[i]);
                    }
                    i += 1;
                }
                result.push(char_tokens::DNULL);
            }
            '$' => {
                result.push(char_tokens::STRING);
                if i + 1 < chars.len() {
                    match chars[i + 1] {
                        '{' => {
                            result.push(char_tokens::INBRACE);
                            i += 1;
                        }
                        '(' => {
                            result.push(char_tokens::INPAR);
                            i += 1;
                        }
                        _ => {}
                    }
                }
            }
            '*' => result.push(char_tokens::STAR),
            '?' => result.push(char_tokens::QUEST),
            '[' => result.push(char_tokens::INBRACK),
            ']' => result.push(char_tokens::OUTBRACK),
            '{' => result.push(char_tokens::INBRACE),
            '}' => result.push(char_tokens::OUTBRACE),
            '~' => result.push(char_tokens::TILDE),
            '#' => result.push(char_tokens::POUND),
            '^' => result.push(char_tokens::HAT),
            _ => result.push(c),
        }
        i += 1;
    }

    Ok(result)
}

/// Untokenize a string - convert tokenized chars back to original
///
/// Port of untokenize() from exec.c (but used by lexer too)
pub fn untokenize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        // Check if it's a token character (in the special range)
        if (c as u32) < 32 {
            // Convert token back to original character
            match c {
                c if c == char_tokens::POUND => result.push('#'),
                c if c == char_tokens::STRING => result.push('$'),
                c if c == char_tokens::HAT => result.push('^'),
                c if c == char_tokens::STAR => result.push('*'),
                c if c == char_tokens::INPAR => result.push('('),
                c if c == char_tokens::OUTPAR => result.push(')'),
                c if c == char_tokens::INPARMATH => result.push('('),
                c if c == char_tokens::OUTPARMATH => result.push(')'),
                c if c == char_tokens::QSTRING => result.push('$'),
                c if c == char_tokens::EQUALS => result.push('='),
                c if c == char_tokens::BAR => result.push('|'),
                c if c == char_tokens::INBRACE => result.push('{'),
                c if c == char_tokens::OUTBRACE => result.push('}'),
                c if c == char_tokens::INBRACK => result.push('['),
                c if c == char_tokens::OUTBRACK => result.push(']'),
                c if c == char_tokens::TICK => result.push('`'),
                c if c == char_tokens::INANG => result.push('<'),
                c if c == char_tokens::OUTANG => result.push('>'),
                c if c == char_tokens::QUEST => result.push('?'),
                c if c == char_tokens::TILDE => result.push('~'),
                c if c == char_tokens::QTICK => result.push('`'),
                c if c == char_tokens::COMMA => result.push(','),
                c if c == char_tokens::DASH => result.push('-'),
                c if c == char_tokens::BANG => result.push('!'),
                c if c == char_tokens::SNULL
                    || c == char_tokens::DNULL
                    || c == char_tokens::BNULL =>
                {
                    // Null markers - skip
                }
                _ => {
                    // Unknown token, try ztokens lookup
                    let idx = c as usize;
                    if idx < char_tokens::ZTOKENS.len() {
                        result.push(char_tokens::ZTOKENS.chars().nth(idx).unwrap_or(c));
                    } else {
                        result.push(c);
                    }
                }
            }
        } else {
            result.push(c);
        }
        i += 1;
    }

    result
}

/// Check if a string contains any token characters
pub fn has_token(s: &str) -> bool {
    s.chars().any(|c| (c as u32) < 32)
}

/// Convert token characters to their printable form for display
pub fn tokens_to_printable(s: &str) -> String {
    untokenize(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let mut lexer = ZshLexer::new("echo hello");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        assert_eq!(lexer.tokstr, Some("echo".to_string()));

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        assert_eq!(lexer.tokstr, Some("hello".to_string()));

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Endinput);
    }

    #[test]
    fn test_pipeline() {
        let mut lexer = ZshLexer::new("ls | grep foo");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Bar);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_redirections() {
        let mut lexer = ZshLexer::new("echo > file");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Outang);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_heredoc() {
        let mut lexer = ZshLexer::new("cat << EOF");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Dinang);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_single_quotes() {
        let mut lexer = ZshLexer::new("echo 'hello world'");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // Should contain Snull markers around literal content
        assert!(lexer.tokstr.is_some());
    }

    #[test]
    fn test_function_tokens() {
        let mut lexer = ZshLexer::new("function foo { }");
        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Func,
            "expected Func, got {:?}",
            lexer.tok
        );

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::String,
            "expected String for 'foo', got {:?}",
            lexer.tok
        );
        assert_eq!(lexer.tokstr, Some("foo".to_string()));

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Inbrace,
            "expected Inbrace, got {:?} tokstr={:?}",
            lexer.tok,
            lexer.tokstr
        );

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Outbrace,
            "expected Outbrace, got {:?} tokstr={:?} incmdpos={}",
            lexer.tok,
            lexer.tokstr,
            lexer.incmdpos
        );
    }

    #[test]
    fn test_double_quotes() {
        let mut lexer = ZshLexer::new("echo \"hello $name\"");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // Should contain tokenized content
        assert!(lexer.tokstr.is_some());
    }

    #[test]
    fn test_command_substitution() {
        let mut lexer = ZshLexer::new("echo $(pwd)");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_env_assignment() {
        let mut lexer = ZshLexer::new("FOO=bar echo");
        lexer.incmdpos = true;
        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Envstring,
            "tok={:?} tokstr={:?}",
            lexer.tok,
            lexer.tokstr
        );

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_array_assignment() {
        let mut lexer = ZshLexer::new("arr=(a b c)");
        lexer.incmdpos = true;
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Envarray);
    }

    #[test]
    fn test_process_substitution() {
        let mut lexer = ZshLexer::new("diff <(ls) >(cat)");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // <(ls) is tokenized into the string

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // >(cat) is tokenized
    }

    #[test]
    fn test_arithmetic() {
        let mut lexer = ZshLexer::new("echo $((1+2))");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_semicolon_variants() {
        let mut lexer = ZshLexer::new("case x in a) cmd;; b) cmd;& c) cmd;| esac");

        // Skip to first ;;
        loop {
            lexer.zshlex();
            if lexer.tok == LexTok::Dsemi || lexer.tok == LexTok::Endinput {
                break;
            }
        }
        assert_eq!(lexer.tok, LexTok::Dsemi);

        // Find ;&
        loop {
            lexer.zshlex();
            if lexer.tok == LexTok::Semiamp || lexer.tok == LexTok::Endinput {
                break;
            }
        }
        assert_eq!(lexer.tok, LexTok::Semiamp);

        // Find ;|
        loop {
            lexer.zshlex();
            if lexer.tok == LexTok::Semibar || lexer.tok == LexTok::Endinput {
                break;
            }
        }
        assert_eq!(lexer.tok, LexTok::Semibar);
    }
}
